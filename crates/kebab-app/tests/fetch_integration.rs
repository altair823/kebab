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
