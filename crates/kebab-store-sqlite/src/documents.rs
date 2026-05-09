//! `DocumentStore` impl: assets, documents, document_tags, blocks, chunks.
//!
//! Transactions: per design §5.8, one ingest of one document is one
//! transaction. We expose the raw trait methods at fine granularity (so
//! `kb-app` can compose), and each one wraps its own short transaction.
//! A higher-level `ingest_document` helper that wraps put_document +
//! put_blocks + put_chunks in a single tx is intentionally NOT shipped in
//! P1-6 — `kb-app` (P1's caller layer) is the right place to compose.
//!
//! Idempotency: re-ingesting `(workspace_path, asset_id, parser_version)`
//! UPSERTs the documents row, bumps `doc_version`, and replaces all
//! blocks / chunks / document_tags. No row duplication.

use anyhow::{Context, Result};
use rusqlite::params;
use time::OffsetDateTime;

use crate::error::StoreError;
use crate::store::{
    SqliteStore, purge_orphan_at_workspace_path, upsert_asset_row, validate_asset_id,
};

impl kebab_core::DocumentStore for SqliteStore {
    fn put_asset(&self, asset: &kebab_core::RawAsset) -> Result<()> {
        // Validate the AssetId shape before any row work — defense in
        // depth against hand-constructed `kebab_core::AssetId` values that
        // bypass `FromStr`. See `validate_asset_id` for rationale.
        validate_asset_id(&asset.asset_id)?;
        // No bytes here — read storage_kind/storage_path from the
        // RawAsset's `stored` field per its convention (§3.3). Callers
        // that have raw bytes go through `put_asset_with_bytes` instead;
        // this branch is for the case where bytes were already persisted
        // (or referenced) and we just want to record the row.
        let (storage_kind, storage_path) = match &asset.stored {
            kebab_core::AssetStorage::Copied { path } => {
                ("copied", path.to_string_lossy().into_owned())
            }
            kebab_core::AssetStorage::Reference { path, .. } => {
                ("reference", path.to_string_lossy().into_owned())
            }
        };
        let conn = self.lock_conn();
        purge_orphan_at_workspace_path(
            &conn,
            &asset.workspace_path.0,
            &asset.asset_id.0,
        )?;
        upsert_asset_row(&conn, asset, storage_kind, &storage_path)
    }

    fn put_document(&self, doc: &kebab_core::CanonicalDocument) -> Result<()> {
        let mut conn = self.lock_conn();
        let tx = conn.transaction().map_err(StoreError::from)?;
        upsert_document(&tx, doc)?;
        replace_document_tags(&tx, &doc.doc_id, &doc.metadata.tags)?;
        tx.commit().map_err(StoreError::from)?;
        Ok(())
    }

    fn put_blocks(
        &self,
        doc: &kebab_core::DocumentId,
        blocks: &[kebab_core::Block],
    ) -> Result<()> {
        let mut conn = self.lock_conn();
        let tx = conn.transaction().map_err(StoreError::from)?;
        // DELETE-then-INSERT: §5.4 has no UNIQUE on (doc_id, ordinal)
        // so we cannot rely on UPSERT to surface block_id collisions. The
        // simplest correct path is to wipe and re-insert; the §5.8
        // per-document transaction wraps both halves so a partial state
        // never lands.
        tx.execute("DELETE FROM blocks WHERE doc_id = ?", params![doc.0])
            .map_err(StoreError::from)?;
        let mut stmt = tx
            .prepare(
                "INSERT INTO blocks (
                    block_id, doc_id, kind, heading_path_json,
                    ordinal, source_span_json, payload_json
                ) VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .map_err(StoreError::from)?;
        // Ordinal here is the position of the block in the document's
        // overall block stream — used for sort-on-load, not the §4.3
        // (heading_path, kind)-scoped ordinal that fed `block_id`.
        for (i, block) in blocks.iter().enumerate() {
            let row = block_to_row(doc, block, i as i64)?;
            stmt.execute(params![
                row.block_id,
                row.doc_id,
                row.kind,
                row.heading_path_json,
                row.ordinal,
                row.source_span_json,
                row.payload_json,
            ])
            .map_err(StoreError::from)?;
        }
        drop(stmt);
        tx.commit().map_err(StoreError::from)?;
        Ok(())
    }

    fn put_chunks(
        &self,
        doc: &kebab_core::DocumentId,
        chunks: &[kebab_core::Chunk],
    ) -> Result<()> {
        let now = OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .context("format chunk created_at")?;
        let mut conn = self.lock_conn();
        let tx = conn.transaction().map_err(StoreError::from)?;
        tx.execute("DELETE FROM chunks WHERE doc_id = ?", params![doc.0])
            .map_err(StoreError::from)?;
        let mut stmt = tx
            .prepare(
                "INSERT INTO chunks (
                    chunk_id, doc_id, text, heading_path_json,
                    section_label, source_spans_json, token_estimate,
                    chunker_version, policy_hash, block_ids_json, created_at
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .map_err(StoreError::from)?;
        for chunk in chunks {
            let heading_path = serde_json::to_string(&chunk.heading_path)
                .context("serialize chunk.heading_path")?;
            let source_spans = serde_json::to_string(&chunk.source_spans)
                .context("serialize chunk.source_spans")?;
            let block_ids = serde_json::to_string(&chunk.block_ids)
                .context("serialize chunk.block_ids")?;
            // §5.5 has a `section_label` column but the in-memory Chunk
            // struct does not carry it (nor does the wire schema §2.6).
            // Persist NULL until a future bump introduces the field.
            // TODO(P2/P3): populate `section_label` once Chunk and the
            // wire schema gain the field; until then NULL is the correct
            // canonical value.
            stmt.execute(params![
                chunk.chunk_id.0,
                chunk.doc_id.0,
                chunk.text,
                heading_path,
                Option::<String>::None,
                source_spans,
                chunk.token_estimate as i64,
                chunk.chunker_version.0,
                chunk.policy_hash,
                block_ids,
                now,
            ])
            .map_err(StoreError::from)?;
        }
        drop(stmt);
        tx.commit().map_err(StoreError::from)?;
        Ok(())
    }

    fn get_document(
        &self,
        id: &kebab_core::DocumentId,
    ) -> Result<Option<kebab_core::CanonicalDocument>> {
        let conn = self.lock_conn();
        let row: Option<DocumentRow> = conn
            .query_row(
                "SELECT
                    doc_id, asset_id, workspace_path, title, lang,
                    source_type, trust_level, parser_version,
                    doc_version, schema_version, metadata_json,
                    provenance_json, created_at, updated_at,
                    last_chunker_version, last_embedding_version
                FROM documents WHERE doc_id = ?",
                params![id.0],
                document_row_from_sql,
            )
            .map(Some)
            .or_else(rows_optional)
            .map_err(StoreError::from)?;
        let Some(row) = row else { return Ok(None) };

        // Rehydrate blocks. Sort by stream-ordinal so the returned
        // CanonicalDocument matches the order originally persisted.
        let mut blocks_stmt = conn
            .prepare(
                "SELECT payload_json FROM blocks
                 WHERE doc_id = ? ORDER BY ordinal ASC",
            )
            .map_err(StoreError::from)?;
        let block_rows = blocks_stmt
            .query_map(params![id.0], |r| {
                let payload_json: String = r.get(0)?;
                Ok(payload_json)
            })
            .map_err(StoreError::from)?;
        let mut blocks: Vec<kebab_core::Block> = Vec::new();
        for row in block_rows {
            let payload_json = row.map_err(StoreError::from)?;
            let block: kebab_core::Block = serde_json::from_str(&payload_json)
                .context("deserialize block payload_json")?;
            blocks.push(block);
        }

        let metadata: kebab_core::Metadata = serde_json::from_str(&row.metadata_json)
            .context("deserialize metadata_json")?;
        let provenance: kebab_core::Provenance =
            serde_json::from_str(&row.provenance_json)
                .context("deserialize provenance_json")?;

        Ok(Some(kebab_core::CanonicalDocument {
            doc_id: kebab_core::DocumentId(row.doc_id),
            source_asset_id: kebab_core::AssetId(row.asset_id),
            workspace_path: kebab_core::WorkspacePath(row.workspace_path),
            title: row.title.unwrap_or_default(),
            lang: kebab_core::Lang(row.lang.unwrap_or_default()),
            blocks,
            metadata,
            provenance,
            parser_version: kebab_core::ParserVersion(row.parser_version),
            // INVARIANT: `doc_version` is bumped by 1 on every re-ingest
            // (see `upsert_document`). The column is INTEGER (i64) but
            // CanonicalDocument carries u32; an overflow would require
            // 2^32 re-ingests of the same document, which is well beyond
            // any realistic ingest frequency. Truncating cast is safe
            // under that invariant.
            schema_version: row.schema_version as u32,
            doc_version: row.doc_version as u32,
            last_chunker_version: row.last_chunker_version.map(kebab_core::ChunkerVersion),
            last_embedding_version: row.last_embedding_version.map(kebab_core::EmbeddingVersion),
        }))
    }

    fn get_chunk(&self, id: &kebab_core::ChunkId) -> Result<Option<kebab_core::Chunk>> {
        let conn = self.lock_conn();
        let row = conn
            .query_row(
                "SELECT
                    chunk_id, doc_id, text, heading_path_json,
                    source_spans_json, token_estimate, chunker_version,
                    policy_hash, block_ids_json
                FROM chunks WHERE chunk_id = ?",
                params![id.0],
                chunk_row_from_sql,
            )
            .map(Some)
            .or_else(rows_optional)
            .map_err(StoreError::from)?;
        let Some(row) = row else { return Ok(None) };
        let heading_path: Vec<String> = serde_json::from_str(&row.heading_path_json)
            .context("deserialize chunk.heading_path_json")?;
        let source_spans: Vec<kebab_core::SourceSpan> =
            serde_json::from_str(&row.source_spans_json)
                .context("deserialize chunk.source_spans_json")?;
        let block_ids: Vec<kebab_core::BlockId> =
            serde_json::from_str(&row.block_ids_json)
                .context("deserialize chunk.block_ids_json")?;
        Ok(Some(kebab_core::Chunk {
            chunk_id: kebab_core::ChunkId(row.chunk_id),
            doc_id: kebab_core::DocumentId(row.doc_id),
            block_ids,
            text: row.text,
            heading_path,
            source_spans,
            token_estimate: row.token_estimate as usize,
            chunker_version: kebab_core::ChunkerVersion(row.chunker_version),
            policy_hash: row.policy_hash,
        }))
    }

    fn get_asset_by_workspace_path(
        &self,
        path: &kebab_core::WorkspacePath,
    ) -> Result<Option<kebab_core::RawAsset>> {
        let conn = self.lock_conn();
        let result = conn.query_row(
            r#"SELECT
                asset_id, source_uri, workspace_path, media_type,
                byte_len, checksum, storage_kind, storage_path,
                discovered_at
            FROM assets
            WHERE workspace_path = ?"#,
            rusqlite::params![path.0.as_str()],
            asset_from_row,
        );
        match result {
            Ok(asset) => Ok(Some(asset)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn list_documents(
        &self,
        filter: &kebab_core::DocFilter,
    ) -> Result<Vec<kebab_core::DocSummary>> {
        // Build a dynamic WHERE clause from the filter. Each condition
        // appends one positional `?` placeholder and one `Box<dyn
        // ToSql>` to `params` so order stays in sync.
        let conn = self.lock_conn();
        let mut sql = String::from(
            "SELECT d.doc_id, d.workspace_path, d.title, d.lang,
                    d.source_type, d.trust_level, d.parser_version,
                    d.created_at, d.updated_at,
                    a.byte_len,
                    (SELECT COUNT(*) FROM chunks c WHERE c.doc_id = d.doc_id) AS chunk_count,
                    -- chunker_version: assume one chunker per doc; pick
                    -- any row's value. NULL if no chunks yet.
                    (SELECT chunker_version FROM chunks c2
                       WHERE c2.doc_id = d.doc_id LIMIT 1) AS chunker_version
             FROM documents d
             JOIN assets a ON a.asset_id = d.asset_id
             WHERE 1=1",
        );
        let mut params_dyn: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(lang) = &filter.lang {
            sql.push_str(" AND d.lang = ?");
            params_dyn.push(Box::new(lang.0.clone()));
        }
        if let Some(trust_min) = &filter.trust_min {
            // Map the enum to its rank: Generated < Secondary < Primary.
            // (Higher trust strictly contains lower trust.)
            sql.push_str(" AND CASE d.trust_level
                WHEN 'primary' THEN 3
                WHEN 'secondary' THEN 2
                WHEN 'generated' THEN 1
                ELSE 0
                END >= ?");
            let rank: i64 = match trust_min {
                kebab_core::TrustLevel::Primary => 3,
                kebab_core::TrustLevel::Secondary => 2,
                kebab_core::TrustLevel::Generated => 1,
            };
            params_dyn.push(Box::new(rank));
        }
        if let Some(glob) = &filter.path_glob {
            sql.push_str(" AND d.workspace_path GLOB ?");
            params_dyn.push(Box::new(glob.clone()));
        }
        if !filter.tags_any.is_empty() {
            // INTERSECT-style filter: doc must own at least one of the
            // requested tags. Use IN with a placeholder list.
            sql.push_str(" AND d.doc_id IN (SELECT doc_id FROM document_tags WHERE tag IN (");
            for (i, tag) in filter.tags_any.iter().enumerate() {
                if i > 0 {
                    sql.push(',');
                }
                sql.push('?');
                params_dyn.push(Box::new(tag.clone()));
            }
            sql.push_str("))");
        }
        sql.push_str(" ORDER BY d.workspace_path");

        let mut stmt = conn.prepare(&sql).map_err(StoreError::from)?;
        let rows = stmt
            .query_map(
                rusqlite::params_from_iter(params_dyn.iter().map(|b| b.as_ref())),
                doc_summary_from_sql,
            )
            .map_err(StoreError::from)?;
        let mut out = Vec::new();
        for r in rows {
            let summary = r.map_err(StoreError::from)?;
            // tags filter at row-load time: pull the tag list per doc.
            let mut tag_stmt = conn
                .prepare("SELECT tag FROM document_tags WHERE doc_id = ? ORDER BY tag")
                .map_err(StoreError::from)?;
            let tag_iter = tag_stmt
                .query_map(params![summary.doc_id.0], |r| r.get::<_, String>(0))
                .map_err(StoreError::from)?;
            let tags: Vec<String> = tag_iter
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(StoreError::from)?;
            out.push(kebab_core::DocSummary { tags, ..summary });
        }
        Ok(out)
    }
}

impl SqliteStore {
    /// p9-fb-35: list `chunk_id`s for a document in deterministic
    /// chunker-emit order. `put_chunks` writes one transaction with a
    /// single `created_at` snapshot, so the secondary `chunk_id` sort
    /// is what actually orders neighbors within a single re-ingest;
    /// the primary `created_at` sort distinguishes successive
    /// re-ingests if they ever co-exist in the table (they shouldn't —
    /// `put_chunks` deletes the old rows first — but the ordering is
    /// still well-defined under that scenario).
    ///
    /// Used by `kebab-app::fetch::surrounding_chunks` to derive ±N
    /// neighbors around a target chunk without leaking SQL into the
    /// facade crate.
    pub fn list_chunk_ids_for_doc(
        &self,
        doc_id: &kebab_core::DocumentId,
    ) -> Result<Vec<kebab_core::ChunkId>> {
        let conn = self.read_conn();
        let mut stmt = conn
            .prepare(
                "SELECT chunk_id FROM chunks
                 WHERE doc_id = ?
                 ORDER BY created_at ASC, chunk_id ASC",
            )
            .map_err(StoreError::from)?;
        let rows = stmt
            .query_map(params![doc_id.0], |r| r.get::<_, String>(0))
            .map_err(StoreError::from)?;
        let ids: Vec<kebab_core::ChunkId> = rows
            .map(|r| r.map(kebab_core::ChunkId))
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(StoreError::from)?;
        Ok(ids)
    }
}

// ── Internal row + (de)serialization helpers ─────────────────────────────

struct DocumentRow {
    doc_id: String,
    asset_id: String,
    workspace_path: String,
    title: Option<String>,
    lang: Option<String>,
    parser_version: String,
    doc_version: i64,
    schema_version: i64,
    metadata_json: String,
    provenance_json: String,
    // source_type / trust_level are loaded back via metadata_json round-trip,
    // so we do not need separate fields here for `get_document`.
    last_chunker_version: Option<String>,
    last_embedding_version: Option<String>,
}

fn document_row_from_sql(row: &rusqlite::Row<'_>) -> rusqlite::Result<DocumentRow> {
    Ok(DocumentRow {
        doc_id: row.get(0)?,
        asset_id: row.get(1)?,
        workspace_path: row.get(2)?,
        title: row.get(3)?,
        lang: row.get(4)?,
        // 5: source_type, 6: trust_level — read but unused here (metadata_json
        // is authoritative). Keeping them in the SELECT makes the column
        // ordering match the INSERT and allows future fields without
        // shifting indexes.
        parser_version: row.get(7)?,
        doc_version: row.get(8)?,
        schema_version: row.get(9)?,
        metadata_json: row.get(10)?,
        provenance_json: row.get(11)?,
        // 12: created_at, 13: updated_at — not stored in DocumentRow
        // (only needed for list_documents). Columns 14-15 follow.
        last_chunker_version: row.get(14)?,
        last_embedding_version: row.get(15)?,
    })
}

struct ChunkRow {
    chunk_id: String,
    doc_id: String,
    text: String,
    heading_path_json: String,
    source_spans_json: String,
    token_estimate: i64,
    chunker_version: String,
    policy_hash: String,
    block_ids_json: String,
}

fn chunk_row_from_sql(row: &rusqlite::Row<'_>) -> rusqlite::Result<ChunkRow> {
    Ok(ChunkRow {
        chunk_id: row.get(0)?,
        doc_id: row.get(1)?,
        text: row.get(2)?,
        heading_path_json: row.get(3)?,
        source_spans_json: row.get(4)?,
        token_estimate: row.get(5)?,
        chunker_version: row.get(6)?,
        policy_hash: row.get(7)?,
        block_ids_json: row.get(8)?,
    })
}

fn doc_summary_from_sql(row: &rusqlite::Row<'_>) -> rusqlite::Result<kebab_core::DocSummary> {
    let doc_id: String = row.get(0)?;
    let workspace_path: String = row.get(1)?;
    let title: Option<String> = row.get(2)?;
    let lang: Option<String> = row.get(3)?;
    let source_type_raw: String = row.get(4)?;
    let trust_level_raw: String = row.get(5)?;
    let parser_version: String = row.get(6)?;
    let created_at_raw: String = row.get(7)?;
    let updated_at_raw: String = row.get(8)?;
    let byte_len: i64 = row.get(9)?;
    let chunk_count: i64 = row.get(10)?;
    let chunker_version: Option<String> = row.get(11)?;

    // De-serialize the lowercase string forms that match
    // `#[serde(rename_all = "lowercase")]` on each enum.
    let source_type: kebab_core::SourceType =
        serde_json::from_value(serde_json::Value::String(source_type_raw))
            .map_err(|e| rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(e)))?;
    let trust_level: kebab_core::TrustLevel =
        serde_json::from_value(serde_json::Value::String(trust_level_raw))
            .map_err(|e| rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, Box::new(e)))?;
    let created_at = OffsetDateTime::parse(
        &created_at_raw,
        &time::format_description::well_known::Rfc3339,
    )
    .map_err(|e| rusqlite::Error::FromSqlConversionFailure(7, rusqlite::types::Type::Text, Box::new(e)))?;
    let updated_at = OffsetDateTime::parse(
        &updated_at_raw,
        &time::format_description::well_known::Rfc3339,
    )
    .map_err(|e| rusqlite::Error::FromSqlConversionFailure(8, rusqlite::types::Type::Text, Box::new(e)))?;

    Ok(kebab_core::DocSummary {
        doc_id: kebab_core::DocumentId(doc_id),
        doc_path: kebab_core::WorkspacePath(workspace_path),
        title: title.unwrap_or_default(),
        lang: kebab_core::Lang(lang.unwrap_or_default()),
        // tags filled in by caller after a per-doc fetch.
        tags: Vec::new(),
        trust_level,
        source_type,
        byte_len: byte_len as u64,
        chunk_count: chunk_count as u32,
        created_at,
        updated_at,
        parser_version: kebab_core::ParserVersion(parser_version),
        // chunker_version may be NULL when the doc has no chunks yet.
        // Empty string is the cleanest fallback consistent with the wire
        // schema's required `chunker_version` field on DocSummary v1.
        chunker_version: kebab_core::ChunkerVersion(chunker_version.unwrap_or_default()),
    })
}

/// Map a `QueryReturnedNoRows` into `Ok(None)` so the trait returns
/// `Option<T>` rather than an error for the common "missing" case.
fn rows_optional<T>(err: rusqlite::Error) -> rusqlite::Result<Option<T>> {
    match err {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        e => Err(e),
    }
}

/// Reconstruct a [`kebab_core::RawAsset`] from one `assets` row.
/// Row mapper for `RawAsset`. Column names are self-documenting; the
/// SELECT in [`DocumentStore::get_asset_by_workspace_path`] must include
/// all nine columns by their schema names.
fn asset_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<kebab_core::RawAsset> {
    use std::path::PathBuf;

    let asset_id: String = row.get("asset_id")?;
    let source_uri_raw: String = row.get("source_uri")?;
    let workspace_path_raw: String = row.get("workspace_path")?;
    let media_type_json: String = row.get("media_type")?;
    let byte_len: i64 = row.get("byte_len")?;
    let checksum_raw: String = row.get("checksum")?;
    let storage_kind: String = row.get("storage_kind")?;
    let storage_path_raw: String = row.get("storage_path")?;
    let discovered_at_raw: String = row.get("discovered_at")?;

    // Parse source_uri: stored as "file://<path>" or "kb://<uri>".
    let source_uri = if let Some(path_str) = source_uri_raw.strip_prefix("file://") {
        kebab_core::SourceUri::File(PathBuf::from(path_str))
    } else {
        kebab_core::SourceUri::Kb(source_uri_raw.clone())
    };

    let workspace_path = kebab_core::WorkspacePath(workspace_path_raw);
    let media_type: kebab_core::MediaType = serde_json::from_str(&media_type_json)
        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(e)))?;
    let checksum = kebab_core::Checksum(checksum_raw.clone());
    let discovered_at = time::OffsetDateTime::parse(
        &discovered_at_raw,
        &time::format_description::well_known::Rfc3339,
    )
    .map_err(|e| rusqlite::Error::FromSqlConversionFailure(8, rusqlite::types::Type::Text, Box::new(e)))?;

    let storage_path = PathBuf::from(&storage_path_raw);
    let stored = if storage_kind == "copied" {
        kebab_core::AssetStorage::Copied { path: storage_path }
    } else {
        kebab_core::AssetStorage::Reference {
            path: storage_path,
            sha: checksum.clone(),
        }
    };

    Ok(kebab_core::RawAsset {
        asset_id: kebab_core::AssetId(asset_id),
        source_uri,
        workspace_path,
        media_type,
        byte_len: u64::try_from(byte_len)
            .map_err(|e| rusqlite::Error::FromSqlConversionFailure(
                // index parameter for named-column path is unused but the
                // type still requires a number; pass 0 with a clear msg.
                0,
                rusqlite::types::Type::Integer,
                Box::new(e),
            ))?,
        checksum,
        discovered_at,
        stored,
    })
}

/// UPSERT the documents row and bump `doc_version` on conflict.
fn upsert_document(
    tx: &rusqlite::Transaction<'_>,
    doc: &kebab_core::CanonicalDocument,
) -> Result<()> {
    let metadata_json = serde_json::to_string(&doc.metadata)
        .context("serialize metadata")?;
    let provenance_json = serde_json::to_string(&doc.provenance)
        .context("serialize provenance")?;
    // String form of the lowercase serde representation. We avoid
    // embedding `serde_json::to_string` quotes (`"markdown"` → just
    // `markdown` for the column).
    let source_type = source_type_label(&doc.metadata.source_type);
    let trust_level = trust_level_label(&doc.metadata.trust_level);
    let created_at = doc
        .metadata
        .created_at
        .format(&time::format_description::well_known::Rfc3339)
        .context("format created_at")?;
    let now = OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .context("format updated_at")?;

    tx.execute(
        "INSERT INTO documents (
            doc_id, asset_id, workspace_path, title, lang,
            source_type, trust_level, parser_version,
            doc_version, schema_version, metadata_json,
            provenance_json, created_at, updated_at,
            last_chunker_version, last_embedding_version
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(doc_id) DO UPDATE SET
            asset_id              = excluded.asset_id,
            workspace_path        = excluded.workspace_path,
            title                 = excluded.title,
            lang                  = excluded.lang,
            source_type           = excluded.source_type,
            trust_level           = excluded.trust_level,
            parser_version        = excluded.parser_version,
            -- doc_version: bump on update. excluded.doc_version is the
            -- caller's submitted value; we ignore it and add 1 to the
            -- existing column so each re-ingest cleanly increments.
            doc_version           = documents.doc_version + 1,
            schema_version        = excluded.schema_version,
            metadata_json         = excluded.metadata_json,
            provenance_json       = excluded.provenance_json,
            updated_at            = excluded.updated_at,
            last_chunker_version  = excluded.last_chunker_version,
            last_embedding_version = excluded.last_embedding_version",
        params![
            doc.doc_id.0,
            doc.source_asset_id.0,
            doc.workspace_path.0,
            doc.title,
            doc.lang.0,
            source_type,
            trust_level,
            doc.parser_version.0,
            doc.doc_version as i64,
            doc.schema_version as i64,
            metadata_json,
            provenance_json,
            created_at,
            now,
            doc.last_chunker_version.as_ref().map(|v| v.0.as_str()),
            doc.last_embedding_version.as_ref().map(|v| v.0.as_str()),
        ],
    )
    .map_err(StoreError::from)?;
    Ok(())
}

fn source_type_label(s: &kebab_core::SourceType) -> &'static str {
    match s {
        kebab_core::SourceType::Markdown => "markdown",
        kebab_core::SourceType::Note => "note",
        kebab_core::SourceType::Paper => "paper",
        kebab_core::SourceType::Reference => "reference",
        kebab_core::SourceType::Inbox => "inbox",
    }
}

fn trust_level_label(s: &kebab_core::TrustLevel) -> &'static str {
    match s {
        kebab_core::TrustLevel::Primary => "primary",
        kebab_core::TrustLevel::Secondary => "secondary",
        kebab_core::TrustLevel::Generated => "generated",
    }
}

fn replace_document_tags(
    tx: &rusqlite::Transaction<'_>,
    doc_id: &kebab_core::DocumentId,
    tags: &[String],
) -> Result<()> {
    tx.execute("DELETE FROM document_tags WHERE doc_id = ?", params![doc_id.0])
        .map_err(StoreError::from)?;
    let mut stmt = tx
        .prepare(
            "INSERT INTO document_tags (doc_id, tag) VALUES (?, ?)
             ON CONFLICT(doc_id, tag) DO NOTHING",
        )
        .map_err(StoreError::from)?;
    for tag in tags {
        stmt.execute(params![doc_id.0, tag])
            .map_err(StoreError::from)?;
    }
    Ok(())
}

struct BlockRow {
    block_id: String,
    doc_id: String,
    kind: &'static str,
    heading_path_json: String,
    ordinal: i64,
    source_span_json: String,
    /// The full Block JSON — round-trip path for `get_document`. Also
    /// future-proofs new variants without schema churn.
    payload_json: String,
}

fn block_to_row(
    doc: &kebab_core::DocumentId,
    block: &kebab_core::Block,
    stream_ordinal: i64,
) -> Result<BlockRow> {
    let (block_id, kind, heading_path_json, source_span_json) = match block {
        kebab_core::Block::Heading(b) => (
            b.common.block_id.0.clone(),
            "heading",
            serde_json::to_string(&b.common.heading_path)?,
            serde_json::to_string(&b.common.source_span)?,
        ),
        kebab_core::Block::Paragraph(b) | kebab_core::Block::Quote(b) => (
            b.common.block_id.0.clone(),
            // Discriminate Paragraph vs Quote on the enum tag: payload
            // round-trip carries the variant, but the column needs a
            // stable label for filtering.
            if matches!(block, kebab_core::Block::Paragraph(_)) {
                "paragraph"
            } else {
                "quote"
            },
            serde_json::to_string(&b.common.heading_path)?,
            serde_json::to_string(&b.common.source_span)?,
        ),
        kebab_core::Block::List(b) => (
            b.common.block_id.0.clone(),
            "list",
            serde_json::to_string(&b.common.heading_path)?,
            serde_json::to_string(&b.common.source_span)?,
        ),
        kebab_core::Block::Code(b) => (
            b.common.block_id.0.clone(),
            "code",
            serde_json::to_string(&b.common.heading_path)?,
            serde_json::to_string(&b.common.source_span)?,
        ),
        kebab_core::Block::Table(b) => (
            b.common.block_id.0.clone(),
            "table",
            serde_json::to_string(&b.common.heading_path)?,
            serde_json::to_string(&b.common.source_span)?,
        ),
        kebab_core::Block::ImageRef(b) => (
            b.common.block_id.0.clone(),
            "imageref",
            serde_json::to_string(&b.common.heading_path)?,
            serde_json::to_string(&b.common.source_span)?,
        ),
        kebab_core::Block::AudioRef(b) => (
            b.common.block_id.0.clone(),
            "audioref",
            serde_json::to_string(&b.common.heading_path)?,
            serde_json::to_string(&b.common.source_span)?,
        ),
    };
    let payload_json = serde_json::to_string(block).context("serialize block")?;
    Ok(BlockRow {
        block_id,
        doc_id: doc.0.clone(),
        kind,
        heading_path_json,
        ordinal: stream_ordinal,
        source_span_json,
        payload_json,
    })
}
