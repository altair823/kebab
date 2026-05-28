//! p9-fb-37: extended stats helpers — per-media / per-lang doc counts,
//! stale doc count, on-disk index byte sums.

use std::collections::BTreeMap;
use std::path::Path;

use kebab_core::{IndexBytes, MEDIA_KINDS};
use rusqlite::Connection;

/// p9-fb-37: result of [`breakdowns`] — three independent counts collected in one pass.
#[derive(Debug, Clone, Default)]
pub struct Breakdowns {
    pub media: BTreeMap<String, u64>,
    pub lang: BTreeMap<String, u64>,
    pub stale_doc_count: u64,
}

/// `media` always contains all 5 `MEDIA_KINDS` (zero-padded).
/// `lang` only contains observed languages; NULL lang is
/// keyed as the literal string `"null"`. `stale_doc_count` is 0 when
/// `threshold_days == 0` (mirrors fb-32 staleness disable semantics).
pub fn breakdowns(conn: &Connection, threshold_days: u64) -> rusqlite::Result<Breakdowns> {
    // media: dual JSON shape — text variant ("markdown") vs object
    // variant ({"image":{"format":"png"}}). Same CASE WHEN as fb-36.
    let mut media: BTreeMap<String, u64> = MEDIA_KINDS
        .iter()
        .map(|k| ((*k).to_string(), 0u64))
        .collect();
    let mut stmt = conn.prepare(
        "SELECT \
           CASE \
             WHEN json_type(a.media_type) = 'text' \
               THEN json_extract(a.media_type, '$') \
             ELSE (SELECT key FROM json_each(a.media_type) LIMIT 1) \
           END AS kind, \
           COUNT(DISTINCT d.doc_id) \
         FROM documents d JOIN assets a ON a.asset_id = d.asset_id \
         GROUP BY kind",
    )?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?)))?;
    for row in rows {
        let (kind, n) = row?;
        media.insert(kind, n);
    }

    let mut lang: BTreeMap<String, u64> = BTreeMap::new();
    let mut stmt = conn.prepare(
        "SELECT COALESCE(lang, 'null') AS l, COUNT(*) \
         FROM documents GROUP BY l",
    )?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?)))?;
    for row in rows {
        let (l, n) = row?;
        lang.insert(l, n);
    }

    let stale_doc_count: u64 = if threshold_days == 0 {
        0
    } else {
        let secs = (threshold_days as i64) * 86_400;
        let cutoff = time::OffsetDateTime::now_utc() - time::Duration::seconds(secs);
        let cutoff_str = cutoff
            .format(&time::format_description::well_known::Rfc3339)
            .expect("RFC3339 format");
        conn.query_row(
            "SELECT COUNT(*) FROM documents WHERE updated_at < ?",
            [cutoff_str],
            |r| r.get(0),
        )?
    };

    Ok(Breakdowns {
        media,
        lang,
        stale_doc_count,
    })
}

/// Sum on-disk bytes of the SQLite database (main + WAL + SHM) and
/// the LanceDB directory tree. Missing files / dir = 0.
pub fn index_bytes(data_dir: &Path) -> std::io::Result<IndexBytes> {
    fn file_size_or_zero(p: &Path) -> u64 {
        std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
    }
    fn dir_walk_sum(p: &Path) -> std::io::Result<u64> {
        if !p.exists() {
            return Ok(0);
        }
        let mut total = 0u64;
        for entry in std::fs::read_dir(p)? {
            let entry = entry?;
            let ty = entry.file_type()?;
            if ty.is_dir() {
                total += dir_walk_sum(&entry.path())?;
            } else if ty.is_file() {
                total += entry.metadata()?.len();
            }
        }
        Ok(total)
    }

    let sqlite_main = data_dir.join("kebab.sqlite");
    let sqlite_wal = data_dir.join("kebab.sqlite-wal");
    let sqlite_shm = data_dir.join("kebab.sqlite-shm");
    let sqlite = file_size_or_zero(&sqlite_main)
        + file_size_or_zero(&sqlite_wal)
        + file_size_or_zero(&sqlite_shm);
    let lancedb = dir_walk_sum(&data_dir.join("lancedb"))?;
    Ok(IndexBytes { sqlite, lancedb })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_fresh() -> (tempfile::TempDir, crate::SqliteStore) {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = kebab_config::Config::defaults();
        cfg.storage.data_dir = dir.path().to_string_lossy().into_owned();
        let store = crate::SqliteStore::open(&cfg).unwrap();
        store.run_migrations().unwrap();
        (dir, store)
    }

    #[test]
    fn breakdowns_empty_corpus() {
        let (_dir, store) = open_fresh();
        let conn = store.read_conn();
        let b = breakdowns(&conn, 0).unwrap();
        // 5 keys all zero, lang map empty, stale 0.
        assert_eq!(b.media.len(), 5);
        for k in MEDIA_KINDS {
            assert_eq!(b.media.get(*k), Some(&0u64));
        }
        assert!(b.lang.is_empty());
        assert_eq!(b.stale_doc_count, 0);
    }

    #[test]
    fn index_bytes_includes_sqlite_main() {
        let (dir, _store) = open_fresh();
        let b = index_bytes(dir.path()).unwrap();
        assert!(
            b.sqlite > 0,
            "main sqlite file should exist after migrations"
        );
        assert_eq!(b.lancedb, 0);
    }

    #[test]
    fn index_bytes_lancedb_dir_walk() {
        let dir = tempfile::tempdir().unwrap();
        let lance = dir.path().join("lancedb");
        std::fs::create_dir_all(lance.join("vectors.lance")).unwrap();
        std::fs::write(
            lance.join("vectors.lance").join("data.bin"),
            vec![0u8; 1024],
        )
        .unwrap();
        let b = index_bytes(dir.path()).unwrap();
        assert_eq!(b.lancedb, 1024);
    }
}
