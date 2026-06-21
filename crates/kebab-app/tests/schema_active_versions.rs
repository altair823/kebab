//! Integration tests for Bug #13: schema.v1.models.active_parsers + active_chunkers.

use kebab_app::schema_with_config;
use kebab_config::Config;
use kebab_core::SourceScope;

fn minimal_config(data_dir: &std::path::Path, workspace_root: &std::path::Path) -> Config {
    let mut cfg = Config::defaults();
    cfg.workspace.root = Some(workspace_root.to_string_lossy().into_owned());
    cfg.workspace.exclude.clear();
    cfg.storage.data_dir = data_dir.to_string_lossy().into_owned();
    cfg.storage.model_dir = data_dir.join("models").to_string_lossy().into_owned();
    cfg.models.embedding.provider = "none".to_string();
    cfg.models.embedding.dimensions = 0;
    cfg.ingest.chunking.target_tokens = 80;
    cfg.ingest.chunking.overlap_tokens = 20;
    cfg
}

fn minimal_scope(workspace_root: &std::path::Path) -> SourceScope {
    SourceScope {
        root: workspace_root.to_path_buf(),
        include: vec![],
        exclude: vec![],
    }
}

#[test]
fn schema_models_active_arrays_empty_on_empty_corpus() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("kb");
    std::fs::create_dir_all(&workspace).unwrap();
    let cfg = minimal_config(dir.path(), &workspace);

    let store = kebab_store_sqlite::SqliteStore::open(&cfg).unwrap();
    store.run_migrations().unwrap();
    drop(store);

    let s = schema_with_config(&cfg).unwrap();
    assert!(
        s.models.active_parsers.is_empty(),
        "empty corpus → no parsers"
    );
    assert!(
        s.models.active_chunkers.is_empty(),
        "empty corpus → no chunkers"
    );
    // backward compat: 기존 단일 field 는 markdown default 보존.
    assert_eq!(s.models.parser_version, kebab_parse_md::PARSER_VERSION);
}

#[test]
fn schema_emits_active_parsers_and_chunkers_array_after_ingest() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("kb");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(workspace.join("a.md"), "# A\nhello world\n").unwrap();
    let cfg = minimal_config(dir.path(), &workspace);
    let scope = minimal_scope(&workspace);

    kebab_app::ingest_with_config(cfg.clone(), scope, false).unwrap();

    let s = schema_with_config(&cfg).unwrap();
    assert!(
        !s.models.active_parsers.is_empty(),
        "active_parsers populated after ingest"
    );
    assert!(
        !s.models.active_chunkers.is_empty(),
        "active_chunkers populated after ingest"
    );
    // active arrays must be sorted (ORDER BY in SQL).
    let mut sorted = s.models.active_parsers.clone();
    sorted.sort();
    assert_eq!(
        s.models.active_parsers, sorted,
        "active_parsers must be sorted"
    );
}
