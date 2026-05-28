//! tools/call with bad config → isError=true + error.v1 content.

use kebab_config::Config;
use kebab_mcp::{KebabAppState, KebabHandler};
use rmcp::model::RawContent;

#[tokio::test]
async fn schema_tool_emits_error_v1_when_db_missing() {
    // Point at a directory that does NOT have kebab.sqlite.
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = Config::defaults();
    cfg.storage.data_dir = dir.path().to_string_lossy().into_owned();
    cfg.workspace.root = dir.path().join("notes").to_string_lossy().into_owned();
    cfg.models.embedding.provider = "none".to_string();
    cfg.models.embedding.dimensions = 0;
    // Note: NO ingest call — kebab.sqlite is absent → schema_with_config
    // calls open_existing → NotIndexed → tool error.

    let state = KebabAppState::new(cfg, None);
    let handler = KebabHandler::new(state);

    let result = kebab_mcp::tools::schema::handle(
        handler.state(),
        kebab_mcp::tools::schema::SchemaInput::default(),
    );
    assert_eq!(
        result.is_error,
        Some(true),
        "expected isError=true on missing DB"
    );

    let content = result.content.first().unwrap();
    let text = match &content.raw {
        RawContent::Text(t) => &t.text,
        other => panic!("expected text content, got {other:?}"),
    };
    let v: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(
        v.get("schema_version").and_then(|s| s.as_str()),
        Some("error.v1")
    );
    assert_eq!(v.get("code").and_then(|s| s.as_str()), Some("not_indexed"));
}
