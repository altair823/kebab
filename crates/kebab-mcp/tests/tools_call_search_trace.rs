//! p9-fb-37: integration test for `mcp__kebab__search` trace input/output.

use std::fs;

use kebab_config::Config;
use kebab_core::SourceScope;
use kebab_mcp::{KebabAppState, KebabHandler};
use rmcp::model::RawContent;

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
    let scope = SourceScope {
        root: workspace_root.clone(),
        include: vec![],
        exclude: vec![],
    };
    let _ = kebab_app::ingest_with_config(config.clone(), scope, false).unwrap();
    let state = KebabAppState::new(config, None);
    let handler = KebabHandler::new(state);
    (dir, handler)
}

fn make_input(trace: Option<bool>) -> kebab_mcp::tools::search::SearchInput {
    kebab_mcp::tools::search::SearchInput {
        query: "kebab".to_string(),
        mode: Some("lexical".to_string()),
        k: Some(5),
        max_tokens: None,
        snippet_chars: None,
        cursor: None,
        tags: None,
        lang: None,
        path_glob: None,
        trust_min: None,
        media: None,
        ingested_after: None,
        doc_id: None,
        trace,
    }
}

fn extract_json(result: &rmcp::model::CallToolResult) -> serde_json::Value {
    assert!(
        !result.is_error.unwrap_or(false),
        "expected isError=false, got {result:?}"
    );
    let content = result.content.first().expect("at least one content item");
    let text = match &content.raw {
        RawContent::Text(t) => &t.text,
        other => panic!("expected Text content, got {other:?}"),
    };
    serde_json::from_str(text).expect("valid JSON")
}

#[tokio::test]
async fn search_with_trace_true_returns_trace_field() {
    let (_dir, handler) = setup();
    let result = kebab_mcp::tools::search::handle(handler.state(), make_input(Some(true)));
    let v = extract_json(&result);
    assert_eq!(v["schema_version"], "search_response.v1");
    assert!(
        v["trace"].is_object(),
        "trace field present when trace:true"
    );
    assert!(v["trace"]["timing"]["total_ms"].is_number());
    assert!(v["trace"]["lexical"].is_array());
    assert!(v["trace"]["vector"].is_array());
    assert!(v["trace"]["rrf_inputs"].is_array());
}

#[tokio::test]
async fn search_without_trace_omits_trace_field() {
    let (_dir, handler) = setup();
    let result = kebab_mcp::tools::search::handle(handler.state(), make_input(None));
    let v = extract_json(&result);
    assert_eq!(v["schema_version"], "search_response.v1");
    assert!(v.get("trace").is_none(), "trace absent when None");
}

#[tokio::test]
async fn search_with_trace_false_omits_trace_field() {
    let (_dir, handler) = setup();
    let result = kebab_mcp::tools::search::handle(handler.state(), make_input(Some(false)));
    let v = extract_json(&result);
    assert!(v.get("trace").is_none(), "trace absent when false");
}
