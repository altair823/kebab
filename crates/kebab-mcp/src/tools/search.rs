//! `search` tool — wraps `kebab_app::search_with_opts_with_config`.
//! Input: { query, mode?, k?, max_tokens?, snippet_chars?, cursor? }.
//! Output: search_response.v1 envelope (hits + next_cursor + truncated).
//!
//! First tool with a non-empty `inputSchema`: `SearchInput` derives
//! `JsonSchema` and `Tool::new` uses
//! `rmcp::handler::server::common::schema_for_type::<SearchInput>()`.

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{to_tool_error, to_tool_success};
use crate::state::KebabAppState;

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SearchInput {
    /// User query (free text).
    pub query: String,
    /// Retrieval mode: "hybrid" (default), "lexical", or "vector".
    pub mode: Option<String>,
    /// Top-K results. Defaults to 10. Clamped to 1–100.
    pub k: Option<usize>,
    /// p9-fb-34: cap result wire size at ~N tokens (chars/4 estimate).
    pub max_tokens: Option<usize>,
    /// p9-fb-34: per-hit snippet character cap.
    pub snippet_chars: Option<usize>,
    /// p9-fb-34: opaque cursor from a previous response.
    pub cursor: Option<String>,
}

pub fn handle(state: &KebabAppState, input: SearchInput) -> CallToolResult {
    let k = input.k.unwrap_or(10).clamp(1, 100);
    let mode_str = input.mode.as_deref().unwrap_or("hybrid");
    let mode = match mode_str {
        "lexical" => kebab_core::SearchMode::Lexical,
        "vector" => kebab_core::SearchMode::Vector,
        _ => kebab_core::SearchMode::Hybrid,
    };
    let query = kebab_core::SearchQuery {
        text: input.query,
        mode,
        k,
        filters: kebab_core::SearchFilters::default(),
    };
    let opts = kebab_core::SearchOpts {
        max_tokens: input.max_tokens,
        snippet_chars: input.snippet_chars,
        cursor: input.cursor,
    };
    let cfg_clone = (*state.config).clone();
    match kebab_app::search_with_opts_with_config(cfg_clone, query, opts) {
        Ok(resp) => {
            // SearchHit (kebab-core) does not carry a `schema_version` field,
            // so we tag each element inline before serialising.
            let tagged: Vec<serde_json::Value> = resp
                .hits
                .iter()
                .map(|h| {
                    let mut v = serde_json::to_value(h).unwrap_or_default();
                    if let serde_json::Value::Object(ref mut map) = v {
                        map.insert(
                            "schema_version".to_string(),
                            serde_json::Value::String("search_hit.v1".to_string()),
                        );
                    }
                    v
                })
                .collect();
            let envelope = serde_json::json!({
                "schema_version": "search_response.v1",
                "hits": tagged,
                "next_cursor": resp.next_cursor,
                "truncated": resp.truncated,
            });
            match serde_json::to_string(&envelope) {
                Ok(json) => to_tool_success(json),
                Err(e) => to_tool_error(&anyhow::anyhow!(e)),
            }
        }
        Err(e) => to_tool_error(&e),
    }
}
