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
//! ## Splitting policy
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
//! - A page with no qualifying segment boundary AND text exceeding the
//!   budget (e.g. a 5,000-byte single sentence) emits one oversized
//!   chunk rather than hard-splitting mid-word — a real tokenizer slot
//!   in P+ replaces this proxy and can do better mid-sentence splitting
//!   when needed.
//! - Common English abbreviations (`Mr.`, `i.e.`, `e.g.`, `Fig. 3`)
//!   trip the sentence-end heuristic and produce spurious boundaries —
//!   accepted as a v1 limit. A real sentence segmenter lands with the
//!   P+ tokenizer slot.
//! - The effective overlap budget is clamped at `target_bytes / 2` so a
//!   pathological policy (`overlap_tokens >= target_tokens`) cannot
//!   make a chunk fully re-emit the previous chunk's text. Same guard
//!   pattern as `md-heading-v1::collect_overlap_seed`.
//!
//! ## `BYTES_PER_TOKEN`
//!
//! 3 — same calibration as `md-heading-v1` (covers Korean ≈ 3 b/tok and
//! over-estimates English ≈ 4 b/tok). The original p7-2 spec literal said
//! `× 4`, but cross-chunker comparability outweighs the spec literal here.
//! Logged in `tasks/HOTFIXES.md`.
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

use kebab_core::{
    Block, BlockId, CanonicalDocument, Chunk, ChunkPolicy, Chunker, ChunkerVersion, DocumentId,
    SourceSpan, id_for_chunk,
};

const VERSION_LABEL: &str = "pdf-page-v1.1";
const BYTES_PER_TOKEN: usize = 3;
const POLICY_HASH_HEX_LEN: usize = 16;

/// Page-aware PDF chunker. See module docs for the splitting policy and
/// the `chunk_id` collision-avoidance deviation.
#[derive(Clone, Copy, Debug, Default)]
pub struct PdfPageV1Chunker;

impl Chunker for PdfPageV1Chunker {
    fn chunker_version(&self) -> ChunkerVersion {
        ChunkerVersion(VERSION_LABEL.to_string())
    }

    /// blake3(canonical_json(policy)) truncated to 16 hex chars. Matches
    /// the `md-heading-v1` recipe so a workspace-wide policy hash lookup
    /// (e.g. for invalidation reports) yields the same digest across
    /// chunkers.
    fn policy_hash(&self, policy: &ChunkPolicy) -> String {
        let bytes = serde_json_canonicalizer::to_vec(policy)
            .expect("canonical JSON serialization of ChunkPolicy must not fail");
        let hex = blake3::hash(&bytes).to_hex().to_string();
        hex[..POLICY_HASH_HEX_LEN].to_string()
    }

    fn chunk(
        &self,
        doc: &CanonicalDocument,
        policy: &ChunkPolicy,
    ) -> anyhow::Result<Vec<Chunk>> {
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
                anyhow::bail!(
                    "PdfPageV1Chunker only handles PDF docs (got non-Page source_span)"
                );
            }
        }

        let base_policy_hash = self.policy_hash(policy);
        let chunker_version = self.chunker_version();
        let target_bytes = policy
            .target_tokens
            .saturating_mul(BYTES_PER_TOKEN)
            .max(1);
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

            for (segment_start, char_start, char_end, slice) in
                chunk_page(&p.text, target_bytes, overlap_bytes)
            {
                // PDF chars-per-page comfortably fits in u32 (a single
                // page maxes out around ~10k chars even for dense
                // typography); silent `as u32` truncation would only
                // surface on corrupted input, where an explicit panic
                // is preferable to an off-by-2^32 span.
                let char_start_u32 = u32::try_from(char_start)
                    .expect("page chars fit in u32");
                let char_end_u32 =
                    u32::try_from(char_end).expect("page chars fit in u32");
                let span = SourceSpan::Page {
                    page: page_num,
                    char_start: Some(char_start_u32),
                    char_end: Some(char_end_u32),
                };
                let block_ids: Vec<BlockId> = vec![p.common.block_id.clone()];
                // v0.20.0 sub-item 1 bugfix (#3): per-chunk policy_hash
                // variant uses `segment_start` (pre-overlap boundary,
                // strictly increasing) instead of `char_start` (post-
                // overlap, may collapse to prev_min). See module docs +
                // spec §4.1 root cause + HOTFIXES.md 2026-05-27.
                let per_chunk_hash = format!("{base_policy_hash}#c{segment_start}");
                let chunk_id =
                    id_for_chunk(&doc.doc_id, &chunker_version, &block_ids, &per_chunk_hash);
                let token_estimate = slice.len().div_ceil(BYTES_PER_TOKEN);

                out.push(Chunk {
                    chunk_id,
                    doc_id: DocumentId(doc.doc_id.0.clone()),
                    block_ids,
                    text: slice,
                    heading_path: Vec::new(),
                    source_spans: vec![span],
                    token_estimate,
                    chunker_version: chunker_version.clone(),
                    policy_hash: base_policy_hash.clone(),
                });
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
fn chunk_page(text: &str, target_bytes: usize, overlap_bytes: usize) -> Vec<(usize, usize, usize, String)> {
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
        let is_sentence_end =
            matches!(c, '.' | '?' | '!') && nx.is_whitespace();
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
    let byte_len = |a: usize, b: usize| -> usize {
        chars[a..b].iter().map(|c| c.len_utf8()).sum()
    };

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

    #[test]
    fn chunker_version_is_pdf_page_v1() {
        assert_eq!(
            PdfPageV1Chunker.chunker_version(),
            ChunkerVersion(VERSION_LABEL.to_string())
        );
    }

    #[test]
    fn three_page_small_emits_one_chunk_per_page() {
        let doc = make_pdf_doc(&["page one", "page two", "page three"]);
        let chunks = PdfPageV1Chunker
            .chunk(&doc, &default_policy(500, 80))
            .unwrap();
        assert_eq!(chunks.len(), 3);
        for (i, c) in chunks.iter().enumerate() {
            assert_eq!(c.block_ids.len(), 1);
            assert_eq!(c.heading_path, Vec::<String>::new());
            assert_eq!(c.source_spans.len(), 1);
            match c.source_spans[0] {
                SourceSpan::Page { page, char_start, char_end } => {
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
        let chunks = PdfPageV1Chunker
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
                SourceSpan::Page { char_end: Some(e), .. } => e,
                _ => panic!("missing char_end"),
            };
            let next_start = match w[1].source_spans[0] {
                SourceSpan::Page { char_start: Some(s), .. } => s,
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
        let chunks = PdfPageV1Chunker
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
        let chunks = PdfPageV1Chunker
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
            },
            provenance: Provenance { events: vec![] },
            parser_version,
            schema_version: 1,
            doc_version: 1,
            last_chunker_version: None,
            last_embedding_version: None,
        };
        let err = PdfPageV1Chunker
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
        let chunks = PdfPageV1Chunker
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
        let baseline: Vec<String> = PdfPageV1Chunker
            .chunk(&doc, &policy)
            .unwrap()
            .into_iter()
            .map(|c| c.chunk_id.0)
            .collect();
        for _ in 0..1000 {
            let again: Vec<String> = PdfPageV1Chunker
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
        let chunks = PdfPageV1Chunker
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
        let chunks = PdfPageV1Chunker.chunk(&doc, &policy).unwrap();
        // For each consecutive pair, the new chunk's actual_start must
        // be strictly greater than the previous chunk's actual_start
        // (no full re-emission). Without the clamp, equality (full
        // overlap) is the failure mode.
        for w in chunks.windows(2) {
            let prev_start = match w[0].source_spans[0] {
                SourceSpan::Page { char_start: Some(s), .. } => s,
                _ => panic!("missing char_start"),
            };
            let next_start = match w[1].source_spans[0] {
                SourceSpan::Page { char_start: Some(s), .. } => s,
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
        let policy = default_policy(500, 80);  // target=1500 byte, overlap=240 byte
        let chunks = PdfPageV1Chunker.chunk(&doc, &policy).unwrap();

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

    #[test]
    fn policy_hash_matches_md_heading_v1_for_identical_policy() {
        // Cross-chunker policy fingerprint identity — important so a
        // workspace-wide "show me chunks with policy_hash = X" query
        // covers both chunkers without per-chunker logic.
        let p = default_policy(500, 80);
        let pdf = PdfPageV1Chunker.policy_hash(&p);
        let md = crate::MdHeadingV1Chunker.policy_hash(&p);
        assert_eq!(pdf, md);
    }
}
