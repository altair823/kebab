---
phase: P6
component: kebab-parse-image (OCR adapter)
task_id: p6-2
title: "OcrEngine trait + Tesseract adapter (Apple Vision feature-gated)"
status: completed
depends_on: [p6-1]
unblocks: [p6-3]
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§3.4 ImageRefBlock.ocr, §3.7a OcrText/OcrRegion, §9.1 OCR vs caption provenance]
---

# p6-2 — OCR adapter

## Goal

Define `OcrEngine` trait + a Tesseract-backed default implementation. Populate `ImageRefBlock.ocr` with `OcrText { joined, regions, engine, engine_version }`. Provide an `apple-vision` feature gate that switches to a sidecar binary on macOS.

## Why now / why this size

Strict separation of OCR (observed text) from caption (model-generated). Confining engine choice to a single trait + adapter lets us swap to Apple Vision or PaddleOCR without touching the extractor or chunker.

## Allowed dependencies

- `kebab-core`
- `kebab-config`
- `kebab-parse-image` (consumes its types)
- `tesseract = "0.13"` (feature `tesseract`, default ON)
- For feature `apple-vision`: `std::process::Command` only (sidecar binary, not a Rust dep)
- `serde`, `serde_json`
- `image`
- `tracing`
- `thiserror`

## Forbidden dependencies

- `kebab-source-fs`, `kebab-parse-md`, `kebab-normalize`, `kebab-chunk`, `kebab-store-*`, `kebab-embed*`, `kebab-search`, `kebab-llm*`, `kebab-rag`, `kebab-tui`, `kebab-desktop`

## Inputs

| input | type | source |
|-------|------|--------|
| image bytes | `&[u8]` | from extractor |
| optional language hint | `kebab_core::Lang` | metadata |
| `kebab-config` OCR settings | engine name, languages | runtime |

## Outputs

| output | type | downstream |
|--------|------|------------|
| `OcrText` | `kebab_core::OcrText` | merged into `ImageRefBlock.ocr` |

## Public surface (signatures only — no new types)

```rust
pub trait OcrEngine: Send + Sync {
    fn engine_name(&self) -> &'static str;
    fn engine_version(&self) -> String;
    fn recognize(&self, image_bytes: &[u8], lang_hint: Option<&kebab_core::Lang>) -> anyhow::Result<kebab_core::OcrText>;
}

pub struct TesseractOcr { /* internal: lazy api handle */ }
impl TesseractOcr { pub fn new(config: &kebab_config::Config) -> anyhow::Result<Self>; }
impl OcrEngine for TesseractOcr { /* per trait */ }

#[cfg(feature = "apple-vision")]
pub struct AppleVisionOcr { /* sidecar path */ }
#[cfg(feature = "apple-vision")]
impl OcrEngine for AppleVisionOcr { /* per trait */ }

pub fn apply_ocr(
    engine: &dyn OcrEngine,
    image_bytes: &[u8],
    block: &mut kebab_core::ImageRefBlock,
    lang_hint: Option<&kebab_core::Lang>,
) -> anyhow::Result<()>;
```

## Behavior contract

- Tesseract:
  - Languages from `config.ocr.languages` (default `["eng", "kor"]`).
  - Recognition produces `OcrRegion { bbox: (x, y, w, h), text, confidence }` for each "word" or "line" (configurable; default "line").
  - Drop regions with `confidence < config.ocr.min_confidence` (default 60.0). If all dropped, return `OcrText { joined: "", regions: vec![], engine, engine_version }`.
  - `joined` = `regions.iter().map(|r| r.text).join(" ")` (no smart layout reconstruction in v1).
  - `engine = "tesseract"`, `engine_version = <tesseract version string>`. The `tesseract` crate (0.13+) does NOT expose a stable Rust `version()` accessor. Use one of: (a) call libtesseract's `TessVersion()` via the bundled FFI surface, OR (b) at adapter construction, shell-out `tesseract --version` once and cache the parsed `"5.3.4"`-style string. Both are deterministic for a fixed install. Pin the chosen approach in the implementation PR.
- Apple Vision sidecar (feature `apple-vision`):
  - Spawn a small Swift binary `kebab-vision-ocr` (path from `config.ocr.apple_vision_binary`) feeding the image via stdin and reading JSON `{ regions: [{x,y,w,h,text,confidence}, ...] }` from stdout.
  - Same threshold and `joined` rules as Tesseract. `engine = "apple-vision"`, `engine_version = sidecar's --version`.
  - This subagent task does NOT write the Swift sidecar; it only wires the Rust side. Document the expected sidecar interface in `docs/spec/sidecar-vision.md` (separate doc spec stub, optional).
- `apply_ocr` calls `engine.recognize`, sets `block.ocr = Some(text)`, and appends a `Provenance::OcrApplied` event in the caller's CanonicalDocument (caller responsibility — this task exposes a helper).
- Streaming / large images: cap decoded image size at 8192×8192 before passing to OCR; downscale with `image::imageops::resize` if larger.
- Trust: `OcrText` is **observed text** (high trust). Captions (`ModelCaption`) are NOT generated here.
- Determinism: Tesseract is deterministic for a fixed input + fixed page-segmentation mode; apply_ocr asserts this by calling twice in dev tests. Apple Vision is also deterministic in practice but may vary across macOS versions; document this and accept.

## Storage / wire effects

- None.

## Test plan

| kind | description | fixture / data |
|------|-------------|----------------|
| unit | Tesseract recognizes English on `fixtures/image/hello-world.png` (joined contains "hello world") | fixture |
| unit | confidence threshold drops noise regions | fixture with low-quality text |
| unit | Korean text recognized when `kor` language enabled | `fixtures/image/안녕.png` |
| unit | empty result returns `OcrText { joined: "", regions: [], .. }` not error | `fixtures/image/no-text.png` |
| unit | `apply_ocr` mutates block.ocr from None → Some | inline |
| determinism | two runs of recognize on same input → identical OcrText | fixture |
| `#[cfg(feature = "apple-vision")]` smoke | sidecar invocation captured (mock binary echoes fixed JSON) | inline mock |

All tests under `cargo test -p kebab-parse-image ocr`. Tesseract install required on CI host.

## Definition of Done

- [ ] `cargo check -p kebab-parse-image --features tesseract` passes
- [ ] `cargo test -p kebab-parse-image ocr` passes
- [ ] `apple-vision` feature compiles on macOS and gracefully no-ops on Linux
- [ ] No imports outside Allowed dependencies
- [ ] PR links design §3.4, §3.7a, §9.1

## Out of scope

- Caption (p6-3).
- Visual embedding (P+).
- Layout-aware reading order (P+).
- PaddleOCR / EasyOCR adapters.

## Risks / notes

- Tesseract performance varies wildly with image quality; document `min_confidence` and default page-segmentation mode.
- Apple Vision sidecar requires code signing for distribution; for v1 dev builds, accept unsigned binary from `~/.local/bin/kebab-vision-ocr`.
- Large image downscale loses small-text recognition; expose `config.ocr.max_pixels` so power users can tune.
