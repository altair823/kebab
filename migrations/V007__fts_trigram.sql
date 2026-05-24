-- V007__fts_trigram.sql — Replace chunks_fts tokenizer: unicode61 → trigram.
--
-- Per design §5.5 (chunks_fts virtual table + chunks_ai/ad/au triggers).
-- The CREATE VIRTUAL TABLE / CREATE TRIGGER block below is reproduced
-- VERBATIM from `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md`
-- §5.5; CI diff-checks this against the design doc (test
-- `fts_v007_matches_design_section_5_5_verbatim` in
-- `crates/kebab-store-sqlite/tests/fts.rs`).
--
-- Tokenizer choice: trigram. Korean is agglutinative — unicode61 tokenizes
-- whole eojeol (조사·어미 attached) so substring matching fails. trigram
-- indexes 3-character grams, enabling Korean partial matches. Trade-offs:
-- DB size grows (~2-10×), English lexical also moves to substring match
-- (recall↑, precision↓), BM25 score distribution shifts. See
-- `tasks/HOTFIXES.md` (2026-05-22) and the v0.17.0 design doc.
--
-- chunks_fts is a shadow of chunks (NOT contentless — V002 DDL has no
-- `content=''`); this migration drops the old shadow, recreates it with
-- the new tokenizer, recreates the sync triggers (verbatim from V002),
-- and backfills from `chunks`. The `chunks` table and embeddings are
-- untouched, so users do NOT need to re-ingest after upgrading to
-- v0.17.0 — the migration is fully automatic.

DROP TRIGGER IF EXISTS chunks_au;
DROP TRIGGER IF EXISTS chunks_ad;
DROP TRIGGER IF EXISTS chunks_ai;
DROP TABLE IF EXISTS chunks_fts;

-- ── §5.5 verbatim block ────────────────────────────────────────────────

CREATE VIRTUAL TABLE chunks_fts USING fts5(
  chunk_id     UNINDEXED,
  doc_id       UNINDEXED,
  heading_path,
  text,
  tokenize = 'trigram'
);

CREATE TRIGGER chunks_ai AFTER INSERT ON chunks BEGIN
  INSERT INTO chunks_fts(chunk_id, doc_id, heading_path, text)
  VALUES (new.chunk_id, new.doc_id, new.heading_path_json, new.text);
END;
CREATE TRIGGER chunks_ad AFTER DELETE ON chunks BEGIN
  DELETE FROM chunks_fts WHERE chunk_id = old.chunk_id;
END;
CREATE TRIGGER chunks_au AFTER UPDATE ON chunks BEGIN
  DELETE FROM chunks_fts WHERE chunk_id = old.chunk_id;
  INSERT INTO chunks_fts(chunk_id, doc_id, heading_path, text)
  VALUES (new.chunk_id, new.doc_id, new.heading_path_json, new.text);
END;

-- ── End §5.5 verbatim block ───────────────────────────────────────────

-- One-shot backfill from existing chunks. Mirrors the V002 backfill
-- pattern — direct INSERT into chunks_fts bypasses chunks_ai trigger
-- (trigger fires on chunks INSERT, not chunks_fts INSERT), so no
-- double-insert. Refinery runs V007 exactly once via its bookkeeping
-- table, so this is naturally idempotent across restarts.
INSERT INTO chunks_fts(chunk_id, doc_id, heading_path, text)
  SELECT chunk_id, doc_id, heading_path_json, text FROM chunks;
