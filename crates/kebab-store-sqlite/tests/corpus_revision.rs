//! p9-fb-19: `corpus_revision` kv counter — exposed on `SqliteStore`
//! so `kebab-app::ingest` can bump after a successful commit and
//! search pagination cursors can snapshot the current value to detect
//! staleness (`stale_cursor`).

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
    let store = SqliteStore::open(&cfg.storage).unwrap();
    store.run_migrations().unwrap();
    store
}

/// Fresh store baseline: V004 seeds `corpus_revision = 0`, then V009,
/// V010, and V011 migrations bump it by one each to invalidate any
/// outstanding pagination cursor — so a fresh store after
/// `run_migrations()` reads back as `3`.
/// (V012 derivation_cache + V013 drop-chunk-aliases are structural/additive
/// and do NOT bump corpus_revision.)
#[test]
fn fresh_store_starts_at_post_migration_baseline() {
    let tmp = TempDir::new().unwrap();
    let store = open_store(&tmp);
    assert_eq!(store.corpus_revision(), 3);
}

/// Each `bump_corpus_revision` returns the new value monotonically
/// from the post-migration baseline (V009 + V010 + V011 → 3).
#[test]
fn bump_increments_monotonically() {
    let tmp = TempDir::new().unwrap();
    let store = open_store(&tmp);
    assert_eq!(store.bump_corpus_revision().unwrap(), 4);
    assert_eq!(store.bump_corpus_revision().unwrap(), 5);
    assert_eq!(store.bump_corpus_revision().unwrap(), 6);
    assert_eq!(store.corpus_revision(), 6);
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
    assert_eq!(store.corpus_revision(), 5);
    assert_eq!(store.bump_corpus_revision().unwrap(), 6);
}
