---
title: 별칭 dense 별도 벡터 — 설계 spec
date: 2026-05-30
status: 설계 확정 (brainstorm + PoC 측정 완료) — plan 대기
phase: Phase 2 (query-paraphrase robustness 처방 — dense 활용)
related:
  - docs/superpowers/specs/2026-05-30-doc-side-expansion-design.md
  - memory: project_paraphrase_robustness, project_ranking_deferred, feedback_search_quality_dogfood
contract_sections:
  - "design §6 (retrieval / vector store + hybrid)"
  - "design §9 (versioning cascade)"
---

# 별칭 dense 별도 벡터

## 0. 한 줄 요약

doc-side expansion 의 별칭(`chunk.aliases`)은 현재 lexical FTS 채널(`chunk_aliases_fts`)에만 색인돼
dense(e5)가 활용하지 못한다. 설명형 패러프레이즈는 dense 의 영역인데(단어 안 겹쳐도 의미 매칭), dense 가
별칭 덕을 못 봐 `recall@50=0` 으로 남았다. **별칭을 별도 dense 벡터로 색인**(sentinel chunk_id, 본문
벡터 불변)해 dense 가 별칭 순수 신호로 설명형을 잡게 한다. **flag off 기본**, variants + 전체 golden 회귀로 측정.

## 1. 진단 (PoC 측정 근거, 2026-05-30)

별칭을 **본문에 concat 해 한 벡터**로 임베딩한 PoC(dogfood topics 7 doc):
- 종합 `fully_consistent 2→6, A_dominant 2→0, B_dominant 4→2, spread@10 0.75→0.25` — **명사형·한국어
  설명형·일부 영어 설명형 회복, 명사형 회귀 0**. dense 가 설명형의 본령임을 실증.
- 남은 미회복: mvcc/raft **영어 설명형**(`how databases serve reads without locking rows`,
  `how nodes agree on a single ordered log`) — vector/hybrid 모두 top-50 밖.
- 질문형 프롬프트 강화(`max_tokens` 384 + "질문 형태 생성") 시도 → 동일 `6/0/2/0.25`, 영어 설명형 미회복.
- **가설**: concat 은 긴 본문 + 짧은 별칭을 한 벡터로 합쳐 **본문 의미가 별칭 신호를 희석**. 한국어
  설명형은 한국어 별칭이 풍부해 회복됐으나, 영어 설명형은 별칭 신호가 약함. → 별칭을 **별도 순수 벡터**로
  색인하면 본문 희석 없이 dense 매칭 가능(미검증 — 본 작업이 검증).

## 2. 설계 결정

| # | 결정 | 선택 | 근거 |
|---|------|------|------|
| D1 | 별칭 dense 색인 방식 | **별도 벡터(sentinel chunk_id)** | concat 은 본문 벡터 변경(전체 corpus 회귀 부담) + 본문 희석. 별도 벡터는 본문 벡터 불변(회귀 안전) + 별칭 순수 신호. lexical `chunk_aliases_fts` 와 대칭. |
| D2 | flag | **`ingest.expansion.embed_aliases` default false** | `expansion.enabled`(별칭 생성)와 별개 축. 독립 on/off 측정([[feedback_search_quality_dogfood]]). |
| D3 | RRF 통합 | VectorRetriever 내부 dedup (2채널 유지) | lexical 의 body+alias merge 와 대칭. `RetrievalDetail`/wire schema `search_hit.v1` 무변경. |

## 3. 아키텍처

### 3.1 데이터 흐름

```
ingest_one_asset (embed + upsert):
  body  : emb.embed(chunk.text)        → VectorRecord{chunk_id: orig}              (변경 없음)
  alias : if embed_aliases && aliases  → emb.embed(aliases)                        [NEW]
          → VectorRecord{chunk_id: "{orig}#alias", text: aliases, doc_id: 동일}
  vec_store.upsert([body, alias])   # LanceDB MergeInsert keyed on chunk_id → 별도 row 공존

검색 (VectorRetriever.search):
  store.search(query_vec) → raw_hits (orig + "{orig}#alias" 섞임)
  각 hit: chunk_id 가 "#alias" 로 끝나면 → 원본 strip
  seen(원본 chunk_id) dedup: 같은 원본이 body+alias 둘 다 → 첫(높은 score) 유지
  hydrate(원본 chunk_id) → SearchHit (원본 chunk_id, body 메타)
  → 단일 vector 결과. HybridRetriever.fuse(lexical, vector) 2채널 그대로.
```

### 3.2 sentinel chunk_id

- `ALIAS_SUFFIX = "#alias"`. ChunkId 는 blake3 hex(32 영숫자)라 `#` 미포함 → 충돌 없음.
- alias VectorRecord: `chunk_id = format!("{orig}{ALIAS_SUFFIX}")`, `embedding_id =
  id_for_embedding(&alias_chunk_id, ...)`, `text = aliases`(별칭 원문), `doc_id`/`heading_path` 동일.
- strip 헬퍼: `fn strip_alias_suffix(id: &str) -> &str { id.strip_suffix(ALIAS_SUFFIX).unwrap_or(id) }`.

### 3.3 컴포넌트

- **ingest (kebab-app/src/lib.rs)**: embed 블록 확장. `embed_aliases` on 이고 별칭 있는 청크는 별칭도
  임베딩 → alias VectorRecord 생성. body VectorRecord 는 그대로(chunk.text). 한 `upsert` 에 body+alias 함께.
- **VectorRetriever.search (kebab-search/src/vector.rs)**: raw_hits 순회 시 chunk_id strip + seen
  dedup. candidate_ids/hydrate 는 strip 된 원본 사용. build_hit 도 원본 chunk_id. overfetch
  multiplier 상향(별칭 벡터로 dedup 후 k 미달 방지 — `VECTOR_OVERFETCH_MULTIPLIER` 2→3).
- **purge**: `purge_vector_orphans_for_workspace_path`(stale_chunk_ids_at 기반) + `sweep_deleted_files`
  가 stale/삭제 chunk_id 의 `{id}#alias` 도 함께 `delete_by_chunk_ids`. (별칭 벡터는 SQLite chunks 에
  없어 stale 목록에 안 잡히므로 명시 추가 — 안 하면 orphan 별칭 벡터 누적.)
- **config**: `IngestExpansionCfg.embed_aliases: bool`(default false) + `KEBAB_INGEST_EXPANSION_EMBED_ALIASES`.

### 3.4 격리 / 회귀 안전

- body 벡터(chunk.text 임베딩) **불변** → 기존 명사형/본문 dense 매칭 회귀 0(concat 과 달리).
- 별칭 벡터는 sentinel row 라 본문 벡터와 독립. flag off 면 별칭 벡터 미생성 → 기존과 동일.

## 4. versioning (design §9)

- 별칭 dense 는 additive(별도 벡터). `try_skip_unchanged` 의 기존 5버전 판단 무변경(별칭 부재가 자동
  재색인 트리거 안 함). 재생성은 `--force-reingest`.
- embed_aliases flag 토글은 임베딩 정책 변경이나 별도 벡터라 body 임베딩 version 불변. wire 무변경(flag off).

## 5. 측정 (§4.6)

- dogfood topics 7 doc, embed_aliases on 재임베딩 → `kebab eval variants`.
- **효과**: 영어 설명형(mvcc/raft) `recall@50` 0→양수 회복되는지(concat 미회복분). 종합 B_dominant↓.
- **회귀**: body 벡터 불변이라 명사형/단일쿼리 회귀 0 기대 — 전체 golden 로 확인.
- concat PoC(6/0/2/0.25) 대비 별도 벡터가 영어 설명형까지 잡으면 추가 개선, 못 잡으면 e5 한계로 기록.

## 6. 범위 밖 (YAGNI)

- dense 모델 교체(e5 유지 — research 권고).
- 별칭별 다중 벡터(별칭 전체를 1벡터로).
- lexical 긴 쿼리 완화(content-OR) — dense 가 설명형 본령이라 폐기(2026-05-30 brainstorm).

## 7. 테스트 (TDD)

- `strip_alias_suffix`: `"abc#alias"`→`"abc"`, `"abc"`→`"abc"`.
- ingest: embed_aliases on + 별칭 청크 → vector store 에 `{orig}#alias` row 존재. off → 없음.
- VectorRetriever dedup: 같은 원본이 body+alias 둘 다 hit → 결과에 1개(원본 chunk_id), 높은 score 유지.
- VectorRetriever strip: alias-only hit → 원본 chunk_id 로 hydrate(원본 chunk 메타).
- purge: 청크 재처리 시 `{orig}#alias` 벡터도 삭제(orphan 잔존 0).
- 회귀: embed_aliases off → vector 결과가 기존과 동일.

## 8. PR / 문서

- doc-side expansion 과 같은 PR. README Configuration 에 `embed_aliases`(off 기본) 명시.
  ARCHITECTURE 에 별칭 dense 별도 벡터(sentinel) 1~2줄. HOTFIXES dated entry(lexical 별칭 + dense 별칭 측정 표).
- versioning cascade 없음(body 임베딩 불변). flag off 라 wire 무변경.
