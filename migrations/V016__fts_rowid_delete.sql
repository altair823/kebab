-- V016__fts_rowid_delete.sql — chunks_fts 삭제를 chunk_id 스캔에서 rowid 조회로.
--
-- Per design §5.5 (chunks_fts virtual table + chunks_ai/ad/au triggers).
-- The CREATE VIRTUAL TABLE / CREATE TRIGGER block below is reproduced
-- VERBATIM from `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md`
-- §5.5; CI diff-checks this against the design doc (test
-- `fts_v016_matches_design_section_5_5_verbatim` in
-- `crates/kebab-store-sqlite/tests/fts.rs`). V009 keeps its own copy of the
-- older block for cold-upgrade replay; V016 is now the source of truth.
--
-- 문제: `chunk_id` 는 FTS5 에서 UNINDEXED 라 색인이 없다. 그런데 V002 이래
-- 삭제 트리거가 그 컬럼으로 행을 찾는다 (`DELETE FROM chunks_fts WHERE
-- chunk_id = old.chunk_id`). FTS5 는 이걸 만족할 색인이 없으므로 테이블
-- 전체를 훑는다 — chunk 한 건 삭제가 O(색인 전체) 다. 60만 chunk 기준 스캔
-- 한 번이 0.29초이고 문서 하나가 평균 21 chunk 이라, 문서 하나 삭제에 FTS
-- 스캔만 6초가 든다. 증분 재색인에서 파일이 수정될 때마다 같은 비용을 낸다.
-- issue #229.
--
-- 실측 (실제 KB 사본, 문서 28,427건 / chunk 600,808건, 문서 200건 삭제):
--   현행 (chunk_id 로 DELETE)   1590.1초
--   rowid 정렬 (이 마이그레이션)    0.73초
-- 삭제 후 남은 chunks 행 수와 chunks_fts 행 수가 양쪽 다 595,741 로 같고,
-- '한국'(15,837) / 'kebab'(3) / 'database'(1,067) 질의의 hit 수도 같다.
--
-- 해결: chunks_fts 의 rowid 를 chunks 의 rowid 와 맞추고, 삭제를 rowid 로
-- 한다. FTS5 는 rowid 로 B-tree 조회를 하므로 O(log n) 이 된다. 컬럼 구성과
-- 토크나이저는 그대로라 검색 경로(`bm25`, `snippet(chunks_fts, 3, ...)`,
-- `f.chunk_id` / `f.doc_id` 참조)는 손대지 않는다.
--
-- 왜 external-content 가 아닌가: issue #229 는 `content='chunks'` 를 제안했다.
-- 그 편이 본문 그림자(`chunks_fts_content`, 실측 550 MB)까지 회수하지만,
-- V009 트리거가 색인하는 값이 `tokenized_korean_text || ' ' || text` 라
-- `chunks` 의 어느 컬럼과도 일치하지 않는다. generated column 을 새로 만들고
-- 검색 경로의 컬럼 참조를 rowid join 으로 바꾸는 변경이 딸려온다. 삭제 비용은
-- rowid 정렬만으로 같은 복잡도로 내려가므로, 그림자 회수는 별 건으로 둔다.
--
-- rowid 정렬의 전제: `chunks` 는 `chunk_id TEXT PRIMARY KEY` 라 INTEGER
-- PRIMARY KEY 가 없다. SQLite 의 VACUUM 은 그런 테이블의 rowid 를 다시 매길
-- 수 있고, 그러면 이 정렬이 깨진다. kebab 은 VACUUM 을 실행하지 않으며
-- (코드베이스 전체에 없음), 사용자가 직접 실행했다면
-- `kebab_store_sqlite::rebuild_chunks_fts` 가 복구 경로다. 참고로 issue 가
-- 제안한 external-content 도 같은 전제를 깔고 있어 이 위험은 선택지 간
-- 차이가 아니다.
--
-- 재색인 불필요: `chunks` 와 임베딩은 손대지 않는다. 이 마이그레이션은
-- chunks_fts 를 drop 후 chunks 에서 그대로 다시 채운다 (60만 chunk 기준 32초
-- 실측).
--
-- corpus_revision 을 올리지 않는 이유: 색인 내용과 tokenizer 가 같으므로 bm25
-- 점수도 snippet 도 같고, 어휘 검색의 정렬은 `ORDER BY score, f.chunk_id` 라
-- rowid 와 무관하다. 즉 결과가 바뀌지 않으므로 미결 pagination cursor 를
-- 무효화할 이유가 없다. 실측에서도 '한국' 15,837 / 'kebab' 3 / 'database'
-- 1,067 로 전후 hit 수가 같았다. V009 처럼 tokenizer 가 바뀌는 경우와 다르다.

-- 기존 chunks_fts 제거 (chunk_id 삭제 트리거).
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
  INSERT INTO chunks_fts(rowid, chunk_id, doc_id, heading_path, text)
  VALUES (new.rowid, new.chunk_id, new.doc_id, new.heading_path_json,
          CASE WHEN new.tokenized_korean_text IS NOT NULL
               THEN new.tokenized_korean_text || ' ' || new.text
               ELSE new.text
          END);
END;
CREATE TRIGGER chunks_ad AFTER DELETE ON chunks BEGIN
  DELETE FROM chunks_fts WHERE rowid = old.rowid;
END;
CREATE TRIGGER chunks_au AFTER UPDATE ON chunks BEGIN
  DELETE FROM chunks_fts WHERE rowid = old.rowid;
  INSERT INTO chunks_fts(rowid, chunk_id, doc_id, heading_path, text)
  VALUES (new.rowid, new.chunk_id, new.doc_id, new.heading_path_json,
          CASE WHEN new.tokenized_korean_text IS NOT NULL
               THEN new.tokenized_korean_text || ' ' || new.text
               ELSE new.text
          END);
END;

-- ── End §5.5 verbatim block ───────────────────────────────────────────

-- chunks 에서 그대로 재구축. rowid 를 명시해 정렬을 만든다.
INSERT INTO chunks_fts(rowid, chunk_id, doc_id, heading_path, text)
  SELECT rowid, chunk_id, doc_id, heading_path_json,
         CASE WHEN tokenized_korean_text IS NOT NULL
              THEN tokenized_korean_text || ' ' || text
              ELSE text
         END
  FROM chunks;
