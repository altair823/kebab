-- V011__drop_embedding_records_fk.sql — embedding_records.chunk_id FK 제거.
-- sentinel chunk_id({orig}#alias, chunks 에 없는 id) 벡터를 허용하기 위함
-- (설계 spec 2026-05-30-dense-alias-vectors-design.md §3.5-1). SQLite 는 ALTER
-- 로 FK 제거 불가 → 테이블 재생성. status/vector_committed(V003) + 인덱스 보존.
-- CASCADE 제거분은 put_chunks/purge 의 명시 DELETE 로 대체(§3.5-2).
PRAGMA foreign_keys=OFF;
-- legacy_alter_table=ON: DROP embedding_records 직후 V003 의
-- chunks_bd_tombstone_embeddings trigger 가 (아직 존재하는 chunks 위에서)
-- 사라진 embedding_records 를 참조하는 dangling 상태가 된다. 이후 RENAME 이
-- 기본(legacy off) 모드면 스키마 전체를 재파싱하며 그 trigger 에서
-- "no such table: embedding_records" 로 실패한다. legacy 모드는 RENAME 시
-- trigger/view 본문 재파싱을 생략하므로 trigger 를 건드리지 않고 통과한다
-- (SQLite ALTER TABLE 문서의 권장 table-redefinition 절차).
PRAGMA legacy_alter_table=ON;

CREATE TABLE embedding_records_new (
  embedding_id   TEXT PRIMARY KEY,
  chunk_id       TEXT NOT NULL,                 -- FK 제거 (was REFERENCES chunks ON DELETE CASCADE)
  model_id       TEXT NOT NULL,
  model_version  TEXT NOT NULL,
  dimensions     INTEGER NOT NULL,
  lance_table    TEXT NOT NULL,
  created_at     TEXT NOT NULL,
  status         TEXT NOT NULL DEFAULT 'pending',
  vector_committed INTEGER NOT NULL DEFAULT 0,
  UNIQUE(chunk_id, model_id, model_version, dimensions)
);
INSERT INTO embedding_records_new
  SELECT embedding_id, chunk_id, model_id, model_version, dimensions,
         lance_table, created_at, status, vector_committed
    FROM embedding_records;
DROP TABLE embedding_records;
ALTER TABLE embedding_records_new RENAME TO embedding_records;
CREATE INDEX idx_embed_chunk  ON embedding_records(chunk_id);
CREATE INDEX idx_embed_model  ON embedding_records(model_id, model_version, dimensions);
CREATE INDEX idx_embed_status ON embedding_records(status);

PRAGMA legacy_alter_table=OFF;
PRAGMA foreign_keys=ON;

UPDATE kv SET value = CAST(CAST(value AS INTEGER) + 1 AS TEXT) WHERE key = 'corpus_revision';
