//! Snapshot tests pinning the §0 Q9 derive output for two fixtures.
//!
//! The baseline JSON next to each fixture is hand-authored / regenerated
//! from a deterministic run. `BodyHints` timestamps are caller-provided
//! and therefore stable; lingua autodetect over our fixtures is also
//! stable for the language set we configured.

use kebab_parse_md::{BodyHints, parse_frontmatter};
use serde::Serialize;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use time::macros::datetime;

/// Stable view of the parser output suitable for JSON snapshotting.
/// We deliberately exclude `FrontmatterSpan` byte offsets here too — they're
/// fully determined by the input file and are exercised by unit tests; the
/// snapshot focuses on the §0 Q9 derive contract.
#[derive(Serialize)]
struct Snapshot {
    metadata: kebab_core::Metadata,
    span_present: bool,
    warnings: Vec<kebab_parse_types::Warning>,
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
        .join("markdown")
}

fn pinned_hints() -> BodyHints {
    BodyHints {
        first_h1: None,
        fs_ctime: datetime!(2024-01-01 00:00:00 UTC),
        fs_mtime: datetime!(2024-01-02 00:00:00 UTC),
        fallback_lang: None,
    }
}

fn assert_snapshot(fixture: &str, baseline: &str) {
    let dir = fixtures_dir();
    let bytes = fs::read(dir.join(fixture)).expect("fixture readable");

    let (meta, span, warns) = parse_frontmatter(&bytes, &pinned_hints()).unwrap();
    let snap = Snapshot {
        metadata: meta,
        span_present: span.is_some(),
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
fn frontmatter_only_snapshot() {
    assert_snapshot("frontmatter-only.md", "frontmatter-only.snapshot.json");
}

/// Run with `cargo test -p kb-parse-md --test frontmatter_snapshots emit_snapshots -- --ignored --nocapture`
/// to regenerate the baseline JSON files from the current parser output.
#[test]
#[ignore]
fn emit_snapshots() {
    let dir = fixtures_dir();
    for (fixture, baseline) in [
        ("frontmatter-only.md", "frontmatter-only.snapshot.json"),
        ("mixed-lang.md", "mixed-lang.snapshot.json"),
    ] {
        let bytes = fs::read(dir.join(fixture)).unwrap();
        let (meta, span, warns) = parse_frontmatter(&bytes, &pinned_hints()).unwrap();
        let snap = Snapshot {
            metadata: meta,
            span_present: span.is_some(),
            warnings: warns,
        };
        let json = serde_json::to_string_pretty(&snap).unwrap();
        fs::write(dir.join(baseline), format!("{json}\n")).unwrap();
        eprintln!("wrote {}", dir.join(baseline).display());
    }
}

#[test]
fn mixed_lang_snapshot() {
    assert_snapshot("mixed-lang.md", "mixed-lang.snapshot.json");
}

/// Determinism: parsing the same fixture twice in a row must give equal output.
#[test]
fn snapshot_is_deterministic_across_runs() {
    let dir = fixtures_dir();
    let bytes = fs::read(dir.join("frontmatter-only.md")).unwrap();
    let (a, _, _) = parse_frontmatter(&bytes, &pinned_hints()).unwrap();
    let (b, _, _) = parse_frontmatter(&bytes, &pinned_hints()).unwrap();
    assert_eq!(serde_json::to_value(&a).unwrap(), serde_json::to_value(&b).unwrap());
}
