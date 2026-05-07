//! Integration: tools/call name=schema — verify response is schema.v1.

use std::fs;

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
async fn schema_tool_returns_schema_v1_json() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let workspace_root = dir.path().join("notes");
    fs::create_dir_all(&data_dir).unwrap();
    fs::create_dir_all(&workspace_root).unwrap();

    let config = minimal_config(&data_dir, &workspace_root);

    // Seed kebab.sqlite via 0-file ingest so open_existing succeeds later.
    let scope = SourceScope {
        root: workspace_root.clone(),
        include: vec![],
        exclude: vec![],
    };
    let _ = kebab_app::ingest_with_config(config.clone(), scope, false).unwrap();

    let state = KebabAppState::new(config, None);
    let handler = KebabHandler::new(state);

    let result = kebab_mcp::tools::schema::handle(
        handler.state(),
        kebab_mcp::tools::schema::SchemaInput::default(),
    );

    assert!(
        !result.is_error.unwrap_or(false),
        "expected isError=false on healthy schema, got {:?}",
        result
    );

    let content = result.content.first().expect("expected at least one content item");

    // Content = Annotated<RawContent>; deref to get the inner RawContent.
    let text = match &content.raw {
        RawContent::Text(t) => &t.text,
        other => panic!("expected text content, got {other:?}"),
    };

    let v: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(
        v.get("schema_version").and_then(|s| s.as_str()),
        Some("schema.v1"),
        "unexpected schema_version in: {v}"
    );
    assert_eq!(
        v.get("capabilities").and_then(|c| c.get("mcp_server")).and_then(|b| b.as_bool()),
        Some(true),
        "mcp_server capability flag should be true after fb-30",
    );
}
