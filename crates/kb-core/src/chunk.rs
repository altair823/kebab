//! Chunk (§3.5).

use serde::{Deserialize, Serialize};

use crate::document::SourceSpan;
use crate::ids::{BlockId, ChunkId, DocumentId};
use crate::versions::ChunkerVersion;

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
}
