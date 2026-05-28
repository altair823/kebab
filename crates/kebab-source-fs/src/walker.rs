//! Directory walker with gitignore-style filtering and symlink-cycle
//! protection.
//!
//! Filter set, in order of application:
//!   - DEFAULT_EXCLUDES (constants — VCS dirs, build artifacts, never-useful)
//!   - `config.workspace.exclude` (user-supplied per workspace)
//!   - `<root>/.kebabignore` (user-supplied kebab-specific exclude)
//!   - Built-in safety-net blacklist (`node_modules/`, `target/`, etc. —
//!     spec §5.2, applied via `crate::code_meta::BUILTIN_BLACKLIST`)
//!   - `<root>/.gitignore` (repo-root only, no nested cascade — spec §5.2)
//!
//! All five are merged via `ignore::overrides::OverrideBuilder`, which
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
//! ## Per-source skip attribution (spec §5.5)
//!
//! `walk_files_with_skips` returns a `WalkOverrides` struct that carries
//! both a `combined` matcher (used for the actual walk decision) and three
//! per-source matchers (`gitignore`, `kebabignore`, `builtin`). When an
//! entry is excluded, `classify_skip` probes the per-source matchers in
//! priority order (built-in > gitignore > kebabignore) to determine which
//! `IngestReport` counter should be incremented — without requiring a
//! second walker pass over the filesystem.
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
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use ignore::overrides::{Override, OverrideBuilder};
use walkdir::WalkDir;

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

/// Per-source `Override` matchers for skip-counter attribution (spec §5.5).
///
/// `combined` is the merged union of all sources — used for the actual
/// "is this entry excluded?" decision in the walker. The three per-source
/// matchers (`gitignore`, `kebabignore`, `builtin`) are used ONLY when
/// classifying an already-excluded path for `IngestReport` counter wiring;
/// they are never consulted for every walked file.
///
/// `default_and_config` covers DEFAULT_EXCLUDES + `config.workspace.exclude`
/// — these do NOT map to any of the three named `IngestReport` counters.
///
/// `include` is the compiled `scope.include` allow-list. When the set is
/// empty (no patterns) every file passes; when non-empty a file must match
/// at least one pattern to be accepted (directories always pass, so the
/// walker can still descend into them).
pub(crate) struct WalkOverrides {
    /// Merged matcher — same as today's `Override`; used for the walk decision.
    pub combined: Override,
    /// Matcher built from `<root>/.gitignore` patterns only.
    pub gitignore: Override,
    /// Matcher built from `<root>/.kebabignore` patterns only.
    pub kebabignore: Override,
    /// Matcher built from `crate::code_meta::BUILTIN_BLACKLIST` only.
    pub builtin: Override,
    /// Compiled allow-list from `scope.include`. Empty set = pass all.
    pub include: GlobSet,
}

/// Skip attribution category. Used by the connector when counting per-source
/// skips for `IngestReport` (spec §5.5).
///
/// Priority order per spec §5.2: built-in > gitignore > kebabignore.
/// A path matching multiple sources is attributed to the first match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SkipCategory {
    BuiltinBlacklist,
    Gitignore,
    Kebabignore,
    /// Matched DEFAULT_EXCLUDES or `config.workspace.exclude`. No dedicated
    /// counter in `IngestReport` — lumped into the existing `skipped` field.
    Other,
}

/// Build a single `Override` from a list of gitignore-style patterns, all
/// registered as excludes (prepend `!`).
///
/// Empty pattern list → an `Override` that matches nothing (i.e. no
/// exclusions). Callers must strip blanks / comments before passing.
fn build_single_matcher(root: &Path, patterns: &[&str]) -> Result<Override> {
    let mut builder = OverrideBuilder::new(root);
    for pat in patterns {
        builder
            .add(&format!("!{pat}"))
            .with_context(|| format!("invalid pattern: {pat}"))?;
    }
    builder.build().context("failed to compile override")
}

/// Build the builtin-blacklist `Override`, adding directory-level patterns in
/// addition to the `/**`-suffix ones from `BUILTIN_BLACKLIST`.
///
/// BUILTIN_BLACKLIST uses `**/X/**` patterns which match files *inside* X but
/// NOT the directory entry `X` itself (because `**/X/**` requires a path
/// component after X). The walker prunes at the directory level (`is_dir=true`),
/// so we need `**/X` (no trailing `/**`) to also match the directory itself
/// for attribution purposes.
fn build_builtin_matcher(root: &Path) -> Result<Override> {
    let mut builder = OverrideBuilder::new(root);
    for pat in crate::code_meta::BUILTIN_BLACKLIST {
        // Register the original pattern (matches files inside the dir).
        builder
            .add(&format!("!{pat}"))
            .with_context(|| format!("builtin pattern: {pat}"))?;
        // Also derive a directory-level match by stripping trailing `/**`.
        // This makes `is_dir=true` checks on the directory itself work.
        if let Some(dir_pat) = pat.strip_suffix("/**") {
            builder
                .add(&format!("!{dir_pat}"))
                .with_context(|| format!("builtin dir pattern: {dir_pat}"))?;
        }
    }
    builder
        .build()
        .context("failed to compile builtin override")
}

/// Owned-string variant of `build_single_matcher` for caller-supplied
/// `Vec<String>` sources (config.workspace.exclude, .kebabignore).
fn build_single_matcher_owned(root: &Path, patterns: &[String]) -> Result<Override> {
    let mut builder = OverrideBuilder::new(root);
    for pat in patterns {
        builder
            .add(&format!("!{pat}"))
            .with_context(|| format!("invalid pattern: {pat}"))?;
    }
    builder.build().context("failed to compile override")
}

/// Build the merged `WalkOverrides` from all five filter sources, in order:
/// DEFAULT_EXCLUDES, `config.workspace.exclude`, `.kebabignore`,
/// built-in safety-net blacklist (`crate::code_meta::BUILTIN_BLACKLIST`),
/// and `<root>/.gitignore` (root-only, no nested cascade).
///
/// Each input pattern is registered as an *exclude* (gitignore-style: a
/// leading `!` flips a positive match to a negative one in the
/// `OverrideBuilder` API). Order doesn't matter — the union is computed by
/// the underlying gitignore engine.
///
/// The three per-source matchers (`gitignore`, `kebabignore`, `builtin`) are
/// built in addition to the combined one so the connector can attribute skips
/// to the correct `IngestReport` counter without a second walker pass.
///
/// `include_patterns` (from `scope.include`) are compiled into an allow-list
/// `GlobSet`. Empty slice → pass-all (backward-compat); non-empty → file
/// must match at least one pattern to be accepted.
pub(crate) fn build_overrides(
    root: &Path,
    config_exclude: &[String],
    kbignore_patterns: &[String],
    include_patterns: &[String],
) -> Result<WalkOverrides> {
    let gitignore_patterns = read_gitignore(root)?;

    // Per-source matchers (for attribution only).
    let gitignore = build_single_matcher(
        root,
        &gitignore_patterns
            .iter()
            .map(std::string::String::as_str)
            .collect::<Vec<_>>(),
    )?;
    let kebabignore = build_single_matcher_owned(root, kbignore_patterns)?;
    // Use the directory-aware builtin matcher so that `is_dir=true` checks on
    // directory entries (e.g., `node_modules/`) are attributed to builtin rather
    // than to an overlapping gitignore pattern.
    let builtin = build_builtin_matcher(root)?;

    // Combined matcher — union of all five sources.
    let mut combined_builder = OverrideBuilder::new(root);

    for pat in DEFAULT_EXCLUDES {
        combined_builder
            .add(&format!("!{pat}"))
            .with_context(|| format!("invalid default-exclude pattern: {pat}"))?;
    }
    for pat in config_exclude {
        combined_builder
            .add(&format!("!{pat}"))
            .with_context(|| format!("invalid workspace.exclude pattern: {pat}"))?;
    }
    for pat in kbignore_patterns {
        combined_builder
            .add(&format!("!{pat}"))
            .with_context(|| format!("invalid .kebabignore pattern: {pat}"))?;
    }
    for pat in crate::code_meta::BUILTIN_BLACKLIST {
        combined_builder
            .add(&format!("!{pat}"))
            .with_context(|| format!("built-in blacklist pattern: {pat}"))?;
    }
    for pat in &gitignore_patterns {
        combined_builder
            .add(&format!("!{pat}"))
            .with_context(|| format!(".gitignore pattern: {pat}"))?;
    }
    let combined = combined_builder
        .build()
        .context("failed to compile combined override set")?;

    // Allow-list GlobSet: empty Vec → matches nothing (= pass all); non-empty
    // → file must match at least one glob to be accepted. We compile with
    // `case_insensitive=false` to keep the semantics consistent with the
    // OverrideBuilder exclude patterns above.
    let include = build_include_globset(include_patterns)?;

    Ok(WalkOverrides {
        combined,
        gitignore,
        kebabignore,
        builtin,
        include,
    })
}

/// Compile `scope.include` patterns into a `GlobSet` allow-list.
///
/// Each pattern uses `GlobBuilder` with `literal_separator = true` so that
/// `**` can cross directory boundaries while `*` stops at `/`, matching the
/// gitignore convention used throughout the rest of the walker.
///
/// An empty slice produces an empty `GlobSet` — callers interpret that as
/// "pass all files" (no allow-list constraint).
fn build_include_globset(patterns: &[String]) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pat in patterns {
        let glob = GlobBuilder::new(pat)
            .literal_separator(true)
            .build()
            .with_context(|| format!("invalid include pattern: {pat}"))?;
        builder.add(glob);
    }
    builder.build().context("failed to compile include globset")
}

/// Classify why a path was excluded, using per-source matchers in spec §5.2
/// priority order: built-in > gitignore > kebabignore > other.
///
/// `rel` must be relative to the walker root (same as `Override::matched`
/// expects). `is_dir` should match what the original walker saw.
pub(crate) fn classify_skip(rel: &Path, is_dir: bool, ov: &WalkOverrides) -> SkipCategory {
    if ov.builtin.matched(rel, is_dir).is_ignore() {
        return SkipCategory::BuiltinBlacklist;
    }
    if ov.gitignore.matched(rel, is_dir).is_ignore() {
        return SkipCategory::Gitignore;
    }
    if ov.kebabignore.matched(rel, is_dir).is_ignore() {
        return SkipCategory::Kebabignore;
    }
    SkipCategory::Other
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
        // If the pattern starts with `!` (gitignore negation/un-ignore), pass through
        // as-is. Trailing-slash normalization is unsafe here — the `!`-prefix and `/`-
        // suffix combined confuse OverrideBuilder (would produce double-`!`).
        if trimmed.starts_with('!') {
            out.push(trimmed.to_string());
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
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(std::string::ToString::to_string)
        .collect())
}

/// Skipped-path record emitted by `walk_files_with_skips`.
///
/// `path` is the absolute path of the excluded entry (dir or file).
/// For excluded directories, this is the directory itself — individual
/// files inside are not enumerated (the subtree is pruned).
pub(crate) struct SkippedEntry {
    pub path: PathBuf,
    pub category: SkipCategory,
}

/// Iterate every regular file under `root`, applying `overrides.combined` and
/// detecting symlink cycles. Returns:
///   - `accepted`: absolute paths of files that passed all filters.
///   - `skipped`: entries that were excluded, with attribution.
///
/// For excluded *directories*, the directory path itself is returned (not the
/// individual files inside — the subtree is pruned in one step, matching the
/// walker's `skip_current_dir` behavior).
///
/// Strategy:
///   - `walkdir::WalkDir::follow_links(true)` to traverse symlinks.
///   - Manual per-entry check (instead of `filter_entry`) so we can capture
///     the excluded paths for skip attribution.
///   - Maintain `visited: HashSet<PathBuf>` of *canonical* paths. Before
///     descending into a directory entry, canonicalize and check the set;
///     if already present, skip. This breaks `a -> b -> a` cycles in O(n)
///     per entry without a custom recursive walker.
pub(crate) fn walk_files_with_skips(
    root: &Path,
    overrides: &WalkOverrides,
) -> Result<(Vec<PathBuf>, Vec<SkippedEntry>)> {
    let mut accepted = Vec::new();
    let mut skipped: Vec<SkippedEntry> = Vec::new();
    let mut visited: HashSet<PathBuf> = HashSet::new();

    // Use a non-filtering iterator so we see excluded entries too.
    let walker = WalkDir::new(root).follow_links(true).into_iter();
    // We still use filter_entry for the *combined* override so that walkdir
    // can short-circuit pruned directories. But we wrap it so we can capture
    // the exclusion reason before discarding the entry.
    //
    // Problem: filter_entry discards without letting us see the entry first.
    // Solution: use the raw iterator (no filter_entry) and manage skip_current_dir
    // manually, which lets us record what was excluded before pruning.
    let mut it = walker;

    while let Some(res) = it.next() {
        let entry = match res {
            Ok(e) => e,
            Err(err) => {
                tracing::warn!(error = %err, "walkdir entry error; skipping");
                continue;
            }
        };

        let path = entry.path();
        let rel = match path.strip_prefix(root) {
            Ok(p) => p,
            Err(_) => path,
        };
        let is_dir = entry.file_type().is_dir();
        let excluded = overrides.combined.matched(rel, is_dir).is_ignore();

        if excluded {
            let cat = classify_skip(rel, is_dir, overrides);
            skipped.push(SkippedEntry {
                path: path.to_path_buf(),
                category: cat,
            });
            if is_dir {
                // Prune the subtree — don't descend into excluded dirs.
                it.skip_current_dir();
            }
            continue;
        }

        // Cycle guard for directories.
        if is_dir {
            match std::fs::canonicalize(path) {
                Ok(canon) => {
                    if !visited.insert(canon) {
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
            // Apply include allow-list: if non-empty, the file's path
            // relative to root must match at least one pattern.
            if !overrides.include.is_empty() && !overrides.include.is_match(rel) {
                // Not in the allow-list — silently drop (no skip counter;
                // the include filter is not a "skip" source in IngestReport).
                continue;
            }
            accepted.push(path.to_path_buf());
        }
    }

    Ok((accepted, skipped))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_inputs_compile_into_an_override() {
        let dir = tempfile::tempdir().unwrap();
        let ov = build_overrides(dir.path(), &[], &[], &[]).unwrap();
        // Default-excludes only; non-special files should not match.
        let m = ov.combined.matched(Path::new("notes/alpha.md"), false);
        assert!(!m.is_ignore());
    }

    #[test]
    fn default_excludes_ds_store_and_resource_forks() {
        let dir = tempfile::tempdir().unwrap();
        let ov = build_overrides(dir.path(), &[], &[], &[]).unwrap();
        assert!(
            ov.combined
                .matched(Path::new(".DS_Store"), false)
                .is_ignore()
        );
        assert!(
            ov.combined
                .matched(Path::new("notes/.DS_Store"), false)
                .is_ignore()
        );
        assert!(
            ov.combined
                .matched(Path::new("._foo.md"), false)
                .is_ignore()
        );
        assert!(
            ov.combined
                .matched(Path::new("notes/._sidecar"), false)
                .is_ignore()
        );
    }

    #[test]
    fn config_exclude_filters_tmp_and_node_modules() {
        let dir = tempfile::tempdir().unwrap();
        let ov = build_overrides(
            dir.path(),
            &["*.tmp".to_string(), "node_modules/**".to_string()],
            &[],
            &[],
        )
        .unwrap();
        assert!(ov.combined.matched(Path::new("a.tmp"), false).is_ignore());
        assert!(
            ov.combined
                .matched(Path::new("notes/x.tmp"), false)
                .is_ignore()
        );
        assert!(
            ov.combined
                .matched(Path::new("node_modules/foo/bar.js"), false)
                .is_ignore()
        );
        assert!(
            !ov.combined
                .matched(Path::new("alpha.md"), false)
                .is_ignore()
        );
    }

    #[test]
    fn kbignore_union_with_config_exclude() {
        // "either set excluding it ⇒ excluded"
        let dir = tempfile::tempdir().unwrap();
        let ov = build_overrides(
            dir.path(),
            &["*.tmp".to_string()],
            &["secret/**".to_string()],
            &[],
        )
        .unwrap();
        assert!(ov.combined.matched(Path::new("a.tmp"), false).is_ignore());
        assert!(
            ov.combined
                .matched(Path::new("secret/key.md"), false)
                .is_ignore()
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

        let overrides = build_overrides(root, &[], &[], &[]).unwrap();
        // Override::matched expects paths relative to the builder's root.
        let m_in = overrides.combined.matched(Path::new("src/main.rs"), false);
        let m_out = overrides
            .combined
            .matched(Path::new("node_modules/foo/bar.js"), false);

        assert!(!m_in.is_ignore(), "src/main.rs should NOT be ignored");
        assert!(
            m_out.is_ignore(),
            "node_modules/foo/bar.js SHOULD be ignored"
        );
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

        let overrides = build_overrides(root, &[], &[], &[]).unwrap();
        // Override::matched expects paths relative to the builder's root.
        for blacklisted in [
            "target/x/y.txt",
            "__pycache__/x/y.txt",
            ".venv/x/y.txt",
            "venv/x/y.txt",
            "env/x/y.txt",
        ] {
            let m = overrides.combined.matched(Path::new(blacklisted), false);
            assert!(m.is_ignore(), "{blacklisted} should be ignored");
        }
        let m_ok = overrides.combined.matched(Path::new("ok/z.txt"), false);
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

        let overrides = build_overrides(root, &[], &[], &[]).unwrap();
        assert!(
            overrides
                .combined
                .matched(Path::new("a.log"), false)
                .is_ignore()
        );
        assert!(
            overrides
                .combined
                .matched(Path::new("dist/bundle.js"), false)
                .is_ignore()
        );
        assert!(
            !overrides
                .combined
                .matched(Path::new("src/main.rs"), false)
                .is_ignore()
        );
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
        let overrides = build_overrides(root, &[], &[], &[]).unwrap();
        assert!(
            !overrides
                .combined
                .matched(Path::new("a.log"), false)
                .is_ignore()
        );
        assert!(
            !overrides
                .combined
                .matched(Path::new("src/main.rs"), false)
                .is_ignore()
        );
    }

    #[test]
    fn gitignore_negation_with_trailing_slash_passes_through() {
        use std::fs;
        use tempfile::TempDir;
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        // Negation pattern. We don't fully implement gitignore negation
        // semantics, but at minimum it must not produce double-`!` corruption.
        fs::write(root.join(".gitignore"), "!keep/\n").unwrap();
        // Just verify build_overrides doesn't error.
        let result = build_overrides(root, &[], &[], &[]);
        assert!(
            result.is_ok(),
            "should not error on negation pattern: {:?}",
            result.err()
        );
    }

    // ── Skip attribution tests ────────────────────────────────────────────────

    #[test]
    fn classify_skip_attributes_builtin_over_gitignore() {
        use std::fs;
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        // node_modules matches both BUILTIN_BLACKLIST and a hypothetical
        // .gitignore entry. Builtin must win (priority order §5.2).
        fs::write(root.join(".gitignore"), "node_modules/\n").unwrap();

        let ov = build_overrides(root, &[], &[], &[]).unwrap();
        // node_modules/ dir itself
        let cat = classify_skip(Path::new("node_modules"), true, &ov);
        assert_eq!(
            cat,
            SkipCategory::BuiltinBlacklist,
            "builtin must have priority"
        );
    }

    #[test]
    fn classify_skip_attributes_gitignore_for_log_files() {
        use std::fs;
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::write(root.join(".gitignore"), "*.log\n").unwrap();

        let ov = build_overrides(root, &[], &[], &[]).unwrap();
        let cat = classify_skip(Path::new("app.log"), false, &ov);
        assert_eq!(cat, SkipCategory::Gitignore);
    }

    #[test]
    fn classify_skip_attributes_kebabignore() {
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        let ov = build_overrides(root, &[], &["*.secret".to_string()], &[]).unwrap();
        let cat = classify_skip(Path::new("creds.secret"), false, &ov);
        assert_eq!(cat, SkipCategory::Kebabignore);
    }

    #[test]
    fn walk_files_with_skips_counts_gitignored_files() {
        use std::fs;
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::write(root.join(".gitignore"), "*.log\n").unwrap();
        fs::write(root.join("ok.md"), "# ok").unwrap();
        fs::write(root.join("skipme.log"), "x").unwrap();

        let ov = build_overrides(root, &[], &[], &[]).unwrap();
        let (accepted, skipped_entries) = walk_files_with_skips(root, &ov).unwrap();

        let accepted_names: Vec<_> = accepted
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert!(
            accepted_names.iter().any(|n| n == "ok.md"),
            "ok.md should be accepted; got: {accepted_names:?}"
        );
        assert!(
            !accepted_names.iter().any(|n| n == "skipme.log"),
            "skipme.log should not be accepted; got: {accepted_names:?}"
        );

        let gitignore_skipped: Vec<_> = skipped_entries
            .iter()
            .filter(|e| e.category == SkipCategory::Gitignore)
            .collect();
        assert!(
            gitignore_skipped
                .iter()
                .any(|e| e.path.file_name().is_some_and(|n| n == "skipme.log")),
            "skipme.log should appear in gitignore_skipped; skipped: {:?}",
            skipped_entries.iter().map(|e| &e.path).collect::<Vec<_>>()
        );
    }

    #[test]
    fn walk_files_with_skips_counts_builtin_blacklist_dirs() {
        use std::fs;
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("node_modules/foo")).unwrap();
        fs::write(root.join("node_modules/foo/bar.js"), "x").unwrap();
        fs::write(root.join("ok.md"), "# ok").unwrap();

        let ov = build_overrides(root, &[], &[], &[]).unwrap();
        let (accepted, skipped_entries) = walk_files_with_skips(root, &ov).unwrap();

        let accepted_names: Vec<_> = accepted
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert!(
            accepted_names.iter().any(|n| n == "ok.md"),
            "ok.md must be accepted; got: {accepted_names:?}"
        );

        let builtin_skipped: Vec<_> = skipped_entries
            .iter()
            .filter(|e| e.category == SkipCategory::BuiltinBlacklist)
            .collect();
        assert!(
            !builtin_skipped.is_empty(),
            "node_modules/ should produce at least one BuiltinBlacklist skip"
        );
        assert!(
            builtin_skipped
                .iter()
                .any(|e| e.path.components().any(|c| c.as_os_str() == "node_modules")),
            "skipped path should contain node_modules; got: {:?}",
            builtin_skipped.iter().map(|e| &e.path).collect::<Vec<_>>()
        );
    }
}
