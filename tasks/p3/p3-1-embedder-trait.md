---
phase: P3
component: kb-embed (trait crate)
task_id: p3-1
title: "Embedder trait + EmbeddingInput/Kind validation"
status: planned
depends_on: [p0-1]
unblocks: [p3-2, p3-3, p3-4]
contract_source: ../../docs/superpowers/specs/2026-04-27-kb-final-form-design.md
contract_sections: [§3.7 SearchHit.embedding_model, §7.1 EmbeddingInput/Kind, §7.2 Embedder, §11 LLM/embedding split]
---

# p3-1 — Embedder trait crate

## Goal

Provide the `kb-embed` crate that re-exports `Embedder` trait, `EmbeddingInput`/`EmbeddingKind`, and offers a mock implementation for downstream tests. This task is **trait-only**; concrete adapters live in p3-2.

## Why now / why this size

Concrete adapters (fastembed, ollama-embed, candle) need a stable trait surface. Owning the trait + a mock implementation in a tiny crate keeps `kb-store-vector` and `kb-search` testable without touching real models.

## Allowed dependencies

- `kb-core`
- `kb-config`
- `serde`
- `thiserror`
- `tracing`

## Forbidden dependencies

- `fastembed`, `ort`, `tokenizers`, `kb-source-fs`, `kb-parse-md`, `kb-normalize`, `kb-chunk`, `kb-store-*`, `kb-search`, `kb-llm*`, `kb-rag`, `kb-tui`, `kb-desktop`

## Inputs

| input | type | source |
|-------|------|--------|
| `EmbeddingInput` | `kb_core::EmbeddingInput<'_>` | callers (parser-side or query-side) |
| model identity | `(EmbeddingModelId, EmbeddingVersion, dimensions)` | adapter at construction |

## Outputs

| output | type | downstream |
|--------|------|------------|
| `Vec<Vec<f32>>` | row-aligned with input | `kb-store-vector`, `kb-search` (vector mode) |

## Public surface (signatures only — no new types)

```rust
pub use kb_core::{EmbeddingInput, EmbeddingKind, EmbeddingModelId, EmbeddingVersion, Embedder};

/// Test-only mock that produces deterministic vectors.
pub struct MockEmbedder { /* internal: model_id, dims, seed */ }
impl MockEmbedder {
    pub fn new(model_id: kb_core::EmbeddingModelId, version: kb_core::EmbeddingVersion, dimensions: usize) -> Self;
}
impl kb_core::Embedder for MockEmbedder { /* per §7.2 */ }
```

## Behavior contract

- `MockEmbedder::embed` produces vectors deterministically from `(text, kind)`: e.g., `vector[i] = hash_to_unit_float(text, kind, i, seed)` so two identical inputs produce identical vectors and different inputs produce nearly-orthogonal vectors. Used by downstream tests.
- `MockEmbedder` must respect `EmbeddingKind::Document` vs `Query` — different prefix mixed into the hash so query embeddings differ from document embeddings of the same text (mirrors real e5 behavior).
- `dimensions()` returns the value passed at construction; callers must trust it.
- Real adapters (p3-2) MUST NOT implement `Embedder` here.
- The crate may expose a tiny helper `pub fn assert_vector_shape(vecs: &[Vec<f32>], expected_dims: usize)` for downstream tests.

## Storage / wire effects

- None.

## Test plan

| kind | description | fixture / data |
|------|-------------|----------------|
| unit | trait dyn dispatch via `Box<dyn Embedder>` works | inline |
| unit | `MockEmbedder` produces identical vector for identical input | inline |
| unit | `EmbeddingKind::Document` vs `Query` for same text yield different vectors | inline |
| unit | dimensions match construction-time value | inline |
| contract | property test: 100 random inputs, each vector has length == dimensions, all finite floats | inline (proptest) |

All tests under `cargo test -p kb-embed`.

## Definition of Done

- [ ] `cargo check -p kb-embed` passes
- [ ] `cargo test -p kb-embed` passes
- [ ] No external embedding dep present
- [ ] PR links design §7.2 Embedder, §11

## Out of scope

- Real adapter (`kb-embed-local` is p3-2).
- Reranker traits (P+).

## Risks / notes

- `MockEmbedder` is for tests; do not let it leak into release builds via default features. Gate behind `cfg(test)` or a `mock` feature flag.
- Trait re-exports keep the call site stable even if `kb-core` reorganizes; downstream crates should `use kb_embed::Embedder` rather than `use kb_core::Embedder`.
