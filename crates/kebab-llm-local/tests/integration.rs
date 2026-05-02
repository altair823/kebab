//! Real-Ollama integration tests, gated behind `#[ignore]`.
//!
//! Run with:
//!
//! ```bash
//! ollama serve &              # if not already running
//! ollama pull qwen2.5:7b-instruct
//! cargo test -p kb-llm-local -- --ignored
//! ```
//!
//! These hit `http://127.0.0.1:11434` directly and require an actual model
//! pulled locally. CI runs default (non-ignored) tests only.

use kebab_config::Config;
use kebab_core::{GenerateRequest, TokenChunk};
use kebab_llm_local::{LanguageModel, OllamaLanguageModel};

#[test]
#[ignore = "requires a local Ollama daemon + pulled model"]
fn real_ollama_streams_non_empty_response() {
    // Use whatever model the workspace defaults select. Override via the
    // KEBAB_MODELS_LLM_MODEL env var if you want a different one for this run
    // (e.g. `KEBAB_MODELS_LLM_MODEL=qwen2.5:7b-instruct cargo test ... -- --ignored`).
    let cfg = Config::load(None).expect("config should load");
    let llm = OllamaLanguageModel::new(&cfg).unwrap();

    let req = GenerateRequest {
        system: "You are a terse assistant.".to_string(),
        user: "Say only the word 'ok'.".to_string(),
        stop: vec![],
        max_tokens: 8,
        temperature: 0.0,
        seed: Some(0),
        images: Vec::new(),
    };

    let stream = llm.generate_stream(req).expect("stream should start");
    let chunks: Vec<TokenChunk> = stream
        .collect::<Result<Vec<_>, _>>()
        .expect("stream should not error");

    let text: String = chunks
        .iter()
        .filter_map(|c| match c {
            TokenChunk::Token(t) => Some(t.as_str()),
            _ => None,
        })
        .collect();
    assert!(!text.is_empty(), "expected non-empty completion");
    assert!(
        matches!(chunks.last(), Some(TokenChunk::Done { .. })),
        "stream must end with Done"
    );
}
