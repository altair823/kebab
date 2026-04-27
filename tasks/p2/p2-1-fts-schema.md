---
phase: P2
component: kb-store-sqlite (FTS5 migration)
task_id: p2-1
title: "FTS5 virtual table + triggers (V002 migration)"
status: planned
depends_on: [p1-6]
unblocks: [p2-2]
contract_source: ../../docs/superpowers/specs/2026-04-27-kb-final-form-design.md
contract_sections: [§5.5 chunks_fts + triggers, §9 versioning]
---

# p2-1 — FTS5 virtual table + triggers

## Goal

Add `chunks_fts` virtual table and three sync triggers via migration `V002__fts.sql`. Backfill existing chunks if any.

## Why now / why this size

`chunks_fts` is the lexical index for `kb-search`. Splitting it from p1-6 keeps P1 focused on relational data; bringing it as `V002` lets users upgrade an existing P1 DB without re-ingesting.

## Allowed dependencies

- `kb-core`
- `kb-config`
- `kb-store-sqlite` (extends migrations)
- `rusqlite`
- `refinery`

## Forbidden dependencies

- `kb-source-fs`, `kb-parse-md`, `kb-normalize`, `kb-chunk`, `kb-store-vector`, `kb-embed*`, `kb-search` (consumer is p2-2), `kb-llm*`, `kb-rag`, `kb-tui`, `kb-desktop`

## Inputs

| input | type | source |
|-------|------|--------|
| existing `chunks` rows | SQLite | from p1-6 |
| migration runner | `refinery` | from p1-6 |

## Outputs

| output | type | downstream |
|--------|------|------------|
| `chunks_fts` virtual table populated | SQLite | p2-2 lexical retriever |
| three triggers synced with `chunks` | SQLite | every later chunk write |

## Public surface (signatures only — no new types)

```rust
pub fn rebuild_chunks_fts(conn: &rusqlite::Connection) -> anyhow::Result<()>;
```

(Used by `kb index --rebuild-fts`. Re-runs `INSERT INTO chunks_fts SELECT ... FROM chunks` after `DELETE FROM chunks_fts;`.)

## Behavior contract

- Migration file `migrations/V002__fts.sql` ships exactly the SQL in design §5.5 (FTS5 virtual table with `unicode61 remove_diacritics 2` tokenizer + `chunks_ai` / `chunks_ad` / `chunks_au` triggers).
- On migration apply, backfill: `INSERT INTO chunks_fts(chunk_id, doc_id, heading_path, text) SELECT chunk_id, doc_id, heading_path_json, text FROM chunks;`.
- `rebuild_chunks_fts` is idempotent: full delete then re-insert from `chunks`.
- Triggers ensure that every future `INSERT`/`UPDATE`/`DELETE` on `chunks` keeps `chunks_fts` in sync within the same transaction.
- `chunks_fts` row count must equal `chunks` row count after any successful migration / rebuild.

## Storage / wire effects

- Writes: `chunks_fts` virtual table inside `kb.sqlite`.
- Reads: existing `chunks` rows for backfill.

## Test plan

| kind | description | fixture / data |
|------|-------------|----------------|
| migration | apply `V002` to a DB seeded with N chunks; `chunks_fts` contains exactly N rows | tmp DB seeded |
| trigger | INSERT into `chunks` propagates to `chunks_fts` | tmp DB |
| trigger | DELETE from `chunks` removes the corresponding `chunks_fts` row | tmp DB |
| trigger | UPDATE of `chunks.text` updates `chunks_fts` text | tmp DB |
| function | `rebuild_chunks_fts` produces deterministic content equal to fresh backfill | tmp DB |
| migration | running `V002` twice is a no-op (refinery handles idempotency) | tmp DB |

All tests under `cargo test -p kb-store-sqlite fts`.

## Definition of Done

- [ ] `cargo check -p kb-store-sqlite` passes
- [ ] `cargo test -p kb-store-sqlite fts` passes
- [ ] `migrations/V002__fts.sql` matches design §5.5 verbatim (CI diff check)
- [ ] No imports outside Allowed dependencies
- [ ] PR links design §5.5

## Out of scope

- Search query implementation (p2-2).
- Vector / hybrid search (P3).
- Korean morphological tokenizer (kept as P+ note; default `unicode61 remove_diacritics 2`).

## Risks / notes

- FTS5 triggers run inside the same transaction as their host `chunks` mutation; bulk ingest performance may need batching considerations later.
- `chunks_fts` is a **content-less** FTS5 table per §5.5 (with UNINDEXED `chunk_id`/`doc_id`). Tests should rely on `bm25(chunks_fts)` ranking only — not on raw scoring values.
