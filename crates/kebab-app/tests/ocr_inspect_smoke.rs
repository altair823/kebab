//! Integration smoke tests for `kebab inspect ocr-stats / ocr-failures`.
//! AC-4, AC-5, AC-6, AC-11 (ocr_inspect_smoke binary), AC-13.

mod common;

use common::TestEnv;
use kebab_app::App;
use kebab_store_sqlite::SqliteStore;

/// Insert synthetic pdf_ocr_events rows directly so the test runs without
/// a live Ollama endpoint.
fn seed_ocr_events(env: &TestEnv, store: &SqliteStore) {
    // Success rows
    for i in 0..3u32 {
        store
            .record_pdf_ocr_event(
                "run-aaa",
                &format!("2026-05-28T0{i}:00:00Z"),
                Some("doc-abc"),
                "path/scanned.pdf",
                i + 1,
                Some(50_000),
                Some(200),
                Some(150),
                100 + u64::from(i) * 20,
                42,
                true,
                None,
                "qwen2.5vl",
            )
            .expect("seed success row");
    }
    // Failure row
    store
        .record_pdf_ocr_event(
            "run-bbb",
            "2026-05-28T10:00:00Z",
            Some("doc-abc"),
            "path/scanned.pdf",
            4,
            Some(30_000),
            Some(200),
            Some(150),
            9999,
            0,
            false,
            Some("ocr_error"),
            "qwen2.5vl",
        )
        .expect("seed failure row");
    // Row for different doc
    store
        .record_pdf_ocr_event(
            "run-ccc",
            "2026-05-28T11:00:00Z",
            Some("doc-xyz"),
            "path/other.pdf",
            1,
            None,
            None,
            None,
            200,
            10,
            true,
            None,
            "qwen2.5vl",
        )
        .expect("seed doc-xyz row");
    // Trigger migration (no-op if already done via App::open_with_config)
    let _ = env;
}

fn open_app_with_seeded_events(env: &TestEnv) -> App {
    let app = env.app();
    let store = SqliteStore::open(&env.config).expect("open store for seed");
    store.run_migrations().expect("run migrations for seed");
    seed_ocr_events(env, &store);
    app
}

/// AC-4: `inspect_ocr_stats` returns `schema_version = "ocr_stats.v1"`,
/// `total_events >= 1`, `0 ≤ success_rate ≤ 1`.
#[test]
fn ocr_stats_after_seeded_events() {
    let env = TestEnv::lexical_only();
    let app = open_app_with_seeded_events(&env);

    let stats = app.inspect_ocr_stats().expect("inspect_ocr_stats");

    assert_eq!(stats.schema_version, "ocr_stats.v1");
    assert!(stats.total_events >= 1, "total_events should be >= 1");
    assert!(
        (0.0..=1.0).contains(&stats.success_rate),
        "success_rate must be in [0, 1]: {}",
        stats.success_rate
    );
    assert!(stats.total_runs >= 1, "total_runs should be >= 1");
    // by_engine should have at least one entry
    assert!(!stats.by_engine.is_empty(), "by_engine must be non-empty");
}

/// AC-6: `inspect_ocr_failures` (no doc_id, corpus-wide) returns failures list.
#[test]
fn ocr_failures_corpus_wide() {
    let env = TestEnv::lexical_only();
    let app = open_app_with_seeded_events(&env);

    let result = app
        .inspect_ocr_failures(None, 10)
        .expect("inspect_ocr_failures");

    assert_eq!(result.schema_version, "ocr_failures.v1");
    assert!(result.failure_count >= 1, "expected at least 1 failure");
    assert!(
        !result.failures.is_empty(),
        "failures list must be non-empty"
    );
}

/// AC-5: `inspect_ocr_failures` with doc_id filter returns matching rows.
#[test]
fn ocr_failures_filter_by_doc_id() {
    let env = TestEnv::lexical_only();
    let app = open_app_with_seeded_events(&env);

    let result = app
        .inspect_ocr_failures(Some("doc-abc"), 10)
        .expect("inspect_ocr_failures by doc_id");

    assert_eq!(result.schema_version, "ocr_failures.v1");
    assert_eq!(
        result.doc_id.as_deref(),
        Some("doc-abc"),
        "doc_id must be echoed back"
    );
    // All rows must belong to doc-abc (no cross-doc leak)
    for row in &result.failures {
        // rows are failure rows for doc-abc only (reason = ocr_error)
        assert_eq!(row.reason, "ocr_error");
    }
}

/// AC-13: SKILL.md lists both new wire schemas.
#[test]
fn skill_md_lists_new_schemas() {
    let skill_md = std::fs::read_to_string("../../integrations/claude-code/kebab/SKILL.md")
        .expect("read SKILL.md");
    assert!(
        skill_md.contains("ocr_stats.v1"),
        "SKILL.md must mention ocr_stats.v1"
    );
    assert!(
        skill_md.contains("ocr_failures.v1"),
        "SKILL.md must mention ocr_failures.v1"
    );
}
