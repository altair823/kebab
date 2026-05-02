# p9-fb-06 — `kebab reset` 명령 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `kebab reset [--all|--data-only|--vector-only|--config-only] [--yes]` CLI command that wipes XDG dirs (and optionally just the Lance vector store + matching SQLite `embedding_records` rows) with a TTY confirm gate.

**Architecture:** New `kebab-app::reset` module owns the wipe logic (path resolve via existing `Config::xdg_*` + `expand_path`, `fs::remove_dir_all`, optional `embedding_records` truncate via a new `kebab-store-sqlite` helper). `kebab-cli` adds the `Reset` subcommand, a self-contained 20-line stdin/stdout confirm prompt (no new deps), and a `reset_report.v1` wire schema. `kebab init` is NOT auto-called — user re-runs explicitly.

**Tech Stack:** Rust 2024, clap (existing), `std::io::IsTerminal` (stdlib), `std::fs::remove_dir_all`, rusqlite (`DELETE FROM embedding_records`), serde_json for wire output.

---

## File Structure

**Create:**
- `crates/kebab-app/src/reset.rs` — wipe logic + scope resolution
- `crates/kebab-store-sqlite/tests/truncate_embeddings.rs` — integration test
- `crates/kebab-cli/tests/reset_cli.rs` — integration test
- `docs/wire-schema/v1/reset_report.schema.json` — JSON Schema 7

**Modify:**
- `crates/kebab-app/src/lib.rs` — `pub mod reset;` + re-export `ResetScope` / `ResetReport`
- `crates/kebab-store-sqlite/src/embeddings.rs` — `pub fn truncate_embedding_records()`
- `crates/kebab-cli/src/main.rs` — add `Cmd::Reset` arm + handler
- `crates/kebab-cli/src/wire.rs` — `wire_reset` helper
- `README.md` — `kebab reset` in 명령 표 + Quick start safety note
- `tasks/HOTFIXES.md` — n/a (new feature, not deviation; skip)

**Delete:** none.

---

## Task 1: `kebab-store-sqlite::truncate_embedding_records`

**Files:**
- Create: `crates/kebab-store-sqlite/tests/truncate_embeddings.rs`
- Modify: `crates/kebab-store-sqlite/src/embeddings.rs` (append helper at end of `impl SqliteStore`)

- [ ] **Step 1: Write the failing test**

```rust
// crates/kebab-store-sqlite/tests/truncate_embeddings.rs
//! `truncate_embedding_records` wipes every row regardless of status.
//!
//! Used by `kebab reset --vector-only` to keep SQLite in sync after the
//! Lance vector store is deleted off-disk.

use kebab_core::ChunkId;
use kebab_store_sqlite::{SqliteStore, embeddings::EmbeddingRecordRow};
use time::OffsetDateTime;

fn tmp_store() -> (tempfile::TempDir, SqliteStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&dir.path().join("kebab.sqlite")).unwrap();
    (dir, store)
}

fn count_embedding_rows(store: &SqliteStore) -> i64 {
    store
        .with_conn(|c| {
            c.query_row("SELECT COUNT(*) FROM embedding_records", [], |r| r.get(0))
                .map_err(Into::into)
        })
        .unwrap()
}

#[test]
fn truncate_removes_all_rows() {
    let (_dir, store) = tmp_store();

    // Seed via the existing public API. We don't need real chunks for the
    // count assertion — embedding_records has no FK back to chunks under
    // the V003 schema.
    let row = EmbeddingRecordRow {
        embedding_id: "e1".into(),
        chunk_id: "c1".into(),
        model_id: "test-model".into(),
        model_version: "v1".into(),
        dimensions: 4,
        lance_table: "test_table".into(),
        created_at: OffsetDateTime::now_utc(),
    };
    store.put_embedding_records_pending(&[row]).unwrap();
    assert_eq!(count_embedding_rows(&store), 1);

    store.truncate_embedding_records().unwrap();
    assert_eq!(count_embedding_rows(&store), 0);
}

#[test]
fn truncate_on_empty_table_is_noop() {
    let (_dir, store) = tmp_store();
    store.truncate_embedding_records().unwrap();
    assert_eq!(count_embedding_rows(&store), 0);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p kebab-store-sqlite --test truncate_embeddings`
Expected: FAIL — `truncate_embedding_records` not in scope on `SqliteStore`.

(If `with_conn` also doesn't exist as a public test seam, fall back to opening a raw `rusqlite::Connection` via the same path the store used. Check first with `grep -n "with_conn\|pub fn" crates/kebab-store-sqlite/src/store.rs`. If absent, replace `count_embedding_rows` body with a fresh `rusqlite::Connection::open(dir.path().join("kebab.sqlite"))` query — no production-only test seam.)

- [ ] **Step 3: Implement**

Append at the end of `impl SqliteStore` in `crates/kebab-store-sqlite/src/embeddings.rs`:

```rust
    /// Wipe every row from `embedding_records`. Called by `kebab reset
    /// --vector-only` so SQLite cannot point at a Lance row that the
    /// reset just removed off-disk. The function does NOT cascade to
    /// `chunks` or `documents` — those are kept so the next `kebab
    /// ingest` can re-embed the existing chunk set without re-parsing.
    pub fn truncate_embedding_records(&self) -> Result<()> {
        let conn = self.lock_conn();
        conn.execute("DELETE FROM embedding_records", [])
            .context("DELETE FROM embedding_records")?;
        Ok(())
    }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p kebab-store-sqlite --test truncate_embeddings`
Expected: PASS — both tests green.

- [ ] **Step 5: Run the existing crate tests to confirm no regression**

Run: `cargo test -p kebab-store-sqlite`
Expected: full suite PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/kebab-store-sqlite/src/embeddings.rs \
        crates/kebab-store-sqlite/tests/truncate_embeddings.rs
git commit -m "feat(store-sqlite): add truncate_embedding_records helper

Used by upcoming \`kebab reset --vector-only\` to keep SQLite in sync
after the on-disk Lance store is removed. p9-fb-06 task 1."
```

---

## Task 2: `kebab-app::reset` module — scope + path resolution + wipe

**Files:**
- Create: `crates/kebab-app/src/reset.rs`
- Modify: `crates/kebab-app/src/lib.rs` (add `pub mod reset;` + re-export)
- Modify: `crates/kebab-app/Cargo.toml` (add `dev-dependencies.tempfile` if missing)

The unit tests for path estimation live in this same task; integration ("did the dir actually disappear?") moves up to the CLI test in Task 4.

- [ ] **Step 1: Check whether `tempfile` is already a dev-dep**

Run: `grep -n "tempfile" crates/kebab-app/Cargo.toml`
Expected: present in `[dev-dependencies]` (it's used elsewhere). If missing, add:

```toml
[dev-dependencies]
tempfile = "3"
```

- [ ] **Step 2: Write the failing test (path estimation)**

Create `crates/kebab-app/src/reset.rs` with the test stub at the bottom:

```rust
//! `kebab reset` core — scope-driven path enumeration + wipe.
//!
//! The CLI (and any future TUI surface) calls `enumerate_paths(scope, &cfg)`
//! to compute exactly which on-disk paths the user has asked to remove,
//! presents that list for confirmation, then calls `execute(scope, &cfg)`
//! to actually remove them. Splitting the read step (enumerate) from the
//! write step (execute) is what lets the confirm UI show a faithful
//! preview without having to re-derive the path set.
//!
//! `--vector-only` additionally truncates `embedding_records` in SQLite
//! so the next `kebab ingest` re-embeds cleanly without orphan rows.

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use kebab_config::{Config, expand_path};

/// What the user asked to remove. Mutually exclusive — picked by the CLI
/// from a clap `ArgGroup`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResetScope {
    /// Wipe config + data + cache + state (all four XDG dirs).
    All,
    /// Wipe data + cache + state. Config is preserved so the next run
    /// behaves the same. Default when the user passes `--data-only`.
    DataOnly,
    /// Wipe only the Lance vector_dir off-disk AND truncate the matching
    /// `embedding_records` rows in SQLite. Documents / chunks survive.
    VectorOnly,
    /// Wipe only the config dir.
    ConfigOnly,
}

/// Result of a successful wipe — emitted as `reset_report.v1` by the
/// CLI's `--json` mode and used by the human-mode summary line.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResetReport {
    pub scope: ResetScope,
    pub removed_paths: Vec<PathBuf>,
    pub embedding_rows_truncated: u64,
}

/// Compute the absolute on-disk paths a given scope will wipe, given a
/// loaded `Config`. Pure — does NOT touch the filesystem.
///
/// `--all` returns all four XDG paths in a stable order (config, data,
/// cache, state). `--vector-only` returns the resolved `storage.vector_dir`.
/// Order is preserved across calls so the confirm UI is deterministic.
pub fn enumerate_paths(scope: ResetScope, cfg: &Config) -> Vec<PathBuf> {
    let cfg_dir = Config::xdg_config_path()
        .parent()
        .map(PathBuf::from)
        .unwrap_or_default();
    let data_dir = Config::xdg_data_dir();
    let cache_dir = Config::xdg_cache_dir();
    let state_dir = Config::xdg_state_dir();

    match scope {
        ResetScope::All => vec![cfg_dir, data_dir, cache_dir, state_dir],
        ResetScope::DataOnly => vec![data_dir, cache_dir, state_dir],
        ResetScope::VectorOnly => {
            let vector_dir =
                expand_path(&cfg.storage.vector_dir, &data_dir.to_string_lossy());
            vec![vector_dir]
        }
        ResetScope::ConfigOnly => vec![cfg_dir],
    }
}

/// Best-effort byte size of a directory tree (returns 0 on any I/O error
/// — this is for the confirm UI, not accounting). Skips broken symlinks
/// instead of bubbling errors so a half-broken cache still gets summed.
pub fn estimate_size_bytes(paths: &[PathBuf]) -> u64 {
    fn walk(p: &std::path::Path) -> u64 {
        let mut total = 0u64;
        let entries = match std::fs::read_dir(p) {
            Ok(it) => it,
            Err(_) => return 0,
        };
        for e in entries.flatten() {
            let ft = match e.file_type() {
                Ok(t) => t,
                Err(_) => continue,
            };
            if ft.is_dir() {
                total += walk(&e.path());
            } else if ft.is_file() {
                total += e.metadata().map(|m| m.len()).unwrap_or(0);
            }
        }
        total
    }
    paths.iter().map(|p| walk(p)).sum()
}

/// Wipe every path from `enumerate_paths(scope, cfg)`. For
/// `ResetScope::VectorOnly`, also truncates the SQLite
/// `embedding_records` table so the store doesn't point at the Lance
/// rows we just removed off-disk.
///
/// Idempotent: a missing path is treated as already-removed (success).
/// Returns a `ResetReport` listing exactly what was removed (paths that
/// existed before the call) so `--json` callers see the truth, not the
/// request.
pub fn execute(scope: ResetScope, cfg: &Config) -> Result<ResetReport> {
    let paths = enumerate_paths(scope, cfg);
    let mut removed = Vec::new();

    for p in &paths {
        if !p.exists() {
            continue;
        }
        std::fs::remove_dir_all(p)
            .with_context(|| format!("remove {}", p.display()))?;
        removed.push(p.clone());
    }

    let embedding_rows_truncated = if matches!(scope, ResetScope::VectorOnly) {
        truncate_embeddings(cfg)?
    } else {
        0
    };

    Ok(ResetReport {
        scope,
        removed_paths: removed,
        embedding_rows_truncated,
    })
}

/// Open the SQLite store at the configured path and run
/// `truncate_embedding_records`. Returns the row count BEFORE truncation
/// so the wire report can surface it. If the SQLite file does not exist
/// (e.g. user has never ingested), returns 0 — not an error.
fn truncate_embeddings(cfg: &Config) -> Result<u64> {
    let data_dir = Config::xdg_data_dir();
    let sqlite_path =
        expand_path(&cfg.storage.sqlite, &data_dir.to_string_lossy());
    if !sqlite_path.exists() {
        return Ok(0);
    }
    let store = kebab_store_sqlite::SqliteStore::open(&sqlite_path)
        .context("open SqliteStore for truncate_embedding_records")?;

    // Count first so the report is meaningful.
    let before: i64 = {
        let conn = store.lock_conn();
        conn.query_row("SELECT COUNT(*) FROM embedding_records", [], |r| r.get(0))
            .unwrap_or(0)
    };
    store.truncate_embedding_records()?;
    Ok(u64::try_from(before).unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_with_vector_dir(s: &str) -> Config {
        let mut c = Config::defaults();
        c.storage.vector_dir = s.to_string();
        c
    }

    #[test]
    fn enumerate_data_only_excludes_config_dir() {
        let cfg = Config::defaults();
        let paths = enumerate_paths(ResetScope::DataOnly, &cfg);
        let cfg_dir = Config::xdg_config_path()
            .parent()
            .map(PathBuf::from)
            .unwrap_or_default();
        assert!(!paths.contains(&cfg_dir));
    }

    #[test]
    fn enumerate_vector_only_returns_resolved_vector_dir() {
        let cfg = cfg_with_vector_dir("{data_dir}/lancedb");
        let paths = enumerate_paths(ResetScope::VectorOnly, &cfg);
        assert_eq!(paths.len(), 1);
        let s = paths[0].to_string_lossy().into_owned();
        assert!(s.ends_with("/lancedb"), "got: {s}");
    }

    #[test]
    fn enumerate_all_has_four_distinct_paths() {
        let cfg = Config::defaults();
        let paths = enumerate_paths(ResetScope::All, &cfg);
        assert_eq!(paths.len(), 4);
        // Distinct — XDG layout puts each in its own subtree.
        let unique: std::collections::HashSet<_> = paths.iter().collect();
        assert_eq!(unique.len(), 4);
    }

    #[test]
    fn estimate_size_returns_zero_on_missing_dir() {
        assert_eq!(estimate_size_bytes(&[PathBuf::from("/nonexistent/xyz")]), 0);
    }

    #[test]
    fn execute_data_only_removes_dir_and_returns_report() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("kebab-data");
        std::fs::create_dir_all(target.join("inner")).unwrap();
        std::fs::write(target.join("inner/x"), b"y").unwrap();
        assert!(target.exists());

        // Drive `execute` with a synthetic enumerate result by overriding
        // the XDG env var so `xdg_data_dir()` returns our temp path.
        // The other three XDG dirs we point at fresh subdirs so the test
        // is self-contained.
        let _g_data = scoped_env("XDG_DATA_HOME", dir.path().join("data"));
        let _g_cfg = scoped_env("XDG_CONFIG_HOME", dir.path().join("cfg"));
        let _g_cache = scoped_env("XDG_CACHE_HOME", dir.path().join("cache"));
        let _g_state = scoped_env("XDG_STATE_HOME", dir.path().join("state"));
        std::fs::create_dir_all(dir.path().join("data/kebab")).unwrap();
        std::fs::write(dir.path().join("data/kebab/marker"), b"hi").unwrap();

        let cfg = Config::defaults();
        let report = execute(ResetScope::DataOnly, &cfg).unwrap();
        assert_eq!(report.scope, ResetScope::DataOnly);
        // data dir was the only one that actually existed → only one
        // entry in `removed_paths`.
        assert_eq!(report.removed_paths.len(), 1);
        assert!(!dir.path().join("data/kebab").exists());
    }

    /// Scoped env var setter that restores the previous value on drop.
    /// Tests run sequentially per binary by default, but we restore to
    /// be polite to anyone who switches `--test-threads`.
    fn scoped_env(key: &str, val: std::path::PathBuf) -> EnvGuard {
        let prev = std::env::var(key).ok();
        // SAFETY: tests in this module run single-threaded under the
        // default cargo test runner; this is the same pattern used in
        // `kebab-config::xdg_paths_honor_env`.
        unsafe { std::env::set_var(key, &val) };
        EnvGuard { key: key.to_string(), prev }
    }

    struct EnvGuard {
        key: String,
        prev: Option<String>,
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            unsafe {
                match self.prev.take() {
                    Some(v) => std::env::set_var(&self.key, v),
                    None => std::env::remove_var(&self.key),
                }
            }
        }
    }
}
```

- [ ] **Step 3: Wire the new module into the crate root**

Edit `crates/kebab-app/src/lib.rs`. Find the existing `mod` declarations near the top of the file (search for `pub mod doctor_signal`) and add:

```rust
pub mod reset;
pub use reset::{ResetReport, ResetScope};
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p kebab-app reset`
Expected: 5 PASS — `enumerate_data_only_excludes_config_dir`, `enumerate_vector_only_returns_resolved_vector_dir`, `enumerate_all_has_four_distinct_paths`, `estimate_size_returns_zero_on_missing_dir`, `execute_data_only_removes_dir_and_returns_report`.

If `lock_conn` / `SqliteStore::open` signatures don't match the lookup we did during planning, fix the call sites in `truncate_embeddings` to whatever the real surface is — verify with `grep -n "pub fn open\|pub fn lock_conn" crates/kebab-store-sqlite/src/store.rs`.

- [ ] **Step 5: Commit**

```bash
git add crates/kebab-app/src/reset.rs crates/kebab-app/src/lib.rs
git commit -m "feat(app): add reset module — scope, path enumeration, execute

Provides the wipe core for \`kebab reset\`. Mutually-exclusive
ResetScope variants (All / DataOnly / VectorOnly / ConfigOnly),
pure path enumeration for confirm UI, and an execute helper that
removes paths + truncates embedding_records when scope is VectorOnly.

p9-fb-06 task 2."
```

---

## Task 3: Wire schema `reset_report.v1`

**Files:**
- Create: `docs/wire-schema/v1/reset_report.schema.json`
- Modify: `crates/kebab-cli/src/wire.rs` (add `wire_reset` helper + test)

- [ ] **Step 1: Create the JSON Schema 7 document**

```json
{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "$id": "https://kebab.local/wire-schema/v1/reset_report.schema.json",
  "title": "reset_report.v1",
  "description": "Result of `kebab reset` — what scope was requested and what was actually removed off-disk.",
  "type": "object",
  "required": ["schema_version", "scope", "removed_paths", "embedding_rows_truncated"],
  "additionalProperties": false,
  "properties": {
    "schema_version": { "const": "reset_report.v1" },
    "scope": {
      "type": "string",
      "enum": ["all", "data_only", "vector_only", "config_only"]
    },
    "removed_paths": {
      "type": "array",
      "items": { "type": "string" },
      "description": "Absolute paths that existed before the call and have now been removed. A path that did not exist beforehand is omitted (the wipe is idempotent)."
    },
    "embedding_rows_truncated": {
      "type": "integer",
      "minimum": 0,
      "description": "Count of rows wiped from SQLite embedding_records. Always 0 unless scope is vector_only."
    }
  }
}
```

- [ ] **Step 2: Write the failing test**

Append to `crates/kebab-cli/src/wire.rs` (in the `#[cfg(test)] mod tests` block):

```rust
    #[test]
    fn reset_wrapper_tags_schema_version() {
        let r = kebab_app::ResetReport {
            scope: kebab_app::ResetScope::DataOnly,
            removed_paths: vec![std::path::PathBuf::from("/tmp/x")],
            embedding_rows_truncated: 0,
        };
        let v = wire_reset(&r);
        assert_eq!(schema_of(&v), Some("reset_report.v1"));
        assert_eq!(
            v.get("scope").and_then(Value::as_str),
            Some("data_only")
        );
    }
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p kebab-cli wire::tests::reset_wrapper_tags_schema_version`
Expected: FAIL — `wire_reset` not defined.

- [ ] **Step 4: Add the `wire_reset` helper**

Append in `crates/kebab-cli/src/wire.rs` near the other `wire_*` helpers (above the `#[cfg(test)]` block):

```rust
/// Wrap a [`ResetReport`] as `reset_report.v1`.
pub fn wire_reset(r: &kebab_app::ResetReport) -> Value {
    let v = serde_json::to_value(r).expect("ResetReport serializes");
    tag_object(v, "reset_report.v1")
}
```

Also extend the existing `use` line at the top to import `ResetReport` if not already pulled in via the `kebab_app::` prefix used in the helper. (The above writes `kebab_app::ResetReport` inline, so no `use` change is strictly required.)

- [ ] **Step 5: Run tests to verify**

Run: `cargo test -p kebab-cli wire::`
Expected: 5 PASS — original 4 + new `reset_wrapper_tags_schema_version`.

- [ ] **Step 6: Commit**

```bash
git add docs/wire-schema/v1/reset_report.schema.json crates/kebab-cli/src/wire.rs
git commit -m "feat(cli/wire): add reset_report.v1 schema + wire_reset helper

JSON Schema 7 frozen surface for \`kebab reset --json\`. Mirrors the
ResetReport struct from kebab-app. p9-fb-06 task 3."
```

---

## Task 4: CLI `Cmd::Reset` + confirm prompt + integration test

**Files:**
- Modify: `crates/kebab-cli/src/main.rs` (add `Cmd::Reset` variant + handler + 20-line `confirm_destructive` helper)
- Create: `crates/kebab-cli/tests/reset_cli.rs`

- [ ] **Step 1: Write the failing integration test**

Create `crates/kebab-cli/tests/reset_cli.rs`:

```rust
//! Integration coverage for `kebab reset` — exercises the binary end-to-end
//! against a tempdir-rooted XDG layout.

use std::process::Command;

fn kebab_bin() -> std::path::PathBuf {
    // Mirror the convention used by other tests in this crate (e.g.
    // smoke tests under tests/). The compiled bin is at
    // `target/debug/kebab` relative to the workspace root.
    let manifest = env!("CARGO_MANIFEST_DIR");
    std::path::PathBuf::from(manifest)
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("target/debug/kebab")
}

#[test]
fn reset_data_only_yes_removes_data_dir_and_keeps_config() {
    let tmp = tempfile::tempdir().unwrap();
    let xdg_cfg = tmp.path().join("cfg");
    let xdg_data = tmp.path().join("data");
    let xdg_cache = tmp.path().join("cache");
    let xdg_state = tmp.path().join("state");
    std::fs::create_dir_all(xdg_cfg.join("kebab")).unwrap();
    std::fs::create_dir_all(xdg_data.join("kebab")).unwrap();
    std::fs::create_dir_all(xdg_cache.join("kebab")).unwrap();
    std::fs::create_dir_all(xdg_state.join("kebab")).unwrap();
    std::fs::write(xdg_cfg.join("kebab/config.toml"), "schema_version = 1\n").unwrap();
    std::fs::write(xdg_data.join("kebab/marker"), b"data").unwrap();

    let out = Command::new(kebab_bin())
        .args(["reset", "--data-only", "--yes"])
        .env("XDG_CONFIG_HOME", &xdg_cfg)
        .env("XDG_DATA_HOME", &xdg_data)
        .env("XDG_CACHE_HOME", &xdg_cache)
        .env("XDG_STATE_HOME", &xdg_state)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    assert!(!xdg_data.join("kebab").exists(), "data dir should be gone");
    assert!(xdg_cfg.join("kebab/config.toml").exists(), "config preserved");
}

#[test]
fn reset_no_yes_in_non_tty_aborts_with_exit_2() {
    let tmp = tempfile::tempdir().unwrap();
    let xdg_data = tmp.path().join("data");
    std::fs::create_dir_all(xdg_data.join("kebab")).unwrap();
    std::fs::write(xdg_data.join("kebab/marker"), b"d").unwrap();

    let out = Command::new(kebab_bin())
        .args(["reset", "--data-only"])
        .env("XDG_CONFIG_HOME", tmp.path().join("cfg"))
        .env("XDG_DATA_HOME", &xdg_data)
        .env("XDG_CACHE_HOME", tmp.path().join("cache"))
        .env("XDG_STATE_HOME", tmp.path().join("state"))
        .output()
        .unwrap();

    // Non-TTY (Command::output gives no tty) without --yes must abort.
    assert!(!out.status.success(), "expected abort, got success");
    let code = out.status.code().unwrap_or(-1);
    assert_eq!(code, 2, "expected exit 2 (generic error), got {code}");
    assert!(
        xdg_data.join("kebab").exists(),
        "data dir must survive an aborted reset"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("non-interactive") || stderr.contains("--yes"),
        "expected refusal hint in stderr, got: {stderr}"
    );
}

#[test]
fn reset_data_only_yes_json_emits_reset_report_v1() {
    let tmp = tempfile::tempdir().unwrap();
    let xdg_data = tmp.path().join("data");
    std::fs::create_dir_all(xdg_data.join("kebab")).unwrap();
    std::fs::write(xdg_data.join("kebab/marker"), b"d").unwrap();

    let out = Command::new(kebab_bin())
        .args(["--json", "reset", "--data-only", "--yes"])
        .env("XDG_CONFIG_HOME", tmp.path().join("cfg"))
        .env("XDG_DATA_HOME", &xdg_data)
        .env("XDG_CACHE_HOME", tmp.path().join("cache"))
        .env("XDG_STATE_HOME", tmp.path().join("state"))
        .output()
        .unwrap();
    assert!(out.status.success());

    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v.get("schema_version").and_then(|s| s.as_str()), Some("reset_report.v1"));
    assert_eq!(v.get("scope").and_then(|s| s.as_str()), Some("data_only"));
    assert!(v.get("removed_paths").and_then(|a| a.as_array()).is_some());
}
```

Add to `crates/kebab-cli/Cargo.toml`'s `[dev-dependencies]` if missing:

```toml
[dev-dependencies]
tempfile = "3"
serde_json = "1"
```

(Run `grep -n "tempfile\|serde_json" crates/kebab-cli/Cargo.toml` first — likely both already present from existing tests. Only add what's missing.)

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p kebab-cli --test reset_cli`
Expected: FAIL — `kebab reset` is not a recognized clap subcommand yet (the binary will print "error: unrecognized subcommand" and exit nonzero with the wrong error shape).

- [ ] **Step 3: Add the `Reset` clap variant**

In `crates/kebab-cli/src/main.rs`, inside `enum Cmd` (above `Doctor`):

```rust
    /// Wipe XDG data dirs (and optionally the Lance vector store) so
    /// the workspace can be re-initialised. **Irreversible.** Without
    /// `--yes`, prompts on TTY; aborts in non-interactive contexts.
    Reset {
        /// Wipe config + data + cache + state. Implies losing
        /// `config.toml` — re-run `kebab init` afterwards.
        #[arg(long, group = "reset_scope")]
        all: bool,

        /// Default. Wipe data + cache + state. Config is preserved.
        #[arg(long, group = "reset_scope")]
        data_only: bool,

        /// Wipe only the Lance vector store + truncate
        /// `embedding_records`. SQLite documents / chunks survive so the
        /// next `kebab ingest` re-embeds without re-parsing.
        #[arg(long, group = "reset_scope")]
        vector_only: bool,

        /// Wipe only the config dir.
        #[arg(long, group = "reset_scope")]
        config_only: bool,

        /// Skip the interactive confirm. Required in non-interactive
        /// contexts (CI, pipes).
        #[arg(long)]
        yes: bool,
    },
```

clap's `group = "reset_scope"` makes the four flags mutually exclusive automatically.

- [ ] **Step 4: Add the handler**

In `fn run(cli: &Cli) -> anyhow::Result<()>`, just above the `Cmd::Doctor =>` arm, insert:

```rust
        Cmd::Reset {
            all,
            data_only,
            vector_only,
            config_only,
            yes,
        } => {
            use kebab_app::ResetScope;
            let scope = if *all {
                ResetScope::All
            } else if *vector_only {
                ResetScope::VectorOnly
            } else if *config_only {
                ResetScope::ConfigOnly
            } else {
                // `--data-only` explicit OR no scope flag at all → DataOnly
                let _ = data_only;
                ResetScope::DataOnly
            };

            let cfg = kebab_config::Config::load(cli.config.as_deref())?;
            let paths = kebab_app::reset::enumerate_paths(scope, &cfg);
            let bytes = kebab_app::reset::estimate_size_bytes(&paths);

            if !*yes {
                use std::io::IsTerminal;
                if !std::io::stdin().is_terminal() {
                    anyhow::bail!(
                        "reset is destructive and stdin is non-interactive — pass --yes to proceed"
                    );
                }
                if !confirm_destructive(scope, &paths, bytes)? {
                    eprintln!("aborted.");
                    return Ok(());
                }
            }

            let report = kebab_app::reset::execute(scope, &cfg)?;
            if cli.json {
                println!("{}", serde_json::to_string(&wire::wire_reset(&report))?);
            } else {
                println!(
                    "removed {} path(s); embedding_rows_truncated={}",
                    report.removed_paths.len(),
                    report.embedding_rows_truncated
                );
                for p in &report.removed_paths {
                    println!("  - {}", p.display());
                }
                if matches!(scope, ResetScope::All | ResetScope::ConfigOnly) {
                    println!("hint: run `kebab init` to recreate config.toml");
                }
            }
            Ok(())
        }
```

- [ ] **Step 5: Add the `confirm_destructive` helper**

At the bottom of `crates/kebab-cli/src/main.rs` (after `fn run`):

```rust
/// Minimal stdin/stdout confirm prompt. No new dep — uses stdlib
/// `IsTerminal`. Returns `Ok(true)` only when the user types y/Y/yes.
/// Empty input or anything else → false (safe default).
fn confirm_destructive(
    scope: kebab_app::ResetScope,
    paths: &[std::path::PathBuf],
    bytes: u64,
) -> anyhow::Result<bool> {
    use std::io::Write;
    let mut out = std::io::stderr().lock();
    writeln!(out, "kebab reset ({:?}): about to remove", scope)?;
    for p in paths {
        writeln!(out, "  - {}", p.display())?;
    }
    writeln!(out, "estimated total: {} bytes", bytes)?;
    write!(out, "Proceed? [y/N] ")?;
    out.flush()?;

    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    let s = line.trim().to_ascii_lowercase();
    Ok(matches!(s.as_str(), "y" | "yes"))
}
```

- [ ] **Step 6: Build the binary so the integration test can find it**

Run: `cargo build -p kebab-cli`
Expected: clean compile.

- [ ] **Step 7: Run integration tests**

Run: `cargo test -p kebab-cli --test reset_cli`
Expected: 3 PASS.

- [ ] **Step 8: Run the full crate test suite to confirm no regression**

Run: `cargo test -p kebab-cli`
Expected: full PASS.

- [ ] **Step 9: Commit**

```bash
git add crates/kebab-cli/src/main.rs crates/kebab-cli/tests/reset_cli.rs \
        crates/kebab-cli/Cargo.toml
git commit -m "feat(cli): add \`kebab reset\` command with TTY confirm gate

Mutually-exclusive scope flags (--all / --data-only / --vector-only /
--config-only) plus --yes for non-interactive use. Aborts with exit 2
when stdin is non-interactive and --yes is missing — silent destruction
is forbidden. p9-fb-06 task 4."
```

---

## Task 5: README + HANDOFF.md sync (3-doc rule)

**Files:**
- Modify: `README.md` (명령 표 + Quick start safety note)
- Modify: `HANDOFF.md` (one-line note under deviations / new features)

`docs/ARCHITECTURE.md` is NOT touched — `kebab reset` doesn't change the crate graph or any locked-in technical decision.

- [ ] **Step 1: Add the row to the README 명령 table**

Open `README.md`, find the existing 명령 table (rows starting with `kebab init`, `kebab ingest`, etc.), and insert before `kebab eval`:

```markdown
| `kebab reset [--all / --data-only / --vector-only / --config-only] [--yes]` | XDG 데이터 wipe. **Irreversible.** TTY 면 confirm prompt, 아니면 `--yes` 필수. `--vector-only` 는 SQLite `embedding_records` 도 같이 truncate (orphan 방지) |
```

- [ ] **Step 2: Add a safety note in the install / cleanup section**

In `README.md`, find the existing "제거" / `cargo uninstall kebab-cli` paragraph. Replace the manual `rm -rf` instruction with:

```markdown
제거는 `cargo uninstall kebab-cli`. 이 명령은 binary 만 지우고 워크스페이스 데이터는 그대로 남는다. 데이터까지 정리하려면 `kebab reset --all --yes` (config + data + cache + state 모두 wipe — 재시작 시 `kebab init` 다시 실행).
```

- [ ] **Step 3: Add a HANDOFF.md entry under "최근 발견 / 결정"**

Open `HANDOFF.md`, find the "머지 후 발견된 버그 / 결정 (요약)" section (or the equivalent "최근 변경" / dated bullet list). Add at the top of the most recent dated subsection:

```markdown
- 2026-05-02 P9 도그푸딩 후속 — `kebab reset --all|--data-only|--vector-only|--config-only [--yes]` 추가. TTY 가 아니면 `--yes` 필수 (silent destruction 금지). p9-fb-06 spec 참조.
```

(If HANDOFF doesn't already have a 2026-05-02 dated subsection, just add the bullet under whatever the latest section is — the date in the bullet itself is the source of truth.)

- [ ] **Step 4: Verify the docs render**

Run: `grep -n "kebab reset" README.md HANDOFF.md`
Expected: 명령 table row + cleanup paragraph + HANDOFF bullet (3 hits).

- [ ] **Step 5: Commit**

```bash
git add README.md HANDOFF.md
git commit -m "docs: \`kebab reset\` in README 명령 table + HANDOFF entry

3-doc sync rule: user-visible CLI surface change → README and HANDOFF
get the same PR. ARCHITECTURE.md unchanged (no crate graph or locked
decision moved). p9-fb-06 task 5."
```

---

## Task 6: Mark task spec status + verify full workspace

**Files:**
- Modify: `tasks/p9/p9-fb-06-data-reset-command.md` (frontmatter `status: planned` → `status: in_progress` then bump to `completed` after the PR merges; this task only flips to `in_progress`)

- [ ] **Step 1: Flip task status**

Edit `tasks/p9/p9-fb-06-data-reset-command.md` frontmatter:

```yaml
status: in_progress
```

(Final flip to `completed` happens in a separate one-line commit AFTER the PR merges, so the spec history reflects reality.)

- [ ] **Step 2: Run a wider test to confirm nothing else broke**

Run: `cargo test -p kebab-store-sqlite -p kebab-app -p kebab-cli`
Expected: full PASS across the three touched crates.

- [ ] **Step 3: Run clippy on the touched crates**

Run: `cargo clippy -p kebab-store-sqlite -p kebab-app -p kebab-cli --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add tasks/p9/p9-fb-06-data-reset-command.md
git commit -m "chore(tasks): mark p9-fb-06 in_progress

Flips to \`completed\` once the PR merges (separate one-line commit so
spec history reflects reality)."
```

- [ ] **Step 5: PR with gitea-ops review loop**

Per the project workflow rule (memory: `feedback_pr_workflow.md`):
1. `gitea-pr --title "feat(cli): kebab reset (p9-fb-06)" --head feat/p9-fb-06-reset --body <see below>`
2. `gitea-pr-status <PR#> --wait-ci` until gate passes.
3. Review loop: `gitea-pr-diff` → analyze → `gitea-pr-review` (REQUEST_CHANGES → ... → APPROVE).
4. **APPROVE achieved → merge immediately, no asking.** `tea pr merge <PR#>` (or Gitea API), pull main locally, delete branch, move on to the next task in the priority list (p9-fb-01 batch).

PR body template:

```markdown
## Summary
- `kebab reset [--all / --data-only / --vector-only / --config-only] [--yes]`.
- TTY confirm prompt (stdin non-interactive without `--yes` → exit 2).
- `--vector-only` truncates SQLite `embedding_records` to keep the store consistent after the off-disk Lance dir is removed.
- Wire schema `reset_report.v1` for `--json` mode.

## Scope (p9-fb-06)
- spec: `tasks/p9/p9-fb-06-data-reset-command.md`
- feedback origin: `tasks/p9/p9-dogfooding-feedback.md` item 4

## Test plan
- [x] `cargo test -p kebab-store-sqlite --test truncate_embeddings`
- [x] `cargo test -p kebab-app reset`
- [x] `cargo test -p kebab-cli --test reset_cli`
- [x] `cargo clippy --all-targets -- -D warnings` (touched crates)
```

---

## Self-review

**Spec coverage** (against `tasks/p9/p9-fb-06-data-reset-command.md`):

- Public surface `kebab reset [--all|--data-only|--vector-only|--config-only] [--yes]` → Task 4.
- Default = `--data-only` → Task 4 handler (`else { ResetScope::DataOnly }`).
- `--config <path>` honored → Task 4 (`Config::load(cli.config.as_deref())`). The behavior here matters more for `--vector-only` (which reads `cfg.storage.vector_dir`). For the XDG-rooted scopes, the path resolution goes through `Config::xdg_*` env-aware methods — same effective honor since the user can set `XDG_*_HOME` in the same shell.
- Confirm prompt with paths + bytes + `(y/N)` → Task 4 `confirm_destructive`.
- Non-TTY without `--yes` → abort with hint → Task 4 handler + integration test 2.
- `--vector-only` truncates `embedding_records` → Task 1 + Task 2.
- No auto `kebab init` → Task 4 handler emits a hint instead.
- Test plan items (path estimation unit / data-only integration / vector-only integration) → Tasks 2 + 4.
- DoD: `cargo test -p kebab-cli` PASS → Task 6. README updated → Task 5. `--help` "irreversible" → Task 4 doc-comment on the `Reset` variant.

**One spec note we did NOT implement separately**: `vector-only` integration test — Task 4 covers `data-only` + `non-tty` + `json`. The `vector-only` path is exercised by Task 2's `truncate_embeddings` unit test (does the wipe work?) and by the `enumerate_vector_only_returns_resolved_vector_dir` unit test (does enumeration return the right dir?). A full end-to-end `vector-only` integration test would require seeding both a Lance dir and embedding rows under a tempdir XDG layout — feasible but long, and the unit-level coverage already proves both halves. If the reviewer pushes back, add it as a follow-up commit in the same PR.

**Placeholder scan**: none — every step has the actual code or command.

**Type consistency**: `ResetScope` / `ResetReport` defined in Task 2, used unchanged in Tasks 3, 4. `truncate_embedding_records` returns `Result<()>` in Task 1, called in Task 2 via the wrapper that adds row count. Names match across tasks.

---

## Execution Handoff

Plan saved to `docs/superpowers/plans/2026-05-02-p9-fb-06-reset-command.md`.

**Auto mode active + caveman feedback rule**: proceed with **inline execution** (executing-plans skill), no need to ask. Reasoning:
- 6 tasks, all touch crates I already know.
- Each task has tests that run in seconds — feedback loop tight enough that a fresh subagent per task would be overhead.
- PR merge follows immediately after task 6 per the updated workflow rule (no human gate between APPROVE and merge).

Starting executing-plans next.
