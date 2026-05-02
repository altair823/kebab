//! Vector store records (§7.2 VectorStore).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ids::{ChunkId, DocumentId, EmbeddingId};
use crate::versions::{EmbeddingModelId, EmbeddingVersion};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VectorRecord {
    pub chunk_id: ChunkId,
    pub embedding_id: EmbeddingId,
    pub vector: Vec<f32>,
    pub doc_id: DocumentId,
    pub text: String,
    pub heading_path: Vec<String>,
    pub model_id: EmbeddingModelId,
    pub model_version: EmbeddingVersion,
    pub dimensions: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VectorHit {
    pub chunk_id: ChunkId,
    pub score: f32,
    pub payload: Value,
}
