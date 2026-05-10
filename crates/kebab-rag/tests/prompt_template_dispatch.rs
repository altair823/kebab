//! p9-fb-40: integration tests for rag-v1 / rag-v2 / unknown-version dispatch.
//!
//! Wraps `MockLanguageModel` in a `CapturingLm` that snapshots
//! `GenerateRequest::system` on every `generate_stream` call so the
//! tests can assert which template constant the pipeline rendered.

mod common;

use std::sync::{Arc, Mutex};

use common::{MockRetriever, RagEnv, id32, mk_hit};
use kebab_core::{FinishReason, LanguageModel, Retriever, SearchMode, TokenChunk, TokenUsage};
use kebab_llm::MockLanguageModel;
use kebab_rag::{AskOpts, RagPipeline};

const TEST_LM_ID: &str = "mock-lm";

/// LM wrapper that captures the system prompt of the most-recent
/// `generate_stream` call, so tests can assert which template was
/// rendered. Mirrors the `CountingLm` pattern from
/// `tests/streaming_events.rs` but stores `req.system` instead of a
/// call counter.
struct CapturingLm {
    inner: MockLanguageModel,
    captured_system: Arc<Mutex<Option<String>>>,
}

impl CapturingLm {
    fn new(captured: Arc<Mutex<Option<String>>>) -> Self {
        Self {
            inner: MockLanguageModel {
                model_id: TEST_LM_ID.to_string(),
                provider: "mock".to_string(),
                context_tokens: 32_768,
                canned_response: "근거가 충분합니다 [#1]".to_string(),
                canned_finish: FinishReason::Stop,
                canned_usage: TokenUsage {
                    prompt_tokens: 10,
                    completion_tokens: 5,
                    latency_ms: 7,
                },
            },
            captured_system: captured,
        }
    }
}

impl LanguageModel for CapturingLm {
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
        *self.captured_system.lock().unwrap() = Some(req.system.clone());
        self.inner.generate_stream(req)
    }
}

/// Mirror of `streaming_events::opts_with_sink` minus the sink — every
/// field is set explicitly because `AskOpts` does not implement `Default`.
fn lexical_opts() -> AskOpts {
    AskOpts {
        k: 3,
        explain: false,
        mode: SearchMode::Lexical,
        temperature: Some(0.0),
        seed: Some(0),
        stream_sink: None,
        history: Vec::new(),
        conversation_id: None,
        turn_index: None,
    }
}

/// Build a `RagPipeline` with the given `prompt_template_version`.
/// Returns the pipeline, the captured-system handle, and the env (kept
/// alive for the test body — drops the SqliteStore + tempdir together).
fn build_pipeline_with_template(
    version: &str,
) -> (RagPipeline, Arc<Mutex<Option<String>>>, RagEnv) {
    let mut env = RagEnv::new();
    env.config.rag.prompt_template_version = version.to_string();
    // Drop score gate so the seeded hit (fusion_score = 0.9) always
    // makes it through — the dispatch we want to exercise lives past
    // the gate.
    env.config.rag.score_gate = 0.0;
    let captured = Arc::new(Mutex::new(None));
    let lm: Arc<dyn LanguageModel> = Arc::new(CapturingLm::new(captured.clone()));
    // Seed one chunk so the [근거] block has content and the LM is
    // actually invoked on the success path.
    let chunk_id = id32("c");
    let doc_id = id32("d");
    env.seed_chunk(&chunk_id, &doc_id, "a.md", "hello world", &["H"]);
    let hit = mk_hit(1, &chunk_id, &doc_id, "a.md", 0.9, &["H"]);
    let retriever: Arc<dyn Retriever> = Arc::new(MockRetriever::new(vec![hit]));
    let pipeline = RagPipeline::new(env.config.clone(), retriever, lm, env.sqlite.clone());
    (pipeline, captured, env)
}

#[test]
fn ask_with_rag_v1_uses_v1_system_prompt() {
    let (pipeline, captured, _env) = build_pipeline_with_template("rag-v1");
    let _ = pipeline.ask("hello", lexical_opts());
    let s = captured
        .lock()
        .unwrap()
        .clone()
        .expect("system prompt captured");
    assert!(
        s.contains("로컬 KB 위에서 동작"),
        "shared V1/V2 prefix expected, got: {s}"
    );
    assert!(
        !s.contains("학습 지식"),
        "V1 must NOT contain V2-only 학습 지식 rule, got: {s}"
    );
    assert!(
        !s.contains("확실하지 않다"),
        "V1 must NOT contain V2-only 확실하지 않다 rule, got: {s}"
    );
}

#[test]
fn ask_with_rag_v2_uses_v2_system_prompt() {
    let (pipeline, captured, _env) = build_pipeline_with_template("rag-v2");
    let _ = pipeline.ask("hello", lexical_opts());
    let s = captured
        .lock()
        .unwrap()
        .clone()
        .expect("system prompt captured");
    assert!(
        s.contains("학습 지식"),
        "V2 must contain 학습 지식 rule, got: {s}"
    );
    assert!(
        s.contains("확실하지 않다"),
        "V2 must contain 확실하지 않다 rule, got: {s}"
    );
    assert!(
        s.contains("큰따옴표"),
        "V2 must contain 큰따옴표 rule, got: {s}"
    );
}

#[test]
fn ask_with_unknown_template_returns_early_error() {
    let (pipeline, _captured, _env) = build_pipeline_with_template("rag-v99");
    let result = pipeline.ask("hello", lexical_opts());
    assert!(result.is_err(), "expected error on unknown version");
    let msg = format!("{:#}", result.unwrap_err());
    assert!(
        msg.contains("rag-v99") && msg.contains("expected"),
        "expected error to mention version + expected list, got: {msg}"
    );
}
