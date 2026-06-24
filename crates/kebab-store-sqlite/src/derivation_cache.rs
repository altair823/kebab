//! Content-hash derivation cache store (design 2026-05-31 §3.2 / §3.5).
//!
//! Backs the `derivation_cache` table (`V012`). The cache stores expensive
//! ingest derivations (embedding vectors, LLM aliases, optional Korean
//! tokens) keyed by `derivation_cache_key` (§3.1). It is a pure performance
//! layer: corruption / deletion only forces recomputation, never wrong
//! results (§3.5). Timestamps follow the same RFC3339 `OffsetDateTime`
//! formatting the asset / document / embedding writers use.

use anyhow::{Context, Result};
use rusqlite::{OptionalExtension, params};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::error::StoreError;
use crate::store::SqliteStore;

impl SqliteStore {
    /// Look up a cached derivation payload by its content-hash key.
    ///
    /// Pure read — does **not** bump `last_used_at`. Callers that want LRU
    /// freshness on a hit collect the hit keys and call [`Self::touch`] once
    /// per batch (cheaper than a write per `get`).
    pub fn derivation_cache_get(&self, cache_key: &str) -> Result<Option<Vec<u8>>> {
        let conn = self.lock_conn();
        let payload: Option<Vec<u8>> = conn
            .query_row(
                "SELECT payload FROM derivation_cache WHERE cache_key = ?",
                params![cache_key],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()
            .map_err(StoreError::from)
            .context("derivation_cache_get")?;
        Ok(payload)
    }

    /// Insert (or overwrite) a cached derivation payload.
    ///
    /// `INSERT OR REPLACE` so a re-computation of the same key (e.g. after a
    /// manual cache clear, or a non-deterministic LLM regenerating) refreshes
    /// `created_at` / `last_used_at` to the new attempt. The key already folds
    /// every version-cascade input (§3.1), so an overwrite is always the same
    /// logical derivation.
    pub fn derivation_cache_put(&self, cache_key: &str, kind: &str, payload: &[u8]) -> Result<()> {
        let now = OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .context("format derivation_cache.created_at")?;
        let conn = self.lock_conn();
        conn.execute(
            "INSERT OR REPLACE INTO derivation_cache
                (cache_key, kind, payload, created_at, last_used_at)
             VALUES (?, ?, ?, ?, ?)",
            params![cache_key, kind, payload, now, now],
        )
        .map_err(StoreError::from)
        .context("derivation_cache_put")?;
        Ok(())
    }

    /// Bump `last_used_at` for the given hit keys (LRU freshness, §3.5).
    ///
    /// Run in a single transaction. Missing keys are a no-op. Called once per
    /// ingest batch with the keys that hit, so the GC pass keeps live chunks.
    pub fn derivation_cache_touch(&self, keys: &[String]) -> Result<()> {
        if keys.is_empty() {
            return Ok(());
        }
        let now = OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .context("format derivation_cache.last_used_at")?;
        let mut conn = self.lock_conn();
        let tx = conn.transaction().map_err(StoreError::from)?;
        {
            let mut stmt = tx
                .prepare("UPDATE derivation_cache SET last_used_at = ? WHERE cache_key = ?")
                .map_err(StoreError::from)?;
            for key in keys {
                stmt.execute(params![now, key])
                    .map_err(StoreError::from)
                    .context("derivation_cache_touch")?;
            }
        }
        tx.commit().map_err(StoreError::from)?;
        Ok(())
    }

    /// Delete cache entries whose `last_used_at` is older than `ttl_days`
    /// (§3.5 lightweight GC). Returns the number of rows removed.
    ///
    /// `ttl_days <= 0` is a no-op guard (never wipe the whole cache by an
    /// accidental zero TTL).
    pub fn derivation_cache_gc(&self, ttl_days: i64) -> Result<usize> {
        if ttl_days <= 0 {
            return Ok(0);
        }
        let cutoff = (OffsetDateTime::now_utc() - time::Duration::days(ttl_days))
            .format(&Rfc3339)
            .context("format derivation_cache gc cutoff")?;
        let conn = self.lock_conn();
        let removed = conn
            .execute(
                "DELETE FROM derivation_cache WHERE last_used_at < ?",
                params![cutoff],
            )
            .map_err(StoreError::from)
            .context("derivation_cache_gc")?;
        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::SqliteStore;

    fn open_store() -> (tempfile::TempDir, SqliteStore) {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = kebab_config::Config::defaults();
        cfg.storage.data_dir = dir.path().to_string_lossy().into_owned();
        let store = SqliteStore::open(&cfg.storage).unwrap();
        store.run_migrations().unwrap();
        (dir, store)
    }

    #[test]
    fn put_then_get_roundtrips() {
        let (_d, store) = open_store();
        store
            .derivation_cache_put("key1", "embedding", &[1, 2, 3, 4])
            .unwrap();
        let got = store.derivation_cache_get("key1").unwrap();
        assert_eq!(got, Some(vec![1, 2, 3, 4]));
    }

    #[test]
    fn get_miss_returns_none() {
        let (_d, store) = open_store();
        assert_eq!(store.derivation_cache_get("absent").unwrap(), None);
    }

    #[test]
    fn put_replaces_existing() {
        let (_d, store) = open_store();
        store.derivation_cache_put("k", "alias", b"old").unwrap();
        store.derivation_cache_put("k", "alias", b"new").unwrap();
        assert_eq!(
            store.derivation_cache_get("k").unwrap(),
            Some(b"new".to_vec())
        );
    }

    #[test]
    fn touch_missing_keys_is_noop() {
        let (_d, store) = open_store();
        store
            .derivation_cache_touch(&["nope".to_string()])
            .unwrap();
        assert_eq!(store.derivation_cache_get("nope").unwrap(), None);
    }

    #[test]
    fn gc_zero_ttl_is_noop() {
        let (_d, store) = open_store();
        store.derivation_cache_put("k", "embedding", b"x").unwrap();
        assert_eq!(store.derivation_cache_gc(0).unwrap(), 0);
        assert!(store.derivation_cache_get("k").unwrap().is_some());
    }

    #[test]
    fn gc_removes_stale_entries() {
        let (_d, store) = open_store();
        store.derivation_cache_put("fresh", "embedding", b"x").unwrap();
        // Backdate one row by 100 days via a direct UPDATE.
        let old = (OffsetDateTime::now_utc() - time::Duration::days(100))
            .format(&Rfc3339)
            .unwrap();
        {
            let conn = store.lock_conn();
            conn.execute(
                "INSERT INTO derivation_cache (cache_key, kind, payload, created_at, last_used_at)
                 VALUES ('stale', 'embedding', ?, ?, ?)",
                params![&b"y"[..], &old, &old],
            )
            .unwrap();
        }
        let removed = store.derivation_cache_gc(30).unwrap();
        assert_eq!(removed, 1);
        assert!(store.derivation_cache_get("stale").unwrap().is_none());
        assert!(store.derivation_cache_get("fresh").unwrap().is_some());
    }
}
