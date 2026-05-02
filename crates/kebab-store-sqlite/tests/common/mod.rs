//! Shared test scaffolding: temp data_dir + freshly opened SqliteStore.

#![allow(dead_code)]

use std::path::PathBuf;

use kebab_config::Config;
use rusqlite::Connection;
use tempfile::TempDir;

pub struct TestEnv {
    pub temp: TempDir,
    pub config: Config,
}

impl TestEnv {
    pub fn new() -> Self {
        Self::with_threshold(100)
    }

    /// Override the copy-threshold (useful for the reference-mode test
    /// where we want a small file to land on the reference branch).
    pub fn with_threshold(copy_threshold_mb: u64) -> Self {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut config = Config::defaults();
        config.storage.data_dir = temp.path().to_string_lossy().into_owned();
        config.storage.copy_threshold_mb = copy_threshold_mb;
        Self { temp, config }
    }

    pub fn config(&self) -> Config {
        self.config.clone()
    }

    pub fn data_dir(&self) -> PathBuf {
        self.temp.path().to_path_buf()
    }

    pub fn db_path(&self) -> PathBuf {
        self.temp.path().join("kb.sqlite")
    }

    /// Open a side-channel rusqlite connection for direct SQL inspection.
    /// The store-owned connection is held inside a Mutex; opening a fresh
    /// one is the simplest way for tests to peek at row counts / pragmas.
    pub fn with_conn<T>(&self, f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> T {
        let conn = Connection::open(self.db_path()).expect("open side conn");
        f(&conn).expect("with_conn closure")
    }
}
