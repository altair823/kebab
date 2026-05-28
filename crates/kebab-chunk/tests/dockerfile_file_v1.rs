//! Behavioural tests for `DockerfileFileV1Chunker`.
//!
//! Documents are constructed manually (no kebab-parse-code dependency) by
//! placing the raw Dockerfile text into a single `Block::Code`, mirroring the
//! pattern used in `k8s_manifest_resource_v1.rs`.

use std::path::PathBuf;

use kebab_chunk::DockerfileFileV1Chunker;
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

/// Build a `CanonicalDocument` with a single `Block::Code` containing `dockerfile_text`.
fn dockerfile_doc(dockerfile_text: &str) -> CanonicalDocument {
    let wp = WorkspacePath("build/Dockerfile".into());
    let aid = AssetId("d".repeat(64));
    let pv = ParserVersion("code-dockerfile-v1".into());
    let doc_id = id_for_doc(&wp, &aid, &pv);

    let line_count = dockerfile_text.lines().count() as u32;
    let span = SourceSpan::Code {
        line_start: 1,
        line_end: line_count.max(1),
        symbol: None,
        lang: Some("dockerfile".into()),
    };
    let bid = id_for_block(&doc_id, "code", &[], 0, &span);
    let block = Block::Code(CodeBlock {
        common: CommonBlock {
            block_id: bid,
            heading_path: vec![],
            source_span: span,
        },
        lang: Some("dockerfile".into()),
        code: dockerfile_text.to_string(),
    });

    CanonicalDocument {
        doc_id,
        source_asset_id: aid,
        workspace_path: wp,
        title: "Dockerfile".into(),
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
            code_lang: Some("dockerfile".into()),
        },
        provenance: Provenance { events: vec![] },
        parser_version: pv,
        schema_version: 1,
        doc_version: 1,
        last_chunker_version: None,
        last_embedding_version: None,
    }
}

fn policy() -> ChunkPolicy {
    ChunkPolicy {
        target_tokens: 500,
        overlap_tokens: 80,
        respect_markdown_headings: false,
        chunker_version: ChunkerVersion("dockerfile-file-v1".into()),
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

/// A simple 5-line Dockerfile fixture must emit exactly 1 chunk with the
/// correct symbol, lang, and line range.
#[test]
fn dockerfile_emits_single_chunk() {
    let fixture_path = fixtures_dir().join("sample.dockerfile");
    let text = std::fs::read_to_string(&fixture_path)
        .unwrap_or_else(|e| panic!("cannot read fixture {}: {e}", fixture_path.display()));

    let doc = dockerfile_doc(&text);
    let chunks = DockerfileFileV1Chunker
        .chunk(&doc, &policy())
        .expect("chunk");

    assert_eq!(
        chunks.len(),
        1,
        "expected 1 chunk, got {}: {chunks:#?}",
        chunks.len()
    );

    // Inspect the Chunk's source_spans for symbol / lang / line range.
    let span = chunks[0].source_spans.first().expect("at least one span");
    match span {
        SourceSpan::Code {
            line_start,
            line_end,
            symbol,
            lang,
        } => {
            assert_eq!(*line_start, 1, "line_start must be 1");
            assert_eq!(*line_end, 5, "line_end must be 5 (5-line fixture)");
            assert_eq!(
                symbol.as_deref(),
                Some("<dockerfile>"),
                "symbol must be '<dockerfile>'"
            );
            assert_eq!(
                lang.as_deref(),
                Some("dockerfile"),
                "lang must be 'dockerfile'"
            );
        }
        other => panic!("expected SourceSpan::Code, got {other:?}"),
    }

    // Verify chunker_version label.
    assert_eq!(chunks[0].chunker_version.0, "dockerfile-file-v1");
}
