//! p9-fb-35 `fetch` tool — wraps `kebab_app::fetch_with_config`.
//!
//! Three modes (chunk / doc / span). Output is `fetch_result.v1`.
//!
//! Mirrors the CLI surface (`kebab fetch <kind> ...`): same input shape,
//! same wire envelope. Missing kind-specific fields produce an `error.v1`
//! with `code = "invalid_input"`.

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{to_tool_error, to_tool_success};
use crate::state::KebabAppState;

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct FetchInput {
    /// "chunk" | "doc" | "span"
    pub kind: String,
    /// Required when kind = "chunk".
    pub chunk_id: Option<String>,
    /// Required when kind = "doc" or "span".
    pub doc_id: Option<String>,
    /// Required when kind = "span" (1-based, inclusive).
    pub line_start: Option<u32>,
    pub line_end: Option<u32>,
    /// chunk only: ±N surrounding chunks.
    pub context: Option<u32>,
    /// doc/span only: chars/4 budget.
    pub max_tokens: Option<usize>,
}

pub fn handle(state: &KebabAppState, input: FetchInput) -> CallToolResult {
    let query = match input.kind.as_str() {
        "chunk" => match input.chunk_id {
            Some(id) => kebab_core::FetchQuery::Chunk(kebab_core::ChunkId(id)),
            None => return invalid_input("kind=chunk requires chunk_id"),
        },
        "doc" => match input.doc_id {
            Some(id) => kebab_core::FetchQuery::Doc(kebab_core::DocumentId(id)),
            None => return invalid_input("kind=doc requires doc_id"),
        },
        "span" => match (input.doc_id, input.line_start, input.line_end) {
            (Some(id), Some(start), Some(end)) => kebab_core::FetchQuery::Span {
                doc_id: kebab_core::DocumentId(id),
                line_start: start,
                line_end: end,
            },
            _ => return invalid_input("kind=span requires doc_id, line_start, line_end"),
        },
        other => {
            return invalid_input(&format!("unknown kind '{other}'; expected chunk|doc|span"));
        }
    };

    let opts = kebab_core::FetchOpts {
        context: input.context,
        max_tokens: input.max_tokens,
    };

    let cfg_clone = (*state.config).clone();
    match kebab_app::fetch_with_config(cfg_clone, query, opts) {
        Ok(r) => {
            // FetchResult does not carry a `schema_version` field, so we
            // tag the envelope inline (mirrors search.rs's pattern).
            let mut v = match serde_json::to_value(&r) {
                Ok(v) => v,
                Err(e) => {
                    return to_tool_error(&anyhow::anyhow!("FetchResult serialize: {e}"));
                }
            };
            if let serde_json::Value::Object(ref mut map) = v {
                map.insert(
                    "schema_version".to_string(),
                    serde_json::Value::String("fetch_result.v1".to_string()),
                );
            }
            match serde_json::to_string(&v) {
                Ok(json) => to_tool_success(json),
                Err(e) => to_tool_error(&anyhow::anyhow!(e)),
            }
        }
        Err(e) => to_tool_error(&e),
    }
}

fn invalid_input(msg: &str) -> CallToolResult {
    use kebab_app::{ErrorV1, StructuredError};
    let err = anyhow::Error::new(StructuredError(ErrorV1 {
        schema_version: "error.v1".to_string(),
        code: "invalid_input".to_string(),
        message: msg.to_string(),
        details: serde_json::Value::Null,
        hint: None,
    }));
    to_tool_error(&err)
}
