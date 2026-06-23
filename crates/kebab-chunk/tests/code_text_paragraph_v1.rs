//! Behavioural tests for `CodeTextParagraphV1Chunker`.
//!
//! Documents are constructed manually (no kebab-parse-code dependency) by
//! placing raw text into a single `Block::Code`, mirroring the pattern used
//! in `k8s_manifest_resource_v1.rs`.

use std::path::PathBuf;

use kebab_chunk::CodeTextParagraphV1Chunker;
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

/// Build a `CanonicalDocument` with a single `Block::Code` containing `text`
/// and the supplied `lang` label.
fn text_doc(lang: &str, text: &str) -> CanonicalDocument {
    let wp = WorkspacePath("scripts/sample.sh".into());
    let aid = AssetId("d".repeat(64));
    let pv = ParserVersion("code-text-paragraph-v1".into());
    let doc_id = id_for_doc(&wp, &aid, &pv);

    let line_count = text.lines().count() as u32;
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
        code: text.to_string(),
    });

    CanonicalDocument {
        doc_id,
        source_asset_id: aid,
        workspace_path: wp,
        title: "sample.sh".into(),
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
            source_id: None,
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
        chunker_version: ChunkerVersion("code-text-paragraph-v1".into()),
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

/// `sample_shell.sh` has 4 paragraphs separated by 3 blank lines:
///   - paragraph 1: lines 1-2  (shebang + set -euo pipefail)
///   - paragraph 2: lines 4-7  (env setup block)
///   - paragraph 3: lines 9-11 (ingest block)
///   - paragraph 4: lines 13-15 (report block)
///
/// We assert:
///   - exactly 4 chunks (one per paragraph)
///   - all symbols are None (Tier 3 spec §9.3)
///   - all langs are "shell"
///   - line ranges are strictly ascending and do NOT include the blank lines
///     (lines 3, 8, 12 must not appear in any range)
#[test]
fn shell_multi_paragraph_splits_on_blank_lines() {
    let fixture_path = fixtures_dir().join("sample_shell.sh");
    let text = std::fs::read_to_string(&fixture_path)
        .unwrap_or_else(|e| panic!("cannot read fixture {}: {e}", fixture_path.display()));

    let doc = text_doc("shell", &text);
    let chunks = CodeTextParagraphV1Chunker
        .chunk(&doc, &policy())
        .expect("chunk");

    assert_eq!(
        chunks.len(),
        4,
        "expected 4 chunks (one per paragraph), got {}: {chunks:#?}",
        chunks.len()
    );

    // All symbols must be None (Tier 3 requirement).
    for (i, chunk) in chunks.iter().enumerate() {
        match &chunk.source_spans[0] {
            SourceSpan::Code { symbol, .. } => {
                assert!(
                    symbol.is_none(),
                    "chunk[{i}] symbol must be None for Tier 3 chunker, got {symbol:?}"
                );
            }
            other => panic!("chunk[{i}]: expected Code span, got {other:?}"),
        }
    }

    // All langs must be "shell".
    for (i, chunk) in chunks.iter().enumerate() {
        match &chunk.source_spans[0] {
            SourceSpan::Code { lang, .. } => {
                assert_eq!(
                    lang.as_deref(),
                    Some("shell"),
                    "chunk[{i}] lang must be 'shell', got {lang:?}"
                );
            }
            other => panic!("chunk[{i}]: expected Code span, got {other:?}"),
        }
    }

    // Line ranges must be strictly ascending with no overlap,
    // and blank lines (3, 8, 12) must not be included in any range.
    let expected_ranges: &[(u32, u32)] = &[(1, 2), (4, 7), (9, 11), (13, 15)];
    let actual_ranges: Vec<(u32, u32)> = chunks
        .iter()
        .map(|c| match &c.source_spans[0] {
            SourceSpan::Code {
                line_start,
                line_end,
                ..
            } => (*line_start, *line_end),
            other => panic!("expected Code span, got {other:?}"),
        })
        .collect();

    assert_eq!(
        actual_ranges, expected_ranges,
        "line ranges mismatch: got {actual_ranges:?}, expected {expected_ranges:?}"
    );
}

/// `sample_long_paragraph.txt` has exactly 200 non-blank lines and no blank
/// lines, so the entire file is one paragraph.  200 > 80 (FALLBACK_LINES_PER_CHUNK),
/// so the oversize window split fires with stride 60:
///   - window 1: lines 1-80
///   - window 2: lines 61-140
///   - window 3: lines 121-200
///
/// All chunk_ids must be distinct (the #L{window_start} split_key suffix).
#[test]
fn single_long_paragraph_line_window_split() {
    let fixture_path = fixtures_dir().join("sample_long_paragraph.txt");
    let text = std::fs::read_to_string(&fixture_path)
        .unwrap_or_else(|e| panic!("cannot read fixture {}: {e}", fixture_path.display()));

    assert_eq!(
        text.lines().count(),
        200,
        "fixture must have exactly 200 lines"
    );

    let doc = text_doc("shell", &text);
    let chunks = CodeTextParagraphV1Chunker
        .chunk(&doc, &policy())
        .expect("chunk");

    assert_eq!(
        chunks.len(),
        3,
        "expected 3 window chunks for 200-line paragraph, got {}: {chunks:#?}",
        chunks.len()
    );

    let expected_ranges: &[(u32, u32)] = &[(1, 80), (61, 140), (121, 200)];
    let actual_ranges: Vec<(u32, u32)> = chunks
        .iter()
        .map(|c| match &c.source_spans[0] {
            SourceSpan::Code {
                line_start,
                line_end,
                ..
            } => (*line_start, *line_end),
            other => panic!("expected Code span, got {other:?}"),
        })
        .collect();

    assert_eq!(
        actual_ranges, expected_ranges,
        "window ranges mismatch: got {actual_ranges:?}, expected {expected_ranges:?}"
    );

    // All chunk_ids must be distinct (#L{window_start} suffix differentiates them).
    let ids: std::collections::HashSet<_> = chunks.iter().map(|c| c.chunk_id.clone()).collect();
    assert_eq!(
        ids.len(),
        chunks.len(),
        "oversize window chunks must have distinct chunk_ids"
    );
}

/// An empty source file (no non-blank lines) must yield zero chunks.
#[test]
fn empty_file_emits_zero_chunks() {
    let doc = text_doc("shell", "");
    let chunks = CodeTextParagraphV1Chunker
        .chunk(&doc, &policy())
        .expect("chunk");

    assert_eq!(
        chunks.len(),
        0,
        "empty file must yield 0 chunks, got {}: {chunks:#?}",
        chunks.len()
    );
}

/// The `lang` field on each emitted chunk must match the `lang` passed to
/// `text_doc`, regardless of content.  `symbol` must be `None` (Tier 3 spec).
#[test]
fn lang_field_preserved_from_input_doc() {
    let doc = text_doc("yaml", "key1: value1\nkey2: value2\n");
    let chunks = CodeTextParagraphV1Chunker
        .chunk(&doc, &policy())
        .expect("chunk");

    assert!(!chunks.is_empty(), "expected at least one chunk");

    match &chunks[0].source_spans[0] {
        SourceSpan::Code { lang, symbol, .. } => {
            assert_eq!(
                lang.as_deref(),
                Some("yaml"),
                "lang must be 'yaml', got {lang:?}"
            );
            assert!(
                symbol.is_none(),
                "symbol must be None for Tier 3 chunker, got {symbol:?}"
            );
        }
        other => panic!("expected Code span, got {other:?}"),
    }
}
