//! Integration tests for `--readonly` and `--quiet` global flags (fb-28).

use std::io::Write;
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

fn fixture_workspace() -> (tempfile::TempDir, std::path::PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("workspace");
    std::fs::create_dir_all(&ws).unwrap();
    let mut a = std::fs::File::create(ws.join("a.md")).unwrap();
    writeln!(a, "# Alpha\n\nfirst doc").unwrap();
    (tmp, ws)
}

fn xdg_envs(tmp_path: &std::path::Path) -> [(&'static str, std::path::PathBuf); 4] {
    [
        ("XDG_CONFIG_HOME", tmp_path.join("cfg")),
        ("XDG_DATA_HOME", tmp_path.join("data")),
        ("XDG_CACHE_HOME", tmp_path.join("cache")),
        ("XDG_STATE_HOME", tmp_path.join("state")),
    ]
}

#[test]
fn readonly_flag_blocks_ingest() {
    let (tmp, ws) = fixture_workspace();
    let out = Command::new(kebab_bin())
        .args(["--readonly", "ingest", "--root", ws.to_str().unwrap()])
        .envs(xdg_envs(tmp.path()))
        .output()
        .unwrap();

    assert_eq!(out.status.code(), Some(1), "expected exit 1");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("readonly mode"),
        "expected 'readonly mode' in stderr, got: {stderr}"
    );
}

#[test]
fn readonly_flag_blocks_ingest_file() {
    let (tmp, ws) = fixture_workspace();
    let file = ws.join("a.md");
    let out = Command::new(kebab_bin())
        .args(["--readonly", "ingest-file", file.to_str().unwrap()])
        .envs(xdg_envs(tmp.path()))
        .output()
        .unwrap();

    assert_eq!(out.status.code(), Some(1), "expected exit 1");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("readonly mode"), "stderr: {stderr}");
}

#[test]
fn readonly_flag_blocks_reset() {
    let (tmp, _ws) = fixture_workspace();
    let out = Command::new(kebab_bin())
        .args(["--readonly", "reset", "--data-only", "--yes"])
        .envs(xdg_envs(tmp.path()))
        .output()
        .unwrap();

    assert_eq!(out.status.code(), Some(1), "expected exit 1");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("readonly mode"), "stderr: {stderr}");
}

#[test]
fn kebab_readonly_env_blocks_ingest() {
    let (tmp, ws) = fixture_workspace();
    let out = Command::new(kebab_bin())
        .args(["ingest", "--root", ws.to_str().unwrap()])
        .env("KEBAB_READONLY", "1")
        .envs(xdg_envs(tmp.path()))
        .output()
        .unwrap();

    assert_eq!(out.status.code(), Some(1), "expected exit 1");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("readonly mode"), "stderr: {stderr}");
}

#[test]
fn readonly_json_mode_emits_error_v1() {
    let (tmp, ws) = fixture_workspace();
    let out = Command::new(kebab_bin())
        .args(["--readonly", "--json", "ingest", "--root", ws.to_str().unwrap()])
        .envs(xdg_envs(tmp.path()))
        .output()
        .unwrap();

    assert_eq!(out.status.code(), Some(1), "expected exit 1");
    let stderr = String::from_utf8_lossy(&out.stderr);
    let v: serde_json::Value = serde_json::from_str(stderr.trim())
        .unwrap_or_else(|e| panic!("expected error.v1 JSON on stderr, got {stderr:?}: {e}"));
    assert_eq!(
        v.get("schema_version").and_then(|s| s.as_str()),
        Some("error.v1"),
        "expected schema_version=error.v1"
    );
    assert_eq!(
        v.get("code").and_then(|s| s.as_str()),
        Some("readonly_mode"),
        "expected code=readonly_mode"
    );
}

#[test]
fn quiet_flag_suppresses_progress_stderr() {
    let (tmp, ws) = fixture_workspace();
    let out = Command::new(kebab_bin())
        .args(["--quiet", "ingest", "--root", ws.to_str().unwrap()])
        .envs(xdg_envs(tmp.path()))
        .output()
        .unwrap();

    assert!(
        out.status.success(),
        "exit: {:?}, stderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.is_empty(),
        "expected empty stderr with --quiet, got: {stderr}"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("scanned"),
        "expected report summary on stdout, got: {stdout}"
    );
}

#[test]
fn quiet_with_json_stdout_has_report_stderr_is_empty() {
    let (tmp, ws) = fixture_workspace();
    let out = Command::new(kebab_bin())
        .args(["--quiet", "--json", "ingest", "--root", ws.to_str().unwrap()])
        .envs(xdg_envs(tmp.path()))
        .output()
        .unwrap();

    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.is_empty(), "expected empty stderr, got: {stderr}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let last_line = stdout.lines().last().unwrap_or("");
    let v: serde_json::Value = serde_json::from_str(last_line)
        .unwrap_or_else(|e| panic!("expected JSON on stdout last line, got {last_line:?}: {e}"));
    assert_eq!(
        v.get("schema_version").and_then(|s| s.as_str()),
        Some("ingest_report.v1")
    );
}
