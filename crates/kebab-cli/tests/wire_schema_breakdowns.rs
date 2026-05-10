//! p9-fb-37: integration tests for `kebab schema --json` extended stats.

mod common;

use serde_json::Value;
use std::fs;
use std::process::Command;

fn run_schema(cfg: &std::path::Path) -> Value {
    let bin = env!("CARGO_BIN_EXE_kebab");
    let out = Command::new(bin)
        .args(["--config", cfg.to_str().unwrap(), "schema", "--json"])
        .output()
        .expect("run kebab schema");
    assert!(
        out.status.success(),
        "schema failed: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("valid JSON")
}

#[test]
fn schema_stats_includes_breakdowns_on_fresh_corpus() {
    let dir = tempfile::tempdir().unwrap();
    let (cfg, workspace, _data) = common::write_config(dir.path(), 0);
    // Run a no-op ingest to bring up migrations + create the SQLite file.
    fs::write(workspace.join("placeholder.md"), "# placeholder\n").unwrap();
    common::ingest(&cfg, &workspace);

    let v = run_schema(&cfg);
    let stats = &v["stats"];
    let m = stats["media_breakdown"].as_object().unwrap();
    assert_eq!(m.len(), 5, "5 media keys padded");
    for k in &["markdown", "pdf", "image", "audio", "other"] {
        assert!(m[*k].is_number(), "media[{k}] is integer");
    }
    assert!(stats["lang_breakdown"].is_object());
    assert!(stats["index_bytes"]["sqlite"].is_number());
    assert!(stats["index_bytes"]["lancedb"].is_number());
    assert!(stats["stale_doc_count"].is_number());
}

#[test]
fn schema_stats_breakdowns_after_ingest() {
    let dir = tempfile::tempdir().unwrap();
    let (cfg, workspace, _data) = common::write_config(dir.path(), 0);
    fs::write(workspace.join("a.md"), "---\nlang: en\n---\nhello\n").unwrap();
    fs::write(workspace.join("b.md"), "---\nlang: ko\n---\n안녕\n").unwrap();
    common::ingest(&cfg, &workspace);

    let v = run_schema(&cfg);
    let stats = &v["stats"];
    assert_eq!(stats["media_breakdown"]["markdown"], 2);
    assert!(stats["lang_breakdown"].is_object());
    assert!(stats["index_bytes"]["sqlite"].as_u64().unwrap() > 0);
}
