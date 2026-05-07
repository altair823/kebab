//! Integration: tools/call name=ingest_stdin → ingest_report.v1.
//! Frontmatter precheck path also covered.

use std::fs;

use kebab_config::Config;
use kebab_mcp::KebabAppState;
use rmcp::model::RawContent;

fn fresh_state(dir: &std::path::Path) -> KebabAppState {
    let workspace = dir.join("notes");
    let data = dir.join("data");
    fs::create_dir_all(&workspace).unwrap();
    fs::create_dir_all(&data).unwrap();

    let mut cfg = Config::defaults();
    cfg.workspace.root = workspace.to_string_lossy().into_owned();
    cfg.storage.data_dir = data.to_string_lossy().into_owned();
    cfg.models.embedding.provider = "none".to_string();
    cfg.models.embedding.dimensions = 0;
    KebabAppState::new(cfg, None)
}

#[tokio::test]
async fn ingest_stdin_tool_returns_ingest_report_v1() {
    let dir = tempfile::tempdir().unwrap();
    let state = fresh_state(dir.path());

    let result = tokio::task::spawn_blocking({
        let state = state.clone();
        move || {
            kebab_mcp::tools::ingest_stdin::handle(
                &state,
                kebab_mcp::tools::ingest_stdin::IngestStdinInput {
                    content: "## Body".to_string(),
                    title: "X".to_string(),
                    source_uri: Some("https://example.com/x".to_string()),
                },
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
async fn ingest_stdin_tool_emits_error_v1_on_existing_frontmatter() {
    let dir = tempfile::tempdir().unwrap();
    let state = fresh_state(dir.path());

    let result = tokio::task::spawn_blocking({
        let state = state.clone();
        move || {
            kebab_mcp::tools::ingest_stdin::handle(
                &state,
                kebab_mcp::tools::ingest_stdin::IngestStdinInput {
                    content: "---\ntitle: Existing\n---\n\n## Body".to_string(),
                    title: "New".to_string(),
                    source_uri: None,
                },
            )
        }
    })
    .await
    .unwrap();

    assert_eq!(result.is_error, Some(true), "{result:?}");
    let text = match &result.content.first().unwrap().raw {
        RawContent::Text(t) => &t.text,
        other => panic!("expected text content, got {other:?}"),
    };
    let v: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(
        v.get("schema_version").and_then(|s| s.as_str()),
        Some("error.v1")
    );
}
