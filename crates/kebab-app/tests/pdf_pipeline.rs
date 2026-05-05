//! P7-3 PDF ingest wiring — end-to-end integration.
//!
//! Each test spins up a `TempDir` workspace + writes one or more PDF
//! fixtures via the same `lopdf` builder pattern
//! `kebab-parse-pdf::tests::common` uses, then runs `kebab_app::
//! ingest_with_config` against it. PDF ingest needs no external HTTP
//! adapter (no OCR / caption / LM), so unlike the image pipeline these
//! tests do NOT need wiremock — they run sync, no async runtime.

mod common;

use std::path::Path;

use common::TestEnv;
use kebab_config::Config;
use kebab_core::{Block, IngestItemKind, SourceSpan};
use lopdf::content::{Content, Operation};
use lopdf::{Document, Object, Stream, dictionary};

// ── Fixture helpers ──────────────────────────────────────────────────────

/// Build a Helvetica-text PDF mirroring `kebab-parse-pdf::tests::common::
/// build_text_pdf`. `pages` is one entry per page; `None` means the page
/// has no `/Contents` stream (the "scanned candidate" shape — extract
/// returns empty + emits a Provenance Warning).
fn build_text_pdf(pages: &[Option<&str>]) -> Vec<u8> {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let font_id = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
    });
    let resources_id = doc.add_object(dictionary! {
        "Font" => dictionary! { "F1" => font_id },
    });

    let mut page_refs: Vec<Object> = Vec::new();
    for page in pages {
        let mut page_dict = dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
        };
        if let Some(text) = page {
            let content = Content {
                operations: vec![
                    Operation::new("BT", vec![]),
                    Operation::new("Tf", vec!["F1".into(), 24.into()]),
                    Operation::new(
                        "Td",
                        vec![Object::Integer(100), Object::Integer(700)],
                    ),
                    Operation::new("Tj", vec![Object::string_literal(*text)]),
                    Operation::new("ET", vec![]),
                ],
            };
            let stream_data = content.encode().expect("content encode");
            let content_id =
                doc.add_object(Stream::new(dictionary! {}, stream_data));
            page_dict.set("Contents", content_id);
        }
        let page_id = doc.add_object(page_dict);
        page_refs.push(page_id.into());
    }

    let count = page_refs.len() as i64;
    let pages_dict = dictionary! {
        "Type" => "Pages",
        "Kids" => page_refs,
        "Count" => count,
        "Resources" => resources_id,
        "MediaBox" => vec![
            Object::Integer(0),
            Object::Integer(0),
            Object::Integer(595),
            Object::Integer(842),
        ],
    };
    doc.objects
        .insert(pages_id, Object::Dictionary(pages_dict));

    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    doc.trailer.set("Root", catalog_id);

    let mut out: Vec<u8> = Vec::new();
    doc.save_to(&mut out).expect("save PDF to memory");
    out
}

/// Wrap any valid PDF byte buffer with a fake `/Encrypt` trailer entry
/// so `Document::is_encrypted()` flips to true. Mirrors
/// `kebab-parse-pdf::tests::common::make_encrypted_pdf`.
fn make_encrypted_pdf() -> Vec<u8> {
    let bytes = build_text_pdf(&[Some("placeholder")]);
    let mut doc = Document::load_mem(&bytes).expect("load round-tripped PDF");
    let enc_id = doc.add_object(dictionary! {
        "Filter" => "Standard",
        "V" => 1,
        "R" => 2,
        "Length" => 40,
        "P" => -4,
    });
    doc.trailer.set("Encrypt", enc_id);
    let mut out = Vec::new();
    doc.save_to(&mut out).expect("save encrypted PDF");
    out
}

fn corrupt_pdf() -> Vec<u8> {
    b"NOT A PDF; just plain bytes".to_vec()
}

fn write_pdf(root: &Path, name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let path = root.join(name);
    std::fs::write(&path, bytes).expect("write PDF fixture");
    path
}

fn cfg_with_pdf(env: &TestEnv) -> Config {
    let mut cfg = env.config.clone();
    // p9-fb-25: workspace.include removed; extension routing is now
    // handled by extractor matching alone (no config knob).
    // PDF ingest does not need OCR / caption / LM — leave defaults
    // (ocr.enabled=false, caption.enabled=false). The image pipeline
    // construction step skips both adapters.
    cfg.image.ocr.enabled = false;
    cfg.image.caption.enabled = false;
    cfg
}

// ── Tests ────────────────────────────────────────────────────────────────

/// 3-page text PDF → 1 doc + 3 chunks, each chunk's `source_spans[0]`
/// is `Page { page: i, .. }`.
#[test]
fn ingest_3_page_pdf_produces_one_doc_and_per_page_chunks() {
    let env = TestEnv::lexical_only();
    let bytes = build_text_pdf(&[
        Some("Hello page 1 body."),
        Some("Hello page 2 body."),
        Some("Hello page 3 body."),
    ]);
    write_pdf(&env.workspace_root, "three.pdf", &bytes);
    let cfg = cfg_with_pdf(&env);

    let report =
        kebab_app::ingest_with_config(cfg.clone(), env.scope(), false)
            .expect("PDF ingest must succeed");

    assert_eq!(report.errors, 0);
    let items = report.items.as_ref().expect("items present");
    let pdf_item = items
        .iter()
        .find(|i| i.doc_path.0.ends_with("three.pdf"))
        .expect("PDF item present");
    assert_eq!(pdf_item.kind, IngestItemKind::New);
    assert_eq!(pdf_item.block_count, Some(3), "one Block::Paragraph per page");
    assert_eq!(pdf_item.chunk_count, Some(3), "one chunk per non-empty page");
    assert_eq!(
        pdf_item.parser_version.as_ref().map(|p| p.0.as_str()),
        Some("pdf-text-v1")
    );
    assert_eq!(
        pdf_item.chunker_version.as_ref().map(|c| c.0.as_str()),
        Some("pdf-page-v1")
    );

    // Inspect the stored doc to confirm SourceSpan::Page round-trip.
    let doc = kebab_app::inspect_doc_with_config(
        cfg,
        pdf_item.doc_id.as_ref().unwrap(),
    )
    .expect("inspect_doc returns the PDF document");
    assert_eq!(doc.blocks.len(), 3);
    for (i, block) in doc.blocks.iter().enumerate() {
        let want_page = (i as u32) + 1;
        let common = match block {
            Block::Paragraph(p) => &p.common,
            other => panic!("expected Paragraph, got {other:?}"),
        };
        match common.source_span {
            SourceSpan::Page { page, .. } => assert_eq!(page, want_page),
            ref other => panic!("expected Page span, got {other:?}"),
        }
    }
}

/// Re-ingest the SAME PDF bytes → identical doc_id, item kind =
/// Unchanged. p9-fb-23 task 7 introduced the early-skip path: when
/// checksum + parser/chunker/embedding versions all match, the second
/// run reports `Unchanged` rather than `Updated` and skips parse /
/// chunk / embed entirely. The pre-p9-fb-23 contract was `Updated`;
/// the `force_reingest=true` path still exercises that branch (see
/// `incremental_ingest.rs`).
#[test]
fn re_ingest_identical_pdf_produces_unchanged_with_same_doc_id() {
    let env = TestEnv::lexical_only();
    let bytes = build_text_pdf(&[Some("page 1"), Some("page 2")]);
    write_pdf(&env.workspace_root, "stable.pdf", &bytes);
    let cfg = cfg_with_pdf(&env);

    let report1 =
        kebab_app::ingest_with_config(cfg.clone(), env.scope(), false).unwrap();
    let item1 = report1
        .items
        .as_ref()
        .unwrap()
        .iter()
        .find(|i| i.doc_path.0.ends_with("stable.pdf"))
        .cloned()
        .unwrap();
    assert_eq!(item1.kind, IngestItemKind::New);

    let report2 =
        kebab_app::ingest_with_config(cfg.clone(), env.scope(), false).unwrap();
    let item2 = report2
        .items
        .unwrap()
        .into_iter()
        .find(|i| i.doc_path.0.ends_with("stable.pdf"))
        .unwrap();
    assert_eq!(item2.kind, IngestItemKind::Unchanged);
    assert_eq!(item2.doc_id, item1.doc_id);
}

/// Edit a PDF (replace bytes) → different blake3 → different asset_id
/// → different doc_id → `new+=1` for the new doc_id; stale doc /
/// asset / chunk / embedding rows for the prior bytes are swept by
/// `purge_orphan_at_workspace_path` (HOTFIXES 2026-05-02 P7-3 storage
/// fix shipped alongside this test).
#[test]
fn re_ingest_edited_pdf_produces_new_doc_id() {
    let env = TestEnv::lexical_only();
    let path = env.workspace_root.join("evolving.pdf");
    let bytes_v1 = build_text_pdf(&[Some("version one body")]);
    std::fs::write(&path, &bytes_v1).unwrap();
    let cfg = cfg_with_pdf(&env);

    let report_v1 =
        kebab_app::ingest_with_config(cfg.clone(), env.scope(), false).unwrap();
    let id_v1 = report_v1
        .items
        .as_ref()
        .unwrap()
        .iter()
        .find(|i| i.doc_path.0.ends_with("evolving.pdf"))
        .unwrap()
        .doc_id
        .clone()
        .unwrap();

    let bytes_v2 =
        build_text_pdf(&[Some("VERSION TWO entirely different body content.")]);
    std::fs::write(&path, &bytes_v2).unwrap();

    let report_v2 =
        kebab_app::ingest_with_config(cfg.clone(), env.scope(), false).unwrap();
    let item_v2 = report_v2
        .items
        .as_ref()
        .unwrap()
        .iter()
        .find(|i| i.doc_path.0.ends_with("evolving.pdf"))
        .unwrap();
    assert_eq!(
        item_v2.kind,
        IngestItemKind::New,
        "edited PDF gets a new asset_id → new doc_id → counted as New"
    );
    assert_ne!(item_v2.doc_id.as_ref().unwrap().0, id_v1.0);
}

/// Encrypted PDF → asset NOT stored; errors+=1; IngestItem.error
/// preserves the qpdf decrypt hint from kebab-parse-pdf verbatim.
#[test]
fn encrypted_pdf_fails_with_qpdf_hint() {
    let env = TestEnv::lexical_only();
    let bytes = make_encrypted_pdf();
    write_pdf(&env.workspace_root, "secret.pdf", &bytes);
    let cfg = cfg_with_pdf(&env);

    let report =
        kebab_app::ingest_with_config(cfg, env.scope(), false).unwrap();
    assert_eq!(report.errors, 1, "encrypted PDF must increment errors exactly once");
    let items = report.items.as_ref().unwrap();
    let pdf_item = items
        .iter()
        .find(|i| i.doc_path.0.ends_with("secret.pdf"))
        .expect("encrypted PDF item present");
    assert_eq!(pdf_item.kind, IngestItemKind::Error);
    let err = pdf_item.error.as_ref().expect("error field set");
    assert!(
        err.contains("encrypted"),
        "error mentions encryption: {err}"
    );
    assert!(
        err.contains("qpdf") || err.contains("decrypt"),
        "error preserves remediation hint: {err}"
    );
}

/// Corrupt header PDF → asset NOT stored; errors+=1.
#[test]
fn corrupt_pdf_fails_without_storing() {
    let env = TestEnv::lexical_only();
    let bytes = corrupt_pdf();
    write_pdf(&env.workspace_root, "corrupt.pdf", &bytes);
    let cfg = cfg_with_pdf(&env);

    let report =
        kebab_app::ingest_with_config(cfg.clone(), env.scope(), false).unwrap();
    assert_eq!(report.errors, 1, "corrupt PDF must increment errors exactly once");
    let items = report.items.as_ref().unwrap();
    let pdf_item = items
        .iter()
        .find(|i| i.doc_path.0.ends_with("corrupt.pdf"))
        .unwrap();
    assert_eq!(pdf_item.kind, IngestItemKind::Error);

    // Confirm the doc was NOT stored — list_docs returns nothing for
    // this path.
    let summaries = kebab_app::list_docs_with_config(
        cfg,
        kebab_core::DocFilter::default(),
    )
    .unwrap();
    assert!(
        !summaries
            .iter()
            .any(|s| s.doc_path.0.ends_with("corrupt.pdf")),
        "corrupt PDF must not have a stored doc row"
    );
}

/// Mixed page PDF (text page 1, empty page 2, text page 3) → asset
/// stored; 2 chunks (pages 1 + 3); doc.provenance.events contains the
/// page-2 Warning emitted by kebab-parse-pdf.
#[test]
fn mixed_page_pdf_stores_asset_with_scanned_candidate_warning() {
    let env = TestEnv::lexical_only();
    let bytes =
        build_text_pdf(&[Some("first page"), None, Some("third page")]);
    write_pdf(&env.workspace_root, "mixed.pdf", &bytes);
    let cfg = cfg_with_pdf(&env);

    let report =
        kebab_app::ingest_with_config(cfg.clone(), env.scope(), false).unwrap();
    assert_eq!(report.errors, 0, "scanned candidate is a Warning, not Error");
    let pdf_item = report
        .items
        .as_ref()
        .unwrap()
        .iter()
        .find(|i| i.doc_path.0.ends_with("mixed.pdf"))
        .unwrap();
    assert_eq!(pdf_item.kind, IngestItemKind::New);
    assert_eq!(
        pdf_item.block_count,
        Some(3),
        "still 3 blocks (P7-1 emits empty Block::Paragraph for the empty page)"
    );
    assert_eq!(
        pdf_item.chunk_count,
        Some(2),
        "pdf-page-v1 emits 0 chunks for the empty page; total = 2"
    );

    let doc = kebab_app::inspect_doc_with_config(
        cfg,
        pdf_item.doc_id.as_ref().unwrap(),
    )
    .unwrap();
    let warnings: Vec<_> = doc
        .provenance
        .events
        .iter()
        .filter(|e| e.kind == kebab_core::ProvenanceKind::Warning)
        .collect();
    assert_eq!(
        warnings.len(),
        1,
        "exactly one Warning event for the empty page"
    );
    let note = warnings[0].note.as_deref().unwrap_or("");
    assert!(
        note.contains("page2") && note.contains("scanned candidate"),
        "Warning note marks page 2 as scanned candidate: {note}"
    );

    // R1: Warning notes also surface on `IngestItem.warnings` so
    // operators can see the partial-success signal in the ingest
    // summary without `kebab inspect doc`.
    assert_eq!(
        pdf_item.warnings.len(),
        1,
        "exactly one warning surfaced on IngestItem"
    );
    assert!(
        pdf_item.warnings[0].contains("page2")
            && pdf_item.warnings[0].contains("scanned candidate"),
        "IngestItem.warnings preserves the Provenance Warning note: {:?}",
        pdf_item.warnings
    );
}

/// IngestReport invariant `scanned == new + updated + skipped + errors`
/// when ingesting a mixed corpus including a corrupt PDF.
#[test]
fn ingest_report_arithmetic_invariant_holds_with_corrupt_pdf() {
    let env = TestEnv::lexical_only();
    write_pdf(
        &env.workspace_root,
        "good.pdf",
        &build_text_pdf(&[Some("ok body")]),
    );
    write_pdf(&env.workspace_root, "broken.pdf", &corrupt_pdf());
    let cfg = cfg_with_pdf(&env);

    let report =
        kebab_app::ingest_with_config(cfg, env.scope(), false).unwrap();
    let total = report.new + report.updated + report.skipped + report.errors;
    assert_eq!(
        report.scanned, total,
        "invariant: scanned ({}) == new ({}) + updated ({}) + skipped ({}) + errors ({})",
        report.scanned, report.new, report.updated, report.skipped, report.errors
    );
    // Sanity: 1 good (new) + 1 broken (error) = 2 scanned for our PDFs;
    // markdown fixtures already in the workspace add to scanned/new
    // alike, so we only assert the invariant rather than absolute counts.
}

/// 50-page PDF → ≥50 chunks (≥1 per page); ingest completes; storage
/// round-trips. Vector embedding is disabled in the lexical-only env
/// so this exercises the SQLite write path only.
#[test]
fn long_pdf_round_trips_through_lexical_pipeline() {
    let env = TestEnv::lexical_only();
    let pages: Vec<String> = (1..=50)
        .map(|i| format!("Page {i} body — lorem ipsum dolor sit amet."))
        .collect();
    let page_refs: Vec<Option<&str>> =
        pages.iter().map(|s| Some(s.as_str())).collect();
    let bytes = build_text_pdf(&page_refs);
    write_pdf(&env.workspace_root, "long.pdf", &bytes);
    let cfg = cfg_with_pdf(&env);

    let report =
        kebab_app::ingest_with_config(cfg.clone(), env.scope(), false).unwrap();
    assert_eq!(report.errors, 0);
    let pdf_item = report
        .items
        .as_ref()
        .unwrap()
        .iter()
        .find(|i| i.doc_path.0.ends_with("long.pdf"))
        .unwrap();
    assert_eq!(pdf_item.block_count, Some(50));
    assert!(
        pdf_item.chunk_count.unwrap() >= 50,
        "chunk_count={:?} should be ≥50",
        pdf_item.chunk_count
    );

    // Round-trip: list_docs sees the long PDF.
    let summaries =
        kebab_app::list_docs_with_config(cfg, kebab_core::DocFilter::default())
            .unwrap();
    assert!(summaries.iter().any(|s| s.doc_path.0.ends_with("long.pdf")));
}

/// `kebab inspect doc <pdf_doc_id>` returns the PDF CanonicalDocument
/// with per-page Block::Paragraph + SourceSpan::Page intact.
#[test]
fn inspect_doc_surfaces_page_spans() {
    let env = TestEnv::lexical_only();
    let bytes =
        build_text_pdf(&[Some("alpha body"), Some("beta body"), Some("gamma body")]);
    write_pdf(&env.workspace_root, "inspect.pdf", &bytes);
    let cfg = cfg_with_pdf(&env);

    let report =
        kebab_app::ingest_with_config(cfg.clone(), env.scope(), false).unwrap();
    let pdf_item = report
        .items
        .as_ref()
        .unwrap()
        .iter()
        .find(|i| i.doc_path.0.ends_with("inspect.pdf"))
        .unwrap();
    let doc = kebab_app::inspect_doc_with_config(
        cfg,
        pdf_item.doc_id.as_ref().unwrap(),
    )
    .unwrap();
    assert_eq!(doc.parser_version.0, "pdf-text-v1");
    assert_eq!(doc.blocks.len(), 3);
    for block in &doc.blocks {
        match block {
            Block::Paragraph(p) => assert!(matches!(
                p.common.source_span,
                SourceSpan::Page { .. }
            )),
            other => panic!("expected Paragraph, got {other:?}"),
        }
    }
}
