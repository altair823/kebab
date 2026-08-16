// crates/kebab-app/tests/ingest_log_smoke.rs
//
// Integration tests for ingest_log feature (v0.20.x). Spec §5 AC-9 + AC-6.

use std::path::PathBuf;

use kebab_app::{IngestOpts, ingest_with_config};
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
    ingest_with_config(cfg, scope, IngestOpts::default())
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

    let valid_kinds = [
        "ocr",
        "parse_error",
        "skip",
        "error",
        "purge",
        "sweep_summary",
        "summary",
    ];
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

    ingest_with_config(cfg, scope, IngestOpts::default())
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


/// Issue #228: the deleted-file sweep wrote nothing to the ndjson log, so
/// a run whose whole wall-clock went into purging left a zero-byte file
/// and no way to reconstruct afterwards what had been deleted. The log is
/// the only post-hoc record — tracing goes to stderr and is gone.
#[test]
fn ingest_log_records_the_deleted_file_sweep() {
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path().join("kb");
    std::fs::create_dir_all(&workspace).unwrap();
    let log_dir = tmp.path().join("logs");

    let doomed = workspace.join("doomed.md");
    std::fs::write(&doomed, "# doomed\n\nthis file is about to vanish\n").unwrap();
    std::fs::write(workspace.join("kept.md"), "# kept\n\nthis one stays\n").unwrap();

    let scope = SourceScope {
        root: workspace.clone(),
        include: vec!["**/*.md".to_string()],
        exclude: Vec::new(),
    };
    ingest_with_config(
        minimal_config(&workspace, &log_dir),
        scope.clone(),
        IngestOpts::default(),
    )
    .expect("first ingest should succeed");

    std::fs::remove_file(&doomed).unwrap();
    let report = ingest_with_config(
        minimal_config(&workspace, &log_dir),
        scope,
        IngestOpts::default(),
    )
    .expect("second ingest should succeed");
    assert_eq!(report.purged_deleted_files, 1);

    // The second run's log is the later one; both runs write into log_dir.
    let mut logs: Vec<PathBuf> = std::fs::read_dir(&log_dir)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "ndjson"))
        .collect();
    logs.sort();
    let body = std::fs::read_to_string(logs.last().expect("a log per run")).unwrap();

    let events: Vec<Value> = body
        .lines()
        .map(|l| serde_json::from_str(l).expect("each line is JSON"))
        .collect();
    let kind = |v: &Value| v.get("kind").and_then(Value::as_str).unwrap_or("").to_string();

    let purges: Vec<&Value> = events.iter().filter(|v| kind(v) == "purge").collect();
    assert_eq!(purges.len(), 1, "one purge line for the deleted file: {body}");
    assert_eq!(
        purges[0].get("doc_path").and_then(Value::as_str),
        Some("doomed.md"),
        "and it names which document went: {body}"
    );

    let sweep = events
        .iter()
        .find(|v| kind(v) == "sweep_summary")
        .unwrap_or_else(|| panic!("the phase totals must be recorded: {body}"));
    assert_eq!(sweep.get("purged").and_then(Value::as_u64), Some(1));
    assert_eq!(
        sweep.get("checked").and_then(Value::as_u64),
        Some(1),
        "one candidate examined: {body}"
    );
    assert!(
        sweep.get("ms").is_some(),
        "with a duration, which is what tells a user whether the phase was \
         the run's bottleneck: {body}"
    );
}
