//! p9-fb-32: CLI emits `indexed_at` + `stale` on JSON; plain output
//! gains a `[stale]` tag prefix on stale hits.
//!
//! Self-contained: each test builds a TempDir workspace + config,
//! invokes the `kebab` binary via `CARGO_BIN_EXE_kebab`, and (for the
//! plain-output stale path) backdates `documents.updated_at` directly
//! via `rusqlite` to simulate an aged-out doc without faking system
//! time. Mirrors the helper pattern in
//! `crates/kebab-app/tests/common/mod.rs::backdate_document_updated_at`.
//!
//! Shared TempDir / ingest / backdate helpers live in
//! `tests/common/mod.rs`; see also `wire_ask_stale.rs`.

mod common;

use std::fs;
use std::path::Path;
use std::process::Command;

fn run_search_lexical(cfg: &Path, query: &str, json: bool) -> std::process::Output {
    let bin = env!("CARGO_BIN_EXE_kebab");
    let mut cmd = Command::new(bin);
    cmd.arg("--config").arg(cfg);
    if json {
        cmd.arg("--json");
    }
    // Force lexical so the test doesn't need fastembed / AVX. Hybrid
    // is the CLI default which would try the vector path.
    cmd.args(["search", "--mode", "lexical", query]);
    let out = cmd.output().unwrap();
    assert!(
        out.status.success(),
        "search failed: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

#[test]
fn search_json_includes_indexed_at_and_stale() {
    let dir = tempfile::tempdir().unwrap();
    let (cfg, workspace, _data) = common::write_config(dir.path(), 30);
    fs::write(workspace.join("a.md"), "# Title\n\napples are fruit\n").unwrap();
    common::ingest(&cfg, &workspace);

    let out = run_search_lexical(&cfg, "apples", true);
    let stdout = String::from_utf8_lossy(&out.stdout);
    // p9-fb-34: top-level wire is now `search_response.v1` wrapping the
    // legacy `search_hit.v1[]` under a `hits` field (with pagination +
    // truncation metadata). Hit shape inside `hits` is unchanged.
    let resp: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("expected JSON object, got {stdout:?}: {e}"));
    assert_eq!(
        resp.get("schema_version").and_then(|v| v.as_str()),
        Some("search_response.v1"),
        "expected search_response.v1 wrapper, got {resp}"
    );
    let arr = resp
        .get("hits")
        .and_then(|h| h.as_array())
        .unwrap_or_else(|| panic!("expected hits array, got {stdout}"));
    let first = arr
        .first()
        .unwrap_or_else(|| panic!("expected ≥1 hit, got empty hits: {stdout}"));
    assert!(
        first.get("indexed_at").is_some(),
        "missing indexed_at in {first}"
    );
    assert!(first.get("stale").is_some(), "missing stale in {first}");
    assert_eq!(
        first["stale"], false,
        "freshly ingested doc must not be stale at default 30d threshold"
    );
}

#[test]
fn search_plain_marks_stale_doc() {
    let dir = tempfile::tempdir().unwrap();
    let (cfg, workspace, data) = common::write_config(dir.path(), 30);
    fs::write(workspace.join("a.md"), "# Title\n\napples are fruit\n").unwrap();
    common::ingest(&cfg, &workspace);
    common::backdate_updated_at(&data, "a.md", 60);

    let out = run_search_lexical(&cfg, "apples", false);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("[stale]"),
        "stale tag missing in plain output:\n{stdout}"
    );
}

#[test]
fn search_plain_no_stale_tag_for_fresh_doc() {
    let dir = tempfile::tempdir().unwrap();
    let (cfg, workspace, _data) = common::write_config(dir.path(), 30);
    fs::write(workspace.join("a.md"), "# Title\n\napples are fruit\n").unwrap();
    common::ingest(&cfg, &workspace);

    let out = run_search_lexical(&cfg, "apples", false);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("[stale]"),
        "unexpected stale tag in plain output for fresh doc:\n{stdout}"
    );
}
