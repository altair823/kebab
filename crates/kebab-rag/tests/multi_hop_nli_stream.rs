//! Tests that the NLI refusal paths emit a `StreamEvent::Final` event
//! into `opts.stream_sink`.
//!
//! Coverage:
//! 1. `nli_verification_fail_emits_final_stream_event_with_refusal` —
//!    `MockNliVerifier::fail()` → `RefusalReason::NliVerificationFailed`
//!    arrives as the payload of the terminal `StreamEvent::Final`.
//! 2. `nli_model_unavailable_emits_final_stream_event_with_refusal` —
//!    `MockNliVerifier::err()` → `RefusalReason::NliModelUnavailable`
//!    arrives as the payload of the terminal `StreamEvent::Final`.
//!
//! Note: `ask_multi_hop` does NOT have a separate `_streaming` entrypoint.
//! Streaming is handled by the `stream_sink: Option<Sender<StreamEvent>>`
//! field on `AskOpts`. Both refusal helpers (`refuse_nli_verification` and
//! `refuse_nli_model_unavailable`) fire `sink.send(StreamEvent::Final { … })`
//! before returning — these tests pin that wire shape.

mod common;

use std::sync::Arc;
use std::sync::mpsc;

use common::{MockNliVerifier, RagEnv, ScriptedLm, ScriptedRetriever, id32, mk_hit};
use kebab_core::{LanguageModel, RefusalReason, Retriever, SearchMode};
use kebab_nli::NliVerifier;
use kebab_rag::{AskOpts, RagPipeline, StreamEvent};

// ── shared helpers ─────────────────────────────────────────────────────────

/// Build the minimal happy-path scenario that reaches step 8.5 (NLI gate):
/// probe passes, decompose → one sub-query, decide → stop, synthesize →
/// non-empty answer.  Returns the env, scripted retriever, and scripted LM.
fn happy_env_for_stream() -> (RagEnv, Arc<ScriptedRetriever>, Arc<ScriptedLm>) {
    let env = RagEnv::new();
    let cid = id32("c1");
    let did = id32("d1");
    env.seed_chunk(&cid, &did, "notes/a.md", "Body text.", &["Intro"]);
    let hits = vec![mk_hit(1, &cid, &did, "notes/a.md", 0.85, &["Intro"])];
    // Entry 0 = probe, entry 1 = decompose-driven retrieve.
    let retriever = Arc::new(ScriptedRetriever::new(vec![hits.clone(), hits]));
    let lm = Arc::new(ScriptedLm::new(vec![
        r#"["q1"]"#,        // decompose
        r"[]",              // decide: stop
        "answer body [#1]", // synthesize: non-empty so NLI gate runs
    ]));
    (env, retriever, lm)
}

/// Multi-hop `AskOpts` with a `stream_sink` wired in so every pipeline
/// stage emits `StreamEvent`s into `tx`.
fn multi_hop_opts_with_sink(tx: mpsc::Sender<StreamEvent>) -> AskOpts {
    AskOpts {
        k: 5,
        explain: false,
        mode: SearchMode::Lexical,
        temperature: Some(0.0),
        seed: Some(0),
        stream_sink: Some(tx),
        history: Vec::new(),
        conversation_id: None,
        turn_index: None,
        multi_hop: true,
    }
}

/// Drain `rx` and return the first `StreamEvent::Final` found, panicking
/// with a clear message if none is present.
fn expect_final_event(rx: mpsc::Receiver<StreamEvent>) -> StreamEvent {
    let events: Vec<StreamEvent> = rx.try_iter().collect();
    events
        .into_iter()
        .find(|e| matches!(e, StreamEvent::Final { .. }))
        .expect("pipeline must emit at least one StreamEvent::Final")
}

// ── 1. NliVerificationFailed ───────────────────────────────────────────────

#[test]
fn nli_verification_fail_emits_final_stream_event_with_refusal() {
    let (env, retriever, lm) = happy_env_for_stream();
    let mut cfg = env.config.clone();
    cfg.rag.nli_threshold = 0.5; // entailment 0.1 < 0.5 → refusal

    let retriever_dyn: Arc<dyn Retriever> = retriever;
    let lm_dyn: Arc<dyn LanguageModel> = lm;
    let verifier = MockNliVerifier::fail(); // entailment score = 0.1
    let verifier_dyn: Arc<dyn NliVerifier> = verifier;

    let (tx, rx) = mpsc::channel::<StreamEvent>();
    let pipeline = RagPipeline::new(cfg, retriever_dyn, lm_dyn, env.sqlite.clone())
        .with_verifier(verifier_dyn);

    let answer = pipeline
        .ask("compound", multi_hop_opts_with_sink(tx))
        .expect("pipeline returns Ok even on NLI refusal");

    // Synchronous return value.
    assert_eq!(
        answer.refusal_reason,
        Some(RefusalReason::NliVerificationFailed),
        "return value must carry NliVerificationFailed"
    );
    assert!(!answer.grounded, "NLI refusal must not be grounded");

    // Stream wire shape: terminal Final event must carry matching refusal.
    let final_event = expect_final_event(rx);
    match final_event {
        StreamEvent::Final { answer: streamed } => {
            assert_eq!(
                streamed.refusal_reason,
                Some(RefusalReason::NliVerificationFailed),
                "Final event's answer must carry NliVerificationFailed"
            );
            assert!(!streamed.grounded);
            // verification summary is stamped even on the refusal path.
            let v = streamed
                .verification
                .expect("NliVerificationFailed carries a VerificationSummary");
            assert!(!v.nli_passed);
            assert!((v.nli_score - 0.1).abs() < 1e-5, "score: {}", v.nli_score);
        }
        other => panic!("expected StreamEvent::Final, got {other:?}"),
    }
}

// ── 2. NliModelUnavailable ─────────────────────────────────────────────────

#[test]
fn nli_model_unavailable_emits_final_stream_event_with_refusal() {
    let (env, retriever, lm) = happy_env_for_stream();
    let mut cfg = env.config.clone();
    cfg.rag.nli_threshold = 0.5; // gate enabled; verifier will error

    let retriever_dyn: Arc<dyn Retriever> = retriever;
    let lm_dyn: Arc<dyn LanguageModel> = lm;
    let verifier = MockNliVerifier::err(); // returns anyhow::Error
    let verifier_dyn: Arc<dyn NliVerifier> = verifier;

    let (tx, rx) = mpsc::channel::<StreamEvent>();
    let pipeline = RagPipeline::new(cfg, retriever_dyn, lm_dyn, env.sqlite.clone())
        .with_verifier(verifier_dyn);

    let answer = pipeline
        .ask("compound", multi_hop_opts_with_sink(tx))
        .expect("pipeline returns Ok even when NLI model is unavailable");

    // Synchronous return value.
    assert_eq!(
        answer.refusal_reason,
        Some(RefusalReason::NliModelUnavailable),
        "return value must carry NliModelUnavailable"
    );
    assert!(!answer.grounded);
    // verification is None — we can't summarize what didn't happen.
    assert!(
        answer.verification.is_none(),
        "NliModelUnavailable must leave Answer.verification = None"
    );

    // Stream wire shape: terminal Final event must carry matching refusal.
    let final_event = expect_final_event(rx);
    match final_event {
        StreamEvent::Final { answer: streamed } => {
            assert_eq!(
                streamed.refusal_reason,
                Some(RefusalReason::NliModelUnavailable),
                "Final event's answer must carry NliModelUnavailable"
            );
            assert!(!streamed.grounded);
            assert!(
                streamed.verification.is_none(),
                "NliModelUnavailable: verification must be None in the streamed Final event"
            );
        }
        other => panic!("expected StreamEvent::Final, got {other:?}"),
    }
}
