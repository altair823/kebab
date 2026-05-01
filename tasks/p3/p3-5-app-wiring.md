---
phase: P3
component: kb-app (facade wiring)
task_id: p3-5
title: "Wire kb-app facade — ingest / search / list / inspect end-to-end"
status: planned
depends_on: [p1-6, p2-2, p3-2, p3-3, p3-4]
unblocks: [p4-3, p9-1, p9-2, p9-4]
contract_source: ../../docs/superpowers/specs/2026-04-27-kb-final-form-design.md
contract_sections: [§7 components, §7.2 traits, §1 UX scenes (ingest, search, list, inspect), §6.4 config, §10 errors]
---

# p3-5 — Wire kb-app facade

## Goal

Replace the `bail!("not yet wired")` stubs in `kb-app` (currently `ingest`, `search`, `list_docs`, `inspect_doc`, `inspect_chunk`) with real bodies that compose the libraries shipped in P1–P3. After this task, `kb index` actually walks a workspace and persists chunks, and `kb search --mode {lexical,vector,hybrid}` returns real `SearchHit`s. `kb-app::ask` stays stubbed (P4-3 owns it).

## Why now / why this size

P1–P3 shipped libraries but the CLI is unusable: every facade method bails. Inserting this glue task between P3-4 (last library that completes the retrieval stack) and P4-1 (LLM trait, where `kb-app::ask` will need this same wiring anyway) lets the user run the tool end-to-end for the first time and validates that the library boundaries actually compose. Limiting it to non-LLM commands keeps it a single-session task; `ask` wiring lives in P4-3.

## Allowed dependencies

`kb-app` may depend on:

- `kb-core`, `kb-config` (already)
- `kb-source-fs`, `kb-parse-md`, `kb-normalize`, `kb-chunk`, `kb-store-sqlite` (P1)
- `kb-search` (P2-2, P3-4)
- `kb-store-vector`, `kb-embed`, `kb-embed-local` (P3-2, P3-3, P3-4)
- `tracing`, `anyhow`, `serde`, `serde_json`, `time`, `dirs`, `toml` (existing)

## Forbidden dependencies

- `kb-llm*`, `kb-rag` (P4 ownership — `ask` stays stubbed)
- `kb-tui`, `kb-desktop` (P9 — those *consume* `kb-app`, not the other way)
- `kb-parse-pdf`, `kb-parse-image`, `kb-parse-audio` (P6/P7/P8)
- network HTTP libs (model download is `kb-embed-local`'s responsibility via fastembed)

## Inputs

| input | type | source |
|-------|------|--------|
| `kb-config::Config` | `kb_config::Config` | loaded once at process start; threaded through every facade call |
| `SourceScope` | `kb_core::SourceScope` | CLI |
| `SearchQuery` | `kb_core::SearchQuery` | CLI |
| `DocFilter` | `kb_core::DocFilter` | CLI |
| `DocumentId` / `ChunkId` | `kb_core::ids::*` | CLI |

## Outputs

| output | type | downstream |
|--------|------|------------|
| `IngestReport` | `kb_core::IngestReport` | `kb-cli` (`ingest_report.v1`), `kb-eval` (P5) |
| `Vec<SearchHit>` | `kb_core::SearchHit` | `kb-cli` (`search_hit.v1`), `kb-rag` (P4-3) |
| `Vec<DocSummary>` | `kb_core::DocSummary` | `kb-cli` (`doc_summary.v1`) |
| `CanonicalDocument` / `Chunk` | `kb_core::*` | `kb-cli` (`chunk_inspection.v1`) |

## Public surface (signatures only — no new types)

The signatures already live on `kb-app`. This task only swaps the bodies. Frozen surface:

```rust
pub fn ingest(scope: SourceScope, summary_only: bool) -> anyhow::Result<IngestReport>;
pub fn list_docs(filter: DocFilter)               -> anyhow::Result<Vec<DocSummary>>;
pub fn inspect_doc(id: &DocumentId)               -> anyhow::Result<CanonicalDocument>;
pub fn inspect_chunk(id: &ChunkId)                -> anyhow::Result<Chunk>;
pub fn search(query: SearchQuery)                 -> anyhow::Result<Vec<SearchHit>>;
// `ask` and `init_workspace` / `doctor` stay as-is.
```

If a constructor helper is needed (e.g., a single `App::open(&Config)` that opens `SqliteStore`, `LanceVectorStore`, embedder, retrievers once), it MAY be added pub-or-pub(crate) to `kb-app`. Keep all newly-added types crate-private unless explicitly required by `kb-cli`.

## Behavior contract

### `ingest(scope, summary_only)`

Pipeline per design §1.2:

1. Resolve `scope` → set of files under `config.workspace.root` filtered by `config.workspace.include`/`exclude`. Use `kb-source-fs::FsSourceConnector` and pass each `RawAsset` + bytes through.
2. For each markdown asset: `kb_parse_md::frontmatter::parse_frontmatter` + `kb_parse_md::blocks::parse_blocks` → `kb_normalize::build_canonical_document` → `kb_chunk::MdHeadingV1Chunker::chunk`.
3. SQLite writes per design §5.8 (one ingest = one transaction): `put_asset_with_bytes` → `put_document` → `put_blocks` → `put_chunks`.
4. If `config.models.embedding.provider == "fastembed"`: build `FastembedEmbedder` once at the top of the run, embed every chunk's `text` as `EmbeddingKind::Document`, batch by `config.models.embedding.batch_size`, then `LanceVectorStore::ensure_table` + `upsert`. Skip vector indexing if provider is `"none"` or `embedding.dimensions == 0` (config opt-out path).
5. Record an `ingest_runs` row via `JobRepo` with the aggregate counts. `summary_only=true` writes `items_json=NULL`.
6. Return `IngestReport` per `wire-schema/v1/ingest_report.schema.json`.

Errors: any per-file parse failure should be recorded as a `Warning` in `Provenance` and skipped, not propagated. Only structural failures (DB unreachable, FS permission denied) abort the whole run.

### `search(query)`

Per `query.mode`:

- `Lexical` → `kb_search::LexicalRetriever::search`. Takes a read-only borrow of the SQLite connection.
- `Vector` → `kb_search::VectorRetriever::search`. Requires `Embedder` (built from config) + `VectorStore` (LanceDB).
- `Hybrid` → `kb_search::HybridRetriever::search` composing the above two.

Each call constructs (or reuses) the retrievers from a single `App` context that holds `Arc<SqliteStore>`, `Arc<LanceVectorStore>`, `Arc<dyn Embedder>` so cold-start cost is paid once per process. Document the lifetime model: a CLI invocation paths through `App::open(&Config)` once, runs the requested op, then drops everything on exit. The TUI (P9) holds the `App` for the session.

`SearchHit.embedding_model` set to `Some(embedder.model_id())` for Vector / Hybrid modes; `None` for Lexical-only. `SearchHit.index_version` follows the retriever's reported `index_version()`.

### `list_docs(filter)` / `inspect_doc(id)` / `inspect_chunk(id)`

Direct delegation to `kb_core::DocumentStore` trait methods on `SqliteStore`. No vector / embedding involvement.

### Lifecycle

Define an internal `pub(crate) struct App { config: Arc<Config>, sqlite: Arc<SqliteStore>, vector: Option<Arc<LanceVectorStore>>, embedder: Option<Arc<dyn Embedder + Send + Sync>>, /* retrievers built per call */ }` with a `pub(crate) fn open(config: &Config) -> Result<Self>`. Each public `kb-app::*` function builds an `App` (or accepts one in tests) and uses it. The free functions stay the public API so `kb-cli` and `kb-tui` don't need to refactor.

`vector` and `embedder` are `Option` because the spec allows running KB without embeddings (lexical-only mode for resource-constrained boxes — `provider == "none"`). When absent, `search` with `Vector` / `Hybrid` modes returns `anyhow::Error` with a hint to enable embeddings; `ingest` skips the vector step.

### Versioning

- `parser_version` from `kb-parse-md` constant.
- `chunker_version` from `MdHeadingV1Chunker::chunker_version()`.
- `embedding_model` / `embedding_version` from the embedder.
- `index_version` from the retriever (or `Lexical`/`Vector` IndexId).

All wired through to the persisted records and the wire output.

## Storage / wire effects

- Writes: `data_dir/kb.sqlite` (assets, documents, blocks, chunks, embedding_records, jobs/ingest_runs), `data_dir/assets/<aa>/<id>` (when copied), `data_dir/lancedb/chunk_embeddings_<model>_<dim>.lance/` (when embeddings on), `data_dir/models/fastembed/` (model cache, first run only).
- Reads on subsequent calls: same DB.
- Wire JSON conforms to `ingest_report.v1`, `search_hit.v1`, `doc_summary.v1`, `chunk_inspection.v1` — `kb-cli` already has the wrappers.

## Test plan

| kind | description | fixture / data |
|------|-------------|----------------|
| integration | `ingest` over a 3-file fixture workspace produces non-empty `IngestReport` and writes the expected SQLite rows + Lance rows | `crates/kb-app/tests/fixtures/workspace/`, tmp `data_dir` |
| integration | `ingest` with `provider="none"` skips Lance and produces an `IngestReport` with `embeddings_indexed=0` | tmp config |
| integration | `ingest` is idempotent: re-running on the same workspace updates `documents.updated_at` and bumps `doc_version` without duplicating rows | tmp `data_dir` |
| integration | `search` with `mode=Lexical` returns hits whose `embedding_model` is `None` | tmp `data_dir` |
| integration | `search` with `mode=Vector` returns hits whose `embedding_model` matches the configured model | tmp `data_dir` (AVX `#[ignore]`) |
| integration | `search` with `mode=Hybrid` returns hits whose `RetrievalDetail.method == Hybrid` | tmp `data_dir` (AVX `#[ignore]`) |
| integration | `search` with `mode=Vector` and `provider="none"` returns a clear error | tmp config |
| integration | `list_docs` with `tags_any=["rust"]` filters correctly | tmp `data_dir` |
| integration | `inspect_doc` round-trips a document; `inspect_chunk` round-trips a chunk | tmp `data_dir` |
| smoke | end-to-end CLI smoke: `kb index` + `kb search --mode hybrid "..."` against the fixture workspace using `cargo run` (manual / `assert_cmd`) | fixture |

`#[ignore]` AVX-gated tests follow the P3-3/P3-4 pattern: `require_avx_or_panic()` at the top of each LanceDB-touching body. Default `cargo test -p kb-app` runs the lexical-only and SQLite-only paths.

All tests under `cargo test -p kb-app`. CLI smoke optional via `assert_cmd` if it doesn't add a heavyweight dep tree.

## Definition of Done

- [ ] `cargo check -p kb-app` passes.
- [ ] `cargo test -p kb-app` passes (default lane).
- [ ] `cargo test -p kb-app -- --ignored` passes on AVX hardware.
- [ ] `cargo run -p kb-cli -- index` succeeds against a non-empty fixture workspace.
- [ ] `cargo run -p kb-cli -- search --mode hybrid "<term>"` returns real hits + citations.
- [ ] `cargo run -p kb-cli -- list` returns the indexed documents.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` clean.
- [ ] No imports outside Allowed dependencies.
- [ ] PR links design §1.2, §1.5, §1.6, §7.

## Out of scope

- `kb-app::ask` body — P4-3 (RAG pipeline).
- `kb index --rebuild-fts` CLI option — wire `kb_store_sqlite::rebuild_chunks_fts` only when needed by a downstream task.
- `kb index --resume` checkpointing — design §1.2 mentions it but spec leaves to a P+ refinement; document but defer.
- `--watch` mode — P+.
- TUI / desktop integration — P9 consumes the wired facade.

## Risks / notes

- Cold-start cost: first `kb index` run downloads the fastembed model (~470MB) and warms the ONNX session. Surface via `tracing::info!` (already wired in P3-2).
- The `App` lifecycle struct is internal but its construction is the natural seam for adding caching / connection pooling later. Keep it `pub(crate)` so future refactors don't break the CLI.
- Mismatched `index_version` across stored records and the live retriever should fail loud at `App::open` (not at first search). Reuse the `tracing::warn!` from `HybridRetriever::new` (P3-4).
- The fastembed adapter holds a `tokio::runtime` (P3-3); `App` must be constructed from a synchronous context. Document on `App::open`.
- Performance: `ingest` over a large workspace (10k files) needs to keep SQLite WAL in healthy shape. One transaction per document is the spec; verify checkpoint behavior under load (manual benchmark, not a unit test).
