//! Component traits (§7) and their input helper types (§7.1).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::asset::{RawAsset, WorkspacePath};
use crate::chunk::Chunk;
use crate::document::{Block, CanonicalDocument};
use crate::ids::{ChunkId, DocumentId};
use crate::jobs::{JobFilter, JobId, JobKind, JobRow, JobStatus};
use crate::media::MediaType;
use crate::search::{DocFilter, DocSummary, SearchFilters, SearchHit, SearchQuery};
use crate::vector::{VectorHit, VectorRecord};
use crate::versions::{
    ChunkerVersion, EmbeddingModelId, EmbeddingVersion, IndexVersion, ParserVersion,
};
use crate::answer::{ModelRef, TokenUsage};

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
    Error(String),
}

// ── Traits (§7.2) ─────────────────────────────────────────────────────────

pub trait SourceConnector {
    fn scan(&self, scope: &SourceScope) -> anyhow::Result<Vec<RawAsset>>;
}

pub trait Extractor: Send + Sync {
    fn supports(&self, media_type: &MediaType) -> bool;
    fn parser_version(&self) -> ParserVersion;
    fn extract(
        &self,
        ctx: &ExtractContext<'_>,
        bytes: &[u8],
    ) -> anyhow::Result<CanonicalDocument>;
}

pub trait Chunker: Send + Sync {
    fn chunker_version(&self) -> ChunkerVersion;
    fn policy_hash(&self, policy: &ChunkPolicy) -> String;
    fn chunk(
        &self,
        doc: &CanonicalDocument,
        policy: &ChunkPolicy,
    ) -> anyhow::Result<Vec<Chunk>>;
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
    /// p9-fb-23: look up an asset row by its workspace path. Used by
    /// the incremental-ingest skip path to compare the freshly
    /// computed blake3 checksum against what's already in SQLite. The
    /// schema enforces a unique workspace_path per asset.
    fn get_asset_by_workspace_path(
        &self,
        path: &WorkspacePath,
    ) -> anyhow::Result<Option<RawAsset>>;
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
    fn finish(
        &self,
        id: &JobId,
        status: JobStatus,
        error: Option<&str>,
    ) -> anyhow::Result<()>;
    fn list(&self, filter: &JobFilter) -> anyhow::Result<Vec<JobRow>>;
}

// ── p9-fb-17: chat session persistence ────────────────────────────────

/// Persistent multi-turn chat session — header row in `chat_sessions`.
/// Per-turn rows live in `chat_turns` (see [`ChatTurnRow`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChatSessionRow {
    pub session_id: String,
    /// Unix epoch seconds at session creation time.
    pub created_at: i64,
    /// Unix epoch seconds, bumped on every `append_turn`.
    pub updated_at: i64,
    /// Optional human-readable label — defaults to the first
    /// question's first ~40 chars on creation.
    pub title: Option<String>,
    /// Snapshot of `prompt_template_version`, `llm.model`,
    /// `max_context_tokens`, etc. — same shape as
    /// `eval_runs.config_snapshot_json`. JSON string so the schema
    /// can grow without an SQLite ALTER.
    pub config_snapshot_json: String,
}

/// One Q/A pair inside a `ChatSessionRow`. `turn_index` is monotonic
/// per session (0-based).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChatTurnRow {
    /// `blake3(session_id || turn_index)` (32 hex). Stable per (session,
    /// turn) so a re-append at the same index is rejected via PK.
    pub turn_id: String,
    pub session_id: String,
    pub turn_index: u32,
    pub question: String,
    pub answer: String,
    /// `Vec<Citation>` JSON-encoded so a session resume can replay
    /// the same citation markers the user saw originally.
    pub citations_json: String,
    pub created_at: i64,
}

/// Persistence trait for multi-turn chat sessions. Implemented by
/// `kebab-store-sqlite::SqliteStore`; consumed by `kebab-app` and the
/// future CLI / TUI session UIs (p9-fb-18).
pub trait ChatSessionRepo {
    /// Create a new session. `session_id` is caller-supplied — auto
    /// derivation lives in `kebab-app`. Errors on PK collision.
    fn create_session(&self, row: &ChatSessionRow) -> anyhow::Result<()>;

    /// Look up a session by id; `Ok(None)` when missing.
    fn get_session(&self, session_id: &str) -> anyhow::Result<Option<ChatSessionRow>>;

    /// Most-recent-updated-first list of sessions, capped at `limit`.
    fn list_sessions(&self, limit: usize) -> anyhow::Result<Vec<ChatSessionRow>>;

    /// Delete a session and (CASCADE) every turn under it.
    fn delete_session(&self, session_id: &str) -> anyhow::Result<()>;

    /// Append a turn at `turn.turn_index`. Bumps the parent's
    /// `updated_at`. PK collision (same session_id + turn_index) is
    /// an error — the caller assigns the next monotonic index.
    fn append_turn(&self, turn: &ChatTurnRow) -> anyhow::Result<()>;

    /// All turns for `session_id`, ordered by `turn_index ASC`.
    /// Empty vec when the session has no turns yet.
    fn list_turns(&self, session_id: &str) -> anyhow::Result<Vec<ChatTurnRow>>;
}
