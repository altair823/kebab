//! `/Info` dictionary extraction (best-effort).
//!
//! PDFs may carry a `/Info` trailer dictionary with `Title`,
//! `Producer`, `Creator`, etc. Strings are encoded as either
//! UTF-16BE prefixed with the BOM `0xFE 0xFF` OR PDFDocEncoding
//! (which agrees with Latin-1 over `0x20–0x7E` + `0xA0–0xFF` and
//! diverges in the `0x18–0x1F` / `0x80–0x9F` ranges). We decode
//! BOM'd strings as proper UTF-16BE; non-BOM strings are decoded
//! as Latin-1 (byte → `char`), which is correct for the common
//! ASCII case and a best-effort approximation for the divergent
//! PDFDocEncoding ranges (full PDFDocEncoding tables aren't worth
//! the maintenance for what is effectively legacy metadata). All
//! fields are optional — a missing `/Info` dict is not an error.

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

    // PDFDocEncoding fallback (no BOM). Direct byte → char cast is
    // a Latin-1 decoder: ASCII (0x00–0x7F) round-trips, and
    // 0xA0–0xFF maps to the matching Unicode code point. `from_utf8_lossy`
    // would have replaced 0x80–0xFF with U+FFFD, mangling legacy
    // PDFDocEncoded titles like "Café".
    let s: String = bytes.iter().map(|&b| b as char).collect();
    if s.is_empty() { None } else { Some(s) }
}
