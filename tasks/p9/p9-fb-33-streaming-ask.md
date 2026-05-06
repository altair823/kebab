---
phase: P9
component: kebab-cli + kebab-app + wire-schema
task_id: p9-fb-33
title: "Streaming ask (ndjson delta) — agent token 즉시 소비"
status: open
target_version: 0.4.0
depends_on: []
unblocks: []
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§7 RAG, §10 UX, wire-schema answer.v1]
source_feedback: 사용자 도그푸딩 2026-05-06 — agent 가 token 도착 즉시 다음 행동 결정 가능해야 — final-only JSON 은 latency 손해.
---

# p9-fb-33 — Streaming ask (ndjson delta)

> ⏳ **백로그 only — 미구현.** 본 spec 은 도그푸딩 피드백 skeleton. 구현 착수 전 [superpowers:brainstorming](../../docs/superpowers/) 으로 설계 단계 선행 필요. delta event 형식 / final-only fallback / TUI vs CLI 차이 brainstorm 후 확정.

## 증상 / 동기

- 현재 `kebab ask --json` 추정 — final answer 한 번에 출력. agent 는 LLM token 도착마다 progressive UI / 조기 종료 / 후속 tool 호출 결정 가능해야 빠름.
- TUI 는 이미 streaming 표시 — CLI / agent 가 동일 surface 못 받음.

## Goal (skeleton)

- `kebab ask --json --stream` — ndjson delta event.
- event shape (제안):
  - `{"kind":"retrieval_done","hits":[...]}`
  - `{"kind":"token","delta":"...", "turn_index":0}`
  - `{"kind":"citation","ref":"[1]","chunk_id":"..."}`
  - `{"kind":"final","answer":"...","citations":[...]}`
- `--stream` 미지정이면 현재 동작 유지 (final-only).
- wire schema `answer_event.v1` 추가.

## 후속 작업 — brainstorm 필요 항목

- event 종류 / 순서 invariant.
- token delta 의 partial markdown — fb-40 fact-grounded / fb-11 markdown render 와 정합성.
- 중간 cancel — agent 가 SIGINT / connection close 하면 LLM 호출 중단.
- daemon (fb-29) HTTP SSE 와 동일 event shape — 이중 구현 방지.

## Risks / notes

- wire schema additive minor — 기존 final-only path 보존.
- TUI 의 streaming 코드 재사용 가능 — kebab-rag 의 generate stream API 가 이미 있을 것.
- fb-30 MCP / fb-29 daemon 과 stream surface 통일 필요.
