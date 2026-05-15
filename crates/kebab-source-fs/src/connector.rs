//! `FsSourceConnector` — public surface for the crate.
//!
//! ```ignore
//! pub struct FsSourceConnector { /* internal */ }
//! impl FsSourceConnector {
//!     pub fn new(config: &kebab_config::Config) -> anyhow::Result<Self>;
//! }
//! impl kebab_core::SourceConnector for FsSourceConnector {
//!     fn scan(&self, scope: &kebab_core::SourceScope) -> anyhow::Result<Vec<kebab_core::RawAsset>>;
//! }
//! ```

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use time::OffsetDateTime;

use kebab_config::Config;
use kebab_core::{
    AssetStorage, Checksum, RawAsset, SkipExamples, SourceConnector, SourceScope, SourceUri,
    id_for_asset, to_posix,
};

use crate::hash::hash_file;
use crate::media::media_type_for;
use crate::walker::{SkipCategory, WalkOverrides, build_overrides, read_kbignore, walk_files_with_skips};

/// Local-filesystem `SourceConnector`. Constructed once from `Config`,
/// reused across `scan` calls.
///
/// State carried between `new` and `scan`:
///   - `default_root`: `config.workspace.root` resolved to a `PathBuf`. Used
///     only when `SourceScope::root` is empty (i.e. the caller did not
///     override the root).
///   - `default_exclude`: snapshot of `config.workspace.exclude` at
///     construction time.
///   - `copy_threshold_bytes`: `config.storage.copy_threshold_mb * 1 MiB`
///     pre-multiplied so we don't recompute per file.
pub struct FsSourceConnector {
    default_root: PathBuf,
    default_exclude: Vec<String>,
    copy_threshold_bytes: u64,
}

impl FsSourceConnector {
    pub fn new(config: &Config) -> Result<Self> {
        // p9-fb-05: tilde / env / `${VAR}` substitutions plus
        // relative-path resolution against the config file's
        // directory (Config.source_dir) — so `--config /tmp/cfg.toml`
        // + `root = "kb"` reads `/tmp/kb`, not the user's cwd.
        let root = config.resolve_workspace_root();

        let copy_threshold_bytes = config
            .storage
            .copy_threshold_mb
            .saturating_mul(1024 * 1024);

        Ok(Self {
            default_root: root,
            default_exclude: config.workspace.exclude.clone(),
            copy_threshold_bytes,
        })
    }

    /// Resolve the effective root and build the merged + per-source overrides.
    fn resolve_scan_params(
        &self,
        scope: &SourceScope,
    ) -> Result<(PathBuf, WalkOverrides)> {
        let root = if scope.root.as_os_str().is_empty() {
            self.default_root.clone()
        } else {
            scope.root.clone()
        };

        let mut excludes = self.default_exclude.clone();
        excludes.extend(scope.exclude.iter().cloned());
        let kbignore = read_kbignore(&root)?;

        let overrides = build_overrides(&root, &excludes, &kbignore)?;
        Ok((root, overrides))
    }

    /// Scan the workspace and return the accepted assets together with
    /// per-category skip counts and sample paths for `IngestReport`.
    ///
    /// This is the **preferred entry point** for `kebab-app`: it provides
    /// all the information needed to populate `IngestReport.skipped_gitignore`,
    /// `skipped_kebabignore`, `skipped_builtin_blacklist`, and `skip_examples`
    /// without a second walker pass.
    pub fn scan_with_skips(
        &self,
        scope: &SourceScope,
    ) -> Result<(Vec<RawAsset>, FsScanSkips)> {
        let (root, overrides) = self.resolve_scan_params(scope)?;

        log_scope_include_warning(scope);

        let (files, skipped_entries) = walk_files_with_skips(&root, &overrides)?;

        // Accumulate per-category skip counts and sample paths.
        let mut fs_skips = FsScanSkips::default();
        for entry in &skipped_entries {
            match entry.category {
                SkipCategory::BuiltinBlacklist => {
                    fs_skips.skipped_builtin_blacklist =
                        fs_skips.skipped_builtin_blacklist.saturating_add(1);
                    push_sample(
                        &mut fs_skips.skip_examples.builtin_blacklist,
                        &entry.path,
                        &root,
                    );
                }
                SkipCategory::Gitignore => {
                    fs_skips.skipped_gitignore =
                        fs_skips.skipped_gitignore.saturating_add(1);
                    push_sample(
                        &mut fs_skips.skip_examples.gitignore,
                        &entry.path,
                        &root,
                    );
                }
                SkipCategory::Kebabignore => {
                    fs_skips.skipped_kebabignore =
                        fs_skips.skipped_kebabignore.saturating_add(1);
                    // kebabignore intentionally NOT in skip_examples per spec §5.5.
                }
                SkipCategory::Other => {
                    // DEFAULT_EXCLUDES or config.workspace.exclude — no dedicated
                    // IngestReport counter; these are lumped into the existing
                    // `skipped` field by kebab-app.
                }
            }
        }

        let assets = build_assets(&files, &root, self.copy_threshold_bytes)?;
        Ok((assets, fs_skips))
    }
}

/// Per-category skip counts and sample paths returned alongside the asset list
/// by [`FsSourceConnector::scan_with_skips`].
///
/// Populated from the walker's per-source matchers without a second pass.
#[derive(Debug, Default)]
pub struct FsScanSkips {
    pub skipped_gitignore: u32,
    pub skipped_kebabignore: u32,
    pub skipped_builtin_blacklist: u32,
    /// Sample paths per spec §5.5 (≤ 5 per category). Paths are
    /// workspace-relative POSIX strings when available, absolute otherwise.
    pub skip_examples: SkipExamples,
}

/// Push a path into a sample vec (cap = 5) as a workspace-relative POSIX
/// string. Falls back to the lossy absolute path if relativisation fails.
fn push_sample(samples: &mut Vec<String>, abs: &Path, root: &Path) {
    if samples.len() >= 5 {
        return;
    }
    let rel = abs.strip_prefix(root).unwrap_or(abs);
    // Best-effort POSIX string; any non-UTF8 char → replacement char.
    let s = rel.to_string_lossy().replace('\\', "/");
    samples.push(s);
}

/// Convert a list of absolute file paths to `Vec<RawAsset>`, sorted by
/// workspace-relative POSIX path for determinism.
fn build_assets(
    files: &[PathBuf],
    root: &Path,
    copy_threshold_bytes: u64,
) -> Result<Vec<RawAsset>> {
    let mut assets = Vec::with_capacity(files.len());
    for abs in files {
        let rel = abs.strip_prefix(root).unwrap_or(abs);
        let workspace_path = match to_posix(rel) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(
                    path = %abs.display(),
                    error = %e,
                    "skipping file: path is not a valid WorkspacePath",
                );
                continue;
            }
        };

        let media_type = media_type_for(abs);
        let (byte_len, full_hex) = hash_file(abs)
            .with_context(|| format!("hashing {}", abs.display()))?;
        let checksum = Checksum(full_hex.clone());
        let asset_id = id_for_asset(&full_hex);

        let stored = if byte_len > copy_threshold_bytes {
            AssetStorage::Reference {
                path: abs.clone(),
                sha: checksum.clone(),
            }
        } else {
            AssetStorage::Copied { path: abs.clone() }
        };

        assets.push(RawAsset {
            asset_id,
            source_uri: SourceUri::File(abs.clone()),
            workspace_path,
            media_type,
            byte_len,
            checksum,
            discovered_at: OffsetDateTime::now_utc(),
            stored,
        });
    }

    assets.sort_by(|a, b| a.workspace_path.0.cmp(&b.workspace_path.0));
    Ok(assets)
}

fn log_scope_include_warning(scope: &SourceScope) {
    if !scope.include.is_empty() {
        tracing::debug!(
            count = scope.include.len(),
            "FsSourceConnector ignores scope.include — handled by extractor router"
        );
    }
}

impl SourceConnector for FsSourceConnector {
    fn scan(&self, scope: &SourceScope) -> Result<Vec<RawAsset>> {
        // Delegate to scan_with_skips; discard the skip counts.
        // Callers that need skip attribution should call scan_with_skips directly.
        let (assets, _skips) = self.scan_with_skips(scope)?;
        Ok(assets)
    }
}

// p9-fb-05: removed local `expand_tilde` + `dirs_home` shim. The
// canonical helper now lives in `kebab-config::resolve_workspace_root`
// (calling `expand_path_with_base`), so this crate just delegates via
// `Config::resolve_workspace_root` above. Keeps tilde / `${VAR}` /
// relative path semantics consistent with kebab-app and kebab-cli.

#[cfg(test)]
mod tests {
    use super::*;
    use kebab_config::Config;

    fn cfg_with_root(root: &str) -> Config {
        let mut c = Config::defaults();
        c.workspace.root = root.to_string();
        c.workspace.exclude.clear();
        c
    }

    #[test]
    fn scan_empty_dir_yields_empty_vec() {
        let dir = tempfile::tempdir().unwrap();
        let conn = FsSourceConnector::new(&cfg_with_root(
            dir.path().to_str().unwrap(),
        ))
        .unwrap();
        let scope = SourceScope::default();
        let v = conn.scan(&scope).unwrap();
        assert!(v.is_empty());
    }

    #[test]
    fn scan_emits_sorted_workspace_paths() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("notes")).unwrap();
        std::fs::write(root.join("README.md"), b"hi").unwrap();
        std::fs::write(root.join("notes/beta.md"), b"b").unwrap();
        std::fs::write(root.join("notes/alpha.md"), b"a").unwrap();

        let conn =
            FsSourceConnector::new(&cfg_with_root(root.to_str().unwrap()))
                .unwrap();
        let v = conn.scan(&SourceScope::default()).unwrap();
        let names: Vec<_> = v.iter().map(|a| a.workspace_path.0.clone()).collect();
        assert_eq!(
            names,
            vec![
                "README.md".to_string(),
                "notes/alpha.md".to_string(),
                "notes/beta.md".to_string(),
            ]
        );
    }

    #[test]
    fn scan_filters_by_kbignore() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join(".kebabignore"), "*.tmp\n").unwrap();
        std::fs::write(root.join("a.md"), b"x").unwrap();
        std::fs::write(root.join("b.tmp"), b"x").unwrap();

        let conn =
            FsSourceConnector::new(&cfg_with_root(root.to_str().unwrap()))
                .unwrap();
        let v = conn.scan(&SourceScope::default()).unwrap();
        let names: Vec<_> = v.iter().map(|a| a.workspace_path.0.clone()).collect();
        // Decision: `.kebabignore` itself IS emitted as a RawAsset (MediaType::Other("")).
        // Rationale: a config file that affects ingest is itself part of the
        // workspace contents; the markdown extractor (P1-2) will reject Other("")
        // on its own. If we ever decide to omit `.kebabignore` from the asset list,
        // this test will catch it.
        assert!(
            names.contains(&".kebabignore".to_string()),
            ".kebabignore must be emitted as an asset; got: {names:?}"
        );
        assert!(names.contains(&"a.md".to_string()));
        assert!(!names.contains(&"b.tmp".to_string()));
    }

    #[test]
    fn scan_filters_default_excludes() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.md"), b"x").unwrap();
        std::fs::write(root.join(".DS_Store"), b"\0\0").unwrap();
        std::fs::write(root.join("._sidecar"), b"\0\0").unwrap();

        let conn =
            FsSourceConnector::new(&cfg_with_root(root.to_str().unwrap()))
                .unwrap();
        let v = conn.scan(&SourceScope::default()).unwrap();
        let names: Vec<_> = v.iter().map(|a| a.workspace_path.0.clone()).collect();
        assert_eq!(names, vec!["a.md".to_string()]);
    }

    #[test]
    fn scan_unions_config_exclude_and_kbignore() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join(".kebabignore"), "*.tmp\n").unwrap();
        std::fs::write(root.join("a.md"), b"x").unwrap();
        std::fs::write(root.join("b.tmp"), b"x").unwrap();
        std::fs::write(root.join("c.log"), b"x").unwrap();

        let mut cfg = cfg_with_root(root.to_str().unwrap());
        cfg.workspace.exclude.push("*.log".to_string());

        let conn = FsSourceConnector::new(&cfg).unwrap();
        let v = conn.scan(&SourceScope::default()).unwrap();
        let names: Vec<_> = v.iter().map(|a| a.workspace_path.0.clone()).collect();
        assert!(names.contains(&"a.md".to_string()));
        assert!(!names.contains(&"b.tmp".to_string()), "kbignore should drop *.tmp");
        assert!(!names.contains(&"c.log".to_string()), "config.exclude should drop *.log");
    }

    #[test]
    fn scan_blake3_pinned_for_known_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("hello.md"), b"hello world").unwrap();

        let conn =
            FsSourceConnector::new(&cfg_with_root(root.to_str().unwrap()))
                .unwrap();
        let v = conn.scan(&SourceScope::default()).unwrap();
        assert_eq!(v.len(), 1);
        let asset = &v[0];
        assert_eq!(
            asset.checksum.0,
            "d74981efa70a0c880b8d8c1985d075dbcbf679b99a5f9914e5aaf96b831a9e24"
        );
        assert_eq!(asset.byte_len, 11);
        // asset_id is derived from the full hex via id_for_asset.
        assert_eq!(asset.asset_id, id_for_asset(&asset.checksum.0));
    }

    #[test]
    fn scan_idempotent_modulo_timestamp() {
        // Same filesystem state → identical Vec<RawAsset> *modulo*
        // discovered_at. Strip that field and compare.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("notes")).unwrap();
        std::fs::write(root.join("notes/a.md"), b"alpha").unwrap();
        std::fs::write(root.join("notes/b.md"), b"beta").unwrap();

        let conn =
            FsSourceConnector::new(&cfg_with_root(root.to_str().unwrap()))
                .unwrap();
        let v1 = conn.scan(&SourceScope::default()).unwrap();
        let v2 = conn.scan(&SourceScope::default()).unwrap();
        assert_eq!(v1.len(), v2.len());
        for (a, b) in v1.iter().zip(v2.iter()) {
            assert_eq!(a.asset_id, b.asset_id);
            assert_eq!(a.workspace_path, b.workspace_path);
            assert_eq!(a.checksum, b.checksum);
            assert_eq!(a.byte_len, b.byte_len);
            assert_eq!(a.media_type, b.media_type);
            assert_eq!(a.source_uri, b.source_uri);
            assert_eq!(a.stored, b.stored);
            // discovered_at intentionally NOT compared
        }
    }

    #[test]
    fn scan_emits_posix_normalized_paths() {
        // End-to-end: the connector must produce POSIX-normalized
        // workspace paths via `kebab_core::to_posix`. We can't construct an
        // input with literal `./` / `//` segments via the filesystem (the
        // OS won't let us), so instead we assert the resulting strings
        // are already POSIX-clean (no leading `./`, no `//`, forward
        // slashes only) — which is the post-conditions side of the
        // round-trip the unit tests in `kb-core::normalize` cover.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("a/b/c")).unwrap();
        std::fs::write(root.join("a/b/c/d.md"), b"x").unwrap();

        let conn =
            FsSourceConnector::new(&cfg_with_root(root.to_str().unwrap()))
                .unwrap();
        let v = conn.scan(&SourceScope::default()).unwrap();
        assert_eq!(v.len(), 1);
        let p = &v[0].workspace_path.0;
        assert_eq!(p, "a/b/c/d.md");
        assert!(!p.starts_with("./"));
        assert!(!p.contains("//"));
        assert!(!p.contains('\\'));
    }

    #[test]
    fn scan_skips_files_whose_name_contains_hash() {
        // `WorkspacePath` rejects `#` (collides with the W3C-Media-Fragments
        // separator used by `Citation`). The connector must drop such
        // files with a warning rather than aborting the scan.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("ok.md"), b"x").unwrap();
        std::fs::write(root.join("has#hash.md"), b"y").unwrap();

        let conn =
            FsSourceConnector::new(&cfg_with_root(root.to_str().unwrap()))
                .unwrap();
        let v = conn.scan(&SourceScope::default()).unwrap();
        let names: Vec<_> = v.iter().map(|a| a.workspace_path.0.clone()).collect();
        assert_eq!(names, vec!["ok.md".to_string()]);
    }

    #[test]
    fn copy_vs_reference_threshold_signals_intent() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("small.md"), b"hi").unwrap();

        let mut cfg = cfg_with_root(root.to_str().unwrap());
        // Threshold = 0 MiB ⇒ even a 2-byte file becomes Reference.
        cfg.storage.copy_threshold_mb = 0;
        let conn = FsSourceConnector::new(&cfg).unwrap();
        let v = conn.scan(&SourceScope::default()).unwrap();
        assert_eq!(v.len(), 1);
        match &v[0].stored {
            AssetStorage::Reference { sha, .. } => {
                assert_eq!(sha, &v[0].checksum);
            }
            other => panic!("expected Reference, got {other:?}"),
        }

        // Threshold high (default 100 MiB) ⇒ Copied.
        let mut cfg2 = cfg_with_root(root.to_str().unwrap());
        cfg2.storage.copy_threshold_mb = 100;
        let conn2 = FsSourceConnector::new(&cfg2).unwrap();
        let v2 = conn2.scan(&SourceScope::default()).unwrap();
        assert!(matches!(v2[0].stored, AssetStorage::Copied { .. }));
    }

    // ── IngestReport skip counter wiring tests ───────────────────────────────

    #[test]
    fn scan_with_skips_counts_gitignored_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join(".gitignore"), "*.log\n").unwrap();
        std::fs::write(root.join("ok.md"), b"# ok").unwrap();
        std::fs::write(root.join("skipme.log"), b"x").unwrap();

        let conn =
            FsSourceConnector::new(&cfg_with_root(root.to_str().unwrap()))
                .unwrap();
        let (_assets, skips) = conn.scan_with_skips(&SourceScope::default()).unwrap();

        assert!(
            skips.skipped_gitignore >= 1,
            "skipped_gitignore should be >= 1; got {}",
            skips.skipped_gitignore
        );
        assert!(
            skips.skip_examples.gitignore.iter().any(|p| p.contains("skipme.log")),
            "skip_examples.gitignore should contain 'skipme.log'; got: {:?}",
            skips.skip_examples.gitignore
        );
        // kebabignore counter must be 0 — file matched gitignore, not kebabignore.
        assert_eq!(skips.skipped_kebabignore, 0);
    }

    #[test]
    fn scan_with_skips_counts_builtin_blacklist_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("node_modules/foo")).unwrap();
        std::fs::write(root.join("node_modules/foo/bar.js"), b"x").unwrap();
        std::fs::write(root.join("ok.md"), b"# ok").unwrap();

        let conn =
            FsSourceConnector::new(&cfg_with_root(root.to_str().unwrap()))
                .unwrap();
        let (_assets, skips) = conn.scan_with_skips(&SourceScope::default()).unwrap();

        assert!(
            skips.skipped_builtin_blacklist >= 1,
            "skipped_builtin_blacklist should be >= 1; got {}",
            skips.skipped_builtin_blacklist
        );
        assert!(
            skips.skip_examples.builtin_blacklist.iter().any(|p| p.contains("node_modules")),
            "skip_examples.builtin_blacklist should contain a node_modules path; got: {:?}",
            skips.skip_examples.builtin_blacklist
        );
    }

    #[test]
    fn scan_with_skips_kebabignore_increments_counter_no_example() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join(".kebabignore"), "*.secret\n").unwrap();
        std::fs::write(root.join("ok.md"), b"x").unwrap();
        std::fs::write(root.join("creds.secret"), b"pw").unwrap();

        let conn =
            FsSourceConnector::new(&cfg_with_root(root.to_str().unwrap()))
                .unwrap();
        let (_assets, skips) = conn.scan_with_skips(&SourceScope::default()).unwrap();

        assert!(
            skips.skipped_kebabignore >= 1,
            "skipped_kebabignore should be >= 1; got {}",
            skips.skipped_kebabignore
        );
        // Per spec §5.5: kebabignore is intentionally NOT in skip_examples.
        assert!(
            skips.skip_examples.gitignore.is_empty(),
            "gitignore examples should be empty; got: {:?}",
            skips.skip_examples.gitignore
        );
        assert!(
            skips.skip_examples.builtin_blacklist.is_empty(),
            "builtin_blacklist examples should be empty; got: {:?}",
            skips.skip_examples.builtin_blacklist
        );
    }

    #[test]
    fn scan_with_skips_builtin_priority_over_gitignore() {
        // node_modules/ matches both BUILTIN_BLACKLIST and a .gitignore entry.
        // It must be attributed to builtin (spec §5.2 priority order).
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join(".gitignore"), "node_modules/\n").unwrap();
        std::fs::create_dir_all(root.join("node_modules/pkg")).unwrap();
        std::fs::write(root.join("node_modules/pkg/index.js"), b"x").unwrap();
        std::fs::write(root.join("ok.md"), b"x").unwrap();

        let conn =
            FsSourceConnector::new(&cfg_with_root(root.to_str().unwrap()))
                .unwrap();
        let (_assets, skips) = conn.scan_with_skips(&SourceScope::default()).unwrap();

        assert!(
            skips.skipped_builtin_blacklist >= 1,
            "builtin counter should be >= 1; got {}",
            skips.skipped_builtin_blacklist
        );
        assert_eq!(
            skips.skipped_gitignore, 0,
            "gitignore counter must be 0 when builtin wins; got {}",
            skips.skipped_gitignore
        );
    }

    #[test]
    fn skip_examples_cap_at_five() {
        // Write 7 .log files — skip_examples.gitignore must cap at 5.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join(".gitignore"), "*.log\n").unwrap();
        for i in 0..7 {
            std::fs::write(root.join(format!("f{i}.log")), b"x").unwrap();
        }
        std::fs::write(root.join("ok.md"), b"x").unwrap();

        let conn =
            FsSourceConnector::new(&cfg_with_root(root.to_str().unwrap()))
                .unwrap();
        let (_assets, skips) = conn.scan_with_skips(&SourceScope::default()).unwrap();

        assert_eq!(skips.skipped_gitignore, 7, "should count all 7");
        assert_eq!(
            skips.skip_examples.gitignore.len(),
            5,
            "skip_examples.gitignore must cap at 5; got: {:?}",
            skips.skip_examples.gitignore
        );
    }
}
