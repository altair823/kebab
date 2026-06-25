//! Derivation-cache payload encoding helpers (design 2026-05-31 §3.3).
//!
//! - embedding: `dimensions × f32` little-endian bytes (1024×4 = 4096 B/chunk).
//! - alias / korean_tokens: UTF-8 as-is (handled inline by the caller — no
//!   helper needed, `String::as_bytes` / `String::from_utf8`).
//! - ocr / caption: full-struct serde-JSON (`OcrText` / `ModelCaption`).

use kebab_core::{ModelCaption, OcrText};

/// Encode an `OcrText` as serde-JSON bytes for the `"ocr"` derivation-cache
/// namespace (§3.4). Full struct — a cache hit reconstructs `block.ocr`
/// byte-identically (all fields, not just `.joined`), so the stored block
/// matches a fresh deterministic OCR run.
pub fn encode_ocr_text(o: &OcrText) -> Vec<u8> {
    serde_json::to_vec(o).expect("OcrText serialize (infallible for owned struct)")
}

/// Decode an `OcrText` from the `"ocr"` namespace. `None` on any decode error
/// → caller treats as a cache miss and recomputes (never serves a wrong value).
pub fn decode_ocr_text(payload: &[u8]) -> Option<OcrText> {
    serde_json::from_slice(payload).ok()
}

/// Encode a `ModelCaption` as serde-JSON bytes for the `"caption"` namespace.
pub fn encode_model_caption(c: &ModelCaption) -> Vec<u8> {
    serde_json::to_vec(c).expect("ModelCaption serialize (infallible for owned struct)")
}

/// Decode a `ModelCaption` from the `"caption"` namespace. `None` on error → miss.
pub fn decode_model_caption(payload: &[u8]) -> Option<ModelCaption> {
    serde_json::from_slice(payload).ok()
}

/// Encode an embedding vector as a little-endian `f32` byte string (§3.3).
pub fn encode_embedding(vector: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(vector.len() * 4);
    for &v in vector {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// Decode a little-endian `f32` byte string back into a vector (§3.3).
///
/// Returns `None` if the payload length is not a multiple of 4 (corrupt
/// entry) — the caller treats this as a cache miss and recomputes, so a bad
/// payload never produces a wrong vector.
pub fn decode_embedding(payload: &[u8]) -> Option<Vec<f32>> {
    if payload.len() % 4 != 0 {
        return None;
    }
    Some(
        payload
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect(),
    )
}

#[cfg(test)]
mod ocr_caption_tests {
    use super::*;
    use kebab_core::{ModelCaption, OcrRegion, OcrText};

    fn sample_ocr() -> OcrText {
        OcrText {
            joined: "안녕 OCR\nsecond line".to_string(),
            regions: vec![OcrRegion {
                bbox: (1, 2, 3, 4),
                text: "안녕".to_string(),
                confidence: 0.97,
            }],
            engine: "paddle-onnx".to_string(),
            engine_version: "ppocrv5-mobile-kor-abc123".to_string(),
        }
    }

    #[test]
    fn ocr_text_roundtrips_full_struct() {
        let o = sample_ocr();
        let bytes = encode_ocr_text(&o);
        assert_eq!(decode_ocr_text(&bytes), Some(o));
    }

    #[test]
    fn ocr_decode_garbage_is_none() {
        assert_eq!(decode_ocr_text(b"\xff\xff not json"), None);
    }

    #[test]
    fn model_caption_roundtrips_full_struct() {
        let c = ModelCaption {
            text: "a red square".to_string(),
            model: "gemma4:e4b".to_string(),
            model_version: "ollama/caption-v1".to_string(),
        };
        let bytes = encode_model_caption(&c);
        assert_eq!(decode_model_caption(&bytes), Some(c));
    }

    #[test]
    fn caption_decode_garbage_is_none() {
        assert_eq!(decode_model_caption(b"\x00\x01"), None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips_vector() {
        let v = vec![0.0_f32, 1.5, -2.25, 3.125e10, f32::MIN, f32::MAX];
        let bytes = encode_embedding(&v);
        assert_eq!(bytes.len(), v.len() * 4);
        assert_eq!(decode_embedding(&bytes), Some(v));
    }

    #[test]
    fn empty_vector_roundtrips() {
        assert_eq!(encode_embedding(&[]), Vec::<u8>::new());
        assert_eq!(decode_embedding(&[]), Some(vec![]));
    }

    #[test]
    fn misaligned_payload_is_none() {
        assert_eq!(decode_embedding(&[1, 2, 3]), None);
    }

    #[test]
    fn little_endian_layout_is_fixed() {
        // 1.0_f32 == 0x3F800000, little-endian bytes [0x00,0x00,0x80,0x3F].
        assert_eq!(encode_embedding(&[1.0]), vec![0x00, 0x00, 0x80, 0x3F]);
    }
}
