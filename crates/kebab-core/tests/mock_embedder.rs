//! Integration tests for `MockEmbedder`. Gated behind the `mock` feature.
//!
//! Canonical invocation: `cargo test -p kebab-core --features mock`.
//! (Without `--features mock` this file compiles to nothing — the `cfg` gate
//! below short-circuits, since the mock lives in `kebab-core`'s own optional
//! `mock` module and cannot be enabled via a self dev-dependency.)

#![cfg(feature = "mock")]

use kebab_core::{
    Embedder, EmbeddingInput, EmbeddingKind, EmbeddingModelId, EmbeddingVersion, MockEmbedder,
    assert_unit_norm, assert_vector_shape,
};
use proptest::prelude::*;

fn mk(dims: usize) -> MockEmbedder {
    MockEmbedder::new(
        EmbeddingModelId("mock-test".into()),
        EmbeddingVersion("0".into()),
        dims,
    )
}

#[test]
fn dyn_dispatch_through_box() {
    let e: Box<dyn Embedder> = Box::new(mk(8));
    assert_eq!(e.dimensions(), 8);
    assert_eq!(e.model_id(), EmbeddingModelId("mock-test".into()));
    assert_eq!(e.model_version(), EmbeddingVersion("0".into()));

    let inputs = [EmbeddingInput {
        text: "a fox",
        kind: EmbeddingKind::Document,
    }];
    let v = e.embed(&inputs).expect("embed via box");
    assert_eq!(v.len(), 1);
    assert_vector_shape(&v, 8);
}

#[test]
fn identical_input_yields_byte_identical_vector() {
    let e = mk(16);
    let a = e
        .embed(&[EmbeddingInput {
            text: "the quick brown fox",
            kind: EmbeddingKind::Document,
        }])
        .unwrap();
    let b = e
        .embed(&[EmbeddingInput {
            text: "the quick brown fox",
            kind: EmbeddingKind::Document,
        }])
        .unwrap();
    // Vec<Vec<f32>> equality is byte-equal because we did not mutate
    // either side and the hash + normalization path is pure.
    assert_eq!(a, b);
}

#[test]
fn document_and_query_kinds_differ_for_same_text() {
    let e = mk(32);
    let inputs = [
        EmbeddingInput {
            text: "needle in haystack",
            kind: EmbeddingKind::Document,
        },
        EmbeddingInput {
            text: "needle in haystack",
            kind: EmbeddingKind::Query,
        },
    ];
    let v = e.embed(&inputs).unwrap();
    assert_eq!(v.len(), 2);
    assert_vector_shape(&v, 32);
    assert_ne!(
        v[0], v[1],
        "Document and Query kinds must produce different vectors for identical text"
    );
}

#[test]
fn dimensions_match_construction() {
    for dims in [1usize, 4, 64, 384, 768, 1024] {
        let e = mk(dims);
        assert_eq!(e.dimensions(), dims);
        let v = e
            .embed(&[EmbeddingInput {
                text: "x",
                kind: EmbeddingKind::Document,
            }])
            .unwrap();
        assert_vector_shape(&v, dims);
    }
}

#[test]
fn different_seeds_produce_different_vectors() {
    let a = MockEmbedder::with_seed(
        EmbeddingModelId("m".into()),
        EmbeddingVersion("0".into()),
        16,
        0,
    );
    let b = MockEmbedder::with_seed(
        EmbeddingModelId("m".into()),
        EmbeddingVersion("0".into()),
        16,
        1,
    );
    let inputs = [EmbeddingInput {
        text: "same input",
        kind: EmbeddingKind::Document,
    }];
    assert_ne!(a.embed(&inputs).unwrap(), b.embed(&inputs).unwrap());
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 100,
        ..ProptestConfig::default()
    })]

    /// 100 random `(text, kind)` pairs: every output vector must have
    /// `len == dimensions`, contain only finite floats, contain no NaNs,
    /// be L2 unit-norm within tolerance, be re-deterministic across calls,
    /// differ between Document/Query kinds, and differ between distinct texts.
    #[test]
    fn random_inputs_yield_well_formed_vectors(
        text in ".{0,256}",
        text2 in ".{0,256}",
        is_query in any::<bool>(),
        // dims ≥ 2: a 1-dim unit-norm vector has only two possible values
        // (`[1.0]` or `[-1.0]`), which makes the kind/text differential
        // assertions degenerate. Pick a floor of 2 so the differentials
        // exercise non-degenerate vector space.
        dims in 2usize..=128,
    ) {
        // Skip degenerate case where the two random texts collide; the
        // "distinct text → distinct vector" assertion below requires them to
        // differ.
        prop_assume!(text != text2);

        let e = mk(dims);
        let kind = if is_query { EmbeddingKind::Query } else { EmbeddingKind::Document };
        let v = e.embed(&[EmbeddingInput { text: &text, kind }]).unwrap();
        prop_assert_eq!(v.len(), 1);
        prop_assert_eq!(v[0].len(), dims);
        for x in &v[0] {
            prop_assert!(x.is_finite(), "component {x} not finite");
            prop_assert!(!x.is_nan(), "component {x} is NaN");
        }

        // L2 unit-norm within tolerance. `5e-4` is a safe upper bound up to
        // dims = 128 here (would-be floor: f32::EPSILON × √dims).
        assert_unit_norm(&v, 5e-4);

        // Re-determinism: embedding `text` as Document twice → byte-equal.
        let doc_a = e
            .embed(&[EmbeddingInput { text: &text, kind: EmbeddingKind::Document }])
            .unwrap();
        let doc_b = e
            .embed(&[EmbeddingInput { text: &text, kind: EmbeddingKind::Document }])
            .unwrap();
        prop_assert_eq!(&doc_a, &doc_b, "Doc(text) must be byte-equal across calls");

        // Kind differential: Doc(text) != Query(text).
        let q = e
            .embed(&[EmbeddingInput { text: &text, kind: EmbeddingKind::Query }])
            .unwrap();
        prop_assert_ne!(&doc_a, &q, "Doc(text) must differ from Query(text)");

        // Text differential: Doc(text) != Doc(text2) when text != text2.
        let doc_other = e
            .embed(&[EmbeddingInput { text: &text2, kind: EmbeddingKind::Document }])
            .unwrap();
        prop_assert_ne!(&doc_a, &doc_other, "distinct texts must yield distinct Doc vectors");
    }
}
