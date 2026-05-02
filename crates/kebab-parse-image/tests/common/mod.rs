//! Test fixture builders for `kebab-parse-image`.
//!
//! Images are generated in-memory at test time rather than committed as
//! binary fixtures so:
//!
//! * The test binary stays self-contained — no `include_bytes!` paths to
//!   keep in sync with the workspace layout.
//! * Fixture provenance is auditable from source (anyone reading this
//!   module can see exactly what bytes the tests run against).
//!
//! All builders are deterministic (no time / RNG dependence).

#![allow(dead_code)]

use std::io::Cursor;

use exif::experimental::Writer as ExifWriter;
use exif::{Field, In, Rational, Tag, Value};
use image::{ImageBuffer, Rgb};
use kebab_core::{
    AssetStorage, Checksum, ExtractConfig, ExtractContext, ImageType, MediaType, RawAsset,
    SourceUri, WorkspacePath,
};
use std::path::PathBuf;
use time::OffsetDateTime;

/// 100×50 solid-red PNG, no EXIF.
pub fn red_100x50_png() -> Vec<u8> {
    let img: ImageBuffer<Rgb<u8>, _> = ImageBuffer::from_fn(100, 50, |_, _| Rgb([255, 0, 0]));
    let mut buf = Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::Png)
        .expect("encoding tiny PNG must not fail");
    buf.into_inner()
}

/// 10×10 solid-blue PNG, no EXIF (smaller fixture for cases where
/// dimensions don't matter).
pub fn no_exif_png() -> Vec<u8> {
    let img: ImageBuffer<Rgb<u8>, _> = ImageBuffer::from_fn(10, 10, |_, _| Rgb([0, 0, 255]));
    let mut buf = Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::Png)
        .expect("encoding tiny PNG must not fail");
    buf.into_inner()
}

/// JPEG with embedded EXIF APP1 segment carrying GPS + Make + Model +
/// DateTimeOriginal + Orientation + Software. The base image is a 4×4
/// solid white square — pixel content is irrelevant; the test cares about
/// the EXIF tags.
///
/// Construction: encode JPEG via the `image` crate, then splice an EXIF
/// APP1 segment immediately after SOI (FF D8). The EXIF blob is built
/// with `exif::experimental::Writer`.
pub fn exif_with_gps_jpg() -> Vec<u8> {
    let base = encode_tiny_jpeg();
    let exif_blob = build_exif_blob_gps();

    let mut out = Vec::with_capacity(base.len() + exif_blob.len() + 16);
    // SOI: FF D8.
    out.push(0xFF);
    out.push(0xD8);
    // APP1 marker: FF E1.
    out.push(0xFF);
    out.push(0xE1);
    // APP1 segment length (BE): 2 (length field itself) + 6 ("Exif\0\0")
    // + exif_blob.len(). Pre-validated against the 0xFFFF segment limit.
    let app1_payload_len = 2 + 6 + exif_blob.len();
    assert!(
        app1_payload_len <= u16::MAX as usize,
        "EXIF segment too large for a single APP1"
    );
    out.extend_from_slice(&(app1_payload_len as u16).to_be_bytes());
    out.extend_from_slice(b"Exif\x00\x00");
    out.extend_from_slice(&exif_blob);
    // Append the rest of the JPEG starting just after the original SOI.
    out.extend_from_slice(&base[2..]);
    out
}

fn encode_tiny_jpeg() -> Vec<u8> {
    let img: ImageBuffer<Rgb<u8>, _> = ImageBuffer::from_fn(4, 4, |_, _| Rgb([255, 255, 255]));
    let mut buf = Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::Jpeg)
        .expect("encoding tiny JPEG must not fail");
    buf.into_inner()
}

fn build_exif_blob_gps() -> Vec<u8> {
    let make = Field {
        tag: Tag::Make,
        ifd_num: In::PRIMARY,
        value: Value::Ascii(vec![b"KebabCam\0".to_vec()]),
    };
    let model = Field {
        tag: Tag::Model,
        ifd_num: In::PRIMARY,
        value: Value::Ascii(vec![b"X1\0".to_vec()]),
    };
    let software = Field {
        tag: Tag::Software,
        ifd_num: In::PRIMARY,
        value: Value::Ascii(vec![b"kebab-test\0".to_vec()]),
    };
    let datetime = Field {
        tag: Tag::DateTimeOriginal,
        ifd_num: In::PRIMARY,
        value: Value::Ascii(vec![b"2024:08:15 12:34:56\0".to_vec()]),
    };
    let orientation = Field {
        tag: Tag::Orientation,
        ifd_num: In::PRIMARY,
        value: Value::Short(vec![1]),
    };
    // GPS — 37.5 N, 127.0 E (Seoul-ish). DMS triple: 37°30'0" N,
    // 127°0'0" E. Each component is num/denom rationals.
    let lat = Field {
        tag: Tag::GPSLatitude,
        ifd_num: In::PRIMARY,
        value: Value::Rational(vec![
            Rational { num: 37, denom: 1 },
            Rational { num: 30, denom: 1 },
            Rational { num: 0, denom: 1 },
        ]),
    };
    let lat_ref = Field {
        tag: Tag::GPSLatitudeRef,
        ifd_num: In::PRIMARY,
        value: Value::Ascii(vec![b"N\0".to_vec()]),
    };
    let lon = Field {
        tag: Tag::GPSLongitude,
        ifd_num: In::PRIMARY,
        value: Value::Rational(vec![
            Rational { num: 127, denom: 1 },
            Rational { num: 0, denom: 1 },
            Rational { num: 0, denom: 1 },
        ]),
    };
    let lon_ref = Field {
        tag: Tag::GPSLongitudeRef,
        ifd_num: In::PRIMARY,
        value: Value::Ascii(vec![b"E\0".to_vec()]),
    };

    let mut writer = ExifWriter::new();
    writer.push_field(&make);
    writer.push_field(&model);
    writer.push_field(&software);
    writer.push_field(&datetime);
    writer.push_field(&orientation);
    writer.push_field(&lat);
    writer.push_field(&lat_ref);
    writer.push_field(&lon);
    writer.push_field(&lon_ref);

    let mut blob = Cursor::new(Vec::new());
    writer
        .write(&mut blob, false)
        .expect("EXIF writer must succeed for the small whitelisted set");
    blob.into_inner()
}

/// PNG header magic followed by truncated payload. The format guess
/// succeeds (eight-byte PNG signature is intact) but `into_dimensions`
/// fails because the IHDR chunk is missing.
pub fn corrupt_png() -> Vec<u8> {
    // 8-byte PNG signature only — every byte after is missing.
    vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]
}

/// Build a `RawAsset` + matching workspace_root + `ExtractContext` for
/// the test. `bytes_for_id` is hashed (BLAKE3) to produce the AssetId
/// per §4.2 — this matches what `kebab-source-fs` does in production.
pub struct ImageFixture {
    pub asset: RawAsset,
    pub workspace_root: PathBuf,
    pub config: ExtractConfig,
}

impl ImageFixture {
    pub fn ctx(&self) -> ExtractContext<'_> {
        ExtractContext {
            asset: &self.asset,
            workspace_root: &self.workspace_root,
            config: &self.config,
        }
    }
}

pub fn fixture_for(workspace_path: &str, image_type: ImageType, bytes: &[u8]) -> ImageFixture {
    let blake = blake3::hash(bytes);
    let full_hex = blake.to_hex().to_string();
    let asset_id = kebab_core::id_for_asset(&full_hex);
    let workspace_path = WorkspacePath::new(workspace_path.to_string()).unwrap();
    // Fixed timestamp so determinism tests can compare outputs across runs.
    let discovered_at = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
    let asset = RawAsset {
        asset_id,
        source_uri: SourceUri::File(PathBuf::from(format!("/tmp/{}", workspace_path.0))),
        workspace_path,
        media_type: MediaType::Image(image_type),
        byte_len: bytes.len() as u64,
        checksum: Checksum(full_hex),
        discovered_at,
        stored: AssetStorage::Reference {
            path: PathBuf::from("/tmp/fake"),
            sha: Checksum("0".repeat(64)),
        },
    };
    ImageFixture {
        asset,
        workspace_root: PathBuf::from("/tmp/fake-root"),
        config: ExtractConfig::default(),
    }
}

/// Strip the two non-deterministic provenance timestamps (Parsed +
/// optional Warning) so determinism / snapshot tests can compare JSON
/// without worrying about wall-clock jitter.
pub fn strip_dynamic_at(json: &mut serde_json::Value) {
    if let Some(events) = json
        .get_mut("provenance")
        .and_then(|p| p.get_mut("events"))
        .and_then(|e| e.as_array_mut())
    {
        for (i, ev) in events.iter_mut().enumerate() {
            if i > 0
                && let Some(obj) = ev.as_object_mut()
            {
                obj.insert("at".into(), serde_json::Value::String("<stripped>".into()));
            }
        }
    }
}

