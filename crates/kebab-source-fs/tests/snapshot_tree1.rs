//! Snapshot + determinism tests against `fixtures/source-fs/tree-1`.
//!
//! Layout (committed under `<repo>/fixtures/source-fs/tree-1/`):
//!
//! ```
//! tree-1/
//! ├── README.md
//! ├── notes/
//! │   ├── alpha.md
//! │   └── beta.md
//! ├── ignored/
//! │   └── skip.tmp           # excluded by .kebabignore
//! ├── .kebabignore              # contains: *.tmp
//! └── .DS_Store              # implicitly excluded
//! ```
//!
//! Two assertions:
//!   1. Snapshot stability — `scan` output (with `discovered_at` stripped)
//!      matches the committed baseline JSON byte-for-byte.
//!   2. Determinism — running `scan` twice produces byte-identical JSON
//!      after stripping `discovered_at`.
//!
//! `discovered_at` is wall-clock and intentionally NOT part of the
//! contract: the task spec says strip it before comparison.

use std::path::PathBuf;

use kebab_config::Config;
use kebab_core::{SourceConnector, SourceScope};
use kebab_source_fs::FsSourceConnector;
use serde_json::Value;

/// Repo root, derived from `CARGO_MANIFEST_DIR` (= `crates/kb-source-fs`).
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn fixture_root() -> PathBuf {
    repo_root().join("fixtures/source-fs/tree-1")
}

fn baseline_path() -> PathBuf {
    repo_root().join("fixtures/source-fs/tree-1.snapshot.json")
}

fn cfg_for_fixture(root: &str) -> Config {
    let mut c = Config::defaults();
    c.workspace.root = root.to_string();
    // Clear default excludes (`.git/**`, `node_modules/**`, `.obsidian/**`)
    // so the snapshot is purely a function of the fixture + .kebabignore +
    // baked-in default-excludes.
    c.workspace.exclude.clear();
    c
}

/// Run `scan` against the fixture and return the JSON value with every
/// `discovered_at` field replaced by the literal string "<stripped>".
/// Also strip `source_uri.value` and `stored.path` because they contain
/// absolute paths that vary by checkout location — the snapshot must be
/// portable across machines and CI checkout dirs.
fn scan_and_strip() -> Value {
    let root = fixture_root();
    let cfg = cfg_for_fixture(root.to_str().unwrap());
    let conn = FsSourceConnector::new(&cfg).expect("connector init");
    let assets = conn
        .scan(&SourceScope::default())
        .expect("scan must succeed against committed fixture");

    let mut v = serde_json::to_value(&assets).expect("serialize");
    if let Value::Array(items) = &mut v {
        for item in items {
            if let Value::Object(map) = item {
                map.insert(
                    "discovered_at".to_string(),
                    Value::String("<stripped>".to_string()),
                );
                // source_uri = { kind: "file", value: "<abs>" } — strip value.
                if let Some(Value::Object(s)) = map.get_mut("source_uri") {
                    if s.contains_key("value") {
                        s.insert("value".to_string(), Value::String("<stripped>".to_string()));
                    }
                }
                // stored = { kind: "copied"|"reference", path: "<abs>", ... } — strip path.
                if let Some(Value::Object(s)) = map.get_mut("stored") {
                    if s.contains_key("path") {
                        s.insert("path".to_string(), Value::String("<stripped>".to_string()));
                    }
                }
            }
        }
    }
    v
}

#[test]
fn tree_1_snapshot_matches_baseline() {
    let actual = scan_and_strip();

    // If KEBAB_REGEN_SNAPSHOT is set, (re)write the baseline and exit
    // *before* attempting to read it. This is the only path that may
    // create the file from scratch.
    if std::env::var_os("KEBAB_REGEN_SNAPSHOT").is_some() {
        let pretty = serde_json::to_string_pretty(&actual).unwrap() + "\n";
        std::fs::write(baseline_path(), pretty).expect("write baseline");
        panic!("regenerated baseline; rerun without KEBAB_REGEN_SNAPSHOT to verify");
    }

    let baseline_text = std::fs::read_to_string(baseline_path()).unwrap_or_else(|_| {
        panic!(
            "missing baseline at {} — regenerate via `KEBAB_REGEN_SNAPSHOT=1 cargo test \
             -p kb-source-fs --test snapshot_tree1 -- tree_1_snapshot_matches_baseline`",
            baseline_path().display()
        )
    });
    let expected: Value = serde_json::from_str(&baseline_text).expect("baseline JSON must parse");

    if actual != expected {
        let actual_pretty = serde_json::to_string_pretty(&actual).unwrap();
        let expected_pretty = serde_json::to_string_pretty(&expected).unwrap();
        panic!(
            "snapshot drift.\n--- expected ---\n{expected_pretty}\n--- actual ---\n{actual_pretty}\n"
        );
    }
}

#[test]
fn tree_1_scan_is_deterministic() {
    let v1 = scan_and_strip();
    let v2 = scan_and_strip();
    let s1 = serde_json::to_string(&v1).unwrap();
    let s2 = serde_json::to_string(&v2).unwrap();
    assert_eq!(s1, s2, "two consecutive scans diverged");
}
