//! p9-fb-32: CLI ask output — JSON path emits `indexed_at` + `stale`
//! on each citation; plain output prefixes stale citations with
//! `[stale]` (yellow on TTY).
//!
//! These end-to-end checks exercise `kebab ask`, which requires a real
//! Ollama on `127.0.0.1:11434` (same constraint as
//! `kebab-app/tests/ask_smoke.rs`). Both tests are therefore
//! `#[ignore]` by default — run with
//! `cargo test -p kebab-cli --test wire_ask_stale -- --ignored`
//! against a live Ollama.
//!
//! The `[stale]` rendering logic itself is also covered by a unit test
//! in `kebab-cli/src/main.rs` (`tests::plain_marks_stale_citation_*`)
//! that constructs a synthetic `Answer` and pipes it through
//! `render_ask_plain_citations` — that path is the always-on guard.
//!
//! Shared TempDir / ingest / backdate helpers live in
//! `tests/common/mod.rs`; see also `wire_search_stale.rs`.

mod common;

use std::fs;
use std::path::Path;
use std::process::Command;

/// Run `kebab ask` in lexical mode (no embedding required). `json`
/// toggles `--json`. The caller asserts on the resulting stdout.
fn run_ask_lexical(cfg: &Path, query: &str, json: bool) -> std::process::Output {
    let bin = env!("CARGO_BIN_EXE_kebab");
    let mut cmd = Command::new(bin);
    cmd.arg("--config").arg(cfg);
    if json {
        cmd.arg("--json");
    }
    cmd.args(["ask", "--mode", "lexical", query]);
    cmd.output().unwrap()
}

#[test]
#[ignore = "requires real Ollama on 127.0.0.1:11434"]
fn ask_json_citations_include_indexed_at_and_stale() {
    let dir = tempfile::tempdir().unwrap();
    let (cfg, workspace, data) = common::write_config_with_llm_model(dir.path(), 30, "gemma4:e4b");
    fs::write(workspace.join("a.md"), "# T\n\napples are fruit\n").unwrap();
    common::ingest(&cfg, &workspace);
    common::backdate_updated_at(&data, "a.md", 60);

    // ask returns exit 1 on refusal; the JSON envelope still goes to
    // stdout. Don't assert on `status.success()` — accept either path
    // and require the citations array to be present + structurally valid.
    let out = run_ask_lexical(&cfg, "what about apples", true);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let answer: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("expected JSON answer, got {stdout:?}: {e}"));
    let cits = answer["citations"]
        .as_array()
        .unwrap_or_else(|| panic!("expected citations array, got {answer}"));
    if let Some(cit) = cits.first() {
        // Schema fields are always present on a structurally-valid
        // AnswerCitation (serde-derived per Task 2 + Task 8).
        assert!(
            cit.get("indexed_at").is_some(),
            "missing indexed_at on citation: {cit}"
        );
        assert!(
            cit.get("stale").is_some(),
            "missing stale on citation: {cit}"
        );
        assert_eq!(
            cit["stale"], true,
            "doc backdated 60d at threshold 30d must be stale: {cit}"
        );
    }
    // If the model refused with zero citations the schema-shape claim
    // is vacuously true; the unit-test path
    // (`tests::plain_marks_stale_citation_*` in main.rs) is the
    // always-on guard.
}

#[test]
#[ignore = "requires real Ollama on 127.0.0.1:11434"]
fn ask_plain_marks_stale_citation() {
    let dir = tempfile::tempdir().unwrap();
    let (cfg, workspace, data) = common::write_config_with_llm_model(dir.path(), 30, "gemma4:e4b");
    fs::write(workspace.join("a.md"), "# T\n\napples are fruit\n").unwrap();
    common::ingest(&cfg, &workspace);
    common::backdate_updated_at(&data, "a.md", 60);

    // Refusal exits 1 — that's still fine here, the renderer prints
    // the citation block before the refusal exit when citations exist.
    // If the model refused with zero citations, this test is
    // best-effort (skip the assert): the unit-test path in main.rs
    // (`tests::plain_marks_stale_citation_*`) is the always-on guard.
    let out = run_ask_lexical(&cfg, "what about apples", false);
    let stdout = String::from_utf8_lossy(&out.stdout);
    if stdout.contains("근거:") {
        assert!(
            stdout.contains("[stale]"),
            "stale tag missing in plain ask output:\n{stdout}"
        );
    }
}
