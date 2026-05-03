---
phase: P9
component: kebab-tui
task_id: p9-fb-14
title: "TUI color theme module (role-based + dark/light toggle)"
status: completed
depends_on: []
unblocks: [p9-fb-11]
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§10 UX]
source_feedback: p9-dogfooding-feedback.md item 12
---

# p9-fb-14 — Color theme

## Goal

TUI 의 정보 종류별 color role 매핑. 모든 pane 이 single source 인 `theme` 모듈에서 Style 가져옴.

## Allowed dependencies

- 기존 kebab-tui deps.

## Public surface

```rust
pub struct Theme { /* role -> Style 맵 */ }
impl Theme {
    pub fn dark() -> Self;
    pub fn light() -> Self;
    pub fn style(&self, role: Role) -> Style;
}

pub enum Role {
    Title, Path, ScoreHigh, ScoreMid, ScoreLow,
    ModeLexical, ModeVector, ModeHybrid,
    Warning, Error, StreamingNew,
    CitationLink, KeywordHighlight,
    ModeNormal, ModeInsert,
    BorderActive, BorderInactive,
    Bullet, CodeBg, BlockquoteBar,
}
```

## Behavior contract

- 기본 theme: dark.
- `theme = "dark" | "light"` config field 신규. `T` 키 (NORMAL 모드) toggle.
- color role 매핑 — 이전 항목 12 의 role 표 따름.
- accessibility: color 단독 의미 전달 X. Score 는 숫자 + color, mode 는 텍스트 + color.

## Test plan

| kind | description |
|------|-------------|
| unit | `Theme::dark().style(Role::Title)` 가 정의된 fg/bg 반환 |
| unit | dark / light 의 모든 Role 변형 정의 누락 X (exhaustive match) |

## DoD

- [ ] `cargo test -p kebab-tui` 통과
- [ ] Library / Search / Ask / Inspect 의 직접 `Style::default().fg(...)` 호출 사라짐 (모두 theme 경유)
- [ ] config.toml 코멘트에 `theme = "dark"` 명시
- [ ] README — theme 토글 키 안내

## Out of scope

- 사용자 정의 color (`[theme.custom]` 절) — P+
- terminal 의 truecolor 미지원 fallback (256-color assumed)
