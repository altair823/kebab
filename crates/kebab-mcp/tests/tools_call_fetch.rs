//! p9-fb-35: tools/call name=fetch — chunk happy path + invalid_input.
//!
//! Mirrors `tools_call_search.rs` setup: a TempDir KB with embedding
//! provider = "none" (no Ollama / fastembed) and a single ingested
//! markdown doc. We discover a `chunk_id` via the search tool, call
//! `fetch` with it, then exercise the missing-arg branch separately.

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

#[tokio::test]
async fn fetch_tool_chunk_returns_fetch_result_v1() {
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

    // Discover a chunk_id via the search tool.
    let search_result = kebab_mcp::tools::search::handle(
        handler.state(),
        kebab_mcp::tools::search::SearchInput {
            query: "kebab".to_string(),
            mode: Some("lexical".to_string()),
            k: Some(1),
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
            trace: None,
        },
    );
    let search_text = match &search_result.content.first().unwrap().raw {
        RawContent::Text(t) => t.text.clone(),
        other => panic!("expected text content, got {other:?}"),
    };
    let search_v: serde_json::Value = serde_json::from_str(&search_text).unwrap();
    let chunk_id = search_v["hits"][0]["chunk_id"]
        .as_str()
        .expect("chunk_id on first hit")
        .to_string();

    // Call fetch with kind=chunk.
    let result = kebab_mcp::tools::fetch::handle(
        handler.state(),
        kebab_mcp::tools::fetch::FetchInput {
            kind: "chunk".to_string(),
            chunk_id: Some(chunk_id),
            doc_id: None,
            line_start: None,
            line_end: None,
            context: None,
            max_tokens: None,
        },
    );

    assert!(
        !result.is_error.unwrap_or(false),
        "expected isError=false, got {result:?}"
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
        Some("fetch_result.v1"),
        "envelope must carry schema_version=fetch_result.v1"
    );
    assert_eq!(
        v.get("kind").and_then(|s| s.as_str()),
        Some("chunk"),
        "kind must be 'chunk'"
    );
    assert!(
        v.get("chunk").is_some_and(serde_json::Value::is_object),
        "chunk payload must be populated for kind=chunk"
    );
}

#[tokio::test]
async fn fetch_tool_invalid_kind_returns_invalid_input() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let workspace_root = dir.path().join("notes");
    fs::create_dir_all(&data_dir).unwrap();
    fs::create_dir_all(&workspace_root).unwrap();

    let config = minimal_config(&data_dir, &workspace_root);

    let state = KebabAppState::new(config, None);
    let handler = KebabHandler::new(state);

    let result = kebab_mcp::tools::fetch::handle(
        handler.state(),
        kebab_mcp::tools::fetch::FetchInput {
            kind: "garbage".to_string(),
            chunk_id: None,
            doc_id: None,
            line_start: None,
            line_end: None,
            context: None,
            max_tokens: None,
        },
    );

    assert!(
        result.is_error.unwrap_or(false),
        "expected isError=true for unknown kind"
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
        Some("error.v1"),
        "must carry error.v1 envelope"
    );
    assert_eq!(
        v.get("code").and_then(|s| s.as_str()),
        Some("invalid_input"),
        "code must be invalid_input for unknown kind"
    );
}

#[tokio::test]
async fn fetch_tool_chunk_missing_id_returns_invalid_input() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let workspace_root = dir.path().join("notes");
    fs::create_dir_all(&data_dir).unwrap();
    fs::create_dir_all(&workspace_root).unwrap();

    let config = minimal_config(&data_dir, &workspace_root);

    let state = KebabAppState::new(config, None);
    let handler = KebabHandler::new(state);

    // kind=chunk but no chunk_id — invalid_input.
    let result = kebab_mcp::tools::fetch::handle(
        handler.state(),
        kebab_mcp::tools::fetch::FetchInput {
            kind: "chunk".to_string(),
            chunk_id: None,
            doc_id: None,
            line_start: None,
            line_end: None,
            context: None,
            max_tokens: None,
        },
    );

    assert!(
        result.is_error.unwrap_or(false),
        "expected isError=true when chunk_id is missing"
    );
    let content = result.content.first().unwrap();
    let text = match &content.raw {
        RawContent::Text(t) => &t.text,
        other => panic!("expected text content, got {other:?}"),
    };
    let v: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(
        v.get("code").and_then(|s| s.as_str()),
        Some("invalid_input")
    );
}
