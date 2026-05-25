//! `ask` tool — wraps `kebab_app::ask_with_config` (single-shot) or
//! `kebab_app::ask_with_session_with_config` when `session_id` is provided.
//! Input: { query, session_id?, mode? }. Output: answer.v1 JSON.
//!
//! `Answer` (kebab-core) does NOT carry a `schema_version` field; we tag
//! it inline here, matching the pattern from `search.rs`.

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{to_tool_error, to_tool_success};
use crate::state::KebabAppState;

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct AskInput {
    /// The user question.
    pub query: String,
    /// Optional session id for multi-turn RAG context.
    pub session_id: Option<String>,
    /// Optional retrieval mode override ("lexical" / "vector" / "hybrid"). Default "hybrid".
    pub mode: Option<String>,
    /// p9-fb-41: opt the ask into the multi-hop pipeline. Default `false`.
    /// When `true`, the query is decomposed into sub-questions, each
    /// retrieved independently, then synthesized over the merged
    /// chunk pool. Cost trade-off: 2–5× LLM calls vs. single-pass.
    /// Use for compound questions / cross-doc reasoning / prereq
    /// chains; keep `false` for simple fact lookups. The full
    /// per-hop trace (`decompose` / `decide` / `synthesize`) is
    /// exposed on `Answer.hops`.
    pub multi_hop: Option<bool>,
}

pub fn handle(state: &KebabAppState, input: AskInput) -> CallToolResult {
    let mode = match input.mode.as_deref() {
        Some("lexical") => kebab_core::SearchMode::Lexical,
        Some("vector") => kebab_core::SearchMode::Vector,
        _ => kebab_core::SearchMode::Hybrid, // default + "hybrid" + unknown
    };
    let opts = kebab_app::AskOpts {
        k: 10,
        explain: false,
        mode,
        temperature: None,
        seed: None,
        stream_sink: None,
        history: Vec::new(),
        conversation_id: None,
        turn_index: None,
        multi_hop: input.multi_hop.unwrap_or(false),
    };
    let cfg_clone = (*state.config).clone();
    let result = match input.session_id {
        Some(sid) => {
            kebab_app::ask_with_session_with_config(cfg_clone, &sid, &input.query, opts)
        }
        None => kebab_app::ask_with_config(cfg_clone, &input.query, opts),
    };
    match result {
        Ok(answer) => {
            // `Answer` does not carry `schema_version`; tag inline (idempotent
            // via entry().or_insert_with in case a future version adds it).
            let mut v = match serde_json::to_value(&answer) {
                Ok(v) => v,
                Err(e) => return to_tool_error(&anyhow::anyhow!("answer serialize failed: {e}")),
            };
            if let serde_json::Value::Object(ref mut map) = v {
                map.entry("schema_version".to_string())
                    .or_insert_with(|| serde_json::Value::String("answer.v1".to_string()));
            }
            match serde_json::to_string(&v) {
                Ok(json) => to_tool_success(json),
                Err(e) => to_tool_error(&anyhow::anyhow!(e)),
            }
        }
        Err(e) => to_tool_error(&e),
    }
}
