//! Component traits (§7) and their input helper types (§7.1).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::answer::{ModelRef, TokenUsage};
use crate::asset::{RawAsset, WorkspacePath};
use crate::chunk::Chunk;
use crate::document::{Block, CanonicalDocument};
use crate::ids::{AssetId, ChunkId, DocumentId};
use crate::jobs::{JobFilter, JobId, JobKind, JobRow, JobStatus};
use crate::media::MediaType;
use crate::search::{DocFilter, DocSummary, SearchFilters, SearchHit, SearchQuery};
use crate::vector::{VectorHit, VectorRecord};
use crate::versions::{
    ChunkerVersion, EmbeddingModelId, EmbeddingVersion, IndexVersion, ParserVersion,
};

// ── Helper input types (§7.1) ─────────────────────────────────────────────

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SourceScope {
    pub root: PathBuf,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
}

/// Forward-declared (§3.7a) — concrete shape decided by extractors. P0
/// keeps the option-of-config-file slot only.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ExtractConfig {
    pub config_path: Option<PathBuf>,
}

/// Carries the raw asset bytes context to an `Extractor::extract` call.
pub struct ExtractContext<'a> {
    pub asset: &'a RawAsset,
    pub workspace_root: &'a Path,
    pub config: &'a ExtractConfig,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChunkPolicy {
    pub target_tokens: usize,
    pub overlap_tokens: usize,
    pub respect_markdown_headings: bool,
    pub chunker_version: ChunkerVersion,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EmbeddingKind {
    Document,
    Query,
}

pub struct EmbeddingInput<'a> {
    pub text: &'a str,
    pub kind: EmbeddingKind,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GenerateRequest {
    pub system: String,
    pub user: String,
    pub stop: Vec<String>,
    pub max_tokens: usize,
    pub temperature: f32,
    pub seed: Option<u64>,
    /// Vision inputs (base64-encoded, one per image). Empty for the
    /// text-only path that P4-2 / P4-3 / RAG uses; non-empty when a
    /// vision-capable adapter (P6-3 caption, future multimodal RAG)
    /// drives the call. The LM adapter is responsible for routing
    /// these onto the wire — Ollama uses `images: [base64, ...]`,
    /// other backends may differ.
    ///
    /// Defaulted on deserialization so older `*.json` payloads /
    /// snapshots that predate the field still parse.
    #[serde(default)]
    pub images: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum TokenChunk {
    Token(String),
    Done {
        finish_reason: FinishReason,
        usage: TokenUsage,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    Stop,
    Length,
    Aborted,
    /// p9-fb-33: caller-side cancel. The pipeline breaks the LM loop
    /// when a `Token` send into `AskOpts.stream_sink` returns
    /// `SendError` (receiver dropped). The persisted answer is
    /// flagged with `RefusalReason::LlmStreamAborted`.
    Cancelled,
    Error(String),
}

// ── Traits (§7.2) ─────────────────────────────────────────────────────────

pub trait SourceConnector {
    fn scan(&self, scope: &SourceScope) -> anyhow::Result<Vec<RawAsset>>;
}

pub trait Extractor: Send + Sync {
    fn supports(&self, media_type: &MediaType) -> bool;
    fn parser_version(&self) -> ParserVersion;
    fn extract(&self, ctx: &ExtractContext<'_>, bytes: &[u8]) -> anyhow::Result<CanonicalDocument>;
}

pub trait Chunker: Send + Sync {
    fn chunker_version(&self) -> ChunkerVersion;
    fn policy_hash(&self, policy: &ChunkPolicy) -> String;
    fn chunk(&self, doc: &CanonicalDocument, policy: &ChunkPolicy) -> anyhow::Result<Vec<Chunk>>;
}

pub trait Embedder: Send + Sync {
    fn model_id(&self) -> EmbeddingModelId;
    fn model_version(&self) -> EmbeddingVersion;
    fn dimensions(&self) -> usize;
    fn embed(&self, inputs: &[EmbeddingInput<'_>]) -> anyhow::Result<Vec<Vec<f32>>>;
}

pub trait Retriever: Send + Sync {
    fn search(&self, query: &SearchQuery) -> anyhow::Result<Vec<SearchHit>>;
    fn index_version(&self) -> IndexVersion;
}

pub trait LanguageModel: Send + Sync {
    fn model_ref(&self) -> ModelRef;
    fn context_tokens(&self) -> usize;
    fn generate_stream(
        &self,
        req: GenerateRequest,
    ) -> anyhow::Result<Box<dyn Iterator<Item = anyhow::Result<TokenChunk>> + Send>>;
}

pub trait DocumentStore {
    fn put_asset(&self, a: &RawAsset) -> anyhow::Result<()>;
    fn put_document(&self, d: &CanonicalDocument) -> anyhow::Result<()>;
    fn put_blocks(&self, doc: &DocumentId, blocks: &[Block]) -> anyhow::Result<()>;
    fn put_chunks(&self, doc: &DocumentId, chunks: &[Chunk]) -> anyhow::Result<()>;
    fn get_document(&self, id: &DocumentId) -> anyhow::Result<Option<CanonicalDocument>>;
    fn get_chunk(&self, id: &ChunkId) -> anyhow::Result<Option<Chunk>>;
    fn list_documents(&self, filter: &DocFilter) -> anyhow::Result<Vec<DocSummary>>;
    /// Look up an asset row by its `asset_id` (PRIMARY KEY = blake3
    /// content hash). Twin-file safe: asset_id is PK so there is
    /// exactly one row per unique content hash, regardless of how many
    /// `documents` rows share it. Use this instead of
    /// `get_asset_by_workspace_path` when you already have a
    /// `CanonicalDocument` (which carries `source_asset_id`).
    fn get_asset(&self, id: &AssetId) -> anyhow::Result<Option<RawAsset>>;

    /// p9-fb-23: look up an asset row by its workspace path. Used by
    /// the incremental-ingest skip path to compare the freshly
    /// computed blake3 checksum against what's already in SQLite. The
    /// schema enforces a unique workspace_path per asset.
    ///
    /// NOTE: for twin files (identical content at different paths),
    /// `assets.workspace_path` is "last-registered path" — it
    /// flip-flops on every ingest. Prefer `get_asset` (by asset_id)
    /// when you have a `CanonicalDocument.source_asset_id`.
    fn get_asset_by_workspace_path(&self, path: &WorkspacePath)
    -> anyhow::Result<Option<RawAsset>>;

    /// Look up a document row by its workspace path. Used by the
    /// document-centric skip path in `try_skip_unchanged` to avoid the
    /// twin-file flip-flop that the asset-side lookup suffers from
    /// (multiple files with identical content share one `assets` row
    /// whose `workspace_path` is overwritten on every UPSERT, so
    /// `get_asset_by_workspace_path` returns the wrong twin's path).
    ///
    /// `documents.workspace_path` is UNIQUE (V001), so each twin has
    /// its own stable document row regardless of the asset de-dup.
    fn get_document_by_workspace_path(
        &self,
        path: &WorkspacePath,
    ) -> anyhow::Result<Option<CanonicalDocument>>;

    /// Return every `workspace_path` stored in the `documents` table.
    ///
    /// Used by the post-walker sweep in `kebab-app::ingest` to detect
    /// documents whose source file has been deleted from the filesystem.
    /// The set difference `(stored - scanned)` yields orphan candidates;
    /// each candidate is then existence-checked on disk so that
    /// out-of-scope files (config narrowing) are NOT purged — only truly
    /// absent files trigger the purge.
    fn all_workspace_paths(&self) -> anyhow::Result<Vec<WorkspacePath>>;
}

pub trait VectorStore {
    fn ensure_table(
        &self,
        model: &EmbeddingModelId,
        dim: usize,
    ) -> anyhow::Result<crate::ids::IndexId>;
    fn upsert(&self, recs: &[VectorRecord]) -> anyhow::Result<()>;
    fn search(
        &self,
        query_vec: &[f32],
        k: usize,
        filters: &SearchFilters,
    ) -> anyhow::Result<Vec<VectorHit>>;

    /// Delete every vector whose `chunk_id` appears in `chunk_ids`.
    ///
    /// Used by `kebab-app` after `purge_orphan_at_workspace_path` sweeps
    /// the SQLite side on a byte-edit re-ingest, so the LanceDB rows
    /// keyed on the now-deleted `chunk_id`s do not stay on disk
    /// forever. Empty input is a no-op. The default impl is a no-op so
    /// older `VectorStore` impls (e.g. test fakes) keep compiling
    /// without behavioural change.
    fn delete_by_chunk_ids(&self, _chunk_ids: &[crate::ids::ChunkId]) -> anyhow::Result<()> {
        Ok(())
    }
}

pub trait JobRepo {
    fn create(&self, kind: JobKind, payload: Value) -> anyhow::Result<JobId>;
    fn update_progress(&self, id: &JobId, progress: Value) -> anyhow::Result<()>;
    fn finish(&self, id: &JobId, status: JobStatus, error: Option<&str>) -> anyhow::Result<()>;
    fn list(&self, filter: &JobFilter) -> anyhow::Result<Vec<JobRow>>;
}

