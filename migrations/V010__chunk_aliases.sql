-- V010__chunk_aliases.sql — doc-side expansion (Phase 2) 검색용 별칭 채널.
--
-- 설계 spec docs/superpowers/specs/2026-05-30-doc-side-expansion-design.md §4.
-- chunks 에 nullable `aliases` 컬럼 + 별도 FTS5 테이블 chunk_aliases_fts +
-- 별도 sync trigger. 기존 chunks_fts / chunks_ai/ad/au (design §5.5 verbatim,
-- CI test fts_v009_matches_design_section_5_5_verbatim) 는 무수정.
-- aliases 는 additive: 미생성/flag off 이면 NULL → chunk_aliases_fts 빈 채로
-- 시작, 검색 UNION 둘째 절 0행 → 기존 동작과 동일. 자동 backfill 없음.

ALTER TABLE chunks ADD COLUMN aliases TEXT;

CREATE VIRTUAL TABLE chunk_aliases_fts USING fts5(
  chunk_id  UNINDEXED,
  doc_id    UNINDEXED,
  aliases,
  tokenize = 'unicode61'
);

CREATE TRIGGER chunk_aliases_ai AFTER INSERT ON chunks WHEN new.aliases IS NOT NULL BEGIN
  INSERT INTO chunk_aliases_fts(chunk_id, doc_id, aliases)
  VALUES (new.chunk_id, new.doc_id, new.aliases);
END;
CREATE TRIGGER chunk_aliases_ad AFTER DELETE ON chunks BEGIN
  DELETE FROM chunk_aliases_fts WHERE chunk_id = old.chunk_id;
END;
CREATE TRIGGER chunk_aliases_au AFTER UPDATE ON chunks BEGIN
  DELETE FROM chunk_aliases_fts WHERE chunk_id = old.chunk_id;
  INSERT INTO chunk_aliases_fts(chunk_id, doc_id, aliases)
    SELECT new.chunk_id, new.doc_id, new.aliases WHERE new.aliases IS NOT NULL;
END;

-- in-process LRU search cache 무효화 (V009 와 동일 패턴).
UPDATE kv SET value = CAST(CAST(value AS INTEGER) + 1 AS TEXT) WHERE key = 'corpus_revision';
