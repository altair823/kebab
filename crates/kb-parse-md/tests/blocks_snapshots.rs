//! Snapshot tests pinning the `parse_blocks` output for two fixtures.
//!
//! Baselines are hand-authored / regenerated via the `--ignored` emitter
//! below. `body_offset_lines = 1` is used for both fixtures (no
//! frontmatter, body starts at file line 1).
//!
//! Note on snapshot shape: `kb_core::Inline` carries a `serde(tag = "kind")`
//! enum representation that cannot serialize newtype variants holding a
//! primitive (`Inline::Text(String)` etc.) — that's a serde limitation, not
//! ours, and is fixed up in a later kb-core task. To keep the snapshot
//! human-readable (and stable across that future fix), we project each
//! `ParsedBlock` into a `BlockView` that flattens inline content to plain
//! strings before serialization. This still pins the *contract* that
//! matters for P1-3: heading paths, source spans, payload kinds, payload
//! text content, table headers/rows, and code lang/body.

use kb_core::{Inline, SourceSpan};
use kb_parse_md::parse_blocks;
use kb_parse_types::{ParsedBlock, ParsedPayload, Warning};
use serde::Serialize;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

#[derive(Serialize)]
struct Snapshot {
    blocks: Vec<BlockView>,
    warnings: Vec<Warning>,
}

#[derive(Serialize)]
struct BlockView {
    kind: String,
    heading_path: Vec<String>,
    source_span: SourceSpan,
    payload: PayloadView,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum PayloadView {
    Heading {
        level: u8,
        text: String,
    },
    Paragraph {
        text: String,
        inlines_flat: String,
    },
    List {
        ordered: bool,
        items_flat: Vec<String>,
    },
    Code {
        lang: Option<String>,
        code: String,
    },
    Table {
        headers: Vec<String>,
        rows: Vec<Vec<String>>,
    },
    Quote {
        text: String,
        inlines_flat: String,
    },
    ImageRef {
        src: String,
        alt: String,
    },
    AudioRef {
        src: String,
    },
}

fn flatten_inline(i: &Inline, out: &mut String) {
    match i {
        Inline::Text(s) | Inline::Code(s) => out.push_str(s),
        Inline::Link { text, href } => {
            out.push('[');
            out.push_str(text);
            out.push_str("](");
            out.push_str(href);
            out.push(')');
        }
        Inline::Strong(v) => {
            out.push_str("**");
            for c in v {
                flatten_inline(c, out);
            }
            out.push_str("**");
        }
        Inline::Emph(v) => {
            out.push('*');
            for c in v {
                flatten_inline(c, out);
            }
            out.push('*');
        }
    }
}

fn flatten(inlines: &[Inline]) -> String {
    let mut out = String::new();
    for i in inlines {
        flatten_inline(i, &mut out);
    }
    out
}

fn block_to_view(b: &ParsedBlock) -> BlockView {
    let kind = format!("{:?}", b.kind).to_lowercase();
    let payload = match &b.payload {
        ParsedPayload::Heading { level, text } => PayloadView::Heading {
            level: *level,
            text: text.clone(),
        },
        ParsedPayload::Paragraph { text, inlines } => PayloadView::Paragraph {
            text: text.clone(),
            inlines_flat: flatten(inlines),
        },
        ParsedPayload::List { ordered, items } => PayloadView::List {
            ordered: *ordered,
            items_flat: items.iter().map(|it| flatten(it)).collect(),
        },
        ParsedPayload::Code { lang, code } => PayloadView::Code {
            lang: lang.clone(),
            code: code.clone(),
        },
        ParsedPayload::Table { headers, rows } => PayloadView::Table {
            headers: headers.clone(),
            rows: rows.clone(),
        },
        ParsedPayload::Quote { text, inlines } => PayloadView::Quote {
            text: text.clone(),
            inlines_flat: flatten(inlines),
        },
        ParsedPayload::ImageRef { src, alt } => PayloadView::ImageRef {
            src: src.clone(),
            alt: alt.clone(),
        },
        ParsedPayload::AudioRef { src } => PayloadView::AudioRef { src: src.clone() },
    };
    BlockView {
        kind,
        heading_path: b.heading_path.clone(),
        source_span: b.source_span.clone(),
        payload,
    }
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
        blocks: blocks.iter().map(block_to_view).collect(),
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
            blocks: blocks.iter().map(block_to_view).collect(),
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
    // Compare via the view (which is fully serializable) and via the
    // structural equality on `ParsedBlock` itself (no serde involved).
    assert_eq!(a_blocks, b_blocks);
    assert_eq!(a_warns, b_warns);
    let av: Vec<_> = a_blocks.iter().map(block_to_view).collect();
    let bv: Vec<_> = b_blocks.iter().map(block_to_view).collect();
    assert_eq!(
        serde_json::to_value(&av).unwrap(),
        serde_json::to_value(&bv).unwrap()
    );
}
