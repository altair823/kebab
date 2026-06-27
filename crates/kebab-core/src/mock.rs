//! Deterministic mock `Embedder` / `LanguageModel` + test helpers.
//!
//! Compiled only when the `mock` feature is enabled. Default builds
//! (`cargo build`, no `--features mock`) MUST NOT contain the `MockEmbedder` /
//! `MockLanguageModel` symbols — verifiable by symbol scan (`nm`/`cargo bloat`).
//!
//! Moved here verbatim from the former `kebab-embed` / `kebab-llm` re-export
//! shim crates (folded into `kebab-core`); those crates defined no new types.
//!
//! # `MockEmbedder` determinism contract
//!
//! For every call to [`MockEmbedder::embed`], component `i` of the output
//! vector for input `(text, kind)` is computed as:
//!
//! ```text
//! h = blake3(seed_le8 || kind_byte || text_len_le8 || text_utf8 || i_le8)
//! raw_i64 = i64::from_le_bytes(h[0..8])
//! comp = (raw_i64 as f64 / i64::MAX as f64) as f32     // ∈ [-1.0, 1.0]
//! ```
//!
//! `kind_byte` is `0u8` for [`EmbeddingKind::Document`] and `1u8` for
//! [`EmbeddingKind::Query`] — mirrors the e5-style prefix behavior (the same
//! text in different roles produces different vectors). `text_len_le8` is the
//! length of `text_utf8` (in bytes) as a little-endian `u64`; it provides
//! domain separation so the boundary between `text` and the trailing `i_le8`
//! cannot be ambiguous (without it, e.g. `("ABCDEFGH", 0)` and
//! `("", u64::from_le_bytes(*b"ABCDEFGH"))` would hash identically).
//!
//! After the per-component pass each vector is **L2-normalized to unit
//! length** so downstream cosine-similarity tests can rely on a unit-norm
//! input (‖v‖ ≈ 1.0 within f32 epsilon × √dims — the per-component f32
//! truncation is bounded by `f32::EPSILON`, summed in quadrature gives
//! roughly `√dims · EPSILON` in the L2 norm). If a vector ends up all-zeros
//! (vanishingly unlikely from BLAKE3), it is left untouched rather than
//! dividing by zero.
//!
//! Invariants the contract guarantees:
//!
//! * Identical `(seed, kind, text, dimensions)` → byte-identical output.
//! * Different `kind` for the same text → different output (kind_byte differs).
//! * Different `text` → different output with overwhelming probability.
//! * All output components are finite (`is_finite()`).
//!
//! # `MockLanguageModel` streaming contract
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
//! No network. No filesystem. No async runtime. No tokenizer — `usage` fields
//! are whatever the constructor was given.

use crate::{
    Embedder, EmbeddingInput, EmbeddingKind, EmbeddingModelId, EmbeddingVersion, FinishReason,
    GenerateRequest, LanguageModel, ModelRef, TokenChunk, TokenUsage,
};

// ── Embed test helpers ────────────────────────────────────────────────────

/// Assert every vector has length `expected_dims` and contains only finite
/// floats. Intended for downstream test crates so they don't each rewrite the
/// shape check.
///
/// Panics on mismatch (test-only helper — callers are tests).
pub fn assert_vector_shape(vecs: &[Vec<f32>], expected_dims: usize) {
    for (i, v) in vecs.iter().enumerate() {
        assert_eq!(
            v.len(),
            expected_dims,
            "vector {i}: dims {} != expected {expected_dims}",
            v.len(),
        );
        for (j, x) in v.iter().enumerate() {
            assert!(x.is_finite(), "vector {i}[{j}] = {x} is not finite");
        }
    }
}

/// Assert every vector has L2 norm within `tolerance` of `1.0`.
///
/// L2 norm is computed in `f64` (per-component square accumulation in `f64`
/// then `sqrt`) before truncating back to `f32`, so the comparison is not
/// dominated by accumulation error in the check itself — only the f32
/// truncation of the input vector's components contributes.
///
/// Tolerance guidance: callers pass their own. For `dims = 384` and
/// f32-truncated unit vectors, `5e-4` is a safe upper bound under quadratic
/// accumulation of per-component f32 truncation (`f32::EPSILON × √dims`).
/// Smaller dims tolerate tighter bounds; larger dims need looser ones.
///
/// Panics on mismatch (test-only helper — callers are tests).
pub fn assert_unit_norm(vecs: &[Vec<f32>], tolerance: f32) {
    for (i, v) in vecs.iter().enumerate() {
        let norm_sq: f64 = v.iter().map(|&x| f64::from(x) * f64::from(x)).sum();
        let norm = norm_sq.sqrt() as f32;
        assert!(
            (norm - 1.0).abs() <= tolerance,
            "vector {i}: ‖v‖ = {norm} (off from 1.0 by {})",
            (norm - 1.0).abs(),
        );
    }
}

// ── LLM test helper ───────────────────────────────────────────────────────

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

// ── MockEmbedder ──────────────────────────────────────────────────────────

/// Deterministic test double. See module docs for the hashing recipe.
pub struct MockEmbedder {
    model_id: EmbeddingModelId,
    version: EmbeddingVersion,
    dimensions: usize,
    seed: u64,
}

impl MockEmbedder {
    /// Construct with `seed = 0`. Use [`Self::with_seed`] to pick a different
    /// seed (e.g., to verify two embedders with the same identity but
    /// different seeds yield different vectors).
    pub fn new(model_id: EmbeddingModelId, version: EmbeddingVersion, dimensions: usize) -> Self {
        Self {
            model_id,
            version,
            dimensions,
            seed: 0,
        }
    }

    /// Construct with an explicit seed. Useful for differential tests.
    pub fn with_seed(
        model_id: EmbeddingModelId,
        version: EmbeddingVersion,
        dimensions: usize,
        seed: u64,
    ) -> Self {
        Self {
            model_id,
            version,
            dimensions,
            seed,
        }
    }

    fn kind_byte(kind: EmbeddingKind) -> u8 {
        match kind {
            EmbeddingKind::Document => 0,
            EmbeddingKind::Query => 1,
        }
    }

    fn component(&self, kind: EmbeddingKind, text: &str, i: usize) -> f32 {
        let mut hasher = blake3::Hasher::new();
        hasher.update(&self.seed.to_le_bytes());
        hasher.update(&[Self::kind_byte(kind)]);
        // Length-prefix `text` (LE u64) so the boundary between `text` and the
        // trailing `i` field is unambiguous — without this, `("ABCDEFGH", 0)`
        // and `("", u64::from_le_bytes(*b"ABCDEFGH"))` would feed identical
        // bytes into the hasher.
        hasher.update(&(text.len() as u64).to_le_bytes());
        hasher.update(text.as_bytes());
        hasher.update(&(i as u64).to_le_bytes());
        let digest = hasher.finalize();
        let bytes = digest.as_bytes();
        let mut head = [0u8; 8];
        head.copy_from_slice(&bytes[..8]);
        let raw = i64::from_le_bytes(head);
        // Map to [-1.0, 1.0]. `i64::MAX` is finite in f64 so the ratio is
        // always finite. Casting back to f32 cannot produce a NaN/Inf for
        // values in this range.
        // Note: i64::MIN/i64::MAX gives -1.0000000000000002 → f32 cast rounds to -1.0; range [-1, 1] holds in f32 even with this asymmetry.
        ((raw as f64) / (i64::MAX as f64)) as f32
    }
}

impl Embedder for MockEmbedder {
    fn model_id(&self) -> EmbeddingModelId {
        self.model_id.clone()
    }

    fn model_version(&self) -> EmbeddingVersion {
        self.version.clone()
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }

    fn embed(&self, inputs: &[EmbeddingInput<'_>]) -> anyhow::Result<Vec<Vec<f32>>> {
        let mut out = Vec::with_capacity(inputs.len());
        for input in inputs {
            let mut v: Vec<f32> = (0..self.dimensions)
                .map(|i| self.component(input.kind, input.text, i))
                .collect();

            // L2-normalize. Skip the rare all-zero case to avoid 0/0 = NaN.
            let norm_sq: f64 = v.iter().map(|&x| f64::from(x) * f64::from(x)).sum();
            if norm_sq > 0.0 {
                let inv = (1.0 / norm_sq.sqrt()) as f32;
                for x in &mut v {
                    *x *= inv;
                }
            }
            out.push(v);
        }
        Ok(out)
    }
}

// ── MockLanguageModel ─────────────────────────────────────────────────────

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
