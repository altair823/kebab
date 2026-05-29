//! Integration tests for `RagPipeline` (P4-3 spec test plan).
//!
//! Real adapters (Ollama, fastembed, LanceDB) are NOT used. Every test
//! injects a `MockLanguageModel` and a `MockRetriever` so the pipeline's
//! behavior is exercised in isolation from network / heavy IO.

mod common;

use std::sync::Arc;
use std::sync::atomic::Ordering;

use common::{MockRetriever, RagEnv, id32, mk_hit, mk_hit_with_indexed_at};
use kebab_core::{FinishReason, LanguageModel, Retriever, SearchMode, TokenChunk, TokenUsage};
use kebab_llm::MockLanguageModel;
use kebab_rag::{AskOpts, RagPipeline, RefusalReason, StreamEvent};

/// LM ID used everywhere — kept short so snapshots stay stable.
const TEST_LM_ID: &str = "mock-lm";

/// Counter wrapper so tests can assert "no LLM call happened".
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
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
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

fn default_opts() -> AskOpts {
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
        multi_hop: false,
    }
}

// ── 1. empty hits → NoChunks, no LLM call ────────────────────────────────

#[test]
fn empty_hits_refuses_no_chunks_without_llm_call() {
    let env = RagEnv::new();
    let retriever: Arc<dyn Retriever> = Arc::new(MockRetriever::new(Vec::new()));
    let lm = Arc::new(CountingLm::new("(unused)"));
    let lm_dyn: Arc<dyn LanguageModel> = lm.clone();
    let pipeline = RagPipeline::new(env.config.clone(), retriever, lm_dyn, env.sqlite.clone());

    let answer = pipeline.ask("anything", default_opts()).unwrap();
    assert_eq!(answer.refusal_reason, Some(RefusalReason::NoChunks));
    assert!(!answer.grounded);
    assert!(answer.citations.is_empty());
    assert_eq!(lm.calls(), 0, "LM must NOT be called on empty hits");
    assert_eq!(env.count_answers(), 1, "answers row written for refusal");
}

// ── 2. score gate refuses without LLM call ────────────────────────────────

#[test]
fn top_below_gate_refuses_score_gate_without_llm_call() {
    let env = RagEnv::new();
    // top score 0.10 below default gate 0.30
    let hits = vec![
        mk_hit(1, &id32("c1"), &id32("d1"), "notes/a.md", 0.10, &["A"]),
        mk_hit(2, &id32("c2"), &id32("d2"), "notes/b.md", 0.05, &["B"]),
    ];
    let retriever: Arc<dyn Retriever> = Arc::new(MockRetriever::new(hits));
    let lm = Arc::new(CountingLm::new("(unused)"));
    let lm_dyn: Arc<dyn LanguageModel> = lm.clone();
    let pipeline = RagPipeline::new(env.config.clone(), retriever, lm_dyn, env.sqlite.clone());

    let answer = pipeline.ask("q", default_opts()).unwrap();
    assert_eq!(answer.refusal_reason, Some(RefusalReason::ScoreGate));
    assert!(!answer.grounded);
    assert_eq!(
        answer.citations.len(),
        2,
        "all near-miss candidates surfaced"
    );
    for c in &answer.citations {
        assert!(c.marker.is_none(), "ScoreGate citations have no marker");
    }
    assert_eq!(lm.calls(), 0, "LM must NOT be called when gate refuses");
    assert_eq!(env.count_answers(), 1);
    assert!(answer.answer.contains("근거 부족"));
    assert!(answer.answer.contains("notes/a.md"));
}

// ── 3. grounded happy path with [#1] ──────────────────────────────────────

#[test]
fn grounded_happy_path_marker_one() {
    let env = RagEnv::new();
    let cid = id32("c1");
    let did = id32("d1");
    env.seed_chunk(
        &cid,
        &did,
        "notes/a.md",
        "Rust is a systems language.",
        &["Intro"],
    );
    let hits = vec![mk_hit(1, &cid, &did, "notes/a.md", 0.85, &["Intro"])];
    let retriever: Arc<dyn Retriever> = Arc::new(MockRetriever::new(hits));
    let canned = "Rust is a systems language. [#1]";
    let lm: Arc<dyn LanguageModel> = Arc::new(CountingLm::new(canned));
    let pipeline = RagPipeline::new(env.config.clone(), retriever, lm, env.sqlite.clone());

    let answer = pipeline.ask("what is rust", default_opts()).unwrap();
    assert!(answer.grounded);
    assert_eq!(answer.refusal_reason, None);
    assert_eq!(answer.citations.len(), 1);
    assert_eq!(answer.citations[0].marker.as_deref(), Some("[1]"));
    assert_eq!(answer.retrieval.chunks_used, 1);
    assert_eq!(env.count_answers(), 1);
}

// ── 4. unknown marker [#7] → LlmSelfJudge ─────────────────────────────────

#[test]
fn unknown_marker_refuses_llm_self_judge() {
    let env = RagEnv::new();
    let cid = id32("c1");
    let did = id32("d1");
    env.seed_chunk(&cid, &did, "notes/a.md", "doc text", &["Intro"]);
    let hits = vec![mk_hit(1, &cid, &did, "notes/a.md", 0.85, &["Intro"])];
    let retriever: Arc<dyn Retriever> = Arc::new(MockRetriever::new(hits));
    // Marker 7 is NOT in the packed set (only #1 is).
    let lm: Arc<dyn LanguageModel> = Arc::new(CountingLm::new("answer text [#7]"));
    let pipeline = RagPipeline::new(env.config.clone(), retriever, lm, env.sqlite.clone());

    let answer = pipeline.ask("q", default_opts()).unwrap();
    assert_eq!(answer.refusal_reason, Some(RefusalReason::LlmSelfJudge));
    assert!(!answer.grounded);
    // Even unknown markers are NOT included in citations (we only report
    // markers that map to the packed set).
    assert!(answer.citations.is_empty());
}

// ── 5. [1] (no #) → LlmSelfJudge (regex strictness) ───────────────────────

#[test]
fn marker_without_hash_is_no_marker() {
    let env = RagEnv::new();
    let cid = id32("c1");
    let did = id32("d1");
    env.seed_chunk(&cid, &did, "notes/a.md", "doc text", &["Intro"]);
    let hits = vec![mk_hit(1, &cid, &did, "notes/a.md", 0.85, &["Intro"])];
    let retriever: Arc<dyn Retriever> = Arc::new(MockRetriever::new(hits));
    // `[1]` is NOT a valid marker — strict regex requires `[#1]`.
    let lm: Arc<dyn LanguageModel> = Arc::new(CountingLm::new("the answer [1]"));
    let pipeline = RagPipeline::new(env.config.clone(), retriever, lm, env.sqlite.clone());

    let answer = pipeline.ask("q", default_opts()).unwrap();
    assert_eq!(answer.refusal_reason, Some(RefusalReason::LlmSelfJudge));
    assert!(!answer.grounded);
}

// ── 6. vec![1] no real citation → LlmSelfJudge (no false positive) ────────

#[test]
fn vec_bracket_one_is_no_false_positive() {
    let env = RagEnv::new();
    let cid = id32("c1");
    let did = id32("d1");
    env.seed_chunk(&cid, &did, "notes/a.md", "doc", &["Intro"]);
    let hits = vec![mk_hit(1, &cid, &did, "notes/a.md", 0.85, &["Intro"])];
    let retriever: Arc<dyn Retriever> = Arc::new(MockRetriever::new(hits));
    // `vec![1]` MUST NOT be misread as a citation marker.
    let lm: Arc<dyn LanguageModel> = Arc::new(CountingLm::new("see vec![1] in code"));
    let pipeline = RagPipeline::new(env.config.clone(), retriever, lm, env.sqlite.clone());

    let answer = pipeline.ask("q", default_opts()).unwrap();
    assert_eq!(answer.refusal_reason, Some(RefusalReason::LlmSelfJudge));
    assert!(!answer.grounded);
}

// ── 7. "근거가 부족합니다" → LlmSelfJudge ────────────────────────────────

#[test]
fn explicit_korean_refusal_is_self_judge() {
    let env = RagEnv::new();
    let cid = id32("c1");
    let did = id32("d1");
    env.seed_chunk(&cid, &did, "notes/a.md", "doc", &["Intro"]);
    let hits = vec![mk_hit(1, &cid, &did, "notes/a.md", 0.85, &["Intro"])];
    let retriever: Arc<dyn Retriever> = Arc::new(MockRetriever::new(hits));
    let lm: Arc<dyn LanguageModel> = Arc::new(CountingLm::new("근거가 부족합니다."));
    let pipeline = RagPipeline::new(env.config.clone(), retriever, lm, env.sqlite.clone());

    let answer = pipeline.ask("q", default_opts()).unwrap();
    assert_eq!(answer.refusal_reason, Some(RefusalReason::LlmSelfJudge));
    assert!(!answer.grounded);
}

// ── 8. context packing budget overflow ────────────────────────────────────

#[test]
fn packing_stops_before_budget_overflow() {
    let env = RagEnv::new();
    // Squeeze the budget so only one chunk fits.
    let mut cfg = env.config.clone();
    cfg.rag.max_context_tokens = 50; // very small budget
    // Three giant chunks
    let huge_text: String = "X".repeat(2_000); // ~500 tokens each
    let mut hits = Vec::new();
    for i in 0..3_u32 {
        let cid = id32(&format!("c{i}"));
        let did = id32(&format!("d{i}"));
        env.seed_chunk(
            &cid,
            &did,
            &format!("notes/a{i}.md"),
            &huge_text,
            &["Intro"],
        );
        hits.push(mk_hit(
            i + 1,
            &cid,
            &did,
            &format!("notes/a{i}.md"),
            0.9,
            &["Intro"],
        ));
    }
    let retriever: Arc<dyn Retriever> = Arc::new(MockRetriever::new(hits));
    let lm: Arc<dyn LanguageModel> = Arc::new(CountingLm::new("ok [#1]"));
    let pipeline = RagPipeline::new(cfg, retriever, lm, env.sqlite.clone());

    let answer = pipeline.ask("q", default_opts()).unwrap();
    // At least one chunk was packed; the budget cap should keep it to <= 1.
    assert_eq!(
        answer.retrieval.chunks_used, 1,
        "exactly one chunk fits when budget is tiny"
    );
    assert_eq!(answer.retrieval.chunks_returned, 3);
    assert!(answer.grounded);
}

// ── 9. streaming forwards tokens to mpsc ──────────────────────────────────

#[test]
fn streaming_forwards_tokens_to_sink() {
    let env = RagEnv::new();
    let cid = id32("c1");
    let did = id32("d1");
    env.seed_chunk(&cid, &did, "notes/a.md", "doc", &["Intro"]);
    let hits = vec![mk_hit(1, &cid, &did, "notes/a.md", 0.85, &["Intro"])];
    let retriever: Arc<dyn Retriever> = Arc::new(MockRetriever::new(hits));
    let canned = "ok [#1]";
    let lm: Arc<dyn LanguageModel> = Arc::new(CountingLm::new(canned));
    let pipeline = RagPipeline::new(env.config.clone(), retriever, lm, env.sqlite.clone());

    let (tx, rx) = std::sync::mpsc::channel::<StreamEvent>();
    let mut opts = default_opts();
    opts.stream_sink = Some(tx);
    let _ = pipeline.ask("q", opts).unwrap();
    // p9-fb-33: extract Token deltas from the staged event stream.
    let collected: String = rx
        .into_iter()
        .filter_map(|ev| match ev {
            StreamEvent::Token { delta, .. } => Some(delta),
            _ => None,
        })
        .collect::<String>();
    assert_eq!(collected, canned);
}

// ── 10. dropped receiver aborts generation, records LlmStreamAborted ──────
//
// p9-fb-33: cancel semantics changed. Pre-fb-33 the pipeline drove
// the LM loop to completion and silently dropped sends. Now a
// SendError breaks the loop and stamps `RefusalReason::LlmStreamAborted`
// onto the persisted row — the partial answer (whatever was buffered
// before the cancel) still gets written for audit.

#[test]
fn dropped_receiver_aborts_with_llm_stream_aborted() {
    let env = RagEnv::new();
    let cid = id32("c1");
    let did = id32("d1");
    env.seed_chunk(&cid, &did, "notes/a.md", "doc", &["Intro"]);
    let hits = vec![mk_hit(1, &cid, &did, "notes/a.md", 0.85, &["Intro"])];
    let retriever: Arc<dyn Retriever> = Arc::new(MockRetriever::new(hits));
    let canned = "ok [#1]";
    let lm: Arc<dyn LanguageModel> = Arc::new(CountingLm::new(canned));
    let pipeline = RagPipeline::new(env.config.clone(), retriever, lm, env.sqlite.clone());

    let (tx, rx) = std::sync::mpsc::channel::<StreamEvent>();
    drop(rx); // receiver gone — first Token send fails, loop breaks
    let mut opts = default_opts();
    opts.stream_sink = Some(tx);
    let answer = pipeline.ask("q", opts).unwrap();
    assert!(!answer.grounded, "cancel takes priority over grounded");
    assert_eq!(
        answer.refusal_reason,
        Some(RefusalReason::LlmStreamAborted),
        "cancel records LlmStreamAborted",
    );
    assert_eq!(env.count_answers(), 1, "answers row still persisted");
}

// ── 11. Send + Sync compile check ─────────────────────────────────────────
// Implemented inside `kb-rag::pipeline::tests::rag_pipeline_is_send_sync`.

// ── 12. usage from final Done chunk ───────────────────────────────────────

#[test]
fn usage_populated_from_done_chunk() {
    let env = RagEnv::new();
    let cid = id32("c1");
    let did = id32("d1");
    env.seed_chunk(&cid, &did, "notes/a.md", "doc", &["Intro"]);
    let hits = vec![mk_hit(1, &cid, &did, "notes/a.md", 0.85, &["Intro"])];
    let retriever: Arc<dyn Retriever> = Arc::new(MockRetriever::new(hits));
    let lm: Arc<dyn LanguageModel> = Arc::new(CountingLm::new("ok [#1]"));
    let pipeline = RagPipeline::new(env.config.clone(), retriever, lm, env.sqlite.clone());

    let answer = pipeline.ask("q", default_opts()).unwrap();
    assert_eq!(answer.usage.prompt_tokens, 10, "from canned_usage");
    assert_eq!(answer.usage.completion_tokens, 5);
}

// ── 13. answers row inserted in all paths (incl. refusals) ────────────────

#[test]
fn answers_row_inserted_for_each_refusal_kind() {
    // NoChunks
    {
        let env = RagEnv::new();
        let retriever: Arc<dyn Retriever> = Arc::new(MockRetriever::new(Vec::new()));
        let lm: Arc<dyn LanguageModel> = Arc::new(CountingLm::new(""));
        let p = RagPipeline::new(env.config.clone(), retriever, lm, env.sqlite.clone());
        p.ask("q", default_opts()).unwrap();
        assert_eq!(env.count_answers(), 1);
    }
    // ScoreGate
    {
        let env = RagEnv::new();
        let cid = id32("c1");
        let did = id32("d1");
        env.seed_chunk(&cid, &did, "notes/a.md", "doc", &["Intro"]);
        let hits = vec![mk_hit(1, &cid, &did, "notes/a.md", 0.05, &["Intro"])];
        let retriever: Arc<dyn Retriever> = Arc::new(MockRetriever::new(hits));
        let lm: Arc<dyn LanguageModel> = Arc::new(CountingLm::new(""));
        let p = RagPipeline::new(env.config.clone(), retriever, lm, env.sqlite.clone());
        p.ask("q", default_opts()).unwrap();
        assert_eq!(env.count_answers(), 1);
    }
    // LlmSelfJudge (silent ungrounded)
    {
        let env = RagEnv::new();
        let cid = id32("c1");
        let did = id32("d1");
        env.seed_chunk(&cid, &did, "notes/a.md", "doc", &["Intro"]);
        let hits = vec![mk_hit(1, &cid, &did, "notes/a.md", 0.85, &["Intro"])];
        let retriever: Arc<dyn Retriever> = Arc::new(MockRetriever::new(hits));
        let lm: Arc<dyn LanguageModel> = Arc::new(CountingLm::new("answer with no marker"));
        let p = RagPipeline::new(env.config.clone(), retriever, lm, env.sqlite.clone());
        p.ask("q", default_opts()).unwrap();
        assert_eq!(env.count_answers(), 1);
    }
}

// ── 14. determinism: temp=0 + seed=0 → identical Answer (mock) ────────────

#[test]
fn determinism_temperature_zero_seed_zero() {
    let env = RagEnv::new();
    let cid = id32("c1");
    let did = id32("d1");
    env.seed_chunk(&cid, &did, "notes/a.md", "doc", &["Intro"]);
    let hits = vec![mk_hit(1, &cid, &did, "notes/a.md", 0.85, &["Intro"])];
    // Two pipelines, two retrievers, two LMs — but identical canned configs.
    let mk_pipeline = || {
        let r: Arc<dyn Retriever> = Arc::new(MockRetriever::new(hits.clone()));
        let lm: Arc<dyn LanguageModel> = Arc::new(CountingLm::new("Rust is. [#1]"));
        RagPipeline::new(env.config.clone(), r, lm, env.sqlite.clone())
    };
    let a1 = mk_pipeline().ask("q", default_opts()).unwrap();
    let a2 = mk_pipeline().ask("q", default_opts()).unwrap();
    assert_eq!(a1.answer, a2.answer);
    assert_eq!(a1.grounded, a2.grounded);
    assert_eq!(a1.citations, a2.citations);
    assert_eq!(a1.retrieval.chunks_used, a2.retrieval.chunks_used);
    assert_eq!(a1.retrieval.k, a2.retrieval.k);
    // trace_id and created_at and latency_ms WILL differ — they include
    // wall-clock — so we don't compare them.
}

// ── 15a. all chunks unfetchable from store → NoChunks fallback ───────────

#[test]
fn unfetchable_chunks_fall_back_to_no_chunks() {
    // Hits exist (so the score gate passes) but their chunk_id rows are
    // never seeded into the store — `DocumentStore::get_chunk` returns
    // None for every one. Pipeline should detect the empty packed list
    // and refuse with NoChunks rather than letting the LLM run with an
    // empty `[근거]` block (which would self-refuse → LlmSelfJudge).
    let env = RagEnv::new();
    let cid = id32("missing");
    let did = id32("d_missing");
    // NOTE: no `env.seed_chunk(...)` call — chunk row absent from store.
    let hits = vec![mk_hit(1, &cid, &did, "notes/missing.md", 0.85, &["X"])];
    let retriever: Arc<dyn Retriever> = Arc::new(MockRetriever::new(hits));
    let lm = Arc::new(CountingLm::new("(should never run)"));
    let lm_dyn: Arc<dyn LanguageModel> = lm.clone();
    let pipeline = RagPipeline::new(env.config.clone(), retriever, lm_dyn, env.sqlite.clone());

    let answer = pipeline.ask("q", default_opts()).unwrap();
    assert_eq!(answer.refusal_reason, Some(RefusalReason::NoChunks));
    assert!(!answer.grounded);
    assert!(answer.citations.is_empty());
    assert_eq!(
        lm.calls(),
        0,
        "LM must NOT be called when every retrieved chunk is unfetchable"
    );
    assert_eq!(env.count_answers(), 1, "answers row written for refusal");
}

// ── 16. p9-fb-32: AnswerCitation carries indexed_at + stale ──────────────
//
// Previously the LLM-citation construction site stamped `UNIX_EPOCH` +
// `false` as a Task-7 placeholder. Task 7 plumbs real values from the
// upstream `SearchHit` through `pack_context` so the wire-side
// `AnswerCitation` reflects the document's actual age.

#[test]
fn grounded_citations_inherit_indexed_at_and_stale_from_hit() {
    let env = RagEnv::new();
    let cid = id32("c1");
    let did = id32("d1");
    env.seed_chunk(&cid, &did, "notes/a.md", "Apples are fruit.", &["Intro"]);
    // 60 days old vs. the default 30-day threshold → stale.
    let now = time::OffsetDateTime::now_utc();
    let sixty_days_ago = now - time::Duration::days(60);
    let hits = vec![mk_hit_with_indexed_at(
        1,
        &cid,
        &did,
        "notes/a.md",
        0.85,
        &["Intro"],
        sixty_days_ago,
    )];
    let retriever: Arc<dyn Retriever> = Arc::new(MockRetriever::new(hits));
    let lm: Arc<dyn LanguageModel> = Arc::new(CountingLm::new("apples are fruit. [#1]"));
    let pipeline = RagPipeline::new(env.config.clone(), retriever, lm, env.sqlite.clone());

    let answer = pipeline.ask("apples", default_opts()).unwrap();
    assert!(answer.grounded);
    assert_eq!(answer.citations.len(), 1, "one cited marker [#1]");
    let c = &answer.citations[0];
    // indexed_at must be the value the retriever produced — NOT the
    // UNIX_EPOCH placeholder the Task 6 cross-task patch left behind.
    assert_eq!(
        c.indexed_at, sixty_days_ago,
        "AnswerCitation.indexed_at must inherit from SearchHit.indexed_at"
    );
    // 60d > default 30d threshold → stale.
    assert!(
        c.stale,
        "60-day-old hit must surface stale=true on the AnswerCitation"
    );
}

#[test]
fn grounded_citations_not_stale_for_fresh_hit() {
    let env = RagEnv::new();
    let cid = id32("c1");
    let did = id32("d1");
    env.seed_chunk(&cid, &did, "notes/a.md", "Apples are fruit.", &["Intro"]);
    // 1 day old vs. the default 30-day threshold → fresh.
    let now = time::OffsetDateTime::now_utc();
    let one_day_ago = now - time::Duration::days(1);
    let hits = vec![mk_hit_with_indexed_at(
        1,
        &cid,
        &did,
        "notes/a.md",
        0.85,
        &["Intro"],
        one_day_ago,
    )];
    let retriever: Arc<dyn Retriever> = Arc::new(MockRetriever::new(hits));
    let lm: Arc<dyn LanguageModel> = Arc::new(CountingLm::new("apples are fruit. [#1]"));
    let pipeline = RagPipeline::new(env.config.clone(), retriever, lm, env.sqlite.clone());

    let answer = pipeline.ask("apples", default_opts()).unwrap();
    assert!(answer.grounded);
    assert_eq!(answer.citations.len(), 1);
    let c = &answer.citations[0];
    assert_eq!(c.indexed_at, one_day_ago);
    assert!(
        !c.stale,
        "1-day-old hit must NOT be stale at default 30d threshold"
    );
}

// ── 15. snapshot Answer JSON stable ───────────────────────────────────────

#[test]
fn answer_json_serializes_with_expected_keys() {
    let env = RagEnv::new();
    let cid = id32("c1");
    let did = id32("d1");
    env.seed_chunk(
        &cid,
        &did,
        "notes/a.md",
        "Rust is a systems language.",
        &["Intro"],
    );
    let hits = vec![mk_hit(1, &cid, &did, "notes/a.md", 0.85, &["Intro"])];
    let retriever: Arc<dyn Retriever> = Arc::new(MockRetriever::new(hits));
    let lm: Arc<dyn LanguageModel> = Arc::new(CountingLm::new("Rust is. [#1]"));
    let pipeline = RagPipeline::new(env.config.clone(), retriever, lm, env.sqlite.clone());
    let answer = pipeline.ask("what", default_opts()).unwrap();
    let v: serde_json::Value = serde_json::to_value(&answer).unwrap();
    // Stable top-level key set per `answer.v1` (§2.3).
    let keys: Vec<&str> = v
        .as_object()
        .unwrap()
        .keys()
        .map(std::string::String::as_str)
        .collect();
    for needed in [
        "answer",
        "citations",
        "grounded",
        "refusal_reason",
        "model",
        "embedding",
        "prompt_template_version",
        "retrieval",
        "usage",
        "created_at",
    ] {
        assert!(keys.contains(&needed), "missing top-level key {needed}");
    }
    // citations is a JSON array
    assert!(v["citations"].is_array());
    // retrieval.trace_id starts with `ret_`
    let trace_id = v["retrieval"]["trace_id"].as_str().unwrap();
    assert!(trace_id.starts_with("ret_"), "got trace_id {trace_id:?}");
}

// ── p9-fb-41: multi-hop dispatch + decompose-failure refusal ─────────────

/// `AskOpts.multi_hop = true` routes into `ask_multi_hop`. When the
/// (single) mock LLM returns garbage that `parse_decompose_response`
/// can't deserialize as `Vec<String>`, the pipeline refuses with
/// `RefusalReason::MultiHopDecomposeFailed`. Pins both the dispatch
/// (different code path than single-pass) and the early-exit refusal.
///
/// Happy-path multi-hop (decompose succeeds → retrieve → synthesize)
/// pins land in PR-3 once a scripted mock supports per-call response
/// scripting (current `MockLanguageModel` returns the same canned
/// string for every call).
#[test]
fn ask_multi_hop_dispatches_and_decompose_garbage_refuses() {
    let env = RagEnv::new();
    let cid = id32("c1");
    let did = id32("d1");
    env.seed_chunk(&cid, &did, "notes/a.md", "Body text.", &["Intro"]);
    let hits = vec![mk_hit(1, &cid, &did, "notes/a.md", 0.85, &["Intro"])];
    let retriever: Arc<dyn Retriever> = Arc::new(MockRetriever::new(hits));
    // Garbage that is NOT a JSON array of strings — the only LLM call
    // multi-hop makes here (decompose) returns this, so the pipeline
    // never gets to synthesize and exits via the decompose-failure
    // refusal path.
    let lm = Arc::new(CountingLm::new("definitely not a JSON array"));
    let lm_handle = lm.clone();
    let pipeline = RagPipeline::new(
        env.config.clone(),
        retriever,
        lm.clone() as Arc<dyn LanguageModel>,
        env.sqlite.clone(),
    );

    let opts = AskOpts {
        multi_hop: true,
        ..default_opts()
    };
    let answer = pipeline.ask("compound question", opts).unwrap();

    assert!(
        !answer.grounded,
        "decompose-failure refusal must report grounded=false"
    );
    assert_eq!(
        answer.refusal_reason,
        Some(RefusalReason::MultiHopDecomposeFailed),
        "garbage decompose response must surface MultiHopDecomposeFailed"
    );
    assert!(
        answer.citations.is_empty(),
        "refusal Answer carries no citations"
    );
    assert_eq!(
        answer.prompt_template_version.0, "rag-multi-hop-v1",
        "multi-hop path must stamp the rag-multi-hop-v1 template version"
    );
    assert_eq!(
        lm_handle.calls(),
        1,
        "decompose-failure exits before synthesize — exactly 1 LLM call"
    );
}

/// Regression pin: `AskOpts.multi_hop = false` keeps the single-pass
/// path. Same fixture as the snapshot test above; verifies that the
/// PR-2 dispatcher doesn't accidentally divert legacy callers.
#[test]
fn ask_with_multi_hop_false_keeps_single_pass_path() {
    let env = RagEnv::new();
    let cid = id32("c1");
    let did = id32("d1");
    env.seed_chunk(
        &cid,
        &did,
        "notes/a.md",
        "Rust is a systems language.",
        &["Intro"],
    );
    let hits = vec![mk_hit(1, &cid, &did, "notes/a.md", 0.85, &["Intro"])];
    let retriever: Arc<dyn Retriever> = Arc::new(MockRetriever::new(hits));
    let lm: Arc<dyn LanguageModel> = Arc::new(CountingLm::new("Rust is. [#1]"));
    let pipeline = RagPipeline::new(env.config.clone(), retriever, lm, env.sqlite.clone());

    let answer = pipeline.ask("what", default_opts()).unwrap();

    assert_eq!(
        answer.prompt_template_version.0,
        // Single-pass stamps the config's prompt_template_version
        // (config default = "rag-v3"), NOT "rag-multi-hop-v1".
        env.config.rag.prompt_template_version,
        "multi_hop=false must keep the config's prompt template (single-pass)"
    );
    assert_ne!(
        answer.prompt_template_version.0, "rag-multi-hop-v1",
        "multi_hop=false must NOT route through ask_multi_hop"
    );
}
