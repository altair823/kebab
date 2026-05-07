//! `schema` tool — wraps `kebab_app::schema_with_config`.
//! Input: {} (no args). Output: schema.v1 JSON.

use rmcp::model::CallToolResult;
use serde::{Deserialize, Serialize};
use schemars::JsonSchema;

use crate::error::{to_tool_error, to_tool_success};
use crate::state::KebabAppState;

#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
pub struct SchemaInput {}

pub fn handle(state: &KebabAppState, _input: SchemaInput) -> CallToolResult {
    match kebab_app::schema_with_config(&state.config) {
        Ok(report) => match serde_json::to_string(&report) {
            Ok(json) => to_tool_success(json),
            Err(e) => to_tool_error(&anyhow::anyhow!(e)),
        },
        Err(e) => to_tool_error(&e),
    }
}
