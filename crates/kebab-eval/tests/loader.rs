//! Loader tests for the golden-fixture YAML parser (P5-1).
//!
//! These tests exercise pure parsing and duplicate-id detection. The
//! DB-validation tests for the crate-private
//! `load_golden_set_validated` live next to the function in
//! `src/loader.rs` (they need `pub(crate)` visibility, which integration
//! tests can't see).

use std::fs;

use kebab_eval::load_golden_set;
use tempfile::tempdir;

// ── 1. parser accepts well-formed YAML with optional fields ──────────────────

#[test]
fn loads_minimal_well_formed_yaml() {
    let tmp = tempdir().unwrap();
    let yaml_path = tmp.path().join("golden.yaml");
    fs::write(
        &yaml_path,
        "- id: g1\n  query: hello\n- id: g2\n  query: \"another\"\n  lang: en\n  must_contain: [\"foo\"]\n  forbidden: [\"bar\"]\n  difficulty: easy\n",
    )
    .unwrap();

    let qs = load_golden_set(&yaml_path).unwrap();
    assert_eq!(qs.len(), 2);
    assert_eq!(qs[0].id, "g1");
    assert_eq!(qs[0].query, "hello");
    assert!(qs[0].must_contain.is_empty());
    assert!(qs[0].forbidden.is_empty());
    assert!(qs[0].difficulty.is_none());

    assert_eq!(qs[1].id, "g2");
    assert_eq!(qs[1].lang.0, "en");
    assert_eq!(qs[1].must_contain, vec!["foo".to_string()]);
    assert_eq!(qs[1].forbidden, vec!["bar".to_string()]);
    assert_eq!(qs[1].difficulty.as_deref(), Some("easy"));
}

// ── 2. fb-41 multi-hop golden fixture loads + sanity-checks ─────────────────

/// fb-41 baseline + post-merge Δ measurement fixture. The shared
/// loader must accept `fixtures/multi_hop_golden.yaml` and the bucket
/// distribution must stay 5 cross-doc + 5 intra-doc + 5 single-fact
/// negative — curators dropping or re-id'ing a question hit a clear
/// failure mode here before it reaches the runner.
#[test]
fn loads_multi_hop_golden_fixture() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
        .join("multi_hop_golden.yaml");
    let qs = load_golden_set(&path).expect("multi_hop_golden.yaml must parse");

    assert_eq!(qs.len(), 15, "fb-41 fixture must have 15 questions");

    let cross_doc = qs.iter().filter(|q| q.id.starts_with("mh-c-")).count();
    let intra_doc = qs.iter().filter(|q| q.id.starts_with("mh-i-")).count();
    let single = qs.iter().filter(|q| q.id.starts_with("mh-s-")).count();
    assert_eq!(cross_doc, 5, "expected 5 mh-c-* (cross-doc) questions");
    assert_eq!(intra_doc, 5, "expected 5 mh-i-* (intra-doc) questions");
    assert_eq!(
        single, 5,
        "expected 5 mh-s-* (single-fact negative) questions"
    );

    // Every question carries at least one `must_contain` so the
    // rule-based answer-correctness metric (P5-2) has a signal even
    // before `expected_chunk_ids` are filled in.
    for q in &qs {
        assert!(
            !q.must_contain.is_empty(),
            "{}: must_contain is empty — baseline measurement needs a signal",
            q.id
        );
    }
}

// ── 3. duplicate IDs error lists every offender (sorted, deduplicated) ───────

#[test]
fn rejects_duplicate_ids() {
    let tmp = tempdir().unwrap();
    let yaml_path = tmp.path().join("dup.yaml");
    fs::write(
        &yaml_path,
        "- id: g1\n  query: a\n- id: g2\n  query: b\n- id: g1\n  query: c\n- id: g2\n  query: d\n- id: g1\n  query: e\n",
    )
    .unwrap();

    let err = load_golden_set(&yaml_path).unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("duplicate query id"), "msg: {msg}");
    // Both dup IDs should appear, sorted (BTreeSet) and deduplicated.
    assert!(msg.contains("g1"), "msg: {msg}");
    assert!(msg.contains("g2"), "msg: {msg}");
}
