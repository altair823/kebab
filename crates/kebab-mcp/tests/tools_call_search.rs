//! Integration: tools/call name=search — verify response is search_hit.v1 array.

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
async fn search_tool_returns_search_hits_array() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let workspace_root = dir.path().join("notes");
    fs::create_dir_all(&data_dir).unwrap();
    fs::create_dir_all(&workspace_root).unwrap();

    let config = minimal_config(&data_dir, &workspace_root);

    // Write a markdown document containing the query term.
    fs::write(
        workspace_root.join("a.md"),
        "# Alpha\n\nThis document mentions kebab and bread.",
    )
    .unwrap();

    // Seed kebab.sqlite via ingest so search has indexed content.
    let scope = SourceScope {
        root: workspace_root.clone(),
        include: vec![],
        exclude: vec![],
    };
    let _ = kebab_app::ingest_with_config(config.clone(), scope, false).unwrap();

    let state = KebabAppState::new(config, None);
    let handler = KebabHandler::new(state);

    let result = kebab_mcp::tools::search::handle(
        handler.state(),
        kebab_mcp::tools::search::SearchInput {
            query: "kebab".to_string(),
            mode: "lexical".to_string(),
            k: 5,
        },
    );

    assert!(
        !result.is_error.unwrap_or(false),
        "expected isError=false, got {:?}",
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
    let arr = v.as_array().expect("search returns a JSON array");
    assert!(
        !arr.is_empty(),
        "expected at least one hit for 'kebab' in 'a.md'"
    );
    assert_eq!(
        arr[0]
            .get("schema_version")
            .and_then(|s| s.as_str()),
        Some("search_hit.v1"),
        "first hit should carry schema_version=search_hit.v1"
    );
}
