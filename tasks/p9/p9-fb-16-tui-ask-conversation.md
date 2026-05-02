---
phase: P9
component: kebab-tui (ask pane)
task_id: p9-fb-16
title: "TUI ask conversation transcript view"
status: planned
depends_on: [p9-fb-15, p9-fb-12]
unblocks: []
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§10 UX]
source_feedback: p9-dogfooding-feedback.md item 13
---

# p9-fb-16 — Ask conversation UI

## Goal

ask pane 을 단발 Q/A 에서 conversation transcript 로 전환. 이전 turn 들 scrollback 가능.

## Allowed dependencies

- 기존 kebab-tui deps.

## Public surface

`kebab-tui::ask::AskState` 가 기존 `latest_question / latest_answer` 대신 `Vec<Turn>` 보유. p9-fb-15 의 `Turn` 재사용.

## Behavior contract

layout:
- 위 (대부분 영역): conversation transcript. 각 turn 은 `Q: ... \n A: ...\n  ▸ 근거 N 건` 블록.
- 아래 1~3 줄: input box + status.
- 좌측 / 우측 padding 으로 readability.

키 (NORMAL, p9-fb-12 따름):
- `j/k`, `PageDown/Up`, `Ctrl-d/u` → transcript scroll.
- `G` → 끝, `gg` → 처음.
- `c` → 현재 focus turn 의 citation fold/unfold.
- `Ctrl-L` 또는 `:new` → history clear (현 session reset, 영속은 p9-fb-17).
- `i` → INSERT (input box focus), `Esc` → NORMAL.
- `Enter` (INSERT) → submit. spawn worker (기존 P9-3 pattern).

streaming:
- 새 token 도착 시 마지막 Turn 의 answer 에 append. transcript 가장 아래까지 자동 scroll (사용자가 위로 scroll 한 상태면 자동 scroll 안 함, "↓ N new tokens" 표시).

## Test plan

| kind | description |
|------|-------------|
| unit | Turn push 후 layout 변경 |
| unit | Ctrl-L → history empty |
| unit | streaming token append → 마지막 Turn.answer 누적 |
| integration | 가짜 RagEvent stream 으로 2 turn 시퀀스 snapshot |

## DoD

- [ ] `cargo test -p kebab-tui` 통과
- [ ] README — ask pane 의 conversation 동작 + 키 안내 추가
- [ ] HOTFIXES P9-3 ask pane 의 단발 동작 → 갱신

## Out of scope

- 영속화 (p9-fb-17)
- CLI 의 multi-turn (p9-fb-18)
- citation fold/unfold 의 jump 키 (p9-fb-20)
