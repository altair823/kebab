---
phase: P1
component: kb-source-fs
task_id: p1-1
title: "Local filesystem source connector"
status: completed
depends_on: [p0-1]
unblocks: [p1-2, p1-3, p1-4, p1-5, p1-6]
contract_source: ../../docs/superpowers/specs/2026-04-27-kb-final-form-design.md
contract_sections: [§3.3, §6.2, §6.6, §7.1, §7.2 SourceConnector, §8]
---

# p1-1 — Local filesystem source connector

## Goal

Walk the workspace root, apply gitignore-style filters, compute BLAKE3 checksums, and produce `Vec<RawAsset>`.

## Why now / why this size

`SourceConnector` is the entry point of every ingest. Stable `RawAsset` output unblocks every downstream P1 task (parser, normalize, chunk, store). Small enough to deliver in one PR with full test coverage.

## Allowed dependencies

- `kb-core`
- `kb-config`
- `ignore` (gitignore semantics)
- `blake3`
- `walkdir`
- `time`
- `serde`
- `thiserror`
- `tracing`

## Forbidden dependencies

- `kb-parse-*`, `kb-normalize`, `kb-chunk`, `kb-store-*`, `kb-embed*`, `kb-search`, `kb-llm*`, `kb-rag`, `kb-tui`, `kb-desktop`

## Inputs

| input | type | source |
|-------|------|--------|
| `SourceScope` | `kb_core::SourceScope` | `kb-app` from config |
| filesystem | `&Path` | OS |
| `.kbignore` | text file | workspace root, optional |

## Outputs

| output | type | downstream consumer |
|--------|------|---------------------|
| `Vec<RawAsset>` | `kb_core::RawAsset` | `kb-parse-md`, asset writer in `kb-store-sqlite` (via `kb-app`) |

## Public surface (signatures only — no new types)

```rust
pub struct FsSourceConnector { /* internal */ }

impl FsSourceConnector {
    pub fn new(config: &kb_config::Config) -> anyhow::Result<Self>;
}

impl kb_core::SourceConnector for FsSourceConnector {
    fn scan(&self, scope: &kb_core::SourceScope) -> anyhow::Result<Vec<kb_core::RawAsset>>;
}
```

## Behavior contract

- POSIX-normalize every emitted `workspace_path` (NFC, leading `./` stripped, single `/`).
- `asset_id` derived per design §4.2 from `blake3(raw bytes)` full hex.
- `media_type` selected from extension + libmagic-like sniff fallback (`.md` → Markdown, others fall through to `MediaType::Other`).
- `discovered_at` = current `OffsetDateTime::now_utc()` at scan time.
- Combine `config.workspace.exclude` ∪ `.kbignore` for filter (union; ordering does not matter).
- Symbolic links: follow once, detect cycles via `canonicalize` + visited set.
- Files larger than `storage.copy_threshold_mb` MB → emit `AssetStorage::Reference { path, sha }` (do not copy bytes here; copying is done by the asset writer task).
- Idempotent: same input → same `Vec<RawAsset>` (sort by `workspace_path`).

## Storage / wire effects

- Reads: filesystem under `config.workspace.root`.
- Writes: nothing. (Asset copy is handled by the asset writer in `kb-store-sqlite`.)

## Test plan

| kind | description | fixture / data |
|------|-------------|----------------|
| unit | POSIX path normalization | inline cases incl. `./a/b.md`, `a//b.md`, `a/b.md` → identical |
| unit | blake3 of known bytes matches expected hex | inline |
| unit | gitignore filter (`*.tmp`, `node_modules/**`) excludes correctly | tmp tree built in test |
| unit | `.kbignore` ∪ config exclude works | tmp tree |
| unit | symlink cycle does not loop | tmp tree with `a -> b -> a` |
| snapshot | `Vec<RawAsset>` serialized JSON for fixture tree is stable | `fixtures/source-fs/tree-1` |
| determinism | re-running scan twice produces byte-identical JSON | `fixtures/source-fs/tree-1` |

All tests run under `cargo test -p kb-source-fs` with no network and no model.

## Definition of Done

- [ ] `cargo check -p kb-source-fs` passes
- [ ] `cargo test -p kb-source-fs` passes
- [ ] Snapshot test `fixtures/source-fs/tree-1` round-trips deterministically
- [ ] No imports outside Allowed dependencies (verified via `cargo tree -p kb-source-fs`)
- [ ] PR description links to design §3.3, §6.2, §7.2

## Out of scope

- File watching (P+).
- Asset copy/reference storage on disk (`kb-store-sqlite` task p1-6).
- Non-fs source connectors (HTTP, S3 — P+).

## Risks / notes

- BLAKE3 of large files (>1 GB) is fast but allocate streaming; do not load whole file in memory.
- macOS resource forks / `.DS_Store` should be excluded by default.
