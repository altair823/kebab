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

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Build a `config.toml` text under `dir`. `workspace_root` and
/// `data_dir` live inside `dir`. `stale_threshold_days` is plumbed
/// into `[search]` so the staleness post-process can fire.
fn write_config(dir: &Path, stale_threshold_days: u32) -> (PathBuf, PathBuf, PathBuf) {
    let workspace = dir.join("workspace");
    let data = dir.join("data");
    fs::create_dir_all(&workspace).unwrap();
    fs::create_dir_all(&data).unwrap();

    let cfg_path = dir.join("config.toml");
    fs::write(
        &cfg_path,
        format!(
            r#"schema_version = 1

[workspace]
root = "{workspace}"
exclude = [".git/**"]

[storage]
data_dir = "{data}"
sqlite = "{{data_dir}}/kebab.sqlite"
vector_dir = "{{data_dir}}/lancedb"
asset_dir = "{{data_dir}}/assets"
artifact_dir = "{{data_dir}}/artifacts"
model_dir = "{{data_dir}}/models"
runs_dir = "{{data_dir}}/runs"
copy_threshold_mb = 100

[indexing]
max_parallel_extractors = 2
max_parallel_embeddings = 1
watch_filesystem = false

[chunking]
target_tokens = 80
overlap_tokens = 20
respect_markdown_headings = true
chunker_version = "md-heading-v1"

[models.embedding]
provider = "none"
model = "none"
version = "v0"
dimensions = 0
batch_size = 1

[models.llm]
provider = "ollama"
model = "gemma4:e4b"
context_tokens = 4096
endpoint = "http://127.0.0.1:11434"
temperature = 0.0
seed = 0

[search]
default_k = 10
hybrid_fusion = "rrf"
rrf_k = 60
snippet_chars = 220
stale_threshold_days = {stale_threshold_days}

[rag]
prompt_template_version = "rag-v1"
score_gate = 0.30
explain_default = false
max_context_tokens = 8000
"#,
            workspace = workspace.display(),
            data = data.display(),
            stale_threshold_days = stale_threshold_days,
        ),
    )
    .unwrap();
    (cfg_path, workspace, data)
}

fn ingest(cfg: &Path, workspace: &Path) {
    let bin = env!("CARGO_BIN_EXE_kebab");
    let out = Command::new(bin)
        .args([
            "--config",
            cfg.to_str().unwrap(),
            "ingest",
            "--root",
            workspace.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "ingest failed: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
}

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

/// Rewrite `documents.updated_at` for one workspace path to
/// `now - days_ago` (RFC3339 UTC). Mirrors
/// `kebab-app/tests/common/mod.rs::backdate_document_updated_at`.
fn backdate_updated_at(data_dir: &Path, workspace_path: &str, days_ago: i64) {
    let backdated = (time::OffsetDateTime::now_utc() - time::Duration::days(days_ago))
        .format(&time::format_description::well_known::Rfc3339)
        .expect("format backdated updated_at");
    let db_path = data_dir.join("kebab.sqlite");
    let conn = rusqlite::Connection::open(&db_path).expect("open kebab.sqlite");
    let updated = conn
        .execute(
            "UPDATE documents SET updated_at = ?1 WHERE workspace_path = ?2",
            rusqlite::params![backdated, workspace_path],
        )
        .expect("UPDATE documents.updated_at");
    assert_eq!(
        updated, 1,
        "backdate_updated_at: expected to update exactly 1 row for {workspace_path}, got {updated}"
    );
}

#[test]
#[ignore = "requires real Ollama on 127.0.0.1:11434"]
fn ask_json_citations_include_indexed_at_and_stale() {
    let dir = tempfile::tempdir().unwrap();
    let (cfg, workspace, data) = write_config(dir.path(), 30);
    fs::write(workspace.join("a.md"), "# T\n\napples are fruit\n").unwrap();
    ingest(&cfg, &workspace);
    backdate_updated_at(&data, "a.md", 60);

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
    let (cfg, workspace, data) = write_config(dir.path(), 30);
    fs::write(workspace.join("a.md"), "# T\n\napples are fruit\n").unwrap();
    ingest(&cfg, &workspace);
    backdate_updated_at(&data, "a.md", 60);

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
