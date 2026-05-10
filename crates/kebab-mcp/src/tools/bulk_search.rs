//! `bulk_search` tool — wraps `kebab_app::bulk_search_with_config`.
//! Input: `{ queries: [<SearchInput shape>, ...] }`.
//! Output: `bulk_search_response.v1` envelope (results + summary).

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{to_tool_error, to_tool_success};
use crate::state::KebabAppState;

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct BulkSearchInput {
    /// Per-query inputs. Each item mirrors the single-query `search`
    /// tool's input shape — `query` is required, all other fields are
    /// optional and default to single-search defaults. Capped at 100
    /// items; exceeding returns an `invalid_input` tool error without
    /// running any query.
    pub queries: Vec<serde_json::Value>,
}

pub fn handle(state: &KebabAppState, input: BulkSearchInput) -> CallToolResult {
    let cfg_clone = (*state.config).clone();
    match kebab_app::bulk_search_with_config(cfg_clone, input.queries) {
        Ok((items, summary)) => {
            let tagged_items: Vec<serde_json::Value> = items
                .iter()
                .map(|it| {
                    let mut v = serde_json::to_value(it).unwrap_or(serde_json::Value::Null);
                    if let serde_json::Value::Object(ref mut map) = v {
                        map.insert(
                            "schema_version".to_string(),
                            serde_json::Value::String("bulk_search_item.v1".to_string()),
                        );
                    }
                    v
                })
                .collect();
            let envelope = serde_json::json!({
                "schema_version": "bulk_search_response.v1",
                "results": tagged_items,
                "summary": {
                    "total": summary.total,
                    "succeeded": summary.succeeded,
                    "failed": summary.failed,
                },
            });
            match serde_json::to_string(&envelope) {
                Ok(json) => to_tool_success(json),
                Err(e) => to_tool_error(&anyhow::anyhow!(e)),
            }
        }
        Err(e) => to_tool_error(&e),
    }
}
