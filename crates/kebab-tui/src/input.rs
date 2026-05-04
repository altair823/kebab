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

/// Compute the cursor column for a text-input pane: prompt width +
/// content cursor, summed in `usize` to avoid `u16` overflow, then
/// clamped to fit within `inner_width` columns from `inner_x`.
///
/// Use as:
/// ```ignore
/// f.set_cursor_position((place_cursor_x(inner.x, inner.width, prompt_w, buf.cursor_col()), inner.y));
/// ```
///
/// If a fourth input pane is added, use this helper rather than
/// open-coding the arithmetic — one place to fix if the clamping
/// policy ever changes.
pub fn place_cursor_x(inner_x: u16, inner_width: u16, prompt_w: usize, cursor_col: usize) -> u16 {
    let raw = (inner_x as usize)
        .saturating_add(prompt_w)
        .saturating_add(cursor_col);
    let max = (inner_x as usize)
        .saturating_add(inner_width.saturating_sub(1) as usize);
    raw.min(max).try_into().unwrap_or(u16::MAX)
}

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

/// Text input buffer with mid-string cursor editing. The cursor
/// position is stored as a byte index into `content` (UTF-8 char
/// boundary), and the display column is derived on demand by
/// summing `unicode-width` over the prefix.
///
/// Wide chars (Hangul / Kanji / fullwidth) count 2 columns; ASCII
/// counts 1; combining marks 0. The cursor lives **between** chars,
/// not on them — `cursor_byte == 0` is "before the first char",
/// `cursor_byte == content.len()` is "after the last char".
///
/// `push_char` / `pop_char` operate **at the cursor**, not at the
/// end. When the cursor is at the end (the freshly-typed state),
/// behavior matches the pre-fb-22 append-only buffer. When the
/// cursor is mid-string (after a Left arrow), `push_char` inserts
/// at that position and `pop_char` deletes the char immediately
/// before the cursor (Backspace semantics).
#[derive(Debug, Default, Clone)]
pub struct InputBuffer {
    content: String,
    cursor_byte: usize,
}

impl InputBuffer {
    /// Create an empty buffer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a single char at the cursor and advance the cursor
    /// past it. Zero-width chars (combining marks) leave the
    /// display column unchanged but still extend `content`.
    pub fn push_char(&mut self, ch: char) {
        self.content.insert(self.cursor_byte, ch);
        self.cursor_byte += ch.len_utf8();
    }

    /// Insert a `&str` char-by-char at the cursor. Same width
    /// semantics as `push_char` per element.
    pub fn push_str(&mut self, s: &str) {
        for ch in s.chars() {
            self.push_char(ch);
        }
    }

    /// Delete the char immediately before the cursor (Backspace)
    /// and rewind the cursor onto its byte position. No-op on
    /// empty input or when the cursor is already at the start.
    pub fn pop_char(&mut self) -> Option<char> {
        if self.cursor_byte == 0 {
            return None;
        }
        let prev = self.content[..self.cursor_byte]
            .chars()
            .next_back()
            .expect("cursor_byte > 0 implies at least one prior char");
        let new_byte = self.cursor_byte - prev.len_utf8();
        self.content.remove(new_byte);
        self.cursor_byte = new_byte;
        Some(prev)
    }

    /// Delete the char at the cursor (Delete key). Cursor stays
    /// in place. No-op when the cursor is at the end.
    pub fn delete_after(&mut self) -> Option<char> {
        if self.cursor_byte >= self.content.len() {
            return None;
        }
        Some(self.content.remove(self.cursor_byte))
    }

    /// Move the cursor one char to the left (toward index 0).
    /// Returns true when the cursor moved.
    pub fn move_left(&mut self) -> bool {
        if self.cursor_byte == 0 {
            return false;
        }
        let prev = self.content[..self.cursor_byte]
            .chars()
            .next_back()
            .expect("cursor_byte > 0 implies at least one prior char");
        self.cursor_byte -= prev.len_utf8();
        true
    }

    /// Move the cursor one char to the right (toward end of content).
    /// Returns true when the cursor moved.
    pub fn move_right(&mut self) -> bool {
        if self.cursor_byte >= self.content.len() {
            return false;
        }
        let next = self.content[self.cursor_byte..]
            .chars()
            .next()
            .expect("cursor_byte < len implies at least one trailing char");
        self.cursor_byte += next.len_utf8();
        true
    }

    /// Move the cursor to the start of the buffer.
    pub fn move_home(&mut self) {
        self.cursor_byte = 0;
    }

    /// Move the cursor to the end of the buffer.
    pub fn move_end(&mut self) {
        self.cursor_byte = self.content.len();
    }

    /// Reset to empty.
    pub fn clear(&mut self) {
        self.content.clear();
        self.cursor_byte = 0;
    }

    /// Move the typed string out, leaving the buffer empty (cursor 0).
    /// Convenience for "submit" flows that consume the input.
    pub fn take(&mut self) -> String {
        self.cursor_byte = 0;
        std::mem::take(&mut self.content)
    }

    /// Borrow the typed text.
    pub fn as_str(&self) -> &str {
        &self.content
    }

    /// Cursor column in display-width units — sum of every char's
    /// `unicode-width` reading from the start of the buffer up to
    /// (but not including) the cursor.
    pub fn cursor_col(&self) -> usize {
        self.content[..self.cursor_byte].width()
    }

    /// True when no chars have been typed.
    pub fn is_empty(&self) -> bool {
        self.content.is_empty()
    }
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
        // "kb-한글" = k(1) + b(1) + -(1) + 한(2) + 글(2) = 7
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

    /// p9-fb-10: ASCII typing advances cursor by 1 per char.
    #[test]
    fn input_buffer_ascii_cursor_advances_by_one() {
        let mut b = InputBuffer::new();
        for ch in "hello".chars() {
            b.push_char(ch);
        }
        assert_eq!(b.cursor_col(), 5);
        assert_eq!(b.as_str(), "hello");
    }

    /// p9-fb-10: Hangul typing advances cursor by 2 per char.
    #[test]
    fn input_buffer_hangul_cursor_advances_by_two() {
        let mut b = InputBuffer::new();
        for ch in "한글".chars() {
            b.push_char(ch);
        }
        assert_eq!(b.cursor_col(), 4);
        assert_eq!(b.as_str(), "한글");
    }

    /// p9-fb-10: Backspace rewinds cursor by the popped char's
    /// width — Hangul rewinds by 2, ASCII by 1.
    #[test]
    fn input_buffer_pop_char_rewinds_cursor_by_width() {
        let mut b = InputBuffer::new();
        b.push_str("러스트");
        assert_eq!(b.cursor_col(), 6);
        let popped = b.pop_char();
        assert_eq!(popped, Some('트'));
        assert_eq!(b.cursor_col(), 4);
        assert_eq!(b.as_str(), "러스");
        // Invariant must still hold after pop, not just after push.
        assert_eq!(b.cursor_col(), display_width(b.as_str()));
        b.push_char('a');
        assert_eq!(b.cursor_col(), 5);
        assert_eq!(b.as_str(), "러스a");
    }

    /// p9-fb-10: cursor invariant — cursor_col always equals
    /// display_width(content).
    #[test]
    fn input_buffer_cursor_matches_display_width() {
        let mut b = InputBuffer::new();
        for ch in "Hello, 세계 mixed".chars() {
            b.push_char(ch);
        }
        assert_eq!(b.cursor_col(), display_width(b.as_str()));
    }

    /// p9-fb-10: clear resets both content and cursor.
    #[test]
    fn input_buffer_clear_resets_state() {
        let mut b = InputBuffer::new();
        b.push_str("한글");
        b.clear();
        assert_eq!(b.cursor_col(), 0);
        assert!(b.is_empty());
    }

    /// p9-fb-10: pop_char on empty input returns None and leaves
    /// cursor at 0 (no underflow).
    #[test]
    fn input_buffer_pop_on_empty_is_noop() {
        let mut b = InputBuffer::new();
        assert!(b.pop_char().is_none());
        assert_eq!(b.cursor_col(), 0);
    }

    /// p9-fb-10: take() returns the content and resets state.
    #[test]
    fn input_buffer_take_returns_content_and_resets() {
        let mut b = InputBuffer::new();
        b.push_str("러스트");
        let s = b.take();
        assert_eq!(s, "러스트");
        assert!(b.is_empty());
        assert_eq!(b.cursor_col(), 0);
    }

    /// p9-fb-10: place_cursor_x clamps within the inner area.
    #[test]
    fn place_cursor_x_clamps_to_inner_right_edge() {
        // inner.x=10, width=20, so the rightmost column is 10+20-1 = 29.
        // prompt_w=2, cursor_col=100 (overflow) → clamped to 29.
        assert_eq!(place_cursor_x(10, 20, 2, 100), 29);
    }

    /// p9-fb-10: place_cursor_x preserves position when within bounds.
    #[test]
    fn place_cursor_x_keeps_position_when_within_bounds() {
        assert_eq!(place_cursor_x(10, 20, 2, 5), 17); // 10 + 2 + 5
    }

    /// p9-fb-22: Left arrow moves cursor back by one char (ASCII).
    #[test]
    fn input_buffer_move_left_ascii() {
        let mut b = InputBuffer::new();
        b.push_str("abc");
        assert_eq!(b.cursor_col(), 3);
        assert!(b.move_left());
        assert_eq!(b.cursor_col(), 2);
        assert!(b.move_left());
        assert_eq!(b.cursor_col(), 1);
        assert!(b.move_left());
        assert_eq!(b.cursor_col(), 0);
        assert!(!b.move_left());
        assert_eq!(b.cursor_col(), 0);
    }

    /// p9-fb-22: Left arrow rewinds by full Hangul width (2 cols, 3 bytes).
    #[test]
    fn input_buffer_move_left_hangul() {
        let mut b = InputBuffer::new();
        b.push_str("러스트");
        assert_eq!(b.cursor_col(), 6);
        assert!(b.move_left());
        assert_eq!(b.cursor_col(), 4);
        assert_eq!(b.as_str(), "러스트");
    }

    /// p9-fb-22: Right arrow advances by one char until the end.
    #[test]
    fn input_buffer_move_right_until_end() {
        let mut b = InputBuffer::new();
        b.push_str("ab");
        b.move_home();
        assert_eq!(b.cursor_col(), 0);
        assert!(b.move_right());
        assert_eq!(b.cursor_col(), 1);
        assert!(b.move_right());
        assert_eq!(b.cursor_col(), 2);
        assert!(!b.move_right());
        assert_eq!(b.cursor_col(), 2);
    }

    /// p9-fb-22: Home / End cursor jumps.
    #[test]
    fn input_buffer_move_home_end() {
        let mut b = InputBuffer::new();
        b.push_str("hello");
        b.move_home();
        assert_eq!(b.cursor_col(), 0);
        b.move_end();
        assert_eq!(b.cursor_col(), 5);
    }

    /// p9-fb-22: typing mid-string inserts at cursor (not append).
    #[test]
    fn input_buffer_insert_at_cursor_mid_string() {
        let mut b = InputBuffer::new();
        b.push_str("abc");
        b.move_left();          // cursor between b and c
        b.move_left();          // cursor between a and b
        b.push_char('X');       // insert X between a and b
        assert_eq!(b.as_str(), "aXbc");
        assert_eq!(b.cursor_col(), 2);
    }

    /// p9-fb-22: Backspace mid-string removes the char before the cursor.
    #[test]
    fn input_buffer_backspace_at_cursor() {
        let mut b = InputBuffer::new();
        b.push_str("abcde");
        b.move_left();          // cursor between d and e
        b.move_left();          // cursor between c and d
        b.pop_char();           // delete c
        assert_eq!(b.as_str(), "abde");
        assert_eq!(b.cursor_col(), 2);
    }

    /// p9-fb-22: Backspace at start of buffer is a no-op.
    #[test]
    fn input_buffer_backspace_at_home_is_noop() {
        let mut b = InputBuffer::new();
        b.push_str("abc");
        b.move_home();
        assert!(b.pop_char().is_none());
        assert_eq!(b.as_str(), "abc");
        assert_eq!(b.cursor_col(), 0);
    }

    /// p9-fb-22: Delete key removes the char AT the cursor; cursor stays.
    #[test]
    fn input_buffer_delete_after_at_cursor() {
        let mut b = InputBuffer::new();
        b.push_str("abc");
        b.move_home();
        assert_eq!(b.delete_after(), Some('a'));
        assert_eq!(b.as_str(), "bc");
        assert_eq!(b.cursor_col(), 0);
    }

    /// p9-fb-22: Delete on empty buffer / at end → no-op.
    #[test]
    fn input_buffer_delete_after_at_end_is_noop() {
        let mut b = InputBuffer::new();
        b.push_str("ab");
        // cursor at end
        assert!(b.delete_after().is_none());
        assert_eq!(b.as_str(), "ab");
    }

    /// p9-fb-22: cursor_col stays consistent after mixed mid-string edits
    /// with wide chars.
    #[test]
    fn input_buffer_cursor_col_after_mixed_hangul_edits() {
        let mut b = InputBuffer::new();
        b.push_str("a한b");      // cursor at end, col = 1 + 2 + 1 = 4
        assert_eq!(b.cursor_col(), 4);
        b.move_left();            // before 'b': col = 3
        assert_eq!(b.cursor_col(), 3);
        b.move_left();            // before '한': col = 1
        assert_eq!(b.cursor_col(), 1);
        b.push_char('글');        // insert 글 → "a글한b", cursor between 글 and 한, col = 1 + 2 = 3
        assert_eq!(b.as_str(), "a글한b");
        assert_eq!(b.cursor_col(), 3);
    }

    /// p9-fb-22: take() resets cursor even when it was mid-string.
    #[test]
    fn input_buffer_take_resets_mid_string_cursor() {
        let mut b = InputBuffer::new();
        b.push_str("abc");
        b.move_left();
        let s = b.take();
        assert_eq!(s, "abc");
        assert!(b.is_empty());
        assert_eq!(b.cursor_col(), 0);
    }
}
