# Spine Simplification — Phase 2 (Config slices + surface trim) Plan

> Execute as 3 sequential cohesive units (compile-coupled — NOT parallel). Each unit = one fresh agent, ending with the Parity Gate (`/home/user/large_data/out/kebab-parity/gate.sh`) + unit-specific checks. Branch `refactor/spine-cuts` (continues from Phase 1).

**Goal:** Collapse OCR config duplication, trim the exposed config/env surface (109→~30 keys, 97→~25 env), and decouple consumers from the god-struct `Config` (take typed slices). Output-preserving; config v4→v5 lossless auto-migration.

**Recon facts (verified):** `CURRENT_SCHEMA_VERSION=4` (`migrate.rs:12`). NO `#[serde(deny_unknown_fields)]` anywhere → removed keys silently ignored (zero breakage). `apply_env` (`lib.rs:1271`) = manual per-field match arms (~115). Migration = pure `migrate_document` → `run_steps` → `reconcile`, uses `move_table`.

## Global Constraints
- Parity Gate after each unit: `bash /home/user/large_data/out/kebab-parity/gate.sh <unit>` → SEARCH/ASK/CHUNKS IDENTICAL (markdown corpus → OCR not exercised, but config-load + search/ask must stay identical). `cargo clippy --workspace --all-targets -- -D warnings` = 0. `CARGO_TARGET_DIR=/home/user/large_data/out/kebab/target`.
- **Use `cargo check/clippy --all-targets`** (Phase 1 lesson: `cargo check` alone misses test-target refs).
- Config v4→v5 migration MUST round-trip (old v4 config → v5 → same effective values). Add a round-trip test.
- ollama `.244` (snowflake-arctic-embed2 + gemma3:4b) up for gates; lemonade down until Phase 2 ends.

---

## Unit 1: OCR consolidation — shared `[ingest.ocr]` engine block + v4→v5 migration

`OcrCfg` (image, `lib.rs:434-500`) has 13 fields, ALL shared with `PdfOcrCfg` (`lib.rs:647-709`); image has 0 unique fields. PDF adds 4 unique: `always_on`, `valid_ratio_threshold`, `min_char_count`, `lang_hint`.

**Files:** `crates/kebab-config/src/lib.rs`, `crates/kebab-config/src/migrate.rs`, OCR consumers (`crates/kebab-parse-image/src/`, `crates/kebab-parse-pdf/src/` — wherever `config.ingest.image.ocr.*` / `config.ingest.pdf.ocr.*` are read), `crates/kebab-app/src/lib.rs` (ingest_config_signature OCR fields), docs.

**Design:**
- New `SharedOcrEngineCfg` struct = the 13 shared fields (enabled, engine, model, endpoint, languages, max_pixels, request_timeout_secs, det_model, rec_model, dict, score_thresh, unclip_ratio, max_boxes).
- `[ingest.ocr]` = `SharedOcrEngineCfg` (workspace-wide OCR engine defaults).
- `[ingest.image.ocr]` → slim override: optional overrides of the shared block (or just keeps `enabled` + per-image overrides). `[ingest.pdf.ocr]` → the 4 PDF-unique fields + optional shared overrides.
- Resolution: image/pdf OCR effective config = `[ingest.ocr]` merged with their override block. Add a resolver method (e.g. `Config::image_ocr() -> ResolvedOcr`, `Config::pdf_ocr() -> ResolvedOcr`) so consumers read resolved values.
- `apply_env`: replace the ~27 duplicated image/pdf OCR arms with one shared-engine set (`KEBAB_OCR_*`) + the 4 PDF-only arms.
- **Migration `step_4_to_5`** (`migrate.rs`): use `move_table` to lift the shared keys present in `[ingest.image.ocr]` / `[ingest.pdf.ocr]` into `[ingest.ocr]` (so existing configs keep effective values). Bump `CURRENT_SCHEMA_VERSION=5`, add `if from < 5` step, add `[ingest.ocr]` to `annotated_default_document()`.

**Verify:** clippy --all-targets 0; `cargo test -p kebab-config` (add v4→v5 round-trip test: a v4 TOML with image/pdf OCR keys → migrate → assert resolved image/pdf OCR values unchanged); `cargo test -p kebab-parse-image -p kebab-parse-pdf`; Parity Gate (search/ask/chunk IDENTICAL — config still loads, markdown unaffected). Commit `refactor(config): OCR 중복 제거 — 공유 [ingest.ocr] 엔진 블록 + v4→v5 마이그레이션`.

---

## Unit 2: Surface trim — env 97→~25, exposed keys 109→~30

No `deny_unknown_fields` → removing struct fields + their `apply_env` arms is safe; dropped TOML keys are ignored on load.

**Files:** `crates/kebab-config/src/lib.rs` (struct fields + `apply_env` arms + `annotated_default_document`), `crates/kebab-config/src/migrate.rs` (reconcile reference), README Configuration section, `docs/SMOKE.md` config example.

**Design:**
- Keep KEBAB_* env ONLY for runtime-override-worthy fields (~25): endpoints, paths/dirs, model names, thread/parallelism counts, enable toggles. DELETE the long-tail arms (per-field tuning knobs: score_thresh, unclip_ratio, max_boxes, rrf_k, multi_hop_max_*, snippet_chars, etc.) — these stay config-only.
- Documented surface: keep the ~30 fields a user actually sets in README/SMOKE; the rarely-tuned ones remain parseable (struct fields stay, with sane defaults) but drop from docs + env. (i.e. trim ENV + DOCS surface; struct fields with defaults remain for advanced TOML use unless truly dead.)
- Truly-dead fields (no consumer reads them): delete entirely.

**Verify:** clippy --all-targets 0; `cargo test -p kebab-config`; Parity Gate IDENTICAL (defaults unchanged → output identical). Update README + SMOKE config block. Commit `refactor(config): env 97→~25 + 노출 키 109→~30 (표면 정리)`.

---

## Unit 3: Slice refactor — consumers take typed slices, not `&Config`

All consumers take `&kebab_config::Config` (whole); `RagPipeline::new` takes it by value. Decouple to typed slices. **Do incrementally per-consumer so each step compiles** (change one constructor + its call sites in kebab-app, build, next).

**Consumers (recon):** `RagPipeline::new(config: Config, ...)` (rag/pipeline.rs:197) → `(rag: RagCfg, models: ModelsCfg, ...)`; `SqliteStore::open(&Config)` (store.rs:123) → `(&StorageCfg)`; `LanceVectorStore::new(&Config, ...)` (vector/store.rs:96) → `(&StorageCfg, ...)`; `FastembedEmbedder::new(&Config)` (embed-local:62) → `(&EmbeddingModelCfg)`; `OllamaEmbedder::new(&Config)` (embed-ollama:110) → `(&EmbeddingModelCfg)`; `HybridRetriever::new(&Config, ...)` (search/hybrid.rs:80) → `(&SearchCfg, ...)`. Also fix `VectorRetriever::new` hidden `Config::defaults()` coupling (vector.rs:74) → take `snippet_chars` param.

**Order:** leaf consumers first (embedders, stores), then retrievers, then RagPipeline, updating each one's call sites in `kebab-app` in the SAME step so the build stays green. Each consumer = its own commit + build check.

**Verify:** after all consumers sliced — clippy --workspace --all-targets 0; `cargo test` touched crates; Parity Gate IDENTICAL. Commit per consumer or one `refactor(config): consumer들이 &Config 대신 타입 슬라이스 수령 (god-struct 결합 해소)`.

---

## Phase 2 exit
- 3 units merged on refactor/spine-cuts, each gated (Parity IDENTICAL + clippy 0 + tests).
- config schema v5, env ≤~25, documented keys ≤~30, no consumer takes whole `&Config` (except where genuinely needed).
- HOTFIXES dated entry with the migration evidence. Then Phase 3 (ingest spine).
