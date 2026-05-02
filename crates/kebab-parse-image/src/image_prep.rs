//! Shared image preparation for OCR / caption / future vision pipelines.
//!
//! Both P6-2 OCR and P6-3 caption need the same pre-LM step: clamp the
//! long edge to a configured max, re-encode as PNG (Ollama's vision
//! channel format), pass through the source bytes when they already
//! satisfy both constraints. Centralising this here keeps the
//! 1px-rounding fix, the PNG passthrough hot path, and the error
//! messages in one place — future modules (PDF page thumbnails,
//! video keyframes, …) plug in without re-deriving the algorithm.

use std::io::Cursor;

use anyhow::{Context, Result};
use image::{ImageFormat, ImageReader};

/// Decode `bytes`, downscale so the long edge is at most `max_long_edge`,
/// and re-encode as PNG. Returns `(png_bytes, final_w, final_h)` so
/// callers that care about the final dimensions (e.g. OCR's
/// `SourceSpan::Region`) get them without re-decoding.
///
/// PNG sources that already fit the cap pass through (zero decodes,
/// just a `Vec` clone). Every other path decodes the image exactly
/// once: a cheap header sniff peeks at the format / dimensions before
/// committing to a decode, so non-PNG passthrough and downscale share
/// the same `decode → optionally resize → re-encode` tail.
pub(crate) fn downscale_to_png(
    bytes: &[u8],
    max_long_edge: u32,
) -> Result<(Vec<u8>, u32, u32)> {
    let reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .context("reading image header")?;
    let format = reader.format();
    let (w, h) = reader
        .into_dimensions()
        .context("reading image dimensions")?;

    let long = w.max(h);

    // Hot path — PNG within budget already matches the wire format we
    // send to vision models, so we ship the bytes verbatim without
    // paying for a decode + re-encode round-trip.
    if long <= max_long_edge && format == Some(ImageFormat::Png) {
        return Ok((bytes.to_vec(), w, h));
    }

    // Every remaining branch needs the pixels — either to re-encode as
    // PNG (non-PNG within budget) or to resize first (over budget).
    // One decode covers both.
    let img = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .context("re-reading image for decode")?
        .decode()
        .context("decoding image")?;

    let (final_w, final_h, final_img) = if long <= max_long_edge {
        (w, h, img)
    } else {
        let scale = max_long_edge as f32 / long as f32;
        let mut new_w = ((w as f32) * scale).round().max(1.0) as u32;
        let mut new_h = ((h as f32) * scale).round().max(1.0) as u32;
        // Independent rounding of the two axes can let `f32`'s
        // round-to-nearest push the long axis one pixel past
        // `max_long_edge` for irrational scales (e.g. `max=1601,
        // long=4001`). Pin the long axis to exactly `max_long_edge`
        // so the doc-comment's "long edge is at most max_long_edge"
        // stays a strict bound.
        if w >= h {
            new_w = new_w.min(max_long_edge);
        } else {
            new_h = new_h.min(max_long_edge);
        }
        let resized =
            img.resize_exact(new_w, new_h, image::imageops::FilterType::Triangle);
        (new_w, new_h, resized)
    };

    let mut out = Cursor::new(Vec::new());
    final_img
        .write_to(&mut out, ImageFormat::Png)
        .context("encoding image as PNG")?;
    Ok((out.into_inner(), final_w, final_h))
}
