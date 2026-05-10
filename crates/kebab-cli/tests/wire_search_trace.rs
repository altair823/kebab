//! p9-fb-37: integration tests for `kebab search --trace --json`.

mod common;

use serde_json::Value;
use std::fs;

#[test]
fn search_trace_json_includes_trace_block() {
    let dir = tempfile::tempdir().unwrap();
    let (cfg, workspace, _data) = common::write_config(dir.path(), 0);
    fs::write(workspace.join("doc1.md"), "# Title\n\nrust async hello\n").unwrap();
    common::ingest(&cfg, &workspace);

    let (stdout, _stderr) = common::run_search_with_args(
        &cfg,
        &["--mode", "lexical", "--trace", "--json", "rust"],
    );
    let v: Value = serde_json::from_str(stdout.trim()).expect("valid JSON");
    assert_eq!(v["schema_version"], "search_response.v1");
    assert!(v["trace"].is_object(), "trace block present");
    assert!(v["trace"]["timing"].is_object());
    assert!(v["trace"]["timing"]["total_ms"].is_number());
    assert!(v["trace"]["lexical"].is_array());
    assert!(v["trace"]["vector"].is_array());
    assert!(v["trace"]["rrf_inputs"].is_array());
}

#[test]
fn search_without_trace_omits_trace_field() {
    let dir = tempfile::tempdir().unwrap();
    let (cfg, workspace, _data) = common::write_config(dir.path(), 0);
    fs::write(workspace.join("doc1.md"), "# Title\n\nrust async hello\n").unwrap();
    common::ingest(&cfg, &workspace);

    let (stdout, _stderr) = common::run_search_with_args(
        &cfg,
        &["--mode", "lexical", "--json", "rust"],
    );
    let v: Value = serde_json::from_str(stdout.trim()).expect("valid JSON");
    assert!(v.get("trace").is_none(), "trace field absent without --trace");
}

#[test]
fn search_trace_lexical_mode_vector_list_empty() {
    let dir = tempfile::tempdir().unwrap();
    let (cfg, workspace, _data) = common::write_config(dir.path(), 0);
    fs::write(workspace.join("doc1.md"), "# Title\n\nrust async hello\n").unwrap();
    common::ingest(&cfg, &workspace);

    let (stdout, _stderr) = common::run_search_with_args(
        &cfg,
        &["--mode", "lexical", "--trace", "--json", "rust"],
    );
    let v: Value = serde_json::from_str(stdout.trim()).expect("valid JSON");
    assert_eq!(v["trace"]["vector"].as_array().unwrap().len(), 0);
    assert_eq!(v["trace"]["timing"]["vector_ms"], 0);
}
