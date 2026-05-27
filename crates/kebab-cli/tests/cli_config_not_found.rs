//! Integration tests for Bug #10: explicit --config <path> that does not exist
//! must fail with exit≠0 and error.v1 code=config_not_found (not silently fall
//! back to XDG defaults).

use std::process::Command;
use serde_json::Value;

fn kebab_bin() -> String {
    env!("CARGO_BIN_EXE_kebab").to_string()
}

fn parse_error_v1(stderr: &str) -> Value {
    let last = stderr.lines().last().expect("expected error.v1 ndjson on stderr");
    serde_json::from_str(last)
        .unwrap_or_else(|e| panic!("expected ndjson on stderr: {e}\nstderr={stderr}"))
}

#[test]
fn invalid_config_path_emits_error_v1_with_nonzero_exit() {
    let absent = "/tmp/__kebab_bugfix3_absolute_nonexistent.toml";
    assert!(!std::path::Path::new(absent).exists());

    let out = Command::new(kebab_bin())
        .args(["search", "rust", "--config", absent, "--json"])
        .output()
        .expect("spawn kebab");

    assert_ne!(out.status.code(), Some(0), "exit must be nonzero on missing --config");
    let stderr = String::from_utf8_lossy(&out.stderr);
    let v = parse_error_v1(&stderr);
    assert_eq!(v["schema_version"], "error.v1");
    assert_eq!(v["code"], "config_not_found");
    assert!(v["hint"].is_string(), "hint must be present");
}

#[test]
fn invalid_relative_config_path_emits_config_not_found() {
    // Bug #10 spec §6 R-1: relative path も cwd-relative で cover.
    let tmp = tempfile::tempdir().unwrap();
    let out = Command::new(kebab_bin())
        .args(["search", "rust", "--config", "nonexistent-rel.toml", "--json"])
        .current_dir(tmp.path())
        .output()
        .expect("spawn kebab");

    assert_ne!(out.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&out.stderr);
    let v = parse_error_v1(&stderr);
    assert_eq!(v["schema_version"], "error.v1");
    assert_eq!(v["code"], "config_not_found");
}
