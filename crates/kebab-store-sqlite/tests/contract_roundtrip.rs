//! Contract: drive the full pipeline (`kb-parse-md` → `kb-normalize` →
//! `kb-chunk`) on a real fixture and prove `DocumentStore` round-trips
//! the resulting `CanonicalDocument` + `Vec<Chunk>` losslessly.
//!
//! `kb-parse-md`, `kb-normalize`, `kb-chunk` are dev-deps only — see the
//! crate's `Cargo.toml`. The store crate's production tree (visible via
//! `cargo tree -p kb-store-sqlite --depth 1`) does NOT include them.

use std::path::PathBuf;

use kebab_chunk::MdHeadingV1Chunker;
use kebab_core::{
    AssetId, AssetStorage, Checksum, ChunkPolicy, ChunkerVersion, Chunker, DocumentStore,
    MediaType, ParserVersion, RawAsset, SourceUri, WorkspacePath,
};
use kebab_normalize::build_canonical_document;
use kebab_parse_md::{BodyHints, parse_blocks, parse_frontmatter};
use kebab_store_sqlite::SqliteStore;
use time::OffsetDateTime;

mod common;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
        .join("markdown")
}

#[test]
fn document_and_chunks_round_trip_through_sqlite() {
    let env = common::TestEnv::new();
    let store = SqliteStore::open(&env.config()).unwrap();
    store.run_migrations().unwrap();

    // ── Build inputs from the fixture ───────────────────────────────
    let dir = fixtures_dir();
    let bytes = std::fs::read(dir.join("code-and-table.md")).expect("read fixture");
    let cs = blake3::hash(&bytes).to_hex().to_string();

    let asset = RawAsset {
        asset_id: AssetId("a".repeat(32)),
        source_uri: SourceUri::File(dir.join("code-and-table.md")),
        workspace_path: WorkspacePath::new("notes/code-and-table.md".into()).unwrap(),
        media_type: MediaType::Markdown,
        byte_len: bytes.len() as u64,
        checksum: Checksum(cs.clone()),
        discovered_at: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
        stored: AssetStorage::Reference {
            path: dir.join("code-and-table.md"),
            sha: Checksum(cs.clone()),
        },
    };

    let hints = BodyHints {
        first_h1: Some("Code And Table".into()),
        fs_ctime: asset.discovered_at,
        fs_mtime: asset.discovered_at,
        fallback_lang: Some("en".into()),
    };
    let (mut metadata, _fm_span, _fm_warns) =
        parse_frontmatter(&bytes, &hints).unwrap();
    let (parsed_blocks, parse_warns) = parse_blocks(&bytes, 1).unwrap();

    metadata.aliases.sort();
    metadata.tags.sort();
    let parser_version = ParserVersion("kb-store-sqlite-roundtrip".into());
    let doc = build_canonical_document(
        &asset,
        metadata,
        parsed_blocks,
        &parser_version,
        parse_warns,
    )
    .unwrap();

    let policy = ChunkPolicy {
        target_tokens: 200,
        overlap_tokens: 40,
        respect_markdown_headings: true,
        chunker_version: ChunkerVersion("md-heading-v1".into()),
    };
    let chunks = MdHeadingV1Chunker.chunk(&doc, &policy).unwrap();
    assert!(!chunks.is_empty(), "fixture must produce ≥1 chunk");

    // ── Persist via the store ────────────────────────────────────────
    store
        .put_asset_with_bytes(&asset, &bytes)
        .expect("put_asset_with_bytes");
    store.put_document(&doc).expect("put_document");
    store
        .put_blocks(&doc.doc_id, &doc.blocks)
        .expect("put_blocks");
    store
        .put_chunks(&doc.doc_id, &chunks)
        .expect("put_chunks");

    // ── Read back ────────────────────────────────────────────────────
    let loaded = store
        .get_document(&doc.doc_id)
        .expect("get_document err")
        .expect("get_document Some");

    // Document-level fields must match. doc_version is bumped by the
    // UPSERT path even on first put (the trigger runs on conflict
    // only; first-insert lands the caller-supplied 1). updated_at is
    // re-stamped server-side and is NOT round-tripped (the loaded
    // CanonicalDocument carries `metadata.updated_at` from the
    // metadata_json blob, which is the input value). So we compare
    // the field-by-field copies that ARE deterministic:
    assert_eq!(loaded.doc_id, doc.doc_id);
    assert_eq!(loaded.workspace_path, doc.workspace_path);
    assert_eq!(loaded.title, doc.title);
    assert_eq!(loaded.lang, doc.lang);
    assert_eq!(loaded.parser_version, doc.parser_version);
    assert_eq!(loaded.schema_version, doc.schema_version);
    assert_eq!(loaded.metadata, doc.metadata, "metadata round-trip");
    assert_eq!(loaded.provenance, doc.provenance, "provenance round-trip");
    assert_eq!(
        loaded.blocks.len(),
        doc.blocks.len(),
        "block count round-trip"
    );
    assert_eq!(loaded.blocks, doc.blocks, "block stream round-trip");

    // Chunks: get_chunk for each id.
    for c in &chunks {
        let back = store
            .get_chunk(&c.chunk_id)
            .expect("get_chunk err")
            .expect("get_chunk Some");
        assert_eq!(&back, c, "chunk round-trip mismatch for {}", c.chunk_id.0);
    }
}
