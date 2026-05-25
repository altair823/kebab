//! `ask` tool returns answer.v1 — refusal path covered (no Ollama
//! required for refusal-on-empty-corpus case).

use kebab_config::Config;
use kebab_core::SourceScope;
use kebab_mcp::{KebabAppState, KebabHandler};
use rmcp::model::RawContent;

fn minimal_config(data_dir: &std::path::Path, workspace_root: &std::path::Path) -> Config {
    let mut cfg = Config::defaults();
    cfg.storage.data_dir = data_dir.to_string_lossy().into_owned();
    cfg.storage.model_dir = data_dir
        .join("models")
        .to_string_lossy()
        .into_owned();
    cfg.workspace.root = workspace_root.to_string_lossy().into_owned();
    cfg.workspace.exclude.clear();
    cfg.models.embedding.provider = "none".to_string();
    cfg.models.embedding.dimensions = 0;
    cfg
}

#[tokio::test]
async fn ask_tool_returns_answer_v1_with_refusal_on_empty_kb() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let workspace_root = dir.path().join("notes");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(&workspace_root).unwrap();

    let cfg = minimal_config(&data_dir, &workspace_root);

    // Seed kebab.sqlite (empty corpus — no documents ingested).
    let scope = SourceScope {
        root: workspace_root.clone(),
        include: vec![],
        exclude: vec![],
    };
    let _ = kebab_app::ingest_with_config(cfg.clone(), scope, false).unwrap();

    let state = KebabAppState::new(cfg, None);
    let handler = KebabHandler::new(state);

    // `ask_with_config` builds a `reqwest::blocking::Client` internally (for
    // `OllamaLanguageModel`), which spins up and drops a tokio runtime — that
    // panics when called from inside an async context. Run it on the blocking
    // thread pool to avoid the conflict.
    let state_clone = handler.state().clone();
    let result = tokio::task::spawn_blocking(move || {
        kebab_mcp::tools::ask::handle(
            &state_clone,
            kebab_mcp::tools::ask::AskInput {
                query: "what is the meaning of life".to_string(),
                session_id: None,
                // Test env uses provider="none" — Hybrid would hard-error on embedding.
                // Pass Lexical explicitly so the test stays functional.
                mode: Some("lexical".to_string()),
                multi_hop: None,
            },
        )
    })
    .await
    .unwrap();

    // Empty KB → refusal (grounded:false) is normal — NOT isError.
    assert!(
        !result.is_error.unwrap_or(false),
        "expected isError=false on refusal, got {:?}",
        result
    );

    let content = result
        .content
        .first()
        .expect("expected at least one content item");

    let text = match &content.raw {
        RawContent::Text(t) => &t.text,
        other => panic!("expected text content, got {other:?}"),
    };

    let v: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(
        v.get("schema_version").and_then(|s| s.as_str()),
        Some("answer.v1"),
        "response should carry schema_version=answer.v1"
    );
    assert_eq!(
        v.get("grounded").and_then(|b| b.as_bool()),
        Some(false),
        "empty KB should produce grounded=false"
    );
}
