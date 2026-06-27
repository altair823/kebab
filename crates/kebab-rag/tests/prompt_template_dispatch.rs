//! Integration tests for rag-v3 / rag-v4 / unknown-version dispatch.
//!
//! Wraps `MockLanguageModel` in a `CapturingLm` that snapshots
//! `GenerateRequest::system` on every `generate_stream` call so the
//! tests can assert which template constant the pipeline rendered.

mod common;

use std::sync::{Arc, Mutex};

use common::{MockRetriever, RagEnv, id32, mk_hit};
use kebab_core::{
    FinishReason, LanguageModel, Retriever, SearchMode, TokenChunk, TokenUsage, TrustLevel,
};
use kebab_core::MockLanguageModel;
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
    /// rag-provenance-label: also snapshot `req.user` (the packed [근거]
    /// block) so tests can assert the per-chunk `source=`/`trust=` header.
    captured_user: Arc<Mutex<Option<String>>>,
}

impl CapturingLm {
    fn new(captured: Arc<Mutex<Option<String>>>) -> Self {
        Self::with_user(captured, Arc::new(Mutex::new(None)))
    }

    fn with_user(
        captured_system: Arc<Mutex<Option<String>>>,
        captured_user: Arc<Mutex<Option<String>>>,
    ) -> Self {
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
            captured_system,
            captured_user,
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
        *self.captured_user.lock().unwrap() = Some(req.user.clone());
        self.inner.generate_stream(req)
    }
}

/// Mirror of `streaming_events::opts_with_sink` minus the sink. p9-fb-41
/// added `impl Default for AskOpts` — these explicit fixtures stay
/// for now so a future field addition fails compilation here too,
/// surfacing intent. New callers should prefer `..Default::default()`.
fn lexical_opts() -> AskOpts {
    AskOpts {
        k: 3,
        explain: false,
        mode: SearchMode::Lexical,
        temperature: Some(0.0),
        seed: Some(0),
        stream_sink: None,
        multi_hop: false,
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
    let pipeline = RagPipeline::new(env.config.rag.clone(), env.config.models.clone(), env.config.search.clone(), retriever, lm, env.sqlite.clone());
    (pipeline, captured, env)
}

#[test]
fn ask_with_rag_v3_uses_v3_system_prompt() {
    let (pipeline, captured, _env) = build_pipeline_with_template("rag-v3");
    let _ = pipeline.ask("hello", lexical_opts());
    let s = captured
        .lock()
        .unwrap()
        .clone()
        .expect("system prompt captured");
    assert!(
        s.contains("로컬 KB 위에서 동작"),
        "shared prefix expected, got: {s}"
    );
    assert!(
        s.contains("학습 지식"),
        "V3 must contain 학습 지식 rule, got: {s}"
    );
    assert!(
        s.contains("원본 질문"),
        "V3 must contain language-matching rule (v3-only), got: {s}"
    );
}

#[test]
fn ask_with_unknown_template_returns_early_error() {
    let (pipeline, _captured, _env) = build_pipeline_with_template("rag-v99");
    let result = pipeline.ask("hello", lexical_opts());
    assert!(result.is_err(), "expected error on unknown version");
    let msg = format!("{:#}", result.unwrap_err());
    assert!(
        msg.contains("rag-v99") && msg.contains("expected") && msg.contains("rag-v3") && msg.contains("rag-v4"),
        "expected error to mention version + expected list, got: {msg}"
    );
}

#[test]
fn ask_with_rag_v4_uses_v4_system_prompt() {
    let (pipeline, captured, _env) = build_pipeline_with_template("rag-v4");
    let _ = pipeline.ask("hello", lexical_opts());
    let s = captured
        .lock()
        .unwrap()
        .clone()
        .expect("system prompt captured");
    assert!(
        s.contains("로컬 KB 위에서 동작"),
        "shared prefix expected, got: {s}"
    );
    // rag-v4 = rag-v3 rules + the two provenance rules.
    assert!(
        s.contains("학습 지식") && s.contains("원본 질문"),
        "V4 must retain V3 rules, got: {s}"
    );
    assert!(
        s.contains("신뢰도 우선") && s.contains("trust=primary"),
        "V4 must contain trust-discount rule, got: {s}"
    );
    assert!(
        s.contains("귀속"),
        "V4 must contain attribution rule, got: {s}"
    );
}

/// rag-provenance-label: build a pipeline whose retriever returns a single
/// hit with the given provenance, capturing the packed [근거] user prompt.
fn pack_user_prompt_for_hit(
    source_id: Option<&str>,
    trust_level: Option<TrustLevel>,
) -> String {
    let mut env = RagEnv::new();
    env.config.rag.prompt_template_version = "rag-v4".to_string();
    env.config.rag.score_gate = 0.0;
    let captured_system = Arc::new(Mutex::new(None));
    let captured_user = Arc::new(Mutex::new(None));
    let lm: Arc<dyn LanguageModel> =
        Arc::new(CapturingLm::with_user(captured_system, captured_user.clone()));
    let chunk_id = id32("c");
    let doc_id = id32("d");
    env.seed_chunk(&chunk_id, &doc_id, "a.md", "hello world", &["H"]);
    let mut hit = mk_hit(1, &chunk_id, &doc_id, "a.md", 0.9, &["H"]);
    hit.source_id = source_id.map(str::to_string);
    hit.trust_level = trust_level;
    let retriever: Arc<dyn Retriever> = Arc::new(MockRetriever::new(vec![hit]));
    let pipeline = RagPipeline::new(env.config.rag.clone(), env.config.models.clone(), env.config.search.clone(), retriever, lm, env.sqlite.clone());
    let _ = pipeline.ask("hello", lexical_opts());
    let out = captured_user
        .lock()
        .unwrap()
        .clone()
        .expect("user prompt captured");
    // Keep env alive until after ask returns.
    drop(env);
    out
}

#[test]
fn pack_context_header_renders_source_and_trust_labels() {
    let user = pack_user_prompt_for_hit(Some("jira"), Some(TrustLevel::Secondary));
    assert!(
        user.contains("[#1] source=jira trust=secondary doc=a.md"),
        "expected provenance label in chunk header, got: {user}"
    );
}

#[test]
fn pack_context_header_uses_default_source_and_unknown_trust_when_none() {
    let user = pack_user_prompt_for_hit(None, None);
    assert!(
        user.contains("[#1] source=default trust=unknown doc=a.md"),
        "expected default/unknown provenance label when fields absent, got: {user}"
    );
}
