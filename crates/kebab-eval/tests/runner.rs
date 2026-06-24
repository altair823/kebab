//! Runner integration tests for `kb-eval` (P5-1).
//!
//! Drives [`kebab_eval::run_eval_with_config`] end-to-end against a
//! TempDir-backed config:
//!
//! - tiny seeded SQLite corpus (3 docs / 3 chunks) used as the
//!   workspace's source-of-truth,
//! - lexical-only retrieval (`SearchMode::Lexical`) so no embedder is
//!   required (`models.embedding.provider = "none"`),
//! - golden YAML pointed at via `KEBAB_EVAL_GOLDEN`.
//!
//! Determinism: lexical-only with a fixed seed corpus produces
//! byte-identical `per_query.jsonl` content (modulo `run_id` /
//! `created_at`, which we strip when comparing).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use kebab_config::Config;
use kebab_core::SearchMode;
use kebab_eval::{EvalRunOpts, QueryResult, run_eval_with_config};
use kebab_store_sqlite::SqliteStore;
use rusqlite::params;
use tempfile::TempDir;

/// `KEBAB_EVAL_GOLDEN` is process-global state. Tests touching it must
/// serialize so they don't trample each other when `cargo test`
/// runs them in parallel.
static GOLDEN_ENV_LOCK: Mutex<()> = Mutex::new(());

// ── shared scaffolding ───────────────────────────────────────────────────────

struct RunEnv {
    temp: TempDir,
    config: Config,
}

impl RunEnv {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let mut config = Config::defaults();
        config.storage.data_dir = temp.path().to_string_lossy().into_owned();
        // Force lexical-only behavior so the runner never tries to
        // load fastembed during integration tests.
        config.models.embedding.provider = "none".to_string();
        config.models.embedding.dimensions = 0;
        // Pin search defaults so test asserts are stable.
        config.search.default_k = 5;

        let store = SqliteStore::open(&config).unwrap();
        store.run_migrations().unwrap();
        seed_corpus(&store);
        Self { temp, config }
    }

    fn data_dir(&self) -> PathBuf {
        self.temp.path().to_path_buf()
    }
}

/// Seed three (asset, document, chunk) triples with text the test
/// queries can match against the FTS5 lexical index.
fn seed_corpus(store: &SqliteStore) {
    let conn = store.read_conn();
    for (i, text) in [
        "Rust ownership and borrow checker basics.",
        "Cargo workspace members are listed in workspace.members.",
        "Markdown chunking respects heading boundaries.",
    ]
    .iter()
    .enumerate()
    {
        let doc_id = format!("doc{i:032}");
        let chunk_id = format!("chunk{i:030}");
        let asset_id = format!("asset{i:030}");
        let path = format!("notes/{i}.md");
        conn.execute(
            "INSERT INTO assets (
                asset_id, source_uri, workspace_path, media_type, byte_len,
                checksum, storage_kind, storage_path, discovered_at
             ) VALUES (?, ?, ?, '\"markdown\"', 0,
                       'deadbeefdeadbeefdeadbeefdeadbeef',
                       'reference', ?, '1970-01-01T00:00:00Z')",
            params![asset_id, format!("file:///{path}"), path, path],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO documents (
                doc_id, asset_id, workspace_path, title, lang, source_type,
                trust_level, parser_version, doc_version, schema_version,
                metadata_json, provenance_json, created_at, updated_at
             ) VALUES (?, ?, ?, NULL, 'en', 'markdown', 'primary', 'v1', 1, 1,
                       '{}', '{}', '1970-01-01T00:00:00Z', '1970-01-01T00:00:00Z')",
            params![doc_id, asset_id, path],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO chunks (
                chunk_id, doc_id, text, heading_path_json, section_label,
                source_spans_json, token_estimate, chunker_version,
                policy_hash, block_ids_json, created_at
             ) VALUES (?, ?, ?, '[]', NULL,
                       '[{\"kind\":\"line\",\"start\":1,\"end\":3}]',
                       1, 'md-heading-v1', 'h', '[]', '1970-01-01T00:00:00Z')",
            params![chunk_id, doc_id, text],
        )
        .unwrap();
    }
    // Build the FTS index so lexical search returns hits. Reuses the
    // same connection guard rather than reopening — the SAVEPOINT
    // protocol nests correctly under the existing read_conn lock.
    kebab_store_sqlite::rebuild_chunks_fts(&conn).unwrap();
    drop(conn);
}

fn write_golden(dir: &Path, body: &str) -> PathBuf {
    let path = dir.join("golden.yaml");
    fs::write(&path, body).unwrap();
    path
}

/// Bind a fresh ephemeral port, then release it. The returned URL
/// points at a port that was just freed; very likely still unbound
/// when the test issues its outbound connection a moment later, in
/// which case `connect()` fails fast with `ECONNREFUSED`. Beats
/// hard-coding port 1 which can timeout slowly on hardened hosts.
fn unreachable_endpoint() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    format!("http://127.0.0.1:{port}")
}

fn lexical_opts() -> EvalRunOpts {
    EvalRunOpts {
        suite: "test".to_string(),
        mode: SearchMode::Lexical,
        with_rag: false,
        k: 5,
        temperature: Some(0.0),
        seed: Some(0),
    }
}

/// Run the eval after pointing `KEBAB_EVAL_GOLDEN` at `yaml`. The env
/// guard must outlive the call so concurrent tests don't reset the
/// var mid-run.
fn run_with_golden<F: FnOnce() -> R, R>(yaml: &Path, f: F) -> R {
    let _g = GOLDEN_ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // SAFETY: `KEBAB_EVAL_GOLDEN` is a benign env var; the GOLDEN_ENV_LOCK
    // serializes mutations so concurrent tests don't race.
    unsafe {
        std::env::set_var("KEBAB_EVAL_GOLDEN", yaml);
    }
    let out = f();
    unsafe {
        std::env::remove_var("KEBAB_EVAL_GOLDEN");
    }
    out
}

// ── 1. elapsed_ms recorded for every query ──────────────────────────────────

#[test]
fn runner_records_elapsed_for_every_query() {
    let env = RunEnv::new();
    let yaml = write_golden(
        env.data_dir().as_path(),
        "- id: q1\n  query: ownership\n- id: q2\n  query: heading\n- id: q3\n  query: workspace\n",
    );

    let run = run_with_golden(&yaml, || {
        run_eval_with_config(&env.config, &lexical_opts()).unwrap()
    });

    assert_eq!(run.per_query.len(), 3);
    for qr in &run.per_query {
        assert_eq!(qr.mode, SearchMode::Lexical);
        // `elapsed_ms` is `u32`; the assertion that it's a valid
        // unsigned value is implicit. We additionally bound it well
        // below the 4G ceiling to detect a stuck/overflow path.
        assert!(
            qr.elapsed_ms < 60_000,
            "elapsed_ms suspicious: {}",
            qr.elapsed_ms
        );
    }
    // The id-list round-trips into the per-query records.
    let ids: Vec<&str> = run.per_query.iter().map(|q| q.query_id.as_str()).collect();
    assert_eq!(ids, vec!["q1", "q2", "q3"]);
}

// ── 2. config snapshot carries the documented version fields ────────────────

#[test]
fn runner_records_config_snapshot_with_versions() {
    let env = RunEnv::new();
    let yaml = write_golden(env.data_dir().as_path(), "- id: q1\n  query: ownership\n");

    let run = run_with_golden(&yaml, || {
        run_eval_with_config(&env.config, &lexical_opts()).unwrap()
    });

    let snap = &run.config_snapshot_json;
    assert!(snap.get("config").is_some(), "config field missing");
    assert_eq!(
        snap.pointer("/chunker_version"),
        // Pre-existing drift from merged PR #209 (md-heading-v2 default):
        // the config default chunker_version is now md-heading-v2.
        Some(&serde_json::Value::String("md-heading-v2".to_string())),
    );
    assert!(snap.pointer("/embedding/model").is_some());
    assert!(snap.pointer("/embedding/dimensions").is_some());
    assert!(snap.pointer("/llm/model_id").is_some());
    assert_eq!(
        snap.pointer("/prompt_template_version"),
        Some(&serde_json::Value::String("rag-v4".to_string())),
    );
    assert!(snap.pointer("/score_gate").is_some());
    assert!(snap.pointer("/rrf_k").is_some());
}

// ── 3. failing query (ask path with no Ollama) records an error ─────────────

#[test]
fn runner_captures_per_query_error_when_rag_unreachable() {
    let env = RunEnv::new();
    // Point Ollama at an unbound port so `ask_with_config` surfaces a
    // connection error per query. We use bind-then-release rather than
    // a hard-coded `:1` because port 1 is reserved-but-not-guaranteed-
    // unbound (some hardened systems answer with ICMP unreachable
    // instantly, others timeout slowly). TOCTOU race is theoretically
    // possible but rare in practice and faster-failing than `:1`.
    let mut config = env.config.clone();
    config.models.llm.endpoint = unreachable_endpoint();

    let yaml = write_golden(env.data_dir().as_path(), "- id: q1\n  query: ownership\n");

    let opts = EvalRunOpts {
        with_rag: true,
        ..lexical_opts()
    };
    let run = run_with_golden(&yaml, || run_eval_with_config(&config, &opts).unwrap());

    let qr = &run.per_query[0];
    // hits_top_k still populated by lexical search before the RAG attempt.
    assert!(
        !qr.hits_top_k.is_empty(),
        "lexical hits should populate before RAG attempt"
    );
    assert!(qr.answer.is_none(), "no answer when RAG fails");
    assert!(qr.error.is_some(), "error must be recorded");
}

// ── 4. eval_runs + eval_query_results rows persisted ────────────────────────

#[test]
fn runner_persists_eval_run_and_query_result_rows() {
    let env = RunEnv::new();
    let yaml = write_golden(
        env.data_dir().as_path(),
        "- id: q1\n  query: ownership\n- id: q2\n  query: heading\n",
    );

    let run = run_with_golden(&yaml, || {
        run_eval_with_config(&env.config, &lexical_opts()).unwrap()
    });

    // Reopen the same SQLite file with a new store handle and read
    // the rows back. We use the inherent `read_conn` helper rather
    // than rusqlite directly because the latter would require kb-eval
    // to add a runtime rusqlite dep (forbidden by the spec).
    let store = SqliteStore::open(&env.config).unwrap();
    let conn = store.read_conn();

    let n_runs: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM eval_runs WHERE run_id = ?",
            params![run.run_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n_runs, 1);

    let n_results: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM eval_query_results WHERE run_id = ?",
            params![run.run_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n_results, 2);
}

// ── 5. per_query.jsonl mirror exists and round-trips ────────────────────────

#[test]
fn runner_writes_per_query_jsonl_mirror() {
    let env = RunEnv::new();
    let yaml = write_golden(
        env.data_dir().as_path(),
        "- id: q1\n  query: ownership\n- id: q2\n  query: heading\n",
    );

    let run = run_with_golden(&yaml, || {
        run_eval_with_config(&env.config, &lexical_opts()).unwrap()
    });

    let mirror = env
        .data_dir()
        .join("runs")
        .join(&run.run_id)
        .join("per_query.jsonl");
    assert!(
        mirror.exists(),
        "per_query.jsonl missing at {}",
        mirror.display()
    );
    let body = fs::read_to_string(&mirror).unwrap();
    let lines: Vec<&str> = body.lines().collect();
    assert_eq!(lines.len(), 2);
    let parsed: Vec<QueryResult> = lines
        .iter()
        .map(|l| serde_json::from_str::<QueryResult>(l).expect("valid JSONL line"))
        .collect();
    assert_eq!(parsed[0].query_id, "q1");
    assert_eq!(parsed[1].query_id, "q2");
}

// ── 6. determinism — repeating the run produces byte-identical per_query JSON ─

#[test]
fn runner_lexical_is_deterministic_per_query_payload() {
    let env = RunEnv::new();
    let yaml = write_golden(
        env.data_dir().as_path(),
        "- id: q1\n  query: ownership\n- id: q2\n  query: heading\n",
    );

    let mut run_a = run_with_golden(&yaml, || {
        run_eval_with_config(&env.config, &lexical_opts()).unwrap()
    });
    let mut run_b = run_with_golden(&yaml, || {
        run_eval_with_config(&env.config, &lexical_opts()).unwrap()
    });

    // Run-level fields (`run_id`, `created_at`) intentionally diverge;
    // the per-query payload (which is what the snapshot fixture pins)
    // must be byte-identical EXCEPT for `elapsed_ms`. Timing-sensitive
    // fields aren't determinism signals — they're µs-scale wall-clock
    // jitter and would otherwise make this assertion a flaky one (a 0
    // vs 1 ms divergence was observed under contended-CI load). Normalize
    // before comparing; see test #7 for the same exclusion done via a
    // projection.
    for qr in run_a.per_query.iter_mut().chain(run_b.per_query.iter_mut()) {
        qr.elapsed_ms = 0;
    }
    let a_json = serde_json::to_string(&run_a.per_query).unwrap();
    let b_json = serde_json::to_string(&run_b.per_query).unwrap();
    assert_eq!(
        a_json, b_json,
        "lexical-only per_query payload must be byte-identical across runs (timing normalized)"
    );
}

// ── 7. snapshot — per_query JSON pinned to fixtures/eval/run-1.json ─────────

#[test]
fn runner_per_query_snapshot_matches_fixture() {
    let env = RunEnv::new();
    let yaml = write_golden(
        env.data_dir().as_path(),
        "- id: q1\n  query: ownership\n- id: q2\n  query: heading\n",
    );

    let run = run_with_golden(&yaml, || {
        run_eval_with_config(&env.config, &lexical_opts()).unwrap()
    });

    // Fixture pins the *shape* of the per-query payload, including the
    // first hit's stable scalar fields (chunk_id, doc_id, heading_path,
    // fusion_score). FTS scores depend on the SQLite version, so the
    // fusion_score is captured into the fixture from one passing run
    // and must remain stable across re-runs against the same seeded
    // corpus. Timing-sensitive fields (`elapsed_ms`, raw `Instant`
    // byproducts) are excluded. Verifying byte stability is the
    // determinism test (#6); this test verifies the field set +
    // ordering is stable.
    let projection: Vec<_> = run
        .per_query
        .iter()
        .map(|qr| {
            let first_hit = qr.hits_top_k.first().map(|h| {
                serde_json::json!({
                    "chunk_id": h.chunk_id,
                    "doc_id": h.doc_id,
                    "heading_path": h.heading_path,
                    "score": h.retrieval.fusion_score,
                })
            });
            serde_json::json!({
                "query_id": qr.query_id,
                "query": qr.query,
                "mode": qr.mode,
                "hits_count": qr.hits_top_k.len(),
                "first_hit": first_hit,
                "has_answer": qr.answer.is_some(),
                "error": qr.error,
            })
        })
        .collect();
    let actual = serde_json::to_string_pretty(&projection).unwrap();

    let fixture_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/eval/run-1.json");

    if std::env::var("UPDATE_SNAPSHOTS").is_ok() {
        fs::create_dir_all(fixture_path.parent().unwrap()).unwrap();
        fs::write(&fixture_path, &actual).unwrap();
    }

    let expected = fs::read_to_string(&fixture_path)
        .unwrap_or_else(|e| panic!("read snapshot {}: {e}", fixture_path.display()));
    assert_eq!(
        actual.trim(),
        expected.trim(),
        "snapshot drift — re-run with UPDATE_SNAPSHOTS=1 to refresh"
    );
}
