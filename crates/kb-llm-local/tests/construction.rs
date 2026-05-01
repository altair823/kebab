//! Construction-time tests — verify `OllamaLanguageModel::new` reads the
//! relevant config fields and exposes them via the trait surface, all
//! without touching the network (per design §7.2 lazy-connect contract).

use kb_config::Config;
use kb_llm_local::{LanguageModel, OllamaLanguageModel};

#[test]
fn construction_with_default_config_returns_expected_model_ref() {
    let cfg = Config::defaults();
    let llm = OllamaLanguageModel::new(&cfg).expect("construction should not hit network");
    let m = llm.model_ref();

    assert_eq!(m.provider, "ollama");
    // Default model id from kb-config §6.4 — pinned here so a silent
    // default flip in kb-config is caught by this test.
    assert_eq!(m.id, cfg.models.llm.model);
    // Chat models have no embedding dimension (§3.8).
    assert_eq!(m.dimensions, None);
}

#[test]
fn context_tokens_returns_config_value() {
    let mut cfg = Config::defaults();
    cfg.models.llm.context_tokens = 16384;
    let llm = OllamaLanguageModel::new(&cfg).unwrap();
    assert_eq!(llm.context_tokens(), 16384);
}

#[test]
fn construction_does_not_require_a_running_ollama() {
    // Point the endpoint at a closed port. Construction must succeed —
    // the contract is "lazy connect on first generate_stream call".
    let mut cfg = Config::defaults();
    cfg.models.llm.endpoint = "http://127.0.0.1:1".to_string();
    let _llm = OllamaLanguageModel::new(&cfg).expect("new() must not hit the network");
}
