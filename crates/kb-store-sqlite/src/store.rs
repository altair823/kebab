//! `SqliteStore` — open + run_migrations + asset writer.
//!
//! The store wraps a single `rusqlite::Connection` behind a
//! `std::sync::Mutex` so the public trait impls (which take `&self`) can
//! still issue mutating SQL. Concurrency is intentionally coarse for P1;
//! later phases can swap to a connection pool if measurement shows the
//! mutex on the hot path.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};
use rusqlite::Connection;

use crate::error::StoreError;
use crate::schema;

/// Default file name under `config.storage.data_dir`. Kept private — the
/// path layout is a §6.3 design decision, not part of the store's public
/// surface.
const SQLITE_FILE: &str = "kb.sqlite";

/// Subdirectory under `data_dir` holding shard-prefixed asset bytes
/// (`<aa>/<asset_id>`). Mirrors design §6.3.
const ASSETS_SUBDIR: &str = "assets";

/// Length of the shard prefix: 2 hex chars → 256 buckets, plenty to keep
/// directory cardinality reasonable on workspaces with thousands of
/// assets without descending into hash-trees.
const ASSET_SHARD_LEN: usize = 2;

/// Bytes-per-MiB conversion. Used by the asset writer to compare
/// `byte_len` against `storage.copy_threshold_mb`.
const BYTES_PER_MIB: u64 = 1024 * 1024;

/// SQLite-backed kb store.
///
/// Construct via [`SqliteStore::open`], then call
/// [`SqliteStore::run_migrations`] to apply the bundled `V001__init.sql`
/// before any read/write call.
pub struct SqliteStore {
    /// Resolved absolute path to `data_dir`. Used by the asset writer.
    pub(crate) data_dir: PathBuf,
    /// Maximum asset size eligible for in-store copy; assets bigger than
    /// this are recorded as `reference` and read from their source path.
    pub(crate) copy_threshold_bytes: u64,
    /// Single mutexed connection — see module docs for rationale.
    pub(crate) conn: Mutex<Connection>,
}

impl SqliteStore {
    /// Open (or create) the SQLite file under `config.storage.data_dir`,
    /// apply pragmas (foreign_keys / WAL / synchronous=NORMAL /
    /// temp_store=MEMORY), and create parent directories as needed.
    /// **Does not run migrations** — call [`Self::run_migrations`] next.
    pub fn open(config: &kb_config::Config) -> Result<Self> {
        let data_dir = expand_data_dir(&config.storage.data_dir);
        std::fs::create_dir_all(&data_dir)
            .with_context(|| format!("create data_dir {}", data_dir.display()))?;
        let db_path = data_dir.join(SQLITE_FILE);

        let conn = Connection::open(&db_path)
            .with_context(|| format!("open sqlite at {}", db_path.display()))?;
        apply_pragmas(&conn)?;

        tracing::debug!(
            target: "kb-store-sqlite",
            data_dir = %data_dir.display(),
            db = %db_path.display(),
            "opened sqlite store"
        );

        Ok(Self {
            data_dir,
            copy_threshold_bytes: config.storage.copy_threshold_mb * BYTES_PER_MIB,
            conn: Mutex::new(conn),
        })
    }

    /// Apply all pending migrations bundled at compile time
    /// (`migrations/V001__init.sql` and any later additions).
    pub fn run_migrations(&self) -> Result<()> {
        let mut conn = self.conn.lock().expect("sqlite mutex poisoned");
        schema::runner()
            .run(&mut *conn)
            .map_err(|e| StoreError::Migration(e.to_string()))?;
        tracing::debug!(target: "kb-store-sqlite", "migrations applied");
        Ok(())
    }

    /// Persist a `RawAsset` *with its raw bytes*: row goes into `assets`,
    /// bytes go to `data_dir/assets/<aa>/<asset_id>` if `byte_len ≤
    /// copy_threshold_mb`, otherwise the row records the source URI's
    /// path and no copy is performed.
    ///
    /// In either branch, `blake3(bytes)` is recomputed and compared to
    /// `asset.checksum.0`. A mismatch returns
    /// `StoreError::Conflict` wrapped in `anyhow::Error`.
    pub fn put_asset_with_bytes(
        &self,
        asset: &kb_core::RawAsset,
        bytes: &[u8],
    ) -> Result<()> {
        // 1. Verify the caller's checksum matches what's actually on the
        // wire. A drift here means the bytes the parser saw and the bytes
        // we're about to durably store disagree — refuse persistence.
        let computed = blake3::hash(bytes).to_hex().to_string();
        if computed != asset.checksum.0 {
            return Err(StoreError::Conflict(format!(
                "checksum mismatch: asset {} declares {} but bytes hash to {}",
                asset.asset_id.0, asset.checksum.0, computed
            ))
            .into());
        }

        // 2. Decide copy vs. reference. The threshold compares the
        // declared `byte_len` (caller-vouched) rather than `bytes.len()`
        // because some sources stream and `byte_len` is authoritative.
        let (storage_kind, storage_path) = if asset.byte_len <= self.copy_threshold_bytes {
            let dest = self.assets_path_for(&asset.asset_id);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).with_context(|| {
                    format!("create asset shard dir {}", parent.display())
                })?;
            }
            std::fs::write(&dest, bytes)
                .with_context(|| format!("write asset bytes to {}", dest.display()))?;
            // Mirror §6.6: files 0o644.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = std::fs::metadata(&dest)?.permissions();
                perms.set_mode(0o644);
                std::fs::set_permissions(&dest, perms).with_context(|| {
                    format!("chmod 0o644 on {}", dest.display())
                })?;
            }
            ("copied", dest.to_string_lossy().into_owned())
        } else {
            // Reference: caller's source path is recorded verbatim. We
            // accept either a `File(path)` or `Kb(uri)` SourceUri; the
            // latter stores the raw `kb://...` string.
            let path = match &asset.source_uri {
                kb_core::SourceUri::File(p) => p.to_string_lossy().into_owned(),
                kb_core::SourceUri::Kb(u) => u.clone(),
            };
            ("reference", path)
        };

        // 3. UPSERT the assets row. A second `put_asset_with_bytes` for
        // the same asset_id (e.g. re-ingest) overwrites in place — the
        // row is uniquely keyed by asset_id and re-derived from the
        // RawAsset every time.
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        upsert_asset_row(&conn, asset, storage_kind, &storage_path)?;
        Ok(())
    }

    /// Compute the `data_dir/assets/<aa>/<asset_id>` path for an asset.
    /// `<aa>` is the first [`ASSET_SHARD_LEN`] hex chars of `asset_id`.
    pub(crate) fn assets_path_for(&self, asset_id: &kb_core::AssetId) -> PathBuf {
        let id = &asset_id.0;
        // Defensive: kb-core enforces 32 hex chars on AssetId construction
        // (`FromStr` validates). If a future code path bypasses that, we
        // fall back to the full id as the shard so we never panic on
        // slicing.
        let shard = if id.len() >= ASSET_SHARD_LEN {
            &id[..ASSET_SHARD_LEN]
        } else {
            id.as_str()
        };
        self.data_dir.join(ASSETS_SUBDIR).join(shard).join(id)
    }
}

/// UPSERT a row into `assets`. Used by both the `put_asset_with_bytes`
/// path (which has bytes + computed `storage_kind/path`) and the
/// `DocumentStore::put_asset` path (which only has the `RawAsset` and
/// reads `storage_kind/path` from `asset.stored`).
pub(crate) fn upsert_asset_row(
    conn: &Connection,
    asset: &kb_core::RawAsset,
    storage_kind: &str,
    storage_path: &str,
) -> Result<()> {
    let source_uri = match &asset.source_uri {
        kb_core::SourceUri::File(p) => format!("file://{}", p.to_string_lossy()),
        kb_core::SourceUri::Kb(u) => u.clone(),
    };
    let media_type = serde_json::to_string(&asset.media_type)
        .context("serialize media_type")?;
    let discovered_at = asset
        .discovered_at
        .format(&time::format_description::well_known::Rfc3339)
        .context("format discovered_at")?;

    conn.execute(
        "INSERT INTO assets (
            asset_id, source_uri, workspace_path, media_type, byte_len,
            checksum, storage_kind, storage_path, discovered_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(asset_id) DO UPDATE SET
            source_uri     = excluded.source_uri,
            workspace_path = excluded.workspace_path,
            media_type     = excluded.media_type,
            byte_len       = excluded.byte_len,
            checksum       = excluded.checksum,
            storage_kind   = excluded.storage_kind,
            storage_path   = excluded.storage_path,
            discovered_at  = excluded.discovered_at",
        rusqlite::params![
            asset.asset_id.0,
            source_uri,
            asset.workspace_path.0,
            media_type,
            asset.byte_len as i64,
            asset.checksum.0,
            storage_kind,
            storage_path,
            discovered_at,
        ],
    )
    .map_err(StoreError::from)?;
    Ok(())
}

/// Apply the design §5 / task-spec pragmas. Called once per connection.
/// Note: WAL is persistent (the journal-mode setting is sticky in the DB
/// header) but `foreign_keys`, `synchronous`, and `temp_store` are
/// per-connection — they MUST be re-applied on every open.
fn apply_pragmas(conn: &Connection) -> Result<()> {
    conn.pragma_update(None, "foreign_keys", "ON")?;
    // `journal_mode = WAL` returns the current mode as a row; use
    // `pragma_query_value` semantics via `query_row` to allow that.
    conn.query_row("PRAGMA journal_mode = WAL", [], |_| Ok(()))?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "temp_store", "MEMORY")?;
    Ok(())
}

/// Expand the placeholders / `~` / env-vars used by `Config::storage.data_dir`.
///
/// Supported substitutions, in order:
/// - `${XDG_DATA_HOME:-~/.local/share}` (and the bare `${XDG_DATA_HOME}`)
/// - leading `~` → `$HOME`
///
/// If neither produces an absolute path, the input is returned as-is
/// (relative paths are kept relative to the caller's CWD).
fn expand_data_dir(raw: &str) -> PathBuf {
    let mut s = raw.to_string();

    // ${XDG_DATA_HOME:-~/.local/share}: respect the env override, else
    // fall back to the suffix after `:-`.
    if let Some(start) = s.find("${XDG_DATA_HOME") {
        if let Some(rel_end) = s[start..].find('}') {
            let end = start + rel_end + 1; // include trailing '}'
            let inner = &s[start + 2..end - 1]; // strip ${ and }
            let replacement = match std::env::var("XDG_DATA_HOME") {
                Ok(v) if !v.is_empty() => v,
                _ => {
                    // inner is e.g. `XDG_DATA_HOME:-~/.local/share`.
                    if let Some((_, default)) = inner.split_once(":-") {
                        default.to_string()
                    } else {
                        // No default supplied; mimic Bash and yield "".
                        String::new()
                    }
                }
            };
            s.replace_range(start..end, &replacement);
        }
    }

    // ~ at the front → $HOME (or `dirs::home_dir`).
    if let Some(rest) = s.strip_prefix('~') {
        if let Some(home) = std::env::var_os("HOME").map(PathBuf::from).or_else(dirs_home_fallback)
        {
            return home.join(rest.trim_start_matches('/'));
        }
    }

    PathBuf::from(s)
}

/// Tiny shim to avoid pulling in the `dirs` crate as a direct dep — we
/// only fall back when `$HOME` is unset, which is exotic on the platforms
/// we target. Returns `None` so the caller keeps the literal `~`.
fn dirs_home_fallback() -> Option<PathBuf> {
    None
}

/// Returns the root of the assets shard tree (`data_dir/assets/`). Used
/// by tests; kept crate-private otherwise.
#[allow(dead_code)]
pub(crate) fn assets_root(data_dir: &Path) -> PathBuf {
    data_dir.join(ASSETS_SUBDIR)
}
