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
    // v0.17.0 trigram tokenizer: terms must be ≥3 Unicode chars to
    // match. The earlier fixture used 2-char tokens like `A1`/`A3` for
    // section bodies — those zero-hit under trigram. Use 5-char unique
    // words per section so the query can pin one chunk deterministically.
    let body = "# H1\n\napples\n\n# H2\n\nbanana\n\n# H3\n\ncherry\n\n# H4\n\ndurian\n\n# H5\n\nelder\n";
    common::ingest_md(&env, "multi.md", body);
    let app = env.app();

    let q = kebab_core::SearchQuery {
        text: "cherry".to_string(),
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
    // p9-fb-35 R2: doc has 3 chunks; ±2 should clamp the total
    // neighbor count to ≤ 2 + 1 (= excludes target).
    //
    // ⚠ Strict "first-chunk → context_before is empty" cannot be
    // asserted here yet because chunks.ordinal column does not exist
    // — `list_chunk_ids_for_doc` orders by `(created_at, chunk_id)`
    // and chunk_id is a blake3 hash, so the "First chunk" content
    // may land at any hash-order position within the doc. The clamp
    // logic itself is correct (target_idx ± n → [0..len]); we just
    // can't pin which chunk is hash-order-first. Tracked as
    // follow-up: V007 chunks.ordinal migration.
    let total = result.context_before.len() + result.context_after.len();
    assert!(
        total <= 2,
        "doc with 3 chunks ±2 → at most 2 neighbors (excludes target), got {total}"
    );
}
