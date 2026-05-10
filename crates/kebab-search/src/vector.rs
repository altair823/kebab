//! Vector retriever — design §3.7 / §7.2 / §1.6.
//!
//! Wraps a `dyn VectorStore` + `dyn Embedder` + the SQLite metadata
//! store into a `kebab_core::Retriever`. The vector store knows how to
//! find the nearest chunks by cosine on the embedding column; SQLite
//! owns the human-readable metadata (heading_path / section_label /
//! source_spans / chunker_version / workspace_path) needed for
//! `SearchHit` and `Citation`. The retriever stitches them together
//! per spec §7.2.
//!
//! Snippet policy: this retriever has no FTS5 highlighter to lean on,
//! so the `snippet` field is the chunk text trimmed to
//! `config.search.snippet_chars` Unicode scalar values. The lexical
//! retriever does query-token highlighting; downstream UI code should
//! continue to surface lexical snippets for hybrid hits where the
//! lexical side contributed (handled in `HybridRetriever::search`).

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result};
use kebab_core::{
    ChunkId, ChunkerVersion, DocumentId, Embedder, EmbeddingInput, EmbeddingKind,
    IndexVersion, RetrievalDetail, Retriever, ScoreKind, SearchHit, SearchMode, SearchQuery,
    SourceSpan, VectorHit, VectorStore, WorkspacePath,
};
use kebab_store_sqlite::SqliteStore;
use rusqlite::params_from_iter;

use crate::citation_helper::citation_from_first_span;

/// Default `k` when `SearchQuery::k == 0`. Mirrors §6.4 default_k=10
/// and the lexical retriever's `DEFAULT_K`.
const DEFAULT_K: usize = 10;

/// Over-fetch multiplier passed to `VectorStore::search` so that
/// SQLite-side filter losses (tags / lang / trust / path_glob) still
/// leave at least `k` candidates. The Lance store already applies the
/// same filters internally; the extra `* 2` is the spec-mandated
/// safety margin for the `Retriever` layer (§7.2 spec line 138).
const VECTOR_OVERFETCH_MULTIPLIER: usize = 2;

/// Wraps a vector store + embedder into a [`Retriever`].
///
/// `VectorStore` is not declared `Send + Sync` in `kb-core::traits`,
/// but `Retriever` requires both. We constrain the trait objects
/// here so callers must hand us implementations that already are
/// (`LanceVectorStore` is `Send + Sync` thanks to its
/// `Connection`/`Runtime` ownership; the trait is sync-method-only).
pub struct VectorRetriever {
    store: Arc<dyn VectorStore + Send + Sync>,
    embed: Arc<dyn Embedder>,
    sqlite: Arc<SqliteStore>,
    index_version: IndexVersion,
    snippet_chars: usize,
}

impl VectorRetriever {
    /// Construct with `index_version` derived from the configured
    /// embedding model + dimensions, and snippet width pulled from
    /// `kb-config`'s defaults.
    ///
    /// The explicit `index_version` form is [`Self::with_settings`].
    pub fn new(
        store: Arc<dyn VectorStore + Send + Sync>,
        embed: Arc<dyn Embedder>,
        sqlite: Arc<SqliteStore>,
        index_version: IndexVersion,
    ) -> Self {
        let cfg = kebab_config::Config::defaults();
        Self::with_settings(store, embed, sqlite, index_version, cfg.search.snippet_chars)
    }

    /// Construct with explicit `snippet_chars`. Mirrors the lexical
    /// retriever's `with_settings` constructor for callers that have
    /// already loaded a `Config`.
    pub fn with_settings(
        store: Arc<dyn VectorStore + Send + Sync>,
        embed: Arc<dyn Embedder>,
        sqlite: Arc<SqliteStore>,
        index_version: IndexVersion,
        snippet_chars: usize,
    ) -> Self {
        Self {
            store,
            embed,
            sqlite,
            index_version,
            snippet_chars,
        }
    }
}

impl Retriever for VectorRetriever {
    fn search(&self, query: &SearchQuery) -> Result<Vec<SearchHit>> {
        let k = if query.k == 0 { DEFAULT_K } else { query.k };
        tracing::debug!(
            text_len = query.text.len(),
            k,
            "kb-search vector: search start"
        );

        // Empty / whitespace-only queries — short-circuit. The
        // embedder would still produce a vector for an empty string,
        // but nearest-neighbours on the centroid of "" is meaningless
        // and only forces a wasted Lance scan.
        if query.text.trim().is_empty() {
            return Ok(Vec::new());
        }

        // 1. Embed the query as `Query` kind (e5-style asymmetry —
        //    documents and queries have different prefixes).
        let inputs = [EmbeddingInput {
            text: &query.text,
            kind: EmbeddingKind::Query,
        }];
        let mut embeddings = self
            .embed
            .embed(&inputs)
            .context("kb-search vector: embed query")?;
        if embeddings.len() != 1 {
            anyhow::bail!(
                "kb-search vector: embedder returned {} vectors for one input",
                embeddings.len()
            );
        }
        let query_vec = embeddings.remove(0);

        // 2. Over-fetch from the vector store. The Lance store
        //    applies `filter_chunks` internally, so we pass `query.filters`
        //    through and trust the post-filter pass to honour them.
        //    `saturating_mul(2)` is always ≥ k for any usize k, so we
        //    don't need an extra `.max(k)` clamp.
        let overfetch = k.saturating_mul(VECTOR_OVERFETCH_MULTIPLIER);
        let raw_hits = self
            .store
            .search(&query_vec, overfetch, &query.filters)
            .context("kb-search vector: VectorStore::search")?;

        if raw_hits.is_empty() {
            tracing::debug!("kb-search vector: store returned no hits");
            return Ok(Vec::new());
        }

        // 3. Hydrate metadata from SQLite for the candidate ids in
        //    one round-trip. Order is preserved by the caller via the
        //    HashMap lookup at hit-construction time.
        let candidate_ids: Vec<&str> =
            raw_hits.iter().map(|h| h.chunk_id.0.as_str()).collect();
        let hydration = hydrate_chunks(&self.sqlite, &candidate_ids)
            .context("kb-search vector: hydrate chunk metadata")?;

        // 4. Build `SearchHit` for the first `k` raw hits that pass
        //    hydration (a missing row would be a filter-induced drop —
        //    Lance returned the chunk but SQLite filtered it out, or
        //    the chunk was deleted between Lance's read and ours).
        let model_id = self.embed.model_id();
        let mut hits: Vec<SearchHit> = Vec::with_capacity(k.min(raw_hits.len()));
        let mut rank: u32 = 0;
        for hit in raw_hits {
            let Some(meta) = hydration.get(hit.chunk_id.0.as_str()) else {
                continue;
            };
            rank = rank.saturating_add(1);
            hits.push(build_hit(
                hit,
                meta,
                rank,
                &self.index_version,
                &model_id,
                self.snippet_chars,
            )?);
            if hits.len() >= k {
                break;
            }
        }

        tracing::debug!(rows = hits.len(), "kb-search vector: search done");
        Ok(hits)
    }

    fn index_version(&self) -> IndexVersion {
        self.index_version.clone()
    }
}

// ── Hydration ────────────────────────────────────────────────────────────

/// Subset of `chunks` + `documents` metadata needed to build a
/// `SearchHit` from a `VectorHit`. Pulled in one round-trip so the
/// per-hit construction loop stays O(1) per row.
struct ChunkMeta {
    text: String,
    heading_path_json: String,
    section_label: Option<String>,
    source_spans_json: String,
    chunker_version: String,
    doc_id: String,
    workspace_path: String,
    /// p9-fb-32: documents.updated_at (RFC3339).
    updated_at: String,
}

fn hydrate_chunks(
    sqlite: &SqliteStore,
    chunk_ids: &[&str],
) -> Result<HashMap<String, ChunkMeta>> {
    if chunk_ids.is_empty() {
        return Ok(HashMap::new());
    }
    // Deduplicate the IN-list — Lance can repeat a chunk_id across
    // batches in pathological cases. A HashMap key dedupes in the
    // result anyway, but keeping the placeholder count tight is good
    // hygiene.
    let mut seen = std::collections::HashSet::new();
    let unique: Vec<&str> = chunk_ids
        .iter()
        .copied()
        .filter(|id| seen.insert(*id))
        .collect();

    let placeholders = vec!["?"; unique.len()].join(",");
    let sql = format!(
        "SELECT \
            c.chunk_id, c.text, c.heading_path_json, c.section_label, \
            c.source_spans_json, c.chunker_version, \
            c.doc_id, d.workspace_path, d.updated_at \
         FROM chunks c \
         JOIN documents d ON d.doc_id = c.doc_id \
         WHERE c.chunk_id IN ({placeholders})"
    );
    let conn = sqlite.read_conn();
    let mut stmt = conn
        .prepare(&sql)
        .context("kb-search vector: prepare hydration statement")?;
    let rows = stmt
        .query_map(
            // `unique` is a `Vec<&str>`; `&str` implements `ToSql`
            // directly, so we hand the iterator straight to
            // `params_from_iter` without copying.
            params_from_iter(unique.iter().copied()),
            |row| {
                let chunk_id: String = row.get(0)?;
                Ok((
                    chunk_id,
                    ChunkMeta {
                        text: row.get(1)?,
                        heading_path_json: row.get(2)?,
                        section_label: row.get(3)?,
                        source_spans_json: row.get(4)?,
                        chunker_version: row.get(5)?,
                        doc_id: row.get(6)?,
                        workspace_path: row.get(7)?,
                        updated_at: row.get(8)?,
                    },
                ))
            },
        )
        .context("kb-search vector: execute hydration query")?;
    let mut out: HashMap<String, ChunkMeta> = HashMap::with_capacity(unique.len());
    for row in rows {
        let (chunk_id, meta) =
            row.context("kb-search vector: read hydration row")?;
        out.insert(chunk_id, meta);
    }
    Ok(out)
}

fn build_hit(
    hit: VectorHit,
    meta: &ChunkMeta,
    rank: u32,
    index_version: &IndexVersion,
    model_id: &kebab_core::EmbeddingModelId,
    snippet_chars: usize,
) -> Result<SearchHit> {
    let heading_path: Vec<String> = serde_json::from_str(&meta.heading_path_json)
        .context("kb-search vector: deserialize heading_path_json")?;
    let source_spans: Vec<SourceSpan> = serde_json::from_str(&meta.source_spans_json)
        .context("kb-search vector: deserialize source_spans_json")?;

    let workspace_path = WorkspacePath::new(meta.workspace_path.clone()).context(
        "kb-search vector: documents.workspace_path violates WorkspacePath invariant",
    )?;
    let citation = citation_from_first_span(
        &hit.chunk_id.0,
        workspace_path.clone(),
        meta.section_label.clone(),
        source_spans.first(),
    );
    let snippet = trim_snippet(&meta.text, snippet_chars);

    // p9-fb-32: documents.updated_at is stored as RFC3339 TEXT (V001
    // migration; written by put_document via OffsetDateTime::now_utc).
    // Mirrors the lexical retriever; see lexical::build_hit for the
    // shared rationale on incremental-ingest skip semantics.
    let indexed_at = time::OffsetDateTime::parse(
        &meta.updated_at,
        &time::format_description::well_known::Rfc3339,
    )
    .context("kb-search vector: parse documents.updated_at as RFC3339")?;

    let score = hit.score;
    Ok(SearchHit {
        rank,
        chunk_id: ChunkId(hit.chunk_id.0),
        doc_id: DocumentId(meta.doc_id.clone()),
        doc_path: workspace_path,
        heading_path,
        section_label: meta.section_label.clone(),
        snippet,
        citation,
        retrieval: RetrievalDetail {
            method: SearchMode::Vector,
            fusion_score: score,
            lexical_score: None,
            vector_score: Some(score),
            lexical_rank: None,
            vector_rank: Some(rank),
        },
        index_version: index_version.clone(),
        embedding_model: Some(model_id.clone()),
        chunker_version: ChunkerVersion(meta.chunker_version.clone()),
        indexed_at,
        // Placeholder — overwritten by `kebab_app::staleness::mark_stale_in_place`
        // (called from `App::search` / `App::search_uncached`) and the equivalent
        // in `RagPipeline::ask` against the configured threshold.
        stale: false,
        score_kind: ScoreKind::Cosine,
    })
}

/// Cap the snippet at `max_chars` Unicode scalar values. Mirrors
/// `lexical::trim_snippet` so the two retrievers produce identically
/// shaped snippets for hybrid output.
fn trim_snippet(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    s.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trim_snippet_caps_at_char_count() {
        let s = "a".repeat(300);
        assert_eq!(trim_snippet(&s, 220).chars().count(), 220);
    }

    #[test]
    fn trim_snippet_passthrough_when_short() {
        assert_eq!(trim_snippet("short", 220), "short");
    }
}
