//! Compile-only test: verifies the crate's public surface (trait re-exports
//! and the `assert_vector_shape` helper) is reachable without the `mock`
//! feature.
//!
//! Runs under both `cargo test -p kb-embed` and
//! `cargo test -p kb-embed --features mock`.

use kb_embed::{
    Embedder, EmbeddingInput, EmbeddingKind, EmbeddingModelId, EmbeddingVersion,
    assert_vector_shape,
};

/// A trivial in-test impl that does NOT rely on the `mock` feature — proves
/// the trait surface alone is enough to write an `Embedder`.
struct ZeroEmbedder {
    dims: usize,
}

impl Embedder for ZeroEmbedder {
    fn model_id(&self) -> EmbeddingModelId {
        EmbeddingModelId("zero".into())
    }
    fn model_version(&self) -> EmbeddingVersion {
        EmbeddingVersion("0".into())
    }
    fn dimensions(&self) -> usize {
        self.dims
    }
    fn embed(&self, inputs: &[EmbeddingInput<'_>]) -> anyhow::Result<Vec<Vec<f32>>> {
        Ok(inputs.iter().map(|_| vec![0.0; self.dims]).collect())
    }
}

#[test]
fn reexports_compile_without_mock_feature() {
    let e: Box<dyn Embedder> = Box::new(ZeroEmbedder { dims: 4 });
    let inputs = [
        EmbeddingInput {
            text: "hello",
            kind: EmbeddingKind::Document,
        },
        EmbeddingInput {
            text: "world",
            kind: EmbeddingKind::Query,
        },
    ];
    let v = e.embed(&inputs).expect("zero embed");
    assert_eq!(v.len(), 2);
    assert_vector_shape(&v, 4);
}

/// Sanity: when built WITHOUT `--features mock`, the `MockEmbedder` symbol
/// is absent. We can't usefully test `nm` from inside a unit test, but we
/// can at least confirm the cfg gate parses both ways. See PR notes for the
/// CI-side `nm`/`cargo bloat` symbol scan.
#[cfg(not(feature = "mock"))]
#[test]
fn mock_feature_off_compiles() {
    // No-op — the test's existence proves the `not(feature = "mock")` gate
    // compiles and the crate is usable without `MockEmbedder`.
}
