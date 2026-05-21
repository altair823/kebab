//! p10-3: Tier 3 paragraph + line-window fallback chunker.
//!
//! Splits code/text files on blank-line paragraph boundaries.  Paragraphs
//! with more than 80 lines are further split into 80-line windows with a
//! 20-line overlap (stride 60) — the same oversize pattern used by Tier 1/2
//! chunkers but without AST structure, hence no symbol.
//!
//! Per spec §9.3: all emitted chunks carry `symbol: None`.

use crate::tier2_shared::{build_chunk_no_symbol, policy_hash};
use anyhow::Result;
use kebab_core::{Block, CanonicalDocument, Chunk, ChunkPolicy, ChunkerVersion, Chunker};

pub const VERSION_LABEL: &str = "code-text-paragraph-v1";

/// Lines-per-window for the oversize fallback (Tier 3).
const FALLBACK_LINES_PER_CHUNK: usize = 80;
/// Overlap between consecutive windows.
const FALLBACK_LINES_OVERLAP: usize = 20;
// stride = FALLBACK_LINES_PER_CHUNK - FALLBACK_LINES_OVERLAP = 60.

#[derive(Clone, Copy, Debug, Default)]
pub struct CodeTextParagraphV1Chunker;

impl Chunker for CodeTextParagraphV1Chunker {
    fn chunker_version(&self) -> ChunkerVersion {
        ChunkerVersion(VERSION_LABEL.to_string())
    }

    fn policy_hash(&self, policy: &ChunkPolicy) -> String {
        policy_hash(policy)
    }

    fn chunk(&self, doc: &CanonicalDocument, policy: &ChunkPolicy) -> Result<Vec<Chunk>> {
        // Expect a single Block::Code carrying the full source text.
        let (text, lang_str) = match doc.blocks.first() {
            Some(Block::Code(cb)) => (cb.code.as_str(), cb.lang.as_deref().unwrap_or("")),
            _ => return Ok(vec![]),
        };

        let mut chunks = Vec::new();
        for para in split_paragraphs(text) {
            push_paragraph(&mut chunks, doc, policy, &para, lang_str)?;
        }

        tracing::debug!(
            target: "kebab-chunk",
            doc_id = %doc.doc_id,
            chunks = chunks.len(),
            "code-text-paragraph-v1 chunked",
        );

        Ok(chunks)
    }
}

/// A contiguous run of non-blank lines from the source text.
struct Paragraph {
    /// Lines joined with `\n` (no trailing newline).
    text: String,
    /// 1-indexed line number of the first line in the source file.
    line_start: u32,
    /// 1-indexed line number of the last line in the source file.
    line_end: u32,
}

/// Split `text` into `Paragraph`s separated by blank (all-whitespace) lines.
///
/// Blank lines are treated as boundaries and are NOT included in any
/// paragraph's line range.  Paragraphs that would consist entirely of blank
/// lines are skipped.
fn split_paragraphs(text: &str) -> Vec<Paragraph> {
    let mut paragraphs = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    let mut current_start: Option<u32> = None;

    for (idx, line) in text.lines().enumerate() {
        let line_no = (idx + 1) as u32;
        let is_blank = line.trim().is_empty();
        if is_blank {
            if let Some(start) = current_start.take() {
                let end = start + current.len() as u32 - 1;
                paragraphs.push(Paragraph {
                    text: current.join("\n"),
                    line_start: start,
                    line_end: end,
                });
                current.clear();
            }
        } else {
            if current_start.is_none() {
                current_start = Some(line_no);
            }
            current.push(line);
        }
    }
    // Flush any trailing paragraph not terminated by a blank line.
    if let Some(start) = current_start {
        let end = start + current.len() as u32 - 1;
        paragraphs.push(Paragraph {
            text: current.join("\n"),
            line_start: start,
            line_end: end,
        });
    }
    paragraphs
}

/// Emit one or more chunks for a single paragraph.
///
/// Paragraphs with ≤ `FALLBACK_LINES_PER_CHUNK` lines become a single chunk.
/// Larger paragraphs are split into overlapping windows of
/// `FALLBACK_LINES_PER_CHUNK` lines with stride `FALLBACK_LINES_PER_CHUNK -
/// FALLBACK_LINES_OVERLAP`.  The last window may be shorter.  Window starts
/// are passed as `split_key` so `id_for_chunk` can produce distinct ids
/// across windows.
fn push_paragraph(
    out: &mut Vec<Chunk>,
    doc: &CanonicalDocument,
    policy: &ChunkPolicy,
    para: &Paragraph,
    lang: &str,
) -> Result<()> {
    let n_lines = (para.line_end - para.line_start + 1) as usize;

    if n_lines <= FALLBACK_LINES_PER_CHUNK {
        // Use line_start as split_key so each paragraph gets a distinct
        // chunk_id even when block_ids is empty (no symbol, no AST structure).
        // Without this, all short paragraphs from the same doc share the same
        // base_policy_hash and therefore the same id_for_chunk result.
        out.push(build_chunk_no_symbol(
            doc,
            policy,
            &para.text,
            para.line_start,
            para.line_end,
            lang,
            VERSION_LABEL,
            Some(para.line_start),
        ));
        return Ok(());
    }

    // Oversize: line-window split with overlap.
    let stride = FALLBACK_LINES_PER_CHUNK - FALLBACK_LINES_OVERLAP;
    let lines: Vec<&str> = para.text.lines().collect();
    let mut i = 0usize;
    loop {
        let end = (i + FALLBACK_LINES_PER_CHUNK).min(lines.len());
        let window_text = lines[i..end].join("\n");
        let window_start = para.line_start + i as u32;
        let window_end = para.line_start + (end as u32) - 1;
        // Use window_start as split_key so chunk_ids are unique across windows.
        out.push(build_chunk_no_symbol(
            doc,
            policy,
            &window_text,
            window_start,
            window_end,
            lang,
            VERSION_LABEL,
            Some(window_start),
        ));
        if end == lines.len() {
            break;
        }
        i += stride;
    }
    Ok(())
}
