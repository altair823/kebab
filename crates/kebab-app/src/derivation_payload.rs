//! Derivation-cache payload encoding helpers (design 2026-05-31 §3.3).
//!
//! - embedding: `dimensions × f32` little-endian bytes (1024×4 = 4096 B/chunk).
//! - alias / korean_tokens: UTF-8 as-is (handled inline by the caller — no
//!   helper needed, `String::as_bytes` / `String::from_utf8`).

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
