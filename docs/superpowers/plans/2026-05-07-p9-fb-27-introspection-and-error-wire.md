# p9-fb-27 Implementation Plan — Introspection + structured error wire

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `kebab schema [--json]` introspection command + `error.v1` wire schema for `--json` mode fatal errors. Unblocks fb-30 (MCP) and gives agents a stable surface for capability/version discovery and machine-readable error parsing.

**Architecture:** Two surfaces, one PR. (1) `kebab schema` builds a `SchemaV1` snapshot (wire / capabilities / models / stats) via a new `kebab_app::schema_with_config(&Config)` facade. (2) `error.v1` is emitted by a new `kebab-cli::error_classify::classify(&anyhow::Error)` function that downcasts known typed errors (`LlmError`, new `ConfigInvalid`, new `NotIndexed`) into structured `code` / `message` / `details` / `hint` records. Existing typed signals (`RefusalSignal`, `NoHitSignal`, `DoctorUnhealthy`) continue to drive exit codes 1 / 1 / 3 unchanged. Non-`--json` stderr text path is untouched.

**Tech Stack:** Rust 2024 workspace, serde + serde_json (existing), thiserror (existing in kebab-llm-local), anyhow (existing). No new dependencies.

**Spec source:** `docs/superpowers/specs/2026-05-07-p9-fb-27-introspection-and-error-wire-design.md` (commit f01f8df).

---

## File map

**Create:**
- `crates/kebab-app/src/error_signal.rs` — re-exports + new typed signal definitions
- `crates/kebab-app/src/schema.rs` — SchemaV1 struct + schema_with_config facade
- `crates/kebab-cli/src/error_classify.rs` — anyhow::Error → ErrorV1 dispatcher
- `crates/kebab-app/tests/schema_report.rs` — integration test for facade
- `crates/kebab-cli/tests/cli_schema.rs` — binary spawn test for `kebab schema --json`
- `crates/kebab-cli/tests/cli_error_wire.rs` — binary spawn test for error.v1 emission
- `docs/wire-schema/v1/schema.schema.json` — JSON Schema literal for schema.v1
- `docs/wire-schema/v1/error.schema.json` — JSON Schema literal for error.v1

**Modify:**
- `crates/kebab-app/src/lib.rs` — add `pub mod error_signal;` + `pub mod schema;` + re-export `SchemaV1`, `schema_with_config`
- `crates/kebab-config/src/lib.rs` — add `ConfigInvalid` typed error + wrap `from_file` errors
- `crates/kebab-store-sqlite/src/store.rs` — add `NotIndexed` typed error + wrap missing-DB / migration paths
- `crates/kebab-cli/src/main.rs` — add `Cmd::Schema` arm, replace `Err(e)` arm with json-mode classify branch, register new module
- `crates/kebab-cli/src/wire.rs` — add `wire_schema` + `wire_error_v1` helpers
- `tasks/p9/p9-fb-27-introspection-and-error-wire.md` — flip `status: open` → `completed`
- `tasks/HOTFIXES.md` — add `2026-05-?? — fb-27` entry
- `HANDOFF.md` — add one-line entry under "머지 후 발견된 결정"
- `README.md` — add `kebab schema` row to 명령 table
- `CLAUDE.md` — add `schema.v1` / `error.v1` to wire schema list
- `integrations/claude-code/kebab/SKILL.md` — additive note about `kebab schema` for capability discovery
- `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md` — §10 add capability matrix subsection + wire schema list extension

---

## Task 1: Define new typed signal module skeleton

**Files:**
- Create: `crates/kebab-app/src/error_signal.rs`
- Modify: `crates/kebab-app/src/lib.rs`

- [ ] **Step 1: Create `crates/kebab-app/src/error_signal.rs`**

```rust
//! Typed signal re-exports + new signals introduced by fb-27.
//!
//! kebab-cli (and future kebab-tui / kebab-desktop) downcast on these to
//! build `error.v1` wire records. The existing signals
//! (`RefusalSignal`, `NoHitSignal`, `DoctorUnhealthy`) live in
//! `doctor_signal.rs` — leave those unchanged and re-export via this
//! module so callers have one place to import from.
//!
//! See `docs/superpowers/specs/2026-05-07-p9-fb-27-introspection-and-error-wire-design.md`.

pub use crate::doctor_signal::{DoctorUnhealthy, NoHitSignal, RefusalSignal};

pub use kebab_config::ConfigInvalid;
pub use kebab_llm_local::LlmError;
pub use kebab_store_sqlite::NotIndexed;
```

- [ ] **Step 2: Wire the module into `crates/kebab-app/src/lib.rs`**

Find the existing line `pub mod doctor_signal;` (search for it; it's near the top of lib.rs). Add this line right after it:

```rust
pub mod error_signal;
```

- [ ] **Step 3: Verify the module skeleton compiles**

Run: `cargo check -p kebab-app`

Expected: build fails because `kebab_config::ConfigInvalid` and `kebab_store_sqlite::NotIndexed` do not exist yet. This is fine — we wire them up in Tasks 2 and 3. The point of this step is to confirm the module file is registered.

If the failure is anything other than missing `ConfigInvalid` / `NotIndexed`, stop and investigate.

- [ ] **Step 4: Comment out the not-yet-defined re-exports temporarily**

Edit `crates/kebab-app/src/error_signal.rs` and replace the bottom three `pub use` lines with:

```rust
pub use kebab_llm_local::LlmError;
// pub use kebab_config::ConfigInvalid;        // wired in Task 2
// pub use kebab_store_sqlite::NotIndexed;     // wired in Task 3
```

- [ ] **Step 5: Verify compile succeeds**

Run: `cargo check -p kebab-app`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/kebab-app/src/error_signal.rs crates/kebab-app/src/lib.rs
git commit -m "$(cat <<'EOF'
🏗️ chore(kebab-app): scaffold error_signal module (fb-27)

Re-exports existing doctor_signal entries (RefusalSignal / NoHitSignal /
DoctorUnhealthy) + LlmError from kebab-llm-local. ConfigInvalid /
NotIndexed re-exports added in subsequent tasks once the source crates
define them.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: Add `ConfigInvalid` typed error to kebab-config

**Files:**
- Modify: `crates/kebab-config/src/lib.rs`
- Test: same file (`#[cfg(test)] mod tests` or new top-level module)

- [ ] **Step 1: Write the failing test for ConfigInvalid downcast**

Add to the bottom of `crates/kebab-config/src/lib.rs` (inside the existing `#[cfg(test)] mod tests` if present, otherwise create one):

```rust
#[cfg(test)]
mod fb27_tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn config_invalid_carries_path_and_cause() {
        let nonexistent = PathBuf::from("/this/path/should/not/exist/kebab.toml");
        let err = Config::from_file(&nonexistent).unwrap_err();
        let signal = err.downcast_ref::<ConfigInvalid>()
            .expect("from_file error should downcast to ConfigInvalid");
        assert_eq!(signal.path, nonexistent);
        assert!(!signal.cause.is_empty(), "cause should be non-empty");
    }

    #[test]
    fn config_invalid_on_malformed_toml() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("bad.toml");
        std::fs::write(&p, "this is not [valid toml").unwrap();
        let err = Config::from_file(&p).unwrap_err();
        let signal = err.downcast_ref::<ConfigInvalid>()
            .expect("malformed TOML should downcast to ConfigInvalid");
        assert_eq!(signal.path, p);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p kebab-config fb27_tests -- --nocapture`
Expected: FAIL — `cannot find type ConfigInvalid in this scope`.

- [ ] **Step 3: Define the ConfigInvalid type at the top of `crates/kebab-config/src/lib.rs`**

Find a spot near the top of the file (after the module-level doc-comment and use statements, before the first `pub struct`). Add:

```rust
use std::path::PathBuf;

/// Signal: `Config::from_file` / `Config::load` failed due to missing path,
/// I/O failure, TOML parse failure, or post-parse validation failure.
///
/// Wrapped into `anyhow::Error` at the API boundary so callers that need
/// structured details (e.g. kebab-cli's `error_classify`) can
/// `downcast_ref::<ConfigInvalid>()` for the wire record.
#[derive(Debug, thiserror::Error)]
#[error("config invalid at {path}: {cause}")]
pub struct ConfigInvalid {
    pub path: PathBuf,
    pub cause: String,
}
```

If `thiserror` is not already a dependency of `kebab-config`, add it to `crates/kebab-config/Cargo.toml`:

```toml
thiserror = { workspace = true }
```

(check `Cargo.toml` workspace dependencies first — `thiserror` is already used by other crates so the workspace entry should exist).

- [ ] **Step 4: Wrap from_file error paths**

Find `pub fn from_file(path: &Path) -> anyhow::Result<Self>` in `crates/kebab-config/src/lib.rs`. Modify it so every `Err` branch wraps the underlying error in `ConfigInvalid`. Example pattern:

```rust
pub fn from_file(path: &Path) -> anyhow::Result<Self> {
    let raw = std::fs::read_to_string(path).map_err(|e| {
        anyhow::Error::new(ConfigInvalid {
            path: path.to_path_buf(),
            cause: format!("read failed: {e}"),
        })
    })?;
    let mut cfg: Config = toml::from_str(&raw).map_err(|e| {
        anyhow::Error::new(ConfigInvalid {
            path: path.to_path_buf(),
            cause: format!("parse failed: {e}"),
        })
    })?;
    cfg.source_dir = path.parent().map(PathBuf::from);
    cfg.validate().map_err(|e| {
        anyhow::Error::new(ConfigInvalid {
            path: path.to_path_buf(),
            cause: format!("validation failed: {e}"),
        })
    })?;
    Ok(cfg)
}
```

(Adapt to whatever the actual existing function does — preserve all current behavior, just add the wrapping.) The key invariant: after this task, every error returned by `from_file` must be downcastable to `ConfigInvalid`.

- [ ] **Step 5: Run test to verify pass**

Run: `cargo test -p kebab-config fb27_tests -- --nocapture`
Expected: PASS.

- [ ] **Step 6: Verify existing tests still pass**

Run: `cargo test -p kebab-config`
Expected: PASS, no regressions.

- [ ] **Step 7: Uncomment the kebab-app re-export**

Edit `crates/kebab-app/src/error_signal.rs`. Uncomment the `pub use kebab_config::ConfigInvalid;` line.

Run: `cargo check -p kebab-app`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add crates/kebab-config/src/lib.rs crates/kebab-config/Cargo.toml crates/kebab-app/src/error_signal.rs
git commit -m "$(cat <<'EOF'
🏗️ feat(kebab-config): add ConfigInvalid typed error (fb-27)

Wraps every error path in `Config::from_file` (read failure, TOML parse,
validation) so downstream callers can `downcast_ref::<ConfigInvalid>()`
to build the `error.v1` wire record. kebab-app re-exports the type via
its `error_signal` module.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: Add `NotIndexed` typed error to kebab-store-sqlite

**Files:**
- Modify: `crates/kebab-store-sqlite/src/store.rs` (or the file containing `SqliteStore::open`)
- Test: same crate

- [ ] **Step 1: Find the existing open / migrate entry point**

Run: `grep -n "fn open\|fn migrate\|pub fn new" crates/kebab-store-sqlite/src/store.rs | head -10`

Note the file + line of `SqliteStore::open` (or the equivalent). This is where the missing-DB / schema-mismatch detection lives.

- [ ] **Step 2: Write the failing test**

Add to `crates/kebab-store-sqlite/src/store.rs` (in the `#[cfg(test)] mod tests` block at the bottom — search for it):

```rust
#[test]
fn not_indexed_signal_emitted_when_db_missing() {
    let dir = tempfile::tempdir().unwrap();
    let nonexistent_db = dir.path().join("does-not-exist.sqlite");
    // Use whatever `open` API the crate exposes; this is the most likely
    // shape based on existing tests:
    let res = SqliteStore::open_existing(&nonexistent_db);
    let err = res.expect_err("opening a missing DB should fail");
    let signal = err.downcast_ref::<NotIndexed>()
        .expect("missing DB error should downcast to NotIndexed");
    assert_eq!(signal.expected, nonexistent_db.to_string_lossy());
}
```

If `SqliteStore::open_existing` does not exist as a separate API from `SqliteStore::open` (which auto-creates), introduce one — see Step 4. Adapt the test name to match the introduced API.

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p kebab-store-sqlite not_indexed_signal -- --nocapture`
Expected: FAIL — `NotIndexed` not defined.

- [ ] **Step 4: Define `NotIndexed` and the open_existing API**

Add to `crates/kebab-store-sqlite/src/store.rs` (top of file, near other type definitions):

```rust
/// Signal: SQLite database file does not exist, or schema_version does
/// not match the binary's expectation.
///
/// Distinct from generic I/O / SQL errors so kebab-cli can surface
/// `code: "not_indexed"` with a hint to run `kebab init` / `kebab ingest`.
#[derive(Debug, thiserror::Error)]
#[error("not indexed: expected={expected}, found={found:?}")]
pub struct NotIndexed {
    pub expected: String,
    pub found: Option<String>,
}
```

Make sure `thiserror = { workspace = true }` is in `crates/kebab-store-sqlite/Cargo.toml`.

Add a public `open_existing` method on `SqliteStore` — it differs from the existing `open` (which auto-creates) by returning `NotIndexed` when the DB file is absent:

```rust
impl SqliteStore {
    /// Open an existing SQLite DB at `path`. Unlike `open`, this does NOT
    /// create the file — if it is missing, returns a `NotIndexed` signal
    /// suitable for `error.v1` translation.
    pub fn open_existing(path: &std::path::Path) -> anyhow::Result<Self> {
        if !path.exists() {
            return Err(anyhow::Error::new(NotIndexed {
                expected: path.to_string_lossy().to_string(),
                found: None,
            }));
        }
        Self::open(path)
    }
}
```

If `open` already detects schema mismatch and returns an error, also wrap that error as `NotIndexed` with `found: Some(actual_version_str)`. (Inspect existing migration code; the schema_version row is in the `_refinery_schema_history` table.)

- [ ] **Step 5: Run test to verify pass**

Run: `cargo test -p kebab-store-sqlite not_indexed_signal -- --nocapture`
Expected: PASS.

- [ ] **Step 6: Verify existing tests still pass**

Run: `cargo test -p kebab-store-sqlite`
Expected: PASS, no regressions. (If anything fails, the new `NotIndexed` wrapping is too broad — narrow it back.)

- [ ] **Step 7: Uncomment the kebab-app re-export**

Edit `crates/kebab-app/src/error_signal.rs`. Uncomment the `pub use kebab_store_sqlite::NotIndexed;` line.

Run: `cargo check -p kebab-app`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add crates/kebab-store-sqlite/src/store.rs crates/kebab-store-sqlite/Cargo.toml crates/kebab-app/src/error_signal.rs
git commit -m "$(cat <<'EOF'
🏗️ feat(kebab-store-sqlite): add NotIndexed typed error (fb-27)

New `SqliteStore::open_existing` API + `NotIndexed` signal for the
missing-DB / schema-mismatch case. kebab-app re-exports the type via
its `error_signal` module so kebab-cli's `error_classify` can map it
to `error.v1 { code: "not_indexed" }`.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: Define `SchemaV1` struct + facade

**Files:**
- Create: `crates/kebab-app/src/schema.rs`
- Modify: `crates/kebab-app/src/lib.rs`

- [ ] **Step 1: Create `crates/kebab-app/src/schema.rs`**

```rust
//! `kebab schema` — introspection report. See spec
//! `docs/superpowers/specs/2026-05-07-p9-fb-27-introspection-and-error-wire-design.md`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use kebab_config::Config;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaV1 {
    pub kebab_version: String,
    pub wire: WireBlock,
    pub capabilities: Capabilities,
    pub models: Models,
    pub stats: Stats,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireBlock {
    pub schemas: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Capabilities {
    pub json_mode: bool,
    pub ingest_progress: bool,
    pub ingest_cancellation: bool,
    pub rag_multi_turn: bool,
    pub search_cache: bool,
    pub incremental_ingest: bool,
    pub streaming_ask: bool,
    pub http_daemon: bool,
    pub mcp_server: bool,
    pub single_file_ingest: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Models {
    pub parser_version: String,
    pub chunker_version: String,
    pub embedding_version: String,
    pub prompt_template_version: String,
    pub index_version: String,
    pub corpus_revision: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stats {
    pub doc_count: u64,
    pub chunk_count: u64,
    pub asset_count: u64,
    pub last_ingest_at: Option<String>,
}

const KEBAB_VERSION: &str = env!("CARGO_PKG_VERSION");

const WIRE_SCHEMAS: &[&str] = &[
    "answer.v1",
    "search_hit.v1",
    "doc_summary.v1",
    "chunk_inspection.v1",
    "doctor.v1",
    "ingest_report.v1",
    "ingest_progress.v1",
    "reset_report.v1",
    "citation.v1",
    "schema.v1",
    "error.v1",
];

pub fn schema_with_config(cfg: &Config) -> anyhow::Result<SchemaV1> {
    let store = open_store_for_stats(cfg)?;
    let stats = collect_stats(&store)?;
    let models = collect_models(cfg, &store)?;
    Ok(SchemaV1 {
        kebab_version: KEBAB_VERSION.to_string(),
        wire: WireBlock {
            schemas: WIRE_SCHEMAS.iter().map(|s| (*s).to_string()).collect(),
        },
        capabilities: capabilities_snapshot(),
        models,
        stats,
    })
}

fn capabilities_snapshot() -> Capabilities {
    Capabilities {
        json_mode: true,
        ingest_progress: true,
        ingest_cancellation: true,
        rag_multi_turn: true,
        search_cache: true,
        incremental_ingest: true,
        streaming_ask: false,
        http_daemon: false,
        mcp_server: false,
        single_file_ingest: false,
    }
}

// open_store_for_stats / collect_stats / collect_models implementation
// uses the existing kebab-app helpers for storage open + the
// kebab-store-sqlite COUNT queries. Inspect crates/kebab-app/src/lib.rs
// for the existing pattern (e.g. `list_docs_with_config` opens the store
// the same way) and mirror it here. The exact code is in Step 2 below.
```

- [ ] **Step 2: Add the helper functions to `crates/kebab-app/src/schema.rs`**

After the `pub fn schema_with_config` definition, add:

```rust
fn open_store_for_stats(cfg: &Config) -> anyhow::Result<kebab_store_sqlite::SqliteStore> {
    let data_dir = cfg.resolve_data_dir()?;
    let db_path = data_dir.join("kebab.sqlite");
    kebab_store_sqlite::SqliteStore::open_existing(&db_path)
}

fn collect_stats(store: &kebab_store_sqlite::SqliteStore) -> anyhow::Result<Stats> {
    let counts = store.count_summary()?; // see Task 5 — adds this method
    Ok(Stats {
        doc_count: counts.doc_count,
        chunk_count: counts.chunk_count,
        asset_count: counts.asset_count,
        last_ingest_at: counts.last_ingest_at,
    })
}

fn collect_models(cfg: &Config, store: &kebab_store_sqlite::SqliteStore) -> anyhow::Result<Models> {
    Ok(Models {
        parser_version: kebab_parse_md::PARSER_VERSION.to_string(),
        chunker_version: cfg.chunking.chunker_version.clone(),
        embedding_version: cfg.models.embedding.id.clone(),
        prompt_template_version: cfg.rag.prompt_template_version.clone(),
        index_version: kebab_store_vector::INDEX_VERSION_STR.to_string(),
        corpus_revision: store.corpus_revision()?,
    })
}
```

NOTE: The `kebab_parse_md::PARSER_VERSION` / `kebab_store_vector::INDEX_VERSION_STR` consts must be made `pub`. If they're currently private, add `pub` in their respective crates as part of this task. Run `grep -n "PARSER_VERSION\|INDEX_VERSION" crates/kebab-parse-md/src crates/kebab-store-vector/src` to locate them.

If the field path `cfg.rag.prompt_template_version` differs (the config schema stamps it under a different key), adjust accordingly — confirm by reading `crates/kebab-config/src/lib.rs` for the `RagCfg` struct.

If `Config::resolve_data_dir` does not exist, use the existing pattern from `kebab_app::list_docs_with_config` to derive the data_dir.

- [ ] **Step 3: Wire schema module into `crates/kebab-app/src/lib.rs`**

Add `pub mod schema;` near the top of `lib.rs` (next to `pub mod error_signal;` from Task 1).

Add re-exports:
```rust
pub use schema::{
    Capabilities, Models, SchemaV1, Stats, WireBlock, schema_with_config,
};
```

- [ ] **Step 4: Verify compile**

Run: `cargo check -p kebab-app`
Expected: PASS, OR fail with a missing API on `SqliteStore::count_summary` / `corpus_revision`. The latter is fine — `corpus_revision` already exists from p9-fb-19; `count_summary` is added in Task 5.

If `corpus_revision()` is missing, search for it: `grep -rn "fn corpus_revision\|bump_corpus_revision" crates/kebab-store-sqlite/src/`. It should exist from p9-fb-19. If not, **stop** — there's a deeper problem with the spec premise.

- [ ] **Step 5: Commit (will not yet build until Task 5 — that's OK, intermediate state is acceptable for atomic feature work)**

Hold off on commit until Task 5 makes things compile. Move directly to Task 5.

---

## Task 5: Add `count_summary` to SqliteStore

**Files:**
- Modify: `crates/kebab-store-sqlite/src/store.rs` (or sibling)
- Test: same crate

- [ ] **Step 1: Write the failing test**

Add at the bottom of `crates/kebab-store-sqlite/src/store.rs` (inside `#[cfg(test)] mod tests`):

```rust
#[test]
fn count_summary_zero_on_fresh_store() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("k.sqlite");
    let store = SqliteStore::open(&p).unwrap();
    let s = store.count_summary().unwrap();
    assert_eq!(s.doc_count, 0);
    assert_eq!(s.chunk_count, 0);
    assert_eq!(s.asset_count, 0);
    assert!(s.last_ingest_at.is_none());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p kebab-store-sqlite count_summary_zero -- --nocapture`
Expected: FAIL — `count_summary` not found.

- [ ] **Step 3: Add `CountSummary` struct + method**

Add to `crates/kebab-store-sqlite/src/store.rs`:

```rust
#[derive(Debug, Clone)]
pub struct CountSummary {
    pub doc_count: u64,
    pub chunk_count: u64,
    pub asset_count: u64,
    pub last_ingest_at: Option<String>,
}

impl SqliteStore {
    pub fn count_summary(&self) -> anyhow::Result<CountSummary> {
        let conn = self.conn();  // or however the crate exposes its Connection
        let doc_count: u64 = conn.query_row(
            "SELECT COUNT(*) FROM documents", [], |r| r.get(0)
        )?;
        let chunk_count: u64 = conn.query_row(
            "SELECT COUNT(*) FROM chunks", [], |r| r.get(0)
        )?;
        let asset_count: u64 = conn.query_row(
            "SELECT COUNT(*) FROM assets", [], |r| r.get(0)
        )?;
        let last_ingest_at: Option<String> = conn.query_row(
            "SELECT MAX(updated_at) FROM documents", [], |r| r.get(0)
        ).ok().flatten();
        Ok(CountSummary { doc_count, chunk_count, asset_count, last_ingest_at })
    }
}
```

The exact way to obtain the `Connection` (`self.conn()`, `&self.pool`, `r2d2`, etc.) depends on the existing crate structure. Inspect a similar method (e.g. how `corpus_revision()` reads from SQLite) and mirror that pattern. If the crate uses an internal `with_conn(|c| ...)` helper, use it.

- [ ] **Step 4: Run test to verify pass**

Run: `cargo test -p kebab-store-sqlite count_summary_zero -- --nocapture`
Expected: PASS.

- [ ] **Step 5: Verify whole crate**

Run: `cargo test -p kebab-store-sqlite`
Expected: PASS, no regressions.

- [ ] **Step 6: Verify kebab-app now compiles**

Run: `cargo check -p kebab-app`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/kebab-store-sqlite/src/store.rs crates/kebab-app/src/schema.rs crates/kebab-app/src/lib.rs
git commit -m "$(cat <<'EOF'
✨ feat(kebab-app): schema_with_config facade (fb-27)

New `SchemaV1` struct + `schema_with_config(&Config)` builder. Surfaces
wire schemas list, capabilities (current + future placeholders), model
versions (parser/chunker/embedding/prompt_template/index/corpus_revision),
and stats (doc/chunk/asset counts + last ingest). kebab-store-sqlite
gains `count_summary()` to back the stats block.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: Integration test — schema_with_config end-to-end

**Files:**
- Create: `crates/kebab-app/tests/schema_report.rs`

- [ ] **Step 1: Write the test**

```rust
//! Integration test: kebab_app::schema_with_config returns a SchemaV1
//! that is internally consistent with a freshly-ingested TempDir KB.

use std::fs;

#[path = "common/mod.rs"]
mod common;

#[test]
fn schema_report_reflects_freshly_ingested_kb() {
    let env = common::TestEnv::new();
    fs::write(env.workspace_root.join("a.md"), "# A\n\nbody A.").unwrap();
    fs::write(env.workspace_root.join("b.md"), "# B\n\nbody B.").unwrap();
    let _report = kebab_app::ingest_with_config(&env.config(), false).unwrap();

    let schema = kebab_app::schema_with_config(&env.config()).unwrap();

    assert!(!schema.kebab_version.is_empty());
    assert!(schema.wire.schemas.contains(&"schema.v1".to_string()));
    assert!(schema.wire.schemas.contains(&"error.v1".to_string()));
    assert!(schema.capabilities.json_mode);
    assert!(!schema.capabilities.streaming_ask);
    assert_eq!(schema.stats.doc_count, 2);
    assert!(schema.stats.last_ingest_at.is_some());
}

#[test]
fn schema_report_on_empty_kb_has_zero_counts() {
    let env = common::TestEnv::new();
    // No ingest.
    let schema = kebab_app::schema_with_config(&env.config()).unwrap();
    assert_eq!(schema.stats.doc_count, 0);
    assert_eq!(schema.stats.chunk_count, 0);
    assert!(schema.stats.last_ingest_at.is_none());
}
```

The `common::TestEnv` helper is the pattern used by the rest of the kebab-app integration tests. Verify with `ls crates/kebab-app/tests/common/` — if it does not exist, copy the helper inline (see `crates/kebab-app/tests/ingest_lexical.rs` for a working reference).

If a fresh, empty KB triggers `NotIndexed` because no `kebab init` has run, either:
- Have the test call `kebab_app::init_workspace_with_config(&env.config(), false).unwrap()` first, OR
- Make `schema_with_config` resilient to missing DB by populating zero counts (preferred). Update the spec's "stats on empty KB" to clarify either behavior. Choose the **init then schema** pattern for the test; document it in the test comment.

- [ ] **Step 2: Run test**

Run: `cargo test -p kebab-app --test schema_report`
Expected: PASS.

If it fails because a fresh TempDir has no DB → init the workspace first in the test. Adjust as noted above.

- [ ] **Step 3: Commit**

```bash
git add crates/kebab-app/tests/schema_report.rs
git commit -m "$(cat <<'EOF'
🧪 test(kebab-app): schema_with_config integration coverage (fb-27)

Two scenarios: freshly-ingested 2-doc KB (stats reflect counts +
last_ingest_at populated) and empty-but-initialized KB (counts zero,
last_ingest_at None).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: Add `wire_schema` + `wire_error_v1` helpers

**Files:**
- Modify: `crates/kebab-cli/src/wire.rs`

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` block at the bottom of `crates/kebab-cli/src/wire.rs`:

```rust
#[test]
fn schema_wrapper_tags_schema_version() {
    use kebab_app::{Capabilities, Models, SchemaV1, Stats, WireBlock};
    let schema = SchemaV1 {
        kebab_version: "0.2.1".to_string(),
        wire: WireBlock { schemas: vec!["answer.v1".to_string()] },
        capabilities: Capabilities {
            json_mode: true, ingest_progress: true, ingest_cancellation: true,
            rag_multi_turn: true, search_cache: true, incremental_ingest: true,
            streaming_ask: false, http_daemon: false, mcp_server: false,
            single_file_ingest: false,
        },
        models: Models {
            parser_version: "x".to_string(),
            chunker_version: "y".to_string(),
            embedding_version: "z".to_string(),
            prompt_template_version: "w".to_string(),
            index_version: "v".to_string(),
            corpus_revision: 7,
        },
        stats: Stats {
            doc_count: 1, chunk_count: 2, asset_count: 1,
            last_ingest_at: None,
        },
    };
    let v = wire_schema(&schema);
    assert_eq!(schema_of(&v), Some("schema.v1"));
    assert_eq!(v.get("kebab_version").and_then(Value::as_str), Some("0.2.1"));
}

#[test]
fn error_wrapper_tags_schema_version_and_emits_code() {
    use crate::error_classify::ErrorV1;
    let err = ErrorV1 {
        code: "config_invalid".to_string(),
        message: "bad config".to_string(),
        details: serde_json::json!({"path": "/tmp/x"}),
        hint: Some("check the path".to_string()),
    };
    let v = wire_error_v1(&err);
    assert_eq!(schema_of(&v), Some("error.v1"));
    assert_eq!(v.get("code").and_then(Value::as_str), Some("config_invalid"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p kebab-cli --lib wire::tests`
Expected: FAIL — `wire_schema` / `wire_error_v1` / `ErrorV1` not defined.

- [ ] **Step 3: Add helpers to `crates/kebab-cli/src/wire.rs`**

```rust
/// Wrap a [`SchemaV1`] as `schema.v1`.
pub fn wire_schema(s: &kebab_app::SchemaV1) -> Value {
    let v = serde_json::to_value(s).expect("SchemaV1 serializes");
    tag_object(v, "schema.v1")
}

/// Wrap an [`ErrorV1`] as `error.v1`.
pub fn wire_error_v1(e: &crate::error_classify::ErrorV1) -> Value {
    let v = serde_json::to_value(e).expect("ErrorV1 serializes");
    tag_object(v, "error.v1")
}
```

Tests will not yet pass because `error_classify::ErrorV1` does not exist — Task 8 adds it. Hold off on the wire test run until Task 8.

- [ ] **Step 4: Move on to Task 8 (no commit yet — wire helpers + classify ship together)**

---

## Task 8: Define `ErrorV1` + `classify` function

**Files:**
- Create: `crates/kebab-cli/src/error_classify.rs`
- Modify: `crates/kebab-cli/src/main.rs` (just adds `mod error_classify;`)

- [ ] **Step 1: Create `crates/kebab-cli/src/error_classify.rs`**

```rust
//! Map `anyhow::Error` (returned by `kebab-app` facade calls) to the
//! `error.v1` wire shape. The classifier downcasts to known typed errors
//! re-exported via `kebab_app::error_signal` (LlmError, ConfigInvalid,
//! NotIndexed) and falls back to `code: "generic"` for everything else.
//!
//! Refusal / no-hit / doctor-unhealthy are NOT routed here — they remain
//! exit-code-only signals (see main.rs `exit_code()`).

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use kebab_app::error_signal::{ConfigInvalid, LlmError, NotIndexed};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorV1 {
    pub code: String,
    pub message: String,
    pub details: Value,
    pub hint: Option<String>,
}

pub fn classify(err: &anyhow::Error, verbose: bool) -> ErrorV1 {
    if let Some(s) = err.downcast_ref::<ConfigInvalid>() {
        return ErrorV1 {
            code: "config_invalid".to_string(),
            message: s.to_string(),
            details: json!({
                "path": s.path.to_string_lossy(),
                "cause": s.cause,
            }),
            hint: Some("check `--config <path>` and TOML syntax".to_string()),
        };
    }
    if let Some(s) = err.downcast_ref::<NotIndexed>() {
        return ErrorV1 {
            code: "not_indexed".to_string(),
            message: s.to_string(),
            details: json!({
                "expected": s.expected,
                "found": s.found,
            }),
            hint: Some("run `kebab init` then `kebab ingest`".to_string()),
        };
    }
    if let Some(s) = err.downcast_ref::<LlmError>() {
        return classify_llm(s);
    }
    if let Some(io) = err.downcast_ref::<std::io::Error>() {
        return ErrorV1 {
            code: "io_error".to_string(),
            message: io.to_string(),
            details: json!({"kind": format!("{:?}", io.kind())}),
            hint: None,
        };
    }
    let mut details = json!({});
    if verbose {
        let chain: Vec<String> = err.chain().map(|c| c.to_string()).collect();
        details = json!({"chain": chain});
    }
    ErrorV1 {
        code: "generic".to_string(),
        message: err.to_string(),
        details,
        hint: None,
    }
}

fn classify_llm(s: &LlmError) -> ErrorV1 {
    match s {
        LlmError::Unreachable { endpoint, source } => ErrorV1 {
            code: "model_unreachable".to_string(),
            message: format!("ollama unreachable at {endpoint}"),
            details: json!({
                "endpoint": endpoint,
                "source": source.to_string(),
            }),
            hint: Some(format!("ensure `ollama serve` is reachable at {endpoint}")),
        },
        LlmError::ModelNotPulled(model) => ErrorV1 {
            code: "model_not_pulled".to_string(),
            message: format!("ollama model `{model}` is not pulled"),
            details: json!({"model": model}),
            hint: Some(format!("run `ollama pull {model}`")),
        },
        LlmError::Timeout(e) => ErrorV1 {
            code: "timeout".to_string(),
            message: format!("ollama timeout: {e}"),
            details: json!({"source": e.to_string()}),
            hint: Some("increase timeout or check Ollama load".to_string()),
        },
        LlmError::Stream(body) => ErrorV1 {
            code: "generic".to_string(),
            message: format!("ollama HTTP error: {body}"),
            details: json!({"body": body}),
            hint: None,
        },
        LlmError::Malformed(line) => ErrorV1 {
            code: "generic".to_string(),
            message: format!("malformed response line: {line}"),
            details: json!({"line": line}),
            hint: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_invalid_classifies_to_config_invalid_code() {
        let err = anyhow::Error::new(ConfigInvalid {
            path: std::path::PathBuf::from("/tmp/x.toml"),
            cause: "missing".to_string(),
        });
        let v1 = classify(&err, false);
        assert_eq!(v1.code, "config_invalid");
        assert_eq!(v1.details.get("path").and_then(|p| p.as_str()), Some("/tmp/x.toml"));
        assert!(v1.hint.is_some());
    }

    #[test]
    fn not_indexed_classifies_correctly() {
        let err = anyhow::Error::new(NotIndexed {
            expected: "/data/k.sqlite".to_string(),
            found: None,
        });
        let v1 = classify(&err, false);
        assert_eq!(v1.code, "not_indexed");
    }

    #[test]
    fn llm_unreachable_classifies_to_model_unreachable() {
        // We cannot construct a reqwest::Error from scratch (private constructor).
        // Use a real network call with a guaranteed-unroutable endpoint:
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_millis(50))
            .build().unwrap();
        let err = client.get("http://127.0.0.1:1").send().unwrap_err();
        let llm = LlmError::Unreachable {
            endpoint: "http://127.0.0.1:1".to_string(),
            source: err,
        };
        let anyhow_err = anyhow::Error::new(llm);
        let v1 = classify(&anyhow_err, false);
        assert_eq!(v1.code, "model_unreachable");
    }

    #[test]
    fn model_not_pulled_classifies_correctly() {
        let llm = LlmError::ModelNotPulled("gemma4:e4b".to_string());
        let v1 = classify(&anyhow::Error::new(llm), false);
        assert_eq!(v1.code, "model_not_pulled");
        assert_eq!(v1.details.get("model").and_then(|p| p.as_str()), Some("gemma4:e4b"));
    }

    #[test]
    fn unknown_error_classifies_to_generic() {
        let err = anyhow::anyhow!("something else");
        let v1 = classify(&err, false);
        assert_eq!(v1.code, "generic");
        assert!(v1.hint.is_none());
    }

    #[test]
    fn generic_with_verbose_includes_chain() {
        let err = anyhow::anyhow!("root").context("middle").context("leaf");
        let v1 = classify(&err, true);
        assert_eq!(v1.code, "generic");
        let chain = v1.details.get("chain").and_then(|c| c.as_array()).unwrap();
        assert_eq!(chain.len(), 3);
    }

    #[test]
    fn io_error_classifies_correctly() {
        let io = std::io::Error::new(std::io::ErrorKind::NotFound, "no such file");
        let err = anyhow::Error::new(io);
        let v1 = classify(&err, false);
        assert_eq!(v1.code, "io_error");
    }
}
```

If `reqwest` is not already a dev-dependency of `kebab-cli`, add it to `[dev-dependencies]` in `crates/kebab-cli/Cargo.toml` (using the workspace dep).

- [ ] **Step 2: Register the module in `crates/kebab-cli/src/main.rs`**

At the top of `main.rs` (alongside other `mod` declarations), add:

```rust
mod error_classify;
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p kebab-cli --lib error_classify::tests`
Expected: PASS (all 7 tests).

Run: `cargo test -p kebab-cli --lib wire::tests`
Expected: PASS (the schema/error wire tests added in Task 7 now pass).

- [ ] **Step 4: Commit**

```bash
git add crates/kebab-cli/src/error_classify.rs crates/kebab-cli/src/main.rs crates/kebab-cli/src/wire.rs crates/kebab-cli/Cargo.toml
git commit -m "$(cat <<'EOF'
✨ feat(kebab-cli): error_classify + wire_error_v1 (fb-27)

Maps anyhow chain → ErrorV1 wire record by downcasting to known typed
errors (LlmError / ConfigInvalid / NotIndexed / std::io::Error). Generic
fallback emits `code: "generic"` with the chain in `details` when
verbose. wire.rs adds wire_schema / wire_error_v1 wrappers consistent
with the existing tag_object pattern.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 9: `Cmd::Schema` CLI subcommand

**Files:**
- Modify: `crates/kebab-cli/src/main.rs`

- [ ] **Step 1: Add the subcommand variant**

Find the `enum Cmd` definition in `crates/kebab-cli/src/main.rs`. Add a new variant:

```rust
/// Print introspection report (wire schemas, capabilities, model versions, stats).
Schema,
```

- [ ] **Step 2: Wire the arm in `fn run`**

Inside `fn run(cli: &Cli) -> anyhow::Result<()>`, in the `match &cli.command` block, add a new arm:

```rust
Cmd::Schema => {
    let cfg = kebab_config::Config::load(cli.config.as_deref())?;
    let report = kebab_app::schema_with_config(&cfg)?;
    if cli.json {
        let v = wire::wire_schema(&report);
        println!("{}", serde_json::to_string(&v)?);
    } else {
        print_schema_text(&report);
    }
    Ok(())
}
```

- [ ] **Step 3: Add the human-friendly text printer**

Add to `crates/kebab-cli/src/main.rs` (near other helpers, e.g. after `fn exit_code`):

```rust
fn print_schema_text(s: &kebab_app::SchemaV1) {
    println!("kebab v{}\n", s.kebab_version);

    println!("wire schemas");
    println!("  {}", s.wire.schemas.join(", "));
    println!();

    println!("capabilities");
    let caps = [
        ("json_mode", s.capabilities.json_mode),
        ("ingest_progress", s.capabilities.ingest_progress),
        ("ingest_cancellation", s.capabilities.ingest_cancellation),
        ("rag_multi_turn", s.capabilities.rag_multi_turn),
        ("search_cache", s.capabilities.search_cache),
        ("incremental_ingest", s.capabilities.incremental_ingest),
        ("streaming_ask", s.capabilities.streaming_ask),
        ("http_daemon", s.capabilities.http_daemon),
        ("mcp_server", s.capabilities.mcp_server),
        ("single_file_ingest", s.capabilities.single_file_ingest),
    ];
    for (name, on) in caps {
        let mark = if on { "✓" } else { "✗" };
        println!("  {mark} {name}");
    }
    println!();

    println!("models");
    println!("  parser_version          {}", s.models.parser_version);
    println!("  chunker_version         {}", s.models.chunker_version);
    println!("  embedding_version       {}", s.models.embedding_version);
    println!("  prompt_template_version {}", s.models.prompt_template_version);
    println!("  index_version           {}", s.models.index_version);
    println!("  corpus_revision         {}", s.models.corpus_revision);
    println!();

    println!("stats");
    println!("  doc_count               {}", s.stats.doc_count);
    println!("  chunk_count             {}", s.stats.chunk_count);
    println!("  asset_count             {}", s.stats.asset_count);
    let last = s.stats.last_ingest_at.as_deref().unwrap_or("(never)");
    println!("  last_ingest_at          {last}");
}
```

- [ ] **Step 4: Smoke check — build the binary**

Run: `cargo build -p kebab-cli`
Expected: PASS.

Run: `target/debug/kebab schema --help 2>&1 | head -5`
Expected: shows the `schema` subcommand help.

- [ ] **Step 5: Manual smoke against /tmp**

```bash
mkdir -p /tmp/kebab-fb27-smoke
cat > /tmp/kebab-fb27-smoke/config.toml <<'EOF'
[workspace]
root = "/tmp/kebab-fb27-smoke/notes"

[storage]
data_dir = "/tmp/kebab-fb27-smoke/data"

[models.embedding]
id = "fastembed-mle5small-384"
EOF
mkdir -p /tmp/kebab-fb27-smoke/notes
target/debug/kebab --config /tmp/kebab-fb27-smoke/config.toml init --force
target/debug/kebab --config /tmp/kebab-fb27-smoke/config.toml schema
target/debug/kebab --config /tmp/kebab-fb27-smoke/config.toml --json schema | jq .
```

Expected: text output shows the layout from Task 5; JSON output is well-formed and contains `schema_version: "schema.v1"`.

- [ ] **Step 6: Commit**

```bash
git add crates/kebab-cli/src/main.rs
git commit -m "$(cat <<'EOF'
✨ feat(kebab-cli): kebab schema subcommand (fb-27)

Text mode: doctor-style key/value layout. JSON mode: schema.v1 wire
record. Honors `--config <path>` via the established
`kebab_app::schema_with_config(&cfg)` facade pattern (per the P3-5 /
P4-3 regression conventions).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 10: Replace `Err(e)` arm in `main()` with json-mode classify branch

**Files:**
- Modify: `crates/kebab-cli/src/main.rs`

- [ ] **Step 1: Locate the existing main() Err arm**

Search: `grep -n "fn main\|exit_code\|cli.json" crates/kebab-cli/src/main.rs | head -20`. Find the `match run(&cli)` block. The current `Err(e)` arm prints to stderr.

- [ ] **Step 2: Replace it with the json-aware branch**

Edit the `Err(e)` arm to look like this:

```rust
Err(e) => {
    let code = exit_code(&e);
    if code != 1 {
        if cli.json {
            let v1 = error_classify::classify(&e, cli.verbose);
            let v = wire::wire_error_v1(&v1);
            eprintln!("{}", serde_json::to_string(&v).unwrap_or_else(|_| {
                "{\"schema_version\":\"error.v1\",\"code\":\"generic\",\"message\":\"serialize failed\"}".to_string()
            }));
        } else {
            eprintln!("error: {e}");
            if cli.verbose {
                for cause in e.chain().skip(1) {
                    eprintln!("  caused by: {cause}");
                }
            }
        }
    }
    ExitCode::from(code)
}
```

The existing branch already has the non-JSON form — just wrap it in the `cli.json` if/else.

- [ ] **Step 3: Build + smoke**

Run: `cargo build -p kebab-cli`
Expected: PASS.

```bash
target/debug/kebab --json --config /nonexistent ingest 2>&1 1>/dev/null | jq .
```

Expected: stderr contains a single ndjson line; `jq .` parses it; `.schema_version == "error.v1"`; `.code == "config_invalid"`.

```bash
target/debug/kebab --config /nonexistent ingest 2>&1 1>/dev/null
```

Expected: stderr shows the legacy text form (`error: config invalid at /nonexistent: read failed: ...`).

- [ ] **Step 4: Commit**

```bash
git add crates/kebab-cli/src/main.rs
git commit -m "$(cat <<'EOF'
✨ feat(kebab-cli): emit error.v1 ndjson on stderr in --json mode (fb-27)

Wraps the existing `Err(e)` arm with a `cli.json` branch:
- `--json`: stderr ndjson `error.v1` via wire_error_v1
- non-`--json`: legacy `error: <msg>` text path (unchanged)

exit_code() unchanged — RefusalSignal/NoHitSignal/DoctorUnhealthy
still drive 1/1/3.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 11: Integration test — `kebab schema --json` end-to-end

**Files:**
- Create: `crates/kebab-cli/tests/cli_schema.rs`

- [ ] **Step 1: Write the test**

```rust
//! Integration: spawn the kebab binary and parse `kebab schema --json`.

use std::process::Command;

#[path = "common/mod.rs"]
mod common;

#[test]
fn cli_schema_json_emits_schema_v1() {
    let env = common::CliEnv::new();
    env.run(&["init", "--force"]).success();
    let out = env.run(&["--json", "schema"]).success().stdout();
    let v: serde_json::Value = serde_json::from_str(&out).expect("valid JSON");
    assert_eq!(v.get("schema_version").and_then(|s| s.as_str()), Some("schema.v1"));
    assert!(v.get("kebab_version").and_then(|s| s.as_str()).unwrap().len() > 0);
    let caps = v.get("capabilities").unwrap().as_object().unwrap();
    assert_eq!(caps.get("json_mode").and_then(|b| b.as_bool()), Some(true));
    assert_eq!(caps.get("mcp_server").and_then(|b| b.as_bool()), Some(false));
}

#[test]
fn cli_schema_text_mode_runs() {
    let env = common::CliEnv::new();
    env.run(&["init", "--force"]).success();
    let out = env.run(&["schema"]).success().stdout();
    assert!(out.contains("kebab v"));
    assert!(out.contains("capabilities"));
    assert!(out.contains("models"));
    assert!(out.contains("stats"));
}
```

`common::CliEnv` is the existing test harness for kebab-cli integration tests. Inspect `crates/kebab-cli/tests/common/mod.rs` to confirm the API; if `run().success().stdout()` differs (e.g. the helper returns an `assert_cmd::Output`), adapt the calls. If the harness does not exist, write a minimal one inline using `std::process::Command`.

- [ ] **Step 2: Run test**

Run: `cargo test -p kebab-cli --test cli_schema`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/kebab-cli/tests/cli_schema.rs
git commit -m "🧪 test(kebab-cli): integration coverage for kebab schema (fb-27)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 12: Integration test — error.v1 emission on stderr

**Files:**
- Create: `crates/kebab-cli/tests/cli_error_wire.rs`

- [ ] **Step 1: Write the test**

```rust
//! Integration: spawn kebab and verify --json mode emits error.v1 ndjson
//! on stderr while non-json mode emits legacy text.

#[path = "common/mod.rs"]
mod common;

#[test]
fn json_mode_emits_error_v1_on_config_missing() {
    let env = common::CliEnv::new();
    let out = env
        .raw_args(&["--json", "--config", "/this/does/not/exist", "ingest"])
        .run_expect_failure();
    assert_eq!(out.exit_code, 2);
    let stderr_line = out.stderr.lines().next().expect("stderr has a line");
    let v: serde_json::Value = serde_json::from_str(stderr_line)
        .expect("stderr first line is JSON");
    assert_eq!(v.get("schema_version").and_then(|s| s.as_str()), Some("error.v1"));
    assert_eq!(v.get("code").and_then(|s| s.as_str()), Some("config_invalid"));
}

#[test]
fn text_mode_emits_legacy_error_format() {
    let env = common::CliEnv::new();
    let out = env
        .raw_args(&["--config", "/this/does/not/exist", "ingest"])
        .run_expect_failure();
    assert_eq!(out.exit_code, 2);
    assert!(out.stderr.starts_with("error:"));
    // Verify it does NOT look like JSON — no leading `{`.
    assert!(!out.stderr.trim_start().starts_with('{'));
}
```

Adapt `raw_args` / `run_expect_failure` to the existing `common::CliEnv` API. If the API is different, mirror the patterns from existing tests like `cli_ingest_progress.rs` or similar.

- [ ] **Step 2: Run test**

Run: `cargo test -p kebab-cli --test cli_error_wire`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/kebab-cli/tests/cli_error_wire.rs
git commit -m "🧪 test(kebab-cli): integration coverage for error.v1 (fb-27)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 13: JSON Schema literal — `schema.v1`

**Files:**
- Create: `docs/wire-schema/v1/schema.schema.json`

- [ ] **Step 1: Write the schema**

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://kebab.local/wire-schema/v1/schema.schema.json",
  "title": "schema.v1",
  "description": "kebab introspection report — wire schemas, capabilities, model versions, and index stats.",
  "type": "object",
  "required": ["schema_version", "kebab_version", "wire", "capabilities", "models", "stats"],
  "properties": {
    "schema_version": { "const": "schema.v1" },
    "kebab_version": { "type": "string" },
    "wire": {
      "type": "object",
      "required": ["schemas"],
      "properties": {
        "schemas": {
          "type": "array",
          "items": { "type": "string", "pattern": "^[a-z_]+\\.v[0-9]+$" }
        }
      }
    },
    "capabilities": {
      "type": "object",
      "additionalProperties": { "type": "boolean" },
      "required": [
        "json_mode", "ingest_progress", "ingest_cancellation",
        "rag_multi_turn", "search_cache", "incremental_ingest",
        "streaming_ask", "http_daemon", "mcp_server", "single_file_ingest"
      ]
    },
    "models": {
      "type": "object",
      "required": [
        "parser_version", "chunker_version", "embedding_version",
        "prompt_template_version", "index_version", "corpus_revision"
      ],
      "properties": {
        "parser_version": { "type": "string" },
        "chunker_version": { "type": "string" },
        "embedding_version": { "type": "string" },
        "prompt_template_version": { "type": "string" },
        "index_version": { "type": "string" },
        "corpus_revision": { "type": "integer", "minimum": 0 }
      }
    },
    "stats": {
      "type": "object",
      "required": ["doc_count", "chunk_count", "asset_count", "last_ingest_at"],
      "properties": {
        "doc_count": { "type": "integer", "minimum": 0 },
        "chunk_count": { "type": "integer", "minimum": 0 },
        "asset_count": { "type": "integer", "minimum": 0 },
        "last_ingest_at": {
          "anyOf": [
            { "type": "string", "format": "date-time" },
            { "type": "null" }
          ]
        }
      }
    }
  }
}
```

- [ ] **Step 2: Validate it parses**

Run: `python3 -c "import json; json.load(open('docs/wire-schema/v1/schema.schema.json'))"`
Expected: no output (valid JSON).

- [ ] **Step 3: Commit**

```bash
git add docs/wire-schema/v1/schema.schema.json
git commit -m "📝 docs(wire-schema): schema.v1 JSON Schema (fb-27)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 14: JSON Schema literal — `error.v1`

**Files:**
- Create: `docs/wire-schema/v1/error.schema.json`

- [ ] **Step 1: Write the schema**

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://kebab.local/wire-schema/v1/error.schema.json",
  "title": "error.v1",
  "description": "Structured fatal error emitted on stderr in --json mode.",
  "type": "object",
  "required": ["schema_version", "code", "message", "details"],
  "properties": {
    "schema_version": { "const": "error.v1" },
    "code": {
      "type": "string",
      "enum": [
        "config_invalid",
        "not_indexed",
        "model_unreachable",
        "model_not_pulled",
        "timeout",
        "io_error",
        "generic"
      ]
    },
    "message": { "type": "string" },
    "details": { "type": "object" },
    "hint": {
      "anyOf": [
        { "type": "string" },
        { "type": "null" }
      ]
    }
  }
}
```

- [ ] **Step 2: Validate**

Run: `python3 -c "import json; json.load(open('docs/wire-schema/v1/error.schema.json'))"`
Expected: no output.

- [ ] **Step 3: Commit**

```bash
git add docs/wire-schema/v1/error.schema.json
git commit -m "📝 docs(wire-schema): error.v1 JSON Schema (fb-27)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 15: Doc sync — README, HANDOFF, CLAUDE.md, integrations skill

**Files:**
- Modify: `README.md`
- Modify: `HANDOFF.md`
- Modify: `CLAUDE.md`
- Modify: `integrations/claude-code/kebab/SKILL.md`

- [ ] **Step 1: README.md — add `kebab schema` row to commands table**

Find the 명령 (commands) table in `README.md`. Add a row describing `kebab schema`:

```markdown
| `kebab schema` | introspection (wire schemas / capabilities / models / stats); `--json` for `schema.v1` wire |
```

The exact column structure depends on the table — match the surrounding rows.

If there's a Configuration / wire schema reference section, add `schema.v1` and `error.v1` to the list.

- [ ] **Step 2: HANDOFF.md — add one-line entry**

In the `## 머지 후 발견된 버그 / 결정 (요약)` section, add (date as appropriate):

```markdown
- **2026-05-?? P9 post-도그푸딩 (p9-fb-27)** — `kebab schema [--json]` introspection 명령 + `error.v1` wire 도입. 정적 (wire schemas / capabilities / models) + 동적 (stats) 한 번에. `--json` 모드에서 fatal error 가 stderr ndjson 으로 emit (비 `--json` 은 기존 stderr text 유지). exit code 0/1/2/3 unchanged — `code` 필드가 fine-grained 분기. fb-30 MCP `initialize` capability matrix 의 prerequisite. spec: `tasks/p9/p9-fb-27-introspection-and-error-wire.md`. design: `docs/superpowers/specs/2026-05-07-p9-fb-27-introspection-and-error-wire-design.md`.
```

- [ ] **Step 3: CLAUDE.md — wire schema list update**

Find the "Wire schema v1" section. Add `schema.v1` and `error.v1` to the wire schema enumeration. Mention that `--json` mode now emits `error.v1` on stderr for fatal errors.

- [ ] **Step 4: integrations/claude-code/kebab/SKILL.md — additive note**

Add a sentence to the description / usage section noting that the skill can call `kebab --json schema` for capability discovery (gates streaming / multi-turn / etc. based on `capabilities.*`). Don't require it — keep additive.

- [ ] **Step 5: design doc — §10 capability matrix subsection**

Edit `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md`. Find §10 (line 1372 baseline). Add a subsection (after the existing exit-code table, before §11):

```markdown
### 10.1 Capability matrix + introspection (fb-27)

`kebab schema [--json]` 가 binary 의 capability set 을 노출한다.
`schema.v1` wire schema 가 `wire.schemas` (지원 wire id 목록), `capabilities`
(bool flag, 미래 surface 의 placeholder 도 항상 포함), `models` (cascade
version 6축), `stats` (doc/chunk/asset count + last_ingest_at) 를 한 호출로 반환한다.

`error.v1` wire schema 가 `--json` 모드에서 fatal error 를 stderr ndjson 으로
emit. code 7개 initial set: `config_invalid` / `not_indexed` /
`model_unreachable` / `model_not_pulled` / `timeout` / `io_error` /
`generic`. exit code 0/1/2/3 unchanged — `error.v1.code` 가 fine-grained
agent 분기 source.
```

- [ ] **Step 6: Commit**

```bash
git add README.md HANDOFF.md CLAUDE.md integrations/claude-code/kebab/SKILL.md docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
git commit -m "$(cat <<'EOF'
📝 docs: sync README / HANDOFF / CLAUDE / skill / design for fb-27

- README 명령 표 에 `kebab schema` 추가
- HANDOFF post-도그푸딩 항목 한 줄
- CLAUDE.md wire schema 절 schema.v1 / error.v1 추가
- integrations skill — schema 활용 안내 (additive)
- design §10.1 capability matrix subsection 신설

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 16: HOTFIXES entry + task spec status flip

**Files:**
- Modify: `tasks/HOTFIXES.md`
- Modify: `tasks/p9/p9-fb-27-introspection-and-error-wire.md`

- [ ] **Step 1: Add HOTFIXES entry**

Insert a new dated entry at the top of `tasks/HOTFIXES.md` (right after the `# Post-merge hotfixes log` header, before the most recent existing entry):

```markdown
## 2026-05-?? — p9-fb-27 (post-dogfooding): introspection (`kebab schema`) + structured error wire

**Source feedback**: 사용자 도그푸딩 2026-05-06 — agent 가 kebab 인스턴스의 wire 버전 / 기능 / 모델 / 인덱스 통계 introspect 못 함; error 가 stderr text 라 substring 분기 필요.

**Live binding 변경**:

- 신규 명령 `kebab schema [--json]` — text / `schema.v1` JSON. `--config <path>` honor.
- 신규 wire `schema.v1` — `kebab_version` / `wire.schemas` / `capabilities` (10 bool, 4 미래 surface 포함) / `models` (parser/chunker/embedding/prompt_template/index/corpus_revision 6축) / `stats` (doc/chunk/asset count + last_ingest_at).
- 신규 wire `error.v1` — `--json` 모드에서 fatal error 가 stderr ndjson 으로 emit. 비 `--json` 은 기존 stderr text 유지.
- error code 7개 initial set: `config_invalid` (`ConfigInvalid` signal in kebab-config) / `not_indexed` (`NotIndexed` in kebab-store-sqlite, `SqliteStore::open_existing` API 신규) / `model_unreachable` (`LlmError::Unreachable`) / `model_not_pulled` (`LlmError::ModelNotPulled`) / `timeout` (`LlmError::Timeout`) / `io_error` (`std::io::Error` chain detection) / `generic` (catch-all, verbose 시 `details.chain` 채움).
- exit code 0/1/2/3 unchanged — `RefusalSignal` / `NoHitSignal` / `DoctorUnhealthy` 만 보고 1/1/3 결정. 신규 5 signal 모두 fall-through → 2.
- `kebab-app::error_signal` 모듈 신규 — `doctor_signal` 과 신규 typed error 들 한 곳에서 re-export.
- `kebab-store-sqlite::SqliteStore::count_summary` 메서드 신규 — `schema.v1.stats` block backing.

**Spec contract impact**: design §10 에 §10.1 capability matrix subsection 추가 — `schema.v1` / `error.v1` wire 명시.

**Tests added**: kebab-config fb27_tests (2: ConfigInvalid downcast / malformed TOML), kebab-store-sqlite (2: NotIndexed signal + count_summary zero state), kebab-cli error_classify::tests (7: 7 code 분류 + verbose chain), kebab-cli wire::tests (2: schema.v1 / error.v1 round-trip), kebab-app schema_report integration (2: ingested + empty), kebab-cli cli_schema integration (2: --json + text), kebab-cli cli_error_wire integration (2: --json error.v1 + legacy text).

**Known limitation (deferred)**:

- `IoFailure` typed signal 도입 안 함 — `std::io::Error` chain detection 으로 충분. 발생지가 새 typed signal 필요해지면 case-by-case.
- `OpTimeout` 별 typed signal 도입 안 함 — 현재 `LlmError::Timeout` 하나로 충분 (LLM stream). embed batch / vector upsert timeout 이 별도로 surface 되면 후속 task.
- error code 확장 (예 `embedding_dim_mismatch`, `daemon_locked`, `mcp_protocol_error`) — 발생지 추가 시점 case-by-case (additive, error.v1 major bump 불필요).
- README / claude-code skill 의 `kebab schema` 사용 예시 확장 — 본 항목은 skill description 한 줄만, 본격 활용 가이드는 fb-30 MCP 머지 시점에 동시 갱신.
```

- [ ] **Step 2: Flip task spec status**

Edit `tasks/p9/p9-fb-27-introspection-and-error-wire.md`. Change the frontmatter line:

```yaml
status: open
```

to:

```yaml
status: completed
```

Also update the warning banner at the top — change the wording from "백로그 only — 미구현" to a "구현 완료. 본 spec 은 구현 시점의 frozen 상태이며, post-merge deviation 은 [HOTFIXES.md](../../tasks/HOTFIXES.md) 의 2026-05-?? — p9-fb-27 항목 참조." line.

- [ ] **Step 3: Commit**

```bash
git add tasks/HOTFIXES.md tasks/p9/p9-fb-27-introspection-and-error-wire.md
git commit -m "$(cat <<'EOF'
📝 docs(tasks): HOTFIXES entry + p9-fb-27 status → completed

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 17: Final workspace verification

**Files:** None modified — verification only.

- [ ] **Step 1: Workspace clippy**

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: PASS, zero warnings.

- [ ] **Step 2: Workspace test (single-thread linker)**

Run: `cargo test --workspace --no-fail-fast -j 1`
Expected: PASS. (Per CLAUDE.md, the `-j 1` is required to keep the linker from being SIGKILL'd.)

This will take 10-20 minutes on a fresh build. Don't skip it.

- [ ] **Step 3: Manual smoke against /tmp**

```bash
# Fresh smoke workspace.
rm -rf /tmp/kebab-fb27-final
mkdir -p /tmp/kebab-fb27-final/notes /tmp/kebab-fb27-final/data
cat > /tmp/kebab-fb27-final/config.toml <<'EOF'
[workspace]
root = "/tmp/kebab-fb27-final/notes"

[storage]
data_dir = "/tmp/kebab-fb27-final/data"

[models.embedding]
id = "fastembed-mle5small-384"
EOF
echo "# A\n\nbody A" > /tmp/kebab-fb27-final/notes/a.md
echo "# B\n\nbody B" > /tmp/kebab-fb27-final/notes/b.md

target/debug/kebab --config /tmp/kebab-fb27-final/config.toml init --force
target/debug/kebab --config /tmp/kebab-fb27-final/config.toml ingest

echo "== text mode =="
target/debug/kebab --config /tmp/kebab-fb27-final/config.toml schema

echo "== json mode =="
target/debug/kebab --config /tmp/kebab-fb27-final/config.toml --json schema | jq .

echo "== error wire (config missing, --json) =="
target/debug/kebab --json --config /nonexistent ingest 2>&1 1>/dev/null | jq .

echo "== legacy error (config missing, no --json) =="
target/debug/kebab --config /nonexistent ingest 2>&1 1>/dev/null
```

Expected:
- text mode shows the 4-section layout
- json mode shows `schema_version: "schema.v1"` + `stats.doc_count: 2`
- error wire shows `schema_version: "error.v1"` + `code: "config_invalid"`
- legacy error shows `error: config invalid at /nonexistent: ...`

- [ ] **Step 4: If all 3 above pass, this task is the final commit point**

There is nothing to commit at this step — the verification confirms prior commits.

If something failed: fix it as a new commit on the same branch (do not amend) and re-run the verification.

---

## Self-review checklist (run after Task 17)

After all tasks land, sweep the spec one more time:

- [ ] Spec section 1 (`kebab schema [--json]`) — Tasks 4, 5, 9, 11, 13. ✅
- [ ] Spec section 2 (`error.v1` wire) — Tasks 7, 8, 10, 12, 14. ✅
- [ ] Spec section 3 (Error code catalog 7 codes) — Task 8. ✅
- [ ] Spec section 4 (`kebab-app::error_signal` + `error_classify`) — Tasks 1–3, 7, 8. ✅
- [ ] Spec section 5 (Testing 7 layers) — Tasks 2, 3, 5, 6, 8, 11, 12. ✅
- [ ] Spec section 6 (Migration / sync) — Tasks 13, 14, 15, 16. ✅
- [ ] Final workspace verification — Task 17. ✅

If any spec requirement is not covered by a task, add the missing task before declaring the plan ready.
