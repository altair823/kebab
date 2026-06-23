//! Round-trip tests for `last_chunker_version` / `last_embedding_version`
//! columns added by the V006 migration (p9-fb-23 task 3).

use std::path::PathBuf;

use kebab_core::{
    AssetId, AssetStorage, Block, CanonicalDocument, Checksum, ChunkerVersion, CommonBlock,
    DocumentId, DocumentStore, EmbeddingVersion, HeadingBlock, Lang, MediaType, Metadata,
    ParserVersion, Provenance, RawAsset, SourceSpan, SourceType, SourceUri, TrustLevel,
    WorkspacePath,
};
use kebab_store_sqlite::SqliteStore;
use time::OffsetDateTime;

mod common;

fn make_asset() -> RawAsset {
    let bytes = b"incremental-ingest-test";
    RawAsset {
        asset_id: AssetId("f".repeat(32)),
        source_uri: SourceUri::File(PathBuf::from("/tmp/inc.md")),
        workspace_path: WorkspacePath::new("notes/inc.md".into()).unwrap(),
        media_type: MediaType::Markdown,
        byte_len: bytes.len() as u64,
        checksum: Checksum(blake3::hash(bytes).to_hex().to_string()),
        discovered_at: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
        stored: AssetStorage::Reference {
            path: PathBuf::from("/tmp/inc.md"),
            sha: Checksum(blake3::hash(bytes).to_hex().to_string()),
        },
    }
}

fn make_doc() -> CanonicalDocument {
    let doc_id = DocumentId("d".repeat(32));
    let block = Block::Heading(HeadingBlock {
        common: CommonBlock {
            block_id: kebab_core::BlockId("b".repeat(32)),
            heading_path: vec![],
            source_span: SourceSpan::Line { start: 1, end: 1 },
        },
        level: 1,
        text: "Incremental Title".into(),
    });
    let metadata = Metadata {
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
        source_id: None,
    };
    CanonicalDocument {
        doc_id,
        source_asset_id: AssetId("f".repeat(32)),
        workspace_path: WorkspacePath::new("notes/inc.md".into()).unwrap(),
        title: "Incremental Title".into(),
        lang: Lang("en".into()),
        blocks: vec![block],
        metadata,
        provenance: Provenance { events: vec![] },
        parser_version: ParserVersion("test-parser".into()),
        schema_version: 1,
        doc_version: 1,
        last_chunker_version: None,
        last_embedding_version: None,
    }
}

#[test]
fn put_then_get_document_roundtrips_version_stamps() {
    let env = common::TestEnv::new();
    let store = SqliteStore::open(&env.config()).unwrap();
    store.run_migrations().unwrap();

    let asset = make_asset();
    store.put_asset(&asset).unwrap();

    let mut doc = make_doc();
    doc.last_chunker_version = Some(ChunkerVersion("md-heading-v1".into()));
    doc.last_embedding_version = Some(EmbeddingVersion("multilingual-e5-small@v1".into()));

    store.put_document(&doc).unwrap();
    let loaded = store
        .get_document(&doc.doc_id)
        .unwrap()
        .expect("doc round-trips");

    assert_eq!(loaded.last_chunker_version, doc.last_chunker_version);
    assert_eq!(loaded.last_embedding_version, doc.last_embedding_version);
}

#[test]
fn put_then_get_document_roundtrips_none_stamps() {
    let env = common::TestEnv::new();
    let store = SqliteStore::open(&env.config()).unwrap();
    store.run_migrations().unwrap();

    let asset = make_asset();
    store.put_asset(&asset).unwrap();

    let doc = make_doc(); // both version stamps are None by default
    store.put_document(&doc).unwrap();
    let loaded = store
        .get_document(&doc.doc_id)
        .unwrap()
        .expect("doc round-trips");

    assert!(
        loaded.last_chunker_version.is_none(),
        "last_chunker_version must be None when not set"
    );
    assert!(
        loaded.last_embedding_version.is_none(),
        "last_embedding_version must be None when not set"
    );
}

#[test]
fn get_asset_by_workspace_path_roundtrips() {
    let env = common::TestEnv::new();
    let store = SqliteStore::open(&env.config()).unwrap();
    store.run_migrations().unwrap();

    let asset = make_asset();
    store.put_asset(&asset).unwrap();

    let loaded = store
        .get_asset_by_workspace_path(&asset.workspace_path)
        .unwrap()
        .expect("asset must round-trip");

    assert_eq!(loaded.asset_id, asset.asset_id);
    assert_eq!(loaded.checksum, asset.checksum);
    assert_eq!(loaded.byte_len, asset.byte_len);
}

#[test]
fn get_asset_by_workspace_path_returns_none_for_unknown() {
    let env = common::TestEnv::new();
    let store = SqliteStore::open(&env.config()).unwrap();
    store.run_migrations().unwrap();

    let path = WorkspacePath::new("notes/missing.md".into()).unwrap();
    assert!(store.get_asset_by_workspace_path(&path).unwrap().is_none());
}
