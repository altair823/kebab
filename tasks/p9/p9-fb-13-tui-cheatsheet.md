---
phase: P9
component: kebab-tui + README
task_id: p9-fb-13
title: "Cheatsheet popup (?) + README keymap table + verb hint line"
status: in_progress
depends_on: [p9-fb-12]
unblocks: []
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§10 UX]
source_feedback: p9-dogfooding-feedback.md item 11
---

# p9-fb-13 — Cheatsheet

## Goal

vim 비익숙 사용자도 TUI 조작 가능. `?` modal popup + 동사구 hint line + README keymap 표.

## Allowed dependencies

- 기존 kebab-tui deps.

## Public surface

`kebab-tui::cheatsheet::Cheatsheet` widget. mode + 현재 pane 별 분기.

## Behavior contract

- `?` 키 (NORMAL 만) → modal popup. 화면 중앙 70% box, 키 + 동사구 설명 표.
- popup 안에서 `?` 또는 `Esc` → close.
- pane 별 cheatsheet 분리 — Library / Search / Ask / Inspect 각각 다른 키. 공통 키 (Tab pane 이동, q quit) 는 footer 영역.
- hint line (status bar 위 1 줄) — 동사구로:
  - 기존: `j/k=move`
  - 신규: `↑/k 위로  ↓/j 아래로  Enter 선택  Esc 취소`
- mode 따라 hint line 다름 (p9-fb-12 와 wire). NORMAL = navigation, INSERT = `Esc 로 명령모드`.

README 갱신:
- **TUI 키 매핑** 표 (전역 + pane 별).
- vim 비유 안내 한 줄 ("vim 처럼 i 로 입력, Esc 로 명령").

## Test plan

| kind | description |
|------|-------------|
| unit | `?` press → cheatsheet visible flag |
| unit | mode + pane 변경 시 hint line 텍스트 변화 |
| snapshot | popup 의 키 표 snapshot (Library / Search / Ask / Inspect) |

## DoD

- [ ] `cargo test -p kebab-tui` 통과
- [ ] README **TUI** 절에 키 매핑 표 + cheatsheet 안내
- [ ] 도그푸딩: 첫 사용자가 `?` 만 알면 나머지 발견 가능

## Out of scope

- 사용자 정의 keymap 파일 (P+)
- popup 의 검색 (`/` 로 키 찾기) — 우선 skip
