//! Smoke tests for V008 pdf_ocr_events migration + record/prune API (Enhancement 2).
//! AC-2, AC-3, AC-8.

mod common;

use kebab_store_sqlite::SqliteStore;
use rusqlite::OptionalExtension;

fn open_migrated() -> (common::TestEnv, SqliteStore) {
    let env = common::TestEnv::new();
    let store = SqliteStore::open(&env.config().storage).expect("open");
    store.run_migrations().expect("run migrations");
    (env, store)
}

/// AC-2: V008 migration creates the pdf_ocr_events table.
#[test]
fn v008_pdf_ocr_events_table_exists() {
    let (env, _store) = open_migrated();
    let name: Option<String> = env.with_conn(|c| {
        c.query_row(
            "SELECT name FROM sqlite_master WHERE type='table' AND name='pdf_ocr_events'",
            [],
            |r| r.get(0),
        )
        .optional()
    });
    assert_eq!(
        name.as_deref(),
        Some("pdf_ocr_events"),
        "pdf_ocr_events table must exist after V008"
    );
}

/// AC-8: insert 2 rows with different timestamps; prune with retention_days=0
/// (cutoff = now) → the old row is deleted, count returns 1.
#[test]
fn record_and_prune_pdf_ocr_event() {
    let (_env, store) = open_migrated();

    // Row 1: very old timestamp (1970)
    store
        .record_pdf_ocr_event(
            "run-old",
            "1970-01-01T00:00:00Z",
            Some("doc-old"),
            "path/old.pdf",
            1,
            Some(12345),
            Some(100),
            Some(80),
            250,
            42,
            true,
            None,
            "qwen2.5vl",
        )
        .expect("insert old row");

    // Row 2: future timestamp (far future, so it survives prune)
    store
        .record_pdf_ocr_event(
            "run-new",
            "2099-01-01T00:00:00Z",
            Some("doc-new"),
            "path/new.pdf",
            1,
            None,
            None,
            None,
            180,
            30,
            true,
            None,
            "qwen2.5vl",
        )
        .expect("insert future row");

    // prune with retention_days=0 → cutoff=now → deletes any row with ts < now.
    // The 1970 row should be deleted; the 2099 row survives.
    let pruned = store.prune_pdf_ocr_events(0).expect("prune");
    assert_eq!(pruned, 1, "should have deleted exactly 1 old row");

    // Verify only the future row remains
    let count: i64 = {
        let conn = store.read_conn();
        conn.query_row("SELECT COUNT(*) FROM pdf_ocr_events", [], |r| r.get(0))
            .expect("count")
    };
    assert_eq!(count, 1, "exactly 1 row should survive after prune");
}
