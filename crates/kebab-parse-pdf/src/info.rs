//! `/Info` dictionary extraction (best-effort).
//!
//! PDFs may carry a `/Info` trailer dictionary with `Title`,
//! `Producer`, `Creator`, etc. Strings are encoded as either
//! PDFDocEncoding (Latin-1 superset) OR UTF-16BE prefixed with the
//! BOM `0xFE 0xFF`. We handle both. Anything else falls back to
//! UTF-8 lossy. All fields are optional — a missing `/Info` dict is
//! not an error.

#[derive(Default)]
pub(crate) struct InfoDict {
    pub title: Option<String>,
    pub producer: Option<String>,
    pub creator: Option<String>,
}

pub(crate) fn extract_info(doc: &lopdf::Document) -> InfoDict {
    let mut out = InfoDict::default();

    let info_obj = match doc.trailer.get(b"Info") {
        Ok(o) => o,
        Err(_) => return out,
    };

    let dict = match info_obj {
        lopdf::Object::Dictionary(d) => Some(d),
        lopdf::Object::Reference(id) => doc
            .get_object(*id)
            .ok()
            .and_then(|o| o.as_dict().ok()),
        _ => None,
    };

    let Some(dict) = dict else { return out };

    out.title = pdf_string(dict, b"Title");
    out.producer = pdf_string(dict, b"Producer");
    out.creator = pdf_string(dict, b"Creator");
    out
}

fn pdf_string(dict: &lopdf::Dictionary, key: &[u8]) -> Option<String> {
    let raw = dict.get(key).ok()?;
    let bytes: &[u8] = match raw {
        lopdf::Object::String(s, _) => s.as_slice(),
        _ => return None,
    };

    // UTF-16BE with BOM (very common for non-ASCII PDF titles).
    if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
        let payload = &bytes[2..];
        if payload.len() % 2 == 0 {
            let units: Vec<u16> = payload
                .chunks_exact(2)
                .map(|c| u16::from_be_bytes([c[0], c[1]]))
                .collect();
            let s = String::from_utf16_lossy(&units);
            if !s.is_empty() {
                return Some(s);
            }
        }
    }

    // PDFDocEncoding overlaps Latin-1 for the printable range we care
    // about, and Latin-1 is byte-identical to UTF-8 only for ASCII;
    // `from_utf8_lossy` is the conservative call here. ASCII-only
    // PDFs (the common case) round-trip cleanly.
    let s = String::from_utf8_lossy(bytes).into_owned();
    if s.is_empty() { None } else { Some(s) }
}
