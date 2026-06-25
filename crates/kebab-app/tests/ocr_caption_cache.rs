//! PR2: OCR/caption derivation cache — deterministic, model-free correctness
//! gate. Drives the b3 PDF per-page OCR seam (`apply_ocr_to_pdf_pages` +
//! `PdfOcrOpts.ocr_cache`) with `MockOcrEngine`, the only publicly
//! mock-injectable OCR seam. The mock COUNTS its `recognize` calls, so a
//! re-run on the same PDF + same engine + same `ocr_version_key` proves a
//! cache HIT (engine NOT re-invoked) that reconstructs byte-identical OCR
//! text, and a version-key change proves a MISS (§3.6 invalidation safety).
//!
//! This test exercises the REAL cache code path
//! (`derivation_cache_get/put/touch` + `decode_ocr_text`) end-to-end against
//! a real `SqliteStore`; the invocation-count assertion is the non-negotiable
//! validity signal (Task 6).

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use common::mock_ocr::MockOcrEngine;
use kebab_app::pdf_ocr_apply::{PdfOcrOpts, apply_ocr_to_pdf_pages};
use kebab_core::{
    AssetStorage, Block, CanonicalDocument, Checksum, ExtractConfig, ExtractContext, Extractor,
    Lang, MediaType, RawAsset, SourceUri, WorkspacePath, id_for_asset,
};
use kebab_parse_pdf::PdfTextExtractor;
use kebab_store_sqlite::SqliteStore;
use time::OffsetDateTime;

// ── Fixture helpers (mirrors pdf_ocr_apply.rs) ────────────────────────────

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
/// F1 (scanned) returns an empty-text Block::Paragraph per page → triggers OCR.
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

/// Pull the page-1 `Block::Paragraph` text out of a canonical (the block the
/// in-place OCR fallback mutates).
fn first_paragraph_text(canonical: &CanonicalDocument) -> String {
    canonical
        .blocks
        .iter()
        .find_map(|b| match b {
            Block::Paragraph(tb) => Some(tb.text.clone()),
            _ => None,
        })
        .expect("page-1 Block::Paragraph present")
}

/// A real `SqliteStore` over a fresh temp dir, migrations applied, wrapped in
/// `Arc` for `PdfOcrOpts.ocr_cache`. Owns the `TempDir` (returned alongside so
/// the caller keeps it alive for the test's lifetime).
fn temp_store() -> (Arc<SqliteStore>, tempfile::TempDir) {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = kebab_config::Config::defaults().storage;
    storage.data_dir = temp.path().to_string_lossy().into_owned();
    let store = SqliteStore::open(&storage).expect("open SqliteStore");
    store.run_migrations().expect("run_migrations");
    (Arc::new(store), temp)
}

/// OCR-enabled opts mirroring `pdf_ocr_apply.rs`'s working in-place fallback
/// (`f1_input_with_ocr_enabled_replaces_empty_block`), with the cache wired.
fn opts_with_cache(store: &Arc<SqliteStore>, version_key: &str) -> PdfOcrOpts {
    PdfOcrOpts {
        enabled: true,
        always_on: false,
        valid_ratio_threshold: 0.5,
        min_char_count: 20,
        lang_hint: Some(Lang("kor".into())),
        cancel: None,
        ocr_cache: Some(Arc::clone(store)),
        ocr_version_key: version_key.to_string(),
    }
}

// ── Test ──────────────────────────────────────────────────────────────────

/// Primary deterministic correctness gate for PR2's OCR cache: a re-run on the
/// same PDF + engine + version key is a cache HIT (engine NOT re-invoked) that
/// reconstructs byte-identical OCR text, and a version-key bump MISSES.
#[test]
fn pdf_ocr_reingest_is_cache_hit_engine_not_reinvoked() {
    let (store, _temp) = temp_store();
    let bytes = f1_pdf_bytes();
    let engine = MockOcrEngine::single("CACHED_PAGE_TEXT", false);

    let opts_v1 = opts_with_cache(&store, "test-ocr-v1");

    // Run 1 (cold): engine recognizes the page, result is cached.
    let mut c1 = canonical_with_empty_block();
    apply_ocr_to_pdf_pages(&mut c1, &engine, &bytes, &opts_v1, |_| {}).unwrap();
    let calls_after_first = engine.call_count();
    assert!(
        calls_after_first >= 1,
        "cold run must invoke the OCR engine, got {calls_after_first}"
    );
    let text_first = first_paragraph_text(&c1);
    assert_eq!(text_first, "CACHED_PAGE_TEXT", "cold run OCRs the empty block");

    // Run 2 (warm, same version_key + same store + same PDF): cache HIT, engine
    // NOT re-invoked.
    let mut c2 = canonical_with_empty_block();
    apply_ocr_to_pdf_pages(&mut c2, &engine, &bytes, &opts_v1, |_| {}).unwrap();
    assert_eq!(
        engine.call_count(),
        calls_after_first,
        "re-run must be a cache HIT — engine.recognize must NOT be called again"
    );
    let text_second = first_paragraph_text(&c2);
    assert_eq!(
        text_first, text_second,
        "cached page text must be byte-identical"
    );

    // Version bump → MISS (engine re-invoked). Proves §3.6 invalidation safety.
    let opts_v2 = opts_with_cache(&store, "test-ocr-v2");
    let mut c3 = canonical_with_empty_block();
    apply_ocr_to_pdf_pages(&mut c3, &engine, &bytes, &opts_v2, |_| {}).unwrap();
    assert!(
        engine.call_count() > calls_after_first,
        "a version_key change must MISS and re-invoke the engine ({} vs {calls_after_first})",
        engine.call_count()
    );
}
