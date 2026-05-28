//! p10-2: manifest whole-file chunker (Tier 2).
//!
//! Reads entire manifest file (Cargo.toml / package.json / pom.xml / go.mod /
//! build.gradle / pyproject.toml / tsconfig.json) and emits a single Chunk
//! with symbol "<manifest>", code_lang read from Block::Code.lang, line range
//! 1..EOF. Oversize >200 lines splits into line-windows sharing the symbol via
//! tier2_shared::push_chunks_with_oversize.

use crate::tier2_shared::{policy_hash, push_chunks_with_oversize};
use anyhow::Result;
use kebab_core::{Block, CanonicalDocument, Chunk, ChunkPolicy, Chunker, ChunkerVersion};

pub const VERSION_LABEL: &str = "manifest-file-v1";

#[derive(Clone, Copy, Debug, Default)]
pub struct ManifestFileV1Chunker;

impl Chunker for ManifestFileV1Chunker {
    fn chunker_version(&self) -> ChunkerVersion {
        ChunkerVersion(VERSION_LABEL.to_string())
    }

    fn policy_hash(&self, policy: &ChunkPolicy) -> String {
        policy_hash(policy)
    }

    fn chunk(&self, doc: &CanonicalDocument, policy: &ChunkPolicy) -> Result<Vec<Chunk>> {
        // Expect a single Block::Code carrying the full manifest text.
        let (text, lang) = match doc.blocks.first() {
            Some(Block::Code(cb)) => (cb.code.as_str(), cb.lang.as_deref().unwrap_or("")),
            _ => return Ok(vec![]),
        };

        let total_lines = text.lines().count().max(1) as u32;
        let mut chunks = Vec::new();

        push_chunks_with_oversize(
            &mut chunks,
            doc,
            policy,
            text,
            1,
            total_lines,
            "<manifest>",
            lang,
            VERSION_LABEL,
            None,
        )?;

        tracing::debug!(
            target: "kebab-chunk",
            doc_id = %doc.doc_id,
            chunks = chunks.len(),
            "manifest-file-v1 chunked",
        );

        Ok(chunks)
    }
}
