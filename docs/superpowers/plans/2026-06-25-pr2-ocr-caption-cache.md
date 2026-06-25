# PR2 — OCR/caption derivation cache Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Cache image-OCR, image-caption, and PDF-page-OCR results in the derivation cache, keyed on SOURCE BYTES + an OCR/caption version key, so a re-ingest (or a chunker/embedding version bump) reuses the cached text instead of re-running the expensive vision engine — while keeping ingest output byte-identical.

**Architecture:** Two new derivation-cache namespaces `"ocr"` and `"caption"` join the existing `"embedding"`. A new `derivation_cache_key_bytes(kind, &[u8], version_key)` (in `kebab-core`) keys on raw image/page bytes (no NFC). `OcrText`/`ModelCaption` are cached as full serde-JSON structs (block-level byte-identical). All cache wrapping is at the `kebab-app` call sites (image OCR, image caption) or threaded via `PdfOcrOpts` into `apply_ocr_to_pdf_pages` (which already lives in `kebab-app`) — never inside a `kebab-parse-*` crate. On a cache hit the vision engine is skipped and NO provenance event is replayed (provenance is in no wire schema / not gate-compared — design §3.6).

**Tech Stack:** Rust 2024, `kebab-core` (derivation key), `kebab-app` (payload + wiring + tests), `blake3`, `serde_json`. Deterministic correctness is proven with a **mock `OcrEngine`** (no models); paddle-onnx (bundled, deterministic) anchors an optional real-engine `#[ignore]` lane.

## Global Constraints (from spec §3 + CLAUDE.md)

- **Byte-identical ingest output (HARD GATE).** Fresh-dir ingest = every key misses = vision engine runs exactly as today. Re-ingest = cache hit reconstructs the SAME `block.ocr`/`block.caption` struct (full serde round-trip ⇒ all fields, not just `.joined`/`.text`). The chunk text fed to the embedder is therefore identical. Markdown parity gate (`gate-ingest.sh`) is untouched and stays IDENTICAL.
- **Provenance NOT replayed on a cache hit** (design §3.6). Skipping `apply_ocr`/`apply_caption` means no `ProvenanceEvent` is pushed — correct, because provenance is in no wire schema (`search_hit`/`answer`/`chunk_inspection`) and the gate strips/ignores it.
- **Cache key on SOURCE BYTES**, not text: `derivation_cache_key_bytes` (no NFC — binary). One canonical helper; no per-site inline hashing.
- **Version keys** fold engine identity + output-shaping params (OCR) / provider + prompt (caption) so a cascade bump is a miss (§3.3).
- **`kebab-parse-*` MUST NOT gain a `kebab-store-*` dep.** All caching is in `kebab-app` (`ingest.rs`, `pdf_ocr_apply.rs`, `derivation_payload.rs`).
- Build/clippy: `CARGO_TARGET_DIR=/home/user/large_data/out/kebab/target`, `cargo clippy --workspace --all-targets -- -D warnings` = 0.
- GPU swap for any ollama-vision dogfood (CLAUDE.md): lemonade→`ollama-r9700` on `.82`, OCR/caption endpoint `http://192.168.0.244:11434`, **restore lemonade unconditionally** after.

## File Structure

- **Modify** `crates/kebab-core/src/derivation.rs` — add `derivation_cache_key_bytes` + tests.
- **Modify** `crates/kebab-app/src/derivation_payload.rs` — add `encode/decode_ocr_text`, `encode/decode_model_caption` + tests.
- **Modify** `crates/kebab-app/src/ingest.rs` — wrap the image OCR call (~1604) and image caption call (~1634) with cache lookup; build the OCR/caption version keys; accumulate touch keys.
- **Modify** `crates/kebab-app/src/pdf_ocr_apply.rs` — extend `PdfOcrOpts` with an optional cache handle + version key; wrap the per-page `engine.recognize` (~175).
- **Create** `crates/kebab-app/tests/ocr_caption_cache.rs` — deterministic mock-engine integration tests (hit/miss, byte-identical block reconstruction, version-key invalidation), no models.

---

### Task 1: `derivation_cache_key_bytes` in kebab-core

**Files:**
- Modify: `crates/kebab-core/src/derivation.rs` (add fn after `derivation_cache_key`, ~line 41; add tests in the `mod tests` block)

**Interfaces:**
- Consumes: `blake3` (already a dep).
- Produces: `pub fn derivation_cache_key_bytes(kind: &str, bytes: &[u8], version_key: &str) -> String` — 32-hex, same framing as `derivation_cache_key` but hashes raw bytes (NO NFC).

- [ ] **Step 1: Write the failing tests** (append inside `mod tests` in `crates/kebab-core/src/derivation.rs`)

```rust
    #[test]
    fn bytes_key_is_32_hex_chars() {
        let k = derivation_cache_key_bytes("ocr", &[0u8, 1, 2, 3], "engine|v1");
        assert_eq!(k.len(), 32);
        assert!(k.bytes().all(|b| b.is_ascii_hexdigit()));
    }

    #[test]
    fn bytes_same_inputs_same_key() {
        let a = derivation_cache_key_bytes("ocr", b"\xff\x00image", "paddle|abc");
        let b = derivation_cache_key_bytes("ocr", b"\xff\x00image", "paddle|abc");
        assert_eq!(a, b);
    }

    #[test]
    fn bytes_different_bytes_different_key() {
        let a = derivation_cache_key_bytes("ocr", b"image-a", "v1");
        let b = derivation_cache_key_bytes("ocr", b"image-b", "v1");
        assert_ne!(a, b);
    }

    #[test]
    fn bytes_different_kind_different_key() {
        let o = derivation_cache_key_bytes("ocr", b"same", "v1");
        let c = derivation_cache_key_bytes("caption", b"same", "v1");
        assert_ne!(o, c);
    }

    #[test]
    fn bytes_version_bump_is_miss() {
        // §3.6 safety: a version_key change MUST change the key so a stale OCR
        // result is never reused after an engine/param bump.
        let v1 = derivation_cache_key_bytes("ocr", b"page-bytes", "paddle-abc|st:0.3");
        let v2 = derivation_cache_key_bytes("ocr", b"page-bytes", "paddle-abc|st:0.5");
        assert_ne!(v1, v2);
    }

    #[test]
    fn bytes_no_nfc_raw_hash() {
        // Unlike the text variant, the bytes variant must NOT NFC-normalize —
        // it hashes raw bytes. Two byte strings that would NFC-collapse as text
        // stay distinct as bytes. (Sanity: empty vs non-empty differ.)
        assert_ne!(
            derivation_cache_key_bytes("ocr", b"", "v1"),
            derivation_cache_key_bytes("ocr", b"\x00", "v1")
        );
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `CARGO_TARGET_DIR=/home/user/large_data/out/kebab/target cargo test -p kebab-core derivation -- bytes`
Expected: FAIL — `cannot find function derivation_cache_key_bytes`.

- [ ] **Step 3: Implement** (in `crates/kebab-core/src/derivation.rs`, right after `derivation_cache_key`, ~line 41)

```rust
/// Byte-addressed derivation-cache key (§3.1) for **binary** inputs (image /
/// PDF-page bytes feeding OCR / caption). Identical framing to
/// [`derivation_cache_key`] — `kind || 0x00 || content_blake3 || 0x00 ||
/// version_key`, first 32 hex chars — but hashes the raw bytes directly with
/// NO NFC normalization (NFC is meaningless on arbitrary bytes).
pub fn derivation_cache_key_bytes(kind: &str, bytes: &[u8], version_key: &str) -> String {
    let content_blake3 = blake3::hash(bytes).to_hex().to_string();

    let mut hasher = blake3::Hasher::new();
    hasher.update(kind.as_bytes());
    hasher.update(&[0x00]);
    hasher.update(content_blake3.as_bytes());
    hasher.update(&[0x00]);
    hasher.update(version_key.as_bytes());

    hasher.finalize().to_hex().to_string()[..32].to_string()
}
```

- [ ] **Step 4: Run to verify they pass**

Run: `CARGO_TARGET_DIR=/home/user/large_data/out/kebab/target cargo test -p kebab-core derivation`
Expected: PASS (all bytes_* + existing tests).

- [ ] **Step 5: Commit**

```bash
git add crates/kebab-core/src/derivation.rs
git commit -m "feat(core): derivation_cache_key_bytes — 바이트 키 변형 (OCR/caption 입력용, NFC 없음)"
```

---

### Task 2: OCR/caption payload encode/decode in kebab-app

**Files:**
- Modify: `crates/kebab-app/src/derivation_payload.rs` (add 4 fns + tests)

**Interfaces:**
- Consumes: `kebab_core::{OcrText, ModelCaption}` (both derive `Serialize`/`Deserialize`); `serde_json` (verified already a dep — `crates/kebab-app/Cargo.toml:51` — no Cargo.toml change needed).
- Produces:
  - `pub fn encode_ocr_text(o: &OcrText) -> Vec<u8>` / `pub fn decode_ocr_text(p: &[u8]) -> Option<OcrText>`
  - `pub fn encode_model_caption(c: &ModelCaption) -> Vec<u8>` / `pub fn decode_model_caption(p: &[u8]) -> Option<ModelCaption>`

Full-struct serde (not just `.joined`/`.text`) so a cache hit reconstructs `block.ocr`/`block.caption` byte-identically at the BLOCK level (the struct is persisted with the block, not only rendered into chunk text). A decode error → `None` → caller treats as miss → recompute (same accuracy-first contract as `decode_embedding`).

- [ ] **Step 1: Write the failing tests** (append a `mod ocr_caption` test block, or extend `mod tests`, in `crates/kebab-app/src/derivation_payload.rs`)

```rust
#[cfg(test)]
mod ocr_caption_tests {
    use super::*;
    use kebab_core::{ModelCaption, OcrRegion, OcrText};

    fn sample_ocr() -> OcrText {
        OcrText {
            joined: "안녕 OCR\nsecond line".to_string(),
            regions: vec![OcrRegion {
                bbox: (1, 2, 3, 4),
                text: "안녕".to_string(),
                confidence: 0.97,
            }],
            engine: "paddle-onnx".to_string(),
            engine_version: "ppocrv5-mobile-kor-abc123".to_string(),
        }
    }

    #[test]
    fn ocr_text_roundtrips_full_struct() {
        let o = sample_ocr();
        let bytes = encode_ocr_text(&o);
        assert_eq!(decode_ocr_text(&bytes), Some(o));
    }

    #[test]
    fn ocr_decode_garbage_is_none() {
        assert_eq!(decode_ocr_text(b"\xff\xff not json"), None);
    }

    #[test]
    fn model_caption_roundtrips_full_struct() {
        let c = ModelCaption {
            text: "a red square".to_string(),
            model: "gemma4:e4b".to_string(),
            model_version: "ollama/caption-v1".to_string(),
        };
        let bytes = encode_model_caption(&c);
        assert_eq!(decode_model_caption(&bytes), Some(c));
    }

    #[test]
    fn caption_decode_garbage_is_none() {
        assert_eq!(decode_model_caption(b"\x00\x01"), None);
    }
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `CARGO_TARGET_DIR=/home/user/large_data/out/kebab/target cargo test -p kebab-app --lib derivation_payload`
Expected: FAIL — `cannot find function encode_ocr_text` (etc.).

- [ ] **Step 3: Implement** (append to `crates/kebab-app/src/derivation_payload.rs`)

```rust
use kebab_core::{ModelCaption, OcrText};

/// Encode an `OcrText` as serde-JSON bytes for the `"ocr"` derivation-cache
/// namespace (§3.4). Full struct — a cache hit reconstructs `block.ocr`
/// byte-identically (all fields, not just `.joined`), so the stored block
/// matches a fresh deterministic OCR run.
pub fn encode_ocr_text(o: &OcrText) -> Vec<u8> {
    serde_json::to_vec(o).expect("OcrText serialize (infallible for owned struct)")
}

/// Decode an `OcrText` from the `"ocr"` namespace. `None` on any decode error
/// → caller treats as a cache miss and recomputes (never serves a wrong value).
pub fn decode_ocr_text(payload: &[u8]) -> Option<OcrText> {
    serde_json::from_slice(payload).ok()
}

/// Encode a `ModelCaption` as serde-JSON bytes for the `"caption"` namespace.
pub fn encode_model_caption(c: &ModelCaption) -> Vec<u8> {
    serde_json::to_vec(c).expect("ModelCaption serialize (infallible for owned struct)")
}

/// Decode a `ModelCaption` from the `"caption"` namespace. `None` on error → miss.
pub fn decode_model_caption(payload: &[u8]) -> Option<ModelCaption> {
    serde_json::from_slice(payload).ok()
}
```

- [ ] **Step 4: Run to verify they pass**

Run: `CARGO_TARGET_DIR=/home/user/large_data/out/kebab/target cargo test -p kebab-app --lib derivation_payload`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/kebab-app/src/derivation_payload.rs
git commit -m "feat(app): ocr/caption derivation payload — full-struct serde encode/decode"
```

---

### Task 3 (b1): cache image OCR at the call site

**Files:**
- Modify: `crates/kebab-app/src/ingest.rs` (image OCR call ~1604-1611, inside `ingest_one_image_asset`)

**Interfaces:**
- Consumes: `apply_ocr(engine: &dyn OcrEngine, &[u8], &mut ImageRefBlock, Option<&Lang>, &mut Vec<ProvenanceEvent>) -> Result<()>` (sets `block.ocr = Some(OcrText)` + pushes provenance on success); `OcrEngine::engine_name() -> &'static str`, `engine_version() -> String`; `derivation_cache_key_bytes`, `derivation_payload::{encode_ocr_text, decode_ocr_text}`; `app.sqlite.derivation_cache_get/put/touch`.
- Produces: nothing new.

The current code (ingest.rs ~1604):
```rust
                let t_ocr = std::time::Instant::now();
                let res = apply_ocr(
                    engine,
                    &bytes,
                    block,
                    lang_hint.as_ref(),
                    &mut canonical.provenance.events,
                );
                ocr_ms = u64::try_from(t_ocr.elapsed().as_millis()).unwrap_or(u64::MAX);
                if let Err(e) = res {
                    record_image_analysis_failure( /* … "OcrFailed" … */ );
                }
```

Wrap it: build the version key, look up by image bytes, on hit set `block.ocr` and skip; on miss run `apply_ocr` then cache whatever it produced.

- [ ] **Step 1: Build the OCR version key** — add a small helper near the other ingest helpers (or inline). It folds engine identity + paddle output-shaping params (§3.3). Place this `fn` in `ingest.rs` (module scope):

```rust
/// Version key for the `"ocr"` derivation-cache namespace (§3.3). Folds the
/// engine identity (name + version — for paddle, version is a blake3 of the
/// model assets; for ollama-vision it is engine/model) plus the paddle
/// output-shaping params, so an engine/asset/param change is a cache miss.
/// `score_thresh`/`unclip_ratio`/`max_boxes` are paddle-only but folding them
/// always is harmless (constant for non-paddle) and future-proofs the key the
/// moment they become user-configurable. Takes the params as primitives so the
/// same fn serves both image (`OcrCfg`) and pdf (`PdfOcrCfg`) — both expose
/// `score_thresh: f32` / `unclip_ratio: f32` / `max_boxes: usize`.
fn ocr_cache_version_key(
    engine: &dyn OcrEngine,
    score_thresh: f32,
    unclip_ratio: f32,
    max_boxes: usize,
) -> String {
    format!(
        "{}/{}|st:{}|uc:{}|mb:{}",
        engine.engine_name(),
        engine.engine_version(),
        score_thresh,
        unclip_ratio,
        max_boxes,
    )
}
```
> Verified: `app.config.image_ocr() -> &OcrCfg` (lib.rs:1574) with concrete `score_thresh: f32`/`unclip_ratio: f32`/`max_boxes: usize` (lib.rs:573/577/581); `app.config.pdf_ocr() -> &PdfOcrCfg` (lib.rs:1581) with the same field names+types (lib.rs:814/817/820). `OcrEngine` is `kebab_parse_image::OcrEngine`, already imported in `ingest.rs` (the `engine` param is one).

- [ ] **Step 2: Replace the OCR call with the cached wrap** (ingest.rs ~1604):

```rust
                let t_ocr = std::time::Instant::now();
                let img_ocr_cfg = app.config.image_ocr();
                let ocr_vkey = ocr_cache_version_key(
                    engine,
                    img_ocr_cfg.score_thresh,
                    img_ocr_cfg.unclip_ratio,
                    img_ocr_cfg.max_boxes,
                );
                let ocr_key = kebab_core::derivation_cache_key_bytes("ocr", &bytes, &ocr_vkey);
                if let Some(cached) = app
                    .sqlite
                    .derivation_cache_get(&ocr_key)?
                    .and_then(|p| crate::derivation_payload::decode_ocr_text(&p))
                {
                    // Cache hit: reconstruct block.ocr, skip the engine + provenance
                    // (design §3.6 — provenance is not output/gate-affecting).
                    block.ocr = Some(cached);
                    ocr_cache_touch.push(ocr_key);
                } else {
                    let res = apply_ocr(
                        engine,
                        &bytes,
                        block,
                        lang_hint.as_ref(),
                        &mut canonical.provenance.events,
                    );
                    if let Err(e) = res {
                        record_image_analysis_failure(
                            asset,
                            &mut canonical.provenance.events,
                            &mut warning_notes,
                            "OcrFailed",
                            e,
                            now,
                        );
                    } else if let Some(produced) = &block.ocr {
                        app.sqlite.derivation_cache_put(
                            &ocr_key,
                            "ocr",
                            &crate::derivation_payload::encode_ocr_text(produced),
                        )?;
                    }
                }
                ocr_ms = u64::try_from(t_ocr.elapsed().as_millis()).unwrap_or(u64::MAX);
```
> Note: only cache on SUCCESS with a produced `block.ocr` (a failed OCR leaves `block.ocr = None` and is NOT cached — the next run retries). `ocr_ms` on a hit is ~0, correctly reflecting the skipped engine.

- [ ] **Step 3: Declare the touch-key accumulator** — near the top of `ingest_one_image_asset` (alongside `ocr_ms`/`caption_ms`), add:

```rust
    let mut ocr_cache_touch: Vec<String> = Vec::new();
    let mut caption_cache_touch: Vec<String> = Vec::new();
```
(The caption one is for Task 4; declare both now to avoid a second edit to the same region.)

- [ ] **Step 4: Touch hit keys after store** — find where the image handler calls `store_document_records(...)` / `derivation_cache_touch` for embeddings (the embedding touch added in PR1, ~after `VectorStore::upsert (image)`). Add the OCR/caption touch alongside it (touching is idempotent and order-independent; doing it after store mirrors the embedding pattern):

```rust
        app.sqlite.derivation_cache_touch(&ocr_cache_touch)?;
        app.sqlite.derivation_cache_touch(&caption_cache_touch)?;
```
> If those touch calls sit inside the `if let (Some(emb), Some(vec_store)) = …` embedding block, hoist the two OCR/caption touches OUT to run unconditionally (OCR/caption happen even when embedding is disabled). Place them right after `store_document_records(...)`.

- [ ] **Step 5: Build + clippy**

Run: `CARGO_TARGET_DIR=/home/user/large_data/out/kebab/target cargo clippy -p kebab-app --all-targets -- -D warnings`
Expected: `Finished`, 0 warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/kebab-app/src/ingest.rs
git commit -m "feat(app): 이미지 OCR 결과 캐시 — 바이트 키 ocr 네임스페이스 (b1)"
```

---

### Task 4 (b2): cache image caption at the call site

**Files:**
- Modify: `crates/kebab-app/src/ingest.rs` (image caption call ~1634-1645, same handler)

**Interfaces:**
- Consumes: `apply_caption(llm: &dyn LanguageModel, &[u8], &mut ImageRefBlock, Option<&Lang>, &Config, &mut Vec<ProvenanceEvent>) -> Result<()>` (gate-checks `cfg.ingest.image.caption.enabled`; on success sets `block.caption = Some(ModelCaption)` + provenance; disabled = no-op); `LanguageModel::model_ref() -> ModelRef` (`.provider`, `.id`); `derivation_payload::{encode_model_caption, decode_model_caption}`.
- Produces: nothing new.

The current code (ingest.rs ~1634):
```rust
                let t_caption = std::time::Instant::now();
                let res = apply_caption(
                    llm,
                    &bytes,
                    block,
                    lang_hint.as_ref(),
                    &app.config,
                    &mut canonical.provenance.events,
                );
                caption_ms = u64::try_from(t_caption.elapsed().as_millis()).unwrap_or(u64::MAX);
                if let Err(e) = res {
                    record_image_analysis_failure( /* … "CaptionFailed" … */ );
                }
```

- [ ] **Step 1: Build the caption version key + wrap** — caption version = provider + prompt_template_version (§3.3; matches `ModelCaption.model_version = "{provider}/{prompt}"`). Replace the caption call with:

```rust
                let t_caption = std::time::Instant::now();
                // Only cache when captioning is enabled (apply_caption no-ops when
                // disabled; mirror that gate so a hit can't bypass the toggle).
                if app.config.ingest.image.caption.enabled {
                    let cap_vkey = format!(
                        "caption|{}|{}",
                        llm.model_ref().provider,
                        app.config.ingest.image.caption.prompt_template_version,
                    );
                    let cap_key =
                        kebab_core::derivation_cache_key_bytes("caption", &bytes, &cap_vkey);
                    if let Some(cached) = app
                        .sqlite
                        .derivation_cache_get(&cap_key)?
                        .and_then(|p| crate::derivation_payload::decode_model_caption(&p))
                    {
                        block.caption = Some(cached);
                        caption_cache_touch.push(cap_key);
                    } else {
                        let res = apply_caption(
                            llm,
                            &bytes,
                            block,
                            lang_hint.as_ref(),
                            &app.config,
                            &mut canonical.provenance.events,
                        );
                        if let Err(e) = res {
                            record_image_analysis_failure(
                                asset,
                                &mut canonical.provenance.events,
                                &mut warning_notes,
                                "CaptionFailed",
                                e,
                                now,
                            );
                        } else if let Some(produced) = &block.caption {
                            app.sqlite.derivation_cache_put(
                                &cap_key,
                                "caption",
                                &crate::derivation_payload::encode_model_caption(produced),
                            )?;
                        }
                    }
                }
                caption_ms = u64::try_from(t_caption.elapsed().as_millis()).unwrap_or(u64::MAX);
```
> The `caption_cache_touch` accumulator was already declared in Task 3 Step 3; the touch call was added in Task 3 Step 4.

- [ ] **Step 2: Build + clippy**

Run: `CARGO_TARGET_DIR=/home/user/large_data/out/kebab/target cargo clippy -p kebab-app --all-targets -- -D warnings`
Expected: 0 warnings.

- [ ] **Step 3: Commit**

```bash
git add crates/kebab-app/src/ingest.rs
git commit -m "feat(app): 이미지 caption 결과 캐시 — 바이트 키 caption 네임스페이스 (b2)"
```

---

### Task 5 (b3): cache PDF per-page OCR via `PdfOcrOpts`

**Files:**
- Modify: `crates/kebab-app/src/pdf_ocr_apply.rs` (`PdfOcrOpts` ~41, `apply_ocr_to_pdf_pages` per-page `engine.recognize` ~175)
- Modify: `crates/kebab-app/src/ingest.rs` (PDF OCR call site ~2214 — populate the new `PdfOcrOpts` fields)

**Interfaces:**
- Consumes: `extract_dctdecode_page_image(&pdf_doc, page_num) -> Result<Option<Vec<u8>>>` (the per-page bytes, ~148); `engine.recognize(&page_image_bytes, lang_hint) -> Result<OcrText>` (~175); `Arc<SqliteStore>` (the call site already builds `store_for_ocr = Arc::clone(&app.sqlite)`); `OcrEngine::{engine_name, engine_version}`.
- Produces: `PdfOcrOpts` gains `ocr_cache: Option<std::sync::Arc<kebab_store_sqlite::SqliteStore>>` and `ocr_version_key: String`. `apply_ocr_to_pdf_pages` returns the same `PdfOcrSummary` (unchanged signature otherwise) — but its `PdfOcrSummary` should expose how many pages were cache hits if cheap; OPTIONAL, skip if it complicates.

`pdf_ocr_apply.rs` is in `kebab-app`, so depending on `kebab_store_sqlite` here does NOT violate the parse-crate boundary (it already imports `kebab_parse_pdf` + app types).

- [ ] **Step 1: Extend `PdfOcrOpts`** (`pdf_ocr_apply.rs` ~41). Add two fields:

```rust
    /// Optional derivation-cache handle for per-page OCR results (`"ocr"`
    /// namespace). `None` ⇒ no caching (unit tests / store-less callers behave
    /// exactly as before).
    pub ocr_cache: Option<std::sync::Arc<kebab_store_sqlite::SqliteStore>>,
    /// Version key folded into the per-page OCR cache key (§3.3). Empty when
    /// `ocr_cache` is `None`.
    pub ocr_version_key: String,
```

- [ ] **Step 2: Wrap the per-page `engine.recognize`** (`pdf_ocr_apply.rs` ~175). The current line is:

```rust
        let ocr = match engine.recognize(&page_image_bytes, opts.lang_hint.as_ref()) {
            Ok(t) => t,
            Err(e) => { /* … warning event, continue … */ }
        };
```

Replace the `engine.recognize(...)` acquisition with a cache-aware version. Insert BEFORE the `match`:

```rust
        // Per-page OCR cache (§3.5 b3): key on the page image bytes + version.
        let page_cache_key = opts.ocr_cache.as_ref().map(|_| {
            kebab_core::derivation_cache_key_bytes("ocr", &page_image_bytes, &opts.ocr_version_key)
        });
        if let (Some(store), Some(key)) = (opts.ocr_cache.as_ref(), page_cache_key.as_ref())
            && let Some(hit) = store
                .derivation_cache_get(key)
                .ok()
                .flatten()
                .and_then(|p| crate::derivation_payload::decode_ocr_text(&p))
        {
            // Cache hit: apply this page's OCR text without running the engine
            // or emitting an OcrApplied provenance event (§3.6), then continue
            // to the next page.
            apply_ocr_text_to_page(/* the same mutation the Ok(t) arm does */ &mut canonical, /* page idx */ , hit);
            store.derivation_cache_touch(std::slice::from_ref(key)).ok();
            continue;
        }
        let ocr = match engine.recognize(&page_image_bytes, opts.lang_hint.as_ref()) {
            Ok(t) => {
                if let (Some(store), Some(key)) = (opts.ocr_cache.as_ref(), page_cache_key.as_ref()) {
                    let _ = store.derivation_cache_put(key, "ocr", &crate::derivation_payload::encode_ocr_text(&t));
                }
                t
            }
            Err(e) => { /* … unchanged warning event, continue … */ }
        };
```
> **Implementer:** the existing `Ok(t) => t` arm is followed by code that mutates the page block with `ocr.joined` (the `tb.text = ocr.joined.clone()` / dual-block logic around lines 214-256). The cache-hit fast path must perform the **same** page mutation as the normal path before `continue`-ing — do NOT duplicate the logic by hand; instead, restructure so the hit path falls through into the SAME mutation code (e.g. set `let ocr = hit;` and skip only the `engine.recognize` + the `OcrApplied` provenance push, rather than `continue`). Choose whichever refactor keeps the page-mutation code single-sourced. The KEY invariants: (1) byte-identical page mutation whether hit or miss, (2) no `OcrApplied` provenance event on a hit, (3) on a miss, cache the produced `OcrText` after a successful `recognize`.

- [ ] **Step 3: Populate the new `PdfOcrOpts` fields at the call site** (`ingest.rs` ~2214). The `PdfOcrOpts { … }` literal currently sets `enabled/always_on/valid_ratio_threshold/min_char_count/lang_hint/cancel`. Add:

Before the `PdfOcrOpts { … }` literal, resolve the pdf OCR params once, then pass them to the shared `ocr_cache_version_key` (Task 3 Step 1). `engine` here is the `pdf_ocr_engine` (the `Some(engine)` bound in the `match pdf_ocr_engine`):

```rust
                    let pdf_ocr_cfg = app.config.pdf_ocr();
                    let pdf_ocr_vkey = ocr_cache_version_key(
                        engine,
                        pdf_ocr_cfg.score_thresh,
                        pdf_ocr_cfg.unclip_ratio,
                        pdf_ocr_cfg.max_boxes,
                    );
                    let ocr_opts = crate::pdf_ocr_apply::PdfOcrOpts {
                        // … existing fields (enabled/always_on/valid_ratio_threshold/
                        //     min_char_count/lang_hint/cancel) unchanged …
                        ocr_cache: Some(Arc::clone(&app.sqlite)),
                        ocr_version_key: pdf_ocr_vkey,
                    };
```
> `app.config.pdf_ocr() -> &PdfOcrCfg` (verified, lib.rs:1581) exposes `score_thresh: f32`/`unclip_ratio: f32`/`max_boxes: usize` (lib.rs:814/817/820), same names/types as `OcrCfg`, so the shared `ocr_cache_version_key` works directly — no new accessor or pdf-specific fn needed. Watch borrow timing: `app.config.pdf_ocr()` borrows `app.config`; bind `pdf_ocr_vkey` (an owned `String`) before constructing the `PdfOcrOpts` that also `Arc::clone(&app.sqlite)`, so no overlapping-borrow issue.

- [ ] **Step 4: Build + clippy + existing pdf_ocr_apply tests**

Run: `CARGO_TARGET_DIR=/home/user/large_data/out/kebab/target cargo clippy -p kebab-app --all-targets -- -D warnings`
Run: `CARGO_TARGET_DIR=/home/user/large_data/out/kebab/target cargo test -p kebab-app --lib pdf_ocr_apply`
Expected: 0 warnings; existing pdf_ocr_apply unit tests still pass (they pass `ocr_cache: None` — add that field to their `PdfOcrOpts` literals, behavior unchanged).

- [ ] **Step 5: Commit**

```bash
git add crates/kebab-app/src/pdf_ocr_apply.rs crates/kebab-app/src/ingest.rs
git commit -m "feat(app): PDF 페이지 OCR 캐시 — PdfOcrOpts 캐시 핸들 (b3, parse-crate 의존 무변경)"
```

---

### Task 6: Deterministic mock-engine cache test (hit/miss + byte-identical + invalidation)

**Files:**
- Create: `crates/kebab-app/tests/ocr_caption_cache.rs`

**Interfaces:**
- Consumes: `kebab_parse_image::OcrEngine` (trait to implement a mock); `kebab_core::{OcrText, OcrRegion}`; `common::TestEnv`; `kebab_app::ingest_with_config`; `rusqlite` (dev-dep) for cache-row counts. This test is **deterministic and model-free** — it is NOT `#[ignore]`; it runs in the default CI lane.

This is the primary correctness gate for b1/b3 (the OCR path) without any real model: a mock `OcrEngine` returns a fixed `OcrText` and COUNTS its invocations. Re-ingest must NOT re-invoke the engine (cache hit) and must reconstruct byte-identical `block.ocr`.

> **Design note for the implementer:** the cleanest deterministic test injects a mock `OcrEngine` into the ingest path. If the ingest facade does not expose engine injection, drive the cache at a lower seam instead: (a) directly unit-test the b1 wrap logic by calling `derivation_cache_get/put` + `decode_ocr_text` against an in-memory `SqliteStore` with a mock engine (proving hit skips the engine, miss invokes + caches, version bump → miss); OR (b) if image ingest CAN run with a mock/stub OCR engine via config (e.g. an engine the test can register), use the full `ingest_with_config` path and assert the mock's call-count is 1 after two ingests. Pick the seam that exists; do NOT weaken the assertions. The invocation-count assertion (engine called once across two ingests) is the non-negotiable core of this test.

- [ ] **Step 1: Write the mock engine + tests** (`crates/kebab-app/tests/ocr_caption_cache.rs`)

```rust
//! PR2: OCR/caption derivation cache. Deterministic, model-free — a mock
//! `OcrEngine` with an invocation counter proves a re-ingest is a cache HIT
//! (engine NOT re-invoked) that reconstructs byte-identical OCR text, and that
//! a version-key change forces a miss.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use kebab_core::{Lang, OcrText};
use kebab_parse_image::OcrEngine;

/// Mock OCR engine: returns a fixed `OcrText` and counts `recognize` calls.
struct CountingMockOcr {
    calls: Arc<AtomicUsize>,
    version: String,
}

impl OcrEngine for CountingMockOcr {
    fn engine_name(&self) -> &'static str {
        "mock-ocr"
    }
    fn engine_version(&self) -> String {
        self.version.clone()
    }
    fn model(&self) -> &str {
        "mock"
    }
    fn recognize(&self, _image_bytes: &[u8], _lang_hint: Option<&Lang>) -> anyhow::Result<OcrText> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(OcrText {
            joined: "안녕 mock OCR".to_string(),
            regions: Vec::new(),
            engine: "mock-ocr".to_string(),
            engine_version: self.version.clone(),
        })
    }
}

// The actual test body depends on the test seam chosen (see the design note).
// REQUIRED assertions, however the seam is wired:
//  1. Two ingests of the SAME image bytes + SAME engine version → engine
//     `recognize` is called EXACTLY ONCE (second is a cache hit).
//  2. The reconstructed `OcrText` after the cache hit equals the first run's
//     (byte-identical: joined + regions + engine + engine_version).
//  3. Changing the engine version (or score_thresh/etc. in the version key)
//     → `recognize` called AGAIN (miss). Proves §3.6 invalidation safety.
```

- [ ] **Step 2: Wire the chosen seam + run**

Run: `CARGO_TARGET_DIR=/home/user/large_data/out/kebab/target cargo test -p kebab-app --test ocr_caption_cache`
Expected: PASS — engine invoked once across two same-input ingests; reconstructed OcrText byte-identical; version bump re-invokes.

- [ ] **Step 3: Sanity — deliberately break, confirm fail**

Temporarily make `decode_ocr_text` always return `None` (forces every lookup to miss). Re-run: the "engine called exactly once" assertion must FAIL (it'd be called twice). Revert.

- [ ] **Step 4: Full clippy**

Run: `CARGO_TARGET_DIR=/home/user/large_data/out/kebab/target cargo clippy --workspace --all-targets -- -D warnings`
Expected: 0 warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/kebab-app/tests/ocr_caption_cache.rs
git commit -m "test(app): OCR 캐시 mock-engine 결정적 검증 — 히트=엔진 미호출 + byte-identical + 버전 무효화"
```

---

### Task 7: Paddle deterministic real-engine gate (optional `#[ignore]` lane) + markdown parity

**Files:**
- Create/extend: `crates/kebab-app/tests/ocr_caption_cache.rs` (add an `#[ignore]` test) OR a smoke script under the dogfood store.

**Interfaces:**
- Consumes: the bundled paddle-onnx assets (`crates/kebab-parse-image/assets/paddleocr-onnx/` — present, no download); `TestEnv` with `[ingest.ocr] engine="paddle-onnx"`, `[ingest.image.ocr] enabled=true`, `[models.embedding] provider="none"`.

The mock test (Task 6) proves the cache LOGIC deterministically. This task proves the REAL paddle engine path caches correctly (end-to-end), and that markdown output is unaffected.

- [ ] **Step 1: Markdown parity gate (must stay IDENTICAL)**

Run: `bash /home/user/large_data/out/kebab-parity/gate-ingest.sh pr2-ocr-cache`
Expected: `SEARCH IDENTICAL ✓`, `ASK IDENTICAL ✓`, `CHUNKS IDENTICAL ✓`. (PR2 does not touch the markdown path; this confirms no accidental regression. Requires the GPU ollama embedding box up per the gate config — if unavailable, note it and rely on the mock + unit gates for this PR, running the markdown gate before merge when the box is up.)

- [ ] **Step 2: Paddle real-engine cache test** (`#[ignore]`, AVX-gated). Generate a small PNG via `cargo run --example gen_smoke_png -p kebab-parse-image -- /tmp/ocr.png` (solid-color is fine — paddle runs deterministically; the test proves cache hit = identical `block.ocr` whether or not OCR finds text). Ingest it twice under a paddle config; assert the second ingest adds NO new `kind='ocr'` cache rows and the stored OCR text is identical. Use the same `embedding_cache_rows`-style helper scoped to `kind='ocr'`.

> If a text-bearing fixture is desired for stronger evidence, commit a couple of small screenshots with known text into the machine-local dogfood store (`/home/user/large_data/out/kebab-dogfood/corpus/images/`) and point the test at them — but this is NOT required for the cache-correctness gate (R2 in the spec; the mock test already pins correctness).

- [ ] **Step 3: GPU-box dogfood (ollama-vision quality, can't byte-gate)**

Per CLAUDE.md §Dogfood: swap lemonade→`ollama-r9700` on `.82`, point OCR/caption at `http://192.168.0.244:11434`, ingest the dogfood image/PDF corpus, confirm a re-ingest is a cache hit (logs show OCR/caption skipped) and search/ask quality is unchanged. **Restore lemonade unconditionally** afterward. Record evidence in `tasks/HOTFIXES.md` + `docs/release-notes/`.

- [ ] **Step 4: Commit any test/fixtures added**

```bash
git add crates/kebab-app/tests/ocr_caption_cache.rs
git commit -m "test(app): paddle 실엔진 OCR 캐시 결정적 게이트 (AVX-gated) + 마크다운 패리티 확인"
```

---

## Verification summary (PR-level HARD GATE)

- **Byte-identical:** full-struct serde ⇒ cache hit reconstructs `block.ocr`/`block.caption` identically (paddle deterministic ⇒ hit==fresh; ollama-vision ⇒ hit reproduces the pinned first result). Chunk text fed to the embedder is identical. Markdown gate IDENTICAL (path untouched).
- **Deterministic correctness (no models):** Task 6 mock-engine test — engine invoked once across two same-input ingests, byte-identical reconstruction, version bump → miss. This is the hard per-PR gate that runs in CI.
- **Provenance:** not replayed on hit (§3.6) — verified no `OcrApplied`/`CaptionApplied` event on a cache hit; provenance is in no wire schema / not gate-compared.
- **Boundary:** all caching in `kebab-app`; `kebab-parse-*` gains no `kebab-store-*` dep.
- **clippy** `--workspace --all-targets` = 0.

## Notes for the PR

- Conventional-commit, trailer-free. PR via gitea-ops; ask single-shot vs review-loop before creating.
- Version bump: PR2 adds two cache namespaces but no CLI flag / no wire change / no migration / identical user-visible output → **patch** (defer to the release/정리 step). If a `[ingest.ocr]` tuning knob is promoted to a user-facing key in the same work → **minor**.
- Risks carried from spec §6: R2 (no text-bearing OCR corpus) is resolved by the mock-engine test being the correctness gate (text corpus is optional dogfood evidence, not a merge gate); R3 (paddle params in version key) handled by `ocr_cache_version_key` folding `score_thresh`/`unclip_ratio`/`max_boxes`; R4 (parse boundary) handled by keeping the PDF cache in `kebab-app` via `PdfOcrOpts`.
