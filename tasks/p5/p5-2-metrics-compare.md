---
phase: P5
component: kb-eval (metrics + compare)
task_id: p5-2
title: "Metrics computation + compare report"
status: completed
depends_on: [p5-1]
unblocks: []
contract_source: ../../docs/superpowers/specs/2026-04-27-kb-final-form-design.md
contract_sections: [§5.7 eval_runs.aggregate_json, phase epic tasks/phase-5-evaluation.md]
---

# p5-2 — Metrics + compare

## Goal

Compute hit@k, MRR, recall@k_doc, citation_coverage, groundedness, empty_result_rate, refusal_correctness from stored `eval_query_results`. Write `aggregate_json` back into `eval_runs`. Provide `kb eval compare a b` that diffs two runs.

## Why now / why this size

Metric formulas + comparison logic are pure computation. Splitting them from p5-1 keeps the runner simple and lets us re-compute metrics over historical runs as formulas evolve.

## Allowed dependencies

- `kb-core`
- `kb-config`
- `kb-store-sqlite` (read eval rows, write `aggregate_json`)
- `serde`, `serde_json`
- `tracing`
- `thiserror`

## Forbidden dependencies

- `kb-app`, `kb-source-fs`, `kb-parse-md`, `kb-normalize`, `kb-chunk`, `kb-store-vector`, `kb-embed*`, `kb-search`, `kb-llm*`, `kb-rag`, `kb-tui`, `kb-desktop`

## Inputs

| input | type | source |
|-------|------|--------|
| `eval_query_results` rows | SQLite | from p5-1 |
| `eval_runs` row | SQLite | from p5-1 |
| `GoldenQuery[..]` | `Vec<GoldenQuery>` | re-loaded for `expected_*` and `must_contain` |

## Outputs

| output | type | downstream |
|--------|------|------------|
| `eval_runs.aggregate_json` updated | SQLite | history, CI checks |
| `CompareReport` | `kb_eval::CompareReport` | `kb-cli` printer |
| optional `runs_dir/<run_id>/report.md` | filesystem | human-readable summary |

## Public surface (signatures only — no new types)

```rust
pub struct AggregateMetrics {
    pub hit_at_k:           std::collections::BTreeMap<u32, f32>,   // k → hit@k
    pub mrr:                f32,
    pub recall_at_k_doc:    std::collections::BTreeMap<u32, f32>,
    pub citation_coverage:  f32,
    pub groundedness:       f32,
    pub empty_result_rate:  f32,
    pub refusal_correctness: f32,
    pub total_queries:      u32,
    pub failed_queries:     u32,
}

pub struct CompareReport {
    pub run_a: String,
    pub run_b: String,
    pub aggregate_a: AggregateMetrics,
    pub aggregate_b: AggregateMetrics,
    pub deltas: serde_json::Value,             // per-metric delta
    pub per_query: Vec<QueryComparison>,
}

pub struct QueryComparison {
    pub query_id: String,
    pub kind: ComparisonKind,                  // Win | Loss | Draw | Regression
    pub a_hit_rank: Option<u32>,
    pub b_hit_rank: Option<u32>,
    pub note: Option<String>,
}

pub enum ComparisonKind { Win, Loss, Draw, Regression }

pub fn compute_aggregate(run_id: &str) -> anyhow::Result<AggregateMetrics>;
pub fn store_aggregate(run_id: &str, agg: &AggregateMetrics) -> anyhow::Result<()>;
pub fn compare_runs(run_id_a: &str, run_id_b: &str) -> anyhow::Result<CompareReport>;
pub fn render_report_md(report: &CompareReport) -> String;
```

## Behavior contract

- `hit@k` for k ∈ {1, 3, 5, 10}: query is a hit if any of its `expected_chunk_ids` appears in the run's top-k for that query (chunk-level). Aggregate = mean across queries with non-empty `expected_chunk_ids`.
- `MRR`: 1 / rank-of-first-correct-chunk; 0 if not found in top-10. Aggregate = mean across applicable queries.
- `recall@k_doc` for k ∈ {1, 3, 5, 10}: fraction of `expected_doc_ids` covered by the top-k hits' `doc_id`s, averaged across applicable queries.
- `citation_coverage`: fraction of RAG answers where every `Answer.citations[*].citation` resolves to a real chunk in the DB. Denominator = grounded RAG answers; if zero → metric is `NaN` and reported as `null` in JSON.
- `groundedness`: fraction of RAG answers where ALL `must_contain` strings appear AND no `forbidden` string appears. Denominator = RAG answers (excluding errors).
- `empty_result_rate`: fraction of queries returning zero `hits_top_k`.
- `refusal_correctness`: fraction of queries with `expected_doc_ids = []` (i.e., should refuse) that the system actually refused (Answer.grounded == false). Denominator = queries marked as "should refuse"; if zero → null.
- All metrics rounded to 4 decimal places for storage.
- `compare_runs`:
  - Per-metric delta (`b - a`).
  - Per-query: `Win` if b found correct chunk, a did not. `Loss` opposite. `Draw` if both same rank. `Regression` if a hit but b miss for the same expected chunk.
  - `note` may explain known causes (chunker version diff, embedding diff, prompt diff).
  - **Cross-version chunk_id matching is graceful, not a refusal.** When `chunker_version_a != chunker_version_b` the chunk-level criterion would be unstable (chunk_ids are part of the key), so per-query matching falls back to *doc_id + span overlap*: a hit counts if the run's top-k contains any chunk whose `doc_id` matches an expected `doc_id` AND whose `source_spans` overlap by at least 50% with one of the expected chunks' spans. The `CompareReport.deltas` JSON includes a top-level `"chunker_version_match": "exact" | "fallback_doc_span"` so consumers see which mode was used. Set `--strict-chunker-version` to revert to the old behavior (refuse). Default is graceful so chunker iteration is the natural workflow it should be.
- `render_report_md` produces a single Markdown file summarizing aggregate deltas + a Wins/Losses/Regressions table; not a wire schema; for human consumption only.
- `store_aggregate` updates `eval_runs.aggregate_json` (`UPDATE eval_runs SET aggregate_json = :json WHERE run_id = :id`).

## Storage / wire effects

- Writes: `eval_runs.aggregate_json`, optional `runs_dir/<run_id>/report.md`.
- Reads: `eval_runs`, `eval_query_results`.

## Test plan

| kind | description | fixture / data |
|------|-------------|----------------|
| unit | hit@k computation on hand-rolled fixture | inline (3 queries, ranks {1, 4, miss}) |
| unit | MRR computation matches expected | inline |
| unit | recall@k_doc computation | inline |
| unit | citation_coverage with broken citation marks 0.0 | inline |
| unit | groundedness false when forbidden string appears | inline |
| unit | refusal_correctness 1.0 when all "should refuse" queries refused | inline |
| unit | NaN metrics (zero denominator) serialize as `null` in JSON | inline |
| unit | `compare_runs` per-query Win/Loss/Draw/Regression on synthetic ranks | inline |
| determinism | running `compute_aggregate` twice produces identical `AggregateMetrics` | inline |
| snapshot | `CompareReport` JSON for a fixed pair of runs stable | `fixtures/eval/compare-1.json` |

All tests under `cargo test -p kb-eval metrics`.

## Definition of Done

- [ ] `cargo check -p kb-eval` passes
- [ ] `cargo test -p kb-eval metrics` passes
- [ ] No imports outside Allowed dependencies
- [ ] `eval_runs.aggregate_json` always populated after `store_aggregate`
- [ ] `kb eval compare` CLI surface integrated via `kb-app` (call `compare_runs` + `render_report_md`)
- [ ] PR links phase epic tasks/phase-5-evaluation.md

## Out of scope

- LLM-as-judge groundedness.
- Cross-corpus evaluation.
- HTTP server / dashboards.
- Metric weighting strategies (MRR weighting, etc.).

## Risks / notes

- Floating-point sums in MRR cause minor cross-platform drift; round to 4 decimals on storage to keep snapshots stable.
- "Should refuse" queries are encoded as `expected_doc_ids: []`. Document this convention in the golden YAML header comment.
- Chunker version drift across runs is the COMMON case, not the error case (you almost always re-chunk before evaluating a chunker change). Default behavior is graceful fallback (doc + span overlap); only `--strict-chunker-version` refuses. The `chunker_version_match` field in `CompareReport.deltas` makes the mode auditable, so silent miscompares are still impossible.

## Implementation deviations (intentional)

Recorded so reviewers don't trip on them; the runtime behavior is the
same one this spec defines, the names / wiring just differ.

- **Graceful fallback is doc-id-only, not doc + 50% span overlap.** The
  `chunker_version_match` audit field is `"fallback_doc"` (not
  `"fallback_doc_span"`). Span-overlap requires reading both runs'
  `chunks.source_spans` simultaneously — but a chunker-version change
  in practice re-indexes (overwrites) the chunks table, so by the time
  you compute run B the run A chunk rows are already gone. Doc-id
  matching is the strongest stable criterion under that workflow.
  Span-overlap moves to a future phase that owns chunker-version
  archival.
- **Helper signatures.** `compute_aggregate_with_config(cfg, run_id)` /
  `store_aggregate_with_config(cfg, run_id, agg)` /
  `compare_runs_with_config(cfg, a, b, opts)` exist alongside the
  spec-pinned `compute_aggregate(run_id)` / `store_aggregate(run_id, agg)`
  / `compare_runs(a, b)` so integration tests can drive the pipeline
  against a TempDir-backed `Config`. The no-arg forms wrap them with
  `Config::load(None)`.
- **CLI surface lives on `kb-cli` directly, not via `kb-app`.** DoD
  asks for `kb eval compare` to be reached "via kb-app", but `kb-app`
  already depends on `kb-eval` (the P5-1 runner uses the App facade),
  so routing the CLI through `kb-app` would form a cycle. `kb-cli` →
  `kb-eval` is wired directly; `kb-app` is unchanged.
- **`AggregateMetrics` is `Serialize + Deserialize`.** The spec defines
  only the field shape; we add `Deserialize` so the stored
  `aggregate_json` can round-trip back into the type for follow-up
  computations.
- **`anyhow`** is used in `Result` returns since the rest of the
  workspace already speaks anyhow; not in the spec's Allowed list but
  matches every other crate.
