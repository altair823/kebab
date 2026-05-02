//! `JobRepo` impl per design §7.2.
//!
//! The `jobs` table is the §5.7 schema. JobIds are minted via blake3 over
//! `(now, kind, payload)` so two `create` calls in the same millisecond
//! still distinguish.
//!
//! This module also owns the `ingest_runs` writer. `ingest_runs` is the
//! §5.7 sibling table that records per-run aggregate counts (`scanned`,
//! `new_count`, `updated_count`, …) alongside the `jobs` row that
//! `kb jobs` shows. The aggregate-counts surface is intentionally a
//! direct INSERT (not a `JobRepo` trait method) because `JobRepo` is
//! generic across job kinds, while `ingest_runs` is ingest-specific
//! schema with dedicated columns.

use anyhow::{Context, Result};
use rusqlite::params;
use serde_json::Value;
use time::OffsetDateTime;

use crate::error::StoreError;
use crate::store::SqliteStore;

/// Aggregate counts for one ingest run. Written into the `ingest_runs`
/// table so `kb jobs` (P+) and audit tooling can surface the per-run
/// summary without re-walking the workspace.
///
/// `items_json` carries the per-item detail when the run was NOT
/// `summary_only`; it is `None` when the caller asked for a summary
/// (the table column is then NULL per design §5.7).
#[derive(Clone, Debug)]
pub struct IngestRunRow<'a> {
    pub run_id: &'a str,
    pub scope_json: &'a str,
    pub scanned: u32,
    pub new_count: u32,
    pub updated_count: u32,
    pub skipped_count: u32,
    pub error_count: u32,
    pub duration_ms: u32,
    pub started_at: OffsetDateTime,
    pub finished_at: OffsetDateTime,
    pub items_json: Option<&'a str>,
}

impl SqliteStore {
    /// Write one row into `ingest_runs` with the aggregate counts. Mirrors
    /// the schema in `migrations/V001__init.sql` (§5.7). Called by
    /// `kb-app::ingest` at the very end of a run, after the per-document
    /// transactions have committed and the totals are known.
    ///
    /// `items_json = None` ↔ the column stores SQL `NULL`, which is the
    /// `summary_only=true` contract.
    pub fn record_ingest_run(&self, row: &IngestRunRow<'_>) -> Result<()> {
        let started = row
            .started_at
            .format(&time::format_description::well_known::Rfc3339)
            .context("format ingest_run started_at")?;
        let finished = row
            .finished_at
            .format(&time::format_description::well_known::Rfc3339)
            .context("format ingest_run finished_at")?;
        let conn = self.lock_conn();
        conn.execute(
            "INSERT INTO ingest_runs (
                run_id, scope_json, scanned, new_count, updated_count,
                skipped_count, error_count, duration_ms,
                started_at, finished_at, items_json
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                row.run_id,
                row.scope_json,
                row.scanned as i64,
                row.new_count as i64,
                row.updated_count as i64,
                row.skipped_count as i64,
                row.error_count as i64,
                row.duration_ms as i64,
                started,
                finished,
                row.items_json,
            ],
        )
        .map_err(StoreError::from)?;
        Ok(())
    }
}

impl kebab_core::JobRepo for SqliteStore {
    fn create(
        &self,
        kind: kebab_core::JobKind,
        payload: Value,
    ) -> Result<kebab_core::JobId> {
        let now_dt = OffsetDateTime::now_utc();
        let now = now_dt
            .format(&time::format_description::well_known::Rfc3339)
            .context("format job created_at")?;
        // JobId recipe: stable hex over (kind, payload_canonical, ns).
        // The nanosecond timestamp is included so two `create` calls with
        // identical `(kind, payload)` still get distinct IDs.
        let job_id = mint_job_id(&kind, &payload, now_dt);
        let kind_label = job_kind_label(&kind);
        let payload_json = serde_json::to_string(&payload)
            .context("serialize job payload")?;
        let conn = self.lock_conn();
        conn.execute(
            "INSERT INTO jobs (
                job_id, kind, status, payload_json, progress_json,
                error_json, created_at, updated_at, finished_at
            ) VALUES (?, ?, 'pending', ?, NULL, NULL, ?, ?, NULL)",
            params![job_id.0, kind_label, payload_json, now, now],
        )
        .map_err(StoreError::from)?;
        Ok(job_id)
    }

    fn update_progress(
        &self,
        id: &kebab_core::JobId,
        progress: Value,
    ) -> Result<()> {
        let progress_json = serde_json::to_string(&progress)
            .context("serialize job progress")?;
        let now = OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .context("format job updated_at")?;
        let conn = self.lock_conn();
        // status='pending' → 'running' on first progress update; later
        // progress calls keep status='running' until finish().
        conn.execute(
            "UPDATE jobs SET
                progress_json = ?,
                status = CASE status WHEN 'pending' THEN 'running' ELSE status END,
                updated_at = ?
             WHERE job_id = ?",
            params![progress_json, now, id.0],
        )
        .map_err(StoreError::from)?;
        Ok(())
    }

    fn finish(
        &self,
        id: &kebab_core::JobId,
        status: kebab_core::JobStatus,
        error: Option<&str>,
    ) -> Result<()> {
        let now = OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .context("format job finished_at")?;
        let status_label = job_status_label(&status);
        let error_json = error
            .map(|e| serde_json::to_string(&serde_json::json!({ "message": e })))
            .transpose()
            .context("serialize job error")?;
        let conn = self.lock_conn();
        conn.execute(
            "UPDATE jobs SET
                status = ?,
                error_json = ?,
                updated_at = ?,
                finished_at = ?
             WHERE job_id = ?",
            params![status_label, error_json, now, now, id.0],
        )
        .map_err(StoreError::from)?;
        Ok(())
    }

    fn list(
        &self,
        filter: &kebab_core::JobFilter,
    ) -> Result<Vec<kebab_core::JobRow>> {
        let conn = self.lock_conn();
        let mut sql = String::from(
            "SELECT job_id, kind, status, payload_json, progress_json,
                    error_json, created_at, updated_at, finished_at
             FROM jobs WHERE 1=1",
        );
        let mut params_dyn: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if let Some(status) = &filter.status {
            sql.push_str(" AND status = ?");
            params_dyn.push(Box::new(job_status_label(status).to_string()));
        }
        if let Some(kind) = &filter.kind {
            sql.push_str(" AND kind = ?");
            params_dyn.push(Box::new(job_kind_label(kind).to_string()));
        }
        sql.push_str(" ORDER BY created_at ASC");

        let mut stmt = conn.prepare(&sql).map_err(StoreError::from)?;
        let rows = stmt
            .query_map(
                rusqlite::params_from_iter(params_dyn.iter().map(|b| b.as_ref())),
                job_row_from_sql,
            )
            .map_err(StoreError::from)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(StoreError::from)?);
        }
        Ok(out)
    }
}

/// Mint a JobId over (kind, canonical(payload), nanos). The 32-hex
/// invariant on `kebab_core::JobId` is honored by taking the first 32 chars
/// of the blake3 hex.
fn mint_job_id(
    kind: &kebab_core::JobKind,
    payload: &Value,
    at: OffsetDateTime,
) -> kebab_core::JobId {
    // Plain serde_json::to_vec is enough — JobIds are not part of the
    // §4.2 ID family and don't need canonical-JSON parity with other IDs.
    // The nanosecond suffix is what guarantees uniqueness, not stable
    // hashing.
    let mut hasher = blake3::Hasher::new();
    hasher.update(job_kind_label(kind).as_bytes());
    if let Ok(bytes) = serde_json::to_vec(payload) {
        hasher.update(&bytes);
    }
    hasher.update(&at.unix_timestamp_nanos().to_be_bytes());
    let hex = hasher.finalize().to_hex().to_string();
    kebab_core::JobId(hex[..32].to_string())
}

fn job_kind_label(k: &kebab_core::JobKind) -> &'static str {
    match k {
        kebab_core::JobKind::Ingest => "ingest",
        kebab_core::JobKind::Chunk => "chunk",
        kebab_core::JobKind::Embed => "embed",
        kebab_core::JobKind::Ocr => "ocr",
        kebab_core::JobKind::Transcribe => "transcribe",
        kebab_core::JobKind::Reindex => "reindex",
        kebab_core::JobKind::Doctor => "doctor",
    }
}

fn job_status_label(s: &kebab_core::JobStatus) -> &'static str {
    match s {
        kebab_core::JobStatus::Pending => "pending",
        kebab_core::JobStatus::Running => "running",
        kebab_core::JobStatus::Succeeded => "succeeded",
        kebab_core::JobStatus::Failed => "failed",
        kebab_core::JobStatus::Canceled => "canceled",
    }
}

fn job_row_from_sql(row: &rusqlite::Row<'_>) -> rusqlite::Result<kebab_core::JobRow> {
    let job_id: String = row.get(0)?;
    let kind_raw: String = row.get(1)?;
    let status_raw: String = row.get(2)?;
    let payload_json: String = row.get(3)?;
    let progress_json: Option<String> = row.get(4)?;
    let error_json: Option<String> = row.get(5)?;
    let created_at_raw: String = row.get(6)?;
    let updated_at_raw: String = row.get(7)?;
    let finished_at_raw: Option<String> = row.get(8)?;

    let kind: kebab_core::JobKind =
        serde_json::from_value(serde_json::Value::String(kind_raw))
            .map_err(conv_err(1))?;
    let status: kebab_core::JobStatus =
        serde_json::from_value(serde_json::Value::String(status_raw))
            .map_err(conv_err(2))?;
    let payload: Value = serde_json::from_str(&payload_json).map_err(conv_err(3))?;
    let progress: Option<Value> = match progress_json {
        Some(s) => Some(serde_json::from_str(&s).map_err(conv_err(4))?),
        None => None,
    };
    // Surface the stored error message back as a plain string per the
    // JobRow schema (§7.2). We stored `{"message": "..."}` for forward
    // compatibility — pull `message` back out, or fall back to the raw
    // JSON if the shape ever drifts.
    let error: Option<String> = match error_json {
        Some(s) => match serde_json::from_str::<Value>(&s) {
            Ok(v) => v
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or(Some(s)),
            Err(_) => Some(s),
        },
        None => None,
    };

    let created_at = OffsetDateTime::parse(
        &created_at_raw,
        &time::format_description::well_known::Rfc3339,
    )
    .map_err(conv_err(6))?;
    let updated_at = OffsetDateTime::parse(
        &updated_at_raw,
        &time::format_description::well_known::Rfc3339,
    )
    .map_err(conv_err(7))?;
    let finished_at = match finished_at_raw {
        Some(s) => Some(
            OffsetDateTime::parse(&s, &time::format_description::well_known::Rfc3339)
                .map_err(conv_err(8))?,
        ),
        None => None,
    };

    Ok(kebab_core::JobRow {
        job_id: kebab_core::JobId(job_id),
        kind,
        status,
        payload,
        progress,
        error,
        created_at,
        updated_at,
        finished_at,
    })
}

fn conv_err<E: std::error::Error + Send + Sync + 'static>(
    col: usize,
) -> impl FnOnce(E) -> rusqlite::Error {
    move |e| {
        rusqlite::Error::FromSqlConversionFailure(col, rusqlite::types::Type::Text, Box::new(e))
    }
}
