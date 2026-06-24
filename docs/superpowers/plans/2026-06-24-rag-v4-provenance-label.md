---
title: "rag-v4 — RAG provenance 라벨 (source/trust + 신뢰도 우선 지시)"
created: 2026-06-24
status: implemented
contract_sections: [§9 versioning, §0 Q3 citation]
design_doc_change: none
---

# rag-v4 — RAG provenance 라벨

## 문제

kebab 은 `[[workspace.sources]]`(각 source: id + trust_level)로 출처를 안다.
**필터**(`--source`/`--trust-min`)는 저신뢰 출처를 retrieval 에서 빼는 레버지만,
"둘 다 retrieval 하되 답변에서 권위 출처를 우선"하는 **생성 측** 제어는 없었다.
혼합 KB(wiki 문서 + jira 이슈)의 competing 질의에서, 저신뢰 jira(secondary) 청크가
권위 wiki(primary) 청크를 답변에서 덮어쓰는 generation-side 실패가 남았다.

## 설계 (5계층 + wire)

1. **`SearchHit`**(kebab-core): `source_id: Option<String>` + `trust_level:
   Option<TrustLevel>` additive optional(`skip_serializing_if=None`).
2. **retriever build_hit**(lexical + vector): documents 조인 SELECT 에
   `d.trust_level, d.source_id` 추가, 채움. trust_level 은 lowercase TEXT →
   `serde_json::from_value(Value::String(..))` + `#[serde(rename_all="lowercase")]`
   로 round-trip(저장=parse 일치, doc_summary read-back 과 동형). RAG 파이프라인이
   retriever.search 를 직접 호출하므로 app 레이어 backfill 우회 — retriever 에서
   채워야 양쪽(검색 wire + RAG)에 노출.
3. **hybrid fusion**: 병합 hit 가 `base.clone()` 로 두 필드 전파.
4. **pack_context**: 청크 헤더 `[#n] source={id} trust={word} doc=… …`.
   word = primary/secondary/generated, None=unknown. **버전 무관 항상 렌더**.
5. **`SYSTEM_PROMPT_RAG_V4`**: rag-v3 8규칙 verbatim + 2규칙(신뢰도 우선 discount,
   [#번호] 귀속). config 기본 rag-v3→rag-v4. multi-hop synth 도 2규칙 → 버전
   v1→v2(prompt 변경 = 버전 bump, design §9).
+ **wire**: `search_hit.schema.json` 에 두 필드 optional 추가(v2 bump 아님).
+ **source_id 검증**: RAG 헤더 렌더되므로 `validate_sources` 에 `[A-Za-z0-9._-]`
  char-set 검증(주입 방지).

## opt-out 의미

`prompt_template_version` 은 single-pass system prompt 를 고른다. rag-v3 핀 →
discount/귀속 **지시**가 빠짐(라벨 자체는 무해하게 컨텍스트에 남음). multi-hop 은
prompt_template_version 으로 선택 불가 → 항상 provenance(v2). label 은 항상
렌더되지만 v3 system prompt 가 그걸 쓰라고 안 함 = 사실상 old 동작.

## 검증

단위/통합 green(25 바이너리), clippy 0. 독립 코드 리뷰 APPROVE(7위험 PASS — 특히
trust round-trip 실 DB 확인). 도그푸딩: **라벨 메커니즘 end-to-end 검증**(`search
--json` 이 competing 쿼리에 wiki/primary + jira/secondary 둘 다 정확 라벨로 노출).
**LLM-judge(rag-v3 vs v4 답변 비교)는 instruction LLM 부재로 보류** — .2/.47 ollama
다운 + lemonade `/api/generate` 가 it-model template 미적용(kebab 은 streaming
`/api/generate` 사용). `dogfood_rag_v4.py`(competing 66 subset, jira-override rate)
준비됨 — LLM 복구 시 실행. 상세: HOTFIXES 2026-06-24 RAG provenance 라벨.

## 버전

`prompt_template_version` 변경(rag-v4) + 신규 wire 필드 = pre-1.0 minor + 도그푸딩
트리거. follow-up #1/#2 와 함께 배치 릴리스에서 일괄.
