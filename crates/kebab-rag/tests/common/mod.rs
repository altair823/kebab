//! Shared scaffolding for kb-rag tests.
//!
//! Provides:
//! - [`RagEnv`] — a tempdir-backed `SqliteStore` with helpers to seed
//!   asset/document/chunk rows directly via SQL (so the test crate's
//!   deps stay inside the allowed list).
//! - [`MockRetriever`] — returns canned `Vec<SearchHit>` regardless of
//!   the query, so the pipeline exercise is independent of any real
//!   indexer.
//! - small helpers to build `Citation` / `SearchHit` / canned LM
//!   responses without rewriting boilerplate in every test.

#![allow(dead_code)]

use std::sync::Arc;

use kebab_config::Config;
use kebab_core::{
    ChunkerVersion, ChunkId, Citation, DocumentId, IndexVersion, RetrievalDetail,
    Retriever, SearchHit, SearchMode, SearchQuery, WorkspacePath,
};
use kebab_store_sqlite::SqliteStore;
use rusqlite::params;
use tempfile::TempDir;

/// Tempdir-backed test environment. Holds an open `SqliteStore` with
/// V001 + V002 + V003 migrations applied so chunk reads work end-to-end.
pub struct RagEnv {
    pub temp: TempDir,
    pub config: Config,
    pub sqlite: Arc<SqliteStore>,
}

impl RagEnv {
    pub fn new() -> Self {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut config = Config::defaults();
        config.storage.data_dir = temp.path().to_string_lossy().into_owned();
        let sqlite = SqliteStore::open(&config).unwrap();
        sqlite.run_migrations().unwrap();
        Self {
            temp,
            config,
            sqlite: Arc::new(sqlite),
        }
    }

    /// Seed the minimal (assets, documents, chunks) row triple needed
    /// for `DocumentStore::get_chunk` to round-trip in tests.
    /// `chunk_id` / `doc_id` must already be 32-hex-char shaped (use
    /// [`id32`] to pad short prefixes).
    pub fn seed_chunk(
        &self,
        chunk_id: &str,
        doc_id: &str,
        workspace_path: &str,
        text: &str,
        heading_path: &[&str],
    ) {
        let asset_id = format!("a{}", &doc_id[..31]);
        let conn = self.sqlite.read_conn();
        conn.execute(
            "INSERT OR IGNORE INTO assets (
                asset_id, source_uri, workspace_path, media_type, byte_len,
                checksum, storage_kind, storage_path, discovered_at
             ) VALUES (?, ?, ?, '\"markdown\"', 0,
                       'deadbeefdeadbeefdeadbeefdeadbeef',
                       'reference', ?, '1970-01-01T00:00:00Z')",
            params![
                asset_id,
                format!("file://{workspace_path}"),
                workspace_path,
                workspace_path,
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO documents (
                doc_id, asset_id, workspace_path, title, lang, source_type,
                trust_level, parser_version, doc_version, schema_version,
                metadata_json, provenance_json, created_at, updated_at
             ) VALUES (?, ?, ?, NULL, 'en', 'markdown', 'primary', 'v1', 1, 1,
                       '{}', '{}', '1970-01-01T00:00:00Z', '1970-01-01T00:00:00Z')",
            params![doc_id, asset_id, workspace_path],
        )
        .unwrap();
        let heading_json = serde_json::to_string(heading_path).unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO chunks (
                chunk_id, doc_id, text, heading_path_json, section_label,
                source_spans_json, token_estimate, chunker_version,
                policy_hash, block_ids_json, created_at
             ) VALUES (?, ?, ?, ?, NULL,
                       '[{\"kind\":\"line\",\"start\":1,\"end\":3}]',
                       1, 'v1', 'h', '[]', '1970-01-01T00:00:00Z')",
            params![chunk_id, doc_id, text, heading_json],
        )
        .unwrap();
    }

    /// Count rows in `answers`. Tests use this to assert that every
    /// `ask` (incl. refusals) writes exactly one row.
    pub fn count_answers(&self) -> i64 {
        let conn = self.sqlite.read_conn();
        conn.query_row("SELECT COUNT(*) FROM answers", [], |r| r.get(0))
            .unwrap()
    }
}

/// Build a `SearchHit` with canned scores. Citation defaults to a
/// `Line { 1..=3 }` over `workspace_path`.
pub fn mk_hit(
    rank: u32,
    chunk_id: &str,
    doc_id: &str,
    workspace_path: &str,
    fusion_score: f32,
    heading: &[&str],
) -> SearchHit {
    mk_hit_with_indexed_at(
        rank,
        chunk_id,
        doc_id,
        workspace_path,
        fusion_score,
        heading,
        time::OffsetDateTime::UNIX_EPOCH,
    )
}

/// Build a `SearchHit` with an explicit `indexed_at` timestamp. Used by
/// p9-fb-32 staleness tests so the pipeline sees realistic per-hit
/// indexed_at values flowing through to `AnswerCitation`.
pub fn mk_hit_with_indexed_at(
    rank: u32,
    chunk_id: &str,
    doc_id: &str,
    workspace_path: &str,
    fusion_score: f32,
    heading: &[&str],
    indexed_at: time::OffsetDateTime,
) -> SearchHit {
    let p = WorkspacePath::new(workspace_path.to_string()).expect("workspace path valid");
    SearchHit {
        rank,
        chunk_id: ChunkId(chunk_id.to_string()),
        doc_id: DocumentId(doc_id.to_string()),
        doc_path: p.clone(),
        heading_path: heading.iter().map(|s| s.to_string()).collect(),
        section_label: None,
        snippet: "snippet".to_string(),
        citation: Citation::Line {
            path: p,
            start: 1,
            end: 3,
            section: None,
        },
        retrieval: RetrievalDetail {
            method: SearchMode::Lexical,
            fusion_score,
            lexical_score: Some(fusion_score),
            vector_score: None,
            lexical_rank: Some(rank),
            vector_rank: None,
        },
        index_version: IndexVersion("test-iv".to_string()),
        embedding_model: None,
        chunker_version: ChunkerVersion("v1".to_string()),
        // p9-fb-32: pipeline post-processes `stale` from `indexed_at`
        // + cfg threshold; tests configure both via this helper.
        indexed_at,
        stale: false,
        score_kind: kebab_core::ScoreKind::Rrf,
    }
}

/// Mock retriever that returns a fixed `Vec<SearchHit>` regardless of
/// the query / k / filters. Captures the invocation count for assertions.
pub struct MockRetriever {
    pub hits: Vec<SearchHit>,
    pub calls: std::sync::atomic::AtomicUsize,
}

impl MockRetriever {
    pub fn new(hits: Vec<SearchHit>) -> Self {
        Self {
            hits,
            calls: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    pub fn calls(&self) -> usize {
        self.calls.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl Retriever for MockRetriever {
    fn search(&self, _q: &SearchQuery) -> anyhow::Result<Vec<SearchHit>> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(self.hits.clone())
    }
    fn index_version(&self) -> IndexVersion {
        IndexVersion("test-iv".to_string())
    }
}

/// Pad a short prefix to the 32-hex shape `kebab_core` newtypes expect.
pub fn id32(prefix: &str) -> String {
    let mut s = prefix.to_string();
    while s.len() < 32 {
        s.push('0');
    }
    s.truncate(32);
    s
}
