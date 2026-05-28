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
//! The cycle guard lives in `walker::walk_files_with_skips`; this test exists to
//! prove it catches the realistic shape (cycle through one or more
//! symlinks) end-to-end via the public API.

#![cfg(unix)]

use std::os::unix::fs::symlink;

use kebab_config::Config;
use kebab_core::{SourceConnector, SourceScope};
use kebab_source_fs::FsSourceConnector;

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

    let conn =
        FsSourceConnector::new(&cfg_with_root(root.to_str().unwrap())).expect("connector init");
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
fn dangling_symlink_pseudo_cycle_does_not_crash() {
    // root/
    // ├── alpha.md
    // ├── a → b   (b does not exist as a real file/dir)
    // └── b → a   (a does not exist as a real file/dir)
    //
    // Both symlinks are dangling — neither resolves to anything. This is
    // NOT a real two-step directory cycle (see
    // `two_step_directory_cycle_visited_set_breaks_loop` for that case);
    // it merely verifies the scan tolerates broken-link pseudo-cycles
    // without crashing or looping.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(root.join("alpha.md"), b"alpha").unwrap();
    symlink(root.join("b"), root.join("a")).unwrap();
    symlink(root.join("a"), root.join("b")).unwrap();

    let conn =
        FsSourceConnector::new(&cfg_with_root(root.to_str().unwrap())).expect("connector init");
    // Even though a→b→a never resolves to a real directory (broken
    // pseudo-cycle of dangling symlinks), the scan must complete and
    // surface alpha.md.
    let v = conn
        .scan(&SourceScope::default())
        .expect("scan must return");
    assert!(v.iter().any(|a| a.workspace_path.0 == "alpha.md"));
}

#[test]
fn two_step_directory_cycle_visited_set_breaks_loop() {
    // Real two-step directory cycle through symlinks:
    //   root/
    //   ├── a/
    //   │   ├── inside_a.md
    //   │   └── loop → ../b   (symlink, target IS a real directory)
    //   └── b/
    //       ├── inside_b.md
    //       └── loop → ../a   (symlink, target IS a real directory)
    //
    // Without the visited-set, walkdir would descend
    //   a → a/loop (=b) → a/loop/loop (=a) → … forever.
    // The canonical-path visited-set in `walker::walk_files_with_skips` must break
    // the loop and yield a finite, deterministic result.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir(root.join("a")).unwrap();
    std::fs::create_dir(root.join("b")).unwrap();
    std::fs::write(root.join("a/inside_a.md"), b"a-content").unwrap();
    std::fs::write(root.join("b/inside_b.md"), b"b-content").unwrap();
    // Use relative targets so the symlink truly points at the sibling
    // directory regardless of where the tempdir lives.
    symlink("../b", root.join("a/loop")).unwrap();
    symlink("../a", root.join("b/loop")).unwrap();

    let conn =
        FsSourceConnector::new(&cfg_with_root(root.to_str().unwrap())).expect("connector init");

    // Run scan twice — both must terminate AND produce identical
    // workspace_path lists (visited-set is deterministic per scan).
    let v1 = conn
        .scan(&SourceScope::default())
        .expect("scan must return");
    let v2 = conn
        .scan(&SourceScope::default())
        .expect("scan must return");

    let names1: Vec<String> = v1.iter().map(|a| a.workspace_path.0.clone()).collect();
    let names2: Vec<String> = v2.iter().map(|a| a.workspace_path.0.clone()).collect();
    assert_eq!(names1, names2, "scan must be deterministic across runs");

    // No duplicate workspace paths (visited-set should suppress
    // re-emission of the same canonical file via the cycle).
    let mut seen = std::collections::HashSet::new();
    for asset in &v1 {
        assert!(
            seen.insert(asset.workspace_path.0.clone()),
            "duplicate workspace_path: {}",
            asset.workspace_path.0
        );
    }

    // Both real files must appear at least once. Their exact relative
    // paths depend on which side of the cycle the walker descended into
    // first; assert by basename to keep the check robust.
    assert!(
        v1.iter()
            .any(|a| a.workspace_path.0.ends_with("inside_a.md")),
        "expected inside_a.md in scan output, got: {names1:?}"
    );
    assert!(
        v1.iter()
            .any(|a| a.workspace_path.0.ends_with("inside_b.md")),
        "expected inside_b.md in scan output, got: {names1:?}"
    );

    // Sanity bound: with two real files and a working cycle guard the
    // output should be tiny. If we ever produce >50 entries the visited
    // set has regressed.
    assert!(
        v1.len() < 50,
        "scan emitted {} assets — cycle guard likely regressed: {:?}",
        v1.len(),
        names1
    );
}
