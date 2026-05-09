//! p9-fb-35: CLI fetch wire shape + plain output + exit codes.
//!
//! Lexical-only — no fastembed / no Ollama. Each test builds its own
//! TempDir KB via `common::write_config` + `common::ingest` and drives
//! `kebab fetch` through `common::run_fetch_with_args`. Verifies:
//!
//! - `--json fetch chunk <id>` emits the `fetch_result.v1` wrapper
//!   with `kind = "chunk"` and a populated `chunk` object.
//! - `--json fetch doc <id> --max-tokens N` flips `truncated: true`
//!   once the budget binds.
//! - Unknown `chunk_id` exits non-zero and emits an `error.v1`
//!   ndjson line on stderr with `code = "chunk_not_found"`.

mod common;

use serde_json::Value;
use std::fs;

#[test]
fn fetch_chunk_json_emits_fetch_result_v1() {
    let dir = tempfile::tempdir().unwrap();
    let (cfg, workspace, _data) = common::write_config(dir.path(), 30);
    fs::write(workspace.join("a.md"), "# T\n\napples are red.\n").unwrap();
    common::ingest(&cfg, &workspace);

    // Find chunk_id via search.
    let (search_stdout, _) = common::run_search_with_args(
        &cfg,
        &["--json", "--mode", "lexical", "--k", "1", "apples"],
    );
    let search: Value = serde_json::from_str(search_stdout.trim())
        .unwrap_or_else(|e| panic!("search not JSON: {search_stdout:?}: {e}"));
    let chunk_id = search["hits"][0]["chunk_id"]
        .as_str()
        .expect("chunk_id on first hit")
        .to_string();

    let (stdout, _) = common::run_fetch_with_args(
        &cfg,
        &["--json", "chunk", &chunk_id],
    );
    let v: Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("fetch not JSON: {stdout:?}: {e}"));
    assert_eq!(v["schema_version"], "fetch_result.v1");
    assert_eq!(v["kind"], "chunk");
    assert!(
        v["chunk"].is_object(),
        "target chunk must be populated: {v}"
    );
    assert_eq!(v["truncated"], false);
}

#[test]
fn fetch_doc_json_with_max_tokens_truncates() {
    let dir = tempfile::tempdir().unwrap();
    let (cfg, workspace, _data) = common::write_config(dir.path(), 30);
    let body: String = "Lorem ipsum dolor sit amet. ".repeat(20);
    fs::write(workspace.join("big.md"), format!("# Big\n\n{body}\n")).unwrap();
    common::ingest(&cfg, &workspace);

    // Find doc_id via search.
    let (search_stdout, _) = common::run_search_with_args(
        &cfg,
        &["--json", "--mode", "lexical", "--k", "1", "Lorem"],
    );
    let search: Value = serde_json::from_str(search_stdout.trim())
        .unwrap_or_else(|e| panic!("search not JSON: {search_stdout:?}: {e}"));
    let doc_id = search["hits"][0]["doc_id"]
        .as_str()
        .expect("doc_id on first hit")
        .to_string();

    let (stdout, _) = common::run_fetch_with_args(
        &cfg,
        &["--json", "doc", &doc_id, "--max-tokens", "20"],
    );
    let v: Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("fetch not JSON: {stdout:?}: {e}"));
    assert_eq!(v["kind"], "doc");
    assert_eq!(
        v["truncated"], true,
        "20-token cap must trip truncation: {v}"
    );
}

#[test]
fn fetch_chunk_unknown_id_exits_with_error_v1() {
    let dir = tempfile::tempdir().unwrap();
    let (cfg, _workspace, _data) = common::write_config(dir.path(), 30);

    // Direct invocation (not via the success-asserting helper) so we
    // can read stderr on failure — mirrors the stale_cursor test in
    // `wire_search_response.rs`.
    let exe = env!("CARGO_BIN_EXE_kebab");
    let cfg_str = cfg.to_str().expect("utf8");
    let out = std::process::Command::new(exe)
        .args([
            "--config",
            cfg_str,
            "--json",
            "fetch",
            "chunk",
            "nonexistent",
        ])
        .output()
        .expect("kebab fetch");

    assert_ne!(out.status.code(), Some(0), "must exit non-zero");
    let stderr = String::from_utf8_lossy(&out.stderr);
    let err_line = stderr
        .lines()
        .find(|l| {
            serde_json::from_str::<Value>(l)
                .ok()
                .and_then(|v| {
                    v.get("schema_version")
                        .and_then(|s| s.as_str())
                        .map(String::from)
                })
                .as_deref()
                == Some("error.v1")
        })
        .unwrap_or_else(|| panic!("no error.v1 line on stderr: {stderr:?}"));

    let v: Value = serde_json::from_str(err_line).expect("error.v1 json");
    assert_eq!(
        v["code"], "chunk_not_found",
        "code must be chunk_not_found: {err_line}"
    );
}
