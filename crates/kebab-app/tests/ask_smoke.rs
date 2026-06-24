//! `kb-app::ask` smoke tests.
//!
//! The pipeline's behavior is exhaustively covered by `kb-rag` tests
//! (which inject `MockLanguageModel` + `MockRetriever`). The kb-app
//! facade is a thin component wirer: it picks the retriever per
//! `opts.mode` and constructs an `OllamaLanguageModel`. Exercising
//! that wiring requires a real Ollama on `127.0.0.1:11434`, so this
//! test is `#[ignore]` by default — run with `cargo test -p kb-app
//! --test ask_smoke -- --ignored` against a live Ollama.

mod common;

use common::TestEnv;

/// Lexical-mode ask end-to-end. Requires a real Ollama on
/// `config.models.llm.endpoint` (default `127.0.0.1:11434`) running the
/// configured model. The pipeline body is otherwise covered by kb-rag's
/// integration tests; this just verifies the facade composes the
/// components correctly.
#[test]
#[ignore = "requires real Ollama on 127.0.0.1:11434"]
fn ask_lexical_smoke() {
    let env = TestEnv::lexical_only();
    kebab_app::ingest_with_config(env.config.clone(), env.scope(), true).unwrap();

    let opts = kebab_app::AskOpts {
        k: 5,
        explain: false,
        mode: kebab_core::SearchMode::Lexical,
        temperature: Some(0.0),
        seed: Some(0),
        stream_sink: None,
        multi_hop: false,
    };
    // The fixture workspace contains "ownership" content; the model's
    // citation behavior depends on its training, so we don't assert on
    // grounded — only that the call returns a structurally-valid Answer.
    let answer = kebab_app::ask_with_config(env.config.clone(), "ownership", opts)
        .expect("ask returns Ok with a real Ollama backend");
    // retrieval summary always populated, regardless of grounded path.
    assert_eq!(answer.retrieval.mode, kebab_core::SearchMode::Lexical);
    assert!(answer.retrieval.k >= 5);
    assert!(answer.retrieval.trace_id.0.starts_with("ret_"));
}
