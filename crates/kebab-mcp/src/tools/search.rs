//! `search` tool — wraps `kebab_app::search_with_opts_with_config`.
//! Input: { query, mode?, k?, max_tokens?, snippet_chars?, cursor?,
//!          tags?, lang?, path_glob?, trust_min?, media?,
//!          ingested_after?, doc_id? }.
//! Output: search_response.v1 envelope (hits + next_cursor + truncated).
//!
//! First tool with a non-empty `inputSchema`: `SearchInput` derives
//! `JsonSchema` and `Tool::new` uses
//! `rmcp::handler::server::common::schema_for_type::<SearchInput>()`.

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use kebab_app::ERROR_V1_ID;

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
    /// p9-fb-36: filter by `metadata.tags` (OR-within).
    pub tags: Option<Vec<String>>,
    /// p9-fb-36: filter by `documents.lang` (ISO code).
    pub lang: Option<String>,
    /// p9-fb-36: filter by `documents.workspace_path` glob.
    pub path_glob: Option<String>,
    /// p9-fb-36: filter by minimum `documents.trust_level`.
    /// Accepts: `"primary"`, `"secondary"`, `"generated"`.
    pub trust_min: Option<String>,
    /// p9-fb-36: filter by `assets.media_type` kind. IN-list. Accepts:
    /// `"markdown"`, `"pdf"`, `"image"`, `"audio"`, `"other"`. Aliases: `md` → `markdown`.
    pub media: Option<Vec<String>>,
    /// p9-fb-36: RFC3339 UTC timestamp. Invalid format → invalid_input.
    pub ingested_after: Option<String>,
    /// p9-fb-36: filter to a single doc.
    pub doc_id: Option<String>,
}

pub fn handle(state: &KebabAppState, input: SearchInput) -> CallToolResult {
    let k = input.k.unwrap_or(10).clamp(1, 100);
    let mode_str = input.mode.as_deref().unwrap_or("hybrid");
    let mode = match mode_str {
        "lexical" => kebab_core::SearchMode::Lexical,
        "vector" => kebab_core::SearchMode::Vector,
        _ => kebab_core::SearchMode::Hybrid,
    };

    // p9-fb-36: parse filter inputs, returning invalid_input on bad values.
    let trust_min = match input.trust_min.as_deref() {
        Some(s) => match s.to_ascii_lowercase().as_str() {
            "primary" => Some(kebab_core::TrustLevel::Primary),
            "secondary" => Some(kebab_core::TrustLevel::Secondary),
            "generated" => Some(kebab_core::TrustLevel::Generated),
            other => {
                return invalid_input(&format!(
                    "trust_min: unknown level '{other}'; expected primary|secondary|generated"
                ));
            }
        },
        None => None,
    };

    let ingested_after = match input.ingested_after.as_deref() {
        Some(s) => {
            match time::OffsetDateTime::parse(
                s,
                &time::format_description::well_known::Rfc3339,
            ) {
                Ok(ts) => Some(ts),
                Err(e) => {
                    return invalid_input(&format!(
                        "ingested_after: invalid RFC3339 '{s}': {e}"
                    ));
                }
            }
        }
        None => None,
    };

    let media: Vec<String> = input
        .media
        .clone()
        .unwrap_or_default()
        .iter()
        .map(|s| normalize_media_alias(s))
        .collect();

    let filters = kebab_core::SearchFilters {
        tags_any: input.tags.clone().unwrap_or_default(),
        lang: input.lang.clone().map(kebab_core::Lang),
        path_glob: input.path_glob.clone(),
        trust_min,
        media,
        ingested_after,
        doc_id: input.doc_id.clone().map(kebab_core::DocumentId),
    };

    let query = kebab_core::SearchQuery {
        text: input.query,
        mode,
        k,
        filters,
    };
    let opts = kebab_core::SearchOpts {
        max_tokens: input.max_tokens,
        snippet_chars: input.snippet_chars,
        cursor: input.cursor,
        trace: false,
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

fn normalize_media_alias(s: &str) -> String {
    match s.to_ascii_lowercase().as_str() {
        "md" => "markdown".to_string(),
        other => other.to_string(),
    }
}

fn invalid_input(msg: &str) -> CallToolResult {
    use kebab_app::{ErrorV1, StructuredError};
    let err = anyhow::Error::new(StructuredError(ErrorV1 {
        schema_version: ERROR_V1_ID.to_string(),
        code: "invalid_input".to_string(),
        message: msg.to_string(),
        details: serde_json::Value::Null,
        hint: None,
    }));
    to_tool_error(&err)
}
