//! `doctor` tool — wraps `kebab_app::doctor_with_config_path`.
//! Input: {} (no args). Output: doctor.v1 JSON.
//!
//! `doctor_with_config_path(Option<&Path>)` re-reads config from disk so
//! the report reflects the live file state. We forward `config_path` from
//! `KebabAppState` so `--config <path>` users see results for their file;
//! callers that pass `None` fall back to the XDG default (same as the CLI
//! bare `kebab doctor`).

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{to_tool_error, to_tool_success};
use crate::state::KebabAppState;

#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
pub struct DoctorInput {}

pub fn handle(state: &KebabAppState, _input: DoctorInput) -> CallToolResult {
    match kebab_app::doctor_with_config_path(state.config_path.as_deref()) {
        Ok(report) => match serde_json::to_string(&report) {
            Ok(json) => to_tool_success(json),
            Err(e) => to_tool_error(&anyhow::anyhow!(e)),
        },
        Err(e) => to_tool_error(&e),
    }
}
