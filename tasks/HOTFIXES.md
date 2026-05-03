---
title: "Post-merge hotfixes log"
date: 2026-05-01
---

# Post-merge hotfixes log

Bugs discovered AFTER a phase task was merged, and the small follow-up
PRs that close them. Each entry: what broke, how it surfaced, what the
fix touched, and which task spec it amends.

The original task specs in `tasks/p<N>/p<N>-<M>-*.md` stay frozen as the
historical contract that was implemented; this file accumulates the
deltas so phase 5+ readers can find the live behavior without diffing
git history.

## 2026-05-03 — p9-fb-17 migration number V004 → V005

**Spec amended**: `tasks/p9/p9-fb-17-chat-session-storage.md` (frozen —
original contract calls the migration `V004__chat_sessions.sql`).

**Why renamed**: `V004__kv.sql` was already taken by p9-fb-19's `kv`
table for the `corpus_revision` counter (merged earlier the same day,
PR #78). Refinery numbers must be globally unique + monotonically
increasing, so chat-session storage shifts to `V005__chat_sessions.sql`.

**Behavior unchanged**: identical schema to the spec (chat_sessions +
chat_turns + idx_chat_turns_session); only the file name moved.

## 2026-05-03 — p9-fb-19 spec `index_version` → impl `corpus_revision` rename

**Spec amended**: `tasks/p9/p9-fb-19-search-cache.md` (frozen — original
contract uses `index_version` for the monotonic counter that ingest
bumps and `App::search` snapshots into its cache key).

**Why renamed**: design §9 already has an `index_version` identifier
(`IndexVersion` newtype, used in the §4.2 `index_id` recipe and on
`SearchHit`) — a *string label* for embedding-index identity. Reusing
the name for the monotonic u64 counter would collide silently on every
grep / type-search.

**Live name**: `corpus_revision` (added as a new row in design §9
versioning table). `SqliteStore::corpus_revision()` /
`bump_corpus_revision()` methods + `kv['corpus_revision']` row.
`SearchCacheKey.corpus_revision` field on `App`.

**Behavior unchanged**: every other detail (monotonic, ingest-commit
bump, in-key snapshot, no-bump on no-op reingest) matches the spec.

## 2026-05-02 — Config defaults: LLM = gemma4:e4b + workspace.root tilde expansion

**Discovered**: 사용자가 도그푸딩 환경에 `kebab init` 으로 생성된 `~/.config/kebab/config.toml` 검토하던 중.

**Symptom 1 (default 변경)**: `Config::defaults().models.llm.model` 가 `qwen2.5:14b-instruct`. OCR (P6-2) / caption (P6-3) 어댑터는 이미 `gemma4:e4b` 기본 사용 — 사용자가 OCR / caption / ask 모두 쓰려면 두 family 모델 (`qwen2.5` + `gemma4`) 을 모두 pull 해야 했음. 사용자 결정 (2026-05-02): **텍스트 LLM 기본도 gemma4 계열로 통일**.

**Symptom 2 (load-bearing)**: `workspace.root = "~/KnowledgeBase"` 같은 `~` 시작 경로가 코드 path 별로 다르게 처리:
- ✅ `kebab-source-fs::connector` 가 `expand_tilde` 사용 → walk 정상.
- ❌ `kebab-app::ingest_one_image_asset` 이 `PathBuf::from(&workspace.root)` 직접 → `~` 미확장 → ExtractContext 에 `~/KnowledgeBase` 그대로.
- ❌ `kebab-app::ingest_one_pdf_asset` 동일.
- ❌ `kebab-tui::search::handle_key_search` editor jump 도 동일 → `vim +12 ~/KnowledgeBase/foo.md` 의미 없는 경로 spawn.

**Fix**:
- `Config::defaults().models.llm.model` → `"gemma4:e4b"`. 코멘트가 OCR / caption family 통일 명시.
- kebab-app 의 image / pdf 분기 두 곳 모두 `expand_tilde(&app.config.workspace.root)` 호출 (markdown path 가 이미 쓰는 self-contained helper).
- kebab-tui::search jump 호출 site 가 `kebab_config::expand_path(&state.config.workspace.root, "")` 사용 — `expand_path` 가 `~` / `${XDG_DATA_HOME}` / `{data_dir}` 모두 처리하는 정식 helper.
- README / docs/SMOKE.md / docs/ARCHITECTURE.md 의 LLM 모델 예시 모두 `qwen2.5` → `gemma4` 갱신 (sync rule).

**Caveat (남은 inconsistency)**: kebab-app 자체 helper `expand_tilde` 와 kebab-config `expand_path` 가 별도 정의. 후자가 superset (env var + `{data_dir}` templating 추가). 통합은 P+ task — 본 PR scope 밖.

**Amends**:
- `Config::defaults` 의 `qwen2.5:14b-instruct` → `gemma4:e4b`.
- README 사전 요구 절 / docs/ARCHITECTURE 핵심 결정 표 / docs/SMOKE 의 ollama pull 예시 갱신.

## 2026-05-02 — P9-4 TUI Inspect: render_inspect generic + Search `i` entry + collapse simplification

**Discovered**: P9-4 implementation start.

**Symptom 1 (cosmetic)**: Same shape as P9-1/2/3 — `tasks/p9/p9-4-tui-inspect.md` § Public surface declares `render_inspect<B: ratatui::backend::Backend>(...)`. ratatui 0.28's `Frame` is backend-agnostic; the generic is unused.

**Symptom 2 (load-bearing)**: Spec § Behavior contract names `Search pressing 'i' (new key on Search pane) passes Chunk(selected_hit.chunk_id)` — but P9-2 (already merged) didn't include `i`. The Inspect entry from Search has to be wired retroactively.

**Symptom 3 (simplification)**: Spec § Behavior contract section on collapse: "focus is implicit by current scroll position; v1 may simplify by toggling all sections". Implementation takes the v1 path — `c` toggles all six sections (metadata / provenance / blocks / spans / text / embeddings) at once. Per-section focus is a P+ enhancement.

**Fix**:
- `render_inspect(f: &mut Frame, area: Rect, state: &App)` — no generic.
- New helper `kebab_tui::enter_inspect(state, target, return_to)` lifted out of pane handlers so both Library `Enter` and Search `i` use the same code path.
- Search pane gains `i` keybinding (pre-pass like `g`, plain modifier only — typing `i` in queries still reaches input). Esc returns the user to the originating pane stored in `return_to`.
- `InspectState.collapsed: HashSet<&'static str>` records collapsed section names. `c` flips all-collapsed ↔ all-expanded based on whether any are currently collapsed.
- `q` joins `Esc` as the back key (Inspect is the only read-only terminal pane in v1, so `q` is unambiguous).

**Trust note**: Embedding inspection is intentionally left as "(not loaded — out of v1 scope)" per spec § Out of scope. The full embedding-record fetch would require an extra facade method (`kebab-app::inspect_embedding`) that is not in the P5/P6/P7 facade surface. P+ task.

**Amends**:
- tasks/p9/p9-4-tui-inspect.md (`render_inspect` non-generic; collapse simplification; entry helper).
- tasks/p9/p9-2-tui-search.md (Search pane gains `i` for chunk inspect — was not in original p9-2 spec).

## 2026-05-02 — P9-3 TUI Ask: render_ask generic + command-vs-insert key disambiguation

**Discovered**: P9-3 implementation start.

**Symptom 1 (cosmetic)**: Same shape as P9-1 / P9-2 — `tasks/p9/p9-3-tui-ask.md` § Public surface declares `render_ask<B: ratatui::backend::Backend>(...)`. ratatui 0.28's `Frame` is backend-agnostic; the generic is unused and clippy `-D warnings` rejects it.

**Symptom 2 (load-bearing)**: Spec key bindings list `e` (toggle explain), `j` / `k` (scroll). All three collide with typing — a user asking "explain javascript" would have the leading `e` toggle explain mode, then `j` scroll, etc. The Library / Search panes don't hit this because their input is either filter-overlay-gated (Library) or the whole pane *is* an input (Search). Ask has both an always-visible input bar AND scrollable answer area.

**Fix**:
- `render_ask(f: &mut Frame, area: Rect, state: &App)` — no generic.
- `e` / `j` / `k` use the **input-empty heuristic**: when `state.ask.input.is_empty()`, they act as command keys (toggle explain / scroll up/down). When the input has content, they reach the input buffer as ordinary characters. Vim's "command vs insert mode" applied at the keystroke level — the user starts typing, the keys behave as text; clears the input (Backspace to empty), the keys behave as commands again.
- `Enter` always submits (when input non-empty AND not already streaming). `Esc` always returns to Library + clears `streaming/rx/thread` (best-effort cancel — worker keeps running but its result is dropped, per spec § Risks "fire and forget").

**Trust note**: The worker thread holds the `mpsc::Sender<String>`; the pane keeps `rx` and drains via `try_iter` once per render frame (no blocking). On Esc we `take()` the `JoinHandle` without `join` so quit is instant; the kernel reaps the orphan when its `ask_with_config` returns.

**Amends**:
- tasks/p9/p9-3-tui-ask.md (`render_ask` non-generic; `e`/`j`/`k` empty-input gating).

## 2026-05-02 — P9-2 TUI Search: render_search generic + jump_to_citation workspace_root

**Discovered**: P9-2 implementation start.

**Symptom 1 (cosmetic)**: Same shape as the P9-1 entry — `tasks/p9/p9-2-tui-search.md` § Public surface declares `render_search<B: ratatui::backend::Backend>(...)`. ratatui 0.28's `Frame` is backend-agnostic; the generic is unused and clippy `-D warnings` rejects it.

**Symptom 2 (load-bearing)**: Spec literal `jump_to_citation(citation: &Citation, editor_env: &str) -> Result<()>`. `Citation.path()` returns a `WorkspacePath` (workspace-relative), but the editor child needs an absolute path — `editor_env` does NOT carry the workspace root. The signature is unimplementable as written.

**Fix**:
- `render_search(f: &mut Frame, area: Rect, state: &App)` — no generic.
- `jump_to_citation(citation: &Citation, editor_env: &str, workspace_root: &Path) -> Result<()>` — added `workspace_root` arg. The run-loop call site reads `state.config.workspace.root`.
- `build_jump_command` extracted as a pure helper so unit tests can assert the `(program, args)` shape without spawning a child process. Lives next to `jump_to_citation` in `kebab-tui::search`.

**Trust note**: The `g` keybinding suspends the TUI (drops raw mode + LeaveAlternateScreen), runs the editor synchronously, then RAII-restores raw mode + AltScreen on return — even on panic in the child. Same shape as `kebab-tui::terminal::TuiTerminal::Drop` from P9-1.

**Amends**:
- tasks/p9/p9-2-tui-search.md (`render_search` non-generic; `jump_to_citation` adds `workspace_root`).

## 2026-05-02 — P9-1 TUI Library: render_library generic + test seam

**Discovered**: P9-1 implementation start.

**Symptom 1 (cosmetic)**: `tasks/p9/p9-1-tui-library.md` § Public surface declares `pub fn render_library<B: ratatui::backend::Backend>(f: &mut ratatui::Frame, area: Rect, state: &App)`. ratatui 0.28 dropped the backend generic from `Frame` (it's bound at `Terminal` initialisation, not at the render call site). The `<B: Backend>` parameter would be unused on the function and clippy `-D warnings` rejects unused generic parameters.

**Fix 1**: `render_library(f: &mut Frame, area: Rect, state: &App)` — no generic parameter. The function still works against any backend the `Terminal` was opened with (CrosstermBackend in production, TestBackend in snapshot tests). No call-site impact.

**Symptom 2 (test seam)**: `LibraryState.inner` is `pub(crate)` per the spec's parallel-safety contract — p9-2/3/4 must not mutate `LibraryState` directly. Snapshot tests in `tests/library.rs` (an integration test, NOT a unit test in the same module) cannot reach `pub(crate)` fields, so they cannot inject docs without going through `kebab-app::list_docs_with_config` (which would stand up a TempDir SQLite KB just to populate three rows).

**Fix 2**: new `App::populate_library_for_testing(&mut self, Vec<DocSummary>)` marked `#[doc(hidden)]`. Lets snapshot tests inject docs hermetically while keeping the parallel-safety boundary intact for normal callers (the helper is officially "test seam, not part of the UI API"). Same shape as `kebab-app::*_with_config` test seams from P3-5.

**Amends**:
- tasks/p9/p9-1-tui-library.md (`render_library` no longer generic; `populate_library_for_testing` test seam added).

## 2026-05-02 — P7-3 PDF ingest wiring: chunker_version deviation + storage UNIQUE bug

**Discovered**: P7-3 implementation start.

**Symptom 1 (deviation, intentional)**: `tasks/p7/p7-3-pdf-ingest-wiring.md` § Chunker selection notes that `config.chunking.chunker_version` is single-valued and serves the markdown path only. PDF ingest hard-codes `pdf-page-v1` regardless of the config value. A user who reads `config.toml` and sees `chunker_version = "md-heading-v1"` reasonably assumes PDFs use the same — they don't.

**Fix 1**: `ingest_one_pdf_asset` (in `kebab-app::lib.rs`) instantiates `PdfPageV1Chunker` directly. The `Chunk.chunker_version` field on emitted PDF chunks records `pdf-page-v1` truthfully. A future P+ task (chunker registry) either splits `Config::chunking.chunker_version` per medium or replaces the dispatch with a runtime registry. No HOTFIX entry needed once that happens — this entry is the cross-reference.

**Symptom 2 (storage-layer bug, fixed in same PR)**: P7-3's edited-bytes re-ingest test (`re_ingest_edited_pdf_produces_new_doc_id`) tripped on `sqlite error: UNIQUE constraint failed: assets.workspace_path: Error code 2067`. The assets table has a UNIQUE constraint on `workspace_path`, but `upsert_asset_row` (in `kebab-store-sqlite::store.rs`) only handles `ON CONFLICT(asset_id)`. When a file's bytes change, the new BLAKE3 produces a new `asset_id` while the `workspace_path` stays the same — INSERT picks the new asset_id branch, then trips the secondary UNIQUE on `workspace_path`.

**Why it didn't surface earlier**: No existing test (markdown / image) exercised edited-bytes re-ingest. The image path's `re_ingest_image_produces_updated_with_same_doc_id` uses identical bytes (same asset_id → `ON CONFLICT(asset_id)` catches it). Real-world editing of a tracked file would hit the same bug across all media types.

**Fix 2** (P7-3 implementation PR): new `purge_orphan_at_workspace_path` helper in `kebab-store-sqlite::store.rs`. Runs immediately before each `upsert_asset_row` call (both `put_asset_with_bytes` paths AND `DocumentStore::put_asset`). It:
1. SELECTs the stale row at `workspace_path` whose `asset_id` differs from the incoming one (none → no-op return).
2. DELETEs from `documents WHERE asset_id = stale` — `documents.asset_id ON DELETE RESTRICT` requires the documents go first; CASCADE on documents → `blocks` / `chunks` / `embedding_records` sweeps the dependent rows in the same statement.
3. DELETEs the stale `assets` row, freeing the `workspace_path` slot.
4. If the stale storage was `copied`, best-effort removes the byte file at `storage_path` so `data_dir/assets/` does not accumulate orphans across edits.

**Vector store cleanup (closed by follow-up PR)**: `embedding_records.chunk_id` CASCADE clears the SQLite side, but LanceDB lives in a separate store. The follow-up PR adds:
- `VectorStore::delete_by_chunk_ids` trait method (default impl no-op for older fakes).
- `LanceVectorStore::delete_by_chunk_ids` iterates every `chunk_embeddings_*` table in the connection and runs `Table::delete("chunk_id IN (...)")` in batches of 200.
- `SqliteStore::stale_chunk_ids_at(workspace_path, new_asset_id)` SELECT helper (read-only) that fetches the stale chunk_ids before they get cascade-deleted.
- `kebab-app::purge_vector_orphans_for_workspace_path` orchestrator. Each per-medium ingest helper (`ingest_one_asset` markdown branch, `ingest_one_image_asset`, `ingest_one_pdf_asset`) calls it immediately before `put_asset_with_bytes` so the stale Lance rows go away in lockstep with the SQLite cascade.

Verified end-to-end via the SMOKE runbook: edit a tracked PDF → re-ingest → vector search for the old body text returns the *new* chunks (semantic nearest-neighbour) and the old chunk_ids are not present in the vector store.

The previously-`#[ignore]`d `re_ingest_edited_pdf_produces_new_doc_id` integration test runs by default after this fix, plus a dedicated unit test `put_asset_with_bytes_sweeps_workspace_path_orphan` in `kebab-store-sqlite::tests::asset_writer` that exercises the no-documents flavour. Verified end-to-end via the SMOKE runbook: `kebab ingest` → edit a tracked PDF → `kebab ingest` reports `new=1` for that asset (rest `updated`) and the prior doc/chunks are gone from `inspect` / `list docs`.

**Amends**:
- tasks/p7/p7-3-pdf-ingest-wiring.md (chunker_version deviation; edited-bytes test runs).
- crates/kebab-store-sqlite (new `purge_orphan_at_workspace_path` helper called from both `put_asset_with_bytes` branches and `DocumentStore::put_asset`).
- crates/kebab-store-sqlite/tests/asset_writer.rs (`put_asset_with_bytes_sweeps_workspace_path_orphan` replaces the prior orphan-cleanup-on-failure test, since the failure path no longer exists).
- docs/SMOKE.md (note that edited-PDF re-ingest produces `new=1` rather than an error).

## 2026-05-02 — P7-2 pdf-page-v1: chunk_id collision + BYTES_PER_TOKEN

**Discovered**: P7-2 implementation start.

**Symptom 1 (load-bearing)**: `tasks/p7/p7-2-pdf-page-chunker.md` § Behavior contract literally says `chunk_id` per design §4.2 with `(doc_id, "pdf-page-v1", block_ids, policy_hash)`. But unlike `md-heading-v1` (which always emits at most one chunk per atomic block), `pdf-page-v1` splits one page-block into multiple chunks when page text exceeds the byte budget. All sub-chunks of the same page have identical `block_ids` → identical `chunk_id` collisions, breaking the §3.5 invariant that `chunk_id` is a primary key.

**Symptom 2 (cosmetic)**: Spec text says `token_estimate = byte_len / 4` and "matches `md-heading-v1` proxy". Looking at the actual md-heading-v1 source (`crates/kebab-chunk/src/md_heading_v1.rs:17`), the constant is `BYTES_PER_TOKEN = 3` (chosen to cover Korean ≈ 3 b/tok and over-estimate English ≈ 4 b/tok). Spec's "/4" claim is inconsistent with the implementation it claims to match.

**Root cause**: §4.2 chunk_id recipe was designed assuming one-chunk-per-block-set. Page-aware chunking violates that assumption.

**Fix** (PR #38, feat/p7-2-pdf-page-chunker):

- **Per-chunk policy_hash variant**: feed `format!("{base_policy_hash}#c{char_start}")` into `id_for_chunk`'s `policy_hash` slot so chunks within the same page get distinct `chunk_id`s. The §4.2 recipe itself stays unchanged — only the *input* to one of its slots differs per chunk. The unmodified `base_policy_hash` is still stored in `Chunk.policy_hash` so the field still answers "what policy was active" (workspace-wide policy invalidation lookups continue to work).
- **`BYTES_PER_TOKEN = 3`** (matches md-heading-v1 actual code, not spec literal). Cross-chunker policy fingerprint identity is verified by a unit test: `policy_hash_matches_md_heading_v1_for_identical_policy`.

**Trust note**: The per-chunk hash variant is opaque (`#c<n>` is just a marker, not interpretable as char_start by downstream tools — they read `Chunk.source_spans[0].char_start` for that). Downstream identifier comparisons on `chunk_id` continue to work as opaque blake3 hashes.

**Amends**:
- tasks/p7/p7-2-pdf-page-chunker.md (chunk_id recipe per-chunk variant; BYTES_PER_TOKEN = 3 not 4).

## 2026-05-02 — P6-3 caption: GenerateRequest.images + cargo feature dropped

**Discovered**: P6-3 implementation start.

**Symptom 1**: `tasks/p6/p6-3-caption-adapter.md` § Public surface declares `caption_image(llm: &dyn kebab_core::LanguageModel, ...)`, but the frozen `LanguageModel` trait + `GenerateRequest` from p4-1 carry no vision input. The spec's behavior contract ("the adapter is responsible for rendering the prompt to wire") implicitly relied on a trait extension that p4-1 never specced.

**Symptom 2**: Spec § Definition of Done asks for `cargo check -p kebab-parse-image --features caption` — i.e. a cargo feature gate. The captioning module's only extra deps are `base64` + `image` + the `kebab-llm` trait, all already pulled in by P6-2. A cargo feature would only complicate the build matrix without saving meaningful binary weight.

**Root cause**: Two small spec gaps that resolve cleanly together — extend the `LanguageModel` trait once for vision routing, and collapse compile-time + runtime gating into a single runtime gate.

**Fix** (PR #34, feat/p6-3-caption-adapter):
- `kebab-core::GenerateRequest` gains an `images: Vec<String>` field (`#[serde(default)]` for backward compat with pre-P6 wire payloads / snapshots). Empty for the text-only RAG path; populated with one or more base64 strings by vision-aware callers.
- `kebab-llm-local::OllamaLanguageModel` routes `req.images` onto the wire as `images: [base64, ...]` (Ollama's vision channel). The wire shape stays byte-identical for empty `images` because the field uses `#[serde(skip_serializing_if = "<[String]>::is_empty")]`.
- `kebab-parse-image::caption` module: `caption_image` / `apply_caption` build `GenerateRequest { images: vec![b64], temperature: 0.0, seed: 0, ... }` and accept any `&dyn LanguageModel`. Korean / English prompt branch picked from `lang_hint`.
- Cargo feature `caption` is **not** introduced — the runtime gate `config.image.caption.enabled = false` (default OFF) suffices.
- All existing `GenerateRequest { ... }` literals (kebab-rag, kebab-llm tests, kebab-llm-local tests) gained `images: Vec::new()` to satisfy the new field.

**Trust note**: Captions stay explicitly model-generated. `ModelCaption.model_version` carries `"<provider>/<prompt_template_version>"` (e.g. `"ollama/caption-v1"`) so a regression in either prompt or model is auditable from the wire.

**`model_version` shape deviation**: spec literal says `model_version: llm.model_ref().provider` (provider as a coarse version proxy). We extend to `<provider>/<prompt_template_version>` because prompt template churn is a real regression vector independent of the model — pinning both axes in one string lets `kebab-eval` (P5) detect either drift without a schema bump. Spec already left the door open ("if a vision model exposes a stable revision, prefer that"); the prompt template version is the closest stable revision we have today. Future PaddleOCR / Apple Vision adapters that expose a real model revision string can substitute it for `prompt_template_version` without breaking the wire shape.

**Amends**:
- tasks/p4/p4-1-llm-trait.md (`GenerateRequest` schema gained `images: Vec<String>`).
- tasks/p4/p4-2-ollama-adapter.md (request body now optionally includes `images: [...]`).
- tasks/p6/p6-3-caption-adapter.md ("Definition of Done" cargo feature `caption` dropped; runtime gate is the only feature gate).

## 2026-05-02 — P6-2 default OCR engine: Tesseract → Ollama-vision

**Discovered**: P6-2 implementation start.

**Symptom**: The original `tasks/p6/p6-2-ocr-adapter.md` spec lists Tesseract as the default OCR engine (`tesseract = "0.13"`, feature `tesseract`, default ON). Bringing Tesseract online requires installing `libtesseract-dev` (and `tesseract-ocr-kor` for the spec-default Korean languages set) on every dev / CI host. The kebab dev environment intentionally avoids system-package installs, so the Tesseract Rust bindings can't link.

**Root cause**: Spec was written assuming a Linux host with `apt install tesseract-ocr-*` available. The reality of single-developer local-first KB is that the same box also runs the Ollama vision endpoint already wired by P4-2 — using it for OCR adds zero new system dependencies.

**Fix** (PR #33, feat/p6-2-ocr-adapter):
- New `OllamaVisionOcr` adapter under `crates/kebab-parse-image/src/ocr.rs`. Implements the spec's `OcrEngine` trait by POSTing the image (base64) to `<endpoint>/api/generate` with a transcription prompt against `gemma4:e4b` (default) or any other vision-capable Ollama model.
- New `kebab-config::ImageCfg.ocr` block (`enabled`, `engine`, `model`, `endpoint`, `languages`, `max_pixels`). `enabled` defaults to `false` because OCR adds a model call per asset; `engine` defaults to `"ollama-vision"`. `endpoint` falls back to `models.llm.endpoint` when empty so the same Ollama host serves both LLM and OCR.
- The `OcrEngine` trait is unchanged from the spec — Tesseract / Apple Vision / PaddleOCR engines plug in as future feature-gated alternatives without touching the extractor or chunker. The trait abstraction is the part the spec actually demanded; only the choice of default implementation changes.
- Tests cover wiremock unit paths (200 happy / 5xx / 200 error envelope / empty response / downscale honours `max_pixels`), `apply_ocr` provenance + error handling, and an opt-in `KEBAB_OCR_INTEGRATION=1` integration test that hits a real Ollama endpoint with a generated `"Hello World 2026"` PNG. Tesseract feature-gated tests from the original spec are deferred to whenever someone is willing to bring `libtesseract` to CI.

**Trust note**: The original spec marked `OcrText` as "observed text (high trust)" to distinguish it from `ModelCaption`. With an LLM-driven default the line blurs — vision LMs can hallucinate. We kept `OcrText.engine = "ollama-vision"` so consumers can decide trust by engine identity. Future Tesseract / Apple Vision adapters write a different `engine` string and downstream code can branch.

**Amends**: tasks/p6/p6-2-ocr-adapter.md (default engine; "Allowed dependencies" list — `reqwest` + `base64` replace `tesseract`; "Apple Vision" feature gate deferred; `min_confidence` config field dropped because the LM doesn't expose per-region confidence).

## 2026-05-01 — `--config` flag silently ignored across all kebab-cli subcommands

**Discovered**: post-P3-5 manual smoke at `/tmp/kebab-smoke/`.

**Symptom**: `kebab --config /path/to/config.toml ingest|search|list|inspect|doctor` ignored the flag and fell back to `~/.config/kebab/config.toml` (XDG default). Users had to use `KEBAB_*` env vars to point at a non-default config.

**Root cause**: `kebab-cli` read `cli.config` only inside `Cmd::Ingest` to build `SourceScope`, then called bare `kebab_app::ingest(scope, summary_only)` which internally re-loaded `Config::load(None)` (XDG path). Same pattern in `Cmd::Search` / `List` / `Inspect` / `Doctor`. P3-5 introduced `*_with_config` test seams via `#[doc(hidden)] pub fn` but kebab-cli never used them.

**Fix** (PR #20, fix/cli-config-flag-and-search-output):
- `kebab-cli` now builds the Config once via `Config::load(cli.config.as_deref())` at the top of every subcommand and threads it into `kebab_app::*_with_config(cfg, ...)` instead of `kebab_app::*(...)`.
- `kebab_app::doctor()` rewritten as `doctor_with_config_path(Option<&Path>)` that reports the actual path probed and hard-fails when `--config <path>` doesn't exist (defaults would otherwise mask user intent).
- `kebab-app` module doc-comment updated: `#[doc(hidden)] pub fn *_with_config` is no longer "test-only seam" — it's the official "config-explicit" API consumed by CLI `--config`, integration tests, and TUI sessions.
- Same PR also improved `kebab search` printer: `{:.4}` score formatting (RRF range collapses on `{:.2}`) and `> heading_path` suffix so chunks from the same document are visually distinct.

**Amends**: tasks/p3/p3-5-app-wiring.md (the test seam was always meant to be the config-explicit API; only the doc-comment lied).

### 2026-05-01 — `--config` regression in `kebab ask` (P4-3 follow-up)

**Discovered**: post-P4-3 manual smoke against 192.168.0.47 Ollama with `gemma4:26b`.

**Symptom**: `kebab --config <path> ask` returned `model.id = qwen2.5:14b-instruct` (XDG default model) and `score_gate = 0.30` (XDG default), instead of `gemma4:26b` / `0.05` from the explicit config. P4-3 added the ask body but kebab-cli's `Cmd::Ask` arm still called bare `kebab_app::ask(query, opts)` — same regression class as the P3-5 fix above, just missed when ask was wired.

**Fix** (PR #24, fix/cli-ask-honor-config-flag):
- `kebab-cli` builds `Config::load(cli.config.as_deref())` once at the top of `Cmd::Ask` and calls `kebab_app::ask_with_config(cfg, query, opts)`.

**Amends**: tasks/p4/p4-3-rag-pipeline.md.

## 2026-05-01 — RRF `fusion_score` incompatible with `config.rag.score_gate` default

**Discovered**: post-P4-3 manual smoke. Top hybrid result returned `fusion_score = 0.0164` against `score_gate = 0.05` → ScoreGate refusal on every hybrid query.

**Root cause**: RRF formula `score(c) = Σ 1/(k_rrf + rank_m(c))` produces values bounded by `num_retrievers / (k_rrf + 1)`. With `num_retrievers = 2` and the default `k_rrf = 60`, the upper bound is `2/61 ≈ 0.0328`. The default `config.rag.score_gate = 0.05` was calibrated for vector / lexical scores already in `[0, 1]` and silently refused every hybrid query. `fusion_score` was also incomparable across modes — Lexical / Vector lived in `[0, 1]`, Hybrid lived in `(0, 0.033]`.

**Fix** (PR #25, fix/rrf-fusion-score-normalize-and-docs):
- `crates/kebab-search/src/hybrid.rs` divides every raw RRF score by `2 / (k_rrf + 1)` so `fusion_score` always lives in `[0, 1]` regardless of mode. Both retrievers contributing rank 1 normalises to `1.0`; chunks present in only one retriever cap around `0.5`. RRF's rank-ordering invariants are preserved (same constant divides every score), so sort + tiebreak behaviour is identical.
- One unit test (`rrf_formula_matches_known_value`) updated to expect the normalised value `(1/61 + 1/62) / (2/61) ≈ 0.9919`.
- The integration snapshot `crates/kebab-search/tests/fixtures/search/hybrid/run-1.json` already used presence checks (`fusion_score_positive: true`) rather than absolute values, so it didn't need regeneration.

**Why not a per-mode `score_gate` config**: separate `lexical_score_gate / vector_score_gate / hybrid_score_gate` would force every downstream consumer (CLI, eval, TUI) to know which mode picks which threshold. Normalising the score itself is a one-line change at the source and makes `Answer.retrieval.score_gate` semantically meaningful without per-mode bookkeeping.

**Amends**: tasks/p3/p3-4-hybrid-fusion.md (RRF formula now divides by `2/(k_rrf+1)` after summation), tasks/phase-3-vector-hybrid.md (RRF section).

**Verification**: post-fix smoke at `/tmp/kebab-smoke/` with default `score_gate = 0.05` succeeded across four scenarios — Korean→Korean, English→English, cross-language, and out-of-corpus refusal.

## How to add an entry

Each fix gets a dated subsection with five fields:

- **Discovered**: when / how the bug surfaced (smoke, integration test, user report).
- **Symptom**: what the user saw / what was wrong.
- **Root cause**: the actual code or design issue.
- **Fix**: PR number / branch + a one-paragraph summary of the change.
- **Amends**: which `tasks/p<N>/...` spec docs the fix retroactively contradicts. Spec text stays frozen; this log is the live source of truth for post-merge deltas.

If a fix is large enough that the original spec is no longer a useful reference, promote the entry into a new task spec (e.g., `p<N>-<M+1>-<topic>.md`) and link from here.
