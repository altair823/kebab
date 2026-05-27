// crates/kebab-parse-pdf/src/text_quality.rs (신규)
//
// Per-page text quality metric — vector PDF 의 valid text vs scanned PDF
// 의 empty vs mojibake (ToUnicode CMap 누락 PUA codepoint) 구분.
// caller (kebab-app::pdf_ocr_apply) 가 threshold 와 비교.

/// Valid char ratio (0.0..=1.0). 빈 string → 0.0.
/// valid := ASCII printable + Hangul (Jamo/Compatibility/Syllables) + CJK + Latin Extended + common Korean punctuation.
pub fn compute_valid_char_ratio(s: &str) -> f32 {
    let mut total = 0u32;
    let mut valid = 0u32;
    for c in s.chars() {
        total += 1;
        if is_valid_text_char(c) { valid += 1; }
    }
    if total == 0 { return 0.0; }
    valid as f32 / total as f32
}

fn is_valid_text_char(c: char) -> bool {
    let cp = c as u32;
    match cp {
        0x0009 | 0x000A | 0x000D => true,                  // tab / LF / CR
        0x0020..=0x007E => true,                            // ASCII printable
        0x00A0..=0x024F => true,                            // Latin-1 Supplement + Latin Extended-A/B
        0x1100..=0x11FF => true,                            // Hangul Jamo
        0x3130..=0x318F => true,                            // Hangul Compatibility Jamo
        0x4E00..=0x9FFF => true,                            // CJK Unified Ideographs
        0xAC00..=0xD7A3 => true,                            // Hangul Syllables
        0x2010..=0x205F => matches!(c,
            '\u{2010}' | '\u{2013}' | '\u{2014}' | '\u{2015}' |
            '\u{2018}' | '\u{2019}' | '\u{201C}' | '\u{201D}' |
            '\u{201E}' | '\u{2026}' | '\u{2027}' | '\u{2032}' | '\u{2033}'
            | '\u{00B7}'),
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
        let s: String = (0xE000u32..0xE010).map(|c| char::from_u32(c).unwrap()).collect();
        let r = compute_valid_char_ratio(&s);
        assert_eq!(r, 0.0);
    }

    #[test]
    fn mixed_half() {
        // 5 valid ASCII + 5 PUA → 0.5
        let mut s = String::from("ABCDE");
        for c in 0xE000u32..0xE005 { s.push(char::from_u32(c).unwrap()); }
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
        let r = compute_valid_char_ratio("\u{1100}\u{1161}");  // Jamo ㄱㅏ
        assert!((r - 1.0).abs() < 1e-6, "got {r}");
    }

    // F4 measurement: valid_ratio = 0.0000 (lopdf returns empty string — ToUnicode CMap 부재로
    // extract_text 가 빈 text 반환). Case A (< 0.3) → active.
    // fixture fix: mojibake.pdf 의 startxref 22130 → 22114 (16-byte offset 오차 수정).
    #[test]
    fn f4_fixture_ratio_under_threshold() {
        use lopdf::Document;
        let bytes = include_bytes!("../tests/fixtures/mojibake.pdf");
        let doc = Document::load_mem(bytes).unwrap();
        let text = doc.extract_text(&[1]).unwrap_or_default();
        let r = compute_valid_char_ratio(&text);
        assert!(r < 0.3, "F4 mojibake fixture 의 valid_ratio < 0.3 (got {r})");
    }
}
