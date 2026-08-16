//! Issue #232: every scanned page must produce a raster, whatever its
//! images are encoded with.
//!
//! These tests need a `libpdfium` and are `#[ignore]`d for the same
//! reason `kebab-store-vector`'s AVX suite is: the capability is
//! genuinely optional, and a lane without it must not report a failure
//! it cannot act on. Opt in with
//!
//! ```text
//! KEBAB_TEST_PDFIUM=/path/to/libpdfium.so \
//!   cargo test -p kebab-parse-pdf --test page_render -- --ignored
//! ```
//!
//! The fallback path — what a machine *without* pdfium does — is covered
//! by `page_image.rs` and needs no library, so the behaviour that ships
//! to a bare install is tested unconditionally.

use kebab_parse_pdf::PageRenderer;

/// Bind to the library named by `KEBAB_TEST_PDFIUM`, or the system one.
///
/// Bound once for the whole binary, which is both what production does
/// (one renderer per ingest run) and what pdfium requires — binding it
/// from several threads at once fails, and cargo runs tests in parallel.
///
/// Panics rather than skipping: an `--ignored` run that silently passes
/// without exercising the renderer is worse than no test.
fn renderer() -> &'static PageRenderer {
    static SHARED: std::sync::OnceLock<PageRenderer> = std::sync::OnceLock::new();
    SHARED.get_or_init(|| {
        let explicit = std::env::var("KEBAB_TEST_PDFIUM").ok();
        let path = explicit.as_deref().map(std::path::Path::new);
        PageRenderer::bind(path).expect(
            "these tests require libpdfium; point KEBAB_TEST_PDFIUM at one or \
             install it where the loader finds it",
        )
    })
}

/// PNG signature, so a test can tell "we got an image" from "we got
/// bytes".
const PNG_MAGIC: &[u8] = b"\x89PNG\r\n\x1a\n";

/// The whole point of the change: a page whose image is **not** a single
/// DCTDecode JPEG still rasterizes. `ccitt.pdf` is the fixture the
/// DCTDecode path returns `None` for (see `page_image.rs`), which is
/// exactly the silent content loss issue #232 reported.
#[test]
#[ignore = "requires libpdfium"]
fn a_ccitt_page_rasterizes_even_though_dctdecode_extraction_cannot() {
    let bytes = include_bytes!("fixtures/ccitt.pdf");

    // Precondition: this fixture is one the old path gives up on.
    let doc = lopdf::Document::load_mem(bytes).unwrap();
    assert!(
        kebab_parse_pdf::extract_dctdecode_page_image(&doc, 1)
            .unwrap()
            .is_none(),
        "fixture no longer exercises the gap — pick one the DCTDecode path drops"
    );

    let r = renderer();
    let pdf = r.open(bytes, None).expect("open ccitt.pdf");
    let png = pdf.render_page_png(1, 300, 1_000).expect("render page 1");
    assert!(png.starts_with(PNG_MAGIC), "rendered bytes are not a PNG");
    assert!(
        png.len() > 1_000,
        "suspiciously small render: {} B",
        png.len()
    );
}

/// FlateDecode raw pixels — the other encoding `page_image.rs` pins as
/// unreadable.
#[test]
#[ignore = "requires libpdfium"]
fn a_flate_page_rasterizes_too() {
    let bytes = include_bytes!("fixtures/flate_raw.pdf");
    let r = renderer();
    let pdf = r.open(bytes, None).expect("open flate_raw.pdf");
    let png = pdf.render_page_png(1, 300, 1_000).expect("render page 1");
    assert!(png.starts_with(PNG_MAGIC));
}

/// The DCTDecode case must not regress: rendering has to cover what the
/// passthrough already covered, or the change trades one gap for another.
#[test]
#[ignore = "requires libpdfium"]
fn the_dctdecode_case_still_works_through_the_renderer() {
    let bytes = include_bytes!("fixtures/scanned_page1.pdf");
    let r = renderer();
    let pdf = r.open(bytes, None).expect("open scanned_page1.pdf");
    let png = pdf.render_page_png(1, 300, 1_000).expect("render page 1");
    assert!(png.starts_with(PNG_MAGIC));
}

/// The pixel budget is a ceiling, not a suggestion — an OCR engine that
/// refuses oversized input would otherwise turn a render into a failure.
/// And the page must not be squashed to fit it: pdfium's clamp-only path
/// applies width and height independently and silently changes the
/// aspect ratio, which is not something OCR recovers from.
#[test]
#[ignore = "requires libpdfium"]
fn the_pixel_budget_is_respected_without_distorting_the_page() {
    let r = renderer();
    for fixture in [
        &include_bytes!("fixtures/scanned_page1.pdf")[..],
        &include_bytes!("fixtures/ccitt.pdf")[..],
    ] {
        let pdf = r.open(fixture, None).expect("open");
        let png = pdf.render_page_png(1, 600, 600).expect("render");
        let img = image::load_from_memory(&png).expect("decode render");
        assert!(
            img.width().max(img.height()) <= 600,
            "long edge {} exceeds the 600px budget",
            img.width().max(img.height())
        );
        // Both fixtures are portrait, so a render that kept the shape is
        // taller than it is wide. A square output means the clamp ran
        // without a scale and each side was cut to the cap on its own.
        assert!(
            img.height() > img.width(),
            "portrait page came back {}x{} — aspect ratio was not preserved",
            img.width(),
            img.height()
        );
    }
}

/// `render_dpi` has to actually change the render. Clamping alone leaves
/// pdfium at 1pt-to-1px (72 DPI) no matter what is asked for, and the
/// knob reads as working because a render still comes back.
#[test]
#[ignore = "requires libpdfium"]
fn render_dpi_changes_the_rendered_size() {
    let bytes = include_bytes!("fixtures/scanned_page1.pdf");
    let r = renderer();
    let pdf = r.open(bytes, None).expect("open");

    let size_at = |dpi: u32| {
        let png = pdf.render_page_png(1, dpi, 10_000).expect("render");
        let img = image::load_from_memory(&png).expect("decode");
        img.width().max(img.height())
    };

    let at_72 = size_at(72);
    let at_300 = size_at(300);
    // A4-ish page at 72 DPI is its point size; at 300 it is ~4.17x that.
    assert!(
        at_300 > at_72 * 3,
        "300 DPI produced {at_300}px against 72 DPI's {at_72}px — \
         the dpi argument is not reaching the renderer"
    );
}

/// A page number past the end is a caller error, not a panic. The OCR
/// loop walks pages from lopdf's count, and the two libraries disagreeing
/// about page count must degrade to a skip.
#[test]
#[ignore = "requires libpdfium"]
fn a_page_past_the_end_errors_rather_than_panicking() {
    let bytes = include_bytes!("fixtures/scanned_page1.pdf");
    let r = renderer();
    let pdf = r.open(bytes, None).expect("open");
    assert!(pdf.render_page_png(9_999, 300, 600).is_err());
}

/// Bytes that are not a PDF must come back as an error from `open`, so
/// the caller falls through to the DCTDecode path instead of aborting
/// the ingest.
#[test]
#[ignore = "requires libpdfium"]
fn garbage_input_is_an_error_not_a_crash() {
    let r = renderer();
    assert!(r.open(b"this is not a pdf at all", None).is_err());
}
