//! Shared scaffolding for kb-search hybrid integration tests.
//!
//! # Test policy
//!
//! Integration tests in `hybrid.rs` that touch `LanceVectorStore`
//! are marked `#[ignore]` AND call [`require_avx_or_panic`] inside
//! the test body so a `--ignored` invocation on a non-AVX host
//! fails loudly with a clear message rather than crashing later
//! inside Lance's f32 SIMD kernel with `SIGILL`.
//!
//! See `crates/kb-store-vector/tests/common/mod.rs` for the
//! original P3-3 rationale; this is a copy because that crate's
//! test commons are test-only and not part of its public surface.

#![allow(dead_code)]

use std::sync::Arc;

use kebab_config::Config;
use kebab_core::{
    ChunkId, DocumentId, EmbeddingId, EmbeddingInput, EmbeddingKind, EmbeddingModelId,
    EmbeddingVersion, IndexVersion, MediaType, Retriever, SearchFilters, SearchHit, SearchMode,
    SearchQuery, VectorRecord, VectorStore,
};
use kebab_embed::{Embedder, MockEmbedder};
use kebab_search::{LexicalRetriever, VectorRetriever};
use kebab_store_sqlite::SqliteStore;
use kebab_store_vector::LanceVectorStore;
use rusqlite::params;
use tempfile::TempDir;

/// Panic if the host CPU lacks AVX. Called from every `#[ignore]`-d
/// integration test body so that `cargo test -- --ignored` on a
/// non-AVX host fails loudly with a clear message instead of crashing
/// later inside a Lance SIMD kernel with `SIGILL`.
pub fn require_avx_or_panic() {
    #[cfg(target_arch = "x86_64")]
    {
        assert!(
            std::is_x86_feature_detected!("avx"),
            "kb-search hybrid integration test requires AVX-capable hardware; \
             host CPU lacks AVX. Run on an AVX-capable machine."
        );
    }
}

/// Index version label used by hybrid integration tests so the
/// `index_version()` composite token is predictable in snapshots.
pub const TEST_LEX_INDEX_VERSION: &str = "v1.0-lex";
pub const TEST_VEC_INDEX_VERSION: &str = "v1.0-vec";

/// Embedding dimensions for tests. Kept small so MockEmbedder runs
/// fast and the Lance table stays compact on disk; production uses
/// 384 (multilingual-e5-small) but the retriever code is dim-agnostic.
pub const TEST_DIMENSIONS: usize = 16;
pub const TEST_MODEL_ID: &str = "mock-e5";

pub struct HybridEnv {
    pub temp: TempDir,
    pub config: Config,
    pub sqlite: Arc<SqliteStore>,
    pub vector_store: Arc<LanceVectorStore>,
    pub embedder: Arc<MockEmbedder>,
}

impl HybridEnv {
    pub fn new() -> Self {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut config = Config::defaults();
        config.storage.data_dir = temp.path().to_string_lossy().into_owned();
        let sqlite = SqliteStore::open(&config.storage).unwrap();
        sqlite.run_migrations().unwrap();
        let sqlite = Arc::new(sqlite);
        let vector_store = Arc::new(LanceVectorStore::new(&config.storage, sqlite.clone()).unwrap());
        let embedder = Arc::new(MockEmbedder::new(
            EmbeddingModelId(TEST_MODEL_ID.to_string()),
            EmbeddingVersion("v1".to_string()),
            TEST_DIMENSIONS,
        ));
        Self {
            temp,
            config,
            sqlite,
            vector_store,
            embedder,
        }
    }

    /// Build a `LexicalRetriever` over the shared SQLite store.
    pub fn lexical_retriever(&self) -> LexicalRetriever {
        LexicalRetriever::new(
            Arc::clone(&self.sqlite),
            IndexVersion(TEST_LEX_INDEX_VERSION.to_string()),
        )
    }

    /// Build a `VectorRetriever` over the shared LanceVectorStore +
    /// MockEmbedder + SQLite store.
    pub fn vector_retriever(&self) -> VectorRetriever {
        let store: Arc<dyn VectorStore + Send + Sync> =
            Arc::clone(&self.vector_store) as Arc<dyn VectorStore + Send + Sync>;
        let embed: Arc<dyn Embedder> = Arc::clone(&self.embedder) as Arc<dyn Embedder>;
        VectorRetriever::new(
            store,
            embed,
            Arc::clone(&self.sqlite),
            IndexVersion(TEST_VEC_INDEX_VERSION.to_string()),
            self.config.search.snippet_chars,
        )
    }

    /// Insert (asset, document, document_tags, chunk) rows directly.
    /// We seed without going through `DocumentStore::put_document`
    /// to keep this crate's test deps inside the Allowed list (no
    /// `kb-parse-md` / `kb-normalize` / `kb-chunk`). The `chunks` row
    /// also fires the V002 FTS5 triggers, so the lexical retriever
    /// can find the row by `MATCH` without a manual rebuild.
    pub fn seed_chunk(
        &self,
        chunk_id: &str,
        doc_id: &str,
        workspace_path: &str,
        text: &str,
        heading_path: &[&str],
        tags: &[&str],
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
        for t in tags {
            conn.execute(
                "INSERT OR IGNORE INTO document_tags (doc_id, tag) VALUES (?, ?)",
                params![doc_id, t],
            )
            .unwrap();
        }
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

    /// High-level helper: seed a doc with the default media type
    /// (Markdown) and embed its text. Returns the `DocumentId` so
    /// callers can use it in `doc_id` filter tests.
    pub fn insert_doc(&self, path: &str, text: &str) -> DocumentId {
        self.insert_doc_with_media(path, text, MediaType::Markdown)
    }

    /// High-level helper: seed a doc with an explicit `MediaType`.
    /// The `media_type` is serialized to JSON (mirrors how
    /// `DocumentStore::put_document` writes it) and stored in `assets`.
    pub fn insert_doc_with_media(&self, path: &str, text: &str, media: MediaType) -> DocumentId {
        // Derive deterministic IDs from the path so repeated calls with
        // the same path are idempotent (INSERT OR IGNORE).
        let path_hash: String = {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut h = DefaultHasher::new();
            path.hash(&mut h);
            format!("{:032x}", h.finish())
        };
        let doc_id = format!("d{}", &path_hash[..31]);
        let chunk_id = format!("c{}", &path_hash[..31]);
        let asset_id = format!("a{}", &path_hash[..31]);

        let media_json = serde_json::to_string(&media).expect("serialize MediaType");
        let conn = self.sqlite.read_conn();
        conn.execute(
            "INSERT OR IGNORE INTO assets (
                asset_id, source_uri, workspace_path, media_type, byte_len,
                checksum, storage_kind, storage_path, discovered_at
             ) VALUES (?, ?, ?, ?, 0,
                       'deadbeefdeadbeefdeadbeefdeadbeef',
                       'reference', ?, '1970-01-01T00:00:00Z')",
            params![asset_id, format!("file:///{path}"), path, media_json, path,],
        )
        .unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO documents (
                doc_id, asset_id, workspace_path, title, lang, source_type,
                trust_level, parser_version, doc_version, schema_version,
                metadata_json, provenance_json, created_at, updated_at
             ) VALUES (?, ?, ?, NULL, 'en', 'markdown', 'primary', 'v1', 1, 1,
                       '{}', '{}', '1970-01-01T00:00:00Z', '1970-01-01T00:00:00Z')",
            params![doc_id, asset_id, path],
        )
        .unwrap();
        let heading_json = "[]";
        conn.execute(
            "INSERT OR IGNORE INTO chunks (
                chunk_id, doc_id, text, heading_path_json, section_label,
                source_spans_json, token_estimate, chunker_version,
                policy_hash, block_ids_json, created_at
             ) VALUES (?, ?, ?, ?, NULL,
                       '[{\"kind\":\"line\",\"start\":1,\"end\":1}]',
                       1, 'v1', 'h', '[]', '1970-01-01T00:00:00Z')",
            params![chunk_id, doc_id, text, heading_json],
        )
        .unwrap();
        drop(conn);
        self.embed_and_upsert(&chunk_id, &doc_id, text, &[]);
        DocumentId(doc_id)
    }

    /// Run a `SearchMode::Vector` query against the seeded corpus and
    /// return the resulting `Vec<SearchHit>`.
    pub fn run_vector_search(&self, query: &str, filters: &SearchFilters) -> Vec<SearchHit> {
        let r = self.vector_retriever();
        let q = SearchQuery {
            text: query.to_string(),
            mode: SearchMode::Vector,
            k: 10,
            filters: filters.clone(),
        };
        r.search(&q).expect("vector search")
    }

    /// Embed `text` as a Document and upsert it as the embedding for
    /// `chunk_id`. Drives the same code path production uses:
    /// MockEmbedder → VectorRecord → LanceVectorStore::upsert →
    /// embedding_records committed.
    pub fn embed_and_upsert(
        &self,
        chunk_id: &str,
        doc_id: &str,
        text: &str,
        heading_path: &[&str],
    ) {
        let inputs = [EmbeddingInput {
            text,
            kind: EmbeddingKind::Document,
        }];
        let mut vecs = self.embedder.embed(&inputs).unwrap();
        let vector = vecs.remove(0);
        let record = VectorRecord {
            chunk_id: ChunkId(chunk_id.to_string()),
            embedding_id: EmbeddingId(format!("e{}", &chunk_id[..31])),
            vector,
            doc_id: DocumentId(doc_id.to_string()),
            text: text.to_string(),
            heading_path: heading_path
                .iter()
                .map(std::string::ToString::to_string)
                .collect(),
            model_id: EmbeddingModelId(TEST_MODEL_ID.to_string()),
            model_version: EmbeddingVersion("v1".to_string()),
            dimensions: TEST_DIMENSIONS,
        };
        self.vector_store.upsert(&[record]).unwrap();
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
