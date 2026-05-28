//! S3 follow-up (2026-05-26): hypothesis-side char budget + token-count
//! fallback retry — multi-hop integration tests via `SpyNliVerifier`.
//!
//! Coverage:
//!
//! 1. `long_en_synth_answer_truncated_before_nli_call` — EN long answer
//!    → char-budget 만으로 충분 (token_count Ok(100)) → retry 0 회 →
//!    hypothesis 가 정확히 1200 chars 로 truncate + Right direction pin
//!    (앞부분 보존).
//! 2. `long_kr_synth_answer_retries_with_smaller_budget` — KR-sim long
//!    answer → token_count fn 이 chars > 1000 → 900 / > 500 → 450 /
//!    else → 220 시뮬레이션 → retry >= 3 회 + 최종 hypothesis ≤ 300
//!    chars + happy path. KR safety pin.
//! 3. `unrelenting_token_overflow_falls_through_to_unavailable` —
//!    token_count fn 이 무조건 Ok(9_999) → retry 소진 → graceful
//!    `NliModelUnavailable` refusal (regression 0).
//!
//! Pipeline construction pattern (Option B inline — plan §2 step 8):
//! 각 test 안에서 `RagEnv::new()` + `ScriptedRetriever::new(...)` +
//! `ScriptedLm::new(vec![decompose, decide, synth])` +
//! `RagPipeline::new(...).with_verifier(verifier)` inline. helper
//! `build_test_pipeline_with_long_answer` 미작성 (1회용 + 매 test 마다
//! long-answer 길이/언어 다름).

mod common;

use std::sync::{Arc, Mutex};

use common::{RagEnv, ScriptedLm, ScriptedRetriever, SpyNliVerifier, id32, mk_hit};
use kebab_core::{LanguageModel, RefusalReason, Retriever, SearchMode};
use kebab_nli::{NliScores, NliVerifier};
use kebab_rag::{AskOpts, RagPipeline};

/// Default `AskOpts` for multi-hop tests: deterministic seed, lexical
/// mode (so the test crate doesn't need to wire up an embedder), and
/// `multi_hop: true` to route through `ask_multi_hop`.
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

/// EN long answer (5 000 chars) → char-budget 만으로 충분 → retry 0회 →
/// grounded. Right direction pin: hypothesis 의 첫 1200 chars 가 input 의
/// 첫 1200 chars 와 일치 (= Right direction = 앞부분 보존).
#[test]
fn long_en_synth_answer_truncated_before_nli_call() {
    let env = RagEnv::new();
    let cid = id32("c1");
    let did = id32("d1");
    env.seed_chunk(&cid, &did, "notes/a.md", "Body text.", &["Intro"]);
    let hits = vec![mk_hit(1, &cid, &did, "notes/a.md", 0.85, &["Intro"])];

    let mut cfg = env.config.clone();
    cfg.rag.nli_threshold = 0.5;

    // ScriptedRetriever: entry 0 = probe, entry 1 = q1 sub-query.
    let retriever = Arc::new(ScriptedRetriever::new(vec![hits.clone(), hits]));
    let retriever_dyn: Arc<dyn Retriever> = retriever;

    // Synthesize answer ~6000 chars (lorem ipsum, 12 chars × 500 reps),
    // citation marker appended after so `grounded_unaware` 통과.
    let long_answer = format!("{} [#1]", "lorem ipsum ".repeat(500));
    let lm = Arc::new(ScriptedLm::new(vec![r#"["q1"]"#, r"[]", &long_answer]));
    let lm_dyn: Arc<dyn LanguageModel> = lm;

    let verifier = SpyNliVerifier::new(
        |_premise, _hypothesis| {
            Ok(NliScores {
                entailment: 0.9,
                neutral: 0.05,
                contradiction: 0.05,
            })
        },
        |_h| Ok(100), // budget 안 — retry 0
    );
    let verifier_handle = verifier.clone();
    let verifier_dyn: Arc<dyn NliVerifier> = verifier;

    let pipeline = RagPipeline::new(cfg, retriever_dyn, lm_dyn, env.sqlite.clone())
        .with_verifier(verifier_dyn);

    let answer = pipeline.ask("compound", multi_hop_opts()).unwrap();

    let received = verifier_handle.received_hypotheses.lock().unwrap();
    assert_eq!(received.len(), 1, "verifier called exactly once");

    let hyp = &received[0];
    assert_eq!(
        hyp.chars().count(),
        1200,
        "hypothesis truncated to MAX_NLI_HYPOTHESIS_CHARS_INITIAL"
    );

    // Right direction pin — hypothesis 의 첫 1200 chars 가 input 의
    // 첫 1200 chars 와 일치 (= Right direction = 앞부분 보존). Left/Middle
    // direction 으로 regress 시 본 test 가 즉시 fail.
    let input_first_1200: String = long_answer.chars().take(1200).collect();
    assert_eq!(
        hyp.as_str(),
        input_first_1200.as_str(),
        "Right direction = front preserved"
    );

    assert_eq!(
        answer.refusal_reason, None,
        "long answer must reach happy path"
    );
}

/// KR long answer → token count > budget → char budget 절반화 retry →
/// eventual fit. KR safety pin (1-2 chars/token density 시뮬레이션).
#[test]
fn long_kr_synth_answer_retries_with_smaller_budget() {
    let env = RagEnv::new();
    let cid = id32("c1");
    let did = id32("d1");
    env.seed_chunk(&cid, &did, "notes/a.md", "Body text.", &["Intro"]);
    let hits = vec![mk_hit(1, &cid, &did, "notes/a.md", 0.85, &["Intro"])];

    let mut cfg = env.config.clone();
    cfg.rag.nli_threshold = 0.5;

    let retriever = Arc::new(ScriptedRetriever::new(vec![hits.clone(), hits]));
    let retriever_dyn: Arc<dyn Retriever> = retriever;

    // ~2500-char KR-sim answer (한국어 6 chars × 416 reps ≈ 2496 chars)
    // + citation marker.
    let kr_long_answer = format!("{} [#1]", "한국어 본문 ".repeat(416));
    let lm = Arc::new(ScriptedLm::new(vec![r#"["q1"]"#, r"[]", &kr_long_answer]));
    let lm_dyn: Arc<dyn LanguageModel> = lm;

    let token_count_call_count = Arc::new(Mutex::new(0_usize));
    let tcc = token_count_call_count.clone();
    // 시뮬레이션: 1200 chars → 900 tokens (cap 초과), 600 chars → 450
    // tokens (cap 초과), 300 chars → 220 tokens (cap 안). retry 3 회.
    let verifier = SpyNliVerifier::new(
        |_premise, _hypothesis| {
            Ok(NliScores {
                entailment: 0.85,
                neutral: 0.10,
                contradiction: 0.05,
            })
        },
        move |h| {
            *tcc.lock().unwrap() += 1;
            let count = h.chars().count();
            if count > 1000 {
                Ok(900)
            } else if count > 500 {
                Ok(450)
            } else {
                Ok(220)
            }
        },
    );
    let verifier_handle = verifier.clone();
    let verifier_dyn: Arc<dyn NliVerifier> = verifier;

    let pipeline = RagPipeline::new(cfg, retriever_dyn, lm_dyn, env.sqlite.clone())
        .with_verifier(verifier_dyn);

    let answer = pipeline.ask("compound", multi_hop_opts()).unwrap();

    assert!(
        *token_count_call_count.lock().unwrap() >= 3,
        "retry loop must call token_count >= 3 (1200, 600, 300 candidates)"
    );
    let received = verifier_handle.received_hypotheses.lock().unwrap();
    assert!(
        received[0].chars().count() <= 300,
        "final hypothesis <= 300 chars after retry, got {}",
        received[0].chars().count()
    );
    assert_eq!(
        answer.refusal_reason, None,
        "KR long answer reaches happy path after retry"
    );
}

/// Retry budget 소진 시 graceful unavailable — fix 의 fallback path 가
/// 기존 unavailable wire shape 유지 (regression 0).
#[test]
fn unrelenting_token_overflow_falls_through_to_unavailable() {
    let env = RagEnv::new();
    let cid = id32("c1");
    let did = id32("d1");
    env.seed_chunk(&cid, &did, "notes/a.md", "Body text.", &["Intro"]);
    let hits = vec![mk_hit(1, &cid, &did, "notes/a.md", 0.85, &["Intro"])];

    let mut cfg = env.config.clone();
    cfg.rag.nli_threshold = 0.5;

    let retriever = Arc::new(ScriptedRetriever::new(vec![hits.clone(), hits]));
    let retriever_dyn: Arc<dyn Retriever> = retriever;

    // ~3000-char answer + marker — over budget regardless.
    let unrelenting_answer = format!("{} [#1]", "단단한 압축 ".repeat(500));
    let lm = Arc::new(ScriptedLm::new(vec![
        r#"["q1"]"#,
        r"[]",
        &unrelenting_answer,
    ]));
    let lm_dyn: Arc<dyn LanguageModel> = lm;

    let verifier = SpyNliVerifier::new(
        |_premise, _hypothesis| {
            unreachable!("score not reached when token-count check fails");
        },
        |_h| Ok(9_999), // 모든 budget 에서 token count 초과 — retry 소진
    );
    let verifier_dyn: Arc<dyn NliVerifier> = verifier;

    let pipeline = RagPipeline::new(cfg, retriever_dyn, lm_dyn, env.sqlite.clone())
        .with_verifier(verifier_dyn);

    let answer = pipeline.ask("compound", multi_hop_opts()).unwrap();

    assert_eq!(
        answer.refusal_reason,
        Some(RefusalReason::NliModelUnavailable),
        "graceful fallback to unavailable when retry exhausted"
    );
    assert!(
        answer.verification.is_none(),
        "NliModelUnavailable: verification stays None"
    );
}
