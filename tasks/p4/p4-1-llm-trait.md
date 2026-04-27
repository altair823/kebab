---
phase: P4
component: kb-llm (trait crate)
task_id: p4-1
title: "LanguageModel trait + GenerateRequest/TokenChunk"
status: planned
depends_on: [p0-1]
unblocks: [p4-2, p4-3]
contract_source: ../../docs/superpowers/specs/2026-04-27-kb-final-form-design.md
contract_sections: [§7.1 GenerateRequest/TokenChunk, §7.2 LanguageModel, §0 Q5 streaming, §3.8 ModelRef]
---

# p4-1 — LanguageModel trait crate

## Goal

Provide the `kb-llm` crate that re-exports the `LanguageModel` trait and helper types (`GenerateRequest`, `TokenChunk`, `FinishReason`, `TokenUsage`, `ModelRef`), plus a `MockLanguageModel` for downstream tests.

## Why now / why this size

`kb-rag` (p4-3) consumes a `LanguageModel` trait object. Owning the trait + a deterministic mock here lets RAG tests run with no Ollama dependency. Real adapters (Ollama, llama.cpp, candle) live in p4-2 and beyond.

## Allowed dependencies

- `kb-core`
- `kb-config`
- `serde`
- `thiserror`
- `tracing`

## Forbidden dependencies

- `reqwest`, `ureq`, `tokio`, `whisper-rs`, `kb-source-fs`, `kb-parse-md`, `kb-normalize`, `kb-chunk`, `kb-store-*`, `kb-embed*`, `kb-search`, `kb-rag`, `kb-tui`, `kb-desktop`

## Inputs

| input | type | source |
|-------|------|--------|
| `GenerateRequest` | `kb_core::GenerateRequest` | RAG pipeline |
| concrete adapter at runtime | `dyn LanguageModel` | p4-2+ |

## Outputs

| output | type | downstream |
|--------|------|------------|
| streaming `TokenChunk` iterator | `Box<dyn Iterator<Item=anyhow::Result<TokenChunk>> + Send>` | RAG pipeline |
| `ModelRef` identity | `kb_core::ModelRef` | Answer.model |

## Public surface (signatures only — no new types)

```rust
pub use kb_core::{LanguageModel, GenerateRequest, TokenChunk, FinishReason, TokenUsage, ModelRef};

/// Test-only deterministic mock.
pub struct MockLanguageModel {
    pub model_id: String,
    pub provider: String,
    pub context_tokens: usize,
    pub canned_response: String,                 // emitted token-by-token
    pub canned_finish: kb_core::FinishReason,
    pub canned_usage:  kb_core::TokenUsage,
}

impl kb_core::LanguageModel for MockLanguageModel { /* per §7.2 */ }
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

All tests under `cargo test -p kb-llm`.

## Definition of Done

- [ ] `cargo check -p kb-llm` passes
- [ ] `cargo test -p kb-llm` passes
- [ ] No HTTP / async runtime deps present
- [ ] PR links design §7.2 LanguageModel, §0 Q5

## Out of scope

- Real adapter (p4-2).
- Token counting against the actual tokenizer (best-effort via `usage.prompt_tokens` reported by the adapter).
- Server-side cancellation / abort signals (P+).

## Risks / notes

- Real adapters return Unicode-incomplete byte sequences mid-stream; the trait emits `TokenChunk::Token(String)` so adapters must handle UTF-8 boundary buffering internally.
- `TokenChunk::Done { usage }` must always fire, even on error — adapters convert errors into `FinishReason::Error(msg)` and a final `Done`.
