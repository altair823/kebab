//! Embedding-records writers used by `kb-store-vector` (P3-3).
//!
//! The `VectorStore` impl in `kb-store-vector` performs a two-phase write:
//! phase 1 stages an `embedding_records` row at `status='pending'` before
//! issuing the Lance write, and phase 3 promotes those same rows to
//! `status='committed'` after the Lance commit lands. We surface those
//! two SQL statements here (rather than expose a generic write
//! connection) so the SQL stays inside the crate that owns the schema —
//! kb-store-vector consumes a typed, narrowly-scoped API and never
//! touches the connection mutex itself.
//!
//! Both helpers wrap a single `INSERT OR REPLACE` / `UPDATE` per row
//! inside a single SQLite transaction, so a partial failure leaves
//! either all rows pending (phase 1) or all rows committed (phase 3),
//! never a mixed batch.

use anyhow::{Context, Result};
use rusqlite::{params, params_from_iter};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::error::StoreError;
use crate::store::SqliteStore;

/// Row payload for [`SqliteStore::put_embedding_records_pending`].
///
/// Mirrors the columns of `embedding_records` minus the lifecycle markers
/// (`status` and `vector_committed`) — those are forced to `'pending'`
/// and `0` by phase 1.
///
/// `created_at` is `OffsetDateTime` rather than a pre-formatted string so
/// the helper owns the RFC3339 formatting (the same formatting choice
/// the asset / document / job writers make).
#[derive(Clone, Debug)]
pub struct EmbeddingRecordRow {
    pub embedding_id: String,
    pub chunk_id: String,
    pub model_id: String,
    pub model_version: String,
    pub dimensions: usize,
    pub lance_table: String,
    pub created_at: OffsetDateTime,
}

impl SqliteStore {
    /// Phase 1 of the kb-store-vector two-phase write: stage every
    /// `embedding_records` row with `status='pending'`,
    /// `vector_committed=0`. `INSERT OR REPLACE` (rather than UPSERT) is
    /// the right shape here because re-running phase 1 for an
    /// already-pending row resets `vector_committed` to 0 and the
    /// `created_at` to the new attempt's timestamp — both desired,
    /// because a retry should look like a fresh attempt to the GC pass.
    ///
    /// All rows are written in a single transaction; if any row fails
    /// the entire batch is rolled back and the caller can retry without
    /// worrying about partial pending state.
    pub fn put_embedding_records_pending(&self, rows: &[EmbeddingRecordRow]) -> Result<()> {
        if rows.is_empty() {
            return Ok(());
        }
        let mut conn = self.lock_conn();
        let tx = conn.transaction().map_err(StoreError::from)?;
        {
            let mut stmt = tx
                .prepare(
                    "INSERT OR REPLACE INTO embedding_records (
                        embedding_id, chunk_id, model_id, model_version,
                        dimensions, lance_table, created_at,
                        status, vector_committed
                    ) VALUES (?, ?, ?, ?, ?, ?, ?, 'pending', 0)",
                )
                .map_err(StoreError::from)?;
            for row in rows {
                let created_at = row
                    .created_at
                    .format(&Rfc3339)
                    .context("format embedding_records.created_at")?;
                stmt.execute(params![
                    row.embedding_id,
                    row.chunk_id,
                    row.model_id,
                    row.model_version,
                    row.dimensions as i64,
                    row.lance_table,
                    created_at,
                ])
                .map_err(StoreError::from)?;
            }
        }
        tx.commit().map_err(StoreError::from)?;
        Ok(())
    }

    /// Phase 3 of the kb-store-vector two-phase write: after the Lance
    /// MergeInsert commits, flip the listed embedding rows to
    /// `status='committed'`, `vector_committed=1`. Rows that aren't
    /// currently `pending` (e.g. already committed by a duplicate batch,
    /// or tombstoned by a chunks DELETE between phase 1 and phase 3)
    /// are deliberately left alone via `WHERE status='pending'` — we
    /// never resurrect a tombstone, and we never blindly re-mark a
    /// committed row.
    ///
    /// All updates run in a single statement (single SQL `UPDATE …
    /// WHERE embedding_id IN (?, ?, …)`) inside one transaction —
    /// avoids the per-row `execute()` round-trip the previous
    /// implementation paid.
    pub fn mark_embedding_records_committed(&self, embedding_ids: &[String]) -> Result<()> {
        if embedding_ids.is_empty() {
            return Ok(());
        }
        let mut conn = self.lock_conn();
        let tx = conn.transaction().map_err(StoreError::from)?;
        {
            let placeholders = std::iter::repeat_n("?", embedding_ids.len())
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!(
                "UPDATE embedding_records
                    SET status='committed', vector_committed=1
                  WHERE status='pending'
                    AND embedding_id IN ({placeholders})"
            );
            tx.execute(&sql, params_from_iter(embedding_ids.iter()))
                .map_err(StoreError::from)?;
        }
        tx.commit().map_err(StoreError::from)?;
        Ok(())
    }

    /// Wipe every row from `embedding_records`, returning the count of
    /// rows that were removed. Called by `kebab reset --vector-only` so
    /// SQLite cannot point at a Lance row that the reset just removed
    /// off-disk.
    ///
    /// The function does NOT cascade to `chunks` or `documents` — those
    /// are kept so the next `kebab ingest` re-embeds the existing chunk
    /// set without re-parsing.
    pub fn truncate_embedding_records(&self) -> Result<u64> {
        let conn = self.lock_conn();
        let n = conn
            .execute("DELETE FROM embedding_records", [])
            .context("DELETE FROM embedding_records")?;
        Ok(n as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kebab_config::Config;
    use tempfile::TempDir;
    use time::OffsetDateTime;

    /// Minimal config pointing at a tempdir for the SQLite file.
    fn config_for(tmp: &TempDir) -> Config {
        let mut c = Config::defaults();
        c.storage.data_dir = tmp.path().to_string_lossy().into_owned();
        c
    }

    /// Seed a chunks row + the doc / asset rows it FKs to. The minimum
    /// needed for embedding_records inserts not to fail the FK to
    /// chunks.
    fn seed_chunk(store: &SqliteStore, chunk_id: &str) {
        let conn = store.lock_conn();
        // Asset, document, chunk — all hand-rolled at the SQL layer to
        // keep the test self-contained (no kb-parse/kb-chunk dep).
        conn.execute(
            "INSERT INTO assets (
                asset_id, source_uri, workspace_path, media_type, byte_len,
                checksum, storage_kind, storage_path, discovered_at
             ) VALUES (?, ?, ?, ?, ?, ?, 'reference', '/tmp/x', ?)",
            params![
                "0123456789abcdef0123456789abcdef",
                "file:///tmp/x",
                "x.md",
                "{}",
                0_i64,
                "deadbeef",
                "1970-01-01T00:00:00Z",
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO documents (
                doc_id, asset_id, workspace_path, title, lang, source_type,
                trust_level, parser_version, doc_version, schema_version,
                metadata_json, provenance_json, created_at, updated_at
             ) VALUES (?, ?, ?, NULL, NULL, 'fs', 'unverified', 'v1', 1, 1, '{}', '{}', ?, ?)",
            params![
                "fedcba9876543210fedcba9876543210",
                "0123456789abcdef0123456789abcdef",
                "x.md",
                "1970-01-01T00:00:00Z",
                "1970-01-01T00:00:00Z",
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO chunks (
                chunk_id, doc_id, text, heading_path_json, section_label,
                source_spans_json, token_estimate, chunker_version,
                policy_hash, block_ids_json, created_at
             ) VALUES (?, ?, 'hi', '[]', NULL, '[]', 1, 'v1', 'hash', '[]', ?)",
            params![
                chunk_id,
                "fedcba9876543210fedcba9876543210",
                "1970-01-01T00:00:00Z"
            ],
        )
        .unwrap();
    }

    fn open_store(tmp: &TempDir) -> SqliteStore {
        let cfg = config_for(tmp);
        let store = SqliteStore::open(&cfg.storage).unwrap();
        store.run_migrations().unwrap();
        store
    }

    #[test]
    fn pending_then_committed_round_trip() {
        let tmp = TempDir::new().unwrap();
        let store = open_store(&tmp);
        let chunk = "11112222333344445555666677778888";
        seed_chunk(&store, chunk);

        let row = EmbeddingRecordRow {
            embedding_id: "aaaa1111bbbb2222cccc3333dddd4444".to_string(),
            chunk_id: chunk.to_string(),
            model_id: "test-model".to_string(),
            model_version: "v1".to_string(),
            dimensions: 4,
            lance_table: "chunk_embeddings_test_model_4".to_string(),
            created_at: OffsetDateTime::now_utc(),
        };
        store
            .put_embedding_records_pending(std::slice::from_ref(&row))
            .unwrap();

        // Inspect: the row exists at status='pending'.
        {
            let conn = store.read_conn();
            let (status, committed): (String, i64) = conn
                .query_row(
                    "SELECT status, vector_committed FROM embedding_records WHERE embedding_id = ?",
                    params![row.embedding_id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
            assert_eq!(status, "pending");
            assert_eq!(committed, 0);
        }

        store
            .mark_embedding_records_committed(std::slice::from_ref(&row.embedding_id))
            .unwrap();
        {
            let conn = store.read_conn();
            let (status, committed): (String, i64) = conn
                .query_row(
                    "SELECT status, vector_committed FROM embedding_records WHERE embedding_id = ?",
                    params![row.embedding_id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
            assert_eq!(status, "committed");
            assert_eq!(committed, 1);
        }
    }

    #[test]
    fn empty_batches_are_noops() {
        let tmp = TempDir::new().unwrap();
        let store = open_store(&tmp);
        store.put_embedding_records_pending(&[]).unwrap();
        store.mark_embedding_records_committed(&[]).unwrap();
    }

    #[test]
    fn replay_phase_one_resets_vector_committed() {
        // INSERT OR REPLACE: a phase-1 retry on a row that briefly
        // reached `committed` (in some adversarial out-of-order replay)
        // resets it to `pending`. Confirms the documented semantics.
        let tmp = TempDir::new().unwrap();
        let store = open_store(&tmp);
        let chunk = "11112222333344445555666677778888";
        seed_chunk(&store, chunk);

        let row = EmbeddingRecordRow {
            embedding_id: "aaaa1111bbbb2222cccc3333dddd4444".to_string(),
            chunk_id: chunk.to_string(),
            model_id: "test-model".to_string(),
            model_version: "v1".to_string(),
            dimensions: 4,
            lance_table: "chunk_embeddings_test_model_4".to_string(),
            created_at: OffsetDateTime::now_utc(),
        };
        store
            .put_embedding_records_pending(std::slice::from_ref(&row))
            .unwrap();
        store
            .mark_embedding_records_committed(std::slice::from_ref(&row.embedding_id))
            .unwrap();
        store
            .put_embedding_records_pending(std::slice::from_ref(&row))
            .unwrap();

        let conn = store.read_conn();
        let status: String = conn
            .query_row(
                "SELECT status FROM embedding_records WHERE embedding_id = ?",
                params![row.embedding_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, "pending");
    }

    #[test]
    fn mark_committed_skips_non_pending() {
        // The phase-3 UPDATE explicitly filters `status='pending'`, so
        // calling it on an embedding_id that was never staged (or that
        // already became a tombstone) is a no-op rather than an error.
        let tmp = TempDir::new().unwrap();
        let store = open_store(&tmp);
        store
            .mark_embedding_records_committed(&["does-not-exist".to_string()])
            .unwrap();
    }
}
