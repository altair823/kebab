//! p9-fb-34: CLI search wire wrapper + budget controls.
//!
//! Lexical-only — no fastembed / no Ollama. Each test builds its own
//! TempDir KB via `common::write_config` + `common::ingest` and drives
//! `kebab search` through `common::run_search_with_args`. Verifies:
//!
//! - `--json` emits the `search_response.v1` wrapper (hits + cursor +
//!   truncated).
//! - `--max-tokens` flips `truncated: true` once the budget binds.
//! - `--cursor` advances paging (page 2 chunk_ids disjoint from page 1).
//! - Plain (non-JSON) output prints the `[truncated; ...]` hint to
//!   stderr (stdout stays the hit list).

mod common;

use serde_json::Value;
use std::fs;

#[test]
fn search_json_emits_search_response_v1_wrapper() {
    let dir = tempfile::tempdir().unwrap();
    let (cfg, workspace, _data) = common::write_config(dir.path(), 30);
    fs::write(workspace.join("a.md"), "# T\n\napples are red.\n").unwrap();
    common::ingest(&cfg, &workspace);

    let (stdout, _stderr) = common::run_search_with_args(
        &cfg,
        &["--json", "--mode", "lexical", "apples"],
    );
    let v: Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("not JSON: {stdout:?}: {e}"));
    assert_eq!(v["schema_version"], "search_response.v1");
    assert!(v["hits"].is_array(), "hits must be array, got {v}");
    assert!(
        v["next_cursor"].is_null() || v["next_cursor"].is_string(),
        "next_cursor must be null or string, got {}",
        v["next_cursor"]
    );
    assert!(
        v["truncated"].is_boolean(),
        "truncated must be bool, got {}",
        v["truncated"]
    );
}

#[test]
fn search_json_truncates_with_max_tokens() {
    let dir = tempfile::tempdir().unwrap();
    let (cfg, workspace, _data) = common::write_config(dir.path(), 30);
    let body: String = "rust ownership is a memory model. ".repeat(10);
    fs::write(workspace.join("a.md"), format!("# T\n\n{body}\n")).unwrap();
    common::ingest(&cfg, &workspace);

    let (stdout, _stderr) = common::run_search_with_args(
        &cfg,
        &["--json", "--mode", "lexical", "--max-tokens", "30", "rust"],
    );
    let v: Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("not JSON: {stdout:?}: {e}"));
    assert_eq!(
        v["truncated"], true,
        "30-token cap must trip truncation: {v}"
    );
}

#[test]
fn search_json_cursor_paginates() {
    let dir = tempfile::tempdir().unwrap();
    let (cfg, workspace, _data) = common::write_config(dir.path(), 30);
    for i in 0..6 {
        fs::write(
            workspace.join(format!("d{i}.md")),
            format!("# T{i}\n\nrust topic {i}\n"),
        )
        .unwrap();
    }
    common::ingest(&cfg, &workspace);

    let (page1, _) = common::run_search_with_args(
        &cfg,
        &["--json", "--mode", "lexical", "--k", "2", "rust"],
    );
    let v1: Value = serde_json::from_str(page1.trim())
        .unwrap_or_else(|e| panic!("page1 not JSON: {page1:?}: {e}"));
    let cursor = v1["next_cursor"]
        .as_str()
        .unwrap_or_else(|| panic!("next_cursor missing on page1: {v1}"));

    let (page2, _) = common::run_search_with_args(
        &cfg,
        &[
            "--json",
            "--mode",
            "lexical",
            "--k",
            "2",
            "--cursor",
            cursor,
            "rust",
        ],
    );
    let v2: Value = serde_json::from_str(page2.trim())
        .unwrap_or_else(|e| panic!("page2 not JSON: {page2:?}: {e}"));

    let p1_ids: Vec<String> = v1["hits"]
        .as_array()
        .expect("page1 hits array")
        .iter()
        .map(|h| {
            h["chunk_id"]
                .as_str()
                .expect("chunk_id string")
                .to_string()
        })
        .collect();
    let p2_ids: Vec<String> = v2["hits"]
        .as_array()
        .expect("page2 hits array")
        .iter()
        .map(|h| {
            h["chunk_id"]
                .as_str()
                .expect("chunk_id string")
                .to_string()
        })
        .collect();
    assert!(
        !p2_ids.is_empty(),
        "page2 must return at least one hit (cursor advanced past page1)"
    );
    assert!(
        p2_ids.iter().all(|id| !p1_ids.contains(id)),
        "page2 must not repeat page1 chunk_ids: page1={p1_ids:?} page2={p2_ids:?}"
    );
}

#[test]
fn search_plain_emits_truncated_hint_to_stderr() {
    let dir = tempfile::tempdir().unwrap();
    let (cfg, workspace, _data) = common::write_config(dir.path(), 30);
    let body: String = "rust ownership is a memory model. ".repeat(10);
    fs::write(workspace.join("a.md"), format!("# T\n\n{body}\n")).unwrap();
    common::ingest(&cfg, &workspace);

    let (_stdout, stderr) = common::run_search_with_args(
        &cfg,
        &["--mode", "lexical", "--max-tokens", "30", "rust"],
    );
    assert!(
        stderr.contains("[truncated;"),
        "stderr must carry truncated hint: {stderr:?}"
    );
}
