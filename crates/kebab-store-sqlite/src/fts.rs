//! FTS5 maintenance helpers (P2-1).
//!
//! `chunks_fts` is an FTS5 virtual table (a shadow of `chunks`, not
//! contentless — no `content=''` in the DDL) created by
//! `migrations/V002__fts.sql`, retokenized by V009, and repointed to
//! rowid-addressed deletes by V016. It is kept in sync with the `chunks`
//! table by the `chunks_ai` / `chunks_ad` / `chunks_au` triggers (design
//! §5.5). Its rowid mirrors `chunks.rowid`; anything that writes rows here
//! must preserve that or the delete trigger stops finding them.
//!
//! Normal operation needs nothing from this module — every mutation on
//! `chunks` propagates automatically inside the host transaction. The
//! only entry point exposed here is [`rebuild_chunks_fts`], the escape
//! hatch for a shadow that has drifted from `chunks`. It is a library
//! API with no CLI wiring — `kebab doctor`'s `fts_shadow` check detects
//! drift, but recovery through the CLI is `kebab reset` plus a
//! re-ingest.

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
        // Mirrors the chunks_ai trigger exactly (V016 §5.5): rowid is
        // written explicitly so the shadow stays aligned with `chunks`
        // — the delete trigger addresses rows by rowid, so a rebuild
        // that let FTS5 assign its own would silently make every later
        // delete a no-op. The CASE is the same one the trigger applies;
        // without it a rebuild would strip the Korean morphemes that
        // V009 indexes and 2-character Korean queries would stop
        // matching until the next re-ingest.
        conn.execute(
            "INSERT INTO chunks_fts(rowid, chunk_id, doc_id, heading_path, text)
             SELECT rowid, chunk_id, doc_id, heading_path_json,
                    CASE WHEN tokenized_korean_text IS NOT NULL
                         THEN tokenized_korean_text || ' ' || text
                         ELSE text
                    END
             FROM chunks",
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
