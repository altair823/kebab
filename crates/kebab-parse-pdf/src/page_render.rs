//! Rasterize PDF pages for OCR, whatever the page is made of.
//!
//! [`page_image`](crate::page_image) pulls a page's embedded JPEG out
//! verbatim, which only works when the page *is* one DCTDecode image.
//! Issue #232: real scanners emit CCITTFaxDecode, JBIG2Decode,
//! FlateDecode and JPXDecode, and Internet Archive scans split the page
//! across a background plus an `/ImageMask`. Every one of those was
//! dropped silently — the text gate correctly said "this page is a scan,
//! OCR it" and then no raster came back.
//!
//! Rendering the page sidesteps the whole question. There is no filter to
//! support, no XObject to pick between, and a page that mixes vector
//! drawing with images comes out as the reader sees it.
//!
//! # Why this is optional
//!
//! pdfium ships as a shared library and no static build is published, so
//! requiring it would end kebab's single-binary property. Instead the
//! renderer binds at runtime: present, and every scanned PDF is covered;
//! absent, and ingest falls back to the DCTDecode path exactly as before
//! and says so. `kebab doctor` reports which of the two is in effect.

use std::path::Path;

use anyhow::{Context, Result};
use pdfium_render::prelude::*;

/// A bound pdfium library. Construct once per run — binding hits the
/// filesystem and the dynamic loader.
pub struct PageRenderer {
    pdfium: Pdfium,
    source: String,
    /// Serializes use of `pdfium`.
    ///
    /// `pdfium-render`'s `thread_safe` feature was not enough on its own:
    /// driving one bound instance from several threads at once aborted
    /// the process with a double-free. Since the ingest loop handles one
    /// PDF at a time anyway, the lock costs nothing today and keeps that
    /// from becoming a memory-safety bug the day the loop parallelizes.
    /// Held by [`RenderedPdf`] for as long as the document is open.
    lock: std::sync::Mutex<()>,
}

impl PageRenderer {
    /// Bind to `libpdfium`, either at an explicit path or wherever the
    /// platform's loader finds it.
    ///
    /// Returns `Err` when the library is missing or unloadable; callers
    /// treat that as "no renderer" rather than a failure, because the
    /// DCTDecode path still works.
    ///
    /// # Bind once
    ///
    /// Call this once and share the result. Binding concurrently from
    /// several threads fails — pdfium's initialization is not reentrant —
    /// and binding repeatedly re-walks the loader path for nothing. The
    /// ingest path binds once per run and hands out an `Arc`.
    pub fn bind(library: Option<&Path>) -> Result<Self> {
        let (bindings, source) = match library {
            Some(path) => (
                Pdfium::bind_to_library(path)
                    .with_context(|| format!("bind pdfium at {}", path.display()))?,
                path.display().to_string(),
            ),
            None => (
                Pdfium::bind_to_system_library()
                    .context("bind pdfium from the system library path")?,
                "system".to_string(),
            ),
        };
        Ok(Self {
            pdfium: Pdfium::new(bindings),
            source,
            lock: std::sync::Mutex::new(()),
        })
    }

    /// Where the bound library came from — an explicit path, or `system`.
    /// Reported by `kebab doctor` so a user can tell which copy is live.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Open a PDF held in memory. The returned handle borrows `self`, so
    /// one bound library serves every document in a run.
    ///
    /// Takes the renderer's lock and holds it until the handle drops, so
    /// a second caller waits rather than racing. One PDF at a time is
    /// what the ingest loop does regardless.
    pub fn open<'a>(&'a self, bytes: &'a [u8], password: Option<&str>) -> Result<RenderedPdf<'a>> {
        // A poisoned lock means some other thread panicked mid-render.
        // pdfium state is not ours to reason about after that, but
        // refusing every later page would turn one bad PDF into a dead
        // capability for the rest of the run — take the guard and carry
        // on, which is what `PoisonError::into_inner` is for.
        let guard = self
            .lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let doc = self
            .pdfium
            .load_pdf_from_byte_slice(bytes, password)
            .context("pdfium: load PDF")?;
        Ok(RenderedPdf { doc, _guard: guard })
    }
}

/// A PDF opened through pdfium, ready to rasterize pages.
///
/// Holds the renderer's lock for its lifetime. Field order is load-
/// bearing: `doc` is declared first so it drops before the guard,
/// because releasing the lock while a document handle is still alive is
/// exactly the race the lock exists to prevent.
pub struct RenderedPdf<'a> {
    doc: PdfDocument<'a>,
    _guard: std::sync::MutexGuard<'a, ()>,
}

impl RenderedPdf<'_> {
    pub fn page_count(&self) -> u32 {
        u32::try_from(self.doc.pages().len()).unwrap_or(0)
    }

    /// Rasterize a 1-based page and return PNG bytes.
    ///
    /// PNG rather than JPEG: OCR reads glyph edges, and JPEG ringing on
    /// the high-contrast black-on-white of a scan is exactly the artifact
    /// that costs recognition accuracy. The bytes go straight to an OCR
    /// engine and are never stored, so the size difference is transient.
    ///
    /// `dpi` is the requested resolution; `max_px` is the ceiling the OCR
    /// engine will accept on either side, and it wins. The page's own
    /// size comes from pdfium, which already parsed it — asking the PDF
    /// dictionary ourselves would mean reimplementing `/MediaBox`
    /// inheritance and `/UserUnit`, and getting it wrong silently
    /// produces a differently-sized render.
    pub fn render_page_png(&self, page_num: u32, dpi: u32, max_px: u32) -> Result<Vec<u8>> {
        let index = i32::try_from(page_num.saturating_sub(1))
            .with_context(|| format!("page {page_num} out of pdfium's index range"))?;
        let page = self
            .doc
            .pages()
            .get(index)
            .with_context(|| format!("pdfium: get page {page_num}"))?;

        let long_edge_pt = page.width().value.max(page.height().value);
        let cap = i32::try_from(long_edge_for_dpi(long_edge_pt, dpi, max_px)).unwrap_or(i32::MAX);

        // All three, and the combination is the point.
        //
        // `set_maximum_*` alone does not scale — it only clamps a render
        // that would otherwise exceed it. With no target and no scale
        // factor pdfium renders at 1pt-to-1px, i.e. 72 DPI, and
        // `render_dpi` silently does nothing. A target alone is what
        // makes a long scan allocate past what pdfium will take: it
        // throws `length_error` from C++ with exceptions disabled, which
        // aborts the process rather than returning an error we could
        // downgrade to a skip. Target sets the scale, the maxima bound
        // it, and together they also keep the aspect ratio — the
        // clamp-only path sets `do_maintain_aspect_ratio = false` and
        // squashes the page.
        let config = PdfRenderConfig::new()
            .set_target_width(cap)
            .set_maximum_width(cap)
            .set_maximum_height(cap);

        let image = page
            .render_with_config(&config)
            .with_context(|| format!("pdfium: render page {page_num}"))?
            .as_image()
            .with_context(|| format!("pdfium: page {page_num} bitmap to image"))?
            .into_rgb8();

        let mut png = Vec::new();
        image
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .with_context(|| format!("encode page {page_num} as PNG"))?;
        Ok(png)
    }
}

/// Long edge in pixels for a page rendered at `dpi`.
///
/// PDF user space is 72 units to the inch, so the multiplier is
/// `dpi / 72`. Clamped to `max_px` because the OCR engine's own pixel
/// budget is the real ceiling — raising `render_dpi` past what the engine
/// accepts would only cost time.
fn long_edge_for_dpi(page_long_edge_pt: f32, dpi: u32, max_px: u32) -> u32 {
    let scaled = (page_long_edge_pt * dpi as f32 / 72.0).round();
    let scaled = if scaled.is_finite() && scaled >= 1.0 {
        scaled as u32
    } else {
        1
    };
    scaled.min(max_px.max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dpi_scales_from_pdf_user_space() {
        // A4 is 842pt on the long edge; 300dpi is the scanning default.
        assert_eq!(long_edge_for_dpi(842.0, 300, 10_000), 3508);
        assert_eq!(long_edge_for_dpi(842.0, 72, 10_000), 842);
    }

    #[test]
    fn dpi_is_capped_by_the_engines_pixel_budget() {
        // The OCR engine will not read more than it will read, so a high
        // render_dpi must not turn into an oversized image it rejects.
        assert_eq!(long_edge_for_dpi(842.0, 600, 2_048), 2_048);
    }

    #[test]
    fn degenerate_inputs_still_produce_a_renderable_size() {
        // A malformed page box must not become a zero-pixel render
        // request, which pdfium rejects outright.
        assert_eq!(long_edge_for_dpi(0.0, 300, 4_096), 1);
        assert_eq!(long_edge_for_dpi(f32::NAN, 300, 4_096), 1);
        assert_eq!(long_edge_for_dpi(842.0, 300, 0), 1);
    }
}
