# p9-fb-31 Implementation Plan — Single-file / stdin ingest

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `kebab ingest-file <path>` + `kebab ingest-stdin --title <T>` CLI subcommands and MCP tools `ingest_file` + `ingest_stdin` (4 → 6 tools) so agents and humans can ingest a single external file or stdin content into the KB without re-walking the workspace.

**Architecture:** Two new `kebab-app` facade fns (`ingest_file_with_config`, `ingest_stdin_with_config`). Both copy the bytes to `<workspace.root>/_external/<blake3-12>.<ext>` (auto-create dir + auto-append `_external/` line to `.kebabignore` so future workspace walks don't re-walk). Then run a single-asset variant of the existing `ingest_with_config_opts` pipeline (incremental ingest from fb-23 handles idempotency — same hash → unchanged). `ingest_stdin_with_config` is a thin wrapper that prepends a frontmatter block (title + optional source_uri) and delegates to `ingest_file_with_config`. CLI gets two new `Cmd` arms; kebab-mcp gets two new tools wired into the `spawn_tool` helper from fb-30 (mutation tools — first MCP write surface).

**Tech Stack:** Rust 2024, blake3 (workspace, already used for asset hashing), serde + serde_json (workspace), `kebab-app` facade pattern, rmcp 1.6 (kebab-mcp).

**Spec source:** `docs/superpowers/specs/2026-05-07-p9-fb-31-single-file-stdin-ingest-design.md` (commit `7772fbc` on `spec/p9-fb-31-single-file-stdin-ingest`).

---

## File map

**Create:**
- `crates/kebab-app/src/external.rs` — helpers: `ensure_external_dir`, `ensure_kebabignore_entry`, `copy_to_external`, `inject_frontmatter`
- `crates/kebab-app/tests/ingest_file.rs` — integration tests for `ingest_file_with_config`
- `crates/kebab-app/tests/ingest_stdin.rs` — integration tests for `ingest_stdin_with_config`
- `crates/kebab-cli/tests/cli_ingest_file.rs` — spawn-based integration test
- `crates/kebab-cli/tests/cli_ingest_stdin.rs` — spawn + stdin pipe test
- `crates/kebab-mcp/src/tools/ingest_file.rs` — tool input + handle
- `crates/kebab-mcp/src/tools/ingest_stdin.rs` — tool input + handle
- `crates/kebab-mcp/tests/tools_call_ingest_file.rs`
- `crates/kebab-mcp/tests/tools_call_ingest_stdin.rs`

**Modify:**
- `crates/kebab-app/src/lib.rs` — register `pub mod external;`, add two new pub fns (`ingest_file_with_config`, `ingest_stdin_with_config`)
- `crates/kebab-cli/src/main.rs` — add `Cmd::IngestFile { path: PathBuf }` + `Cmd::IngestStdin { title: String, source_uri: Option<String> }` variants + arms
- `crates/kebab-mcp/src/lib.rs` — extend `build_tools_vec` (4 → 6) + add `"ingest_file"` / `"ingest_stdin"` arms in `call_tool` (use `spawn_tool` helper)
- `crates/kebab-mcp/src/tools/mod.rs` — register two new tool modules
- `crates/kebab-mcp/tests/tools_list.rs` — assertion update (4 → 6 tools)
- `README.md` — two new commands + MCP tool list update
- `HANDOFF.md` — post-도그푸딩 entry
- `CLAUDE.md` — `_external/` dir mention (no wire schema change)
- `integrations/claude-code/kebab/SKILL.md` — MCP `ingest_file` / `ingest_stdin` usage + agent fetch flow recipe
- `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md` — §3 / §6 `_external/` policy
- `tasks/HOTFIXES.md` — new entry
- `tasks/p9/p9-fb-31-single-file-stdin-ingest.md` — status `open` → `completed`

---

## Task 1 — `external` module: directory + .kebabignore + copy + frontmatter inject

**Files:**
- Create: `crates/kebab-app/src/external.rs`
- Modify: `crates/kebab-app/src/lib.rs` (register `pub mod external;`)

- [ ] **Step 1: Create the module skeleton**

Write `crates/kebab-app/src/external.rs`:

```rust
//! Helpers for the `_external/` workspace subdirectory used by
//! `ingest_file_with_config` and `ingest_stdin_with_config` (p9-fb-31).
//!
//! - `ensure_external_dir`: create `<workspace.root>/_external/` if absent.
//! - `ensure_kebabignore_entry`: append `_external/` to `<workspace.root>/.kebabignore`
//!   if missing — prevents subsequent `kebab ingest` workspace walks from
//!   re-walking files that were imported via single-file ingest.
//! - `copy_to_external`: write bytes to `_external/<blake3-12>.<ext>`, idempotent.
//! - `inject_frontmatter`: prepend a YAML frontmatter block to a markdown body
//!   string (used by `ingest_stdin_with_config`).

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

pub const EXTERNAL_DIR: &str = "_external";
const KEBABIGNORE_LINE: &str = "_external/";

/// Ensure `<workspace_root>/_external/` exists. Returns the directory path.
pub fn ensure_external_dir(workspace_root: &Path) -> Result<PathBuf> {
    let dir = workspace_root.join(EXTERNAL_DIR);
    fs::create_dir_all(&dir)
        .with_context(|| format!("create _external dir at {}", dir.display()))?;
    Ok(dir)
}

/// Append `_external/` line to `<workspace_root>/.kebabignore` if not already
/// present. Idempotent — checks for the exact line before appending.
pub fn ensure_kebabignore_entry(workspace_root: &Path) -> Result<()> {
    let path = workspace_root.join(".kebabignore");
    let existing = if path.exists() {
        fs::read_to_string(&path)
            .with_context(|| format!("read existing .kebabignore at {}", path.display()))?
    } else {
        String::new()
    };
    let already = existing
        .lines()
        .any(|line| line.trim() == KEBABIGNORE_LINE);
    if already {
        return Ok(());
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("open .kebabignore for append at {}", path.display()))?;
    if !existing.is_empty() && !existing.ends_with('\n') {
        file.write_all(b"\n")?;
    }
    writeln!(file, "{}", KEBABIGNORE_LINE)?;
    Ok(())
}

/// Copy bytes to `<external_dir>/<blake3-12>.<ext>`. Idempotent — if the
/// destination file already exists with the expected hash, the existing
/// file is reused (no second write). Returns the destination path.
pub fn copy_to_external(
    external_dir: &Path,
    bytes: &[u8],
    ext: &str,
) -> Result<PathBuf> {
    let hash = blake3::hash(bytes);
    let prefix = &hash.to_hex().as_str()[..12];
    let filename = format!("{prefix}.{ext}");
    let dest = external_dir.join(&filename);
    if !dest.exists() {
        fs::write(&dest, bytes)
            .with_context(|| format!("write external file at {}", dest.display()))?;
    }
    Ok(dest)
}

/// Prepend a YAML frontmatter block to a markdown body. Returns the wrapped
/// markdown string. Errors if `body` already starts with `---` (the user
/// should use `ingest_file_with_config` for files that already carry
/// frontmatter).
pub fn inject_frontmatter(
    body: &str,
    title: &str,
    source_uri: Option<&str>,
) -> Result<String> {
    if body.trim_start().starts_with("---\n") || body.trim_start().starts_with("---\r\n") {
        anyhow::bail!(
            "stdin already has frontmatter; use `kebab ingest-file` for files with metadata"
        );
    }
    let title_yaml = yaml_quote(title);
    let mut header = String::new();
    header.push_str("---\n");
    header.push_str(&format!("title: {title_yaml}\n"));
    if let Some(uri) = source_uri {
        let uri_yaml = yaml_quote(uri);
        header.push_str(&format!("source_uri: {uri_yaml}\n"));
    }
    header.push_str("---\n\n");
    header.push_str(body);
    Ok(header)
}

/// YAML-quote a string. Always uses double-quoted form with backslash-escape
/// for `"` and `\`. Defensive against agent-supplied titles that contain
/// quotes / control chars.
fn yaml_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
```

- [ ] **Step 2: Register module in lib.rs**

Open `crates/kebab-app/src/lib.rs`. Find the line `pub mod schema;` (added by fb-27). Add right after:

```rust
pub mod external;
```

- [ ] **Step 3: Write unit tests**

Add to the bottom of `crates/kebab-app/src/external.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn ensure_external_dir_creates_dir() {
        let dir = tempdir().unwrap();
        let result = ensure_external_dir(dir.path()).unwrap();
        assert_eq!(result, dir.path().join("_external"));
        assert!(result.is_dir());
    }

    #[test]
    fn ensure_external_dir_is_idempotent() {
        let dir = tempdir().unwrap();
        let _ = ensure_external_dir(dir.path()).unwrap();
        let result = ensure_external_dir(dir.path()).unwrap();
        assert!(result.is_dir());
    }

    #[test]
    fn ensure_kebabignore_entry_creates_file_with_line() {
        let dir = tempdir().unwrap();
        ensure_kebabignore_entry(dir.path()).unwrap();
        let content = fs::read_to_string(dir.path().join(".kebabignore")).unwrap();
        assert!(content.lines().any(|l| l.trim() == "_external/"));
    }

    #[test]
    fn ensure_kebabignore_entry_appends_to_existing() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join(".kebabignore"), "*.tmp\n").unwrap();
        ensure_kebabignore_entry(dir.path()).unwrap();
        let content = fs::read_to_string(dir.path().join(".kebabignore")).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert!(lines.contains(&"*.tmp"));
        assert!(lines.contains(&"_external/"));
    }

    #[test]
    fn ensure_kebabignore_entry_idempotent() {
        let dir = tempdir().unwrap();
        ensure_kebabignore_entry(dir.path()).unwrap();
        ensure_kebabignore_entry(dir.path()).unwrap();
        let content = fs::read_to_string(dir.path().join(".kebabignore")).unwrap();
        let count = content.lines().filter(|l| l.trim() == "_external/").count();
        assert_eq!(count, 1, "should not duplicate");
    }

    #[test]
    fn ensure_kebabignore_entry_handles_missing_trailing_newline() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join(".kebabignore"), "*.tmp").unwrap(); // no \n
        ensure_kebabignore_entry(dir.path()).unwrap();
        let content = fs::read_to_string(dir.path().join(".kebabignore")).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert!(lines.contains(&"*.tmp"));
        assert!(lines.contains(&"_external/"));
    }

    #[test]
    fn copy_to_external_writes_with_hash_prefix_filename() {
        let dir = tempdir().unwrap();
        let ext_dir = ensure_external_dir(dir.path()).unwrap();
        let path = copy_to_external(&ext_dir, b"hello", "md").unwrap();
        assert!(path.exists());
        assert!(path.file_name().unwrap().to_string_lossy().ends_with(".md"));
        let stem = path.file_stem().unwrap().to_string_lossy();
        assert_eq!(stem.len(), 12);
    }

    #[test]
    fn copy_to_external_is_idempotent_for_same_bytes() {
        let dir = tempdir().unwrap();
        let ext_dir = ensure_external_dir(dir.path()).unwrap();
        let p1 = copy_to_external(&ext_dir, b"hello", "md").unwrap();
        let p2 = copy_to_external(&ext_dir, b"hello", "md").unwrap();
        assert_eq!(p1, p2);
    }

    #[test]
    fn copy_to_external_different_bytes_produce_different_filenames() {
        let dir = tempdir().unwrap();
        let ext_dir = ensure_external_dir(dir.path()).unwrap();
        let p1 = copy_to_external(&ext_dir, b"hello", "md").unwrap();
        let p2 = copy_to_external(&ext_dir, b"world", "md").unwrap();
        assert_ne!(p1, p2);
    }

    #[test]
    fn inject_frontmatter_basic() {
        let out = inject_frontmatter("## Body", "Article X", None).unwrap();
        assert!(out.starts_with("---\ntitle: \"Article X\"\n---\n\n## Body"));
    }

    #[test]
    fn inject_frontmatter_with_source_uri() {
        let out = inject_frontmatter("## Body", "X", Some("https://example.com/x")).unwrap();
        assert!(out.contains("title: \"X\""));
        assert!(out.contains("source_uri: \"https://example.com/x\""));
        assert!(out.contains("\n## Body"));
    }

    #[test]
    fn inject_frontmatter_errors_on_existing_frontmatter() {
        let body = "---\ntitle: Existing\n---\n\n## Body";
        let err = inject_frontmatter(body, "New", None).unwrap_err();
        assert!(err.to_string().contains("already has frontmatter"));
    }

    #[test]
    fn inject_frontmatter_errors_on_existing_frontmatter_crlf() {
        let body = "---\r\ntitle: Existing\r\n---\r\n\r\n## Body";
        let err = inject_frontmatter(body, "New", None).unwrap_err();
        assert!(err.to_string().contains("already has frontmatter"));
    }

    #[test]
    fn yaml_quote_escapes_quotes_and_backslashes() {
        assert_eq!(yaml_quote("hello \"world\""), "\"hello \\\"world\\\"\"");
        assert_eq!(yaml_quote("path\\to"), "\"path\\\\to\"");
        assert_eq!(yaml_quote("line\nbreak"), "\"line\\nbreak\"");
    }
}
```

- [ ] **Step 4: Verify tests pass**

```bash
cd /Users/user/Workspace/projects/kebab
cargo test -p kebab-app --lib external 2>&1 | tail -10
```

Expected: 12 tests pass.

If `blake3` isn't a direct dependency of `kebab-app`, add it. Check first:

```bash
grep -n "blake3" /Users/user/Workspace/projects/kebab/crates/kebab-app/Cargo.toml
```

If absent, add `blake3 = { workspace = true }` to `[dependencies]` (the workspace already has it for asset hashing).

- [ ] **Step 5: Workspace clippy**

```bash
cargo clippy -p kebab-app --all-targets -- -D warnings 2>&1 | tail -3
```

Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/kebab-app/src/external.rs crates/kebab-app/src/lib.rs crates/kebab-app/Cargo.toml
git commit -m "$(cat <<'EOF'
🏗️ feat(kebab-app): external module — _external dir + frontmatter inject (fb-31)

Pure-fn helpers for the `_external/` workspace subdirectory:
- `ensure_external_dir(workspace_root)` — mkdir if absent
- `ensure_kebabignore_entry(workspace_root)` — append `_external/` line
  to .kebabignore if missing (idempotent)
- `copy_to_external(ext_dir, bytes, ext)` — write to
  `<ext_dir>/<blake3-12>.<ext>`, idempotent on same content
- `inject_frontmatter(body, title, source_uri?)` — prepend YAML block
  with strict double-quote escaping; errors if body already starts
  with `---`
- `yaml_quote(s)` — defensive escaping for agent-supplied strings

12 unit tests cover happy + idempotency + edge (CRLF frontmatter
detection, YAML escape).

ingest_file / ingest_stdin facades (Tasks 4 + 5) compose these.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2 — `ingest_file_with_config` facade — single-file ingest

**Files:**
- Modify: `crates/kebab-app/src/lib.rs` (add `pub fn ingest_file_with_config`)

- [ ] **Step 1: Inspect existing single-asset processing**

Run:

```bash
grep -n "fn ingest_one_md_asset\|fn ingest_one_image_asset\|fn ingest_one_pdf_asset\|fn ingest_with_config_opts" /Users/user/Workspace/projects/kebab/crates/kebab-app/src/lib.rs | head -10
```

These per-medium helper fns already exist (used by the workspace-walk loop). The new facade calls one of them based on file extension.

- [ ] **Step 2: Add the facade fn**

Append to `crates/kebab-app/src/lib.rs` (after the existing `ingest_with_config_opts` definition, before any test mods):

```rust
/// Single-file ingest (p9-fb-31). Copies the file to
/// `<workspace.root>/_external/<blake3-12>.<ext>` and runs the
/// per-medium ingest pipeline on that single asset. Returns an
/// `IngestReport` with `scanned: 1` (and either `new: 1` or
/// `unchanged: 1` depending on whether the content hash + version
/// cascade match an existing doc — incremental ingest from p9-fb-23).
///
/// `path` may point inside or outside the workspace.
///
/// `.kebabignore` patterns matching `path` are bypassed with a stderr
/// `warn:` line — explicit ingest is intent.
pub fn ingest_file_with_config(
    config: kebab_config::Config,
    path: &std::path::Path,
) -> anyhow::Result<IngestReport> {
    use std::io::Write;

    if !path.exists() {
        anyhow::bail!("ingest-file: source path does not exist: {}", path.display());
    }
    if !path.is_file() {
        anyhow::bail!("ingest-file: not a regular file: {}", path.display());
    }

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .ok_or_else(|| anyhow::anyhow!("ingest-file: source has no extension: {}", path.display()))?;

    let bytes = std::fs::read(path)
        .with_context(|| format!("ingest-file: read source {}", path.display()))?;

    // Resolve workspace root (mirrors how SqliteStore::open does it).
    let workspace_root = std::path::PathBuf::from(
        kebab_config::expand_path(&config.workspace.root, ""),
    );

    // .kebabignore check — warn but continue.
    let ignore_match = check_kebabignore_match(&workspace_root, path);
    if ignore_match {
        eprintln!(
            "warn: {} matches .kebabignore patterns; proceeding (explicit ingest bypasses ignore)",
            path.display()
        );
    }

    // Set up _external/ dir + auto-ignore line.
    let external_dir = crate::external::ensure_external_dir(&workspace_root)
        .context("ingest-file: ensure _external/ dir")?;
    crate::external::ensure_kebabignore_entry(&workspace_root)
        .context("ingest-file: append _external/ to .kebabignore")?;

    // Copy bytes to _external/<hash>.<ext>.
    let dest = crate::external::copy_to_external(&external_dir, &bytes, ext)
        .context("ingest-file: copy to _external")?;

    // Build a SourceScope that targets just the dest file's parent dir,
    // with an include filter restricting the walk to the dest filename.
    // (FsSourceConnector is dir-based; the simplest hack is to scope to
    // _external/ with `include: vec![<filename>]` so the walk picks up
    // exactly one asset. Workspace exclude is preserved.)
    let filename = dest
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("ingest-file: dest has no filename"))?
        .to_string_lossy()
        .into_owned();
    let scope = kebab_core::SourceScope {
        root: external_dir.clone(),
        include: vec![filename],
        exclude: config.workspace.exclude.clone(),
    };

    let mut opts = IngestOpts::default();
    opts.force_reingest = false; // honour incremental ingest

    ingest_with_config_opts(config, scope, /* summary_only = */ false, opts)
}

/// Returns true if `source_path` matches any `.kebabignore` pattern
/// rooted at `workspace_root`. Used by `ingest_file_with_config` to
/// emit a stderr warn before bypassing the ignore.
fn check_kebabignore_match(workspace_root: &std::path::Path, source_path: &std::path::Path) -> bool {
    let kebabignore = workspace_root.join(".kebabignore");
    if !kebabignore.exists() {
        return false;
    }
    let text = match std::fs::read_to_string(&kebabignore) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let mut builder = ignore::gitignore::GitignoreBuilder::new(workspace_root);
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let _ = builder.add_line(None, line);
    }
    let matcher = match builder.build() {
        Ok(m) => m,
        Err(_) => return false,
    };
    matcher.matched(source_path, source_path.is_dir()).is_ignore()
}
```

`ignore = "0.4"` is already a dependency of `kebab-source-fs` (`grep -n "ignore" crates/kebab-source-fs/Cargo.toml` to confirm). Add `ignore = { workspace = true }` to `crates/kebab-app/Cargo.toml` `[dependencies]` if not already present.

If `IngestOpts` doesn't have a `Default` impl yet, construct it explicitly with all fields — e.g. `IngestOpts { progress: None, cancel: None, force_reingest: false }` (verify the actual fields with `grep -n "pub struct IngestOpts" crates/kebab-app/src/lib.rs`).

- [ ] **Step 3: Verify build**

```bash
cd /Users/user/Workspace/projects/kebab
cargo check -p kebab-app 2>&1 | tail -5
```

Expected: PASS. If `ignore` crate not exposed, add `ignore = "0.4"` to workspace deps + kebab-app dep (or use an existing helper from `kebab-source-fs`).

- [ ] **Step 4: Commit**

```bash
git add crates/kebab-app/src/lib.rs crates/kebab-app/Cargo.toml
git commit -m "$(cat <<'EOF'
✨ feat(kebab-app): ingest_file_with_config facade (fb-31)

Single-file ingest entry. Copies bytes to _external/<hash12>.<ext>,
runs the per-medium pipeline on that single asset (reuses
ingest_with_config_opts via a SourceScope { root: _external/, include:
[<filename>], exclude: config.workspace.exclude }).

`.kebabignore` matches log a stderr warn line and proceed (explicit
ingest is bypass intent).

Returns the standard IngestReport (incremental ingest from fb-23
handles re-ingest as `unchanged`).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 3 — Integration test: `ingest_file_with_config`

**Files:**
- Create: `crates/kebab-app/tests/ingest_file.rs`

- [ ] **Step 1: Write the test**

```rust
//! Integration: kebab_app::ingest_file_with_config copies external file
//! to _external/, ingests as single asset, idempotent on second call.

use std::fs;

use kebab_config::Config;

#[test]
fn ingest_file_copies_external_md_and_reports_new() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("notes");
    let data = dir.path().join("data");
    fs::create_dir_all(&workspace).unwrap();
    fs::create_dir_all(&data).unwrap();

    let mut cfg = Config::defaults();
    cfg.workspace.root = workspace.to_string_lossy().into_owned();
    cfg.storage.data_dir = data.to_string_lossy().into_owned();
    cfg.models.embedding.provider = "none".to_string();
    cfg.models.embedding.dimensions = 0;

    // Source file outside the workspace.
    let external_src = dir.path().join("source.md");
    fs::write(&external_src, "# Hello\n\nbody.").unwrap();

    let report = kebab_app::ingest_file_with_config(cfg.clone(), &external_src).unwrap();
    assert_eq!(report.scanned, 1, "{report:?}");
    assert_eq!(report.new, 1, "{report:?}");
    assert_eq!(report.unchanged, 0, "{report:?}");

    // _external/ dir created, file copied with hash prefix.
    let ext_dir = workspace.join("_external");
    assert!(ext_dir.is_dir());
    let entries: Vec<_> = fs::read_dir(&ext_dir).unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert_eq!(entries.len(), 1, "exactly one file in _external/");
    let name = entries[0].file_name().to_string_lossy().into_owned();
    assert!(name.ends_with(".md"));

    // .kebabignore has _external/ line.
    let ki = fs::read_to_string(workspace.join(".kebabignore")).unwrap();
    assert!(ki.lines().any(|l| l.trim() == "_external/"));
}

#[test]
fn ingest_file_idempotent_on_second_call() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("notes");
    let data = dir.path().join("data");
    fs::create_dir_all(&workspace).unwrap();
    fs::create_dir_all(&data).unwrap();

    let mut cfg = Config::defaults();
    cfg.workspace.root = workspace.to_string_lossy().into_owned();
    cfg.storage.data_dir = data.to_string_lossy().into_owned();
    cfg.models.embedding.provider = "none".to_string();
    cfg.models.embedding.dimensions = 0;

    let src = dir.path().join("doc.md");
    fs::write(&src, "# A\n\nbody.").unwrap();

    let r1 = kebab_app::ingest_file_with_config(cfg.clone(), &src).unwrap();
    assert_eq!(r1.new, 1);

    let r2 = kebab_app::ingest_file_with_config(cfg.clone(), &src).unwrap();
    assert_eq!(r2.new, 0, "{r2:?}");
    assert_eq!(r2.unchanged, 1, "{r2:?}");
}

#[test]
fn ingest_file_errors_on_missing_path() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("notes");
    let data = dir.path().join("data");
    fs::create_dir_all(&workspace).unwrap();
    fs::create_dir_all(&data).unwrap();

    let mut cfg = Config::defaults();
    cfg.workspace.root = workspace.to_string_lossy().into_owned();
    cfg.storage.data_dir = data.to_string_lossy().into_owned();
    cfg.models.embedding.provider = "none".to_string();
    cfg.models.embedding.dimensions = 0;

    let nonexistent = dir.path().join("nope.md");
    let err = kebab_app::ingest_file_with_config(cfg, &nonexistent).unwrap_err();
    assert!(err.to_string().contains("does not exist"), "{err}");
}
```

- [ ] **Step 2: Run test**

```bash
cd /Users/user/Workspace/projects/kebab
cargo test -p kebab-app --test ingest_file 2>&1 | tail -10
```

Expected: 3 tests pass.

If the SourceScope `include: vec![filename]` filter doesn't actually constrain the walk to one file (because FsSourceConnector might walk siblings too in subsequent ingests), the test will fail with `report.new > 1` on the second run. Investigate by adding `eprintln!("{report:?}")` and adapt the include filter pattern. The fix may be `include: vec![format!("/{filename}")]` for absolute path match.

- [ ] **Step 3: Commit**

```bash
git add crates/kebab-app/tests/ingest_file.rs
git commit -m "🧪 test(kebab-app): ingest_file_with_config integration (fb-31)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 4 — `ingest_stdin_with_config` facade

**Files:**
- Modify: `crates/kebab-app/src/lib.rs`

- [ ] **Step 1: Add the facade fn**

Append to `crates/kebab-app/src/lib.rs`, after `ingest_file_with_config`:

```rust
/// Stdin ingest (p9-fb-31, v1 markdown only). Prepends a YAML
/// frontmatter block (`title` + optional `source_uri`) to `body`,
/// writes the wrapped markdown to `_external/<hash12>.md`, and runs
/// `ingest_file_with_config` on the resulting file.
///
/// Errors if `body` already starts with `---` (the user should call
/// `ingest_file_with_config` directly for files that already carry
/// frontmatter).
pub fn ingest_stdin_with_config(
    config: kebab_config::Config,
    body: &str,
    title: &str,
    source_uri: Option<&str>,
) -> anyhow::Result<IngestReport> {
    let wrapped = crate::external::inject_frontmatter(body, title, source_uri)?;

    // Resolve workspace root + ensure _external/ dir.
    let workspace_root = std::path::PathBuf::from(
        kebab_config::expand_path(&config.workspace.root, ""),
    );
    let external_dir = crate::external::ensure_external_dir(&workspace_root)?;
    crate::external::ensure_kebabignore_entry(&workspace_root)?;

    // Write the wrapped markdown to _external/<hash>.md.
    let dest = crate::external::copy_to_external(
        &external_dir,
        wrapped.as_bytes(),
        "md",
    )?;

    // Delegate to ingest_file_with_config — uses the same SourceScope
    // include-filter trick so only the new asset is ingested.
    ingest_file_with_config(config, &dest)
}
```

- [ ] **Step 2: Verify compile**

```bash
cargo check -p kebab-app 2>&1 | tail -3
```

Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/kebab-app/src/lib.rs
git commit -m "✨ feat(kebab-app): ingest_stdin_with_config facade (fb-31)

Wraps body with YAML frontmatter (title + source_uri) and delegates
to ingest_file_with_config. Markdown only in v1.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 5 — Integration test: `ingest_stdin_with_config`

**Files:**
- Create: `crates/kebab-app/tests/ingest_stdin.rs`

- [ ] **Step 1: Write the test**

```rust
//! Integration: kebab_app::ingest_stdin_with_config injects frontmatter,
//! writes to _external/, ingests as single asset.

use std::fs;

use kebab_config::Config;

fn fresh_cfg(dir: &std::path::Path) -> Config {
    let workspace = dir.join("notes");
    let data = dir.join("data");
    fs::create_dir_all(&workspace).unwrap();
    fs::create_dir_all(&data).unwrap();

    let mut cfg = Config::defaults();
    cfg.workspace.root = workspace.to_string_lossy().into_owned();
    cfg.storage.data_dir = data.to_string_lossy().into_owned();
    cfg.models.embedding.provider = "none".to_string();
    cfg.models.embedding.dimensions = 0;
    cfg
}

#[test]
fn ingest_stdin_writes_frontmatter_and_reports_new() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = fresh_cfg(dir.path());

    let report = kebab_app::ingest_stdin_with_config(
        cfg.clone(),
        "## Body content\n\nMore.",
        "Article X",
        Some("https://example.com/x"),
    ).unwrap();
    assert_eq!(report.new, 1, "{report:?}");

    // _external/ contains exactly one .md file with frontmatter.
    let ext_dir = std::path::PathBuf::from(&cfg.workspace.root).join("_external");
    let entries: Vec<_> = fs::read_dir(&ext_dir).unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert_eq!(entries.len(), 1);
    let content = fs::read_to_string(entries[0].path()).unwrap();
    assert!(content.starts_with("---\n"));
    assert!(content.contains("title: \"Article X\""));
    assert!(content.contains("source_uri: \"https://example.com/x\""));
    assert!(content.contains("## Body content"));
}

#[test]
fn ingest_stdin_without_source_uri() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = fresh_cfg(dir.path());

    let report = kebab_app::ingest_stdin_with_config(
        cfg.clone(),
        "## Body",
        "Title",
        None,
    ).unwrap();
    assert_eq!(report.new, 1);

    let ext_dir = std::path::PathBuf::from(&cfg.workspace.root).join("_external");
    let entries: Vec<_> = fs::read_dir(&ext_dir).unwrap()
        .filter_map(|e| e.ok())
        .collect();
    let content = fs::read_to_string(entries[0].path()).unwrap();
    assert!(content.contains("title: \"Title\""));
    assert!(!content.contains("source_uri"));
}

#[test]
fn ingest_stdin_errors_on_existing_frontmatter() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = fresh_cfg(dir.path());

    let body = "---\ntitle: Already\n---\n\n## Body";
    let err = kebab_app::ingest_stdin_with_config(cfg, body, "New", None).unwrap_err();
    assert!(err.to_string().contains("already has frontmatter"), "{err}");
}
```

- [ ] **Step 2: Run + commit**

```bash
cargo test -p kebab-app --test ingest_stdin 2>&1 | tail -10
git add crates/kebab-app/tests/ingest_stdin.rs
git commit -m "🧪 test(kebab-app): ingest_stdin_with_config integration (fb-31)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 6 — CLI `Cmd::IngestFile` arm

**Files:**
- Modify: `crates/kebab-cli/src/main.rs`

- [ ] **Step 1: Add the variant**

Open `crates/kebab-cli/src/main.rs`. Find `enum Cmd` definition. Add (alongside other Cmd variants):

```rust
    /// Ingest a single file (workspace external paths allowed).
    /// Bytes are copied into `<workspace.root>/_external/<hash>.<ext>`.
    IngestFile {
        /// File path to ingest.
        path: std::path::PathBuf,
    },
```

- [ ] **Step 2: Add the arm in `fn run`**

In the `match &cli.command` block:

```rust
        Cmd::IngestFile { path } => {
            let cfg = kebab_config::Config::load(cli.config.as_deref())?;
            let report = kebab_app::ingest_file_with_config(cfg, path)?;
            if cli.json {
                let v = wire::wire_ingest(&report);
                println!("{}", serde_json::to_string(&v)?);
            } else {
                println!(
                    "ingest-file: scanned={} new={} updated={} unchanged={} skipped={} errors={}",
                    report.scanned, report.new, report.updated,
                    report.unchanged, report.skipped, report.errors
                );
            }
            Ok(())
        }
```

- [ ] **Step 3: Build + smoke**

```bash
cd /Users/user/Workspace/projects/kebab
cargo build -p kebab-cli 2>&1 | tail -3
echo "# Smoke" > /tmp/fb31-smoke.md
target/debug/kebab ingest-file /tmp/fb31-smoke.md --help 2>&1 | head -5
```

Expected: build clean; `ingest-file` shows up in help.

- [ ] **Step 4: Commit**

```bash
git add crates/kebab-cli/src/main.rs
git commit -m "✨ feat(kebab-cli): kebab ingest-file subcommand (fb-31)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 7 — CLI `Cmd::IngestStdin` arm

**Files:**
- Modify: `crates/kebab-cli/src/main.rs`

- [ ] **Step 1: Add the variant**

In `enum Cmd`:

```rust
    /// Ingest markdown content from stdin. v1 markdown only.
    /// Frontmatter (title + source_uri) is auto-injected.
    IngestStdin {
        /// Title — required, written to frontmatter.
        #[arg(long)]
        title: String,
        /// Source URI — optional, written to frontmatter when present.
        #[arg(long)]
        source_uri: Option<String>,
    },
```

- [ ] **Step 2: Add the arm**

```rust
        Cmd::IngestStdin { title, source_uri } => {
            use std::io::Read;
            let mut body = String::new();
            std::io::stdin()
                .read_to_string(&mut body)
                .context("kebab ingest-stdin: read stdin")?;
            let cfg = kebab_config::Config::load(cli.config.as_deref())?;
            let report = kebab_app::ingest_stdin_with_config(
                cfg,
                &body,
                title,
                source_uri.as_deref(),
            )?;
            if cli.json {
                let v = wire::wire_ingest(&report);
                println!("{}", serde_json::to_string(&v)?);
            } else {
                println!(
                    "ingest-stdin: scanned={} new={} updated={} unchanged={} skipped={} errors={}",
                    report.scanned, report.new, report.updated,
                    report.unchanged, report.skipped, report.errors
                );
            }
            Ok(())
        }
```

- [ ] **Step 3: Build + smoke**

```bash
cargo build -p kebab-cli 2>&1 | tail -3
target/debug/kebab ingest-stdin --help 2>&1 | head -5
```

Expected: build clean; `--title` flag shows.

- [ ] **Step 4: Commit**

```bash
git add crates/kebab-cli/src/main.rs
git commit -m "✨ feat(kebab-cli): kebab ingest-stdin subcommand (fb-31)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 8 — CLI integration tests for both subcommands

**Files:**
- Create: `crates/kebab-cli/tests/cli_ingest_file.rs`
- Create: `crates/kebab-cli/tests/cli_ingest_stdin.rs`

- [ ] **Step 1: Write cli_ingest_file.rs**

```rust
//! Integration: spawn `kebab ingest-file <path>` and verify ingest_report.v1.

use std::fs;
use std::process::Command;

#[test]
fn cli_ingest_file_emits_ingest_report_v1() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("notes");
    let data = dir.path().join("data");
    fs::create_dir_all(&workspace).unwrap();
    fs::create_dir_all(&data).unwrap();

    let cfg_path = dir.path().join("config.toml");
    fs::write(
        &cfg_path,
        format!(
            "[workspace]\nroot = \"{}\"\n\n[storage]\ndata_dir = \"{}\"\n\n[models.embedding]\nprovider = \"none\"\nmodel = \"none\"\nversion = \"v0\"\ndimensions = 0\n",
            workspace.display(),
            data.display(),
        ),
    ).unwrap();

    let src = dir.path().join("doc.md");
    fs::write(&src, "# A\n\nbody.").unwrap();

    let bin = env!("CARGO_BIN_EXE_kebab");
    let out = Command::new(bin)
        .args(["--json", "--config", cfg_path.to_str().unwrap(), "ingest-file"])
        .arg(&src)
        .output()
        .unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));

    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(v.get("schema_version").and_then(|s| s.as_str()), Some("ingest_report.v1"));
    assert_eq!(v.get("new").and_then(|n| n.as_u64()), Some(1));
}
```

- [ ] **Step 2: Write cli_ingest_stdin.rs**

```rust
//! Integration: spawn `kebab ingest-stdin --title X` with stdin pipe.

use std::fs;
use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn cli_ingest_stdin_emits_ingest_report_v1() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("notes");
    let data = dir.path().join("data");
    fs::create_dir_all(&workspace).unwrap();
    fs::create_dir_all(&data).unwrap();

    let cfg_path = dir.path().join("config.toml");
    fs::write(
        &cfg_path,
        format!(
            "[workspace]\nroot = \"{}\"\n\n[storage]\ndata_dir = \"{}\"\n\n[models.embedding]\nprovider = \"none\"\nmodel = \"none\"\nversion = \"v0\"\ndimensions = 0\n",
            workspace.display(),
            data.display(),
        ),
    ).unwrap();

    let bin = env!("CARGO_BIN_EXE_kebab");
    let mut child = Command::new(bin)
        .args([
            "--json", "--config", cfg_path.to_str().unwrap(),
            "ingest-stdin", "--title", "X",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    {
        let stdin = child.stdin.as_mut().unwrap();
        stdin.write_all(b"## Body\n\nbody text.\n").unwrap();
    }
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));

    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(v.get("schema_version").and_then(|s| s.as_str()), Some("ingest_report.v1"));
    assert_eq!(v.get("new").and_then(|n| n.as_u64()), Some(1));
}
```

- [ ] **Step 3: Run + commit**

```bash
cargo test -p kebab-cli --test cli_ingest_file --test cli_ingest_stdin 2>&1 | tail -10
git add crates/kebab-cli/tests/cli_ingest_file.rs crates/kebab-cli/tests/cli_ingest_stdin.rs
git commit -m "🧪 test(kebab-cli): cli_ingest_file + cli_ingest_stdin integration (fb-31)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 9 — MCP `ingest_file` + `ingest_stdin` tools

**Files:**
- Create: `crates/kebab-mcp/src/tools/ingest_file.rs`
- Create: `crates/kebab-mcp/src/tools/ingest_stdin.rs`
- Modify: `crates/kebab-mcp/src/tools/mod.rs`
- Modify: `crates/kebab-mcp/src/lib.rs`
- Modify: `crates/kebab-mcp/tests/tools_list.rs`

- [ ] **Step 1: Write ingest_file.rs**

```rust
//! `ingest_file` tool — wraps `kebab_app::ingest_file_with_config`.
//! Input: { path }. Output: ingest_report.v1 JSON.

use std::path::PathBuf;

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{to_tool_error, to_tool_success};
use crate::state::KebabAppState;

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct IngestFileInput {
    /// Absolute or relative path to the file to ingest. Workspace external
    /// paths are allowed — bytes are copied into `_external/`.
    pub path: String,
}

pub fn handle(state: &KebabAppState, input: IngestFileInput) -> CallToolResult {
    let cfg_clone = (*state.config).clone();
    let path = PathBuf::from(input.path);
    match kebab_app::ingest_file_with_config(cfg_clone, &path) {
        Ok(report) => match serde_json::to_value(&report) {
            Ok(mut v) => {
                if let serde_json::Value::Object(ref mut map) = v {
                    map.entry("schema_version".to_string())
                        .or_insert_with(|| serde_json::Value::String("ingest_report.v1".to_string()));
                }
                match serde_json::to_string(&v) {
                    Ok(json) => to_tool_success(json),
                    Err(e) => to_tool_error(&anyhow::anyhow!(e)),
                }
            }
            Err(e) => to_tool_error(&anyhow::anyhow!(e)),
        },
        Err(e) => to_tool_error(&e),
    }
}
```

- [ ] **Step 2: Write ingest_stdin.rs**

```rust
//! `ingest_stdin` tool — wraps `kebab_app::ingest_stdin_with_config`.
//! Input: { content, title, source_uri? }. Output: ingest_report.v1 JSON.

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{to_tool_error, to_tool_success};
use crate::state::KebabAppState;

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct IngestStdinInput {
    /// Markdown body content. v1 supports markdown only.
    pub content: String,
    /// Title for frontmatter injection.
    pub title: String,
    /// Optional source URI (e.g. https URL agent fetched from).
    pub source_uri: Option<String>,
}

pub fn handle(state: &KebabAppState, input: IngestStdinInput) -> CallToolResult {
    let cfg_clone = (*state.config).clone();
    match kebab_app::ingest_stdin_with_config(
        cfg_clone,
        &input.content,
        &input.title,
        input.source_uri.as_deref(),
    ) {
        Ok(report) => match serde_json::to_value(&report) {
            Ok(mut v) => {
                if let serde_json::Value::Object(ref mut map) = v {
                    map.entry("schema_version".to_string())
                        .or_insert_with(|| serde_json::Value::String("ingest_report.v1".to_string()));
                }
                match serde_json::to_string(&v) {
                    Ok(json) => to_tool_success(json),
                    Err(e) => to_tool_error(&anyhow::anyhow!(e)),
                }
            }
            Err(e) => to_tool_error(&anyhow::anyhow!(e)),
        },
        Err(e) => to_tool_error(&e),
    }
}
```

- [ ] **Step 3: Register modules**

Open `crates/kebab-mcp/src/tools/mod.rs`. Add:

```rust
pub mod ingest_file;
pub mod ingest_stdin;
```

- [ ] **Step 4: Wire into KebabHandler**

Open `crates/kebab-mcp/src/lib.rs`. Find `pub fn build_tools_vec()`. Add two entries to the vec:

```rust
        Tool::new(
            "ingest_file",
            "Ingest a single file (path) into the knowledge base. Workspace external paths allowed — bytes are copied into _external/.",
            schema_for_type::<tools::ingest_file::IngestFileInput>(),
        ),
        Tool::new(
            "ingest_stdin",
            "Ingest markdown content into the knowledge base. v1 markdown only. Frontmatter (title + source_uri) auto-injected.",
            schema_for_type::<tools::ingest_stdin::IngestStdinInput>(),
        ),
```

In `call_tool` match, add two arms (mirrors search/ask spawn_tool pattern):

```rust
            "ingest_file" => {
                let args = request.arguments.unwrap_or_default();
                self.spawn_tool(args, |state, input| {
                    tools::ingest_file::handle(&state, input)
                })
                .await
            }
            "ingest_stdin" => {
                let args = request.arguments.unwrap_or_default();
                self.spawn_tool(args, |state, input| {
                    tools::ingest_stdin::handle(&state, input)
                })
                .await
            }
```

- [ ] **Step 5: Update tools_list test**

Open `crates/kebab-mcp/tests/tools_list.rs`. Find the assertion `assert_eq!(names.len(), 4)` (or similar). Update to `6` and add asserts for `"ingest_file"` and `"ingest_stdin"` presence.

- [ ] **Step 6: Build + clippy**

```bash
cargo build -p kebab-mcp 2>&1 | tail -3
cargo clippy -p kebab-mcp --all-targets -- -D warnings 2>&1 | tail -3
cargo test -p kebab-mcp --test tools_list 2>&1 | tail -5
```

Expected: PASS, clean.

- [ ] **Step 7: Commit**

```bash
git add crates/kebab-mcp
git commit -m "$(cat <<'EOF'
✨ feat(kebab-mcp): ingest_file + ingest_stdin tools (fb-31)

5th + 6th MCP tools — first mutation surface (fb-30 v1 was read-only).
Both wrap the new kebab-app facade fns + use spawn_blocking via the
existing spawn_tool helper. tools/list now returns 6 tools.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 10 — MCP integration tests

**Files:**
- Create: `crates/kebab-mcp/tests/tools_call_ingest_file.rs`
- Create: `crates/kebab-mcp/tests/tools_call_ingest_stdin.rs`

- [ ] **Step 1: Write tools_call_ingest_file.rs**

```rust
//! Integration: tools/call name=ingest_file → ingest_report.v1.

use std::fs;

use kebab_config::Config;
use kebab_mcp::{KebabAppState, KebabHandler};

#[tokio::test]
async fn ingest_file_tool_returns_ingest_report_v1() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("notes");
    let data = dir.path().join("data");
    fs::create_dir_all(&workspace).unwrap();
    fs::create_dir_all(&data).unwrap();

    let mut cfg = Config::defaults();
    cfg.workspace.root = workspace.to_string_lossy().into_owned();
    cfg.storage.data_dir = data.to_string_lossy().into_owned();
    cfg.models.embedding.provider = "none".to_string();
    cfg.models.embedding.dimensions = 0;

    let src = dir.path().join("doc.md");
    fs::write(&src, "# Title\n\nbody.").unwrap();

    let state = KebabAppState::new(cfg, None);
    let handler = KebabHandler::new(state);

    let result = tokio::task::spawn_blocking({
        let state = handler.state().clone();
        let path = src.to_string_lossy().into_owned();
        move || {
            kebab_mcp::tools::ingest_file::handle(
                &state,
                kebab_mcp::tools::ingest_file::IngestFileInput { path },
            )
        }
    })
    .await
    .unwrap();

    assert!(!result.is_error.unwrap_or(false), "{result:?}");
    let text = match &result.content.first().unwrap().raw {
        rmcp::model::RawContent::Text(t) => &t.text,
        other => panic!("expected text content, got {other:?}"),
    };
    let v: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(v.get("schema_version").and_then(|s| s.as_str()), Some("ingest_report.v1"));
    assert_eq!(v.get("new").and_then(|n| n.as_u64()), Some(1));
}
```

- [ ] **Step 2: Write tools_call_ingest_stdin.rs**

```rust
//! Integration: tools/call name=ingest_stdin → ingest_report.v1.
//! Frontmatter precheck path also covered.

use std::fs;

use kebab_config::Config;
use kebab_mcp::{KebabAppState, KebabHandler};

fn fresh_state(dir: &std::path::Path) -> KebabAppState {
    let workspace = dir.join("notes");
    let data = dir.join("data");
    fs::create_dir_all(&workspace).unwrap();
    fs::create_dir_all(&data).unwrap();

    let mut cfg = Config::defaults();
    cfg.workspace.root = workspace.to_string_lossy().into_owned();
    cfg.storage.data_dir = data.to_string_lossy().into_owned();
    cfg.models.embedding.provider = "none".to_string();
    cfg.models.embedding.dimensions = 0;
    KebabAppState::new(cfg, None)
}

#[tokio::test]
async fn ingest_stdin_tool_returns_ingest_report_v1() {
    let dir = tempfile::tempdir().unwrap();
    let state = fresh_state(dir.path());

    let result = tokio::task::spawn_blocking({
        let state = state.clone();
        move || {
            kebab_mcp::tools::ingest_stdin::handle(
                &state,
                kebab_mcp::tools::ingest_stdin::IngestStdinInput {
                    content: "## Body".to_string(),
                    title: "X".to_string(),
                    source_uri: Some("https://example.com/x".to_string()),
                },
            )
        }
    })
    .await
    .unwrap();

    assert!(!result.is_error.unwrap_or(false), "{result:?}");
    let text = match &result.content.first().unwrap().raw {
        rmcp::model::RawContent::Text(t) => &t.text,
        other => panic!("expected text content, got {other:?}"),
    };
    let v: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(v.get("schema_version").and_then(|s| s.as_str()), Some("ingest_report.v1"));
    assert_eq!(v.get("new").and_then(|n| n.as_u64()), Some(1));
}

#[tokio::test]
async fn ingest_stdin_tool_emits_error_v1_on_existing_frontmatter() {
    let dir = tempfile::tempdir().unwrap();
    let state = fresh_state(dir.path());

    let result = tokio::task::spawn_blocking({
        let state = state.clone();
        move || {
            kebab_mcp::tools::ingest_stdin::handle(
                &state,
                kebab_mcp::tools::ingest_stdin::IngestStdinInput {
                    content: "---\ntitle: Existing\n---\n\n## Body".to_string(),
                    title: "New".to_string(),
                    source_uri: None,
                },
            )
        }
    })
    .await
    .unwrap();

    assert_eq!(result.is_error, Some(true), "{result:?}");
    let text = match &result.content.first().unwrap().raw {
        rmcp::model::RawContent::Text(t) => &t.text,
        other => panic!("expected text content, got {other:?}"),
    };
    let v: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(v.get("schema_version").and_then(|s| s.as_str()), Some("error.v1"));
}
```

- [ ] **Step 3: Run + commit**

```bash
cargo test -p kebab-mcp --test tools_call_ingest_file --test tools_call_ingest_stdin 2>&1 | tail -10
git add crates/kebab-mcp/tests
git commit -m "🧪 test(kebab-mcp): ingest_file + ingest_stdin integration (fb-31)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 11 — Doc sync

**Files:**
- Modify: `README.md`
- Modify: `HANDOFF.md`
- Modify: `CLAUDE.md`
- Modify: `integrations/claude-code/kebab/SKILL.md`
- Modify: `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md`

- [ ] **Step 1: README — commands table + MCP tool list update**

Open `README.md`. In the `## 명령` table, add two rows:

```markdown
| `kebab ingest-file <path>` | 단일 파일 ingest (workspace 외부 가능). 바이트는 `<workspace.root>/_external/<hash12>.<ext>` 로 copy. `.kebabignore` 매치 시 stderr warn 후 진행 (explicit ingest 가 bypass intent). |
| `kebab ingest-stdin --title <T> [--source-uri <URI>]` | stdin 의 markdown 본문 ingest. frontmatter (title + source_uri) 자동 prepend. v1 markdown only. |
```

In the existing MCP usage section, update tool list "4 tool" → "6 tool" + add `ingest_file` / `ingest_stdin` descriptions.

- [ ] **Step 2: HANDOFF entry**

In `## 머지 후 발견된 결정 (요약)` (or equivalent), add at top:

```markdown
- **2026-05-07 P9 post-도그푸딩 (p9-fb-31)** — `kebab ingest-file <path>` + `kebab ingest-stdin --title <T>` 두 신규 subcommand + MCP tool `ingest_file` / `ingest_stdin` (4 → 6 tool). agent 가 fetch 한 web markdown / 외부 file 을 KB 에 즉시 저장. workspace 외부 file 은 `<workspace.root>/_external/<blake3-12>.<ext>` 로 copy (deterministic 명명 → idempotent). `_external/` 디렉토리 첫 생성 시 `.kebabignore` 자동 append (walk 무한 루프 방지). stdin 은 markdown 전용 + flag (`--title`, `--source-uri`) → frontmatter 자동 prepend. .kebabignore 매치 시 stderr warn 후 진행 (explicit ingest = bypass intent). fb-30 의 v1 read-only MCP 정책 변경 — 첫 mutation tool 도입. spec: `tasks/p9/p9-fb-31-single-file-stdin-ingest.md`. design: `docs/superpowers/specs/2026-05-07-p9-fb-31-single-file-stdin-ingest-design.md`.
```

- [ ] **Step 3: CLAUDE.md — `_external/` mention**

Open `CLAUDE.md`. Find the "Naming + paths" section (or similar). Add a line:

```markdown
- `_external/` (under `workspace.root`): single-file / stdin ingest 가 외부 file 을 deterministic 명명 (`<blake3-12>.<ext>`) 으로 copy. 첫 생성 시 `.kebabignore` 자동 append.
```

- [ ] **Step 4: integrations skill — agent fetch flow recipe**

Open `integrations/claude-code/kebab/SKILL.md`. After the existing MCP server section, add:

```markdown
## Recipe D — agent fetched a web doc, save to KB

When you've fetched a markdown article (e.g. via WebFetch) that the user might query later:

1. Call MCP tool `ingest_stdin` with:
   - `content`: the markdown body
   - `title`: a stable title (article H1 or page title)
   - `source_uri`: the URL you fetched from

The doc lands in `<workspace.root>/_external/<hash>.md` and is indexed for `search` / `ask` immediately. Subsequent calls with identical content are no-ops (incremental ingest detects unchanged hash).

Don't loop ingest the same article — content-hash dedup makes it safe but wastes embedding cost.
```

Also update the existing MCP tool list (4 tool → 6 tool) and mention `ingest_file` for paths the user already has on disk.

- [ ] **Step 5: design doc §3 / §6**

Open `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md`. Find §6 (Filesystem + config layout). Add a subsection:

```markdown
### 6.3 `_external/` subdirectory (fb-31)

`<workspace.root>/_external/` 가 single-file / stdin ingest 의 destination. 명명: `<blake3-12>.<ext>` (12-char hex prefix of content hash + 원래 extension). deterministic — 동일 content 재 ingest 면 idempotent.

첫 생성 시 `<workspace.root>/.kebabignore` 에 `_external/` line 자동 append — 향후 `kebab ingest` 전체 walk 가 이 디렉토리 재 walk 안 함 (re-ingestion 무한 루프 방지).
```

- [ ] **Step 6: Commit**

```bash
git add README.md HANDOFF.md CLAUDE.md integrations/claude-code/kebab/SKILL.md docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
git commit -m "$(cat <<'EOF'
📝 docs: sync README / HANDOFF / CLAUDE / skill / design for fb-31

- README: 명령 표 에 `kebab ingest-file` + `kebab ingest-stdin` 두 row + MCP tool list 4 → 6.
- HANDOFF: post-도그푸딩 entry.
- CLAUDE.md: `_external/` 디렉토리 + naming convention 한 줄.
- integrations skill: Recipe D (agent fetched a web doc) + MCP tool list 갱신.
- design §6.3 `_external/` subdirectory 절 신설.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 12 — HOTFIXES + status flip + final verification

**Files:**
- Modify: `tasks/HOTFIXES.md`
- Modify: `tasks/p9/p9-fb-31-single-file-stdin-ingest.md`

- [ ] **Step 1: HOTFIXES entry**

Insert at top of `tasks/HOTFIXES.md` (after opening paragraphs, before the most recent entry):

```markdown
## 2026-05-07 — p9-fb-31 (post-dogfooding): single-file / stdin ingest

**Source feedback**: 사용자 도그푸딩 2026-05-06 — agent (Claude Code via MCP, fb-30) 가 web fetch 한 markdown / 단일 외부 file 을 KB 에 저장하려면 `kebab ingest` 전체 walk 재실행 비효율. agent 메모리상 string contents 도 stdin ingest 가능해야.

**Live binding 변경**:

- 신규 subcommand `kebab ingest-file <path>` — 단일 file ingest, workspace 외부 path 가능.
- 신규 subcommand `kebab ingest-stdin --title <T> [--source-uri <URI>]` — stdin 의 markdown 본문 ingest, v1 markdown only.
- 신규 MCP tool `ingest_file` + `ingest_stdin` — fb-30 v1 read-only 정책 변경, 첫 mutation surface 도입 (의도된 진화).
- 외부 file 저장 정책: `<workspace.root>/_external/<blake3-12>.<ext>` 로 copy. deterministic 명명 → idempotent. `_external/` 첫 생성 시 `.kebabignore` 자동 append (walk 무한 루프 방지).
- `.kebabignore` 매치 시 stderr warn (`warn: <path> matches .kebabignore patterns; proceeding (explicit ingest bypasses ignore)`) 후 진행. `--force-ignore` flag 불필요 — explicit ingest 가 default bypass intent.
- stdin frontmatter 처리: 본문이 `---` 으로 시작하면 error (`use kebab ingest-file`); 그 외 frontmatter block prepend (title + 옵션 source_uri, YAML 더블쿼트 escape).
- `kebab-app::external` 신규 모듈 — `ensure_external_dir`, `ensure_kebabignore_entry`, `copy_to_external`, `inject_frontmatter` helper. kebab-cli + kebab-mcp 둘 다 facade 통해 호출.
- `kebab-app::ingest_file_with_config` + `ingest_stdin_with_config` 신규 facade fn.
- MCP `tools/list` 4 → 6.

**Spec contract impact**: design §6 에 §6.3 `_external/` subdirectory 절 추가.

**Tests added**: kebab-app external::tests (12: dir / kebabignore append / copy / inject_frontmatter), kebab-app integration (3 + 3: ingest_file + ingest_stdin), kebab-cli integration (2: cli_ingest_file + cli_ingest_stdin spawn-based), kebab-mcp integration (2: tools_call_ingest_file + tools_call_ingest_stdin), tools_list assertion update.

**Known limitation (deferred)**:

- PDF / image stdin — binary stream + base64 처리 v2.
- `--title` + `--source-uri` 외 metadata field (tags, language, custom kv) — v2.
- 자동 dedup by source_uri — content hash 기반 dedup 만 (incremental ingest). URI lookup 별 task.
- Storage quota / TTL — agent 무한 ingest 시 KB 비대 우려. monitor + 별 task.
- frontmatter merge (stdin 이 이미 frontmatter 보유 시 머지) — v1 은 error.
- MCP `ingest_file` 의 multi-file batch 입력 — v1 single path. 여러 file 호출은 agent 가 N 회.

**Amends**:
- design §6 (§6.3 `_external/` subdirectory subsection 추가).
- spec `tasks/p9/p9-fb-31-single-file-stdin-ingest.md` (status `open` → `completed`).
```

- [ ] **Step 2: Status flip**

Open `tasks/p9/p9-fb-31-single-file-stdin-ingest.md`. Change frontmatter:

```yaml
status: open
```

to:

```yaml
status: completed
```

Replace banner with:

```markdown
> ✅ **구현 완료.** 본 spec 은 구현 시점의 frozen 상태. post-merge deviation 은 [HOTFIXES.md](../HOTFIXES.md) 의 `2026-05-07 — p9-fb-31` 항목 참조 — live source of truth.
```

- [ ] **Step 3: Workspace verify**

```bash
cd /Users/user/Workspace/projects/kebab
cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -10
cargo test --workspace --no-fail-fast -j 1 2>&1 | tail -30
```

Expected: clippy clean. Tests: only the 2 known reset.rs env-dependent failures.

- [ ] **Step 4: Manual smoke**

```bash
rm -rf /tmp/kebab-fb31-final
mkdir -p /tmp/kebab-fb31-final/notes /tmp/kebab-fb31-final/data
cat > /tmp/kebab-fb31-final/config.toml <<'EOF'
[workspace]
root = "/tmp/kebab-fb31-final/notes"

[storage]
data_dir = "/tmp/kebab-fb31-final/data"

[models.embedding]
provider = "none"
model = "fastembed-mle5small-384"
dimensions = 0
version = "fastembed-mle5small-384-v1"
EOF

echo "# External\n\nbody." > /tmp/external.md

echo "== ingest-file =="
target/debug/kebab --json --config /tmp/kebab-fb31-final/config.toml ingest-file /tmp/external.md | jq .

echo "== ingest-stdin =="
echo "## Body\n\nfrom stdin" | target/debug/kebab --json --config /tmp/kebab-fb31-final/config.toml ingest-stdin --title "From Agent" --source-uri "https://example.com/x" | jq .

echo "== _external/ contents =="
ls /tmp/kebab-fb31-final/notes/_external/

echo "== .kebabignore =="
cat /tmp/kebab-fb31-final/notes/.kebabignore

echo "== schema --json shows mcp_server + 6 wire schemas =="
target/debug/kebab --json --config /tmp/kebab-fb31-final/config.toml schema | jq '.capabilities.mcp_server, (.wire.schemas | length)'

echo "== mcp tools/list returns 6 tools =="
printf '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"smoke","version":"0"}}}\n{"jsonrpc":"2.0","method":"notifications/initialized"}\n{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}\n' | target/debug/kebab --config /tmp/kebab-fb31-final/config.toml mcp 2>/dev/null | tail -1 | jq '.result.tools | length'
```

Expected: ingest-file → new=1; ingest-stdin → new=1; `_external/` has 2 files (`<hash>.md` each); `.kebabignore` has `_external/` line; schema shows mcp_server=true; mcp tools/list returns 6.

- [ ] **Step 5: Commit**

```bash
git add tasks/HOTFIXES.md tasks/p9/p9-fb-31-single-file-stdin-ingest.md
git commit -m "$(cat <<'EOF'
📝 docs(tasks): HOTFIXES entry + p9-fb-31 status → completed

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Self-review checklist

- [ ] Spec section 1 (`kebab ingest-file`) — Tasks 2 + 3 + 6. ✅
- [ ] Spec section 2 (`kebab ingest-stdin`) — Tasks 4 + 5 + 7. ✅
- [ ] Spec section 3 (MCP tools) — Tasks 9 + 10. ✅
- [ ] Spec section "_external/ policy" — Task 1. ✅
- [ ] Spec section "doc sync" — Task 11. ✅
- [ ] Spec section "release trigger" — handled by separate version-bump PR after merge (mirroring fb-27 / fb-30 pattern, not in this plan).
- [ ] HOTFIXES + status flip + final verification — Task 12. ✅

If any spec requirement is uncovered, add the task before declaring the plan ready.
