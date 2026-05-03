//! p9-fb-10: CJK / wide-char width helpers.
//!
//! TUI rendering needs **column width**, not char count. ASCII = 1
//! column, Hangul / CJK / fullwidth Latin = 2 columns, combining
//! diacriticals = 0. Naive `s.chars().count()` overflows boxes when
//! the user types `한글` (5 chars × 2 cols = 10 columns — twice
//! what a 5-char ASCII string would be).
//!
//! These helpers wrap `unicode-width` (already a workspace dep used
//! by `library.rs` for the doc-list title column). Centralizing
//! avoids drift between panes that all need the same calculation.
//!
//! ## What this crate does NOT do
//!
//! * **IME composing**: crossterm doesn't surface IME composition
//!   events on any platform (raw `KeyCode::Char(c)` per finalized
//!   jamo). Users on macOS / Windows IME stacks see one char per
//!   commit; on Linux ibus / fcitx similar. The TUI sees the
//!   already-composed character — no preedit handling needed.
//! * **Grapheme clusters** beyond what `unicode-width` covers (e.g.
//!   emoji + skin-tone modifier rendering as 1 visual but 2 chars).
//!   The dominant CJK use case is single-char-per-glyph; emoji
//!   fallback is best-effort via `unicode_width::UnicodeWidthStr`.
//!
//! ## Backspace + boundary safety
//!
//! `String::pop()` is char-aware (returns `Option<char>`, removes
//! one Unicode scalar value, never splits a UTF-8 sequence
//! mid-byte). Every existing pane's `Backspace` handler uses
//! `pop()`, so byte-slicing bugs are out of scope. The helpers
//! below are purely for **rendering width**.

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Display width of `s` in terminal columns. CJK / fullwidth = 2
/// per char, ASCII = 1, combining marks = 0. Sums every char's
/// `unicode-width` reading — same calculation Ratatui uses
/// internally, exposed here so callers can pre-compute layout.
pub fn display_width(s: &str) -> usize {
    s.width()
}

/// Truncate `s` to fit within `max_cols` terminal columns,
/// appending `…` when truncated. The `…` itself counts as 1
/// column. Returns `s` unchanged when it already fits.
///
/// Boundary contract: never splits a multi-byte UTF-8 sequence
/// (`for ch in s.chars()` walks code points). Wide chars are
/// either kept whole or fully omitted — never half-rendered.
pub fn truncate_to_display_width(s: &str, max_cols: usize) -> String {
    if s.width() <= max_cols {
        return s.to_string();
    }
    if max_cols == 0 {
        return String::new();
    }
    let cap = max_cols.saturating_sub(1);
    let mut out = String::new();
    let mut cols = 0usize;
    for ch in s.chars() {
        let w = ch.width().unwrap_or(0);
        if cols + w > cap {
            out.push('…');
            return out;
        }
        cols += w;
        out.push(ch);
    }
    // Loop ended without exceeding cap — but we know s.width() >
    // max_cols (early-return covered the easy case), so the only
    // way to land here is zero-width tail (combining marks). Add
    // the ellipsis and stop.
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// p9-fb-10: ASCII = 1 col per char.
    #[test]
    fn ascii_width_is_one_per_char() {
        assert_eq!(display_width(""), 0);
        assert_eq!(display_width("hello"), 5);
        assert_eq!(display_width("kebab"), 5);
    }

    /// p9-fb-10: Hangul = 2 cols per char (single composed syllable).
    #[test]
    fn hangul_width_is_two_per_char() {
        assert_eq!(display_width("가"), 2);
        assert_eq!(display_width("한글"), 4);
        assert_eq!(display_width("러스트"), 6);
    }

    /// p9-fb-10: mixed ASCII + Hangul sums correctly.
    #[test]
    fn mixed_ascii_hangul_width() {
        // "kb-한글" = 2 ASCII + 1 dash + 2 Hangul × 2 = 5 + 4 = wait
        // "k" 1 + "b" 1 + "-" 1 + "한" 2 + "글" 2 = 7
        assert_eq!(display_width("kb-한글"), 7);
        // "Hello, 세계" = "Hello"(5) + ","(1) + " "(1) + "세"(2) + "계"(2) = 11
        assert_eq!(display_width("Hello, 세계"), 11);
    }

    /// p9-fb-10: Japanese kana / kanji also wide.
    #[test]
    fn japanese_width_is_two_per_char() {
        assert_eq!(display_width("こんにちは"), 10);
        assert_eq!(display_width("漢字"), 4);
    }

    /// p9-fb-10: truncate fits when possible, no allocation.
    #[test]
    fn truncate_returns_same_when_already_fits() {
        assert_eq!(truncate_to_display_width("hello", 5), "hello");
        assert_eq!(truncate_to_display_width("hello", 100), "hello");
        assert_eq!(truncate_to_display_width("한글", 4), "한글");
    }

    /// p9-fb-10: truncate emits ellipsis when overflow.
    #[test]
    fn truncate_emits_ellipsis_on_overflow() {
        assert_eq!(truncate_to_display_width("hello", 4), "hel…");
        assert_eq!(truncate_to_display_width("hello world", 8), "hello w…");
    }

    /// p9-fb-10: truncate respects wide-char boundary — never splits
    /// a Hangul syllable to fit one column.
    #[test]
    fn truncate_does_not_split_wide_char() {
        // "한글테스트" = 10 cols. max_cols=5 → fits "한글" (4) + "…" (1).
        // Cannot include "테" because that would push to 4+2 > 4 (cap).
        let out = truncate_to_display_width("한글테스트", 5);
        assert_eq!(out, "한글…");
        assert_eq!(display_width(&out), 5);
    }

    /// p9-fb-10: max_cols=0 returns empty (degenerate; no room
    /// even for the ellipsis).
    #[test]
    fn truncate_zero_cols_is_empty() {
        assert_eq!(truncate_to_display_width("hello", 0), "");
        assert_eq!(truncate_to_display_width("한글", 0), "");
    }

    /// p9-fb-10: backspace via String::pop is char-aware (sanity
    /// pin — exercises the contract these helpers depend on).
    #[test]
    fn string_pop_handles_hangul_boundary_safely() {
        let mut s = String::from("러스트");
        let popped = s.pop();
        assert_eq!(popped, Some('트'));
        assert_eq!(s, "러스");
        assert_eq!(display_width(&s), 4);
        // Pop again — still char-aware.
        s.pop();
        assert_eq!(s, "러");
        assert_eq!(display_width(&s), 2);
    }
}
