//! `DocumentStore::list_documents` filter coverage.

use std::path::PathBuf;

use kebab_core::{
    AssetId, AssetStorage, Block, CanonicalDocument, Checksum, CommonBlock, DocFilter,
    DocumentId, DocumentStore, HeadingBlock, Lang, MediaType, Metadata, ParserVersion,
    Provenance, RawAsset, SourceSpan, SourceType, SourceUri, TrustLevel, WorkspacePath,
};
use kebab_store_sqlite::SqliteStore;
use time::OffsetDateTime;

mod common;

fn make_doc(
    suffix: char,
    workspace_path: &str,
    lang: &str,
    tags: Vec<&str>,
    trust: TrustLevel,
) -> (RawAsset, CanonicalDocument) {
    let bytes: Vec<u8> = vec![suffix as u8; 16];
    let cs = blake3::hash(&bytes).to_hex().to_string();
    let asset_id = AssetId(format!("{suffix}").repeat(32));
    let asset = RawAsset {
        asset_id: asset_id.clone(),
        source_uri: SourceUri::File(PathBuf::from(format!("/tmp/{suffix}.md"))),
        workspace_path: WorkspacePath::new(workspace_path.into()).unwrap(),
        media_type: MediaType::Markdown,
        byte_len: bytes.len() as u64,
        checksum: Checksum(cs.clone()),
        discovered_at: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
        stored: AssetStorage::Reference {
            path: PathBuf::from(format!("/tmp/{suffix}.md")),
            sha: Checksum(cs),
        },
    };
    let doc_id = DocumentId(format!("d{suffix}").repeat(16));
    let block = Block::Heading(HeadingBlock {
        common: CommonBlock {
            block_id: kebab_core::BlockId(format!("b{suffix}").repeat(16)),
            heading_path: vec![],
            source_span: SourceSpan::Line { start: 1, end: 1 },
        },
        level: 1,
        text: format!("Title {suffix}"),
    });
    let metadata = Metadata {
        aliases: vec![],
        tags: tags.into_iter().map(String::from).collect(),
        created_at: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
        updated_at: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
        source_type: SourceType::Markdown,
        trust_level: trust,
        user_id_alias: None,
        user: Default::default(),
        repo: None,
        git_branch: None,
        git_commit: None,
        code_lang: None,
    };
    let doc = CanonicalDocument {
        doc_id,
        source_asset_id: asset_id,
        workspace_path: asset.workspace_path.clone(),
        title: format!("Title {suffix}"),
        lang: Lang(lang.into()),
        blocks: vec![block],
        metadata,
        provenance: Provenance { events: vec![] },
        parser_version: ParserVersion("test".into()),
        schema_version: 1,
        doc_version: 1,
        last_chunker_version: None,
        last_embedding_version: None,
    };
    (asset, doc)
}

#[test]
fn list_documents_filters_lang_and_tags() {
    let env = common::TestEnv::new();
    let store = SqliteStore::open(&env.config()).unwrap();
    store.run_migrations().unwrap();

    for (asset, doc) in [
        make_doc('a', "notes/a.md", "en", vec!["rust", "kb"], TrustLevel::Primary),
        make_doc('b', "notes/b.md", "ko", vec!["rust"], TrustLevel::Secondary),
        make_doc('c', "papers/c.md", "en", vec!["bio"], TrustLevel::Generated),
    ] {
        store.put_asset(&asset).unwrap();
        store.put_document(&doc).unwrap();
    }

    // No filter → all three docs.
    let all = store.list_documents(&DocFilter::default()).unwrap();
    assert_eq!(all.len(), 3);

    // lang filter.
    let en = store
        .list_documents(&DocFilter {
            lang: Some(Lang("en".into())),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(en.len(), 2);
    assert!(en.iter().all(|d| d.lang == Lang("en".into())));

    // path glob.
    let papers = store
        .list_documents(&DocFilter {
            path_glob: Some("papers/*.md".into()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(papers.len(), 1);
    assert_eq!(papers[0].doc_path.0, "papers/c.md");

    // tags_any.
    let rust = store
        .list_documents(&DocFilter {
            tags_any: vec!["rust".into()],
            ..Default::default()
        })
        .unwrap();
    assert_eq!(rust.len(), 2);
    // tags must be hydrated on the result.
    for d in &rust {
        assert!(
            d.tags.iter().any(|t| t == "rust"),
            "expected `rust` tag on {}: {:?}",
            d.doc_path.0,
            d.tags
        );
    }

    // trust_min — Primary only.
    let primary = store
        .list_documents(&DocFilter {
            trust_min: Some(TrustLevel::Primary),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(primary.len(), 1);
    assert_eq!(primary[0].trust_level, TrustLevel::Primary);
}
