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
