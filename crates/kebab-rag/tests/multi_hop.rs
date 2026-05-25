//! p9-fb-41 PR-3b-ii: integration tests for the dynamic multi-hop
//! decide loop in [`RagPipeline::ask_multi_hop`].
//!
//! Each test uses [`ScriptedLm`] to drive a different LLM response
//! per call (decompose → 0..N decide → synthesize) and, where the
//! scenario requires, [`ScriptedRetriever`] to drive different hit
//! lists per retrieval round. The test fixture stays mock-only —
//! no Ollama / fastembed / LanceDB.
//!
//! Coverage:
//!
//! 1. `decide_stop_triggers_synthesize` — decide returns `[]`,
//!    pipeline transitions straight to synthesize.
//! 2. `decide_continue_adds_more_chunks` — decide returns
//!    `["q2"]`, iter 2 retrieves and grows the pool.
//! 3. `max_depth_force_stops` — `multi_hop_max_depth = 1` forces
//!    `forced_stop = true` on the depth-1 decide hop and skips the
//!    decide LLM call.
//! 4. `pool_chunks_dedup_by_chunk_id` — two sub-queries return the
//!    same chunk; pool dedups by `chunk_id`.
//! 5. `decide_parse_failure_falls_through_to_synthesize` — decide
//!    LLM emits non-JSON garbage; pipeline graceful-degrades to
//!    synthesize (NOT a refusal).

mod common;

use std::sync::Arc;

use common::{RagEnv, ScriptedLm, ScriptedRetriever, id32, mk_hit};
use kebab_core::{HopKind, LanguageModel, RefusalReason, Retriever, SearchMode};
use kebab_rag::{AskOpts, RagPipeline};

/// Default `AskOpts` for multi-hop tests: deterministic seed,
/// lexical mode (so the test crate doesn't need to wire up an
/// embedder), and `multi_hop: true` to route through
/// `ask_multi_hop`.
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

// ── 1. decide returns [] → synthesize immediately ─────────────────────────

#[test]
fn multi_hop_decide_stop_triggers_synthesize() {
    let env = RagEnv::new();
    let cid = id32("c1");
    let did = id32("d1");
    env.seed_chunk(&cid, &did, "notes/a.md", "Body text.", &["Intro"]);
    let hits = vec![mk_hit(1, &cid, &did, "notes/a.md", 0.85, &["Intro"])];
    let retriever = Arc::new(ScriptedRetriever::new(vec![hits]));
    let retriever_handle = retriever.clone();
    let retriever_dyn: Arc<dyn Retriever> = retriever;

    // Three LLM calls in order: decompose → decide → synthesize.
    let lm = Arc::new(ScriptedLm::new(vec![
        r#"["q1"]"#,
        r#"[]"#,
        "answer body [#1]",
    ]));
    let lm_handle = lm.clone();
    let lm_dyn: Arc<dyn LanguageModel> = lm;
    let pipeline =
        RagPipeline::new(env.config.clone(), retriever_dyn, lm_dyn, env.sqlite.clone());

    let answer = pipeline.ask("compound", multi_hop_opts()).unwrap();

    assert!(answer.grounded, "decide-stop synthesize must be grounded");
    assert_eq!(answer.refusal_reason, None);
    assert_eq!(
        lm_handle.calls(),
        3,
        "decompose + decide + synthesize = 3 LLM calls"
    );
    assert_eq!(
        retriever_handle.calls(),
        1,
        "single sub-query → single retrieval"
    );

    let hops = answer.hops.expect("multi-hop happy path stamps Some(hops)");
    assert_eq!(hops.len(), 3, "[Decompose, Decide(stop), Synthesize]");
    assert_eq!(hops[0].kind, HopKind::Decompose);
    assert_eq!(hops[0].sub_queries, vec!["q1"]);
    assert_eq!(hops[1].kind, HopKind::Decide);
    assert!(
        hops[1].sub_queries.is_empty(),
        "decide stop signal → empty sub_queries on the HopRecord"
    );
    assert!(
        !hops[1].forced_stop,
        "LLM stop signal is NOT a forced_stop (forced_stop = cap-driven only)"
    );
    assert_eq!(hops[2].kind, HopKind::Synthesize);
}

// ── 2. decide ["q2"] → iter 2 retrieves and grows the pool ────────────────

#[test]
fn multi_hop_decide_continue_adds_more_chunks() {
    let env = RagEnv::new();
    let cid1 = id32("c1");
    let did1 = id32("d1");
    let cid2 = id32("c2");
    let did2 = id32("d2");
    env.seed_chunk(&cid1, &did1, "notes/a.md", "Chunk one.", &["A"]);
    env.seed_chunk(&cid2, &did2, "notes/b.md", "Chunk two.", &["B"]);
    // iter 1 retrieves chunk 1; iter 2 retrieves chunk 2 (different
    // chunk_id → pool grows).
    let retriever = Arc::new(ScriptedRetriever::new(vec![
        vec![mk_hit(1, &cid1, &did1, "notes/a.md", 0.85, &["A"])],
        vec![mk_hit(1, &cid2, &did2, "notes/b.md", 0.80, &["B"])],
    ]));
    let retriever_handle = retriever.clone();
    let retriever_dyn: Arc<dyn Retriever> = retriever;

    let lm = Arc::new(ScriptedLm::new(vec![
        r#"["q1"]"#,
        r#"["q2"]"#,
        r#"[]"#,
        "synthesized [#1] [#2]",
    ]));
    let lm_handle = lm.clone();
    let lm_dyn: Arc<dyn LanguageModel> = lm;
    let pipeline =
        RagPipeline::new(env.config.clone(), retriever_dyn, lm_dyn, env.sqlite.clone());

    let answer = pipeline.ask("compound", multi_hop_opts()).unwrap();

    assert!(answer.grounded);
    assert_eq!(answer.refusal_reason, None);
    assert_eq!(
        lm_handle.calls(),
        4,
        "decompose + 2 decide + synthesize = 4 LLM calls"
    );
    assert_eq!(
        retriever_handle.calls(),
        2,
        "iter 1 retrieves q1, iter 2 retrieves q2"
    );
    assert_eq!(
        answer.retrieval.chunks_returned, 2,
        "pool accumulates one new chunk per iter"
    );

    let hops = answer.hops.expect("happy path stamps hops");
    assert_eq!(hops.len(), 4, "[Decompose, Decide(continue), Decide(stop), Synthesize]");
    assert_eq!(hops[0].kind, HopKind::Decompose);
    assert_eq!(hops[1].kind, HopKind::Decide);
    assert_eq!(hops[1].sub_queries, vec!["q2"], "iter 1 decide emits q2");
    assert_eq!(
        hops[1].context_chunks_added, 1,
        "iter 1 retrieve added chunk 1"
    );
    assert_eq!(hops[2].kind, HopKind::Decide);
    assert!(hops[2].sub_queries.is_empty(), "iter 2 decide signals stop");
    assert_eq!(
        hops[2].context_chunks_added, 1,
        "iter 2 retrieve added chunk 2"
    );
    assert_eq!(hops[3].kind, HopKind::Synthesize);
}

// ── 3. max_depth=1 → forced_stop, decide LLM call skipped ─────────────────

#[test]
fn multi_hop_max_depth_force_stops() {
    let env = RagEnv::new();
    let cid = id32("c1");
    let did = id32("d1");
    env.seed_chunk(&cid, &did, "notes/a.md", "Body text.", &["Intro"]);
    let mut cfg = env.config.clone();
    // depth 1 means: iter 1 is the last iter, so the per-iter
    // `depth_force_stop = iter >= max_depth` fires and the decide
    // LLM call is skipped entirely.
    cfg.rag.multi_hop_max_depth = 1;

    let hits = vec![mk_hit(1, &cid, &did, "notes/a.md", 0.85, &["Intro"])];
    let retriever = Arc::new(ScriptedRetriever::new(vec![hits]));
    let retriever_handle = retriever.clone();
    let retriever_dyn: Arc<dyn Retriever> = retriever;

    // Only 2 LLM calls scripted — decompose + synthesize. If the
    // pipeline tries to call decide (a bug), ScriptedLm panics on
    // exhaustion and the test fails loudly with the call index.
    let lm = Arc::new(ScriptedLm::new(vec![
        r#"["q1"]"#,
        "answer [#1]",
    ]));
    let lm_handle = lm.clone();
    let lm_dyn: Arc<dyn LanguageModel> = lm;
    let pipeline = RagPipeline::new(cfg, retriever_dyn, lm_dyn, env.sqlite.clone());

    let answer = pipeline.ask("q", multi_hop_opts()).unwrap();

    assert!(answer.grounded);
    assert_eq!(
        lm_handle.calls(),
        2,
        "depth-cap skips decide → only decompose + synthesize"
    );
    assert_eq!(retriever_handle.calls(), 1);

    let hops = answer.hops.expect("happy path stamps hops");
    assert_eq!(hops.len(), 3, "[Decompose, Decide(forced_stop), Synthesize]");
    assert_eq!(hops[1].kind, HopKind::Decide);
    assert!(
        hops[1].forced_stop,
        "depth cap must surface forced_stop=true on the Decide hop"
    );
    assert!(
        hops[1].sub_queries.is_empty(),
        "skipped decide carries no sub_queries"
    );
    assert_eq!(
        hops[1].llm_call_ms, 0,
        "skipped decide records 0ms — no LLM call happened"
    );
}

// ── 4. dedup: two sub-queries hit same chunk_id, pool keeps 1 ─────────────

#[test]
fn multi_hop_pool_chunks_dedup_by_chunk_id() {
    let env = RagEnv::new();
    let cid = id32("c1");
    let did = id32("d1");
    env.seed_chunk(&cid, &did, "notes/a.md", "Shared chunk text.", &["X"]);
    // Both sub-queries retrieve the same chunk_id — dedup must
    // keep exactly one pool entry.
    let shared_hit = mk_hit(1, &cid, &did, "notes/a.md", 0.85, &["X"]);
    let retriever = Arc::new(ScriptedRetriever::new(vec![
        vec![shared_hit.clone()],
        vec![shared_hit],
    ]));
    let retriever_handle = retriever.clone();
    let retriever_dyn: Arc<dyn Retriever> = retriever;

    let lm = Arc::new(ScriptedLm::new(vec![
        r#"["q1", "q2"]"#,
        r#"[]"#,
        "merged answer [#1]",
    ]));
    let lm_handle = lm.clone();
    let lm_dyn: Arc<dyn LanguageModel> = lm;
    let pipeline =
        RagPipeline::new(env.config.clone(), retriever_dyn, lm_dyn, env.sqlite.clone());

    let answer = pipeline.ask("q", multi_hop_opts()).unwrap();

    assert!(answer.grounded);
    assert_eq!(
        retriever_handle.calls(),
        2,
        "two sub-queries → two retrieval calls"
    );
    assert_eq!(
        answer.retrieval.chunks_returned, 1,
        "dedup by chunk_id keeps pool at 1"
    );
    assert_eq!(answer.citations.len(), 1, "only one chunk cited as [#1]");
    assert_eq!(answer.citations[0].marker.as_deref(), Some("[1]"));
    assert_eq!(
        lm_handle.calls(),
        3,
        "decompose + decide + synthesize = 3"
    );

    let hops = answer.hops.expect("happy path stamps hops");
    assert_eq!(hops.len(), 3, "[Decompose, Decide, Synthesize]");
    assert_eq!(hops[0].sub_queries, vec!["q1", "q2"]);
    assert_eq!(
        hops[1].context_chunks_added, 1,
        "dedup reduces 2 retrievals → 1 new pool entry"
    );
}

// ── 5. decide parse failure → graceful synthesize (NOT a refusal) ─────────

#[test]
fn multi_hop_decide_parse_failure_falls_through_to_synthesize() {
    let env = RagEnv::new();
    let cid = id32("c1");
    let did = id32("d1");
    env.seed_chunk(&cid, &did, "notes/a.md", "Body text.", &["Intro"]);
    let hits = vec![mk_hit(1, &cid, &did, "notes/a.md", 0.85, &["Intro"])];
    let retriever = Arc::new(ScriptedRetriever::new(vec![hits]));
    let retriever_dyn: Arc<dyn Retriever> = retriever;

    // Decide LLM emits non-JSON garbage. Spec §9: this is NOT a
    // refusal — pipeline graceful-degrades to synthesize as if the
    // decide had returned `[]`. Only the *initial* decompose's
    // parse failure is a refusal (MultiHopDecomposeFailed).
    let lm = Arc::new(ScriptedLm::new(vec![
        r#"["q1"]"#,
        "definitely not a JSON array",
        "answer [#1]",
    ]));
    let lm_handle = lm.clone();
    let lm_dyn: Arc<dyn LanguageModel> = lm;
    let pipeline =
        RagPipeline::new(env.config.clone(), retriever_dyn, lm_dyn, env.sqlite.clone());

    let answer = pipeline.ask("q", multi_hop_opts()).unwrap();

    assert!(
        answer.grounded,
        "decide parse failure must NOT block synthesis"
    );
    assert_eq!(
        answer.refusal_reason, None,
        "decide parse failure is graceful degrade, not refusal — \
         MultiHopDecomposeFailed is reserved for the initial decompose hop"
    );
    assert_eq!(
        lm_handle.calls(),
        3,
        "decompose + (garbage) decide + synthesize"
    );

    let hops = answer.hops.expect("happy path stamps hops");
    assert_eq!(hops.len(), 3, "[Decompose, Decide(parse-fail→stop), Synthesize]");
    assert_eq!(hops[1].kind, HopKind::Decide);
    assert!(
        hops[1].sub_queries.is_empty(),
        "parse failure → empty sub_queries (same shape as LLM stop)"
    );
    assert!(
        !hops[1].forced_stop,
        "parse-degraded decide is not a cap-driven forced_stop — \
         flag stays false even though we synthesize early"
    );
}

// ── 6. refuse path: NoChunks preserves partial hop trace ──────────────────
//
// PR-3b-ii widens `refuse_no_chunks` to accept `hops:
// Option<Vec<HopRecord>>` and wires `ask_multi_hop` to forward the
// partial trace. This test pins that contract — a refusal Answer
// still carries the decompose + decide hops the loop accumulated
// before pool came up empty.

#[test]
fn multi_hop_refuse_no_chunks_preserves_hops_trace() {
    let env = RagEnv::new();
    // Retriever returns 0 hits — pool stays empty → refuse_no_chunks.
    let retriever = Arc::new(ScriptedRetriever::new(vec![vec![]]));
    let retriever_handle = retriever.clone();
    let retriever_dyn: Arc<dyn Retriever> = retriever;

    // Only one LM call needed (decompose). Decide is skipped because
    // `pool.is_empty()` triggers the (Vec::new(), 0) shortcut. If a
    // bug calls the LM beyond decompose, ScriptedLm panics on
    // exhaustion and the test fails loudly.
    let lm = Arc::new(ScriptedLm::new(vec![r#"["q1"]"#]));
    let lm_handle = lm.clone();
    let lm_dyn: Arc<dyn LanguageModel> = lm;
    let pipeline =
        RagPipeline::new(env.config.clone(), retriever_dyn, lm_dyn, env.sqlite.clone());

    let answer = pipeline.ask("q", multi_hop_opts()).unwrap();

    assert!(!answer.grounded);
    assert_eq!(answer.refusal_reason, Some(RefusalReason::NoChunks));
    assert_eq!(retriever_handle.calls(), 1, "single sub-query → single retrieve");
    assert_eq!(lm_handle.calls(), 1, "decompose only — decide skipped (empty pool), no synthesize");

    let hops = answer
        .hops
        .expect("PR-3b-ii: refuse_no_chunks must preserve the partial hop trace");
    assert_eq!(
        hops.len(),
        2,
        "[Decompose, Decide(empty_pool_skip)] — synthesize never ran"
    );
    assert_eq!(hops[0].kind, HopKind::Decompose);
    assert_eq!(hops[0].sub_queries, vec!["q1"]);
    assert_eq!(hops[1].kind, HopKind::Decide);
    assert!(hops[1].sub_queries.is_empty());
    assert_eq!(
        hops[1].context_chunks_added, 0,
        "retrieve returned 0 hits → 0 added to pool"
    );
}

// ── 7. refuse path: ScoreGate preserves partial hop trace ─────────────────

#[test]
fn multi_hop_refuse_score_gate_preserves_hops_trace() {
    let env = RagEnv::new();
    let cid = i32_below_gate_chunk(&env);
    // Top score 0.10 is well below the default gate (0.30) — the
    // score-gate refusal fires after the pool has been built, so
    // the decide LLM call did run and the hop trace contains both
    // Decompose and Decide entries.
    let hits = vec![mk_hit(1, &cid, &id32("d_low"), "notes/low.md", 0.10, &["Low"])];
    let retriever = Arc::new(ScriptedRetriever::new(vec![hits]));
    let retriever_dyn: Arc<dyn Retriever> = retriever;

    // decompose + decide (pool not empty so decide fires) — synthesize
    // never runs because we refuse before pack_context.
    let lm = Arc::new(ScriptedLm::new(vec![
        r#"["q1"]"#,
        r#"[]"#,
    ]));
    let lm_handle = lm.clone();
    let lm_dyn: Arc<dyn LanguageModel> = lm;
    let pipeline =
        RagPipeline::new(env.config.clone(), retriever_dyn, lm_dyn, env.sqlite.clone());

    let answer = pipeline.ask("q", multi_hop_opts()).unwrap();

    assert!(!answer.grounded);
    assert_eq!(answer.refusal_reason, Some(RefusalReason::ScoreGate));
    assert_eq!(
        lm_handle.calls(),
        2,
        "decompose + decide ran; synthesize skipped by gate"
    );

    let hops = answer
        .hops
        .expect("PR-3b-ii: refuse_score_gate must preserve the partial hop trace");
    assert_eq!(
        hops.len(),
        2,
        "[Decompose, Decide(stop)] — synthesize never ran"
    );
    assert_eq!(hops[0].kind, HopKind::Decompose);
    assert_eq!(hops[1].kind, HopKind::Decide);
    assert_eq!(
        hops[1].context_chunks_added, 1,
        "the low-score chunk did enter the pool — gate fires after pool build"
    );
}

/// Seed a chunk + return its id. Helper for the score-gate test so
/// the test body stays focused on the hop-trace assertions.
fn i32_below_gate_chunk(env: &RagEnv) -> String {
    let cid = id32("c_low");
    let did = id32("d_low");
    env.seed_chunk(&cid, &did, "notes/low.md", "low score text", &["Low"]);
    cid
}
