//! Shared scaffolding for kb-store-vector integration tests.
//!
//! # Test policy
//!
//! Integration tests in this crate are marked `#[ignore]` and require
//! AVX-capable hardware. They are excluded from the default `cargo
//! test -p kb-store-vector` lane and only run when explicitly opted
//! in:
//!
//! ```text
//! cargo test -p kb-store-vector -- --ignored
//! ```
//!
//! The reason: LanceDB's f32 SIMD path uses unconditional AVX
//! intrinsics (`__m256` in `lance-linalg::simd::f32`). On x86_64
//! CPUs without AVX support — notably QEMU's default `qemu64` model
//! in CI sandboxes and some bare-metal dev boxes — those instructions
//! trigger `SIGILL: illegal instruction` at the first `vector_search`
//! call. Rather than silently turn that into a "passing" test (which
//! it isn't), we gate the integration suite behind `#[ignore]` and
//! call [`require_avx_or_panic`] inside each test body so that an
//! `--ignored` invocation on a non-AVX host fails loudly rather than
//! crashing later inside a Lance kernel.
//!
//! This mirrors P3-2's `#[ignore]` policy on tests that require a
//! model download — both are CI-lane decisions, not silent skips.
//!
//! Each test owns a `TempDir` (vector_dir + sqlite db live underneath
//! it), a fully-migrated `SqliteStore`, and a `LanceVectorStore`
//! pointed at both. We seed `documents` / `chunks` rows directly via
//! SQL (rather than going through `DocumentStore::put_document`) so
//! the tests stay independent of kb-parse-md / kb-normalize / kb-chunk
//! and so we can construct adversarial fixtures (filtered tags,
//! mismatched langs) without reproducing a Markdown round-trip.

#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::Arc;

/// Panic if the host CPU lacks AVX. Called from every `#[ignore]`-d
/// integration test body so that `cargo test -- --ignored` on a
/// non-AVX host fails loudly with a clear message instead of crashing
/// later inside a Lance SIMD kernel with `SIGILL`.
///
/// On non-x86_64 hosts this is a no-op (Lance's AVX requirement is
/// x86-only — ARM/Apple Silicon paths use different intrinsics that
/// the workspace doesn't currently target).
pub fn require_avx_or_panic() {
    #[cfg(target_arch = "x86_64")]
    {
        if !std::is_x86_feature_detected!("avx") {
            panic!(
                "kb-store-vector integration test requires AVX-capable hardware; \
                 host CPU lacks AVX. Run on an AVX-capable machine. \
                 See crates/kb-store-vector/tests/common/mod.rs."
            );
        }
    }
}

use kebab_config::Config;
use kebab_core::{
    ChunkId, DocumentId, EmbeddingId, EmbeddingModelId, EmbeddingVersion, VectorRecord,
};
use kebab_store_sqlite::SqliteStore;
use kebab_store_vector::LanceVectorStore;
use rusqlite::params;
use tempfile::TempDir;

pub struct TestEnv {
    pub temp: TempDir,
    pub config: Config,
    pub sqlite: Arc<SqliteStore>,
    pub vector: LanceVectorStore,
}

impl TestEnv {
    pub fn new() -> Self {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut config = Config::defaults();
        config.storage.data_dir = temp.path().to_string_lossy().into_owned();
        let sqlite = SqliteStore::open(&config).unwrap();
        sqlite.run_migrations().unwrap();
        let sqlite = Arc::new(sqlite);
        let vector = LanceVectorStore::new(&config, sqlite.clone()).unwrap();
        Self {
            temp,
            config,
            sqlite,
            vector,
        }
    }

    pub fn data_dir(&self) -> PathBuf {
        self.temp.path().to_path_buf()
    }

    /// Insert minimum (asset, document, chunk) rows so phase-1
    /// embedding_records inserts don't trip the FK to chunks /
    /// documents.
    pub fn seed_chunk(
        &self,
        chunk_id: &str,
        doc_id: &str,
        workspace_path: &str,
        lang: &str,
        tags: &[&str],
        trust_level: &str,
    ) {
        // Asset id derived from doc_id deterministically — every
        // chunk gets its own asset to keep things simple.
        let asset_id = format!("a{}", &doc_id[..31]);
        let conn = self.sqlite.read_conn();
        conn.execute(
            "INSERT OR IGNORE INTO assets (
                asset_id, source_uri, workspace_path, media_type, byte_len,
                checksum, storage_kind, storage_path, discovered_at
             ) VALUES (?, ?, ?, ?, 0, ?, 'reference', ?, '1970-01-01T00:00:00Z')",
            params![
                asset_id,
                format!("file://{workspace_path}"),
                workspace_path,
                "{}",
                "deadbeefdeadbeefdeadbeefdeadbeef",
                workspace_path,
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO documents (
                doc_id, asset_id, workspace_path, title, lang, source_type,
                trust_level, parser_version, doc_version, schema_version,
                metadata_json, provenance_json, created_at, updated_at
             ) VALUES (?, ?, ?, NULL, ?, 'markdown', ?, 'v1', 1, 1, '{}', '{}',
                       '1970-01-01T00:00:00Z', '1970-01-01T00:00:00Z')",
            params![doc_id, asset_id, workspace_path, lang, trust_level],
        )
        .unwrap();
        for t in tags {
            conn.execute(
                "INSERT OR IGNORE INTO document_tags (doc_id, tag) VALUES (?, ?)",
                params![doc_id, t],
            )
            .unwrap();
        }
        conn.execute(
            "INSERT OR IGNORE INTO chunks (
                chunk_id, doc_id, text, heading_path_json, section_label,
                source_spans_json, token_estimate, chunker_version,
                policy_hash, block_ids_json, created_at
             ) VALUES (?, ?, 'hi', '[]', NULL, '[]', 1, 'v1', 'h', '[]', '1970-01-01T00:00:00Z')",
            params![chunk_id, doc_id],
        )
        .unwrap();
    }
}

/// Build a deterministic test VectorRecord from a few simple inputs.
/// `vector` is taken verbatim, `dimensions` is set from `vector.len()`.
pub fn make_record(
    chunk_idx: u8,
    doc_idx: u8,
    vector: Vec<f32>,
    text: &str,
    heading: &[&str],
    model: &str,
) -> VectorRecord {
    let dim = vector.len();
    let chunk_id = ChunkId(format!("{:032x}", 0x1100u32 + chunk_idx as u32));
    let doc_id = DocumentId(format!("{:032x}", 0xd0c0u32 + doc_idx as u32));
    let embedding_id =
        EmbeddingId(format!("{:032x}", 0xeeee0000u32 + chunk_idx as u32));
    VectorRecord {
        chunk_id,
        embedding_id,
        vector,
        doc_id,
        text: text.to_string(),
        heading_path: heading.iter().map(|s| s.to_string()).collect(),
        model_id: EmbeddingModelId(model.to_string()),
        model_version: EmbeddingVersion("v1".to_string()),
        dimensions: dim,
    }
}
