---
phase: P7
component: kb-parse-pdf (text extractor)
task_id: p7-1
title: "Text PDF extractor → CanonicalDocument with page-level blocks"
status: planned
depends_on: [p0-1, p1-6]
unblocks: [p7-2]
contract_source: ../../docs/superpowers/specs/2026-04-27-kb-final-form-design.md
contract_sections: [§3.4 SourceSpan::Page, §3.4 Block::Paragraph, §9.2 PDF text extraction, §9 versioning]
---

# p7-1 — PDF text extractor

## Goal

Implement `Extractor` for `MediaType::Pdf`. Extracts text page-by-page, emits one `Block::Paragraph` per page with `SourceSpan::Page`. Failed-text pages get an empty paragraph + `Provenance::Warning` so they can be picked up later by an OCR fallback pipeline.

## Why now / why this size

Strict scope: page text + page numbers. Layout reconstruction (multi-column merge, table extraction) is intentionally NOT in scope — it's its own engineering project. This task gets a usable PDF retrieval surface online with minimal moving parts.

## Allowed dependencies

- `kb-core`
- `kb-config`
- `pdf-extract = "0.7"` (or current stable)
- `lopdf = "0.32"` for page metadata (count, optional title from /Info)
- `serde`, `serde_json`
- `time`
- `tracing`
- `thiserror`

## Forbidden dependencies

- `kb-source-fs`, `kb-parse-md`, `kb-normalize`, `kb-chunk`, `kb-store-*`, `kb-embed*`, `kb-search`, `kb-llm*`, `kb-rag`, `kb-tui`, `kb-desktop`, OCR libraries (OCR fallback is a separate task, not this one)

## Inputs

| input | type | source |
|-------|------|--------|
| `RawAsset` | `kb_core::RawAsset` | `kb-source-fs` |
| PDF bytes | `&[u8]` | filesystem |

## Outputs

| output | type | downstream |
|--------|------|------------|
| `CanonicalDocument` | `kb_core::CanonicalDocument` | `kb-chunk` (`pdf-page-v1` chunker in p7-2) |

## Public surface (signatures only — no new types)

```rust
pub struct PdfTextExtractor;

impl kb_core::Extractor for PdfTextExtractor {
    fn supports(&self, m: &kb_core::MediaType) -> bool { matches!(m, kb_core::MediaType::Pdf) }
    fn parser_version(&self) -> kb_core::ParserVersion { kb_core::ParserVersion("pdf-text-v1".into()) }
    fn extract(&self, ctx: &kb_core::ExtractContext, bytes: &[u8]) -> anyhow::Result<kb_core::CanonicalDocument>;
}
```

## Behavior contract

- Page count obtained via `lopdf::Document::load_mem`; iterate `1..=n`.
- For each page:
  - Try `pdf-extract::extract_text_from_mem_by_pages(bytes)` (or equivalent) to get a `Vec<String>` aligned with pages.
  - If extraction returns text for page i: produce `Block::Paragraph(TextBlock { common, text, inlines: vec![Inline::Text(text)] })` with `common.source_span = SourceSpan::Page { page: i, char_start: Some(0), char_end: Some(text.len() as u32) }` and `common.heading_path = vec![]`.
  - If text is empty or extraction errored: produce `Block::Paragraph` with `text: ""`, `Provenance::Warning { note: "page<i> empty (scanned candidate)" }`.
- `title` precedence: `/Info/Title` from `lopdf` (when non-empty) → filename without extension.
- `lang = Lang("und")` (PDFs rarely declare; lingua detection over the body could be a future enhancement).
- `metadata.user["pdf"] = { "page_count": n, "producer": "...", "creator": "..." }` from `/Info`.
- `metadata.source_type = SourceType::Paper`; `trust_level = TrustLevel::Primary`.
- `provenance` events: `Discovered`, `Parsed` (per page text or warning).
- `block_id` per design §4.2 with `block_kind = "paragraph"`, `heading_path = []`, `ordinal = page - 1`, `source_span = SourceSpan::Page { page }`.
- Streaming: read PDF in memory only once; do not load `pdf-extract` per page (that re-parses N times).
- Failure modes:
  - File not a PDF / corrupt header → `anyhow::Error`.
  - Encrypted PDF → `anyhow::Error` with hint to remove encryption (no decryption attempt in v1).
- Determinism: identical bytes → identical doc/block IDs and text.

## Storage / wire effects

- None directly.

## Test plan

| kind | description | fixture / data |
|------|-------------|----------------|
| unit | 3-page PDF produces 3 paragraph blocks with `SourceSpan::Page { page: 1..=3 }` | `fixtures/pdf/three-page-en.pdf` |
| unit | PDF with image-only page 2 (no text) emits warning + empty text for page 2 | `fixtures/pdf/scanned-mixed.pdf` |
| unit | encrypted PDF returns error with helpful hint | `fixtures/pdf/encrypted.pdf` |
| unit | corrupt header PDF returns error | `fixtures/pdf/corrupt.pdf` |
| unit | `metadata.user.pdf.page_count` matches actual count | inline |
| unit | Korean text PDF preserved (CID mapping permitting) | `fixtures/pdf/korean.pdf` |
| determinism | identical bytes → identical CanonicalDocument JSON across two runs | inline |
| snapshot | CanonicalDocument JSON for fixture stable | `fixtures/pdf/three-page-en.pdf` |

All tests under `cargo test -p kb-parse-pdf`.

## Definition of Done

- [ ] `cargo check -p kb-parse-pdf` passes
- [ ] `cargo test -p kb-parse-pdf` passes
- [ ] No OCR / LLM code present
- [ ] No imports outside Allowed dependencies
- [ ] PR links design §3.4 SourceSpan::Page, §9.2

## Out of scope

- OCR for scanned PDFs (separate future task; reuses p6-2 OCR adapter).
- Layout reconstruction (multi-column reading order, tables).
- Math rendering / formula detection.
- Form-field extraction.
- Bookmark / outline ingestion (could become heading_path later — note for P+).

## Risks / notes

- `pdf-extract` text quality varies wildly. For broken-glyph PDFs, the text may be unicode noise; downstream embedding still works but quality is poor. Mark such pages with a confidence-style warning when feasible.
- Some PDFs have layered text (selectable text + scanned image overlay). v1 captures the selectable text only.
- For very large PDFs (> 1k pages), memory usage may spike. Document a soft limit (`config.pdf.max_pages` default 5000) and refuse beyond it.
