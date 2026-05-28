//! `ingest_stdin` tool — wraps `kebab_app::ingest_stdin_with_config`.
//! Input: { content, title, source_uri? }. Output: ingest_report.v1 JSON.

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{to_tool_error, to_tool_success};
use crate::state::KebabAppState;

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct IngestStdinInput {
    /// Markdown body content. v1 supports markdown only.
    pub content: String,
    /// Title for frontmatter injection.
    pub title: String,
    /// Optional source URI (e.g. https URL agent fetched from).
    pub source_uri: Option<String>,
}

pub fn handle(state: &KebabAppState, input: IngestStdinInput) -> CallToolResult {
    let cfg_clone = (*state.config).clone();
    match kebab_app::ingest_stdin_with_config(
        cfg_clone,
        &input.content,
        &input.title,
        input.source_uri.as_deref(),
    ) {
        Ok(report) => match serde_json::to_value(&report) {
            Ok(mut v) => {
                if let serde_json::Value::Object(ref mut map) = v {
                    map.entry("schema_version".to_string()).or_insert_with(|| {
                        serde_json::Value::String("ingest_report.v1".to_string())
                    });
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
