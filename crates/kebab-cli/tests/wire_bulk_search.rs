//! p9-fb-42: integration tests for `kebab search --bulk`.
//!
//! Lexical-only — no fastembed / no Ollama. Each test builds its own
//! TempDir KB via `common::write_config` + `common::ingest` and drives
//! `kebab search --bulk` through stdin. Verifies:
//!
//! - Two queries over stdin emit per-query ndjson `bulk_search_item.v1` lines.
//! - Empty stdin returns empty results with zero summary.
//! - Malformed ndjson exits with code 2 (config_invalid).
//! - Input over the 100-item cap fails with "max 100" error message.
//! - Invalid item field (e.g. bad `mode`) emits per-item error and continues.

mod common;

use serde_json::Value;
use std::fs;
use std::io::Write;
use std::process::{Command, Stdio};

fn cargo_bin() -> &'static str {
    env!("CARGO_BIN_EXE_kebab")
}

fn run_bulk_with_stdin(cfg: &std::path::Path, stdin_body: &str, json: bool) -> std::process::Output {
    let mut cmd = Command::new(cargo_bin());
    cmd.arg("--config").arg(cfg).arg("search").arg("--bulk");
    if json {
        cmd.arg("--json");
    }
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn kebab");
    {
        let mut sin = child.stdin.take().expect("stdin");
        sin.write_all(stdin_body.as_bytes()).expect("write stdin");
    }
    child.wait_with_output().expect("wait")
}

fn seed_workspace(workspace: &std::path::Path) {
    fs::write(workspace.join("a.md"), "# Alpha\n\nrust async hello").unwrap();
    fs::write(workspace.join("b.md"), "# Bravo\n\nbread and kebab").unwrap();
}

// ---------------------------------------------------------------------------
// Test 1: Two queries over stdin emit per-query ndjson
// ---------------------------------------------------------------------------

#[test]
fn two_query_bulk_emits_per_query_ndjson() {
    let dir = tempfile::tempdir().unwrap();
    let (cfg, workspace, _data) = common::write_config(dir.path(), 0);
    seed_workspace(&workspace);
    common::ingest(&cfg, &workspace);

    let out = run_bulk_with_stdin(
        &cfg,
        "{\"query\":\"rust\",\"mode\":\"lexical\"}\n{\"query\":\"kebab\",\"mode\":\"lexical\"}\n",
        true,
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(lines.len(), 2, "expected 2 ndjson lines, got {lines:?}");
    for line in &lines {
        let v: Value = serde_json::from_str(line).expect("valid JSON line");
        assert_eq!(v["schema_version"], "bulk_search_item.v1");
        assert!(v["response"].is_object());
        assert!(v["error"].is_null());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("bulk_summary: total=2 succeeded=2 failed=0"),
        "stderr summary missing: {stderr}"
    );
}

// ---------------------------------------------------------------------------
// Test 2: Empty stdin returns empty results with zero summary
// ---------------------------------------------------------------------------

#[test]
fn empty_stdin_returns_empty_results_with_zero_summary() {
    let dir = tempfile::tempdir().unwrap();
    let (cfg, workspace, _data) = common::write_config(dir.path(), 0);
    seed_workspace(&workspace);
    common::ingest(&cfg, &workspace);

    let out = run_bulk_with_stdin(&cfg, "", true);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.trim().is_empty(), "expected empty stdout, got: {stdout}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("bulk_summary: total=0 succeeded=0 failed=0"));
}

// ---------------------------------------------------------------------------
// Test 3: Malformed ndjson line emits config_invalid exit 2
// ---------------------------------------------------------------------------

#[test]
fn malformed_ndjson_line_emits_config_invalid_exit_2() {
    let dir = tempfile::tempdir().unwrap();
    let (cfg, workspace, _data) = common::write_config(dir.path(), 0);
    seed_workspace(&workspace);
    common::ingest(&cfg, &workspace);

    let out = run_bulk_with_stdin(&cfg, "not json\n", true);
    assert_eq!(out.status.code(), Some(2), "expected exit 2");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("config_invalid") || stderr.contains("parse error"),
        "expected config_invalid or parse error in stderr: {stderr}"
    );
}

// ---------------------------------------------------------------------------
// Test 4: Over cap input (>100) emits error
// ---------------------------------------------------------------------------

#[test]
fn over_cap_input_emits_error() {
    let dir = tempfile::tempdir().unwrap();
    let (cfg, workspace, _data) = common::write_config(dir.path(), 0);
    seed_workspace(&workspace);
    common::ingest(&cfg, &workspace);

    let body: String = (0..101)
        .map(|_| "{\"query\":\"x\",\"mode\":\"lexical\"}\n")
        .collect();
    let out = run_bulk_with_stdin(&cfg, &body, true);
    // bulk_search_with_config returns Err — surfaces as exit 1 (anyhow chain)
    // or 2 if classified by error_wire. Accept either, but message must mention `max 100`.
    assert!(out.status.code().is_some());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("max 100"),
        "expected 'max 100' in stderr: {stderr}"
    );
}

// ---------------------------------------------------------------------------
// Test 5: Invalid item field (bad mode) emits per-item error and continues
// ---------------------------------------------------------------------------

#[test]
fn invalid_item_field_emits_per_item_error_continues() {
    let dir = tempfile::tempdir().unwrap();
    let (cfg, workspace, _data) = common::write_config(dir.path(), 0);
    seed_workspace(&workspace);
    common::ingest(&cfg, &workspace);

    let out = run_bulk_with_stdin(
        &cfg,
        "{\"query\":\"rust\",\"mode\":\"lexical\"}\n{\"query\":\"x\",\"mode\":\"bogus\"}\n",
        true,
    );
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(lines.len(), 2);
    let v0: Value = serde_json::from_str(lines[0]).unwrap();
    let v1: Value = serde_json::from_str(lines[1]).unwrap();
    assert!(v0["error"].is_null());
    assert!(v1["error"].is_object());
    assert_eq!(v1["error"]["code"], "invalid_input");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("succeeded=1 failed=1"));
}
