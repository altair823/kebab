//! Behavioural tests for `ManifestFileV1Chunker`.
//!
//! Documents are constructed manually (no kebab-parse-code dependency) by
//! placing the raw manifest text into a single `Block::Code`, mirroring the
//! pattern used in `dockerfile_file_v1.rs`.

use std::path::PathBuf;

use kebab_chunk::ManifestFileV1Chunker;
use kebab_core::{
    AssetId, Block, CanonicalDocument, ChunkPolicy, Chunker, ChunkerVersion, CodeBlock,
    CommonBlock, Lang, Metadata, ParserVersion, Provenance, SourceSpan, SourceType, TrustLevel,
    WorkspacePath, id_for_block, id_for_doc,
};
use time::OffsetDateTime;

// ── helpers ──────────────────────────────────────────────────────────────────

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

/// Build a `CanonicalDocument` with a single `Block::Code` containing manifest text.
fn manifest_doc(lang: &str, manifest_text: &str) -> CanonicalDocument {
    let wp = WorkspacePath(format!("build/{}", manifest_filename(lang)));
    let aid = AssetId("m".repeat(64));
    let pv = ParserVersion("code-manifest-v1".into());
    let doc_id = id_for_doc(&wp, &aid, &pv);

    let line_count = manifest_text.lines().count() as u32;
    let span = SourceSpan::Code {
        line_start: 1,
        line_end: line_count.max(1),
        symbol: None,
        lang: Some(lang.into()),
    };
    let bid = id_for_block(&doc_id, "code", &[], 0, &span);
    let block = Block::Code(CodeBlock {
        common: CommonBlock {
            block_id: bid,
            heading_path: vec![],
            source_span: span,
        },
        lang: Some(lang.into()),
        code: manifest_text.to_string(),
    });

    CanonicalDocument {
        doc_id,
        source_asset_id: aid,
        workspace_path: wp,
        title: format!("Manifest ({lang})"),
        lang: Lang("und".into()),
        blocks: vec![block],
        metadata: Metadata {
            aliases: vec![],
            tags: vec![],
            created_at: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
            updated_at: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
            source_type: SourceType::Note,
            trust_level: TrustLevel::Primary,
            user_id_alias: None,
            user: Default::default(),
            repo: Some("kebab".into()),
            git_branch: Some("main".into()),
            git_commit: Some("0".repeat(40)),
            code_lang: Some(lang.into()),
        },
        provenance: Provenance { events: vec![] },
        parser_version: pv,
        schema_version: 1,
        doc_version: 1,
        last_chunker_version: None,
        last_embedding_version: None,
    }
}

fn manifest_filename(lang: &str) -> &'static str {
    match lang {
        "toml" => "Cargo.toml",
        "json" => "package.json",
        "xml" => "pom.xml",
        "go-mod" => "go.mod",
        _ => "manifest",
    }
}

fn policy() -> ChunkPolicy {
    ChunkPolicy {
        target_tokens: 500,
        overlap_tokens: 80,
        respect_markdown_headings: false,
        chunker_version: ChunkerVersion("manifest-file-v1".into()),
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

/// A Cargo.toml fixture must emit exactly 1 chunk with the correct symbol,
/// lang, and line range.
#[test]
fn cargo_toml_single_chunk_with_toml_lang() {
    let fixture_path = fixtures_dir().join("sample_cargo.toml");
    let text = std::fs::read_to_string(&fixture_path)
        .unwrap_or_else(|e| panic!("cannot read fixture {}: {e}", fixture_path.display()));

    let doc = manifest_doc("toml", &text);
    let chunks = ManifestFileV1Chunker
        .chunk(&doc, &policy())
        .expect("chunk");

    assert_eq!(
        chunks.len(),
        1,
        "expected 1 chunk, got {}: {chunks:#?}",
        chunks.len()
    );

    let span = chunks[0].source_spans.first().expect("at least one span");
    match span {
        SourceSpan::Code {
            line_start,
            line_end: _,
            symbol,
            lang,
        } => {
            assert_eq!(*line_start, 1, "line_start must be 1");
            assert_eq!(
                symbol.as_deref(),
                Some("<manifest>"),
                "symbol must be '<manifest>'"
            );
            assert_eq!(lang.as_deref(), Some("toml"), "lang must be 'toml'");
        }
        other => panic!("expected SourceSpan::Code, got {other:?}"),
    }

    assert_eq!(chunks[0].chunker_version.0, "manifest-file-v1");
}

/// A package.json fixture must emit exactly 1 chunk with the correct symbol,
/// lang, and line range.
#[test]
fn package_json_single_chunk_with_json_lang() {
    let fixture_path = fixtures_dir().join("sample_package.json");
    let text = std::fs::read_to_string(&fixture_path)
        .unwrap_or_else(|e| panic!("cannot read fixture {}: {e}", fixture_path.display()));

    let doc = manifest_doc("json", &text);
    let chunks = ManifestFileV1Chunker
        .chunk(&doc, &policy())
        .expect("chunk");

    assert_eq!(
        chunks.len(),
        1,
        "expected 1 chunk, got {}: {chunks:#?}",
        chunks.len()
    );

    let span = chunks[0].source_spans.first().expect("at least one span");
    match span {
        SourceSpan::Code {
            line_start,
            line_end: _,
            symbol,
            lang,
        } => {
            assert_eq!(*line_start, 1, "line_start must be 1");
            assert_eq!(
                symbol.as_deref(),
                Some("<manifest>"),
                "symbol must be '<manifest>'"
            );
            assert_eq!(lang.as_deref(), Some("json"), "lang must be 'json'");
        }
        other => panic!("expected SourceSpan::Code, got {other:?}"),
    }

    assert_eq!(chunks[0].chunker_version.0, "manifest-file-v1");
}

/// A pom.xml fixture must emit exactly 1 chunk with the correct symbol,
/// lang, and line range.
#[test]
fn pom_xml_single_chunk_with_xml_lang() {
    let fixture_path = fixtures_dir().join("sample_pom.xml");
    let text = std::fs::read_to_string(&fixture_path)
        .unwrap_or_else(|e| panic!("cannot read fixture {}: {e}", fixture_path.display()));

    let doc = manifest_doc("xml", &text);
    let chunks = ManifestFileV1Chunker
        .chunk(&doc, &policy())
        .expect("chunk");

    assert_eq!(
        chunks.len(),
        1,
        "expected 1 chunk, got {}: {chunks:#?}",
        chunks.len()
    );

    let span = chunks[0].source_spans.first().expect("at least one span");
    match span {
        SourceSpan::Code {
            line_start,
            line_end: _,
            symbol,
            lang,
        } => {
            assert_eq!(*line_start, 1, "line_start must be 1");
            assert_eq!(
                symbol.as_deref(),
                Some("<manifest>"),
                "symbol must be '<manifest>'"
            );
            assert_eq!(lang.as_deref(), Some("xml"), "lang must be 'xml'");
        }
        other => panic!("expected SourceSpan::Code, got {other:?}"),
    }

    assert_eq!(chunks[0].chunker_version.0, "manifest-file-v1");
}

/// A go.mod fixture must emit exactly 1 chunk with the correct symbol,
/// lang, and line range.
#[test]
fn go_mod_single_chunk_with_go_mod_lang() {
    let fixture_path = fixtures_dir().join("sample_go.mod");
    let text = std::fs::read_to_string(&fixture_path)
        .unwrap_or_else(|e| panic!("cannot read fixture {}: {e}", fixture_path.display()));

    let doc = manifest_doc("go-mod", &text);
    let chunks = ManifestFileV1Chunker
        .chunk(&doc, &policy())
        .expect("chunk");

    assert_eq!(
        chunks.len(),
        1,
        "expected 1 chunk, got {}: {chunks:#?}",
        chunks.len()
    );

    let span = chunks[0].source_spans.first().expect("at least one span");
    match span {
        SourceSpan::Code {
            line_start,
            line_end: _,
            symbol,
            lang,
        } => {
            assert_eq!(*line_start, 1, "line_start must be 1");
            assert_eq!(
                symbol.as_deref(),
                Some("<manifest>"),
                "symbol must be '<manifest>'"
            );
            assert_eq!(lang.as_deref(), Some("go-mod"), "lang must be 'go-mod'");
        }
        other => panic!("expected SourceSpan::Code, got {other:?}"),
    }

    assert_eq!(chunks[0].chunker_version.0, "manifest-file-v1");
}
