---
phase: P3
component: kebab-store-vector (LanceDB)
task_id: p3-3
title: "LanceDB VectorStore + embedding_records writer"
status: completed
depends_on: [p3-2, p1-6]
unblocks: [p3-4]
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§5.6 embedding_records, §6.3 lancedb table naming, §7.2 VectorStore, §9 versioning]
---

# p3-3 — LanceDB VectorStore

## Goal

Implement `VectorStore` over LanceDB (embedded). Stores per-model tables (`chunk_embeddings_<model>_<dim>.lance`), upserts vectors transactionally with a row in `embedding_records` (SQLite), and serves `search` for the vector retrieval mode.

## Why now / why this size

Closes the loop chunk → vector. Splits cleanly from `kebab-search` so hybrid (p3-4) can compose lexical + vector retrievers without leaking storage details.

## Allowed dependencies

- `kebab-core`
- `kebab-config`
- `kebab-store-sqlite` (only for writing/reading rows in `embedding_records`)
- `lancedb`
- `arrow` (and `arrow-array`, `arrow-schema`)
- `serde`, `serde_json`
- `tracing`
- `thiserror`

## Forbidden dependencies

- `kebab-source-fs`, `kebab-parse-md`, `kebab-normalize`, `kebab-chunk`, `kebab-embed*` (consumes `Vec<f32>` via input only — no embedding logic here), `kebab-search`, `kebab-llm*`, `kebab-rag`, `kebab-tui`, `kebab-desktop`

## Inputs

| input | type | source |
|-------|------|--------|
| `VectorRecord[..]` | `kebab_core::VectorRecord` | `kebab-app::embed_index` (P3 facade) |
| query vector | `&[f32]` | `kebab-embed-local` (`Embedder::embed` for query) |
| filters | `kebab_core::SearchFilters` | `SearchQuery` |
| `kebab-config::Config.storage.vector_dir` | path | runtime |

## Outputs

| output | type | downstream |
|--------|------|------------|
| Lance tables under `vector_dir/chunk_embeddings_<model>_<dim>.lance/` | filesystem | future searches |
| `embedding_records` rows | SQLite | reverse lookup, reindex bookkeeping |
| `Vec<VectorHit>` | `kebab_core::VectorHit` | hybrid retriever (p3-4) |

## Public surface (signatures only — no new types)

```rust
pub struct LanceVectorStore { /* internal: connection + sqlite handle */ }

impl LanceVectorStore {
    pub fn new(config: &kebab_config::Config, sqlite: std::sync::Arc<kebab_store_sqlite::SqliteStore>) -> anyhow::Result<Self>;
}

impl kebab_core::VectorStore for LanceVectorStore {
    fn ensure_table(&self, model: &kebab_core::EmbeddingModelId, dim: usize) -> anyhow::Result<kebab_core::IndexId>;
    fn upsert(&self, recs: &[kebab_core::VectorRecord]) -> anyhow::Result<()>;
    fn search(&self, query_vec: &[f32], k: usize, filters: &kebab_core::SearchFilters) -> anyhow::Result<Vec<kebab_core::VectorHit>>;
}
```

## Behavior contract

- Table naming: `chunk_embeddings_<model_id>_<dim>.lance`. Model IDs must be sanitized (replace non `[A-Za-z0-9-]` with `_`) to avoid filesystem issues.
- `ensure_table` is idempotent: opens existing or creates with explicit Arrow schema:
  ```
  chunk_id : Utf8 (primary)
  doc_id   : Utf8
  embedding: FixedSizeList<Float32, dim>
  model_id : Utf8
  embedding_version : Utf8
  text     : Utf8
  heading_path : Utf8
  created_at : Timestamp(Microsecond, UTC)
  ```
- For corpora < 100k rows, no IVF index — flat cosine. Above that threshold, the next migration task (P+) introduces IVF; this task does not.
- `upsert` ordering: **SQLite-first, Lance-second** with an explicit 3-state marker so reconciliation is unambiguous (no \"best-effort 2PC\" hand-wave).
  1. `INSERT OR REPLACE INTO embedding_records (..., status='pending', vector_committed=0)` for every input row (single SQLite tx).
  2. Issue Lance upsert (`MergeInsert` keyed on `chunk_id`).
  3. On Lance success: `UPDATE embedding_records SET status='committed', vector_committed=1 WHERE embedding_id IN (...)`.
  4. On Lance failure or process crash: rows stay at `status='pending'`. Next `upsert` re-tries them automatically (idempotent — Lance `MergeInsert` dedupes on `chunk_id`).
- `embedding_records.status` is the single source of truth: `search` joins `embedding_records` and filters `WHERE status='committed'`, so partial-write Lance rows are never returned even if they exist on disk. This guarantees `search` results' `embedding_id` always points at a committed Lance row.
- Adds two columns to `embedding_records` (additive — `V003__embedding_status.sql` migration, not a v1 wire schema change): `status TEXT NOT NULL CHECK (status IN ('pending','committed','tombstone'))` default `'pending'`, and `vector_committed INTEGER NOT NULL DEFAULT 0`.
- Tombstones: when a chunk is deleted (CASCADE from `chunks`), a `BEFORE DELETE` trigger flips `status='tombstone'` instead of letting the row be deleted, so a later GC can drop the matching Lance row in lockstep. GC scheduling itself is out of scope for v1; reserving the slot here keeps the schema honest.
- Dimension mismatch (record dim ≠ table dim) returns `anyhow::Error` from `upsert` and writes nothing.
- `search` performs cosine similarity, applies `SearchFilters` post-fetch (filter-then-limit may over-fetch internally — fetch `2 * k` then trim).
- `VectorHit { chunk_id, score, doc_id, text, heading_path }`. LanceDB returns *cosine distance* in [0, 2] (= `1 - cosine_similarity` for L2-normalized vectors, range [-1, 1] → distance [0, 2]). Convert: `similarity = 1.0 - distance` ∈ [-1, 1], then **shift** to [0, 1] via `score = (similarity + 1.0) / 2.0` rather than clamping. Clamping would crush all negative similarities to 0 and discard ranking signal between \"unrelated\" (sim ≈ 0) and \"opposite\" (sim ≈ -1). The shift preserves order. Clamping is reserved for floating-point sentinels (`NaN` → score 0, log warning).
- `search` returns empty `Vec` (not error) when table absent.
- `index_id` for `ensure_table` per design §4.2 with `collection = "chunk_embeddings"`, `index_kind = "flat"`, `params_hash = blake3(serde_json(table_schema))`.

## Storage / wire effects

- Writes Lance tables under `data_dir/lancedb/`.
- Writes/reads `embedding_records` rows.
- Reads chunks/documents not from this crate (the caller pre-fetches text + heading via `VectorRecord`).

## Test plan

| kind | description | fixture / data |
|------|-------------|----------------|
| unit | `ensure_table` creates dir; second call returns same `IndexId` | tmp data_dir |
| unit | `upsert` of 10 records makes them retrievable via `search` (k=5) | tmp data_dir |
| unit | dimension mismatch → error, no Lance row written | tmp data_dir |
| unit | filter `tags_any` removes non-matching docs | tmp data_dir + seeded sqlite tags |
| unit | model isolation: two models live in two directories with same `chunk_id` | tmp data_dir |
| unit | search before any upsert returns empty Vec | tmp data_dir |
| determinism | same query vector + same data → same top-k order | tmp data_dir |
| snapshot | `Vec<VectorHit>` JSON for fixed corpus stable | `fixtures/vector/run-1.json` |

All tests under `cargo test -p kebab-store-vector`.

## Definition of Done

- [ ] `cargo check -p kebab-store-vector` passes
- [ ] `cargo test -p kebab-store-vector` passes
- [ ] No imports outside Allowed dependencies
- [ ] `embedding_records` rows align 1:1 with Lance rows after a successful upsert batch
- [ ] PR links design §5.6, §6.3, §7.2

## Out of scope

- IVF / PQ index tuning (P+).
- Image / multimodal vector tables (P6).
- `kebab-app` orchestration of indexing jobs (`embed_index` facade method body).

## Risks / notes

- LanceDB's Rust API requires Arrow batches; constructing them per upsert is allocation-heavy — batch by configurable chunk size to avoid memory spikes.
- Filter-then-limit can starve `k` results; over-fetch by `2 * k` initially and double on retry up to a cap.
- WAL stability: ensure Lance commits before SQLite `INSERT INTO embedding_records` to avoid orphan SQLite rows.
