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
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};

use crate::error::StoreError;
use crate::schema;

/// Signal: SQLite database file does not exist, or schema_version does
/// not match the binary's expectation.
///
/// Distinct from generic I/O / SQL errors so kebab-cli can surface
/// `code: "not_indexed"` with a hint to run `kebab init` / `kebab ingest`.
#[derive(Debug, thiserror::Error)]
#[error("not indexed: expected={expected}, found={found:?}")]
pub struct NotIndexed {
    pub expected: String,
    /// When the DB file exists but the schema is incompatible, this holds
    /// the highest applied migration version string (e.g. `"V005"`).
    /// `None` means the file was absent entirely (current Task 3 behavior;
    /// schema-mismatch wrapping is a deferred follow-up).
    pub found: Option<String>,
}

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
    /// Open an existing SQLite DB at `path`.
    ///
    /// Unlike [`Self::open`], this does NOT create the file — if it is
    /// missing, returns a [`NotIndexed`] signal suitable for `error.v1`
    /// translation. Opens read-write to support WAL pragmas; callers should
    /// not issue mutations through this connection — use [`Self::open`] for
    /// ingest paths.
    ///
    /// **Does not run migrations** — call [`Self::run_migrations`] next if
    /// you need the schema initialised.
    pub fn open_existing(path: &std::path::Path) -> anyhow::Result<Self> {
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_URI,
        )
        .map_err(|_| {
            anyhow::Error::new(NotIndexed {
                expected: path.to_string_lossy().to_string(),
                found: None,
            })
        })?;
        apply_pragmas(&conn)?;

        let data_dir = path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .to_path_buf();

        tracing::debug!(
            target: "kebab-store-sqlite",
            db = %path.display(),
            "opened existing sqlite store"
        );

        Ok(Self {
            data_dir,
            copy_threshold_bytes: 0,
            conn: Mutex::new(conn),
        })
    }

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
        self.conn
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
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
        self.conn
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Persist a `RawAsset` *with its raw bytes*: row goes into `assets`,
    /// bytes go to `data_dir/assets/<aa>/<asset_id>` if `byte_len ≤
    /// copy_threshold_mb`, otherwise the row records the source URI's
    /// path and no copy is performed.
    ///
    /// In either branch, `blake3(bytes)` is recomputed and compared to
    /// `asset.checksum.0`. A mismatch returns
    /// `StoreError::Conflict` wrapped in `anyhow::Error`.
    pub fn put_asset_with_bytes(&self, asset: &kebab_core::RawAsset, bytes: &[u8]) -> Result<()> {
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
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("create asset shard dir {}", parent.display()))?;
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
                    f.write_all(bytes)
                        .with_context(|| format!("write asset bytes to {}", temp_path.display()))?;
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
                    std::fs::set_permissions(&temp_path, perms)
                        .with_context(|| format!("chmod 0o644 on {}", temp_path.display()))?;
                }
                // UPSERT the row first; only after a successful row write
                // do we publish the file via rename. A second
                // `put_asset_with_bytes` for the same asset_id overwrites
                // in place.
                {
                    let conn = self.lock_conn();
                    purge_orphan_at_workspace_path(
                        &conn,
                        &asset.workspace_path.0,
                        &asset.asset_id.0,
                    )?;
                    upsert_asset_row(&conn, asset, "copied", &dest.to_string_lossy())?;
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
            purge_orphan_at_workspace_path(&conn, &asset.workspace_path.0, &asset.asset_id.0)?;
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
    if asset_id.0.len() != ASSET_ID_HEX_LEN || !asset_id.0.bytes().all(|b| b.is_ascii_hexdigit()) {
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
        .map_or_else(|| "asset".to_string(), |s| s.to_string_lossy().into_owned());
    parent.join(format!("{file_name}.tmp.{pid}.{n}"))
}

impl SqliteStore {
    /// p9-fb-19: read the persisted `corpus_revision` from the `kv`
    /// table. Returns `0` if the row is missing (not migrated yet) or
    /// unparseable — defensive: callers use the value as a cache-key
    /// salt, never as an authority.
    pub fn corpus_revision(&self) -> u64 {
        let conn = self.read_conn();
        let row: rusqlite::Result<String> = conn.query_row(
            "SELECT value FROM kv WHERE key = 'corpus_revision'",
            [],
            |r| r.get(0),
        );
        match row {
            Ok(s) => s.parse().unwrap_or(0),
            Err(rusqlite::Error::QueryReturnedNoRows) => 0,
            Err(e) => {
                tracing::warn!(
                    target: "kebab-store-sqlite",
                    error = %e,
                    "kv['corpus_revision'] read failed; defaulting to 0"
                );
                0
            }
        }
    }

    /// p9-fb-19: monotonically bump `corpus_revision` by one and
    /// return the new value. Called by every `kebab-app::ingest`
    /// path after a successful commit (any `new` / `updated`).
    /// Atomic via SQLite's `UPDATE ... SET value = CAST(value AS
    /// INTEGER) + 1` — no read-modify-write race.
    pub fn bump_corpus_revision(&self) -> Result<u64> {
        let conn = self.lock_conn();
        // INSERT-OR-IGNORE first to handle a fresh DB where the
        // V004 seed hasn't run yet (paranoia — the migration always
        // seeds, but SqliteStore's contract is "one method works
        // even if the constructor was unusual"). Then bump.
        conn.execute(
            "INSERT OR IGNORE INTO kv (key, value) VALUES ('corpus_revision', '0')",
            [],
        )
        .map_err(StoreError::from)?;
        conn.execute(
            "UPDATE kv SET value = CAST(CAST(value AS INTEGER) + 1 AS TEXT) \
             WHERE key = 'corpus_revision'",
            [],
        )
        .map_err(StoreError::from)?;
        let new_val: String = conn
            .query_row(
                "SELECT value FROM kv WHERE key = 'corpus_revision'",
                [],
                |r| r.get(0),
            )
            .map_err(StoreError::from)?;
        Ok(new_val.parse().unwrap_or(0))
    }

    /// SELECT every `chunks.chunk_id` whose owning document points at a
    /// stale `asset_id` for `workspace_path` (i.e. the file's bytes have
    /// changed since the last ingest, producing a brand-new
    /// `asset_id`).
    ///
    /// Called by `kebab-app::ingest_one_*_asset` BEFORE
    /// `put_asset_with_bytes` so the caller can hand the IDs to
    /// `VectorStore::delete_by_chunk_ids`. After the SQLite cleanup
    /// runs (CASCADE on `documents` → `chunks`) the same chunk_ids
    /// would be unreadable. Returns an empty Vec when no stale row
    /// exists at `workspace_path`.
    ///
    /// Read-only — does not mutate. The actual sweep happens inside
    /// `purge_orphan_at_workspace_path` further down the pipeline.
    pub fn stale_chunk_ids_at(
        &self,
        workspace_path: &str,
        new_asset_id: &str,
    ) -> Result<Vec<kebab_core::ChunkId>> {
        let conn = self.lock_conn();
        let mut stmt = conn
            .prepare(
                "SELECT c.chunk_id
                 FROM chunks c
                 INNER JOIN documents d ON c.doc_id = d.doc_id
                 INNER JOIN assets a ON d.asset_id = a.asset_id
                 WHERE a.workspace_path = ?1 AND a.asset_id != ?2",
            )
            .map_err(StoreError::from)?;
        let rows = stmt
            .query_map(params![workspace_path, new_asset_id], |row| {
                row.get::<_, String>(0)
            })
            .map_err(StoreError::from)?;
        let mut out: Vec<kebab_core::ChunkId> = Vec::new();
        for row in rows {
            let id = row.map_err(StoreError::from)?;
            out.push(kebab_core::ChunkId(id));
        }
        Ok(out)
    }

    /// v0.17.0 PR-B: sister of [`Self::stale_chunk_ids_at`] for the
    /// `parser_version` bump cascade. When `doc_id` depends on
    /// `parser_version` (design §9) and an extractor ships a new
    /// `PARSER_VERSION`, the next ingest computes a fresh `doc_id` for
    /// the *same* `(workspace_path, asset_id)` pair. The existing
    /// asset_id-keyed [`Self::stale_chunk_ids_at`] does NOT fire (same
    /// asset), so the legacy `chunks` rows and their LanceDB shadows
    /// would orphan. This helper queries by `workspace_path` instead,
    /// excluding the freshly-computed `keep_doc_id` so a re-entry
    /// during the same ingest doesn't re-sweep the new row.
    ///
    /// Caller usage: pass the *new* `doc_id` if known; pass an empty
    /// string when called before the new INSERT (the case in
    /// `try_skip_unchanged`) — all existing docs at `workspace_path`
    /// are then collected as stale.
    pub fn stale_chunk_ids_for_workspace_path_except_doc_id(
        &self,
        workspace_path: &str,
        keep_doc_id: &str,
    ) -> Result<Vec<kebab_core::ChunkId>> {
        let conn = self.lock_conn();
        let mut stmt = conn
            .prepare(
                "SELECT c.chunk_id
                 FROM chunks c
                 INNER JOIN documents d ON c.doc_id = d.doc_id
                 WHERE d.workspace_path = ?1 AND d.doc_id != ?2",
            )
            .map_err(StoreError::from)?;
        let rows = stmt
            .query_map(params![workspace_path, keep_doc_id], |row| {
                row.get::<_, String>(0)
            })
            .map_err(StoreError::from)?;
        let mut out: Vec<kebab_core::ChunkId> = Vec::new();
        for row in rows {
            let id = row.map_err(StoreError::from)?;
            out.push(kebab_core::ChunkId(id));
        }
        Ok(out)
    }

    /// V007 → V009 업그레이드 시 기존 chunks 의 `tokenized_korean_text` 가 NULL — 이
    /// 메서드가 NULL 인 row 를 batch 로 읽어 `tokenize` 콜백으로 형태소 분해 후 UPDATE.
    /// chunks_au trigger 가 chunks_fts 를 자동 재-index.
    ///
    /// - `tokenize`: `kebab_chunk::tokenize_korean_morphological` 등 `&str → Option<String>`.
    ///   `None` 반환 시 row 를 skip (UPDATE 없음).
    /// - `progress`: `(done, total)` 콜백. 1000 row 마다 발화.
    /// - 반환값: lindera Some 으로 UPDATE 된 row 수 (idempotent — 이미 채워진 row 는 0).
    /// - 실패 시 App open 을 block 하지 않도록 호출자가 `unwrap_or_else` 로 감쌀 것.
    pub fn backfill_tokenized_korean_text<F, T>(&self, progress: F, tokenize: T) -> Result<u64>
    where
        F: Fn(u64, u64),
        T: Fn(&str) -> Option<String>,
    {
        // 1. NULL 후보 수집.
        let rows: Vec<(String, String)> = {
            let conn = self.lock_conn();
            let mut stmt = conn
                .prepare(
                    "SELECT chunk_id, text FROM chunks \
                     WHERE tokenized_korean_text IS NULL \
                     ORDER BY chunk_id",
                )
                .map_err(StoreError::from)?;
            let iter = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(StoreError::from)?;
            let mut out = Vec::new();
            for r in iter {
                out.push(r.map_err(StoreError::from)?);
            }
            out
        };

        let total = rows.len() as u64;
        let mut updated: u64 = 0;

        // 2. 1000 row 마다 transaction 으로 batch UPDATE.
        for chunk in rows.chunks(1000) {
            let conn = self.lock_conn();
            let tx = conn.unchecked_transaction().map_err(StoreError::from)?;
            for (chunk_id, text) in chunk {
                if let Some(tokenized) = tokenize(text) {
                    tx.execute(
                        "UPDATE chunks SET tokenized_korean_text = ?1 WHERE chunk_id = ?2",
                        params![tokenized, chunk_id],
                    )
                    .map_err(StoreError::from)?;
                    updated += 1;
                }
            }
            tx.commit().map_err(StoreError::from)?;
            progress(updated, total);
        }

        Ok(updated)
    }

    /// v0.17.0 PR-B: sweep the SQLite document chain (`documents` →
    /// `blocks` / `chunks` / `embedding_records` via CASCADE) for every
    /// row at `workspace_path` whose `doc_id` differs from `keep_doc_id`.
    /// Pair with [`Self::stale_chunk_ids_for_workspace_path_except_doc_id`]
    /// — caller fetches the chunk_ids first, hands them to
    /// `VectorStore::delete_by_chunk_ids`, then calls this sweep.
    /// `assets` row is preserved (same bytes, same asset_id — only the
    /// derived `doc_id` changed).
    ///
    /// `keep_doc_id = ""` deletes every doc at `workspace_path`
    /// (semantics mirror the sister helper above — used by
    /// `try_skip_unchanged` before the new INSERT exists).
    pub fn purge_document_at_workspace_path_except_doc_id(
        &self,
        workspace_path: &str,
        keep_doc_id: &str,
    ) -> Result<()> {
        let conn = self.lock_conn();
        conn.execute(
            "DELETE FROM documents WHERE workspace_path = ?1 AND doc_id != ?2",
            params![workspace_path, keep_doc_id],
        )
        .map_err(StoreError::from)?;
        Ok(())
    }
}

/// Sweep stale `assets` + `documents` + downstream rows when the file
/// at `workspace_path` is being re-ingested with bytes that produce a
/// **different** `asset_id` (i.e. the file was edited).
///
/// Why this exists (HOTFIXES 2026-05-02 P7-3): `idx_assets_workspace_path`
/// is a UNIQUE index. The original `upsert_asset_row` only handles
/// `ON CONFLICT(asset_id)`, so a brand-new `asset_id` colliding on
/// `workspace_path` raises `Error code 2067` and the ingest fails. This
/// helper does the cleanup `ON CONFLICT(workspace_path)` would have done
/// if SQLite let UPSERT target two indexes at once.
///
/// Order matters:
/// 1. `documents.asset_id` is `ON DELETE RESTRICT`, so we must drop the
///    old `documents` rows first. CASCADE on documents → blocks /
///    chunks / embedding_records sweeps the dependent rows in the same
///    statement.
/// 2. Then DELETE the stale `assets` row, freeing the
///    `workspace_path` slot for the new one.
/// 3. If the stale asset was stored in `copied` mode, best-effort
///    delete the on-disk byte file at `storage_path` so the data dir
///    doesn't accumulate orphans across edits.
///
/// **Vector store cleanup**: `embedding_records.chunk_id` CASCADE
/// clears the SQLite side, but the LanceDB rows live in a separate
/// store. The caller (`kebab-app::ingest_one_*_asset`) is responsible
/// for fetching `stale_chunk_ids_at` BEFORE this purge runs and
/// calling `VectorStore::delete_by_chunk_ids` on those IDs. The
/// follow-up PR for HOTFIXES 2026-05-02 P7-3 wires this in.
pub(crate) fn purge_orphan_at_workspace_path(
    conn: &Connection,
    workspace_path: &str,
    new_asset_id: &str,
) -> Result<()> {
    let stale: Option<(String, String, String)> = conn
        .query_row(
            "SELECT asset_id, storage_kind, storage_path
             FROM assets
             WHERE workspace_path = ? AND asset_id != ?",
            params![workspace_path, new_asset_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(StoreError::from)?;

    let Some((stale_asset_id, storage_kind, storage_path)) = stale else {
        return Ok(());
    };

    // CASCADE 제거(V011) 대체: 이 asset 의 문서 chunk 임베딩 레코드를 명시 정리.
    // 원본 + sentinel({id}#alias) 둘 다. 별칭 dense 벡터는 chunks FK 가 없어
    // documents→chunks CASCADE 로 자동 정리되지 않으므로 chunks 가 살아있는 동안
    // 직접 지운다. 설계 spec 2026-05-30-dense-alias-vectors-design.md §3.5-2.
    conn.execute(
        "DELETE FROM embedding_records WHERE chunk_id IN \
         (SELECT chunk_id FROM chunks WHERE doc_id IN \
            (SELECT doc_id FROM documents WHERE asset_id = ?1) \
          UNION SELECT chunk_id || '#alias' FROM chunks WHERE doc_id IN \
            (SELECT doc_id FROM documents WHERE asset_id = ?1))",
        params![stale_asset_id],
    )
    .map_err(StoreError::from)?;
    // documents → blocks / chunks via CASCADE.
    conn.execute(
        "DELETE FROM documents WHERE asset_id = ?",
        params![stale_asset_id],
    )
    .map_err(StoreError::from)?;
    conn.execute(
        "DELETE FROM assets WHERE asset_id = ?",
        params![stale_asset_id],
    )
    .map_err(StoreError::from)?;

    if storage_kind == "copied" {
        let _ = std::fs::remove_file(&storage_path);
    }

    tracing::debug!(
        target: "kebab-store-sqlite",
        workspace_path = %workspace_path,
        stale_asset_id = %stale_asset_id,
        new_asset_id = %new_asset_id,
        "purged stale asset (file edited; bytes changed)"
    );
    Ok(())
}

/// Purge all stored data for a document whose on-disk file has been
/// deleted (as opposed to content-changed, which is handled by
/// `purge_orphan_at_workspace_path`).
///
/// Returns the `chunk_id`s that were associated with the document so
/// the caller can issue a matching `VectorStore::delete_by_chunk_ids`
/// on the LanceDB side.
///
/// Deletion order:
/// 1. Collect chunk_ids (before cascade removes them).
/// 2. DELETE the `documents` row → CASCADE clears `blocks`, `chunks`,
///    `embedding_records`.
/// 3. DELETE the `assets` row **only if no other document still
///    references it** (twin-file protection — `assets` can be shared
///    across identical-content files via the blake3 PK).
/// 4. If the asset was `storage_kind = 'copied'`, best-effort delete
///    the on-disk byte file at `storage_path`.
///
/// Returns `Ok(vec![])` when no document exists at `workspace_path`
/// (idempotent — caller doesn't need to pre-check).
pub fn purge_deleted_workspace_path(
    store: &SqliteStore,
    workspace_path: &kebab_core::WorkspacePath,
) -> anyhow::Result<Vec<kebab_core::ChunkId>> {
    let conn = store.lock_conn();

    // Look up the document + its asset_id.
    let doc_row: Option<(String, String)> = conn
        .query_row(
            "SELECT doc_id, asset_id FROM documents WHERE workspace_path = ?",
            rusqlite::params![workspace_path.0],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(StoreError::from)?;

    let Some((doc_id, asset_id)) = doc_row else {
        return Ok(Vec::new());
    };

    // 1. Collect chunk_ids before CASCADE removes them.
    let mut stmt = conn
        .prepare("SELECT chunk_id FROM chunks WHERE doc_id = ?")
        .map_err(StoreError::from)?;
    let rows = stmt
        .query_map(rusqlite::params![doc_id], |r| r.get::<_, String>(0))
        .map_err(StoreError::from)?;
    let chunk_ids: Vec<kebab_core::ChunkId> = rows
        .map(|r| r.map(kebab_core::ChunkId))
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(StoreError::from)?;
    drop(stmt);

    // 1b. CASCADE 제거(V011) 대체: chunk 임베딩 레코드를 명시 정리(원본 +
    //     sentinel {id}#alias). 별칭 dense 벡터는 chunks FK 가 없어
    //     documents→chunks CASCADE 로 자동 정리되지 않는다. chunks 가
    //     살아있는 동안(2번 DELETE 직전) 실행. spec §3.5-2.
    conn.execute(
        "DELETE FROM embedding_records WHERE chunk_id IN \
         (SELECT chunk_id FROM chunks WHERE doc_id = ?1 \
          UNION SELECT chunk_id || '#alias' FROM chunks WHERE doc_id = ?1)",
        rusqlite::params![doc_id],
    )
    .map_err(StoreError::from)?;

    // 2. DELETE the document row (CASCADE clears blocks / chunks via the
    //    FK constraints in V001; embedding_records handled above).
    conn.execute(
        "DELETE FROM documents WHERE doc_id = ?",
        rusqlite::params![doc_id],
    )
    .map_err(StoreError::from)?;

    // 3. Delete the asset row only when no other document still
    //    references it (twin-file safety: two files with identical
    //    bytes share a single asset row via the blake3 PK).
    let remaining_refs: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM documents WHERE asset_id = ?",
            rusqlite::params![asset_id],
            |r| r.get(0),
        )
        .map_err(StoreError::from)?;

    if remaining_refs == 0 {
        // 4. Capture storage details before deleting the row.
        let asset_storage: Option<(String, String)> = conn
            .query_row(
                "SELECT storage_kind, storage_path FROM assets WHERE asset_id = ?",
                rusqlite::params![asset_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(StoreError::from)?;

        conn.execute(
            "DELETE FROM assets WHERE asset_id = ?",
            rusqlite::params![asset_id],
        )
        .map_err(StoreError::from)?;

        // 5. Best-effort: remove the on-disk copied asset file.
        if let Some((storage_kind, storage_path)) = asset_storage {
            if storage_kind == "copied" {
                let _ = std::fs::remove_file(&storage_path);
            }
        }
    }

    tracing::debug!(
        target: "kebab-store-sqlite",
        workspace_path = %workspace_path.0,
        doc_id = %doc_id,
        chunk_count = chunk_ids.len(),
        "purged deleted-file document from store"
    );

    Ok(chunk_ids)
}

/// UPSERT a row into `assets`. Used by both the `put_asset_with_bytes`
/// path (which has bytes + computed `storage_kind/path`) and the
/// `DocumentStore::put_asset` path (which only has the `RawAsset` and
/// reads `storage_kind/path` from `asset.stored`).
///
/// **`assets.workspace_path` is "last-registered path" semantics for
/// twin files** (two source files with identical content share one
/// `assets` row keyed on `asset_id = blake3(content)`). Each ingest
/// of either twin overwrites `workspace_path` with whichever path was
/// seen most recently — this is intentional and correct after PR #146
/// made `try_skip_unchanged` document-centric (uses
/// `get_document_by_workspace_path`, not `get_asset_by_workspace_path`)
/// and PR #149 made `reset --orphans-only` document-centric too.
/// Do NOT "fix" the flip-flop by adding a UNIQUE constraint on
/// `workspace_path` in the `assets` table — twin de-dup is load-bearing.
/// When you need media_type for a known document, use the 2-step lookup
/// `get_document_by_workspace_path` → `doc.source_asset_id` →
/// `get_asset(asset_id)` so the result is twin-safe.
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
    let media_type = serde_json::to_string(&asset.media_type).context("serialize media_type")?;
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

/// p9-fb-27: aggregate counts for `SchemaV1.stats` block.
///
/// Returned by [`SqliteStore::count_summary`] and consumed by
/// `kebab-app::schema_with_config` to populate the `stats` sub-object of the
/// `schema.v1` wire record.
#[derive(Debug, Clone)]
pub struct CountSummary {
    pub doc_count: u64,
    pub chunk_count: u64,
    pub asset_count: u64,
    /// ISO-8601 timestamp of the most-recently updated document row, or
    /// `None` when the store is empty.
    pub last_ingest_at: Option<String>,
    /// p9-fb-37: per-media-kind doc count (5 keys, zero-padded).
    pub media_breakdown: std::collections::BTreeMap<String, u64>,
    /// p9-fb-37: per-language doc count, NULL keyed as `"null"`.
    pub lang_breakdown: std::collections::BTreeMap<String, u64>,
    /// p9-fb-37: docs whose `updated_at < now - threshold_days`. 0 when threshold=0.
    pub stale_doc_count: u64,
}

impl SqliteStore {
    /// Return aggregate counts from the three primary tables plus the
    /// most-recent `documents.updated_at` timestamp.
    ///
    /// Uses `read_conn()` (no mutations) — mirrors the pattern used by
    /// Shared helper: counts and breakdowns in a single pass with given threshold.
    fn count_summary_inner(&self, threshold_days: u64) -> anyhow::Result<CountSummary> {
        use anyhow::Context;
        use rusqlite::OptionalExtension;

        let conn = self.read_conn();

        let doc_count: u64 = conn
            .query_row("SELECT COUNT(*) FROM documents", [], |r| r.get(0))
            .context("count documents")?;
        let chunk_count: u64 = conn
            .query_row("SELECT COUNT(*) FROM chunks", [], |r| r.get(0))
            .context("count chunks")?;
        let asset_count: u64 = conn
            .query_row("SELECT COUNT(*) FROM assets", [], |r| r.get(0))
            .context("count assets")?;
        let last_ingest_at: Option<String> = conn
            .query_row("SELECT MAX(updated_at) FROM documents", [], |r| r.get(0))
            .optional()
            .context("max updated_at")?
            .flatten();

        let bd = crate::stats_ext::breakdowns(&conn, threshold_days).context("breakdowns")?;

        Ok(CountSummary {
            doc_count,
            chunk_count,
            asset_count,
            last_ingest_at,
            media_breakdown: bd.media,
            lang_breakdown: bd.lang,
            stale_doc_count: bd.stale_doc_count,
        })
    }

    /// [`Self::corpus_revision`].
    pub fn count_summary(&self) -> anyhow::Result<CountSummary> {
        // p9-fb-37: default uses threshold_days=0 (matches fb-32 disable
        // semantics). Callers that need real stale_doc_count call
        // count_summary_with_threshold.
        self.count_summary_inner(0)
    }

    /// p9-fb-37: variant that honors `config.search.stale_threshold_days`.
    /// Callers who need a meaningful `stale_doc_count` (e.g. `kebab schema`)
    /// pass the configured threshold; the older `count_summary` returns 0.
    pub fn count_summary_with_threshold(
        &self,
        threshold_days: u64,
    ) -> anyhow::Result<CountSummary> {
        self.count_summary_inner(threshold_days)
    }

    /// p10-1A-2: per-code-language doc count for `schema.v1`.
    ///
    /// Reads `metadata_json->'$.code_lang'`, groups by the value, and
    /// skips rows where `code_lang` is NULL (i.e. non-code documents).
    /// Returns `BTreeMap<String, u32>` — key is the canonical lowercase
    /// language identifier (e.g. `"rust"`), value is the doc count.
    pub fn code_lang_breakdown(&self) -> anyhow::Result<std::collections::BTreeMap<String, u32>> {
        use anyhow::Context;
        let conn = self.read_conn();
        let mut stmt = conn
            .prepare(
                "SELECT json_extract(metadata_json, '$.code_lang') AS cl, COUNT(*) \
                 FROM documents \
                 WHERE cl IS NOT NULL \
                 GROUP BY cl",
            )
            .context("prepare code_lang_breakdown")?;
        let rows = stmt
            .query_map([], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u32))
            })
            .context("query code_lang_breakdown")?;
        let mut out = std::collections::BTreeMap::new();
        for row in rows {
            let (k, v) = row.context("read code_lang_breakdown row")?;
            out.insert(k, v);
        }
        Ok(out)
    }

    /// v0.17.0 PR-C: per-code-language **chunk** count for
    /// `schema.v1.stats`. Companion to [`Self::code_lang_breakdown`] —
    /// that one returns *document* counts. Stats observers wanting
    /// indexing-pressure granularity (a single PDF spec → 200 chunks,
    /// vs a single Rust file → 5 chunks) need the chunk-level view.
    ///
    /// SQL joins `chunks → documents`, reads
    /// `metadata_json->'$.code_lang'` on the doc side, groups by the
    /// language, and skips rows where `code_lang IS NULL`. Returns
    /// `BTreeMap<String, u32>` mirroring the doc-count helper above
    /// so callers can serialize both with the same shape.
    pub fn code_lang_chunk_breakdown(
        &self,
    ) -> anyhow::Result<std::collections::BTreeMap<String, u32>> {
        use anyhow::Context;
        let conn = self.read_conn();
        let mut stmt = conn
            .prepare(
                "SELECT json_extract(d.metadata_json, '$.code_lang') AS cl, \
                        COUNT(c.chunk_id) \
                 FROM chunks c \
                 INNER JOIN documents d ON c.doc_id = d.doc_id \
                 WHERE cl IS NOT NULL \
                 GROUP BY cl",
            )
            .context("prepare code_lang_chunk_breakdown")?;
        let rows = stmt
            .query_map([], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u32))
            })
            .context("query code_lang_chunk_breakdown")?;
        let mut out = std::collections::BTreeMap::new();
        for row in rows {
            let (k, v) = row.context("read code_lang_chunk_breakdown row")?;
            out.insert(k, v);
        }
        Ok(out)
    }

    /// p10-1A-2 follow-up (dogfooding 2026-05-20): per-repo doc count for
    /// `schema.v1`.
    ///
    /// Reads `metadata_json->'$.repo'`, groups by the value, and skips rows
    /// where `repo` is NULL (documents without an explicit repo tag).
    /// Returns `BTreeMap<String, u32>` — key is the repo name as stored in
    /// frontmatter, value is the doc count.
    pub fn repo_breakdown(&self) -> anyhow::Result<std::collections::BTreeMap<String, u32>> {
        use anyhow::Context;
        let conn = self.read_conn();
        let mut stmt = conn
            .prepare(
                "SELECT json_extract(metadata_json, '$.repo') AS rp, COUNT(*) \
                 FROM documents \
                 WHERE rp IS NOT NULL \
                 GROUP BY rp",
            )
            .context("prepare repo_breakdown")?;
        let rows = stmt
            .query_map([], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u32))
            })
            .context("query repo_breakdown")?;
        let mut out = std::collections::BTreeMap::new();
        for row in rows {
            let (k, v) = row.context("read repo_breakdown row")?;
            out.insert(k, v);
        }
        Ok(out)
    }

    /// p20-bugfix3 Bug #13: schema.v1.models.active_parsers 의 source.
    /// `documents.parser_version` 컬럼의 DISTINCT 값을 정렬해 반환.
    /// 빈 corpus → 빈 Vec.
    pub fn fetch_distinct_parser_versions(&self) -> anyhow::Result<Vec<String>> {
        use anyhow::Context;
        let conn = self.read_conn();
        let mut stmt = conn
            .prepare(
                "SELECT DISTINCT parser_version FROM documents \
                  WHERE parser_version IS NOT NULL AND parser_version != '' \
                  ORDER BY parser_version",
            )
            .context("prepare fetch_distinct_parser_versions")?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .context("query fetch_distinct_parser_versions")?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.context("read parser_version row")?);
        }
        Ok(out)
    }

    /// p20-bugfix3 Bug #13: schema.v1.models.active_chunkers 의 source.
    /// `chunks.chunker_version` 컬럼의 DISTINCT 값을 정렬해 반환.
    pub fn fetch_distinct_chunker_versions(&self) -> anyhow::Result<Vec<String>> {
        use anyhow::Context;
        let conn = self.read_conn();
        let mut stmt = conn
            .prepare(
                "SELECT DISTINCT chunker_version FROM chunks \
                  WHERE chunker_version IS NOT NULL AND chunker_version != '' \
                  ORDER BY chunker_version",
            )
            .context("prepare fetch_distinct_chunker_versions")?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .context("query fetch_distinct_chunker_versions")?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.context("read chunker_version row")?);
        }
        Ok(out)
    }

    // ── v0.20.x r2 Enhancement 2: pdf_ocr_events ─────────────────────────

    /// Insert one OCR sample row into `pdf_ocr_events` (V008 migration).
    /// Follows the existing `Mutex<Connection>` lock pattern (F2).
    #[allow(clippy::too_many_arguments)]
    pub fn record_pdf_ocr_event(
        &self,
        run_id: &str,
        ts: &str,
        doc_id: Option<&str>,
        doc_path: &str,
        page: u32,
        image_byte_size: Option<u64>,
        image_width: Option<u32>,
        image_height: Option<u32>,
        ms: u64,
        chars: u32,
        success: bool,
        reason: Option<&str>,
        ocr_engine: &str,
    ) -> anyhow::Result<()> {
        let conn = self.conn.lock().expect("sqlite lock poisoned");
        conn.execute(
            "INSERT INTO pdf_ocr_events
             (run_id, ts, doc_id, doc_path, page,
              image_byte_size, image_width, image_height,
              ms, chars, success, reason, ocr_engine)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params![
                run_id,
                ts,
                doc_id,
                doc_path,
                page,
                image_byte_size,
                image_width,
                image_height,
                ms,
                chars,
                i32::from(success),
                reason,
                ocr_engine
            ],
        )?;
        Ok(())
    }

    /// Delete rows from `pdf_ocr_events` older than `retention_days`.
    /// Returns the number of deleted rows.
    /// Cutoff is computed as `now_utc - retention_days`; a value of 0
    /// means "delete everything older than now" (i.e. all past rows).
    pub fn prune_pdf_ocr_events(&self, retention_days: u32) -> anyhow::Result<u64> {
        use time::format_description::well_known::Rfc3339;
        let cutoff =
            time::OffsetDateTime::now_utc() - time::Duration::days(i64::from(retention_days));
        let cutoff_ts = cutoff
            .format(&Rfc3339)
            .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string());
        let conn = self.conn.lock().expect("sqlite lock poisoned");
        let n = conn.execute(
            "DELETE FROM pdf_ocr_events WHERE ts < ?",
            rusqlite::params![cutoff_ts],
        )?;
        Ok(n as u64)
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn open_fresh_store() -> (tempfile::TempDir, SqliteStore) {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = kebab_config::Config::defaults();
        cfg.storage.data_dir = dir.path().to_string_lossy().into_owned();
        let store = SqliteStore::open(&cfg).unwrap();
        store.run_migrations().unwrap();
        (dir, store)
    }

    #[test]
    fn count_summary_zero_on_fresh_store() {
        let (_dir, store) = open_fresh_store();
        let s = store.count_summary().unwrap();
        assert_eq!(s.doc_count, 0);
        assert_eq!(s.chunk_count, 0);
        assert_eq!(s.asset_count, 0);
        assert!(s.last_ingest_at.is_none());
        assert_eq!(s.media_breakdown.len(), 5);
        assert!(s.lang_breakdown.is_empty());
        assert_eq!(s.stale_doc_count, 0);
    }

    /// p10-1A-2: `code_lang_breakdown` counts docs by `metadata_json.code_lang`.
    ///
    /// Inserts:
    /// - one doc with `code_lang = "rust"` → must appear with count 1
    /// - one doc with `code_lang = null`   → must NOT appear (NULL skipped)
    ///
    /// Uses a side rusqlite connection that bypasses the `assets` FK via
    /// `PRAGMA foreign_keys = OFF` so the test is self-contained.
    #[test]
    fn code_lang_breakdown_counts_by_code_lang() {
        let (dir, store) = open_fresh_store();

        // Insert two document rows directly. Disabling FK enforcement lets
        // us skip the companion `assets` insert.
        let db_path = dir.path().join("kebab.sqlite");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.pragma_update(None, "foreign_keys", "OFF").unwrap();

        // Doc 1: Rust code file — code_lang = "rust"
        conn.execute(
            "INSERT INTO documents (
                doc_id, asset_id, workspace_path,
                source_type, trust_level, parser_version,
                doc_version, schema_version,
                metadata_json, provenance_json,
                created_at, updated_at
            ) VALUES (
                'doc-rust-1', 'asset-1', 'src/main.rs',
                'reference', 'primary', 'test-v1',
                1, 1,
                '{\"code_lang\":\"rust\"}', '{}',
                '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z'
            )",
            [],
        )
        .unwrap();

        // Doc 2: Markdown doc — code_lang absent (null in JSON)
        conn.execute(
            "INSERT INTO documents (
                doc_id, asset_id, workspace_path,
                source_type, trust_level, parser_version,
                doc_version, schema_version,
                metadata_json, provenance_json,
                created_at, updated_at
            ) VALUES (
                'doc-md-1', 'asset-2', 'notes/readme.md',
                'markdown', 'primary', 'test-v1',
                1, 1,
                '{\"code_lang\":null}', '{}',
                '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z'
            )",
            [],
        )
        .unwrap();

        drop(conn); // release side connection before querying via store

        let bd = store.code_lang_breakdown().unwrap();

        // rust must appear with count 1
        assert_eq!(
            bd.get("rust"),
            Some(&1u32),
            "expected rust=1 in code_lang_breakdown, got: {bd:?}"
        );
        // null code_lang must NOT appear as any key
        assert!(
            !bd.contains_key("null"),
            "null code_lang must not appear in breakdown, got: {bd:?}"
        );
        // only one key total
        assert_eq!(bd.len(), 1, "expected exactly 1 entry, got: {bd:?}");
    }

    /// v0.17.0 PR-C: `code_lang_chunk_breakdown` counts *chunks* (not
    /// docs) grouped by `documents.metadata_json.code_lang`. Differs
    /// from `code_lang_breakdown` (doc count) by joining `chunks` and
    /// summing chunk rows so one Rust file with 3 chunks reports
    /// `rust=3` here vs `rust=1` in the doc-count helper.
    ///
    /// Uses a side rusqlite connection (FK enforcement off) so a single
    /// doc + multiple chunks fixture can be inserted without standing
    /// up `assets` companions.
    #[test]
    fn code_lang_chunk_breakdown_counts_chunks_not_docs() {
        let (dir, store) = open_fresh_store();
        let db_path = dir.path().join("kebab.sqlite");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.pragma_update(None, "foreign_keys", "OFF").unwrap();

        // 1 Rust doc + 3 chunks → chunk_breakdown rust=3 / doc_breakdown rust=1.
        conn.execute(
            "INSERT INTO documents (
                doc_id, asset_id, workspace_path,
                source_type, trust_level, parser_version,
                doc_version, schema_version,
                metadata_json, provenance_json,
                created_at, updated_at
            ) VALUES (
                'doc-rust-1', 'asset-1', 'src/main.rs',
                'reference', 'primary', 'test-v1',
                1, 1,
                '{\"code_lang\":\"rust\"}', '{}',
                '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z'
            )",
            [],
        )
        .unwrap();
        for i in 0..3u32 {
            conn.execute(
                "INSERT INTO chunks (
                    chunk_id, doc_id, text, heading_path_json, section_label,
                    source_spans_json, token_estimate, chunker_version,
                    policy_hash, block_ids_json, created_at
                ) VALUES (?, 'doc-rust-1', ?, '[]', NULL, '[]', 0, 'cv1', 'h', '[]', '2024-01-01T00:00:00Z')",
                rusqlite::params![format!("rust-chunk-{i:0>26}"), format!("body {i}")],
            )
            .unwrap();
        }

        // 1 markdown doc + 1 chunk → code_lang = null → must be skipped.
        conn.execute(
            "INSERT INTO documents (
                doc_id, asset_id, workspace_path,
                source_type, trust_level, parser_version,
                doc_version, schema_version,
                metadata_json, provenance_json,
                created_at, updated_at
            ) VALUES (
                'doc-md-1', 'asset-2', 'notes/readme.md',
                'markdown', 'primary', 'test-v1',
                1, 1,
                '{\"code_lang\":null}', '{}',
                '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z'
            )",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO chunks (
                chunk_id, doc_id, text, heading_path_json, section_label,
                source_spans_json, token_estimate, chunker_version,
                policy_hash, block_ids_json, created_at
            ) VALUES ('md-chunk-00000000000000000000000', 'doc-md-1', 'm', '[]', NULL, '[]', 0, 'cv1', 'h', '[]', '2024-01-01T00:00:00Z')",
            [],
        )
        .unwrap();

        drop(conn);

        let chunk_bd = store.code_lang_chunk_breakdown().unwrap();
        assert_eq!(
            chunk_bd.get("rust"),
            Some(&3u32),
            "expected rust=3 chunks (1 doc × 3 chunks): {chunk_bd:?}"
        );
        assert!(
            !chunk_bd.contains_key("null"),
            "null code_lang must be skipped: {chunk_bd:?}"
        );
        assert_eq!(
            chunk_bd.len(),
            1,
            "expected exactly 1 language entry: {chunk_bd:?}"
        );

        // Sanity: the existing doc-count helper still returns 1 for rust,
        // proving the two metrics differ as intended.
        let doc_bd = store.code_lang_breakdown().unwrap();
        assert_eq!(
            doc_bd.get("rust"),
            Some(&1u32),
            "doc-count helper unchanged: {doc_bd:?}"
        );
    }

    /// p10-1A-2 follow-up: `repo_breakdown` counts docs by
    /// `metadata_json.repo`.
    ///
    /// Inserts:
    /// - one doc with `repo = "my-repo"` → must appear with count 1
    /// - one doc with `repo = null`       → must NOT appear (NULL skipped)
    ///
    /// Uses a side rusqlite connection that bypasses the `assets` FK via
    /// `PRAGMA foreign_keys = OFF` so the test is self-contained.
    #[test]
    fn repo_breakdown_counts_by_repo() {
        let (dir, store) = open_fresh_store();

        let db_path = dir.path().join("kebab.sqlite");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.pragma_update(None, "foreign_keys", "OFF").unwrap();

        // Doc 1: doc with repo = "my-repo"
        conn.execute(
            "INSERT INTO documents (
                doc_id, asset_id, workspace_path,
                source_type, trust_level, parser_version,
                doc_version, schema_version,
                metadata_json, provenance_json,
                created_at, updated_at
            ) VALUES (
                'doc-repo-1', 'asset-r1', 'my-repo/README.md',
                'markdown', 'primary', 'test-v1',
                1, 1,
                '{\"repo\":\"my-repo\"}', '{}',
                '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z'
            )",
            [],
        )
        .unwrap();

        // Doc 2: doc with repo absent (null in JSON)
        conn.execute(
            "INSERT INTO documents (
                doc_id, asset_id, workspace_path,
                source_type, trust_level, parser_version,
                doc_version, schema_version,
                metadata_json, provenance_json,
                created_at, updated_at
            ) VALUES (
                'doc-norepo-1', 'asset-r2', 'standalone/notes.md',
                'markdown', 'primary', 'test-v1',
                1, 1,
                '{\"repo\":null}', '{}',
                '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z'
            )",
            [],
        )
        .unwrap();

        drop(conn); // release side connection before querying via store

        let bd = store.repo_breakdown().unwrap();

        // "my-repo" must appear with count 1
        assert_eq!(
            bd.get("my-repo"),
            Some(&1u32),
            "expected my-repo=1 in repo_breakdown, got: {bd:?}"
        );
        // null repo must NOT appear as any key
        assert!(
            !bd.contains_key("null"),
            "null repo must not appear in breakdown, got: {bd:?}"
        );
        // only one key total
        assert_eq!(bd.len(), 1, "expected exactly 1 entry, got: {bd:?}");
    }
}
