---
phase: P9
component: integrations + new crate (kebab-mcp)
task_id: p9-fb-30
title: "MCP server — agent host 무관 protocol surface"
status: completed
target_version: 0.3.0
depends_on: [p9-fb-27]
unblocks: []
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§7 RAG, §10 UX, externalAI 통합 절]
source_feedback: 사용자 도그푸딩 2026-05-06 — Claude Code 같은 AI agent 가 kebab CLI 를 사용하는 것이 궁극 목표. 현재 surface 는 Claude Code 전용 skill (subprocess wrapper) 만 — host 무관 표준 통신 없음.
---

# p9-fb-30 — MCP server

> ✅ **구현 완료.** 본 spec 은 구현 시점의 frozen 상태. post-merge deviation (특히 `error.v1` 에 schema_version 필드 추가, ask/search spawn_blocking, manual dispatch 채택) 은 [HOTFIXES.md](../HOTFIXES.md) 의 `2026-05-07 — p9-fb-30` 항목 참조 — live source of truth.

## 증상 / 동기

- 현재 외부 AI 통합은 `integrations/claude-code/kebab/` skill 한 종류 — Claude Code subprocess wrapper.
- Cursor / OpenAI Agents / Copilot CLI 등 다른 host 는 별도 wrapper 작성 필요.
- MCP (Model Context Protocol) 가 표준 — 한 번 server 구현하면 MCP-aware host 모두 지원.

## Goal (skeleton)

- `kebab mcp` subcommand 또는 별도 binary `kebab-mcp` — stdio MCP server.
- Tool surface (최소): `search`, `ask`, `fetch`, `ingest_file`, `ingest_stdin`, `stats`, `schema`.
- Resources: 옵션 — chunk / doc 을 MCP resource 로 노출 (host subscribe 가능).
- Prompts: 옵션 — agent 가 재사용 가능한 prompt template (예: "summarize this KB section").
- skill 과 병행 — skill 은 backward compat, 신규는 MCP 권장.

## 후속 작업 — brainstorm 필요 항목

- transport: stdio only (default + sole). fb-29 HTTP daemon 은 deferred — HTTP-SSE 옵션은 browser agent / remote 시나리오 demand 발생 시 fb-29 와 함께 재개.
- tool 이름 / 인자 스키마 — wire schema v1 재사용 가능?
- authentication — local-only 면 무인증, daemon 위면 token.
- 새 crate `kebab-mcp` 위치 / 의존성 boundary (kebab-app facade 만 import).

## Risks / notes

- MCP spec 진화 중 — 버전 lock 명시 필요.
- skill 과 surface 중복 — 사용자 혼란 방지 README 안내.
- fb-29 deferral 결과 — MCP transport 는 stdio 단일. HTTP 변형은 future task.
