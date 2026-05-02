---
phase: P4
component: kebab-llm (trait crate)
task_id: p4-1
title: "LanguageModel trait + GenerateRequest/TokenChunk"
status: completed
depends_on: [p0-1]
unblocks: [p4-2, p4-3]
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§7.1 GenerateRequest/TokenChunk, §7.2 LanguageModel, §0 Q5 streaming, §3.8 ModelRef]
---

# p4-1 — LanguageModel trait crate

## Goal

Provide the `kebab-llm` crate that re-exports the `LanguageModel` trait and helper types (`GenerateRequest`, `TokenChunk`, `FinishReason`, `TokenUsage`, `ModelRef`), plus a `MockLanguageModel` for downstream tests.

## Why now / why this size

`kebab-rag` (p4-3) consumes a `LanguageModel` trait object. Owning the trait + a deterministic mock here lets RAG tests run with no Ollama dependency. Real adapters (Ollama, llama.cpp, candle) live in p4-2 and beyond.

## Allowed dependencies

- `kebab-core`
- `kebab-config`
- `serde`
- `thiserror`
- `tracing`
- `[features] mock = []` — opt-in feature flag exposing `MockLanguageModel`. Default OFF. Release builds compile mock out entirely.

## Forbidden dependencies

- `reqwest`, `ureq`, `tokio`, `whisper-rs`, `kebab-source-fs`, `kebab-parse-md`, `kebab-normalize`, `kebab-chunk`, `kebab-store-*`, `kebab-embed*`, `kebab-search`, `kebab-rag`, `kebab-tui`, `kebab-desktop`

## Inputs

| input | type | source |
|-------|------|--------|
| `GenerateRequest` | `kebab_core::GenerateRequest` | RAG pipeline |
| concrete adapter at runtime | `dyn LanguageModel` | p4-2+ |

## Outputs

| output | type | downstream |
|--------|------|------------|
| streaming `TokenChunk` iterator | `Box<dyn Iterator<Item=anyhow::Result<TokenChunk>> + Send>` | RAG pipeline |
| `ModelRef` identity | `kebab_core::ModelRef` | Answer.model |

## Public surface (signatures only — no new types)

```rust
pub use kebab_core::{LanguageModel, GenerateRequest, TokenChunk, FinishReason, TokenUsage, ModelRef};

/// Test-only deterministic mock. Compiled only when `mock` feature is on.
#[cfg(feature = "mock")]
pub struct MockLanguageModel {
    pub model_id: String,
    pub provider: String,
    pub context_tokens: usize,
    pub canned_response: String,                 // emitted token-by-token
    pub canned_finish: kebab_core::FinishReason,
    pub canned_usage:  kebab_core::TokenUsage,
}

#[cfg(feature = "mock")]
impl kebab_core::LanguageModel for MockLanguageModel { /* per §7.2 */ }
```

## Behavior contract

- `MockLanguageModel::generate_stream` produces a `Box<dyn Iterator>` that yields the canned response one Unicode character at a time as `TokenChunk::Token`, then a final `TokenChunk::Done { finish_reason, usage }`.
- The mock honors `GenerateRequest.stop`: if any stop string appears in the canned response, truncate before emitting.
- `model_ref()` returns `ModelRef { id, provider, dimensions: None }`.
- The mock must NOT touch the network or filesystem.
- Real adapters (p4-2+) MUST NOT live in this crate.

## Storage / wire effects

- None.

## Test plan

| kind | description | fixture / data |
|------|-------------|----------------|
| unit | mock streams 5 tokens then `Done` | inline |
| unit | mock honors stop strings | inline |
| unit | trait dyn dispatch via `Box<dyn LanguageModel>` works | inline |
| unit | concatenation of streamed `TokenChunk::Token` equals canned text (truncated by stop strings) | inline |
| contract | `model_ref()` populates `provider` and leaves `dimensions = None` | inline |

All tests under `cargo test -p kebab-llm`.

## Definition of Done

- [ ] `cargo check -p kebab-llm` passes
- [ ] `cargo test -p kebab-llm` passes
- [ ] No HTTP / async runtime deps present
- [ ] PR links design §7.2 LanguageModel, §0 Q5

## Out of scope

- Real adapter (p4-2).
- Token counting against the actual tokenizer (best-effort via `usage.prompt_tokens` reported by the adapter).
- Server-side cancellation / abort signals (P+).

## Risks / notes

- Real adapters return Unicode-incomplete byte sequences mid-stream; the trait emits `TokenChunk::Token(String)` so adapters must handle UTF-8 boundary buffering internally.
- `TokenChunk::Done { usage }` must always fire, even on error — adapters convert errors into `FinishReason::Error(msg)` and a final `Done`.
