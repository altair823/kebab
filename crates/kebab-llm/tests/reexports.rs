//! Compile-only test: verifies the crate's public surface (trait re-exports
//! and the `assert_finish_chunk` helper) is reachable without the `mock`
//! feature.
//!
//! Runs under both `cargo test -p kb-llm` and
//! `cargo test -p kb-llm --features mock`.

use kebab_llm::{
    FinishReason, GenerateRequest, LanguageModel, ModelRef, TokenChunk, TokenUsage,
    assert_finish_chunk,
};

/// A trivial in-test impl that does NOT rely on the `mock` feature — proves
/// the trait surface alone is enough to write a `LanguageModel`. It returns a
/// stream that terminates immediately with `Done`.
struct ZeroLanguageModel;

impl LanguageModel for ZeroLanguageModel {
    fn model_ref(&self) -> ModelRef {
        ModelRef {
            id: "zero".into(),
            provider: "zero".into(),
            dimensions: None,
        }
    }
    fn context_tokens(&self) -> usize {
        0
    }
    fn generate_stream(
        &self,
        _req: GenerateRequest,
    ) -> anyhow::Result<Box<dyn Iterator<Item = anyhow::Result<TokenChunk>> + Send>> {
        let chunks = vec![TokenChunk::Done {
            finish_reason: FinishReason::Stop,
            usage: TokenUsage {
                prompt_tokens: 0,
                completion_tokens: 0,
                latency_ms: 0,
            },
        }];
        Ok(Box::new(chunks.into_iter().map(Ok)))
    }
}

#[test]
fn dyn_dispatch_via_box_works() {
    let m: Box<dyn LanguageModel> = Box::new(ZeroLanguageModel);
    assert_eq!(m.model_ref().id, "zero");
    assert_eq!(m.context_tokens(), 0);

    let req = GenerateRequest {
        system: "sys".into(),
        user: "usr".into(),
        stop: vec![],
        max_tokens: 16,
        temperature: 0.0,
        seed: None,
    };
    let stream = m.generate_stream(req).expect("stream");
    let chunks: Vec<TokenChunk> = stream.map(|r| r.expect("ok chunk")).collect();
    assert_eq!(chunks.len(), 1);
    assert_finish_chunk(&chunks);
}

/// Sanity: when built WITHOUT `--features mock`, the `MockLanguageModel`
/// symbol is absent. We can't usefully test `nm` from inside a unit test, but
/// we can at least confirm the cfg gate parses both ways. See PR notes for
/// the CI-side `nm`/`cargo bloat` symbol scan.
#[cfg(not(feature = "mock"))]
#[test]
fn mock_feature_off_compiles() {
    // No-op — the test's existence proves the `not(feature = "mock")` gate
    // compiles and the crate is usable without `MockLanguageModel`.
}
