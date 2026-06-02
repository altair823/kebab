-- V013__drop_chunk_aliases.sql — doc-side expansion(별칭) 채널 제거.
--
-- 근거: docs/superpowers/specs/2026-06-03-remove-doc-expansion-spec.md +
-- tasks/HOTFIXES.md 2026-06-03 entry. V010 이 도입한 chunks.aliases 컬럼 +
-- chunk_aliases_fts FTS5 테이블 + sync trigger 를 forward-only 로 DROP 한다.
-- V010 자체는 과거 마이그레이션 freeze 규칙에 따라 무수정 — 본 마이그레이션이
-- 덮어서 제거한다.
--
-- 별칭은 default-off 였으므로 대부분의 KB 는 빈 데이터 → 손실 없음. 본문/임베딩은
-- 불변이라 corpus_revision cascade 불필요(spec §결정 사항) — in-process LRU 캐시는
-- 프로세스별 휘발성이고 마이그레이션은 query 이전(store open 시)에 돌므로 bump 가
-- 무의미. 따라서 본 마이그레이션은 순수 구조 변경(DROP)만 수행한다.
-- body chunks_fts (chunks_ai/ad/au) 와 그 컬럼은 aliases 를 참조하지 않으므로
-- DROP COLUMN 의 영향 없음. 번들 SQLite 3.45+ 가 ALTER TABLE DROP COLUMN 지원.

-- 1. aliases 를 참조하는 trigger 를 먼저 제거 (DROP COLUMN 전제).
DROP TRIGGER IF EXISTS chunk_aliases_ai;
DROP TRIGGER IF EXISTS chunk_aliases_ad;
DROP TRIGGER IF EXISTS chunk_aliases_au;

-- 2. 별칭 전용 FTS5 테이블 제거 (shadow 테이블 chunk_aliases_fts_* 함께 정리됨).
DROP TABLE IF EXISTS chunk_aliases_fts;

-- 3. 본문 chunks 의 별칭 컬럼 제거.
ALTER TABLE chunks DROP COLUMN aliases;
