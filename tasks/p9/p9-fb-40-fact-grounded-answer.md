---
phase: P9
component: kebab-rag + kebab-llm
task_id: p9-fb-40
title: "Fact-grounded answer 강화 (citation 강제 + 근거 없음 fallback)"
status: open
target_version: 0.5.0
depends_on: []
unblocks: []
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§7 RAG, prompt template]
source_feedback: 사용자 도그푸딩 2026-05-06 — Claude Code 가 kebab CLI 사용 후 "fact extraction 은 RAG 한계" 지적. fact 단위 질문에서 LLM 이 retrieved chunk 외 internal knowledge 로 답하거나 hallucinate.
---

# p9-fb-40 — Fact-grounded answer 강화

> ⏳ **백로그 only — 미구현.** 본 spec 은 도그푸딩 피드백 skeleton. 구현 착수 전 [superpowers:brainstorming](../../docs/superpowers/) 으로 설계 단계 선행 필요. citation 강제 형식 / 검증 layer / "모름" fallback trigger / prompt_template_version cascade 영향 brainstorm 후 확정.

## 증상 / 동기

- "X 의 정확한 값 / 날짜 / 숫자" 류 질문에서 LLM 이 retrieved chunk 의 fact 와 internal knowledge 충돌 시 internal 우세.
- 근거 부족한 질문에도 LLM 이 그럴듯한 답 생성 — hallucinate.
- RAG 본질적 한계지만 prompt / 검증 layer 로 완화 가능.

## Goal (skeleton — brainstorm 단계에서 확정)

- 답변의 모든 fact 가 retrieved chunk 안 span 으로 매핑되도록 강제.
- 근거 부족 시 "모름" 답변 fallback.
- citation 미포함 답변 거부 또는 경고.

## 후속 작업 — brainstorm 필요 항목

- prompt template 수정 — citation 강제 형식 (예: `[doc_id#L]` inline).
- post-generation 검증 — 답변의 fact span 이 retrieved chunk 에 있는지 substring / fuzzy 매치.
- "모름" fallback 의 trigger 조건 (top score gate, chunk count 등).
- prompt_template_version cascade — bump 필요.

## Risks / notes

- 너무 strict 하면 정상 답변도 차단 — 경고만 / 거부의 trade-off.
- post-generation 검증은 latency 증가.
- prompt_template_version bump → eval re-run 필요.
- p9-fb-15 (RAG multi-turn) 와 prompt 변경 영역 겹침 — 같은 batch 가능.
