//! Public domain types for the eval runner (signatures pinned by
//! `tasks/p5/p5-1-golden-fixture-runner.md` "Public surface").

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use kebab_core::{Answer, ChunkId, DocumentId, Lang, SearchHit, SearchMode};

/// One golden query loaded from `fixtures/golden_queries.yaml`.
///
/// Required fields: `id`, `query`. Everything else defaults to
/// empty / `None` per the loader contract.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GoldenQuery {
    pub id: String,
    pub query: String,
    #[serde(default = "default_lang")]
    pub lang: Lang,
    #[serde(default)]
    pub expected_doc_ids: Vec<DocumentId>,
    #[serde(default)]
    pub expected_chunk_ids: Vec<ChunkId>,
    #[serde(default)]
    pub must_contain: Vec<String>,
    #[serde(default)]
    pub forbidden: Vec<String>,
    #[serde(default)]
    pub difficulty: Option<String>,
}

fn default_lang() -> Lang {
    // `Lang` is a BCP-47 string newtype (§3.3); the empty string is
    // the safe default for golden entries that omit `lang`. Curators
    // may fill it in later; the runner does not branch on this field.
    Lang(String::new())
}

/// Caller-supplied knobs for one [`crate::run_eval`] invocation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EvalRunOpts {
    /// Suite label persisted into `eval_runs.suite`. The shipped
    /// fixture is `"golden"`; other suites can reuse the same runner.
    pub suite: String,
    /// Retrieval mode forwarded to every `kebab_app::search` /
    /// `kebab_app::ask` call inside the run.
    pub mode: SearchMode,
    /// When `true`, also call `kebab_app::ask` per query and record the
    /// resulting `Answer` on the `QueryResult`.
    pub with_rag: bool,
    /// Top-k forwarded to retrieval (and `AskOpts.k` when `with_rag`).
    pub k: usize,
    /// Override `config.models.llm.temperature` when `with_rag`.
    /// Determinism contract requires `Some(0.0)` + a fixed `seed`.
    pub temperature: Option<f32>,
    /// Override `config.models.llm.seed` when `with_rag`.
    pub seed: Option<u64>,
}

/// One full eval run. Persisted to `eval_runs` + `eval_query_results`
/// (design §5.7) and mirrored to `runs_dir/<run_id>/per_query.jsonl`
/// (design §6.3).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EvalRun {
    pub run_id: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    pub commit_hash: Option<String>,
    /// Snapshot of the `Config` plus auxiliary version fields
    /// (`chunker_version`, embedding/llm/prompt versions, fusion
    /// params, `index_version`). See [`crate::run_eval`] for the
    /// exact shape.
    pub config_snapshot_json: serde_json::Value,
    pub per_query: Vec<QueryResult>,
}

/// One per-query record. Every row in `eval_query_results` has its
/// `result_json` filled with `serde_json::to_string(&QueryResult)`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QueryResult {
    pub query_id: String,
    pub query: String,
    pub mode: SearchMode,
    pub hits_top_k: Vec<SearchHit>,
    pub answer: Option<Answer>,
    pub elapsed_ms: u32,
    pub error: Option<String>,
}
