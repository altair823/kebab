-- V009__fts_korean_morphological.sql — Replace chunks_fts tokenizer: trigram → unicode61.
--
-- Per design §5.5 (chunks_fts virtual table + chunks_ai/ad/au triggers).
-- The CREATE VIRTUAL TABLE / CREATE TRIGGER block below is reproduced
-- VERBATIM from `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md`
-- §5.5; CI diff-checks this against the design doc (test
-- `fts_v009_matches_design_section_5_5_verbatim` in
-- `crates/kebab-store-sqlite/tests/fts.rs`).
--
-- Tokenizer choice: unicode61 + pre-tokenized Korean column.
-- V007 trigram enabled substring matching for Korean ≥3 chars but
-- 2-char Korean queries (e.g. '한국', '서울') always returned 0 hits.
-- V009 adds `tokenized_korean_text TEXT` column to `chunks` — the ingest
-- path (S2+) runs lindera ko-dic morphological analysis and writes the
-- space-separated morpheme sequence to this column. The chunks_ai/chunks_au
-- triggers concatenate tokenized_korean_text with the raw text before
-- indexing into chunks_fts, so both Korean morphemes AND English tokens
-- are searchable via a single FTS query. English substring matching
-- (V007 ad-hoc feature) reverts to whole-token matching (V002 behavior).
-- corpus_revision is bumped so the in-process search cache is automatically
-- invalidated. See tasks/HOTFIXES.md (2026-05-28) for the deviation log.
--
-- chunks_fts is a shadow of chunks (NOT contentless — V002 DDL has no
-- `content=''`); this migration drops the old shadow, recreates it with
-- the new tokenizer, recreates the sync triggers (CASE expression for
-- tokenized_korean_text), and backfills from `chunks`. The `chunks` table
-- and embeddings are untouched, so users do NOT need to re-ingest after
-- upgrading — the migration is fully automatic. tokenized_korean_text
-- starts as NULL for all pre-V009 rows; a subsequent kebab ingest
-- (S2+ path) will fill it in via UPDATE, firing chunks_au to re-index.

-- ── Korean morphological tokenizer (V009) ─────────────────────────────

-- chunks 테이블에 한국어 형태소 분해된 text 를 저장할 열 추가.
ALTER TABLE chunks ADD COLUMN tokenized_korean_text TEXT;

-- 기존 chunks_fts 제거 (trigram tokenizer).
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
  tokenize = 'unicode61'
);

CREATE TRIGGER chunks_ai AFTER INSERT ON chunks BEGIN
  INSERT INTO chunks_fts(chunk_id, doc_id, heading_path, text)
  VALUES (new.chunk_id, new.doc_id, new.heading_path_json,
          CASE WHEN new.tokenized_korean_text IS NOT NULL
               THEN new.tokenized_korean_text || ' ' || new.text
               ELSE new.text
          END);
END;
CREATE TRIGGER chunks_ad AFTER DELETE ON chunks BEGIN
  DELETE FROM chunks_fts WHERE chunk_id = old.chunk_id;
END;
CREATE TRIGGER chunks_au AFTER UPDATE ON chunks BEGIN
  DELETE FROM chunks_fts WHERE chunk_id = old.chunk_id;
  INSERT INTO chunks_fts(chunk_id, doc_id, heading_path, text)
  VALUES (new.chunk_id, new.doc_id, new.heading_path_json,
          CASE WHEN new.tokenized_korean_text IS NOT NULL
               THEN new.tokenized_korean_text || ' ' || new.text
               ELSE new.text
          END);
END;

-- ── End §5.5 verbatim block ───────────────────────────────────────────

-- One-shot backfill from existing chunks. tokenized_korean_text is NULL
-- for all pre-V009 rows so the CASE expression falls to the ELSE branch
-- (raw text only). Subsequent re-ingest via S2+ will UPDATE
-- tokenized_korean_text and fire chunks_au to re-index with morphemes.
INSERT INTO chunks_fts(chunk_id, doc_id, heading_path, text)
  SELECT chunk_id, doc_id, heading_path_json,
         CASE WHEN tokenized_korean_text IS NOT NULL
              THEN tokenized_korean_text || ' ' || text
              ELSE text
         END
  FROM chunks;

-- Bump corpus_revision so the in-process LRU search cache is invalidated.
-- kv table columns are `key` TEXT + `value` TEXT (V004__kv.sql).
-- value is TEXT so CAST is required for integer arithmetic.
UPDATE kv SET value = CAST(CAST(value AS INTEGER) + 1 AS TEXT) WHERE key = 'corpus_revision';
