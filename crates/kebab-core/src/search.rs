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

/// p9-fb-36: canonical kind labels for `SearchFilters.media`. Mirrors
/// `MediaType` variant tags; CLI / MCP normalize aliases (`md` → `markdown`)
/// before populating this Vec.
pub const MEDIA_KINDS: &[&str] = &["markdown", "pdf", "image", "audio", "other"];

/// p9-fb-38: top-level `SearchHit.score` declaration.
/// `Rrf` (hybrid) / `Bm25` (lexical-only) / `Cosine` (vector-only).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScoreKind {
    #[default]
    Rrf,
    Bm25,
    Cosine,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SearchFilters {
    pub tags_any: Vec<String>,
    pub lang: Option<Lang>,
    pub path_glob: Option<String>,
    pub trust_min: Option<TrustLevel>,
    /// p9-fb-36: media_type filter — IN-list of `MediaType.kind`
    /// strings (`"markdown"`, `"pdf"`, `"image"`, `"audio"`, `"other"`).
    /// Empty Vec = no filter. Match is on the variant tag only;
    /// e.g. `["image"]` matches `Image(Png)` and `Image(Jpeg)`.
    #[serde(default)]
    pub media: Vec<String>,
    /// p9-fb-36: hits whose source doc's `documents.updated_at` is at
    /// or after this timestamp. None = no filter. RFC3339 / UTC.
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub ingested_after: Option<OffsetDateTime>,
    /// p9-fb-36: restrict hits to a single document. None = no filter.
    #[serde(default)]
    pub doc_id: Option<DocumentId>,
    /// p10-1A-1: filter by `metadata.repo`. Empty = no filter; multi-value = OR.
    #[serde(default)]
    pub repo: Vec<String>,
    /// p10-1A-1: filter by `metadata.code_lang`. Empty = no filter; multi-value = OR.
    /// Identifiers are lowercase canonical names (`rust`, `python`, `typescript`, ...).
    /// Unknown values produce empty hits (consistent with `media` policy).
    #[serde(default)]
    pub code_lang: Vec<String>,
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
    /// p9-fb-38: declares the meaning of the top-level `score`.
    /// `Rrf` (hybrid mode), `Bm25` (lexical-only), `Cosine` (vector-only).
    /// 옛 wire (fb-38 미만) 부재 시 `Rrf` default — hybrid 가 기본 mode.
    #[serde(default)]
    pub score_kind: ScoreKind,
    /// p10-1A-1: optional. Filled when the source file lives in a git repo
    /// (`.git/` walk-up). null for markdown / pdf / image hits and for code
    /// hits ingested via `kebab ingest-file` outside a repo boundary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// p10-1A-1: optional. Programming language identifier (lowercase). Set for
    /// every code/manifest/k8s chunk; null for markdown / pdf / image hits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_lang: Option<String>,
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

impl Default for RetrievalDetail {
    fn default() -> Self {
        Self {
            method: SearchMode::Hybrid,
            fusion_score: 0.0,
            lexical_score: None,
            vector_score: None,
            lexical_rank: None,
            vector_rank: None,
        }
    }
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
    /// p9-fb-37: when true, capture pipeline trace (cache bypassed,
    /// lex / vec pre-fusion lists + timing populated on the response).
    #[serde(default)]
    pub trace: bool,
}

/// p9-fb-37: search retrieval pipeline trace. Populated only when
/// `SearchOpts.trace = true`; `None` on the wrapping `SearchResponse`
/// otherwise. `lexical` / `vector` are pre-fusion candidate lists
/// (each retriever's full output for the fanout query). `rrf_inputs`
/// is the union (chunk_id) used by RRF, with each side's rank
/// captured. `timing` is wall-clock per stage.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SearchTrace {
    pub lexical: Vec<TraceCandidate>,
    pub vector: Vec<TraceCandidate>,
    pub rrf_inputs: Vec<TraceFusionInput>,
    pub timing: TraceTiming,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TraceCandidate {
    pub chunk_id: ChunkId,
    pub doc_id: DocumentId,
    pub doc_path: WorkspacePath,
    pub rank: u32,
    pub score: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TraceFusionInput {
    pub chunk_id: ChunkId,
    pub lexical_rank: Option<u32>,
    pub vector_rank: Option<u32>,
    /// Hybrid mode: normalized RRF score in `[0, 1]`.
    /// Lexical / Vector mode: equals the underlying retriever's score
    /// (no fusion ran). 0.0 for chunks dropped past `target_k`.
    pub fusion_score: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceTiming {
    pub lexical_ms: u64,
    pub vector_ms: u64,
    pub fusion_ms: u64,
    pub total_ms: u64,
}

/// p9-fb-37: on-disk index size breakdown. Mirrored on the
/// wire `schema.v1.stats.index_bytes` block.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexBytes {
    pub sqlite: u64,
    pub lancedb: u64,
}

/// p9-fb-42: per-query result in bulk search. `response` XOR `error` —
/// exactly one is `Some`. `query` is the input echo (raw JSON value)
/// so consumers can correlate input to output without index tracking.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BulkSearchItem {
    pub query: serde_json::Value,
    pub response: Option<serde_json::Value>,
    pub error: Option<serde_json::Value>,
}

/// p9-fb-42: bulk summary counts. Invariant: total == succeeded + failed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BulkSearchSummary {
    pub total: u32,
    pub succeeded: u32,
    pub failed: u32,
}

/// p9-fb-42: MCP-only envelope. CLI emits raw ndjson without envelope.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BulkSearchResponse {
    pub schema_version: String,
    pub results: Vec<BulkSearchItem>,
    pub summary: BulkSearchSummary,
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
            score_kind: ScoreKind::Rrf,
            repo: None,
            code_lang: None,
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

    #[test]
    fn search_filters_default_includes_new_fb36_fields() {
        let f = SearchFilters::default();
        assert!(f.media.is_empty(), "media default empty");
        assert!(f.ingested_after.is_none(), "ingested_after default None");
        assert!(f.doc_id.is_none(), "doc_id default None");
        assert!(f.tags_any.is_empty());
        assert!(f.lang.is_none());
        assert!(f.path_glob.is_none());
        assert!(f.trust_min.is_none());
    }

    #[test]
    fn search_filters_serialize_with_serde_default_compat() {
        let old: SearchFilters = serde_json::from_str(r#"{"tags_any":[],"lang":null,"path_glob":null,"trust_min":null}"#).unwrap();
        assert!(old.media.is_empty());
        assert!(old.ingested_after.is_none());
        assert!(old.doc_id.is_none());
    }

    #[test]
    fn search_trace_serde_roundtrip() {
        let t = SearchTrace {
            lexical: vec![TraceCandidate {
                chunk_id: ChunkId("c1".into()),
                doc_id: DocumentId("d1".into()),
                doc_path: WorkspacePath::new("a.md".into()).unwrap(),
                rank: 1,
                score: 0.42,
            }],
            vector: vec![],
            rrf_inputs: vec![TraceFusionInput {
                chunk_id: ChunkId("c1".into()),
                lexical_rank: Some(1),
                vector_rank: None,
                fusion_score: 0.0234,
            }],
            timing: TraceTiming {
                lexical_ms: 12,
                vector_ms: 0,
                fusion_ms: 1,
                total_ms: 14,
            },
        };
        let v = serde_json::to_value(&t).unwrap();
        assert_eq!(v["timing"]["lexical_ms"], 12);
        assert_eq!(
            v["lexical"][0]["score"].as_f64().unwrap() as f32,
            0.42_f32
        );
        let back: SearchTrace = serde_json::from_value(v).unwrap();
        assert_eq!(back, t);
    }

    #[test]
    fn index_bytes_default_is_zero() {
        let b = IndexBytes::default();
        assert_eq!(b.sqlite, 0);
        assert_eq!(b.lancedb, 0);
    }

    #[test]
    fn search_opts_trace_default_false() {
        let opts = SearchOpts::default();
        assert!(!opts.trace);
    }

    #[test]
    fn score_kind_serde_roundtrip() {
        use ScoreKind::*;
        for (kind, expected) in [(Rrf, "rrf"), (Bm25, "bm25"), (Cosine, "cosine")] {
            let v = serde_json::to_value(kind).unwrap();
            assert_eq!(v.as_str(), Some(expected));
            let back: ScoreKind = serde_json::from_value(v).unwrap();
            assert_eq!(back, kind);
        }
    }

    #[test]
    fn score_kind_default_is_rrf() {
        assert_eq!(ScoreKind::default(), ScoreKind::Rrf);
    }

    #[test]
    fn search_hit_deserialize_without_score_kind_defaults_to_rrf() {
        let json = serde_json::json!({
            "rank": 1,
            "chunk_id": "c1",
            "doc_id": "d1",
            "doc_path": "a.md",
            "heading_path": [],
            "section_label": null,
            "snippet": "x",
            "citation": { "kind": "line", "path": "a.md", "start": 1, "end": 1, "section": null },
            "retrieval": {
                "method": "lexical",
                "fusion_score": 0.5,
                "lexical_score": 0.5,
                "vector_score": null,
                "lexical_rank": 1,
                "vector_rank": null
            },
            "index_version": "v1",
            "embedding_model": null,
            "chunker_version": "c1",
            "indexed_at": "2026-05-10T12:00:00Z",
            "stale": false
        });
        let hit: SearchHit = serde_json::from_value(json).unwrap();
        assert_eq!(hit.score_kind, ScoreKind::Rrf);
    }

    #[test]
    fn bulk_search_summary_serde_roundtrip() {
        let s = BulkSearchSummary {
            total: 5,
            succeeded: 4,
            failed: 1,
        };
        let v = serde_json::to_value(s).unwrap();
        assert_eq!(v["total"], 5);
        assert_eq!(v["succeeded"], 4);
        assert_eq!(v["failed"], 1);
        let back: BulkSearchSummary = serde_json::from_value(v).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn bulk_search_summary_default_is_zeros() {
        let s = BulkSearchSummary::default();
        assert_eq!(s.total, 0);
        assert_eq!(s.succeeded, 0);
        assert_eq!(s.failed, 0);
    }

    #[test]
    fn bulk_search_item_serde_response_variant() {
        let item = BulkSearchItem {
            query: serde_json::json!({"query": "rust"}),
            response: Some(serde_json::json!({"hits": []})),
            error: None,
        };
        let v = serde_json::to_value(&item).unwrap();
        assert!(v["response"].is_object());
        assert!(v["error"].is_null());
    }

    #[test]
    fn bulk_search_item_serde_error_variant() {
        let item = BulkSearchItem {
            query: serde_json::json!({"query": "rust"}),
            response: None,
            error: Some(serde_json::json!({"code": "config_invalid", "message": "bad"})),
        };
        let v = serde_json::to_value(&item).unwrap();
        assert!(v["response"].is_null());
        assert_eq!(v["error"]["code"], "config_invalid");
    }

    #[test]
    fn search_hit_repo_and_code_lang_are_optional_and_omit_when_none() {
        let hit = SearchHit {
            rank: 1,
            chunk_id: ChunkId("c1".into()),
            doc_id: DocumentId("d1".into()),
            doc_path: WorkspacePath("a.md".into()),
            heading_path: vec![],
            section_label: None,
            snippet: String::new(),
            citation: Citation::Line {
                path: WorkspacePath("a.md".into()),
                start: 1,
                end: 2,
                section: None,
            },
            retrieval: RetrievalDetail::default(),
            index_version: IndexVersion("v1".into()),
            embedding_model: None,
            chunker_version: ChunkerVersion("md-heading-v1".into()),
            indexed_at: time::OffsetDateTime::UNIX_EPOCH,
            stale: false,
            score_kind: ScoreKind::Rrf,
            repo: None,
            code_lang: None,
        };
        let v = serde_json::to_value(&hit).unwrap();
        assert!(v.get("repo").is_none(), "repo should be omitted when None");
        assert!(v.get("code_lang").is_none(), "code_lang should be omitted when None");
    }

    #[test]
    fn search_hit_repo_and_code_lang_present_when_some() {
        let hit = SearchHit {
            rank: 1,
            chunk_id: ChunkId("c1".into()),
            doc_id: DocumentId("d1".into()),
            doc_path: WorkspacePath("a.rs".into()),
            heading_path: vec![],
            section_label: None,
            snippet: String::new(),
            citation: Citation::Code {
                path: WorkspacePath("a.rs".into()),
                line_start: 1,
                line_end: 2,
                symbol: None,
                lang: Some("rust".into()),
            },
            retrieval: RetrievalDetail::default(),
            index_version: IndexVersion("v1".into()),
            embedding_model: None,
            chunker_version: ChunkerVersion("code-rust-ast-v1".into()),
            indexed_at: time::OffsetDateTime::UNIX_EPOCH,
            stale: false,
            score_kind: ScoreKind::Rrf,
            repo: Some("kebab".into()),
            code_lang: Some("rust".into()),
        };
        let v = serde_json::to_value(&hit).unwrap();
        assert_eq!(v["repo"], "kebab");
        assert_eq!(v["code_lang"], "rust");
    }

    #[test]
    fn search_filters_repo_and_code_lang_default_to_empty_vec() {
        let f = SearchFilters::default();
        assert!(f.repo.is_empty());
        assert!(f.code_lang.is_empty());
    }
}
