//! p9-fb-42: bulk multi-query facade. Sequential for-loop reusing
//! one App instance so embedder cold-start + LRU cache amortize
//! across the N queries.

use anyhow::Context;
use kebab_core::{
    BulkSearchItem, BulkSearchSummary, DocumentId, Lang, SearchFilters, SearchHit, SearchMode,
    SearchOpts, SearchQuery, TrustLevel,
};
use serde_json::Value;

use crate::{App, SearchResponse};

/// Hard cap on items per bulk call. Documented in spec — agents that
/// hit this should batch-split.
pub const BULK_QUERIES_MAX: usize = 100;

/// p9-fb-42: bulk search facade. Returns `(items, summary)` always
/// — per-query failures embed `error.v1` JSON in the item rather
/// than aborting the bulk call. Returns `Err` only for input
/// validation failures (e.g. >100 queries).
#[doc(hidden)]
pub fn bulk_search_with_config(
    config: kebab_config::Config,
    raw_items: Vec<Value>,
) -> anyhow::Result<(Vec<BulkSearchItem>, BulkSearchSummary)> {
    if raw_items.len() > BULK_QUERIES_MAX {
        anyhow::bail!(
            "queries: max {} items, got {}",
            BULK_QUERIES_MAX,
            raw_items.len()
        );
    }

    let app = App::open_with_config(config).context("kebab-app: open for bulk_search")?;

    let mut results: Vec<BulkSearchItem> = Vec::with_capacity(raw_items.len());
    let mut succeeded: u32 = 0;
    let mut failed: u32 = 0;

    for raw in raw_items {
        let item = run_one(&app, raw);
        if item.error.is_some() {
            failed += 1;
        } else {
            succeeded += 1;
        }
        results.push(item);
    }

    let summary = BulkSearchSummary {
        total: succeeded + failed,
        succeeded,
        failed,
    };
    Ok((results, summary))
}

fn run_one(app: &App, raw: Value) -> BulkSearchItem {
    let echo = raw.clone();
    match parse_one(&raw) {
        Ok((query, opts)) => match app.search_with_opts(query, opts) {
            Ok(resp) => BulkSearchItem {
                query: echo,
                response: Some(serialize_search_response(&resp)),
                error: None,
            },
            Err(e) => BulkSearchItem {
                query: echo,
                response: None,
                error: Some(error_v1_json("retrieval_error", &format!("{e:#}"), None)),
            },
        },
        Err(msg) => BulkSearchItem {
            query: echo,
            response: None,
            error: Some(error_v1_json("invalid_input", &msg, None)),
        },
    }
}

/// Mirror of `kebab-cli::wire::wire_search_response` — `SearchResponse`
/// itself is not `Serialize`, so we build the `search_response.v1`-shaped
/// JSON manually. Each hit also gets `score` promoted from
/// `retrieval.fusion_score` per §2.2, matching the CLI wire layer.
fn serialize_search_response(r: &SearchResponse) -> Value {
    let mut v = serde_json::json!({
        "schema_version": "search_response.v1",
        "hits": r.hits.iter().map(serialize_search_hit).collect::<Vec<_>>(),
        "next_cursor": r.next_cursor,
        "truncated": r.truncated,
    });
    if let Value::Object(ref mut map) = v {
        let trace_v = match &r.trace {
            Some(t) => serde_json::to_value(t).unwrap_or(Value::Null),
            None => Value::Null,
        };
        map.insert("trace".to_string(), trace_v);
    }
    v
}

fn serialize_search_hit(h: &SearchHit) -> Value {
    let mut v = serde_json::to_value(h).unwrap_or(Value::Null);
    if let Value::Object(ref mut map) = v {
        if let Some(Value::Object(retrieval)) = map.get("retrieval") {
            if let Some(score) = retrieval.get("fusion_score").cloned() {
                map.insert("score".to_string(), score);
            }
        }
        map.insert(
            "schema_version".to_string(),
            Value::String("search_hit.v1".to_string()),
        );
    }
    v
}

fn parse_one(raw: &Value) -> Result<(SearchQuery, SearchOpts), String> {
    let obj = raw.as_object().ok_or("expected JSON object")?;
    let text = obj
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or("missing required field: query")?
        .to_string();

    let mode = match obj.get("mode").and_then(|v| v.as_str()) {
        None => SearchMode::Hybrid,
        Some("hybrid") => SearchMode::Hybrid,
        Some("lexical") => SearchMode::Lexical,
        Some("vector") => SearchMode::Vector,
        Some(other) => return Err(format!("invalid mode: {other:?}")),
    };

    let k = obj
        .get("k")
        .and_then(|v| v.as_u64())
        .map(|n| n as usize)
        .unwrap_or(0); // 0 → use config default in app

    let trust_min = match obj.get("trust_min").and_then(|v| v.as_str()) {
        None => None,
        Some("primary") => Some(TrustLevel::Primary),
        Some("secondary") => Some(TrustLevel::Secondary),
        Some("generated") => Some(TrustLevel::Generated),
        Some(other) => return Err(format!("invalid trust_min: {other:?}")),
    };

    let ingested_after = match obj.get("ingested_after").and_then(|v| v.as_str()) {
        None => None,
        Some(s) => Some(
            time::OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339)
                .map_err(|e| format!("invalid ingested_after RFC3339 {s:?}: {e}"))?,
        ),
    };

    let media: Vec<String> = obj
        .get("media")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(normalize_media_alias))
                .collect()
        })
        .unwrap_or_default();

    let tags_any: Vec<String> = obj
        .get("tag")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let lang = obj
        .get("lang")
        .and_then(|v| v.as_str())
        .map(|s| Lang(s.to_string()));

    let path_glob = obj
        .get("path_glob")
        .and_then(|v| v.as_str())
        .map(String::from);

    let doc_id = obj
        .get("doc_id")
        .and_then(|v| v.as_str())
        .map(|s| DocumentId(s.to_string()));

    let filters = SearchFilters {
        tags_any,
        lang,
        path_glob,
        trust_min,
        media,
        ingested_after,
        doc_id,
        repo: vec![],
        code_lang: vec![],
    };

    let opts = SearchOpts {
        max_tokens: obj
            .get("max_tokens")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize),
        snippet_chars: obj
            .get("snippet_chars")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize),
        cursor: obj.get("cursor").and_then(|v| v.as_str()).map(String::from),
        trace: obj.get("trace").and_then(|v| v.as_bool()).unwrap_or(false),
    };

    Ok((
        SearchQuery {
            text,
            mode,
            k,
            filters,
        },
        opts,
    ))
}

fn normalize_media_alias(s: &str) -> String {
    match s.to_ascii_lowercase().as_str() {
        "md" => "markdown".to_string(),
        other => other.to_string(),
    }
}

fn error_v1_json(code: &str, message: &str, hint: Option<&str>) -> Value {
    serde_json::json!({
        "schema_version": "error.v1",
        "code": code,
        "message": message,
        "hint": hint,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_temp() -> kebab_config::Config {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = kebab_config::Config::defaults();
        cfg.storage.data_dir = dir.path().to_string_lossy().into_owned();
        // Bring up migrations so SqliteStore::open_existing succeeds inside App::open.
        let store = kebab_store_sqlite::SqliteStore::open(&cfg).unwrap();
        store.run_migrations().unwrap();
        drop(store);
        // Leak the tempdir into a static — tests are short-lived; not worth threading.
        std::mem::forget(dir);
        cfg
    }

    #[test]
    fn empty_input_returns_empty_summary() {
        let cfg = open_temp();
        let (items, summary) = bulk_search_with_config(cfg, vec![]).unwrap();
        assert!(items.is_empty());
        assert_eq!(summary.total, 0);
        assert_eq!(summary.succeeded, 0);
        assert_eq!(summary.failed, 0);
    }

    #[test]
    fn over_cap_returns_err() {
        let cfg = open_temp();
        let raw: Vec<Value> = (0..101)
            .map(|_| serde_json::json!({"query": "x"}))
            .collect();
        let err = bulk_search_with_config(cfg, raw).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("max 100"));
    }

    #[test]
    fn invalid_item_emits_error_keeps_total_count() {
        let cfg = open_temp();
        let raw = vec![
            serde_json::json!({"query": "ok", "mode": "lexical"}),
            serde_json::json!({"mode": "lexical"}), // missing required `query`
        ];
        let (items, summary) = bulk_search_with_config(cfg, raw).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(summary.total, 2);
        // First item: lexical mode against empty corpus succeeds with empty hits.
        assert!(items[0].error.is_none());
        // Second item: missing required field.
        assert!(items[1].error.is_some());
        assert_eq!(items[1].error.as_ref().unwrap()["code"], "invalid_input");
    }
}
