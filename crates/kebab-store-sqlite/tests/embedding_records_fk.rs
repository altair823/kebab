//! V011: `embedding_records.chunk_id` FK 제거 + CASCADE 대체 명시 DELETE.
//!
//! 별칭 dense 벡터는 sentinel chunk_id(`{orig}#alias`)로 색인되는데, 이 id 는
//! `chunks` 에 행이 없다. V001 의 `chunk_id REFERENCES chunks ON DELETE CASCADE`
//! FK 가 살아 있으면 sentinel `embedding_records` INSERT 가 SQLite 787 로 실패한다.
//! V011 이 FK 를 제거하고, 사라진 CASCADE 는 `put_chunks` / purge 경로의 명시
//! DELETE 로 대체한다(설계 spec 2026-05-30-dense-alias-vectors-design.md §3.5).

use kebab_config::Config;
use kebab_core::{
    Chunk, ChunkId, ChunkerVersion, DocumentId, DocumentStore,
};
use kebab_store_sqlite::{EmbeddingRecordRow, SqliteStore};
use rusqlite::params;
use tempfile::TempDir;
use time::OffsetDateTime;

fn open_store(tmp: &TempDir) -> SqliteStore {
    let mut c = Config::defaults();
    c.storage.data_dir = tmp.path().to_string_lossy().into_owned();
    let store = SqliteStore::open(&c).unwrap();
    store.run_migrations().unwrap();
    store
}

const DOC_ID: &str = "fedcba9876543210fedcba9876543210";

/// Seed asset + document + one chunk so the *original* chunk_id has a
/// `chunks` row. The sentinel `{chunk_id}#alias` deliberately gets NO
/// chunks row — that is the case V011 must allow.
fn seed_chunk(store: &SqliteStore, chunk_id: &str) {
    let conn = store.read_conn();
    conn.execute(
        "INSERT INTO assets (
            asset_id, source_uri, workspace_path, media_type, byte_len,
            checksum, storage_kind, storage_path, discovered_at
         ) VALUES (?, ?, ?, '{}', 0, 'deadbeefdeadbeefdeadbeefdeadbeef',
                   'reference', '/tmp/x', '1970-01-01T00:00:00Z')",
        params!["0123456789abcdef0123456789abcdef", "file:///tmp/x", "x.md"],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO documents (
            doc_id, asset_id, workspace_path, title, lang, source_type,
            trust_level, parser_version, doc_version, schema_version,
            metadata_json, provenance_json, created_at, updated_at
         ) VALUES (?, ?, 'x.md', NULL, 'en', 'markdown', 'primary', 'v1', 1, 1,
                   '{}', '{}', '1970-01-01T00:00:00Z', '1970-01-01T00:00:00Z')",
        params![DOC_ID, "0123456789abcdef0123456789abcdef"],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO chunks (
            chunk_id, doc_id, text, heading_path_json, section_label,
            source_spans_json, token_estimate, chunker_version,
            policy_hash, block_ids_json, created_at
         ) VALUES (?, ?, 'hi', '[]', NULL, '[]', 1, 'v1', 'h', '[]',
                   '1970-01-01T00:00:00Z')",
        params![chunk_id, DOC_ID],
    )
    .unwrap();
}

fn embed_row(embedding_id: &str, chunk_id: &str) -> EmbeddingRecordRow {
    EmbeddingRecordRow {
        embedding_id: embedding_id.to_string(),
        chunk_id: chunk_id.to_string(),
        model_id: "m".to_string(),
        model_version: "v1".to_string(),
        dimensions: 4,
        lance_table: "t".to_string(),
        created_at: OffsetDateTime::UNIX_EPOCH,
    }
}

fn embed_count(store: &SqliteStore, chunk_id: &str) -> i64 {
    let conn = store.read_conn();
    conn.query_row(
        "SELECT COUNT(*) FROM embedding_records WHERE chunk_id = ?",
        params![chunk_id],
        |r| r.get::<_, i64>(0),
    )
    .unwrap()
}

/// Count embedding rows whose chunk_id begins with `prefix`. Used to
/// assert that *every* per-alias sentinel (`{id}#alias#0`, `#alias#1`, …)
/// is gone, not just the legacy single `{id}#alias`.
fn embed_count_prefix(store: &SqliteStore, prefix: &str) -> i64 {
    let conn = store.read_conn();
    conn.query_row(
        "SELECT COUNT(*) FROM embedding_records WHERE chunk_id LIKE ? || '%'",
        params![prefix],
        |r| r.get::<_, i64>(0),
    )
    .unwrap()
}

/// V011 후 sentinel chunk_id(`chunks` 에 없는 id)로 `embedding_records` 를
/// INSERT 해도 FK 위반 없이 성공해야 한다.
#[test]
fn sentinel_embedding_record_insert_succeeds_without_fk() {
    let tmp = TempDir::new().unwrap();
    let store = open_store(&tmp);
    let c1 = "11111111111111111111111111111111";
    seed_chunk(&store, c1);

    // sentinel: chunks 에 행이 없는 `{c1}#alias`.
    let sentinel = format!("{c1}{}", kebab_core::ALIAS_SUFFIX);
    let result =
        store.put_embedding_records_pending(&[embed_row("e_sentinel_0000000000000000000000", &sentinel)]);
    assert!(
        result.is_ok(),
        "sentinel embedding_records insert must not violate a chunks FK after V011: {result:?}"
    );
    assert_eq!(
        embed_count(&store, &sentinel),
        1,
        "sentinel embedding row must be persisted"
    );
}

/// `put_chunks` 재호출(재인제스트) 시, 명시 DELETE 가 그 doc 의 원본 + sentinel
/// `embedding_records` 를 모두 정리해 orphan 0 이 되어야 한다(CASCADE 대체).
#[test]
fn put_chunks_cleans_original_and_sentinel_embeddings() {
    let tmp = TempDir::new().unwrap();
    let store = open_store(&tmp);
    let c1 = "11111111111111111111111111111111";
    seed_chunk(&store, c1);
    let sentinel = format!("{c1}{}", kebab_core::ALIAS_SUFFIX);

    // 원본 + sentinel embedding_records 색인 (committed).
    store
        .put_embedding_records_pending(&[
            embed_row("e_orig_000000000000000000000000000", c1),
            embed_row("e_sentinel_0000000000000000000000", &sentinel),
        ])
        .unwrap();
    store
        .mark_embedding_records_committed(&[
            "e_orig_000000000000000000000000000".to_string(),
            "e_sentinel_0000000000000000000000".to_string(),
        ])
        .unwrap();
    assert_eq!(embed_count(&store, c1), 1);
    assert_eq!(embed_count(&store, &sentinel), 1);

    // 재인제스트: 같은 chunk 를 put_chunks 로 다시 쓴다. 명시 DELETE 가
    // 원본 + sentinel embedding_records 를 정리한 뒤 chunk 재삽입.
    let doc_id = DocumentId(DOC_ID.to_string());
    let chunk = Chunk {
        chunk_id: ChunkId(c1.to_string()),
        doc_id: doc_id.clone(),
        block_ids: Vec::new(),
        text: "hi".to_string(),
        heading_path: Vec::new(),
        source_spans: Vec::new(),
        token_estimate: 1,
        chunker_version: ChunkerVersion("v1".to_string()),
        policy_hash: "h".to_string(),
        tokenized_korean_text: None,
        aliases: None,
    };
    store.put_chunks(&doc_id, std::slice::from_ref(&chunk)).unwrap();

    assert_eq!(
        embed_count(&store, c1),
        0,
        "original embedding_records must be cleaned on re-ingest (CASCADE replacement)"
    );
    assert_eq!(
        embed_count(&store, &sentinel),
        0,
        "sentinel embedding_records must be cleaned on re-ingest (no chunks FK → explicit DELETE)"
    );
}

/// Task 4.5 리뷰 MAJOR: `purge_document_at_workspace_path_except_doc_id`
/// (parser-bump 재인제스트 경로)도 원본 + sentinel embedding_records 를
/// 명시 DELETE 로 정리해 orphan 0 이어야 한다. (이 경로 누락 시 tombstone 누적.)
#[test]
fn purge_except_doc_id_cleans_original_and_sentinel_embeddings() {
    let tmp = TempDir::new().unwrap();
    let store = open_store(&tmp);
    let c1 = "11111111111111111111111111111111";
    seed_chunk(&store, c1); // doc DOC_ID @ workspace 'x.md'
    let sentinel = format!("{c1}{}", kebab_core::ALIAS_SUFFIX);

    store
        .put_embedding_records_pending(&[
            embed_row("e_orig_000000000000000000000000000", c1),
            embed_row("e_sentinel_0000000000000000000000", &sentinel),
        ])
        .unwrap();
    store
        .mark_embedding_records_committed(&[
            "e_orig_000000000000000000000000000".to_string(),
            "e_sentinel_0000000000000000000000".to_string(),
        ])
        .unwrap();
    assert_eq!(embed_count(&store, c1), 1);
    assert_eq!(embed_count(&store, &sentinel), 1);

    // workspace 'x.md' 에서 DOC_ID(=현재 문서) 외 문서만 보존 → DOC_ID 가
    // 삭제 대상(parser-bump: 같은 path 의 옛 doc_id 정리). keep_doc_id 를
    // DOC_ID 와 다른 값으로 주면 DOC_ID 문서 + 그 chunk embedding 이 정리돼야.
    store
        .purge_document_at_workspace_path_except_doc_id("x.md", "0000000000000000000000000000ffff")
        .unwrap();

    assert_eq!(
        embed_count(&store, c1),
        0,
        "purge_except_doc_id: 원본 embedding_records 정리 (CASCADE 대체)"
    );
    assert_eq!(
        embed_count(&store, &sentinel),
        0,
        "purge_except_doc_id: sentinel embedding_records 정리 (chunks FK 없음 → 명시 DELETE)"
    );
}

/// Seed body chunk + its per-line alias sentinel embedding rows
/// (`{c1}#alias#0`, `{c1}#alias#1`) plus the legacy `{c1}#alias`. Returns
/// the chunk's bare id. Used by the PR #195 per-alias orphan regressions.
fn seed_body_and_alias_sentinels(store: &SqliteStore, c1: &str) {
    seed_chunk(store, c1);
    store
        .put_embedding_records_pending(&[
            embed_row("e_orig_000000000000000000000000000", c1),
            embed_row("e_alias0_00000000000000000000000", &format!("{c1}#alias#0")),
            embed_row("e_alias1_00000000000000000000000", &format!("{c1}#alias#1")),
            // legacy single sentinel (docs ingested before per-line split).
            embed_row("e_alias_legacy_00000000000000000", &format!("{c1}#alias")),
        ])
        .unwrap();
    store
        .mark_embedding_records_committed(&[
            "e_orig_000000000000000000000000000".to_string(),
            "e_alias0_00000000000000000000000".to_string(),
            "e_alias1_00000000000000000000000".to_string(),
            "e_alias_legacy_00000000000000000".to_string(),
        ])
        .unwrap();
}

/// PR #195 MAJOR regression: alias dense 벡터가 단일 `{id}#alias` 에서 줄별
/// `{id}#alias#0`, `#alias#1`, … 로 바뀐 뒤, `put_chunks` 재인제스트 시 명시
/// DELETE 가 본문 + **모든** per-alias sentinel embedding_records 를 정리해야
/// 한다. 이전 코드(`|| '#alias'` 정확 일치)는 `#alias#N` 을 놓쳐 누수했다.
#[test]
fn put_chunks_cleans_per_alias_sentinel_embeddings() {
    let tmp = TempDir::new().unwrap();
    let store = open_store(&tmp);
    let c1 = "11111111111111111111111111111111";
    seed_body_and_alias_sentinels(&store, c1);
    assert_eq!(embed_count(&store, c1), 1);
    assert_eq!(embed_count_prefix(&store, &format!("{c1}#alias")), 3);

    let doc_id = DocumentId(DOC_ID.to_string());
    let chunk = Chunk {
        chunk_id: ChunkId(c1.to_string()),
        doc_id: doc_id.clone(),
        block_ids: Vec::new(),
        text: "hi".to_string(),
        heading_path: Vec::new(),
        source_spans: Vec::new(),
        token_estimate: 1,
        chunker_version: ChunkerVersion("v1".to_string()),
        policy_hash: "h".to_string(),
        tokenized_korean_text: None,
        aliases: None,
    };
    store.put_chunks(&doc_id, std::slice::from_ref(&chunk)).unwrap();

    assert_eq!(
        embed_count(&store, c1),
        0,
        "본문 embedding_records 정리 (CASCADE 대체)"
    );
    assert_eq!(
        embed_count_prefix(&store, &format!("{c1}#alias")),
        0,
        "모든 per-alias sentinel embedding_records 정리 (#alias#N + legacy #alias)"
    );
}

/// PR #195 MAJOR regression: parser-bump 재인제스트 경로
/// (`purge_document_at_workspace_path_except_doc_id`)도 본문 + 모든 per-alias
/// sentinel embedding_records 를 정리해야 한다.
#[test]
fn purge_except_doc_id_cleans_per_alias_sentinel_embeddings() {
    let tmp = TempDir::new().unwrap();
    let store = open_store(&tmp);
    let c1 = "11111111111111111111111111111111";
    seed_body_and_alias_sentinels(&store, c1); // doc DOC_ID @ 'x.md'
    assert_eq!(embed_count(&store, c1), 1);
    assert_eq!(embed_count_prefix(&store, &format!("{c1}#alias")), 3);

    store
        .purge_document_at_workspace_path_except_doc_id("x.md", "0000000000000000000000000000ffff")
        .unwrap();

    assert_eq!(embed_count(&store, c1), 0, "본문 정리");
    assert_eq!(
        embed_count_prefix(&store, &format!("{c1}#alias")),
        0,
        "모든 per-alias sentinel 정리 (#alias#N + legacy #alias)"
    );
}

/// PR #195 MAJOR regression: 파일 삭제 sweep 경로
/// (`purge_deleted_workspace_path`)도 본문 + 모든 per-alias sentinel
/// embedding_records 를 정리해야 한다.
#[test]
fn purge_deleted_workspace_path_cleans_per_alias_sentinel_embeddings() {
    let tmp = TempDir::new().unwrap();
    let store = open_store(&tmp);
    let c1 = "11111111111111111111111111111111";
    seed_body_and_alias_sentinels(&store, c1); // doc DOC_ID @ 'x.md'
    assert_eq!(embed_count(&store, c1), 1);
    assert_eq!(embed_count_prefix(&store, &format!("{c1}#alias")), 3);

    let returned = kebab_store_sqlite::purge_deleted_workspace_path(
        &store,
        &kebab_core::WorkspacePath("x.md".to_string()),
    )
    .unwrap();
    // 반환된 body chunk_ids 는 kebab-app 이 LanceDB 측 별칭 sentinel 까지
    // 삭제하는 데 쓰인다(`alias_sentinel_ids_to_delete`). 본문 1개.
    assert_eq!(returned.len(), 1);

    assert_eq!(embed_count(&store, c1), 0, "본문 정리");
    assert_eq!(
        embed_count_prefix(&store, &format!("{c1}#alias")),
        0,
        "모든 per-alias sentinel 정리 (#alias#N + legacy #alias)"
    );
}
