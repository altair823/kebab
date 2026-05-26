//! Integration: spawn `kebab ingest-file <path>` and verify ingest_report.v1.

use std::fs;
use std::process::Command;

#[test]
fn cli_ingest_file_emits_ingest_report_v1() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("notes");
    let data = dir.path().join("data");
    fs::create_dir_all(&workspace).unwrap();
    fs::create_dir_all(&data).unwrap();

    let cfg_path = dir.path().join("config.toml");
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
target_tokens = 500
overlap_tokens = 80
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

[rag]
prompt_template_version = "rag-v1"
score_gate = 0.30
explain_default = false
max_context_tokens = 8000
"#,
            workspace = workspace.display(),
            data = data.display(),
        ),
    ).unwrap();

    let src = dir.path().join("doc.md");
    fs::write(&src, "# A\n\nbody.").unwrap();

    let bin = env!("CARGO_BIN_EXE_kebab");
    let out = Command::new(bin)
        .args(["--json", "--config", cfg_path.to_str().unwrap(), "ingest-file"])
        .arg(&src)
        .output()
        .unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));

    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(v.get("schema_version").and_then(|s| s.as_str()), Some("ingest_report.v1"));
    assert_eq!(v.get("new").and_then(serde_json::Value::as_u64), Some(1));
}
