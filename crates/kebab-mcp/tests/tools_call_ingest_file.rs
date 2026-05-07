//! Integration: tools/call name=ingest_file → ingest_report.v1.

use std::fs;

use kebab_config::Config;
use kebab_mcp::{KebabAppState, KebabHandler};
use rmcp::model::RawContent;

#[tokio::test]
async fn ingest_file_tool_returns_ingest_report_v1() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("notes");
    let data = dir.path().join("data");
    fs::create_dir_all(&workspace).unwrap();
    fs::create_dir_all(&data).unwrap();

    let mut cfg = Config::defaults();
    cfg.workspace.root = workspace.to_string_lossy().into_owned();
    cfg.storage.data_dir = data.to_string_lossy().into_owned();
    cfg.models.embedding.provider = "none".to_string();
    cfg.models.embedding.dimensions = 0;

    let src = dir.path().join("doc.md");
    fs::write(&src, "# Title\n\nbody.").unwrap();

    let state = KebabAppState::new(cfg, None);
    let handler = KebabHandler::new(state);

    let result = tokio::task::spawn_blocking({
        let state = handler.state().clone();
        let path = src.to_string_lossy().into_owned();
        move || {
            kebab_mcp::tools::ingest_file::handle(
                &state,
                kebab_mcp::tools::ingest_file::IngestFileInput { path },
            )
        }
    })
    .await
    .unwrap();

    assert!(!result.is_error.unwrap_or(false), "{result:?}");
    let text = match &result.content.first().unwrap().raw {
        RawContent::Text(t) => &t.text,
        other => panic!("expected text content, got {other:?}"),
    };
    let v: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(
        v.get("schema_version").and_then(|s| s.as_str()),
        Some("ingest_report.v1")
    );
    assert_eq!(v.get("new").and_then(|n| n.as_u64()), Some(1));
}

#[tokio::test]
async fn ingest_file_tool_idempotent_on_second_call() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("notes");
    let data = dir.path().join("data");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(&data).unwrap();

    let mut cfg = kebab_config::Config::defaults();
    cfg.workspace.root = workspace.to_string_lossy().into_owned();
    cfg.storage.data_dir = data.to_string_lossy().into_owned();
    cfg.models.embedding.provider = "none".to_string();
    cfg.models.embedding.dimensions = 0;

    let src = dir.path().join("doc.md");
    std::fs::write(&src, "# A\n\nbody.").unwrap();

    let state = kebab_mcp::KebabAppState::new(cfg, None);
    let handler = kebab_mcp::KebabHandler::new(state);

    // First call.
    let r1 = tokio::task::spawn_blocking({
        let state = handler.state().clone();
        let path = src.to_string_lossy().into_owned();
        move || {
            kebab_mcp::tools::ingest_file::handle(
                &state,
                kebab_mcp::tools::ingest_file::IngestFileInput { path },
            )
        }
    })
    .await
    .unwrap();
    assert!(!r1.is_error.unwrap_or(false));
    let text1 = match &r1.content.first().unwrap().raw {
        rmcp::model::RawContent::Text(t) => &t.text,
        other => panic!("expected text, got {other:?}"),
    };
    let v1: serde_json::Value = serde_json::from_str(text1).unwrap();
    assert_eq!(v1.get("new").and_then(|n| n.as_u64()), Some(1));

    // Second call — same content, expect unchanged=1.
    let r2 = tokio::task::spawn_blocking({
        let state = handler.state().clone();
        let path = src.to_string_lossy().into_owned();
        move || {
            kebab_mcp::tools::ingest_file::handle(
                &state,
                kebab_mcp::tools::ingest_file::IngestFileInput { path },
            )
        }
    })
    .await
    .unwrap();
    assert!(!r2.is_error.unwrap_or(false));
    let text2 = match &r2.content.first().unwrap().raw {
        rmcp::model::RawContent::Text(t) => &t.text,
        other => panic!("expected text, got {other:?}"),
    };
    let v2: serde_json::Value = serde_json::from_str(text2).unwrap();
    assert_eq!(v2.get("new").and_then(|n| n.as_u64()), Some(0), "{v2:?}");
    assert_eq!(v2.get("unchanged").and_then(|n| n.as_u64()), Some(1), "{v2:?}");
}
