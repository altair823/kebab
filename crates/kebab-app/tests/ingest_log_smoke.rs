// crates/kebab-app/tests/ingest_log_smoke.rs
//
// Integration tests for ingest_log feature (v0.20.x). Spec §5 AC-9 + AC-6.

use std::path::PathBuf;

use kebab_app::{IngestOpts, ingest_with_config_opts};
use kebab_config::{Config, LoggingCfg};
use kebab_core::SourceScope;
use serde_json::Value;
use tempfile::TempDir;

fn minimal_config(workspace: &std::path::Path, log_dir: &std::path::Path) -> Config {
    let data_dir = workspace.parent().unwrap().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let model_dir = workspace.parent().unwrap().join("models");
    std::fs::create_dir_all(&model_dir).unwrap();

    let mut cfg = Config::defaults();
    cfg.workspace.root = Some(workspace.to_string_lossy().into_owned());
    cfg.workspace.exclude.clear();
    cfg.storage.data_dir = data_dir.to_string_lossy().into_owned();
    cfg.storage.model_dir = model_dir.to_string_lossy().into_owned();
    cfg.models.embedding.provider = "none".to_string();
    cfg.models.embedding.dimensions = 0;
    cfg.ingest.chunking.target_tokens = 80;
    cfg.ingest.chunking.overlap_tokens = 20;
    cfg.logging = LoggingCfg {
        ingest_log_enabled: true,
        ingest_log_dir: log_dir.to_path_buf(),
        ..Default::default()
    };
    cfg
}

/// AC-9: ingest → log file exists + each line valid JSON + last line kind=summary + scanned>0.
#[test]
fn ingest_log_smoke() {
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path().join("kb");
    std::fs::create_dir_all(&workspace).unwrap();
    let log_dir = tmp.path().join("logs");

    // 1. Minimal corpus: 1 markdown + 1 scanned PDF (OCR disabled — no Ollama needed).
    std::fs::write(
        workspace.join("hello.md"),
        "# Hello\n\nThis is a smoke test.\n",
    )
    .unwrap();
    let pdf_src = PathBuf::from("../kebab-parse-pdf/tests/fixtures/scanned_page1.pdf");
    if pdf_src.exists() {
        std::fs::copy(&pdf_src, workspace.join("scanned.pdf")).unwrap();
    }

    // 2. Config with logging enabled.
    let cfg = minimal_config(&workspace, &log_dir);
    let scope = SourceScope {
        root: workspace.clone(),
        exclude: vec![],
        ..Default::default()
    };

    // 3. Run ingest.
    ingest_with_config_opts(cfg, scope, false, IngestOpts::default())
        .expect("ingest should succeed");

    // 4. Assert log file exists in log_dir.
    let log_files: Vec<_> = std::fs::read_dir(&log_dir)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| {
            e.file_name().to_string_lossy().starts_with("ingest-")
                && e.file_name().to_string_lossy().ends_with(".ndjson")
        })
        .collect();
    assert_eq!(
        log_files.len(),
        1,
        "expected exactly 1 ingest-*.ndjson file, found: {log_files:?}"
    );

    // 5. Parse each line as JSON — assert kind field present and valid.
    let body = std::fs::read_to_string(log_files[0].path()).unwrap();
    let lines: Vec<&str> = body.lines().collect();
    assert!(!lines.is_empty(), "log file should not be empty");

    let valid_kinds = ["ocr", "parse_error", "skip", "error", "summary"];
    for line in &lines {
        let v: Value = serde_json::from_str(line)
            .unwrap_or_else(|e| panic!("line is not valid JSON: {e}\nline: {line}"));
        let kind = v
            .get("kind")
            .and_then(|k| k.as_str())
            .unwrap_or_else(|| panic!("line missing 'kind' field: {line}"));
        assert!(
            valid_kinds.contains(&kind),
            "unexpected kind '{kind}' in line: {line}"
        );
    }

    // 6. Last line must be kind=summary with scanned > 0.
    let last = lines.last().unwrap();
    let last_v: Value = serde_json::from_str(last).unwrap();
    assert_eq!(
        last_v.get("kind").and_then(|k| k.as_str()),
        Some("summary"),
        "last line must be kind=summary, got: {last}"
    );
    let scanned = last_v.get("scanned").and_then(Value::as_u64).unwrap_or(0);
    assert!(scanned > 0, "summary.scanned should be > 0, got: {last}");
}

/// AC-6: ingest_log_enabled=false → no log file created.
#[test]
fn ingest_log_disabled_emits_no_file() {
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path().join("kb");
    std::fs::create_dir_all(&workspace).unwrap();
    let log_dir = tmp.path().join("logs");

    std::fs::write(
        workspace.join("hello.md"),
        "# Hello\n\nDisabled log test.\n",
    )
    .unwrap();

    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let model_dir = tmp.path().join("models");
    std::fs::create_dir_all(&model_dir).unwrap();

    let mut cfg = Config::defaults();
    cfg.workspace.root = Some(workspace.to_string_lossy().into_owned());
    cfg.workspace.exclude.clear();
    cfg.storage.data_dir = data_dir.to_string_lossy().into_owned();
    cfg.storage.model_dir = model_dir.to_string_lossy().into_owned();
    cfg.models.embedding.provider = "none".to_string();
    cfg.models.embedding.dimensions = 0;
    cfg.logging = LoggingCfg {
        ingest_log_enabled: false,
        ingest_log_dir: log_dir.clone(),
        ..Default::default()
    };

    let scope = SourceScope {
        root: workspace.clone(),
        exclude: vec![],
        ..Default::default()
    };

    ingest_with_config_opts(cfg, scope, false, IngestOpts::default())
        .expect("ingest should succeed");

    // log_dir should either not exist or contain 0 ingest-*.ndjson files.
    let log_file_count = if log_dir.exists() {
        std::fs::read_dir(&log_dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| {
                e.file_name().to_string_lossy().starts_with("ingest-")
                    && e.file_name().to_string_lossy().ends_with(".ndjson")
            })
            .count()
    } else {
        0
    };
    assert_eq!(
        log_file_count, 0,
        "no ingest-*.ndjson file should be created when disabled"
    );
}
