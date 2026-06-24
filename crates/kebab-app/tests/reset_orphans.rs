//! Integration test for `kebab reset --orphans-only`.
//!
//! Verifies that stored docs outside the current walker scope are purged
//! from the store without removing any files from the filesystem.
//!
//! Test outline:
//! 1. Ingest 3 .rs files (a.rs, b.rs, c.rs) — all New.
//! 2. Narrow the config `include` to `["a.rs"]` only; b.rs and c.rs are
//!    still on disk but outside the walker scope.
//! 3. Run `execute(ResetScope::OrphansOnly, &cfg)` — report must show
//!    `orphans_purged == 2` and `purged_paths` contains b.rs + c.rs.
//! 4. `list docs` must show only a.rs.
//! 5. b.rs and c.rs must still exist on disk (no filesystem removal).
//! 6. Second reset → `orphans_purged == 0` (idempotent).

mod common;

use common::TestEnv;
use kebab_app::{IngestOpts, ingest_with_config};
use kebab_app::reset::{ResetScope, execute};
use kebab_core::{DocFilter, DocumentStore, SourceScope};

/// Open the SqliteStore and list all `workspace_path` values.
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
fn reset_orphans_only_purges_out_of_scope_docs() {
    let env = TestEnv::lexical_only();

    // Write three .rs files into the workspace.
    let a_path = env.workspace_root.join("a.rs");
    let b_path = env.workspace_root.join("b.rs");
    let c_path = env.workspace_root.join("c.rs");
    std::fs::write(&a_path, "// file a\nfn alpha() {}\n").unwrap();
    std::fs::write(&b_path, "// file b\nfn bravo() {}\n").unwrap();
    std::fs::write(&c_path, "// file c\nfn charlie() {}\n").unwrap();

    // Ingest all three with a wide scope.
    let wide_scope = SourceScope {
        root: env.workspace_root.clone(),
        include: vec!["**/*.rs".to_string()],
        exclude: env.config.workspace.exclude.clone(),
    };
    let first = ingest_with_config(env.config.clone(), wide_scope, IngestOpts::default())
        .expect("first ingest must succeed");
    // The fixture workspace may contain other .rs files — just assert we
    // got at least 3 new docs (our a.rs, b.rs, c.rs).
    assert!(first.new >= 3, "expected at least 3 new docs: {first:?}");
    assert_eq!(first.errors, 0, "no errors on first ingest");

    // Narrow config to include only a.rs; b.rs + c.rs are still on disk.
    let mut narrow_cfg = env.config.clone();
    narrow_cfg.workspace.exclude.clear();
    // Re-point workspace root (already correct) and restrict include via
    // the SourceScope in the connector. The config's `workspace.root` is
    // used by `enumerate_orphans` to build its scope — we keep that
    // pointing at the workspace root. We simulate narrowing by setting a
    // glob that only matches a.rs.
    //
    // NOTE: `kebab_config::WorkspaceCfg` does not have an `include` field
    // (it was removed in p9-fb-25). We narrow the scope via the walker
    // exclude list: exclude b.rs and c.rs explicitly.
    narrow_cfg.workspace.exclude = vec!["b.rs".to_string(), "c.rs".to_string()];

    // Run orphans-only reset.
    let report =
        execute(ResetScope::OrphansOnly, &narrow_cfg).expect("orphans-only reset must succeed");

    assert_eq!(
        report.orphans_purged, 2,
        "expected 2 orphans purged (b.rs + c.rs): {report:?}"
    );

    let mut purged: Vec<String> = report.purged_paths.iter().map(|p| p.0.clone()).collect();
    purged.sort();
    assert_eq!(
        purged,
        vec!["b.rs".to_string(), "c.rs".to_string()],
        "purged_paths must list b.rs and c.rs in sorted order: {purged:?}"
    );

    // list docs must show only a.rs (and any pre-existing fixture files
    // that are not excluded by the narrow config).
    let doc_paths = list_doc_paths(&env);
    // The narrow_cfg excludes b.rs + c.rs — they must no longer be in store.
    assert!(
        !doc_paths.iter().any(|p| p == "b.rs"),
        "b.rs must be gone from store after orphans-only reset; got: {doc_paths:?}"
    );
    assert!(
        !doc_paths.iter().any(|p| p == "c.rs"),
        "c.rs must be gone from store after orphans-only reset; got: {doc_paths:?}"
    );
    assert!(
        doc_paths.iter().any(|p| p == "a.rs"),
        "a.rs must still be in store; got: {doc_paths:?}"
    );

    // Both b.rs and c.rs must still exist on the filesystem — no file
    // removal is performed by orphans-only.
    assert!(
        b_path.exists(),
        "b.rs must still be on disk after orphans-only reset"
    );
    assert!(
        c_path.exists(),
        "c.rs must still be on disk after orphans-only reset"
    );

    // Second reset must be idempotent: nothing left to purge.
    let second = execute(ResetScope::OrphansOnly, &narrow_cfg)
        .expect("second orphans-only reset must succeed");
    assert_eq!(
        second.orphans_purged, 0,
        "second reset must be idempotent (orphans_purged == 0): {second:?}"
    );
    assert!(
        second.purged_paths.is_empty(),
        "second reset purged_paths must be empty: {:?}",
        second.purged_paths
    );
}
