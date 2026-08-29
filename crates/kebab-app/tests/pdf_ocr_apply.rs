//! Integration tests for pdf_ocr_apply helper. spec §5.5 MockOcrEngine pattern.

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use common::mock_ocr::MockOcrEngine;
use kebab_app::pdf_ocr_apply::{PdfOcrOpts, apply_ocr_to_pdf_pages};
use kebab_core::{
    AssetStorage, Block, CanonicalDocument, Checksum, ExtractConfig, ExtractContext, Extractor,
    Inline, Lang, MediaType, RawAsset, SourceSpan, SourceUri, WorkspacePath, id_for_asset,
};
use kebab_parse_pdf::PdfTextExtractor;
use time::OffsetDateTime;

// ── Fixture helpers ───────────────────────────────────────────────────────

fn f1_pdf_bytes() -> Vec<u8> {
    std::fs::read("../kebab-parse-pdf/tests/fixtures/scanned_page1.pdf")
        .expect("F1 fixture missing")
}

fn make_raw_asset(path: &str, media_type: MediaType, byte_len: u64) -> RawAsset {
    let fake_hash = "0".repeat(64);
    let asset_id = id_for_asset(&fake_hash);
    RawAsset {
        asset_id,
        source_uri: SourceUri::File(PathBuf::from(path)),
        workspace_path: WorkspacePath::new(path.to_string()).unwrap(),
        media_type,
        byte_len,
        checksum: Checksum(fake_hash.clone()),
        discovered_at: OffsetDateTime::UNIX_EPOCH,
        stored: AssetStorage::Copied {
            path: PathBuf::from(path),
        },
    }
}

/// Build a CanonicalDocument from raw PDF bytes using PdfTextExtractor.
/// F1 (scanned) returns an empty-text Block::Paragraph per page.
fn extract_canonical_from_bytes(bytes: &[u8]) -> CanonicalDocument {
    let asset = make_raw_asset("test.pdf", MediaType::Pdf, bytes.len() as u64);
    let workspace_root = Path::new("/");
    let config = ExtractConfig::default();
    let ctx = ExtractContext {
        asset: &asset,
        workspace_root,
        config: &config,
        source_id: None,
        source_trust: None,
    };
    PdfTextExtractor::new().extract(&ctx, bytes).unwrap()
}

/// F1 bytes → canonical with 1 empty Block::Paragraph for page 1.
fn canonical_with_empty_block() -> CanonicalDocument {
    extract_canonical_from_bytes(&f1_pdf_bytes())
}

/// F1-based canonical with block text replaced by `text` (high valid_ratio, chars≥20).
fn canonical_with_filled_block(text: &str) -> CanonicalDocument {
    let mut canonical = extract_canonical_from_bytes(&f1_pdf_bytes());
    if let Some(Block::Paragraph(tb)) = canonical.blocks.first_mut() {
        let char_count = text.chars().count() as u32;
        tb.text = text.to_string();
        tb.inlines = vec![Inline::Text {
            text: text.to_string(),
        }];
        if let SourceSpan::Page { char_end, .. } = &mut tb.common.source_span {
            *char_end = Some(char_count);
        }
    }
    canonical
}

/// F1-based canonical with block text replaced by PUA codepoints (low valid_ratio).
fn canonical_with_mojibake_block() -> CanonicalDocument {
    let mut canonical = extract_canonical_from_bytes(&f1_pdf_bytes());
    if let Some(Block::Paragraph(tb)) = canonical.blocks.first_mut() {
        let pua = "\u{E000}".repeat(25); // 25 PUA codepoints → valid_ratio ≈ 0
        let char_count = pua.chars().count() as u32;
        tb.text = pua.clone();
        tb.inlines = vec![Inline::Text { text: pua }];
        if let SourceSpan::Page { char_end, .. } = &mut tb.common.source_span {
            *char_end = Some(char_count);
        }
    }
    canonical
}

fn default_opts(enabled: bool) -> PdfOcrOpts {
    PdfOcrOpts {
        enabled,
        always_on: false,
        valid_ratio_threshold: 0.5,
        min_char_count: 20,
        lang_hint: None,
        cancel: None,
        ocr_cache: None,
        ocr_version_key: String::new(),
        // No renderer in these unit tests: they pin the fallback path's
        // behavior, which is what a machine without pdfium gets.
        renderer: None,
        render_dpi: 300,
        max_pixels: 4096,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

// Test 1: F1 + enabled=true → in-place mutate
#[test]
fn f1_input_with_ocr_enabled_replaces_empty_block() {
    let bytes = f1_pdf_bytes();
    let mut canonical = canonical_with_empty_block();
    let engine = MockOcrEngine::single("MOCK_OCR_TEXT", false);
    let opts = PdfOcrOpts {
        enabled: true,
        always_on: false,
        valid_ratio_threshold: 0.5,
        min_char_count: 20,
        lang_hint: Some(Lang("kor".into())),
        cancel: None,
        ocr_cache: None,
        ocr_version_key: String::new(),
        // No renderer in these unit tests: they pin the fallback path's
        // behavior, which is what a machine without pdfium gets.
        renderer: None,
        render_dpi: 300,
        max_pixels: 4096,
    };

    let summary = apply_ocr_to_pdf_pages(&mut canonical, &engine, &bytes, &opts, |_| {}).unwrap();

    assert_eq!(summary.pages_ocrd, 1);
    let first_para = canonical.blocks.iter().find_map(|b| match b {
        Block::Paragraph(tb) => Some(tb),
        _ => None,
    });
    assert!(first_para.is_some());
    assert_eq!(first_para.unwrap().text, "MOCK_OCR_TEXT");
}

// Test 2: F3 vector (mock filled canonical) + enabled=true → OCR skip (needs_ocr=false)
#[test]
fn f3_input_with_ocr_enabled_keeps_text_detect_blocks() {
    let bytes = f1_pdf_bytes(); // reuse F1 bytes; decision is based on canonical text
    let text = "충분한 한국어 텍스트 컨텐츠입니다. This has more than twenty characters.";
    let mut canonical = canonical_with_filled_block(text);
    let engine = MockOcrEngine::single("SHOULD_NOT_BE_CALLED", false);
    let opts = default_opts(true);

    let summary = apply_ocr_to_pdf_pages(&mut canonical, &engine, &bytes, &opts, |_| {}).unwrap();

    assert_eq!(summary.pages_ocrd, 0, "vector PDF 의 OCR 호출 0");
    let first_para = canonical.blocks.iter().find_map(|b| match b {
        Block::Paragraph(tb) => Some(tb),
        _ => None,
    });
    if let Some(tb) = first_para {
        assert!(tb.text.starts_with("충분한"), "원본 text 보존");
    }
}

// Test 3: F1 + enabled=false → no-op
#[test]
fn f1_input_with_ocr_disabled_keeps_empty_block() {
    let bytes = f1_pdf_bytes();
    let mut canonical = canonical_with_empty_block();
    let engine = MockOcrEngine::single("IGNORED", false);
    let opts = default_opts(false);

    let summary = apply_ocr_to_pdf_pages(&mut canonical, &engine, &bytes, &opts, |_| {}).unwrap();

    assert_eq!(summary.pages_ocrd, 0);
    assert_eq!(summary.ms_total, 0);
}

// Test 4: mojibake canonical (PUA chars) + enabled=true → in-place mutate
#[test]
fn f4_input_with_ocr_enabled_replaces_mojibake_block() {
    let bytes = f1_pdf_bytes(); // F1 bytes carry DCTDecode image
    let mut canonical = canonical_with_mojibake_block();
    let engine = MockOcrEngine::single("OCR_MOJIBAKE_REPLACEMENT", false);
    let opts = PdfOcrOpts {
        enabled: true,
        always_on: false,
        valid_ratio_threshold: 0.5,
        min_char_count: 20,
        lang_hint: None,
        cancel: None,
        ocr_cache: None,
        ocr_version_key: String::new(),
        // No renderer in these unit tests: they pin the fallback path's
        // behavior, which is what a machine without pdfium gets.
        renderer: None,
        render_dpi: 300,
        max_pixels: 4096,
    };

    let summary = apply_ocr_to_pdf_pages(&mut canonical, &engine, &bytes, &opts, |_| {}).unwrap();

    assert_eq!(summary.pages_ocrd, 1, "mojibake page 의 OCR 호출");
    let first_para = canonical.blocks.iter().find_map(|b| match b {
        Block::Paragraph(tb) => Some(tb),
        _ => None,
    });
    if let Some(tb) = first_para {
        assert_eq!(tb.text, "OCR_MOJIBAKE_REPLACEMENT");
    }
}

// Test 5: filled canonical + always_on=true → dual-block (+1 OCR block)
#[test]
fn f3_input_with_always_on_pushes_dual_blocks() {
    let bytes = f1_pdf_bytes();
    let text = "vector PDF 충분한 텍스트 컨텐츠입니다. This has enough characters for valid ratio.";
    let mut canonical = canonical_with_filled_block(text);
    let original_block_count = canonical.blocks.len();
    let engine = MockOcrEngine::single("OCR_DUAL", false);
    let opts = PdfOcrOpts {
        enabled: true,
        always_on: true,
        valid_ratio_threshold: 0.5,
        min_char_count: 20,
        lang_hint: None,
        cancel: None,
        ocr_cache: None,
        ocr_version_key: String::new(),
        // No renderer in these unit tests: they pin the fallback path's
        // behavior, which is what a machine without pdfium gets.
        renderer: None,
        render_dpi: 300,
        max_pixels: 4096,
    };

    let summary = apply_ocr_to_pdf_pages(&mut canonical, &engine, &bytes, &opts, |_| {}).unwrap();

    assert_eq!(summary.pages_ocrd, 1);
    assert_eq!(
        canonical.blocks.len(),
        original_block_count + 1,
        "always_on 시 새 Block::Paragraph push"
    );
    let texts: Vec<&str> = canonical
        .blocks
        .iter()
        .filter_map(|b| match b {
            Block::Paragraph(tb) => Some(tb.text.as_str()),
            _ => None,
        })
        .collect();
    assert!(texts.contains(&"OCR_DUAL"), "OCR block 포함");
    assert!(
        texts.iter().any(|t| t.starts_with("vector")),
        "원본 text-detect block 보존"
    );
}

// Test 6: F6 FlateDecode → extract_dctdecode_page_image=None → skip + warning
#[test]
fn f6_flatedecode_skipped_with_warning() {
    let bytes = std::fs::read("../kebab-parse-pdf/tests/fixtures/flate_raw.pdf")
        .expect("F6 fixture missing");
    let mut canonical = canonical_with_empty_block(); // page-1 block from F1
    let engine = MockOcrEngine::single("SHOULD_NOT_BE_CALLED", false);
    let opts = default_opts(true);

    let summary = apply_ocr_to_pdf_pages(&mut canonical, &engine, &bytes, &opts, |_| {}).unwrap();

    assert_eq!(
        summary.pages_ocrd, 0,
        "FlateDecode page 는 skip (DCTDecode-only v1 invariant)"
    );
    let warning_count = canonical
        .provenance
        .events
        .iter()
        .filter(|e| e.kind == kebab_core::ProvenanceKind::Warning)
        .count();
    assert!(warning_count >= 1, "FlateDecode skip 시 Warning event 발행");
}

// Test 7: F7 CCITTFax → skip + warning (verifier M-4 split)
#[test]
fn f7_ccittfax_skipped_with_warning() {
    let bytes =
        std::fs::read("../kebab-parse-pdf/tests/fixtures/ccitt.pdf").expect("F7 fixture missing");
    let mut canonical = canonical_with_empty_block(); // page-1 block from F1
    let engine = MockOcrEngine::single("SHOULD_NOT_BE_CALLED", false);
    let opts = default_opts(true);

    let summary = apply_ocr_to_pdf_pages(&mut canonical, &engine, &bytes, &opts, |_| {}).unwrap();

    assert_eq!(summary.pages_ocrd, 0, "CCITTFax page 는 skip");
    let warning_count = canonical
        .provenance
        .events
        .iter()
        .filter(|e| e.kind == kebab_core::ProvenanceKind::Warning)
        .count();
    assert!(warning_count >= 1, "CCITTFax skip 시 Warning event 발행");
}

// Test 8: OCR engine failure → warning event + skip
#[test]
fn ocr_engine_failure_surfaces_as_warning() {
    let bytes = f1_pdf_bytes();
    let mut canonical = canonical_with_empty_block();
    let engine = MockOcrEngine::single("", true);
    let opts = default_opts(true);

    let summary = apply_ocr_to_pdf_pages(&mut canonical, &engine, &bytes, &opts, |_| {}).unwrap();

    assert_eq!(summary.pages_ocrd, 0, "OCR failure 시 pages_ocrd=0");
    let warning_with_failure = canonical.provenance.events.iter().any(|e| {
        e.kind == kebab_core::ProvenanceKind::Warning
            && e.note.as_deref().unwrap_or("").contains("mock failure")
    });
    assert!(
        warning_with_failure,
        "OCR failure 의 error message 가 warning event 의 note 안"
    );
    // issue #239: the note must carry the whole error chain, not just the
    // outermost layer. The real cause (ORT's "Invalid input shape") sits under
    // a `.context`, so a note formatted with `{e}` instead of `{e:#}` drops it
    // and the KB can no longer be searched for which documents were hit.
    let warning_with_cause = canonical.provenance.events.iter().any(|e| {
        e.kind == kebab_core::ProvenanceKind::Warning
            && e.note.as_deref().unwrap_or("").contains("mock inner cause")
    });
    assert!(
        warning_with_cause,
        "provenance note 가 error chain 의 안쪽 원인까지 담아야 한다 (`{{e:#}}`)"
    );
}

// Test 9: dual-block ordinals are deterministic and unique
#[test]
fn dual_block_ordinals_are_deterministic_and_unique() {
    let bytes = f1_pdf_bytes(); // 1-page PDF → page_count=1
    let text = "vector 충분한 텍스트. This text has more than twenty characters total.";
    let mut canonical = canonical_with_filled_block(text);
    let engine = MockOcrEngine::single("DUAL", false);
    let opts = PdfOcrOpts {
        enabled: true,
        always_on: true,
        valid_ratio_threshold: 0.5,
        min_char_count: 20,
        lang_hint: None,
        cancel: None,
        ocr_cache: None,
        ocr_version_key: String::new(),
        // No renderer in these unit tests: they pin the fallback path's
        // behavior, which is what a machine without pdfium gets.
        renderer: None,
        render_dpi: 300,
        max_pixels: 4096,
    };

    apply_ocr_to_pdf_pages(&mut canonical, &engine, &bytes, &opts, |_| {}).unwrap();

    // page_count=1 → text-detect ordinal=0, ocr ordinal=1 (page_num-1 + page_count = 0+1=1)
    let para_count = canonical
        .blocks
        .iter()
        .filter(|b| matches!(b, Block::Paragraph(_)))
        .count();
    assert_eq!(para_count, 2, "dual-block: text-detect + OCR");

    let all_page_1 = canonical
        .blocks
        .iter()
        .filter_map(|b| match b {
            Block::Paragraph(tb) => Some(&tb.common.source_span),
            _ => None,
        })
        .all(|s| matches!(s, SourceSpan::Page { page: 1, .. }));
    assert!(all_page_1, "두 block 모두 page=1");
}

// Test 10: cancel handle aborts mid-PDF
#[test]
fn cancel_handle_aborts_mid_pdf() {
    let bytes = f1_pdf_bytes();
    let mut canonical = canonical_with_empty_block();
    let cancel = Arc::new(AtomicBool::new(true)); // pre-cancel
    let engine = MockOcrEngine::single("IGNORED", false);
    let opts = PdfOcrOpts {
        enabled: true,
        always_on: false,
        valid_ratio_threshold: 0.5,
        min_char_count: 20,
        lang_hint: None,
        cancel: Some(cancel.clone()),
        ocr_cache: None,
        ocr_version_key: String::new(),
        // No renderer in these unit tests: they pin the fallback path's
        // behavior, which is what a machine without pdfium gets.
        renderer: None,
        render_dpi: 300,
        max_pixels: 4096,
    };

    let result = apply_ocr_to_pdf_pages(&mut canonical, &engine, &bytes, &opts, |_| {});
    let err = result.expect_err("cancel=true 시 error 반환");
    assert!(
        format!("{err}").contains("cancelled mid-PDF"),
        "error message 가 'cancelled mid-PDF' 포함: {err}"
    );
}

// ── Renderer path (issue #232) ────────────────────────────────────────────
//
// The tests above pin what a machine *without* pdfium does. These pin the
// other half. They are `#[ignore]`d behind `KEBAB_TEST_PDFIUM` for the
// same reason the renderer itself is optional — a lane without the
// library must not report a failure it cannot act on:
//
//     KEBAB_TEST_PDFIUM=/path/to/libpdfium.so \
//       cargo test -p kebab-app --test pdf_ocr_apply -- --ignored
//
// The OCR engine is mocked, so these exercise rasterization and the
// branch that chooses it without touching a network or a model.

/// Bound once for the binary: pdfium's initialization is not reentrant
/// and cargo runs tests in parallel.
fn test_renderer() -> Arc<kebab_parse_pdf::PageRenderer> {
    static SHARED: std::sync::OnceLock<Arc<kebab_parse_pdf::PageRenderer>> =
        std::sync::OnceLock::new();
    SHARED
        .get_or_init(|| {
            let explicit = std::env::var("KEBAB_TEST_PDFIUM").ok();
            let path = explicit.as_deref().map(Path::new);
            Arc::new(
                kebab_parse_pdf::PageRenderer::bind(path)
                    .expect("these tests require libpdfium; point KEBAB_TEST_PDFIUM at one"),
            )
        })
        .clone()
}

fn opts_with_renderer() -> PdfOcrOpts {
    PdfOcrOpts {
        renderer: Some(test_renderer()),
        ..default_opts(true)
    }
}

/// The whole point of issue #232. `f7_ccittfax_skipped_with_warning`
/// above pins that this exact fixture is skipped with no renderer; with
/// one, the same bytes must reach the OCR engine instead.
#[test]
#[ignore = "requires libpdfium"]
fn a_ccitt_page_reaches_the_ocr_engine_once_a_renderer_is_configured() {
    let bytes =
        std::fs::read("../kebab-parse-pdf/tests/fixtures/ccitt.pdf").expect("F7 fixture missing");
    let mut canonical = canonical_with_empty_block();
    let engine = MockOcrEngine::single("RASTERIZED AND READ", false);

    let summary = apply_ocr_to_pdf_pages(
        &mut canonical,
        &engine,
        &bytes,
        &opts_with_renderer(),
        |_| {},
    )
    .unwrap();

    assert_eq!(
        summary.pages_ocrd, 1,
        "the CCITT page must be OCR'd, not skipped: {summary:?}"
    );
    assert_eq!(
        summary.pages_skipped, 0,
        "and must not be counted as a page with no raster"
    );
}

/// A renderer must not cost the DCTDecode path its coverage. Same
/// fixture as `f1_enabled_true_mutates_block_in_place`, with a renderer
/// added — the outcome has to be the same.
#[test]
#[ignore = "requires libpdfium"]
fn a_dctdecode_page_still_works_with_a_renderer_configured() {
    let bytes = f1_pdf_bytes();
    let mut canonical = canonical_with_empty_block();
    let engine = MockOcrEngine::single("STILL READ", false);

    let summary = apply_ocr_to_pdf_pages(
        &mut canonical,
        &engine,
        &bytes,
        &opts_with_renderer(),
        |_| {},
    )
    .unwrap();

    assert_eq!(summary.pages_ocrd, 1);
    assert_eq!(summary.pages_skipped, 0);
}

/// A PDF that lopdf parses but pdfium refuses. The renderer is
/// configured and working — it simply cannot open *this* file — so the
/// page must fall through to the DCTDecode path and, when that also has
/// nothing, be reported as `unopenable_pdf` rather than `no_renderer`.
/// Telling a user who already configured a renderer to configure one is
/// the wrong instruction.
///
/// The fixture is an encrypted PDF with no password: lopdf reads the
/// object graph without decrypting, pdfium refuses outright
/// (`PasswordError`). A first attempt at this test used truncated bytes,
/// which lopdf rejected before the branch was ever reached — the test
/// passed while exercising nothing.
#[test]
#[ignore = "requires libpdfium"]
fn a_pdf_the_renderer_cannot_open_falls_back_instead_of_blaming_config() {
    let bytes = std::fs::read("../kebab-parse-pdf/tests/fixtures/encrypted_no_password.pdf")
        .expect("encrypted fixture missing");
    let mut canonical = canonical_with_empty_block();
    let engine = MockOcrEngine::single("SHOULD_NOT_BE_CALLED", false);

    let mut reasons = Vec::new();
    let summary = apply_ocr_to_pdf_pages(
        &mut canonical,
        &engine,
        &bytes,
        &opts_with_renderer(),
        |p| {
            if let kebab_app::pdf_ocr_apply::PdfOcrProgress::Finished {
                failure_reason: Some(r),
                ..
            } = p
            {
                reasons.push(r);
            }
        },
    )
    .expect("a PDF the renderer cannot open must not abort the run");

    assert_eq!(summary.pages_ocrd, 0, "nothing could be rasterized");
    assert_eq!(
        summary.pages_skipped, 1,
        "and the page is counted as skipped"
    );
    assert_eq!(
        reasons,
        vec!["unopenable_pdf".to_string()],
        "the reason must name the real problem, not a missing renderer"
    );
}
