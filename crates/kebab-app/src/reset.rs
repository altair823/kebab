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
//!
//! `--orphans-only` purges stored docs that are outside the current walker
//! scope (config narrowing / removed sub-directory). No filesystem paths are
//! removed — this is purely a store-level reconciliation.

use std::collections::HashSet;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use kebab_config::{Config, expand_path};
use kebab_core::WorkspacePath;

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
    /// Purge stored docs that are outside the current walker scope (no
    /// filesystem paths are removed). Filesystem existence is NOT checked —
    /// anything the current walker would not visit is considered an orphan.
    /// The explicit complement to the conservative `sweep_deleted_files`
    /// that runs during ingest (which leaves on-disk-but-out-of-scope docs
    /// alone for data safety).
    OrphansOnly,
}

/// Result of a successful wipe — emitted as `reset_report.v1` by the
/// CLI's `--json` mode and used by the human-mode summary line.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResetReport {
    pub scope: ResetScope,
    pub removed_paths: Vec<PathBuf>,
    pub embedding_rows_truncated: u64,
    /// Number of stored docs purged because they are outside the current
    /// walker scope. Non-zero only when `scope == OrphansOnly`.
    /// `#[serde(default)]` preserves back-compat with older callers that
    /// do not include this field.
    #[serde(default)]
    pub orphans_purged: u32,
    /// Paths of the orphaned docs that were purged. Sorted for deterministic
    /// output. Non-empty only when `scope == OrphansOnly`.
    #[serde(default)]
    pub purged_paths: Vec<WorkspacePath>,
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
        // OrphansOnly operates purely at the store level — no filesystem paths
        // are removed. Return empty so `estimate_size_bytes` stays zero and
        // the existing confirm UI path for directory wipes is skipped.
        ResetScope::OrphansOnly => vec![],
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

/// Compute the workspace paths stored in SQLite that are NOT visited by
/// the current walker scope (i.e. they are "orphans" — on disk but
/// outside the configured include/exclude rules, or from a sub-directory
/// that has since been removed from the workspace).
///
/// Does NOT check filesystem existence — `OrphansOnly` is the explicit
/// "I know what I'm doing" variant; callers that want the conservative
/// fs-aware sweep should use `sweep_deleted_files` inside ingest.
///
/// Returns the list sorted for deterministic output. Called twice by the
/// CLI path (once for the confirm UI preview, once inside `execute`);
/// the double scan is acceptable for a rare destructive operation.
pub fn enumerate_orphans(cfg: &Config) -> Result<Vec<WorkspacePath>> {
    use kebab_core::DocumentStore as _;
    use kebab_source_fs::FsSourceConnector;
    use kebab_core::SourceScope;

    let store = kebab_store_sqlite::SqliteStore::open(cfg)
        .context("enumerate_orphans: open SqliteStore")?;

    let stored = store
        .all_workspace_paths()
        .context("enumerate_orphans: all_workspace_paths")?;

    if stored.is_empty() {
        return Ok(Vec::new());
    }

    // Build the same SourceScope the CLI's ingest path uses: root from
    // config, exclude list from config, no include override (full scope).
    let root = cfg.resolve_workspace_root();
    let scope = SourceScope {
        root: root.clone(),
        exclude: cfg.workspace.exclude.clone(),
        ..Default::default()
    };

    let connector = FsSourceConnector::new(cfg)
        .context("enumerate_orphans: build FsSourceConnector")?;
    let (assets, _skips) = connector
        .scan_with_skips(&scope)
        .context("enumerate_orphans: scan workspace")?;

    let scanned: HashSet<WorkspacePath> = assets
        .into_iter()
        .map(|a| a.workspace_path)
        .collect();

    let mut orphans: Vec<WorkspacePath> = stored
        .into_iter()
        .filter(|p| !scanned.contains(p))
        .collect();
    orphans.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(orphans)
}

/// Wipe every path from `enumerate_paths(scope, cfg)`. For
/// `ResetScope::VectorOnly`, also truncates the SQLite
/// `embedding_records` table so the store doesn't point at the Lance
/// rows we just removed off-disk.
///
/// For `ResetScope::OrphansOnly`, no filesystem directories are removed.
/// Instead the store is reconciled: stored docs outside the current walker
/// scope are purged from SQLite (+ vector store when configured). The
/// caller is expected to have already shown the confirm UI using
/// `enumerate_orphans`.
///
/// Idempotent: a missing path is treated as already-removed (success).
/// Returns a `ResetReport` listing exactly what was removed (paths that
/// existed before the call) so `--json` callers see the truth, not the
/// request.
pub fn execute(scope: ResetScope, cfg: &Config) -> Result<ResetReport> {
    if matches!(scope, ResetScope::OrphansOnly) {
        return execute_orphans_only(cfg);
    }

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
        orphans_purged: 0,
        purged_paths: Vec::new(),
    })
}

/// Execute the `OrphansOnly` variant: reconcile stored docs against the
/// current walker scope without touching any filesystem directory.
fn execute_orphans_only(cfg: &Config) -> Result<ResetReport> {
    let orphans = enumerate_orphans(cfg)
        .context("execute_orphans_only: enumerate orphans")?;

    if orphans.is_empty() {
        return Ok(ResetReport {
            scope: ResetScope::OrphansOnly,
            removed_paths: Vec::new(),
            embedding_rows_truncated: 0,
            orphans_purged: 0,
            purged_paths: Vec::new(),
        });
    }

    let store = std::sync::Arc::new(
        kebab_store_sqlite::SqliteStore::open(cfg)
            .context("execute_orphans_only: open SqliteStore")?,
    );

    // Open vector store if configured. Mirror the same guard the ingest
    // path uses: only construct when the provider is not "none" / dims > 0.
    let vector_store: Option<kebab_store_vector::LanceVectorStore> =
        open_vector_store_if_configured(cfg, store.clone())?;

    let mut purged_paths: Vec<WorkspacePath> = Vec::new();

    for path in &orphans {
        let chunk_ids = kebab_store_sqlite::purge_deleted_workspace_path(&store, path)
            .with_context(|| format!("execute_orphans_only: purge {}", path.0))?;

        if let Some(ref vs) = vector_store {
            if !chunk_ids.is_empty() {
                use kebab_core::VectorStore as _;
                if let Err(e) = vs.delete_by_chunk_ids(&chunk_ids) {
                    tracing::warn!(
                        target: "kebab-app",
                        path = %path.0,
                        count = chunk_ids.len(),
                        error = %e,
                        "reset --orphans-only: vector delete failed; SQLite side already cleaned"
                    );
                }
            }
        }

        tracing::info!(
            target: "kebab-app",
            path = %path.0,
            "reset --orphans-only: purged orphan document"
        );
        purged_paths.push(path.clone());
    }

    let orphans_purged = u32::try_from(purged_paths.len()).unwrap_or(u32::MAX);

    Ok(ResetReport {
        scope: ResetScope::OrphansOnly,
        removed_paths: Vec::new(),
        embedding_rows_truncated: 0,
        orphans_purged,
        purged_paths,
    })
}

/// Open the Lance vector store if the configured embedding provider is
/// active (non-"none", dimensions > 0). Returns `None` for lexical-only
/// configs. Mirrors the guard in `App::vector`.
fn open_vector_store_if_configured(
    cfg: &Config,
    store: std::sync::Arc<kebab_store_sqlite::SqliteStore>,
) -> Result<Option<kebab_store_vector::LanceVectorStore>> {
    if cfg.models.embedding.provider == "none" || cfg.models.embedding.dimensions == 0 {
        return Ok(None);
    }
    match kebab_store_vector::LanceVectorStore::new(cfg, store) {
        Ok(vs) => Ok(Some(vs)),
        Err(e) => {
            tracing::warn!(
                target: "kebab-app",
                error = %e,
                "reset --orphans-only: could not open vector store; skipping vector delete"
            );
            Ok(None)
        }
    }
}

/// Open the SQLite store at the configured path and run
/// `truncate_embedding_records`. Returns the count of truncated rows
/// (the helper itself reports `DELETE` rowcount). If the SQLite file
/// does not exist (e.g. user has never ingested), returns 0 — not an
/// error.
fn truncate_embeddings(cfg: &Config) -> Result<u64> {
    let data_dir = expand_path(&cfg.storage.data_dir, "");
    let sqlite_path = data_dir.join("kebab.sqlite");
    if !sqlite_path.exists() {
        return Ok(0);
    }
    let store = kebab_store_sqlite::SqliteStore::open(cfg)
        .context("open SqliteStore for truncate_embedding_records")?;
    store.truncate_embedding_records()
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
        let unique: std::collections::HashSet<_> = paths.iter().collect();
        assert_eq!(unique.len(), 4);
    }

    #[test]
    fn estimate_size_returns_zero_on_missing_dir() {
        assert_eq!(estimate_size_bytes(&[PathBuf::from("/nonexistent/xyz")]), 0);
    }

    #[test]
    fn estimate_size_sums_file_lengths() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a"), b"hello").unwrap();
        std::fs::create_dir(dir.path().join("nested")).unwrap();
        std::fs::write(dir.path().join("nested/b"), b"world!").unwrap();
        let bytes = estimate_size_bytes(&[dir.path().to_path_buf()]);
        assert_eq!(bytes, 5 + 6);
    }

    #[test]
    fn enumerate_orphans_only_returns_empty_paths() {
        let cfg = Config::defaults();
        let paths = enumerate_paths(ResetScope::OrphansOnly, &cfg);
        assert!(
            paths.is_empty(),
            "OrphansOnly must return empty vec from enumerate_paths"
        );
    }
}
