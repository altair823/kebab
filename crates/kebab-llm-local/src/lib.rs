//! `kb-llm-local` — Ollama HTTP adapter implementing
//! [`kebab_core::LanguageModel`] over the local `POST /api/generate` endpoint.
//!
//! ## Why a separate crate
//!
//! `kebab-core` exposes the [`LanguageModel`] trait + a feature-gated
//! `MockLanguageModel` for downstream tests. Real adapters (Ollama, llama.cpp,
//! candle) live outside `kebab-core` so swapping providers stays config-only
//! and so the core crate stays free of heavy adapter dependencies. p4-2
//! ("first real LM") is the home of [`OllamaLanguageModel`] and the
//! [`LlmError`] enum the rest of the workspace will pattern-match against.
//!
//! ## Runtime contract
//!
//! - **Synchronous surface.** Built on `reqwest::blocking`. This crate's
//!   source contains zero `async`/`await`/`tokio::*` symbols and exposes
//!   no async surface to callers.
//!
//!   Note on tokio: reqwest 0.12's `blocking` feature internally wraps a
//!   private current-thread tokio runtime, so
//!   `cargo tree -p kb-llm-local --edges normal | grep tokio` WILL show
//!   tokio in the runtime graph. The auditable invariant is "no top-level
//!   tokio dep + no async surface exposed to callers" rather than "tokio
//!   absent from the tree".
//! - **Streaming.** The adapter posts `stream: true` and returns a
//!   `Box<dyn Iterator<Item = Result<TokenChunk>> + Send>` that reads
//!   line-delimited JSON frames lazily — tokens reach the caller as the
//!   server emits them.
//! - **Lazy connect.** [`OllamaLanguageModel::new`] does not hit the network;
//!   the first error surfaces on [`LanguageModel::generate_stream`].
//!
//! See `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md` §7.2,
//! §6.4 (`[models.llm]`), §0 Q5 (streaming), §10 (errors), and report §11.2
//! (Ollama protocol notes).

mod error;
mod ollama;

pub use error::LlmError;
pub use ollama::OllamaLanguageModel;

// Re-export the trait surface so adapter consumers can `use kebab_llm_local::*`
// without also depending on `kb-core` directly. This crate adds **no new
// types** to the trait surface (`LlmError` and `OllamaLanguageModel` are
// implementation-side only).
pub use kebab_core::{
    FinishReason, GenerateRequest, LanguageModel, ModelRef, TokenChunk, TokenUsage,
};
