---
phase: P9
component: kebab-tui
task_id: p9-fb-09
title: "External editor return — terminal restore + force redraw"
status: in_progress
depends_on: []
unblocks: []
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§10 UX]
source_feedback: p9-dogfooding-feedback.md item 7
---

# p9-fb-09 — Editor return restore

## Goal

`g` (search), `o` (citation jump 후속) 으로 vim/code 띄우고 종료 후 TUI 화면이 깨지는 버그 수정.

## Allowed dependencies

- 기존 kebab-tui deps.

## Public surface

`kebab-tui::run` 내부에 `with_external_program(|term| -> Result<()> { ... })` helper. spawn 직전 / 종료 후 terminal 상태 toggle.

## Behavior contract

spawn 직전:
1. `terminal.show_cursor()`
2. `terminal.backend_mut().execute(LeaveAlternateScreen)?`
3. `disable_raw_mode()?`

child wait 후:
1. `enable_raw_mode()?`
2. `EnterAlternateScreen`
3. `terminal.clear()?` — 강제 redraw
4. main loop 의 `force_redraw_next_frame: bool` flag set → 다음 draw 가 dirty rect 무시 전체 그림.

## Test plan

수동 테스트 (단위 테스트 어려움 — terminal io 의존). 하지만 helper 의 sequence 자체는 mock backend 로 검증 가능.

| kind | description |
|------|-------------|
| unit | mock backend 의 호출 sequence 검증 (Leave → spawn → Enter → Clear) |

## DoD

- [ ] `cargo test -p kebab-tui` 통과
- [ ] 도그푸딩: `g` 누르고 `:q` 후 화면 정상 redraw 확인
- [ ] 같은 helper 가 p9-fb-20 의 citation jump 에도 사용

## Out of scope

- editor 종료 코드 처리 (실패해도 TUI 복귀)
- editor stdin 전달 (현재 path 만)
