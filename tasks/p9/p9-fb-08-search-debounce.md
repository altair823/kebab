---
phase: P9
component: kebab-tui (search pane)
task_id: p9-fb-08
title: "Search debounce + Enter-immediate trigger"
status: in_progress
depends_on: []
unblocks: []
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§10 UX]
source_feedback: p9-dogfooding-feedback.md item 6
---

# p9-fb-08 — Search debounce

## Goal

TUI search pane 의 keystroke-by-keystroke 검색 제거. debounce 250ms + Enter 즉시 trigger.

## Allowed dependencies

- 기존 kebab-tui deps.

## Public surface

`kebab-tui::search::SearchState` 에 `debounce_at: Option<Instant>` 추가. main run-loop tick 에서 check.

## Behavior contract

- 글자 입력 / backspace → `debounce_at = Instant::now() + 250ms`. 기존 in-flight worker 는 cancel 신호 받음 (다음 step 에서 drop, 결과 stale 으로 폐기).
- main loop 가 매 tick 마다 `if Instant::now() >= debounce_at && state.dirty { spawn search worker; debounce_at=None }`.
- Enter 누름 → debounce 무시 즉시 spawn.
- 같은 query 로 재 spawn 방지 (간단 dedupe — 직전 query 와 비교).
- worker 결과 도착 시 generation counter 비교: 사용자가 추가 입력해 query 가 바뀌면 stale 결과 drop.
- generation counter pattern 은 p9-fb-19 cache 와 같은 prerequisite — 코드 공유.

## Test plan

| kind | description |
|------|-------------|
| unit | 글자 5 회 빠르게 입력 → worker spawn 1 회 |
| unit | Enter 즉시 spawn |
| unit | 입력 → 결과 도착 → 추가 입력 → stale drop |

## DoD

- [ ] `cargo test -p kebab-tui` 통과
- [ ] README TUI search 절에 debounce 동작 명시

## Out of scope

- search 결과 캐싱 (p9-fb-19 별도)
- CLI search 동작 변경 (CLI 는 단발)
