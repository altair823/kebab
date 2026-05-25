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
    /// p9-fb-15: same conversation 의 turn 들이 공유. CLI single-shot
    /// (history 없음) / TUI 첫 turn 은 None. blake3 해시 또는 사용자
    /// 명시 (`kebab ask --session <id>`, p9-fb-18).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    /// p9-fb-15: 같은 conversation 안 0-based 순서. 첫 turn = 0. None
    /// 이면 single-shot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_index: Option<u32>,
    /// p9-fb-41: multi-hop hop trace. `None` for single-pass asks.
    /// Each entry records one hop (`decompose` / `decide` / `synthesize`)
    /// — the LLM call category, the sub-queries emitted, retrieval
    /// counts, and a `forced_stop` flag for cap-driven termination.
    /// Wire-additive: `answer.v1` schema_version unchanged; consumers
    /// reading older single-pass answers see `hops: None` (or absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hops: Option<Vec<HopRecord>>,
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

/// p9-fb-15: history 가 prompt 에 들어갈 때의 한 turn. RAG facade 가
/// `Vec<Turn>` 받아 system + history + retrieval + new question 으로
/// prompt 빌드. token budget 안에 fit 안 되면 oldest turn 부터 drop
/// (newest 우선 보존).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Turn {
    pub question: String,
    pub answer: String,
    pub citations: Vec<AnswerCitation>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
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
