//! `kb-embed` — thin re-export crate for the [`Embedder`] trait surface.
//!
//! This crate exists so downstream code (`kb-store-vector`, `kb-search`,
//! adapters in p3-2) can `use kebab_embed::Embedder` and stay stable across
//! kb-core reorganizations. It defines **no new types**; everything is a
//! re-export of [`kebab_core`].
//!
//! ## Mock implementation
//!
//! [`MockEmbedder`] (gated behind the `mock` feature, default **OFF**) is a
//! deterministic test double. Real adapters (fastembed, candle, ollama-embed)
//! live in p3-2 and MUST NOT be implemented here.
//!
//! See `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md` §7.1, §7.2,
//! §11 for the contract.

// ── Trait re-exports ──────────────────────────────────────────────────────
//
// Per spec §7.2 — these are the only public-surface types this crate offers.
// Adding new types is forbidden by the task contract.

pub use kebab_core::{Embedder, EmbeddingInput, EmbeddingKind, EmbeddingModelId, EmbeddingVersion};

// ── Test helper ───────────────────────────────────────────────────────────

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

// ── MockEmbedder (feature = "mock") ───────────────────────────────────────

#[cfg(feature = "mock")]
mod mock;

#[cfg(feature = "mock")]
pub use mock::MockEmbedder;
