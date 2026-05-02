---
phase: P7
component: kebab-app (PDF ingest dispatch + chunker selection)
task_id: p7-3
title: "Wire PdfTextExtractor + PdfPageV1Chunker into kebab-app::ingest end-to-end"
status: planned
depends_on: [p7-1, p7-2, p1-6, p3-5, p6-4]
unblocks: []
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§3.4 SourceSpan::Page, §3.5 Chunk, §6.1 ingest pipeline, §7.2 Extractor/Chunker traits, §9.2 PDF text extraction]
---

# p7-3 — PDF ingest wiring (kebab-app)

## Goal

Make `kebab ingest` end-to-end functional for PDF assets. P7-1 (`PdfTextExtractor`) and P7-2 (`PdfPageV1Chunker`) each ship a tested library; this task connects the wires from `kebab-source-fs` (which already classifies `.pdf` as `MediaType::Pdf`) through `kebab-app::ingest_with_config` to `kebab-store-sqlite` + `kebab-store-vector`, so a user running `kebab ingest` against a workspace containing PDF papers / books / reports sees those assets indexed and searchable, with page-level citations preserved.

## Why now / why this size

P7-1 / P7-2 deliver value only after `kebab-app::ingest` learns to dispatch on `MediaType::Pdf`. The wiring is small — one new dispatch arm in `ingest_one_asset`, one new private helper `ingest_one_pdf_asset`, and a per-medium chunker selection step — but it is materially user-facing: without it, the entire P7 phase is invisible from the CLI. P6-4 (image wiring) established the pattern; P7-3 follows it identically with two structural differences:

1. PDF chunking uses a **separate** chunker (`pdf-page-v1`) rather than an in-place branch inside `md-heading-v1`. The chunker selection happens at dispatch time keyed on `MediaType`.
2. PDF documents commonly produce **many** chunks per asset (one per page, sometimes more for long pages); the embedder loop must scale to hundreds of chunks per doc without breaking the per-asset transaction.

Pulling this into its own task keeps P7-1 / P7-2 specs frozen as written while letting integration evolve under its own contract.

## Allowed dependencies

`kebab-app` 의 현재 Cargo.toml + `kebab-parse-pdf` 한 줄 추가.

- `kebab-core`
- `kebab-config`
- `kebab-source-fs`
- `kebab-parse-md`, `kebab-parse-types`
- `kebab-normalize`
- `kebab-chunk` (uses both `MdHeadingV1Chunker` AND `PdfPageV1Chunker` — chunker selection at dispatch time)
- `kebab-store-sqlite`, `kebab-store-vector`
- `kebab-search`
- `kebab-embed`, `kebab-embed-local`
- `kebab-llm`, `kebab-llm-local`
- `kebab-rag`
- `kebab-parse-image`
- **`kebab-parse-pdf` (NEW — added by this task)**
- `anyhow`, `serde_json`, `tracing`

## Forbidden dependencies

- `kebab-tui`, `kebab-desktop` (P9 미시작 — UI crate 가 ingest 호출하면 layering 위반).
- `kebab-eval` (cycle 위험 — eval 이 ingest 를 호출).
- 본 task 안에서 PDF parsing / chunking 로직을 재구현 금지. `kebab-parse-pdf` + `kebab-chunk::PdfPageV1Chunker` 의 thin dispatch + glue 만 허용.

## Inputs

| input | type | source |
|-------|------|--------|
| workspace assets | `RawAsset` stream | `kebab-source-fs::SourceConnector::scan` (already classifies `MediaType::Pdf`) |
| PDF bytes | `&[u8]` | filesystem read in `kebab-app` |
| `Config` | `kebab_config::Config` | CLI `--config` flag → `Config::load` |
| `ChunkPolicy` | `kebab_core::ChunkPolicy` | derived from `config.chunking` (existing) |

## Outputs

| output | type | downstream |
|--------|------|------------|
| `CanonicalDocument` per PDF | written via `DocumentStore::put_document` | `kebab-store-sqlite` |
| `Vec<Chunk>` per PDF (≥1 chunk per non-empty page) | `kebab-store-sqlite::put_chunks` + `kebab-store-vector::upsert` | `kebab-store-sqlite` + `kebab-store-vector` |
| Updated `IngestReport` counters | `scanned / new / updated / skipped / errors` | wire output (`ingest_report.v1`) |

## Public surface

No new public types. The wiring exists inside `kebab-app::ingest_with_config` (and its private helpers). One new private function:

```rust
// crates/kebab-app/src/lib.rs (additions only — sketch)

/// P7-3: process one `MediaType::Pdf` asset end-to-end.
fn ingest_one_pdf_asset(
    app: &App,
    asset: &RawAsset,
    chunk_policy: &ChunkPolicy,
    embedder: Option<&Arc<dyn Embedder + Send + Sync>>,
    vector_store: Option<&Arc<kebab_store_vector::LanceVectorStore>>,
    existing_doc_ids: &std::collections::HashSet<String>,
) -> anyhow::Result<kebab_core::IngestItem> { ... }
```

`ingest_one_asset` gets a new `match` arm:

```rust
match &asset.media_type {
    MediaType::Markdown => { /* existing fall-through */ }
    MediaType::Image(_) => return ingest_one_image_asset(...),
    MediaType::Pdf => return ingest_one_pdf_asset(...),  // NEW
    _ => return Ok(IngestItem { kind: Skipped, ... }),
}
```

## Behavior contract

### Ingest dispatch (kebab-app)

- For each `RawAsset`:
  - `MediaType::Markdown` → existing markdown extractor path (unchanged).
  - `MediaType::Image(_)` → P6-4 image branch (unchanged).
  - `MediaType::Pdf` → new PDF branch (this task).
  - `MediaType::Audio(_) | MediaType::Other(_)` → `skipped += 1` (existing behaviour).
- **Operational jump**: before this task, `MediaType::Pdf` fell into the `_` arm and was counted as `Skipped`. After merge, every PDF asset shifts from `skipped` to `scanned` / `new` / `updated`. A workspace with N PDF files reports `skipped` decreasing by N and `scanned` (and `new` on the first ingest after merge) increasing by N — flag this in the implementation PR description so eval / smoke / runtime users can interpret the one-time discontinuity.
- PDF branch (`ingest_one_pdf_asset`):
  1. Read bytes via `std::fs::read` (consistent with markdown / image branches).
  2. Build `kebab_core::ExtractContext { asset, workspace_root, config: &ExtractConfig::default() }`.
  3. Call `PdfTextExtractor::new().extract(&ctx, &bytes)`. Failure (`Err(_)`) → `IngestItemKind::Error` with the formatted error in `IngestItem.error`. Continue to next asset (do not abort the whole ingest).
     - Encrypted PDFs hit this branch (P7-1 returns `Err` with the `qpdf --decrypt` hint preserved verbatim — the operator sees the actionable message in the `kebab ingest` output).
     - Corrupt / non-PDF bytes likewise.
  4. The returned `CanonicalDocument` may carry per-page `Provenance::Warning` events (P7-1 emits one for each empty / extract-failed page, marked "scanned candidate"). Pass these through unchanged — the chunker correctly emits 0 chunks for empty pages, so the asset is still indexed but those pages are not searchable until a future scanned-PDF OCR fallback lands (out of scope here).
  5. Pass the `CanonicalDocument` to **`PdfPageV1Chunker`** (NOT `MdHeadingV1Chunker` — chunker selection is keyed on `MediaType::Pdf`). The chunker validates that every block is `Block::Paragraph` with `SourceSpan::Page`; if validation fails (which would mean P7-1's contract drifted), the chunker's error propagates up as `IngestItemKind::Error`.
  6. Persist `CanonicalDocument` + `Vec<Chunk>` via the same `DocumentStore::put_document` + `put_chunks` calls the markdown branch uses.
  7. Embed each chunk if `embedder.is_some()`. Each PDF chunk gets one vector — the embedder loop processes them in batches of `config.indexing.max_parallel_embeddings` like markdown chunks (no PDF-specific batching).

### Chunker selection

- Per-medium chunker selection is the new architectural piece. Today the markdown branch hard-codes `MdHeadingV1Chunker`; the PDF branch hard-codes `PdfPageV1Chunker`. There is no runtime config switch — the medium → chunker mapping is compiled in.
- A future task (P+ "chunker registry") may make this configurable, at which point the mapping moves to `Config::chunking.chunker_for_media`. P7-3 deliberately does not introduce that config slot — premature config surface.
- `config.chunking.chunker_version` is a fingerprint, not a dispatcher. Markdown sets it to `"md-heading-v1"`, PDF would set it to `"pdf-page-v1"` — but in the current `Config` schema the field is single-valued and serves the markdown path only. **Deviation logged in HOTFIXES**: PDF ingest ignores `config.chunking.chunker_version`, hard-codes `pdf-page-v1`. A future P+ task either splits the config field per medium or builds the chunker registry above.

### Determinism stress

- The existing markdown path's `kb-normalize::build_canonical_document` and the image path's `ingest_one_image_asset` share one `OffsetDateTime::now_utc()` reading per Provenance event group. P7-1's extractor already shares one `now` reading across its Discovered + Parsed + per-page Warning events. The PDF branch must not insert a second `now()` between extract and chunk — chunking is a pure function of the `CanonicalDocument`, so this constraint is structural, not stylistic.
- Re-ingest of the same PDF bytes produces identical `doc_id`, identical `block_id`s (per-page deterministic via `id_for_block(doc_id, "paragraph", &[], page-1, span)`), and identical `chunk_id`s (P7-2's per-chunk policy_hash variant `#c{char_start}` makes `chunk_id` deterministic-and-collision-free across the document).

### Failure semantics summary

| failure | counter | doc stored? | provenance |
|---|---|---|---|
| `PdfTextExtractor::extract` Err (corrupt header / not a PDF) | `errors+=1` | no | n/a (no doc emitted) |
| `PdfTextExtractor::extract` Err (encrypted PDF) | `errors+=1` | no | n/a — error message includes `qpdf --decrypt` hint |
| `PdfPageV1Chunker::chunk` Err (validates non-Page span / non-Paragraph block) | `errors+=1` | no | n/a — defensive validation: fires on P7-1 contract drift OR future routing bug (e.g. a chunker registry mis-routes a markdown doc here) |
| Per-page text extraction Err (panic absorbed by `catch_unwind`) | unchanged | yes | `Provenance::Warning` (page N "scanned candidate") — emitted by P7-1, propagated through |
| Empty page (no `/Contents` stream) | unchanged | yes | `Provenance::Warning` (page N "empty (scanned candidate)") — emitted by P7-1 |
| `Embedder::embed(...)` Err (any chunk) | `errors+=1` | yes (doc + chunk rows already written before embed call — see below) | n/a |

The embedding call sits *after* `put_document` / `put_blocks` / `put_chunks` in `kebab-app::ingest_one_asset` (markdown path, lines 615+), so a failed embed leaves doc + chunk rows on disk while no vector exists. This is consistent with the markdown path and accepted as v1 behaviour — re-running `kebab ingest` re-attempts the embed for any chunk whose `embedding_id` is missing from the vector store. Whole-asset rollback on embed-fail is a P+ task (atomic ingest transaction).

## Storage / wire effects

- `kebab-store-sqlite::documents` table gains rows whose `parser_version = "pdf-text-v1"`.
- `kebab-store-sqlite::blocks` table gains rows of `block_kind = "paragraph"` whose `source_span` is `Page { page, char_start, char_end }`. This is the first time the workspace stores blocks with `SourceSpan::Page`; any downstream reader that did not handle this variant is exposed.
- `kebab-store-sqlite::chunks` table gains rows whose `chunker_version = "pdf-page-v1"` AND whose `source_spans[0]` is `SourceSpan::Page`. Same exposure note for downstream readers.
- `kebab-store-vector::chunk_embeddings_<model>_<dim>` gains one vector per PDF chunk.
- `IngestReport` (wire `ingest_report.v1`) counters update naturally: `scanned` includes PDFs, `new` / `updated` track PDF docs, `errors` counts decode / encryption / corrupt failures, `skipped` continues to count audio / unknown formats.
- No new wire schemas or `kebab-core` types.

### Citation surface

- `Citation` (search hits / RAG answers) already carries `SourceSpan::Page`. The CLI / wire layer must render it — current `kebab search` JSON output passes `source_span` through verbatim; no change needed in this task. UI rendering of "page 12" labels for PDF citations is the responsibility of P9 (TUI / desktop) or whatever consumer is reading the wire.

## Test plan

| kind | description | fixture / data |
|------|-------------|----------------|
| integration | TempDir KB + 1 small text PDF (3 pages) → `kebab ingest` produces 1 doc + 3 chunks; each chunk's `source_spans[0]` is `Page { page: i, .. }`; chunks stored + embedded | `kebab-app/tests/pdf_pipeline.rs` (new), in-memory PDF via `lopdf` builder (mirrors `kebab-parse-pdf::tests::common`) |
| integration | Re-ingest same PDF (identical bytes) → identical `doc_id` and identical `chunk_id` set (P1 idempotency contract) | inline |
| integration | Edit a PDF (replace bytes — different blake3 → different `asset_id` → different `doc_id`) and re-ingest → `new+=1` for the new `doc_id`; old `doc_id` row remains untouched (orphan handling is a P+ task) | inline |
| integration | Encrypted PDF → asset NOT stored; `errors+=1`; `IngestItem.error` mentions `qpdf` / `decrypt` | inline (lopdf builder + fake `/Encrypt` trailer) |
| integration | Corrupt header PDF → asset NOT stored; `errors+=1`; error message mentions PDF parse failure | inline |
| integration | Mixed page PDF (page 1 text, page 2 empty / scanned, page 3 text) → asset stored; 2 chunks (pages 1 + 3); `doc.provenance.events` contains exactly 1 `Warning` for page 2 marked "scanned candidate" | inline |
| integration | `kebab inspect doc <pdf_doc_id>` returns the PDF `CanonicalDocument` with per-page `Block::Paragraph` and `SourceSpan::Page` intact | inline |
| integration | Hybrid search across mixed corpus (1 markdown + 1 PDF) returns the PDF chunk for a query whose terms appear only in the PDF body | inline (real `multilingual-e5-small` embedding) |
| integration | `IngestReport` invariant `scanned == new + updated + skipped + errors` holds when ingesting a mixed corpus including a corrupt PDF | inline |
| integration | Long PDF (50 pages × ~1.5 KB body each = ~75 KB) produces ≥50 chunks (≥1 per page); embedding loop completes; storage round-trips | inline |
| smoke | Update `docs/SMOKE.md` so the runbook ingests at least one PDF fixture and verifies search-by-page-text works + `inspect doc` shows `SourceSpan::Page` | docs change |

The opt-in real-Ollama integration tests stay in `kebab-llm-local` / `kebab-parse-image`. P7-3 adds zero LM dependency — PDF text extraction is local-only, so there is no equivalent hermetic-vs-real split to manage.

## Definition of Done

### Spec PR (this PR — `spec/p7-3-pdf-ingest-wiring`)

- [ ] `tasks/p7/p7-3-pdf-ingest-wiring.md` 작성 + self-review (placeholder / 모순 / 모호성 / scope)
- [ ] `tasks/INDEX.md` "P7 — 3 components" 반영
- [ ] PR 본문에 design §3.4, §3.5, §6.1, §9.2 링크
- [ ] HOTFIXES note 가 필요한 deviation (chunker selection, `config.chunking.chunker_version` PDF 무시) 본문에 명시

### Implementation PR (follow-up — `feat/p7-3-pdf-ingest-wiring`)

- [ ] `cargo check --workspace` passes
- [ ] `cargo test --workspace --no-fail-fast -j 1` passes (all new integration tests green)
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` passes
- [ ] `kebab ingest` against a TempDir KB containing 1 markdown + 1 image + 1 PDF produces `scanned 3 / new 3 / errors 0`
- [ ] `kebab search --mode hybrid "<text from PDF page 7>"` returns a chunk whose `source_span` is `Page { page: 7, .. }`
- [ ] `kebab inspect doc <pdf_doc_id>` shows per-page `Block::Paragraph` with `SourceSpan::Page`
- [ ] HOTFIXES entry written for the `config.chunking.chunker_version` deviation (PDF ignores; hard-coded `pdf-page-v1`)
- [ ] `docs/SMOKE.md` includes a PDF-fixture step

## Out of scope

- **RAG `kebab ask` PDF citation** — verifying that `Answer::Citation::source_span = Page { ... }` round-trips through the RAG pipeline is structurally a P4-3 (RAG pipeline) responsibility, not a P7-3 ingest-wiring responsibility. P4-3 already exercises the citation contract over markdown chunks; PDF chunks share the exact same `Citation` shape (the difference is only `source_span` variant). A future PR can bolt on a "PDF chunks survive RAG citation" assertion to either P4-3's existing tests or a dedicated `kebab-rag` integration test — bringing wiremock + RAG fixture infrastructure into `kebab-app` integration tests is out of proportion for the P7-3 invariant (which is "PDF chunks emerge from search with `Page` spans"). Captured here so reviewers can find this decision later.
- **Scanned PDF OCR fallback** — empty/extract-failed pages stay searchable=false in v1. A future task ("P+ scanned-PDF-ocr") routes those pages through P6-2's `OllamaVisionOcr` after rasterising the page via a PDF renderer (e.g. `mupdf-rs`, `pdfium-render`). Excluded here because (a) it requires a new system / Rust dep we don't have yet, and (b) v1 user scenario is text-embed PDFs (papers, exported reports).
- **Multi-column reading order / table extraction / formula detection / form-field extraction / bookmark or outline ingestion** — all deferred to future PDF-layout task. P7-1 already lists these as out of scope and the wiring inherits.
- **Body multilingual via CID font support** — handled at the parser layer (P7-1). UTF-16BE Title metadata works today; non-Latin body text depends on the PDF's font CID mapping.
- **Per-medium `chunker_version` config** — current `Config::chunking.chunker_version` is single-valued and serves markdown only. PDF ingest ignores it (hard-codes `pdf-page-v1`). A future P+ task either splits the field per medium or introduces a chunker registry. Logged as a deviation in `tasks/HOTFIXES.md` once implementation lands.
- **Chunker dispatch as a runtime registry** — current dispatch is a compile-time `match` on `MediaType`. Adequate while the workspace has 3 chunkers (md-heading-v1, pdf-page-v1, future audio); a registry makes sense once the count grows.
- **Parallelism beyond `config.indexing.max_parallel_extractors`** — the existing knob applies. Per-PDF parallelism is a P+ scale-hardening task (a 500-page book produces ≥500 chunks; embedding throughput is the bottleneck, not extraction).
- **Progress reporting** (`kebab ingest --progress`) — a 500-page book produces visible-but-silent work; UX gap acknowledged but a P+ enhancement.
- **Wire schema bump** — no new wire types; `ingest_report.v1` counters absorb PDF events naturally; `search_hit.v1` already carries `source_span` polymorphically.
- **`kebab-store-sqlite` schema migration for `SourceSpan::Page` columns** — the existing `blocks.source_span_kind` / `source_span_payload` columns store the JSON discriminator polymorphically (per P1-6). PDF rows reuse the existing schema without alteration.

## Risks / notes

- **`config.chunking.chunker_version` becomes ambiguous.** A user reading their `config.toml` sees `chunker_version = "md-heading-v1"` and reasonably assumes PDFs use the same. They don't. The implementation PR must either log a `tracing::info!` at ingest start ("PDF assets use chunker_version=pdf-page-v1 regardless of config.chunking.chunker_version") OR leave a `TODO` to address in the chunker-registry task. The HOTFIXES entry documents the deviation persistently.
- **A 500-page book produces 500+ chunks in one transaction.** The existing `put_chunks` call already loops chunks but the SQLite transaction boundary may need tuning. The implementation PR should benchmark and decide whether to chunk-batch the writes (e.g. 100 chunks per transaction) or trust the existing path. Not a correctness risk — only a throughput / WAL-size risk.
- **Encrypted-PDF error message is the only operator-visible signal.** The `kebab ingest` summary shows `errors=1` and the `IngestItem.error` message; a user who didn't read the full output may miss the `qpdf --decrypt` hint. Acceptable in v1 — a future `kebab inspect ingest <run_id>` (P9) renders structured per-asset errors. For now, ensure the test asserts the hint is preserved verbatim from P7-1's bail string.
- **Determinism stress with `now()` calls.** Same constraint as P6-4 — extract → chunk pipeline must not insert wall-clock reads between steps. P7-1 emits its own `now()` once for all per-page Provenance events; the PDF wiring branch must add no further `now()`s inside the per-asset path.
- **`pdf-page-v1` per-chunk hash variant `#c{char_start}` is opaque.** Downstream tools comparing `chunk_id`s by exact match work fine (it's still a deterministic blake3 input). Tools attempting to derive a stable position from the `chunk_id` alone would fail — they must read `chunk.source_spans[0].char_start`. Documented in P7-2's HOTFIXES entry; cross-referenced here for findability.
- **HEIC / RAW image and PDF/A subspecies share `MediaType::Other`.** PDF/A is detected by `kebab-source-fs` as `MediaType::Pdf` (header is still `%PDF-`); PDF subtype variants do not branch separately. Acceptable for v1.
