---
phase: P1
component: kebab-normalize
task_id: p1-4
title: "Lift parser output → CanonicalDocument with deterministic IDs"
status: completed
depends_on: [p1-2, p1-3]
unblocks: [p1-5, p1-6]
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§3.4, §3.7b kebab-parse-types, §4 ID recipe, §3.6 Provenance, §8 module boundaries]
---

# p1-4 — Lift to CanonicalDocument

## Goal

Combine `Metadata` (p1-2) + `Vec<kebab_parse_types::ParsedBlock>` (p1-3) + `RawAsset` (p1-1) into a `kebab_core::CanonicalDocument` with deterministic `doc_id` and `block_id`s per design §4 recipe.

## Why now / why this size

Single responsibility: ID generation + struct assembly. Keeps `kebab-parse-md` purely a parser and isolates the (security-critical) deterministic ID logic in one crate.

## Allowed dependencies

- `kebab-core`
- `kebab-parse-types` (input shapes — `ParsedBlock`, `ParsedPayload`, `Warning`)
- `kebab-config`
- `serde`
- `serde-json-canonicalizer` (canonical JSON for ID hashing)
- `blake3`
- `unicode-normalization` (NFC)
- `time`
- `thiserror`

## Forbidden dependencies

- `kebab-source-fs`, `kebab-parse-md` (consumed via shared `kebab-parse-types` only — `kebab-parse-md` must NOT appear in this crate's `cargo tree`), `kebab-parse-pdf`, `kebab-parse-image`, `kebab-parse-audio`, `kebab-chunk`, `kebab-store-*`, `kebab-embed*`, `kebab-search`, `kebab-llm*`, `kebab-rag`, `kebab-tui`, `kebab-desktop`

## Inputs

| input | type | source |
|-------|------|--------|
| `RawAsset` | `kebab_core::RawAsset` | p1-1 |
| `Metadata` + frontmatter span + warnings | `(kebab_core::Metadata, Option<FrontmatterSpan>, Vec<kebab_parse_types::Warning>)` | p1-2 |
| `Vec<ParsedBlock>` + warnings | `(Vec<kebab_parse_types::ParsedBlock>, Vec<kebab_parse_types::Warning>)` | p1-3 |
| `parser_version` | `kebab_core::ParserVersion` | constant in `kebab-parse-md` |

## Outputs

| output | type | downstream |
|--------|------|------------|
| `CanonicalDocument` | `kebab_core::CanonicalDocument` | `kebab-chunk`, `kebab-store-sqlite` |

## Public surface (signatures only — no new types)

```rust
pub fn build_canonical_document(
    asset: &kebab_core::RawAsset,
    metadata: kebab_core::Metadata,
    blocks: Vec<kebab_parse_types::ParsedBlock>,
    parser_version: &kebab_core::ParserVersion,
    warnings: Vec<kebab_parse_types::Warning>,
) -> anyhow::Result<kebab_core::CanonicalDocument>;

pub fn id_for_doc(workspace_path: &kebab_core::WorkspacePath, asset: &kebab_core::AssetId, parser_version: &kebab_core::ParserVersion) -> kebab_core::DocumentId;
pub fn id_for_block(doc: &kebab_core::DocumentId, kind: &str, heading_path: &[String], ordinal: u32, span: &kebab_core::SourceSpan) -> kebab_core::BlockId;
```

## Behavior contract

- ID generation strictly follows design §4.2 (canonical JSON of tagged tuple, blake3 hex truncated to 32 chars).
- `block_id` ordinal: per `(heading_path, kind)` group, 0-based, in document order.
- All input strings normalized to NFC before hashing.
- POSIX path normalization applied to `workspace_path`.
- Unicode line endings normalized internally; `SourceSpan::Line` indices preserved as-is from p1-3.
- `Provenance` built with one event per pipeline stage encountered: `Discovered`, `Parsed`, `Normalized`. Warnings appended as `ProvenanceKind::Warning` with `note`.
- Determinism property test: same inputs → byte-identical `CanonicalDocument` JSON, including ID stability across runs.

## Storage / wire effects

- None.

## Test plan

| kind | description | fixture / data |
|------|-------------|----------------|
| unit | id_for_doc deterministic across 1000 runs | inline |
| unit | NFC vs NFD Korean inputs produce identical IDs | inline |
| unit | POSIX path with `./` and `//` collapse to same `doc_id` | inline |
| unit | block ordinal numbering inside same heading_path is correct | inline |
| unit | provenance contains Discovered/Parsed/Normalized in order | inline |
| snapshot | `fixtures/markdown/code-and-table.md` → CanonicalDocument JSON stable (incl. all IDs) | fixture |

All tests under `cargo test -p kebab-normalize`.

## Definition of Done

- [ ] `cargo check -p kebab-normalize` passes
- [ ] `cargo test -p kebab-normalize` passes
- [ ] Determinism test runs ≥ 1000 iterations under 1 second
- [ ] No `kebab-parse-md` (or any other parser crate) appears in `cargo tree -p kebab-normalize` — input types come from `kebab-parse-types` only
- [ ] PR links design §3.7b, §4.2, §4.3, §8

## Out of scope

- Chunking (p1-5).
- DB writes (p1-6).
- Block validation beyond what is needed to assign IDs (e.g., we do NOT verify image src exists on disk here).

## Risks / notes

- If ID recipe changes, all dependent records become stale. Treat any change to `id_for_doc`/`id_for_block` as a `parser_version` bump (design §9).
