//! Pins the documented facade-invariant panic in `ask_multi_hop`.
//!
//! When `cfg.rag.nli_threshold > 0` but no verifier is attached via
//! `.with_verifier()`, the `expect` at `pipeline.rs` step 8.5 fires
//! with the message "verifier must be Some when nli_threshold > 0.0".
//!
//! This is a **contract test**: it documents the invariant so that a
//! future refactor replacing the `expect` with `bail!` (or a different
//! message) is caught by the test suite, prompting an explicit decision
//! rather than a silent behavior change.
//!
//! The kebab-app facade (`App::open_with_config`) always pairs
//! `nli_threshold > 0` with a constructed `OnnxNliVerifier`, so this
//! panic is unreachable via the normal CLI / MCP / TUI paths — only
//! a direct `RagPipeline::new(...)` caller without `.with_verifier()`
//! can trigger it.

mod common;

use std::sync::Arc;

use common::{RagEnv, ScriptedLm, ScriptedRetriever, id32, mk_hit};
use kebab_core::{LanguageModel, Retriever, SearchMode};
use kebab_rag::{AskOpts, RagPipeline};

/// Minimal multi-hop `AskOpts` mirroring the pattern used in
/// `multi_hop.rs` — lexical mode, deterministic seed, no streaming.
fn multi_hop_opts() -> AskOpts {
    AskOpts {
        k: 5,
        explain: false,
        mode: SearchMode::Lexical,
        temperature: Some(0.0),
        seed: Some(0),
        stream_sink: None,
        history: Vec::new(),
        conversation_id: None,
        turn_index: None,
        multi_hop: true,
    }
}

/// Building the "happy-path" scenario inline: probe retrieve passes
/// the score gate, decompose emits one sub-query, decide signals stop,
/// and synthesize produces a non-empty cited answer. This is the minimal
/// scenario that reaches step 8.5 (NLI gate) in `ask_multi_hop`.
fn setup_happy_pipeline_no_verifier(nli_threshold: f32) -> (RagPipeline, RagEnv) {
    let env = RagEnv::new();
    let cid = id32("c1");
    let did = id32("d1");
    env.seed_chunk(&cid, &did, "notes/a.md", "Body text.", &["Intro"]);
    let hits = vec![mk_hit(1, &cid, &did, "notes/a.md", 0.85, &["Intro"])];
    // Entry 0 = probe retrieve (pre-decompose gate check).
    // Entry 1 = decompose-driven retrieve for "q1".
    let retriever = Arc::new(ScriptedRetriever::new(vec![hits.clone(), hits]));
    let retriever_dyn: Arc<dyn Retriever> = retriever;

    // Three LLM calls: decompose → decide (stop) → synthesize.
    // Synthesize emits a non-empty answer so step 8.5 is reached.
    let lm = Arc::new(ScriptedLm::new(vec![
        r#"["q1"]"#,   // decompose
        r"[]",         // decide: stop signal
        "answer body [#1]", // synthesize: non-empty → step 8.5 entered
    ]));
    let lm_dyn: Arc<dyn LanguageModel> = lm;

    let mut cfg = env.config.clone();
    cfg.rag.nli_threshold = nli_threshold;

    // Intentionally NO `.with_verifier()` — this is the condition under test.
    let pipeline = RagPipeline::new(cfg, retriever_dyn, lm_dyn, env.sqlite.clone());
    (pipeline, env)
}

#[test]
#[should_panic(expected = "verifier must be Some when nli_threshold > 0.0")]
fn ask_multi_hop_panics_when_threshold_positive_but_verifier_none() {
    // nli_threshold = 0.5 (gate enabled) but the pipeline has no verifier
    // because `.with_verifier()` was never called. The `expect` at
    // pipeline.rs step 8.5 fires once synthesize produces a non-empty answer.
    let (pipeline, _env) = setup_happy_pipeline_no_verifier(0.5);
    // Unwrap is intentional: we're asserting the panic, not an Ok/Err return.
    let _ = pipeline.ask("compound", multi_hop_opts());
}

/// Companion: threshold = 0.0 (gate disabled) with no verifier must
/// NOT panic — the `if nli_threshold > 0.0` guard short-circuits the
/// entire step 8.5 block.
#[test]
fn ask_multi_hop_does_not_panic_when_threshold_zero_and_verifier_none() {
    let (pipeline, _env) = setup_happy_pipeline_no_verifier(0.0);
    let answer = pipeline
        .ask("compound", multi_hop_opts())
        .expect("threshold = 0.0 skips NLI gate; no panic expected");
    // Gate is disabled → verification summary stays None.
    assert!(
        answer.verification.is_none(),
        "nli_threshold = 0.0 must leave Answer.verification = None"
    );
}
