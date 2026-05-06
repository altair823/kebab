---
phase: P9
component: kebab-cli + new crate (kebab-server)
task_id: p9-fb-29
title: "HTTP daemon (`kebab serve`) — subprocess overhead 제거"
status: open
target_version: 0.3.0
depends_on: []
unblocks: [p9-fb-30]
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§7 RAG, §10 UX]
source_feedback: 사용자 도그푸딩 2026-05-06 — agent loop 가 kebab CLI 를 반복 호출 시 subprocess fork + Lance/SQLite cold start 비용 누적. local HTTP daemon 이 latency 해결.
---

# p9-fb-29 — HTTP daemon (`kebab serve`)

> ⏳ **백로그 only — 미구현.** 본 spec 은 도그푸딩 피드백 skeleton. 구현 착수 전 [superpowers:brainstorming](../../docs/superpowers/) 으로 설계 단계 선행 필요. bind / auth / endpoint scheme / lifecycle (auto-start vs explicit) brainstorm 후 확정.

## 증상 / 동기

- 현재 `kebab search` / `kebab ask` 가 매 호출 process fork — Lance / SQLite / fastembed 모델 로드 cold start.
- agent 가 10 회 search 도는 loop 면 cold start × 10. local-first 단일 사용자라도 latency 누적.
- daemon 으로 띄우면 hot — sub-100ms search 가능.

## Goal (skeleton)

- `kebab serve --port <N> --bind 127.0.0.1` — local HTTP API.
- endpoint: `/search`, `/ask`, `/fetch`, `/ingest`, `/stats`, `/schema`. wire schema v1 재사용.
- auth: local bind 면 무인증 (외부 host 면 token).
- streaming `/ask` (Server-Sent Events 또는 chunked).
- lifecycle: 사용자 명시 실행 vs CLI 자동 spawn (XDG runtime path 의 socket).

## 후속 작업 — brainstorm 필요 항목

- web framework: axum / hyper / actix — workspace 통일성 + binary size.
- 단일 인스턴스 보장 (PID file / socket lock).
- daemon ↔ CLI shim — CLI 가 daemon 살아있으면 HTTP 사용, 없으면 fork.
- TUI 와 daemon 공존 — TUI 도 daemon 있으면 HTTP 통해 (현재는 in-process).
- fb-30 (MCP) 와 transport 공유 — MCP-over-HTTP-SSE.

## Risks / notes

- 단일 사용자 local 환경 — daemon 없는 단순함 trade-off.
- fb-30 와 강결합 — 함께 brainstorm 하면 architecture 일관.
- security: bind 127.0.0.1 default 강제 — 0.0.0.0 은 명시 opt-in.
