//! Byte-identical regression for the vector PDF extraction path (spec §5.4).
//! Uses F4 (mojibake.pdf) — the only fixture with extractable text content.
//! First invocation creates the baseline snapshot; subsequent runs verify
//! identity to detect silent regressions across all Step 1-8 changes.

use std::path::Path;

use kebab_core::{
    AssetStorage, Checksum, ExtractConfig, ExtractContext, Extractor, MediaType, RawAsset,
    SourceUri, WorkspacePath, id_for_asset,
};
use kebab_parse_pdf::PdfTextExtractor;
use time::OffsetDateTime;

/// Normalize all provenance timestamps to UNIX_EPOCH so the snapshot is
/// byte-stable across runs (R-3 mitigation — no workspace helper exists).
fn normalize_provenance_timestamps(doc: &mut kebab_core::CanonicalDocument) {
    for event in &mut doc.provenance.events {
        event.at = OffsetDateTime::UNIX_EPOCH;
    }
}

fn make_raw_asset(path: &str) -> RawAsset {
    let fake_hash = "0".repeat(64);
    let asset_id = id_for_asset(&fake_hash);
    RawAsset {
        asset_id,
        source_uri: SourceUri::File(std::path::PathBuf::from(path)),
        workspace_path: WorkspacePath::new(path.to_string()).unwrap(),
        media_type: MediaType::Pdf,
        byte_len: 0,
        checksum: Checksum(fake_hash),
        discovered_at: OffsetDateTime::UNIX_EPOCH,
        stored: AssetStorage::Copied {
            path: std::path::PathBuf::from(path),
        },
    }
}

#[test]
fn vector_pdf_extract_byte_identical_to_baseline() {
    let bytes = include_bytes!("fixtures/mojibake.pdf");
    let asset = make_raw_asset("mojibake.pdf");
    let workspace_root = Path::new("/");
    let config = ExtractConfig::default();
    let ctx = ExtractContext {
        asset: &asset,
        workspace_root,
        config: &config,
    };

    let mut canonical = PdfTextExtractor::new()
        .extract(&ctx, bytes)
        .expect("PdfTextExtractor::extract");
    normalize_provenance_timestamps(&mut canonical);

    let actual = serde_json::to_string_pretty(&canonical).expect("serialize canonical");

    let baseline_path = "tests/snapshots/vector_pdf_canonical.json";
    let baseline = std::fs::read_to_string(baseline_path).unwrap_or_else(|_| {
        std::fs::create_dir_all("tests/snapshots").ok();
        std::fs::write(baseline_path, &actual).expect("write baseline snapshot");
        actual.clone()
    });

    assert_eq!(
        actual, baseline,
        "vector PDF canonical must be byte-identical to baseline (Step 1-8 regression)"
    );
}
