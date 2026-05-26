//! Integration tests for `kebab_parse_image::ImageExtractor` (P6-1).

mod common;

use kebab_core::{Block, Extractor, ImageType, ProvenanceKind, SourceSpan};
use kebab_parse_image::ImageExtractor;
use serde_json::Value;

use crate::common::{
    corrupt_png, exif_gps_no_ref_jpg, exif_gps_out_of_range_jpg, exif_with_gps_jpg, fixture_for,
    no_exif_png, red_100x50_png, strip_dynamic_at,
};

fn extract_block(doc: &kebab_core::CanonicalDocument) -> &kebab_core::ImageRefBlock {
    assert_eq!(doc.blocks.len(), 1, "exactly one block expected");
    match &doc.blocks[0] {
        Block::ImageRef(b) => b,
        other => panic!("expected ImageRef, got {other:?}"),
    }
}

#[test]
fn png_decode_produces_correct_dimensions() {
    let bytes = red_100x50_png();
    let fx = fixture_for("photos/red-100x50.png", ImageType::Png, &bytes);
    let doc = ImageExtractor::new()
        .extract(&fx.ctx(), &bytes)
        .expect("PNG extraction must succeed");

    assert_eq!(doc.title, "red-100x50");
    assert_eq!(doc.lang.0, "und");
    assert_eq!(doc.parser_version.0, kebab_parse_image::PARSER_VERSION);

    let dims = doc
        .metadata
        .user
        .get("dimensions")
        .expect("dimensions key present");
    let obj = dims.as_object().expect("dimensions is an object");
    assert_eq!(obj.get("w"), Some(&Value::Number(100.into())));
    assert_eq!(obj.get("h"), Some(&Value::Number(50.into())));
    assert_eq!(obj.get("format"), Some(&Value::String("png".into())));

    let block = extract_block(&doc);
    assert_eq!(block.alt, "red-100x50.png");
    assert_eq!(block.src, "photos/red-100x50.png");
    assert_eq!(block.asset_id, Some(fx.asset.asset_id.clone()));
    assert!(block.ocr.is_none());
    assert!(block.caption.is_none());
    match &block.common.source_span {
        SourceSpan::Region { x, y, w, h } => {
            assert_eq!((*x, *y, *w, *h), (0, 0, 100, 50));
        }
        other => panic!("expected Region span, got {other:?}"),
    }
}

#[test]
fn jpeg_with_exif_gps_captures_whitelisted_tags() {
    let bytes = exif_with_gps_jpg();
    let fx = fixture_for("img/seoul.jpg", ImageType::Jpeg, &bytes);
    let doc = ImageExtractor::new()
        .extract(&fx.ctx(), &bytes)
        .expect("JPEG extraction must succeed");

    let exif = doc
        .metadata
        .user
        .get("exif")
        .and_then(|v| v.as_object())
        .expect("exif object present");
    assert_eq!(exif.get("make"), Some(&Value::String("KebabCam".into())));
    assert_eq!(exif.get("model"), Some(&Value::String("X1".into())));
    assert_eq!(
        exif.get("software"),
        Some(&Value::String("kebab-test".into()))
    );
    assert_eq!(
        exif.get("date_time_original"),
        Some(&Value::String("2024-08-15T12:34:56".into()))
    );
    assert_eq!(exif.get("orientation"), Some(&Value::Number(1.into())));
    let lat = exif.get("gps_lat").and_then(serde_json::Value::as_f64).expect("gps_lat");
    let lon = exif.get("gps_lon").and_then(serde_json::Value::as_f64).expect("gps_lon");
    assert!((lat - 37.5).abs() < 1e-6, "lat={lat}");
    assert!((lon - 127.0).abs() < 1e-6, "lon={lon}");

    // Maker notes / thumbnails / unrelated tags must NOT have leaked in.
    let allowed: std::collections::HashSet<&str> = [
        "make",
        "model",
        "software",
        "date_time_original",
        "orientation",
        "gps_lat",
        "gps_lon",
    ]
    .into_iter()
    .collect();
    for k in exif.keys() {
        assert!(
            allowed.contains(k.as_str()),
            "non-whitelisted EXIF key leaked: {k}"
        );
    }
}

#[test]
fn no_exif_image_yields_empty_exif_map() {
    let bytes = no_exif_png();
    let fx = fixture_for("img/blank.png", ImageType::Png, &bytes);
    let doc = ImageExtractor::new()
        .extract(&fx.ctx(), &bytes)
        .expect("PNG extraction must succeed");
    let exif = doc
        .metadata
        .user
        .get("exif")
        .and_then(|v| v.as_object())
        .expect("exif object present");
    assert!(exif.is_empty(), "no-EXIF PNG must yield empty exif map: {exif:?}");
}

#[test]
fn corrupt_image_emits_warning_no_panic() {
    let bytes = corrupt_png();
    let fx = fixture_for("img/corrupt.png", ImageType::Png, &bytes);
    let doc = ImageExtractor::new()
        .extract(&fx.ctx(), &bytes)
        .expect("corrupt PNG must NOT cause an Err — warning provenance event instead");

    // dimensions = null
    assert_eq!(
        doc.metadata.user.get("dimensions"),
        Some(&Value::Null),
        "corrupt image must record dimensions = null"
    );
    // exif = {}
    let exif = doc
        .metadata
        .user
        .get("exif")
        .and_then(|v| v.as_object())
        .expect("exif object present");
    assert!(exif.is_empty());
    // Span is Region(0,0,0,0).
    let block = extract_block(&doc);
    assert!(matches!(
        block.common.source_span,
        SourceSpan::Region { x: 0, y: 0, w: 0, h: 0 }
    ));
    // Warning provenance event.
    let warnings: Vec<_> = doc
        .provenance
        .events
        .iter()
        .filter(|e| e.kind == ProvenanceKind::Warning)
        .collect();
    assert_eq!(warnings.len(), 1, "expected exactly one Warning event");
    assert_eq!(warnings[0].agent, "kb-parse-image");
}

#[test]
fn unsupported_bytes_return_err() {
    let bytes = b"not an image at all".to_vec();
    let fx = fixture_for("img/garbage.png", ImageType::Png, &bytes);
    let r = ImageExtractor::new().extract(&fx.ctx(), &bytes);
    assert!(
        r.is_err(),
        "unrecognised format must propagate Err so caller skips"
    );
}

#[test]
fn provenance_events_are_in_order() {
    let bytes = red_100x50_png();
    let fx = fixture_for("a/b.png", ImageType::Png, &bytes);
    let doc = ImageExtractor::new().extract(&fx.ctx(), &bytes).unwrap();
    let kinds: Vec<_> = doc.provenance.events.iter().map(|e| e.kind).collect();
    assert_eq!(
        kinds,
        vec![ProvenanceKind::Discovered, ProvenanceKind::Parsed]
    );
    assert_eq!(doc.provenance.events[0].agent, "kb-source-fs");
    assert_eq!(doc.provenance.events[0].at, fx.asset.discovered_at);
    assert_eq!(doc.provenance.events[1].agent, "kb-parse-image");
}

#[test]
fn determinism_identical_bytes_produce_identical_ids() {
    let bytes = red_100x50_png();
    let fx_a = fixture_for("a/b.png", ImageType::Png, &bytes);
    let fx_b = fixture_for("a/b.png", ImageType::Png, &bytes);
    let extractor = ImageExtractor::new();
    let doc1 = extractor.extract(&fx_a.ctx(), &bytes).unwrap();
    let doc2 = extractor.extract(&fx_b.ctx(), &bytes).unwrap();
    assert_eq!(doc1.doc_id, doc2.doc_id);
    let id1 = &extract_block(&doc1).common.block_id;
    let id2 = &extract_block(&doc2).common.block_id;
    assert_eq!(id1, id2);
}

#[test]
fn snapshot_red_100x50_canonical_document_stable() {
    let bytes = red_100x50_png();
    let fx = fixture_for("photos/red-100x50.png", ImageType::Png, &bytes);
    let extractor = ImageExtractor::new();
    let doc1 = extractor.extract(&fx.ctx(), &bytes).unwrap();
    let doc2 = extractor.extract(&fx.ctx(), &bytes).unwrap();

    let mut j1 = serde_json::to_value(&doc1).unwrap();
    let mut j2 = serde_json::to_value(&doc2).unwrap();
    strip_dynamic_at(&mut j1);
    strip_dynamic_at(&mut j2);
    assert_eq!(
        j1, j2,
        "two extractions of identical bytes must serialise byte-for-byte equal (modulo dynamic timestamps)"
    );

    // Pin a few fields by exact value so a future regression in the
    // ID recipe / serialisation order surfaces here, not at the JSON
    // diff level only.
    assert_eq!(j1["title"], "red-100x50");
    assert_eq!(j1["lang"], "und");
    assert_eq!(j1["parser_version"], kebab_parse_image::PARSER_VERSION);
    assert_eq!(j1["schema_version"], 1);
    assert_eq!(j1["doc_version"], 1);
    assert_eq!(j1["blocks"].as_array().unwrap().len(), 1);
    assert_eq!(j1["blocks"][0]["kind"], "imageref");
    assert_eq!(j1["metadata"]["source_type"], "reference");
    assert_eq!(j1["metadata"]["trust_level"], "primary");
}

#[test]
fn supports_only_image_media_type() {
    let e = ImageExtractor::new();
    assert!(e.supports(&kebab_core::MediaType::Image(ImageType::Png)));
    assert!(e.supports(&kebab_core::MediaType::Image(ImageType::Jpeg)));
    assert!(!e.supports(&kebab_core::MediaType::Markdown));
    assert!(!e.supports(&kebab_core::MediaType::Pdf));
}

#[test]
fn jpeg_with_gps_missing_ref_drops_coordinates() {
    let bytes = exif_gps_no_ref_jpg();
    let fx = fixture_for("img/no-ref.jpg", ImageType::Jpeg, &bytes);
    let doc = ImageExtractor::new().extract(&fx.ctx(), &bytes).unwrap();
    let exif = doc
        .metadata
        .user
        .get("exif")
        .and_then(|v| v.as_object())
        .expect("exif object present");
    // Other whitelisted tags still load (Make / Model / …); GPS is
    // dropped because the *Ref tags are missing.
    assert!(exif.contains_key("make"));
    assert!(
        !exif.contains_key("gps_lat"),
        "missing GPSLatitudeRef must drop gps_lat"
    );
    assert!(
        !exif.contains_key("gps_lon"),
        "missing GPSLongitudeRef must drop gps_lon"
    );
}

#[test]
fn jpeg_with_gps_out_of_range_drops_latitude() {
    let bytes = exif_gps_out_of_range_jpg();
    let fx = fixture_for("img/oor.jpg", ImageType::Jpeg, &bytes);
    let doc = ImageExtractor::new().extract(&fx.ctx(), &bytes).unwrap();
    let exif = doc
        .metadata
        .user
        .get("exif")
        .and_then(|v| v.as_object())
        .expect("exif object present");
    // Latitude (300° + 30' = ~300.5) is outside ±90, so it must be
    // dropped. Longitude (127°) stays in range and survives.
    assert!(
        !exif.contains_key("gps_lat"),
        "out-of-range latitude must be dropped"
    );
    let lon = exif.get("gps_lon").and_then(serde_json::Value::as_f64).expect("gps_lon");
    assert!((lon - 127.0).abs() < 1e-6);
}

#[test]
fn rejects_extract_when_media_type_mismatches() {
    let bytes = red_100x50_png();
    let mut fx = fixture_for("a/b.md", ImageType::Png, &bytes);
    fx.asset.media_type = kebab_core::MediaType::Markdown;
    let r = ImageExtractor::new().extract(&fx.ctx(), &bytes);
    assert!(r.is_err());
}
