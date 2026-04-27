---
phase: P4
component: kb-llm-local (Ollama adapter)
task_id: p4-2
title: "OllamaLanguageModel — streaming /api/generate"
status: planned
depends_on: [p4-1]
unblocks: [p4-3]
contract_source: ../../docs/superpowers/specs/2026-04-27-kb-final-form-design.md
contract_sections: [design §7.2 LanguageModel, report §11.2 Ollama, design §6.4 [models.llm], design §0 Q5 streaming, design §10 errors]
---

# p4-2 — Ollama adapter

## Goal

Implement `OllamaLanguageModel` against Ollama's local HTTP API (`POST /api/generate` with `stream: true`). Honors temperature/seed for determinism, maps Ollama error states to `LlmError` per §10, and surfaces helpful hints (e.g., `ollama pull <model>`).

## Why now / why this size

First real LM. Required for `kb ask` to function. Isolated from RAG pipeline so swapping providers stays config-only.

## Allowed dependencies

- `kb-core`
- `kb-config`
- `kb-llm`
- `reqwest = { version = "0.12", default-features = false, features = ["blocking", "json", "rustls-tls"] }`
- `serde`, `serde_json`
- `tracing`
- `thiserror`

## Forbidden dependencies

- `tokio`, `async-std`, `kb-source-fs`, `kb-parse-md`, `kb-normalize`, `kb-chunk`, `kb-store-*`, `kb-embed*`, `kb-search`, `kb-rag`, `kb-tui`, `kb-desktop`. (Streaming uses `reqwest::blocking::Response::bytes_stream` via line-delimited JSON; no async runtime needed.)

## Inputs

| input | type | source |
|-------|------|--------|
| `kb-config::Config.models.llm` | endpoint, model, context, temperature, seed | runtime |
| `GenerateRequest` | `kb_core::GenerateRequest` | RAG pipeline |
| Ollama HTTP server (local) | `http://127.0.0.1:11434` | external process |

## Outputs

| output | type | downstream |
|--------|------|------------|
| streaming `TokenChunk` iterator | per §7.2 | `kb-rag` |
| `ModelRef` | `{ id, provider="ollama", dimensions=None }` | `Answer.model` |

## Public surface (signatures only — no new types)

```rust
pub struct OllamaLanguageModel { /* internal: reqwest::blocking::Client + config */ }

impl OllamaLanguageModel {
    pub fn new(config: &kb_config::Config) -> anyhow::Result<Self>;
}

impl kb_core::LanguageModel for OllamaLanguageModel {
    fn model_ref(&self) -> kb_core::ModelRef;
    fn context_tokens(&self) -> usize;
    fn generate_stream(&self, req: kb_core::GenerateRequest)
        -> anyhow::Result<Box<dyn Iterator<Item = anyhow::Result<kb_core::TokenChunk>> + Send>>;
}
```

## Behavior contract

- HTTP: `POST {endpoint}/api/generate` with body
  ```json
  {
    "model": "<config.models.llm.model>",
    "prompt": "<system + '\n\n' + user>",
    "stream": true,
    "options": {
      "temperature": <config.temperature ?? req.temperature ?? 0.0>,
      "seed":        <config.seed ?? req.seed ?? 0>,
      "num_ctx":     <config.context_tokens>,
      "stop":        <req.stop>
    }
  }
  ```
- Response is line-delimited JSON. Each line:
  - `{"response": "...", "done": false}` → emit `TokenChunk::Token(text)`
  - `{"response": "", "done": true, "prompt_eval_count": p, "eval_count": c, "total_duration": ns, ...}` → emit final `TokenChunk::Done { finish_reason: Stop, usage: TokenUsage { prompt_tokens: p, completion_tokens: c, latency_ms: total_duration / 1_000_000 } }`.
- HTTP errors:
  - connection refused → `LlmError::Unreachable`, `anyhow` message includes `hint: ensure 'ollama serve' is running and reachable at <endpoint>`.
  - 404 with `model "<id>" not found` → `LlmError::ModelNotPulled(model_id)`, hint `ollama pull <model_id>`.
  - timeouts → `LlmError::Timeout`.
  - other 4xx/5xx → `LlmError::Stream(body)`.
- UTF-8 boundary: buffer incomplete byte sequences across stream lines before emitting `TokenChunk::Token`.
- Determinism: with `temperature=0` and fixed `seed`, Ollama's output is reproducible (modulo nondeterminism in the model itself); tests that verify determinism use a fixed seed and may rely on aggregate hash with tolerance, NOT byte equality.
- `model_ref().provider = "ollama"`, `dimensions = None`.
- Reachability check: `OllamaLanguageModel::new` does NOT eagerly hit the network; first failure surfaces on `generate_stream`. Use `kb doctor` (separate task) to probe.

## Storage / wire effects

- Reads/writes only the local HTTP socket. No DB or filesystem effects.

## Test plan

| kind | description | fixture / data |
|------|-------------|----------------|
| unit | construction with default config returns expected `ModelRef` | inline |
| unit | streamed line `{"response":"hi","done":false}` followed by `{"done":true,...}` produces 2 chunks then Done | mocked via `wiremock` or `tiny_http` |
| unit | UTF-8 splits across two HTTP chunks reassemble correctly | mocked HTTP |
| unit | unreachable endpoint → `LlmError::Unreachable` with hint | mocked (closed port) |
| unit | 404 missing model → `LlmError::ModelNotPulled` with hint | mocked HTTP |
| unit | concatenation of streamed tokens equals server's full text | mocked HTTP |
| determinism | identical request + temperature=0 + seed=0 produces identical token stream against mock | mocked HTTP |
| `#[ignore]` integration | real Ollama on `localhost:11434` with `qwen2.5:14b-instruct` produces non-empty output | requires user opt-in |

All non-ignored tests under `cargo test -p kb-llm-local`. Real-LM integration runs via `cargo test -p kb-llm-local -- --ignored`.

## Definition of Done

- [ ] `cargo check -p kb-llm-local` passes
- [ ] `cargo test -p kb-llm-local` passes (mocked tests; real LM behind `#[ignore]`)
- [ ] No async runtime present (uses `reqwest::blocking`)
- [ ] No imports outside Allowed dependencies
- [ ] PR links design §11.2, §0 Q5, §10

## Out of scope

- llama.cpp / candle adapters (P+).
- Embedding via Ollama's `/api/embed` endpoint (alternate adapter inside `kb-embed-local` if requested later).
- Cancellation / abort tokens (P+).
- Connection pooling tuning (default `reqwest::blocking` is sufficient for single-user CLI).

## Risks / notes

- Ollama versions sometimes change response field names. Pin a target version range and assert on missing fields with a friendly message.
- `prompt_eval_count` / `eval_count` may be absent on older Ollama; default to `0` and emit a warning span, do NOT fail the stream.
- If Ollama returns a `done` line with `done_reason: "length"`, map to `FinishReason::Length`.
