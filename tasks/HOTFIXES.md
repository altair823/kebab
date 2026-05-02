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

## 2026-05-02 — P7-3 PDF ingest wiring: chunker_version deviation + storage UNIQUE bug

**Discovered**: P7-3 implementation start.

**Symptom 1 (deviation, intentional)**: `tasks/p7/p7-3-pdf-ingest-wiring.md` § Chunker selection notes that `config.chunking.chunker_version` is single-valued and serves the markdown path only. PDF ingest hard-codes `pdf-page-v1` regardless of the config value. A user who reads `config.toml` and sees `chunker_version = "md-heading-v1"` reasonably assumes PDFs use the same — they don't.

**Fix 1**: `ingest_one_pdf_asset` (in `kebab-app::lib.rs`) instantiates `PdfPageV1Chunker` directly. The `Chunk.chunker_version` field on emitted PDF chunks records `pdf-page-v1` truthfully. A future P+ task (chunker registry) either splits `Config::chunking.chunker_version` per medium or replaces the dispatch with a runtime registry. No HOTFIX entry needed once that happens — this entry is the cross-reference.

**Symptom 2 (storage-layer bug, exposed but not fixed by P7-3)**: P7-3's edited-bytes re-ingest test (`re_ingest_edited_pdf_produces_new_doc_id`) tripped on `sqlite error: UNIQUE constraint failed: assets.workspace_path: Error code 2067`. The assets table has a UNIQUE constraint on `workspace_path`, but `upsert_asset_row` (in `kebab-store-sqlite::store.rs:305`) only handles `ON CONFLICT(asset_id)`. When a file's bytes change, the new BLAKE3 produces a new `asset_id` while the `workspace_path` stays the same — INSERT picks the new asset_id branch, then trips the secondary UNIQUE on workspace_path.

**Why it didn't surface earlier**: No existing test (markdown / image) exercises edited-bytes re-ingest. The image path's `re_ingest_image_produces_updated_with_same_doc_id` uses identical bytes (same asset_id → ON CONFLICT(asset_id) catches it). Real-world editing of a tracked file would hit the same bug across all media types.

**Fix 2 (deferred)**: Storage-layer fix is out of scope for P7-3. The P7-3 implementation PR `#[ignore]`s the `re_ingest_edited_pdf_produces_new_doc_id` test with a doc-comment pointing here. A P+ storage task either:
- Adds `ON CONFLICT(workspace_path) DO UPDATE` alongside the existing `ON CONFLICT(asset_id)` clause (DELETE-the-old + INSERT-the-new in a single statement, since UPSERT can only target one conflict path).
- Or drops the UNIQUE constraint on `assets.workspace_path` and relies on application-level uniqueness (workspace_path → asset_id mapping in a separate index table).

**Amends**:
- tasks/p7/p7-3-pdf-ingest-wiring.md (chunker_version deviation, edited-bytes test ignored).
- (Implicitly) every previous task spec that assumed `assets.workspace_path` UNIQUE was safe — the constraint is in fact too strict for the byte-edit re-ingest case.

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
