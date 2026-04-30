//! Idempotency: re-ingesting the same `(workspace_path, asset_id,
//! parser_version)` keeps documents at one row but bumps `doc_version`
//! and replaces blocks/chunks rather than duplicating them.

use std::path::PathBuf;

use kb_core::{
    AssetId, AssetStorage, Block, CanonicalDocument, Checksum, Chunk, ChunkerVersion,
    CommonBlock, DocumentId, DocumentStore, HeadingBlock, Lang, MediaType, Metadata,
    ParserVersion, Provenance, RawAsset, SourceSpan, SourceType, SourceUri, TextBlock,
    TrustLevel, WorkspacePath,
};
use kb_store_sqlite::SqliteStore;
use time::OffsetDateTime;

mod common;

fn make_asset() -> RawAsset {
    let bytes = b"dummy";
    RawAsset {
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
        tags: vec!["one".into(), "two".into()],
        created_at: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
        updated_at: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
        source_type: SourceType::Markdown,
        trust_level: TrustLevel::Primary,
        user_id_alias: None,
        user: Default::default(),
    }
}

fn make_doc() -> CanonicalDocument {
    let doc_id = DocumentId("d".repeat(32));
    let span = SourceSpan::Line { start: 1, end: 1 };
    let block = Block::Heading(HeadingBlock {
        common: CommonBlock {
            block_id: kb_core::BlockId("b".repeat(32)),
            heading_path: vec![],
            source_span: span.clone(),
        },
        level: 1,
        text: "Title".into(),
    });
    let para = Block::Paragraph(TextBlock {
        common: CommonBlock {
            block_id: kb_core::BlockId("c".repeat(32)),
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
    }
}

fn make_chunks(doc_id: &DocumentId) -> Vec<Chunk> {
    vec![Chunk {
        chunk_id: kb_core::ChunkId("e".repeat(32)),
        doc_id: doc_id.clone(),
        block_ids: vec![kb_core::BlockId("b".repeat(32))],
        text: "Title\n\nbody".into(),
        heading_path: vec!["Title".into()],
        source_spans: vec![SourceSpan::Line { start: 1, end: 1 }],
        token_estimate: 5,
        chunker_version: ChunkerVersion("md-heading-v1".into()),
        policy_hash: "deadbeefdeadbeef".into(),
    }]
}

#[test]
fn put_document_idempotent_bumps_doc_version() {
    let env = common::TestEnv::new();
    let store = SqliteStore::open(&env.config()).unwrap();
    store.run_migrations().unwrap();

    let asset = make_asset();
    store.put_asset(&asset).expect("put_asset 1");

    let doc = make_doc();
    store.put_document(&doc).expect("put_document 1");

    // First ingest → exactly one row, doc_version=1.
    let (count, dv1): (i64, i64) = env.with_conn(|c| {
        c.query_row(
            "SELECT COUNT(*), MAX(doc_version) FROM documents WHERE doc_id = ?",
            [&doc.doc_id.0],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
    });
    assert_eq!(count, 1);
    assert_eq!(dv1, 1);

    // Re-ingest the same doc → still one row, doc_version=2.
    store.put_document(&doc).expect("put_document 2");
    let (count2, dv2): (i64, i64) = env.with_conn(|c| {
        c.query_row(
            "SELECT COUNT(*), MAX(doc_version) FROM documents WHERE doc_id = ?",
            [&doc.doc_id.0],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
    });
    assert_eq!(count2, 1, "second put must not duplicate the row");
    assert_eq!(dv2, 2, "doc_version must increment on re-ingest");

    // Tags were re-derived: still exactly the two original tags.
    let tags: Vec<String> = env.with_conn(|c| {
        let mut stmt =
            c.prepare("SELECT tag FROM document_tags WHERE doc_id = ? ORDER BY tag")?;
        let rows = stmt.query_map([&doc.doc_id.0], |r| r.get::<_, String>(0))?;
        rows.collect::<rusqlite::Result<Vec<String>>>()
    });
    assert_eq!(tags, vec!["one".to_string(), "two".to_string()]);
}

#[test]
fn put_blocks_and_put_chunks_replace_not_duplicate() {
    let env = common::TestEnv::new();
    let store = SqliteStore::open(&env.config()).unwrap();
    store.run_migrations().unwrap();

    let asset = make_asset();
    store.put_asset(&asset).unwrap();
    let doc = make_doc();
    store.put_document(&doc).unwrap();

    store.put_blocks(&doc.doc_id, &doc.blocks).unwrap();
    store.put_chunks(&doc.doc_id, &make_chunks(&doc.doc_id)).unwrap();

    let (b1, ch1): (i64, i64) = env.with_conn(|c| {
        Ok((
            c.query_row(
                "SELECT COUNT(*) FROM blocks WHERE doc_id = ?",
                [&doc.doc_id.0],
                |r| r.get(0),
            )?,
            c.query_row(
                "SELECT COUNT(*) FROM chunks WHERE doc_id = ?",
                [&doc.doc_id.0],
                |r| r.get(0),
            )?,
        ))
    });
    assert_eq!(b1, 2);
    assert_eq!(ch1, 1);

    // Re-put same data → counts unchanged (DELETE-then-INSERT).
    store.put_blocks(&doc.doc_id, &doc.blocks).unwrap();
    store.put_chunks(&doc.doc_id, &make_chunks(&doc.doc_id)).unwrap();
    let (b2, ch2): (i64, i64) = env.with_conn(|c| {
        Ok((
            c.query_row(
                "SELECT COUNT(*) FROM blocks WHERE doc_id = ?",
                [&doc.doc_id.0],
                |r| r.get(0),
            )?,
            c.query_row(
                "SELECT COUNT(*) FROM chunks WHERE doc_id = ?",
                [&doc.doc_id.0],
                |r| r.get(0),
            )?,
        ))
    });
    assert_eq!(b2, 2, "blocks must not double on re-put");
    assert_eq!(ch2, 1, "chunks must not double on re-put");
}

/// `put_blocks` runs in a transaction. If we feed it a block whose
/// `doc_id` references a document that does not exist, the FK
/// constraint (`blocks.doc_id REFERENCES documents(doc_id)`) trips,
/// the transaction rolls back, and the table count is unchanged.
#[test]
fn put_blocks_transactional_rollback_on_fk_violation() {
    let env = common::TestEnv::new();
    let store = SqliteStore::open(&env.config()).unwrap();
    store.run_migrations().unwrap();

    let asset = make_asset();
    store.put_asset(&asset).unwrap();
    let doc = make_doc();
    store.put_document(&doc).unwrap();
    // Establish a baseline row in `blocks`.
    store.put_blocks(&doc.doc_id, &doc.blocks).unwrap();
    let baseline: i64 = env.with_conn(|c| {
        c.query_row("SELECT COUNT(*) FROM blocks", [], |r| r.get(0))
    });
    assert_eq!(baseline, 2);

    // Now ask put_blocks to write to a doc_id that does NOT exist.
    // The implementation issues `DELETE FROM blocks WHERE doc_id = ?`
    // (no-op for the missing doc) followed by INSERTs that violate the
    // FK constraint. The whole tx must roll back, so `blocks` count
    // stays at `baseline`.
    let phantom = DocumentId("0".repeat(32));
    let phantom_blocks = vec![Block::Heading(HeadingBlock {
        common: CommonBlock {
            block_id: kb_core::BlockId("9".repeat(32)),
            heading_path: vec![],
            source_span: SourceSpan::Line { start: 1, end: 1 },
        },
        level: 1,
        text: "phantom".into(),
    })];
    let res = store.put_blocks(&phantom, &phantom_blocks);
    assert!(res.is_err(), "FK violation must surface as Err");

    let after: i64 = env.with_conn(|c| {
        c.query_row("SELECT COUNT(*) FROM blocks", [], |r| r.get(0))
    });
    assert_eq!(
        after, baseline,
        "transaction must roll back; blocks count must be unchanged"
    );
}
