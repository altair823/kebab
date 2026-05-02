//! `truncate_embedding_records` wipes every row regardless of status.
//!
//! Used by `kebab reset --vector-only` to keep SQLite in sync after the
//! Lance vector store is deleted off-disk. The helper is exposed at the
//! integration-test boundary so consumers (kebab-app's reset module) can
//! verify its semantics without reaching into private store internals.

use kebab_config::Config;
use kebab_store_sqlite::{EmbeddingRecordRow, SqliteStore};
use rusqlite::params;
use tempfile::TempDir;
use time::OffsetDateTime;

fn config_for(tmp: &TempDir) -> Config {
    let mut c = Config::defaults();
    c.storage.data_dir = tmp.path().to_string_lossy().into_owned();
    c
}

fn open_store(tmp: &TempDir) -> SqliteStore {
    let cfg = config_for(tmp);
    let store = SqliteStore::open(&cfg).unwrap();
    store.run_migrations().unwrap();
    store
}

/// Seed an asset + document + chunk so an `embedding_records` row inserted
/// against `chunk_id` does not violate the chunks FK. Mirrors the helper
/// used by the in-crate `embeddings::tests` module — copied here because
/// integration tests cannot reach the private `seed_chunk` from outside
/// the crate.
fn seed_chunk(store: &SqliteStore, chunk_id: &str) {
    let conn = store.read_conn();
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

fn count_rows(store: &SqliteStore) -> i64 {
    let conn = store.read_conn();
    conn.query_row("SELECT COUNT(*) FROM embedding_records", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn truncate_removes_all_rows_and_returns_count() {
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
    assert_eq!(count_rows(&store), 1);

    let removed = store.truncate_embedding_records().unwrap();
    assert_eq!(removed, 1);
    assert_eq!(count_rows(&store), 0);
}

#[test]
fn truncate_on_empty_table_is_noop() {
    let tmp = TempDir::new().unwrap();
    let store = open_store(&tmp);
    let removed = store.truncate_embedding_records().unwrap();
    assert_eq!(removed, 0);
    assert_eq!(count_rows(&store), 0);
}
