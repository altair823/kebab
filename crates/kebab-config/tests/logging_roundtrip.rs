// crates/kebab-config/tests/logging_roundtrip.rs
//
// Integration tests for [logging] config section (v0.20.x ingest log feature).

use kebab_config::{Config, LoggingCfg};
use std::path::PathBuf;

#[derive(serde::Deserialize)]
struct LoggingWrapper {
    logging: LoggingCfg,
}

// Test 1: default LoggingCfg roundtrip — enabled=true, dir="{state_dir}/logs".
#[test]
fn logging_defaults_are_enabled_with_state_dir_placeholder() {
    let cfg = Config::defaults();
    assert!(cfg.logging.ingest_log_enabled);
    assert_eq!(
        cfg.logging.ingest_log_dir,
        PathBuf::from("{state_dir}/logs")
    );
}

// Test 2: [logging] override — enabled=false, custom dir.
#[test]
fn logging_toml_override() {
    let toml = r#"
[logging]
ingest_log_enabled = false
ingest_log_dir = "/tmp/custom-logs"
"#;
    let w: LoggingWrapper = toml::from_str(toml).expect("parse toml");
    assert!(!w.logging.ingest_log_enabled);
    assert_eq!(w.logging.ingest_log_dir, PathBuf::from("/tmp/custom-logs"));
}

// Test 3: pre-v0.20 config (no [logging] section) → LoggingCfg::default() (AC-10).
#[test]
fn pre_v020_config_without_logging_section_gets_defaults() {
    let toml = "[logging]\n";
    let w: LoggingWrapper = toml::from_str(toml).expect("parse toml with empty logging section");
    assert!(w.logging.ingest_log_enabled);
    assert_eq!(w.logging.ingest_log_dir, PathBuf::from("{state_dir}/logs"));
}

// Test 4 (AC-9 v0.20.x r2): old config with only ingest_log_enabled + ingest_log_dir
// parses without error and produces correct defaults for keep_recent_runs + retention_days.
#[test]
fn old_logging_config_parses_with_defaults() {
    let toml = r#"
[logging]
ingest_log_enabled = true
ingest_log_dir = "{state_dir}/logs"
"#;
    let w: LoggingWrapper = toml::from_str(toml).expect("old logging config must parse");
    assert!(w.logging.ingest_log_enabled);
    assert_eq!(w.logging.ingest_log_dir, PathBuf::from("{state_dir}/logs"));
    assert_eq!(
        w.logging.keep_recent_runs, 100,
        "keep_recent_runs must default to 100"
    );
    assert_eq!(
        w.logging.retention_days, 30,
        "retention_days must default to 30"
    );
}
