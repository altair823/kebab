//! p9-fb-38: integration tests for `search_hit.v1.score_kind`.

mod common;

use serde_json::Value;
use std::fs;

fn doc_with_term(workspace: &std::path::Path) {
    fs::write(workspace.join("doc1.md"), "# Title\n\nrust async hello\n").unwrap();
}

#[test]
fn lexical_mode_hits_carry_bm25_score_kind() {
    let dir = tempfile::tempdir().unwrap();
    let (cfg, workspace, _data) = common::write_config(dir.path(), 0);
    doc_with_term(&workspace);
    common::ingest(&cfg, &workspace);

    let (stdout, _stderr) = common::run_search_with_args(
        &cfg,
        &["--mode", "lexical", "--json", "rust"],
    );
    let v: Value = serde_json::from_str(stdout.trim()).expect("valid JSON");
    let hits = v["hits"].as_array().expect("hits array");
    assert!(!hits.is_empty(), "expected at least 1 hit");
    for h in hits {
        assert_eq!(h["score_kind"], "bm25");
    }
}

#[test]
fn old_wire_reader_compat_score_kind_optional_field() {
    // The wire schema marks `score_kind` as additive (not required).
    // We can't easily simulate an old reader from inside Rust, but we
    // can confirm the JSON includes the field — old readers that
    // ignore unknown fields are unaffected. This test just ensures
    // the field is always present in fb-38+ output.
    let dir = tempfile::tempdir().unwrap();
    let (cfg, workspace, _data) = common::write_config(dir.path(), 0);
    doc_with_term(&workspace);
    common::ingest(&cfg, &workspace);

    let (stdout, _stderr) = common::run_search_with_args(
        &cfg,
        &["--mode", "lexical", "--json", "rust"],
    );
    let v: Value = serde_json::from_str(stdout.trim()).unwrap();
    let hit = &v["hits"][0];
    assert!(hit.get("score_kind").is_some(), "score_kind always emitted");
}
