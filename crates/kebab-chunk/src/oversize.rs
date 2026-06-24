//! Shared oversize-chunk split primitives.
//!
//! Two chunkers need to guarantee that no emitted chunk exceeds the
//! configured `max_chunk_tokens` budget (byte/3 proxy):
//!
//! * `md-heading-v2` ([`crate::md_heading_v2`]) — a generic post-pass over
//!   every block kind (list, code, paragraph, table, image-OCR).
//! * `pdf-page-v1.2` ([`crate::pdf_page_v1`]) — a tier-2 fallback for a
//!   dense scanned page OCR'd into one over-budget run with no sentence /
//!   paragraph boundary for the tier-1 greedy splitter to cut on.
//!
//! Both share the SAME two-tier splitting primitive so the byte-budget
//! bound (and its char-boundary fallback for a no-whitespace page) is
//! defined in exactly one place. This module is the single source of
//! [`BYTES_PER_TOKEN`] for the chunk crate's size proxy — both callers use
//! `crate::oversize::BYTES_PER_TOKEN` so a third divergent copy can't drift.
//!
//! The split disambiguation (chunk_id suffix recipe) and the source-span
//! handling stay with each caller, because they differ: md clones the
//! original chunk's block-granular `source_spans` while pdf narrows the
//! page-relative char range per sub-piece (Page spans are char-indexed, so
//! pdf can be strictly more precise). Only the text-decomposition logic —
//! which is identical — lives here.

/// Bytes-per-token proxy — single source for the chunk crate. 3 bytes/token
/// over-estimates token count for both Korean (E5 ≈ 3) and English
/// (BPE ≈ 4) so chunks sized against this proxy always fit a real
/// tokenizer's budget. Mirrors the `md-heading-v1` calibration; both
/// `md-heading-v2` and `pdf-page-v1.2` reference this constant.
pub(crate) const BYTES_PER_TOKEN: usize = 3;

/// Decompose `text` into sub-pieces each with `len().div_ceil(BYTES_PER_TOKEN)
/// <= budget`. Returns ≥1 piece. The pieces join back to the original with
/// `\n` (line splits) or direct concatenation (char splits within a single
/// line), preserving the full text.
///
/// The returned vec is ordered; concatenating the pieces in order with `\n`
/// reconstructs `text` exactly when `text` contains newlines. For a
/// single-line (newline-free) text, the pieces concatenate directly.
pub(crate) fn text_pieces(text: &str, budget: usize) -> Vec<String> {
    // A budget of 0 is degenerate — treat as 1 to avoid infinite loops.
    let budget = budget.max(1);

    // Split on '\n' first.  A trailing '\n' yields a final empty element
    // which we preserve so joining with '\n' reconstructs the original.
    let lines: Vec<&str> = text.split('\n').collect();

    let mut result: Vec<String> = Vec::new();
    let mut current_piece: Vec<&str> = Vec::new();
    let mut current_bytes: usize = 0;

    for line in &lines {
        let line_bytes = line.len();
        // +1 for the '\n' that re-joins this line to the previous one.
        let sep_bytes = usize::from(!current_piece.is_empty());

        if line_bytes.div_ceil(BYTES_PER_TOKEN) > budget {
            // This single line alone exceeds the budget → flush any
            // accumulated piece, then char-split the line.
            if !current_piece.is_empty() {
                result.push(current_piece.join("\n"));
                current_piece.clear();
                current_bytes = 0;
            }
            result.extend(char_pieces(line, budget));
        } else if !current_piece.is_empty()
            && (current_bytes + sep_bytes + line_bytes).div_ceil(BYTES_PER_TOKEN) > budget
        {
            // Adding this line would push the current piece over budget
            // → flush, then start a new piece with this line.
            result.push(current_piece.join("\n"));
            current_piece = vec![line];
            current_bytes = line_bytes;
        } else {
            // Fits: accumulate.
            current_bytes += sep_bytes + line_bytes;
            current_piece.push(line);
        }
    }
    if !current_piece.is_empty() {
        result.push(current_piece.join("\n"));
    }
    if result.is_empty() {
        // Degenerate (empty text) — return one empty piece so the caller
        // always gets ≥1 chunk.
        result.push(String::new());
    }
    result
}

/// Char-split a single newline-free string `s` into sub-pieces each with
/// `len() <= budget * BYTES_PER_TOKEN`, cutting at UTF-8 char boundaries.
/// Returns ≥1 piece; direct concatenation of all pieces reconstructs `s`.
pub(crate) fn char_pieces(s: &str, budget: usize) -> Vec<String> {
    let byte_budget = budget * BYTES_PER_TOKEN;
    let mut result: Vec<String> = Vec::new();
    let mut piece_start = 0usize;
    let mut piece_bytes = 0usize;

    for (byte_idx, ch) in s.char_indices() {
        let ch_bytes = ch.len_utf8();
        if piece_bytes > 0 && piece_bytes + ch_bytes > byte_budget {
            // Flush current piece.
            result.push(s[piece_start..byte_idx].to_string());
            piece_start = byte_idx;
            piece_bytes = 0;
        }
        piece_bytes += ch_bytes;
    }
    // Remaining tail.
    result.push(s[piece_start..].to_string());
    if result.is_empty() {
        result.push(String::new());
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `text_pieces` on a multi-line string reconstructs with join("\n").
    #[test]
    fn text_pieces_multiline_roundtrip() {
        let lines: Vec<String> = (0..20)
            .map(|i| format!("line {i:02}: some content here"))
            .collect();
        let text = lines.join("\n");
        let budget = 10usize; // small to force splits
        let pieces = text_pieces(&text, budget);
        assert!(pieces.len() >= 2, "must split multi-line text");
        for p in &pieces {
            assert!(
                p.len().div_ceil(BYTES_PER_TOKEN) <= budget,
                "piece exceeds budget: {} bytes / 3 = {} > {budget}",
                p.len(),
                p.len().div_ceil(BYTES_PER_TOKEN)
            );
        }
        assert_eq!(pieces.join("\n"), text, "pieces must reconstruct original");
    }

    /// `char_pieces` on a newline-free string reconstructs by concatenation.
    #[test]
    fn char_pieces_utf8_roundtrip() {
        // Mix of ASCII and 3-byte Korean.
        let s = "hello가나다world마바사".repeat(10);
        let budget = 5usize;
        let pieces = char_pieces(&s, budget);
        assert!(pieces.len() >= 2);
        for p in &pieces {
            assert!(
                p.len() <= budget * BYTES_PER_TOKEN,
                "char piece too long: {} > {}",
                p.len(),
                budget * BYTES_PER_TOKEN
            );
            assert!(std::str::from_utf8(p.as_bytes()).is_ok(), "not valid UTF-8");
        }
        assert_eq!(pieces.concat(), s, "char pieces must reconstruct original");
    }
}
