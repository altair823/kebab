//! FTS5 maintenance helpers (P2-1).
//!
//! `chunks_fts` is a contentless FTS5 virtual table created by
//! `migrations/V002__fts.sql` and kept in sync with the `chunks` table by
//! the `chunks_ai` / `chunks_ad` / `chunks_au` triggers (design §5.5).
//!
//! Normal operation needs nothing from this module — every mutation on
//! `chunks` propagates automatically inside the host transaction. The
//! only entry point exposed here is [`rebuild_chunks_fts`], used as the
//! escape hatch for `kb index --rebuild-fts` (wired by `kb-cli` later;
//! out of scope for P2-1).

use anyhow::{Context, Result};
use rusqlite::Connection;

/// Wipe `chunks_fts` and repopulate it from `chunks`.
///
/// Useful when:
/// - the FTS index is suspected to have drifted (manual SQL,
///   crash-during-migration on a future schema bump, etc.);
/// - a tokenizer / schema change ships in a later migration and an
///   already-running deployment needs to re-tokenize without re-ingest.
///
/// The two statements run inside a single transaction so a failure
/// between DELETE and INSERT cannot leave `chunks_fts` empty.
///
/// # Concurrency
///
/// Caller is expected to hold the `SqliteStore` mutex (or otherwise own
/// a private `Connection`); two concurrent rebuilds on the same DB file
/// would race the DELETE / INSERT pair. The SAVEPOINT acquires SQLite's
/// reserved-write lock at the DELETE; in WAL mode SQLite serializes
/// writers, so concurrent INSERTs into `chunks` from another connection
/// block until RELEASE — there is no duplicate-FTS-row race. Calling
/// from inside a caller-owned transaction is safe; SAVEPOINT nests
/// correctly. A panic inside the DELETE/INSERT closure leaks the
/// savepoint name on the connection until the connection is dropped;
/// that's acceptable because the next caller's `SAVEPOINT
/// rebuild_chunks_fts` legally shadows the leaked one.
pub fn rebuild_chunks_fts(conn: &Connection) -> Result<()> {
    // SAVEPOINT (instead of `transaction()`) keeps this function callable
    // from inside a caller-owned transaction. `&Connection` does not
    // permit `conn.transaction()` anyway (that needs `&mut Connection`),
    // so SAVEPOINT is the right primitive here.
    conn.execute("SAVEPOINT rebuild_chunks_fts", [])
        .context("open savepoint rebuild_chunks_fts")?;

    let result: Result<()> = (|| {
        conn.execute("DELETE FROM chunks_fts", [])
            .context("DELETE FROM chunks_fts")?;
        conn.execute(
            "INSERT INTO chunks_fts(chunk_id, doc_id, heading_path, text)
             SELECT chunk_id, doc_id, heading_path_json, text FROM chunks",
            [],
        )
        .context("repopulate chunks_fts from chunks")?;
        Ok(())
    })();

    match result {
        Ok(()) => {
            conn.execute("RELEASE rebuild_chunks_fts", [])
                .context("release savepoint rebuild_chunks_fts")?;
            Ok(())
        }
        Err(e) => {
            // Best-effort rollback; bubble the original error.
            let _ = conn.execute("ROLLBACK TO rebuild_chunks_fts", []);
            let _ = conn.execute("RELEASE rebuild_chunks_fts", []);
            Err(e)
        }
    }
}
