---
phase: P6
component: kb-parse-image (caption adapter)
task_id: p6-3
title: "ModelCaption adapter (LanguageModel-driven, feature-gated)"
status: planned
depends_on: [p6-1, p4-2]
unblocks: []
contract_source: ../../docs/superpowers/specs/2026-04-27-kb-final-form-design.md
contract_sections: [§3.4 ImageRefBlock.caption, §3.7a ModelCaption, §9.1 caption (model-generated, low trust)]
---

# p6-3 — Caption adapter

## Goal

Optionally populate `ImageRefBlock.caption` with `ModelCaption { text, model, model_version }` produced by a vision-capable LM (e.g., `qwen2.5-vl:7b` via Ollama). Feature-gated; default OFF.

## Why now / why this size

Captioning closes the multimodal loop. Strict separation from OCR keeps trust levels distinct: captions are generated, OCR is observed. Adapter is small — single trait method + one prompt.

## Allowed dependencies

- `kb-core`
- `kb-config`
- `kb-parse-image`
- `kb-llm` (LanguageModel trait)
- `base64`
- `serde`, `serde_json`
- `image` (resize for prompt cost control)
- `tracing`
- `thiserror`

## Forbidden dependencies

- `kb-source-fs`, `kb-parse-md`, `kb-normalize`, `kb-chunk`, `kb-store-*`, `kb-embed*`, `kb-search`, `kb-rag`, `kb-llm-local` (only via trait), `kb-tui`, `kb-desktop`

## Inputs

| input | type | source |
|-------|------|--------|
| image bytes | `&[u8]` | extractor |
| `dyn LanguageModel` (vision-capable) | runtime | injected |
| `kb-config.image.caption` | `{ enabled, max_pixels, prompt_template_version }` | runtime |

## Outputs

| output | type | downstream |
|--------|------|------------|
| `ModelCaption` | `kb_core::ModelCaption` | merged into `ImageRefBlock.caption` |

## Public surface (signatures only — no new types)

```rust
pub fn caption_image(
    llm: &dyn kb_core::LanguageModel,
    image_bytes: &[u8],
    cfg: &kb_config::Config,
) -> anyhow::Result<kb_core::ModelCaption>;

pub fn apply_caption(
    llm: &dyn kb_core::LanguageModel,
    image_bytes: &[u8],
    block: &mut kb_core::ImageRefBlock,
    cfg: &kb_config::Config,
) -> anyhow::Result<()>;
```

## Behavior contract

- Feature gate: if `config.image.caption.enabled = false` (default), `apply_caption` is a no-op (returns `Ok(())` without invoking LM).
- Pre-process: downscale image to `config.image.caption.max_pixels` (default 768×768 long edge) preserving aspect; encode as PNG.
- Build prompt:
  - `system = "이미지를 한 문장으로 객관적으로 설명한다. 추측은 피하고, 보이는 것만 적는다."`
  - `user` = `[image_base64]\n\n위 이미지를 한국어로 한 문장으로 설명하라.` (if `lang` hint == "ko") or English variant otherwise.
  - The base64 wrapper assumes the LM adapter routes vision inputs via Ollama's `images: [base64]` field (this is provider-specific; the adapter is responsible for rendering the prompt to wire). For non-vision LMs, return an error and skip.
- Call `llm.generate_stream(GenerateRequest { system, user, stop: vec!["\n\n"], max_tokens: 96, temperature: 0.0, seed: Some(0) })`. Collect tokens until `Done`.
- `ModelCaption { text: collected, model: llm.model_ref().id, model_version: llm.model_ref().provider }` (use provider as a coarse "version" proxy; if a vision model exposes a stable revision, prefer that).
- `apply_caption` sets `block.caption = Some(...)` and appends `Provenance::CaptionApplied` event.
- Trust: caption is **model-generated** and labeled `trust_level = TrustLevel::Generated` if the caller propagates trust into chunk-level UI; this task only emits the `ModelCaption`.
- Failure modes:
  - LM error → return `anyhow::Error`; caller may decide to skip (do not fail the entire ingest).
  - Empty LM output → still set `block.caption = Some(ModelCaption { text: "" })` so downstream code can distinguish "captioning attempted, no result" from "captioning never attempted".
- Determinism: `temperature=0` + `seed=0`. Tests use `MockLanguageModel` to assert deterministic captions.

## Storage / wire effects

- None directly. Caller persists via `kb-store-sqlite`.

## Test plan

| kind | description | fixture / data |
|------|-------------|----------------|
| unit | feature disabled → `apply_caption` no-op | inline (config.enabled = false) |
| unit | mock LM emits "사진 한 장" → `block.caption.text = "사진 한 장"` | inline |
| unit | mock LM emits empty token stream → `block.caption = Some(ModelCaption { text: "" })` | inline |
| unit | Korean lang hint produces Korean prompt; English hint → English prompt | inline |
| unit | downscale honors `max_pixels` (resulting bytes < some threshold) | fixture large image |
| determinism | identical input + temperature=0 + seed=0 → identical caption (mock) | inline |

All tests under `cargo test -p kb-parse-image caption` with mock LM only.

## Definition of Done

- [ ] `cargo check -p kb-parse-image --features caption` passes
- [ ] `cargo test -p kb-parse-image caption` passes
- [ ] No imports outside Allowed dependencies
- [ ] Feature default OFF; only on when user opts in via config
- [ ] PR links design §3.4 ImageRefBlock.caption, §9.1

## Out of scope

- Multimodal RAG that uses caption text in answer (P+).
- CLIP / image embedding for cross-modal search (P+).
- Caption translation (P+).

## Risks / notes

- Vision LMs hallucinate. The system prompt explicitly forbids guessing, but expect false captions; UI and RAG must always label captions as model-generated.
- Ollama `qwen2.5-vl` accepts base64 images via `images:[]` — this is provider-specific; documenting the wire shape in the spec keeps adapter swaps cheap.
- Large images bloat prompt costs; cap aggressively (768×768 long edge default).
