//! `kb-core` — frozen domain types, traits, and ID recipe.
//!
//! Per design §3, §4, §7. This crate has zero dependencies on any other
//! `kb-*` crate, so every other crate in the workspace can depend on it
//! freely.
//!
//! See `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md` for
//! the canonical type bodies — this crate is the byte-for-byte mirror.

pub mod ids;
pub mod versions;
pub mod media;
pub mod asset;
pub mod document;
pub mod chunk;
pub mod citation;
pub mod metadata;
pub mod search;
pub mod answer;
pub mod ingest;
pub mod jobs;
pub mod vector;
pub mod errors;
pub mod traits;
pub mod normalize;

// Re-export the most commonly used items at the crate root, mirroring the
// public surface listed in the task spec.

pub use ids::{
    AssetId, BlockId, ChunkId, DocumentId, EmbeddingId, IndexId,
    id_for_asset, id_for_block, id_for_chunk, id_for_doc, id_for_embedding,
    id_for_index, id_from,
};
pub use versions::{
    ChunkerVersion, EmbeddingModelId, EmbeddingVersion, IndexVersion,
    ParserVersion, PromptTemplateVersion, SchemaVersion,
};
pub use media::{AudioType, Checksum, ImageType, Lang, MediaType};
pub use asset::{AssetStorage, RawAsset, SourceUri, WorkspacePath};
pub use document::{
    AudioRefBlock, Block, CanonicalDocument, CodeBlock, CommonBlock,
    HeadingBlock, ImageRefBlock, Inline, ListBlock, ModelCaption, OcrRegion,
    OcrText, SourceSpan, TableBlock, TextBlock, Transcript, TranscriptSegment,
};
pub use chunk::Chunk;
pub use citation::Citation;
pub use metadata::{
    Metadata, Provenance, ProvenanceEvent, ProvenanceKind, SourceType,
    TrustLevel,
};
pub use search::{
    DocFilter, DocSummary, RetrievalDetail, SearchFilters, SearchHit,
    SearchMode, SearchOpts, SearchQuery,
};
pub use answer::{
    Answer, AnswerCitation, AnswerRetrievalSummary, ModelRef, RefusalReason, TokenUsage,
    TraceId, Turn,
};
pub use ingest::{IngestItem, IngestItemKind, IngestReport};
pub use jobs::{JobFilter, JobId, JobKind, JobRow, JobStatus};
pub use vector::{VectorHit, VectorRecord};
pub use errors::CoreError;
pub use traits::{
    ChatSessionRepo, ChatSessionRow, ChatTurnRow, ChunkPolicy, Chunker, DocumentStore,
    Embedder, EmbeddingInput, EmbeddingKind, ExtractConfig, ExtractContext, Extractor,
    FinishReason, GenerateRequest, JobRepo, LanguageModel, Retriever, SourceConnector,
    SourceScope, TokenChunk, VectorStore,
};
pub use normalize::{nfc, to_posix};
