//! Content-hash derivation cache key (design 2026-05-31 §3.1).
//!
//! Expensive ingest derivations (embedding vectors, LLM aliases, optional
//! Korean morphological tokens) are cached by the *content hash* of the chunk
//! text so that re-indexing an updated document skips recomputation for any
//! chunk whose text is unchanged — independent of position / `chunk_id`
//! (which is position-based, see `ids::id_for_block`).
//!
//! ```text
//! cache_key = blake3_hex( kind || 0x00 || text_blake3 || 0x00 || version_key )[:32]
//! ```
//! - `text_blake3` = blake3(NFC-normalized UTF-8 bytes of the chunk text).
//! - `kind` ∈ { "embedding", "alias", "korean_tokens" }.
//! - `version_key` folds every §9 version-cascade input for that kind
//!   (model / prompt / tokenizer version). A version bump changes the key →
//!   automatic cache miss → recompute, keeping the cache consistent with the
//!   cascade contract (§3.5 / §3.6).
//!
//! Pure: depends only on `blake3` + `unicode-normalization`. No other
//! `kebab-*` crate is referenced (deps boundary §5).

use crate::normalize::nfc;

/// Derivation-cache key per design §3.1.
///
/// `text` is NFC-normalized before hashing so the same logical content always
/// maps to the same key regardless of Unicode encoding form. `kind` and
/// `version_key` are folded in with `0x00` separators (which cannot occur in
/// hex digests) so distinct kinds / versions never collide.
pub fn derivation_cache_key(kind: &str, text: &str, version_key: &str) -> String {
    let text_blake3 = blake3::hash(nfc(text).as_bytes()).to_hex().to_string();

    let mut hasher = blake3::Hasher::new();
    hasher.update(kind.as_bytes());
    hasher.update(&[0x00]);
    hasher.update(text_blake3.as_bytes());
    hasher.update(&[0x00]);
    hasher.update(version_key.as_bytes());

    hasher.finalize().to_hex().to_string()[..32].to_string()
}

/// Byte-addressed derivation-cache key (§3.1) for **binary** inputs (image /
/// PDF-page bytes feeding OCR / caption). Identical framing to
/// [`derivation_cache_key`] — `kind || 0x00 || content_blake3 || 0x00 ||
/// version_key`, first 32 hex chars — but hashes the raw bytes directly with
/// NO NFC normalization (NFC is meaningless on arbitrary bytes).
pub fn derivation_cache_key_bytes(kind: &str, bytes: &[u8], version_key: &str) -> String {
    let content_blake3 = blake3::hash(bytes).to_hex().to_string();

    let mut hasher = blake3::Hasher::new();
    hasher.update(kind.as_bytes());
    hasher.update(&[0x00]);
    hasher.update(content_blake3.as_bytes());
    hasher.update(&[0x00]);
    hasher.update(version_key.as_bytes());

    hasher.finalize().to_hex().to_string()[..32].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_is_32_hex_chars() {
        let k = derivation_cache_key("embedding", "hello world", "v1");
        assert_eq!(k.len(), 32);
        assert!(k.bytes().all(|b| b.is_ascii_hexdigit()));
    }

    #[test]
    fn same_inputs_same_key() {
        let a = derivation_cache_key("embedding", "러스트 소유권", "model|1|1024");
        let b = derivation_cache_key("embedding", "러스트 소유권", "model|1|1024");
        assert_eq!(a, b);
    }

    #[test]
    fn nfc_normalization_collapses_encoding_forms() {
        // "가" as a precomposed syllable (NFC) vs decomposed jamo (NFD) must
        // hash to the same key after NFC normalization.
        let precomposed = "\u{AC00}"; // 가
        let decomposed = "\u{1100}\u{1161}"; // ᄀ + ᅡ
        assert_ne!(precomposed, decomposed);
        let a = derivation_cache_key("embedding", precomposed, "v1");
        let b = derivation_cache_key("embedding", decomposed, "v1");
        assert_eq!(a, b);
    }

    #[test]
    fn different_kind_different_key() {
        let e = derivation_cache_key("embedding", "same text", "v1");
        let a = derivation_cache_key("alias", "same text", "v1");
        assert_ne!(e, a);
    }

    #[test]
    fn different_version_key_different_key_miss() {
        // §3.6 correctness guard: a version_key change MUST produce a different
        // cache_key (so a stale derivation never gets reused after a cascade
        // bump). This is the most safety-critical invariant of the cache.
        let v1 = derivation_cache_key("embedding", "same text", "modelA|1|1024");
        let v2 = derivation_cache_key("embedding", "same text", "modelA|2|1024");
        assert_ne!(v1, v2);

        // alias prompt_version bump → miss.
        let p1 = derivation_cache_key("alias", "문단", "expansion-v1|8|");
        let p2 = derivation_cache_key("alias", "문단", "expansion-v2|8|");
        assert_ne!(p1, p2);
    }

    #[test]
    fn different_text_different_key() {
        let a = derivation_cache_key("embedding", "text one", "v1");
        let b = derivation_cache_key("embedding", "text two", "v1");
        assert_ne!(a, b);
    }

    #[test]
    fn separator_prevents_field_smearing() {
        // Without the 0x00 separators, ("ab","","c") and ("a","b","c") shaped
        // inputs could collide. The kind/version boundaries must be distinct.
        let a = derivation_cache_key("ab", "x", "c");
        let b = derivation_cache_key("a", "x", "bc");
        assert_ne!(a, b);
    }

    #[test]
    fn bytes_key_is_32_hex_chars() {
        let k = derivation_cache_key_bytes("ocr", &[0u8, 1, 2, 3], "engine|v1");
        assert_eq!(k.len(), 32);
        assert!(k.bytes().all(|b| b.is_ascii_hexdigit()));
    }

    #[test]
    fn bytes_same_inputs_same_key() {
        let a = derivation_cache_key_bytes("ocr", b"\xff\x00image", "paddle|abc");
        let b = derivation_cache_key_bytes("ocr", b"\xff\x00image", "paddle|abc");
        assert_eq!(a, b);
    }

    #[test]
    fn bytes_different_bytes_different_key() {
        let a = derivation_cache_key_bytes("ocr", b"image-a", "v1");
        let b = derivation_cache_key_bytes("ocr", b"image-b", "v1");
        assert_ne!(a, b);
    }

    #[test]
    fn bytes_different_kind_different_key() {
        let o = derivation_cache_key_bytes("ocr", b"same", "v1");
        let c = derivation_cache_key_bytes("caption", b"same", "v1");
        assert_ne!(o, c);
    }

    #[test]
    fn bytes_version_bump_is_miss() {
        // §3.6 safety: a version_key change MUST change the key so a stale OCR
        // result is never reused after an engine/param bump.
        let v1 = derivation_cache_key_bytes("ocr", b"page-bytes", "paddle-abc|st:0.3");
        let v2 = derivation_cache_key_bytes("ocr", b"page-bytes", "paddle-abc|st:0.5");
        assert_ne!(v1, v2);
    }

    #[test]
    fn bytes_no_nfc_raw_hash() {
        // Unlike the text variant, the bytes variant must NOT NFC-normalize —
        // it hashes raw bytes. Two byte strings that would NFC-collapse as text
        // stay distinct as bytes. (Sanity: empty vs non-empty differ.)
        assert_ne!(
            derivation_cache_key_bytes("ocr", b"", "v1"),
            derivation_cache_key_bytes("ocr", b"\x00", "v1")
        );
    }
}
