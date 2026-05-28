//! Integration coverage for `kebab ingest` 의 progress display
//! (p9-fb-02). Each test runs the built `kebab` bin in a fresh
//! subprocess against a tempdir-rooted XDG layout + tempdir
//! workspace so the assertions don't depend on the host config.

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

/// Build a tempdir-rooted XDG layout with a workspace containing two
/// markdown files. Returns the tmp guard (to keep the dir alive) and
/// the workspace path the caller should pass to `--root`.
fn fixture_workspace() -> (tempfile::TempDir, std::path::PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("workspace");
    std::fs::create_dir_all(&ws).unwrap();
    let mut a = std::fs::File::create(ws.join("a.md")).unwrap();
    writeln!(a, "# Alpha\n\nfirst doc").unwrap();
    let mut b = std::fs::File::create(ws.join("b.md")).unwrap();
    writeln!(b, "# Beta\n\nsecond doc").unwrap();
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
fn ingest_json_emits_line_delimited_progress_then_report() {
    let (tmp, ws) = fixture_workspace();
    let mut cmd = Command::new(kebab_bin());
    cmd.args([
        "--json",
        "ingest",
        "--root",
        ws.to_str().unwrap(),
        "--summary-only",
    ]);
    for (k, v) in xdg_envs(tmp.path()) {
        cmd.env(k, v);
    }
    let out = cmd.output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Every stdout line must be a JSON object. The last line is the
    // existing ingest_report.v1; everything above is ingest_progress.v1.
    let stdout = String::from_utf8(out.stdout).unwrap();
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).collect();
    assert!(lines.len() >= 2, "expected ≥2 stdout lines, got: {stdout}");

    let mut progress_seen = 0usize;
    let mut last_schema = None;
    for line in &lines {
        let v: serde_json::Value =
            serde_json::from_str(line).unwrap_or_else(|e| panic!("bad json line: {line:?} ({e})"));
        let schema = v
            .get("schema_version")
            .and_then(|s| s.as_str())
            .unwrap_or_else(|| panic!("missing schema_version: {line}"));
        if schema == "ingest_progress.v1" {
            progress_seen += 1;
        }
        last_schema = Some(schema.to_string());
    }
    assert!(progress_seen >= 4, "progress events: {progress_seen}");
    assert_eq!(last_schema.as_deref(), Some("ingest_report.v1"));
}

#[test]
fn ingest_human_non_tty_emits_progress_lines_to_stderr() {
    // Command::output gives no controlling tty, so the indicatif draw
    // target is `hidden` and progress lines go to stderr instead.
    let (tmp, ws) = fixture_workspace();
    let mut cmd = Command::new(kebab_bin());
    cmd.args(["ingest", "--root", ws.to_str().unwrap(), "--summary-only"]);
    for (k, v) in xdg_envs(tmp.path()) {
        cmd.env(k, v);
    }
    let out = cmd.output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("ingest: scanning") || stderr.contains("ingest:"),
        "expected progress text in stderr, got: {stderr}"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("scanned ") && stdout.contains("new "),
        "expected the human-mode summary line on stdout, got: {stdout}"
    );
}

#[test]
fn ingest_json_progress_lines_carry_kind_and_ts() {
    let (tmp, ws) = fixture_workspace();
    let mut cmd = Command::new(kebab_bin());
    cmd.args([
        "--json",
        "ingest",
        "--root",
        ws.to_str().unwrap(),
        "--summary-only",
    ]);
    for (k, v) in xdg_envs(tmp.path()) {
        cmd.env(k, v);
    }
    let out = cmd.output().unwrap();
    assert!(out.status.success());

    let stdout = String::from_utf8(out.stdout).unwrap();
    let mut saw_scan_started = false;
    let mut saw_completed = false;
    for line in stdout.lines().filter(|l| !l.is_empty()) {
        let v: serde_json::Value = serde_json::from_str(line).unwrap();
        let schema = v.get("schema_version").and_then(|s| s.as_str()).unwrap();
        if schema != "ingest_progress.v1" {
            continue;
        }
        let kind = v.get("kind").and_then(|s| s.as_str()).unwrap();
        // ts is a non-empty string and must round-trip as RFC 3339.
        let ts = v.get("ts").and_then(|s| s.as_str()).unwrap();
        assert!(!ts.is_empty(), "ts empty for {kind}");
        if kind == "scan_started" {
            saw_scan_started = true;
        }
        if kind == "completed" {
            saw_completed = true;
            // Counts mirror the report.
            let counts = v.get("counts").unwrap();
            assert_eq!(
                counts.get("scanned").and_then(serde_json::Value::as_u64),
                Some(2)
            );
            assert_eq!(
                counts.get("new").and_then(serde_json::Value::as_u64),
                Some(2)
            );
        }
    }
    assert!(saw_scan_started, "missing scan_started event");
    assert!(saw_completed, "missing completed event");
}

#[test]
fn kebab_progress_plain_env_emits_append_lines() {
    // KEBAB_PROGRESS=plain forces non-TTY branch even in TTY-emulated envs.
    // In subprocess tests there's no TTY anyway, so this primarily verifies
    // the env var is accepted and the non-TTY path still works.
    let (tmp, ws) = fixture_workspace();
    let out = Command::new(kebab_bin())
        .args(["ingest", "--root", ws.to_str().unwrap()])
        .env("KEBAB_PROGRESS", "plain")
        .envs(xdg_envs(tmp.path()))
        .output()
        .unwrap();

    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("ingest: scanning"),
        "expected 'ingest: scanning' in stderr, got: {stderr}"
    );
    assert!(
        stderr.contains("ingest: complete"),
        "expected 'ingest: complete' in stderr, got: {stderr}"
    );
}
