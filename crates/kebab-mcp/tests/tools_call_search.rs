//! Integration: tools/call name=search — verify response is search_response.v1.

use std::fs;

use kebab_config::Config;
use kebab_core::SourceScope;
use kebab_mcp::{KebabAppState, KebabHandler};
use rmcp::model::RawContent;

fn minimal_config(data_dir: &std::path::Path, workspace_root: &std::path::Path) -> Config {
    let mut cfg = Config::defaults();
    cfg.storage.data_dir = data_dir.to_string_lossy().into_owned();
    cfg.storage.model_dir = data_dir.join("models").to_string_lossy().into_owned();
    cfg.workspace.root = Some(workspace_root.to_string_lossy().into_owned());
    cfg.workspace.exclude.clear();
    cfg.models.embedding.provider = "none".to_string();
    cfg.models.embedding.dimensions = 0;
    cfg
}

#[tokio::test]
async fn search_tool_returns_search_response_v1() {
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
            trace: None,
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
        Some("search_response.v1"),
        "envelope should carry schema_version=search_response.v1"
    );
    let hits = v
        .get("hits")
        .and_then(|h| h.as_array())
        .expect("hits must be a JSON array");
    assert!(
        !hits.is_empty(),
        "expected at least one hit for 'kebab' in 'a.md'"
    );
    assert_eq!(
        hits[0].get("schema_version").and_then(|s| s.as_str()),
        Some("search_hit.v1"),
        "first hit should carry schema_version=search_hit.v1"
    );
    // truncated must be present (bool); next_cursor may be null on last page.
    assert!(
        v.get("truncated")
            .and_then(serde_json::Value::as_bool)
            .is_some(),
        "envelope should carry truncated:bool"
    );
    assert!(
        v.get("next_cursor").is_some(),
        "envelope should carry next_cursor (possibly null)"
    );
}

/// p9-fb-36: search with doc_id filter — only hits from the target doc.
#[tokio::test]
async fn search_with_doc_id_filter_returns_only_target() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let workspace_root = dir.path().join("notes");
    fs::create_dir_all(&data_dir).unwrap();
    fs::create_dir_all(&workspace_root).unwrap();

    let config = minimal_config(&data_dir, &workspace_root);

    // Write two markdown documents, both containing the query term.
    fs::write(
        workspace_root.join("a.md"),
        "# Alpha\n\nThis document mentions kebab and flatbread.",
    )
    .unwrap();
    fs::write(
        workspace_root.join("b.md"),
        "# Beta\n\nAnother document about kebab wraps and fillings.",
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

    // First: unfiltered search to discover a doc_id from one of the docs.
    let unfiltered = kebab_mcp::tools::search::handle(
        handler.state(),
        kebab_mcp::tools::search::SearchInput {
            query: "kebab".to_string(),
            mode: Some("lexical".to_string()),
            k: Some(10),
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
    assert!(
        !unfiltered.is_error.unwrap_or(false),
        "unfiltered search failed: {unfiltered:?}"
    );
    let unfiltered_text = match &unfiltered.content.first().unwrap().raw {
        RawContent::Text(t) => t.text.clone(),
        other => panic!("expected text content, got {other:?}"),
    };
    let unfiltered_v: serde_json::Value = serde_json::from_str(&unfiltered_text).unwrap();
    let hits = unfiltered_v["hits"].as_array().expect("hits must be array");
    assert!(hits.len() >= 2, "expected hits from both docs");

    // Pick the doc_id of the first hit.
    let target_doc_id = hits[0]["doc_id"]
        .as_str()
        .expect("doc_id on first hit")
        .to_string();

    // Now search with doc_id filter — all results must belong to that doc.
    let filtered = kebab_mcp::tools::search::handle(
        handler.state(),
        kebab_mcp::tools::search::SearchInput {
            query: "kebab".to_string(),
            mode: Some("lexical".to_string()),
            k: Some(10),
            max_tokens: None,
            snippet_chars: None,
            cursor: None,
            tags: None,
            lang: None,
            path_glob: None,
            trust_min: None,
            media: None,
            ingested_after: None,
            doc_id: Some(target_doc_id.clone()),
            trace: None,
        },
    );
    assert!(
        !filtered.is_error.unwrap_or(false),
        "filtered search failed: {filtered:?}"
    );
    let filtered_text = match &filtered.content.first().unwrap().raw {
        RawContent::Text(t) => t.text.clone(),
        other => panic!("expected text content, got {other:?}"),
    };
    let filtered_v: serde_json::Value = serde_json::from_str(&filtered_text).unwrap();
    let filtered_hits = filtered_v["hits"].as_array().expect("hits must be array");

    assert!(
        !filtered_hits.is_empty(),
        "expected at least one hit for target doc"
    );
    for hit in filtered_hits {
        assert_eq!(
            hit["doc_id"].as_str(),
            Some(target_doc_id.as_str()),
            "all filtered hits must belong to the target doc"
        );
    }
}

/// p9-fb-36: invalid RFC3339 for ingested_after → invalid_input error.v1.
#[tokio::test]
async fn search_with_invalid_ingested_after_returns_invalid_input() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let workspace_root = dir.path().join("notes");
    fs::create_dir_all(&data_dir).unwrap();
    fs::create_dir_all(&workspace_root).unwrap();

    let config = minimal_config(&data_dir, &workspace_root);
    let state = KebabAppState::new(config, None);
    let handler = KebabHandler::new(state);

    let result = kebab_mcp::tools::search::handle(
        handler.state(),
        kebab_mcp::tools::search::SearchInput {
            query: "kebab".to_string(),
            mode: None,
            k: None,
            max_tokens: None,
            snippet_chars: None,
            cursor: None,
            tags: None,
            lang: None,
            path_glob: None,
            trust_min: None,
            media: None,
            ingested_after: Some("garbage".to_string()),
            doc_id: None,
            trace: None,
        },
    );

    assert!(
        result.is_error.unwrap_or(false),
        "expected isError=true for invalid ingested_after"
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
        "code must be invalid_input for bad RFC3339"
    );
}
