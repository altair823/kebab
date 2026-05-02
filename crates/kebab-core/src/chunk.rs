//! Chunk (§3.5).

use serde::{Deserialize, Serialize};

use crate::document::SourceSpan;
use crate::ids::{BlockId, ChunkId, DocumentId};
use crate::versions::ChunkerVersion;

/// A unit of retrievable text per design §3.5 + §5.5.
///
/// `policy_hash` is the chunker's hex digest of the active `ChunkPolicy`
/// (e.g. `target_tokens`, `overlap_tokens`). It mirrors the §5.5 SQLite
/// schema column so persistence is a straight copy, and feeds the
/// `chunk_id` recipe (§4.2) so policy edits invalidate downstream IDs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Chunk {
    pub chunk_id: ChunkId,
    pub doc_id: DocumentId,
    pub block_ids: Vec<BlockId>,
    pub text: String,
    pub heading_path: Vec<String>,
    pub source_spans: Vec<SourceSpan>,
    pub token_estimate: usize,
    pub chunker_version: ChunkerVersion,
    pub policy_hash: String,
}
