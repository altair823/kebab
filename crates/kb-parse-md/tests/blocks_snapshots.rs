//! Snapshot tests pinning the `parse_blocks` output for two fixtures.
//!
//! Baselines are hand-authored / regenerated via the `--ignored` emitter
//! below. `body_offset_lines = 1` is used for both fixtures (no
//! frontmatter, body starts at file line 1).
//!
//! Following the kb_core::Inline schema migration (struct-variant shape),
//! `ParsedBlock` now serializes directly through serde — no projection
//! shim is required. Inlines surface as structured objects, e.g.
//! `[{"kind":"text","text":"…"},{"kind":"code","code":"…"}]`.

use kb_parse_md::parse_blocks;
use kb_parse_types::{ParsedBlock, Warning};
use serde::Serialize;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

#[derive(Serialize)]
struct Snapshot {
    blocks: Vec<ParsedBlock>,
    warnings: Vec<Warning>,
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
        .join("markdown")
}

fn assert_snapshot(fixture: &str, baseline: &str) {
    let dir = fixtures_dir();
    let bytes = fs::read(dir.join(fixture)).expect("fixture readable");

    let (blocks, warns) = parse_blocks(&bytes, 1).unwrap();
    let snap = Snapshot {
        blocks,
        warnings: warns,
    };
    let actual: Value = serde_json::to_value(&snap).unwrap();

    let expected_text =
        fs::read_to_string(dir.join(baseline)).expect("snapshot baseline readable");
    let expected: Value = serde_json::from_str(&expected_text).expect("baseline parses as json");

    if actual != expected {
        let actual_pretty = serde_json::to_string_pretty(&actual).unwrap();
        panic!(
            "snapshot drift for {fixture}\n\
             --- expected ({baseline}) ---\n{expected_text}\n\
             --- actual ---\n{actual_pretty}\n\
             If the change is intentional, update {baseline}."
        );
    }
}

#[test]
fn nested_headings_blocks_snapshot() {
    assert_snapshot(
        "nested-headings.md",
        "nested-headings.blocks.snapshot.json",
    );
}

#[test]
fn code_and_table_blocks_snapshot() {
    assert_snapshot(
        "code-and-table.md",
        "code-and-table.blocks.snapshot.json",
    );
}

/// Run with `cargo test -p kb-parse-md --test blocks_snapshots emit_blocks_snapshots -- --ignored --nocapture`
/// to regenerate the baseline JSON files from the current parser output.
#[test]
#[ignore]
fn emit_blocks_snapshots() {
    let dir = fixtures_dir();
    for (fixture, baseline) in [
        ("nested-headings.md", "nested-headings.blocks.snapshot.json"),
        ("code-and-table.md", "code-and-table.blocks.snapshot.json"),
    ] {
        let bytes = fs::read(dir.join(fixture)).unwrap();
        let (blocks, warns) = parse_blocks(&bytes, 1).unwrap();
        let snap = Snapshot {
            blocks,
            warnings: warns,
        };
        let json = serde_json::to_string_pretty(&snap).unwrap();
        fs::write(dir.join(baseline), format!("{json}\n")).unwrap();
        eprintln!("wrote {}", dir.join(baseline).display());
    }
}

/// Determinism: parsing the same fixture twice in a row must give equal output.
#[test]
fn snapshot_is_deterministic_across_runs() {
    let dir = fixtures_dir();
    let bytes = fs::read(dir.join("nested-headings.md")).unwrap();
    let (a_blocks, a_warns) = parse_blocks(&bytes, 1).unwrap();
    let (b_blocks, b_warns) = parse_blocks(&bytes, 1).unwrap();
    assert_eq!(a_blocks, b_blocks);
    assert_eq!(a_warns, b_warns);
    assert_eq!(
        serde_json::to_value(&a_blocks).unwrap(),
        serde_json::to_value(&b_blocks).unwrap()
    );
}
