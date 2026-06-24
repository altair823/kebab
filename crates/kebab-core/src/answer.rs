//! Answer + RAG types (§3.8).

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::citation::Citation;
use crate::search::SearchMode;
use crate::versions::PromptTemplateVersion;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Answer {
    pub answer: String,
    pub citations: Vec<AnswerCitation>,
    pub grounded: bool,
    pub refusal_reason: Option<RefusalReason>,
    pub model: ModelRef,
    pub embedding: Option<ModelRef>,
    pub prompt_template_version: PromptTemplateVersion,
    pub retrieval: AnswerRetrievalSummary,
    pub usage: TokenUsage,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    /// p9-fb-41: multi-hop hop trace. `None` for single-pass asks.
    /// Each entry records one hop (`decompose` / `decide` / `synthesize`)
    /// — the LLM call category, the sub-queries emitted, retrieval
    /// counts, and a `forced_stop` flag for cap-driven termination.
    /// Wire-additive: `answer.v1` schema_version unchanged; consumers
    /// reading older single-pass answers see `hops: None` (or absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hops: Option<Vec<HopRecord>>,
    /// p9-fb-41 PR-9c-1: NLI-based post-synthesis verification summary.
    /// `None` for single-pass asks and for multi-hop runs with
    /// `[rag].nli_threshold == 0` (verification disabled — the default).
    /// Present only when the multi-hop pipeline reached the post-
    /// synthesize verification step (PR-9c-2 wires step 8.5). Wire-
    /// additive: `answer.v1` schema_version unchanged; consumers
    /// reading pre-v0.18 answers see `verification: None` (or absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification: Option<VerificationSummary>,
}

/// p9-fb-41 PR-9c-1: post-synthesize NLI verification summary stamped
/// onto [`Answer::verification`] when multi-hop runs reach step 8.5
/// (NLI gate). Three required fields ride together on every wire emit:
/// `nli_score` is the entailment channel of the XNLI verifier,
/// `nli_threshold` mirrors `[rag].nli_threshold` for audit, and
/// `nli_passed` is `nli_score >= nli_threshold`. The whole struct is
/// omitted (serde skip) when no verification ran.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct VerificationSummary {
    pub nli_score: f32,
    pub nli_threshold: f32,
    pub nli_passed: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnswerCitation {
    pub marker: Option<String>,
    pub citation: Citation,
    /// p9-fb-32: cited doc's `documents.updated_at`.
    #[serde(with = "time::serde::rfc3339")]
    pub indexed_at: OffsetDateTime,
    /// p9-fb-32: server-computed staleness flag per config threshold.
    pub stale: bool,
}

/// p9-fb-41: one entry in [`Answer::hops`] — the per-iteration trace
/// of a multi-hop ask. The pipeline appends a `HopRecord` per LLM
/// call (decompose / decide / synthesize) so a `--multi-hop` user
/// can see what sub-queries the LLM emitted, how many chunks each
/// hop contributed, whether the iter stopped on the model's own
/// signal or hit a cap, and the per-hop LLM latency.
///
/// Wire-additive — every field uses `#[serde(default)]` where it
/// could plausibly be omitted by a future schema reader.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HopRecord {
    /// 0-based hop index within this ask. `iter=0` is always the
    /// initial decompose call; subsequent iters are decide calls;
    /// the final iter is the synthesize call.
    pub iter: u32,
    pub kind: HopKind,
    /// Sub-queries associated with this hop. The meaning depends on
    /// `kind`:
    ///
    /// - [`HopKind::Decompose`]: the initial sub-queries the LLM
    ///   broke the original user query into. These drive the
    ///   `iter=1` retrieval round.
    /// - [`HopKind::Decide`]: the *new* sub-queries the LLM
    ///   emitted to drive the next retrieval round. Empty when the
    ///   LLM signalled stop OR when `forced_stop = true` (cap hit
    ///   or parse-degraded).
    /// - [`HopKind::Synthesize`]: always empty — the final hop
    ///   produces the user-visible answer, not more sub-queries.
    #[serde(default)]
    pub sub_queries: Vec<String>,
    /// Number of *new* chunks the retrieval round contributed to the
    /// pool (dedup'd by `chunk_id` — repeated hits from a previous
    /// iter do not count). `0` for the decompose hop (no retrieval
    /// yet) and the synthesize hop.
    pub context_chunks_added: u32,
    /// `true` when the pipeline cut the iter loop short because a
    /// safety cap fired (`max_depth` / `max_total_sub_queries` /
    /// `max_pool_chunks`) rather than because the LLM signalled
    /// stop. The user-visible answer still reflects all chunks
    /// accumulated up to that point — `forced_stop` is a tracing
    /// signal, not a refusal.
    pub forced_stop: bool,
    /// Wall-clock latency of the LLM call for this hop, in
    /// milliseconds. Useful for cost / latency analysis when a
    /// `kebab eval` run records `Answer.hops`.
    ///
    /// `0` is overloaded: it means "no LLM call happened at this
    /// hop" when (a) the hop was a Decide skipped due to
    /// `forced_stop` (depth-cap or pool-cap fired before the LLM
    /// was asked) or (b) the pool was empty before any decide
    /// could run. Treat `0` as "absent or instantaneous" rather
    /// than as a genuine measurement.
    pub llm_call_ms: u32,
}

/// p9-fb-41: which stage of the multi-hop pipeline a [`HopRecord`]
/// describes. The serde tag matches the wire shape so agents /
/// CLIs can branch on the snake_case string without referencing
/// the Rust enum.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HopKind {
    /// First hop — LLM decomposed the user query into sub-queries.
    Decompose,
    /// Subsequent hop — LLM was asked whether more retrieval is
    /// needed and either emitted new sub-queries (`continue`) or
    /// returned an empty array (`stop`).
    Decide,
    /// Terminal hop — LLM produced the final user-visible answer
    /// over the accumulated chunk pool.
    Synthesize,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefusalReason {
    ScoreGate,
    LlmSelfJudge,
    NoIndex,
    NoChunks,
    /// p9-fb-15: ask 가 LLM 토큰 stream 도중 cancel 됨. partial answer
    /// 가 채워져 있을 수 있음 (사용자가 본 부분까지). RAG retrieval
    /// 자체는 정상 — 모델 generation 단계에서만 중단.
    LlmStreamAborted,
    /// p9-fb-41: multi-hop pipeline 의 decompose LLM call 이 JSON
    /// parse 가능한 sub-question array 를 반환하지 못함 (parse
    /// error, 빈 응답, 또는 잘못된 형식). retrieval / synthesize
    /// 단계 진입 못 함. CLI / MCP / TUI 가 받는 wire error code
    /// = `"multi_hop_decompose_failed"` (PR-4 의 error_wire 매핑).
    MultiHopDecomposeFailed,
    /// p9-fb-41 PR-9c-1: post-synthesize NLI verification gate fired —
    /// `NliScores::faithfulness()` (entailment channel) fell below
    /// `[rag].nli_threshold`. Wire string = `"nli_verification_failed"`
    /// (single source of truth: also the matching `error.v1.code`).
    /// Multi-hop only; behavior wiring lands in PR-9c-2.
    NliVerificationFailed,
    /// p9-fb-41 PR-9c-1: NLI verifier was configured (threshold > 0)
    /// but the model / runtime is unavailable (download failure,
    /// missing tokenizer, ONNX session init error). Treated as a soft
    /// refusal — the user sees an unverified-answer outcome rather
    /// than crashing the ask. Wire string = `"nli_model_unavailable"`.
    NliModelUnavailable,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelRef {
    pub id: String,
    pub provider: String,
    pub dimensions: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnswerRetrievalSummary {
    pub trace_id: TraceId,
    pub mode: SearchMode,
    pub k: usize,
    pub score_gate: f32,
    pub top_score: f32,
    pub chunks_returned: u32,
    pub chunks_used: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TokenUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub latency_ms: u32,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct TraceId(pub String);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asset::WorkspacePath;
    use crate::citation::Citation;
    use time::macros::datetime;

    /// p9-fb-41 PR-9c-1: pin the wire-side spelling of the new
    /// `RefusalReason` variants. The strings here must match
    /// `answer.schema.json::refusal_reason.enum` AND
    /// `error.schema.json::code.enum` byte-for-byte (single source of
    /// truth per spec §2.4).
    #[test]
    fn refusal_reason_nli_variants_serialize_to_snake_case() {
        assert_eq!(
            serde_json::to_string(&RefusalReason::NliVerificationFailed).unwrap(),
            "\"nli_verification_failed\""
        );
        assert_eq!(
            serde_json::to_string(&RefusalReason::NliModelUnavailable).unwrap(),
            "\"nli_model_unavailable\""
        );
    }

    /// p9-fb-41 PR-9c-1: `Answer.verification` is `Option<...>` with
    /// `skip_serializing_if = None`. A `verification: None` answer
    /// must NOT emit a `"verification"` key on the wire — the field
    /// is additive and pre-v0.18 readers see no new key.
    #[test]
    fn answer_omits_verification_field_when_none() {
        let ans = Answer {
            answer: "x".into(),
            citations: vec![],
            grounded: true,
            refusal_reason: None,
            model: ModelRef {
                id: "m".into(),
                provider: "p".into(),
                dimensions: None,
            },
            embedding: None,
            prompt_template_version: PromptTemplateVersion("rag-v2".into()),
            retrieval: AnswerRetrievalSummary {
                trace_id: TraceId("t".into()),
                mode: crate::SearchMode::Lexical,
                k: 1,
                score_gate: 0.0,
                top_score: 0.0,
                chunks_returned: 0,
                chunks_used: 0,
            },
            usage: TokenUsage {
                prompt_tokens: 0,
                completion_tokens: 0,
                latency_ms: 0,
            },
            created_at: datetime!(2026-05-09 12:00:00 UTC),
            hops: None,
            verification: None,
        };
        let v = serde_json::to_value(&ans).unwrap();
        assert!(
            v.get("verification").is_none(),
            "verification: None must be omitted from wire output, got: {v}"
        );
    }

    #[test]
    fn verification_summary_serializes_all_three_required_fields() {
        let vs = VerificationSummary {
            nli_score: 0.87,
            nli_threshold: 0.5,
            nli_passed: true,
        };
        let v = serde_json::to_value(vs).unwrap();
        assert!((v["nli_score"].as_f64().unwrap() - 0.87).abs() < 1e-5);
        assert!((v["nli_threshold"].as_f64().unwrap() - 0.5).abs() < 1e-5);
        assert_eq!(v["nli_passed"], true);
    }

    #[test]
    fn answer_citation_serializes_indexed_at_and_stale() {
        let ac = AnswerCitation {
            marker: Some("[1]".to_string()),
            citation: Citation::Line {
                path: WorkspacePath::new("a.md".to_string()).unwrap(),
                start: 1,
                end: 1,
                section: None,
            },
            indexed_at: datetime!(2026-05-09 12:00:00 UTC),
            stale: false,
        };
        let v = serde_json::to_value(&ac).unwrap();
        assert_eq!(v["indexed_at"], "2026-05-09T12:00:00Z");
        assert_eq!(v["stale"], false);
    }
}
