//! Integration tests for `kebab_parse_pdf::PdfTextExtractor` (P7-1).

mod common;

use kebab_core::{Block, Extractor, ProvenanceKind, SourceSpan};
use kebab_parse_pdf::PdfTextExtractor;
use serde_json::Value;

use crate::common::{
    InfoDict, build_text_pdf, build_text_pdf_with_info, corrupt_pdf, fixture_for,
    make_encrypted_pdf, strip_dynamic_at, utf16be_bom,
};

fn paragraph_blocks(doc: &kebab_core::CanonicalDocument) -> Vec<&kebab_core::TextBlock> {
    doc.blocks
        .iter()
        .map(|b| match b {
            Block::Paragraph(t) => t,
            other => panic!("expected Paragraph, got {other:?}"),
        })
        .collect()
}

#[test]
fn three_page_pdf_emits_one_paragraph_block_per_page() {
    let bytes = build_text_pdf(&[
        Some("Hello page 1"),
        Some("Hello page 2"),
        Some("Hello page 3"),
    ]);
    let fx = fixture_for("docs/three.pdf", &bytes);
    let doc = PdfTextExtractor::new()
        .extract(&fx.ctx(), &bytes)
        .expect("3-page extraction must succeed");

    assert_eq!(doc.title, "three");
    assert_eq!(doc.lang.0, "und");
    assert_eq!(doc.parser_version.0, kebab_parse_pdf::PARSER_VERSION);
    assert_eq!(
        doc.metadata.user["pdf"]["page_count"],
        Value::Number(3.into())
    );

    let blocks = paragraph_blocks(&doc);
    assert_eq!(blocks.len(), 3);
    for (i, b) in blocks.iter().enumerate() {
        let want_page = (i as u32) + 1;
        match b.common.source_span {
            SourceSpan::Page {
                page,
                char_start,
                char_end,
            } => {
                assert_eq!(page, want_page);
                assert_eq!(char_start, Some(0));
                let chars = b.text.chars().count() as u32;
                assert_eq!(char_end, Some(chars));
            }
            ref other => panic!("expected Page span, got {other:?}"),
        }
        assert!(
            b.text.contains(&format!("Hello page {want_page}")),
            "page {want_page} text mismatch: {:?}",
            b.text
        );
    }
}

#[test]
fn empty_page_emits_warning_and_empty_paragraph() {
    let bytes = build_text_pdf(&[Some("page one text"), None, Some("page three text")]);
    let fx = fixture_for("docs/scanned-mixed.pdf", &bytes);
    let doc = PdfTextExtractor::new()
        .extract(&fx.ctx(), &bytes)
        .expect("scanned-mixed extraction must succeed");

    let blocks = paragraph_blocks(&doc);
    assert_eq!(blocks.len(), 3);
    assert!(blocks[1].text.is_empty(), "page 2 should have empty text");
    assert!(
        blocks[1].inlines.is_empty(),
        "page 2 inlines should be empty"
    );
    match blocks[1].common.source_span {
        SourceSpan::Page {
            page,
            char_start,
            char_end,
        } => {
            assert_eq!(page, 2);
            assert_eq!(char_start, Some(0));
            assert_eq!(char_end, Some(0));
        }
        ref other => panic!("expected Page, got {other:?}"),
    }

    let warnings: Vec<_> = doc
        .provenance
        .events
        .iter()
        .filter(|e| e.kind == ProvenanceKind::Warning)
        .collect();
    assert_eq!(warnings.len(), 1, "exactly one warning for the empty page");
    assert!(
        warnings[0]
            .note
            .as_deref()
            .unwrap_or("")
            .contains("page2 empty (scanned candidate)"),
        "warning note must mark page 2 as scanned candidate: {:?}",
        warnings[0].note
    );
}

#[test]
fn encrypted_pdf_returns_helpful_error() {
    let bytes = make_encrypted_pdf();
    let fx = fixture_for("docs/encrypted.pdf", &bytes);
    let err = PdfTextExtractor::new()
        .extract(&fx.ctx(), &bytes)
        .expect_err("encrypted PDF must be refused");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("encrypted"),
        "error must mention encryption: {msg}"
    );
    assert!(
        msg.contains("qpdf") || msg.contains("decrypt"),
        "error should point at remediation: {msg}"
    );
}

#[test]
fn corrupt_header_returns_error() {
    let bytes = corrupt_pdf();
    let fx = fixture_for("docs/corrupt.pdf", &bytes);
    let err = PdfTextExtractor::new()
        .extract(&fx.ctx(), &bytes)
        .expect_err("corrupt PDF must error");
    let msg = format!("{err:#}");
    assert!(
        msg.to_lowercase().contains("pdf") || msg.contains("parse"),
        "error must mention PDF parse failure: {msg}"
    );
}

#[test]
fn page_count_matches_actual_count() {
    let bytes = build_text_pdf(&[Some("a"), Some("b"), Some("c"), Some("d"), Some("e")]);
    let fx = fixture_for("docs/five.pdf", &bytes);
    let doc = PdfTextExtractor::new()
        .extract(&fx.ctx(), &bytes)
        .expect("5-page extraction must succeed");

    assert_eq!(
        doc.metadata.user["pdf"]["page_count"],
        Value::Number(5.into())
    );
    assert_eq!(doc.blocks.len(), 5);
}

#[test]
fn info_dict_title_utf16be_bom_decoded() {
    // Korean Title encoded as UTF-16BE with BOM is the standard PDF
    // path for any non-ASCII metadata. We don't try to decode the
    // body text in non-Latin scripts here (CID font support is out
    // of scope for v1) — but the metadata path is in scope.
    let info = InfoDict {
        title: Some(utf16be_bom("케밥 문서")),
        producer: Some("kebab-test"),
        creator: None,
    };
    let bytes = build_text_pdf_with_info(&[Some("body")], &info);
    let fx = fixture_for("docs/korean-title.pdf", &bytes);
    let doc = PdfTextExtractor::new()
        .extract(&fx.ctx(), &bytes)
        .expect("PDF with UTF-16BE Title must extract");

    assert_eq!(doc.title, "케밥 문서");
    assert_eq!(
        doc.metadata.user["pdf"]["producer"],
        Value::String("kebab-test".into())
    );
}

#[test]
fn info_dict_title_utf16be_surrogate_pair_decoded() {
    // 🥙 (U+1F959 STUFFED FLATBREAD) sits in the supplementary plane,
    // so encoding it as UTF-16BE produces a surrogate pair (D83E DD59).
    // BMP-only inputs would never exercise the pair-joining path of
    // `String::from_utf16_lossy` — this asserts that path round-trips.
    let info = InfoDict {
        title: Some(utf16be_bom("케밥 🥙 문서")),
        producer: None,
        creator: None,
    };
    let bytes = build_text_pdf_with_info(&[Some("body")], &info);
    let fx = fixture_for("docs/emoji-title.pdf", &bytes);
    let doc = PdfTextExtractor::new()
        .extract(&fx.ctx(), &bytes)
        .expect("PDF with surrogate-pair Title must extract");
    assert_eq!(doc.title, "케밥 🥙 문서");
}

#[test]
fn info_dict_title_pdfdocencoding_latin1_high_bytes_decoded() {
    // BOM-less PDFDocEncoded title with a high-byte char (0xE9 = 'é').
    // `from_utf8_lossy` would have replaced this with U+FFFD; the
    // byte-as-char path keeps it intact.
    let info = InfoDict {
        title: Some(b"Caf\xE9".to_vec()),
        producer: None,
        creator: None,
    };
    let bytes = build_text_pdf_with_info(&[Some("body")], &info);
    let fx = fixture_for("docs/cafe-title.pdf", &bytes);
    let doc = PdfTextExtractor::new()
        .extract(&fx.ctx(), &bytes)
        .expect("PDF with Latin-1 Title must extract");
    assert_eq!(doc.title, "Café");
}

#[test]
fn info_dict_title_falls_back_to_filename_when_missing() {
    let bytes = build_text_pdf(&[Some("body")]);
    let fx = fixture_for("docs/no-info.pdf", &bytes);
    let doc = PdfTextExtractor::new()
        .extract(&fx.ctx(), &bytes)
        .expect("no-info PDF must extract");
    assert_eq!(doc.title, "no-info");
}

#[test]
fn determinism_identical_bytes_produce_identical_documents() {
    let bytes = build_text_pdf(&[Some("alpha"), Some("beta"), Some("gamma")]);
    let fx = fixture_for("docs/det.pdf", &bytes);

    let mut a = serde_json::to_value(
        PdfTextExtractor::new()
            .extract(&fx.ctx(), &bytes)
            .expect("first extract"),
    )
    .unwrap();
    let mut b = serde_json::to_value(
        PdfTextExtractor::new()
            .extract(&fx.ctx(), &bytes)
            .expect("second extract"),
    )
    .unwrap();

    strip_dynamic_at(&mut a);
    strip_dynamic_at(&mut b);
    assert_eq!(a, b, "two extracts of identical bytes must be byte-equal");
}

#[test]
fn snapshot_three_page_canonical_document_stable() {
    let bytes = build_text_pdf(&[Some("p1"), Some("p2"), Some("p3")]);
    let fx = fixture_for("docs/snapshot.pdf", &bytes);
    let doc = PdfTextExtractor::new()
        .extract(&fx.ctx(), &bytes)
        .expect("snapshot extract");
    let mut json = serde_json::to_value(&doc).unwrap();
    strip_dynamic_at(&mut json);

    // Spot-check the load-bearing shape rather than committing a full
    // golden file (the full JSON contains BLAKE3 ids that would
    // change if `id_from(...)`'s tuple shape ever shifts — that would
    // be a separate, intentional break).
    assert_eq!(json["parser_version"], Value::String("pdf-text-v1".into()));
    assert_eq!(json["lang"], Value::String("und".into()));
    assert_eq!(json["schema_version"], Value::Number(1.into()));
    assert_eq!(json["doc_version"], Value::Number(1.into()));
    assert_eq!(json["blocks"].as_array().unwrap().len(), 3);
    for (i, block) in json["blocks"].as_array().unwrap().iter().enumerate() {
        assert_eq!(block["kind"], Value::String("paragraph".into()));
        assert_eq!(
            block["common"]["source_span"]["kind"],
            Value::String("page".into())
        );
        assert_eq!(
            block["common"]["source_span"]["page"],
            Value::Number(((i as u64) + 1).into())
        );
    }
    assert_eq!(
        json["metadata"]["source_type"],
        Value::String("paper".into())
    );
    assert_eq!(
        json["metadata"]["trust_level"],
        Value::String("primary".into())
    );
}
