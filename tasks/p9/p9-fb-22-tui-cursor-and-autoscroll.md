---
phase: P9
component: kebab-tui
task_id: p9-fb-22
title: "Mid-string cursor editing + Ask follow-tail auto-scroll (post-merge dogfooding)"
status: completed
depends_on: [p9-fb-10, p9-3]
unblocks: []
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§1 UX, §10 UX]
source_feedback: 사용자 도그푸딩 2026-05-04 — Gitea #94 (입력 후 커서 이동 안 됨), Gitea #95 (새 응답이 아래로 추가돼도 자동 스크롤 안 됨).
---

# p9-fb-22 — InputBuffer cursor editing + Ask follow-tail

## Goal

- 모든 input pane (Ask / Search / Library filter overlay) 에서 화살표 / Home / End / Delete 로 mid-string 커서 편집 가능.
- Ask 트랜스크립트가 새 응답 도착 시 자동으로 viewport bottom 을 따라감 (auto-tail). 사용자가 위로 스크롤하면 freeze, 명시적 키 (`Shift-G`) 로 다시 활성화.

## Background

`p9-fb-10` 의 `InputBuffer` 는 의도적으로 append-only — `cursor_col == display_width(content)` invariant 가 항상 성립. 좋은 결정이지만 mid-string 편집 (한글 한 글자 잘못 쳤을 때 backspace 로 다 지우지 않고 화살표로 그 자리만 고치기) 가 불가.

`p9-3` 의 Ask 트랜스크립트는 `Paragraph::scroll((s.scroll, 0))` 의 offset 을 위에서부터 카운트. 새 답변 도착 시 `s.scroll = 0` 으로 리셋 — viewport 가 위쪽 고정. 트랜스크립트가 길어지면 새 응답이 시야 밖으로 밀림. 사용자는 매번 `j` 로 직접 스크롤해야 함.

## Allowed dependencies

- 기존 `kebab-tui` 의존성.
- `ratatui` 의 `unstable-rendered-line-info` feature — `Paragraph::line_count(width)` 사용을 위해 활성화. ratatui 0.28 에 pin 된 동안 안정.

## Public surface

신규 `InputBuffer` 메서드: `move_left`, `move_right`, `move_home`, `move_end`, `delete_after`. 기존 `push_char` / `pop_char` 는 cursor 위치에서 동작하도록 의미 변경 (cursor 가 끝에 있을 때 기존 동작과 동일).

신규 `AskState` 필드: `follow_tail: bool` (default `true`).

## Behavior contract

### InputBuffer

- `cursor_byte` 가 새 source of truth (UTF-8 char boundary). `cursor_col()` 는 prefix slice 의 `unicode-width` 합으로 derive.
- `push_char(ch)`: `content.insert(cursor_byte, ch)` 후 `cursor_byte += ch.len_utf8()`. cursor 가 끝에 있을 때 기존 append 동작과 동일.
- `pop_char()`: cursor 직전 char 제거. cursor 가 시작에 있을 때 `None` 반환.
- `delete_after()`: cursor 위치 char 제거 (cursor 그대로). 끝에서는 `None`.
- `move_left() / move_right()`: char-boundary 단위 이동. `bool` 반환 (이동 성공 여부).
- `move_home() / move_end()`: 양 끝점 점프.
- backwards-compat: cursor 가 끝일 때 모든 메서드 동작이 p9-fb-10 spec 과 동일. 30+ 기존 테스트 변경 없이 통과.

### Ask follow-tail

- `AskState::default()` 가 `follow_tail = true` 로 초기화 (수동 `Default` impl 추가 — `derive(Default)` 는 `false` 가 됨).
- `render_answer` 가 `follow_tail` 동안 매 프레임 `Paragraph::line_count(inner.width)` 로 wrapped row 수 계산, `scroll = line_count - inner_height` 로 pin. wrap-aware 이므로 viewport 너비 변경 시에도 정확히 bottom.
- `j` (scroll down): `follow_tail = false` 로 disengage. `s.scroll += 1`.
- `k` (scroll up): `follow_tail = false`. `s.scroll -= 1`.
- `Shift-G`: `follow_tail = true` + `s.scroll = 0`. Normal 모드에서만.
- 새 submission, `Ctrl-L` 도 `follow_tail = true` 재설정.

### Pane key handler 추가

- Ask: `Left / Right / Home / End / Delete` mode 무관 (Mode::Insert / Normal 양쪽). `Shift-G` Normal 한정.
- Search: 동일 5 key. `Delete` 만 input_dirty_at reset (cursor 이동 ≠ 쿼리 변경 → debounce timer 유지).
- Library filter overlay: 동일 5 key, 활성 field (Tags / Lang) 의 buffer 에 적용.

## Tests

- 11 신규 InputBuffer unit (move_left/right ASCII/Hangul, home/end, mid-string insert, backspace at cursor + at home no-op, delete_after at cursor + at end no-op, mixed-width cursor invariant, take 후 cursor reset).
- 10 신규 Ask integration (Left/Right/Home/End/Delete on Ask input, Hangul left arrow, follow_tail default, k disengages, Shift-G re-engages, Ctrl-L resets, follow-tail rendering bottom of long transcript).
- 기존 38 개 테스트는 그대로 통과 (cursor 가 끝일 때 backwards-compat).

## Risks / notes

- `ratatui::Paragraph::line_count` 가 unstable feature flag 뒤에 있음 — ratatui 0.28 → 0.29 bump 시 stable surface 여부 재확인 필요. unstable surface 가 사라지면 manual estimator (per-Line `ceil(display_cols / inner_width)`) 로 fallback 가능.
- cheatsheet popup body 가 Search +3 row, Ask +4 row 늘어남. p9-fb-21 의 deferred 한계 (75% height 안에 Inspect section 잘림 가능) 가 더 빡빡해짐 — 후속 task 로 popup scroll 또는 multi-column layout 고려 필요.

Live deviations 반영 위치: `tasks/HOTFIXES.md` `2026-05-04 — p9-fb-22` 항목.
