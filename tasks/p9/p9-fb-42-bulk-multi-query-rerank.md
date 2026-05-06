---
phase: P9
component: kebab-cli + kebab-search
task_id: p9-fb-42
title: "Bulk multi-query + re-rank hint — agent loop 효율"
status: open
target_version: 0.6.0+
depends_on: []
unblocks: []
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§4 search]
source_feedback: 사용자 도그푸딩 2026-05-06 — agent 가 N 개 query 동시 검색 / 결과 set 을 다른 관점으로 재정렬 원함. 현재는 N 회 subprocess 호출 + 단일 정렬 기준.
---

# p9-fb-42 — Bulk multi-query + re-rank hint

> ⏳ **백로그 only — 미구현 (Nice-to-have).** 본 spec 은 도그푸딩 피드백 skeleton. 구현 착수 전 [superpowers:brainstorming](../../docs/superpowers/) 으로 설계 단계 선행 필요. multi-query input 형식 / 결과 합성 정책 / re-rank hint 의 LLM 호출 비용 brainstorm 후 확정.

## 증상 / 동기

- agent 가 query decomposition (fb-41) 후 N 개 sub-query 검색 — N 회 subprocess fork.
- 검색 결과 set 보고 "이 중 X 관점으로 다시 정렬" 요청 — 현재는 client 측에서 재호출.

## Goal (skeleton)

- `kebab search --queries '[{"q":"a","k":5},{"q":"b","k":5}]'` — bulk JSON input. response 는 query 별 결과 array.
- `kebab search Q --rerank-hint "focus on X"` — top-N retrieve 후 LLM 재정렬 (cross-encoder 가능 시 selection).

## 후속 작업 — brainstorm 필요 항목

- bulk input 형식 — JSON array / ndjson stdin.
- 결과 stream vs final — 큰 multi-query 면 stream 유리.
- re-rank hint 의 LLM 모델 — kebab-llm 의 default 사용.
- fb-39 (precision tuning) 의 cross-encoder 와 re-rank hint 통합 가능.
- fb-29 daemon 위에서 더 의미 — subprocess overhead 이미 daemon 으로 해소되면 우선순위 낮음.

## Risks / notes

- Nice-to-have — fb-30 / 29 / 31 / 34 / 35 보다 우선순위 낮음.
- re-rank hint 는 LLM 호출 추가 — latency / cost.
- fb-39, fb-41 와 영역 겹침.
