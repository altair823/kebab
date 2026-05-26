//! Vector / Hybrid lane — AVX-gated. Marked `#[ignore]` because Lance
//! crashes with `SIGILL` on hosts without AVX, and CI lanes that are
//! AVX-less should not run these. Local hosts run them via
//! `cargo test -p kb-app -- --ignored`.

mod common;

use common::TestEnv;

/// Panic if the host CPU lacks AVX. Mirrors the helper in
/// `kb-store-vector/tests/common/mod.rs` and `kb-search` so a
/// `--ignored` invocation on a non-AVX host fails loudly with a
/// clear message instead of crashing inside Lance's SIMD kernel.
fn require_avx_or_panic() {
    #[cfg(target_arch = "x86_64")]
    {
        assert!(std::is_x86_feature_detected!("avx"), 
            "kb-app vector integration test requires AVX-capable hardware; \
             host CPU lacks AVX. Run on an AVX-capable machine."
        );
    }
}

// First run downloads ~470MB; expect ~30-60s warm, several minutes cold.
#[test]
#[ignore = "AVX-required (Lance SIMD kernels)"]
fn ingest_then_hybrid_search_returns_hits() {
    require_avx_or_panic();

    let env = TestEnv::with_embeddings();
    let report =
        kebab_app::ingest_with_config(env.config.clone(), env.scope(), true).unwrap();
    assert_eq!(report.errors, 0, "no per-file errors: {report:?}");
    assert_eq!(report.new, 3);

    let q = kebab_core::SearchQuery {
        text: "ownership".to_string(),
        mode: kebab_core::SearchMode::Hybrid,
        k: 10,
        filters: kebab_core::SearchFilters::default(),
    };
    let hits = kebab_app::search_with_config(env.config.clone(), q).unwrap();
    assert!(!hits.is_empty(), "expected hybrid hits for 'ownership'");
    let methods: Vec<_> = hits.iter().map(|h| h.retrieval.method).collect();
    assert!(
        methods.iter().all(|m| *m == kebab_core::SearchMode::Hybrid),
        "every hit must report method=Hybrid: {methods:?}"
    );
}

// First run downloads ~470MB; expect ~30-60s warm, several minutes cold.
#[test]
#[ignore = "AVX-required (Lance SIMD kernels)"]
fn ingest_then_vector_search_carries_embedding_model() {
    require_avx_or_panic();

    let env = TestEnv::with_embeddings();
    let report =
        kebab_app::ingest_with_config(env.config.clone(), env.scope(), true).unwrap();
    assert_eq!(report.errors, 0, "no per-file errors: {report:?}");
    assert_eq!(report.new, 3);

    let q = kebab_core::SearchQuery {
        text: "ownership".to_string(),
        mode: kebab_core::SearchMode::Vector,
        k: 10,
        filters: kebab_core::SearchFilters::default(),
    };
    let hits = kebab_app::search_with_config(env.config.clone(), q).unwrap();
    assert!(!hits.is_empty(), "expected vector hits for 'ownership'");

    // Vector mode dispatches through `VectorRetriever` and MUST stamp
    // each hit with the configured embedding_model id.
    let expected = kebab_core::EmbeddingModelId(env.config.models.embedding.model.clone());
    for h in &hits {
        assert_eq!(
            h.embedding_model,
            Some(expected.clone()),
            "vector-mode hit must carry embedding_model={expected:?}: {h:?}"
        );
        assert_eq!(
            h.retrieval.method,
            kebab_core::SearchMode::Vector,
            "vector-mode hit must report method=Vector"
        );
    }
}
