//! Bug #3 regression: multi-scanned PDF ingest must produce globally unique chunk_ids.
//! v0.20.0 sub-item 1 bugfix.
//!
//! Strategy: helper-level chain test (apply_ocr_to_pdf_pages → PdfPageV1Chunker).
//! Facade mock injection is unavailable (kebab-app hardcodes OllamaVisionOcr), so
//! this test covers the full OCR→chunk pipeline with real PDF fixtures + MockOcrEngine,
//! adding value beyond kebab-chunk unit test B5 (which tests PdfPageV1Chunker alone).

mod common;

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use common::mock_ocr::MockOcrEngine;
use kebab_app::pdf_ocr_apply::{PdfOcrOpts, apply_ocr_to_pdf_pages};
use kebab_chunk::PdfPageV1Chunker;
use kebab_core::{
    AssetStorage, Checksum, ChunkPolicy, Chunker, ExtractConfig, ExtractContext, Extractor,
    MediaType, RawAsset, SourceUri, WorkspacePath, id_for_asset,
};
use kebab_parse_image::OcrEngine;
use kebab_parse_pdf::PdfTextExtractor;
use time::OffsetDateTime;

fn make_pdf_asset(path: &str, hash_char: char, byte_len: u64) -> RawAsset {
    let fake_hash: String = hash_char.to_string().repeat(64);
    let asset_id = id_for_asset(&fake_hash);
    RawAsset {
        asset_id,
        source_uri: SourceUri::File(PathBuf::from(path)),
        workspace_path: WorkspacePath::new(path.to_string()).unwrap(),
        media_type: MediaType::Pdf,
        byte_len,
        checksum: Checksum(fake_hash),
        discovered_at: OffsetDateTime::UNIX_EPOCH,
        stored: AssetStorage::Copied {
            path: PathBuf::from(path),
        },
    }
}

fn extract_and_ocr(
    bytes: &[u8],
    path: &str,
    hash_char: char,
    engine: &dyn OcrEngine,
) -> kebab_core::CanonicalDocument {
    let asset = make_pdf_asset(path, hash_char, bytes.len() as u64);
    let workspace_root = Path::new("/");
    let config = ExtractConfig::default();
    let ctx = ExtractContext {
        asset: &asset,
        workspace_root,
        config: &config,
    };
    let mut canonical = PdfTextExtractor::new().extract(&ctx, bytes).unwrap();
    let opts = PdfOcrOpts {
        enabled: true,
        always_on: false,
        valid_ratio_threshold: 0.5,
        min_char_count: 20,
        lang_hint: None,
        cancel: None,
    };
    apply_ocr_to_pdf_pages(&mut canonical, engine, bytes, &opts, |_| {}).unwrap();
    canonical
}

#[test]
fn multi_scanned_pdf_ingest_no_chunk_id_collision() {
    let f1_bytes = std::fs::read("../kebab-parse-pdf/tests/fixtures/scanned_page1.pdf")
        .expect("F1 fixture missing");
    let f2_bytes = std::fs::read("../kebab-parse-pdf/tests/fixtures/scanned_page2.pdf")
        .expect("F2 fixture missing");

    // Bug #3 trigger shape: 10-char early segment + ". " + 500-char tail.
    // byte_len = 10*3 + 2 + 500*3 = 1532 > target_bytes=1500 → multi-chunk.
    // overlap_bytes = min(240, 750) = 240 / chars=80 → second chunk's actual_start
    // collapses to prev_min=0 without the fix → same #c0 suffix → chunk_id collision.
    let trigger_text = format!("{}. {}", "가".repeat(10), "나".repeat(500));

    let f1_engine = MockOcrEngine::single("F1 mock OCR page text", false);
    let f2_engine = MockOcrEngine::single(&trigger_text, false);

    let f1_canonical = extract_and_ocr(&f1_bytes, "page1.pdf", '1', &f1_engine);
    let f2_canonical = extract_and_ocr(&f2_bytes, "page2.pdf", '2', &f2_engine);

    // v1.2: PdfPageV1Chunker carries a tier-2 oversize budget. Use a
    // generous budget so the tier-2 split never fires — this test exercises
    // the tier-1 sentence/paragraph `#c{segment_start}` collision-avoidance,
    // which is unchanged from v1.1.
    let chunker = PdfPageV1Chunker {
        max_chunk_tokens: 100_000,
    };
    let chunk_policy = ChunkPolicy {
        target_tokens: 500,
        overlap_tokens: 80,
        respect_markdown_headings: false,
        chunker_version: chunker.chunker_version(),
    };

    let f1_chunks = chunker.chunk(&f1_canonical, &chunk_policy).unwrap();
    let f2_chunks = chunker.chunk(&f2_canonical, &chunk_policy).unwrap();

    assert!(
        f2_chunks.len() >= 2,
        "F2 trigger text must produce ≥2 chunks for the collision to be possible; got {}",
        f2_chunks.len()
    );

    let all_ids: Vec<&str> = f1_chunks
        .iter()
        .chain(f2_chunks.iter())
        .map(|c| c.chunk_id.0.as_str())
        .collect();
    let total = all_ids.len();
    let unique: HashSet<&str> = all_ids.iter().copied().collect();
    assert_eq!(
        unique.len(),
        total,
        "all chunk_ids must be globally unique across F1 + F2 ({} unique vs {} total — collision detected)",
        unique.len(),
        total,
    );
}
