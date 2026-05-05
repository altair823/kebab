# p9-fb-23 — Incremental ingest Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Skip parse / chunk / embed / vector upsert for documents whose blake3 checksum AND parser/chunker/embedding versions all match what's already in SQLite, so re-running `kebab ingest` only does work for new or changed files.

**Architecture:** Add two columns to `documents` (V006 migration) tracking the chunker + embedding versions used last ingest. Add a new `DocumentStore::get_asset_by_workspace_path` read path. Insert an early-skip block at the top of the per-asset processing loop in `kebab-app::ingest_with_config_*` that returns `IngestItemKind::Unchanged` when all four conditions match. New `IngestOpts.force_reingest` flag bypasses the skip. `IngestReport` + `AggregateCounts` gain an `unchanged` count, surfaced in the wire schema, CLI summary, and TUI status bar.

**Tech Stack:** Rust 2024, refinery (SQLite migrations), serde, anyhow. No new deps.

**Spec:** `docs/superpowers/specs/2026-05-04-p9-fb-23-incremental-ingest-design.md`

---

## File Structure

**Created:**
- `migrations/V006__incremental_ingest.sql` — `ALTER TABLE documents` adds `last_chunker_version` + `last_embedding_version` TEXT (nullable).

**Modified:**
- `crates/kebab-core/src/ingest.rs` — `IngestItemKind::Unchanged` variant, `IngestReport.unchanged: u32`.
- `crates/kebab-core/src/document.rs` — `CanonicalDocument` gains `last_chunker_version: Option<ChunkerVersion>` + `last_embedding_version: Option<EmbeddingVersion>`. 14 construction sites add the two `None` fields.
- `crates/kebab-core/src/traits.rs` — new `DocumentStore::get_asset_by_workspace_path(&WorkspacePath) -> Option<RawAsset>` method.
- `crates/kebab-store-sqlite/src/documents.rs` — `put_document` writes new columns, `get_document` reads them.
- `crates/kebab-store-sqlite/src/store.rs` (or `assets.rs` — verify location during impl) — `get_asset_by_workspace_path` impl.
- `crates/kebab-app/src/lib.rs` — `IngestOpts { progress, cancel, force_reingest }` struct, ingest fn chain refactor, early-skip logic in asset loop, stamp versions on CanonicalDocument.
- `crates/kebab-app/src/ingest_progress.rs` — `AggregateCounts.unchanged: u32`, status_line text update.
- `crates/kebab-cli/src/commands/ingest.rs` (or similar) — `--force-reingest` flag plumbed.
- `crates/kebab-tui/src/ingest_progress.rs` — status_line final text gains `unchanged=N`.
- `docs/wire-schema/v1/ingest_report.schema.json` — additive `unchanged` (integer, minimum 0).
- `README.md` / `HANDOFF.md` / `tasks/HOTFIXES.md` / `tasks/INDEX.md` / `tasks/p9/p9-fb-23-incremental-ingest.md` — docs sync.

---

### Task 1: Extend ingest reporting types — `Unchanged` variant + counts

**Files:**
- Modify: `crates/kebab-core/src/ingest.rs`
- Modify: `crates/kebab-app/src/ingest_progress.rs`
- Modify: `docs/wire-schema/v1/ingest_report.schema.json`

The reporting types are foundational — every downstream task reads / writes them. Land first as a no-op (no callers produce `Unchanged` yet, no counter increments) so subsequent tasks can target the new shapes.

- [ ] **Step 1: Add `Unchanged` to `IngestItemKind`**

Open `crates/kebab-core/src/ingest.rs`. Replace the enum (around lines 38-45):

```rust
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IngestItemKind {
    New,
    Updated,
    /// Media-type filter / kb:// URI / non-supported source — never made
    /// it into the parse step.
    Skipped,
    /// p9-fb-23: blake3 checksum + parser_version + chunker_version +
    /// embedding_version all matched the existing record. Parse / chunk
    /// / embed / vector upsert all skipped.
    Unchanged,
    Error,
}
```

- [ ] **Step 2: Add `unchanged` to `IngestReport`**

In the same file, replace `IngestReport` (around lines 10-21):

```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IngestReport {
    pub scope: SourceScope,
    pub scanned: u32,
    pub new: u32,
    pub updated: u32,
    /// Media-type / source filter (`kb://`, unsupported types).
    pub skipped: u32,
    /// p9-fb-23: assets whose checksum + all version inputs matched —
    /// parse / chunk / embed / vector upsert all skipped.
    pub unchanged: u32,
    pub errors: u32,
    pub duration_ms: u32,
    /// `None` ↔ wire `items: null` (`--summary-only`).
    pub items: Option<Vec<IngestItem>>,
}
```

- [ ] **Step 3: Add `unchanged` to `AggregateCounts`**

Open `crates/kebab-app/src/ingest_progress.rs`. Replace the struct (around lines 25-34):

```rust
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct AggregateCounts {
    pub scanned: u32,
    pub new: u32,
    pub updated: u32,
    pub skipped: u32,
    /// p9-fb-23: assets whose checksum + all version inputs matched the
    /// existing DB record — parse / chunk / embed / vector upsert all
    /// skipped.
    pub unchanged: u32,
    pub errors: u32,
    pub chunks_indexed: u32,
    pub embeddings_indexed: u32,
}
```

`#[derive(Default)]` automatically zero-fills the new field.

- [ ] **Step 4: Update wire schema**

Open `docs/wire-schema/v1/ingest_report.schema.json`. Find the `properties` block. Add `unchanged` next to `skipped`:

```json
        "unchanged": {
          "type": "integer",
          "minimum": 0,
          "description": "p9-fb-23: assets whose checksum + parser_version + chunker_version + embedding_version all matched the existing record. Parse / chunk / embed / vector upsert all skipped."
        },
```

If the schema has a `required` array that lists `skipped`, also add `unchanged` to that array — `IngestReport` always carries the field (defaulted to 0).

- [ ] **Step 5: Build + fix any compile errors at construction sites**

Run: `cargo build --workspace`
Expected: compile errors at every site that constructs `IngestReport { ... }` literally without the new `unchanged` field.

For each error reported by the compiler, add `unchanged: 0,` next to `skipped: ...,` at the construction site. The compiler list is exhaustive — work through it linearly.

- [ ] **Step 6: Per-crate test smoke**

Run: `cargo test -p kebab-core --lib`
Run: `cargo test -p kebab-app --lib`
Expected: all pass. Existing tests should round-trip unchanged through serde with no behavioral change (default 0).

- [ ] **Step 7: Commit**

```bash
git add crates/kebab-core/src/ingest.rs crates/kebab-app/src/ingest_progress.rs docs/wire-schema/v1/ingest_report.schema.json
# Add any other files the compile-fix step touched.
git add -u
git commit -m "feat(kebab-core): p9-fb-23 task 1 — IngestItemKind::Unchanged + IngestReport.unchanged

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: Extend `CanonicalDocument` with version stamps

**Files:**
- Modify: `crates/kebab-core/src/document.rs`
- Modify: 14 construction sites across the workspace.

- [ ] **Step 1: Add the two fields to `CanonicalDocument`**

Open `crates/kebab-core/src/document.rs`. Find the `CanonicalDocument` struct (around lines 12-25). Add the imports for `ChunkerVersion` + `EmbeddingVersion`:

```rust
use crate::versions::{ChunkerVersion, EmbeddingVersion, ParserVersion};
```

Replace the struct:

```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CanonicalDocument {
    pub doc_id: DocumentId,
    pub source_asset_id: AssetId,
    pub workspace_path: WorkspacePath,
    pub title: String,
    pub lang: Lang,
    pub blocks: Vec<Block>,
    pub metadata: Metadata,
    pub provenance: Provenance,
    pub parser_version: ParserVersion,
    pub schema_version: u32,
    pub doc_version: u32,
    /// p9-fb-23: chunker version active when this document was last
    /// chunked. `None` for rows ingested before V006 migration; the
    /// next ingest stamps the current version. Compared against the
    /// active chunker version for the incremental-ingest skip path.
    pub last_chunker_version: Option<ChunkerVersion>,
    /// p9-fb-23: embedding model version active when this document
    /// was last embedded. `None` if no embedder is configured (skip
    /// path treats `None == None` as a match — see design doc).
    pub last_embedding_version: Option<EmbeddingVersion>,
}
```

- [ ] **Step 2: Build + fix all 14 construction sites**

Run: `cargo build --workspace`
Expected: compile errors at every `CanonicalDocument { ... }` literal in the workspace. Confirm the count by running:

```bash
grep -rn "CanonicalDocument {" crates/ | wc -l
```

For each error, add `last_chunker_version: None, last_embedding_version: None,` at the end of the struct literal. Sites (verify each — the list may have shifted since the plan was written):

- `crates/kebab-parse-image/src/lib.rs` (around line 203)
- `crates/kebab-parse-pdf/src/lib.rs` (around line 207)
- `crates/kebab-normalize/src/lib.rs` (around line 160)
- `crates/kebab-tui/tests/inspect.rs` (around line 66)
- `crates/kebab-chunk/src/md_heading_v1.rs` test fixture (around line 459)
- `crates/kebab-chunk/src/pdf_page_v1.rs` test fixture (around line 300)
- `crates/kebab-store-sqlite/tests/list_docs.rs` (around line 58)
- ... plus any others the compiler surfaces.

These are mechanical `None, None` insertions — no logic changes.

- [ ] **Step 3: Build clean**

Run: `cargo build --workspace`
Expected: clean.

- [ ] **Step 4: Test smoke**

Run: `cargo test -p kebab-core -p kebab-normalize -p kebab-parse-image -p kebab-parse-pdf -p kebab-chunk --lib`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add -u
git commit -m "feat(kebab-core): p9-fb-23 task 2 — CanonicalDocument gains last_chunker_version + last_embedding_version

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: V006 migration + SQLite put/get_document round-trip

**Files:**
- Create: `migrations/V006__incremental_ingest.sql`
- Modify: `crates/kebab-store-sqlite/src/documents.rs`

- [ ] **Step 1: Write the V006 migration**

Create `migrations/V006__incremental_ingest.sql`:

```sql
-- p9-fb-23: incremental ingest needs to know which chunker / embedding
-- versions were used to populate this document so a re-ingest can
-- decide whether to skip (versions match) or re-process (any mismatch).
-- parser_version is already on documents from V001.
ALTER TABLE documents ADD COLUMN last_chunker_version TEXT;
ALTER TABLE documents ADD COLUMN last_embedding_version TEXT;
```

Both columns default to NULL — refinery applies the migration in order, existing rows get NULL for the new columns.

- [ ] **Step 2: Find the existing `put_document` SQLite impl + extend SQL**

Open `crates/kebab-store-sqlite/src/documents.rs`. Find `put_document` (around line 51). Read the surrounding code to understand the INSERT / UPSERT statement shape — the new columns must be added to both the column list and the values list, AND to the `ON CONFLICT ... DO UPDATE SET` clause if present.

A representative shape after the change (adapt to the actual SQL in the file):

```rust
const INSERT_DOC_SQL: &str = r#"
    INSERT INTO documents (
        doc_id, asset_id, workspace_path, title, lang,
        source_type, trust_level, parser_version,
        doc_version, schema_version,
        metadata_json, provenance_json,
        created_at, updated_at,
        last_chunker_version, last_embedding_version
    ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
    ON CONFLICT(doc_id) DO UPDATE SET
        asset_id = excluded.asset_id,
        workspace_path = excluded.workspace_path,
        title = excluded.title,
        lang = excluded.lang,
        source_type = excluded.source_type,
        trust_level = excluded.trust_level,
        parser_version = excluded.parser_version,
        doc_version = excluded.doc_version + 1,
        schema_version = excluded.schema_version,
        metadata_json = excluded.metadata_json,
        provenance_json = excluded.provenance_json,
        updated_at = excluded.updated_at,
        last_chunker_version = excluded.last_chunker_version,
        last_embedding_version = excluded.last_embedding_version
"#;
```

In the binding step (`stmt.execute(...)` or similar), append two more positional params at the end:

```rust
doc.last_chunker_version.as_ref().map(|v| v.0.as_str()),
doc.last_embedding_version.as_ref().map(|v| v.0.as_str()),
```

(The exact rusqlite API depends on whether the file uses `params!` or `[...]`. Match the existing style in the file.)

- [ ] **Step 3: Extend `get_document` SQLite impl**

In the same file, find `get_document` (around line 157). The SELECT statement and row-mapper closure must include the two new columns:

```rust
const SELECT_DOC_SQL: &str = r#"
    SELECT
        doc_id, asset_id, workspace_path, title, lang,
        source_type, trust_level, parser_version,
        doc_version, schema_version,
        metadata_json, provenance_json,
        created_at, updated_at,
        last_chunker_version, last_embedding_version
    FROM documents
    WHERE doc_id = ?
"#;
```

Row-mapper closure (adapt to the file's pattern):

```rust
let last_chunker_version: Option<String> = row.get("last_chunker_version")?;
let last_embedding_version: Option<String> = row.get("last_embedding_version")?;
let last_chunker_version = last_chunker_version.map(kebab_core::ChunkerVersion);
let last_embedding_version = last_embedding_version.map(kebab_core::EmbeddingVersion);

// In the CanonicalDocument literal:
last_chunker_version,
last_embedding_version,
```

- [ ] **Step 4: Write a round-trip integration test**

Open `crates/kebab-store-sqlite/tests/list_docs.rs` (or create a new test file `tests/incremental_ingest.rs` if the existing test suite has a different focus). Append:

```rust
#[test]
fn put_then_get_document_roundtrips_version_stamps() {
    let store = build_test_store();  // existing helper — adapt to actual name
    let mut doc = sample_doc();      // existing helper
    doc.last_chunker_version = Some(kebab_core::ChunkerVersion("md-heading-v1".into()));
    doc.last_embedding_version = Some(kebab_core::EmbeddingVersion("multilingual-e5-small@v1".into()));
    store.put_document(&doc).unwrap();
    let loaded = store.get_document(&doc.doc_id).unwrap().expect("doc round-trips");
    assert_eq!(loaded.last_chunker_version, doc.last_chunker_version);
    assert_eq!(loaded.last_embedding_version, doc.last_embedding_version);
}

#[test]
fn put_then_get_document_roundtrips_none_stamps() {
    let store = build_test_store();
    let doc = sample_doc();  // sample_doc default: both None
    store.put_document(&doc).unwrap();
    let loaded = store.get_document(&doc.doc_id).unwrap().expect("doc round-trips");
    assert_eq!(loaded.last_chunker_version, None);
    assert_eq!(loaded.last_embedding_version, None);
}
```

If `build_test_store` / `sample_doc` are not the actual helper names, look at the top of `list_docs.rs` and use whatever pattern that file uses to construct an isolated SQLite store + a sample CanonicalDocument.

- [ ] **Step 5: Run the migration + tests**

Run: `cargo test -p kebab-store-sqlite`
Expected: all pass, including the two new round-trip tests.

- [ ] **Step 6: Commit**

```bash
git add migrations/V006__incremental_ingest.sql crates/kebab-store-sqlite/src/documents.rs crates/kebab-store-sqlite/tests/list_docs.rs
git commit -m "feat(kebab-store-sqlite): p9-fb-23 task 3 — V006 migration + put/get_document round-trip version stamps

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

### Task 4: New `DocumentStore::get_asset_by_workspace_path`

**Files:**
- Modify: `crates/kebab-core/src/traits.rs`
- Modify: `crates/kebab-store-sqlite/src/store.rs` (or `assets.rs` — find via grep `fn put_asset_with_bytes`)

- [ ] **Step 1: Add the trait method**

Open `crates/kebab-core/src/traits.rs`. Find the `pub trait DocumentStore` block (around line 151). Add a new method directly after `get_document`:

```rust
    /// p9-fb-23: look up an asset row by its workspace path. Used by
    /// the incremental-ingest skip path to compare the freshly
    /// computed blake3 checksum against what's already in SQLite. The
    /// schema enforces a unique workspace_path per asset.
    fn get_asset_by_workspace_path(
        &self,
        path: &WorkspacePath,
    ) -> anyhow::Result<Option<RawAsset>>;
```

If `WorkspacePath` / `RawAsset` are not yet imported at the top of `traits.rs`, add them: the existing imports already pull the types used by other trait methods — extend that block.

- [ ] **Step 2: Implement on SQLite store**

Find the file that contains `put_asset_with_bytes` (likely `crates/kebab-store-sqlite/src/store.rs` or `crates/kebab-store-sqlite/src/assets.rs`). Locate it via:

```bash
grep -rn "fn put_asset_with_bytes" crates/kebab-store-sqlite/
```

Add the new method to the same `impl DocumentStore for SqliteStore` (or `impl SqliteStore`) block:

```rust
    fn get_asset_by_workspace_path(
        &self,
        path: &kebab_core::WorkspacePath,
    ) -> anyhow::Result<Option<kebab_core::RawAsset>> {
        let conn = self.conn.lock();  // or whatever the file's connection pattern is
        let row = conn.query_row(
            r#"SELECT
                asset_id, source_uri, workspace_path, media_type,
                byte_len, checksum, storage_kind, storage_path,
                discovered_at
            FROM assets
            WHERE workspace_path = ?"#,
            rusqlite::params![path.0.as_str()],
            |row| {
                // Build RawAsset from columns. Reuse the existing
                // row-mapper helper if `put_asset_with_bytes` already
                // has one (avoid duplicating the parse logic).
                Ok(/* RawAsset { ... } literal */)
            },
        );
        match row {
            Ok(asset) => Ok(Some(asset)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
```

The exact body depends on this crate's helpers — if there's already an `asset_from_row` private fn, reuse it. If not, add one and call it from BOTH `get_asset_by_workspace_path` AND any future getter (DRY — don't open-code the column→RawAsset mapping in two places).

- [ ] **Step 3: Add a test**

Append to the existing test suite that covers asset writes (find via `grep -rn "put_asset_with_bytes" crates/kebab-store-sqlite/tests/`):

```rust
#[test]
fn get_asset_by_workspace_path_roundtrips() {
    let store = build_test_store();
    let (asset, bytes) = sample_asset_with_bytes("notes/foo.md", b"hello");
    store.put_asset_with_bytes(&asset, &bytes).unwrap();
    let loaded = store
        .get_asset_by_workspace_path(&asset.workspace_path)
        .unwrap()
        .expect("asset must round-trip");
    assert_eq!(loaded.asset_id, asset.asset_id);
    assert_eq!(loaded.checksum, asset.checksum);
    assert_eq!(loaded.byte_len, asset.byte_len);
}

#[test]
fn get_asset_by_workspace_path_returns_none_for_unknown() {
    let store = build_test_store();
    let path = kebab_core::WorkspacePath::new("notes/missing.md".into()).unwrap();
    assert!(store.get_asset_by_workspace_path(&path).unwrap().is_none());
}
```

If `sample_asset_with_bytes` doesn't exist in the test fixture, build a `RawAsset` by hand using the existing pattern from other asset tests in the same crate.

- [ ] **Step 4: Run + commit**

```bash
cargo test -p kebab-store-sqlite
git add crates/kebab-core/src/traits.rs crates/kebab-store-sqlite/src/  # whichever file you modified
git add -u  # for the test file
git commit -m "feat(kebab-store-sqlite): p9-fb-23 task 4 — get_asset_by_workspace_path

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

### Task 5: Stamp current versions on `CanonicalDocument` in ingest

**Files:**
- Modify: `crates/kebab-app/src/lib.rs` (or wherever `CanonicalDocument` is constructed in the ingest pipeline — search via `grep -n "CanonicalDocument" crates/kebab-app/src/lib.rs`)

This task does NOT add the skip path yet. It just makes every freshly-ingested doc carry the current chunker + embedding versions, so Task 7's skip detection has data to work with on the SECOND ingest run.

- [ ] **Step 1: Find the CanonicalDocument construction sites in ingest**

Run:

```bash
grep -n "CanonicalDocument" crates/kebab-app/src/lib.rs
```

Markdown / image / PDF flows likely each construct one. Note: most actual `CanonicalDocument` literals are inside parser crates (kebab-parse-md, kebab-parse-image, kebab-parse-pdf) — the parsers don't know about the chunker / embedder version. The right insertion point is in `kebab-app::ingest_with_config_*` AFTER parse + AFTER chunk, where the chunker that ran is in scope and the embedder reference is at hand.

The pattern: parsers return a `CanonicalDocument` with `last_chunker_version: None, last_embedding_version: None` (Task 2's mechanical change). The ingest pipeline then mutates the doc to stamp the versions before `put_document`.

- [ ] **Step 2: Stamp logic in the ingest pipeline**

Find each `put_document(&canonical)` call site in `crates/kebab-app/src/lib.rs`. Just before each call, mutate the doc:

```rust
canonical.last_chunker_version = Some(chunker.chunker_version());  // or whatever variable holds the chunker
if let Some(emb) = embedder.as_ref() {
    canonical.last_embedding_version = Some(emb.model_version());
}
// else leave as None — embedder is not configured, so the skip path
// will treat None == None as a match (no stale state to compare).
```

The variable names depend on the existing scope at each call site — read 30 lines above to see what's bound. The chunker is constructed as `let chunker = MdHeadingV1Chunker::new(...)` (or `PdfPageV1Chunker`); follow that variable.

`canonical` must be `let mut canonical = ...` — if it's currently bound immutably, change to `let mut`.

Apply the same pattern to all three flows (markdown, image, pdf — verify via grep).

- [ ] **Step 3: Build + run existing ingest tests**

Run: `cargo test -p kebab-app --test '*'`
Expected: all pass. Existing tests don't assert on the new fields but the pipeline must still write a valid document.

- [ ] **Step 4: Add a test that asserts the stamps land in the DB**

Find a kebab-app integration test that does an end-to-end ingest (e.g. `crates/kebab-app/tests/ingest_smoke.rs` or similar). Append:

```rust
#[test]
fn ingest_stamps_chunker_and_embedding_versions_on_document() {
    let (config, _tmp) = test_config_with_md_fixture();
    let report = kebab_app::ingest_with_config(
        config.clone(),
        SourceScope::workspace_root(&config),  // or whatever helper exists
        false,  // summary_only
    ).unwrap();
    assert_eq!(report.new, 1);

    let app = kebab_app::App::open_with_config(config).unwrap();
    let doc_id = app.store.list_documents(&Default::default()).unwrap()[0].doc_id.clone();
    let doc = app.store.get_document(&doc_id).unwrap().expect("doc exists");
    assert!(doc.last_chunker_version.is_some(), "chunker version stamped");
    // Embedding version is Some only if the test config enables an embedder.
    // If `test_config_with_md_fixture` sets up fastembed, assert Some; otherwise None.
}
```

Adapt to the actual test infrastructure in the file. The key assertion is that `last_chunker_version` is `Some(_)` after a normal ingest.

- [ ] **Step 5: Commit**

```bash
git add -u
git commit -m "feat(kebab-app): p9-fb-23 task 5 — stamp chunker + embedding versions on CanonicalDocument before put_document

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

### Task 6: `IngestOpts` struct + plumb through ingest fn chain

**Files:**
- Modify: `crates/kebab-app/src/lib.rs`

Introduce `IngestOpts { progress, cancel, force_reingest }` matching the `AskOpts` pattern from p9-fb-15. The bottom fn (`ingest_with_config_cancellable`) currently takes 5 positional args; rename it to `ingest_with_config_opts(config, scope, summary_only, opts: IngestOpts)` and let the higher-level wrappers build `IngestOpts` from their positional params.

- [ ] **Step 1: Define `IngestOpts`**

Open `crates/kebab-app/src/lib.rs`. Near the top (with other public `pub struct *Opts`), add:

```rust
/// p9-fb-23: optional per-call ingest controls. Kept as a struct (vs.
/// a growing positional arg list) so future flags (e.g. `dry_run`,
/// per-asset `concurrency`) land additively without churning every
/// caller. Mirrors the `AskOpts` pattern from p9-fb-15.
#[derive(Default)]
pub struct IngestOpts {
    /// Streaming progress sink. `None` suppresses emission entirely.
    pub progress: Option<std::sync::mpsc::Sender<crate::ingest_progress::IngestEvent>>,
    /// Cooperative cancel token. `None` = uncancellable.
    pub cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    /// p9-fb-23: when `true`, the per-asset early-skip block is bypassed
    /// — every asset is re-parsed / re-chunked / re-embedded as if the
    /// DB were empty. Default `false` preserves the auto-skip path.
    pub force_reingest: bool,
}
```

- [ ] **Step 2: Rename the bottom fn to `ingest_with_config_opts`**

Find `pub fn ingest_with_config_cancellable` (around line 248). Rename to `ingest_with_config_opts`, keeping the body, but change the signature from positional `progress` + `cancel` to a single `opts: IngestOpts`:

```rust
#[doc(hidden)]
pub fn ingest_with_config_opts(
    config: kebab_config::Config,
    scope: SourceScope,
    summary_only: bool,
    opts: IngestOpts,
) -> anyhow::Result<IngestReport> {
    let progress = opts.progress.as_ref();
    let cancelled = || {
        opts.cancel
            .as_ref()
            .map(|c| c.load(std::sync::atomic::Ordering::Relaxed))
            .unwrap_or(false)
    };
    // ... rest of the body unchanged. `opts.force_reingest` will be
    // consumed by Task 7's skip-detection block.
    let _ = opts.force_reingest;  // silence unused warning until Task 7 wires it up
    // ... existing logic ...
}
```

- [ ] **Step 3: Convert the three wrapper fns to build `IngestOpts`**

`ingest_with_config_cancellable` (the OLD name) must stay so external callers (test fixtures, possibly other code) keep compiling. Re-introduce it as a thin wrapper:

```rust
#[doc(hidden)]
pub fn ingest_with_config_cancellable(
    config: kebab_config::Config,
    scope: SourceScope,
    summary_only: bool,
    progress: Option<std::sync::mpsc::Sender<crate::ingest_progress::IngestEvent>>,
    cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
) -> anyhow::Result<IngestReport> {
    ingest_with_config_opts(
        config,
        scope,
        summary_only,
        IngestOpts {
            progress,
            cancel,
            force_reingest: false,
        },
    )
}
```

Same shape for `ingest_with_config_progress` and `ingest_with_config` — they all collapse into wrappers that build a default-ish `IngestOpts`.

- [ ] **Step 4: Build + test**

Run: `cargo build -p kebab-app`
Expected: clean.

Run: `cargo test -p kebab-app`
Expected: existing tests pass (no behaviour change yet).

- [ ] **Step 5: Add a test that uses `ingest_with_config_opts` directly**

Append to an existing kebab-app test file:

```rust
#[test]
fn ingest_with_config_opts_default_matches_legacy_behaviour() {
    let (config, _tmp) = test_config_with_md_fixture();  // existing helper
    let report = kebab_app::ingest_with_config_opts(
        config,
        SourceScope::workspace_root(&_tmp),  // adapt
        false,
        kebab_app::IngestOpts::default(),
    ).unwrap();
    assert!(report.new >= 1);
    assert_eq!(report.errors, 0);
}
```

- [ ] **Step 6: Commit**

```bash
git add -u
git commit -m "refactor(kebab-app): p9-fb-23 task 6 — IngestOpts struct + ingest_with_config_opts entry

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

### Task 7: Early-skip detection in ingest pipeline

**Files:**
- Modify: `crates/kebab-app/src/lib.rs` (asset processing loop in `ingest_with_config_opts`)

This is the load-bearing change — the skip path that closes the user's feedback. TDD: write the failing test first, then implement.

- [ ] **Step 1: Write the failing integration test**

Find or create `crates/kebab-app/tests/incremental_ingest.rs`:

```rust
//! p9-fb-23: incremental ingest — skip parse/chunk/embed when nothing
//! has changed.

use kebab_app::{AggregateCounts, IngestOpts, ingest_with_config_opts};
// ... existing test helper imports ...

#[test]
fn second_ingest_of_unchanged_corpus_marks_all_unchanged() {
    let (config, tmp_dir) = test_config_with_md_fixture();  // 1 markdown file
    let scope = SourceScope::workspace_root(&tmp_dir);

    // First ingest — populates the DB.
    let first = ingest_with_config_opts(
        config.clone(),
        scope.clone(),
        false,
        IngestOpts::default(),
    ).unwrap();
    assert_eq!(first.new, 1);
    assert_eq!(first.unchanged, 0);

    // Second ingest — same file, same versions → all unchanged.
    let second = ingest_with_config_opts(
        config,
        scope,
        false,
        IngestOpts::default(),
    ).unwrap();
    assert_eq!(second.new, 0, "no new docs on re-ingest");
    assert_eq!(second.updated, 0, "nothing should be marked updated");
    assert_eq!(second.unchanged, 1, "the one doc must be Unchanged");
}

#[test]
fn force_reingest_bypasses_skip() {
    let (config, tmp_dir) = test_config_with_md_fixture();
    let scope = SourceScope::workspace_root(&tmp_dir);

    let _first = ingest_with_config_opts(
        config.clone(),
        scope.clone(),
        false,
        IngestOpts::default(),
    ).unwrap();

    let second = ingest_with_config_opts(
        config,
        scope,
        false,
        IngestOpts {
            force_reingest: true,
            ..Default::default()
        },
    ).unwrap();
    assert_eq!(second.unchanged, 0, "force_reingest must bypass skip");
    assert_eq!(second.updated, 1, "doc must be re-processed as Updated");
}
```

- [ ] **Step 2: Run — expect failure**

Run: `cargo test -p kebab-app incremental_ingest -- --nocapture`
Expected: both tests fail. Second ingest currently produces `updated=1, unchanged=0` (no skip path).

- [ ] **Step 3: Implement the skip block**

In `crates/kebab-app/src/lib.rs::ingest_with_config_opts`, find the asset processing loop (the loop that iterates over `connector.scan()` results and per-asset does parse / put_asset / put_document / etc.). At the TOP of each iteration — AFTER the asset has been scanned (so `asset_blake3` / `RawAsset` is in scope) but BEFORE parse / chunk / embed:

```rust
// p9-fb-23: incremental ingest skip path. If `force_reingest` is
// false AND the existing record's checksum + parser_version +
// chunker_version + embedding_version all match, this asset doesn't
// need to be re-processed. We still emit an AssetFinished event so
// the progress consumer sees the asset accounted for, and we bump
// the unchanged counter for the final report.
if !opts.force_reingest {
    if let Ok(Some(existing_asset)) = app.store.get_asset_by_workspace_path(&raw_asset.workspace_path) {
        if existing_asset.checksum == raw_asset.checksum {
            // Asset bytes match. Check the doc_id (parser_version aware) + version stamps.
            let candidate_doc_id = kebab_core::ids::id_for_doc(
                &raw_asset.workspace_path,
                &raw_asset.asset_id,
                &current_parser_version,  // bind near scan; for md = KEBAB_PARSE_MD_VERSION
            );
            if let Ok(Some(existing_doc)) = app.store.get_document(&candidate_doc_id) {
                let chunker_match = existing_doc.last_chunker_version.as_ref()
                    == Some(&current_chunker_version);
                let embedder_match = existing_doc.last_embedding_version
                    == current_embedding_version;
                if chunker_match && embedder_match {
                    // SKIP path.
                    crate::ingest_progress::emit(
                        progress,
                        crate::ingest_progress::IngestEvent::AssetFinished {
                            idx: asset_idx,
                            total,
                            result: kebab_core::IngestItemKind::Unchanged,
                            chunks: 0,
                        },
                    );
                    aggregate.unchanged += 1;
                    aggregate.scanned += 1;
                    if !summary_only {
                        items.push(kebab_core::IngestItem {
                            kind: kebab_core::IngestItemKind::Unchanged,
                            doc_id: Some(candidate_doc_id),
                            doc_path: raw_asset.workspace_path.clone(),
                            asset_id: Some(raw_asset.asset_id.clone()),
                            byte_len: Some(raw_asset.byte_len),
                            block_count: None,
                            chunk_count: None,
                            parser_version: Some(current_parser_version.clone()),
                            chunker_version: existing_doc.last_chunker_version.clone(),
                            warnings: Vec::new(),
                            error: None,
                        });
                    }
                    continue;  // next asset
                }
            }
        }
    }
}
```

The variable names (`raw_asset`, `current_parser_version`, `current_chunker_version`, `current_embedding_version`, `aggregate`, `items`, `asset_idx`, `total`, `progress`, `summary_only`) MUST match what's already in scope at the insertion point — read 50 lines above to map them. The branch logic stays the same.

`current_chunker_version` and `current_embedding_version` may not exist in the outer scope yet (they're computed per-media inside the loop today). If the chunker isn't constructed until after parse, restructure: construct the chunker EARLIER (it's stateless — just a version string + policy), bind `current_chunker_version` once at top of loop iter, then reuse for both the skip check and the post-parse stamp from Task 5. The embedder reference is at-app-construction scope (`embedder = ...` near `App::open_with_config`), trivially available.

- [ ] **Step 4: Run the test — should pass**

Run: `cargo test -p kebab-app incremental_ingest -- --nocapture`
Expected: both tests pass.

- [ ] **Step 5: Run full kebab-app suite**

Run: `cargo test -p kebab-app`
Expected: all pass. Existing single-ingest tests are unaffected (they all start from empty DB → no skip).

- [ ] **Step 6: Clippy**

Run: `cargo clippy -p kebab-app --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add -u
git commit -m "feat(kebab-app): p9-fb-23 task 7 — early-skip Unchanged path in ingest

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

### Task 8: CLI `--force-reingest` flag

**Files:**
- Modify: `crates/kebab-cli/src/...` (find via `grep -rn "ingest_with_config\|ingest_with_config_opts" crates/kebab-cli/`)

- [ ] **Step 1: Locate the ingest CLI subcommand**

Run:

```bash
grep -rn "fn handle_ingest\|fn ingest_command\|kebab_app::ingest_with_config" crates/kebab-cli/src/
```

Find the function that dispatches `kebab ingest`. The CLI uses `clap` derive — there will be a struct annotated with `#[derive(Args)]` or similar holding flags like `--summary-only`.

- [ ] **Step 2: Add the flag to the clap struct**

Append to the existing `#[derive(Args)]` struct (replace `IngestArgs` with the actual name):

```rust
    /// p9-fb-23: bypass the per-asset early-skip path. Every asset is
    /// re-parsed, re-chunked, re-embedded, and re-upserted regardless
    /// of whether the DB already has a record with matching checksum
    /// and version stamps. Useful after manual schema bumps or when
    /// the user suspects the corpus is in a stale state.
    #[arg(long)]
    pub force_reingest: bool,
```

- [ ] **Step 3: Plumb the flag into `IngestOpts`**

In the dispatcher, find the call to `kebab_app::ingest_with_config_*` and switch to `ingest_with_config_opts`:

```rust
    let opts = kebab_app::IngestOpts {
        progress: Some(progress_tx),
        cancel: Some(cancel_token),
        force_reingest: args.force_reingest,
    };
    let report = kebab_app::ingest_with_config_opts(config, scope, summary_only, opts)?;
```

- [ ] **Step 4: Run CLI build + smoke test**

Run: `cargo build -p kebab-cli`
Expected: clean.

Run: `cargo test -p kebab-cli`
Expected: pass. The CLI test suite likely doesn't have a `--force-reingest` integration test — that's covered in `kebab-app::tests::incremental_ingest` from Task 7.

- [ ] **Step 5: Commit**

```bash
git add -u
git commit -m "feat(kebab-cli): p9-fb-23 task 8 — --force-reingest flag

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

### Task 9: TUI status_line surfaces `unchanged`

**Files:**
- Modify: `crates/kebab-tui/src/ingest_progress.rs` (`status_line` fn)

- [ ] **Step 1: Find `status_line`**

Open `crates/kebab-tui/src/ingest_progress.rs`. Locate `pub fn status_line` (around line 170). It currently produces:

```
✓ ingest: 100 docs (5 new, 3 updated, 2 skipped), 142 chunks indexed in 12s
```

- [ ] **Step 2: Update the format**

Replace the `final` (terminal) text branch to include `unchanged`:

```rust
        return format!(
            "✓ ingest: {} docs ({} new, {} updated, {} unchanged, {} skipped), {} chunks indexed in {}s",
            state.counts.scanned,
            state.counts.new,
            state.counts.updated,
            state.counts.unchanged,
            state.counts.skipped,
            state.counts.chunks_indexed,
            secs,
        );
```

Also update the `aborted` branch to include `unchanged`:

```rust
        return format!(
            "✗ ingest aborted at {}/{} after {}s (new={} updated={} unchanged={} skipped={} errors={})",
            state.counts.scanned.saturating_sub(state.counts.errors),
            state.counts.scanned,
            secs,
            state.counts.new,
            state.counts.updated,
            state.counts.unchanged,
            state.counts.skipped,
            state.counts.errors,
        );
```

The in-flight (mid-progress) branch doesn't need changes — it shows per-asset granularity.

- [ ] **Step 3: Update tests**

Find the existing test for `status_line` (likely in the same file under `#[cfg(test)] mod tests`). Add an `unchanged` field to the test's `AggregateCounts` literal (Task 1 already made `unchanged: u32` mandatory) and update the expected string.

- [ ] **Step 4: Run + commit**

```bash
cargo test -p kebab-tui --lib ingest_progress
git add -u
git commit -m "feat(kebab-tui): p9-fb-23 task 9 — status_line surfaces unchanged count

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

### Task 10: Workspace verification + docs sync

**Files:**
- Modify: `README.md`, `HANDOFF.md`, `tasks/HOTFIXES.md`, `tasks/INDEX.md`
- Create: `tasks/p9/p9-fb-23-incremental-ingest.md`

- [ ] **Step 1: Full workspace test + clippy**

Run: `cargo test --workspace --no-fail-fast -j 1`
Expected: All previously-passing tests still pass. New tests (Tasks 3, 4, 5, 7) added. Total should be ~720 + ~6-8 new = 726+. Zero failures.

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: clean.

If anything fails, report `BLOCKED` — earlier task implementer must fix.

- [ ] **Step 2: README**

Open `README.md`. Find the `kebab ingest` row in the command table (or the prose section that describes ingest). APPEND inside the existing cell:

```
**Incremental** (p9-fb-23): 두 번째 이후의 ingest 는 변하지 않은 doc (blake3 + parser/chunker/embedder version 모두 동일) 의 parse/chunk/embed/vector upsert 를 자동 스킵. final summary 에 `N unchanged` 카운트 표시. `--force-reingest` 로 skip 무시 강제 재처리.
```

- [ ] **Step 3: HANDOFF.md**

Open `HANDOFF.md`. Find the `## 머지 후 발견된 버그 / 결정 (요약)` section. Add a new entry directly above the most recent (`p9-fb-24`) row:

```
- **2026-05-04 P9 post-도그푸딩 (p9-fb-23)** — Incremental ingest. 사용자 도그푸딩 피드백: 변하지 않은 문서는 다시 ingest 하지 않기. blake3 checksum + parser_version + chunker_version + embedding_version 4개 input 이 모두 일치할 때 parse/chunk/embed/vector upsert 모두 회피. SQLite V006 마이그레이션 — `documents` 에 `last_chunker_version` + `last_embedding_version` 컬럼 추가. 신규 `IngestItemKind::Unchanged` variant + `IngestReport.unchanged` + `AggregateCounts.unchanged` (wire schema additive). `IngestOpts { progress, cancel, force_reingest }` struct 도입 — `AskOpts` 패턴. `--force-reingest` CLI flag 로 skip 우회. 비용 dominator (fastembed) 가 변경된 / 새 doc 에만 발생. spec: `tasks/p9/p9-fb-23-incremental-ingest.md`. HOTFIXES `2026-05-04 — p9-fb-23` 항목이 version cascade 명시 동작의 source of truth.
```

- [ ] **Step 4: HOTFIXES.md**

Open `tasks/HOTFIXES.md`. Add a new section directly above `## 2026-05-04 — p9-fb-24`:

```markdown
## 2026-05-04 — p9-fb-23 (post-dogfooding): Incremental ingest

**Source feedback**: 사용자 도그푸딩 2026-05-04 — "새 문서들이 폴더에 추가되면 ingest 시 변하지 않은 문서는 다시 ingest 하지 않고 변하거나 새로 추가된 문서만 처리하고 싶어."

**Live binding 변경**:

- SQLite V006 migration — `documents` 에 `last_chunker_version` + `last_embedding_version` TEXT (nullable) 추가. 기존 row 는 NULL → 첫 번째 ingest 시 항상 mismatch → 강제 재처리 (안전 default).
- `kebab-core::IngestItemKind::Unchanged` variant 신규 (기존 `Skipped` 와 의미 분리: `Skipped` = media-type 필터, `Unchanged` = 모든 versions match).
- `IngestReport.unchanged: u32` + `AggregateCounts.unchanged: u32` 신규. wire schema `ingest_report.v1` 에 `unchanged` 필드 additive (v1 호환 유지).
- `kebab-app::IngestOpts { progress, cancel, force_reingest }` struct 신규 — `AskOpts` 패턴. 기존 `ingest_with_config_cancellable` 등 wrapper 보존, 신규 `ingest_with_config_opts` 가 IngestOpts 받음.
- `kebab-app::ingest_with_config_opts` asset 루프에 early-skip 블록: `force_reingest=false` + 4 조건 (asset_blake3 일치 + doc_id 존재 + last_chunker_version 일치 + last_embedding_version 일치) 모두 성립 시 `IngestEvent::AssetFinished{result: Unchanged}` emit + `aggregate.unchanged += 1` + `continue` (parse/chunk/embed/vector upsert 모두 회피).
- 정상 path 끝에서 `CanonicalDocument.last_chunker_version` + `last_embedding_version` 을 현 active version 으로 stamp.
- `kebab-cli` 에 `--force-reingest` flag 추가 (skip 우회 강제 재처리).
- `kebab-tui::ingest_progress::status_line` final / aborted 라인 모두 `unchanged=N` 노출.

**Spec contract impact**: design §9 versioning cascade 의 명시적 동작 추가 — parser/chunker/embedder version bump 시 다음 ingest 가 자동으로 모든 doc 을 `updated` 로 처리. 기존엔 silently 새 version 으로 overwrite (idempotent UPSERT) 였으나 본 변경으로 explicit refresh + 비용 회피 모두 보장. design §3.x IngestReport / §2.4a IngestEvent 에 `Unchanged` variant 추가 (additive, wire v1 호환).

**Tests added**: 약 8 신규 (incremental_ingest 통합 2: unchanged path / force_reingest, sqlite store 단위 4: round-trip version stamps + None stamps + get_asset_by_workspace_path roundtrip + missing path None, app ingest 통합 1: 첫 ingest 가 stamps 남김, kebab-app IngestOpts default 동작 확인 1). 기존 ~720 워크스페이스 테스트 무수정 통과.

**Known limitation (deferred)**:

- Mtime-based pre-hash skip (파일 읽기 자체 회피) 미구현 — blake3 streaming 은 매 scan 마다 무조건 발생. 큰 corpus 에서는 추가 최적화 가능.
- Watch-mode (실시간 file change detection) 후속 task.
- Stale skip risk: 사용자가 외부에서 embedder 모델 swap 후 config 의 `models.embedding.id` 갱신 안 하면 last_embedding_version 매치 → silently skip. doctor 명령이 mismatch 감지 → 권고하는 후속 task 가능.
```

- [ ] **Step 5: INDEX.md**

Open `tasks/INDEX.md`. Find the `p9-fb-24` row and append below:

```
  - [p9-fb-23 incremental ingest (post-도그푸딩)](p9/p9-fb-23-incremental-ingest.md)
```

- [ ] **Step 6: Per-task spec file**

Create `tasks/p9/p9-fb-23-incremental-ingest.md`:

```markdown
---
phase: P9
component: kebab-app
task_id: p9-fb-23
title: "Incremental ingest — skip unchanged docs (post-merge dogfooding)"
status: completed
depends_on: [p9-fb-03, p9-fb-07]
unblocks: []
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§9 Versioning cascade, §2.4a IngestEvent, §3.x IngestReport]
source_feedback: 사용자 도그푸딩 2026-05-04 — 변하지 않은 문서 재처리 회피 요청.
---

# p9-fb-23 — Incremental ingest

상세 설계: `docs/superpowers/specs/2026-05-04-p9-fb-23-incremental-ingest-design.md`.
구현 계획: `docs/superpowers/plans/2026-05-04-p9-fb-23-incremental-ingest.md`.

## Goal

`kebab ingest` 가 변경/신규 doc 만 처리. 변하지 않은 doc 은 parse/chunk/embed/vector upsert 모두 회피.

## Behavior contract

Skip 조건 4 모두 만족:
1. 신규 blake3 == `assets.checksum`.
2. `documents.parser_version` == 현 active.
3. `documents.last_chunker_version` == 현 active.
4. `documents.last_embedding_version` == 현 active (None == None 도 match).

위 중 하나라도 mismatch → 정상 path. parse/chunk/embed/vector upsert 모두.

`IngestOpts.force_reingest=true` → skip 무시 강제 재처리.

## Tests

- 통합: 두 번째 ingest 가 unchanged 1 / new 0 / updated 0.
- 통합: `--force-reingest` 가 skip 우회.
- 단위: V006 migration, SQLite put/get_document roundtrip 신규 컬럼, get_asset_by_workspace_path roundtrip.
- 통합: 첫 ingest 가 chunker/embedding version stamp.

## Risks / notes

- mtime pre-hash skip 미구현 (YAGNI, 후속 가능).
- 외부 embedder model swap 후 config 갱신 안 하면 silently skip — doctor 명령이 mismatch 감지하는 후속 task 가능.

Live deviations 반영 위치: `tasks/HOTFIXES.md` `2026-05-04 — p9-fb-23` 항목.
```

- [ ] **Step 7: Final commit**

```bash
git add README.md HANDOFF.md tasks/HOTFIXES.md tasks/INDEX.md tasks/p9/p9-fb-23-incremental-ingest.md
git commit -m "docs(p9-fb-23): README + HANDOFF + HOTFIXES + INDEX + per-task spec

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Self-Review Notes (writer)

**Spec coverage:**
- Skip 4 conditions → Task 7 implementation matches. Each condition mapped to a specific check (`existing_asset.checksum == raw_asset.checksum`, `id_for_doc` includes parser_version, version-stamp comparisons).
- V006 migration → Task 3 (also adds put/get round-trip).
- `IngestItemKind::Unchanged` → Task 1.
- `IngestReport.unchanged` + `AggregateCounts.unchanged` → Task 1.
- `IngestEvent` event for unchanged → Task 7 emits `AssetFinished { result: Unchanged }`.
- `CanonicalDocument` extension → Task 2 (mechanical) + Task 5 (stamp on write).
- `get_asset_by_workspace_path` → Task 4.
- `IngestOpts.force_reingest` + `--force-reingest` flag → Tasks 6 + 8.
- Wire schema additive → Task 1 step 4.
- TUI status_line → Task 9.
- Docs sync → Task 10.

**Type / API consistency:**
- `IngestOpts` struct introduced in Task 6, consumed in Tasks 7 + 8 (both use the same field names: `progress`, `cancel`, `force_reingest`).
- `CanonicalDocument` field names `last_chunker_version` + `last_embedding_version` (Tasks 2, 3, 5, 7) — consistent.
- `DocumentStore::get_asset_by_workspace_path(&WorkspacePath) -> Option<RawAsset>` (Task 4) — consumed in Task 7.
- `AggregateCounts.unchanged` (Task 1) — consumed in Task 7 (`aggregate.unchanged += 1`) and Task 9 (`state.counts.unchanged`).

**Placeholder scan:** No `TBD` / `TODO`. Each step has full code or explicit "find via grep" guidance with the exact grep command.

**Risks documented:**
- Variable name uncertainty in Task 7 (real ingest loop has bound names that differ slightly from the plan's `raw_asset` / `total` / etc.). Plan flags "read 50 lines above" so the implementer adapts to actual context.
- Task 5 may need a chunker construction reordering (currently chunker built post-parse) — plan flags this explicitly.
- Task 8's CLI-test-coverage gap intentional — kebab-app integration test (Task 7) carries the meaningful coverage; CLI test would need a TempDir + tempfile fixture which is heavier than warranted.
