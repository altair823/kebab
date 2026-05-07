//! Integration: kebab_app::ingest_stdin_with_config injects frontmatter,
//! writes to _external/, ingests as single asset.

use std::fs;

use kebab_config::Config;

fn fresh_cfg(dir: &std::path::Path) -> Config {
    let workspace = dir.join("notes");
    let data = dir.join("data");
    fs::create_dir_all(&workspace).unwrap();
    fs::create_dir_all(&data).unwrap();

    let mut cfg = Config::defaults();
    cfg.workspace.root = workspace.to_string_lossy().into_owned();
    cfg.storage.data_dir = data.to_string_lossy().into_owned();
    cfg.models.embedding.provider = "none".to_string();
    cfg.models.embedding.dimensions = 0;
    cfg
}

#[test]
fn ingest_stdin_writes_frontmatter_and_reports_new() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = fresh_cfg(dir.path());

    let report = kebab_app::ingest_stdin_with_config(
        cfg.clone(),
        "## Body content\n\nMore.",
        "Article X",
        Some("https://example.com/x"),
    ).unwrap();
    assert_eq!(report.new, 1, "{report:?}");

    // _external/ contains exactly one .md file with frontmatter.
    let ext_dir = std::path::PathBuf::from(&cfg.workspace.root).join("_external");
    let entries: Vec<_> = fs::read_dir(&ext_dir).unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert_eq!(entries.len(), 1);
    let content = fs::read_to_string(entries[0].path()).unwrap();
    assert!(content.starts_with("---\n"));
    assert!(content.contains("title: \"Article X\""));
    assert!(content.contains("source_uri: \"https://example.com/x\""));
    assert!(content.contains("## Body content"));
}

#[test]
fn ingest_stdin_without_source_uri() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = fresh_cfg(dir.path());

    let report = kebab_app::ingest_stdin_with_config(
        cfg.clone(),
        "## Body",
        "Title",
        None,
    ).unwrap();
    assert_eq!(report.new, 1);

    let ext_dir = std::path::PathBuf::from(&cfg.workspace.root).join("_external");
    let entries: Vec<_> = fs::read_dir(&ext_dir).unwrap()
        .filter_map(|e| e.ok())
        .collect();
    let content = fs::read_to_string(entries[0].path()).unwrap();
    assert!(content.contains("title: \"Title\""));
    assert!(!content.contains("source_uri"));
}

#[test]
fn ingest_stdin_errors_on_existing_frontmatter() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = fresh_cfg(dir.path());

    let body = "---\ntitle: Already\n---\n\n## Body";
    let err = kebab_app::ingest_stdin_with_config(cfg, body, "New", None).unwrap_err();
    assert!(err.to_string().contains("already has frontmatter"), "{err}");
}
