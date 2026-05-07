//! Integration: tools/call name=doctor — returns doctor.v1.

use kebab_config::Config;
use kebab_mcp::{KebabAppState, KebabHandler};
use rmcp::model::RawContent;

#[tokio::test]
async fn doctor_tool_returns_doctor_v1_json() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = Config::defaults();
    cfg.storage.data_dir = dir.path().join("data").to_string_lossy().into_owned();
    cfg.workspace.root = dir.path().join("notes").to_string_lossy().into_owned();
    cfg.models.embedding.provider = "none".to_string();
    cfg.models.embedding.dimensions = 0;
    std::fs::create_dir_all(&cfg.workspace.root).unwrap();

    // Pass None for config_path — doctor falls back to XDG default probe
    // (path won't exist in the tempdir, which is fine; doctor reports it
    // as missing / error rather than panicking).
    let state = KebabAppState::new(cfg, None);
    let handler = KebabHandler::new(state);

    let result = kebab_mcp::tools::doctor::handle(
        handler.state(),
        kebab_mcp::tools::doctor::DoctorInput::default(),
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
        Some("doctor.v1"),
        "unexpected schema_version in: {v}"
    );
    // `ok` boolean must be present (value may be false in CI where Ollama
    // is not reachable — that's expected and acceptable).
    assert!(
        v.get("ok").and_then(|b| b.as_bool()).is_some(),
        "`ok` field missing in doctor.v1 response: {v}"
    );
}
