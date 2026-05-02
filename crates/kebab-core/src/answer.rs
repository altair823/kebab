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
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnswerCitation {
    pub marker: Option<String>,
    pub citation: Citation,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefusalReason {
    ScoreGate,
    LlmSelfJudge,
    NoIndex,
    NoChunks,
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
