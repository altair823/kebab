//! `JobRepo` smoke tests: create → progress → finish, list filters.

use kebab_core::{JobFilter, JobKind, JobRepo, JobStatus};
use kebab_store_sqlite::SqliteStore;
use serde_json::json;

mod common;

#[test]
fn create_then_progress_then_finish() {
    let env = common::TestEnv::new();
    let store = SqliteStore::open(&env.config().storage).unwrap();
    store.run_migrations().unwrap();

    let id = store
        .create(JobKind::Ingest, json!({"path": "notes/x.md"}))
        .unwrap();
    // Status starts pending.
    let row = store.list(&JobFilter::default()).unwrap();
    assert_eq!(row.len(), 1);
    assert_eq!(row[0].status, JobStatus::Pending);

    // First progress flips pending → running.
    store
        .update_progress(&id, json!({"processed": 1, "total": 10}))
        .unwrap();
    let row = store.list(&JobFilter::default()).unwrap();
    assert_eq!(row[0].status, JobStatus::Running);
    assert_eq!(row[0].progress.as_ref().unwrap()["total"], json!(10));

    // Finish with success.
    store.finish(&id, JobStatus::Succeeded, None).unwrap();
    let row = store.list(&JobFilter::default()).unwrap();
    assert_eq!(row[0].status, JobStatus::Succeeded);
    assert!(row[0].finished_at.is_some());
    assert!(row[0].error.is_none());
}

#[test]
fn finish_with_error_message_is_round_trippable() {
    let env = common::TestEnv::new();
    let store = SqliteStore::open(&env.config().storage).unwrap();
    store.run_migrations().unwrap();

    let id = store.create(JobKind::Embed, json!({})).unwrap();
    store
        .finish(&id, JobStatus::Failed, Some("boom: model not pulled"))
        .unwrap();

    let row = store.list(&JobFilter::default()).unwrap();
    assert_eq!(row[0].status, JobStatus::Failed);
    assert_eq!(
        row[0].error.as_deref(),
        Some("boom: model not pulled"),
        "error message must round-trip"
    );
}

#[test]
fn list_filters_status_and_kind() {
    let env = common::TestEnv::new();
    let store = SqliteStore::open(&env.config().storage).unwrap();
    store.run_migrations().unwrap();

    // Two ingest jobs (one finished succeeded, one pending) + one embed.
    let a = store.create(JobKind::Ingest, json!({"a": 1})).unwrap();
    let _b = store.create(JobKind::Ingest, json!({"b": 1})).unwrap();
    let _c = store.create(JobKind::Embed, json!({"c": 1})).unwrap();
    store.finish(&a, JobStatus::Succeeded, None).unwrap();

    let by_status_succeeded = store
        .list(&JobFilter {
            status: Some(JobStatus::Succeeded),
            kind: None,
        })
        .unwrap();
    assert_eq!(by_status_succeeded.len(), 1);
    assert_eq!(by_status_succeeded[0].kind, JobKind::Ingest);

    let by_kind_embed = store
        .list(&JobFilter {
            status: None,
            kind: Some(JobKind::Embed),
        })
        .unwrap();
    assert_eq!(by_kind_embed.len(), 1);
    assert_eq!(by_kind_embed[0].kind, JobKind::Embed);
}
