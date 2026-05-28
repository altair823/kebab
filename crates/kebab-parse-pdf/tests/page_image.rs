// crates/kebab-parse-pdf/tests/page_image.rs (신규)

use kebab_parse_pdf::extract_dctdecode_page_image;
use lopdf::Document;

// happy path — F1 fixture (DCTDecode JPEG passthrough)
#[test]
fn f1_fixture_yields_dctdecode_jpeg_bytes() {
    let bytes = include_bytes!("fixtures/scanned_page1.pdf");
    let doc = Document::load_mem(bytes).unwrap();
    let result = extract_dctdecode_page_image(&doc, 1).unwrap();
    let jpeg = result.expect("F1 의 page 1 이 DCTDecode image 보유");
    assert!(jpeg.starts_with(b"\xFF\xD8"), "JPEG magic missing");
    assert!(
        jpeg.len() > 1000,
        "JPEG bytes too small (got {})",
        jpeg.len()
    );
}

// negative path — F6 fixture (FlateDecode raw pixel — Ok(None))
#[test]
fn flate_raw_fixture_yields_none() {
    let bytes = include_bytes!("fixtures/flate_raw.pdf");
    let doc = Document::load_mem(bytes).unwrap();
    let result = extract_dctdecode_page_image(&doc, 1).unwrap();
    assert!(
        result.is_none(),
        "FlateDecode page 가 Ok(None) 반환 — DCTDecode-only v1 invariant"
    );
}
