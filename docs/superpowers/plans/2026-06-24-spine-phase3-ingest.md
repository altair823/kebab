# Spine Simplification — Phase 3 (Ingest spine) Plan

> Execute as cohesive units, sequentially, on the MAIN worktree `refactor/spine-cuts` (NOT worktree-isolated — Phase 2 proved isolation:"worktree" branches off the wrong base). Each unit ends with the **re-ingest Parity Gate**: `bash /home/user/large_data/out/kebab-parity/gate-ingest.sh <unit>` → re-ingests the corpus with the new binary and asserts CHUNKS/SEARCH/ASK byte-IDENTICAL vs the Phase-0 baseline. (Validated: re-ingest is fully deterministic; the gate is the real test of an ingest-code change.)

**Goal:** Tame the kebab-app ingest monolith (`lib.rs` ~4193 lines): collapse the 6-variant ingest API to 2, centralize chunker dispatch + the per-asset fingerprint, and (risk-gated) extract the asset path into explicit stages. INGEST OUTPUT MUST STAY BYTE-IDENTICAL (no re-chunk, no re-embed-version change).

**Recon (HEAD 38990b4, post Phase 1/2):** 6 ingest fns (delegation chain) + `ingest_file`/`ingest_stdin`. Orchestrator `ingest_with_config_opts` (L264-835). Per-asset `ingest_one_asset` (L1220) + 3 media handlers + inline markdown. `try_skip_unchanged` (L985). `ingest_config_signature`/`effective_parser_version` (L3269) called inline 4×. `md_chunker_from_config`/`pdf_chunker_from_config` (L3168/3182) + inline code-lang match — NO central selector. `App::extract_for` registry (11 extractors; markdown NOT in registry).

## Global Constraints
- Re-ingest Parity Gate after each unit: CHUNKS/SEARCH/ASK IDENTICAL. `cargo clippy --workspace --all-targets -- -D warnings` = 0. `CARGO_TARGET_DIR=/home/user/large_data/out/kebab/target`. Use `--all-targets`.
- ingest output byte-identical: parser/chunker/embedding versions + `ingest_config_signature` effective values UNCHANGED.
- ollama `.244` up for gates; lemonade down until Phase 3 ends.

---

## Unit 3.1: Collapse ingest API 6 → 2  (LOW risk)

Current chain (`lib.rs`): `ingest`(L202) → `ingest_with_config`(L217) → `_progress`(L233) → `_cancellable`(L847) → `_opts`(L264, the real orchestrator). Plus `ingest_file_with_config`(L3712), `ingest_stdin_with_config`(L3788).

**Target:** keep exactly TWO workspace ingest entry points + the two file/stdin ones:
- `pub fn ingest(scope, opts: IngestOpts) -> Result<IngestReport>` — facade form: loads `Config::load(None)`, forwards. (Facade rule: bare form re-loads XDG config.)
- `#[doc(hidden)] pub fn ingest_with_config(config, scope, opts: IngestOpts) -> Result<IngestReport>` — the real orchestrator (renamed from `_opts`).
- `IngestOpts` gains `summary_only: bool` (folded in from the positional arg).
- DELETE `ingest_with_config_progress`, `ingest_with_config_cancellable`; their progress/cancel go through `IngestOpts`. Keep `ingest_file_with_config`/`ingest_stdin_with_config` (orthogonal), updating their internal call to the new signature.
- Update ALL callers: `crates/kebab-cli/src/main.rs`, `crates/kebab-mcp/src/tools/`, `crates/kebab-eval/src/`, and tests (`rg -n 'ingest_with_config|ingest_with_config_progress|ingest_with_config_cancellable|ingest_with_config_opts|\.ingest\(' crates`).

**Verify:** clippy --all-targets 0; `cargo test -p kebab-app -p kebab-cli -p kebab-mcp -p kebab-eval`; **gate-ingest.sh u3.1** IDENTICAL. Commit `refactor(app): ingest API 6변종 → 2 (ingest + ingest_with_config{IngestOpts})`.

---

## Unit 3.2: Central `chunker_for` selector  (LOW risk)

Chunker selection is scattered: `md_chunker_from_config` (L3168), `pdf_chunker_from_config` (L3182), inline code-lang match in `ingest_one_code_asset` (L2648-2666).

**Target:** one selector `fn chunker_for(config: &Config, media: &MediaType, code_lang: Option<&str>) -> Box<dyn Chunker>` (in kebab-app, or a `kebab_chunk::select` fn). It returns the exact same chunker each path uses today (MdHeadingV2 for markdown+image, PdfPageV1 for pdf, the per-lang code chunkers, CodeTextParagraphV1 for tier-3). Replace the 3 scattered selections with calls to it. **Output identical** (same chunker, same `max_chunk_tokens`).

**Verify:** clippy 0; tests; **gate-ingest.sh u3.2** IDENTICAL. Commit `refactor(chunk): 중앙 chunker_for 셀렉터 — 흩어진 청커 디스패치 통합`.

---

## Unit 3.3: Centralize fingerprint / effective version  (MEDIUM risk)

`effective_parser_version(config, asset, base)` (L3369) + `try_skip_unchanged` (L985) are called inline in all 4 handlers (markdown L1376, image L1674, pdf L2265, code L2688) — the "what version am I + should I skip" logic is replicated.

**Target:** a small `AssetFingerprint` helper that, given `(config, asset, base_versions, force_reingest)`, returns the effective versions + the skip decision in one place. Each handler calls it once at its top. Behavior identical (the version composite + skip checks are unchanged — only de-duplicated). Watch the tier-3 fallback sentinel (`none-v1` parser + `fallback_chunker_version` bypass) — keep that path exact.

**Verify:** clippy 0; tests; **gate-ingest.sh u3.3** IDENTICAL. Commit `refactor(app): AssetFingerprint — effective version/skip 결정 중앙화 (4× 복제 제거)`.

---

## Unit 3.4: (RISK-GATED) Stage pipeline extraction

Extract the per-asset path into explicit stages `scan → fingerprint → extract → chunk → embed → store`. **HIGH risk** — the recon flagged: PDF-OCR side-channel Arcs (8 params for progress/log/metrics/cancel), tier-3 mutable chunker-version state, markdown bypassing the extractor registry, `existing_doc_ids` preload. Byte-identical output is hardest here.

**Decision:** do NOT attempt until 3.1-3.3 land + are gated green. Then re-assess scope with the user — likely incremental (one stage at a time, each re-ingest-gated) or deferred. The store stage (identical 4× `put_*` + `vec.upsert`) is the safest single extraction to pilot.

## Phase 3 exit
- 3.1-3.3 merged + each re-ingest-gated IDENTICAL. lib.rs shrinks, dispatch centralized, API surface 6→2.
- 3.4 explicitly scoped (done incrementally, deferred, or dropped) per risk re-assessment.
- HOTFIXES dated entry.
