//! `ingest_file` tool — wraps `kebab_app::ingest_file_with_config`.
//! Input: { path }. Output: ingest_report.v1 JSON.

use std::path::PathBuf;

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{to_tool_error, to_tool_success};
use crate::state::KebabAppState;

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct IngestFileInput {
    /// Absolute or relative path to the file to ingest. Workspace external
    /// paths are allowed — bytes are copied into `_external/`.
    pub path: String,
}

pub fn handle(state: &KebabAppState, input: IngestFileInput) -> CallToolResult {
    let cfg_clone = (*state.config).clone();
    let path = PathBuf::from(input.path);
    match kebab_app::ingest_file_with_config(cfg_clone, &path) {
        Ok(report) => match serde_json::to_value(&report) {
            Ok(mut v) => {
                if let serde_json::Value::Object(ref mut map) = v {
                    map.entry("schema_version".to_string())
                        .or_insert_with(|| serde_json::Value::String("ingest_report.v1".to_string()));
                }
                match serde_json::to_string(&v) {
                    Ok(json) => to_tool_success(json),
                    Err(e) => to_tool_error(&anyhow::anyhow!(e)),
                }
            }
            Err(e) => to_tool_error(&anyhow::anyhow!(e)),
        },
        Err(e) => to_tool_error(&e),
    }
}
