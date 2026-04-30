//! Integration test: a `notes/` symlink whose target points back at the
//! workspace root MUST NOT cause `scan` to loop forever or panic.
//!
//! Layout (built per-test in a tempdir):
//!   root/
//!   ├── alpha.md
//!   ├── notes/  (symlink → root)        ← cycle: root → notes → root → …
//!
//! Expected: `scan` returns in O(seconds), every emitted path is unique,
//! and `alpha.md` appears at least once.
//!
//! The cycle guard lives in `walker::walk_files`; this test exists to
//! prove it catches the realistic shape (cycle through one or more
//! symlinks) end-to-end via the public API.

#![cfg(unix)]

use std::os::unix::fs::symlink;

use kb_config::Config;
use kb_core::{SourceConnector, SourceScope};
use kb_source_fs::FsSourceConnector;

fn cfg_with_root(root: &str) -> Config {
    let mut c = Config::defaults();
    c.workspace.root = root.to_string();
    c.workspace.exclude.clear();
    c
}

#[test]
fn symlink_cycle_does_not_loop_or_crash() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    std::fs::write(root.join("alpha.md"), b"alpha").unwrap();
    // Symlink: root/notes → root  (a → a cycle through the link `notes`).
    symlink(root, root.join("notes")).unwrap();

    let conn = FsSourceConnector::new(&cfg_with_root(root.to_str().unwrap()))
        .expect("connector init");
    let v = conn
        .scan(&SourceScope::default())
        .expect("scan must return, not loop");

    // Determinism check: no duplicate workspace paths.
    let mut seen = std::collections::HashSet::new();
    for asset in &v {
        assert!(
            seen.insert(asset.workspace_path.0.clone()),
            "duplicate workspace_path: {}",
            asset.workspace_path.0
        );
    }
    // The original alpha.md must appear.
    assert!(
        v.iter().any(|a| a.workspace_path.0 == "alpha.md"),
        "expected alpha.md in scan output, got: {:?}",
        v.iter().map(|a| &a.workspace_path.0).collect::<Vec<_>>()
    );
}

#[test]
fn two_step_symlink_cycle_does_not_loop() {
    // root/
    // ├── alpha.md
    // ├── a → b
    // └── b → a
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(root.join("alpha.md"), b"alpha").unwrap();
    symlink(root.join("b"), root.join("a")).unwrap();
    symlink(root.join("a"), root.join("b")).unwrap();

    let conn = FsSourceConnector::new(&cfg_with_root(root.to_str().unwrap()))
        .expect("connector init");
    // Even though a→b→a never resolves to a real directory (broken
    // pseudo-cycle of dangling symlinks), the scan must complete and
    // surface alpha.md.
    let v = conn.scan(&SourceScope::default()).expect("scan must return");
    assert!(v.iter().any(|a| a.workspace_path.0 == "alpha.md"));
}
