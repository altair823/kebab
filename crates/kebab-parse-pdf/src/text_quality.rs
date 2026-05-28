// crates/kebab-parse-pdf/src/text_quality.rs (신규)
//
// Per-page text quality metric — vector PDF 의 valid text vs scanned PDF
// 의 empty vs mojibake (ToUnicode CMap 누락 PUA codepoint) 구분.
// caller (kebab-app::pdf_ocr_apply) 가 threshold 와 비교.

// Source of truth: lopdf-0.32.0/src/document.rs:523 (Document::decode_text).
// Only one Unimplemented marker is emitted by lopdf 0.32.0; other CMap
// encodings fall through to `String::from_utf8_lossy(bytes)`, which yields
// PUA / replacement-char territory already covered by `pure_pua_zero`.
// Re-verify on lopdf dependency upgrade.
const MOJIBAKE_MARKERS: &[&str] = &["?Identity-H Unimplemented?"];

/// Valid char ratio (0.0..=1.0). 빈 string → 0.0.
/// valid := ASCII printable + Hangul (Jamo/Compatibility/Syllables) + CJK + Latin Extended + common Korean punctuation.
pub fn compute_valid_char_ratio(s: &str) -> f32 {
    // 1) Strip known mojibake markers before counting valid chars.
    //    Identity-H CID fonts without ToUnicode CMap emit ASCII-only marker
    //    substrings (bypassing PUA detection).
    let mut cleaned: String = s.to_string();
    // `had_marker` guard preserves prior behavior for whitespace-only input
    // (returns ratio of whitespace validity, not 0.0) when no markers found.
    // With markers stripped, the guard enables the trim-empty check.
    let mut had_marker = false;
    for marker in MOJIBAKE_MARKERS {
        if cleaned.contains(marker) {
            had_marker = true;
            cleaned = cleaned.replace(marker, "");
        }
    }
    // 2) Whitespace-only cleaned text → 0.0 (marker-only page).
    if had_marker && cleaned.trim().is_empty() {
        return 0.0;
    }
    // 3) Marker-dominance heuristic — when stripped chars exceed remaining
    //    chars (i.e. marker > 50% of original), the page is "mostly mojibake
    //    with some decodeable page-furniture" (e.g. metro-korea.pdf has
    //    header text in a separate font + body that is Identity-H CID).
    //    Force ratio downward to trigger OCR fallback (parent spec §1.3 intent).
    if had_marker {
        let stripped_chars = s.len().saturating_sub(cleaned.len());
        if stripped_chars > cleaned.len() {
            // Marker dominates — cap ratio at 0.3 (below 0.5 OCR threshold).
            // The 0.3 cap (not 0.0) preserves a small signal that some text
            // WAS decodeable, useful for downstream metrics if ever exposed.
            let mut total = 0u32;
            let mut valid = 0u32;
            for c in cleaned.chars() {
                total += 1;
                if is_valid_text_char(c) {
                    valid += 1;
                }
            }
            let raw_ratio = if total == 0 {
                0.0
            } else {
                valid as f32 / total as f32
            };
            return raw_ratio.min(0.3);
        }
    }
    // 4) Otherwise compute ratio on cleaned text (existing logic).
    let mut total = 0u32;
    let mut valid = 0u32;
    for c in cleaned.chars() {
        total += 1;
        if is_valid_text_char(c) {
            valid += 1;
        }
    }
    if total == 0 {
        return 0.0;
    }
    valid as f32 / total as f32
}

fn is_valid_text_char(c: char) -> bool {
    let cp = c as u32;
    match cp {
        0x0009 | 0x000A | 0x000D => true, // tab / LF / CR
        0x0020..=0x007E => true,          // ASCII printable
        0x00A0..=0x024F => true,          // Latin-1 Supplement + Latin Extended-A/B
        0x1100..=0x11FF => true,          // Hangul Jamo
        0x3130..=0x318F => true,          // Hangul Compatibility Jamo
        0x4E00..=0x9FFF => true,          // CJK Unified Ideographs
        0xAC00..=0xD7A3 => true,          // Hangul Syllables
        0x2010..=0x205F => matches!(
            c,
            '\u{2010}'
                | '\u{2013}'
                | '\u{2014}'
                | '\u{2015}'
                | '\u{2018}'
                | '\u{2019}'
                | '\u{201C}'
                | '\u{201D}'
                | '\u{201E}'
                | '\u{2026}'
                | '\u{2027}'
                | '\u{2032}'
                | '\u{2033}'
                | '\u{00B7}'
        ),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_string_zero() {
        assert_eq!(compute_valid_char_ratio(""), 0.0);
    }

    #[test]
    fn pure_ascii_one() {
        let r = compute_valid_char_ratio("Hello, World! 12345.");
        assert!((r - 1.0).abs() < 1e-6, "got {r}");
    }

    #[test]
    fn pure_hangul_syllables_one() {
        let r = compute_valid_char_ratio("안녕하세요 한글 테스트");
        assert!((r - 1.0).abs() < 1e-6, "got {r}");
    }

    #[test]
    fn pure_pua_zero() {
        // Private Use Area codepoints — mojibake 의 patten.
        // U+E000..U+F8FF 가 valid char list 에 없음.
        let s: String = (0xE000u32..0xE010)
            .map(|c| char::from_u32(c).unwrap())
            .collect();
        let r = compute_valid_char_ratio(&s);
        assert_eq!(r, 0.0);
    }

    #[test]
    fn mixed_half() {
        // 5 valid ASCII + 5 PUA → 0.5
        let mut s = String::from("ABCDE");
        for c in 0xE000u32..0xE005 {
            s.push(char::from_u32(c).unwrap());
        }
        let r = compute_valid_char_ratio(&s);
        assert!((r - 0.5).abs() < 1e-6, "got {r}");
    }

    #[test]
    fn cjk_ideograph_valid() {
        let r = compute_valid_char_ratio("漢字大韓民國");
        assert!((r - 1.0).abs() < 1e-6, "got {r}");
    }

    #[test]
    fn hangul_jamo_valid() {
        let r = compute_valid_char_ratio("\u{1100}\u{1161}"); // Jamo ㄱㅏ
        assert!((r - 1.0).abs() < 1e-6, "got {r}");
    }

    // F4 measurement: pikepdf-fixed fixture (Bug #4). Pages tree 복원 후 lopdf 가
    // page 1 을 로드하고 CID 2-byte code 를 fallback decode → 일부 Latin 범위
    // codepoint 와 충돌 → ratio ≈ 0.375 (non-zero 이지만 production
    // valid_ratio_threshold=0.5 미만). OCR trigger 조건 valid.
    #[test]
    fn f4_fixture_ratio_under_threshold() {
        use lopdf::Document;
        let bytes = include_bytes!("../tests/fixtures/mojibake.pdf");
        let doc = Document::load_mem(bytes).unwrap();
        let text = doc.extract_text(&[1]).unwrap_or_default();
        let r = compute_valid_char_ratio(&text);
        assert!(
            r < 0.5,
            "F4 mojibake fixture 의 valid_ratio < 0.5 (production OCR trigger threshold — got {r})"
        );
    }

    #[test]
    fn identity_h_marker_dominance_caps_ratio_below_threshold() {
        // metro-korea.pdf-class: 20× marker (560 char) + 11 char ASCII header.
        // Without dominance heuristic: ratio = 11/11 = 1.0 (bypasses OCR).
        // With dominance heuristic: ratio ≤ 0.3 (triggers OCR fallback).
        let s = format!("Page 1 of 5 {}", "?Identity-H Unimplemented?".repeat(20));
        let r = compute_valid_char_ratio(&s);
        assert!(
            r <= 0.3,
            "marker-dominant mixed page → ratio ≤ 0.3 (OCR fallback); got {r}"
        );
    }

    #[test]
    fn identity_h_marker_minority_with_long_valid_text_keeps_high_ratio() {
        // Inverse case: short marker noise + long valid text → ratio stays high
        // (no false OCR trigger on otherwise-good pages).
        let header = "x".repeat(200); // 200 char valid ASCII
        let s = format!("{header} ?Identity-H Unimplemented?"); // 1× marker = 26 char
        let r = compute_valid_char_ratio(&s);
        assert!(r > 0.9, "marker-minority page keeps high ratio; got {r}");
    }
}
