//! Image-dimension probing for the `ImageExtractor` (P6-1).
//!
//! Reads just enough of the file header to obtain `(width, height, format)`.
//! The contract is:
//!
//! * `Err(_)` — the bytes don't resolve to any known image format. The
//!   caller propagates this so the asset is skipped (per task spec
//!   "Unsupported format → anyhow::Error").
//! * `Ok(DimOutcome::Failed { reason })` — the format is recognised but
//!   dimensions cannot be read (truncated header, oversized image,
//!   decoder error). The caller emits a Warning provenance event and
//!   stores `dimensions = null` in user metadata.
//! * `Ok(DimOutcome::Ok { .. })` — width/height/format read successfully.

use std::io::Cursor;

use anyhow::{Context, Result};
use image::{ImageFormat, ImageReader};

use crate::MAX_DECODE_DIM;

#[derive(Debug, Clone)]
pub(crate) enum DimOutcome {
    Ok {
        width: u32,
        height: u32,
        /// Lowercase format string — `"png"`, `"jpeg"`, `"webp"`, …
        format: &'static str,
    },
    Failed {
        reason: String,
    },
}

pub(crate) fn probe(bytes: &[u8]) -> Result<DimOutcome> {
    let reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .context("reading image header")?;

    let format = reader
        .format()
        .context("unsupported or unrecognised image format")?;
    let format_str = format_label(format);

    match reader.into_dimensions() {
        Ok((w, h)) => {
            if w > MAX_DECODE_DIM || h > MAX_DECODE_DIM {
                Ok(DimOutcome::Failed {
                    reason: format!(
                        "image dimensions {w}x{h} exceed cap {MAX_DECODE_DIM}x{MAX_DECODE_DIM}"
                    ),
                })
            } else {
                Ok(DimOutcome::Ok {
                    width: w,
                    height: h,
                    format: format_str,
                })
            }
        }
        Err(e) => Ok(DimOutcome::Failed {
            reason: format!("decode error: {e}"),
        }),
    }
}

fn format_label(f: ImageFormat) -> &'static str {
    match f {
        ImageFormat::Png => "png",
        ImageFormat::Jpeg => "jpeg",
        ImageFormat::WebP => "webp",
        ImageFormat::Gif => "gif",
        ImageFormat::Tiff => "tiff",
        // The `image` crate's enum is non-exhaustive and may grow new
        // variants in minor versions. Map anything else to a stable
        // catch-all so callers see a deterministic label.
        _ => "other",
    }
}
