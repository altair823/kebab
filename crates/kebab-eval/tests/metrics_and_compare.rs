//! Integration tests for P5-2: write two synthetic eval runs into a
//! SQLite store, then drive `compute_aggregate` / `store_aggregate` /
//! `compare_runs` end-to-end. Mirrors the test plan in
//! `tasks/p5/p5-2-metrics-compare.md`.
//!
//! Snapshot of `CompareReport` JSON is pinned at
//! `tests/fixtures/eval/compare-1.json`.

use std::fs;
use std::path::PathBuf;

use kebab_config::Config;
use kebab_core::{
    ChunkId, ChunkerVersion, Citation, DocumentId, IndexVersion, Lang,
    RetrievalDetail, SearchHit, SearchMode,
    asset::WorkspacePath,
};
use kebab_eval::{
    AggregateMetrics, CompareOpts, CompareReport, ComparisonKind, GoldenQuery, QueryResult,
    compare_runs_with_config, compute_aggregate_with_config, store_aggregate_with_config,
};
use kebab_store_sqlite::{EvalRunRow, SqliteStore};
use tempfile::TempDir;
use time::OffsetDateTime;

fn cfg_with_data_dir(tmp: &TempDir, golden_yaml: &str) -> Config {
    let mut cfg = Config::defaults();
    cfg.storage.data_dir = tmp.path().to_string_lossy().into_owned();
    cfg.storage.runs_dir = tmp.path().join("runs").to_string_lossy().into_owned();
    cfg.storage.copy_threshold_mb = 0;
    let golden_path = tmp.path().join("golden.yaml");
    fs::write(&golden_path, golden_yaml).unwrap();
    // Point both metrics + compare at the temp golden via env override.
    // SAFELY scoped — `set_var` is process-global so callers serialise
    // tests via the `serial_test`-style guard below.
    unsafe {
        std::env::set_var("KEBAB_EVAL_GOLDEN", &golden_path);
    }
    cfg
}

fn golden_yaml_basic() -> &'static str {
    r#"
- id: q-001
  query: hit at rank 1
  expected_doc_ids: ["doc-1"]
  expected_chunk_ids: ["chunk-1"]
- id: q-002
  query: hit at rank 4
  expected_doc_ids: ["doc-2"]
  expected_chunk_ids: ["chunk-2"]
- id: q-003
  query: miss everywhere
  expected_doc_ids: ["doc-3"]
  expected_chunk_ids: ["chunk-3"]
"#
}

fn hit(rank: u32, chunk_id: &str, doc_id: &str) -> SearchHit {
    SearchHit {
        rank,
        chunk_id: ChunkId(chunk_id.into()),
        doc_id: DocumentId(doc_id.into()),
        doc_path: WorkspacePath::new(format!("docs/{doc_id}.md")).unwrap(),
        heading_path: vec!["root".into()],
        section_label: None,
        snippet: "snip".into(),
        citation: Citation::Line {
            path: WorkspacePath::new(format!("docs/{doc_id}.md")).unwrap(),
            start: 1,
            end: 1,
            section: None,
        },
        retrieval: RetrievalDetail {
            method: SearchMode::Lexical,
            fusion_score: 1.0 / f32::from(u16::try_from(rank).unwrap_or(1)),
            lexical_score: Some(1.0),
            vector_score: None,
            lexical_rank: Some(rank),
            vector_rank: None,
        },
        index_version: IndexVersion("idx@1".into()),
        embedding_model: None,
        chunker_version: ChunkerVersion("test@1".into()),
        // fb-32: synthetic eval fixtures don't exercise staleness;
        // pin UNIX_EPOCH + stale=false so hits stay deterministic.
        indexed_at: OffsetDateTime::UNIX_EPOCH,
        stale: false,
        score_kind: kebab_core::ScoreKind::Rrf,
    }
}

fn qr(query_id: &str, hits: Vec<SearchHit>) -> QueryResult {
    QueryResult {
        query_id: query_id.into(),
        query: format!("query for {query_id}"),
        mode: SearchMode::Lexical,
        hits_top_k: hits,
        answer: None,
        elapsed_ms: 1,
        error: None,
    }
}

fn write_run(
    store: &SqliteStore,
    run_id: &str,
    chunker_version: &str,
    created_at: OffsetDateTime,
    queries: Vec<QueryResult>,
) {
    let snapshot = serde_json::json!({
        "config": {},
        "chunker_version": chunker_version,
    });
    let snapshot_text = serde_json::to_string(&snapshot).unwrap();
    let row = EvalRunRow {
        run_id,
        suite: "golden",
        config_snapshot_json: &snapshot_text,
        aggregate_json: "{}",
        commit_hash: Some("0000000"),
        created_at,
    };
    let results: Vec<(String, String)> = queries
        .into_iter()
        .map(|qr| {
            let json = serde_json::to_string(&qr).unwrap();
            (qr.query_id, json)
        })
        .collect();
    store.record_eval_run_with_results(&row, &results).unwrap();
}

/// Each test mutates a process-global env var (`KEBAB_EVAL_GOLDEN`) and
/// expects to see its own write. Take this mutex around the body of
/// every test that touches `KEBAB_EVAL_GOLDEN` so two concurrent test
/// threads don't trip over each other's golden YAML.
fn env_guard() -> std::sync::MutexGuard<'static, ()> {
    use std::sync::{Mutex, OnceLock};
    static M: OnceLock<Mutex<()>> = OnceLock::new();
    M.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

#[test]
fn compute_and_store_aggregate_round_trips() {
    let _g = env_guard();
    let tmp = TempDir::new().unwrap();
    let cfg = cfg_with_data_dir(&tmp, golden_yaml_basic());
    let store = SqliteStore::open(&cfg).unwrap();
    store.run_migrations().unwrap();
    let now = OffsetDateTime::UNIX_EPOCH;
    write_run(
        &store,
        "run_a",
        "test@1",
        now,
        vec![
            qr("q-001", vec![hit(1, "chunk-1", "doc-1")]),
            qr(
                "q-002",
                vec![
                    hit(1, "x", "x"),
                    hit(2, "x", "x"),
                    hit(3, "x", "x"),
                    hit(4, "chunk-2", "doc-2"),
                ],
            ),
            qr("q-003", vec![hit(1, "x", "x")]),
        ],
    );
    drop(store);

    let agg = compute_aggregate_with_config(&cfg, "run_a").unwrap();
    // hit@1 = 1/3, hit@5 = 2/3, MRR = (1 + 0.25 + 0)/3 ≈ 0.4167.
    assert_eq!(agg.hit_at_k[&1], 0.3333);
    assert_eq!(agg.hit_at_k[&5], 0.6667);
    assert_eq!(agg.mrr, 0.4167);

    store_aggregate_with_config(&cfg, "run_a", &agg).unwrap();
    let store = SqliteStore::open(&cfg).unwrap();
    let row = store.load_eval_run("run_a").unwrap().unwrap();
    let parsed: AggregateMetrics = serde_json::from_str(&row.aggregate_json).unwrap();
    // f32 round-trip via JSON is exact for our 4-decimal-rounded
    // values, so direct equality is OK here (NaN fields are handled
    // by the `serialize_f32_nan_as_null` round-trip — `citation_coverage`
    // and `refusal_correctness` come back as NaN). Compare on JSON
    // bytes instead, which is what `store_aggregate` writes.
    assert_eq!(
        serde_json::to_string(&parsed).unwrap(),
        serde_json::to_string(&agg).unwrap()
    );
}

#[test]
fn store_aggregate_rejects_missing_run() {
    let _g = env_guard();
    let tmp = TempDir::new().unwrap();
    let cfg = cfg_with_data_dir(&tmp, golden_yaml_basic());
    let agg = AggregateMetrics {
        hit_at_k: Default::default(),
        mrr: 0.0,
        recall_at_k_doc: Default::default(),
        precision_at_k_chunk: Default::default(),
        citation_coverage: f32::NAN,
        groundedness: 0.0,
        empty_result_rate: 0.0,
        refusal_correctness: f32::NAN,
        total_queries: 0,
        failed_queries: 0,
    };
    let err = store_aggregate_with_config(&cfg, "run_does_not_exist", &agg).unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("run_does_not_exist"), "msg = {msg}");
}

#[test]
fn compare_runs_classifies_win_loss_draw_regression() {
    let _g = env_guard();
    let tmp = TempDir::new().unwrap();
    let cfg = cfg_with_data_dir(&tmp, golden_yaml_basic());
    let store = SqliteStore::open(&cfg).unwrap();
    store.run_migrations().unwrap();
    let now = OffsetDateTime::UNIX_EPOCH;
    // Run A:
    //   q-001 rank 1 → hit
    //   q-002 rank 4 → hit
    //   q-003 miss
    write_run(
        &store,
        "run_a",
        "test@1",
        now,
        vec![
            qr("q-001", vec![hit(1, "chunk-1", "doc-1")]),
            qr(
                "q-002",
                vec![
                    hit(1, "x", "x"),
                    hit(2, "x", "x"),
                    hit(3, "x", "x"),
                    hit(4, "chunk-2", "doc-2"),
                ],
            ),
            qr("q-003", vec![hit(1, "x", "x")]),
        ],
    );
    // Run B:
    //   q-001 rank 2 → still hit (Loss vs A — worse rank)
    //   q-002 rank 1 → hit (Win — improved rank)
    //   q-003 hit @ rank 1 → hit (Win — was miss in A)
    write_run(
        &store,
        "run_b",
        "test@1",
        now,
        vec![
            qr("q-001", vec![hit(1, "x", "x"), hit(2, "chunk-1", "doc-1")]),
            qr("q-002", vec![hit(1, "chunk-2", "doc-2")]),
            qr("q-003", vec![hit(1, "chunk-3", "doc-3")]),
        ],
    );
    drop(store);

    let report = compare_runs_with_config(&cfg, "run_a", "run_b", &CompareOpts::default()).unwrap();
    let by_id: std::collections::HashMap<&str, &kebab_eval::QueryComparison> =
        report.per_query.iter().map(|c| (c.query_id.as_str(), c)).collect();
    assert_eq!(by_id["q-001"].kind, ComparisonKind::Loss);
    assert_eq!(by_id["q-002"].kind, ComparisonKind::Win);
    assert_eq!(by_id["q-003"].kind, ComparisonKind::Win);
    assert_eq!(report.deltas["chunker_version_match"], "exact");
}

#[test]
fn compare_strict_mode_refuses_chunker_version_mismatch() {
    let _g = env_guard();
    let tmp = TempDir::new().unwrap();
    let cfg = cfg_with_data_dir(&tmp, golden_yaml_basic());
    let store = SqliteStore::open(&cfg).unwrap();
    store.run_migrations().unwrap();
    let now = OffsetDateTime::UNIX_EPOCH;
    write_run(&store, "run_a", "test@1", now, vec![qr("q-001", vec![hit(1, "chunk-1", "doc-1")])]);
    write_run(&store, "run_b", "test@2", now, vec![qr("q-001", vec![hit(1, "chunk-1", "doc-1")])]);
    drop(store);

    let opts = CompareOpts {
        strict_chunker_version: true,
    };
    let err = compare_runs_with_config(&cfg, "run_a", "run_b", &opts).unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("chunker_version mismatch"), "msg = {msg}");
}

#[test]
fn compare_graceful_falls_back_to_doc_id() {
    let _g = env_guard();
    let tmp = TempDir::new().unwrap();
    let cfg = cfg_with_data_dir(&tmp, golden_yaml_basic());
    let store = SqliteStore::open(&cfg).unwrap();
    store.run_migrations().unwrap();
    let now = OffsetDateTime::UNIX_EPOCH;
    // Run A uses test@1 chunker; run B uses test@2 — chunk_ids no longer
    // align, but doc_ids do.
    write_run(&store, "run_a", "test@1", now, vec![qr("q-001", vec![hit(1, "chunk-1", "doc-1")])]);
    write_run(
        &store,
        "run_b",
        "test@2",
        now,
        // Different chunk_id, same doc_id → exact-mode matching would
        // miss; doc-id fallback should still register a hit.
        vec![qr("q-001", vec![hit(1, "chunk-1-renamed", "doc-1")])],
    );
    drop(store);

    let report = compare_runs_with_config(&cfg, "run_a", "run_b", &CompareOpts::default()).unwrap();
    assert_eq!(report.deltas["chunker_version_match"], "fallback_doc");
    let q1 = report.per_query.iter().find(|c| c.query_id == "q-001").unwrap();
    // Both runs hit doc-1 at rank 1 → Draw.
    assert_eq!(q1.kind, ComparisonKind::Draw);
    assert_eq!(q1.a_hit_rank, Some(1));
    assert_eq!(q1.b_hit_rank, Some(1));
}

#[test]
fn compare_report_snapshot_matches_fixture() {
    let _g = env_guard();
    let tmp = TempDir::new().unwrap();
    let cfg = cfg_with_data_dir(&tmp, golden_yaml_basic());
    let store = SqliteStore::open(&cfg).unwrap();
    store.run_migrations().unwrap();
    let now = OffsetDateTime::UNIX_EPOCH;
    write_run(
        &store,
        "run_a",
        "test@1",
        now,
        vec![
            qr("q-001", vec![hit(1, "chunk-1", "doc-1")]),
            qr(
                "q-002",
                vec![
                    hit(1, "x", "x"),
                    hit(2, "x", "x"),
                    hit(3, "x", "x"),
                    hit(4, "chunk-2", "doc-2"),
                ],
            ),
            qr("q-003", vec![hit(1, "x", "x")]),
        ],
    );
    write_run(
        &store,
        "run_b",
        "test@1",
        now,
        vec![
            qr("q-001", vec![hit(1, "x", "x"), hit(2, "chunk-1", "doc-1")]),
            qr("q-002", vec![hit(1, "chunk-2", "doc-2")]),
            qr("q-003", vec![hit(1, "chunk-3", "doc-3")]),
        ],
    );
    drop(store);

    let report = compare_runs_with_config(&cfg, "run_a", "run_b", &CompareOpts::default()).unwrap();
    let actual = projection(&report);
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("eval")
        .join("compare-1.json");
    if std::env::var("UPDATE_SNAPSHOTS").is_ok() {
        fs::write(&fixture, format!("{}\n", serde_json::to_string_pretty(&actual).unwrap()))
            .unwrap();
    }
    let expected_text = fs::read_to_string(&fixture)
        .unwrap_or_else(|e| panic!("missing fixture {}: {e}", fixture.display()));
    let expected: serde_json::Value = serde_json::from_str(&expected_text).unwrap();
    assert_eq!(actual, expected, "compare report drift — re-run with UPDATE_SNAPSHOTS=1 if intended");
}

/// Project a `CompareReport` to the stable-across-runs subset.
/// `aggregate_*` and `deltas` are deterministic; per-query rows keep
/// only `(query_id, kind, ranks, note)` and discard volatile fields.
fn projection(r: &CompareReport) -> serde_json::Value {
    serde_json::json!({
        "run_a": r.run_a,
        "run_b": r.run_b,
        "aggregate_a": r.aggregate_a,
        "aggregate_b": r.aggregate_b,
        "deltas": r.deltas,
        "per_query": r.per_query,
    })
}

#[test]
fn render_report_md_is_human_readable() {
    let _g = env_guard();
    let tmp = TempDir::new().unwrap();
    let cfg = cfg_with_data_dir(&tmp, golden_yaml_basic());
    let store = SqliteStore::open(&cfg).unwrap();
    store.run_migrations().unwrap();
    let now = OffsetDateTime::UNIX_EPOCH;
    write_run(
        &store,
        "run_a",
        "test@1",
        now,
        vec![qr("q-001", vec![hit(1, "chunk-1", "doc-1")])],
    );
    write_run(
        &store,
        "run_b",
        "test@1",
        now,
        vec![qr("q-001", vec![hit(2, "chunk-1", "doc-1")])],
    );
    drop(store);

    let report = compare_runs_with_config(&cfg, "run_a", "run_b", &CompareOpts::default()).unwrap();
    let md = kebab_eval::render_report_md(&report);
    assert!(md.starts_with("# Eval compare:"), "md = {md}");
    assert!(md.contains("hit@1"));
    assert!(md.contains("MRR"));
    assert!(md.contains("Wins"));
    assert!(md.contains("q-001"));
}

#[test]
fn lang_default_is_used_when_omitted_in_yaml() {
    // Round-trip safety: GoldenQuery without `lang` should parse fine.
    let yaml = "- id: only\n  query: q\n";
    let _g = env_guard();
    let tmp = TempDir::new().unwrap();
    let golden = tmp.path().join("g.yaml");
    fs::write(&golden, yaml).unwrap();
    let qs: Vec<GoldenQuery> = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(qs.len(), 1);
    assert_eq!(qs[0].lang, Lang(String::new()));
}
