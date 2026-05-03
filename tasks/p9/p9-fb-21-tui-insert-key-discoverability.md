---
phase: P9
component: kebab-tui
task_id: p9-fb-21
title: "Insert-mode key + cheatsheet discoverability (post-merge dogfooding)"
status: completed
depends_on: [p9-fb-12, p9-fb-13]
unblocks: []
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§10 UX]
source_feedback: 사용자 도그푸딩 2026-05-03 — Ask Insert→Esc→Normal 후 Insert 로 돌아가는 키 모름. 전반적 키바인딩 안내 부족.
---

# p9-fb-21 — Insert-mode toggle + F1 visibility

## Goal

- 모든 pane 의 NORMAL 모드에서 `i` 가 INSERT 로 토글. 사용자가 Search/Ask 의 자동 INSERT → Esc → NORMAL 후 Insert 로 돌아가는 경로 확보.
- footer hint line 첫 fragment 가 항상 `F1 도움말` — F1 cheatsheet binding 의 discoverability 보장.

## Background

p9-fb-12 의 mode_intercept rule:
- NORMAL→INSERT 의 `i` intercept 가 Library/Inspect/Jobs 만.
- Search/Ask 는 자동 INSERT 라 `i` 가 typed char 로 fall-through.

문제: 사용자가 Search/Ask 에서 `Esc` 로 NORMAL 진입 후 Insert 로 돌아가는 키 없음. footer hint 도 안내 없음. F1 cheatsheet 자체도 invisible.

## Allowed dependencies

- 기존 kebab-tui 만.

## Public surface

기존 `mode_intercept` + `footer_hints` + cheatsheet sections 갱신. 신규 public type 없음.

## Behavior contract

- **`mode_intercept`**: `(Char('i'), Mode::Normal, _)` — pane 무관 모두 INSERT 로 flip + intercept consume.
- **Search 의 chunk inspect 키**: 기존 `i` → `o` rebind (vim "open"). `i` 가 universal Insert toggle 로 자유로워짐.
- **footer hint 모든 (pane, mode, filter) 조합**: `F1 도움말  ...` 으로 시작.
- **Search/Ask Normal hint**: `i 입력모드` fragment 추가.
- **cheatsheet 갱신**: Global `i` 설명 = "Normal → Insert (every pane)". Search 의 `i` row 분리 — `o = inspect`, `i = Normal → Insert`. Ask 에 `i = Normal → Insert` 추가.

## Test plan

| kind | description |
|------|-------------|
| unit | `i_on_search_or_ask_in_normal_flips_to_insert` — Normal → `i` → Insert intercept |
| unit | `i_on_search_or_ask_in_insert_falls_through_to_pane` — Insert 에서 `i` 는 typed char (회귀 방지) |
| unit | `o_in_normal_with_hits_enters_inspect` — Search Normal `o` → SwitchPane(Inspect) |
| unit | `o_in_normal_with_empty_hits_is_continue` — `o` no-op when hits empty |
| unit | `o_in_insert_types_into_input` — Insert 에서 `o` 는 typed char |
| unit | `every_hint_starts_with_f1_help_prefix` — 모든 (pane, mode, filter) 조합이 `F1 도움말` 으로 시작 (exhaustive) |
| unit | `search_ask_normal_hint_advertises_i_insert_toggle` — Search/Ask Normal hint 에 `i 입력모드` fragment 존재 |
| unit | `search_normal_hint_lists_commands_directly` — 기존 테스트 갱신 (`i 인스펙트` → `o 인스펙트` + `i 입력모드`) |

## DoD

- [x] `cargo test -p kebab-tui` 통과
- [x] `cargo clippy -p kebab-tui --all-targets -- -D warnings` clean
- [x] 도그푸딩: 사용자가 Insert→Esc→Normal 후 `i` 로 즉시 복귀 가능
- [x] README + HANDOFF + HOTFIXES 갱신

## Out of scope

- Library/Inspect 에서도 `i` 누르면 INSERT 로 flip — 기존 동작 유지 (pre-fb-21 부터 동작).
- Sticky-per-pane mode (사용자가 명시적으로 Esc 한 pane 만 Normal 유지) — 후속 task.
- footer hint 자동 줄바꿈 (긴 hint 가 80-col 에서 잘릴 수 있음) — 별도 task.

## Notes

- Search 의 `i`→`o` rebind 은 frozen spec 에 명시된 키 변경. `o` = vim "open" 의 mnemonic.
- HOTFIXES 에 키 rebind 명시.
