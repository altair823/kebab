---
phase: P1
component: kebab-store-sqlite (P1 subset)
task_id: p1-6
title: "SQLite store: assets/documents/blocks/chunks + asset writer + migrations"
status: completed
depends_on: [p1-1, p1-4, p1-5]
unblocks: [p2-1, p3-3, p4-3]
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§5 DDL (5.1, 5.2, 5.3, 5.4, 5.5 chunks only — FTS handled in p2-1), §5.7 jobs/ingest_runs, §5.8 transactions, §6.3 data_dir layout]
---

# p1-6 — SQLite store (P1 subset)

## Goal

Persist `RawAsset`, `CanonicalDocument`, `Block`s, `Chunk`s into SQLite per design §5; copy raw asset bytes into `data_dir/assets/<aa>/<asset_id>` (or reference if larger than threshold); record an `ingest_runs` row.

## Why now / why this size

P1's terminal task. Closes the loop `walk → parse → chunk → store`. The FTS5 virtual table and triggers are intentionally deferred to p2-1 to keep this task focused on the relational schema and asset I/O.

## Allowed dependencies

- `kebab-core`
- `kebab-config`
- `rusqlite` (with `bundled-sqlcipher` disabled; use `bundled` feature)
- `refinery` for migrations
- `serde_json`
- `time`
- `blake3` (asset copy verification)
- `tracing`
- `thiserror`

## Forbidden dependencies

- `kebab-source-fs` (only types via `kebab-core`), `kebab-parse-md`, `kebab-normalize`, `kebab-chunk` (only types via `kebab-core`), `kebab-store-vector`, `kebab-embed*`, `kebab-search`, `kebab-llm*`, `kebab-rag`, `kebab-tui`, `kebab-desktop`

## Inputs

| input | type | source |
|-------|------|--------|
| migrations | `migrations/V001__init.sql` | repo |
| `RawAsset` + bytes | `(RawAsset, Vec<u8>)` | p1-1 + reader |
| `CanonicalDocument` | `kebab_core::CanonicalDocument` | p1-4 |
| `Vec<Chunk>` | `kebab_core::Chunk` | p1-5 |
| `IngestRun` aggregates | `(scope, counts, duration)` | `kebab-app` |

## Outputs

| output | type | downstream |
|--------|------|------------|
| `data_dir/kebab.sqlite` rows in `assets`, `documents`, `blocks`, `chunks`, `document_tags`, `ingest_runs`, `jobs`, `schema_meta`, `migrations` | – | every later phase |
| `data_dir/assets/<aa>/<asset_id>` bytes (when copied) | – | future re-extraction, integrity verification |
| `IngestReport` (wire schema v1) | `kebab_core::IngestReport` | `kebab-cli`, eval |

## Public surface (signatures only — no new types)

```rust
pub struct SqliteStore { /* internal */ }

impl SqliteStore {
    pub fn open(config: &kebab_config::Config) -> anyhow::Result<Self>;
    pub fn run_migrations(&self) -> anyhow::Result<()>;

    pub fn put_asset_with_bytes(&self, asset: &kebab_core::RawAsset, bytes: &[u8]) -> anyhow::Result<()>;
}

impl kebab_core::DocumentStore for SqliteStore {
    fn put_asset(&self, a: &kebab_core::RawAsset) -> anyhow::Result<()>;
    fn put_document(&self, d: &kebab_core::CanonicalDocument) -> anyhow::Result<()>;
    fn put_blocks(&self, doc: &kebab_core::DocumentId, blocks: &[kebab_core::Block]) -> anyhow::Result<()>;
    fn put_chunks(&self, doc: &kebab_core::DocumentId, chunks: &[kebab_core::Chunk]) -> anyhow::Result<()>;
    fn get_document(&self, id: &kebab_core::DocumentId) -> anyhow::Result<Option<kebab_core::CanonicalDocument>>;
    fn get_chunk(&self, id: &kebab_core::ChunkId) -> anyhow::Result<Option<kebab_core::Chunk>>;
    fn list_documents(&self, filter: &kebab_core::DocFilter) -> anyhow::Result<Vec<kebab_core::DocSummary>>;
}

impl kebab_core::JobRepo for SqliteStore { /* per design §7.2 signatures */ }
```

## Behavior contract

- DDL: `migrations/V001__init.sql` ships exactly the SQL in design §5.1, §5.2, §5.3, §5.4, §5.5 (chunks table only — FTS table & triggers come in p2-1 as `V002`), §5.7 jobs/ingest_runs/answers/eval_runs/eval_query_results, §5.6 embedding_records.
- Pragmas at open: `foreign_keys=ON`, `journal_mode=WAL`, `synchronous=NORMAL`, `temp_store=MEMORY`.
- One ingest of one document = one transaction (BEGIN..COMMIT). Partial failures roll back; warnings are not failures.
- Bulk ingest commits per-document.
- Asset writer:
  - if `asset.byte_len <= storage.copy_threshold_mb * 1_048_576`: write bytes to `assets_dir/<asset_id[..2]>/<asset_id>` (mode 0o644), record `storage_kind='copied'`.
  - else: do not copy; record `storage_kind='reference'` with `storage_path = asset.source_uri`'s file path.
  - In either case, recompute `blake3` of the source bytes once on write/verify and store in `assets.checksum`. Mismatch → return `StoreError::Conflict`.
- Idempotency: re-ingesting the same `(workspace_path, asset_id, parser_version)` updates `documents.updated_at`, increments `doc_version`, replaces blocks/chunks. No row duplication.
- `document_tags`: re-derived from `Metadata.tags` on each put.
- `ingest_runs.items_json` is null when caller passes `summary_only=true`.
- All wire JSON returned (`IngestReport`) conforms to `docs/wire-schema/v1/ingest_report.schema.json`. Fail loudly if schema not present (caller must vendor it).

## Storage / wire effects

- Writes: `kebab.sqlite` (multiple tables), `data_dir/assets/<aa>/<asset_id>` (copied case).
- Reads on subsequent calls: same DB.

## Test plan

| kind | description | fixture / data |
|------|-------------|----------------|
| migration | fresh DB after `run_migrations` has all P1 tables and indexes | tmp dir |
| unit | put_asset_with_bytes copy mode writes file with correct mode and bytes | tmp dir |
| unit | put_asset_with_bytes reference mode does not write file but records path | tmp dir + large fake size |
| unit | checksum mismatch returns Conflict error | tmp dir + tampered bytes |
| unit | put_document idempotency: same input twice → 1 row, doc_version bumped | tmp dir |
| unit | put_blocks + put_chunks transactional rollback on simulated failure | tmp dir |
| contract | DocumentStore trait round-trip for fixture document | `fixtures/markdown/code-and-table.md` |
| snapshot | IngestReport JSON for fixture run | fixture |

All tests under `cargo test -p kebab-store-sqlite` with no network.

## Definition of Done

- [ ] `cargo check -p kebab-store-sqlite` passes
- [ ] `cargo test -p kebab-store-sqlite` passes
- [ ] migration `V001__init.sql` matches design §5 verbatim (diff-checked in CI)
- [ ] Writes to `~/.local/share/kebab/` are gated by `kebab-config`'s `data_dir` and never escape it
- [ ] No imports outside Allowed dependencies
- [ ] PR links design §5

## Out of scope

- FTS5 virtual table and triggers (p2-1).
- Vector store (p3-3).
- Embedding records writer (p3-2).
- Search queries (p2-2).

## Risks / notes

- WAL mode requires careful test cleanup: tests must drop the connection before removing `kebab.sqlite-wal` / `-shm`.
- Asset directory shard prefix uses `asset_id[..2]`; using `asset_id[..1]` would create at most 16 dirs (insufficient).
