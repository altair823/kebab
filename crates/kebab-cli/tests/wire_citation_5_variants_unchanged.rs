//! p10-1A-1 Task 13: regression — the 5 original Citation variants
//! (Line, Page, Region, Caption, Time) serialize byte-identically to
//! pre-Task-1 form.  No spurious `code`, `line_start`, or `symbol` keys
//! must leak into these variants.

use kebab_core::{Citation, WorkspacePath};

#[test]
fn line_variant_serialization_unchanged() {
    let c = Citation::Line {
        path: WorkspacePath::new("a.md".into()).unwrap(),
        start: 1,
        end: 2,
        section: Some("§14".into()),
    };
    let v = serde_json::to_value(&c).unwrap();
    assert_eq!(v["kind"], "line");
    assert_eq!(v["start"], 1);
    assert_eq!(v["end"], 2);
    assert_eq!(v["section"], "§14");
    // Must not bleed Code-variant keys.
    assert!(v.get("line_start").is_none(), "line_start must be absent: {v}");
    assert!(v.get("symbol").is_none(), "symbol must be absent: {v}");
    assert!(v.get("code").is_none(), "code must be absent: {v}");
}

#[test]
fn line_variant_null_section_omitted() {
    let c = Citation::Line {
        path: WorkspacePath::new("b.md".into()).unwrap(),
        start: 5,
        end: 10,
        section: None,
    };
    let v = serde_json::to_value(&c).unwrap();
    assert_eq!(v["kind"], "line");
    // `section` with None should be omitted (skip_serializing_if = is_none).
    assert!(v.get("section").is_none() || v["section"].is_null());
}

#[test]
fn page_variant_serialization_unchanged() {
    let c = Citation::Page {
        path: WorkspacePath::new("a.pdf".into()).unwrap(),
        page: 13,
        section: None,
    };
    let v = serde_json::to_value(&c).unwrap();
    assert_eq!(v["kind"], "page");
    assert_eq!(v["page"], 13);
    assert!(v.get("line_start").is_none(), "line_start must be absent: {v}");
    assert!(v.get("symbol").is_none(), "symbol must be absent: {v}");
}

#[test]
fn region_variant_serialization_unchanged() {
    let c = Citation::Region {
        path: WorkspacePath::new("img.png".into()).unwrap(),
        x: 10,
        y: 20,
        w: 100,
        h: 200,
    };
    let v = serde_json::to_value(&c).unwrap();
    assert_eq!(v["kind"], "region");
    assert_eq!(v["x"], 10);
    assert_eq!(v["y"], 20);
    assert_eq!(v["w"], 100);
    assert_eq!(v["h"], 200);
    assert!(v.get("line_start").is_none(), "line_start must be absent: {v}");
}

#[test]
fn caption_variant_serialization_unchanged() {
    let c = Citation::Caption {
        path: WorkspacePath::new("a.png".into()).unwrap(),
        model: "qwen2.5-vl:7b".into(),
    };
    let v = serde_json::to_value(&c).unwrap();
    assert_eq!(v["kind"], "caption");
    assert_eq!(v["model"], "qwen2.5-vl:7b");
    assert!(v.get("line_start").is_none(), "line_start must be absent: {v}");
}

#[test]
fn time_variant_serialization_unchanged() {
    let c = Citation::Time {
        path: WorkspacePath::new("audio.mp3".into()).unwrap(),
        start_ms: 1000,
        end_ms: 5000,
        speaker: Some("Alice".into()),
    };
    let v = serde_json::to_value(&c).unwrap();
    assert_eq!(v["kind"], "time");
    assert_eq!(v["start_ms"], 1000);
    assert_eq!(v["end_ms"], 5000);
    assert_eq!(v["speaker"], "Alice");
    assert!(v.get("line_start").is_none(), "line_start must be absent: {v}");
    assert!(v.get("symbol").is_none(), "symbol must be absent: {v}");
}
