//! Regression test for the twin-file fetch_span media-type lookup bug.
//!
//! Twin files (identical content at different workspace paths) share one
//! `assets` row whose PRIMARY KEY is the blake3 content hash. The old
//! `fetch_span` implementation called
//! `get_asset_by_workspace_path(&doc.workspace_path)` to check whether the
//! media type was PDF/audio (and therefore reject span fetch). For a twin
//! file that lookup could silently return the *other* twin's asset row if
//! `assets.workspace_path` had been overwritten on the most recent ingest of
//! the sibling — making the media-type branch decision incorrect.
//!
//! Fix: `fetch_span` now uses the 2-step lookup
//!   `get_document_by_workspace_path` → `doc.source_asset_id` → `get_asset`
//! so the result is always anchored to the requesting document, not
//! whichever twin last updated `assets.workspace_path`.
//!
//! This test builds a twin-file scenario (two .md files at different paths
//! with identical content), ingests both, then calls `fetch_span` on each
//! twin's `doc_id` and asserts it succeeds. Before the fix, if the asset
//! row's workspace_path happened to point at the wrong twin the span could
//! return an incorrect `span_not_supported` for a non-PDF/audio file, or
//! conversely allow span on a PDF twin by accident. After the fix, the
//! lookup is always doc-specific.

mod common;

use common::TestEnv;
use kebab_app::ingest_with_config;
use kebab_core::{DocumentStore, FetchKind, FetchOpts, FetchQuery, IngestItemKind};

#[test]
fn twin_files_fetch_span_uses_correct_asset() {
    let env = TestEnv::lexical_only();

    // Write two markdown files with identical content at different paths.
    let dir_a = env.workspace_root.join("src_a");
    let dir_b = env.workspace_root.join("src_b");
    std::fs::create_dir_all(&dir_a).unwrap();
    std::fs::create_dir_all(&dir_b).unwrap();

    // The content must produce at least 1 line so span fetch is non-trivial.
    let content = "# Twin\n\nLine one.\n\nLine two.\n\nLine three.\n";
    std::fs::write(dir_a.join("note.md"), content).unwrap();
    std::fs::write(dir_b.join("note.md"), content).unwrap();

    // Ingest all files (fixture workspace + our two new twins).
    let report = ingest_with_config(env.config.clone(), env.scope(), false)
        .expect("ingest must succeed");
    assert_eq!(report.errors, 0, "no ingest errors; report={report:?}");

    // Both twin paths must appear as New in the report.
    let items = report.items.as_ref().expect("items must be present");
    let twin_items: Vec<_> = items
        .iter()
        .filter(|i| {
            i.doc_path.0.ends_with("src_a/note.md")
                || i.doc_path.0.ends_with("src_b/note.md")
        })
        .collect();
    assert_eq!(
        twin_items.len(),
        2,
        "exactly 2 twin items expected; items={items:?}"
    );
    for item in &twin_items {
        assert_eq!(
            item.kind,
            IngestItemKind::New,
            "each twin must be New; item={item:?}"
        );
    }

    // Resolve doc_ids for both workspace paths.
    // The ingest layer normalises workspace_path to the path relative to
    // workspace_root (e.g. "src_a/note.md"), so we look up by that form.
    let store = kebab_store_sqlite::SqliteStore::open(&env.config).unwrap();
    store.run_migrations().unwrap();

    // Find the twin items by matching on suffix so the test is robust to
    // however the workspace root is represented.
    let items = report.items.as_ref().expect("items must be present");
    let path_a_str = items
        .iter()
        .find(|i| i.doc_path.0.ends_with("src_a/note.md"))
        .map(|i| i.doc_path.0.clone())
        .expect("src_a/note.md must appear in ingest report");
    let path_b_str = items
        .iter()
        .find(|i| i.doc_path.0.ends_with("src_b/note.md"))
        .map(|i| i.doc_path.0.clone())
        .expect("src_b/note.md must appear in ingest report");

    let path_a = kebab_core::WorkspacePath(path_a_str);
    let path_b = kebab_core::WorkspacePath(path_b_str);

    let doc_a = store
        .get_document_by_workspace_path(&path_a)
        .expect("get_document_by_workspace_path path_a")
        .expect("doc_a must exist after ingest");
    let doc_b = store
        .get_document_by_workspace_path(&path_b)
        .expect("get_document_by_workspace_path path_b")
        .expect("doc_b must exist after ingest");

    // Both twins share one asset_id (same content hash).
    assert_eq!(
        doc_a.source_asset_id, doc_b.source_asset_id,
        "twin files must share one asset_id"
    );

    // Open App and issue span fetch on each twin's doc_id.
    let app = env.app();

    let result_a = app
        .fetch(
            FetchQuery::Span {
                doc_id: doc_a.doc_id.clone(),
                line_start: 1,
                line_end: 2,
            },
            FetchOpts::default(),
        )
        .expect("fetch_span on twin A must succeed for a markdown file");
    assert_eq!(result_a.kind, FetchKind::Span);
    assert!(
        result_a.text.as_deref().is_some_and(|t| !t.is_empty()),
        "span text for twin A must not be empty"
    );

    let result_b = app
        .fetch(
            FetchQuery::Span {
                doc_id: doc_b.doc_id.clone(),
                line_start: 1,
                line_end: 2,
            },
            FetchOpts::default(),
        )
        .expect("fetch_span on twin B must succeed for a markdown file");
    assert_eq!(result_b.kind, FetchKind::Span);
    assert!(
        result_b.text.as_deref().is_some_and(|t| !t.is_empty()),
        "span text for twin B must not be empty"
    );

    // Ingest again to force the asset.workspace_path flip-flop, then
    // re-check. Pre-fix this was the scenario that triggered the bug:
    // after the second ingest the asset row's workspace_path could point
    // at either twin, making one twin's span fetch behave incorrectly.
    let report2 = ingest_with_config(env.config.clone(), env.scope(), false)
        .expect("second ingest must succeed");
    assert_eq!(report2.errors, 0, "no ingest errors on second run; report={report2:?}");

    // Re-open app after second ingest and verify span still works on both.
    let app2 = env.app();

    app2.fetch(
        FetchQuery::Span {
            doc_id: doc_a.doc_id.clone(),
            line_start: 1,
            line_end: 3,
        },
        FetchOpts::default(),
    )
    .expect("fetch_span on twin A after flip-flop must still succeed");

    app2.fetch(
        FetchQuery::Span {
            doc_id: doc_b.doc_id.clone(),
            line_start: 1,
            line_end: 3,
        },
        FetchOpts::default(),
    )
    .expect("fetch_span on twin B after flip-flop must still succeed");
}
