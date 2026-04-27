---
phase: P3
component: kb-embed-local (fastembed adapter)
task_id: p3-2
title: "fastembed-rs Embedder for multilingual-e5-small"
status: planned
depends_on: [p3-1]
unblocks: [p3-3, p3-4]
contract_source: ../../docs/superpowers/specs/2026-04-27-kb-final-form-design.md
contract_sections: [design §7.2 Embedder, report §11.3 local embedding, design §6.4 [models.embedding], design §9 versioning]
---

# p3-2 — fastembed adapter

## Goal

Provide `FastembedEmbedder` implementing `Embedder` for `multilingual-e5-small` (default) using `fastembed-rs` (ONNX runtime). Apply Document/Query prefix per §11.3. Honor `batch_size` from config.

## Why now / why this size

First real `Embedder`. Drives `EmbeddingId` recipe (model_id + model_version + dims) downstream. Isolated from store/search so model swaps remain config-only.

## Allowed dependencies

- `kb-core`
- `kb-config`
- `kb-embed`
- `fastembed = "4"` (or current stable)
- `tokenizers`
- `ort` (transitive via fastembed)
- `tracing`
- `thiserror`

## Forbidden dependencies

- `kb-source-fs`, `kb-parse-md`, `kb-normalize`, `kb-chunk`, `kb-store-*`, `kb-search`, `kb-llm*`, `kb-rag`, `kb-tui`, `kb-desktop`, network HTTP libs (model download is fastembed's responsibility)

## Inputs

| input | type | source |
|-------|------|--------|
| `kb-config::Config.models.embedding` | settings | runtime |
| `EmbeddingInput[..]` | `kb_core::EmbeddingInput<'_>[]` | callers |
| model cache | `data_dir/models/fastembed/` | filesystem |

## Outputs

| output | type | downstream |
|--------|------|------------|
| `Vec<Vec<f32>>` | row-aligned, `dimensions = 384` | `kb-store-vector`, query vectors for hybrid search |
| model identity | `(EmbeddingModelId, EmbeddingVersion, usize)` | record fields, `embedding_id` recipe |

## Public surface (signatures only — no new types)

```rust
pub struct FastembedEmbedder { /* internal: TextEmbedding instance + model meta */ }

impl FastembedEmbedder {
    pub fn new(config: &kb_config::Config) -> anyhow::Result<Self>;
}

impl kb_core::Embedder for FastembedEmbedder {
    fn model_id(&self) -> kb_core::EmbeddingModelId;
    fn model_version(&self) -> kb_core::EmbeddingVersion;
    fn dimensions(&self) -> usize;
    fn embed(&self, inputs: &[kb_core::EmbeddingInput<'_>]) -> anyhow::Result<Vec<Vec<f32>>>;
}
```

## Behavior contract

- Default model `multilingual-e5-small` (384 dims). `model_id()` returns `EmbeddingModelId("multilingual-e5-small")`.
- `model_version()` returns `EmbeddingVersion("v1")` initially. Bump per §9 if fastembed upgrades the bundled weights.
- Apply e5 prefix per §11.3: input prefixed with `"passage: "` for `EmbeddingKind::Document`, `"query: "` for `EmbeddingKind::Query` BEFORE tokenization.
- Batch processing respects `config.models.embedding.batch_size`. Inputs longer than the batch are split into multiple inference calls and concatenated.
- L2-normalize each vector before returning (e5 convention).
- Dimensions must equal `config.models.embedding.dimensions` AND the model's actual dim. Mismatch returns `anyhow::Error` at construction (not at first `embed`).
- Model files cached under `config.storage.model_dir/fastembed/` (downloaded on first use).
- Determinism: identical input + identical model version → identical vectors (tolerance < 1e-6 on aggregate hash for snapshot tests).
- No async runtime: the trait is synchronous. fastembed is sync internally.

## Storage / wire effects

- Reads/writes `data_dir/models/fastembed/` (model cache).
- Otherwise no DB or wire effects.

## Test plan

| kind | description | fixture / data |
|------|-------------|----------------|
| unit | construction with default config returns dims=384 | tmp config |
| unit | construction with mismatched dims returns error | tmp config |
| unit | `EmbeddingKind::Query` vs `Document` for same text yield different vectors (cosine < 1.0) | inline |
| unit | output vectors are L2-normalized (norm ~= 1.0 ± 1e-3) | inline |
| determinism | identical input twice → identical output (hash-of-floats compare) | inline |
| performance | batch of 64 short inputs completes in < 5s on CI host | tmp config (skip on slow CI via `#[ignore]`) |
| snapshot | aggregate hash of vectors for 5 known sentences stable across runs | `fixtures/embed/known-sentences.json` |

All tests under `cargo test -p kb-embed-local`. Mark slow tests `#[ignore]` and run via `cargo test -- --ignored` in dedicated CI lane.

## Definition of Done

- [ ] `cargo check -p kb-embed-local` passes
- [ ] `cargo test -p kb-embed-local` passes (excluding `#[ignore]`)
- [ ] First-run model download works under `data_dir/models/fastembed/`
- [ ] No imports outside Allowed dependencies
- [ ] PR links design §11.3, §6.4, §9

## Out of scope

- Reranker (P+).
- Other model providers (Ollama embedding endpoint, candle) — separate adapter crates.
- Visual / image embeddings (P6).

## Risks / notes

- ONNX runtime first-load latency on M-series Macs (Metal) can be 1-2 s; tests share a `OnceCell<FastembedEmbedder>`.
- Forgetting the e5 prefix silently degrades retrieval quality. Tests must assert query/document yield distinct vectors.
- Bumping `EmbeddingVersion` invalidates every `embedding_id`. Treat as a versioning event per §9 — provides justification in PR body.
