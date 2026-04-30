//! `FsSourceConnector` — public surface for the crate.
//!
//! ```ignore
//! pub struct FsSourceConnector { /* internal */ }
//! impl FsSourceConnector {
//!     pub fn new(config: &kb_config::Config) -> anyhow::Result<Self>;
//! }
//! impl kb_core::SourceConnector for FsSourceConnector {
//!     fn scan(&self, scope: &kb_core::SourceScope) -> anyhow::Result<Vec<kb_core::RawAsset>>;
//! }
//! ```

use std::path::PathBuf;

use anyhow::{Context, Result};
use time::OffsetDateTime;

use kb_config::Config;
use kb_core::{
    AssetStorage, Checksum, RawAsset, SourceConnector, SourceScope, SourceUri,
    id_for_asset, to_posix,
};

use crate::hash::hash_file;
use crate::media::media_type_for;
use crate::walker::{build_overrides, read_kbignore, walk_files};

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
        // `config.workspace.root` is a String that may contain `~` or env
        // expansions. P0-* did not yet provide a path-expansion helper in
        // kb-config; for P1-1 we expand `~` ourselves and leave `${VAR}`
        // for a follow-up. The vast majority of users hit the `~` case.
        let root = expand_tilde(&config.workspace.root);

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
}

impl SourceConnector for FsSourceConnector {
    fn scan(&self, scope: &SourceScope) -> Result<Vec<RawAsset>> {
        // `SourceScope::root` overrides config root when non-empty. This
        // matches the design's "scope is the per-call lens; config is the
        // default" split (§7.1).
        let root = if scope.root.as_os_str().is_empty() {
            self.default_root.clone()
        } else {
            scope.root.clone()
        };

        // Union: config.workspace.exclude ∪ scope.exclude ∪ .kbignore.
        // Per §6.2 the union of `.kbignore` and `config.workspace.exclude`
        // is the filter set. `scope.exclude` is added on top so a caller
        // can layer a per-call narrowing.
        let mut excludes = self.default_exclude.clone();
        excludes.extend(scope.exclude.iter().cloned());
        let kbignore = read_kbignore(&root)?;

        let overrides = build_overrides(&root, &excludes, &kbignore)?;

        // `scope.include` is intentionally ignored at this stage of the
        // pipeline: per §6.2 the workspace-level include lives in
        // `WorkspaceCfg` and is enforced by the asset writer / extractors.
        // Surfacing it here would double-filter Markdown vs PDF before the
        // extractor router gets to see them.
        if !scope.include.is_empty() {
            tracing::debug!(
                count = scope.include.len(),
                "FsSourceConnector ignores scope.include — handled by extractor router"
            );
        }

        let files = walk_files(&root, &overrides)?;

        let mut assets = Vec::with_capacity(files.len());
        for abs in &files {
            // `to_posix` does NFC + leading `./` strip + `#` rejection.
            // Compute the workspace-relative path before handing to it so
            // emitted `WorkspacePath` is always relative.
            let rel = abs.strip_prefix(&root).unwrap_or(abs);
            let workspace_path = match to_posix(rel) {
                Ok(p) => p,
                Err(e) => {
                    // A path containing `#` is the only documented reason
                    // `to_posix` fails today. Drop the file with a warning
                    // rather than aborting the entire scan — a single bad
                    // filename should not nuke a 10 000-file ingest.
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

            // Storage variant signals *intent*, not an actual copy.
            // P1-6 (asset writer) is responsible for the on-disk copy.
            let stored = if byte_len > self.copy_threshold_bytes {
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

        // Determinism: sort by workspace_path. WorkspacePath is a String
        // newtype with stable lexicographic ordering. Two scans of the
        // same tree must produce identical Vec<RawAsset> modulo the
        // wall-clock `discovered_at` field.
        assets.sort_by(|a, b| a.workspace_path.0.cmp(&b.workspace_path.0));

        Ok(assets)
    }
}

/// Expand a leading `~` to the current user's home directory. No-op for
/// any other shape (absolute, relative, `${VAR}`-style).
fn expand_tilde(s: &str) -> PathBuf {
    if let Some(rest) = s.strip_prefix("~/") {
        if let Some(home) = dirs_home() {
            return home.join(rest);
        }
    } else if s == "~" {
        if let Some(home) = dirs_home() {
            return home;
        }
    }
    PathBuf::from(s)
}

/// Tiny `dirs::home_dir`-compat shim that does NOT add the `dirs` crate to
/// our dep set (we explicitly enumerate allowed deps in the task spec).
/// Reads `$HOME` directly.
fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kb_config::Config;

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
        std::fs::write(root.join(".kbignore"), "*.tmp\n").unwrap();
        std::fs::write(root.join("a.md"), b"x").unwrap();
        std::fs::write(root.join("b.tmp"), b"x").unwrap();

        let conn =
            FsSourceConnector::new(&cfg_with_root(root.to_str().unwrap()))
                .unwrap();
        let v = conn.scan(&SourceScope::default()).unwrap();
        let names: Vec<_> = v.iter().map(|a| a.workspace_path.0.clone()).collect();
        // .kbignore itself starts with `.` and is not in DEFAULT_EXCLUDES,
        // so it is *not* automatically hidden — but the task spec only
        // requires `*.tmp` and `.DS_Store` / `._*` filtering, and the
        // `.kbignore` file is a legitimate "Other(\"\")" asset. Either
        // present-or-absent is acceptable; the assertion below pins
        // current behaviour: .kbignore appears, b.tmp does not.
        assert!(names.contains(&".kbignore".to_string()));
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
        std::fs::write(root.join(".kbignore"), "*.tmp\n").unwrap();
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
        // workspace paths via `kb_core::to_posix`. We can't construct an
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
}
