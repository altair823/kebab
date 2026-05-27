//! Integration: spawn kebab and verify --json mode emits error.v1 ndjson
//! on stderr while non-json mode emits the legacy `error:` text prefix.
//!
//! The `config_invalid` code is triggered by supplying an *existing* but
//! malformed TOML file via `--config`. A file that exists but fails TOML
//! parsing is the reliable path to `config_invalid`. Supplying a path that
//! does not exist emits `config_not_found` instead (Bug #10 fix, v0.20.0
//! bugfix3); see `cli_config_not_found.rs` for those tests.

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

#[test]
fn json_mode_emits_error_v1_on_config_invalid() {
    let tmp = tempfile::tempdir().unwrap();
    // Write a file that exists but is not valid TOML.
    let bad_config = tmp.path().join("bad.toml");
    std::fs::write(&bad_config, b"this is not { valid toml !!!").unwrap();

    let mut cmd = Command::new(kebab_bin());
    cmd.args([
        "--json",
        "--config",
        bad_config.to_str().unwrap(),
        "ingest",
    ]);
    for (k, v) in xdg_envs(tmp.path()) {
        cmd.env(k, v);
    }

    let out = cmd.output().unwrap();
    assert!(
        !out.status.success(),
        "expected non-zero exit for config_invalid"
    );
    let exit_code = out.status.code().unwrap_or(-1);
    assert_eq!(exit_code, 2, "expected exit code 2, got {exit_code}");

    let stderr = String::from_utf8(out.stderr).unwrap();
    let first_line = stderr.lines().next().expect("stderr must have at least one line");
    let v: serde_json::Value =
        serde_json::from_str(first_line).expect("stderr first line must be valid JSON");

    assert_eq!(
        v.get("schema_version").and_then(|s| s.as_str()),
        Some("error.v1"),
        "schema_version must be error.v1"
    );
    assert_eq!(
        v.get("code").and_then(|s| s.as_str()),
        Some("config_invalid"),
        "code must be config_invalid"
    );
}

#[test]
fn text_mode_emits_legacy_error_format() {
    let tmp = tempfile::tempdir().unwrap();
    // Same trigger: an existing file with malformed TOML.
    let bad_config = tmp.path().join("bad.toml");
    std::fs::write(&bad_config, b"this is not { valid toml !!!").unwrap();

    let mut cmd = Command::new(kebab_bin());
    cmd.args(["--config", bad_config.to_str().unwrap(), "ingest"]);
    for (k, v) in xdg_envs(tmp.path()) {
        cmd.env(k, v);
    }

    let out = cmd.output().unwrap();
    assert!(
        !out.status.success(),
        "expected non-zero exit for config_invalid"
    );
    let exit_code = out.status.code().unwrap_or(-1);
    assert_eq!(exit_code, 2, "expected exit code 2, got {exit_code}");

    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(
        stderr.starts_with("error:"),
        "text mode stderr must start with 'error:', got: {stderr:?}"
    );
    assert!(
        !stderr.trim_start().starts_with('{'),
        "text mode stderr must NOT be JSON, got: {stderr:?}"
    );
}
