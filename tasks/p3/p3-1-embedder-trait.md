---
phase: P3
component: kebab-embed (trait crate)
task_id: p3-1
title: "Embedder trait + EmbeddingInput/Kind validation"
status: completed
depends_on: [p0-1]
unblocks: [p3-2, p3-3, p3-4]
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [design §3.7 SearchHit.embedding_model, design §7.1 EmbeddingInput/Kind, design §7.2 Embedder, report §11 LLM/embedding split]
---

# p3-1 — Embedder trait crate

## Goal

Provide the `kebab-embed` crate that re-exports `Embedder` trait, `EmbeddingInput`/`EmbeddingKind`, and offers a mock implementation for downstream tests. This task is **trait-only**; concrete adapters live in p3-2.

## Why now / why this size

Concrete adapters (fastembed, ollama-embed, candle) need a stable trait surface. Owning the trait + a mock implementation in a tiny crate keeps `kebab-store-vector` and `kebab-search` testable without touching real models.

## Allowed dependencies

- `kebab-core`
- `kebab-config`
- `serde`
- `thiserror`
- `tracing`
- `[features] mock = []` — opt-in feature flag exposing `MockEmbedder`. Default OFF. Release builds (omit `--features mock`) compile `MockEmbedder` out entirely.

## Forbidden dependencies

- `fastembed`, `ort`, `tokenizers`, `kebab-source-fs`, `kebab-parse-md`, `kebab-normalize`, `kebab-chunk`, `kebab-store-*`, `kebab-search`, `kebab-llm*`, `kebab-rag`, `kebab-tui`, `kebab-desktop`

## Inputs

| input | type | source |
|-------|------|--------|
| `EmbeddingInput` | `kebab_core::EmbeddingInput<'_>` | callers (parser-side or query-side) |
| model identity | `(EmbeddingModelId, EmbeddingVersion, dimensions)` | adapter at construction |

## Outputs

| output | type | downstream |
|--------|------|------------|
| `Vec<Vec<f32>>` | row-aligned with input | `kebab-store-vector`, `kebab-search` (vector mode) |

## Public surface (signatures only — no new types)

```rust
pub use kebab_core::{EmbeddingInput, EmbeddingKind, EmbeddingModelId, EmbeddingVersion, Embedder};

/// Test-only mock that produces deterministic vectors. Compiled only when `mock` feature is on.
#[cfg(feature = "mock")]
pub struct MockEmbedder { /* internal: model_id, dims, seed */ }
#[cfg(feature = "mock")]
impl MockEmbedder {
    pub fn new(model_id: kebab_core::EmbeddingModelId, version: kebab_core::EmbeddingVersion, dimensions: usize) -> Self;
}
#[cfg(feature = "mock")]
impl kebab_core::Embedder for MockEmbedder { /* per §7.2 */ }
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

All tests under `cargo test -p kebab-embed`.

## Definition of Done

- [ ] `cargo check -p kebab-embed` passes
- [ ] `cargo test -p kebab-embed` passes
- [ ] No external embedding dep present
- [ ] PR links design §7.2 Embedder, §11

## Out of scope

- Real adapter (`kebab-embed-local` is p3-2).
- Reranker traits (P+).

## Risks / notes

- `MockEmbedder` is gated by `mock` feature (default OFF). Downstream tests opt in via `[dev-dependencies] kebab-embed = { path = "...", features = ["mock"] }`. CI build of release binary (`cargo build --release` without `--features mock`) MUST NOT include `MockEmbedder` symbol — verifiable via `cargo bloat` or `nm` symbol scan.
- Trait re-exports keep the call site stable even if `kebab-core` reorganizes; downstream crates should `use kebab_embed::Embedder` rather than `use kebab_core::Embedder`.
