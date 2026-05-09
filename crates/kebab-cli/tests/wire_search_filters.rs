//! p9-fb-36: CLI integration tests for search filter flags.
//!
//! Lexical-only — no fastembed / no Ollama. Each test builds its own
//! TempDir KB via `common::write_config` + `common::ingest` and drives
//! `kebab search` through `common::run_search_with_args` or direct
//! `Command` invocations. Verifies:
//!
//! - `--doc-id <id>` restricts all returned hits to the target document.
//! - `--ingested-after <bad>` exits non-zero and emits `error.v1` on
//!   stderr with `code = "config_invalid"`.
//! - `--media md` (alias) normalises to `markdown` and matches `.md` docs.
//! - `--tag <tag>` (repeatable, OR-within) filters by frontmatter tags.

mod common;

use serde_json::Value;
use std::fs;
use std::process::Command;

// ---------------------------------------------------------------------------
// Test 1: --doc-id restricts hits to a single document
// ---------------------------------------------------------------------------

#[test]
fn search_with_doc_id_filter_returns_only_target_doc() {
    let dir = tempfile::tempdir().unwrap();
    let (cfg, workspace, _data) = common::write_config(dir.path(), 30);

    // Two docs that both contain the search term.
    fs::write(workspace.join("a.md"), "# Alpha\n\nrust ownership rules\n").unwrap();
    fs::write(workspace.join("b.md"), "# Beta\n\nrust borrow checker\n").unwrap();
    common::ingest(&cfg, &workspace);

    // First, search without a doc-id filter to find what doc_ids exist.
    let (stdout, _) = common::run_search_with_args(
        &cfg,
        &["--json", "--mode", "lexical", "rust"],
    );
    let resp: Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("not JSON: {stdout:?}: {e}"));
    let hits = resp["hits"].as_array().expect("hits array");
    assert!(
        hits.len() >= 2,
        "expected ≥2 hits from two docs before filter: {resp}"
    );

    // Grab one doc_id from the results.
    let target_doc_id = hits[0]["doc_id"]
        .as_str()
        .expect("doc_id string")
        .to_string();

    // Re-search with --doc-id set to the first hit's doc_id.
    let (stdout2, _) = common::run_search_with_args(
        &cfg,
        &[
            "--json",
            "--mode",
            "lexical",
            "--doc-id",
            &target_doc_id,
            "rust",
        ],
    );
    let resp2: Value = serde_json::from_str(stdout2.trim())
        .unwrap_or_else(|e| panic!("not JSON after filter: {stdout2:?}: {e}"));
    let filtered_hits = resp2["hits"].as_array().expect("hits array (filtered)");

    assert!(
        !filtered_hits.is_empty(),
        "expected at least one hit for the target doc"
    );
    for hit in filtered_hits {
        let got = hit["doc_id"].as_str().expect("doc_id string in hit");
        assert_eq!(
            got, target_doc_id,
            "--doc-id filter must restrict all hits to target doc, got {got}"
        );
    }
}

// ---------------------------------------------------------------------------
// Test 2: --ingested-after with bad RFC3339 → exit non-zero + error.v1
// ---------------------------------------------------------------------------

#[test]
fn search_with_invalid_ingested_after_emits_config_invalid() {
    let dir = tempfile::tempdir().unwrap();
    let (cfg, workspace, _data) = common::write_config(dir.path(), 30);
    fs::write(workspace.join("a.md"), "# T\n\nrust stuff\n").unwrap();
    common::ingest(&cfg, &workspace);

    let bin = env!("CARGO_BIN_EXE_kebab");
    let out = Command::new(bin)
        .args([
            "--config",
            cfg.to_str().unwrap(),
            "--json",
            "search",
            "--mode",
            "lexical",
            "--ingested-after",
            "not-a-date",
            "rust",
        ])
        .output()
        .expect("kebab search --ingested-after bad");

    assert!(
        !out.status.success(),
        "expected non-zero exit for invalid --ingested-after, got: status={} stderr={}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );

    let stderr = String::from_utf8_lossy(&out.stderr);
    // Find the error.v1 ndjson line on stderr (one JSON event per line).
    let err_line = stderr
        .lines()
        .find(|l| {
            serde_json::from_str::<Value>(l)
                .ok()
                .and_then(|v| {
                    v.get("schema_version")
                        .and_then(|s| s.as_str())
                        .map(String::from)
                })
                .as_deref()
                == Some("error.v1")
        })
        .unwrap_or_else(|| panic!("no error.v1 line on stderr: {stderr:?}"));

    let v: Value = serde_json::from_str(err_line).expect("error.v1 json");
    assert_eq!(
        v["code"], "config_invalid",
        "code must be config_invalid for bad RFC3339: {err_line}"
    );
}

// ---------------------------------------------------------------------------
// Test 3: --media md (alias) normalises to markdown and matches .md docs
// ---------------------------------------------------------------------------

#[test]
fn search_with_media_filter_md_alias_normalizes_to_markdown() {
    let dir = tempfile::tempdir().unwrap();
    let (cfg, workspace, _data) = common::write_config(dir.path(), 30);

    // Only a markdown file — the `md` alias should match it.
    fs::write(workspace.join("notes.md"), "# Notes\n\nrust async programming\n").unwrap();
    common::ingest(&cfg, &workspace);

    let (stdout, _) = common::run_search_with_args(
        &cfg,
        &["--json", "--mode", "lexical", "--media", "md", "rust"],
    );
    let resp: Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("not JSON: {stdout:?}: {e}"));
    let hits = resp["hits"].as_array().expect("hits array");

    assert!(
        !hits.is_empty(),
        "--media md must match the markdown doc; got 0 hits: {resp}"
    );
}

// ---------------------------------------------------------------------------
// Test 4: --tag (repeatable, OR-within) filters by frontmatter tags
// ---------------------------------------------------------------------------

#[test]
fn search_with_tag_filter_matches_frontmatter_tags() {
    let dir = tempfile::tempdir().unwrap();
    let (cfg, workspace, _data) = common::write_config(dir.path(), 30);

    // Doc with `rust` tag.
    fs::write(
        workspace.join("rust_doc.md"),
        "---\ntags: [rust, systems]\n---\n# Rust\n\nrust ownership\n",
    )
    .unwrap();
    // Doc without the tag (but same keyword in body so it appears in
    // unfiltered results — the tag filter must exclude it).
    fs::write(
        workspace.join("other_doc.md"),
        "# Other\n\nrust programming\n",
    )
    .unwrap();
    common::ingest(&cfg, &workspace);

    // Without filter — both docs must produce hits.
    let (unfiltered, _) = common::run_search_with_args(
        &cfg,
        &["--json", "--mode", "lexical", "rust"],
    );
    let uresp: Value = serde_json::from_str(unfiltered.trim())
        .unwrap_or_else(|e| panic!("not JSON (unfiltered): {unfiltered:?}: {e}"));
    let uhits = uresp["hits"].as_array().expect("unfiltered hits array");
    assert!(
        uhits.len() >= 2,
        "expected ≥2 hits before tag filter: {uresp}"
    );

    // With --tag rust — only the tagged doc's hits should appear.
    let (filtered, _) = common::run_search_with_args(
        &cfg,
        &["--json", "--mode", "lexical", "--tag", "rust", "rust"],
    );
    let fresp: Value = serde_json::from_str(filtered.trim())
        .unwrap_or_else(|e| panic!("not JSON (tag-filtered): {filtered:?}: {e}"));
    let fhits = fresp["hits"].as_array().expect("filtered hits array");

    assert!(
        !fhits.is_empty(),
        "--tag rust must match the tagged doc; got 0 hits: {fresp}"
    );

    // Every returned hit must come from rust_doc.md (the tagged file).
    for hit in fhits {
        let path = hit["doc_path"].as_str().unwrap_or("");
        assert!(
            path.ends_with("rust_doc.md"),
            "--tag rust must only return hits from the tagged doc, got path={path}"
        );
    }
}
