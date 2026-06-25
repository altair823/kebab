//! `kb-core` — frozen domain types, traits, and ID recipe.
//!
//! Per design §3, §4, §7. This crate has zero dependencies on any other
//! `kb-*` crate, so every other crate in the workspace can depend on it
//! freely.
//!
//! See `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md` for
//! the canonical type bodies — this crate is the byte-for-byte mirror.

pub mod answer;
pub mod asset;
pub mod chunk;
pub mod citation;
pub mod derivation;
pub mod document;
pub mod errors;
pub mod fetch;
pub mod ids;
pub mod ingest;
pub mod jobs;
pub mod media;
pub mod metadata;
pub mod normalize;
pub mod search;
pub mod traits;
pub mod vector;
pub mod versions;

// Re-export the most commonly used items at the crate root, mirroring the
// public surface listed in the task spec.

pub use answer::{
    Answer, AnswerCitation, AnswerRetrievalSummary, HopKind, HopRecord, ModelRef, RefusalReason,
    TokenUsage, TraceId, VerificationSummary,
};
pub use asset::{AssetStorage, RawAsset, SourceUri, WorkspacePath};
pub use chunk::Chunk;
pub use citation::Citation;
pub use derivation::{derivation_cache_key, derivation_cache_key_bytes};
pub use document::{
    AudioRefBlock, Block, CanonicalDocument, CodeBlock, CommonBlock, HeadingBlock, ImageRefBlock,
    Inline, ListBlock, ModelCaption, OcrRegion, OcrText, SourceSpan, TableBlock, TextBlock,
    Transcript, TranscriptSegment,
};
pub use errors::CoreError;
pub use fetch::{FetchKind, FetchOpts, FetchQuery, FetchResult};
pub use ids::{
    ALIAS_SUFFIX, AssetId, BlockId, ChunkId, DocumentId, EmbeddingId, IndexId, id_for_asset,
    id_for_block, id_for_chunk, id_for_doc, id_for_embedding, id_for_index, id_from,
    strip_alias_suffix,
};
pub use ingest::{IngestItem, IngestItemKind, IngestReport, SkipExamples};
pub use jobs::{JobFilter, JobId, JobKind, JobRow, JobStatus};
pub use media::{AudioType, Checksum, ImageType, Lang, MediaType};
pub use metadata::{Metadata, Provenance, ProvenanceEvent, ProvenanceKind, SourceType, TrustLevel};
pub use normalize::{nfc, to_posix};
pub use search::{
    BulkSearchItem, BulkSearchResponse, BulkSearchSummary, DocFilter, DocSummary, IndexBytes,
    MEDIA_KINDS, RetrievalDetail, ScoreKind, SearchFilters, SearchHit, SearchMode, SearchOpts,
    SearchQuery, SearchTrace, TraceCandidate, TraceFusionInput, TraceTiming,
};
pub use traits::{
    ChunkPolicy, Chunker, DocumentStore, Embedder,
    EmbeddingInput, EmbeddingKind, ExtractConfig, ExtractContext, Extractor, FinishReason,
    GenerateRequest, JobRepo, LanguageModel, Retriever, SourceConnector, SourceScope, TokenChunk,
    VectorStore,
};
pub use vector::{VectorHit, VectorRecord};
pub use versions::{
    ChunkerVersion, EmbeddingModelId, EmbeddingVersion, IndexVersion, ParserVersion,
    PromptTemplateVersion, SchemaVersion,
};
