//! `md-heading-v1` — heading-aware Markdown chunker.

use kb_core::{
    CanonicalDocument, Chunk, ChunkPolicy, Chunker, ChunkerVersion,
};

/// Version label emitted by [`MdHeadingV1Chunker`]. Bumping this label
/// invalidates every downstream embedding record (design §9), so any change
/// must ship with a documented migration plan.
const VERSION_LABEL: &str = "md-heading-v1";

/// Heading-aware Markdown chunker.
///
/// Implements [`kb_core::Chunker`] for Markdown-derived
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
///    `heading_path` of its first contributing non-Heading block, or the
///    Heading block itself if that is the first.
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
/// (~3 bytes/token under E5/M-BERT). See `BYTES_PER_TOKEN` for rationale.
#[derive(Clone, Copy, Debug, Default)]
pub struct MdHeadingV1Chunker;

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

impl Chunker for MdHeadingV1Chunker {
    fn chunker_version(&self) -> ChunkerVersion {
        ChunkerVersion(VERSION_LABEL.to_string())
    }

    fn policy_hash(&self, policy: &ChunkPolicy) -> String {
        let bytes = serde_json_canonicalizer::to_vec(policy)
            .expect("canonical JSON serialization of ChunkPolicy must not fail");
        let hex = blake3::hash(&bytes).to_hex().to_string();
        hex[..POLICY_HASH_HEX_LEN].to_string()
    }

    fn chunk(
        &self,
        _doc: &CanonicalDocument,
        _policy: &ChunkPolicy,
    ) -> anyhow::Result<Vec<Chunk>> {
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunker_version_is_md_heading_v1() {
        assert_eq!(
            MdHeadingV1Chunker.chunker_version(),
            ChunkerVersion(VERSION_LABEL.to_string())
        );
    }

    #[test]
    fn policy_hash_is_deterministic_and_16_hex() {
        let policy = ChunkPolicy {
            target_tokens: 500,
            overlap_tokens: 80,
            respect_markdown_headings: true,
            chunker_version: ChunkerVersion(VERSION_LABEL.to_string()),
        };
        let h1 = MdHeadingV1Chunker.policy_hash(&policy);
        let h2 = MdHeadingV1Chunker.policy_hash(&policy);
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), POLICY_HASH_HEX_LEN);
        assert!(h1.bytes().all(|b| b.is_ascii_hexdigit()));
    }

    #[test]
    fn policy_hash_differs_when_policy_differs() {
        let p1 = ChunkPolicy {
            target_tokens: 500,
            overlap_tokens: 80,
            respect_markdown_headings: true,
            chunker_version: ChunkerVersion(VERSION_LABEL.to_string()),
        };
        let p2 = ChunkPolicy {
            target_tokens: 500,
            overlap_tokens: 0, // <-- only this differs
            respect_markdown_headings: true,
            chunker_version: ChunkerVersion(VERSION_LABEL.to_string()),
        };
        assert_ne!(
            MdHeadingV1Chunker.policy_hash(&p1),
            MdHeadingV1Chunker.policy_hash(&p2)
        );
    }
}
