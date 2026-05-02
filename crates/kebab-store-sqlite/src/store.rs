//! `SqliteStore` — open + run_migrations + asset writer.
//!
//! The store wraps a single `rusqlite::Connection` behind a
//! `std::sync::Mutex` so the public trait impls (which take `&self`) can
//! still issue mutating SQL. Concurrency is intentionally coarse for P1;
//! later phases can swap to a connection pool if measurement shows the
//! mutex on the hot path.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};

use anyhow::{Context, Result};
use rusqlite::Connection;

use crate::error::StoreError;
use crate::schema;

/// Monotonic counter used to namespace per-process temp file names so
/// concurrent `put_asset_with_bytes` calls in the same millisecond cannot
/// collide on `<final>.tmp.<pid>.<n>`.
static TEMP_SUFFIX_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Length, in hex chars, of a valid `kebab_core::AssetId`. blake3 first-half
/// truncated, mirrored from `kb-core`'s newtype invariant.
const ASSET_ID_HEX_LEN: usize = 32;

/// Default file name under `config.storage.data_dir`. Kept private — the
/// path layout is a §6.3 design decision, not part of the store's public
/// surface.
const SQLITE_FILE: &str = "kebab.sqlite";

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
    pub fn open(config: &kebab_config::Config) -> Result<Self> {
        let data_dir = kebab_config::expand_path(&config.storage.data_dir, "");
        std::fs::create_dir_all(&data_dir)
            .with_context(|| format!("create data_dir {}", data_dir.display()))?;
        let db_path = data_dir.join(SQLITE_FILE);

        let conn = Connection::open(&db_path)
            .with_context(|| format!("open sqlite at {}", db_path.display()))?;
        apply_pragmas(&conn)?;

        tracing::debug!(
            target: "kebab-store-sqlite",
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
        let mut conn = self.lock_conn();
        schema::runner()
            .run(&mut *conn)
            .map_err(|e| StoreError::Migration(e.to_string()))?;
        tracing::debug!(target: "kebab-store-sqlite", "migrations applied");
        Ok(())
    }

    /// Acquire the connection mutex, recovering from poison.
    ///
    /// Poisoning here means a previous holder panicked while holding the
    /// guard. The active rusqlite transaction (if any) was rolled back by
    /// the `Transaction` `Drop` impl, so the connection state is still
    /// safe to reuse — we simply unwrap the inner guard rather than
    /// propagate the panic to every subsequent call.
    pub(crate) fn lock_conn(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Read-only borrow of the connection.
    ///
    /// Provided so sibling crates (e.g. `kb-search`) can run SELECTs
    /// against the schema owned by this crate without re-opening the
    /// SQLite file. Callers MUST treat the returned `Connection` as
    /// read-only — issuing mutating SQL (INSERT / UPDATE / DELETE / DDL)
    /// through this guard bypasses the per-document transaction discipline
    /// (`put_*` methods) and the FTS5 backfill helpers that the store
    /// layer enforces. Mutating callers must use `kb-store-sqlite`'s own
    /// public write methods instead.
    ///
    /// Poisoning is recovered the same way as [`Self::lock_conn`].
    pub fn read_conn(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|p| p.into_inner())
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
        asset: &kebab_core::RawAsset,
        bytes: &[u8],
    ) -> Result<()> {
        // 0. Validate the AssetId shape before any I/O. `kebab_core::AssetId`
        // is a `pub String` newtype: `FromStr` enforces the 32-hex-char
        // invariant, but a hand-constructed `AssetId("../etc/passwd…")`
        // can bypass that and reach `assets_path_for`. Refuse such IDs at
        // the store boundary to keep shard-dir slicing safe.
        validate_asset_id(&asset.asset_id)?;

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
        if asset.byte_len <= self.copy_threshold_bytes {
            // Copy mode. To prevent file orphans on UPSERT failure we use
            // the temp-file + atomic-rename pattern:
            //   (a) write bytes to `<final>.tmp.<pid>.<counter>`
            //   (b) fsync the temp file
            //   (c) UPSERT the row
            //   (d) on UPSERT success: rename temp → final (atomic on
            //       same fs)
            //   (e) on any failure between (a) and (d): best-effort delete
            //       of the temp file so we never leak bytes on disk.
            let dest = self.assets_path_for(&asset.asset_id);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).with_context(|| {
                    format!("create asset shard dir {}", parent.display())
                })?;
            }
            let temp_path = temp_path_for(&dest);
            // Inline closure so any `?` in (a)/(b) cleans up the temp
            // file before bubbling out.
            let write_and_upsert = || -> Result<()> {
                {
                    let mut f = std::fs::File::create(&temp_path).with_context(|| {
                        format!("create temp asset file {}", temp_path.display())
                    })?;
                    use std::io::Write;
                    f.write_all(bytes).with_context(|| {
                        format!("write asset bytes to {}", temp_path.display())
                    })?;
                    f.sync_all().with_context(|| {
                        format!("fsync temp asset file {}", temp_path.display())
                    })?;
                }
                // Mirror §6.6: files 0o644.
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let mut perms = std::fs::metadata(&temp_path)?.permissions();
                    perms.set_mode(0o644);
                    std::fs::set_permissions(&temp_path, perms).with_context(|| {
                        format!("chmod 0o644 on {}", temp_path.display())
                    })?;
                }
                // UPSERT the row first; only after a successful row write
                // do we publish the file via rename. A second
                // `put_asset_with_bytes` for the same asset_id overwrites
                // in place.
                {
                    let conn = self.lock_conn();
                    upsert_asset_row(
                        &conn,
                        asset,
                        "copied",
                        &dest.to_string_lossy(),
                    )?;
                }
                std::fs::rename(&temp_path, &dest).with_context(|| {
                    format!(
                        "atomic rename {} -> {}",
                        temp_path.display(),
                        dest.display()
                    )
                })?;
                Ok(())
            };
            match write_and_upsert() {
                Ok(()) => Ok(()),
                Err(e) => {
                    // Best-effort cleanup; ignore errors so the original
                    // failure (likely the more useful one) propagates.
                    let _ = std::fs::remove_file(&temp_path);
                    Err(e)
                }
            }
        } else {
            // Reference: caller's source path is recorded verbatim. We
            // accept either a `File(path)` or `Kb(uri)` SourceUri; the
            // latter stores the raw `kb://...` string. No file I/O ⇒ no
            // orphan risk; just UPSERT the row.
            let storage_path = match &asset.source_uri {
                kebab_core::SourceUri::File(p) => p.to_string_lossy().into_owned(),
                kebab_core::SourceUri::Kb(u) => u.clone(),
            };
            let conn = self.lock_conn();
            upsert_asset_row(&conn, asset, "reference", &storage_path)?;
            Ok(())
        }
    }

    /// Compute the `data_dir/assets/<aa>/<asset_id>` path for an asset.
    /// `<aa>` is the first [`ASSET_SHARD_LEN`] hex chars of `asset_id`.
    ///
    /// Callers that build paths from caller-controlled IDs MUST first
    /// invoke [`validate_asset_id`] (already enforced at every store
    /// entry that takes a `RawAsset`). The `id.len() >= ASSET_SHARD_LEN`
    /// guard below is a defense-in-depth fallback only.
    pub(crate) fn assets_path_for(&self, asset_id: &kebab_core::AssetId) -> PathBuf {
        let id = &asset_id.0;
        let shard = if id.len() >= ASSET_SHARD_LEN {
            &id[..ASSET_SHARD_LEN]
        } else {
            id.as_str()
        };
        self.data_dir.join(ASSETS_SUBDIR).join(shard).join(id)
    }
}

/// Reject an `AssetId` whose shape would let a malicious caller escape
/// the `data_dir/assets/<aa>/` shard tree. `kebab_core::AssetId(pub String)`
/// permits hand-construction, so any function that turns an `AssetId`
/// into a filesystem path must call this first.
pub(crate) fn validate_asset_id(asset_id: &kebab_core::AssetId) -> Result<()> {
    if asset_id.0.len() != ASSET_ID_HEX_LEN
        || !asset_id.0.bytes().all(|b| b.is_ascii_hexdigit())
    {
        anyhow::bail!(
            "invalid AssetId shape (expected {} ASCII hex chars): {:?}",
            ASSET_ID_HEX_LEN,
            asset_id.0
        );
    }
    Ok(())
}

/// Compute a per-call temp-file path next to `dest` that is unlikely to
/// collide with any other in-flight writer (process pid + monotonic
/// counter). The temp file lives in the same parent directory so the
/// final `rename` is an atomic same-filesystem rename on POSIX.
fn temp_path_for(dest: &Path) -> PathBuf {
    let pid = std::process::id();
    let n = TEMP_SUFFIX_COUNTER.fetch_add(1, Ordering::Relaxed);
    let parent = dest.parent().unwrap_or_else(|| Path::new("."));
    let file_name = dest
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "asset".to_string());
    parent.join(format!("{file_name}.tmp.{pid}.{n}"))
}

/// UPSERT a row into `assets`. Used by both the `put_asset_with_bytes`
/// path (which has bytes + computed `storage_kind/path`) and the
/// `DocumentStore::put_asset` path (which only has the `RawAsset` and
/// reads `storage_kind/path` from `asset.stored`).
pub(crate) fn upsert_asset_row(
    conn: &Connection,
    asset: &kebab_core::RawAsset,
    storage_kind: &str,
    storage_path: &str,
) -> Result<()> {
    let source_uri = match &asset.source_uri {
        kebab_core::SourceUri::File(p) => format!("file://{}", p.to_string_lossy()),
        kebab_core::SourceUri::Kb(u) => u.clone(),
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

