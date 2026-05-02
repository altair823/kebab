//! EXIF whitelist extraction for the `ImageExtractor` (P6-1).
//!
//! Only the small set of tags listed in the task spec is captured into
//! `metadata.user["exif"]`. Everything else (thumbnails, maker notes, full
//! camera state) is dropped on the floor so the on-disk wire form keeps a
//! tight PII surface.
//!
//! Whitelisted tags:
//!
//! | tag                | output JSON shape          |
//! |--------------------|----------------------------|
//! | DateTimeOriginal   | `"YYYY-MM-DDTHH:MM:SS"`    |
//! | GPSLatitude / Ref  | merged into `gps_lat: f64` |
//! | GPSLongitude / Ref | merged into `gps_lon: f64` |
//! | Make               | `String`                   |
//! | Model              | `String`                   |
//! | Orientation        | `u32` (1..=8)              |
//! | Software           | `String`                   |
//!
//! Any tag whose source value cannot be parsed into the documented shape
//! is silently dropped — extractor failure must never fail the whole
//! document.

use std::io::Cursor;

use exif::{In, Reader, Tag, Value};
use serde_json::{Map, Value as JsonValue};

/// Read EXIF from `bytes` (any container the `exif` crate understands —
/// JPEG APP1, PNG eXIf, TIFF, HEIF). Always returns a map; if there is no
/// EXIF block (or parsing fails), the map is empty.
pub(crate) fn extract_whitelisted(bytes: &[u8]) -> Map<String, JsonValue> {
    let mut out = Map::new();
    let exif = match Reader::new().read_from_container(&mut Cursor::new(bytes)) {
        Ok(e) => e,
        Err(_) => return out,
    };

    if let Some(s) = ascii_field(&exif, Tag::DateTimeOriginal, In::PRIMARY) {
        if let Some(iso) = exif_datetime_to_iso(&s) {
            out.insert("DateTimeOriginal".into(), JsonValue::String(iso));
        }
    }

    if let Some(lat) = gps_decimal(&exif, Tag::GPSLatitude, Tag::GPSLatitudeRef) {
        if let Some(num) = serde_json::Number::from_f64(lat) {
            out.insert("gps_lat".into(), JsonValue::Number(num));
        }
    }
    if let Some(lon) = gps_decimal(&exif, Tag::GPSLongitude, Tag::GPSLongitudeRef) {
        if let Some(num) = serde_json::Number::from_f64(lon) {
            out.insert("gps_lon".into(), JsonValue::Number(num));
        }
    }

    if let Some(s) = ascii_field(&exif, Tag::Make, In::PRIMARY) {
        out.insert("Make".into(), JsonValue::String(s));
    }
    if let Some(s) = ascii_field(&exif, Tag::Model, In::PRIMARY) {
        out.insert("Model".into(), JsonValue::String(s));
    }
    if let Some(o) = u32_field(&exif, Tag::Orientation, In::PRIMARY) {
        out.insert("Orientation".into(), JsonValue::Number(o.into()));
    }
    if let Some(s) = ascii_field(&exif, Tag::Software, In::PRIMARY) {
        out.insert("Software".into(), JsonValue::String(s));
    }

    out
}

fn ascii_field(exif: &exif::Exif, tag: Tag, ifd: In) -> Option<String> {
    let f = exif.get_field(tag, ifd)?;
    match &f.value {
        Value::Ascii(parts) => {
            // The EXIF 2.x ASCII type is one or more null-terminated C
            // strings. We concatenate without separators since the
            // whitelisted tags here (Make, Model, Software, DateTime)
            // never legitimately split into multiple parts.
            let mut s = String::new();
            for part in parts {
                s.push_str(&String::from_utf8_lossy(part));
            }
            let trimmed = s.trim_matches(char::from(0)).trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        }
        _ => None,
    }
}

fn u32_field(exif: &exif::Exif, tag: Tag, ifd: In) -> Option<u32> {
    let f = exif.get_field(tag, ifd)?;
    match &f.value {
        Value::Short(v) => v.first().map(|x| *x as u32),
        Value::Long(v) => v.first().copied(),
        _ => None,
    }
}

/// EXIF datetime tags use `"YYYY:MM:DD HH:MM:SS"`. We rewrite to ISO-8601
/// `"YYYY-MM-DDTHH:MM:SS"` for downstream consumers (no timezone — EXIF
/// stores local time, and there's a separate OffsetTime tag we don't read).
fn exif_datetime_to_iso(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.len() != 19 {
        return None;
    }
    let bytes = raw.as_bytes();
    if bytes[4] != b':' || bytes[7] != b':' || bytes[10] != b' ' {
        return None;
    }
    // Replace the three structural separators; leave digits + ':' in time
    // section untouched.
    let mut out = String::with_capacity(19);
    out.push_str(&raw[..4]);
    out.push('-');
    out.push_str(&raw[5..7]);
    out.push('-');
    out.push_str(&raw[8..10]);
    out.push('T');
    out.push_str(&raw[11..]);
    Some(out)
}

/// Convert a GPS DMS triple (degrees / minutes / seconds, each
/// `Rational`) into a signed decimal degree using the matching N/S/E/W
/// reference tag. Returns `None` if either tag is missing or shaped
/// unexpectedly.
fn gps_decimal(exif: &exif::Exif, value_tag: Tag, ref_tag: Tag) -> Option<f64> {
    let f = exif.get_field(value_tag, In::PRIMARY)?;
    let dms = match &f.value {
        Value::Rational(r) if r.len() == 3 => r,
        _ => return None,
    };
    let deg = rational_to_f64(&dms[0])?;
    let min = rational_to_f64(&dms[1])?;
    let sec = rational_to_f64(&dms[2])?;
    let mut decimal = deg + min / 60.0 + sec / 3600.0;
    if let Some(reference) = ascii_field(exif, ref_tag, In::PRIMARY) {
        let r = reference.to_ascii_uppercase();
        if r.starts_with('S') || r.starts_with('W') {
            decimal = -decimal;
        }
    }
    if decimal.is_finite() {
        Some(decimal)
    } else {
        None
    }
}

fn rational_to_f64(r: &exif::Rational) -> Option<f64> {
    if r.denom == 0 {
        None
    } else {
        Some(r.num as f64 / r.denom as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn datetime_well_formed_converts_to_iso() {
        let iso = exif_datetime_to_iso("2024:08:15 12:34:56").unwrap();
        assert_eq!(iso, "2024-08-15T12:34:56");
    }

    #[test]
    fn datetime_wrong_separator_rejected() {
        assert!(exif_datetime_to_iso("2024-08-15 12:34:56").is_none());
    }

    #[test]
    fn datetime_short_string_rejected() {
        assert!(exif_datetime_to_iso("2024:08:15").is_none());
    }

    #[test]
    fn extract_on_empty_bytes_yields_empty_map() {
        let m = extract_whitelisted(&[]);
        assert!(m.is_empty());
    }
}
