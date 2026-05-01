-- V002__fts.sql — FTS5 virtual table + sync triggers.
--
-- Per design §5.5 (chunks_fts virtual table + chunks_ai/ad/au triggers).
-- The CREATE VIRTUAL TABLE / CREATE TRIGGER block below is reproduced
-- VERBATIM from `docs/superpowers/specs/2026-04-27-kb-final-form-design.md`
-- §5.5 lines 866–885; CI diff-checks this against the design doc.
--
-- Tokenizer choice: `unicode61 remove_diacritics 2` follows the design
-- default for P2-1 (Korean morphological tokenizer is a P+ note).
--
-- Backfill: V001 already shipped the `chunks` table without an FTS
-- shadow; on V002 apply we seed `chunks_fts` from the existing rows so
-- already-ingested workspaces become searchable without re-ingesting.
-- Per design §9 (versioning), V002 is additive: no destructive change
-- to V001 tables.

-- ── §5.5 verbatim block ────────────────────────────────────────────────

CREATE VIRTUAL TABLE chunks_fts USING fts5(
  chunk_id     UNINDEXED,
  doc_id       UNINDEXED,
  heading_path,
  text,
  tokenize = 'unicode61 remove_diacritics 2'
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

-- One-shot backfill for existing chunks. The triggers above only fire
-- on future mutations; V001 may have left `chunks` populated. Refinery
-- runs V002 exactly once via its bookkeeping table, so this INSERT is
-- naturally idempotent across restarts.
INSERT INTO chunks_fts(chunk_id, doc_id, heading_path, text)
  SELECT chunk_id, doc_id, heading_path_json, text FROM chunks;
