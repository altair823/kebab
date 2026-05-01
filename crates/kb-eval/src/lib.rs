//! `kb-eval` — golden-fixture eval runner (P5-1).
//!
//! Loads `fixtures/golden_queries.yaml`, runs each entry through the
//! [`kb_app`] facade (lexical / vector / hybrid + optional RAG), and
//! persists results into `eval_runs` / `eval_query_results` plus
//! `runs_dir/<run_id>/per_query.jsonl` (design §5.7, §6.3).
//!
//! Metric computation lives in P5-2 (`kb-eval::metrics`); this crate is
//! the **data collector** only.
//!
//! ## Allowed deps (per task spec)
//!
//! `kb-core`, `kb-config`, `kb-app`, `kb-store-sqlite`, plus `serde`,
//! `serde_yaml`, `serde_json`, `time`, `tracing`,
//! `anyhow`, `uuid`. Retrieval / embedding / LLM crates are NOT
//! reachable here — every retrieval and `ask` call must go through
//! `kb-app`.
//!
//! ## `run_id` recipe
//!
//! `run_id` uses UUIDv7 simple — timestamp-ordered, lowercase hex.

mod loader;
mod runner;
mod types;

pub use loader::load_golden_set;
pub use runner::{run_eval, run_eval_with_config};
pub use types::{EvalRun, EvalRunOpts, GoldenQuery, QueryResult};
