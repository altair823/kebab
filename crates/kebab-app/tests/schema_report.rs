//! Integration test: kebab_app::schema_with_config returns a SchemaV1
//! that is internally consistent with a freshly-ingested TempDir KB.

use std::fs;

use kebab_config::Config;
use kebab_core::SourceScope;

fn minimal_config(data_dir: &std::path::Path, workspace_root: &std::path::Path) -> Config {
    let mut config = Config::defaults();
    config.workspace.root = workspace_root.to_string_lossy().into_owned();
    config.workspace.exclude.clear();
    config.storage.data_dir = data_dir.to_string_lossy().into_owned();
    config.storage.model_dir = data_dir.join("models").to_string_lossy().into_owned();
    config.models.embedding.provider = "none".to_string();
    config.models.embedding.dimensions = 0;
    config.ingest.chunking.target_tokens = 80;
    config.ingest.chunking.overlap_tokens = 20;
    config
}

fn minimal_scope(workspace_root: &std::path::Path) -> SourceScope {
    SourceScope {
        root: workspace_root.to_path_buf(),
        include: vec![],
        exclude: vec![],
    }
}

#[test]
fn schema_report_reflects_freshly_ingested_kb() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace_root = temp.path().join("workspace");
    let data_dir = temp.path().join("data");
    fs::create_dir_all(&workspace_root).unwrap();
    fs::create_dir_all(&data_dir).unwrap();

    fs::write(workspace_root.join("a.md"), "# A\n\nbody A.").unwrap();
    fs::write(workspace_root.join("b.md"), "# B\n\nbody B.").unwrap();

    let config = minimal_config(&data_dir, &workspace_root);
    let _report =
        kebab_app::ingest_with_config(config.clone(), minimal_scope(&workspace_root), false)
            .unwrap();

    let schema = kebab_app::schema_with_config(&config).unwrap();

    assert!(!schema.kebab_version.is_empty());
    assert!(
        schema.wire.schemas.contains(&"schema.v1".to_string()),
        "schema.v1 missing from wire.schemas: {:?}",
        schema.wire.schemas
    );
    assert!(
        schema.wire.schemas.contains(&"error.v1".to_string()),
        "error.v1 missing from wire.schemas: {:?}",
        schema.wire.schemas
    );
    assert!(schema.capabilities.json_mode);
    assert!(schema.capabilities.streaming_ask); // Bug #9: streaming_ask is now true
    assert!(
        schema.capabilities.mcp_server,
        "mcp_server should be true after fb-30",
    );
    assert_eq!(
        schema.stats.doc_count, 2,
        "expected 2 docs (a.md + b.md): {:?}",
        schema.stats
    );
    assert!(
        schema.stats.last_ingest_at.is_some(),
        "last_ingest_at must be set after ingest: {:?}",
        schema.stats
    );
    assert!(
        schema.stats.chunk_count >= 2,
        "expected ≥2 chunks (a.md + b.md): {:?}",
        schema.stats
    );
    assert_eq!(
        schema.stats.asset_count, 2,
        "expected 2 assets (a.md + b.md): {:?}",
        schema.stats
    );
}

#[test]
fn schema_report_on_empty_kb_has_zero_counts() {
    // An empty workspace dir with no .md files: ingest_with_config scans 0
    // files but still creates + migrates kebab.sqlite. This seeds the DB so
    // open_existing (used inside schema_with_config) succeeds and returns
    // all-zero counts.
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace_root = temp.path().join("workspace");
    let data_dir = temp.path().join("data");
    fs::create_dir_all(&workspace_root).unwrap();
    fs::create_dir_all(&data_dir).unwrap();

    let config = minimal_config(&data_dir, &workspace_root);
    // Run ingest over the empty workspace — creates kebab.sqlite, runs
    // migrations, records 0 docs. schema_with_config can then open_existing.
    let report =
        kebab_app::ingest_with_config(config.clone(), minimal_scope(&workspace_root), false)
            .unwrap();
    assert_eq!(report.new, 0, "empty workspace should yield 0 new docs");

    let schema = kebab_app::schema_with_config(&config).unwrap();
    assert_eq!(
        schema.stats.doc_count, 0,
        "empty KB doc_count: {:?}",
        schema.stats
    );
    assert_eq!(
        schema.stats.chunk_count, 0,
        "empty KB chunk_count: {:?}",
        schema.stats
    );
    assert!(
        schema.stats.last_ingest_at.is_none(),
        "last_ingest_at must be None when no docs ingested: {:?}",
        schema.stats
    );
}
