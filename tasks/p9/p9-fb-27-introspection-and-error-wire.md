---
phase: P9
component: kebab-cli + kebab-app + wire-schema
task_id: p9-fb-27
title: "Introspection (`kebab schema`) + structured error wire"
status: open
target_version: 0.3.0
depends_on: []
unblocks: [p9-fb-30]
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§10 UX, wire-schema 전반]
source_feedback: 사용자 도그푸딩 2026-05-06 — agent 가 kebab 인스턴스의 wire 버전 / 기능 / 모델 / 인덱스 통계 알아야 통합 안전. error 도 stderr text 가 아닌 structured JSON 필요.
---

# p9-fb-27 — Introspection + structured error wire

> ⏳ **백로그 only — 미구현.** 본 spec 은 도그푸딩 피드백 skeleton. 구현 착수 전 [superpowers:brainstorming](../../docs/superpowers/) 으로 설계 단계 선행 필요. capability matrix 정의 / error code enumerate / exit code 매핑 brainstorm 후 확정.

## 증상 / 동기

- agent 가 kebab 의 wire schema 버전 / 기능 플래그 / 모델 정보 / 인덱스 통계 introspect 못 함.
- error 는 stderr text — agent parse 어려움. timeout vs no-results vs config-missing vs not-indexed 구분 불가.

## Goal (skeleton)

- `kebab schema --json` — wire schema 버전 list, capability flags (mcp / daemon / streaming / 등), model versions (parser / chunker / embedding / prompt_template / index), index stats (doc count, chunk count, last ingest).
- `kebab stats --json` 으로 분리 가능 — schema 는 정적, stats 는 동적. brainstorm 단계 결정.
- 모든 명령의 error 출력을 structured JSON (stderr ndjson 또는 stdout 의 error.v1 wire).
- exit code: 0 = OK, 1 = generic, 2 = config, 3 = not-indexed, 4 = timeout, 5 = no-results, …

## 후속 작업 — brainstorm 필요 항목

- error.v1 wire schema — fields (`code`, `message`, `details`, `hint`).
- 기존 명령의 error path 전수 변환 — anyhow chain → error.v1.
- schema vs stats 분리 여부.
- fb-30 MCP `initialize` response 와 capability matrix 공유.

## Risks / notes

- error wire 변경 = breaking — 기존 stderr text 출력은 유지 (둘 다 출력 또는 `--json` 일 때만 wire).
- exit code 안정성 — README 에 표 명시.
- fb-30 / 29 의 prerequisite — agent 가 server 능력 먼저 introspect.
