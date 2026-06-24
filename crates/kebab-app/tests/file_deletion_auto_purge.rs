//! Dogfood: auto-purge stored docs for filesystem-deleted files.
//!
//! Two tests:
//!
//! 1. `file_deletion_auto_purge` — ingest 2 files, delete one, re-ingest.
//!    The re-ingest must report `purged_deleted_files = 1`, the deleted
//!    file must no longer appear in `list_docs`, and lexical search for
//!    its unique content must return no hits.
//!
//! 2. `include_scope_narrowing_does_not_purge` — ingest 2 files under a
//!    wide glob, narrow the walker scope to only one file, re-ingest.
//!    The narrowed ingest must NOT purge the out-of-scope file because
//!    the file is still on disk (just excluded from this run). Protects
//!    users against accidental data loss via config edits.

mod common;

use common::TestEnv;
use kebab_app::IngestOpts;
use kebab_app::ingest_with_config_opts;
use kebab_core::{DocFilter, DocumentStore, SearchMode, SearchQuery, SourceScope};

/// Helper: open the store via `TestEnv` and run `list_documents`.
fn list_doc_paths(env: &TestEnv) -> Vec<String> {
    use kebab_store_sqlite::SqliteStore;
    let store = SqliteStore::open(&env.config.storage).unwrap();
    store.run_migrations().unwrap();
    store
        .list_documents(&DocFilter::default())
        .unwrap()
        .into_iter()
        .map(|d| d.doc_path.0)
        .collect()
}

#[test]
fn file_deletion_auto_purge() {
    let env = TestEnv::lexical_only();

    // Write two .rs files into the workspace.
    let a_path = env.workspace_root.join("a.rs");
    let b_path = env.workspace_root.join("b.rs");
    std::fs::write(&a_path, "// file a\nfn alpha() {}\n").unwrap();
    std::fs::write(&b_path, "// file b\nfn bravo() {}\n").unwrap();

    // First ingest — both must be New.
    let first = ingest_with_config_opts(
        env.config.clone(),
        env.scope(),
        false,
        IngestOpts::default(),
    )
    .expect("first ingest must succeed");
    // Only count the .rs files we added (there may be fixture files too).
    let first_new = first.new;
    assert!(first_new >= 2, "expected at least 2 new docs: {first:?}");
    assert_eq!(
        first.purged_deleted_files, 0,
        "no purges on first ingest: {first:?}"
    );
    assert_eq!(first.errors, 0, "no errors on first ingest: {first:?}");

    // Delete one file from the filesystem.
    std::fs::remove_file(&b_path).expect("remove b.rs");

    // Second ingest — scanned count drops by 1; b.rs should be purged.
    let second = ingest_with_config_opts(
        env.config.clone(),
        env.scope(),
        false,
        IngestOpts::default(),
    )
    .expect("second ingest must succeed");

    assert_eq!(
        second.purged_deleted_files, 1,
        "exactly 1 file should be purged: {second:?}"
    );
    assert_eq!(second.new, 0, "no new docs after deletion: {second:?}");
    assert_eq!(second.updated, 0, "no updated docs: {second:?}");
    assert_eq!(second.errors, 0, "no errors: {second:?}");

    // b.rs must no longer appear in list_docs.
    let doc_paths = list_doc_paths(&env);
    let b_ws_path = "b.rs";
    assert!(
        !doc_paths.iter().any(|p| p == b_ws_path),
        "b.rs must be gone from list_docs; got: {doc_paths:?}"
    );
    // a.rs must still be present.
    let a_ws_path = "a.rs";
    assert!(
        doc_paths.iter().any(|p| p == a_ws_path),
        "a.rs must still be in list_docs; got: {doc_paths:?}"
    );

    // Lexical search for b.rs's unique content returns no hits.
    let app = env.app();
    let query = SearchQuery {
        text: "bravo".to_string(),
        mode: SearchMode::Lexical,
        k: 10,
        filters: kebab_core::SearchFilters::default(),
    };
    let hits = app.search(query).expect("search must not error");
    assert!(
        hits.is_empty(),
        "search for deleted file's content must return no hits; got: {hits:?}"
    );
}

#[test]
fn include_scope_narrowing_does_not_purge() {
    let env = TestEnv::lexical_only();

    // Write two .rs files.
    let a_path = env.workspace_root.join("a_narrow.rs");
    let b_path = env.workspace_root.join("b_narrow.rs");
    std::fs::write(&a_path, "// narrow a\nfn alpha_narrow() {}\n").unwrap();
    std::fs::write(&b_path, "// narrow b\nfn bravo_narrow() {}\n").unwrap();

    // Wide scope: first ingest — both must be New.
    let wide_scope = SourceScope {
        root: env.workspace_root.clone(),
        include: vec!["**/*.rs".to_string()],
        exclude: env.config.workspace.exclude.clone(),
    };
    let first =
        ingest_with_config_opts(env.config.clone(), wide_scope, false, IngestOpts::default())
            .expect("first ingest (wide) must succeed");
    assert!(first.new >= 2, "expected at least 2 new docs: {first:?}");
    assert_eq!(
        first.purged_deleted_files, 0,
        "no purges on first ingest: {first:?}"
    );

    // Narrow scope: only a_narrow.rs in include — b_narrow.rs is still
    // on disk but excluded from the walker scope.
    let narrow_scope = SourceScope {
        root: env.workspace_root.clone(),
        include: vec!["a_narrow.rs".to_string()],
        exclude: env.config.workspace.exclude.clone(),
    };
    let second = ingest_with_config_opts(
        env.config.clone(),
        narrow_scope,
        false,
        IngestOpts::default(),
    )
    .expect("second ingest (narrow) must succeed");

    // CRITICAL: b_narrow.rs is still on disk — must NOT be purged.
    assert_eq!(
        second.purged_deleted_files, 0,
        "scope-narrowing must NOT purge on-disk files; got: {second:?}"
    );
    assert_eq!(second.errors, 0, "no errors: {second:?}");

    // b_narrow.rs must still exist in the store.
    let doc_paths = list_doc_paths(&env);
    let b_ws_path = "b_narrow.rs";
    assert!(
        doc_paths.iter().any(|p| p == b_ws_path),
        "b_narrow.rs must still be in list_docs after scope narrowing; got: {doc_paths:?}"
    );
    // And the file must still be on disk.
    assert!(
        b_path.exists(),
        "b_narrow.rs must still be on disk (we didn't delete it)"
    );
}
