//! Deterministic mock embedder for downstream tests.
//!
//! Compiled only when the `mock` feature is enabled. Default builds
//! (`cargo build --release -p kb-embed`) MUST NOT contain the `MockEmbedder`
//! symbol — verifiable by symbol scan (`nm`, `cargo bloat`).
//!
//! ## Determinism contract
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

use kebab_core::{Embedder, EmbeddingInput, EmbeddingKind, EmbeddingModelId, EmbeddingVersion};

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
    pub fn new(
        model_id: EmbeddingModelId,
        version: EmbeddingVersion,
        dimensions: usize,
    ) -> Self {
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
