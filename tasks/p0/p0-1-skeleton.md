---
phase: P0
component: workspace + kb-core + kb-config + kb-app + kb-cli
task_id: p0-1
title: "Workspace skeleton + frozen domain types/traits + ID recipe + facade"
status: planned
depends_on: []
unblocks: [p1-1, p1-2, p1-3, p1-4, p1-5, p1-6, p2-1, p2-2, p3-1, p3-2, p3-3, p3-4, p4-1, p4-2, p4-3, p5-1, p5-2, p6-1, p6-2, p6-3, p7-1, p7-2, p8-1, p8-2, p9-1, p9-2, p9-3, p9-4, p9-5]
contract_source: ../../docs/superpowers/specs/2026-04-27-kb-final-form-design.md
contract_sections: [§3 (all), §4, §5.1 schema_meta+migrations, §6 (config + XDG), §7 (all traits), §8 module boundaries, §9 versioning, §10 errors+exit codes, §2.8 wire schema_version]
---

# p0-1 — Workspace skeleton + frozen contracts

## Goal

Stand up the Cargo workspace (Rust 2024, resolver=3) with `kb-core`, `kb-parse-types`, `kb-config`, `kb-app`, `kb-cli` crates. Freeze every domain type, trait, ID recipe, error type, and CLI entry shape per the frozen design doc so that all subsequent component tasks compile against stable contracts.

## Why now / why this size

Every other task imports `kb-core`. If types or trait signatures wobble after this point, every downstream task spec drifts. This task is large but indivisible: types + traits + ID recipe + facade + CLI skeleton + wire schema stubs must land together so the rest of the workspace can compile against them.

## Allowed dependencies

- workspace `[workspace.dependencies]`: `anyhow = "1"`, `thiserror = "2"`, `serde = { version = "1", features = ["derive"] }`, `serde_json = "1"`, `time = { version = "0.3", features = ["serde", "macros"] }`, `uuid = { version = "1", features = ["v7", "serde"] }`, `blake3 = "1"`, `tracing = "0.1"`
- per crate:
  - `kb-core`: workspace deps + `serde_json::Map`, `serde-json-canonicalizer`, `unicode-normalization`
  - `kb-parse-types`: workspace deps + `kb-core` ONLY (no parsers, no stores, no normalize). Defines parser intermediate representations per design §3.7b.
  - `kb-config`: workspace deps + `toml = "0.8"`, `dirs = "5"` (XDG paths)
  - `kb-app`: workspace deps + `kb-core`, `kb-config`, `tracing-subscriber`, `tracing-appender`
  - `kb-cli`: workspace deps + `kb-core`, `kb-config`, `kb-app`, `clap = { version = "4", features = ["derive"] }`

## Forbidden dependencies

- `kb-core` MUST NOT depend on any other `kb-*` crate.
- `kb-parse-types` MUST depend ONLY on `kb-core`. No parser libraries (`pulldown-cmark`, `pdf-extract`, `image`, `whisper-rs`, …), no other `kb-*` crate.
- `kb-config` MUST NOT depend on `kb-app`, `kb-cli`, parsers, stores, embedders, search, llm, rag, tui, desktop.
- `kb-app` MUST NOT yet depend on parsers/stores/embedders/search/llm/rag (those crates do not exist yet — facade methods stub out and return `unimplemented!()` or `anyhow::bail!("not yet wired (Pn-i)")`).
- `kb-cli` MUST NOT call any non-`kb-app` crate directly.

## Inputs

| input | type | source |
|-------|------|--------|
| frozen design doc | Markdown | `docs/superpowers/specs/2026-04-27-kb-final-form-design.md` |
| user `kb` invocation | command-line args | end user |

## Outputs

| output | type | downstream consumer |
|--------|------|---------------------|
| compiling workspace | Rust crates | every later task |
| `kb-core` types/traits | Rust API | every other crate |
| `kb-core` ID functions | Rust API | parsers, normalize, chunkers, embedders, search, rag |
| `kb-config::Config` | Rust struct | every other crate |
| `kb-app` facade methods (stubs) | Rust API | `kb-cli`, future TUI/desktop |
| `kb` binary | executable | end user |
| `docs/wire-schema/v1/*.schema.json` stubs | JSON Schema files | future wire emitters and consumers |
| `docs/spec/*.md` stubs (link to frozen design) | Markdown | future contributors |

## Public surface (signatures only — no new types)

All types/traits below are defined in `kb-core` exactly per design §3 and §7 (no additions, no renames). Subagent must copy field-for-field.

```rust
// ── kb-core ─────────────────────────────────────────────────────────────────

// Newtype IDs (design §3.1) — Display + FromStr implemented.
pub struct AssetId(pub String);
pub struct DocumentId(pub String);
pub struct BlockId(pub String);
pub struct ChunkId(pub String);
pub struct EmbeddingId(pub String);
pub struct IndexId(pub String);

// Versions / labels (§3.2)
pub struct ParserVersion(pub String);
pub struct ChunkerVersion(pub String);
pub struct EmbeddingModelId(pub String);
pub struct EmbeddingVersion(pub String);
pub struct IndexVersion(pub String);
pub struct PromptTemplateVersion(pub String);
pub struct SchemaVersion(pub &'static str);

// Forward-declared (§3.7a)
pub struct OcrText { /* per §3.7a */ }
pub struct OcrRegion { /* per §3.7a */ }
pub struct ModelCaption { /* per §3.7a */ }
pub struct Transcript { /* per §3.7a */ }
pub struct TranscriptSegment { /* per §3.7a */ }
pub struct Checksum(pub String);
pub struct Lang(pub String);
pub enum   ImageType { Png, Jpeg, Webp, Gif, Tiff, Other(String) }
pub enum   AudioType { M4a, Mp3, Wav, Flac, Ogg, Other(String) }

// RawAsset (§3.3)
pub struct RawAsset { /* per §3.3 */ }
pub enum   SourceUri { File(std::path::PathBuf), Kb(String) }
pub struct WorkspacePath(pub String);
pub enum   MediaType { Markdown, Pdf, Image(ImageType), Audio(AudioType), Other(String) }
pub enum   AssetStorage { Copied { path: std::path::PathBuf }, Reference { path: std::path::PathBuf, sha: Checksum } }

// CanonicalDocument + Block + SourceSpan + Inline (§3.4)
pub struct CanonicalDocument { /* per §3.4 */ }
pub enum   Block { /* per §3.4 */ }
pub struct CommonBlock { /* per §3.4 */ }
pub struct HeadingBlock { /* per §3.4 */ }
pub struct TextBlock { /* per §3.4 */ }
pub struct ListBlock { /* per §3.4 */ }
pub struct CodeBlock { /* per §3.4 */ }
pub struct TableBlock { /* per §3.4 */ }
pub struct ImageRefBlock { /* per §3.4 */ }
pub struct AudioRefBlock { /* per §3.4 */ }
pub enum   Inline { /* per §3.4 */ }
pub enum   SourceSpan { /* per §3.4 */ }

// (ParsedBlock + parser intermediates live in kb-parse-types per design §3.7b — NOT in kb-core.)

// Chunk + Citation (§3.5)
pub struct Chunk { /* per §3.5 */ }
pub enum   Citation { /* 5 variants per §3.5 */ }
impl Citation {
    pub fn path(&self) -> &WorkspacePath;
    pub fn to_uri(&self) -> String;          // W3C Media Fragments per §0 Q3
    pub fn parse(s: &str) -> anyhow::Result<Self>;
}

// Metadata + Provenance (§3.6)
pub struct Metadata { /* per §3.6 */ }
pub enum   SourceType { Markdown, Note, Paper, Reference, Inbox }
pub enum   TrustLevel { Primary, Secondary, Generated }
pub struct Provenance { /* per §3.6 */ }
pub struct ProvenanceEvent { /* per §3.6 */ }
pub enum   ProvenanceKind { Discovered, Parsed, Normalized, Chunked, OcrApplied, CaptionApplied, Transcribed, Embedded, Indexed, Warning, Error }

// Search types (§3.7)
pub enum   SearchMode { Lexical, Vector, Hybrid }
pub struct SearchQuery { /* per §3.7 */ }
pub struct SearchFilters { /* per §3.7 */ }
pub struct SearchHit { /* per §3.7 */ }
pub struct RetrievalDetail { /* per §3.7 */ }
pub struct DocFilter { /* tags_any/lang/path_glob/trust_min */ }
pub struct DocSummary { /* per §2.5 wire — mirrored internally */ }

// Answer / RAG (§3.8)
pub struct Answer { /* per §3.8 */ }
pub struct AnswerCitation { /* per §3.8 */ }
pub enum   RefusalReason { ScoreGate, LlmSelfJudge, NoIndex, NoChunks }
pub struct ModelRef { /* per §3.8 */ }
pub struct AnswerRetrievalSummary { /* per §3.8 */ }
pub struct TokenUsage { /* per §3.8 */ }
pub struct TraceId(pub String);

// IngestReport (mirrored from wire §2.4 for facade return)
pub struct IngestReport { /* per §2.4 */ }
pub struct IngestItem { /* per §2.4 items */ }

// JobRepo support types (forward-declared; full shapes can land here)
pub enum   JobKind { Ingest, Chunk, Embed, Ocr, Transcribe, Reindex, Doctor }
pub enum   JobStatus { Pending, Running, Succeeded, Failed, Canceled }
pub struct JobId(pub String);
pub struct JobFilter { /* status/kind */ }
pub struct JobRow { /* row mirror */ }

// Vector (forward-declared per §7.2)
pub struct VectorRecord { /* chunk_id, embedding_id, vector, doc_id, text, heading_path, model_id, model_version, dimensions */ }
pub struct VectorHit { /* chunk_id, score, payload */ }

// Errors (§10)
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("invalid id: {0}")]      InvalidId(String),
    #[error("invalid citation: {0}")] InvalidCitation(String),
    #[error("invalid source span: {0}")] InvalidSpan(String),
    #[error("malformed input: {0}")]  Malformed(String),
}

// ── Traits (§7.2) ───────────────────────────────────────────────────────────
pub trait SourceConnector { fn scan(&self, scope: &SourceScope) -> anyhow::Result<Vec<RawAsset>>; }
pub trait Extractor: Send + Sync {
    fn supports(&self, media_type: &MediaType) -> bool;
    fn parser_version(&self) -> ParserVersion;
    fn extract(&self, ctx: &ExtractContext, bytes: &[u8]) -> anyhow::Result<CanonicalDocument>;
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
    fn generate_stream(&self, req: GenerateRequest)
        -> anyhow::Result<Box<dyn Iterator<Item = anyhow::Result<TokenChunk>> + Send>>;
}
pub trait DocumentStore { /* full set per §7.2 */ }
pub trait VectorStore { /* full set per §7.2 */ }
pub trait JobRepo { /* full set per §7.2 */ }

// Helper input types (§7.1)
pub struct SourceScope { pub root: std::path::PathBuf, pub include: Vec<String>, pub exclude: Vec<String> }
pub struct ExtractContext<'a> { /* per §7.1 */ }
pub struct ExtractConfig { /* TBD by extractors; carry path-only for now */ }
pub struct ChunkPolicy { /* per §7.1 */ }
pub enum   EmbeddingKind { Document, Query }
pub struct EmbeddingInput<'a> { pub text: &'a str, pub kind: EmbeddingKind }
pub struct GenerateRequest { /* per §7.1 */ }
pub enum   TokenChunk { Token(String), Done { finish_reason: FinishReason, usage: TokenUsage } }
pub enum   FinishReason { Stop, Length, Aborted, Error(String) }

// ── ID functions (§4.2) ─────────────────────────────────────────────────────
pub fn id_from<T: serde::Serialize>(tuple: T) -> String;        // hex prefix 32
pub fn id_for_asset(asset_blake3_full_hex: &str) -> AssetId;
pub fn id_for_doc(workspace_path: &WorkspacePath, asset: &AssetId, parser_version: &ParserVersion) -> DocumentId;
pub fn id_for_block(doc: &DocumentId, block_kind: &str, heading_path: &[String], ordinal: u32, span: &SourceSpan) -> BlockId;
pub fn id_for_chunk(doc: &DocumentId, chunker_version: &ChunkerVersion, block_ids: &[BlockId], policy_hash: &str) -> ChunkId;
pub fn id_for_embedding(chunk: &ChunkId, model: &EmbeddingModelId, version: &EmbeddingVersion, dims: usize) -> EmbeddingId;
pub fn id_for_index(collection: &str, model: &EmbeddingModelId, dims: usize, version: &IndexVersion, kind: &str, params_hash: &str) -> IndexId;

pub fn to_posix(path: &std::path::Path) -> WorkspacePath;       // §6.6
pub fn nfc(input: &str) -> String;                              // §4.1
```

```rust
// ── kb-parse-types ──────────────────────────────────────────────────────────
// Per design §3.7b. Defines parser intermediate representations consumed by
// kb-normalize. Depends on kb-core only — never on parser libraries.

pub struct ParsedBlock {
    pub kind: ParsedBlockKind,
    pub heading_path: Vec<String>,
    pub source_span: kb_core::SourceSpan,
    pub payload: ParsedPayload,
}

pub enum ParsedBlockKind { Heading, Paragraph, List, Code, Table, Quote, ImageRef, AudioRef }

pub enum ParsedPayload {
    Heading   { level: u8, text: String },
    Paragraph { text: String, inlines: Vec<kb_core::Inline> },
    List      { ordered: bool, items: Vec<Vec<kb_core::Inline>> },
    Code      { lang: Option<String>, code: String },
    Table     { headers: Vec<String>, rows: Vec<Vec<String>> },
    Quote     { text: String, inlines: Vec<kb_core::Inline> },
    ImageRef  { src: String, alt: String },
    AudioRef  { src: String },
}

// `Inline` itself lives in kb-core (§3.4) — parse-types references it, never duplicates it.

pub struct Warning { pub kind: WarningKind, pub note: String }
pub enum WarningKind { MalformedFrontmatter, MalformedTable, EncodingFallback, ExtractFailed }

// Forward-ref for P6/P7/P8 — defined when those phases land.
pub struct ParsedImageRegion;
pub struct ParsedPdfPage;
pub struct ParsedAudioSegment;
```

```rust
// ── kb-config ───────────────────────────────────────────────────────────────
pub struct Config { /* full schema per §6.4 */ }
impl Config {
    pub fn load(path: Option<&std::path::Path>) -> anyhow::Result<Self>;
    pub fn from_file(path: &std::path::Path) -> anyhow::Result<Self>;
    pub fn defaults() -> Self;
    pub fn apply_env(self, env: &std::collections::HashMap<String, String>) -> Self;
    pub fn xdg_config_path() -> std::path::PathBuf;             // ~/.config/kb/config.toml
    pub fn xdg_data_dir() -> std::path::PathBuf;                // ~/.local/share/kb
    pub fn xdg_cache_dir() -> std::path::PathBuf;
    pub fn xdg_state_dir() -> std::path::PathBuf;
}
```

```rust
// ── kb-app ──────────────────────────────────────────────────────────────────
pub fn init_workspace(force: bool) -> anyhow::Result<()>;
pub fn ingest(scope: kb_core::SourceScope, summary_only: bool) -> anyhow::Result<kb_core::IngestReport>;
pub fn list_docs(filter: kb_core::DocFilter) -> anyhow::Result<Vec<kb_core::DocSummary>>;
pub fn inspect_doc(id: &kb_core::DocumentId) -> anyhow::Result<kb_core::CanonicalDocument>;
pub fn inspect_chunk(id: &kb_core::ChunkId) -> anyhow::Result<kb_core::Chunk>;
pub fn search(query: kb_core::SearchQuery) -> anyhow::Result<Vec<kb_core::SearchHit>>;
pub fn ask(query: &str, opts: AskOpts) -> anyhow::Result<kb_core::Answer>;
pub fn doctor() -> anyhow::Result<DoctorReport>;
pub struct AskOpts { pub k: usize, pub explain: bool, pub mode: kb_core::SearchMode, pub temperature: Option<f32>, pub seed: Option<u64> }
pub struct DoctorReport { pub ok: bool, pub checks: Vec<DoctorCheck> }
pub struct DoctorCheck { pub name: String, pub ok: bool, pub detail: String, pub hint: Option<String> }
```

P0 facade implementations call `anyhow::bail!("not yet wired (P<n>-<i>)")`; later phases replace bodies but never change signatures.

```rust
// ── kb-cli ──────────────────────────────────────────────────────────────────
// clap subcommands: init | ingest | list (docs) | inspect (doc|chunk) | search | ask | doctor | eval (subcommand placeholder)
// Each maps 1:1 to a kb_app function. Exit code mapping per §10.
```

## Behavior contract

- Workspace `Cargo.toml` sets `resolver = "3"`, `[workspace.package] edition = "2024"`, `rust-version = "1.85"`.
- Every newtype ID implements `Display` (returns inner) and `FromStr` (validates hex length 32).
- `id_from` uses `serde-json-canonicalizer` exactly as design §4.2 specifies and truncates blake3 to 32 hex chars.
- `Citation::to_uri` emits W3C Media Fragments URIs per §0 Q3 (`#L<a>-L<b>`, `#p=<n>`, `#xywh=…`, `#caption`, `#t=hh:mm:ss,hh:mm:ss[&speaker=…]`).
- `Citation::parse` is the strict inverse (round-trip property).
- `kb-config` resolves XDG paths via `dirs` crate; respects `XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `XDG_CACHE_HOME`, `XDG_STATE_HOME` if set.
- Config layer order: defaults → file → env (`KB_<SECTION>_<KEY>`) → CLI flag (CLI override is applied by `kb-cli` after `Config::load`).
- `kb-cli` global flags: `--config <path>`, `--verbose`, `--debug`, `--json`, `--explain` (where applicable). On `--json`, output conforms to wire schema v1.
- `kb-cli` exit codes: 0 success, 1 no-hit/refusal, 2 error, 3 doctor unhealthy (per §10).
- All facade-returned wire objects emit `schema_version` per §2 (e.g., `"answer.v1"`, `"search_hit.v1"`).

## Storage / wire effects

- Filesystem: creates `~/.config/kb/`, `~/.local/share/kb/`, `~/KnowledgeBase/` only when `kb init` runs; never on `Config::load`.
- Wire schemas: ships `docs/wire-schema/v1/{citation,search_hit,answer,ingest_report,doc_summary,chunk_inspection,doctor}.schema.json` as **stubs** declaring the top-level `schema_version` and required fields per §2. Full property validation can land later.
- DB: workspace ships `migrations/V001__init.sql` containing **only** §5.1 `schema_meta` + `migrations` tables (the full schema lands in p1-6's migration file or p0-1 may pre-stage the empty migrations directory; choose the former to keep this task within `kb-core`/`kb-config`/`kb-app`/`kb-cli` scope).
- Logging: `tracing` initialized in `kb-cli`; daily-rolling file in `~/.local/state/kb/logs/`.

## Test plan

| kind | description | fixture / data |
|------|-------------|----------------|
| unit | `id_from` deterministic across 1000 runs for fixed inputs | inline |
| unit | each `id_for_*` recipe matches design §4.2 byte-for-byte (verify against fixed expected hex) | inline |
| unit | `to_posix` collapses `./a//b.md` → `a/b.md` and NFC-normalizes Korean | inline |
| unit | `Citation::to_uri` and `parse` round-trip for all 5 variants | inline |
| unit | newtype `Display`/`FromStr` rejects invalid lengths/chars | inline |
| unit | `Config::defaults` + env override + CLI override produces expected merged config | inline |
| snapshot | `Config::defaults` JSON serde stable | inline (round-trip) |
| smoke | `kb --help`, `kb init`, `kb doctor` run; doctor reports config_loaded ✓ data_dir_writable ✓ even with no DB present (downstream checks may fail with hint) | tmp `XDG_*` env |
| build  | `cargo check --workspace` and `cargo test --workspace` pass | repo |

All tests must run with no network, no Ollama, no models.

## Definition of Done

- [ ] `Cargo.toml` workspace lists `kb-core`, `kb-parse-types`, `kb-config`, `kb-app`, `kb-cli` and resolver=3, edition 2024
- [ ] `cargo check --workspace` passes
- [ ] `cargo test --workspace` passes
- [ ] `kb-parse-types` `cargo tree` shows ONLY `kb-core` + `serde`/`thiserror` style deps (no parser libs, no other `kb-*`)
- [ ] `kb --help` prints subcommands
- [ ] `kb init` creates XDG dirs idempotently and writes `config.toml`
- [ ] `kb doctor` returns wire JSON conforming to `doctor.v1` (in `--json` mode)
- [ ] `docs/wire-schema/v1/*.schema.json` stubs exist (7 files: citation, search_hit, answer, ingest_report, doc_summary, chunk_inspection, doctor)
- [ ] `docs/spec/` stubs exist linking to the frozen design (one file per: domain-model, ids, canonical-document, chunk-policy, citation-policy, module-boundaries, ai-generation-guidelines)
- [ ] No imports outside Allowed dependencies (CI deny check)
- [ ] PR body links design §3, §3.7b, §4, §6, §7, §8, §9, §10

## Out of scope

- Any parser / store / embedder / search / llm / rag / tui / desktop logic (downstream phases).
- Full schema migrations (most DDL lands in p1-6 / p2-1 / p3-3).
- Wire schema deep validation (only required fields + `schema_version` checked here).
- Real `kb-app` business logic (functions stub with `unimplemented!()` or explicit `bail!`).

## Risks / notes

- ID recipe is the contract that every later record depends on. Any change after this task lands forces a `parser_version` / `chunker_version` / `embedding_version` cascade per §9. Treat changes as schema migrations and update the design doc first.
- Newtype IDs use `String` (not `[u8; 16]`) to keep serde simple; tests must still enforce 32-char hex constraint on `FromStr`.
- `kb-app` stubs must use `bail!` not `panic!` so the CLI exits with code 2 cleanly per §10.
- `clap` v4 derive: subcommand `inspect` has nested `doc` / `chunk` variants; ensure exit code 0/1/2 mapping wraps the facade call uniformly.
- XDG path discovery on macOS: spec uses XDG (not `Application Support`) per §6.1 — `dirs` crate honors XDG env vars; tests must set them explicitly.
