//! CLI-side wire-schema wrappers.
//!
//! Convention (per design §2): every JSON object emitted on stdout in
//! `--json` mode MUST carry a top-level `schema_version` of the form
//! `"<object>.v1"`. The kb-core types are pure domain types and do NOT
//! carry `schema_version` themselves; the CLI wraps them on emit. The one
//! exception is `DoctorReport`, where `schema_version` is part of the wire
//! type because the doctor wire object IS its own structured surface.
//!
//! Future tasks (P1-5, P3, P4, P5) replacing stub `bail!` paths must call
//! these helpers from the relevant CLI subcommand handler before
//! `serde_json::to_string`.
//!
//! Each helper is total (returns `serde_json::Value`, never an error) — the
//! input is a fully-typed `serde::Serialize` value, so the only way to fail
//! is OOM, which would have killed the process anyway.

use serde_json::Value;

use kebab_app::DoctorReport;
use kebab_core::{Answer, Chunk, DocSummary, IngestReport, SearchHit};

/// Insert `schema_version` into an object-shaped `Value`. Helper for the
/// "serialize, then tag" pattern used by all the per-type wrappers below.
fn tag_object(mut v: Value, schema_version: &str) -> Value {
    if let Value::Object(ref mut map) = v {
        map.insert(
            "schema_version".to_string(),
            Value::String(schema_version.to_string()),
        );
    }
    v
}

/// Wrap an [`IngestReport`] as `ingest_report.v1`.
pub fn wire_ingest(r: &IngestReport) -> Value {
    let v = serde_json::to_value(r).expect("IngestReport serializes");
    tag_object(v, "ingest_report.v1")
}

/// Wrap a single [`DocSummary`] as `doc_summary.v1`.
pub fn wire_doc_summary(d: &DocSummary) -> Value {
    let v = serde_json::to_value(d).expect("DocSummary serializes");
    tag_object(v, "doc_summary.v1")
}

/// Wrap a list of [`DocSummary`] values as a JSON array of `doc_summary.v1`
/// objects (one tag per element, per design §2.5 — there is no list-envelope
/// schema; the list shape is `[{schema_version: "doc_summary.v1", ...}, ...]`).
pub fn wire_doc_summaries(d: &[DocSummary]) -> Value {
    Value::Array(d.iter().map(wire_doc_summary).collect())
}

/// Wrap a [`Chunk`] as `chunk_inspection.v1` (§2.6). NOTE: the wire schema
/// requires `doc_path`, which the kb-core `Chunk` does not currently carry —
/// when P1-5 wires the Ok-path, the implementation should either enrich
/// `Chunk` or pass `doc_path` alongside. For now this helper emits whatever
/// fields `Chunk` serializes with, plus the `schema_version` tag.
pub fn wire_chunk_inspection(c: &Chunk) -> Value {
    let v = serde_json::to_value(c).expect("Chunk serializes");
    tag_object(v, "chunk_inspection.v1")
}

/// Wrap a single [`SearchHit`] as `search_hit.v1`.
pub fn wire_search_hit(h: &SearchHit) -> Value {
    let mut v = serde_json::to_value(h).expect("SearchHit serializes");
    // Promote `retrieval.fusion_score` to a top-level `score` per §2.2.
    if let Value::Object(ref mut map) = v {
        if let Some(Value::Object(retrieval)) = map.get("retrieval") {
            if let Some(score) = retrieval.get("fusion_score").cloned() {
                map.insert("score".to_string(), score);
            }
        }
    }
    tag_object(v, "search_hit.v1")
}

/// p9-fb-34: tag a `SearchResponse` as `search_response.v1`. Wraps
/// the existing `search_hit.v1[]` array with pagination + truncation
/// metadata. Replaces the previous bare `search_hit.v1[]` top-level
/// array (`wire_search_hits`) — see HOTFIXES / fb-34 for the
/// breaking shape change.
pub fn wire_search_response(r: &kebab_app::SearchResponse) -> Value {
    let mut v = serde_json::json!({
        "hits": r.hits.iter().map(wire_search_hit).collect::<Vec<_>>(),
        "next_cursor": r.next_cursor,
        "truncated": r.truncated,
    });
    if let Some(trace) = &r.trace {
        let trace_v = serde_json::to_value(trace).expect("SearchTrace serializes");
        if let Value::Object(ref mut map) = v {
            map.insert("trace".to_string(), trace_v);
        }
    }
    tag_object(v, "search_response.v1")
}

/// Wrap an [`Answer`] as `answer.v1`.
pub fn wire_answer(a: &Answer) -> Value {
    let v = serde_json::to_value(a).expect("Answer serializes");
    tag_object(v, "answer.v1")
}

/// p9-fb-33: tag a [`StreamEvent`] as `answer_event.v1` ndjson.
///
/// The timestamp is added at emit time (caller fills `ts`), since the
/// pipeline doesn't carry one in the in-process enum — mirrors the
/// `wire_ingest_progress` pattern (§2 ingest_progress.v1).
pub fn wire_answer_event(
    ev: &kebab_app::StreamEvent,
    ts: time::OffsetDateTime,
) -> Value {
    let mut v = serde_json::to_value(ev).expect("StreamEvent serializes");
    let ts_str = ts
        .format(&time::format_description::well_known::Rfc3339)
        .expect("OffsetDateTime formats as RFC3339");
    if let Value::Object(ref mut map) = v {
        map.insert("ts".to_string(), Value::String(ts_str));
    }
    tag_object(v, "answer_event.v1")
}

/// Idempotent pass-through for [`DoctorReport`] — the type already carries
/// `schema_version: "doctor.v1"` (struct-field convention, the one
/// exception called out in the module doc above). This helper exists so
/// every `--json` branch in `kb-cli` goes through `wire::*`, keeping the
/// emit pattern uniform.
pub fn wire_doctor(d: &DoctorReport) -> Value {
    // Round-trip through `to_value` to confirm the field is serialized;
    // then re-tag (no-op when the field is already present, defensive
    // when a future refactor drops the struct-field).
    let v = serde_json::to_value(d).expect("DoctorReport serializes");
    if let Value::Object(ref map) = v {
        if matches!(
            map.get("schema_version"),
            Some(Value::String(s)) if s == "doctor.v1"
        ) {
            return v;
        }
    }
    tag_object(v, "doctor.v1")
}

/// Wrap a [`kebab_app::ResetReport`] as `reset_report.v1`.
pub fn wire_reset(r: &kebab_app::ResetReport) -> Value {
    let v = serde_json::to_value(r).expect("ResetReport serializes");
    tag_object(v, "reset_report.v1")
}

/// Wrap an [`kebab_app::IngestEvent`] as `ingest_progress.v1`. Adds
/// the `schema_version` discriminator on top of serde's existing
/// `kind` discriminator, plus an `ts` field with the current
/// wall-clock — the emit site is the only place that knows the moment
/// of emission, so the timestamp is stamped here rather than carried
/// on the event itself.
pub fn wire_ingest_progress(
    event: &kebab_app::IngestEvent,
) -> anyhow::Result<Value> {
    let mut v = serde_json::to_value(event)?;
    if let Value::Object(ref mut map) = v {
        map.insert(
            "ts".to_string(),
            Value::String(crate::progress::now_rfc3339()?),
        );
    }
    Ok(tag_object(v, "ingest_progress.v1"))
}

/// Wrap a [`kebab_app::SchemaV1`] as `schema.v1`.
///
/// Uses the idempotent re-tag pattern (mirrors `wire_doctor`) because
/// `SchemaV1` already carries `schema_version: "schema.v1"` as a struct
/// field. The re-tag is a defensive no-op when the field is present; it
/// stamps the correct version if a future refactor ever drops the field.
pub fn wire_schema(s: &kebab_app::SchemaV1) -> Value {
    let v = serde_json::to_value(s).expect("SchemaV1 serializes");
    if let Value::Object(ref map) = v {
        if matches!(
            map.get("schema_version"),
            Some(Value::String(s)) if s == kebab_app::SCHEMA_V1_ID
        ) {
            return v;
        }
    }
    tag_object(v, kebab_app::SCHEMA_V1_ID)
}

/// Wrap an [`kebab_app::ErrorV1`] as `error.v1`.
///
/// Uses the simple `tag_object` pattern because `ErrorV1` is a
/// type that does NOT carry `schema_version` itself
/// (kebab-core convention).
pub fn wire_error_v1(e: &kebab_app::ErrorV1) -> Value {
    let v = serde_json::to_value(e).expect("ErrorV1 serializes");
    tag_object(v, "error.v1")
}

/// p9-fb-35: tag a [`kebab_core::FetchResult`] as `fetch_result.v1`.
pub fn wire_fetch_result(r: &kebab_core::FetchResult) -> Value {
    let v = serde_json::to_value(r).expect("FetchResult serializes");
    tag_object(v, "fetch_result.v1")
}

/// p9-fb-42: tag a `BulkSearchItem` (already serialized as a Value)
/// as `bulk_search_item.v1`. The inner `query` / `response` / `error`
/// fields stay verbatim — only the envelope gets the schema_version stamp.
pub fn wire_bulk_search_item(item: &kebab_core::BulkSearchItem) -> Value {
    let mut v = serde_json::to_value(item).expect("BulkSearchItem serializes");
    if let Value::Object(ref mut map) = v {
        map.insert(
            "schema_version".to_string(),
            Value::String("bulk_search_item.v1".to_string()),
        );
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema_of(v: &Value) -> Option<&str> {
        v.as_object()?.get("schema_version")?.as_str()
    }

    #[test]
    fn doctor_round_trip_preserves_schema_version() {
        let d = DoctorReport {
            schema_version: "doctor.v1".to_string(),
            ok: true,
            checks: Vec::new(),
        };
        let v = wire_doctor(&d);
        assert_eq!(schema_of(&v), Some("doctor.v1"));
        // Sanity: ok/checks are preserved.
        assert_eq!(v.get("ok").and_then(Value::as_bool), Some(true));
        assert!(v.get("checks").and_then(Value::as_array).is_some());
    }

    #[test]
    fn ingest_wrapper_tags_schema_version() {
        use kebab_core::{SkipExamples, SourceScope};
        let r = IngestReport {
            scope: SourceScope {
                root: std::path::PathBuf::from("/tmp"),
                include: vec![],
                exclude: vec![],
            },
            scanned: 0,
            new: 0,
            updated: 0,
            skipped: 0,
            unchanged: 0,
            errors: 0,
            duration_ms: 0,
            skipped_by_extension: std::collections::BTreeMap::new(),
            skipped_gitignore: 0,
            skipped_kebabignore: 0,
            skipped_builtin_blacklist: 0,
            skipped_generated: 0,
            skipped_size_exceeded: 0,
            skip_examples: SkipExamples::default(),
            items: None,
        };
        let v = wire_ingest(&r);
        assert_eq!(schema_of(&v), Some("ingest_report.v1"));
        assert!(v.get("items").is_some());
    }

    #[test]
    fn doc_summaries_wraps_each_element() {
        let v = wire_doc_summaries(&[]);
        assert!(v.is_array());
        assert_eq!(v.as_array().unwrap().len(), 0);
    }

    #[test]
    fn tag_object_inserts_into_object() {
        let v = Value::Object(serde_json::Map::new());
        let tagged = tag_object(v, "x.v1");
        assert_eq!(schema_of(&tagged), Some("x.v1"));
    }

    #[test]
    fn search_response_carries_pagination_metadata() {
        // p9-fb-34: empty-hits SearchResponse round-trips through the
        // wrapper with its `next_cursor` + `truncated` fields preserved
        // and the top-level `schema_version` set to `search_response.v1`.
        let r = kebab_app::SearchResponse {
            hits: vec![],
            next_cursor: Some("opaque-cursor-abc".to_string()),
            truncated: true,
            trace: None,
        };
        let v = wire_search_response(&r);
        assert_eq!(schema_of(&v), Some("search_response.v1"));
        assert!(v.get("hits").and_then(|h| h.as_array()).is_some());
        assert_eq!(
            v.get("hits").and_then(|h| h.as_array()).unwrap().len(),
            0
        );
        assert_eq!(
            v.get("next_cursor").and_then(|c| c.as_str()),
            Some("opaque-cursor-abc")
        );
        assert_eq!(v.get("truncated").and_then(|t| t.as_bool()), Some(true));
    }

    #[test]
    fn schema_wrapper_tags_schema_version() {
        use kebab_app::{Capabilities, Models, SchemaV1, Stats, WireBlock};
        let schema = SchemaV1 {
            schema_version: "schema.v1".to_string(),
            kebab_version: "0.2.1".to_string(),
            wire: WireBlock { schemas: vec!["answer.v1".to_string()] },
            capabilities: Capabilities {
                json_mode: true, ingest_progress: true, ingest_cancellation: true,
                rag_multi_turn: true, search_cache: true, incremental_ingest: true,
                streaming_ask: false, http_daemon: false, mcp_server: false,
                single_file_ingest: false, bulk_search: true,
            },
            models: Models {
                parser_version: "x".to_string(),
                chunker_version: "y".to_string(),
                embedding_version: "z".to_string(),
                prompt_template_version: "w".to_string(),
                index_version: "v".to_string(),
                corpus_revision: 7,
            },
            stats: Stats {
                doc_count: 1, chunk_count: 2, asset_count: 1,
                last_ingest_at: None,
                media_breakdown: Default::default(),
                lang_breakdown: Default::default(),
                index_bytes: Default::default(),
                stale_doc_count: 0,
                // p10-1A-1: new fields added to Stats; use Default for the test fixture.
                ..Default::default()
            },
        };
        let v = wire_schema(&schema);
        assert_eq!(schema_of(&v), Some("schema.v1"));
        assert_eq!(v.get("kebab_version").and_then(Value::as_str), Some("0.2.1"));
    }

    #[test]
    fn error_wrapper_tags_schema_version_and_emits_code() {
        use kebab_app::ErrorV1;
        let err = ErrorV1 {
            schema_version: "error.v1".to_string(),
            code: "config_invalid".to_string(),
            message: "bad config".to_string(),
            details: serde_json::json!({"path": "/tmp/x"}),
            hint: Some("check the path".to_string()),
        };
        let v = wire_error_v1(&err);
        assert_eq!(schema_of(&v), Some("error.v1"));
        assert_eq!(v.get("code").and_then(Value::as_str), Some("config_invalid"));
    }

    #[test]
    fn reset_wrapper_tags_schema_version_and_serializes_scope() {
        let r = kebab_app::ResetReport {
            scope: kebab_app::ResetScope::DataOnly,
            removed_paths: vec![std::path::PathBuf::from("/tmp/x")],
            embedding_rows_truncated: 0,
        };
        let v = wire_reset(&r);
        assert_eq!(schema_of(&v), Some("reset_report.v1"));
        assert_eq!(v.get("scope").and_then(Value::as_str), Some("data_only"));
        assert_eq!(
            v.get("embedding_rows_truncated").and_then(Value::as_u64),
            Some(0)
        );
        let paths = v.get("removed_paths").and_then(Value::as_array).unwrap();
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].as_str(), Some("/tmp/x"));
    }

    #[test]
    fn search_response_with_trace_serializes_trace_field() {
        use kebab_core::{SearchTrace, TraceCandidate, TraceFusionInput,
                         TraceTiming, ChunkId, DocumentId, WorkspacePath};
        let r = kebab_app::SearchResponse {
            hits: vec![],
            next_cursor: None,
            truncated: false,
            trace: Some(SearchTrace {
                lexical: vec![TraceCandidate {
                    chunk_id: ChunkId("c1".into()),
                    doc_id: DocumentId("d1".into()),
                    doc_path: WorkspacePath::new("a.md".into()).unwrap(),
                    rank: 1,
                    score: 0.42,
                }],
                vector: vec![],
                rrf_inputs: vec![TraceFusionInput {
                    chunk_id: ChunkId("c1".into()),
                    lexical_rank: Some(1),
                    vector_rank: None,
                    fusion_score: 0.0,
                }],
                timing: TraceTiming { lexical_ms: 5, vector_ms: 0, fusion_ms: 1, total_ms: 7 },
            }),
        };
        let v = wire_search_response(&r);
        assert_eq!(schema_of(&v), Some("search_response.v1"));
        assert!(v["trace"].is_object());
        assert_eq!(v["trace"]["timing"]["lexical_ms"], 5);
        assert_eq!(v["trace"]["lexical"][0]["chunk_id"], "c1");
    }

    #[test]
    fn search_response_without_trace_omits_field() {
        let r = kebab_app::SearchResponse {
            hits: vec![],
            next_cursor: None,
            truncated: false,
            trace: None,
        };
        let v = wire_search_response(&r);
        assert!(v.get("trace").is_none(), "trace field absent when None");
    }
}
