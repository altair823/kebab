---
phase: P9
component: kebab-rag + kebab-search
task_id: p9-fb-41
title: "Multi-hop reasoning / query decomposition (P+, 큰 작업)"
status: open
target_version: 0.6.0+
depends_on: []
unblocks: []
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§7 RAG]
source_feedback: 사용자 도그푸딩 2026-05-06 — Claude Code 가 kebab CLI 사용 후 "추론 약함" 지적. RAG 가 chunk 독립 처리, multi-hop inference (A→B→C) 못 봄.
---

# p9-fb-41 — Multi-hop reasoning / query decomposition

> ⏳ **백로그 only — 미구현 (P+, 큰 작업).** 본 spec 은 도그푸딩 피드백 skeleton. 구현 착수 전 [superpowers:brainstorming](../../docs/superpowers/) 으로 설계 단계 선행 필요. MVP 범위 / iteration 분할 / decomposition vs graph-retrieval 접근 선택 brainstorm 후 결정. 다른 fb 항목보다 우선순위 낮음.

## 증상 / 동기

- 다단계 추론 질문 ("X 와 Y 의 공통 prerequisite 인 Z 는?") 에서 single-pass retrieval 로는 chunk 간 관계 못 읽음.
- 사용자 질문을 sub-question 으로 분해 + 각각 retrieve + 결과 합성하면 답 가능.

## Goal (skeleton — brainstorm 단계에서 확정)

- query decomposition pipeline — LLM 이 사용자 질문을 sub-question N 개로 분해.
- 각 sub-question 으로 separate retrieval → 결과 합성 → 최종 답변.
- 또는 graph-based retrieval — chunk 간 link (citation, entity, doc 관계) 활용.

## 후속 작업 — brainstorm 필요 항목

- decomposition 의 trigger — 모든 질문에 적용 vs 사용자 명시 / heuristic 탐지.
- LLM 호출 횟수 증가 → latency / cost. 단일 사용자 local 에서 acceptable 한지.
- graph 구조면 SQLite 새 테이블 + parser 가 link 추출 — schema migration 필요.
- evaluation — multi-hop golden set 추가 필요.

## Risks / notes

- 큰 작업 (XL). MVP 범위 / iteration 분할 brainstorm 단계 결정.
- p9-fb-15 (multi-turn) 의 follow-up turn 으로 자연 분해되는 부분 있음 — overlap 검토.
- 효과 측정 어려움 — eval golden set 없으면 체감 평가만 가능.
