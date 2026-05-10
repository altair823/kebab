//! Aggregate metrics over a stored eval run (P5-2 — design §5.7).
//!
//! Reads `eval_query_results` rows for one `run_id`, re-loads the
//! golden YAML (so `expected_*` / `must_contain` / `forbidden` are at
//! hand), and produces an [`AggregateMetrics`]. [`store_aggregate`]
//! writes the JSON form back into `eval_runs.aggregate_json`.
//!
//! Pure computation — no `kb-app` / retrieval / embedding imports.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use kebab_config::Config;
use kebab_core::{ChunkId, Citation, DocumentId};
use kebab_store_sqlite::SqliteStore;

use crate::loader::load_golden_set;
use crate::types::{GoldenQuery, QueryResult};

/// `k` values reported in `hit@k` and `recall@k_doc`. Pinned by spec
/// (`tasks/p5/p5-2-metrics-compare.md`); a 4-element array keeps the
/// downstream `BTreeMap<u32, f32>` keys stable across runs.
pub const TOP_K_VARIANTS: &[u32] = &[1, 3, 5, 10];

/// `MRR` floor: chunks ranked outside the top-10 contribute 0 to the
/// reciprocal sum (matches the spec — "0 if not found in top-10").
const MRR_TOP: u32 = 10;

/// Number of fractional digits aggregate metric values are rounded to
/// before storage / snapshot. Small enough that floating-point sum
/// drift across architectures cancels, large enough that genuine
/// differences (e.g., one extra hit out of ~50 queries) survive.
const STORAGE_DECIMALS: u32 = 4;

/// Env var that overrides the default `fixtures/golden_queries.yaml`
/// path during metric computation. Must be the same path the runner
/// (P5-1) used — otherwise `expected_*` / `must_contain` won't line up
/// with the stored `query_id`s. `pub(crate)` so the runner shares the
/// exact same name + default rather than duplicating constants.
pub(crate) const KEBAB_EVAL_GOLDEN: &str = "KEBAB_EVAL_GOLDEN";

/// Default golden YAML path (relative to CWD when set). Same
/// rationale as [`KEBAB_EVAL_GOLDEN`] — single source of truth.
pub(crate) const DEFAULT_GOLDEN_PATH: &str = "fixtures/golden_queries.yaml";

/// Aggregate metrics for one stored eval run.
///
/// The `f32` fields use a custom serializer that emits JSON `null` for
/// `NaN` (zero-denominator metrics). `BTreeMap<u32, f32>` keys produce
/// stringified-integer JSON object keys, which is the standard
/// `serde_json` behavior — downstream comparisons / snapshots rely on
/// that ordering, hence `BTreeMap` (not `HashMap`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AggregateMetrics {
    pub hit_at_k: BTreeMap<u32, f32>,
    pub mrr: f32,
    pub recall_at_k_doc: BTreeMap<u32, f32>,
    #[serde(
        serialize_with = "serialize_f32_nan_as_null",
        deserialize_with = "deserialize_f32_or_nan"
    )]
    pub citation_coverage: f32,
    pub groundedness: f32,
    pub empty_result_rate: f32,
    #[serde(
        serialize_with = "serialize_f32_nan_as_null",
        deserialize_with = "deserialize_f32_or_nan"
    )]
    pub refusal_correctness: f32,
    pub total_queries: u32,
    pub failed_queries: u32,
}

/// Custom serializer that maps `f32::NAN` to JSON `null`. Used on the
/// two fields whose denominator can legitimately be zero (no RAG
/// answers; no "should refuse" queries) — every other metric defaults
/// to `0.0` when the denominator is zero, since the corresponding
/// "this should be measured" set is always non-empty in practice.
fn serialize_f32_nan_as_null<S: Serializer>(v: &f32, s: S) -> std::result::Result<S::Ok, S::Error> {
    if v.is_nan() {
        s.serialize_none()
    } else {
        s.serialize_f32(*v)
    }
}

/// Inverse of [`serialize_f32_nan_as_null`]: JSON `null` → `f32::NAN`.
/// Lets `serde_json::from_str::<AggregateMetrics>` round-trip the
/// stored `aggregate_json`.
fn deserialize_f32_or_nan<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<f32, D::Error> {
    let opt: Option<f32> = Option::deserialize(d)?;
    Ok(opt.unwrap_or(f32::NAN))
}

/// Compute aggregate metrics for `run_id` against the active
/// XDG-loaded [`Config`]. Wraps [`compute_aggregate_with_config`] with
/// `Config::load(None)`.
pub fn compute_aggregate(run_id: &str) -> Result<AggregateMetrics> {
    let cfg = Config::load(None).context("load Config for compute_aggregate")?;
    compute_aggregate_with_config(&cfg, run_id)
}

/// Compute aggregate metrics for `run_id` against an explicit
/// [`Config`] (used by tests with a TempDir-backed `data_dir`).
pub fn compute_aggregate_with_config(cfg: &Config, run_id: &str) -> Result<AggregateMetrics> {
    let store = SqliteStore::open(cfg).context("open SqliteStore for compute_aggregate")?;
    store
        .run_migrations()
        .context("run migrations for compute_aggregate")?;
    if store
        .load_eval_run(run_id)
        .context("load eval_runs row")?
        .is_none()
    {
        anyhow::bail!("compute_aggregate: no eval_runs row for run_id {run_id}");
    }
    let rows = store
        .load_eval_query_results(run_id)
        .context("load eval_query_results")?;
    let queries = load_golden_for_metrics()?;
    aggregate_from_rows(&queries, &rows)
}

/// Persist `agg` into `eval_runs.aggregate_json` for `run_id`. Wraps
/// [`store_aggregate_with_config`] with `Config::load(None)`.
pub fn store_aggregate(run_id: &str, agg: &AggregateMetrics) -> Result<()> {
    let cfg = Config::load(None).context("load Config for store_aggregate")?;
    store_aggregate_with_config(&cfg, run_id, agg)
}

/// Persist `agg` into `eval_runs.aggregate_json` for `run_id` against
/// an explicit [`Config`].
pub fn store_aggregate_with_config(
    cfg: &Config,
    run_id: &str,
    agg: &AggregateMetrics,
) -> Result<()> {
    let store = SqliteStore::open(cfg).context("open SqliteStore for store_aggregate")?;
    store.run_migrations().context("run migrations")?;
    let json = serde_json::to_string(agg).context("serialize AggregateMetrics")?;
    store
        .update_eval_run_aggregate(run_id, &json)
        .with_context(|| format!("update eval_runs.aggregate_json for {run_id}"))?;
    Ok(())
}

/// Resolve the golden YAML path for metric reload — same env override
/// the runner uses, same default path. Pulled into its own helper so
/// `compare_runs` can share it.
pub(crate) fn resolve_golden_path() -> PathBuf {
    match std::env::var(KEBAB_EVAL_GOLDEN) {
        Ok(s) if !s.is_empty() => PathBuf::from(s),
        _ => PathBuf::from(DEFAULT_GOLDEN_PATH),
    }
}

fn load_golden_for_metrics() -> Result<Vec<GoldenQuery>> {
    let path = resolve_golden_path();
    load_golden_set(&path).with_context(|| {
        format!(
            "load golden set from {} (override via KEBAB_EVAL_GOLDEN)",
            path.display()
        )
    })
}

/// Pure computation kernel. Split out so unit tests can drive metrics
/// off hand-rolled `(GoldenQuery, QueryResult)` fixtures without
/// touching SQLite. No `&SqliteStore` parameter — the current metric
/// formulas don't need DB lookups; once `citation_coverage` graduates
/// to a per-citation `document_exists_by_path` probe (see deferral in
/// `tasks/p5/p5-2-metrics-compare.md`), this will need to take one.
pub(crate) fn aggregate_from_rows(
    queries: &[GoldenQuery],
    rows: &[kebab_store_sqlite::EvalQueryResultRecord],
) -> Result<AggregateMetrics> {
    let golden_by_id: HashMap<&str, &GoldenQuery> =
        queries.iter().map(|q| (q.id.as_str(), q)).collect();

    let total_queries = u32::try_from(rows.len()).unwrap_or(u32::MAX);
    let mut failed_queries: u32 = 0;

    let mut hit_at_k: BTreeMap<u32, (u32, u32)> =
        TOP_K_VARIANTS.iter().map(|k| (*k, (0_u32, 0_u32))).collect();
    let mut recall_at_k_doc: BTreeMap<u32, (f64, u32)> =
        TOP_K_VARIANTS.iter().map(|k| (*k, (0.0_f64, 0_u32))).collect();

    let mut mrr_sum: f64 = 0.0;
    let mut mrr_denom: u32 = 0;

    let mut empty_result_count: u32 = 0;

    let mut groundedness_num: u32 = 0;
    let mut groundedness_denom: u32 = 0;

    let mut citation_num: u32 = 0;
    let mut citation_denom: u32 = 0;

    let mut refusal_num: u32 = 0;
    let mut refusal_denom: u32 = 0;

    for row in rows {
        let qr: QueryResult = serde_json::from_str(&row.result_json)
            .with_context(|| format!("parse result_json for {}", row.query_id))?;
        if qr.error.is_some() {
            failed_queries += 1;
        }
        if qr.hits_top_k.is_empty() {
            empty_result_count += 1;
        }

        let Some(gq) = golden_by_id.get(qr.query_id.as_str()) else {
            // Stored row has no golden entry — skip metric updates;
            // the run still counts in `total_queries` so the run-vs-
            // golden mismatch is auditable.
            continue;
        };

        // hit@k + MRR (chunk-level, requires non-empty expected_chunk_ids)
        if !gq.expected_chunk_ids.is_empty() {
            let expected: HashSet<&ChunkId> = gq.expected_chunk_ids.iter().collect();
            let first_hit_rank = qr
                .hits_top_k
                .iter()
                .filter(|h| expected.contains(&h.chunk_id))
                .map(|h| h.rank)
                .min();
            for k in TOP_K_VARIANTS {
                let entry = hit_at_k.get_mut(k).expect("init");
                entry.1 += 1;
                if let Some(rank) = first_hit_rank
                    && rank <= *k
                {
                    entry.0 += 1;
                }
            }
            mrr_denom += 1;
            if let Some(rank) = first_hit_rank
                && rank <= MRR_TOP
            {
                mrr_sum += 1.0 / f64::from(rank);
            }
        }

        // recall@k_doc (doc-level, requires non-empty expected_doc_ids
        // and `>0` is the "should retrieve" condition; refusal queries
        // (`expected_doc_ids = []`) are excluded by spec).
        if !gq.expected_doc_ids.is_empty() {
            let expected_docs: HashSet<&DocumentId> = gq.expected_doc_ids.iter().collect();
            for k in TOP_K_VARIANTS {
                let entry = recall_at_k_doc.get_mut(k).expect("init");
                entry.1 += 1;
                let topk_docs: HashSet<&DocumentId> = qr
                    .hits_top_k
                    .iter()
                    .filter(|h| h.rank <= *k)
                    .map(|h| &h.doc_id)
                    .collect();
                let covered = expected_docs.iter().filter(|d| topk_docs.contains(*d)).count();
                let frac = covered as f64 / expected_docs.len() as f64;
                entry.0 += frac;
            }
        } else {
            // refusal_correctness: golden marks "should refuse" via empty
            // expected_doc_ids. We can only judge this on RAG runs — a
            // lexical-only run produces no Answer, so "refusal" is
            // undefined. Excluding such queries from the denominator
            // (rather than counting them as failures) keeps the metric
            // honest: a search-only run reports refusal_correctness as
            // NaN/null, not 0.0.
            if let Some(ans) = &qr.answer {
                refusal_denom += 1;
                if !ans.grounded {
                    refusal_num += 1;
                }
            }
        }

        // groundedness + citation_coverage (only meaningful with RAG
        // answers; skip queries that errored or had no Answer).
        if let Some(answer) = &qr.answer
            && qr.error.is_none()
        {
            // Skip "no-check" goldens (both must_contain and forbidden
            // empty) so an unconfigured golden entry doesn't get a free
            // 1.0 / 0.0 split. Refusal-class queries land here too;
            // their groundedness is judged via refusal_correctness.
            if !gq.must_contain.is_empty() || !gq.forbidden.is_empty() {
                groundedness_denom += 1;
                let grounded_ok = gq.must_contain.iter().all(|s| answer.answer.contains(s))
                    && !gq.forbidden.iter().any(|s| answer.answer.contains(s));
                if grounded_ok {
                    groundedness_num += 1;
                }
            }
            // citation_coverage: denominator is grounded RAG answers
            // (refusals don't drag it down). The spec calls for "every
            // citation resolves to a real chunk in the DB"; the current
            // implementation is intentionally weaker — see
            // `tasks/p5/p5-2-metrics-compare.md` "Implementation
            // deviations" for the deferral rationale. Today: an Answer
            // counts as fully covered iff (a) it carries at least one
            // citation (so empty-citations doesn't sneak through
            // `Iterator::all`'s vacuous-true) and (b) every citation's
            // path is non-empty. Tightening to a per-citation
            // SqliteStore probe is the obvious next step once
            // `document_exists_by_path` lands in `kb-store-sqlite`.
            if answer.grounded {
                citation_denom += 1;
                let covered = !answer.citations.is_empty()
                    && answer.citations.iter().all(|c| match &c.citation {
                        Citation::Line { path, .. }
                        | Citation::Page { path, .. }
                        | Citation::Region { path, .. }
                        | Citation::Caption { path, .. }
                        | Citation::Time { path, .. } => !path.0.is_empty(),
                    });
                if covered {
                    citation_num += 1;
                }
            }
        }
    }

    Ok(AggregateMetrics {
        hit_at_k: round_ratio_map(&hit_at_k),
        mrr: round_storage(if mrr_denom == 0 {
            0.0
        } else {
            mrr_sum / f64::from(mrr_denom)
        }),
        recall_at_k_doc: round_recall_map(&recall_at_k_doc),
        citation_coverage: ratio_or_nan(citation_num, citation_denom),
        groundedness: ratio_or_zero(groundedness_num, groundedness_denom),
        empty_result_rate: ratio_or_zero(empty_result_count, total_queries),
        refusal_correctness: ratio_or_nan(refusal_num, refusal_denom),
        total_queries,
        failed_queries,
    })
}

fn round_storage(v: f64) -> f32 {
    if v.is_nan() {
        return f32::NAN;
    }
    let scale = 10_f64.powi(STORAGE_DECIMALS as i32);
    ((v * scale).round() / scale) as f32
}

fn round_ratio_map(m: &BTreeMap<u32, (u32, u32)>) -> BTreeMap<u32, f32> {
    m.iter()
        .map(|(k, (num, denom))| {
            let v = if *denom == 0 {
                0.0
            } else {
                f64::from(*num) / f64::from(*denom)
            };
            (*k, round_storage(v))
        })
        .collect()
}

fn round_recall_map(m: &BTreeMap<u32, (f64, u32)>) -> BTreeMap<u32, f32> {
    m.iter()
        .map(|(k, (sum, denom))| {
            let v = if *denom == 0 {
                0.0
            } else {
                *sum / f64::from(*denom)
            };
            (*k, round_storage(v))
        })
        .collect()
}

fn ratio_or_nan(num: u32, denom: u32) -> f32 {
    if denom == 0 {
        f32::NAN
    } else {
        round_storage(f64::from(num) / f64::from(denom))
    }
}

fn ratio_or_zero(num: u32, denom: u32) -> f32 {
    if denom == 0 {
        0.0
    } else {
        round_storage(f64::from(num) / f64::from(denom))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kebab_core::{
        ChunkId, ChunkerVersion, Citation, DocumentId, IndexVersion, RetrievalDetail, SearchHit,
        SearchMode,
    };
    use kebab_core::asset::WorkspacePath;
    use kebab_core::media::Lang;
    use kebab_core::answer::{Answer, AnswerCitation, AnswerRetrievalSummary, ModelRef, TokenUsage, TraceId};
    use kebab_core::versions::PromptTemplateVersion;
    use time::OffsetDateTime;

    fn gq(id: &str, expected_chunks: &[&str], expected_docs: &[&str]) -> GoldenQuery {
        GoldenQuery {
            id: id.into(),
            query: format!("q-{id}"),
            lang: Lang(String::new()),
            expected_doc_ids: expected_docs.iter().map(|s| DocumentId((*s).into())).collect(),
            expected_chunk_ids: expected_chunks.iter().map(|s| ChunkId((*s).into())).collect(),
            must_contain: vec![],
            forbidden: vec![],
            difficulty: None,
        }
    }

    fn hit(rank: u32, chunk_id: &str, doc_id: &str) -> SearchHit {
        SearchHit {
            rank,
            chunk_id: ChunkId(chunk_id.into()),
            doc_id: DocumentId(doc_id.into()),
            doc_path: WorkspacePath::new(format!("docs/{doc_id}.md")).unwrap(),
            heading_path: vec!["root".into()],
            section_label: None,
            snippet: "s".into(),
            citation: Citation::Line {
                path: WorkspacePath::new(format!("docs/{doc_id}.md")).unwrap(),
                start: 1,
                end: 1,
                section: None,
            },
            retrieval: RetrievalDetail {
                method: SearchMode::Lexical,
                fusion_score: 1.0,
                lexical_score: Some(1.0),
                vector_score: None,
                lexical_rank: Some(rank),
                vector_rank: None,
            },
            index_version: IndexVersion(format!("idx@{rank}")),
            embedding_model: None,
            chunker_version: ChunkerVersion("test@1".into()),
            // fb-32: synthetic eval fixtures don't exercise staleness;
            // pin UNIX_EPOCH + stale=false so hits stay deterministic.
            indexed_at: OffsetDateTime::UNIX_EPOCH,
            stale: false,
            score_kind: kebab_core::ScoreKind::Rrf,
        }
    }

    fn qr(id: &str, hits: Vec<SearchHit>, error: Option<String>, answer: Option<Answer>) -> QueryResult {
        QueryResult {
            query_id: id.into(),
            query: format!("q-{id}"),
            mode: SearchMode::Lexical,
            hits_top_k: hits,
            answer,
            elapsed_ms: 1,
            error,
        }
    }

    fn record(id: &str, hits: Vec<SearchHit>, error: Option<String>, answer: Option<Answer>)
        -> kebab_store_sqlite::EvalQueryResultRecord
    {
        kebab_store_sqlite::EvalQueryResultRecord {
            query_id: id.into(),
            result_json: serde_json::to_string(&qr(id, hits, error, answer)).unwrap(),
        }
    }

    fn answer(text: &str, grounded: bool, citation_paths: &[&str]) -> Answer {
        Answer {
            answer: text.into(),
            citations: citation_paths.iter().map(|p| AnswerCitation {
                marker: None,
                citation: Citation::Line {
                    path: WorkspacePath::new((*p).into()).unwrap(),
                    start: 1,
                    end: 1,
                    section: None,
                },
                // fb-32: synthetic eval citations don't exercise staleness.
                indexed_at: OffsetDateTime::UNIX_EPOCH,
                stale: false,
            }).collect(),
            grounded,
            refusal_reason: None,
            model: ModelRef { id: "m".into(), provider: "p".into(), dimensions: None },
            embedding: None,
            prompt_template_version: PromptTemplateVersion("p@1".into()),
            retrieval: AnswerRetrievalSummary {
                trace_id: TraceId("t".into()),
                mode: SearchMode::Lexical,
                k: 5,
                score_gate: 0.0,
                top_score: 1.0,
                chunks_returned: 1,
                chunks_used: 1,
            },
            usage: TokenUsage { prompt_tokens: 1, completion_tokens: 1, latency_ms: 1 },
            created_at: OffsetDateTime::UNIX_EPOCH,
            conversation_id: None,
            turn_index: None,
        }
    }

    #[test]
    fn hit_at_k_handles_ranks_1_4_miss() {
        // q1: hit @ rank 1, q2: hit @ rank 4, q3: miss
        let queries = vec![
            gq("q1", &["c1"], &["d1"]),
            gq("q2", &["c2"], &["d2"]),
            gq("q3", &["c3"], &["d3"]),
        ];
        let rows = vec![
            record("q1", vec![hit(1, "c1", "d1")], None, None),
            record("q2", vec![hit(1, "x", "y"), hit(2, "x", "y"), hit(3, "x", "y"), hit(4, "c2", "d2")], None, None),
            record("q3", vec![hit(1, "x", "y")], None, None),
        ];
        let agg = aggregate_from_rows(&queries, &rows).unwrap();
        // hit@1 = 1/3 (q1 only), hit@3 = 1/3, hit@5 = 2/3, hit@10 = 2/3
        assert_eq!(agg.hit_at_k[&1], 0.3333);
        assert_eq!(agg.hit_at_k[&3], 0.3333);
        assert_eq!(agg.hit_at_k[&5], 0.6667);
        assert_eq!(agg.hit_at_k[&10], 0.6667);
    }

    #[test]
    fn mrr_matches_expected() {
        // q1 rank 1 → 1/1, q2 rank 4 → 1/4, q3 miss → 0. mean = (1 + 0.25 + 0) / 3 ≈ 0.4167
        let queries = vec![
            gq("q1", &["c1"], &["d1"]),
            gq("q2", &["c2"], &["d2"]),
            gq("q3", &["c3"], &["d3"]),
        ];
        let rows = vec![
            record("q1", vec![hit(1, "c1", "d1")], None, None),
            record("q2", vec![hit(1, "x", "y"), hit(2, "x", "y"), hit(3, "x", "y"), hit(4, "c2", "d2")], None, None),
            record("q3", vec![hit(1, "x", "y")], None, None),
        ];
        let agg = aggregate_from_rows(&queries, &rows).unwrap();
        assert_eq!(agg.mrr, 0.4167);
    }

    #[test]
    fn recall_at_k_doc_partial() {
        // q1 expects {d1, d2}; top-3 returns {d1}. recall@3 = 0.5
        let queries = vec![gq("q1", &[], &["d1", "d2"])];
        let rows = vec![record("q1", vec![hit(1, "c1", "d1"), hit(2, "c2", "d3")], None, None)];
        let agg = aggregate_from_rows(&queries, &rows).unwrap();
        assert_eq!(agg.recall_at_k_doc[&3], 0.5);
        assert_eq!(agg.recall_at_k_doc[&10], 0.5);
    }

    #[test]
    fn citation_coverage_full_when_paths_resolve() {
        let mut q = gq("q1", &[], &["d1"]);
        q.must_contain = vec!["alpha".into()];
        let queries = vec![q];
        let ans = answer("contains alpha", true, &["docs/d1.md"]);
        let rows = vec![record("q1", vec![hit(1, "c1", "d1")], None, Some(ans))];
        let agg = aggregate_from_rows(&queries, &rows).unwrap();
        assert_eq!(agg.citation_coverage, 1.0);
    }

    #[test]
    fn groundedness_false_when_forbidden_present() {
        let mut q = gq("q1", &[], &["d1"]);
        q.must_contain = vec!["alpha".into()];
        q.forbidden = vec!["beta".into()];
        let queries = vec![q];
        let ans = answer("alpha and beta", true, &["docs/d1.md"]);
        let rows = vec![record("q1", vec![hit(1, "c1", "d1")], None, Some(ans))];
        let agg = aggregate_from_rows(&queries, &rows).unwrap();
        assert_eq!(agg.groundedness, 0.0);
    }

    #[test]
    fn refusal_correctness_one_when_should_refuse_and_did() {
        let queries = vec![gq("q1", &[], &[])]; // expected_doc_ids empty → "should refuse"
        let ans = answer("I cannot answer", false, &[]);
        let rows = vec![record("q1", vec![], None, Some(ans))];
        let agg = aggregate_from_rows(&queries, &rows).unwrap();
        assert_eq!(agg.refusal_correctness, 1.0);
    }

    #[test]
    fn refusal_correctness_nan_for_non_rag_run() {
        // Even with a "should refuse" query, a lexical-only run has no
        // Answer and so refusal cannot be judged → metric is NaN, not 0.
        let queries = vec![gq("q1", &[], &[])];
        let rows = vec![record("q1", vec![], None, None)];
        let agg = aggregate_from_rows(&queries, &rows).unwrap();
        assert!(agg.refusal_correctness.is_nan(), "got {}", agg.refusal_correctness);
    }

    #[test]
    fn citation_coverage_zero_when_answer_has_no_citations() {
        // A grounded answer with empty citations[] used to count as
        // covered via Iterator::all's vacuous-true; now must score 0.
        let mut q = gq("q1", &[], &["d1"]);
        q.must_contain = vec!["alpha".into()];
        let queries = vec![q];
        let ans = answer("contains alpha", true, &[]);
        let rows = vec![record("q1", vec![hit(1, "c1", "d1")], None, Some(ans))];
        let agg = aggregate_from_rows(&queries, &rows).unwrap();
        assert_eq!(agg.citation_coverage, 0.0);
    }

    #[test]
    fn groundedness_skips_unconfigured_goldens() {
        // A non-error RAG answer for a golden with neither must_contain
        // nor forbidden must NOT score 1.0 by default — it should be
        // excluded from the denominator entirely. Refusal-class
        // queries are tracked via refusal_correctness instead.
        let queries = vec![gq("q1", &["c1"], &["d1"])]; // no must_contain / forbidden
        let ans = answer("anything", true, &["docs/d1.md"]);
        let rows = vec![record("q1", vec![hit(1, "c1", "d1")], None, Some(ans))];
        let agg = aggregate_from_rows(&queries, &rows).unwrap();
        // denominator is 0 → ratio_or_zero returns 0.0 (not NaN, since
        // groundedness isn't a NaN-flagged metric per spec).
        assert_eq!(agg.groundedness, 0.0);
    }

    #[test]
    fn nan_metrics_serialize_as_null() {
        // No RAG answers → citation_coverage NaN. No "should refuse" → refusal_correctness NaN.
        let queries = vec![gq("q1", &["c1"], &["d1"])];
        let rows = vec![record("q1", vec![hit(1, "c1", "d1")], None, None)];
        let agg = aggregate_from_rows(&queries, &rows).unwrap();
        let json: serde_json::Value = serde_json::to_value(&agg).unwrap();
        assert!(json["citation_coverage"].is_null(), "expected null, got {:?}", json["citation_coverage"]);
        assert!(json["refusal_correctness"].is_null(), "expected null, got {:?}", json["refusal_correctness"]);
    }

    #[test]
    fn determinism_two_runs_match() {
        let queries = vec![gq("q1", &["c1"], &["d1"]), gq("q2", &["c2"], &["d2"])];
        let rows = vec![
            record("q1", vec![hit(1, "c1", "d1")], None, None),
            record("q2", vec![hit(1, "x", "y"), hit(2, "c2", "d2")], None, None),
        ];
        let a = aggregate_from_rows(&queries, &rows).unwrap();
        let b = aggregate_from_rows(&queries, &rows).unwrap();
        // NaN != NaN under PartialEq, but the JSON encoding maps NaN
        // to null and is the actual storage form, so compare on that.
        assert_eq!(
            serde_json::to_string(&a).unwrap(),
            serde_json::to_string(&b).unwrap()
        );
    }

    #[test]
    fn empty_result_rate_counts_zero_hits() {
        let queries = vec![gq("q1", &["c1"], &["d1"]), gq("q2", &["c2"], &["d2"])];
        let rows = vec![
            record("q1", vec![], None, None),
            record("q2", vec![hit(1, "c2", "d2")], None, None),
        ];
        let agg = aggregate_from_rows(&queries, &rows).unwrap();
        assert_eq!(agg.empty_result_rate, 0.5);
    }

    #[test]
    fn failed_queries_counted() {
        let queries = vec![gq("q1", &["c1"], &["d1"])];
        let rows = vec![record("q1", vec![], Some("boom".into()), None)];
        let agg = aggregate_from_rows(&queries, &rows).unwrap();
        assert_eq!(agg.failed_queries, 1);
        assert_eq!(agg.total_queries, 1);
    }
}
