//! `pdf-page-v1` — page-aware PDF chunker.
//!
//! Consumes a [`CanonicalDocument`] produced by `kebab-parse-pdf` (one
//! [`Block::Paragraph`] per page, every block carrying [`SourceSpan::Page`])
//! and emits one or more [`Chunk`]s per page. Chunks NEVER cross a page
//! boundary (citation locality is the whole reason §3.4 introduced
//! `SourceSpan::Page`), and each chunk's `source_spans` is a single
//! `Page { page, char_start, char_end }` with positions in **characters**
//! within the page text — matching `Citation::Page` fragment semantics
//! across the rest of the workspace.
//!
//! Per design §3.5 (Chunk), §4.2 (chunk_id recipe — see deviation note
//! below), §0 Q3 (citation), §9 (versioning).
//!
//! ## Splitting policy (two tiers, `pdf-page-v1.2`)
//!
//! **Tier 1 — sentence / paragraph greedy split (unchanged from v1.1):**
//!
//! - If a page's bytes fit under `policy.target_tokens * BYTES_PER_TOKEN`
//!   the entire page is a single chunk.
//! - Otherwise the page text is segmented at paragraph breaks (`\n\n`) and
//!   sentence ends (`.`/`?`/`!` followed by whitespace). Adjacent
//!   segments are greedily glued until the running byte budget would be
//!   exceeded; the chunk is emitted at that boundary. The next chunk's
//!   prefix is seeded with the trailing `policy.overlap_tokens *
//!   BYTES_PER_TOKEN` bytes of the prior chunk so retrieval handles
//!   queries that fall on the boundary.
//! - Common English abbreviations (`Mr.`, `i.e.`, `e.g.`, `Fig. 3`)
//!   trip the sentence-end heuristic and produce spurious boundaries —
//!   accepted as a v1 limit. A real sentence segmenter lands with the
//!   P+ tokenizer slot.
//! - The effective overlap budget is clamped at `target_bytes / 2` so a
//!   pathological policy (`overlap_tokens >= target_tokens`) cannot
//!   make a chunk fully re-emit the previous chunk's text. Same guard
//!   pattern as `md-heading-v1::collect_overlap_seed`.
//!
//! **Tier 2 — generic oversize fallback (NEW in `pdf-page-v1.2`):**
//!
//! v1.1 had an ACCEPTED HOLE: a page with no qualifying segment boundary
//! AND text exceeding the budget (e.g. a dense scanned page OCR'd into one
//! 5,000-byte run with no sentence/paragraph break) emitted ONE oversized
//! chunk regardless of budget. That single over-budget chunk overflows a
//! strict embedder (e.g. AMD Lemonade). v1.2 closes the hole: every tier-1
//! segment whose byte/3 estimate still exceeds `self.max_chunk_tokens` is
//! handed to [`crate::oversize::text_pieces`] (line split → UTF-8 char
//! fallback), so each emitted chunk is GUARANTEED ≤ budget. The char
//! fallback bounds even a no-whitespace page that the tier-1 greedy
//! splitter cannot cut. This is the same generic post-pass `md-heading-v2`
//! applies — shared via [`crate::oversize`].
//!
//! ## `BYTES_PER_TOKEN`
//!
//! 3 — same calibration as `md-heading-v1` (covers Korean ≈ 3 b/tok and
//! over-estimates English ≈ 4 b/tok). The original p7-2 spec literal said
//! `× 4`, but cross-chunker comparability outweighs the spec literal here.
//! Logged in `tasks/HOTFIXES.md`. Sourced from [`crate::oversize`] so the
//! proxy can't drift between this chunker and `md-heading-v2`.
//!
//! ## `chunk_id` collision deviation
//!
//! Design §4.2's `chunk_id = blake3(doc_id || chunker_version || sort(block_ids)
//! || policy_hash)` collides when one block (= one PDF page) is split
//! into multiple chunks: every chunk on that page has identical
//! `block_ids`. md-heading-v1 sidesteps this by always emitting at most
//! one chunk per atomic block. PdfPageV1 cannot.
//!
//! Workaround that doesn't change the §4.2 recipe: feed a per-chunk
//! variant `format!("{base_policy_hash}#c{segment_start}")` into the
//! recipe's `policy_hash` slot. `segment_start` is the pre-overlap
//! segment boundary, strictly increasing across the returned chunks
//! even when the overlap walk collapses `actual_start` to a previous
//! chunk's `prev_min`. Unmodified `base_policy_hash` is stored in
//! `Chunk.policy_hash` so the field still answers "what policy was
//! active". v1.1 second-iteration patch — logged in
//! `tasks/HOTFIXES.md` (2026-05-27).
//!
//! v1.2 extends the suffix for tier-2 sub-pieces: `#c{segment_start}s{i}`
//! for sub-piece `i` (0-based) of a tier-1 segment that had to be
//! oversize-split. A single-piece (non-oversize) segment keeps the bare
//! `#c{segment_start}` so common-case chunk_ids are byte-identical to the
//! pre-pass. `segment_start` is per-segment-unique and `i` disambiguates
//! within a segment, so the full suffix is strictly unique across the page.
//!
//! ## Budget in `policy_hash` (NEW in `pdf-page-v1.2`)
//!
//! Because the tier-2 split is keyed on `self.max_chunk_tokens`, changing
//! the budget changes the produced chunks — so the budget MUST participate
//! in the chunk-id cascade (design §9), exactly as `md-heading-v2` does.
//! `policy_hash()` folds the 8 LE bytes of `self.max_chunk_tokens` after
//! the canonical `ChunkPolicy` bytes. This moves PDF chunk_ids on a budget
//! change, consistent with markdown. (The `ingest_config_signature`
//! already folds `max_chunk_tokens` into the skip-check, so the no-`--force`
//! re-index already worked; this aligns the chunk_id cascade with it.)

use kebab_core::{
    Block, BlockId, CanonicalDocument, Chunk, ChunkPolicy, Chunker, ChunkerVersion, DocumentId,
    SourceSpan, id_for_chunk,
};

use crate::oversize::{BYTES_PER_TOKEN, text_pieces};

const VERSION_LABEL: &str = "pdf-page-v1.2";
const POLICY_HASH_HEX_LEN: usize = 16;

/// Page-aware PDF chunker. See module docs for the two-tier splitting
/// policy and the `chunk_id` collision-avoidance deviation.
///
/// Not a unit struct as of v1.2 — it carries the tier-2 split budget
/// threaded from `config.ingest.chunking.max_chunk_tokens` (mirrors
/// `MdHeadingV2Chunker`). The budget folds into `policy_hash` so a change
/// re-chunks every PDF via the cascade (design §9).
#[derive(Clone, Copy, Debug)]
pub struct PdfPageV1Chunker {
    /// Max byte/3 token estimate per emitted chunk. Any tier-1 segment whose
    /// `token_estimate` exceeds this is oversize-split at line (then UTF-8
    /// char) boundaries by the shared [`crate::oversize`] primitive. Folded
    /// into `policy_hash` so changing this budget triggers a re-chunk via
    /// the cascade (design §9).
    pub max_chunk_tokens: usize,
}

impl Chunker for PdfPageV1Chunker {
    fn chunker_version(&self) -> ChunkerVersion {
        ChunkerVersion(VERSION_LABEL.to_string())
    }

    /// Policy hash with the v1.2 tier-2 budget folded in.
    ///
    /// We append the 8 LE bytes of `self.max_chunk_tokens` to the canonical
    /// `ChunkPolicy` JSON before hashing. The canonical JSON is
    /// self-delimiting (a balanced JSON object), so there is no structural
    /// ambiguity between the policy bytes and the trailing 8 budget bytes.
    /// Same recipe as [`crate::md_heading_v2::MdHeadingV2Chunker::policy_hash`]
    /// — so two PDF instances with different `max_chunk_tokens` produce
    /// different `policy_hash` (and chunk_ids), re-indexing all PDFs on a
    /// budget change rather than leaving stale oversized chunks.
    ///
    /// # Panics
    ///
    /// Panics if canonical JSON serialization of `ChunkPolicy` fails —
    /// unreachable in practice.
    fn policy_hash(&self, policy: &ChunkPolicy) -> String {
        let mut bytes = serde_json_canonicalizer::to_vec(policy)
            .expect("canonical JSON serialization of ChunkPolicy must not fail");
        bytes.extend_from_slice(&self.max_chunk_tokens.to_le_bytes());
        let hex = blake3::hash(&bytes).to_hex().to_string();
        hex[..POLICY_HASH_HEX_LEN].to_string()
    }

    fn chunk(&self, doc: &CanonicalDocument, policy: &ChunkPolicy) -> anyhow::Result<Vec<Chunk>> {
        // Validate up front — every block must be a Paragraph carrying
        // SourceSpan::Page. A mixed document signals a routing bug in
        // the caller (e.g. running this chunker on Markdown) and is
        // worth surfacing loudly.
        for b in &doc.blocks {
            let common = match b {
                Block::Paragraph(p) => &p.common,
                _ => anyhow::bail!(
                    "PdfPageV1Chunker only handles PDF docs (got non-Paragraph block)"
                ),
            };
            if !matches!(common.source_span, SourceSpan::Page { .. }) {
                anyhow::bail!("PdfPageV1Chunker only handles PDF docs (got non-Page source_span)");
            }
        }

        let base_policy_hash = self.policy_hash(policy);
        let chunker_version = self.chunker_version();
        let target_bytes = policy.target_tokens.saturating_mul(BYTES_PER_TOKEN).max(1);
        // Clamp the overlap to half the target. Without this, a policy
        // with `overlap_tokens >= target_tokens` would make every chunk
        // fully re-emit the previous chunk's text — mirrors
        // md-heading-v1's `seed_budget = overlap_tokens.min(target/2)`.
        let overlap_bytes = policy
            .overlap_tokens
            .saturating_mul(BYTES_PER_TOKEN)
            .min(target_bytes / 2);

        let mut out: Vec<Chunk> = Vec::new();
        for b in &doc.blocks {
            let p = match b {
                Block::Paragraph(t) => t,
                _ => unreachable!("validated above"),
            };
            let page_num = match p.common.source_span {
                SourceSpan::Page { page, .. } => page,
                _ => unreachable!("validated above"),
            };

            // Empty page → 0 chunks. Page is still searchable via the
            // CanonicalDocument's per-page `Provenance::Warning`
            // ("scanned candidate") — chunking just has nothing to say
            // about it.
            if p.text.trim().is_empty() {
                continue;
            }

            for (segment_start, char_start, seg_char_end, slice) in
                chunk_page(&p.text, target_bytes, overlap_bytes)
            {
                // ── Tier 2: oversize fallback (pdf-page-v1.2) ────────────
                // The tier-1 greedy splitter (`chunk_page`) can still hand
                // back an over-budget slice when a page has no qualifying
                // sentence/paragraph boundary (e.g. a dense scanned page
                // OCR'd into one long run). Split such a slice into
                // sub-pieces each ≤ budget via the shared primitive; the
                // common case (slice ≤ budget) returns a single piece and
                // its chunk_id / span are byte-identical to the v1.1
                // pre-pass.
                let pieces: Vec<String> = if slice.len().div_ceil(BYTES_PER_TOKEN)
                    > self.max_chunk_tokens
                {
                    text_pieces(&slice, self.max_chunk_tokens)
                } else {
                    vec![slice]
                };
                let single_piece = pieces.len() == 1;

                // Page span for the sub-pieces. Exact per-sub-piece char
                // narrowing is NOT recoverable from `text_pieces`' `Vec<String>`
                // alone: line splits drop the inter-piece '\n' separators (each
                // is consumed at the split boundary, present in neither adjacent
                // piece), so summing piece char counts drifts earlier by the
                // number of those boundaries. So every sub-piece carries the
                // PARENT SEGMENT's char range (`char_start..seg_char_end`) — the
                // md-heading-v2 block-granular approach. A citation points at the
                // correct page region, never a drifted offset; it is coarser than
                // per-sub-piece, never wrong. The common single-piece case is the
                // whole segment span → byte-identical to the v1.1 pre-pass.
                //
                // PDF chars-per-page comfortably fits in u32 (~10k chars even for
                // dense typography); an explicit panic beats a silent off-by-2^32.
                let seg_char_start_u32 =
                    u32::try_from(char_start).expect("page chars fit in u32");
                let seg_char_end_u32 =
                    u32::try_from(seg_char_end).expect("page chars fit in u32");
                let span = SourceSpan::Page {
                    page: page_num,
                    char_start: Some(seg_char_start_u32),
                    char_end: Some(seg_char_end_u32),
                };

                for (i, piece) in pieces.into_iter().enumerate() {
                    let block_ids: Vec<BlockId> = vec![p.common.block_id.clone()];
                    // v0.20.0 sub-item 1 bugfix (#3): per-chunk policy_hash
                    // variant uses `segment_start` (pre-overlap boundary,
                    // strictly increasing) instead of `char_start` (post-
                    // overlap, may collapse to prev_min). See module docs +
                    // spec §4.1 root cause + HOTFIXES.md 2026-05-27.
                    //
                    // v1.2: append `s{i}` for tier-2 sub-pieces so each
                    // oversize-split piece gets a unique id, while a single
                    // (non-oversize) piece keeps the bare `#c{segment_start}`
                    // — common-case chunk_ids stay byte-identical to v1.1.
                    let per_chunk_hash = if single_piece {
                        format!("{base_policy_hash}#c{segment_start}")
                    } else {
                        format!("{base_policy_hash}#c{segment_start}s{i}")
                    };
                    let chunk_id =
                        id_for_chunk(&doc.doc_id, &chunker_version, &block_ids, &per_chunk_hash);
                    let token_estimate = piece.len().div_ceil(BYTES_PER_TOKEN);

                    out.push(Chunk {
                        chunk_id,
                        doc_id: DocumentId(doc.doc_id.0.clone()),
                        block_ids,
                        tokenized_korean_text: crate::tokenize_korean_morphological(&piece),
                        text: piece,
                        heading_path: Vec::new(),
                        source_spans: vec![span.clone()],
                        token_estimate,
                        chunker_version: chunker_version.clone(),
                        policy_hash: base_policy_hash.clone(),
                    });
                }
            }
        }

        tracing::debug!(
            target: "kebab-chunk",
            doc_id = %doc.doc_id,
            chunks = out.len(),
            "pdf-page-v1 chunked",
        );

        Ok(out)
    }
}

/// Split a single page's text into ordered chunks, each represented as
/// `(segment_start, actual_start, chunk_end, text_slice)`.
///
/// - `segment_start` = pre-overlap segment boundary. Strictly increasing
///   across the returned vec. Use this for chunk_id uniqueness suffixes.
/// - `actual_start` = post-overlap start char index. May collapse to a
///   previous chunk's `actual_start` under aggressive overlap policy.
///   Use this for `SourceSpan::Page::char_start`.
/// - `chunk_end` = chunk's end char index (exclusive).
///
/// Returns an empty vector when `text` is empty or whitespace-only.
fn chunk_page(
    text: &str,
    target_bytes: usize,
    overlap_bytes: usize,
) -> Vec<(usize, usize, usize, String)> {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    if n == 0 {
        return Vec::new();
    }
    if text.len() <= target_bytes {
        return vec![(0, 0, n, text.to_string())];
    }

    // Build candidate boundary positions (char indices where a chunk
    // *may* start). 0 and n are always boundaries; interior boundaries
    // are after a paragraph break (`\n\n`) or after a sentence-ending
    // punctuation followed by whitespace.
    let mut bounds: Vec<usize> = vec![0];
    let mut k = 0;
    while k + 1 < n {
        let c = chars[k];
        let nx = chars[k + 1];
        let is_paragraph_break = c == '\n' && nx == '\n';
        let is_sentence_end = matches!(c, '.' | '?' | '!') && nx.is_whitespace();
        if (is_paragraph_break || is_sentence_end) && k + 2 <= n {
            bounds.push(k + 2);
        }
        k += 1;
    }
    if *bounds.last().unwrap() != n {
        bounds.push(n);
    }
    bounds.dedup();

    // UTF-8 byte length of the slice between two char indices.
    let byte_len = |a: usize, b: usize| -> usize { chars[a..b].iter().map(|c| c.len_utf8()).sum() };

    let mut chunks: Vec<(usize, usize, usize, String)> = Vec::new();
    let mut seg_idx: usize = 0;
    while seg_idx + 1 < bounds.len() {
        let start = bounds[seg_idx];
        let mut end_idx = seg_idx + 1;
        let mut acc = byte_len(start, bounds[end_idx]);

        // Greedy grow: glue subsequent segments while we stay under
        // budget. We always include at least one segment per chunk
        // (`acc > 0` guard) so a single oversize segment doesn't loop.
        while end_idx + 1 < bounds.len() {
            let next_bytes = byte_len(bounds[end_idx], bounds[end_idx + 1]);
            if acc + next_bytes > target_bytes && acc > 0 {
                break;
            }
            acc += next_bytes;
            end_idx += 1;
        }

        let chunk_end = bounds[end_idx];

        // Apply overlap: walk `actual_start` left of `start` until we
        // have absorbed up to `overlap_bytes` of bytes, but never past
        // the previous chunk's start (no full re-emission).
        let actual_start = if let Some(prev) = chunks.last() {
            // prev tuple shape = (segment_start, actual_start, chunk_end, slice).
            // overlap walk floor = previous chunk's actual_start (prev.1).
            let prev_min = prev.1;
            let mut a = start;
            let mut acc_o: usize = 0;
            while a > prev_min {
                let cl = chars[a - 1].len_utf8();
                if acc_o + cl > overlap_bytes {
                    break;
                }
                acc_o += cl;
                a -= 1;
            }
            a
        } else {
            start
        };

        let slice: String = chars[actual_start..chunk_end].iter().collect();
        chunks.push((start, actual_start, chunk_end, slice));
        seg_idx = end_idx;
    }

    chunks
}

#[cfg(test)]
mod tests {
    use super::*;
    use kebab_core::{
        AssetId, CommonBlock, Inline, Lang, Metadata, ParserVersion, Provenance, SourceType,
        TextBlock, TrustLevel, WorkspacePath, id_for_block, id_for_doc,
    };
    use time::OffsetDateTime;

    fn make_pdf_doc(pages: &[&str]) -> CanonicalDocument {
        let workspace_path = WorkspacePath::new("docs/test.pdf".into()).unwrap();
        let asset_id = AssetId("a".repeat(64));
        let parser_version = ParserVersion("pdf-text-v1".into());
        let doc_id = id_for_doc(&workspace_path, &asset_id, &parser_version);

        let mut blocks: Vec<Block> = Vec::new();
        for (i, text) in pages.iter().enumerate() {
            let page = (i as u32) + 1;
            let char_count = text.chars().count() as u32;
            let span = SourceSpan::Page {
                page,
                char_start: Some(0),
                char_end: Some(char_count),
            };
            let block_id = id_for_block(&doc_id, "paragraph", &[], i as u32, &span);
            let inlines = if text.is_empty() {
                Vec::new()
            } else {
                vec![Inline::Text {
                    text: (*text).to_string(),
                }]
            };
            blocks.push(Block::Paragraph(TextBlock {
                common: CommonBlock {
                    block_id,
                    heading_path: Vec::new(),
                    source_span: span,
                },
                text: (*text).to_string(),
                inlines,
            }));
        }

        CanonicalDocument {
            doc_id,
            source_asset_id: asset_id,
            workspace_path,
            title: "test".into(),
            lang: Lang("und".into()),
            blocks,
            metadata: Metadata {
                aliases: vec![],
                tags: vec![],
                created_at: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
                updated_at: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
                source_type: SourceType::Paper,
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
            parser_version,
            schema_version: 1,
            doc_version: 1,
            last_chunker_version: None,
            last_embedding_version: None,
        }
    }

    fn default_policy(target: usize, overlap: usize) -> ChunkPolicy {
        ChunkPolicy {
            target_tokens: target,
            overlap_tokens: overlap,
            respect_markdown_headings: false,
            chunker_version: ChunkerVersion(VERSION_LABEL.into()),
        }
    }

    /// Default test budget large enough that the tier-2 oversize fallback
    /// never fires — so tests targeting the tier-1 sentence/paragraph
    /// splitter keep their pre-v1.2 behavior. Tier-2 tests pass an explicit
    /// small budget.
    const BIG_BUDGET: usize = 100_000;

    /// Construct the chunker with an explicit tier-2 budget.
    fn chunker(max_chunk_tokens: usize) -> PdfPageV1Chunker {
        PdfPageV1Chunker { max_chunk_tokens }
    }

    #[test]
    fn chunker_version_is_pdf_page_v1() {
        assert_eq!(
            chunker(BIG_BUDGET).chunker_version(),
            ChunkerVersion(VERSION_LABEL.to_string())
        );
    }

    #[test]
    fn three_page_small_emits_one_chunk_per_page() {
        let doc = make_pdf_doc(&["page one", "page two", "page three"]);
        let chunks = chunker(BIG_BUDGET)
            .chunk(&doc, &default_policy(500, 80))
            .unwrap();
        assert_eq!(chunks.len(), 3);
        for (i, c) in chunks.iter().enumerate() {
            assert_eq!(c.block_ids.len(), 1);
            assert_eq!(c.heading_path, Vec::<String>::new());
            assert_eq!(c.source_spans.len(), 1);
            match c.source_spans[0] {
                SourceSpan::Page {
                    page,
                    char_start,
                    char_end,
                } => {
                    assert_eq!(page, (i as u32) + 1);
                    assert_eq!(char_start, Some(0));
                    assert!(char_end.unwrap() > 0);
                }
                ref other => panic!("expected Page span, got {other:?}"),
            }
        }
        assert_eq!(chunks[0].text, "page one");
        assert_eq!(chunks[1].text, "page two");
        assert_eq!(chunks[2].text, "page three");
    }

    #[test]
    fn one_page_huge_text_splits_into_multiple_chunks_with_overlap() {
        // Build a single page with 8 paragraphs of ~150 bytes each.
        // target=50 tokens × 3 b/tok = 150 byte budget → each paragraph
        // is itself just under budget, so 2-paragraph accumulation
        // overshoots → ~8 chunks.
        let para = "a".repeat(150);
        let page_text = std::iter::repeat_n(para, 8)
            .collect::<Vec<_>>()
            .join("\n\n");
        let doc = make_pdf_doc(&[&page_text]);
        let chunks = chunker(BIG_BUDGET)
            .chunk(&doc, &default_policy(50, 20))
            .unwrap();
        assert!(
            chunks.len() >= 4,
            "expected ≥4 chunks for a 1200-byte page; got {}: text len={}",
            chunks.len(),
            page_text.len()
        );
        // All chunks live on page 1.
        for c in &chunks {
            match c.source_spans[0] {
                SourceSpan::Page { page, .. } => assert_eq!(page, 1),
                _ => panic!("non-Page span"),
            }
        }
        // Overlap: chunk N's text starts with chunk N-1's tail bytes
        // (or, equivalently, chunk N's char_start lies before chunk
        // N-1's char_end).
        for w in chunks.windows(2) {
            let prev_end = match w[0].source_spans[0] {
                SourceSpan::Page {
                    char_end: Some(e), ..
                } => e,
                _ => panic!("missing char_end"),
            };
            let next_start = match w[1].source_spans[0] {
                SourceSpan::Page {
                    char_start: Some(s),
                    ..
                } => s,
                _ => panic!("missing char_start"),
            };
            assert!(
                next_start < prev_end,
                "expected overlap (next.start < prev.end): {next_start} vs {prev_end}"
            );
        }
        // chunk_ids stay distinct despite identical block_ids — the
        // per-chunk policy_hash variant is doing its job.
        let mut ids: Vec<&str> = chunks.iter().map(|c| c.chunk_id.0.as_str()).collect();
        ids.sort_unstable();
        let total = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), total, "all chunk_ids must be unique");
    }

    #[test]
    fn empty_page_produces_no_chunks_for_that_page() {
        let doc = make_pdf_doc(&["page one", "", "page three"]);
        let chunks = chunker(BIG_BUDGET)
            .chunk(&doc, &default_policy(500, 80))
            .unwrap();
        assert_eq!(chunks.len(), 2);
        let pages: Vec<u32> = chunks
            .iter()
            .map(|c| match c.source_spans[0] {
                SourceSpan::Page { page, .. } => page,
                _ => 0,
            })
            .collect();
        assert_eq!(pages, vec![1, 3]);
    }

    #[test]
    fn whitespace_only_page_skipped_too() {
        let doc = make_pdf_doc(&["page one", "   \n  ", "page three"]);
        let chunks = chunker(BIG_BUDGET)
            .chunk(&doc, &default_policy(500, 80))
            .unwrap();
        assert_eq!(chunks.len(), 2);
    }

    #[test]
    fn non_pdf_doc_returns_error() {
        // A doc whose blocks carry SourceSpan::Line (Markdown shape).
        let workspace_path = WorkspacePath::new("notes/note.md".into()).unwrap();
        let asset_id = AssetId("a".repeat(64));
        let parser_version = ParserVersion("md-block-v1".into());
        let doc_id = id_for_doc(&workspace_path, &asset_id, &parser_version);
        let span = SourceSpan::Line { start: 1, end: 1 };
        let block_id = id_for_block(&doc_id, "paragraph", &[], 0, &span);
        let blocks = vec![Block::Paragraph(TextBlock {
            common: CommonBlock {
                block_id,
                heading_path: vec![],
                source_span: span,
            },
            text: "markdown body".into(),
            inlines: vec![],
        })];
        let doc = CanonicalDocument {
            doc_id,
            source_asset_id: asset_id,
            workspace_path,
            title: "n".into(),
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
            parser_version,
            schema_version: 1,
            doc_version: 1,
            last_chunker_version: None,
            last_embedding_version: None,
        };
        let err = chunker(BIG_BUDGET)
            .chunk(&doc, &default_policy(500, 80))
            .expect_err("non-PDF doc must error");
        assert!(
            err.to_string().contains("PdfPageV1Chunker"),
            "error mentions chunker: {err}"
        );
    }

    #[test]
    fn no_chunk_crosses_page_boundary() {
        // Synthetic 4-page doc with mixed page sizes — each chunk
        // must claim exactly one page in its single source_span.
        let big_x = "x".repeat(2000);
        let big_y = "y".repeat(800);
        let pages = vec![
            "tiny page one.",
            big_x.as_str(),
            "another tiny one.",
            big_y.as_str(),
        ];
        let doc = make_pdf_doc(&pages);
        let chunks = chunker(BIG_BUDGET)
            .chunk(&doc, &default_policy(50, 10))
            .unwrap();
        for c in &chunks {
            assert_eq!(c.source_spans.len(), 1, "chunk should hold one Page span");
            assert!(matches!(c.source_spans[0], SourceSpan::Page { .. }));
        }
        // Group chunks by page, verify pages are non-decreasing in
        // chunk order (no interleaving across pages).
        let mut prev_page = 0u32;
        for c in &chunks {
            let page = match c.source_spans[0] {
                SourceSpan::Page { page, .. } => page,
                _ => unreachable!(),
            };
            assert!(
                page >= prev_page,
                "page numbers must be non-decreasing in chunk order: {prev_page} → {page}"
            );
            prev_page = page;
        }
    }

    #[test]
    fn deterministic_chunk_ids_1000() {
        let doc = make_pdf_doc(&[
            "first page text. and another sentence here.",
            &("xyz ".repeat(500)),
        ]);
        let policy = default_policy(80, 20);
        let baseline: Vec<String> = chunker(BIG_BUDGET)
            .chunk(&doc, &policy)
            .unwrap()
            .into_iter()
            .map(|c| c.chunk_id.0)
            .collect();
        for _ in 0..1000 {
            let again: Vec<String> = chunker(BIG_BUDGET)
                .chunk(&doc, &policy)
                .unwrap()
                .into_iter()
                .map(|c| c.chunk_id.0)
                .collect();
            assert_eq!(again, baseline);
        }
    }

    #[test]
    fn snapshot_three_page_chunks_stable() {
        let doc = make_pdf_doc(&[
            "Hello page 1.",
            "Hello page 2 with some more body text.",
            "Hello page 3.",
        ]);
        let chunks = chunker(BIG_BUDGET)
            .chunk(&doc, &default_policy(500, 80))
            .unwrap();
        assert_eq!(chunks.len(), 3);
        for (i, c) in chunks.iter().enumerate() {
            assert_eq!(c.chunker_version.0, VERSION_LABEL);
            assert_eq!(c.heading_path, Vec::<String>::new());
            assert_eq!(c.source_spans.len(), 1);
            match c.source_spans[0] {
                SourceSpan::Page {
                    page,
                    char_start,
                    char_end,
                } => {
                    assert_eq!(page, (i as u32) + 1);
                    assert_eq!(char_start, Some(0));
                    assert_eq!(char_end, Some(c.text.chars().count() as u32));
                }
                _ => panic!("expected Page"),
            }
            assert!(c.policy_hash.len() == POLICY_HASH_HEX_LEN);
            assert!(c.policy_hash.bytes().all(|b| b.is_ascii_hexdigit()));
        }
    }

    #[test]
    fn overlap_clamped_when_overlap_exceeds_target() {
        // Pathological policy: overlap = 4× target. Without the
        // `target_bytes / 2` clamp, every chunk would fully re-emit
        // the previous chunk's text (chunk N's actual_start collapses
        // to chunk N-1's actual_start).
        let para = "a".repeat(150);
        let page_text = std::iter::repeat_n(para, 6)
            .collect::<Vec<_>>()
            .join("\n\n");
        let doc = make_pdf_doc(&[&page_text]);
        let policy = ChunkPolicy {
            target_tokens: 50,
            overlap_tokens: 200,
            respect_markdown_headings: false,
            chunker_version: ChunkerVersion(VERSION_LABEL.into()),
        };
        let chunks = chunker(BIG_BUDGET).chunk(&doc, &policy).unwrap();
        // For each consecutive pair, the new chunk's actual_start must
        // be strictly greater than the previous chunk's actual_start
        // (no full re-emission). Without the clamp, equality (full
        // overlap) is the failure mode.
        for w in chunks.windows(2) {
            let prev_start = match w[0].source_spans[0] {
                SourceSpan::Page {
                    char_start: Some(s),
                    ..
                } => s,
                _ => panic!("missing char_start"),
            };
            let next_start = match w[1].source_spans[0] {
                SourceSpan::Page {
                    char_start: Some(s),
                    ..
                } => s,
                _ => panic!("missing char_start"),
            };
            assert!(
                next_start > prev_start,
                "overlap must not fully re-emit prior chunk: prev_start={prev_start}, next_start={next_start}"
            );
        }
        // chunk_ids stay distinct (the per-chunk hash variant keys off
        // char_start which is now strictly increasing).
        let mut ids: Vec<&str> = chunks.iter().map(|c| c.chunk_id.0.as_str()).collect();
        ids.sort_unstable();
        let total = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), total, "chunk_ids must remain unique");
    }

    #[test]
    fn multi_chunk_page_with_aggressive_overlap_produces_unique_chunk_ids() {
        // 한국어 OCR text 의 trigger shape: 10 char "가" + ". " + 500 char "나".
        // → first segment [0, 12), second segment [12, n).
        //   page_text byte_len = 10*3 + 2 + 500*3 = 1532 > target_bytes=1500
        //   → multi-chunk. overlap_bytes = min(240, 750) = 240 chars=80
        //   → second chunk 의 actual_start 가 prev_min=0 collapse → same `#c0`.
        //
        // default_policy(500, 80) — target_tokens=500 → target_bytes=500*3=1500
        // (한국어 3byte/char 환산), overlap_tokens=80 → overlap_bytes=min(240, 750)=240.
        // verifier round 1 L-3 보강.
        let early_seg = "가".repeat(10);
        let tail = "나".repeat(500);
        let page_text = format!("{early_seg}. {tail}");

        let doc = make_pdf_doc(&[&page_text]);
        let policy = default_policy(500, 80); // target=1500 byte, overlap=240 byte
        let chunks = chunker(BIG_BUDGET).chunk(&doc, &policy).unwrap();

        assert!(
            chunks.len() >= 2,
            "expected ≥2 chunks for {} byte page; got {}",
            page_text.len(),
            chunks.len()
        );

        let mut ids: Vec<&str> = chunks.iter().map(|c| c.chunk_id.0.as_str()).collect();
        ids.sort_unstable();
        let total = ids.len();
        ids.dedup();
        assert_eq!(
            ids.len(),
            total,
            "all chunk_ids must be unique even when overlap walks actual_start back to prev_min"
        );
    }

    /// Retargeted from `policy_hash_matches_md_heading_v1_for_identical_policy`.
    ///
    /// v1.1's PDF `policy_hash` matched `md-heading-v1` (neither folded a
    /// budget). v1.2 folds `max_chunk_tokens` into the PDF hash exactly as
    /// `md-heading-v2` does — so the meaningful cross-chunker fingerprint
    /// identity is now PDF-v1.2 ≡ md-heading-v2 for the SAME budget. We
    /// assert that identity (both append the same 8 budget bytes to the same
    /// canonical `ChunkPolicy` JSON), preserving the "one policy_hash query
    /// covers both chunkers" property across the budget-aware chunkers.
    #[test]
    fn policy_hash_matches_md_heading_v2_for_identical_policy_and_budget() {
        let p = default_policy(500, 80);
        let budget = 4000;
        let pdf = chunker(budget).policy_hash(&p);
        let md = crate::MdHeadingV2Chunker {
            max_chunk_tokens: budget,
        }
        .policy_hash(&p);
        assert_eq!(
            pdf, md,
            "pdf-page-v1.2 and md-heading-v2 must share policy_hash for an identical policy + budget"
        );
    }

    /// Budget-sensitivity: two PDF instances with different `max_chunk_tokens`
    /// must produce different `policy_hash` (and therefore different
    /// chunk_ids), so a budget change re-indexes all PDFs via the cascade
    /// (design §9). Mirrors `md_heading_v2::budget_in_policy_hash`.
    #[test]
    fn budget_in_policy_hash() {
        let p = default_policy(500, 80);
        let a = chunker(100).policy_hash(&p);
        let b = chunker(200).policy_hash(&p);
        assert_ne!(a, b, "different budgets must yield different policy_hash");
    }

    /// NEW (pdf-page-v1.2): a single page that exceeds a small budget with
    /// NO sentence/paragraph boundary (one long run) — the exact v1.1 hole
    /// — must now tier-2 split into ≥2 chunks, each ≤ budget. Also asserts
    /// text reconstruction, chunk_id uniqueness, and that every sub-piece
    /// carries the PARENT SEGMENT's Page char span (segment-granular).
    #[test]
    fn oversize_pdf_page_splits() {
        // 600 chars of "x" with NO whitespace / sentence end / paragraph
        // break. target_tokens=500 → 1500 byte tier-1 budget, so tier-1
        // (`chunk_page`) returns the 600-byte page as a SINGLE segment
        // (v1.1 would emit one 600-byte chunk — the hole). budget = 20
        // tokens = 60 bytes, so tier-2 must char-split it into ≥10 pieces.
        let page_text = "x".repeat(600);
        let doc = make_pdf_doc(&[&page_text]);
        let budget = 20;
        let policy = default_policy(500, 80);
        let chunks = chunker(budget).chunk(&doc, &policy).unwrap();

        // ≥2 chunks (the hole is closed).
        assert!(
            chunks.len() >= 2,
            "oversize no-boundary page must tier-2 split, got {}",
            chunks.len()
        );

        // Every chunk ≤ budget.
        for c in &chunks {
            assert!(
                c.text.len().div_ceil(BYTES_PER_TOKEN) <= budget,
                "piece exceeds budget: {} > {budget}",
                c.text.len().div_ceil(BYTES_PER_TOKEN)
            );
        }

        // Concatenation reconstructs the page text (single tier-1 segment,
        // char-split → direct concat).
        let rejoined: String = chunks.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(rejoined, page_text, "sub-pieces must reconstruct the page text");

        // chunk_ids unique.
        let mut ids: Vec<&str> = chunks.iter().map(|c| c.chunk_id.0.as_str()).collect();
        let total = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), total, "all sub-piece chunk_ids must be unique");

        // Every sub-piece carries the PARENT SEGMENT's Page char span
        // (segment-granular, md-style). This page is a single tier-1 segment,
        // so every tier-2 sub-piece spans the whole page `[0, page_chars]` —
        // a citation points at the right page region, never a drifted offset.
        let page_chars = page_text.chars().count() as u32;
        for c in &chunks {
            match c.source_spans[0] {
                SourceSpan::Page {
                    page,
                    char_start: Some(s),
                    char_end: Some(e),
                } => {
                    assert_eq!(page, 1, "all pieces on page 1");
                    assert_eq!(s, 0, "sub-piece span = parent segment start (0)");
                    assert_eq!(e, page_chars, "sub-piece span = parent segment end");
                }
                ref other => panic!("expected fully-populated Page span, got {other:?}"),
            }
        }
    }

    /// Regression for the v1.2 span-drift bug: a tier-2 split whose oversize
    /// slice CONTAINS `\n` line boundaries (the realistic dense-OCR shape).
    /// The earlier per-piece char-narrowing summed piece char counts, but
    /// `text_pieces` consumes the inter-piece `\n` separators, so the running
    /// offset drifted earlier on every line boundary. The segment-granular
    /// span (every piece = parent segment range) sidesteps that entirely.
    #[test]
    fn oversize_pdf_page_with_newlines_splits_without_span_drift() {
        // A page of newline-separated short lines, no sentence-end / blank
        // line, so tier-1 keeps it as one segment; total > the small budget
        // so tier-2 line-splits it into multiple pieces.
        let page_text = std::iter::repeat_n("wordword", 80)
            .collect::<Vec<_>>()
            .join("\n");
        let doc = make_pdf_doc(&[&page_text]);
        let budget = 20; // 60 bytes/piece → forces a multi-piece line split
        let policy = default_policy(500, 80);
        let chunks = chunker(budget).chunk(&doc, &policy).unwrap();

        assert!(chunks.len() >= 2, "newline page must tier-2 split");

        // Every piece ≤ budget.
        for c in &chunks {
            assert!(
                c.text.len().div_ceil(BYTES_PER_TOKEN) <= budget,
                "piece exceeds budget"
            );
        }
        // '\n'-join reconstructs the page (line splits drop the separator,
        // re-added by the join — proves no content is lost or duplicated).
        let rejoined = chunks
            .iter()
            .map(|c| c.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(rejoined, page_text, "line-split pieces must reconstruct the page");

        // Every piece carries the parent-segment span [0, page_chars] — no
        // drift (the old running-offset would have under-counted by the lost
        // '\n' separators and produced char_end < page_chars).
        let page_chars = page_text.chars().count() as u32;
        for c in &chunks {
            match c.source_spans[0] {
                SourceSpan::Page {
                    char_start: Some(s),
                    char_end: Some(e),
                    ..
                } => {
                    assert_eq!(s, 0, "segment start");
                    assert_eq!(e, page_chars, "segment end (no drift)");
                }
                ref other => panic!("expected Page span, got {other:?}"),
            }
        }
        // chunk_ids unique.
        let mut ids: Vec<&str> = chunks.iter().map(|c| c.chunk_id.0.as_str()).collect();
        let total = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), total, "sub-piece chunk_ids unique");
    }

    /// NEW (pdf-page-v1.2): a page comfortably under budget must produce
    /// output byte-identical to the v1.1 pre-pass — one chunk, full text,
    /// full-page char span, and the BARE `#c{segment_start}` id form (no
    /// `s{i}` suffix). This locks the common case to "unchanged from v1.1".
    #[test]
    fn non_oversize_pdf_unchanged() {
        let page_text = "A short page that fits well under the budget.";
        let doc = make_pdf_doc(&[page_text]);
        let policy = default_policy(500, 80);

        // Generous budget — tier-2 never fires.
        let v12 = chunker(BIG_BUDGET).chunk(&doc, &policy).unwrap();
        assert_eq!(v12.len(), 1, "under-budget page is a single chunk");
        let c = &v12[0];
        assert_eq!(c.text, page_text, "text is the full page");
        assert_eq!(c.token_estimate, page_text.len().div_ceil(BYTES_PER_TOKEN));
        assert_eq!(c.heading_path, Vec::<String>::new());

        // Full-page char span (char_start = 0, char_end = page char count).
        match c.source_spans[0] {
            SourceSpan::Page {
                page,
                char_start,
                char_end,
            } => {
                assert_eq!(page, 1);
                assert_eq!(char_start, Some(0));
                assert_eq!(char_end, Some(page_text.chars().count() as u32));
            }
            ref other => panic!("expected Page span, got {other:?}"),
        }

        // chunk_id uses the bare `#c0` id form (segment_start = 0, no `s{i}`
        // suffix). We reconstruct the expected id with the v1.1 recipe and
        // compare — proving the common-case id is byte-identical to a
        // single-piece (non-oversize) emission.
        let base = chunker(BIG_BUDGET).policy_hash(&policy);
        let block_id = match &doc.blocks[0] {
            Block::Paragraph(p) => p.common.block_id.clone(),
            _ => unreachable!(),
        };
        let expected_id = id_for_chunk(
            &doc.doc_id,
            &chunker(BIG_BUDGET).chunker_version(),
            &[block_id],
            &format!("{base}#c0"),
        );
        assert_eq!(
            c.chunk_id, expected_id,
            "single-piece chunk_id must use the bare #c{{segment_start}} form (no s-suffix)"
        );
    }
}
