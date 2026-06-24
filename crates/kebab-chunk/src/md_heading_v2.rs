//! `md-heading-v2` — heading-aware Markdown chunker with a generic
//! oversize-chunk post-pass.
//!
//! ## Why a new variant (design §9 label-bump)
//!
//! `md-heading-v1` emits every block — code, table, list, paragraph — as
//! a single chunk regardless of size. Pathologically large blocks observed
//! in real Jira exports: a `Block::List` of 76 189 tokens (SERVER-22906,
//! source lines 60–1303, a log dump rendered as bullet points) and a
//! `Block::List` of 20 643 tokens (SERVER-23097). These exceed any
//! practical embedder context window and fail to embed on strict servers.
//!
//! `md-heading-v2`'s main `chunk()` is **byte-identical to v1** — it
//! produces the same chunk set v1 would (same block merging, overlap,
//! heading boundaries). The difference is a **generic post-pass**: after
//! producing v1-equivalent chunks, any chunk whose `token_estimate`
//! exceeds `self.max_chunk_tokens` is replaced by its line-split (then,
//! for single-line content, UTF-8 char-split) sub-pieces. Chunks at/below
//! the budget pass through entirely unchanged, guaranteeing v1 parity for
//! all normal content.
//!
//! ## Split disambiguation
//!
//! All pieces of one oversize chunk share the same `block_ids` (the chunk
//! may span multiple blocks). We disambiguate chunk_ids with a `#seg{i}`
//! suffix on the policy-hash argument to `id_for_chunk`, while storing the
//! bare base hash in `Chunk.policy_hash` — the same recipe
//! `code_rust_ast_v1` uses with `#L{line}`.
//!
//! ## Budget in `policy_hash`
//!
//! The budget changes output, so it MUST participate in the chunk-id
//! cascade (design §9). The shared `ChunkPolicy` has no budget field
//! (adding one would widen the cascade to every chunker including pdf/code).
//! Instead, v2 overrides `policy_hash()` to concatenate the canonical
//! `ChunkPolicy` bytes with the 8 LE bytes of `self.max_chunk_tokens`.
//! v1's `policy_hash` is left untouched.
//!
//! ## Source-span citation
//!
//! Split pieces inherit the **original chunk's `source_spans`** (block-
//! granular citation). This is intentional: we do not have sub-line source
//! map data for arbitrary block kinds, and a citation to the enclosing
//! block/region is always correct — never wrong, just not sub-line-precise.

use kebab_core::{
    Block, BlockId, CanonicalDocument, Chunk, ChunkPolicy, Chunker, ChunkerVersion, DocumentId,
    SourceSpan, id_for_chunk,
};

/// Version label emitted by [`MdHeadingV2Chunker`]. Distinct from
/// `md-heading-v1` so the version cascade (design §9) re-chunks every
/// markdown doc on the first ingest after v2 becomes the default.
const VERSION_LABEL: &str = "md-heading-v2";

/// Bytes-per-token proxy — identical to v1. 3 bytes/token over-estimates
/// token count for both Korean (E5 ≈ 3) and English (BPE ≈ 4) so chunks
/// sized against this proxy always fit a real tokenizer's budget.
const BYTES_PER_TOKEN: usize = 3;

/// Maximum hex characters of the policy hash. 16 hex = 64 bits, matching v1.
const POLICY_HASH_HEX_LEN: usize = 16;

/// Heading-aware Markdown chunker with a generic oversize-chunk post-pass.
///
/// Main `chunk()` is byte-identical to v1. After building the v1-equivalent
/// chunk list, any chunk whose `token_estimate > self.max_chunk_tokens` is
/// replaced by line-split (then char-split for single-line content)
/// sub-pieces each ≤ budget. Chunks at/below budget pass through unchanged.
///
/// Not a unit struct — it carries the split budget threaded from
/// `config.ingest.chunking.max_chunk_tokens`.
#[derive(Clone, Copy, Debug)]
pub struct MdHeadingV2Chunker {
    /// Max byte/3 token estimate per emitted chunk. Any chunk produced by
    /// the v1-equivalent pass whose `token_estimate` exceeds this is split
    /// at line (then UTF-8 char) boundaries. Folded into `policy_hash` so
    /// changing this budget triggers a re-chunk via the cascade (design §9).
    pub max_chunk_tokens: usize,
}

impl Chunker for MdHeadingV2Chunker {
    fn chunker_version(&self) -> ChunkerVersion {
        ChunkerVersion(VERSION_LABEL.to_string())
    }

    /// Policy hash with the v2-only budget folded in.
    ///
    /// We append the 8 LE bytes of `self.max_chunk_tokens` to the canonical
    /// `ChunkPolicy` JSON before hashing. The canonical JSON is
    /// self-delimiting (a balanced JSON object), so there is no structural
    /// ambiguity between the policy bytes and the trailing 8 budget bytes.
    ///
    /// This means two v2 instances with different `max_chunk_tokens` produce
    /// different `policy_hash` values — and therefore different chunk_ids —
    /// for the same document, ensuring a budget change re-indexes all md
    /// docs rather than leaving stale oversized chunks from the previous run.
    ///
    /// # Panics
    ///
    /// Panics if canonical JSON serialization of `ChunkPolicy` fails —
    /// unreachable in practice (see `MdHeadingV1Chunker::policy_hash` for
    /// the full guard rationale).
    fn policy_hash(&self, policy: &ChunkPolicy) -> String {
        let mut bytes = serde_json_canonicalizer::to_vec(policy)
            .expect("canonical JSON serialization of ChunkPolicy must not fail");
        bytes.extend_from_slice(&self.max_chunk_tokens.to_le_bytes());
        let hex = blake3::hash(&bytes).to_hex().to_string();
        hex[..POLICY_HASH_HEX_LEN].to_string()
    }

    fn chunk(&self, doc: &CanonicalDocument, policy: &ChunkPolicy) -> anyhow::Result<Vec<Chunk>> {
        let policy_hash = self.policy_hash(policy);
        let chunker_version = self.chunker_version();

        // ── Phase 1: v1-equivalent chunking ─────────────────────────────
        // Byte-identical to MdHeadingV1Chunker::chunk() — same block-merge
        // logic, same heading boundaries, same overlap seeding, same atomic
        // Code/Table arm. The only callers that see a difference are those
        // that hold chunks exceeding `self.max_chunk_tokens`.
        let v1_chunks = chunk_v1_equivalent(doc, &chunker_version, &policy_hash, policy);

        // ── Phase 2: generic oversize post-pass ─────────────────────────
        // Any chunk at/below budget passes through UNTOUCHED (this is what
        // guarantees byte-identical output to v1 for all normal content).
        // Any chunk above budget is replaced by its split sub-pieces.
        let budget = self.max_chunk_tokens;
        let mut out: Vec<Chunk> = Vec::with_capacity(v1_chunks.len());
        for chunk in v1_chunks {
            // Decide on the chunk's ACTUAL embedded text size, NOT the stored
            // `token_estimate`. For a text chunk the two are equal
            // (`token_estimate == text.len()/BYTES_PER_TOKEN`), so non-image
            // output is unchanged. But ImageRef/AudioRef chunks report
            // `token_estimate = 0` by the image-only convention (build_chunk),
            // while their `text` (alt + OCR + caption) can be arbitrarily large
            // — e.g. a dense screenshot OCR'd to tens of KB. Keying the split
            // on `text.len()` is what the embedder actually receives, so an
            // oversize image/audio chunk splits like any other.
            let embed_tokens = chunk.text.len().div_ceil(BYTES_PER_TOKEN);
            if embed_tokens <= budget {
                out.push(chunk);
            } else {
                // Oversize: split into line-then-char pieces each ≤ budget.
                // The original chunk's block_ids / source_spans / heading_path
                // are shared across all pieces (see module-level doc comment
                // on source-span citation rationale).
                let pieces = split_oversize_chunk(&chunk, budget, &chunker_version, &policy_hash);
                out.extend(pieces);
            }
        }

        tracing::debug!(
            target: "kebab-chunk",
            doc_id = %doc.doc_id,
            chunks = out.len(),
            "md-heading-v2 chunked",
        );

        Ok(out)
    }
}

// ── v1-equivalent chunking (phase 1) ────────────────────────────────────────

/// Produce the same `Vec<Chunk>` that `MdHeadingV1Chunker` would produce.
/// Extracted as a free function so v2's `chunk()` can call it then apply
/// the post-pass cleanly.
fn chunk_v1_equivalent(
    doc: &CanonicalDocument,
    chunker_version: &ChunkerVersion,
    policy_hash: &str,
    policy: &ChunkPolicy,
) -> Vec<Chunk> {
    let mut out: Vec<Chunk> = Vec::new();
    let mut acc = ChunkAcc::default();

    for block in &doc.blocks {
        match block {
            Block::Heading(_) => {
                flush(&mut acc, doc, chunker_version, policy_hash, &mut out);
                acc.push_block(block);
            }
            // Atomic blocks — Code never splits per v1 priority 2; Table
            // stays single per priority 3. The generic post-pass (phase 2)
            // handles splitting if either turns out oversize.
            Block::Code(_) | Block::Table(_) => {
                flush(&mut acc, doc, chunker_version, policy_hash, &mut out);
                let mut single = ChunkAcc::default();
                single.push_block(block);
                flush(&mut single, doc, chunker_version, policy_hash, &mut out);
            }
            Block::ImageRef(_) | Block::AudioRef(_) => {
                flush(&mut acc, doc, chunker_version, policy_hash, &mut out);
                let mut single = ChunkAcc::default();
                single.push_block(block);
                flush(&mut single, doc, chunker_version, policy_hash, &mut out);
            }
            Block::Paragraph(_) | Block::List(_) | Block::Quote(_) => {
                let next_tokens = estimate_block_tokens(block);
                let would_exceed = acc.text_tokens + next_tokens > policy.target_tokens
                    && acc.has_non_heading_content();
                if would_exceed {
                    let overlap_seed =
                        collect_overlap_seed(&acc, policy.overlap_tokens, policy.target_tokens);
                    flush(&mut acc, doc, chunker_version, policy_hash, &mut out);
                    for b in overlap_seed {
                        acc.push_block(b);
                    }
                }
                acc.push_block(block);
            }
        }
    }
    flush(&mut acc, doc, chunker_version, policy_hash, &mut out);
    out
}

// ── Generic oversize-chunk splitter (phase 2) ───────────────────────────────

/// Split an oversize `Chunk` into sub-pieces each with
/// `token_estimate <= budget`.
///
/// ## Splitting strategy (two tiers):
///
/// 1. **Line split**: split `chunk.text` on `'\n'` and greedily accumulate
///    whole lines into pieces each ≤ budget (byte/3). A single line that
///    alone exceeds the budget drops to the next tier.
///
/// 2. **Char split (lone-oversize-line fallback)**: if a single line's
///    byte/3 estimate still exceeds `budget`, split it at UTF-8 char
///    boundaries, accumulating chars until the NEXT char would push the
///    piece over `budget * BYTES_PER_TOKEN` bytes. This guarantees the
///    budget bound for every piece, including pathological inputs like a
///    mega-paragraph rendered as one line.
///
/// ## Piece metadata
///
/// Each piece is a new `Chunk` that inherits:
/// - `doc_id`, `block_ids`, `heading_path`, `chunker_version`,
///   `source_spans`: copied from the original (block-granular citation).
/// - `text`: the piece text.
/// - `token_estimate`: piece `len().div_ceil(BYTES_PER_TOKEN)`.
/// - `tokenized_korean_text`: recomputed for the piece.
/// - `policy_hash`: the BARE base hash (the `#seg{i}` suffix lives only
///   in the id-input hash, never in the persisted field).
/// - `chunk_id`: `id_for_chunk(doc_id, version, block_ids,
///   "{base_hash}#seg{i}")` where `i` is the 0-based piece index —
///   the sole id disambiguator since `block_ids` is identical across pieces.
fn split_oversize_chunk(
    chunk: &Chunk,
    budget: usize,
    chunker_version: &ChunkerVersion,
    base_policy_hash: &str,
) -> Vec<Chunk> {
    // Collect all sub-piece texts first.
    let pieces: Vec<String> = text_pieces(&chunk.text, budget);

    // Safety invariant: split must produce ≥1 piece.
    debug_assert!(!pieces.is_empty(), "text_pieces must return ≥1 piece");

    pieces
        .into_iter()
        .enumerate()
        .map(|(i, text)| {
            let id_hash = format!("{base_policy_hash}#seg{i}");
            let chunk_id =
                id_for_chunk(&chunk.doc_id, chunker_version, &chunk.block_ids, &id_hash);
            let token_estimate = text.len().div_ceil(BYTES_PER_TOKEN);
            Chunk {
                chunk_id,
                doc_id: chunk.doc_id.clone(),
                block_ids: chunk.block_ids.clone(),
                tokenized_korean_text: crate::tokenize_korean_morphological(&text),
                text,
                heading_path: chunk.heading_path.clone(),
                // Source spans are cloned from the original chunk: block-
                // granular citation is always correct for a split piece (it
                // cites the enclosing block/region). We don't have sub-line
                // source-map data for arbitrary block kinds.
                source_spans: chunk.source_spans.clone(),
                token_estimate,
                chunker_version: chunker_version.clone(),
                // Store the BARE base hash; the #seg suffix is id-input only.
                policy_hash: base_policy_hash.to_string(),
            }
        })
        .collect()
}

/// Decompose `text` into sub-pieces each with `len().div_ceil(BYTES_PER_TOKEN)
/// <= budget`. Returns ≥1 piece. The pieces join back to the original with
/// `\n` (line splits) or direct concatenation (char splits within a single
/// line), preserving the full text.
///
/// The returned vec is ordered; concatenating the pieces in order with `\n`
/// reconstructs `text` exactly when `text` contains newlines. For a
/// single-line (newline-free) text, the pieces concatenate directly.
fn text_pieces(text: &str, budget: usize) -> Vec<String> {
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
fn char_pieces(s: &str, budget: usize) -> Vec<String> {
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

// ── v1-equivalent helpers (verbatim from md_heading_v1) ─────────────────────

#[derive(Default)]
struct ChunkAcc<'a> {
    blocks: Vec<&'a Block>,
    text_tokens: usize,
}

impl<'a> ChunkAcc<'a> {
    fn push_block(&mut self, b: &'a Block) {
        self.text_tokens += estimate_block_tokens(b);
        self.blocks.push(b);
    }

    fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    fn has_non_heading_content(&self) -> bool {
        self.blocks.iter().any(|b| !matches!(b, Block::Heading(_)))
    }
}

fn flush(
    acc: &mut ChunkAcc<'_>,
    doc: &CanonicalDocument,
    chunker_version: &ChunkerVersion,
    policy_hash: &str,
    out: &mut Vec<Chunk>,
) {
    if acc.is_empty() {
        return;
    }
    let blocks = std::mem::take(&mut acc.blocks);
    acc.text_tokens = 0;
    out.push(build_chunk(doc, &blocks, chunker_version, policy_hash));
}

fn collect_overlap_seed<'a>(
    acc: &ChunkAcc<'a>,
    overlap_tokens: usize,
    target_tokens: usize,
) -> Vec<&'a Block> {
    let seed_budget = overlap_tokens.min(target_tokens / 2);
    if seed_budget == 0 {
        return Vec::new();
    }
    let mut taken = Vec::new();
    let mut budget = seed_budget;
    for b in acc.blocks.iter().rev() {
        if matches!(b, Block::Heading(_)) {
            continue;
        }
        let est = estimate_block_tokens(b);
        if est > budget && !taken.is_empty() {
            break;
        }
        taken.push(*b);
        budget = budget.saturating_sub(est);
        if budget == 0 {
            break;
        }
    }
    taken.reverse();
    taken
}

fn build_chunk(
    doc: &CanonicalDocument,
    blocks: &[&Block],
    chunker_version: &ChunkerVersion,
    policy_hash: &str,
) -> Chunk {
    debug_assert!(!blocks.is_empty(), "build_chunk requires ≥1 block");

    let block_ids: Vec<BlockId> = blocks.iter().map(|b| common(b).block_id.clone()).collect();
    let source_spans: Vec<SourceSpan> = blocks
        .iter()
        .map(|b| common(b).source_span.clone())
        .collect();

    let heading_path = match blocks[0] {
        Block::Heading(h) => {
            let mut path = h.common.heading_path.clone();
            path.push(h.text.clone());
            path
        }
        _ => common(blocks[0]).heading_path.clone(),
    };

    let mut text = String::new();
    let mut is_image_or_audio_only = true;
    for (i, b) in blocks.iter().enumerate() {
        let part = render_block_text(b);
        if !matches!(b, Block::ImageRef(_) | Block::AudioRef(_)) {
            is_image_or_audio_only = false;
        }
        if i > 0 {
            text.push_str("\n\n");
        }
        text.push_str(&part);
    }

    let token_estimate = if is_image_or_audio_only {
        0
    } else {
        text.len().div_ceil(BYTES_PER_TOKEN)
    };

    let chunk_id = id_for_chunk(&doc.doc_id, chunker_version, &block_ids, policy_hash);

    Chunk {
        chunk_id,
        doc_id: DocumentId(doc.doc_id.0.clone()),
        block_ids,
        tokenized_korean_text: crate::tokenize_korean_morphological(&text),
        text,
        heading_path,
        source_spans,
        token_estimate,
        chunker_version: chunker_version.clone(),
        policy_hash: policy_hash.to_string(),
    }
}

fn render_block_text(b: &Block) -> String {
    match b {
        Block::Heading(h) => h.text.clone(),
        Block::Paragraph(p) | Block::Quote(p) => p.text.clone(),
        Block::List(l) => l
            .items
            .iter()
            .map(|it| it.text.as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        Block::Code(c) => c.code.clone(),
        Block::Table(t) => {
            let mut s = t.headers.join(" | ");
            for row in &t.rows {
                s.push('\n');
                s.push_str(&row.join(" | "));
            }
            s
        }
        Block::ImageRef(i) => {
            let alt = if i.alt.is_empty() {
                i.src
                    .rsplit('/')
                    .next()
                    .filter(|s| !s.is_empty())
                    .unwrap_or("[image]")
                    .to_string()
            } else {
                i.alt.clone()
            };
            let ocr = i.ocr.as_ref().map_or("", |o| o.joined.as_str());
            let cap = i.caption.as_ref().map_or("", |c| c.text.as_str());
            [alt.as_str(), ocr, cap]
                .iter()
                .filter(|s| !s.is_empty())
                .copied()
                .collect::<Vec<_>>()
                .join("\n\n")
        }
        Block::AudioRef(_) => String::new(),
    }
}

fn estimate_block_tokens(b: &Block) -> usize {
    match b {
        Block::ImageRef(_) | Block::AudioRef(_) => 0,
        _ => render_block_text(b).len().div_ceil(BYTES_PER_TOKEN),
    }
}

fn common(b: &Block) -> &kebab_core::CommonBlock {
    match b {
        Block::Heading(h) => &h.common,
        Block::Paragraph(t) | Block::Quote(t) => &t.common,
        Block::List(l) => &l.common,
        Block::Code(c) => &c.common,
        Block::Table(t) => &t.common,
        Block::ImageRef(i) => &i.common,
        Block::AudioRef(a) => &a.common,
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MdHeadingV1Chunker;
    use kebab_core::{
        AssetId, CodeBlock, CommonBlock, HeadingBlock, ImageRefBlock, Lang, ListBlock,
        Metadata, OcrText, Provenance, SourceType, TextBlock, TrustLevel, WorkspacePath,
        id_for_block,
    };
    use time::OffsetDateTime;

    // ── Document / block helpers ─────────────────────────────────────────

    fn make_doc(blocks: Vec<Block>) -> CanonicalDocument {
        CanonicalDocument {
            doc_id: kebab_core::DocumentId("d".repeat(32)),
            source_asset_id: AssetId("a".repeat(32)),
            workspace_path: WorkspacePath::new("notes/test.md".into()).unwrap(),
            title: "Test".into(),
            lang: Lang("en".into()),
            blocks,
            metadata: Metadata {
                aliases: vec![],
                tags: vec![],
                created_at: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
                updated_at: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
                source_type: SourceType::Note,
                trust_level: TrustLevel::Primary,
                user_id_alias: None,
                user: Default::default(),
                repo: None,
                git_branch: None,
                git_commit: None,
                code_lang: None,
                source_id: None,
            },
            provenance: Provenance { events: vec![] },
            parser_version: kebab_core::ParserVersion("test-parser-0".into()),
            schema_version: 1,
            doc_version: 1,
            last_chunker_version: None,
            last_embedding_version: None,
        }
    }

    fn doc_id() -> kebab_core::DocumentId {
        kebab_core::DocumentId("d".repeat(32))
    }

    fn span(start: u32, end: u32) -> SourceSpan {
        SourceSpan::Line { start, end }
    }

    fn common_for(
        kind: &str,
        heading_path: &[String],
        ordinal: u32,
        s: SourceSpan,
    ) -> CommonBlock {
        CommonBlock {
            block_id: id_for_block(&doc_id(), kind, heading_path, ordinal, &s),
            heading_path: heading_path.to_vec(),
            source_span: s,
        }
    }

    fn heading(level: u8, text: &str, ordinal: u32, line: u32) -> Block {
        Block::Heading(HeadingBlock {
            common: common_for("heading", &[], ordinal, span(line, line)),
            level,
            text: text.into(),
        })
    }

    fn paragraph(text: &str, heading_path: &[&str], ordinal: u32, line: u32) -> Block {
        let hp: Vec<String> = heading_path.iter().map(|s| (*s).into()).collect();
        Block::Paragraph(TextBlock {
            common: common_for("paragraph", &hp, ordinal, span(line, line)),
            text: text.into(),
            inlines: vec![],
        })
    }

    /// Build a `Block::List` whose rendered text is `items.join("\n")`.
    /// Each item becomes a `TextBlock`; `render_block_text` joins them
    /// with `"\n"` matching the v1/v2 list rendering contract.
    fn list_block(items: &[&str], heading_path: &[&str], ordinal: u32, s: SourceSpan) -> Block {
        let hp: Vec<String> = heading_path.iter().map(|s| (*s).into()).collect();
        Block::List(ListBlock {
            common: common_for("list", &hp, ordinal, s.clone()),
            ordered: false,
            items: items
                .iter()
                .enumerate()
                .map(|(i, t)| TextBlock {
                    common: common_for("list_item", &hp, i as u32, s.clone()),
                    text: (*t).to_string(),
                    inlines: vec![],
                })
                .collect(),
        })
    }

    fn code_block(code: &str, heading_path: &[&str], ordinal: u32, s: SourceSpan) -> Block {
        let hp: Vec<String> = heading_path.iter().map(|s| (*s).into()).collect();
        Block::Code(CodeBlock {
            common: common_for("code", &hp, ordinal, s),
            lang: Some("rust".into()),
            code: code.into(),
        })
    }

    /// Build a `Block::ImageRef` whose OCR `joined` text is `ocr_text`.
    /// `render_block_text(ImageRef)` = `[alt, ocr.joined, caption]` joined by
    /// `\n\n`, and `build_chunk` forces `token_estimate = 0` for an
    /// image-only chunk — so this models a dense screenshot whose OCR output
    /// is large yet reports a zero token_estimate.
    fn image_block_with_ocr(ocr_text: &str, ordinal: u32, line: u32) -> Block {
        Block::ImageRef(ImageRefBlock {
            common: common_for("imageref", &[], ordinal, span(line, line)),
            asset_id: None,
            src: "dense.png".into(),
            alt: "dense".into(),
            ocr: Some(OcrText {
                joined: ocr_text.into(),
                regions: vec![],
                engine: "paddle-onnx".into(),
                engine_version: "ppocrv5".into(),
            }),
            caption: None,
        })
    }

    fn policy_v2(budget: usize) -> (ChunkPolicy, MdHeadingV2Chunker) {
        let p = ChunkPolicy {
            target_tokens: 500,
            overlap_tokens: 80,
            respect_markdown_headings: true,
            chunker_version: ChunkerVersion(VERSION_LABEL.into()),
        };
        let c = MdHeadingV2Chunker {
            max_chunk_tokens: budget,
        };
        (p, c)
    }

    fn policy_v1() -> ChunkPolicy {
        ChunkPolicy {
            target_tokens: 500,
            overlap_tokens: 80,
            respect_markdown_headings: true,
            chunker_version: ChunkerVersion("md-heading-v1".into()),
        }
    }

    // ── Tests ─────────────────────────────────────────────────────────────

    #[test]
    fn chunker_version_is_md_heading_v2() {
        let (_, c) = policy_v2(4000);
        assert_eq!(c.chunker_version(), ChunkerVersion(VERSION_LABEL.to_string()));
    }

    /// Regression for the image-OCR hole found during the matching dogfood
    /// re-test: an `ImageRef` chunk reports `token_estimate = 0` (image-only
    /// convention) even when its OCR text is huge. The oversize post-pass must
    /// key the split on the ACTUAL embedded `text` length, not the stored
    /// `token_estimate`, or a dense screenshot's OCR would reach the embedder
    /// whole and fail on a strict backend (e.g. AMD Lemonade).
    #[test]
    fn oversize_image_ocr_chunk_splits() {
        let budget = 200;
        let (policy, chunker) = policy_v2(budget);
        // ~3000 bytes of OCR text → byte/3 ≈ 1000 tokens ≫ 200 budget,
        // but the source ImageRef chunk's token_estimate is 0.
        let ocr = "WiredTiger cache eviction stalls under heavy write load. ".repeat(54);
        let doc = make_doc(vec![image_block_with_ocr(&ocr, 0, 1)]);
        let chunks = chunker.chunk(&doc, &policy).unwrap();

        // The single image chunk's stored token_estimate is 0 (image-only)...
        assert_eq!(
            estimate_block_tokens(&Block::ImageRef(match &doc.blocks[0] {
                Block::ImageRef(i) => i.clone(),
                _ => unreachable!(),
            })),
            0,
            "ImageRef token_estimate must be 0 (image-only convention)"
        );
        // ...yet the oversize OCR text is split into ≥2 pieces, each ≤ budget.
        assert!(
            chunks.len() >= 2,
            "oversize image OCR must split, got {} chunk(s)",
            chunks.len()
        );
        for ch in &chunks {
            assert!(
                ch.text.len().div_ceil(BYTES_PER_TOKEN) <= budget,
                "every split piece's embedded text must be ≤ budget"
            );
        }
        // chunk_ids are unique across the image-OCR split pieces.
        let mut ids: Vec<String> = chunks.iter().map(|c| c.chunk_id.0.clone()).collect();
        let n = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), n, "split piece chunk_ids must be unique");
    }

    /// The primary regression the pivot was designed to fix: a Jira-style
    /// log dump rendered as a `Block::List` (items joined by `\n`) whose
    /// rendered text vastly exceeds any embedder budget.
    ///
    /// Asserts: ≥2 chunks produced; every chunk ≤ budget; concatenating
    /// pieces' text with `\n` reconstructs the list's full rendered text;
    /// all chunk_ids are unique.
    #[test]
    fn oversize_list_block_splits() {
        // 60 items × 30 bytes ≈ 1800 bytes ≈ 600 tokens, well above budget=50.
        let items: Vec<String> = (0..60)
            .map(|i| format!("log line {i:03}: some event description here"))
            .collect();
        let item_refs: Vec<&str> = items.iter().map(String::as_str).collect();
        let blocks = vec![list_block(&item_refs, &[], 0, span(1, 60))];
        let doc = make_doc(blocks);
        let (p, c) = policy_v2(50);
        let chunks = c.chunk(&doc, &p).unwrap();

        assert!(
            chunks.len() >= 2,
            "oversize list must split, got {}: {chunks:#?}",
            chunks.len()
        );
        for ch in &chunks {
            assert!(
                ch.token_estimate <= 50,
                "piece exceeds budget: {} > 50",
                ch.token_estimate
            );
        }

        // Concatenating piece texts with '\n' reconstructs the original
        // rendered list text (items.join("\n")).
        let expected = item_refs.join("\n");
        let rejoined = chunks
            .iter()
            .map(|c| c.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(rejoined, expected, "pieces must reconstruct the list text");

        // Chunk_ids unique.
        let mut ids: Vec<&str> = chunks.iter().map(|c| c.chunk_id.0.as_str()).collect();
        let n = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), n, "chunk_ids must be unique");
    }

    /// Single-line paragraph that vastly exceeds budget when rendered —
    /// triggers the UTF-8 char-split fallback. Includes multibyte Korean
    /// to verify no mid-codepoint split.
    #[test]
    fn oversize_paragraph_single_line_char_splits() {
        // "가나다라마바사아자차카타파하" is 14 Korean syllables × 3 bytes each.
        // Repeat to well exceed a small budget.
        let korean_base = "가나다라마바사아자차카타파하";
        let text = korean_base.repeat(30); // 420 chars × 3 bytes = 1260 bytes ≈ 420 tokens
        let blocks = vec![paragraph(&text, &[], 0, 1)];
        let doc = make_doc(blocks);
        let (p, c) = policy_v2(20); // budget = 20 tokens ≈ 60 bytes
        let chunks = c.chunk(&doc, &p).unwrap();

        assert!(
            chunks.len() >= 2,
            "single-line oversize paragraph must char-split, got {}: {chunks:#?}",
            chunks.len()
        );
        for ch in &chunks {
            assert!(
                ch.token_estimate <= 20,
                "piece exceeds budget: {} > 20",
                ch.token_estimate
            );
            // Verify no replacement chars from a mid-codepoint cut.
            assert!(
                !ch.text.contains('\u{FFFD}'),
                "replacement char detected — mid-codepoint split!"
            );
            // Every piece is valid UTF-8 (Rust strings always are if
            // they are constructed correctly, but this confirms no panic).
            assert!(std::str::from_utf8(ch.text.as_bytes()).is_ok());
        }

        // Concatenation reconstructs the original (no newlines introduced).
        let rejoined: String = chunks.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(rejoined, text, "char pieces must reconstruct original");
    }

    /// A code block that exceeds budget is now handled by the generic
    /// post-pass (not a code-specific branch). Same guarantees apply.
    #[test]
    fn oversize_code_block_still_splits() {
        // 30 lines × ~30 bytes ≈ 900 bytes ≈ 300 tokens. budget = 50.
        let body: String = (0..30)
            .map(|i| format!("    let x{i:02} = compute({i});"))
            .collect::<Vec<_>>()
            .join("\n");
        let blocks = vec![code_block(&body, &[], 0, span(5, 34))];
        let doc = make_doc(blocks);
        let (p, c) = policy_v2(50);
        let chunks = c.chunk(&doc, &p).unwrap();

        assert!(
            chunks.len() >= 2,
            "oversize code must split via post-pass, got {}: {chunks:#?}",
            chunks.len()
        );
        for ch in &chunks {
            assert!(
                ch.token_estimate <= 50,
                "piece exceeds budget: {} > 50",
                ch.token_estimate
            );
        }

        // Rejoining with '\n' reconstructs the original code.
        let rejoined = chunks
            .iter()
            .map(|c| c.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(rejoined, body, "pieces must reconstruct the code text");
    }

    /// For a doc with headings + paragraphs + a small (≤budget) code
    /// block, v2 output (chunk count, texts, heading_paths, source_spans,
    /// token_estimate, block_ids) equals v1 output. This is the parity
    /// contract: v2 ≡ v1 for all under-budget content.
    ///
    /// Note: chunk_ids DIFFER by design because chunker_version is in the
    /// id recipe (`"md-heading-v1"` vs `"md-heading-v2"`). We compare the
    /// non-id fields that should be stable.
    #[test]
    fn non_oversize_identical_to_v1() {
        let blocks = vec![
            heading(2, "First", 0, 1),
            paragraph("body of the first section here", &["First"], 0, 2),
            paragraph("more first-section prose", &["First"], 1, 3),
            heading(2, "Second", 1, 4),
            paragraph("second section body text", &["Second"], 0, 5),
            // Small code block — below budget so post-pass leaves it alone.
            code_block("fn small() {}", &["Second"], 0, span(6, 6)),
        ];
        let doc = make_doc(blocks);

        let v1 = MdHeadingV1Chunker
            .chunk(&doc, &policy_v1())
            .unwrap();
        let (p, c) = policy_v2(4000);
        let v2 = c.chunk(&doc, &p).unwrap();

        assert_eq!(v1.len(), v2.len(), "chunk count must match v1");
        for (a, b) in v1.iter().zip(v2.iter()) {
            assert_eq!(a.text, b.text, "text must match v1");
            assert_eq!(a.heading_path, b.heading_path, "heading_path must match v1");
            assert_eq!(a.source_spans, b.source_spans, "source_spans must match v1");
            assert_eq!(a.token_estimate, b.token_estimate, "token_estimate must match v1");
            assert_eq!(a.block_ids, b.block_ids, "block_ids must match v1");
        }
    }

    /// Split pieces have unique chunk_ids; running chunk() twice produces
    /// a byte-identical id sequence (1000-iteration determinism check).
    #[test]
    fn split_pieces_unique_deterministic_ids() {
        let items: Vec<String> = (0..40)
            .map(|i| format!("item {i:02}: line number with some padding text here"))
            .collect();
        let item_refs: Vec<&str> = items.iter().map(String::as_str).collect();
        let blocks = vec![list_block(&item_refs, &[], 0, span(1, 40))];
        let doc = make_doc(blocks);
        let (p, c) = policy_v2(40);

        let baseline: Vec<String> = c
            .chunk(&doc, &p)
            .unwrap()
            .into_iter()
            .map(|ch| ch.chunk_id.0)
            .collect();
        assert!(baseline.len() >= 2, "must split into ≥2 pieces");

        // Unique.
        let mut sorted = baseline.clone();
        let n = sorted.len();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), n, "chunk_ids unique across split pieces");

        // Deterministic.
        for _ in 0..1000 {
            let again: Vec<String> = c
                .chunk(&doc, &p)
                .unwrap()
                .into_iter()
                .map(|ch| ch.chunk_id.0)
                .collect();
            assert_eq!(again, baseline, "chunk_id sequence must be deterministic");
        }
    }

    /// Two MdHeadingV2Chunker instances with different `max_chunk_tokens`
    /// produce different `policy_hash` values for the same policy, causing
    /// different chunk_ids — so a budget change triggers a re-chunk cascade.
    #[test]
    fn budget_in_policy_hash() {
        let p = ChunkPolicy {
            target_tokens: 500,
            overlap_tokens: 80,
            respect_markdown_headings: true,
            chunker_version: ChunkerVersion(VERSION_LABEL.into()),
        };
        let a = MdHeadingV2Chunker {
            max_chunk_tokens: 100,
        };
        let b = MdHeadingV2Chunker {
            max_chunk_tokens: 200,
        };
        assert_ne!(
            a.policy_hash(&p),
            b.policy_hash(&p),
            "different budgets must yield different policy_hash"
        );

        // Propagates to chunk_ids on an oversize doc.
        let items: Vec<String> = (0..30)
            .map(|i| format!("    let x{i:02} = compute({i});"))
            .collect();
        let item_refs: Vec<&str> = items.iter().map(String::as_str).collect();
        let blocks = vec![list_block(&item_refs, &[], 0, span(1, 30))];
        let doc = make_doc(blocks);
        let ids_a: Vec<String> = a
            .chunk(&doc, &p)
            .unwrap()
            .into_iter()
            .map(|c| c.chunk_id.0)
            .collect();
        let ids_b: Vec<String> = b
            .chunk(&doc, &p)
            .unwrap()
            .into_iter()
            .map(|c| c.chunk_id.0)
            .collect();
        assert_ne!(ids_a, ids_b, "different budgets must produce different chunk_ids");
    }

    /// A below-budget chunk stores the BARE policy_hash (no `#seg` suffix).
    #[test]
    fn under_budget_stores_bare_policy_hash() {
        let blocks = vec![code_block("fn x() {}", &[], 0, span(1, 1))];
        let doc = make_doc(blocks);
        let (p, c) = policy_v2(4000);
        let chunks = c.chunk(&doc, &p).unwrap();
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].policy_hash.len(), POLICY_HASH_HEX_LEN);
        assert!(
            !chunks[0].policy_hash.contains('#'),
            "stored hash has no #seg suffix: {}",
            chunks[0].policy_hash
        );
    }

    // ── Unit tests for the split helpers ─────────────────────────────────

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
