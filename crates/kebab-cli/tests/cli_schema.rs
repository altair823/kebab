//! Integration: spawn the kebab binary and parse `kebab schema [--json]`.
//!
//! Each test builds an isolated TempDir-rooted XDG layout, runs
//! `kebab ingest` over an empty workspace (which creates and migrates
//! kebab.sqlite), then exercises `kebab schema` in JSON and text modes.
//! Using an empty workspace avoids the embedding model dependency while
//! still seeding the DB so `open_existing` inside schema_with_config
//! succeeds (a NotIndexed error fires when the DB file is absent).

use std::process::Command;

fn kebab_bin() -> std::path::PathBuf {
    let manifest = env!("CARGO_MANIFEST_DIR");
    std::path::PathBuf::from(manifest)
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("target/debug/kebab")
}

fn xdg_envs(tmp: &std::path::Path) -> [(&'static str, std::path::PathBuf); 4] {
    [
        ("XDG_CONFIG_HOME", tmp.join("cfg")),
        ("XDG_DATA_HOME", tmp.join("data")),
        ("XDG_CACHE_HOME", tmp.join("cache")),
        ("XDG_STATE_HOME", tmp.join("state")),
    ]
}

/// Seed kebab.sqlite by running `kebab ingest` over an empty workspace dir.
/// This is the minimum required for `kebab schema` to succeed: the store
/// uses `open_existing`, which errors when the DB file is absent.
fn seed_db(tmp: &tempfile::TempDir) {
    let ws = tmp.path().join("ws");
    std::fs::create_dir_all(&ws).unwrap();

    let mut cmd = Command::new(kebab_bin());
    cmd.args(["ingest", "--root", ws.to_str().unwrap(), "--summary-only"]);
    for (k, v) in xdg_envs(tmp.path()) {
        cmd.env(k, v);
    }
    let out = cmd.output().unwrap();
    assert!(
        out.status.success(),
        "seed ingest failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn cli_schema_json_emits_schema_v1() {
    let tmp = tempfile::tempdir().unwrap();
    seed_db(&tmp);

    let mut cmd = Command::new(kebab_bin());
    cmd.args(["--json", "schema"]);
    for (k, v) in xdg_envs(tmp.path()) {
        cmd.env(k, v);
    }
    let out = cmd.output().unwrap();
    assert!(
        out.status.success(),
        "kebab --json schema failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8(out.stdout).unwrap();
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("stdout must be valid JSON");

    assert_eq!(
        v.get("schema_version").and_then(|s| s.as_str()),
        Some("schema.v1"),
        "schema_version must be schema.v1"
    );
    assert!(
        v.get("kebab_version")
            .and_then(|s| s.as_str())
            .map(|s| !s.is_empty())
            .unwrap_or(false),
        "kebab_version must be a non-empty string"
    );

    let caps = v
        .get("capabilities")
        .and_then(|c| c.as_object())
        .expect("capabilities must be a JSON object");
    assert_eq!(
        caps.get("json_mode").and_then(|b| b.as_bool()),
        Some(true),
        "capabilities.json_mode must be true"
    );
    assert_eq!(
        caps.get("mcp_server").and_then(|b| b.as_bool()),
        Some(false),
        "capabilities.mcp_server must be false (not yet shipped)"
    );
}

#[test]
fn cli_schema_text_mode_runs() {
    let tmp = tempfile::tempdir().unwrap();
    seed_db(&tmp);

    let mut cmd = Command::new(kebab_bin());
    cmd.args(["schema"]);
    for (k, v) in xdg_envs(tmp.path()) {
        cmd.env(k, v);
    }
    let out = cmd.output().unwrap();
    assert!(
        out.status.success(),
        "kebab schema (text) failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        stdout.contains("kebab v"),
        "text output must contain 'kebab v', got: {stdout}"
    );
    assert!(
        stdout.contains("capabilities"),
        "text output must contain 'capabilities', got: {stdout}"
    );
    assert!(
        stdout.contains("models"),
        "text output must contain 'models', got: {stdout}"
    );
    assert!(
        stdout.contains("stats"),
        "text output must contain 'stats', got: {stdout}"
    );
}
