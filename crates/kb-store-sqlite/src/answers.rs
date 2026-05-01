//! `answers` row writer (P4-3 — design §5.7).
//!
//! `kb-rag` always persists an `answers` row at the end of every
//! `RagPipeline::ask` — including refusal paths (`NoChunks`,
//! `ScoreGate`, `LlmSelfJudge`). The trait `kb_core::DocumentStore`
//! does not surface this method (answers aren't documents); we add it
//! as an inherent method on `SqliteStore` so kb-rag can call
//! `self.docs.put_answer(...)` directly.

use anyhow::{Context, Result};
use kb_core::{Answer, RefusalReason, SearchMode};
use rusqlite::params;

use crate::error::StoreError;
use crate::store::SqliteStore;

impl SqliteStore {
    /// Insert one row into `answers` (per V001 schema). The `query` is
    /// the original user query and is NOT recoverable from `Answer` —
    /// it lives only on the wire payload, not on the in-memory struct.
    /// `packed_chunks_json` is `Some` only when the caller asked for
    /// `--explain` (kb-rag's `AskOpts.explain == true`); otherwise the
    /// column stores SQL `NULL` per design §5.7.
    ///
    /// Idempotency: inserts only. The PRIMARY KEY is `trace_id`, which
    /// kb-rag mints with a nanosecond suffix so collisions are
    /// effectively impossible. If a duplicate trace_id ever does land
    /// (e.g., a test harness reuses one), the underlying SQLite
    /// `UNIQUE` violation surfaces verbatim through `StoreError`.
    pub fn put_answer(
        &self,
        answer: &Answer,
        query: &str,
        packed_chunks_json: Option<&str>,
    ) -> Result<()> {
        let created_at = answer
            .created_at
            .format(&time::format_description::well_known::Rfc3339)
            .context("format answer.created_at")?;
        let citations_json = serde_json::to_string(&answer.citations)
            .context("serialize answer.citations")?;
        let refusal_label: Option<&'static str> =
            answer.refusal_reason.as_ref().map(refusal_reason_label);
        let mode_label = search_mode_label(&answer.retrieval.mode);
        let embedding_id: Option<&str> = answer.embedding.as_ref().map(|m| m.id.as_str());
        let embedding_dim: Option<i64> =
            answer.embedding.as_ref().and_then(|m| m.dimensions.map(|d| d as i64));

        let conn = self.lock_conn();
        conn.execute(
            "INSERT INTO answers (
                trace_id, query, answer, grounded, refusal_reason,
                model_id, model_provider,
                embedding_model_id, embedding_dimensions,
                prompt_template_version,
                retrieval_mode, retrieval_k, score_gate, top_score,
                chunks_returned, chunks_used,
                citations_json, packed_chunks_json,
                prompt_tokens, completion_tokens, latency_ms,
                created_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                answer.retrieval.trace_id.0,
                query,
                answer.answer,
                if answer.grounded { 1_i64 } else { 0_i64 },
                refusal_label,
                answer.model.id,
                answer.model.provider,
                embedding_id,
                embedding_dim,
                answer.prompt_template_version.0,
                mode_label,
                answer.retrieval.k as i64,
                answer.retrieval.score_gate as f64,
                answer.retrieval.top_score as f64,
                answer.retrieval.chunks_returned as i64,
                answer.retrieval.chunks_used as i64,
                citations_json,
                packed_chunks_json,
                answer.usage.prompt_tokens as i64,
                answer.usage.completion_tokens as i64,
                answer.usage.latency_ms as i64,
                created_at,
            ],
        )
        .map_err(StoreError::from)?;
        Ok(())
    }
}

/// Stable lower-case label used in the `answers.refusal_reason` column
/// (design §5.7). Mirrors the `serde(rename_all = "snake_case")`
/// representation on `RefusalReason` so wire and DB labels coincide.
fn refusal_reason_label(r: &RefusalReason) -> &'static str {
    match r {
        RefusalReason::ScoreGate => "score_gate",
        RefusalReason::LlmSelfJudge => "llm_self_judge",
        RefusalReason::NoIndex => "no_index",
        RefusalReason::NoChunks => "no_chunks",
    }
}

/// Stable label used in the `answers.retrieval_mode` column. Mirrors
/// the `serde(rename_all = "lowercase")` representation on
/// `SearchMode` so wire and DB labels coincide.
fn search_mode_label(m: &SearchMode) -> &'static str {
    match m {
        SearchMode::Lexical => "lexical",
        SearchMode::Vector => "vector",
        SearchMode::Hybrid => "hybrid",
    }
}
