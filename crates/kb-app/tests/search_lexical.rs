//! Lexical search integration tests. The vector / hybrid lanes are
//! AVX-gated and live in `search_vector.rs` (`#[ignore]`).

mod common;

use common::TestEnv;

fn lexical_query(text: &str) -> kb_core::SearchQuery {
    kb_core::SearchQuery {
        text: text.to_string(),
        mode: kb_core::SearchMode::Lexical,
        k: 10,
        filters: kb_core::SearchFilters::default(),
    }
}

#[test]
fn lexical_search_returns_hits_after_ingest() {
    let env = TestEnv::lexical_only();
    kb_app::ingest_with_config(env.config.clone(), env.scope(), true).unwrap();

    // "Ownership" appears as a heading + paragraph in intro.md and
    // matches FTS5 default tokenizer easily.
    let hits =
        kb_app::search_with_config(env.config.clone(), lexical_query("ownership"))
            .unwrap();
    assert!(!hits.is_empty(), "expected ≥1 hit for 'ownership'");

    for h in &hits {
        // Lexical retriever sets embedding_model=None per spec.
        assert!(
            h.embedding_model.is_none(),
            "lexical-mode hit must have None embedding_model: {h:?}"
        );
        assert_eq!(
            h.retrieval.method,
            kb_core::SearchMode::Lexical,
            "method label should be Lexical"
        );
    }
}

#[test]
fn lexical_search_empty_query_returns_empty() {
    let env = TestEnv::lexical_only();
    kb_app::ingest_with_config(env.config.clone(), env.scope(), true).unwrap();
    let hits = kb_app::search_with_config(env.config.clone(), lexical_query("   "))
        .unwrap();
    assert!(hits.is_empty(), "blank query must short-circuit empty");
}

#[test]
fn vector_mode_with_provider_none_errors_clearly() {
    let env = TestEnv::lexical_only();
    kb_app::ingest_with_config(env.config.clone(), env.scope(), true).unwrap();

    let q = kb_core::SearchQuery {
        text: "ownership".to_string(),
        mode: kb_core::SearchMode::Vector,
        k: 10,
        filters: kb_core::SearchFilters::default(),
    };
    let err = kb_app::search_with_config(env.config.clone(), q).unwrap_err();
    let msg = format!("{err:#}");
    assert!(
        msg.contains("embeddings disabled") || msg.contains("disabled"),
        "error must mention embeddings disabled: {msg}"
    );
}
