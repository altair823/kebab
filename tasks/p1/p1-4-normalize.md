---
phase: P1
component: kb-normalize
task_id: p1-4
title: "Lift parser output → CanonicalDocument with deterministic IDs"
status: completed
depends_on: [p1-2, p1-3]
unblocks: [p1-5, p1-6]
contract_source: ../../docs/superpowers/specs/2026-04-27-kb-final-form-design.md
contract_sections: [§3.4, §3.7b kb-parse-types, §4 ID recipe, §3.6 Provenance, §8 module boundaries]
---

# p1-4 — Lift to CanonicalDocument

## Goal

Combine `Metadata` (p1-2) + `Vec<kb_parse_types::ParsedBlock>` (p1-3) + `RawAsset` (p1-1) into a `kb_core::CanonicalDocument` with deterministic `doc_id` and `block_id`s per design §4 recipe.

## Why now / why this size

Single responsibility: ID generation + struct assembly. Keeps `kb-parse-md` purely a parser and isolates the (security-critical) deterministic ID logic in one crate.

## Allowed dependencies

- `kb-core`
- `kb-parse-types` (input shapes — `ParsedBlock`, `ParsedPayload`, `Warning`)
- `kb-config`
- `serde`
- `serde-json-canonicalizer` (canonical JSON for ID hashing)
- `blake3`
- `unicode-normalization` (NFC)
- `time`
- `thiserror`

## Forbidden dependencies

- `kb-source-fs`, `kb-parse-md` (consumed via shared `kb-parse-types` only — `kb-parse-md` must NOT appear in this crate's `cargo tree`), `kb-parse-pdf`, `kb-parse-image`, `kb-parse-audio`, `kb-chunk`, `kb-store-*`, `kb-embed*`, `kb-search`, `kb-llm*`, `kb-rag`, `kb-tui`, `kb-desktop`

## Inputs

| input | type | source |
|-------|------|--------|
| `RawAsset` | `kb_core::RawAsset` | p1-1 |
| `Metadata` + frontmatter span + warnings | `(kb_core::Metadata, Option<FrontmatterSpan>, Vec<kb_parse_types::Warning>)` | p1-2 |
| `Vec<ParsedBlock>` + warnings | `(Vec<kb_parse_types::ParsedBlock>, Vec<kb_parse_types::Warning>)` | p1-3 |
| `parser_version` | `kb_core::ParserVersion` | constant in `kb-parse-md` |

## Outputs

| output | type | downstream |
|--------|------|------------|
| `CanonicalDocument` | `kb_core::CanonicalDocument` | `kb-chunk`, `kb-store-sqlite` |

## Public surface (signatures only — no new types)

```rust
pub fn build_canonical_document(
    asset: &kb_core::RawAsset,
    metadata: kb_core::Metadata,
    blocks: Vec<kb_parse_types::ParsedBlock>,
    parser_version: &kb_core::ParserVersion,
    warnings: Vec<kb_parse_types::Warning>,
) -> anyhow::Result<kb_core::CanonicalDocument>;

pub fn id_for_doc(workspace_path: &kb_core::WorkspacePath, asset: &kb_core::AssetId, parser_version: &kb_core::ParserVersion) -> kb_core::DocumentId;
pub fn id_for_block(doc: &kb_core::DocumentId, kind: &str, heading_path: &[String], ordinal: u32, span: &kb_core::SourceSpan) -> kb_core::BlockId;
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

All tests under `cargo test -p kb-normalize`.

## Definition of Done

- [ ] `cargo check -p kb-normalize` passes
- [ ] `cargo test -p kb-normalize` passes
- [ ] Determinism test runs ≥ 1000 iterations under 1 second
- [ ] No `kb-parse-md` (or any other parser crate) appears in `cargo tree -p kb-normalize` — input types come from `kb-parse-types` only
- [ ] PR links design §3.7b, §4.2, §4.3, §8

## Out of scope

- Chunking (p1-5).
- DB writes (p1-6).
- Block validation beyond what is needed to assign IDs (e.g., we do NOT verify image src exists on disk here).

## Risks / notes

- If ID recipe changes, all dependent records become stale. Treat any change to `id_for_doc`/`id_for_block` as a `parser_version` bump (design §9).
