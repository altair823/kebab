//! Integration test: `scope.include` enforces an allow-list.
//!
//! Semantics (gitignore convention):
//!   - `include` is empty Vec → all files pass through (backward-compat).
//!   - `include` is non-empty → only files matching at least one pattern
//!     are accepted. `exclude` rules still apply after include.
//!
//! Layout (built per-test in a TempDir):
//!   root/
//!   ├── a.md
//!   ├── b.py
//!   ├── c.png
//!   └── d.pdf

use std::fs;

use kebab_config::Config;
use kebab_core::{SourceConnector, SourceScope};
use kebab_source_fs::FsSourceConnector;

fn cfg_with_root(root: &str) -> Config {
    let mut c = Config::defaults();
    c.workspace.root = root.to_string();
    c.workspace.exclude.clear();
    // Disable size / generated caps so small test files always pass.
    c.ingest.code.max_file_bytes = u64::MAX;
    c.ingest.code.max_file_lines = u32::MAX;
    c.ingest.code.skip_generated_header = false;
    c
}

fn setup_mixed_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("a.md"), b"md").unwrap();
    fs::write(root.join("b.py"), b"py").unwrap();
    fs::write(root.join("c.png"), b"\x89PNG").unwrap();
    fs::write(root.join("d.pdf"), b"%PDF").unwrap();
    dir
}

/// Empty include → all 4 files pass (backward-compat).
#[test]
fn include_empty_accepts_all_files() {
    let dir = setup_mixed_dir();
    let conn = FsSourceConnector::new(&cfg_with_root(dir.path().to_str().unwrap())).unwrap();
    let scope = SourceScope {
        include: vec![],
        ..SourceScope::default()
    };
    let assets = conn.scan(&scope).unwrap();
    let names: Vec<_> = assets.iter().map(|a| a.workspace_path.0.clone()).collect();
    assert!(names.contains(&"a.md".to_string()), "a.md missing; got: {names:?}");
    assert!(names.contains(&"b.py".to_string()), "b.py missing; got: {names:?}");
    assert!(names.contains(&"c.png".to_string()), "c.png missing; got: {names:?}");
    assert!(names.contains(&"d.pdf".to_string()), "d.pdf missing; got: {names:?}");
    assert_eq!(names.len(), 4, "expected exactly 4 files; got: {names:?}");
}

/// Non-empty include → only md + py come back; png + pdf are excluded.
#[test]
fn include_nonempty_is_allowlist() {
    let dir = setup_mixed_dir();
    let conn = FsSourceConnector::new(&cfg_with_root(dir.path().to_str().unwrap())).unwrap();
    let scope = SourceScope {
        include: vec!["**/*.md".to_string(), "**/*.py".to_string()],
        ..SourceScope::default()
    };
    let assets = conn.scan(&scope).unwrap();
    let names: Vec<_> = assets.iter().map(|a| a.workspace_path.0.clone()).collect();
    assert!(names.contains(&"a.md".to_string()), "a.md should be accepted; got: {names:?}");
    assert!(names.contains(&"b.py".to_string()), "b.py should be accepted; got: {names:?}");
    assert!(
        !names.contains(&"c.png".to_string()),
        "c.png must be rejected by include allowlist; got: {names:?}"
    );
    assert!(
        !names.contains(&"d.pdf".to_string()),
        "d.pdf must be rejected by include allowlist; got: {names:?}"
    );
    assert_eq!(names.len(), 2, "expected exactly 2 files; got: {names:?}");
}

/// include + exclude are ANDed: a file matching include but also matching
/// exclude must be rejected.
#[test]
fn include_and_exclude_are_anded() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("keep.md"), b"keep").unwrap();
    fs::write(root.join("drop.md"), b"drop").unwrap();
    fs::write(root.join("other.py"), b"py").unwrap();

    let conn = FsSourceConnector::new(&cfg_with_root(root.to_str().unwrap())).unwrap();
    let scope = SourceScope {
        include: vec!["**/*.md".to_string()],
        exclude: vec!["drop.md".to_string()],
        ..SourceScope::default()
    };
    let assets = conn.scan(&scope).unwrap();
    let names: Vec<_> = assets.iter().map(|a| a.workspace_path.0.clone()).collect();
    assert!(names.contains(&"keep.md".to_string()), "keep.md should be accepted; got: {names:?}");
    assert!(
        !names.contains(&"drop.md".to_string()),
        "drop.md should be excluded (matched exclude); got: {names:?}"
    );
    assert!(
        !names.contains(&"other.py".to_string()),
        "other.py should be excluded (not in include); got: {names:?}"
    );
}
