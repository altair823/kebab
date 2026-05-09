//! p9-fb-33: pipeline-level streaming behavior — order invariants,
//! cancel propagation, refusal flagging.

mod common;

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc;

use common::{MockRetriever, RagEnv, id32, mk_hit};
use kebab_core::{
    FinishReason, LanguageModel, RefusalReason, Retriever, SearchMode, TokenChunk, TokenUsage,
};
use kebab_llm::MockLanguageModel;
use kebab_rag::{AskOpts, RagPipeline, StreamEvent};

const TEST_LM_ID: &str = "mock-lm";

/// Minimal LM mirroring `tests/pipeline.rs::CountingLm` so the
/// streaming-events suite stays self-contained.
struct CountingLm {
    inner: MockLanguageModel,
    calls: std::sync::atomic::AtomicUsize,
}

impl CountingLm {
    fn new(canned: &str) -> Self {
        Self {
            inner: MockLanguageModel {
                model_id: TEST_LM_ID.to_string(),
                provider: "mock".to_string(),
                context_tokens: 32_768,
                canned_response: canned.to_string(),
                canned_finish: FinishReason::Stop,
                canned_usage: TokenUsage {
                    prompt_tokens: 10,
                    completion_tokens: 5,
                    latency_ms: 7,
                },
            },
            calls: std::sync::atomic::AtomicUsize::new(0),
        }
    }
}

impl LanguageModel for CountingLm {
    fn model_ref(&self) -> kebab_core::ModelRef {
        self.inner.model_ref()
    }
    fn context_tokens(&self) -> usize {
        self.inner.context_tokens()
    }
    fn generate_stream(
        &self,
        req: kebab_core::GenerateRequest,
    ) -> anyhow::Result<Box<dyn Iterator<Item = anyhow::Result<TokenChunk>> + Send>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.generate_stream(req)
    }
}

fn opts_with_sink(tx: mpsc::Sender<StreamEvent>) -> AskOpts {
    AskOpts {
        k: 3,
        explain: false,
        mode: SearchMode::Lexical,
        temperature: Some(0.0),
        seed: Some(0),
        stream_sink: Some(tx),
        history: Vec::new(),
        conversation_id: None,
        turn_index: None,
    }
}

/// Build a pipeline with one seeded chunk + canned LM response so
/// retrieval lands a single hit and the LM emits at least one token.
fn env_with_one_hit(canned: &str) -> (RagEnv, RagPipeline) {
    let env = RagEnv::new();
    let cid = id32("c1");
    let did = id32("d1");
    env.seed_chunk(&cid, &did, "notes/a.md", "apples are red.", &["Intro"]);
    let hits = vec![mk_hit(1, &cid, &did, "notes/a.md", 0.85, &["Intro"])];
    let retriever: Arc<dyn Retriever> = Arc::new(MockRetriever::new(hits));
    let lm: Arc<dyn LanguageModel> = Arc::new(CountingLm::new(canned));
    let pipeline = RagPipeline::new(env.config.clone(), retriever, lm, env.sqlite.clone());
    (env, pipeline)
}

#[test]
fn ask_emits_retrieval_then_tokens_then_final() {
    let (_env, pipeline) = env_with_one_hit("apples are red. [#1]");
    let (tx, rx) = mpsc::channel::<StreamEvent>();
    let _ans = pipeline.ask("apples", opts_with_sink(tx)).unwrap();
    let events: Vec<StreamEvent> = rx.iter().collect();

    // First event must be RetrievalDone.
    assert!(
        matches!(events.first(), Some(StreamEvent::RetrievalDone { .. })),
        "first event must be RetrievalDone, got {:?}",
        events.first()
    );

    // Last event must be Final.
    assert!(
        matches!(events.last(), Some(StreamEvent::Final { .. })),
        "last event must be Final, got {:?}",
        events.last()
    );

    // Everything in between is Token.
    for ev in &events[1..events.len() - 1] {
        assert!(
            matches!(ev, StreamEvent::Token { .. }),
            "middle events must be Token, got {ev:?}"
        );
    }
}

#[test]
fn ask_records_llm_stream_aborted_when_receiver_drops() {
    let (env, pipeline) = env_with_one_hit("apples are red. [#1]");
    let (tx, rx) = mpsc::channel::<StreamEvent>();
    // Drop the receiver immediately so the first Token send fails.
    drop(rx);
    let ans = pipeline.ask("apples", opts_with_sink(tx)).unwrap();
    assert!(!ans.grounded);
    assert_eq!(ans.refusal_reason, Some(RefusalReason::LlmStreamAborted));
    // Persistence still happens on cancel — the row is the audit trail.
    assert_eq!(env.count_answers(), 1, "answers row written on cancel");
}
