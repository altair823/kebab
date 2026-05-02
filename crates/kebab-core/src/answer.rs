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
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnswerCitation {
    pub marker: Option<String>,
    pub citation: Citation,
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
