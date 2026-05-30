//! V010 doc-side expansion: `put_chunks` 가 `chunk.aliases` 를 chunks.aliases
//! 컬럼에 영속화하고, chunk_aliases_ai trigger 가 별도 `chunk_aliases_fts`
//! 가상 테이블로 mirror 하는지 검증.
//!
//! `put_chunks` 는 store-owned conn(FK ON)에서 도므로 chunks 의
//! `doc_id REFERENCES documents(doc_id)` FK 를 만족시키려면 asset +
//! document 그래프가 먼저 있어야 한다. 헬퍼는 `idempotency.rs` 패턴 복제.
//! 인덱싱 검증은 side-channel `env.with_conn` 으로 chunk_aliases_fts 를 직접
//! MATCH 한다(같은 established 패턴).

use std::path::PathBuf;

use kebab_core::{
    AssetId, AssetStorage, Block, CanonicalDocument, Checksum, Chunk, ChunkerVersion, CommonBlock,
    DocumentId, DocumentStore, HeadingBlock, Lang, MediaType, Metadata, ParserVersion, Provenance,
    SourceSpan, SourceType, SourceUri, TextBlock, TrustLevel, WorkspacePath,
};
use kebab_store_sqlite::SqliteStore;
use time::OffsetDateTime;

mod common;

fn make_asset() -> kebab_core::RawAsset {
    let bytes = b"dummy";
    kebab_core::RawAsset {
        asset_id: AssetId("a".repeat(32)),
        source_uri: SourceUri::File(PathBuf::from("/tmp/foo.md")),
        workspace_path: WorkspacePath::new("notes/foo.md".into()).unwrap(),
        media_type: MediaType::Markdown,
        byte_len: bytes.len() as u64,
        checksum: Checksum(blake3::hash(bytes).to_hex().to_string()),
        discovered_at: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
        stored: AssetStorage::Reference {
            path: PathBuf::from("/tmp/foo.md"),
            sha: Checksum(blake3::hash(bytes).to_hex().to_string()),
        },
    }
}

fn make_metadata() -> Metadata {
    Metadata {
        aliases: vec![],
        tags: vec![],
        created_at: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
        updated_at: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
        source_type: SourceType::Markdown,
        trust_level: TrustLevel::Primary,
        user_id_alias: None,
        user: Default::default(),
        repo: None,
        git_branch: None,
        git_commit: None,
        code_lang: None,
    }
}

fn make_doc() -> CanonicalDocument {
    let doc_id = DocumentId("d".repeat(32));
    let span = SourceSpan::Line { start: 1, end: 1 };
    let block = Block::Heading(HeadingBlock {
        common: CommonBlock {
            block_id: kebab_core::BlockId("b".repeat(32)),
            heading_path: vec![],
            source_span: span.clone(),
        },
        level: 1,
        text: "Title".into(),
    });
    let para = Block::Paragraph(TextBlock {
        common: CommonBlock {
            block_id: kebab_core::BlockId("c".repeat(32)),
            heading_path: vec!["Title".into()],
            source_span: span,
        },
        text: "body".into(),
        inlines: vec![],
    });
    CanonicalDocument {
        doc_id,
        source_asset_id: AssetId("a".repeat(32)),
        workspace_path: WorkspacePath::new("notes/foo.md".into()).unwrap(),
        title: "Title".into(),
        lang: Lang("en".into()),
        blocks: vec![block, para],
        metadata: make_metadata(),
        provenance: Provenance { events: vec![] },
        parser_version: ParserVersion("test-parser".into()),
        schema_version: 1,
        doc_version: 1,
        last_chunker_version: None,
        last_embedding_version: None,
    }
}

/// 단일 청크 생성. `aliases` 만 호출측이 지정.
fn base_chunk(chunk_id: &str, doc_id: &DocumentId, aliases: Option<String>) -> Chunk {
    Chunk {
        chunk_id: kebab_core::ChunkId(chunk_id.into()),
        doc_id: doc_id.clone(),
        block_ids: vec![kebab_core::BlockId("b".repeat(32))],
        text: "Rust ownership and borrowing".into(),
        heading_path: vec!["Title".into()],
        source_spans: vec![SourceSpan::Line { start: 1, end: 1 }],
        token_estimate: 5,
        chunker_version: ChunkerVersion("md-heading-v1".into()),
        policy_hash: "h".into(),
        tokenized_korean_text: None,
        aliases,
    }
}

/// asset + document 그래프를 깔고 마이그레이션된 store 를 돌려준다.
fn open_store_with_document(env: &common::TestEnv) -> SqliteStore {
    let store = SqliteStore::open(&env.config()).unwrap();
    store.run_migrations().unwrap();
    store.put_asset(&make_asset()).expect("put_asset");
    store.put_document(&make_doc()).expect("put_document");
    store
}

#[test]
fn aliases_indexed_into_chunk_aliases_fts() {
    let env = common::TestEnv::new();
    let store = open_store_with_document(&env);
    let doc = DocumentId("d".repeat(32));
    let chunk = base_chunk(
        &"e".repeat(32),
        &doc,
        Some("메모리 안전성\nwho owns the value".into()),
    );
    store.put_chunks(&doc, &[chunk]).unwrap();

    // 별칭에만 있는 한국어 term 으로 chunk_aliases_fts 검색 → 청크 회수.
    let n: i64 = env.with_conn(|c| {
        c.query_row(
            "SELECT count(*) FROM chunk_aliases_fts \
             WHERE chunk_aliases_fts MATCH 'aliases : (\"메모리\")'",
            [],
            |r| r.get(0),
        )
    });
    assert_eq!(
        n, 1,
        "aliases 의 한국어 term 이 chunk_aliases_fts 에 색인돼야 한다"
    );
}

#[test]
fn none_aliases_not_indexed() {
    let env = common::TestEnv::new();
    let store = open_store_with_document(&env);
    let doc = DocumentId("d".repeat(32));
    let chunk = base_chunk(&"e".repeat(32), &doc, None);
    store.put_chunks(&doc, &[chunk]).unwrap();

    let n: i64 = env.with_conn(|c| {
        c.query_row("SELECT count(*) FROM chunk_aliases_fts", [], |r| r.get(0))
    });
    assert_eq!(
        n, 0,
        "aliases=None 이면 chunk_aliases_fts 에 행이 없어야 한다"
    );
}
