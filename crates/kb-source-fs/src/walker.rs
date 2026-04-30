//! Directory walker with gitignore-style filtering and symlink-cycle
//! protection.
//!
//! Filter set (per task spec, design §6.2):
//!   - `config.workspace.exclude` (passed in by `FsSourceConnector`)
//!   - `<root>/.kbignore` (optional file at workspace root)
//!   - default-excludes for `.DS_Store` and macOS resource forks (`._*`)
//!
//! All three are merged via `ignore::overrides::OverrideBuilder`, which
//! gives full gitignore semantics (anchors, `!` negation, `**`, etc.). We
//! prepend `!` to each pattern because `OverrideBuilder` treats positive
//! patterns as "include" and negative as "exclude" — see §"Filter set"
//! comment in `build_walker` for the full reasoning.
//!
//! Symlink handling: we want to follow links (so a workspace using a
//! symlinked `notes/` directory works), but we must NOT loop forever on
//! `a -> b -> a`. `walkdir` does NOT detect cycles for us when
//! `follow_links(true)`; we layer our own visited-set on top, keyed by the
//! canonical path of every entry, and skip any entry we've already seen.
//!
//! ## Why `walkdir` instead of `ignore::WalkBuilder`?
//!
//! `ignore::WalkBuilder` bundles gitignore semantics + cycle detection in
//! one API. We use `walkdir` directly because we need explicit control
//! over canonical-path comparison for sibling-subtree symlinks (a case
//! `walkdir`'s ancestor-only check can miss). Override-based filtering
//! still uses the `ignore` crate's `Override` matcher, just decoupled from
//! its walker.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use ignore::overrides::{Override, OverrideBuilder};
use walkdir::{DirEntry, WalkDir};

/// Default-excludes baked into the connector. These are NOT configurable;
/// they cover noise that is never useful to ingest and would otherwise need
/// to appear in every user's `.kbignore`.
const DEFAULT_EXCLUDES: &[&str] = &[
    // Finder metadata
    ".DS_Store",
    "**/.DS_Store",
    // macOS resource forks (AppleDouble files)
    "._*",
    "**/._*",
];

/// Build the merged `Override` from `config.workspace.exclude` ∪ `.kbignore`
/// ∪ baked-in default excludes.
///
/// Each input pattern is registered as an *exclude* (gitignore-style: a
/// leading `!` flips a positive match to a negative one in the
/// `OverrideBuilder` API). Order doesn't matter — the union is computed by
/// the underlying gitignore engine.
pub(crate) fn build_overrides(
    root: &Path,
    config_exclude: &[String],
    kbignore_patterns: &[String],
) -> Result<Override> {
    let mut builder = OverrideBuilder::new(root);

    for pat in DEFAULT_EXCLUDES {
        builder
            .add(&format!("!{pat}"))
            .with_context(|| format!("invalid default-exclude pattern: {pat}"))?;
    }
    for pat in config_exclude {
        builder
            .add(&format!("!{pat}"))
            .with_context(|| format!("invalid workspace.exclude pattern: {pat}"))?;
    }
    for pat in kbignore_patterns {
        builder
            .add(&format!("!{pat}"))
            .with_context(|| format!("invalid .kbignore pattern: {pat}"))?;
    }

    builder.build().context("failed to compile override set")
}

/// Read `<root>/.kbignore` if it exists. Each non-blank, non-comment line is
/// a gitignore pattern. Missing file → empty Vec (not an error).
pub(crate) fn read_kbignore(root: &Path) -> Result<Vec<String>> {
    let path = root.join(".kbignore");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    Ok(text
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.to_string())
        .collect())
}

/// Iterate every regular file under `root`, applying `overrides` and
/// detecting symlink cycles. Returns absolute file paths.
///
/// Strategy:
///   - `walkdir::WalkDir::follow_links(true)` to traverse symlinks.
///   - Maintain `visited: HashSet<PathBuf>` of *canonical* paths. Before
///     descending into a directory entry, canonicalize and check the set;
///     if already present, skip. This breaks `a -> b -> a` cycles in O(n)
///     per entry without a custom recursive walker.
///   - For each yielded entry, ask `overrides` whether it is excluded; if
///     so, drop it. If the entry is a directory, also short-circuit
///     `WalkDir`'s descent via `it.skip_current_dir()`.
pub(crate) fn walk_files(root: &Path, overrides: &Override) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut visited: HashSet<PathBuf> = HashSet::new();

    let walker = WalkDir::new(root).follow_links(true).into_iter();
    let mut it = walker.filter_entry(|e| !is_excluded(e, root, overrides));

    while let Some(res) = it.next() {
        let entry = match res {
            Ok(e) => e,
            Err(err) => {
                // `walkdir` surfaces I/O errors AND its own cycle detector
                // (when follow_links is on it sometimes catches them).
                // Either way: log and skip; do not abort the whole scan.
                tracing::warn!(error = %err, "walkdir entry error; skipping");
                continue;
            }
        };

        let path = entry.path();

        // Cycle guard: only canonicalize symlinks (cheap on the common case
        // of plain files/dirs) and on directories that are followed via a
        // symlink. `walkdir`'s `path_is_symlink()` is true when the entry's
        // *original* path is a symlink (it returns true for the link, not
        // for the resolved target). For non-symlinked directories we still
        // record the canonical path so a *later* symlink that points back
        // to one of them is detected.
        if entry.file_type().is_dir() {
            match std::fs::canonicalize(path) {
                Ok(canon) => {
                    if !visited.insert(canon) {
                        // Already visited via another path → break cycle.
                        it.skip_current_dir();
                        continue;
                    }
                }
                Err(err) => {
                    tracing::debug!(
                        path = %path.display(),
                        error = %err,
                        "skipping: canonicalize failed (broken/permission-denied symlink target)"
                    );
                    continue;
                }
            }
        }

        if entry.file_type().is_file() {
            out.push(path.to_path_buf());
        }
    }

    Ok(out)
}

fn is_excluded(entry: &DirEntry, root: &Path, overrides: &Override) -> bool {
    // `Override::matched(path, is_dir)` uses the path *relative to* the
    // override builder's root. `walkdir` gives absolute paths when
    // `WalkDir::new` was given an absolute path — strip the root prefix
    // before consulting the override.
    let rel = match entry.path().strip_prefix(root) {
        Ok(p) => p,
        Err(_) => entry.path(),
    };
    overrides
        .matched(rel, entry.file_type().is_dir())
        .is_ignore()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_inputs_compile_into_an_override() {
        let dir = tempfile::tempdir().unwrap();
        let ov = build_overrides(dir.path(), &[], &[]).unwrap();
        // Default-excludes only; non-special files should not match.
        let m = ov.matched(Path::new("notes/alpha.md"), false);
        assert!(!m.is_ignore());
    }

    #[test]
    fn default_excludes_ds_store_and_resource_forks() {
        let dir = tempfile::tempdir().unwrap();
        let ov = build_overrides(dir.path(), &[], &[]).unwrap();
        assert!(ov.matched(Path::new(".DS_Store"), false).is_ignore());
        assert!(
            ov.matched(Path::new("notes/.DS_Store"), false).is_ignore()
        );
        assert!(ov.matched(Path::new("._foo.md"), false).is_ignore());
        assert!(
            ov.matched(Path::new("notes/._sidecar"), false).is_ignore()
        );
    }

    #[test]
    fn config_exclude_filters_tmp_and_node_modules() {
        let dir = tempfile::tempdir().unwrap();
        let ov = build_overrides(
            dir.path(),
            &["*.tmp".to_string(), "node_modules/**".to_string()],
            &[],
        )
        .unwrap();
        assert!(ov.matched(Path::new("a.tmp"), false).is_ignore());
        assert!(ov.matched(Path::new("notes/x.tmp"), false).is_ignore());
        assert!(
            ov.matched(Path::new("node_modules/foo/bar.js"), false)
                .is_ignore()
        );
        assert!(!ov.matched(Path::new("alpha.md"), false).is_ignore());
    }

    #[test]
    fn kbignore_union_with_config_exclude() {
        // "either set excluding it ⇒ excluded"
        let dir = tempfile::tempdir().unwrap();
        let ov = build_overrides(
            dir.path(),
            &["*.tmp".to_string()],
            &["secret/**".to_string()],
        )
        .unwrap();
        assert!(ov.matched(Path::new("a.tmp"), false).is_ignore());
        assert!(
            ov.matched(Path::new("secret/key.md"), false).is_ignore()
        );
    }

    #[test]
    fn read_kbignore_missing_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let v = read_kbignore(dir.path()).unwrap();
        assert!(v.is_empty());
    }

    #[test]
    fn read_kbignore_strips_blanks_and_comments() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(".kbignore"),
            "# comment\n*.tmp\n\nignored/**\n",
        )
        .unwrap();
        let v = read_kbignore(dir.path()).unwrap();
        assert_eq!(v, vec!["*.tmp".to_string(), "ignored/**".to_string()]);
    }
}
