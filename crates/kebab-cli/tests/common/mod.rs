//! Shared CLI integration-test helpers.
//!
//! Each consumer (`tests/wire_search_stale.rs`, `tests/wire_ask_stale.rs`)
//! does `mod common;` and calls these via `common::write_config(...)`,
//! `common::ingest(...)`, `common::backdate_updated_at(...)`.
//!
//! `#![allow(dead_code)]` because each consumer typically uses only a
//! subset of the helpers; rustc would otherwise warn about the unused
//! ones in any single consumer's compilation.

#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Build a `config.toml` text under `dir`. `workspace_root` and
/// `data_dir` live inside `dir`. `stale_threshold_days` is plumbed
/// into `[search]` so the staleness post-process can fire.
///
/// Returns `(cfg_path, workspace_dir, data_dir)`.
pub fn write_config(dir: &Path, stale_threshold_days: u32) -> (PathBuf, PathBuf, PathBuf) {
    write_config_with_llm_model(dir, stale_threshold_days, "none")
}

/// Like [`write_config`] but lets the caller pin a specific
/// `[models.llm].model` value — needed by `wire_ask_stale.rs` which
/// hits a real Ollama and wants `gemma4:e4b` instead of `none`.
pub fn write_config_with_llm_model(
    dir: &Path,
    stale_threshold_days: u32,
    llm_model: &str,
) -> (PathBuf, PathBuf, PathBuf) {
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
model = "{llm_model}"
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
prompt_template_version = "rag-v4"
score_gate = 0.30
explain_default = false
max_context_tokens = 8000
"#,
            workspace = workspace.display(),
            data = data.display(),
            llm_model = llm_model,
            stale_threshold_days = stale_threshold_days,
        ),
    )
    .unwrap();
    (cfg_path, workspace, data)
}

/// Run `kebab ingest --root <workspace>` against the given config.
/// Asserts success — failures abort the calling test.
pub fn ingest(cfg: &Path, workspace: &Path) {
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

/// p9-fb-34: invoke `kebab search` with arbitrary trailing flags +
/// query, capture stdout + stderr. Caller is responsible for
/// supplying `--mode lexical` / `--json` etc. as needed; this helper
/// stays unopinionated so a single test can exercise both wire shapes
/// (JSON wrapper + plain stderr hint). Asserts the binary exited 0;
/// non-zero exits fail the test with stderr included.
pub fn run_search_with_args(cfg: &Path, args: &[&str]) -> (String, String) {
    let bin = env!("CARGO_BIN_EXE_kebab");
    let mut cmd = Command::new(bin);
    cmd.arg("--config").arg(cfg).arg("search");
    cmd.args(args);
    let out = cmd.output().expect("kebab search");
    assert!(
        out.status.success(),
        "search failed: args={args:?} stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

/// p9-fb-33: invoke `kebab ask --stream --mode lexical <query>` and
/// capture stdout + stderr. Lexical mode skips embeddings (matches
/// `wire_ask_stale.rs::run_ask_lexical`). Caller asserts on the
/// resulting (stdout, stderr) pair.
pub fn run_ask_stream(cfg: &Path, query: &str) -> (String, String) {
    let bin = env!("CARGO_BIN_EXE_kebab");
    let out = Command::new(bin)
        .args([
            "--config",
            cfg.to_str().unwrap(),
            "ask",
            "--stream",
            "--mode",
            "lexical",
            query,
        ])
        .output()
        .expect("kebab ask --stream");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

/// p9-fb-33: invoke `kebab --json ask --mode lexical <query>` (no
/// `--stream`) — used by `wire_ask_stream::non_stream_path_unchanged`
/// to confirm the non-streaming JSON path still emits a single
/// `answer.v1` line on stdout. Returns stdout only (mirrors
/// `wire_ask_stale.rs::run_ask_lexical(json=true)` minus the
/// `Output` indirection).
pub fn run_ask_json(cfg: &Path, query: &str) -> String {
    let bin = env!("CARGO_BIN_EXE_kebab");
    let out = Command::new(bin)
        .args([
            "--config",
            cfg.to_str().unwrap(),
            "--json",
            "ask",
            "--mode",
            "lexical",
            query,
        ])
        .output()
        .expect("kebab ask --json");
    String::from_utf8_lossy(&out.stdout).to_string()
}

/// p9-fb-35: invoke `kebab fetch` with arbitrary trailing flags,
/// capture stdout + stderr. Caller is responsible for supplying
/// `--json` (global flag) before the subcommand position via the
/// `args` slice (e.g. `&["--json", "chunk", &id]`). Asserts the
/// binary exited 0; non-zero exits fail the test with stderr
/// included — for negative-path tests (unknown chunk_id etc.) drive
/// the binary directly via `std::process::Command`.
pub fn run_fetch_with_args(cfg: &Path, args: &[&str]) -> (String, String) {
    let bin = env!("CARGO_BIN_EXE_kebab");
    let mut cmd = Command::new(bin);
    cmd.arg("--config").arg(cfg).arg("fetch");
    cmd.args(args);
    let out = cmd.output().expect("kebab fetch");
    assert!(
        out.status.success(),
        "fetch failed: args={args:?} stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

/// Rewrite `documents.updated_at` for one workspace path to
/// `now - days_ago` (RFC3339 UTC). Mirrors
/// `kebab-app/tests/common/mod.rs::backdate_document_updated_at`.
/// Asserts exactly one row is updated — typo-proofs the workspace path.
pub fn backdate_updated_at(data_dir: &Path, workspace_path: &str, days_ago: i64) {
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
