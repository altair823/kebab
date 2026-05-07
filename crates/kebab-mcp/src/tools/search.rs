//! `search` tool — wraps `kebab_app::search_with_config`.
//! Input: { query, mode?, k? }. Output: search_hit.v1 array JSON.
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
    #[serde(default = "default_mode")]
    pub mode: String,
    /// Top-K results. Defaults to 10. Clamped to 1–100.
    #[serde(default = "default_k")]
    pub k: usize,
}

fn default_mode() -> String {
    "hybrid".to_string()
}
fn default_k() -> usize {
    10
}

pub fn handle(state: &KebabAppState, input: SearchInput) -> CallToolResult {
    let k = input.k.clamp(1, 100);
    let mode = match input.mode.as_str() {
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
    match kebab_app::search_with_config((*state.config).clone(), query) {
        Ok(hits) => {
            // SearchHit (kebab-core) does not carry a `schema_version` field,
            // so we tag each element inline before serialising.
            let tagged: Vec<serde_json::Value> = hits
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
            match serde_json::to_string(&serde_json::Value::Array(tagged)) {
                Ok(json) => to_tool_success(json),
                Err(e) => to_tool_error(&anyhow::anyhow!(e)),
            }
        }
        Err(e) => to_tool_error(&e),
    }
}
