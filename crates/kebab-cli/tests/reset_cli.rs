//! Integration coverage for `kebab reset` — exercises the binary
//! end-to-end against a tempdir-rooted XDG layout. Each test runs the
//! built `kebab` bin in a fresh subprocess so the per-process XDG env
//! overrides don't bleed into sibling tests.

use std::process::Command;

fn kebab_bin() -> std::path::PathBuf {
    // The compiled bin is at `target/debug/kebab` relative to the
    // workspace root. CARGO_MANIFEST_DIR points at the kebab-cli crate
    // dir; the workspace root is two levels above (../../).
    let manifest = env!("CARGO_MANIFEST_DIR");
    std::path::PathBuf::from(manifest)
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("target/debug/kebab")
}

#[test]
fn reset_data_only_yes_removes_data_dir_and_keeps_config() {
    let tmp = tempfile::tempdir().unwrap();
    let xdg_cfg = tmp.path().join("cfg");
    let xdg_data = tmp.path().join("data");
    let xdg_cache = tmp.path().join("cache");
    let xdg_state = tmp.path().join("state");
    std::fs::create_dir_all(xdg_cfg.join("kebab")).unwrap();
    std::fs::create_dir_all(xdg_data.join("kebab")).unwrap();
    std::fs::create_dir_all(xdg_cache.join("kebab")).unwrap();
    std::fs::create_dir_all(xdg_state.join("kebab")).unwrap();
    // No `config.toml` written — Config::load(None) falls back to
    // defaults when the file is absent (see kebab-config). The marker
    // file under cfg/kebab/ is what we assert survives.
    std::fs::write(xdg_cfg.join("kebab/marker"), b"cfg").unwrap();
    std::fs::write(xdg_data.join("kebab/marker"), b"data").unwrap();

    let out = Command::new(kebab_bin())
        .args(["reset", "--data-only", "--yes"])
        .env("XDG_CONFIG_HOME", &xdg_cfg)
        .env("XDG_DATA_HOME", &xdg_data)
        .env("XDG_CACHE_HOME", &xdg_cache)
        .env("XDG_STATE_HOME", &xdg_state)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    assert!(!xdg_data.join("kebab").exists(), "data dir should be gone");
    assert!(!xdg_cache.join("kebab").exists(), "cache dir should be gone");
    assert!(!xdg_state.join("kebab").exists(), "state dir should be gone");
    assert!(xdg_cfg.join("kebab/marker").exists(), "config dir preserved");
}

#[test]
fn reset_no_yes_in_non_tty_aborts_with_exit_2() {
    let tmp = tempfile::tempdir().unwrap();
    let xdg_data = tmp.path().join("data");
    std::fs::create_dir_all(xdg_data.join("kebab")).unwrap();
    std::fs::write(xdg_data.join("kebab/marker"), b"d").unwrap();

    let out = Command::new(kebab_bin())
        .args(["reset", "--data-only"])
        .env("XDG_CONFIG_HOME", tmp.path().join("cfg"))
        .env("XDG_DATA_HOME", &xdg_data)
        .env("XDG_CACHE_HOME", tmp.path().join("cache"))
        .env("XDG_STATE_HOME", tmp.path().join("state"))
        .output()
        .unwrap();

    // Non-TTY (Command::output gives no tty) without --yes must abort.
    assert!(!out.status.success(), "expected abort, got success");
    let code = out.status.code().unwrap_or(-1);
    assert_eq!(code, 2, "expected exit 2 (generic error), got {code}");
    assert!(
        xdg_data.join("kebab").exists(),
        "data dir must survive an aborted reset"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("non-interactive") || stderr.contains("--yes"),
        "expected refusal hint in stderr, got: {stderr}"
    );
}

#[test]
fn reset_data_only_yes_json_emits_reset_report_v1() {
    let tmp = tempfile::tempdir().unwrap();
    let xdg_data = tmp.path().join("data");
    std::fs::create_dir_all(xdg_data.join("kebab")).unwrap();
    std::fs::write(xdg_data.join("kebab/marker"), b"d").unwrap();

    let out = Command::new(kebab_bin())
        .args(["--json", "reset", "--data-only", "--yes"])
        .env("XDG_CONFIG_HOME", tmp.path().join("cfg"))
        .env("XDG_DATA_HOME", &xdg_data)
        .env("XDG_CACHE_HOME", tmp.path().join("cache"))
        .env("XDG_STATE_HOME", tmp.path().join("state"))
        .output()
        .unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));

    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        v.get("schema_version").and_then(|s| s.as_str()),
        Some("reset_report.v1")
    );
    assert_eq!(v.get("scope").and_then(|s| s.as_str()), Some("data_only"));
    assert!(v.get("removed_paths").and_then(|a| a.as_array()).is_some());
}

#[test]
fn reset_mutually_exclusive_scope_flags_rejected() {
    // clap's `group = "reset_scope"` should reject --all and
    // --data-only together. The bin must exit nonzero with a clap
    // usage error before touching any path.
    let tmp = tempfile::tempdir().unwrap();
    let out = Command::new(kebab_bin())
        .args(["reset", "--all", "--data-only", "--yes"])
        .env("XDG_CONFIG_HOME", tmp.path().join("cfg"))
        .env("XDG_DATA_HOME", tmp.path().join("data"))
        .env("XDG_CACHE_HOME", tmp.path().join("cache"))
        .env("XDG_STATE_HOME", tmp.path().join("state"))
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("cannot be used") || stderr.contains("conflicts"),
        "expected clap conflict error, got: {stderr}"
    );
}
