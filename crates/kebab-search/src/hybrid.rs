//! Hybrid retriever — design §3.7 / §6.4 / §0 Q3 / §1.6.
//!
//! Composes a lexical and a vector retriever (both `dyn Retriever`)
//! and dispatches by `SearchMode`. For `Hybrid`, results are fused via
//! Reciprocal Rank Fusion (RRF):
//!
//! ```text
//! score(c) = Σ_{m ∈ {lex, vec}}  1 / (k_rrf + rank_m(c))
//! ```
//!
//! where `rank_m(c)` is the 1-based rank of chunk `c` in retriever
//! `m`'s output (chunks not appearing in `m` contribute 0).
//!
//! Each `SearchHit.retrieval` is rebuilt with the per-mode scores /
//! ranks the fusion observed, so `kb search --explain` (§1.6) can
//! show users exactly which retriever contributed what to the final
//! ordering.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use kebab_core::{
    IndexVersion, RetrievalDetail, Retriever, SearchHit, SearchMode, SearchQuery, SearchTrace,
};

use crate::trace::{ScoreKind, TraceBuilder, build_fusion_input_skeleton, candidates_from_hits};

/// Default `k_rrf` if `kb-config::SearchCfg::rrf_k` is misconfigured.
/// Matches §6.4's documented default (60).
const DEFAULT_K_RRF: u32 = 60;

/// When fanning out for hybrid fusion we ask each side for `k *
/// HYBRID_FANOUT_MULTIPLIER` candidates so the disjoint set of
/// chunks (those a single retriever surfaces but the other does not)
/// is wide enough to feed a useful fused top-k.
///
/// `2` is the spec-suggested floor; raising it helps recall on
/// adversarial corpora at linear cost. Documented in
/// `tasks/p3/p3-4-hybrid-fusion.md` "Risks / notes".
const HYBRID_FANOUT_MULTIPLIER: usize = 2;

/// Default `k` when `SearchQuery::k == 0`. Mirrors §6.4 default_k=10.
const DEFAULT_K: usize = 10;

/// Fusion algorithm. Today only Reciprocal Rank Fusion is supported;
/// listing as an enum so future score-calibration policies (P+) can
/// land without an API break.
#[derive(Clone, Copy, Debug)]
pub enum FusionPolicy {
    /// Reciprocal Rank Fusion. `k_rrf` is the standard rank-bias
    /// hyperparameter (§6.4); larger values flatten the rank-bias
    /// curve, smaller values privilege top-of-list hits.
    Rrf { k_rrf: u32 },
}

/// Hybrid retriever composing a lexical and a vector retriever.
///
/// For chunks that appear in both retrievers, the lexical-side hit
/// supplies `snippet`, `citation`, `heading_path`, `chunker_version`,
/// and `embedding_model` — lexical search has FTS5 highlighting that's
/// more user-relevant than the vector retriever's truncated text.
/// Vector-only chunks fall through to the vector hit's data verbatim.
/// This matches `kb search --explain` (§1.6) expectations for snippet
/// provenance.
pub struct HybridRetriever {
    lexical: Arc<dyn Retriever>,
    vector: Arc<dyn Retriever>,
    fusion: FusionPolicy,
    /// Default `k` for queries that arrive with `k == 0`. Pulled from
    /// `config.search.default_k` at construction.
    default_k: usize,
}

impl HybridRetriever {
    /// Construct from a `kb-config` Config + the two underlying
    /// retrievers. Reads `config.search.hybrid_fusion` (only `"rrf"`
    /// is recognised today) and `config.search.rrf_k`.
    pub fn new(
        config: &kebab_config::Config,
        lexical: Arc<dyn Retriever>,
        vector: Arc<dyn Retriever>,
    ) -> Self {
        let fusion = parse_fusion(&config.search.hybrid_fusion, config.search.rrf_k);
        let default_k = if config.search.default_k == 0 {
            DEFAULT_K
        } else {
            config.search.default_k
        };
        // Surface mismatched index_version up front so users see it
        // (e.g. lexical at v2, vector at v1 means a stale index that
        // the user should refresh). Spec line 144 calls this out as
        // a "flag at construction".
        let lex_iv = lexical.index_version();
        let vec_iv = vector.index_version();
        if lex_iv.0 != vec_iv.0 {
            tracing::warn!(
                target: "kebab-search",
                lexical_index = %lex_iv.0,
                vector_index = %vec_iv.0,
                "kb-search hybrid: lexical and vector index_version differ; consider re-indexing"
            );
        }
        Self {
            lexical,
            vector,
            fusion,
            default_k,
        }
    }

    /// Construct with explicit policy / `k`. Used by tests that want
    /// to pin RRF parameters without going through `kb-config`.
    pub fn with_policy(
        lexical: Arc<dyn Retriever>,
        vector: Arc<dyn Retriever>,
        fusion: FusionPolicy,
        default_k: usize,
    ) -> Self {
        Self {
            lexical,
            vector,
            fusion,
            default_k: if default_k == 0 { DEFAULT_K } else { default_k },
        }
    }
}

impl Retriever for HybridRetriever {
    fn search(&self, query: &SearchQuery) -> Result<Vec<SearchHit>> {
        match query.mode {
            SearchMode::Lexical => self.lexical.search(query),
            SearchMode::Vector => self.vector.search(query),
            SearchMode::Hybrid => self.fuse(query),
        }
    }

    fn index_version(&self) -> IndexVersion {
        // Composite token so callers (e.g. snapshot tests) can detect
        // either side drifting without inspecting both retrievers.
        let lex = self.lexical.index_version().0;
        let vec = self.vector.index_version().0;
        IndexVersion(format!("hybrid:{lex}+{vec}"))
    }
}

impl HybridRetriever {
    fn fuse(&self, query: &SearchQuery) -> Result<Vec<SearchHit>> {
        let target_k = if query.k == 0 {
            self.default_k
        } else {
            query.k
        };
        let fanout_k = target_k.saturating_mul(HYBRID_FANOUT_MULTIPLIER);
        let lex_query = SearchQuery {
            k: fanout_k,
            ..query.clone()
        };
        let lex_hits = self.lexical.search(&lex_query)?;
        let vec_hits = self.vector.search(&lex_query)?;
        self.fuse_with_inputs(&lex_hits, &vec_hits, target_k)
    }

    fn fuse_with_inputs(
        &self,
        lex_hits: &[SearchHit],
        vec_hits: &[SearchHit],
        target_k: usize,
    ) -> Result<Vec<SearchHit>> {
        tracing::debug!(
            lex = lex_hits.len(),
            vec = vec_hits.len(),
            target_k,
            "kb-search hybrid: pre-fusion candidate counts"
        );

        // Build (chunk_id → (rank, hit)) maps. The rank stored here
        // is the `rank` field on each retriever's output, which is
        // already 1-based by both LexicalRetriever and VectorRetriever
        // (and any well-behaved Retriever should mirror).
        let lex_index: HashMap<String, (u32, SearchHit)> = lex_hits
            .iter()
            .cloned()
            .map(|h| (h.chunk_id.0.clone(), (h.rank, h)))
            .collect();
        let vec_index: HashMap<String, (u32, SearchHit)> = vec_hits
            .iter()
            .cloned()
            .map(|h| (h.chunk_id.0.clone(), (h.rank, h)))
            .collect();

        // Union of chunk_ids from both sides.
        let mut all_ids: Vec<String> = Vec::with_capacity(lex_index.len() + vec_index.len());
        for k in lex_index.keys() {
            all_ids.push(k.clone());
        }
        for k in vec_index.keys() {
            if !lex_index.contains_key(k) {
                all_ids.push(k.clone());
            }
        }

        // Compute fused score per chunk.
        //
        // Raw RRF: `Σ 1/(k_rrf + rank_m(c))` over the retrievers a chunk
        // appears in. With two retrievers the raw upper bound is
        // `2/(k_rrf + 1)` — at k_rrf=60 that's only ≈0.0328, which makes
        // a single `config.rag.score_gate` default of 0.05 silently
        // refuse every hybrid query (and is incomparable with lexical /
        // vector `fusion_score` already in [0, 1]).
        //
        // Normalize by the theoretical max so `fusion_score` lives in
        // [0, 1] across all three SearchModes. The normalization factor
        // is `num_retrievers / (k_rrf + 1)`. With both retrievers
        // contributing rank=1 the normalized score is exactly 1.0;
        // chunks present in only one retriever cap at ≈0.5 (≈ 1 / 2);
        // all other rank combinations fall in between. RRF's rank-
        // ordering invariants are preserved (we divide every score by
        // the same positive constant), so the sort + tiebreak path is
        // unchanged. Wire schema label `fusion_score` keeps its slot in
        // `RetrievalDetail`; only the magnitude shifts.
        let FusionPolicy::Rrf { k_rrf } = self.fusion;
        let k_rrf_f = f64::from(k_rrf);
        // Both retrievers can contribute, so the per-mode RRF max is
        // 2 / (k_rrf + 1). Even when a chunk lands in only one mode, we
        // still divide by this same constant — the score then caps
        // around 0.5 which is exactly the "half-aligned" semantic we
        // want users to compare against `score_gate`.
        let rrf_normalizer = 2.0_f64 / (k_rrf_f + 1.0);

        struct Scored {
            chunk_id: String,
            rrf: f64,
            lex_rank: Option<u32>,
            vec_rank: Option<u32>,
        }
        let mut scored: Vec<Scored> = all_ids
            .into_iter()
            .map(|cid| {
                let lex_rank = lex_index.get(&cid).map(|(r, _)| *r);
                let vec_rank = vec_index.get(&cid).map(|(r, _)| *r);
                let mut rrf = 0.0_f64;
                if let Some(r) = lex_rank {
                    rrf += 1.0 / (k_rrf_f + f64::from(r));
                }
                if let Some(r) = vec_rank {
                    rrf += 1.0 / (k_rrf_f + f64::from(r));
                }
                rrf /= rrf_normalizer;
                Scored {
                    chunk_id: cid,
                    rrf,
                    lex_rank,
                    vec_rank,
                }
            })
            .collect();

        // Sort: rrf DESC, then lex_rank ASC (None last), then chunk_id ASC.
        // f64 ordering uses `total_cmp` so NaN stays deterministic
        // (won't occur today — k_rrf > 0 → denominators > 0 — but
        // total_cmp keeps the sort stable under future tweaks).
        scored.sort_by(|a, b| {
            b.rrf
                .total_cmp(&a.rrf)
                .then_with(|| {
                    let am = a.lex_rank.unwrap_or(u32::MAX);
                    let bm = b.lex_rank.unwrap_or(u32::MAX);
                    am.cmp(&bm)
                })
                .then_with(|| a.chunk_id.cmp(&b.chunk_id))
        });

        // Build final SearchHits, taking the top `target_k`.
        let mut hits: Vec<SearchHit> = Vec::with_capacity(target_k.min(scored.len()));
        let mut rank: u32 = 0;
        for s in scored.into_iter().take(target_k) {
            // Pull the underlying hit. Prefer the lexical side when
            // available — its snippet has FTS5 highlighting which
            // gives users the most useful preview. Fall back to
            // vector if the chunk only appeared in vector results.
            let mut base = match (lex_index.get(&s.chunk_id), vec_index.get(&s.chunk_id)) {
                (Some((_, lex)), _) => lex.clone(),
                (None, Some((_, vec))) => vec.clone(),
                // `all_ids` is the union of `lex_index` and
                // `vec_index` keys, so this arm cannot fire.
                (None, None) => {
                    unreachable!("chunk_id was in union but absent from both indices")
                }
            };

            // `unwrap_or(fusion_score)` covers a defensive-coding case
            // that doesn't arise today: when a chunk only appears in
            // one retriever, RRF sums a single term so `fusion_score`
            // already equals that side's normalized score, making the
            // fallback harmless.
            let lex_score = lex_index.get(&s.chunk_id).map(|(_, h)| {
                h.retrieval
                    .lexical_score
                    .unwrap_or(h.retrieval.fusion_score)
            });
            let vec_score = vec_index
                .get(&s.chunk_id)
                .map(|(_, h)| h.retrieval.vector_score.unwrap_or(h.retrieval.fusion_score));

            rank = rank.saturating_add(1);
            base.rank = rank;
            base.retrieval = RetrievalDetail {
                method: SearchMode::Hybrid,
                // RRF is computed in f64 inside `fuse` and cast to f32
                // here at the boundary. `1/(k_rrf+rank)` is bounded
                // roughly in `(0, 2/k_rrf]` (≤ ~0.033 at k_rrf=60), so
                // the magnitude is well within f32 range and f32
                // precision is more than sufficient for ranking.
                fusion_score: s.rrf as f32,
                lexical_score: lex_score,
                vector_score: vec_score,
                lexical_rank: s.lex_rank,
                vector_rank: s.vec_rank,
            };
            // p9-fb-38: base was cloned from a lex/vec hit (Bm25/Cosine);
            // fuse output is RRF-scored so override.
            base.score_kind = kebab_core::ScoreKind::Rrf;
            hits.push(base);
        }

        tracing::debug!(rows = hits.len(), "kb-search hybrid: search done");
        Ok(hits)
    }

    /// p9-fb-37: parallel to `Retriever::search` but additionally returns
    /// a trace of pre-fusion lex/vec lists, RRF inputs (union with each
    /// side's rank), and per-stage timing.
    pub fn search_with_trace(
        &self,
        query: &SearchQuery,
    ) -> anyhow::Result<(Vec<SearchHit>, SearchTrace)> {
        let start_total = Instant::now();
        let target_k = if query.k == 0 {
            self.default_k
        } else {
            query.k
        };
        let fanout_k = target_k.saturating_mul(HYBRID_FANOUT_MULTIPLIER);
        let fanout_query = SearchQuery {
            k: fanout_k,
            ..query.clone()
        };

        let mut tb = TraceBuilder::default();

        let (lex_hits, vec_hits): (Vec<SearchHit>, Vec<SearchHit>) = match query.mode {
            SearchMode::Lexical => {
                let t0 = Instant::now();
                let lh = self.lexical.search(&fanout_query)?;
                tb.timing.lexical_ms = t0.elapsed().as_millis() as u64;
                (lh, Vec::new())
            }
            SearchMode::Vector => {
                let t0 = Instant::now();
                let vh = self.vector.search(&fanout_query)?;
                tb.timing.vector_ms = t0.elapsed().as_millis() as u64;
                (Vec::new(), vh)
            }
            SearchMode::Hybrid => {
                let t0 = Instant::now();
                let lh = self.lexical.search(&fanout_query)?;
                tb.timing.lexical_ms = t0.elapsed().as_millis() as u64;
                let t1 = Instant::now();
                let vh = self.vector.search(&fanout_query)?;
                tb.timing.vector_ms = t1.elapsed().as_millis() as u64;
                (lh, vh)
            }
        };

        tb.lexical = candidates_from_hits(&lex_hits, ScoreKind::Lexical);
        tb.vector = candidates_from_hits(&vec_hits, ScoreKind::Vector);
        tb.rrf_inputs = build_fusion_input_skeleton(&lex_hits, &vec_hits);

        let t_fusion = Instant::now();
        let final_hits = match query.mode {
            SearchMode::Lexical => {
                let mut h = lex_hits.clone();
                h.truncate(target_k);
                h
            }
            SearchMode::Vector => {
                let mut h = vec_hits.clone();
                h.truncate(target_k);
                h
            }
            SearchMode::Hybrid => self.fuse_with_inputs(&lex_hits, &vec_hits, target_k)?,
        };
        tb.timing.fusion_ms = t_fusion.elapsed().as_millis() as u64;

        let score_by_chunk: std::collections::HashMap<String, f32> = final_hits
            .iter()
            .map(|h| (h.chunk_id.0.clone(), h.retrieval.fusion_score))
            .collect();
        for entry in &mut tb.rrf_inputs {
            if let Some(s) = score_by_chunk.get(&entry.chunk_id.0) {
                entry.fusion_score = *s;
            }
        }

        // total_ms is wall-clock from start; per-stage `lexical_ms` /
        // `vector_ms` / `fusion_ms` each truncate to whole millis via
        // `as_millis() as u64`, so their sum can drift below total
        // (sub-ms losses) — DO NOT assert `total_ms >= sum(stages)`.
        tb.timing.total_ms = start_total.elapsed().as_millis() as u64;
        Ok((final_hits, tb.into_trace()))
    }
}

/// Parse the `hybrid_fusion` config string into a [`FusionPolicy`].
/// Today only `"rrf"` is recognised; anything else falls back to RRF
/// with a warn log so misconfiguration is visible but not fatal.
fn parse_fusion(name: &str, k_rrf: u32) -> FusionPolicy {
    let k = if k_rrf == 0 { DEFAULT_K_RRF } else { k_rrf };
    match name {
        "rrf" => FusionPolicy::Rrf { k_rrf: k },
        other => {
            tracing::warn!(
                target: "kebab-search",
                policy = other,
                "kb-search hybrid: unknown fusion policy; falling back to RRF"
            );
            FusionPolicy::Rrf { k_rrf: k }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kebab_core::{
        ChunkId, ChunkerVersion, Citation, DocumentId, IndexVersion, SearchFilters, SearchHit,
        SearchMode, WorkspacePath,
    };
    use std::sync::Mutex;

    /// Test double: returns a canned `Vec<SearchHit>` and records
    /// every call so we can assert delegation.
    struct CannedRetriever {
        hits: Vec<SearchHit>,
        calls: Mutex<Vec<SearchQuery>>,
        version: IndexVersion,
    }

    impl CannedRetriever {
        fn new(hits: Vec<SearchHit>, version: &str) -> Self {
            Self {
                hits,
                calls: Mutex::new(Vec::new()),
                version: IndexVersion(version.to_string()),
            }
        }
    }

    impl Retriever for CannedRetriever {
        fn search(&self, query: &SearchQuery) -> Result<Vec<SearchHit>> {
            self.calls.lock().unwrap().push(query.clone());
            Ok(self.hits.clone())
        }
        fn index_version(&self) -> IndexVersion {
            self.version.clone()
        }
    }

    fn wp(p: &str) -> WorkspacePath {
        WorkspacePath::new(p.to_string()).unwrap()
    }

    /// Build a synthetic `SearchHit`. Most fields take inert defaults
    /// because the hybrid logic only reads `chunk_id`, `rank`,
    /// `retrieval.{lexical,vector}_score`, and (transitively) the rest
    /// when building the fused output.
    fn mk_hit(chunk_id: &str, rank: u32, method: SearchMode, score: f32) -> SearchHit {
        let cid = ChunkId(chunk_id.to_string());
        let did = DocumentId(format!("d-{chunk_id}"));
        let path = wp(&format!("notes/{chunk_id}.md"));
        SearchHit {
            rank,
            chunk_id: cid,
            doc_id: did,
            doc_path: path.clone(),
            heading_path: vec![],
            section_label: None,
            snippet: format!("snippet for {chunk_id}"),
            citation: Citation::Line {
                path,
                start: 1,
                end: 1,
                section: None,
            },
            retrieval: RetrievalDetail {
                method,
                fusion_score: score,
                lexical_score: matches!(method, SearchMode::Lexical | SearchMode::Hybrid)
                    .then_some(score),
                vector_score: matches!(method, SearchMode::Vector | SearchMode::Hybrid)
                    .then_some(score),
                lexical_rank: matches!(method, SearchMode::Lexical | SearchMode::Hybrid)
                    .then_some(rank),
                vector_rank: matches!(method, SearchMode::Vector | SearchMode::Hybrid)
                    .then_some(rank),
            },
            index_version: IndexVersion("v1".to_string()),
            embedding_model: None,
            chunker_version: ChunkerVersion("v1".to_string()),
            // p9-fb-32: hybrid unit tests don't exercise staleness; pin
            // a fixed UNIX_EPOCH so synthetic hits remain deterministic.
            indexed_at: time::OffsetDateTime::UNIX_EPOCH,
            stale: false,
            score_kind: kebab_core::ScoreKind::Rrf,
            repo: None,
            code_lang: None,
        }
    }

    fn rrf_policy(k_rrf: u32) -> FusionPolicy {
        FusionPolicy::Rrf { k_rrf }
    }

    fn make_query(mode: SearchMode, k: usize) -> SearchQuery {
        SearchQuery {
            text: "rust".to_string(),
            mode,
            k,
            filters: SearchFilters::default(),
        }
    }

    #[test]
    fn hybrid_lexical_mode_delegates_to_lexical() {
        let lex_hits = vec![mk_hit("aaaa", 1, SearchMode::Lexical, 0.9)];
        let lex = Arc::new(CannedRetriever::new(lex_hits.clone(), "lex-v1"));
        let vec = Arc::new(CannedRetriever::new(vec![], "vec-v1"));
        let h = HybridRetriever::with_policy(lex.clone(), vec.clone(), rrf_policy(60), 5);
        let out = h.search(&make_query(SearchMode::Lexical, 5)).unwrap();
        assert_eq!(out, lex_hits, "lexical mode must pass through verbatim");
        assert_eq!(lex.calls.lock().unwrap().len(), 1, "lexical called once");
        assert_eq!(vec.calls.lock().unwrap().len(), 0, "vector NOT called");
    }

    #[test]
    fn hybrid_vector_mode_delegates_to_vector() {
        let vec_hits = vec![mk_hit("bbbb", 1, SearchMode::Vector, 0.8)];
        let lex = Arc::new(CannedRetriever::new(vec![], "lex-v1"));
        let vec = Arc::new(CannedRetriever::new(vec_hits.clone(), "vec-v1"));
        let h = HybridRetriever::with_policy(lex.clone(), vec.clone(), rrf_policy(60), 5);
        let out = h.search(&make_query(SearchMode::Vector, 5)).unwrap();
        assert_eq!(out, vec_hits, "vector mode must pass through verbatim");
        assert_eq!(lex.calls.lock().unwrap().len(), 0, "lexical NOT called");
        assert_eq!(vec.calls.lock().unwrap().len(), 1, "vector called once");
    }

    #[test]
    fn hybrid_chunk_only_in_lexical_keeps_vector_none() {
        // Chunk X is in lexical only.
        let lex = Arc::new(CannedRetriever::new(
            vec![mk_hit("xxxx", 1, SearchMode::Lexical, 0.9)],
            "lex-v1",
        ));
        let vec = Arc::new(CannedRetriever::new(
            vec![mk_hit("yyyy", 1, SearchMode::Vector, 0.8)],
            "vec-v1",
        ));
        let h = HybridRetriever::with_policy(lex, vec, rrf_policy(60), 5);
        let out = h.search(&make_query(SearchMode::Hybrid, 5)).unwrap();
        // Both X and Y are present.
        let xx = out.iter().find(|h| h.chunk_id.0 == "xxxx").unwrap();
        assert_eq!(xx.retrieval.method, SearchMode::Hybrid);
        assert!(xx.retrieval.lexical_score.is_some());
        assert_eq!(xx.retrieval.vector_score, None);
        assert_eq!(xx.retrieval.lexical_rank, Some(1));
        assert_eq!(xx.retrieval.vector_rank, None);
        assert!(xx.retrieval.fusion_score > 0.0);

        let yy = out.iter().find(|h| h.chunk_id.0 == "yyyy").unwrap();
        assert_eq!(yy.retrieval.lexical_score, None);
        assert!(yy.retrieval.vector_score.is_some());
        assert_eq!(yy.retrieval.lexical_rank, None);
        assert_eq!(yy.retrieval.vector_rank, Some(1));
    }

    #[test]
    fn rrf_formula_matches_known_value() {
        // chunk A appears at lexical rank 1, vector rank 2; k_rrf=60.
        // Raw RRF: 1/(60+1) + 1/(60+2) = 1/61 + 1/62.
        // After normalization by `2 / (60 + 1)` (theoretical max with
        // both retrievers contributing rank=1), the score lives in
        // [0, 1]: `(1/61 + 1/62) / (2/61) = 0.5 + 61/124 ≈ 0.9919`.
        let raw = 1.0_f64 / 61.0 + 1.0_f64 / 62.0;
        let expected = raw / (2.0_f64 / 61.0);
        let lex = Arc::new(CannedRetriever::new(
            vec![mk_hit("aaaa", 1, SearchMode::Lexical, 0.5)],
            "lex-v1",
        ));
        let vec_hits = vec![
            mk_hit("zzzz", 1, SearchMode::Vector, 0.9),
            mk_hit("aaaa", 2, SearchMode::Vector, 0.7),
        ];
        let vec = Arc::new(CannedRetriever::new(vec_hits, "vec-v1"));
        let h = HybridRetriever::with_policy(lex, vec, rrf_policy(60), 5);
        let out = h.search(&make_query(SearchMode::Hybrid, 5)).unwrap();
        let a = out.iter().find(|h| h.chunk_id.0 == "aaaa").unwrap();
        let actual = f64::from(a.retrieval.fusion_score);
        // Tolerance: the score is computed in f64 and cast to f32 at
        // the API boundary, so any discrepancy must fit within f32
        // precision. `1e-7` is below `f32::EPSILON` (~1.19e-7), which
        // makes the check brittle on edge cases. Use a small multiple
        // of EPSILON to stay robust.
        let tol = f64::from(f32::EPSILON) * 10.0;
        assert!(
            (actual - expected).abs() < tol,
            "RRF score {actual} drifted from expected {expected} (tol {tol})"
        );
    }

    #[test]
    fn hybrid_tiebreak_prefers_lower_lexical_rank_then_chunk_id() {
        // Construct two chunks with identical fused scores.
        // Strategy: A appears at lex rank 2 only → score = 1/62.
        //           B appears at vec rank 2 only → score = 1/62.
        // Tie-break: lex_rank ascending (Some(2) < None), so A wins.
        let lex = Arc::new(CannedRetriever::new(
            vec![
                mk_hit("zzzz", 1, SearchMode::Lexical, 0.9), // rank 1: high RRF, leader
                mk_hit("aaaa", 2, SearchMode::Lexical, 0.5), // rank 2
            ],
            "lex-v1",
        ));
        let vec = Arc::new(CannedRetriever::new(
            vec![
                mk_hit("zzzz", 1, SearchMode::Vector, 0.9),
                mk_hit("bbbb", 2, SearchMode::Vector, 0.5),
            ],
            "vec-v1",
        ));
        let h = HybridRetriever::with_policy(lex, vec, rrf_policy(60), 5);
        let out = h.search(&make_query(SearchMode::Hybrid, 5)).unwrap();

        // zzzz has both ranks → strictly higher RRF → rank 1.
        assert_eq!(out[0].chunk_id.0, "zzzz");

        // aaaa and bbbb both have a single rank-2 contribution → identical
        // RRF. Tie-break: aaaa has lex_rank=Some(2), bbbb has lex_rank=None,
        // so aaaa comes first.
        assert_eq!(out[1].chunk_id.0, "aaaa");
        assert_eq!(out[2].chunk_id.0, "bbbb");

        // Now construct two chunks with identical lex rank to verify
        // the chunk_id tie-break. CannedRetriever can't produce two
        // hits at the same rank via mk_hit's normal flow, so we patch
        // `retrieval.lexical_rank` directly after construction.
        let mut tied_a = mk_hit("aaaa", 2, SearchMode::Lexical, 0.4);
        tied_a.retrieval.lexical_rank = Some(2);
        let mut tied_b = mk_hit("bbbb", 2, SearchMode::Lexical, 0.4);
        tied_b.retrieval.lexical_rank = Some(2);
        let lex3 = Arc::new(CannedRetriever::new(vec![tied_a, tied_b], "lex-v1"));
        let vec3 = Arc::new(CannedRetriever::new(vec![], "vec-v1"));
        let h3 = HybridRetriever::with_policy(lex3, vec3, rrf_policy(60), 5);
        let out3 = h3.search(&make_query(SearchMode::Hybrid, 5)).unwrap();
        // Same lex_rank=2 → tie-break on chunk_id ascending: aaaa < bbbb.
        assert_eq!(out3[0].chunk_id.0, "aaaa");
        assert_eq!(out3[1].chunk_id.0, "bbbb");
    }

    #[test]
    fn hybrid_index_version_is_composite() {
        let lex = Arc::new(CannedRetriever::new(vec![], "lex-v1"));
        let vec = Arc::new(CannedRetriever::new(vec![], "vec-v2"));
        let h = HybridRetriever::with_policy(lex, vec, rrf_policy(60), 5);
        assert_eq!(h.index_version().0, "hybrid:lex-v1+vec-v2");
    }

    #[test]
    fn hybrid_disjoint_recall_returns_all_when_k_large_enough() {
        // lex returns [A, B], vec returns [C, D]; k=4 → all 4 in result.
        let lex = Arc::new(CannedRetriever::new(
            vec![
                mk_hit("aaaa", 1, SearchMode::Lexical, 0.9),
                mk_hit("bbbb", 2, SearchMode::Lexical, 0.7),
            ],
            "lex-v1",
        ));
        let vec = Arc::new(CannedRetriever::new(
            vec![
                mk_hit("cccc", 1, SearchMode::Vector, 0.9),
                mk_hit("dddd", 2, SearchMode::Vector, 0.7),
            ],
            "vec-v1",
        ));
        let h = HybridRetriever::with_policy(lex, vec, rrf_policy(60), 4);
        let out = h.search(&make_query(SearchMode::Hybrid, 4)).unwrap();
        let mut ids: Vec<&str> = out.iter().map(|h| h.chunk_id.0.as_str()).collect();
        ids.sort_unstable();
        assert_eq!(ids, vec!["aaaa", "bbbb", "cccc", "dddd"]);
    }

    #[test]
    fn hybrid_zero_k_uses_default() {
        // With query.k=0, hybrid should use the configured default_k.
        let lex = Arc::new(CannedRetriever::new(
            (0..20)
                .map(|i| mk_hit(&format!("c{i:04}"), i + 1, SearchMode::Lexical, 0.5))
                .collect(),
            "lex-v1",
        ));
        let vec = Arc::new(CannedRetriever::new(vec![], "vec-v1"));
        let h = HybridRetriever::with_policy(lex, vec, rrf_policy(60), 7);
        let out = h.search(&make_query(SearchMode::Hybrid, 0)).unwrap();
        assert_eq!(out.len(), 7);
    }

    #[test]
    fn parse_fusion_falls_back_to_rrf_on_unknown() {
        let p = parse_fusion("nonsense", 60);
        let FusionPolicy::Rrf { k_rrf } = p;
        assert_eq!(k_rrf, 60);
    }

    #[test]
    fn parse_fusion_zero_k_falls_back_to_default() {
        let FusionPolicy::Rrf { k_rrf } = parse_fusion("rrf", 0);
        assert_eq!(k_rrf, DEFAULT_K_RRF);
    }

    #[test]
    fn search_with_trace_returns_lex_and_vec_lists() {
        use kebab_core::{
            ChunkId, ChunkerVersion, Citation, DocumentId, IndexVersion, RetrievalDetail,
            SearchHit, SearchMode, SearchQuery, WorkspacePath,
        };
        use std::sync::Arc;

        fn mk_hit(rank: u32, chunk: &str, score: f32, mode: SearchMode) -> SearchHit {
            SearchHit {
                rank,
                chunk_id: ChunkId(chunk.into()),
                doc_id: DocumentId(format!("d-{chunk}")),
                doc_path: WorkspacePath::new(format!("{chunk}.md")).unwrap(),
                heading_path: vec![],
                section_label: None,
                snippet: chunk.into(),
                citation: Citation::Line {
                    path: WorkspacePath::new(format!("{chunk}.md")).unwrap(),
                    start: 1,
                    end: 1,
                    section: None,
                },
                retrieval: RetrievalDetail {
                    method: mode,
                    fusion_score: score,
                    lexical_score: if mode == SearchMode::Lexical {
                        Some(score)
                    } else {
                        None
                    },
                    vector_score: if mode == SearchMode::Vector {
                        Some(score)
                    } else {
                        None
                    },
                    lexical_rank: if mode == SearchMode::Lexical {
                        Some(rank)
                    } else {
                        None
                    },
                    vector_rank: if mode == SearchMode::Vector {
                        Some(rank)
                    } else {
                        None
                    },
                },
                index_version: IndexVersion("v1".into()),
                embedding_model: None,
                chunker_version: ChunkerVersion("c1".into()),
                indexed_at: time::OffsetDateTime::UNIX_EPOCH,
                stale: false,
                score_kind: kebab_core::ScoreKind::Rrf,
                repo: None,
                code_lang: None,
            }
        }

        struct Stub {
            hits: Vec<SearchHit>,
        }
        impl Retriever for Stub {
            fn search(&self, _q: &SearchQuery) -> anyhow::Result<Vec<SearchHit>> {
                Ok(self.hits.clone())
            }
            fn index_version(&self) -> IndexVersion {
                IndexVersion("v1".into())
            }
        }

        let lex = Arc::new(Stub {
            hits: vec![
                mk_hit(1, "c1", 0.9, SearchMode::Lexical),
                mk_hit(2, "c2", 0.5, SearchMode::Lexical),
            ],
        });
        let vec_r = Arc::new(Stub {
            hits: vec![
                mk_hit(1, "c2", 0.8, SearchMode::Vector),
                mk_hit(2, "c3", 0.6, SearchMode::Vector),
            ],
        });
        let hybrid = HybridRetriever::with_policy(
            lex.clone(),
            vec_r.clone(),
            FusionPolicy::Rrf { k_rrf: 60 },
            2,
        );
        let q = SearchQuery {
            text: "x".into(),
            mode: SearchMode::Hybrid,
            k: 2,
            filters: Default::default(),
        };
        let (hits, trace) = hybrid.search_with_trace(&q).unwrap();
        assert!(!hits.is_empty());
        assert_eq!(trace.lexical.len(), 2);
        assert_eq!(trace.vector.len(), 2);
        // Union: c1, c2, c3 → 3 entries.
        assert_eq!(trace.rrf_inputs.len(), 3);
    }

    #[test]
    fn search_with_trace_lexical_mode_empty_vector() {
        use kebab_core::{IndexVersion, SearchMode, SearchQuery};
        use std::sync::Arc;
        struct EmptyR;
        impl Retriever for EmptyR {
            fn search(&self, _q: &SearchQuery) -> anyhow::Result<Vec<kebab_core::SearchHit>> {
                Ok(vec![])
            }
            fn index_version(&self) -> IndexVersion {
                IndexVersion("v1".into())
            }
        }
        let lex = Arc::new(EmptyR);
        let vec_r = Arc::new(EmptyR);
        let hybrid = HybridRetriever::with_policy(lex, vec_r, FusionPolicy::Rrf { k_rrf: 60 }, 2);
        let q = SearchQuery {
            text: "x".into(),
            mode: SearchMode::Lexical,
            k: 2,
            filters: Default::default(),
        };
        let (_hits, trace) = hybrid.search_with_trace(&q).unwrap();
        assert!(trace.vector.is_empty());
        assert_eq!(trace.timing.vector_ms, 0);
    }

    #[test]
    fn hybrid_fuse_labels_hits_as_rrf() {
        use kebab_core::{ScoreKind, SearchMode, SearchQuery};
        use std::sync::Arc;

        struct Stub {
            hits: Vec<kebab_core::SearchHit>,
        }
        impl Retriever for Stub {
            fn search(&self, _q: &SearchQuery) -> anyhow::Result<Vec<kebab_core::SearchHit>> {
                Ok(self.hits.clone())
            }
            fn index_version(&self) -> kebab_core::IndexVersion {
                kebab_core::IndexVersion("v1".into())
            }
        }

        let lex = Arc::new(Stub {
            hits: vec![mk_hit("c1", 1, SearchMode::Lexical, 0.9)],
        });
        let vec_r = Arc::new(Stub {
            hits: vec![mk_hit("c1", 1, SearchMode::Vector, 0.8)],
        });
        let hybrid = HybridRetriever::with_policy(lex, vec_r, FusionPolicy::Rrf { k_rrf: 60 }, 2);
        let q = SearchQuery {
            text: "x".into(),
            mode: SearchMode::Hybrid,
            k: 1,
            filters: Default::default(),
        };
        let hits = hybrid.search(&q).unwrap();
        assert!(!hits.is_empty());
        assert_eq!(hits[0].score_kind, ScoreKind::Rrf);
    }

    #[test]
    fn hybrid_search_with_trace_lexical_mode_passes_through_bm25() {
        use kebab_core::{ScoreKind, SearchMode, SearchQuery};
        use std::sync::Arc;

        struct Stub {
            hits: Vec<kebab_core::SearchHit>,
        }
        impl Retriever for Stub {
            fn search(&self, _q: &SearchQuery) -> anyhow::Result<Vec<kebab_core::SearchHit>> {
                Ok(self.hits.clone())
            }
            fn index_version(&self) -> kebab_core::IndexVersion {
                kebab_core::IndexVersion("v1".into())
            }
        }

        // mk_hit defaults to Rrf; override per spec for this test.
        let mut lex_hit = mk_hit("c1", 1, SearchMode::Lexical, 0.5);
        lex_hit.score_kind = ScoreKind::Bm25;
        let lex = Arc::new(Stub {
            hits: vec![lex_hit],
        });
        let vec_r = Arc::new(Stub { hits: vec![] });
        let hybrid = HybridRetriever::with_policy(lex, vec_r, FusionPolicy::Rrf { k_rrf: 60 }, 2);
        let q = SearchQuery {
            text: "x".into(),
            mode: SearchMode::Lexical,
            k: 1,
            filters: Default::default(),
        };
        let (hits, _trace) = hybrid.search_with_trace(&q).unwrap();
        assert!(!hits.is_empty());
        // search_with_trace mode=Lexical passes through underlying hits.
        assert_eq!(hits[0].score_kind, ScoreKind::Bm25);
    }
}
