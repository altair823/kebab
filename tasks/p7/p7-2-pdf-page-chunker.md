---
phase: P7
component: kebab-chunk (pdf-page-v1)
task_id: p7-2
title: "PDF page-aware chunker (pdf-page-v1)"
status: completed
depends_on: [p7-1]
unblocks: []
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§3.5 Chunk, §4.2 chunk_id recipe, §0 Q3 citation, §9 versioning]
---

# p7-2 — PDF page chunker

## Goal

Implement `Chunker` with `chunker_version = "pdf-page-v1"`. Honors page boundaries (no chunk crosses a page) and subdivides long pages by paragraph budget. Produces the same `Chunk` shape as `md-heading-v1` so retrieval is uniform.

## Why now / why this size

Per-medium chunkers must stay tiny and obvious. Page-aware logic is small but its `chunker_version` label is load-bearing for downstream embedding records.

## Allowed dependencies

- `kebab-core`
- `kebab-config`
- `serde`, `serde_json`
- `blake3` (policy_hash)
- `serde-json-canonicalizer`
- `thiserror`

## Forbidden dependencies

- `kebab-source-fs`, `kebab-parse-md`, `kebab-parse-pdf` (consumes `CanonicalDocument` via `kebab-core` only), `kebab-normalize`, `kebab-store-*`, `kebab-embed*`, `kebab-search`, `kebab-llm*`, `kebab-rag`, `kebab-tui`, `kebab-desktop`

## Inputs

| input | type | source |
|-------|------|--------|
| `CanonicalDocument` (produced by `pdf-text-v1`) | `kebab_core::CanonicalDocument` | p7-1 |
| `ChunkPolicy` | `kebab_core::ChunkPolicy` | `kebab-app` |

## Outputs

| output | type | downstream |
|--------|------|------------|
| `Vec<Chunk>` | `kebab_core::Chunk` | `kebab-store-sqlite`, `kebab-embed*` |

## Public surface (signatures only — no new types)

```rust
pub struct PdfPageV1Chunker;

impl kebab_core::Chunker for PdfPageV1Chunker {
    fn chunker_version(&self) -> kebab_core::ChunkerVersion { kebab_core::ChunkerVersion("pdf-page-v1".into()) }
    fn policy_hash(&self, policy: &kebab_core::ChunkPolicy) -> String;
    fn chunk(&self, doc: &kebab_core::CanonicalDocument, policy: &kebab_core::ChunkPolicy) -> anyhow::Result<Vec<kebab_core::Chunk>>;
}
```

`policy_hash` = `blake3(canonical_json(policy))` truncated to 16 hex chars.

## Behavior contract

- Only operates on documents whose blocks all carry `SourceSpan::Page` (i.e., from `kebab-parse-pdf`). Other documents → return `anyhow::Error("PdfPageV1Chunker only handles PDF docs")`.
- For each page block (1 block per page after p7-1):
  - If `text.len()` (byte estimate) ≤ `policy.target_tokens * 4` (proxy for tokens) → emit one chunk for the entire page.
  - Else → split by paragraphs (split text on `\n\n` or sentence-ending punctuation followed by whitespace) and group adjacent paragraphs until the running byte total approaches `policy.target_tokens * 4`. Apply `policy.overlap_tokens * 4` bytes of trailing overlap into the next chunk's prefix.
- A chunk NEVER crosses a page boundary.
- Each chunk's `source_spans` contains exactly one `SourceSpan::Page { page: i, char_start: Some(start), char_end: Some(end) }` with `start`/`end` in characters within the page.
- `heading_path = []` (PDFs have no heading tree at v1).
- `block_ids = [page_block.block_id]` (one block per chunk).
- `text` = the chunk's slice of page text. If overlap is applied, the slice includes the overlap prefix from the previous chunk.
- `token_estimate = byte_len / 4` (matches `md-heading-v1` proxy).
- `chunk_id` per design §4.2 with `(doc_id, "pdf-page-v1", block_ids, policy_hash)`.
- Determinism: identical inputs + identical policy → identical chunk IDs and text slices.

## Storage / wire effects

- None.

## Test plan

| kind | description | fixture / data |
|------|-------------|----------------|
| unit | 3-page PDF where each page < target_tokens → 3 chunks, 1 per page | seeded `CanonicalDocument` |
| unit | 1-page PDF whose text >> target_tokens → multiple chunks all on page 1 with overlap honored | seeded |
| unit | chunk crossing page boundary never produced | property test (10 random docs) |
| unit | empty page block → 0 chunks for that page (skipped) | inline |
| unit | non-PDF doc returns error | inline (Markdown-style doc) |
| determinism | same input → same chunk_ids twice | inline |
| snapshot | `Vec<Chunk>` JSON for fixture stable | `fixtures/pdf/three-page-en.pdf` (chunked) |

All tests under `cargo test -p kebab-chunk pdf`.

## Definition of Done

- [ ] `cargo check -p kebab-chunk` passes (existing `md-heading-v1` continues to pass)
- [ ] `cargo test -p kebab-chunk pdf` passes
- [ ] Snapshot stable across two runs
- [ ] No imports outside Allowed dependencies
- [ ] PR links design §3.5, §0 Q3, §9

## Out of scope

- Token-accurate splitting (real tokenizer integration is P+).
- Cross-page sentence merging (kept off; page citation simplicity wins).
- Section/heading inference from font metadata (P+).

## Risks / notes

- Byte-based proxy can over- or under-estimate. The chunker is intentionally crude; a proper tokenizer slot lives in P3+ and replaces this proxy across all chunkers in one PR.
- Sentence-splitting uses simple regex; languages without clear sentence punctuation (e.g., Japanese) may produce uneven chunks. Document this and accept for v1.
- Bumping `chunker_version` to `pdf-page-v2` invalidates downstream embedding records for all PDFs; treat as a versioning event per §9.
