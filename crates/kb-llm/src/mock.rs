//! Deterministic mock language model for downstream tests.
//!
//! Compiled only when the `mock` feature is enabled. Default builds
//! (`cargo build --release -p kb-llm`) MUST NOT contain the `MockLanguageModel`
//! symbol — verifiable by symbol scan (`nm`/`cargo bloat`).
//!
//! ## Streaming contract
//!
//! For every call to [`MockLanguageModel::generate_stream`]:
//!
//! 1. The configured `canned_response` is examined for any of `req.stop`. If
//!    one or more stop strings are substrings of the response, the response
//!    is truncated at the **earliest byte position** of any match (i.e., the
//!    first stop string to land — ties broken by the order entries appear in
//!    `req.stop`, since `Iterator::min` returns the first equal element on
//!    ties, breaking by `req.stop` declaration order).
//! 2. The (possibly truncated) string is iterated by Unicode scalar
//!    (`str::chars()`) and each character is yielded as
//!    [`TokenChunk::Token`]`(c.to_string())`. This makes streaming UTF-8 safe
//!    by construction (no character is split across chunks). Emits one
//!    `TokenChunk` per Unicode scalar value (`char`), not per grapheme
//!    cluster — Hangul jamo, emoji ZWJ sequences, and combining marks split
//!    into multiple chunks. Acceptable for trait-shape testing; real adapters
//!    MAY combine.
//! 3. After all tokens, a single terminal [`TokenChunk::Done`] is yielded
//!    with:
//!     * `finish_reason = FinishReason::Stop` if a stop string truncated the
//!       canned text — mirroring real LLM behavior, which reports Stop on
//!       stop-sequence termination regardless of the configured finish.
//!     * `finish_reason = canned_finish.clone()` otherwise.
//!     * `usage = canned_usage.clone()` always.
//!
//! ## Non-effects
//!
//! - No network. No filesystem. No async runtime.
//! - No tokenizer. `usage.prompt_tokens` / `completion_tokens` are whatever
//!   the constructor was given — the mock does not count.

use kb_core::{
    FinishReason, GenerateRequest, LanguageModel, ModelRef, TokenChunk, TokenUsage,
};

/// Deterministic test double. See module docs for the streaming recipe.
pub struct MockLanguageModel {
    pub model_id: String,
    pub provider: String,
    pub context_tokens: usize,
    pub canned_response: String,
    pub canned_finish: FinishReason,
    pub canned_usage: TokenUsage,
}

impl MockLanguageModel {
    /// Apply `req.stop` to `canned_response`. Returns `(truncated_text,
    /// stop_hit)` where `stop_hit` is true iff any stop string was found.
    fn apply_stop<'a>(canned: &'a str, stop: &[String]) -> (&'a str, bool) {
        // Earliest byte position wins. Ties break by first occurrence in
        // `stop` (Iterator::min returns the first equal element, and we
        // iterate `stop` in its declared order). Empty stop strings are
        // ignored — they would otherwise match at position 0 and silently
        // eat the entire response.
        let earliest = stop
            .iter()
            .filter(|s| !s.is_empty())
            .filter_map(|s| canned.find(s.as_str()))
            .min();
        match earliest {
            // `str::find` returns a UTF-8 char boundary by contract, so direct byte-slice is sound.
            Some(idx) => (&canned[..idx], true),
            None => (canned, false),
        }
    }
}

impl LanguageModel for MockLanguageModel {
    fn model_ref(&self) -> ModelRef {
        ModelRef {
            id: self.model_id.clone(),
            provider: self.provider.clone(),
            // Per §3.8: `dimensions` carries the embedder's output dim and is
            // intentionally None for chat models.
            dimensions: None,
        }
    }

    fn context_tokens(&self) -> usize {
        self.context_tokens
    }

    fn generate_stream(
        &self,
        req: GenerateRequest,
    ) -> anyhow::Result<Box<dyn Iterator<Item = anyhow::Result<TokenChunk>> + Send>> {
        let (truncated, stop_hit) = Self::apply_stop(&self.canned_response, &req.stop);

        // Pre-materialize the full chunk sequence into an owned Vec. This
        // sidesteps lifetime juggling around `&self.canned_response` inside
        // a `'static` iterator and trivially gives `Send` (Vec<TokenChunk>
        // is Send because TokenChunk is Send).
        let mut chunks: Vec<TokenChunk> = truncated
            .chars()
            .map(|c| TokenChunk::Token(c.to_string()))
            .collect();

        let finish_reason = if stop_hit {
            FinishReason::Stop
        } else {
            self.canned_finish.clone()
        };
        chunks.push(TokenChunk::Done {
            finish_reason,
            usage: self.canned_usage.clone(),
        });

        Ok(Box::new(chunks.into_iter().map(Ok)))
    }
}
