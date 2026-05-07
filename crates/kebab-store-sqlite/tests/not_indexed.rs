//! Signal test: `SqliteStore::open_existing` emits `NotIndexed` when the DB
//! file is absent.

use kebab_store_sqlite::{NotIndexed, SqliteStore};

#[test]
fn not_indexed_signal_emitted_when_db_missing() {
    let dir = tempfile::tempdir().unwrap();
    let nonexistent_db = dir.path().join("does-not-exist.sqlite");
    let res = SqliteStore::open_existing(&nonexistent_db);
    let err = match res {
        Ok(_) => panic!("opening a missing DB should fail"),
        Err(e) => e,
    };
    let signal = err
        .downcast_ref::<NotIndexed>()
        .expect("missing DB error should downcast to NotIndexed");
    assert_eq!(signal.expected, nonexistent_db.to_string_lossy().as_ref());
}

#[test]
fn open_existing_does_not_create_missing_db() {
    let dir = tempfile::tempdir().unwrap();
    let nonexistent_db = dir.path().join("does-not-exist.sqlite");
    let _ = SqliteStore::open_existing(&nonexistent_db);
    assert!(!nonexistent_db.exists(), "open_existing must NOT create the file");
}
