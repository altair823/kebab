//! `kb-llm` — thin re-export crate for the [`LanguageModel`] trait surface.
//!
//! This crate exists so downstream code (`kb-rag`, adapters in p4-2) can
//! `use kebab_llm::LanguageModel` and stay stable across kb-core reorganizations.
//! It defines **no new types**; everything is a re-export of [`kebab_core`].
//!
//! ## Mock implementation
//!
//! [`MockLanguageModel`] (gated behind the `mock` feature, default **OFF**) is
//! a deterministic test double. Real adapters (Ollama, llama.cpp, candle) live
//! in p4-2 and MUST NOT be implemented here. Real adapters MAY return `Err`
//! from `generate_stream` itself (e.g., connection refused) before any chunk
//! is yielded; the mock never does.
//!
//! See `docs/superpowers/specs/2026-04-27-kb-final-form-design.md` §7.1, §7.2,
//! §0 Q5 (streaming), §3.8 (`ModelRef`) for the contract.

// ── Trait re-exports ──────────────────────────────────────────────────────
//
// Per spec §7.2 — these are the only public-surface types this crate offers.
// Adding new types is forbidden by the task contract.

pub use kebab_core::{
    FinishReason, GenerateRequest, LanguageModel, ModelRef, TokenChunk, TokenUsage,
};

// ── Test helper ───────────────────────────────────────────────────────────

/// Assert the streamed `TokenChunk` sequence ends with a [`TokenChunk::Done`]
/// frame. Per spec §7.2 / §0 Q5 every stream — even an erroring one — must
/// terminate with a `Done` chunk; this helper centralizes that contract check
/// so downstream test crates don't each rewrite it.
///
/// Panics on mismatch (test-only helper — callers are tests).
pub fn assert_finish_chunk(chunks: &[TokenChunk]) {
    assert!(
        matches!(chunks.last(), Some(TokenChunk::Done { .. })),
        "stream must end with TokenChunk::Done; got {:?}",
        chunks.last(),
    );
}

// ── MockLanguageModel (feature = "mock") ──────────────────────────────────

#[cfg(feature = "mock")]
mod mock;

#[cfg(feature = "mock")]
pub use mock::MockLanguageModel;
