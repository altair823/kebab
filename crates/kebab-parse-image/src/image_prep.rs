//! Shared image preparation for any image-to-LM pipeline.
//!
//! P6-2 OCR and P6-3 caption both need the same pre-LM step: clamp
//! the long edge to a configured max, re-encode as PNG (the wire
//! format vision channels expect — Ollama's `images: [base64, ...]`
//! takes PNG/JPEG, but PNG keeps the alpha + lossless invariant we
//! prefer for hand-drawn / screenshot inputs), pass through the
//! source bytes when they already satisfy both constraints.
//! Centralising this here keeps the 1px-rounding fix, the PNG
//! passthrough hot path, and the error messages in one place —
//! future image-to-LM channels (PDF page thumbnails, video
//! keyframes, …) plug in without re-deriving the algorithm.

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

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::Cursor;

    use image::{ImageBuffer, Rgb};

    /// Solid-colour PNG of the given dimensions. Solid colour
    /// compresses aggressively so even 4001×3001 stays under a few
    /// kilobytes.
    fn solid_png(w: u32, h: u32) -> Vec<u8> {
        let img: ImageBuffer<Rgb<u8>, _> =
            ImageBuffer::from_pixel(w, h, Rgb([0, 0, 255]));
        let mut buf = Cursor::new(Vec::new());
        img.write_to(&mut buf, ImageFormat::Png)
            .expect("encoding solid PNG must not fail");
        buf.into_inner()
    }

    fn solid_jpeg(w: u32, h: u32) -> Vec<u8> {
        let img: ImageBuffer<Rgb<u8>, _> =
            ImageBuffer::from_pixel(w, h, Rgb([255, 255, 255]));
        let mut buf = Cursor::new(Vec::new());
        img.write_to(&mut buf, ImageFormat::Jpeg)
            .expect("encoding solid JPEG must not fail");
        buf.into_inner()
    }

    /// PNG within budget skips the decode + re-encode round-trip
    /// entirely. Source bytes survive byte-for-byte.
    #[test]
    fn png_within_cap_passes_through_zero_decode() {
        let bytes = solid_png(100, 50);
        let (out, w, h) =
            downscale_to_png(&bytes, 1024).expect("PNG passthrough must succeed");
        assert_eq!((w, h), (100, 50));
        assert_eq!(out, bytes, "PNG passthrough must return source bytes verbatim");
    }

    /// JPEG within budget gets re-encoded as PNG (the wire format)
    /// while preserving dimensions.
    #[test]
    fn jpeg_within_cap_reencodes_as_png() {
        let bytes = solid_jpeg(100, 50);
        let (out, w, h) =
            downscale_to_png(&bytes, 1024).expect("JPEG re-encode must succeed");
        assert_eq!((w, h), (100, 50));
        // Byte stream must now start with the PNG magic.
        assert_eq!(
            &out[..8],
            &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A],
            "output must be PNG-encoded after JPEG input"
        );
    }

    /// Pathological irrational scale — `max=1601, long=4001` would let
    /// independent f32 round-to-nearest push the long axis to 1602.
    /// The post-resize clamp pins it back to `max_long_edge`.
    #[test]
    fn long_edge_clamped_strictly_to_max_for_irrational_scale() {
        let bytes = solid_png(4001, 3001);
        let (_out, w, h) =
            downscale_to_png(&bytes, 1601).expect("downscale must succeed");
        let long = w.max(h);
        assert!(long <= 1601, "long edge must be ≤ max, got {long}");
    }

    /// Aspect ratio survives the downscale within 2%.
    #[test]
    fn aspect_ratio_preserved_within_rounding() {
        let bytes = solid_png(4000, 3000);
        let (_out, w, h) =
            downscale_to_png(&bytes, 1024).expect("downscale must succeed");
        let ratio = w as f32 / h as f32;
        assert!(
            (ratio - 4.0 / 3.0).abs() < 0.02,
            "aspect drift: in=4/3 out={}/{}={ratio}",
            w,
            h
        );
    }

    /// Truncated PNG header — format guess succeeds (8-byte signature
    /// intact) but `into_dimensions` fails. Surfaced as Err so
    /// callers can route to "skip + warning" without confusing the
    /// downstream pipeline with a zero-size image.
    #[test]
    fn corrupt_bytes_return_err() {
        let truncated = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        let r = downscale_to_png(&truncated, 1024);
        assert!(r.is_err(), "corrupt PNG must surface as Err");
    }

    /// Unrecognised bytes (not any image format) — header sniff fails
    /// before dimension read.
    #[test]
    fn unrecognised_bytes_return_err() {
        let r = downscale_to_png(b"definitely not an image", 1024);
        assert!(r.is_err(), "non-image bytes must surface as Err");
    }
}
