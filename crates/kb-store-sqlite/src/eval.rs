//! `eval_runs` / `eval_query_results` row writers (P5-1 — design §5.7).
//!
//! `kb-eval` calls these directly via the inherent methods on
//! [`SqliteStore`]. The pattern mirrors [`crate::answers`]: the trait
//! `kb_core::DocumentStore` is the document surface, and run-level
//! audit rows (jobs, ingest_runs, answers, eval_runs) are inherent
//! methods so the trait surface stays small.

use anyhow::{Context, Result};
use rusqlite::params;
use time::OffsetDateTime;

use crate::error::StoreError;
use crate::store::SqliteStore;

/// One row about to land in `eval_runs` (per V001 schema).
///
/// `aggregate_json` is filled by P5-1 with the literal `"{}"` —
/// metric computation lives in P5-2 and updates the row in place.
#[derive(Clone, Debug)]
pub struct EvalRunRow<'a> {
    pub run_id: &'a str,
    pub suite: &'a str,
    pub config_snapshot_json: &'a str,
    pub aggregate_json: &'a str,
    pub commit_hash: Option<&'a str>,
    pub created_at: OffsetDateTime,
}

impl SqliteStore {
    /// Return `true` iff a row with `doc_id = ?` exists in
    /// `documents`. Lightweight existence probe used by
    /// `kb-eval`'s golden-fixture validator — full
    /// `DocumentStore::get_document` deserializes blocks + metadata
    /// JSON, which is overkill for "does this ID exist?"
    pub fn document_exists(&self, doc_id: &str) -> Result<bool> {
        let conn = self.lock_conn();
        let row: Result<i64, rusqlite::Error> = conn.query_row(
            "SELECT 1 FROM documents WHERE doc_id = ? LIMIT 1",
            params![doc_id],
            |r| r.get(0),
        );
        match row {
            Ok(_) => Ok(true),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(false),
            Err(e) => Err(StoreError::from(e).into()),
        }
    }

    /// Same shape as [`Self::document_exists`] but probes the
    /// `chunks` table by `chunk_id`.
    pub fn chunk_exists(&self, chunk_id: &str) -> Result<bool> {
        let conn = self.lock_conn();
        let row: Result<i64, rusqlite::Error> = conn.query_row(
            "SELECT 1 FROM chunks WHERE chunk_id = ? LIMIT 1",
            params![chunk_id],
            |r| r.get(0),
        );
        match row {
            Ok(_) => Ok(true),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(false),
            Err(e) => Err(StoreError::from(e).into()),
        }
    }

    /// Insert one row into `eval_runs`. Mirrors the schema in
    /// `migrations/V001__init.sql` (§5.7). Called by
    /// `kb-eval::run_eval` once per run, after every per-query result
    /// row has been written.
    pub fn record_eval_run(&self, row: &EvalRunRow<'_>) -> Result<()> {
        let created_at = row
            .created_at
            .format(&time::format_description::well_known::Rfc3339)
            .context("format eval_runs.created_at")?;
        let conn = self.lock_conn();
        conn.execute(
            "INSERT INTO eval_runs (
                run_id, suite, config_snapshot_json, aggregate_json,
                commit_hash, created_at
            ) VALUES (?, ?, ?, ?, ?, ?)",
            params![
                row.run_id,
                row.suite,
                row.config_snapshot_json,
                row.aggregate_json,
                row.commit_hash,
                created_at,
            ],
        )
        .map_err(StoreError::from)?;
        Ok(())
    }

    /// Insert one row into `eval_query_results`. PRIMARY KEY is
    /// `(run_id, query_id)` so writing the same `(run, query)` twice
    /// surfaces a `UNIQUE` violation through `StoreError`.
    pub fn record_eval_query_result(
        &self,
        run_id: &str,
        query_id: &str,
        result_json: &str,
    ) -> Result<()> {
        let conn = self.lock_conn();
        conn.execute(
            "INSERT INTO eval_query_results (run_id, query_id, result_json)
             VALUES (?, ?, ?)",
            params![run_id, query_id, result_json],
        )
        .map_err(StoreError::from)?;
        Ok(())
    }

    /// Insert the `eval_runs` row plus every `eval_query_results` row
    /// for the same run inside a single SQLite transaction. This is the
    /// preferred path for `kb-eval::run_eval` — a panic between the run
    /// row and the per-query rows can't leave orphan run rows.
    ///
    /// `results` is a slice of `(query_id, result_json)` tuples mirroring
    /// the per-call `record_eval_query_result` arguments.
    pub fn record_eval_run_with_results(
        &self,
        row: &EvalRunRow<'_>,
        results: &[(String, String)],
    ) -> Result<()> {
        let created_at = row
            .created_at
            .format(&time::format_description::well_known::Rfc3339)
            .context("format eval_runs.created_at")?;
        let mut conn = self.lock_conn();
        let tx = conn.transaction().map_err(StoreError::from)?;
        tx.execute(
            "INSERT INTO eval_runs (
                run_id, suite, config_snapshot_json, aggregate_json,
                commit_hash, created_at
            ) VALUES (?, ?, ?, ?, ?, ?)",
            params![
                row.run_id,
                row.suite,
                row.config_snapshot_json,
                row.aggregate_json,
                row.commit_hash,
                created_at,
            ],
        )
        .map_err(StoreError::from)?;
        {
            let mut stmt = tx
                .prepare(
                    "INSERT INTO eval_query_results (run_id, query_id, result_json)
                     VALUES (?, ?, ?)",
                )
                .map_err(StoreError::from)?;
            for (query_id, result_json) in results {
                stmt.execute(params![row.run_id, query_id, result_json])
                    .map_err(StoreError::from)?;
            }
        }
        tx.commit().map_err(StoreError::from)?;
        Ok(())
    }
}
