//! Integration tests for pdf_ocr_apply helper. spec §5.5 MockOcrEngine pattern.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use anyhow::Result;
use kebab_app::pdf_ocr_apply::{PdfOcrOpts, apply_ocr_to_pdf_pages};
use kebab_core::{
    AssetStorage, Block, CanonicalDocument, Checksum, ExtractConfig, ExtractContext,
    Extractor, Inline, Lang, MediaType, OcrText, RawAsset, SourceSpan,
    SourceUri, WorkspacePath, id_for_asset,
};
use kebab_parse_image::OcrEngine;
use kebab_parse_pdf::PdfTextExtractor;
use time::OffsetDateTime;

// ── MockOcrEngine fixture ─────────────────────────────────────────────────

struct MockOcrEngine {
    expected_text: String,
    fail: bool,
}

impl OcrEngine for MockOcrEngine {
    fn engine_name(&self) -> &'static str {
        "mock-ocr"
    }

    fn engine_version(&self) -> String {
        "mock-v1".to_string()
    }

    fn recognize(&self, _img: &[u8], _hint: Option<&Lang>) -> Result<OcrText> {
        if self.fail {
            anyhow::bail!("mock failure");
        }
        Ok(OcrText {
            joined: self.expected_text.clone(),
            regions: Vec::new(),
            engine: self.engine_name().to_string(),
            engine_version: self.engine_version(),
        })
    }
}

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
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

// Test 1: F1 + enabled=true → in-place mutate
#[test]
fn f1_input_with_ocr_enabled_replaces_empty_block() {
    let bytes = f1_pdf_bytes();
    let mut canonical = canonical_with_empty_block();
    let engine = MockOcrEngine {
        expected_text: "MOCK_OCR_TEXT".into(),
        fail: false,
    };
    let opts = PdfOcrOpts {
        enabled: true,
        always_on: false,
        valid_ratio_threshold: 0.5,
        min_char_count: 20,
        lang_hint: Some(Lang("kor".into())),
        cancel: None,
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
    let engine = MockOcrEngine {
        expected_text: "SHOULD_NOT_BE_CALLED".into(),
        fail: false,
    };
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
    let engine = MockOcrEngine {
        expected_text: "IGNORED".into(),
        fail: false,
    };
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
    let engine = MockOcrEngine {
        expected_text: "OCR_MOJIBAKE_REPLACEMENT".into(),
        fail: false,
    };
    let opts = PdfOcrOpts {
        enabled: true,
        always_on: false,
        valid_ratio_threshold: 0.5,
        min_char_count: 20,
        lang_hint: None,
        cancel: None,
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
    let engine = MockOcrEngine {
        expected_text: "OCR_DUAL".into(),
        fail: false,
    };
    let opts = PdfOcrOpts {
        enabled: true,
        always_on: true,
        valid_ratio_threshold: 0.5,
        min_char_count: 20,
        lang_hint: None,
        cancel: None,
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
    let engine = MockOcrEngine {
        expected_text: "SHOULD_NOT_BE_CALLED".into(),
        fail: false,
    };
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
    let bytes = std::fs::read("../kebab-parse-pdf/tests/fixtures/ccitt.pdf")
        .expect("F7 fixture missing");
    let mut canonical = canonical_with_empty_block(); // page-1 block from F1
    let engine = MockOcrEngine {
        expected_text: "SHOULD_NOT_BE_CALLED".into(),
        fail: false,
    };
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
    let engine = MockOcrEngine {
        expected_text: String::new(),
        fail: true,
    };
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
}

// Test 9: dual-block ordinals are deterministic and unique
#[test]
fn dual_block_ordinals_are_deterministic_and_unique() {
    let bytes = f1_pdf_bytes(); // 1-page PDF → page_count=1
    let text = "vector 충분한 텍스트. This text has more than twenty characters total.";
    let mut canonical = canonical_with_filled_block(text);
    let engine = MockOcrEngine {
        expected_text: "DUAL".into(),
        fail: false,
    };
    let opts = PdfOcrOpts {
        enabled: true,
        always_on: true,
        valid_ratio_threshold: 0.5,
        min_char_count: 20,
        lang_hint: None,
        cancel: None,
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
    let engine = MockOcrEngine {
        expected_text: "IGNORED".into(),
        fail: false,
    };
    let opts = PdfOcrOpts {
        enabled: true,
        always_on: false,
        valid_ratio_threshold: 0.5,
        min_char_count: 20,
        lang_hint: None,
        cancel: Some(cancel.clone()),
    };

    let result = apply_ocr_to_pdf_pages(&mut canonical, &engine, &bytes, &opts, |_| {});
    let err = result.expect_err("cancel=true 시 error 반환");
    assert!(
        format!("{err}").contains("cancelled mid-PDF"),
        "error message 가 'cancelled mid-PDF' 포함: {err}"
    );
}
