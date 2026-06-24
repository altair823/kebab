//! Regression test for the twin-file idempotency bug.
//!
//! Identical-content files at different workspace paths share one
//! `assets` row (`asset_id` = blake3 content hash, PRIMARY KEY). The
//! old UPSERT `ON CONFLICT(asset_id) DO UPDATE SET workspace_path =
//! excluded.workspace_path` made each twin overwrite the other's path
//! on every ingest, so `get_asset_by_workspace_path(path1)` returned
//! None (or the wrong twin) → re-process every time.
//!
//! Fix: `try_skip_unchanged` now uses `get_document_by_workspace_path`
//! instead.  `documents.workspace_path` is UNIQUE (V001) so each twin
//! has its own stable document row.
//!
//! Assertion contract:
//!   1st ingest → 2 New (one per twin)
//!   2nd ingest → 0 New, 0 Updated, 2 Unchanged

mod common;

use common::TestEnv;
use kebab_app::ingest_with_config;
use kebab_core::IngestItemKind;

#[test]
fn twin_files_second_ingest_is_unchanged() {
    let env = TestEnv::lexical_only();

    // Write two files with identical content at different paths.
    let pkg_a = env.workspace_root.join("pkg_a");
    let pkg_b = env.workspace_root.join("pkg_b");
    std::fs::create_dir_all(&pkg_a).unwrap();
    std::fs::create_dir_all(&pkg_b).unwrap();

    let content = b"# shared\nThis content is identical in both files.\n";
    std::fs::write(pkg_a.join("__init__.py"), content).unwrap();
    std::fs::write(pkg_b.join("__init__.py"), content).unwrap();

    // First ingest — both files must be New.
    let first = ingest_with_config(env.config.clone(), env.scope(), kebab_app::IngestOpts::default())
        .expect("first ingest must succeed");
    assert_eq!(first.errors, 0, "first ingest: no errors; report={first:?}");

    let items = first.items.as_ref().expect("items must be present");
    let twin_items: Vec<_> = items
        .iter()
        .filter(|i| i.doc_path.0.ends_with("__init__.py"))
        .collect();
    assert_eq!(
        twin_items.len(),
        2,
        "first ingest: expected exactly 2 __init__.py items; items={items:?}"
    );
    for item in &twin_items {
        assert_eq!(
            item.kind,
            IngestItemKind::New,
            "first ingest: each twin must be New; item={item:?}"
        );
    }

    // Second ingest — same files, same content → both must be Unchanged.
    let second = ingest_with_config(env.config.clone(), env.scope(), kebab_app::IngestOpts::default())
        .expect("second ingest must succeed");
    assert_eq!(
        second.errors, 0,
        "second ingest: no errors; report={second:?}"
    );
    assert_eq!(
        second.new, 0,
        "second ingest: no new docs; report={second:?}"
    );
    assert_eq!(
        second.updated, 0,
        "second ingest: no updated docs (twin-file bug would set this to 2); report={second:?}"
    );

    let second_items = second.items.as_ref().expect("items must be present");
    let twin_items2: Vec<_> = second_items
        .iter()
        .filter(|i| i.doc_path.0.ends_with("__init__.py"))
        .collect();
    assert_eq!(
        twin_items2.len(),
        2,
        "second ingest: expected exactly 2 __init__.py items; items={second_items:?}"
    );
    for item in &twin_items2 {
        assert_eq!(
            item.kind,
            IngestItemKind::Unchanged,
            "second ingest: each twin must be Unchanged; item={item:?}"
        );
    }
}
