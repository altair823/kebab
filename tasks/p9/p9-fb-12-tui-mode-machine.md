---
phase: P9
component: kebab-tui
task_id: p9-fb-12
title: "TUI mode state machine (NORMAL / INSERT)"
status: planned
depends_on: []
unblocks: [p9-fb-10, p9-fb-13]
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§10 UX]
source_feedback: p9-dogfooding-feedback.md item 10
---

# p9-fb-12 — Mode machine

## Goal

TUI 전체에 vim 식 NORMAL / INSERT 모드 도입. 입력 모호성 (e/j/k 가 typing vs command) 제거.

## Allowed dependencies

- 기존 kebab-tui deps.

## Public surface

```rust
pub enum Mode { Normal, Insert }
pub(crate) struct App {
    mode: Mode,
    // ... 기존
}
```

key dispatch 가 mode 따라 분기.

## Behavior contract

기본 진입 모드: NORMAL (Library 가 starting pane). Search / Ask 는 query 칠 일이 잦으므로 pane 전환 시 자동 INSERT 진입 (configurable, 우선 자동).

NORMAL 모드 키 (전역):
- `i` → INSERT
- `:` → command line (`:q` quit, `:cite`, `:new` 등)
- `j/k`, `g/G`, `Ctrl-d/u`, `PageDown/Up` → scroll
- `Tab/Shift-Tab` → pane 이동
- `?` → cheatsheet popup (p9-fb-13)
- pane 별 키 (e=explain, c=cite toggle, r=refresh, …)

INSERT 모드 키:
- 모든 `Char` → input buffer push
- `Esc` → NORMAL
- `Enter` → submit (search / ask trigger)
- `Backspace`, 화살표 키 → buffer 편집 + cursor 이동
- 기타 navigation 키 (j/k 등) 는 typing 으로만

status bar 표시: `-- INSERT --` / `-- NORMAL --` (color: INSERT=green, NORMAL=blue).
focus 표시: 활성 pane 의 테두리 색 강조.

기존 P9-3 ask 의 e/j/k input-empty heuristic 제거 — mode 로 명확히.

## Test plan

| kind | description |
|------|-------------|
| unit | NORMAL 에서 `j` → scroll, INSERT 에서 `j` → buffer 'j' |
| unit | `i` → INSERT, `Esc` → NORMAL |
| unit | Search pane 전환 시 자동 INSERT |
| integration | mode 전환 + key sequence snapshot |

## DoD

- [ ] `cargo test -p kebab-tui` 통과
- [ ] 기존 input-empty heuristic 제거 (HOTFIXES P9-3 e/j/k 갱신)
- [ ] README + cheatsheet (p9-fb-13) 갱신

## Out of scope

- VISUAL 모드 (P+)
- mode 별 cursor shape (`Block` vs `Bar`) — 터미널마다 다름, 우선 skip
