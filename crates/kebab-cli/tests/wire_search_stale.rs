//! p9-fb-32: CLI emits `indexed_at` + `stale` on JSON; plain output
//! gains a `[stale]` tag prefix on stale hits.
//!
//! Self-contained: each test builds a TempDir workspace + config,
//! invokes the `kebab` binary via `CARGO_BIN_EXE_kebab`, and (for the
//! plain-output stale path) backdates `documents.updated_at` directly
//! via `rusqlite` to simulate an aged-out doc without faking system
//! time. Mirrors the helper pattern in
//! `crates/kebab-app/tests/common/mod.rs::backdate_document_updated_at`.

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
model = "none"
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
fn search_json_includes_indexed_at_and_stale() {
    let dir = tempfile::tempdir().unwrap();
    let (cfg, workspace, _data) = write_config(dir.path(), 30);
    fs::write(workspace.join("a.md"), "# Title\n\napples are fruit\n").unwrap();
    ingest(&cfg, &workspace);

    let out = run_search_lexical(&cfg, "apples", true);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let arr: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("expected JSON array, got {stdout:?}: {e}"));
    let arr = arr.as_array().unwrap_or_else(|| panic!("expected array, got {stdout}"));
    let first = arr.first().unwrap_or_else(|| panic!("expected ≥1 hit, got empty array: {stdout}"));
    assert!(
        first.get("indexed_at").is_some(),
        "missing indexed_at in {first}"
    );
    assert!(
        first.get("stale").is_some(),
        "missing stale in {first}"
    );
    assert_eq!(
        first["stale"], false,
        "freshly ingested doc must not be stale at default 30d threshold"
    );
}

#[test]
fn search_plain_marks_stale_doc() {
    let dir = tempfile::tempdir().unwrap();
    let (cfg, workspace, data) = write_config(dir.path(), 30);
    fs::write(workspace.join("a.md"), "# Title\n\napples are fruit\n").unwrap();
    ingest(&cfg, &workspace);
    backdate_updated_at(&data, "a.md", 60);

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
    let (cfg, workspace, _data) = write_config(dir.path(), 30);
    fs::write(workspace.join("a.md"), "# Title\n\napples are fruit\n").unwrap();
    ingest(&cfg, &workspace);

    let out = run_search_lexical(&cfg, "apples", false);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("[stale]"),
        "unexpected stale tag in plain output for fresh doc:\n{stdout}"
    );
}
