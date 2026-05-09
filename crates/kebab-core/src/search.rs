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
    /// p9-fb-32: source doc's `documents.updated_at` (last actual re-process).
    /// fb-23 incremental ingest skip path leaves this unchanged.
    #[serde(with = "time::serde::rfc3339")]
    pub indexed_at: OffsetDateTime,
    /// p9-fb-32: server-computed `now - indexed_at > threshold` per
    /// `config.search.stale_threshold_days`. `false` when threshold = 0.
    pub stale: bool,
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

/// p9-fb-34: caller-supplied output budget knobs for `App::search_with_opts`.
/// All `None` = no enforcement (existing behavior).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SearchOpts {
    /// chars/4 approximation of wire JSON token cost. None = no cap.
    pub max_tokens: Option<usize>,
    /// Per-hit snippet character cap. None = use config default.
    pub snippet_chars: Option<usize>,
    /// Opaque base64 cursor from a previous response. None = first page.
    pub cursor: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    #[test]
    fn search_hit_serializes_indexed_at_and_stale() {
        let hit = SearchHit {
            rank: 1,
            chunk_id: ChunkId("c".to_string()),
            doc_id: DocumentId("d".to_string()),
            doc_path: WorkspacePath::new("a/b.md".to_string()).unwrap(),
            heading_path: vec!["H".to_string()],
            section_label: None,
            snippet: "s".to_string(),
            citation: Citation::Line {
                path: WorkspacePath::new("a/b.md".to_string()).unwrap(),
                start: 1,
                end: 1,
                section: None,
            },
            retrieval: RetrievalDetail {
                method: SearchMode::Lexical,
                fusion_score: 0.5,
                lexical_score: Some(0.5),
                vector_score: None,
                lexical_rank: Some(1),
                vector_rank: None,
            },
            index_version: IndexVersion("v1".to_string()),
            embedding_model: None,
            chunker_version: ChunkerVersion("c1".to_string()),
            indexed_at: datetime!(2026-05-09 12:00:00 UTC),
            stale: true,
        };
        let v = serde_json::to_value(&hit).unwrap();
        assert_eq!(v["indexed_at"], "2026-05-09T12:00:00Z");
        assert_eq!(v["stale"], true);
    }

    #[test]
    fn search_opts_default_is_all_none() {
        let opts = SearchOpts::default();
        assert!(opts.max_tokens.is_none());
        assert!(opts.snippet_chars.is_none());
        assert!(opts.cursor.is_none());
    }
}
