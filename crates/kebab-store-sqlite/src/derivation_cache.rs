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
        // `prepare_cached`, not `query_row`: the latter prepares (and so
        // re-parses the SQL) on every call. Issue #231.
        let payload: Option<Vec<u8>> = conn
            .prepare_cached("SELECT payload FROM derivation_cache WHERE cache_key = ?")
            .map_err(StoreError::from)?
            .query_row(params![cache_key], |row| row.get::<_, Vec<u8>>(0))
            .optional()
            .map_err(StoreError::from)
            .context("derivation_cache_get")?;
        Ok(payload)
    }

    /// Look up many keys at once, returning only the ones present.
    ///
    /// One statement per `SQLITE_MAX_VARIABLE_NUMBER`-sized batch instead
    /// of one round trip per key (issue #231). The per-key form made a
    /// cache *hit* cost as many `lock_conn()` acquisitions as a miss —
    /// and a miss at least does its expensive half (the embedding) on the
    /// GPU, batched, outside the lock. Duplicate keys in `keys` are fine;
    /// the result is keyed by cache_key so they collapse.
    pub fn derivation_cache_get_many(
        &self,
        keys: &[String],
    ) -> Result<std::collections::HashMap<String, Vec<u8>>> {
        let mut found = std::collections::HashMap::with_capacity(keys.len());
        if keys.is_empty() {
            return Ok(found);
        }
        let conn = self.lock_conn();
        // Conservative against the 999-parameter default: the bundled
        // build allows far more, but the cost of a few extra statements
        // is nothing next to the per-key round trips this replaces.
        for batch in keys.chunks(900) {
            let placeholders = std::iter::repeat_n("?", batch.len())
                .collect::<Vec<_>>()
                .join(",");
            let mut stmt = conn
                .prepare_cached(&format!(
                    "SELECT cache_key, payload FROM derivation_cache WHERE cache_key IN ({placeholders})"
                ))
                .map_err(StoreError::from)
                .context("derivation_cache_get_many: prepare")?;
            let rows = stmt
                .query_map(rusqlite::params_from_iter(batch.iter()), |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
                })
                .map_err(StoreError::from)
                .context("derivation_cache_get_many: query")?;
            for row in rows {
                let (k, v) = row.map_err(StoreError::from)?;
                found.insert(k, v);
            }
        }
        Ok(found)
    }

    /// Insert many payloads in one transaction.
    ///
    /// The single-key [`Self::derivation_cache_put`] runs outside any
    /// explicit transaction, so each call is its own implicit commit — a
    /// WAL frame write and wal-index update per embedded chunk (issue
    /// #231). A batch of misses is one logical unit of work and commits
    /// as one.
    pub fn derivation_cache_put_many(&self, entries: &[(String, String, Vec<u8>)]) -> Result<()> {
        if entries.is_empty() {
            return Ok(());
        }
        let now = OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .context("format derivation_cache.created_at")?;
        let mut conn = self.lock_conn();
        let tx = conn.transaction().map_err(StoreError::from)?;
        {
            let mut stmt = tx
                .prepare_cached(
                    "INSERT OR REPLACE INTO derivation_cache
                        (cache_key, kind, payload, created_at, last_used_at)
                     VALUES (?, ?, ?, ?, ?)",
                )
                .map_err(StoreError::from)?;
            for (key, kind, payload) in entries {
                stmt.execute(params![key, kind, payload, now, now])
                    .map_err(StoreError::from)
                    .context("derivation_cache_put_many")?;
            }
        }
        tx.commit().map_err(StoreError::from)?;
        Ok(())
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
        conn.prepare_cached(
            "INSERT OR REPLACE INTO derivation_cache
                (cache_key, kind, payload, created_at, last_used_at)
             VALUES (?, ?, ?, ?, ?)",
        )
        .map_err(StoreError::from)?
        .execute(params![cache_key, kind, payload, now, now])
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
                .prepare_cached("UPDATE derivation_cache SET last_used_at = ? WHERE cache_key = ?")
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
    fn get_many_returns_only_present_keys() {
        let (_d, store) = open_store();
        store.derivation_cache_put("a", "embedding", b"A").unwrap();
        store.derivation_cache_put("c", "embedding", b"C").unwrap();

        let found = store
            .derivation_cache_get_many(&[
                "a".to_string(),
                "b".to_string(),
                "c".to_string(),
                // A repeat: the embed path derives keys from chunk text,
                // and a document with two identical chunks produces the
                // same key twice. The map must collapse it, not trip.
                "a".to_string(),
            ])
            .unwrap();
        assert_eq!(found.len(), 2, "absent keys are simply not in the map");
        assert_eq!(found.get("a").map(Vec::as_slice), Some(b"A".as_slice()));
        assert_eq!(found.get("c").map(Vec::as_slice), Some(b"C".as_slice()));
        assert!(!found.contains_key("b"));
    }

    #[test]
    fn get_many_is_empty_for_no_keys() {
        let (_d, store) = open_store();
        assert!(store.derivation_cache_get_many(&[]).unwrap().is_empty());
    }

    /// The batch splits at 900 parameters. A run that embeds more chunks
    /// than that in one call must still see every one of them — an
    /// off-by-one in the chunking would silently turn hits into misses,
    /// which costs nothing but correctness-invisible re-embedding.
    #[test]
    fn get_many_spans_more_keys_than_one_statement_holds() {
        let (_d, store) = open_store();
        let keys: Vec<String> = (0..2_000).map(|i| format!("k{i:05}")).collect();
        for k in &keys {
            store
                .derivation_cache_put(k, "embedding", k.as_bytes())
                .unwrap();
        }
        let found = store.derivation_cache_get_many(&keys).unwrap();
        assert_eq!(found.len(), keys.len());
        for k in &keys {
            assert_eq!(
                found.get(k).map(Vec::as_slice),
                Some(k.as_bytes()),
                "key {k} must survive the batch split"
            );
        }
    }

    #[test]
    fn put_many_writes_all_and_replaces() {
        let (_d, store) = open_store();
        store
            .derivation_cache_put("dup", "embedding", b"old")
            .unwrap();
        store
            .derivation_cache_put_many(&[
                ("x".to_string(), "embedding".to_string(), b"X".to_vec()),
                ("y".to_string(), "embedding".to_string(), b"Y".to_vec()),
                ("dup".to_string(), "embedding".to_string(), b"new".to_vec()),
            ])
            .unwrap();
        let found = store
            .derivation_cache_get_many(&["x".to_string(), "y".to_string(), "dup".to_string()])
            .unwrap();
        assert_eq!(found.get("x").map(Vec::as_slice), Some(b"X".as_slice()));
        assert_eq!(found.get("y").map(Vec::as_slice), Some(b"Y".as_slice()));
        assert_eq!(
            found.get("dup").map(Vec::as_slice),
            Some(b"new".as_slice()),
            "the batch form keeps INSERT OR REPLACE semantics"
        );
    }

    #[test]
    fn put_many_with_no_entries_is_noop() {
        let (_d, store) = open_store();
        store.derivation_cache_put_many(&[]).unwrap();
        assert!(store.derivation_cache_get_many(&[]).unwrap().is_empty());
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
