//! Map `anyhow::Error` returned by kebab-app facades to MCP
//! `CallToolResult` with `isError: true` + error.v1 JSON content.

use rmcp::model::{CallToolResult, Content};

use kebab_app::classify;

/// Convert an `anyhow::Error` to a `CallToolResult` with `isError: true`
/// and the serialised `error.v1` envelope as the text content.
pub fn to_tool_error(err: &anyhow::Error) -> CallToolResult {
    let v1 = classify(err, false);
    let body = serde_json::to_string(&v1).unwrap_or_else(|_| {
        r#"{"schema_version":"error.v1","code":"generic","message":"serialize failed"}"#
            .to_string()
    });
    CallToolResult::error(vec![Content::text(body)])
}

/// Wrap a successful wire-schema JSON string as a `CallToolResult`.
pub fn to_tool_success(json: String) -> CallToolResult {
    CallToolResult::success(vec![Content::text(json)])
}
