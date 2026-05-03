---
phase: P9
component: kebab-tui (ask pane)
task_id: p9-fb-11
title: "Ask answer markdown rendering (bold/italic/code/list/table)"
status: in_progress
depends_on: [p9-fb-14]
unblocks: []
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§7 RAG, §10 UX]
source_feedback: p9-dogfooding-feedback.md item 9
---

# p9-fb-11 — Ask markdown render

## Goal

ask 답변의 markdown 문법을 ratatui Span / Line 으로 변환해 시각 구분. raw `**bold**` 사라지고 실제 bold 표시.

## Allowed dependencies

- `pulldown-cmark` (이미 워크스페이스에 있음).
- ratatui (기존).

## Public surface

`kebab-tui::markdown::render(text: &str, theme: &Theme) -> Vec<Line<'static>>`. theme 은 p9-fb-14.

## Behavior contract

inline:
- `**bold**` / `__bold__` → `Modifier::BOLD`.
- `*italic*` / `_italic_` → `Modifier::ITALIC`.
- inline code `` ` `` → bg `theme.code_bg`.
- 링크 `[text](url)` → underline + theme.link.

block:
- heading `#`, `##`, ... → fg color 에 따라 hierarchy.
- list bullet `-` / `*` / `1.` → indent + bullet char.
- code fence ``` ``` → 박스 widget + monospace assumed.
- table `| col |` → ratatui `Table` widget. column auto-width.
- blockquote `>` → 좌측 vertical bar + dim fg.

streaming 처리: 마지막 incomplete inline span (e.g. 닫지 않은 `**`) 은 raw 로 표시. complete 부분만 styled. 매 frame 재 parse — cheap, ms 단위.

## Test plan

| kind | description |
|------|-------------|
| unit | `**hi**` → 1 Span with BOLD modifier |
| unit | code fence → CodeBlock 변환 |
| unit | table 2x2 → ratatui Table |
| snapshot | 복합 답변 (heading + list + code) → snapshot 비교 |

## DoD

- [ ] `cargo test -p kebab-tui` 통과
- [ ] 도그푸딩: bold / italic / table 답변 정상 렌더
- [ ] CLI ask 출력은 raw markdown 유지 (terminal 호환성)

## Out of scope

- 이미지 (markdown img tag) 렌더링 — 터미널 한계
- 링크 클릭 / 따라가기 (P+)
