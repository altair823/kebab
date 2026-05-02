//! Search query / filters / hit (§3.7) + DocFilter / DocSummary (§2.5).

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::asset::WorkspacePath;
use crate::citation::Citation;
use crate::ids::{ChunkId, DocumentId};
use crate::media::Lang;
use crate::metadata::{SourceType, TrustLevel};
use crate::versions::{ChunkerVersion, EmbeddingModelId, IndexVersion, ParserVersion};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchMode {
    Lexical,
    Vector,
    Hybrid,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SearchQuery {
    pub text: String,
    pub mode: SearchMode,
    pub k: usize,
    pub filters: SearchFilters,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SearchFilters {
    pub tags_any: Vec<String>,
    pub lang: Option<Lang>,
    pub path_glob: Option<String>,
    pub trust_min: Option<TrustLevel>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SearchHit {
    pub rank: u32,
    pub chunk_id: ChunkId,
    pub doc_id: DocumentId,
    pub doc_path: WorkspacePath,
    pub heading_path: Vec<String>,
    pub section_label: Option<String>,
    pub snippet: String,
    pub citation: Citation,
    pub retrieval: RetrievalDetail,
    pub index_version: IndexVersion,
    pub embedding_model: Option<EmbeddingModelId>,
    pub chunker_version: ChunkerVersion,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RetrievalDetail {
    pub method: SearchMode,
    pub fusion_score: f32,
    pub lexical_score: Option<f32>,
    pub vector_score: Option<f32>,
    pub lexical_rank: Option<u32>,
    pub vector_rank: Option<u32>,
}

/// Filter for `kb-app::list_docs` (§7.2 DocumentStore::list_documents).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DocFilter {
    pub tags_any: Vec<String>,
    pub lang: Option<Lang>,
    pub path_glob: Option<String>,
    pub trust_min: Option<TrustLevel>,
}

/// Internal mirror of wire `doc_summary.v1` (§2.5).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DocSummary {
    pub doc_id: DocumentId,
    pub doc_path: WorkspacePath,
    pub title: String,
    pub lang: Lang,
    pub tags: Vec<String>,
    pub trust_level: TrustLevel,
    pub source_type: SourceType,
    pub byte_len: u64,
    pub chunk_count: u32,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    pub parser_version: ParserVersion,
    pub chunker_version: ChunkerVersion,
}
