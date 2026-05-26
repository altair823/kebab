//! Integration: kebab_app::ingest_file_with_config copies external file
//! to _external/, ingests as single asset, idempotent on second call.

use std::fs;

use kebab_config::Config;

#[test]
fn ingest_file_copies_external_md_and_reports_new() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("notes");
    let data = dir.path().join("data");
    fs::create_dir_all(&workspace).unwrap();
    fs::create_dir_all(&data).unwrap();

    let mut cfg = Config::defaults();
    cfg.workspace.root = workspace.to_string_lossy().into_owned();
    cfg.storage.data_dir = data.to_string_lossy().into_owned();
    cfg.models.embedding.provider = "none".to_string();
    cfg.models.embedding.dimensions = 0;

    // Source file outside the workspace.
    let external_src = dir.path().join("source.md");
    fs::write(&external_src, "# Hello\n\nbody.").unwrap();

    let report = kebab_app::ingest_file_with_config(cfg.clone(), &external_src).unwrap();
    assert_eq!(report.scanned, 1, "{report:?}");
    assert_eq!(report.new, 1, "{report:?}");
    assert_eq!(report.unchanged, 0, "{report:?}");

    // _external/ dir created, file copied with hash prefix.
    let ext_dir = workspace.join("_external");
    assert!(ext_dir.is_dir());
    let entries: Vec<_> = fs::read_dir(&ext_dir)
        .unwrap()
        .filter_map(std::result::Result::ok)
        .collect();
    assert_eq!(entries.len(), 1, "exactly one file in _external/");
    let name = entries[0].file_name().to_string_lossy().into_owned();
    assert!(name.ends_with(".md"));

    // .kebabignore has _external/ line.
    let ki = fs::read_to_string(workspace.join(".kebabignore")).unwrap();
    assert!(ki.lines().any(|l| l.trim() == "_external/"));
}

#[test]
fn ingest_file_idempotent_on_second_call() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("notes");
    let data = dir.path().join("data");
    fs::create_dir_all(&workspace).unwrap();
    fs::create_dir_all(&data).unwrap();

    let mut cfg = Config::defaults();
    cfg.workspace.root = workspace.to_string_lossy().into_owned();
    cfg.storage.data_dir = data.to_string_lossy().into_owned();
    cfg.models.embedding.provider = "none".to_string();
    cfg.models.embedding.dimensions = 0;

    let src = dir.path().join("doc.md");
    fs::write(&src, "# A\n\nbody.").unwrap();

    let r1 = kebab_app::ingest_file_with_config(cfg.clone(), &src).unwrap();
    assert_eq!(r1.new, 1);

    let r2 = kebab_app::ingest_file_with_config(cfg.clone(), &src).unwrap();
    assert_eq!(r2.new, 0, "{r2:?}");
    assert_eq!(r2.unchanged, 1, "{r2:?}");
}

#[test]
fn ingest_file_errors_on_missing_path() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("notes");
    let data = dir.path().join("data");
    fs::create_dir_all(&workspace).unwrap();
    fs::create_dir_all(&data).unwrap();

    let mut cfg = Config::defaults();
    cfg.workspace.root = workspace.to_string_lossy().into_owned();
    cfg.storage.data_dir = data.to_string_lossy().into_owned();
    cfg.models.embedding.provider = "none".to_string();
    cfg.models.embedding.dimensions = 0;

    let nonexistent = dir.path().join("nope.md");
    let err = kebab_app::ingest_file_with_config(cfg, &nonexistent).unwrap_err();
    assert!(err.to_string().contains("does not exist"), "{err}");
}

#[test]
fn ingest_file_errors_on_unsupported_extension() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("notes");
    let data = dir.path().join("data");
    fs::create_dir_all(&workspace).unwrap();
    fs::create_dir_all(&data).unwrap();

    let mut cfg = Config::defaults();
    cfg.workspace.root = workspace.to_string_lossy().into_owned();
    cfg.storage.data_dir = data.to_string_lossy().into_owned();
    cfg.models.embedding.provider = "none".to_string();
    cfg.models.embedding.dimensions = 0;

    let docx = dir.path().join("doc.docx");
    fs::write(&docx, b"fake docx bytes").unwrap();

    let err = kebab_app::ingest_file_with_config(cfg, &docx).unwrap_err();
    assert!(err.to_string().contains("unsupported extension"), "{err}");
    assert!(err.to_string().contains(".docx") || err.to_string().contains("docx"), "{err}");
}
