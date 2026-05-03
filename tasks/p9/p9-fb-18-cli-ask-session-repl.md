---
phase: P9
component: kebab-cli + kebab-app
task_id: p9-fb-18
title: "CLI ask --session / --repl"
status: in_progress
depends_on: [p9-fb-15, p9-fb-17]
unblocks: []
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§7 RAG, §externalAI]
source_feedback: p9-dogfooding-feedback.md item 14
---

# p9-fb-18 — CLI ask multi-turn

## Goal

CLI 에서도 conversation history 사용. `--session <id>` (영속) + `--repl` (in-memory loop).

## Allowed dependencies

- 기존 kebab-cli deps + p9-fb-17 의 ChatSessionRepo.

## Public surface

CLI:
```
kebab ask "Q" [--session <id>] [--repl]
```

- 둘 다 없음: 단발 (현 동작 유지).
- `--session foo`: SQLite chat_sessions 에 `foo` 가 있으면 history 로 사용 + 새 turn append. 없으면 새 session 생성.
- `--repl`: stdin loop. 각 question 후 답변 출력. `:q` 또는 EOF 종료. `--session` 결합 시 영속, 아니면 in-memory.
- `--repl` 에서 빈 줄 + `:` 명령: `:q` (quit), `:new` (session reset, in-memory), `:save <id>` (현 in-memory → session 저장 + 이후 영속).

`--json` 모드: line-delimited 답변 JSON. 각 줄 `answer.v1` (이미 정의), `conversation_id` + `turn_index` 필드 추가 (p9-fb-15 의 schema bump 와 함께).

## Behavior contract

- session 없이 단발 호출은 wire schema `answer.v1` 의 `conversation_id` 필드 = null. 호환.
- `--repl` 에서 Ctrl-C → graceful exit. session 저장된 상태면 finalized.

## Test plan

| kind | description |
|------|-------------|
| unit | `--session foo` 첫 호출 → 새 session 생성 |
| unit | `--session foo` 두번째 호출 → 이전 turn history 로 prompt 빌드 |
| integration | `--repl` stdin "Q1\nQ2\n:q\n" → 2 답변 + clean exit |

## DoD

- [ ] `cargo test -p kebab-cli` 통과
- [ ] `answer.v1` schema 갱신 (conversation_id / turn_index 추가, optional)
- [ ] README **명령** 표 + **외부 AI 통합** 절 — `--session` 으로 Claude Code skill / MCP 가 multi-turn 가능

## Out of scope

- session list / show / delete CLI 명령 (P+)
- session export (markdown / JSON dump)
