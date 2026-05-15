//! Directory walker with gitignore-style filtering and symlink-cycle
//! protection.
//!
//! Filter set (per task spec, design §6.2):
//!   - `config.workspace.exclude` (passed in by `FsSourceConnector`)
//!   - `<root>/.kebabignore` (optional file at workspace root)
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
/// to appear in every user's `.kebabignore`.
const DEFAULT_EXCLUDES: &[&str] = &[
    // Finder metadata
    ".DS_Store",
    "**/.DS_Store",
    // macOS resource forks (AppleDouble files)
    "._*",
    "**/._*",
];

/// Build the merged `Override` from `config.workspace.exclude` ∪ `.kebabignore`
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
            .with_context(|| format!("invalid .kebabignore pattern: {pat}"))?;
    }

    // p10-1A-1: built-in safety-net blacklist (spec §5.2). 6 patterns
    // universal across ecosystems. User can negate via `.kebabignore`.
    for pat in kebab_parse_code::BUILTIN_BLACKLIST {
        builder
            .add(&format!("!{pat}"))
            .with_context(|| format!("built-in blacklist pattern: {pat}"))?;
    }

    // p10-1A-1: honor repo-root `.gitignore` (spec §5.2). Read once,
    // merge with same convention as user `.kebabignore`. Nested
    // cascade deferred to P+.
    let gitignore_patterns = read_gitignore(root)?;
    for pat in &gitignore_patterns {
        builder
            .add(&format!("!{pat}"))
            .with_context(|| format!(".gitignore pattern: {pat}"))?;
    }

    builder.build().context("failed to compile override set")
}

/// Read `<root>/.gitignore` (single-file, root-only — nested cascade is P+).
/// Missing file → empty Vec. Comments / blanks stripped.
///
/// Trailing-slash patterns (`dist/`) in real gitignore mean "match the
/// directory AND everything inside it". `OverrideBuilder::matched(path,
/// is_dir=false)` only checks `is_dir` for the trailing-slash variant, so
/// `dist/bundle.js` would not be matched. We normalize by also emitting a
/// `<stem>/**` variant so files inside the directory are caught.
pub(crate) fn read_gitignore(root: &Path) -> Result<Vec<String>> {
    let p = root.join(".gitignore");
    if !p.exists() {
        return Ok(vec![]);
    }
    let s = std::fs::read_to_string(&p)
        .with_context(|| format!("read .gitignore at {}", p.display()))?;
    let mut out = Vec::new();
    for line in s.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some(stem) = trimmed.strip_suffix('/') {
            // Keep the dir-only form so `is_dir=true` matches are still
            // excluded (e.g., for skip_current_dir in the walker).
            out.push(trimmed.to_string());
            // Also emit a glob that catches files inside the directory,
            // since `is_dir=false` won't satisfy the trailing-slash form.
            out.push(format!("{stem}/**"));
        } else {
            out.push(trimmed.to_string());
        }
    }
    Ok(out)
}

/// Read `<root>/.kebabignore` if it exists. Each non-blank, non-comment line is
/// a gitignore pattern. Missing file → empty Vec (not an error).
pub(crate) fn read_kbignore(root: &Path) -> Result<Vec<String>> {
    let path = root.join(".kebabignore");
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
            dir.path().join(".kebabignore"),
            "# comment\n*.tmp\n\nignored/**\n",
        )
        .unwrap();
        let v = read_kbignore(dir.path()).unwrap();
        assert_eq!(v, vec!["*.tmp".to_string(), "ignored/**".to_string()]);
    }

    #[test]
    fn built_in_blacklist_excludes_node_modules() {
        use std::fs;
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(root.join("node_modules/foo")).unwrap();
        fs::write(root.join("src/main.rs"), "x").unwrap();
        fs::write(root.join("node_modules/foo/bar.js"), "x").unwrap();

        let overrides = build_overrides(root, &[], &[]).unwrap();
        // Override::matched expects paths relative to the builder's root.
        let m_in = overrides.matched(Path::new("src/main.rs"), false);
        let m_out = overrides.matched(Path::new("node_modules/foo/bar.js"), false);

        assert!(!m_in.is_ignore(), "src/main.rs should NOT be ignored");
        assert!(m_out.is_ignore(), "node_modules/foo/bar.js SHOULD be ignored");
    }

    #[test]
    fn built_in_blacklist_excludes_target_pycache_venv() {
        use std::fs;
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        for dir in ["target/x", "__pycache__/x", ".venv/x", "venv/x", "env/x"] {
            fs::create_dir_all(root.join(dir)).unwrap();
            fs::write(root.join(dir).join("y.txt"), "z").unwrap();
        }
        fs::create_dir_all(root.join("ok")).unwrap();
        fs::write(root.join("ok/z.txt"), "z").unwrap();

        let overrides = build_overrides(root, &[], &[]).unwrap();
        // Override::matched expects paths relative to the builder's root.
        for blacklisted in [
            "target/x/y.txt",
            "__pycache__/x/y.txt",
            ".venv/x/y.txt",
            "venv/x/y.txt",
            "env/x/y.txt",
        ] {
            let m = overrides.matched(Path::new(blacklisted), false);
            assert!(m.is_ignore(), "{blacklisted} should be ignored");
        }
        let m_ok = overrides.matched(Path::new("ok/z.txt"), false);
        assert!(!m_ok.is_ignore(), "ok/z.txt should not be ignored");
    }

    #[test]
    fn gitignore_at_repo_root_excludes_matching_files() {
        use std::fs;
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join(".gitignore"), "*.log\ndist/\n").unwrap();
        fs::write(root.join("a.log"), "x").unwrap();
        fs::write(root.join("src/main.rs"), "x").unwrap();
        fs::create_dir_all(root.join("dist")).unwrap();
        fs::write(root.join("dist/bundle.js"), "x").unwrap();

        let overrides = build_overrides(root, &[], &[]).unwrap();
        assert!(overrides.matched(Path::new("a.log"), false).is_ignore());
        assert!(overrides.matched(Path::new("dist/bundle.js"), false).is_ignore());
        assert!(!overrides.matched(Path::new("src/main.rs"), false).is_ignore());
    }

    #[test]
    fn gitignore_missing_is_no_op() {
        use std::fs;
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::write(root.join("a.log"), "x").unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/main.rs"), "x").unwrap();

        // No .gitignore present — patterns from .gitignore should not affect overrides.
        let overrides = build_overrides(root, &[], &[]).unwrap();
        assert!(!overrides.matched(Path::new("a.log"), false).is_ignore());
        assert!(!overrides.matched(Path::new("src/main.rs"), false).is_ignore());
    }
}
