-- V012__derivation_cache.sql — 내용 해시 기반 파생물 캐시 (Derivation Cache).
--
-- 설계 spec docs/superpowers/specs/2026-05-31-derivation-cache-design.md §3.2.
-- 비용 큰 ingest 파생물(embedding 벡터 / LLM 별칭 / 선택적 한국어 형태소)을
-- 청크 text 의 *내용 해시* 키로 캐싱해, 문서 갱신·재색인 시 변경되지 않은
-- 청크의 재계산을 없앤다. cache_key = blake3(kind ‖ text_blake3 ‖ version_key)[:32]
-- (§3.1) — 위치 기반 chunk_id 와 달리 내용이 같으면 문서·위치 무관 동일 키.
--
-- 순수 가산(additive): 기존 데이터를 무효화하지 않으므로 corpus_revision 을
-- bump 하지 않는다(§3.2). 캐시는 순수 성능 레이어 — 손상/삭제되어도 정확성
-- 영향 없음(miss → 재계산). `kebab reset` 시 같은 sqlite 라 함께 비워진다.

CREATE TABLE derivation_cache (
  cache_key    TEXT PRIMARY KEY,   -- §3.1 blake3 32-hex
  kind         TEXT NOT NULL,      -- 'embedding' | 'alias' | 'korean_tokens'
  payload      BLOB NOT NULL,      -- kind 별 인코딩 (§3.3)
  created_at   TEXT NOT NULL,
  last_used_at TEXT NOT NULL       -- LRU/TTL 정리용 (§3.5)
);

CREATE INDEX idx_dcache_kind      ON derivation_cache(kind);
CREATE INDEX idx_dcache_last_used ON derivation_cache(last_used_at);
