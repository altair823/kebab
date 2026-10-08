//! One ndjson line per MCP tool call, appended to
//! `{data_dir}/logs/mcp-calls.ndjson` (`~/.local/share/kebab/logs/` by
//! default). It follows `storage.data_dir`, so a `--config` user finds it in
//! their own store and tests with a temp data_dir never touch the real one.
//!
//! Why this exists (2026-10-09): the only way to learn how the agent actually
//! used kebab was to mine Claude Code transcripts after the fact. This log
//! answers "which tools, which queries, how many hits, how slow" directly, so
//! the next usage review is a `jq` one-liner instead of a day of scripting.
//!
//! Observability only. It never changes a tool result, and a failure to write
//! is reported with `tracing::warn!` and otherwise ignored. There is no
//! rotation: one line is a few hundred bytes and a busy year is a few
//! thousand lines. The `query` field is truncated to 200 chars.

use std::io::Write;

use rmcp::model::CallToolResult;
use serde_json::{Map, Value, json};

const QUERY_MAX_CHARS: usize = 200;

/// Build the log record. Pure, so it can be unit-tested without touching
/// the filesystem.
pub fn record(
    tool: &str,
    args: &Map<String, Value>,
    result: &CallToolResult,
    duration_ms: u128,
) -> Value {
    let ts = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default();
    let mut rec = json!({
        "schema_version": "mcp_call_log.v1",
        "ts": ts,
        "tool": tool,
        "ok": !result.is_error.unwrap_or(false),
        "duration_ms": duration_ms,
    });

    // Arguments worth keeping. `query` is the only free-text one.
    if let Some(q) = args.get("query").and_then(Value::as_str) {
        rec["query"] = Value::String(q.chars().take(QUERY_MAX_CHARS).collect());
    }
    if let Some(qs) = args.get("queries").and_then(Value::as_array) {
        rec["queries"] = Value::from(qs.len());
    }
    for key in [
        "mode",
        "k",
        "kind",
        "doc_id",
        "chunk_id",
        "session_id",
        "max_tokens",
    ] {
        if let Some(v) = args.get(key) {
            rec[key] = v.clone();
        }
    }

    // Result summary, keyed on the wire schema the tool returned.
    let text = result
        .content
        .iter()
        .find_map(|c| c.as_text())
        .map_or("", |t| t.text.as_str());
    if let Ok(v) = serde_json::from_str::<Value>(text) {
        match v.get("schema_version").and_then(Value::as_str) {
            Some("search_response.v1") => {
                let hits = v.get("hits").and_then(Value::as_array);
                rec["hits"] = Value::from(hits.map_or(0, Vec::len));
                if let Some(top) = hits.and_then(|h| h.first()).and_then(|h| h.get("doc_path")) {
                    rec["top_doc"] = top.clone();
                }
            }
            Some("bulk_search_response.v1") => {
                rec["items"] =
                    Value::from(v.get("items").and_then(Value::as_array).map_or(0, Vec::len));
            }
            Some("answer.v1") => {
                if let Some(g) = v.get("grounded") {
                    rec["grounded"] = g.clone();
                }
                rec["citations"] = Value::from(
                    v.get("citations")
                        .and_then(Value::as_array)
                        .map_or(0, Vec::len),
                );
                if let Some(r) = v.get("refusal_reason").filter(|r| !r.is_null()) {
                    rec["refusal_reason"] = r.clone();
                }
            }
            Some("error.v1") => {
                if let Some(c) = v.get("code") {
                    rec["error_code"] = c.clone();
                }
            }
            _ => {}
        }
    }
    rec
}

/// Append one record to the default log path. Never fails the caller.
pub fn log_call(
    cfg: &kebab_config::Config,
    tool: &str,
    args: &Map<String, Value>,
    result: &CallToolResult,
    duration_ms: u128,
) {
    let path = kebab_config::expand_path(&cfg.storage.data_dir, "")
        .join("logs")
        .join("mcp-calls.ndjson");
    if let Err(e) = append(&path, &record(tool, args, result, duration_ms)) {
        tracing::warn!(path = %path.display(), error = %e, "mcp call log write failed");
    }
}

fn append(path: &std::path::Path, rec: &Value) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(f, "{rec}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::Content;

    fn args(pairs: &[(&str, Value)]) -> Map<String, Value> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn search_record_has_hits_top_doc_and_truncated_query() {
        let body = json!({
            "schema_version": "search_response.v1",
            "hits": [{"doc_path": "wiki/a.md", "rank": 1}, {"doc_path": "jira/b.md", "rank": 2}],
        })
        .to_string();
        let result = CallToolResult::success(vec![Content::text(body)]);
        let long_q = "q".repeat(500);
        let rec = record(
            "search",
            &args(&[
                ("query", json!(long_q)),
                ("k", json!(10)),
                ("mode", json!("hybrid")),
            ]),
            &result,
            42,
        );
        assert_eq!(rec["schema_version"], "mcp_call_log.v1");
        assert_eq!(rec["tool"], "search");
        assert_eq!(rec["ok"], true);
        assert_eq!(rec["hits"], 2);
        assert_eq!(rec["top_doc"], "wiki/a.md");
        assert_eq!(rec["k"], 10);
        assert_eq!(rec["mode"], "hybrid");
        assert_eq!(rec["duration_ms"], 42);
        assert_eq!(
            rec["query"].as_str().unwrap().chars().count(),
            QUERY_MAX_CHARS
        );
    }

    #[test]
    fn ask_and_error_records() {
        let ask = CallToolResult::success(vec![Content::text(
            json!({"schema_version": "answer.v1", "grounded": false, "citations": [], "refusal_reason": "score_gate"}).to_string(),
        )]);
        let rec = record("ask", &args(&[("query", json!("x"))]), &ask, 1);
        assert_eq!(rec["grounded"], false);
        assert_eq!(rec["citations"], 0);
        assert_eq!(rec["refusal_reason"], "score_gate");

        let err = CallToolResult::error(vec![Content::text(
            json!({"schema_version": "error.v1", "code": "invalid_input", "message": "bad"})
                .to_string(),
        )]);
        let rec = record("fetch", &args(&[("kind", json!("doc"))]), &err, 3);
        assert_eq!(rec["ok"], false);
        assert_eq!(rec["error_code"], "invalid_input");
        assert_eq!(rec["kind"], "doc");
    }

    #[test]
    fn append_writes_one_line_per_record() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("nested").join("mcp-calls.ndjson");
        append(&p, &json!({"a": 1})).unwrap();
        append(&p, &json!({"b": 2})).unwrap();
        let s = std::fs::read_to_string(&p).unwrap();
        assert_eq!(s.lines().count(), 2);
        assert!(s.lines().all(|l| serde_json::from_str::<Value>(l).is_ok()));
    }
}
