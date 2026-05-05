//! `md-heading-v1` — heading-aware Markdown chunker.

use kebab_core::{
    Block, BlockId, CanonicalDocument, Chunk, ChunkPolicy, Chunker,
    ChunkerVersion, DocumentId, SourceSpan, id_for_chunk,
};

/// Version label emitted by [`MdHeadingV1Chunker`]. Bumping this label
/// invalidates every downstream embedding record (design §9), so any change
/// must ship with a documented migration plan.
const VERSION_LABEL: &str = "md-heading-v1";

/// Bytes-per-token proxy. We over-estimate (smaller divisor → larger
/// token count) so that real tokenizers downstream never see a chunk
/// exceeding their budget. English averages ~4 bytes/token under BPE,
/// Korean averages ~3 bytes/token under E5; picking 3 covers both.
const BYTES_PER_TOKEN: usize = 3;

/// Maximum hex characters of `blake3(canonical_json(policy))` retained
/// in `policy_hash`. 16 hex chars = 64 bits of policy entropy, which is
/// far beyond enough to disambiguate the handful of policy variants a
/// single workspace will see.
const POLICY_HASH_HEX_LEN: usize = 16;

/// Heading-aware Markdown chunker.
///
/// Implements [`kebab_core::Chunker`] for Markdown-derived
/// [`CanonicalDocument`]s.
///
/// **Behavior contract** (design §0 / §14, in priority order):
///
/// 1. **Heading boundary first.** Chunks never span a `Block::Heading`.
///    The Heading block itself starts a new chunk and is included in that
///    chunk's `block_ids` so heading text is retrievable.
/// 2. **Never split a code block.** A `Block::Code` always lives in a
///    single chunk even when it exceeds `target_tokens`.
/// 3. **Tables stay in one chunk.** A `Block::Table` is emitted as a
///    single chunk regardless of size — the row-split refinement is
///    deferred per the P1-5 task spec.
/// 4. **Long sections split by paragraph.** Within a heading section
///    the chunker accumulates blocks until adding the next would exceed
///    `target_tokens`; it then emits the chunk and seeds the next chunk
///    with the previous chunk's tail blocks contributing roughly
///    `overlap_tokens` of content (paragraph-level overlap).
/// 5. **`heading_path` propagates.** Each chunk's `heading_path` is the
///    `heading_path` of its first contributing non-Heading block, or —
///    when the chunk leads with (or contains only) a Heading — the
///    parent path **plus the heading's own text** so heading-only or
///    heading-led chunks never lose their citation context.
/// 6. **`source_spans` merge.** A chunk lists every contributing block's
///    `source_span` in document order.
/// 7. **Version + policy hash recorded.** Each chunk records
///    `chunker_version = "md-heading-v1"`. The current `policy_hash` is
///    folded into the `chunk_id` recipe (design §4.2) so changing
///    `target_tokens` / `overlap_tokens` produces fresh chunk IDs.
///
/// `ImageRef` and `AudioRef` blocks are emitted as their own chunks so
/// future image/audio search can locate them. Their `text` is the alt /
/// caption preview (empty string if unavailable) and `token_estimate = 0`.
///
/// **Token-estimate proxy.** Until a real tokenizer is wired in (P3), the
/// estimator counts UTF-8 bytes and divides by [`BYTES_PER_TOKEN`]. The
/// constant is deliberately small (3) so the proxy *over*-estimates token
/// count — chunks sized against this proxy are guaranteed to fit in any
/// real BPE tokenizer's budget for English (~4 bytes/token) or Korean
/// (~3 bytes/token under E5/M-BERT). See [`BYTES_PER_TOKEN`] for rationale.
///
/// **`policy.respect_markdown_headings`.** This field flows into
/// `policy_hash` (so flipping it yields fresh chunk IDs), but the
/// chunker variant `md-heading-v1` unconditionally treats headings as
/// boundaries by design — the `md-heading-v1` name is the contract. To
/// disable heading awareness, ship a different `chunker_version`; none
/// is shipped in P1-5.
#[derive(Clone, Copy, Debug, Default)]
pub struct MdHeadingV1Chunker;

impl Chunker for MdHeadingV1Chunker {
    fn chunker_version(&self) -> ChunkerVersion {
        ChunkerVersion(VERSION_LABEL.to_string())
    }

    /// Compute the policy hash folded into `chunk_id` per design §4.2.
    ///
    /// # Panics
    ///
    /// Panics if canonical JSON serialization of `ChunkPolicy` fails.
    /// This is unreachable in practice — `ChunkPolicy` is composed of
    /// owned primitives (`usize`, `bool`, owned `String`) and
    /// `serde_json_canonicalizer::to_vec` only fails on
    /// non-serializable values such as non-finite floats or maps with
    /// non-string keys, neither of which can be constructed via
    /// `ChunkPolicy`'s public surface. The `expect` is preserved as a
    /// future-proofing guard against drift if `ChunkPolicy` ever gains
    /// a field with such a property.
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
        let policy_hash = self.policy_hash(policy);
        let chunker_version = self.chunker_version();
        let mut out: Vec<Chunk> = Vec::new();

        // Running accumulator: the paragraphs/lists/quotes (and the
        // optional leading heading) that will be glued into the next
        // emitted chunk.
        let mut acc = ChunkAcc::default();

        for block in &doc.blocks {
            match block {
                Block::Heading(_) => {
                    // §0/§14 priority 1: heading is a hard boundary.
                    // Flush whatever has accumulated, then seed a new
                    // accumulator that owns this heading.
                    flush(&mut acc, doc, &chunker_version, &policy_hash, &mut out);
                    acc.push_block(block);
                }
                Block::Code(_) | Block::Table(_) => {
                    // Atomic non-splittable text blocks. Flush running
                    // accumulator, then emit the atomic block as its
                    // own chunk. (Code never splits per priority 2;
                    // tables stay single per priority 3.)
                    flush(&mut acc, doc, &chunker_version, &policy_hash, &mut out);
                    let mut single = ChunkAcc::default();
                    single.push_block(block);
                    flush(&mut single, doc, &chunker_version, &policy_hash, &mut out);
                }
                Block::ImageRef(_) | Block::AudioRef(_) => {
                    // Independent searchable artifacts. token_estimate=0
                    // is enforced inside `build_chunk` for these kinds.
                    flush(&mut acc, doc, &chunker_version, &policy_hash, &mut out);
                    let mut single = ChunkAcc::default();
                    single.push_block(block);
                    flush(&mut single, doc, &chunker_version, &policy_hash, &mut out);
                }
                Block::Paragraph(_) | Block::List(_) | Block::Quote(_) => {
                    // Soft-split candidates. If adding this block would
                    // exceed target_tokens (and we already have at least
                    // one non-heading block in the accumulator), emit
                    // the current chunk and seed the next one with
                    // overlap from the prior tail.
                    let next_tokens = estimate_block_tokens(block);
                    // Note: `acc.text_tokens` already includes the prior
                    // chunk's overlap seed. The clamp in
                    // `collect_overlap_seed` keeps seed ≤ target/2, so
                    // a flush here never produces a chunk smaller than
                    // the seed budget.
                    let would_exceed = acc.text_tokens + next_tokens
                        > policy.target_tokens
                        && acc.has_non_heading_content();
                    if would_exceed {
                        let overlap_seed = collect_overlap_seed(
                            &acc,
                            policy.overlap_tokens,
                            policy.target_tokens,
                        );
                        flush(
                            &mut acc,
                            doc,
                            &chunker_version,
                            &policy_hash,
                            &mut out,
                        );
                        // Seed next accumulator with the prior chunk's
                        // tail blocks (paragraph-level overlap). The
                        // heading is *not* re-included here — it lives
                        // on the prior chunk. The follow-on chunk's
                        // heading_path is taken from the first seeded
                        // block (which carries the same path, as it sat
                        // under the same heading).
                        for b in overlap_seed {
                            acc.push_block(b);
                        }
                    }
                    acc.push_block(block);
                }
            }
        }
        flush(&mut acc, doc, &chunker_version, &policy_hash, &mut out);

        tracing::debug!(
            target: "kebab-chunk",
            doc_id = %doc.doc_id,
            chunks = out.len(),
            "md-heading-v1 chunked",
        );

        Ok(out)
    }
}

/// Internal accumulator: pointers to blocks (lifetime-bound to the
/// `CanonicalDocument`) plus the running token estimate of their text.
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

    /// True if any non-heading block sits in the accumulator. Used to
    /// avoid splitting a chunk that contains only its leading heading
    /// (which would emit a heading-only chunk before any prose).
    fn has_non_heading_content(&self) -> bool {
        self.blocks.iter().any(|b| !matches!(b, Block::Heading(_)))
    }
}

/// Drain `acc` into a fresh `Chunk` and push to `out`. No-op when empty.
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

/// Collect the trailing blocks of `acc` (in document order) whose
/// combined token estimate fits under the seed budget. The heading
/// block (if it leads the accumulator) is excluded from the seed —
/// re-emitting the heading would conflate it with the next chunk's
/// own heading_path provenance.
///
/// The seed budget is clamped to `min(overlap_tokens, target_tokens / 2)`.
/// Without the clamp, an `overlap_tokens >= target_tokens` policy
/// degenerates into 1-block-per-chunk: the seed already exceeds budget
/// before any new content lands, so the very next paragraph trips the
/// `would_exceed` flush. Halving the target guarantees the seed leaves
/// at least target/2 worth of room for fresh content in the next chunk.
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
            // Don't propagate the heading itself into the next chunk;
            // its `heading_path` carries naturally on the next blocks
            // (kb-normalize stamps every block under a heading with
            // that heading's path).
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

/// Construct a `Chunk` from a non-empty contiguous slice of blocks.
fn build_chunk(
    doc: &CanonicalDocument,
    blocks: &[&Block],
    chunker_version: &ChunkerVersion,
    policy_hash: &str,
) -> Chunk {
    debug_assert!(!blocks.is_empty(), "build_chunk requires ≥1 block");

    let block_ids: Vec<BlockId> =
        blocks.iter().map(|b| common(b).block_id.clone()).collect();
    let source_spans: Vec<SourceSpan> =
        blocks.iter().map(|b| common(b).source_span.clone()).collect();

    // heading_path: pick the first non-Heading block's heading_path
    // (which already includes every parent heading per kb-normalize).
    // When the FIRST block is a Heading — either a heading-only chunk,
    // or a chunk that leads with `# H1` immediately followed by another
    // Heading or atomic block — the Heading block's own
    // `common.heading_path` records only its *parents* (kb-normalize
    // does not include a heading inside its own path). We synthesize
    // the leading heading into the path so the citation context is not
    // lost on patterns like `# Alpha\n## Beta\n...`.
    let heading_path = match blocks[0] {
        Block::Heading(h) => {
            let mut path = h.common.heading_path.clone();
            path.push(h.text.clone());
            path
        }
        _ => common(blocks[0]).heading_path.clone(),
    };

    // Text rendering: simple double-newline join of each block's
    // contribution. We deliberately pick a stable, low-fidelity
    // representation — embedding-quality rewrites land in P3.
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
        // Token estimate is bytes / BYTES_PER_TOKEN, rounded up so the
        // proxy never under-counts.
        text.len().div_ceil(BYTES_PER_TOKEN)
    };

    let chunk_id = id_for_chunk(
        &doc.doc_id,
        chunker_version,
        &block_ids,
        policy_hash,
    );

    Chunk {
        chunk_id,
        doc_id: DocumentId(doc.doc_id.0.clone()),
        block_ids,
        text,
        heading_path,
        source_spans,
        token_estimate,
        chunker_version: chunker_version.clone(),
        policy_hash: policy_hash.to_string(),
    }
}

/// Render a block's contribution to a chunk's `text`. The rendering is
/// deliberately minimal — embedding-time normalization is a P3 concern.
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
            // Headers row joined with " | ", then each row likewise.
            let mut s = t.headers.join(" | ");
            for row in &t.rows {
                s.push('\n');
                s.push_str(&row.join(" | "));
            }
            s
        }
        // ImageRef text portion follows the P6-4 (β) plain-concat
        // contract — `[alt, ocr.joined, caption.text]` joined by
        // `\n\n`, dropping empty parts. Filename fallback for empty
        // alt keeps lexical search hits on filenames working even when
        // P6-1's filename auto-fill is bypassed.
        Block::ImageRef(i) => {
            let alt = if !i.alt.is_empty() {
                i.alt.clone()
            } else {
                // P6-1 falls back to filename so this branch is
                // defensive — keep it lest a future test fixture or
                // synthetic block path skip the auto-fill.
                i.src
                    .rsplit('/')
                    .next()
                    .filter(|s| !s.is_empty())
                    .unwrap_or("[image]")
                    .to_string()
            };
            let ocr = i
                .ocr
                .as_ref()
                .map(|o| o.joined.as_str())
                .unwrap_or("");
            let cap = i
                .caption
                .as_ref()
                .map(|c| c.text.as_str())
                .unwrap_or("");
            [alt.as_str(), ocr, cap]
                .iter()
                .filter(|s| !s.is_empty())
                .copied()
                .collect::<Vec<_>>()
                .join("\n\n")
        }
        // AudioRef has no caption preview yet (transcript joins land
        // in P8). Empty string per task spec.
        Block::AudioRef(_) => String::new(),
    }
}

fn estimate_block_tokens(b: &Block) -> usize {
    match b {
        // ImageRef / AudioRef contribute 0 — they are independent
        // chunks and never participate in size accounting.
        Block::ImageRef(_) | Block::AudioRef(_) => 0,
        _ => render_block_text(b).len().div_ceil(BYTES_PER_TOKEN),
    }
}

/// Borrow the `CommonBlock` of any [`Block`] variant.
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

#[cfg(test)]
mod tests {
    use super::*;
    use kebab_core::{
        AssetId, CodeBlock, CommonBlock, HeadingBlock, ImageRefBlock, Lang,
        Metadata, Provenance, SourceType, TableBlock, TextBlock, TrustLevel,
        WorkspacePath, id_for_block,
    };
    use time::OffsetDateTime;

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

    /// Heading variant that carries a parent path — kb-normalize stamps
    /// every block under `# Alpha` with `heading_path = []` for the H1
    /// itself but `["Alpha"]` for the H2 that follows. Tests covering
    /// the heading-only chunk path (I2) need that asymmetry.
    fn heading_with_parents(
        level: u8,
        text: &str,
        parents: &[&str],
        ordinal: u32,
        line: u32,
    ) -> Block {
        let hp: Vec<String> = parents.iter().map(|s| (*s).into()).collect();
        Block::Heading(HeadingBlock {
            common: common_for("heading", &hp, ordinal, span(line, line)),
            level,
            text: text.into(),
        })
    }

    fn paragraph(
        text: &str,
        heading_path: &[&str],
        ordinal: u32,
        line: u32,
    ) -> Block {
        let hp: Vec<String> = heading_path.iter().map(|s| (*s).into()).collect();
        Block::Paragraph(TextBlock {
            common: common_for("paragraph", &hp, ordinal, span(line, line)),
            text: text.into(),
            inlines: vec![],
        })
    }

    fn code_block(
        code: &str,
        heading_path: &[&str],
        ordinal: u32,
        s: SourceSpan,
    ) -> Block {
        let hp: Vec<String> = heading_path.iter().map(|s| (*s).into()).collect();
        Block::Code(CodeBlock {
            common: common_for("code", &hp, ordinal, s),
            lang: Some("rust".into()),
            code: code.into(),
        })
    }

    fn table(
        headers: Vec<&str>,
        rows: Vec<Vec<&str>>,
        heading_path: &[&str],
        ordinal: u32,
        s: SourceSpan,
    ) -> Block {
        let hp: Vec<String> = heading_path.iter().map(|s| (*s).into()).collect();
        Block::Table(TableBlock {
            common: common_for("table", &hp, ordinal, s),
            headers: headers.into_iter().map(String::from).collect(),
            rows: rows
                .into_iter()
                .map(|r| r.into_iter().map(String::from).collect())
                .collect(),
        })
    }

    fn image_ref(
        alt: &str,
        heading_path: &[&str],
        ordinal: u32,
        line: u32,
    ) -> Block {
        let hp: Vec<String> = heading_path.iter().map(|s| (*s).into()).collect();
        Block::ImageRef(ImageRefBlock {
            common: common_for("imageref", &hp, ordinal, span(line, line)),
            asset_id: None,
            src: "img.png".into(),
            alt: alt.into(),
            ocr: None,
            caption: None,
        })
    }

    fn default_policy(target: usize, overlap: usize) -> ChunkPolicy {
        ChunkPolicy {
            target_tokens: target,
            overlap_tokens: overlap,
            respect_markdown_headings: true,
            chunker_version: ChunkerVersion(VERSION_LABEL.into()),
        }
    }

    #[test]
    fn chunker_version_is_md_heading_v1() {
        assert_eq!(
            MdHeadingV1Chunker.chunker_version(),
            ChunkerVersion(VERSION_LABEL.to_string())
        );
    }

    #[test]
    fn policy_hash_is_deterministic_and_16_hex() {
        let p = default_policy(500, 80);
        let h1 = MdHeadingV1Chunker.policy_hash(&p);
        let h2 = MdHeadingV1Chunker.policy_hash(&p);
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), POLICY_HASH_HEX_LEN);
        assert!(h1.bytes().all(|b| b.is_ascii_hexdigit()));
    }

    #[test]
    fn policy_hash_differs_when_policy_differs() {
        let p1 = default_policy(500, 80);
        let p2 = default_policy(500, 0);
        assert_ne!(
            MdHeadingV1Chunker.policy_hash(&p1),
            MdHeadingV1Chunker.policy_hash(&p2)
        );
    }

    /// Heading boundary respected: two H2 sections produce separate
    /// chunks; no chunk's block_ids straddle the H2→H2 boundary.
    #[test]
    fn heading_boundary_respected() {
        let blocks = vec![
            heading(2, "First", 0, 1),
            paragraph("body of first", &["First"], 0, 2),
            heading(2, "Second", 1, 3),
            paragraph("body of second", &["Second"], 0, 4),
        ];
        let doc = make_doc(blocks);
        let chunks = MdHeadingV1Chunker
            .chunk(&doc, &default_policy(10_000, 0))
            .unwrap();
        assert_eq!(chunks.len(), 2);
        // First chunk = (heading "First", paragraph)
        assert_eq!(chunks[0].block_ids.len(), 2);
        // Second chunk = (heading "Second", paragraph)
        assert_eq!(chunks[1].block_ids.len(), 2);
        // heading_path on chunk 0 belongs to "First" section.
        assert_eq!(chunks[0].heading_path, vec!["First".to_string()]);
        assert_eq!(chunks[1].heading_path, vec!["Second".to_string()]);
    }

    /// A code block of ~800 tokens (≈2400 bytes) stays in a single
    /// chunk even when target=500.
    #[test]
    fn code_block_never_splits() {
        // 2400 bytes ≈ 800 tokens at BYTES_PER_TOKEN=3.
        let big = "x".repeat(2400);
        let blocks = vec![code_block(&big, &[], 0, span(1, 50))];
        let doc = make_doc(blocks);
        let chunks = MdHeadingV1Chunker
            .chunk(&doc, &default_policy(500, 80))
            .unwrap();
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].block_ids.len(), 1);
        assert!(chunks[0].token_estimate > 500);
    }

    /// A table of size < 2× target stays in a single chunk.
    #[test]
    fn table_stays_single_chunk_when_small() {
        let t = table(
            vec!["a", "b", "c"],
            vec![vec!["1", "2", "3"], vec!["4", "5", "6"]],
            &[],
            0,
            span(1, 4),
        );
        let blocks = vec![t];
        let doc = make_doc(blocks);
        let chunks = MdHeadingV1Chunker
            .chunk(&doc, &default_policy(500, 80))
            .unwrap();
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].block_ids.len(), 1);
    }

    /// A long sequence of paragraphs splits at target_tokens with
    /// overlap_tokens worth of seeded paragraph from the prior chunk.
    #[test]
    fn long_section_splits_with_overlap() {
        // Each paragraph is 60 bytes ≈ 20 tokens. target=50, overlap=20
        // → after ~3 paragraphs we hit the target; the next chunk
        // starts seeded with one paragraph from the prior tail.
        let mut bs = vec![heading(2, "Long", 0, 1)];
        for i in 0..6u32 {
            bs.push(paragraph(&"x".repeat(60), &["Long"], i, i + 2));
        }
        let doc = make_doc(bs);
        let chunks = MdHeadingV1Chunker
            .chunk(&doc, &default_policy(50, 20))
            .unwrap();
        assert!(
            chunks.len() >= 2,
            "expected ≥2 chunks, got {}: {chunks:#?}",
            chunks.len()
        );
        // Every chunk lives under the same heading_path "Long".
        for c in &chunks {
            assert_eq!(c.heading_path, vec!["Long".to_string()]);
        }
        // Overlap propagates: the last block_id of chunk N appears in
        // chunk N+1's block_ids (paragraph-level overlap rule).
        for w in chunks.windows(2) {
            let prev_tail = w[0].block_ids.last().unwrap();
            assert!(
                w[1].block_ids.contains(prev_tail),
                "chunk N+1 must seed from chunk N's tail; \
                 prev_tail={prev_tail:?}, next ids={:?}",
                w[1].block_ids
            );
        }
    }

    /// P6-4 (β) plain concatenation — alt + ocr.joined + caption.text
    /// joined by `\n\n`, dropping empty parts. Verifies all four
    /// (alt-only, alt+ocr, alt+caption, alt+ocr+caption) shapes.
    #[test]
    fn image_ref_p6_4_plain_concat_drops_empty_parts() {
        use kebab_core::{ModelCaption, OcrText};

        let mk = |alt: &str, ocr: Option<&str>, cap: Option<&str>| {
            Block::ImageRef(ImageRefBlock {
                common: common_for("imageref", &[], 0, span(1, 1)),
                asset_id: None,
                src: "img.png".into(),
                alt: alt.into(),
                ocr: ocr.map(|t| OcrText {
                    joined: t.into(),
                    regions: vec![],
                    engine: "test".into(),
                    engine_version: "v1".into(),
                }),
                caption: cap.map(|t| ModelCaption {
                    text: t.into(),
                    model: "m".into(),
                    model_version: "v".into(),
                }),
            })
        };

        // alt-only — no separators between empty parts.
        assert_eq!(render_block_text(&mk("photo.png", None, None)), "photo.png");

        // alt + ocr — joined by exactly one `\n\n`.
        assert_eq!(
            render_block_text(&mk("photo.png", Some("Hello"), None)),
            "photo.png\n\nHello"
        );

        // alt + caption.
        assert_eq!(
            render_block_text(&mk("photo.png", None, Some("a red square"))),
            "photo.png\n\na red square"
        );

        // alt + ocr + caption — three parts joined by `\n\n` each.
        assert_eq!(
            render_block_text(&mk("photo.png", Some("Hello"), Some("a red square"))),
            "photo.png\n\nHello\n\na red square"
        );

        // empty alt — falls back to filename derived from `src`.
        let blk = mk("", Some("text from image"), None);
        assert_eq!(
            render_block_text(&blk),
            "img.png\n\ntext from image",
            "empty alt must fall back to the basename of `src`"
        );
    }

    /// ImageRef → own chunk, token_estimate=0.
    #[test]
    fn image_ref_emits_own_chunk_zero_tokens() {
        let blocks = vec![
            heading(2, "With image", 0, 1),
            paragraph("intro", &["With image"], 0, 2),
            image_ref("a cat", &["With image"], 0, 3),
            paragraph("after", &["With image"], 1, 4),
        ];
        let doc = make_doc(blocks);
        let chunks = MdHeadingV1Chunker
            .chunk(&doc, &default_policy(10_000, 0))
            .unwrap();
        // Expect: (heading + intro), (image), (after). The image must
        // be its own chunk and carry token_estimate=0.
        assert!(chunks.len() >= 3, "unexpected chunk count: {chunks:#?}");
        let img_chunk = chunks
            .iter()
            .find(|c| c.text == "a cat")
            .expect("image chunk present");
        assert_eq!(img_chunk.token_estimate, 0);
        assert_eq!(img_chunk.block_ids.len(), 1);
    }

    /// Identical input + identical policy → identical chunk_ids over
    /// 1000 iterations.
    #[test]
    fn deterministic_chunk_ids_1000() {
        let blocks = vec![
            heading(2, "Det", 0, 1),
            paragraph("body 1", &["Det"], 0, 2),
            paragraph("body 2", &["Det"], 1, 3),
            heading(2, "Det 2", 1, 4),
            paragraph("body 3", &["Det 2"], 0, 5),
        ];
        let doc = make_doc(blocks);
        let policy = default_policy(50, 10);
        let baseline: Vec<String> = MdHeadingV1Chunker
            .chunk(&doc, &policy)
            .unwrap()
            .into_iter()
            .map(|c| c.chunk_id.0)
            .collect();
        for _ in 0..1000 {
            let again: Vec<String> = MdHeadingV1Chunker
                .chunk(&doc, &policy)
                .unwrap()
                .into_iter()
                .map(|c| c.chunk_id.0)
                .collect();
            assert_eq!(again, baseline);
        }
    }

    /// I2 regression: when a Heading is followed immediately by another
    /// Heading or atomic block (no intervening prose), the resulting
    /// heading-only / heading-led chunk must carry the heading text in
    /// its own `heading_path`. Pattern: `# Alpha`, `## Beta`, code.
    ///
    /// Before the fix, chunk[0] (Heading-only "Alpha") would have
    /// `heading_path = []` because `kb-normalize` does not stamp a
    /// heading inside its own path; the chunker fell back to the
    /// heading's parent path. After the fix it is `["Alpha"]`.
    ///
    /// `chunk_id` recipe (`doc_id, chunker_version, block_ids,
    /// policy_hash`) does NOT include `heading_path`, so this fix does
    /// NOT shift chunk_ids — only `heading_path` fields.
    #[test]
    fn heading_only_chunk_carries_self_in_path() {
        // # Alpha (H1, no parents)
        // ## Beta (H2, parent = ["Alpha"])
        // ```rust ... ``` (code, heading_path = ["Alpha", "Beta"])
        let blocks = vec![
            heading_with_parents(1, "Alpha", &[], 0, 1),
            heading_with_parents(2, "Beta", &["Alpha"], 0, 2),
            code_block("fn x() {}", &["Alpha", "Beta"], 0, span(3, 3)),
        ];
        let doc = make_doc(blocks);
        let chunks = MdHeadingV1Chunker
            .chunk(&doc, &default_policy(10_000, 0))
            .unwrap();
        // Three chunks: Heading-only Alpha, Heading-only Beta, code.
        assert_eq!(chunks.len(), 3, "got {chunks:#?}");
        assert_eq!(chunks[0].heading_path, vec!["Alpha".to_string()]);
        assert_eq!(
            chunks[1].heading_path,
            vec!["Alpha".to_string(), "Beta".to_string()]
        );
        assert_eq!(
            chunks[2].heading_path,
            vec!["Alpha".to_string(), "Beta".to_string()]
        );
    }

    /// I3 regression: a pathological policy with
    /// `overlap_tokens >= target_tokens` must NOT degenerate into
    /// 1-block-per-chunk. The seed budget is clamped to `target/2`,
    /// guaranteeing every flushed chunk has space for fresh content.
    #[test]
    fn overlap_clamped_when_overlap_exceeds_target() {
        // 5 paragraphs of ~20 tokens each (60 bytes / 3 BPT).
        // target = 50, overlap = 200 (4× target → would trip flush
        // immediately without clamp).
        let mut bs = vec![heading_with_parents(2, "Long", &[], 0, 1)];
        for i in 0..5u32 {
            bs.push(paragraph(&"x".repeat(60), &["Long"], i, i + 2));
        }
        let doc = make_doc(bs);
        let policy = ChunkPolicy {
            target_tokens: 50,
            overlap_tokens: 200,
            respect_markdown_headings: true,
            chunker_version: ChunkerVersion(VERSION_LABEL.into()),
        };
        let chunks = MdHeadingV1Chunker.chunk(&doc, &policy).unwrap();
        // Without the clamp, every chunk after the first would have
        // exactly 1 paragraph (because seed alone already exceeds
        // target and acc.has_non_heading_content() is true the moment
        // any seed lands). With the clamp, follow-on chunks must hold
        // at least the seed paragraph + the new paragraph = ≥2 blocks.
        for (i, c) in chunks.iter().enumerate() {
            // The very first chunk includes the heading + first para
            // (no seed), so it is also ≥2. Subsequent chunks must be
            // seed+new ≥ 2.
            assert!(
                c.block_ids.len() >= 2,
                "chunk {i} degenerated to {} block(s); pathology not \
                 prevented: {chunks:#?}",
                c.block_ids.len()
            );
        }
    }
}
