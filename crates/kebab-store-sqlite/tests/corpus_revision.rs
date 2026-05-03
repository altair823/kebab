//! p9-fb-19: `corpus_revision` kv counter — exposed on `SqliteStore`
//! so `kebab-app::ingest` can bump after a successful commit and
//! `App::search`'s LRU cache key can snapshot the current value for
//! invalidation.

use kebab_config::Config;
use kebab_store_sqlite::SqliteStore;
use tempfile::TempDir;

fn config_for(tmp: &TempDir) -> Config {
    let mut c = Config::defaults();
    c.storage.data_dir = tmp.path().to_string_lossy().into_owned();
    c
}

fn open_store(tmp: &TempDir) -> SqliteStore {
    let cfg = config_for(tmp);
    let store = SqliteStore::open(&cfg).unwrap();
    store.run_migrations().unwrap();
    store
}

/// Fresh store seeds `corpus_revision = 0` (per V004 INSERT).
#[test]
fn fresh_store_starts_at_zero() {
    let tmp = TempDir::new().unwrap();
    let store = open_store(&tmp);
    assert_eq!(store.corpus_revision(), 0);
}

/// Each `bump_corpus_revision` returns the new value monotonically.
#[test]
fn bump_increments_monotonically() {
    let tmp = TempDir::new().unwrap();
    let store = open_store(&tmp);
    assert_eq!(store.bump_corpus_revision().unwrap(), 1);
    assert_eq!(store.bump_corpus_revision().unwrap(), 2);
    assert_eq!(store.bump_corpus_revision().unwrap(), 3);
    assert_eq!(store.corpus_revision(), 3);
}

/// `corpus_revision` survives a store re-open (persisted in SQLite).
#[test]
fn revision_persists_across_reopen() {
    let tmp = TempDir::new().unwrap();
    {
        let store = open_store(&tmp);
        store.bump_corpus_revision().unwrap();
        store.bump_corpus_revision().unwrap();
    } // store dropped — file closed
    let store = open_store(&tmp);
    assert_eq!(store.corpus_revision(), 2);
    assert_eq!(store.bump_corpus_revision().unwrap(), 3);
}
