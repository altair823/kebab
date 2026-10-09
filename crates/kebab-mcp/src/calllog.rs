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
//! thousand lines. The `query` field is truncated to 200 chars. Each line is
//! written with a single `write_all` on an `O_APPEND` handle so concurrent
//! servers (several Claude Code sessions) never interleave fragments.

use std::io::Write;

use rmcp::model::CallToolResult;
use serde_json::{Map, Value, json};

const QUERY_MAX_CHARS: usize = 200;

/// Argument keys worth keeping. `query` is the only free-text one; `queries`
/// (bulk_search) is reduced to a count. Everything else (`content` of
/// `ingest_stdin`, filters, cursors) is dropped before the call runs so the
/// log never copies a document body.
const KEPT_ARGS: [&str; 7] = [
    "query", "queries", "mode", "k", "kind", "doc_id", "chunk_id",
];

/// Pick the loggable subset of a tool's arguments. Cheap: at most seven
/// small values are cloned, never the whole input.
pub fn pick_args(args: Option<&Map<String, Value>>) -> Map<String, Value> {
    let mut out = Map::new();
    let Some(args) = args else { return out };
    for key in KEPT_ARGS {
        match (key, args.get(key)) {
            (_, None) => {}
            ("query", Some(Value::String(q))) => {
                out.insert(
                    key.into(),
                    Value::String(q.chars().take(QUERY_MAX_CHARS).collect()),
                );
            }
            ("queries", Some(Value::Array(qs))) => {
                out.insert(key.into(), Value::from(qs.len()));
            }
            ("query" | "queries", Some(_)) => {}
            (_, Some(v)) => {
                out.insert(key.into(), v.clone());
            }
        }
    }
    out
}

fn base_record(tool: &str, args: &Map<String, Value>, ok: bool, duration_ms: u128) -> Value {
    let ts = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default();
    let mut rec = json!({
        "schema_version": "mcp_call_log.v1",
        "ts": ts,
        "tool": tool,
        "ok": ok,
        "duration_ms": duration_ms,
    });
    for (k, v) in args {
        rec[k] = v.clone();
    }
    rec
}

/// Build the record for a call that produced a `CallToolResult` (success or
/// `isError`). No I/O except reading the clock, so it is unit-testable.
pub fn record(
    tool: &str,
    args: &Map<String, Value>,
    result: &CallToolResult,
    duration_ms: u128,
) -> Value {
    let mut rec = base_record(tool, args, !result.is_error.unwrap_or(false), duration_ms);

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
                // The envelope key is `results` (see tools/bulk_search.rs).
                rec["results"] = Value::from(
                    v.get("results")
                        .and_then(Value::as_array)
                        .map_or(0, Vec::len),
                );
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

/// Build the record for a call that never produced a `CallToolResult`: the
/// JSON-RPC layer rejected it (unknown tool name, a panicking tool task).
/// `ok` is false and `error_code` is `rpc_<code>` so these stay countable.
pub fn record_failure(
    tool: &str,
    args: &Map<String, Value>,
    rpc_code: i32,
    duration_ms: u128,
) -> Value {
    let mut rec = base_record(tool, args, false, duration_ms);
    rec["error_code"] = Value::String(format!("rpc_{rpc_code}"));
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
    write_record(cfg, &record(tool, args, result, duration_ms));
}

/// Append a failure record (see [`record_failure`]). Never fails the caller.
pub fn log_failure(
    cfg: &kebab_config::Config,
    tool: &str,
    args: &Map<String, Value>,
    rpc_code: i32,
    duration_ms: u128,
) {
    write_record(cfg, &record_failure(tool, args, rpc_code, duration_ms));
}

fn write_record(cfg: &kebab_config::Config, rec: &Value) {
    let path = kebab_config::expand_path(&cfg.storage.data_dir, "")
        .join("logs")
        .join("mcp-calls.ndjson");
    if let Err(e) = append(&path, rec) {
        tracing::warn!(path = %path.display(), error = %e, "mcp call log write failed");
    }
}

fn append(path: &std::path::Path, rec: &Value) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // One write per line: with O_APPEND a single write_all is not interleaved
    // with other writers on local filesystems. `writeln!(f, "{rec}")` would
    // issue one syscall per serde_json Display fragment and could mix lines.
    let mut line = rec.to_string();
    line.push('\n');
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    f.write_all(line.as_bytes())
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
    fn pick_args_keeps_small_keys_truncates_query_and_drops_bodies() {
        let long_q = "q".repeat(500);
        let input = args(&[
            ("query", json!(long_q)),
            ("k", json!(10)),
            ("mode", json!("hybrid")),
            (
                "content",
                json!("a very long document body that must not be logged"),
            ),
            ("tags", json!(["a", "b"])),
        ]);
        let picked = pick_args(Some(&input));
        assert_eq!(
            picked["query"].as_str().unwrap().chars().count(),
            QUERY_MAX_CHARS
        );
        assert_eq!(picked["k"], 10);
        assert_eq!(picked["mode"], "hybrid");
        assert!(!picked.contains_key("content"));
        assert!(!picked.contains_key("tags"));
        assert!(pick_args(None).is_empty());

        let bulk = pick_args(Some(&args(&[("queries", json!(["a", "b", "c"]))])));
        assert_eq!(bulk["queries"], 3);
    }

    #[test]
    fn search_record_has_hits_and_top_doc() {
        let body = json!({
            "schema_version": "search_response.v1",
            "hits": [{"doc_path": "wiki/a.md", "rank": 1}, {"doc_path": "jira/b.md", "rank": 2}],
        })
        .to_string();
        let result = CallToolResult::success(vec![Content::text(body)]);
        let rec = record(
            "search",
            &pick_args(Some(&args(&[("query", json!("x")), ("k", json!(10))]))),
            &result,
            42,
        );
        assert_eq!(rec["schema_version"], "mcp_call_log.v1");
        assert_eq!(rec["tool"], "search");
        assert_eq!(rec["ok"], true);
        assert_eq!(rec["hits"], 2);
        assert_eq!(rec["top_doc"], "wiki/a.md");
        assert_eq!(rec["k"], 10);
        assert_eq!(rec["duration_ms"], 42);
    }

    #[test]
    fn bulk_search_record_counts_results_key() {
        let body = json!({
            "schema_version": "bulk_search_response.v1",
            "results": [{"id": "a"}, {"id": "b"}, {"id": "c"}],
            "summary": {"total": 3, "succeeded": 3, "failed": 0},
        })
        .to_string();
        let result = CallToolResult::success(vec![Content::text(body)]);
        let rec = record(
            "bulk_search",
            &pick_args(Some(&args(&[("queries", json!(["a", "b", "c"]))]))),
            &result,
            7,
        );
        assert_eq!(rec["results"], 3);
        assert_eq!(rec["queries"], 3);
    }

    #[test]
    fn ask_error_and_rpc_failure_records() {
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

        let rec = record_failure("nope", &Map::new(), -32601, 0);
        assert_eq!(rec["ok"], false);
        assert_eq!(rec["error_code"], "rpc_-32601");
        assert_eq!(rec["tool"], "nope");
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
