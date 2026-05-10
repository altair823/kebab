//! p9-fb-42: integration tests for `mcp__kebab__bulk_search`.

use std::fs;

use kebab_config::Config;
use kebab_core::SourceScope;
use kebab_mcp::{KebabAppState, KebabHandler};
use rmcp::model::RawContent;
use serde_json::json;

fn minimal_config(data_dir: &std::path::Path, workspace_root: &std::path::Path) -> Config {
    let mut cfg = Config::defaults();
    cfg.storage.data_dir = data_dir.to_string_lossy().into_owned();
    cfg.storage.model_dir = data_dir.join("models").to_string_lossy().into_owned();
    cfg.workspace.root = workspace_root.to_string_lossy().into_owned();
    cfg.workspace.exclude.clear();
    cfg.models.embedding.provider = "none".to_string();
    cfg.models.embedding.dimensions = 0;
    cfg
}

fn setup() -> (tempfile::TempDir, KebabHandler) {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let workspace_root = dir.path().join("notes");
    fs::create_dir_all(&data_dir).unwrap();
    fs::create_dir_all(&workspace_root).unwrap();
    let config = minimal_config(&data_dir, &workspace_root);
    fs::write(
        workspace_root.join("a.md"),
        "# Alpha\n\nThis document mentions kebab and bread.",
    )
    .unwrap();
    let scope = SourceScope { root: workspace_root.clone(), include: vec![], exclude: vec![] };
    let _ = kebab_app::ingest_with_config(config.clone(), scope, false).unwrap();
    let state = KebabAppState::new(config, None);
    let handler = KebabHandler::new(state);
    (dir, handler)
}

fn extract_json(result: &rmcp::model::CallToolResult) -> serde_json::Value {
    assert!(!result.is_error.unwrap_or(false), "expected isError=false, got {result:?}");
    let content = result.content.first().expect("at least one content item");
    let text = match &content.raw {
        RawContent::Text(t) => &t.text,
        other => panic!("expected Text content, got {other:?}"),
    };
    serde_json::from_str(text).expect("valid JSON")
}

#[tokio::test]
async fn bulk_search_two_queries_returns_envelope() {
    let (_dir, handler) = setup();
    let input = kebab_mcp::tools::bulk_search::BulkSearchInput {
        queries: vec![
            json!({"query": "kebab", "mode": "lexical", "k": 5}),
            json!({"query": "bread", "mode": "lexical", "k": 5}),
        ],
    };
    let result = kebab_mcp::tools::bulk_search::handle(handler.state(), input);
    let v = extract_json(&result);
    assert_eq!(v["schema_version"], "bulk_search_response.v1");
    let results = v["results"].as_array().expect("results array");
    assert_eq!(results.len(), 2);
    for r in results {
        assert_eq!(r["schema_version"], "bulk_search_item.v1");
        assert!(r["response"].is_object());
        assert!(r["error"].is_null());
    }
    assert_eq!(v["summary"]["total"], 2);
    assert_eq!(v["summary"]["succeeded"], 2);
    assert_eq!(v["summary"]["failed"], 0);
}

#[tokio::test]
async fn bulk_search_empty_queries_returns_empty_envelope() {
    let (_dir, handler) = setup();
    let input = kebab_mcp::tools::bulk_search::BulkSearchInput { queries: vec![] };
    let result = kebab_mcp::tools::bulk_search::handle(handler.state(), input);
    let v = extract_json(&result);
    assert_eq!(v["schema_version"], "bulk_search_response.v1");
    assert_eq!(v["results"].as_array().unwrap().len(), 0);
    assert_eq!(v["summary"]["total"], 0);
}

#[tokio::test]
async fn bulk_search_invalid_item_field_continues_with_per_item_error() {
    let (_dir, handler) = setup();
    let input = kebab_mcp::tools::bulk_search::BulkSearchInput {
        queries: vec![
            json!({"query": "kebab", "mode": "lexical"}),
            json!({"query": "bread", "mode": "bogus"}),  // invalid mode
        ],
    };
    let result = kebab_mcp::tools::bulk_search::handle(handler.state(), input);
    let v = extract_json(&result);
    let results = v["results"].as_array().unwrap();
    assert_eq!(results.len(), 2);
    assert!(results[0]["error"].is_null());
    assert!(results[1]["error"].is_object());
    assert_eq!(results[1]["error"]["code"], "invalid_input");
    assert_eq!(v["summary"]["succeeded"], 1);
    assert_eq!(v["summary"]["failed"], 1);
}

#[tokio::test]
async fn bulk_search_over_cap_returns_tool_error() {
    let (_dir, handler) = setup();
    let queries: Vec<serde_json::Value> = (0..101)
        .map(|_| json!({"query": "x", "mode": "lexical"}))
        .collect();
    let input = kebab_mcp::tools::bulk_search::BulkSearchInput { queries };
    let result = kebab_mcp::tools::bulk_search::handle(handler.state(), input);
    assert!(result.is_error.unwrap_or(false), "expected isError=true");
    let content = result.content.first().expect("error content");
    let text = match &content.raw {
        RawContent::Text(t) => &t.text,
        other => panic!("expected Text content, got {other:?}"),
    };
    assert!(text.contains("max 100"), "expected 'max 100' in error: {text}");
}
