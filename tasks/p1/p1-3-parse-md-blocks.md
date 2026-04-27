---
phase: P1
component: kb-parse-md (blocks submodule)
task_id: p1-3
title: "Markdown body → Block tree with line spans"
status: planned
depends_on: [p0-1]
unblocks: [p1-4]
contract_source: ../../docs/superpowers/specs/2026-04-27-kb-final-form-design.md
contract_sections: [§3.4 Block, §3.4 SourceSpan, §3.7b kb-parse-types, §0 Q3 citation]
---

# p1-3 — Markdown body → Block tree

## Goal

Parse Markdown body bytes into a flat `Vec<kb_parse_types::ParsedBlock>` with heading paths and line ranges preserved, ready for `kb-normalize` to lift into `CanonicalDocument`.

## Why now / why this size

This is the heaviest part of P1 parser. Separating it from frontmatter and from normalization keeps each piece tractable. Determinism of line ranges directly determines citation quality (design §0 Q3 / §3.4 SourceSpan::Line).

## Allowed dependencies

- `kb-core`
- `kb-parse-types` (defines `ParsedBlock`, `ParsedPayload`, `Warning`)
- `pulldown-cmark` (CommonMark with source-map; GFM tables enabled via feature)
- `serde`
- `thiserror`

## Forbidden dependencies

- `kb-store-*`, `kb-llm*`, `kb-rag`, `kb-embed*`, `kb-search`, `kb-source-fs`, `kb-chunk`, `kb-normalize`, `kb-tui`, `kb-desktop`, `comrak` (alternative parser; pick one)

## Inputs

| input | type | source |
|-------|------|--------|
| Markdown body bytes | `&[u8]` | extractor (after frontmatter stripped) |
| `body_offset_lines` | `u32` | extractor (so line ranges are reported relative to original file) |

## Outputs

| output | type | downstream |
|--------|------|------------|
| `Vec<kb_parse_types::ParsedBlock>` | shared type from `kb-parse-types` | `kb-normalize` |
| `Vec<kb_parse_types::Warning>` | shared type | propagated into Provenance |

## Public surface (signatures only — no new types)

```rust
pub fn parse_blocks(
    body: &[u8],
    body_offset_lines: u32,
) -> anyhow::Result<(Vec<kb_parse_types::ParsedBlock>, Vec<kb_parse_types::Warning>)>;
```

`ParsedBlock` is defined in `kb-parse-types` (design §3.7b). `kb-parse-md` does NOT define its own; it consumes the shared type. Lift to `kb_core::Block` (with `BlockId` assignment) is `kb-normalize`'s job (p1-4).

## Behavior contract

- Source-map: each `ParsedBlock` carries `SourceSpan::Line { start, end }` relative to the original file (i.e., add `body_offset_lines`).
- Heading tree: every block records its ancestor heading texts in order (e.g., `["아키텍처", "Chunking 정책"]`).
- Code blocks: `ParsedPayload::Code { lang: Some("rust"), code }` — fenced content not split.
- Tables: GFM tables produce `ParsedPayload::Table { headers, rows }`; if a table cell is malformed, fall back to `ParsedPayload::Paragraph` + `Warning::MalformedTable`.
- Image references: `![alt](src)` produces `ParsedPayload::ImageRef { src, alt }`. `AssetId` resolution happens later in `kb-normalize` (when image src can be matched to a workspace asset).
- Lists: ordered/unordered preserved via `ParsedPayload::List { ordered, items }`; nested list items flattened so each `items[i]` is a `Vec<kb_core::Inline>` for one top-level item.
- Inline elements: only `Text`, `Code`, `Link`, `Strong`, `Emph` (per `kb_core::Inline` per design §3.4). Drop other inlines silently.
- Malformed input never panics. Worst case: empty `Vec<ParsedBlock>` + `Warning::ExtractFailed`.

## Storage / wire effects

- None.

## Test plan

| kind | description | fixture / data |
|------|-------------|----------------|
| unit | heading tree depth + heading_path correctness | inline |
| unit | code block lang tag preserved | inline |
| unit | GFM table parses; malformed table degrades to paragraph + warning | inline |
| unit | line range correct under various line-ending styles (LF / CRLF) | inline |
| unit | image ref captured with src/alt | inline |
| unit | nested list flattens correctly | inline |
| unit | malformed input does not panic | inline (random byte slices) |
| snapshot | `fixtures/markdown/nested-headings.md` → ParsedBlock JSON stable | fixture |
| snapshot | `fixtures/markdown/code-and-table.md` → JSON stable | fixture |

All tests under `cargo test -p kb-parse-md --lib blocks`.

## Definition of Done

- [ ] `cargo check -p kb-parse-md` passes
- [ ] `cargo test -p kb-parse-md blocks` passes
- [ ] Snapshot tests stable across two runs
- [ ] No imports outside Allowed dependencies
- [ ] PR links design §3.4

## Out of scope

- Frontmatter (p1-2).
- Lifting `kb_parse_types::ParsedBlock` → `kb_core::Block` with `BlockId` (p1-4 normalize).
- Chunking (p1-5).

## Risks / notes

- `pulldown-cmark` source-map may not include exact byte ranges for all event kinds; line ranges are the binding contract per design (line-range citation is the primary form for Markdown).
- CRLF normalization: convert internally to LF for span math but report line numbers from the original byte stream.
