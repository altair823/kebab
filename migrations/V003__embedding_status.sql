-- V003__embedding_status.sql — additive embedding lifecycle markers (§5.6).
--
-- P3-3 introduces a two-phase write to `embedding_records` paired with
-- a Lance MergeInsert. Phase 1 inserts the row at `status='pending'`;
-- phase 2 issues the Lance write; phase 3 flips the row to
-- `status='committed'`. `search` joins back through this table with
-- `WHERE status='committed'` so partial-write Lance rows never surface
-- to callers, and a crashed phase 2 retry simply re-runs against the
-- still-pending row (Lance MergeInsert dedupes on `chunk_id`).
--
-- The third state, `tombstone`, is reserved for the deletion pipeline:
-- when a chunk row goes away, the matching Lance row should also be
-- garbage-collected, but the GC scheduler is out of P3-3 scope. The
-- BEFORE DELETE trigger below stages the marker so a future GC has a
-- well-defined claim; see the comment block on the trigger for why
-- it currently coexists with V001's `ON DELETE CASCADE` FK rather than
-- replacing it.

ALTER TABLE embedding_records ADD COLUMN status TEXT NOT NULL DEFAULT 'pending'
  CHECK (status IN ('pending','committed','tombstone'));

ALTER TABLE embedding_records ADD COLUMN vector_committed INTEGER NOT NULL DEFAULT 0;

CREATE INDEX idx_embed_status ON embedding_records(status);

-- Tombstone trigger.
--
-- Intent: when a `chunks` row is about to be deleted, mark its
-- dependent `embedding_records` rows as `status='tombstone'` so a later
-- GC pass can drop the matching Lance rows in lockstep.
--
-- Caveat (carried into a future migration): V001 declared the FK as
-- `chunk_id REFERENCES chunks(chunk_id) ON DELETE CASCADE`. SQLite's
-- documented order is "BEFORE-DELETE trigger fires first, then CASCADE
-- runs", so this UPDATE will land a `tombstone` value that is
-- immediately followed by the CASCADE removing the row. The trigger is
-- therefore best-effort under the current FK; the only path that
-- actually preserves the tombstone is to drop the CASCADE (table
-- recreation, since SQLite has no DROP CONSTRAINT) — that is queued
-- for a P+ migration once the GC scheduler exists and we have actual
-- production rows to migrate. Keeping the trigger here documents the
-- design intent and gives the deletion-pipeline observer a stable hook
-- to wire into.
CREATE TRIGGER chunks_bd_tombstone_embeddings BEFORE DELETE ON chunks BEGIN
  UPDATE embedding_records SET status='tombstone' WHERE chunk_id = old.chunk_id;
END;
