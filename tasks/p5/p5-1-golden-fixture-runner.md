---
phase: P5
component: kb-eval (runner)
task_id: p5-1
title: "Golden query fixture loader + per-query runner"
status: planned
depends_on: [p4-3]
unblocks: [p5-2]
contract_source: ../../docs/superpowers/specs/2026-04-27-kb-final-form-design.md
contract_sections: [§5.7 eval_runs/eval_query_results, §6.3 runs_dir, phase epic tasks/phase-5-evaluation.md]
---

# p5-1 — Golden fixture runner

## Goal

Load `fixtures/golden_queries.yaml`, run each query through `kb-app` (lexical / vector / hybrid / rag), and persist results into `eval_query_results` + `runs_dir/<run_id>/per_query.jsonl`.

## Why now / why this size

The runner is the data collector; metrics computation is p5-2's job. Splitting them makes each piece simple and lets us re-compute metrics from stored runs without re-querying.

## Allowed dependencies

- `kb-core`
- `kb-config`
- `kb-app` (calls facade for search / ask)
- `kb-store-sqlite` (writes eval rows)
- `serde`, `serde_yaml`, `serde_json`
- `time`
- `tracing`
- `thiserror`

## Forbidden dependencies

- `kb-source-fs`, `kb-parse-md`, `kb-normalize`, `kb-chunk`, `kb-store-vector`, `kb-embed*`, `kb-search`, `kb-llm*`, `kb-rag` (all reached via `kb-app` facade only), `kb-tui`, `kb-desktop`

## Inputs

| input | type | source |
|-------|------|--------|
| `fixtures/golden_queries.yaml` | YAML | repo-shipped |
| `EvalRunOpts` | suite, mode, with_rag, k, temperature, seed | CLI |
| `kb-app` facade | search/ask | runtime |

## Outputs

| output | type | downstream |
|--------|------|------------|
| `eval_runs` row | SQLite | p5-2, history |
| `eval_query_results` rows | SQLite | p5-2 |
| `runs_dir/<run_id>/per_query.jsonl` | filesystem | external tools, audits |
| `EvalRun` struct | `kb_eval::EvalRun` | caller |

## Public surface (signatures only — no new types)

```rust
pub struct GoldenQuery {
    pub id: String,
    pub query: String,
    pub lang: kb_core::Lang,
    pub expected_doc_ids: Vec<kb_core::DocumentId>,
    pub expected_chunk_ids: Vec<kb_core::ChunkId>,
    pub must_contain: Vec<String>,
    pub forbidden: Vec<String>,
    pub difficulty: Option<String>,
}

pub struct EvalRunOpts {
    pub suite: String,                    // "golden" default
    pub mode:  kb_core::SearchMode,
    pub with_rag: bool,
    pub k: usize,
    pub temperature: Option<f32>,
    pub seed: Option<u64>,
}

pub struct EvalRun {
    pub run_id: String,
    pub created_at: time::OffsetDateTime,
    pub commit_hash: Option<String>,
    pub config_snapshot_json: serde_json::Value,
    pub per_query: Vec<QueryResult>,
}

pub struct QueryResult {
    pub query_id: String,
    pub query: String,
    pub mode: kb_core::SearchMode,
    pub hits_top_k: Vec<kb_core::SearchHit>,
    pub answer: Option<kb_core::Answer>,
    pub elapsed_ms: u32,
    pub error: Option<String>,
}

pub fn load_golden_set(path: &std::path::Path) -> anyhow::Result<Vec<GoldenQuery>>;
pub fn run_eval(opts: &EvalRunOpts) -> anyhow::Result<EvalRun>;
```

## Behavior contract

- `load_golden_set`:
  - Parses YAML; required fields: `id`, `query`. Optional: everything else (defaults to empty / `None`).
  - Validates uniqueness of `id` and that `expected_doc_ids` / `expected_chunk_ids` exist in DB; missing → return error listing the offenders.
- `run_eval`:
  - Loads `fixtures/golden_queries.yaml` (path overridable via env `KB_EVAL_GOLDEN`).
  - Generates `run_id = "run_" + ulid_lower()`.
  - Captures `config_snapshot_json`: serialized `kb_config::Config` plus `chunker_version`, `embedding_model+version+dims`, `llm.model_id`, `prompt_template_version`, `score_gate`, `rrf_k`, `index_version`.
  - For each query: call `kb_app::search(SearchQuery { mode: opts.mode, k: opts.k, .. })`. If `opts.with_rag`, also call `kb_app::ask(query, AskOpts { mode: opts.mode, k: opts.k, explain: true, temperature: opts.temperature, seed: opts.seed, .. })`.
  - Each `QueryResult` measured by elapsed wall-clock (ms).
  - Errors are caught per-query (do not abort the run). Failed queries record `error: Some(msg)` and `hits_top_k = vec![]`.
  - Determinism: with `temperature=0` and fixed `seed`, two consecutive runs produce byte-identical `per_query.jsonl` for non-RAG queries; RAG queries may differ in negligible token budget telemetry.
  - Persists `eval_runs` row with `aggregate_json = {}` (filled by p5-2). Persists `eval_query_results` rows. Also writes `per_query.jsonl` to `runs_dir/<run_id>/`.
- `run_eval` does NOT compute hit@k or other metrics (that is p5-2).

## Storage / wire effects

- Writes: `eval_runs`, `eval_query_results`, `runs_dir/<run_id>/per_query.jsonl`.
- Reads: golden YAML, chunk/doc rows (via DB).

## Test plan

| kind | description | fixture / data |
|------|-------------|----------------|
| unit | YAML loader rejects duplicate IDs | inline YAML |
| unit | YAML loader rejects unknown `expected_chunk_id` | seeded DB |
| unit | runner records `elapsed_ms ≥ 0` for each query | tiny corpus + 3 queries |
| unit | runner captures config_snapshot with all expected version fields | inline |
| unit | failing query (forced via mock retriever) records `error: Some(_)` and continues | mock |
| determinism | re-running same suite + fixed seed → identical `per_query.jsonl` (lexical only) | tmp DB, fixed corpus |
| snapshot | `EvalRun` (with mock LM for `with_rag`) JSON stable | `fixtures/eval/run-1.json` |

All tests under `cargo test -p kb-eval runner`.

## Definition of Done

- [ ] `cargo check -p kb-eval` passes
- [ ] `cargo test -p kb-eval runner` passes
- [ ] `fixtures/golden_queries.yaml` template shipped (≥ 5 example entries)
- [ ] No imports outside Allowed dependencies
- [ ] PR links design §5.7

## Out of scope

- Metric computation (p5-2).
- LLM-as-judge.
- Compare report generation.
- HTTP/server integrations.

## Risks / notes

- Large RAG suites can be slow. Consider `--max-queries` for incremental runs (kept here as a flag spec; implementation is the responsibility of this task).
- `expected_chunk_id` references depend on `chunker_version`. If chunker bumps, golden set must be re-curated. Fail fast in the loader.
- Use `time::OffsetDateTime::now_utc()` for `created_at`; never local TZ.
