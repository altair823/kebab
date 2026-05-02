---
phase: P9
component: kebab-tui
task_id: p9-fb-10
title: "CJK input + wide-char rendering audit"
status: planned
depends_on: [p9-fb-12]
unblocks: []
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§10 UX]
source_feedback: p9-dogfooding-feedback.md item 8
---

# p9-fb-10 — CJK input

## Goal

한글 / 일본어 / 중국어 입력 + 출력이 깨지지 않게. mode machine (p9-fb-12) 위에 IME safe 입력 흐름.

## Allowed dependencies

- `unicode-width = "0.2"` (이미 워크스페이스에 있는지 확인 후 도입).

## Public surface

`kebab-tui::input::InputBuffer` — String + cursor (column 단위 wide-char width 인지). ratatui Span 렌더링 시 `unicode-width::UnicodeWidthStr` 로 정확한 width.

## Behavior contract

- IME composing event: crossterm 은 native IME composing surface X — 자모 단위 `KeyCode::Char(c)` 로 도착. mode machine (p9-fb-12) 에서 INSERT 모드면 모든 Char 가 buffer push, NORMAL 모드면 single-key command.
- wide char width: `c.width()` 로 cursor column 진행. ASCII=1, CJK=2.
- buffer 의 byte index vs char index 구분 — backspace 는 마지막 char (`pop_char`) 단위 삭제, byte slice 금지.
- 한글 fixture 추가: `fixtures/markdown/한글-테스트.md`, query `러스트 비동기`, 답변 streaming `테스트 답변` 등.

## Test plan

| kind | description |
|------|-------------|
| unit | InputBuffer 한글 5 자 push → display_width = 10 |
| unit | backspace 가 자모 1 단위가 아닌 완성형 글자 1 단위 (utf-8 char boundary) |
| unit | 한글 query → SQLite FTS5 정상 검색 (이미 NFC 정규화) |
| integration | TUI run-loop 모의로 한글 query 입력 → status bar 글자 깨짐 X |

## DoD

- [ ] `cargo test -p kebab-tui` 통과
- [ ] 한글 fixture 추가
- [ ] README — CJK 입력 동작 정상 명시

## Out of scope

- macOS IME (Korean composing 시 system level) 회피 — fallback 안내 (외부 editor 사용 권장)
- emoji surrogate pair (현재 pulldown-cmark 가 처리)
