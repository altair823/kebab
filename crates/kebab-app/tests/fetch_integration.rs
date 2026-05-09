//! p9-fb-35 App::fetch integration tests.

mod common;

use kebab_app::App;
use kebab_core::{FetchKind, FetchOpts, FetchQuery};

fn open(env: &common::TestEnv) -> App {
    env.app()
}

#[test]
fn fetch_chunk_returns_target_only_when_no_context() {
    let env = common::TestEnv::new();
    common::ingest_md(&env, "a.md", "# Title\n\nFirst paragraph.\n\n## Section\n\nSecond.\n");
    let app = open(&env);

    // Find a chunk via search to obtain its id.
    let q = kebab_core::SearchQuery {
        text: "First".to_string(),
        mode: kebab_core::SearchMode::Lexical,
        k: 1,
        filters: kebab_core::SearchFilters::default(),
    };
    let hits = app.search(q).unwrap();
    let chunk_id = hits[0].chunk_id.clone();

    let result = app
        .fetch(FetchQuery::Chunk(chunk_id), FetchOpts::default())
        .unwrap();
    assert_eq!(result.kind, FetchKind::Chunk);
    assert!(result.chunk.is_some(), "target chunk populated");
    assert!(result.context_before.is_empty());
    assert!(result.context_after.is_empty());
    assert!(!result.truncated);
}

#[test]
fn fetch_chunk_with_context_returns_neighbors() {
    let env = common::TestEnv::new();
    let body = "# H1\n\nA1\n\n# H2\n\nA2\n\n# H3\n\nA3\n\n# H4\n\nA4\n\n# H5\n\nA5\n";
    common::ingest_md(&env, "multi.md", body);
    let app = env.app();

    let q = kebab_core::SearchQuery {
        text: "A3".to_string(),
        mode: kebab_core::SearchMode::Lexical,
        k: 1,
        filters: kebab_core::SearchFilters::default(),
    };
    let hits = app.search(q).unwrap();
    let chunk_id = hits[0].chunk_id.clone();

    let result = app
        .fetch(
            FetchQuery::Chunk(chunk_id),
            FetchOpts {
                context: Some(2),
                max_tokens: None,
            },
        )
        .unwrap();
    assert_eq!(result.kind, FetchKind::Chunk);
    assert!(result.chunk.is_some());
    let total = result.context_before.len() + result.context_after.len();
    assert!(total >= 1, "at least one neighbor expected");
    assert!(total <= 4, "context capped at +-2 ⇒ max 4 neighbors");
}

#[test]
fn fetch_chunk_unknown_id_returns_chunk_not_found() {
    let env = common::TestEnv::new();
    let app = env.app();
    let err = app
        .fetch(
            FetchQuery::Chunk(kebab_core::ChunkId("nonexistent-id".to_string())),
            FetchOpts::default(),
        )
        .unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("chunk_not_found") || msg.contains("nonexistent-id"),
        "expected chunk_not_found error, got: {msg}"
    );
}

#[test]
fn fetch_doc_returns_serialized_markdown() {
    let env = common::TestEnv::new();
    let body = "# Heading One\n\nFirst paragraph.\n\n## Sub\n\nSecond.\n";
    common::ingest_md(&env, "doc.md", body);
    let app = env.app();

    // Discover doc_id via search hit (avoids depending on list_docs API shape).
    let q = kebab_core::SearchQuery {
        text: "First".to_string(),
        mode: kebab_core::SearchMode::Lexical,
        k: 1,
        filters: kebab_core::SearchFilters::default(),
    };
    let hits = app.search(q).unwrap();
    let doc_id = hits[0].doc_id.clone();

    let result = app
        .fetch(FetchQuery::Doc(doc_id), FetchOpts::default())
        .unwrap();
    assert_eq!(result.kind, FetchKind::Doc);
    let text = result.text.expect("doc text");
    assert!(text.contains("Heading One"), "doc text contains heading: {text:?}");
    assert!(text.contains("First paragraph"), "doc text contains body");
    assert!(!result.truncated);
}

#[test]
fn fetch_doc_unknown_id_returns_doc_not_found() {
    let env = common::TestEnv::new();
    let app = env.app();
    let err = app
        .fetch(
            FetchQuery::Doc(kebab_core::DocumentId("nonexistent-doc".to_string())),
            FetchOpts::default(),
        )
        .unwrap_err();
    assert!(err.to_string().contains("doc_not_found"), "got: {err}");
}

#[test]
fn fetch_doc_with_max_tokens_truncates() {
    let env = common::TestEnv::new();
    let p = "Lorem ipsum dolor sit amet consectetur adipiscing elit. ".repeat(20);
    let body = format!("# Big\n\n{p}\n");
    common::ingest_md(&env, "big.md", &body);
    let app = env.app();
    let q = kebab_core::SearchQuery {
        text: "Lorem".to_string(),
        mode: kebab_core::SearchMode::Lexical,
        k: 1,
        filters: kebab_core::SearchFilters::default(),
    };
    let hits = app.search(q).unwrap();
    let doc_id = hits[0].doc_id.clone();

    let result = app
        .fetch(
            FetchQuery::Doc(doc_id),
            FetchOpts {
                context: None,
                max_tokens: Some(20), // ~80 chars
            },
        )
        .unwrap();
    assert!(result.truncated);
    let text = result.text.expect("doc text");
    assert!(text.chars().count() <= 100, "trimmed text len {}", text.chars().count());
}

#[test]
fn fetch_span_returns_line_range() {
    let env = common::TestEnv::new();
    // Use a list so the canonical-to-markdown roundtrip emits 5
    // single-line entries joined by `\n` (paragraphs would be joined by
    // `\n\n`, and CommonMark soft breaks inside one paragraph collapse to
    // spaces — see crates/kebab-parse-md/src/blocks.rs `Event::SoftBreak`).
    let body = "- Line one.\n- Line two.\n- Line three.\n- Line four.\n- Line five.\n";
    common::ingest_md(&env, "lines.md", body);
    let app = env.app();

    let q = kebab_core::SearchQuery {
        text: "Line".to_string(),
        mode: kebab_core::SearchMode::Lexical,
        k: 1,
        filters: kebab_core::SearchFilters::default(),
    };
    let hits = app.search(q).unwrap();
    let doc_id = hits[0].doc_id.clone();

    let result = app
        .fetch(
            FetchQuery::Span {
                doc_id,
                line_start: 2,
                line_end: 4,
            },
            FetchOpts::default(),
        )
        .unwrap();
    assert_eq!(result.kind, FetchKind::Span);
    let text = result.text.expect("span text");
    let line_count = text.lines().count();
    assert_eq!(line_count, 3, "span should be 3 lines: {text:?}");
    assert_eq!(result.line_start, Some(2));
    assert_eq!(result.line_end, Some(4));
    assert_eq!(result.effective_end, Some(4));
    assert!(!result.truncated);
}

#[test]
fn fetch_span_clamps_line_end_when_out_of_range() {
    let env = common::TestEnv::new();
    common::ingest_md(&env, "short.md", "Line one.\nLine two.\n");
    let app = env.app();
    let q = kebab_core::SearchQuery {
        text: "Line".to_string(),
        mode: kebab_core::SearchMode::Lexical,
        k: 1,
        filters: kebab_core::SearchFilters::default(),
    };
    let hits = app.search(q).unwrap();
    let doc_id = hits[0].doc_id.clone();

    let result = app
        .fetch(
            FetchQuery::Span {
                doc_id,
                line_start: 1,
                line_end: 999,
            },
            FetchOpts::default(),
        )
        .unwrap();
    let text = result.text.expect("span text");
    let actual_lines = text.lines().count();
    assert_eq!(result.effective_end, Some(actual_lines as u32));
    assert!(actual_lines < 999);
}

#[test]
fn fetch_span_invalid_input_when_zero_lines() {
    let env = common::TestEnv::new();
    common::ingest_md(&env, "a.md", "Line one.\n");
    let app = env.app();
    let q = kebab_core::SearchQuery {
        text: "Line".to_string(),
        mode: kebab_core::SearchMode::Lexical,
        k: 1,
        filters: kebab_core::SearchFilters::default(),
    };
    let hits = app.search(q).unwrap();
    let doc_id = hits[0].doc_id.clone();

    let err = app
        .fetch(
            FetchQuery::Span {
                doc_id,
                line_start: 0,
                line_end: 0,
            },
            FetchOpts::default(),
        )
        .unwrap_err();
    assert!(err.to_string().contains("invalid_input"), "got: {err}");
}

#[test]
fn fetch_span_line_start_beyond_total_returns_empty_text() {
    let env = common::TestEnv::new();
    let body = "- Line one.\n- Line two.\n";
    common::ingest_md(&env, "two_lines.md", body);
    let app = env.app();
    let q = kebab_core::SearchQuery {
        text: "Line".to_string(),
        mode: kebab_core::SearchMode::Lexical,
        k: 1,
        filters: kebab_core::SearchFilters::default(),
    };
    let hits = app.search(q).unwrap();
    let doc_id = hits[0].doc_id.clone();

    let result = app
        .fetch(
            FetchQuery::Span {
                doc_id,
                line_start: 100,
                line_end: 200,
            },
            FetchOpts::default(),
        )
        .unwrap();
    let text = result.text.expect("text field");
    assert!(text.is_empty(), "out-of-range request returns empty text");
    assert!(
        !result.truncated,
        "out-of-range is NOT truncated (budget-only flag)"
    );
}

#[test]
fn fetch_chunk_context_at_first_chunk_clamps_lower_bound() {
    let env = common::TestEnv::new();
    // Multi-chunk markdown so context ±N has neighbors.
    let body =
        "# H1\n\nFirst chunk text body.\n\n# H2\n\nSecond chunk.\n\n# H3\n\nThird chunk.\n";
    common::ingest_md(&env, "boundary.md", body);
    let app = env.app();
    let q = kebab_core::SearchQuery {
        text: "First".to_string(),
        mode: kebab_core::SearchMode::Lexical,
        k: 1,
        filters: kebab_core::SearchFilters::default(),
    };
    let hits = app.search(q).unwrap();
    let chunk_id = hits[0].chunk_id.clone();

    let result = app
        .fetch(
            FetchQuery::Chunk(chunk_id),
            FetchOpts {
                context: Some(2),
                max_tokens: None,
            },
        )
        .unwrap();
    // context_before may be empty if target is the first chunk;
    // context_after should have ≤ 2 entries. Both clamped at doc boundaries.
    assert!(
        result.context_before.len() + result.context_after.len() <= 4,
        "doc boundary should clamp ±N to fit chunk count"
    );
}
