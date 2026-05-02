---
phase: P6
component: kebab-parse-image (image extractor + EXIF)
task_id: p6-1
title: "Image Extractor producing single-block CanonicalDocument + EXIF metadata"
status: planned
depends_on: [p0-1, p1-6]
unblocks: [p6-2, p6-3]
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§3.4 Block::ImageRef + ImageRefBlock, §3.7a OcrText/ModelCaption stubs, §9.1 image extraction policy, §9 versioning]
---

# p6-1 — Image extractor (EXIF + structure)

## Goal

Implement `Extractor` for `MediaType::Image(_)` that produces a `CanonicalDocument` whose body is exactly one `ImageRefBlock`. EXIF is captured into `metadata.user.exif`. OCR and caption are intentionally left `None`; later tasks (p6-2, p6-3) populate them.

## Why now / why this size

Establishes the image-as-document contract and decouples extraction (asset → ImageRefBlock) from analysis (OCR / caption). Keeps the multimodal merge surface small.

## Allowed dependencies

- `kebab-core`
- `kebab-config`
- `image = "0.25"` (decoding for size + format detect)
- `kamadak-exif` for EXIF
- `serde`, `serde_json`
- `time`
- `tracing`
- `thiserror`

## Forbidden dependencies

- `kebab-source-fs`, `kebab-parse-md`, `kebab-normalize`, `kebab-chunk`, `kebab-store-*`, `kebab-embed*`, `kebab-search`, `kebab-llm*`, `kebab-rag`, `kebab-tui`, `kebab-desktop`, OCR libs, LLM libs

## Inputs

| input | type | source |
|-------|------|--------|
| `RawAsset` | `kebab_core::RawAsset` | from `kebab-source-fs` |
| image bytes | `&[u8]` | filesystem |
| `parser_version` | `kebab_core::ParserVersion` | constant in this crate (`"image-meta-v1"`) |

## Outputs

| output | type | downstream |
|--------|------|------------|
| `CanonicalDocument` | `kebab_core::CanonicalDocument` | `kebab-chunk` (image-region chunker) → `kebab-store-sqlite` |

## Public surface (signatures only — no new types)

```rust
pub struct ImageExtractor;

impl kebab_core::Extractor for ImageExtractor {
    fn supports(&self, m: &kebab_core::MediaType) -> bool { matches!(m, kebab_core::MediaType::Image(_)) }
    fn parser_version(&self) -> kebab_core::ParserVersion { kebab_core::ParserVersion("image-meta-v1".into()) }
    fn extract(&self, ctx: &kebab_core::ExtractContext, bytes: &[u8]) -> anyhow::Result<kebab_core::CanonicalDocument>;
}
```

## Behavior contract

- One asset → one document. `title` = filename without extension; `lang = Lang("und")`.
- `blocks` contains exactly one entry: `Block::ImageRef(ImageRefBlock { common, asset_id: Some(asset.asset_id), src: workspace_path, alt: filename, ocr: None, caption: None })`.
- `common.source_span` = `SourceSpan::Region { x:0, y:0, w: width, h: height }` covering the entire image (width/height obtained from `image::ImageReader::without_guessed_format().with_guessed_format()?.into_dimensions()`).
- `metadata.source_type = SourceType::Reference` (per design enum); `trust_level = TrustLevel::Primary`; `tags`/`aliases` empty.
- `metadata.user["exif"]` = JSON object with whitelisted EXIF tags (DateTimeOriginal, GPS lat/lon, Make, Model, Orientation, Software). Missing tags omitted.
- `metadata.user["dimensions"] = { "w": <u32>, "h": <u32>, "format": "<png|jpeg|...>" }`.
- `provenance` includes `Discovered`, `Parsed` events (no Normalized — ID assignment happens here directly per §3.4 stub from p1-4 logic, OR pipe through `kebab-normalize` if available; this task's choice: emit a fully formed CanonicalDocument with deterministic IDs by calling `kebab_core::id_for_doc` and `kebab_core::id_for_block` directly).
- Failure modes:
  - Truncated/corrupt image → still emits a CanonicalDocument with `dimensions = null`, EXIF empty, `Provenance` warning event with the decoder error message.
  - Unsupported format → `anyhow::Error` (caller skips).
- Determinism: identical bytes + identical parser_version → identical `doc_id` and `block_id`.

## Storage / wire effects

- None directly (the caller persists via `kebab-store-sqlite`).

## Test plan

| kind | description | fixture / data |
|------|-------------|----------------|
| unit | PNG decode produces correct dimensions in `metadata.user.dimensions` | `fixtures/image/red-100x50.png` |
| unit | JPEG with EXIF GPS captured into `metadata.user.exif` | `fixtures/image/exif-with-gps.jpg` |
| unit | image with no EXIF produces `metadata.user.exif = {}` | `fixtures/image/no-exif.png` |
| unit | corrupt image: warning provenance, no panic | `fixtures/image/corrupt.png` |
| determinism | identical bytes → identical `doc_id`, `block_id` across two runs | inline |
| snapshot | `CanonicalDocument` JSON stable for fixture | `fixtures/image/red-100x50.png` |

All tests under `cargo test -p kebab-parse-image`.

## Definition of Done

- [ ] `cargo check -p kebab-parse-image` passes
- [ ] `cargo test -p kebab-parse-image` passes
- [ ] No OCR/caption/embedding code present
- [ ] No imports outside Allowed dependencies
- [ ] PR links design §3.4, §9.1

## Out of scope

- OCR text (p6-2).
- Captioning (p6-3).
- CLIP / visual embedding (P+).
- HEIC / RAW formats (out of scope; record as Other and accept failure for v1).

## Risks / notes

- `image` crate doesn't decode HEIC; document and accept skip. Apple Vision sidecar (P+) can fill this gap.
- EXIF whitelist keeps PII surface small (no thumbnails, no maker notes). Document the list in the spec section.
- Cap decode dimensions to ~16k×16k; oversized → warning + null dimensions instead of attempted decode.
