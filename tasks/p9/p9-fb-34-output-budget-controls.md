---
phase: P9
component: kebab-cli + kebab-app + wire-schema
task_id: p9-fb-34
title: "Output budget controls (--max-tokens / --snippet-chars / pagination)"
status: completed
target_version: 0.5.0
depends_on: []
unblocks: []
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§4 search, §10 UX, wire-schema search_hit.v1]
source_feedback: 사용자 도그푸딩 2026-05-06 — agent context window 제한적. 검색 결과 양 / snippet 길이 / 페이지네이션 control 필요.
---

# p9-fb-34 — Output budget controls

> ✅ **구현 완료.** 본 spec 은 구현 시점의 frozen 상태. post-merge deviation 은 [HOTFIXES.md](../HOTFIXES.md) 의 `2026-05-09 — p9-fb-34` 항목 참조 — live source of truth.

상세 설계: `docs/superpowers/specs/2026-05-09-p9-fb-34-output-budget-controls-design.md`.
구현 계획: `docs/superpowers/plans/2026-05-09-p9-fb-34-output-budget-controls.md`.

## 증상 / 동기

- agent context window 한정 — 검색 결과 5KB 이하로 받고 싶을 때 control 없음.
- snippet 길이 고정 → narrow context 에서 한 hit 만 받아도 차고 넘침.
- top-5 본 후 추가 5 보고 싶을 때 페이지네이션 없음.

## Goal (skeleton)

- `kebab search --max-tokens N` — 결과 직렬화 size 가 N tokens 안에 들도록 truncate / k 자동 축소.
- `kebab search --snippet-chars N` — 각 hit 의 snippet 최대 chars.
- `kebab search --cursor <opaque>` — 이전 호출의 cursor 로 다음 페이지.
- response 에 `next_cursor` 필드 (남은 hit 있을 때).

## 후속 작업 — brainstorm 필요 항목

- token 카운트 — tiktoken 류 dependency vs 단순 byte/4 근사.
- truncate 우선순위 — snippet 단축 → k 축소 → metadata 제거.
- cursor 의 안정성 — index 변경 후 cursor 유효성.
- `kebab ask` 도 동일 인자 (`--max-tokens` 결과 답변 길이 제한)?

## Risks / notes

- wire schema additive — `next_cursor` 필드 추가 minor.
- agent UX — truncate 발생 시 명시적 hint (`truncated: true`) 필요.
- 기본값 — agent 친화 (작은 budget) vs 사람 친화 (큰 budget) trade-off.
