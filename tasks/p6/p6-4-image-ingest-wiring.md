---
phase: P6
component: kebab-app (image ingest dispatch + chunking)
task_id: p6-4
title: "Wire ImageExtractor + OCR + caption into kebab-app::ingest end-to-end"
status: planned
depends_on: [p6-1, p6-2, p6-3, p1-6, p3-5]
unblocks: []
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§3.4 ImageRefBlock, §6.1 ingest pipeline, §7.2 Extractor/Chunker traits, §9.1 image extraction policy]
---

# p6-4 — Image ingest wiring (kebab-app + chunker)

## Goal

Make `kebab ingest` end-to-end functional for image assets. The library-level pipeline (`ImageExtractor`, `OllamaVisionOcr`, `apply_caption`) is complete and tested in isolation — this task connects the wires from `kebab-source-fs` (which already classifies `.png` / `.jpg` / `.webp` / `.gif` / `.tiff` as `MediaType::Image(_)`) through `kebab-app::ingest_with_config` to `kebab-store-sqlite` + `kebab-store-vector`, so a user running `kebab ingest` against a workspace containing diagrams / screenshots / camera photos sees those assets indexed and searchable.

## Why now / why this size

P6-1 / P6-2 / P6-3 each shipped a focused library and a passing test suite, but all three deliver value only after `kebab-app::ingest` learns how to dispatch on `MediaType::Image(_)`. The wiring is small (one new dispatch arm, one chunking branch, one LM-construction call site) but materially user-facing — without it, the entire P6 phase is invisible from the CLI. Pulling this into its own task keeps the P6-1/2/3 specs frozen as written while letting the integration evolve under its own contract.

## Allowed dependencies

`kebab-app` 의 현재 Cargo.toml 그대로의 surface — 본 task 는 그 위에 `kebab-parse-image` 한 줄을 추가합니다.

- `kebab-core`
- `kebab-config`
- `kebab-source-fs`
- `kebab-parse-md`, `kebab-parse-types`
- `kebab-normalize`
- `kebab-chunk` (image-document branch in `md-heading-v1` — this task extends it)
- `kebab-store-sqlite`, `kebab-store-vector`
- `kebab-search`
- `kebab-embed`, `kebab-embed-local`
- `kebab-llm`, `kebab-llm-local` (constructs `OllamaLanguageModel` for caption)
- `kebab-rag`
- **`kebab-parse-image` (NEW — added by this task)**
- `anyhow`, `serde_json`, `tracing`

## Forbidden dependencies

- `kebab-tui`, `kebab-desktop` (P9 미시작 — UI crate 가 ingest 를 호출하면 layering 위반)
- `kebab-eval` (cycle 위험 — eval 이 ingest 를 호출하므로 그 반대는 금지)
- 본 task 가 신설하는 자체 image extractor / OCR / caption 비즈니스 로직 — 모두 `kebab-parse-image` 에 이미 존재. `kebab-app` 안에 image-specific 로직 추가 금지 (얇은 dispatch + glue 만 허용).

## Inputs

| input | type | source |
|-------|------|--------|
| workspace assets | `RawAsset` stream | `kebab-source-fs::SourceConnector::scan` (already classifies image types) |
| image bytes | `&[u8]` | filesystem read in `kebab-app` |
| `Config` | `kebab_config::Config` | CLI `--config` flag → `Config::load` |
| OCR config | `config.image.ocr` | P6-2 |
| Caption config | `config.image.caption` | P6-3 |

## Outputs

| output | type | downstream |
|--------|------|------------|
| `CanonicalDocument` per image | written via `DocumentStore::put_document` | `kebab-store-sqlite` |
| One synthesized chunk per image | `Vec<Chunk>` (length 1) | `kebab-store-sqlite::put_chunks` + `kebab-store-vector::upsert` |
| Updated `IngestReport` counters | `scanned / new / updated / skipped / errors` | wire output (`ingest_report.v1`) |

## Public surface

No new public types. The wiring exists inside `kebab-app::ingest_with_config` (and its private helpers). `kebab-chunk` gains one image-only branch in `md_heading_v1`:

```rust
// crates/kebab-chunk/src/md_heading_v1.rs (additions only — sketch)
fn chunk(&self, doc: &CanonicalDocument, policy: &ChunkPolicy) -> Result<Vec<Chunk>> {
    if is_image_only_document(doc) {
        return Ok(vec![image_chunk(doc, policy)?]);
    }
    // ... existing markdown heading logic untouched ...
}

/// Returns true iff `doc` is an image-only document (single `ImageRef`
/// block). P6-1's `ImageExtractor` already guarantees this shape today
/// — the predicate exists as a defensive guard against (a) a future
/// task that introduces multi-block image documents, and (b) accidental
/// route-through of a `[Block::Heading, Block::ImageRef, ...]` shape
/// that would look image-ish but should still flow through the
/// markdown chunker.
fn is_image_only_document(doc: &CanonicalDocument) -> bool {
    doc.blocks.len() == 1
        && matches!(doc.blocks.first(), Some(Block::ImageRef(_)))
}

fn image_chunk(doc: &CanonicalDocument, policy: &ChunkPolicy) -> Result<Chunk> {
    let block = match &doc.blocks[0] {
        Block::ImageRef(b) => b,
        _ => unreachable!("guarded by is_image_only_document"),
    };
    let text = compose_image_chunk_text(block);
    // chunk_id derived from doc_id + chunker_version + [block_id] + policy_hash
    // (existing §4.2 recipe — no new ID kind).
    Ok(Chunk { /* ... */ })
}
```

`compose_image_chunk_text` is the canonical place where the (β) plain-concatenation policy lives:

```rust
fn compose_image_chunk_text(block: &ImageRefBlock) -> String {
    let alt = if block.alt.is_empty() {
        // alt should never be empty at this stage because P6-1 falls
        // back to the filename when it is, but the chunker stays
        // defensive — an empty first line would degrade lexical
        // search hits on filenames.
        block.src.rsplit('/').next().unwrap_or("[image]").to_string()
    } else {
        block.alt.clone()
    };
    let ocr = block.ocr.as_ref().map(|o| o.joined.as_str()).unwrap_or("");
    let cap = block.caption.as_ref().map(|c| c.text.as_str()).unwrap_or("");
    [alt, ocr.to_string(), cap.to_string()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}
```

## Behavior contract

### ingest dispatch (kebab-app)

- `kebab-app::ingest_with_config` reads the asset stream from `kebab-source-fs`. For each `RawAsset`:
  - `MediaType::Markdown` → existing markdown extractor path (unchanged).
  - `MediaType::Image(_)` → new image branch (this task).
  - `MediaType::Pdf | MediaType::Audio(_) | MediaType::Other(_)` → `skipped += 1` (existing behaviour).
- Image branch:
  1. Read bytes via `kebab-source-fs` (existing helper).
  2. Build a `kebab_core::ExtractContext { asset, workspace_root, config: &ExtractConfig::default() }`.
  3. Call `ImageExtractor::new().extract(&ctx, &bytes)`. Failure (`Err(_)`) → `errors += 1`, log, continue to next asset (do not abort the whole ingest).
  4. Take a mutable borrow of `doc.blocks[0]` (must be `Block::ImageRef`); take a mutable borrow of `doc.provenance.events`.
  5. If `config.image.ocr.enabled`:
     - Build `OllamaVisionOcr::new(&cfg)`.
     - Call `apply_ocr(&engine, &bytes, block, lang_hint, &mut events)`.
     - Failure → log a `tracing::warn!`, append `ProvenanceKind::Warning` event with the error message via the helper, continue.
     - `block.ocr` stays `None` on failure (P6-2 contract).
  6. If `config.image.caption.enabled`:
     - Build the LM **once per ingest session** (not per asset) — see "LM construction" below.
     - Call `apply_caption(&*llm, &bytes, block, lang_hint, &cfg, &mut events)`.
     - Failure → same lenient policy as OCR: warn, log, continue. `block.caption` stays `None`.
  7. Pass the (possibly partially-populated) `CanonicalDocument` to the existing chunker → embedder → store path, identical to markdown.

### Parallelism

- The image branch shares the existing markdown worker pool — one asset dispatch unit (markdown OR image) per worker — so `config.indexing.max_parallel_extractors` keeps its current meaning. The current `kebab-app` ingest is sequential per-asset (single worker irrespective of the knob value); image branch adds zero new concurrency. A future P+ task may parallelise both branches, at which point the OCR / caption HTTP calls naturally become the throughput ceiling (one in-flight request per worker — `reqwest::blocking::Client` is shared but each call blocks its worker thread until response).
- Implication for sizing: a 5000-asset ingest with OCR enabled runs as roughly `5000 × (per-asset OCR latency)` end-to-end. With `gemma4:e4b` at ~3-5s per call this is the 4-7 hour range the brainstorming flagged. Books-as-PDF route bypasses this entirely.

### Lang hint

- `lang_hint: Option<&Lang>` passed to `apply_ocr` / `apply_caption` reads from `doc.lang` (set to `Lang("und")` by P6-1).
- `kebab-parse-image` already special-cases `"und"` so the prompt does not embed a misleading hint.

### LM / OCR engine construction

Both the caption LM and the OCR adapter wrap `reqwest::blocking::Client`, whose internal `Arc` makes a single instance cheap to share across all assets. Both are constructed **once per ingest invocation**, before the asset loop, gated on the matching `enabled` flag.

- **Caption** — when `config.image.caption.enabled = true`, build `OllamaLanguageModel::new(&cfg)` once. Stored as `Box<dyn LanguageModel>` (or trait object behind `&`) and passed to every `apply_caption` call.
- **OCR** — when `config.image.ocr.enabled = true`, build `OllamaVisionOcr::new(&cfg)` once. Same `Arc`-share property; passed by `&` to every `apply_ocr` call.
- Endpoints — `OllamaVisionOcr::new` already falls back to `models.llm.endpoint` when `image.ocr.endpoint` is `None`, so a single host typically serves both LLM and OCR. The two adapters can therefore share an Ollama host or run against separate hosts independently.
- **Construction failure** (e.g. invalid endpoint string, empty model id) → ingest aborts with the constructor's error before any asset is scanned. Never silently disables OCR / caption.
- Per-asset cost — only the HTTP call to Ollama. Adapter struct is reused.

### Chunking (kebab-chunk md-heading-v1)

- The chunker gains an image-only branch as sketched above. The branch:
  - Returns exactly one `Chunk` per image document.
  - `chunk.text` = (β) plain concatenation of `[alt, ocr_joined, caption_text]` joined by `\n\n`, dropping empty parts.
  - `chunk.block_ids = vec![block.common.block_id.clone()]`.
  - `chunk.heading_path = vec![]` (image documents have no heading hierarchy).
  - `chunk.source_spans = vec![block.common.source_span.clone()]` — `Vec<SourceSpan>` per `kebab_core::Chunk` definition; image branch contributes one element holding the `Region { x, y, w, h }` from P6-1.
  - `chunk.token_estimate` follows the existing `md-heading-v1` token-count convention (whitespace-segmented words clamped to `policy.target_tokens`).
  - `chunk.policy_hash` is the existing `ChunkPolicy` hex digest the chunker already computes for markdown; image branch reuses the same value to keep policy edits invalidating image chunks alongside markdown chunks.
- Determinism: the existing `id_for_chunk(doc_id, chunker_version, &[block_id], policy_hash)` recipe applies unchanged.
- Oversized text: an image whose `ocr.joined` exceeds `policy.target_tokens` produces an oversized chunk. Acceptable in v1 — the user-facing scenario (diagrams / screenshots / camera photos) typically yields ≤ 1 page of OCR. Books are routed through P7 PDF instead (see "Out of scope"). A `tracing::warn!` fires when this happens so a future P+ task can quantify how often the boundary is hit.

### `enabled = false` for both

- When `config.image.ocr.enabled = false` AND `config.image.caption.enabled = false`, the image is still extracted, stored, and chunked — the user gets EXIF + dimensions + filename indexed, with empty OCR / caption.
- The synthesized chunk text falls back to just the filename. Lexical search on filenames still works; vector search produces a best-effort embedding from a one-line input.

### Failure semantics summary

| failure | counter | doc stored? | provenance |
|---|---|---|---|
| `ImageExtractor::extract` Err (decode fail / unrecognised) | `errors+=1` | no | n/a |
| `MediaType::Image(_)` but `supports()` returns false (won't happen with current trait — defensive) | `skipped+=1` | no | n/a |
| `apply_ocr` Err | unchanged | yes (block.ocr = None) | `ProvenanceKind::Warning`, agent `kb-app` |
| `apply_caption` Err | unchanged | yes (block.caption = None) | `ProvenanceKind::Warning`, agent `kb-app` |
| HEIC / RAW → `MediaType::Other(_)` | `skipped+=1` (existing) | no | n/a |

## Storage / wire effects

- `kebab-store-sqlite::documents` table gains rows whose `parser_version = "image-meta-v1"`.
- `kebab-store-sqlite::blocks` table gains rows of `block_kind = "imageref"`.
- `kebab-store-sqlite::chunks` table gains rows whose `chunker_version = "md-heading-v1"` AND whose `block_ids` reference a single `imageref` block.
- `kebab-store-vector::chunk_embeddings_<model>_<dim>` gains one vector per image chunk.
- `IngestReport` (wire `ingest_report.v1`) counters update naturally: `scanned` includes images, `new` / `updated` track image docs, `errors` counts decode failures, `skipped` counts unsupported formats.
- No new wire schemas or `kebab-core` types.

## Test plan

| kind | description | fixture / data |
|------|-------------|----------------|
| integration | TempDir KB + 1 PNG (`hello-world.png` with text) + `image.ocr.enabled = true` + `image.caption.enabled = false` (mock LM unused) → `kebab-app::ingest` produces 1 doc + 1 chunk; chunk text contains the filename + OCR text | `kebab-app/tests/image_pipeline.rs` (new), wiremock for OCR Ollama call |
| integration | Same fixture but `caption.enabled = true` with mock LM returning `"a red square with text"` → chunk text contains alt + OCR + caption joined by `\n\n` | wiremock for Ollama (both `/api/generate` calls) |
| integration | Determinism: ingest the same PNG twice → identical `doc_id`, `chunk_id` (P1 idempotency contract holds) | inline |
| integration | OCR Ollama returns 503 → asset still indexed; `block.ocr = None`; provenance has Warning event; `errors` counter NOT incremented; ingest returns Ok | wiremock |
| integration | Caption Ollama returns 503 → asset still indexed; `block.caption = None`; provenance Warning; `errors` not incremented | wiremock |
| integration | `image.ocr.enabled = false` AND `image.caption.enabled = false` → image still indexed; chunk text = filename only | inline |
| integration | Hybrid search across mixed corpus (1 markdown + 1 PNG) returns image chunk for an OCR-text query | inline (real `multilingual-e5-small` embedding) |
| integration | `kebab inspect doc <image_doc_id>` returns the image `CanonicalDocument` with `block.ocr` / `block.caption` populated | inline |
| unit | `kebab-chunk::md_heading_v1::is_image_only_document` returns true for `[ImageRef]`, false for `[Heading, ImageRef]` (image embedded in markdown — currently a P+ case but the predicate must not misfire) | unit |
| unit | `compose_image_chunk_text` drops empty parts: alt-only, alt+ocr, alt+caption, alt+ocr+caption all formatted correctly | unit |
| smoke | Update `docs/SMOKE.md` so the runbook ingests at least one image fixture and verifies search-by-OCR-text works | docs change |

The opt-in real-Ollama integration test from P6-2 / P6-3 stays inside `kebab-parse-image`; this task's integration tests use wiremock so `cargo test --workspace` stays hermetic.

## Definition of Done

### Spec PR (this PR — `spec/p6-4-image-ingest-wiring`)

- [x] `tasks/p6/p6-4-image-ingest-wiring.md` 작성 + self-review (placeholder / 모순 / 모호성 / scope)
- [x] `tasks/INDEX.md` "P6 — 4 components" 반영
- [x] PR 본문에 design §3.4, §6.1, §9.1 링크
- [x] brainstorming 의 모든 결정 (옵션 A 청킹 / β 청크 텍스트 / Lenient 실패 정책 / LM 1회 빌드 / 책 P7 이관 / P6-5 미시작) 본문 반영

### Implementation PR (follow-up — `feat/p6-4-image-ingest-wiring`)

- [ ] `cargo check --workspace` passes
- [ ] `cargo test --workspace --no-fail-fast -j 1` passes (all new integration tests green)
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` passes
- [ ] `kebab ingest` against a TempDir KB containing 1 markdown + 1 PNG produces `scanned 2 / new 2 / errors 0`
- [ ] `kebab search --mode lexical "<OCR text>"` returns the image chunk
- [ ] `kebab inspect doc <image_doc_id>` shows non-empty `block.ocr` / `block.caption`
- [ ] `docs/SMOKE.md` includes an image-fixture step

## Out of scope

- **Books / scanned PDFs** — routed through P7 PDF pipeline. P7's PDF text extractor handles text-embed PDFs natively; scanned PDFs are P7's internal concern (page render → OCR call to the same `kebab-parse-image::OllamaVisionOcr` adapter).
- **Image-region chunker** — current `OllamaVisionOcr` returns a single full-image region, so per-region chunking has no signal. When a region-aware engine (Tesseract / Apple Vision sidecar) lands, a separate `image-region-chunker` task can split on `OcrText.regions[]`.
- **Long-OCR splitting** — image documents whose OCR exceeds `target_tokens` produce oversized chunks. Acceptable for diagrams / screenshots / photos (the v1 user scenario per the brainstorming with the user). Books deliberately use the PDF path instead.
- **Retry mechanism for partial OCR / caption failures** — current escape hatch is "delete the asset, re-ingest". A `kebab ingest --retry-image-analysis` flag is a P+ enhancement once operational data shows it's needed.
- **Parallelism beyond `config.indexing.max_parallel_extractors`** — the existing knob applies. Per-image OCR-bound parallelism (multiple in-flight Ollama calls) is a P+ scale-hardening task that has been explicitly de-scoped because the user routes books through PDF instead of image.
- **Progress reporting** (`kebab ingest --progress`) — same reasoning as parallelism.
- **W3C Media Fragments citation form** (`path#xywh=...`) — `Citation` already carries `Region` source span; fragment URI rendering can be a wire-only follow-up when downstream UI needs it.
- **Wire schema bump** — no new wire types; `ingest_report.v1` counters absorb image events naturally.

## Risks / notes

- **OCR / caption failures are silent in `IngestReport` counters.** The user only sees them via `--debug` traces or `kebab inspect doc <id>` (Provenance Warning events). This is the intentional Lenient policy from the brainstorming; flag in the PR description so users know how to detect partial failures. A future spec extension could introduce a `image_ocr_failed` / `image_caption_failed` counter alongside `errors`.
- **LM endpoint validation runs once per ingest.** A misconfigured `models.llm.endpoint` aborts ingest before any asset is processed. This is correct fail-fast behaviour but means a single broken endpoint takes the whole ingest down — even markdown assets that don't need the LM. The user fix is `image.caption.enabled = false` or correcting the endpoint.
- **`unreachable!` in the chunker branch is guarded by `is_image_only_document`.** If a future task introduces multi-block image documents (e.g. embedded markdown caption alongside the image), the predicate must change in lockstep. Keep both functions in the same module so the invariant is local.
- **Determinism stress.** The existing markdown path's `kb-normalize::build_canonical_document` shares one `now_utc()` reading across the per-document Provenance events. The image path needs the same: P6-1 already does `let now = OffsetDateTime::now_utc();` once and reuses it for both events; this task's wiring must not introduce a second `now()` call between extract and apply_ocr/caption that would break per-document timestamp parity.
