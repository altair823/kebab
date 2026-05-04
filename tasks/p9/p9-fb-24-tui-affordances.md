---
phase: P9
component: kebab-tui
task_id: p9-fb-24
title: "TUI status/key bar + Library 컬럼 헤더 + Ask/Inspect PgUp/PgDn (post-merge dogfooding)"
status: completed
depends_on: [p9-fb-03, p9-fb-13, p9-fb-22]
unblocks: []
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§1 UX, §10 UX]
source_feedback: 사용자 도그푸딩 2026-05-04 — Library 컬럼 의미 부재, 페이지 스크롤 키 부재, 항상 떠 있는 상태바 (버전 포함) 요청.
---

# p9-fb-24 — TUI status/key bar + Library 헤더 + page scroll

상세 설계: `docs/superpowers/specs/2026-05-04-p9-fb-24-tui-affordances-design.md`.
구현 계획: `docs/superpowers/plans/2026-05-04-p9-fb-24-tui-affordances.md`.

## Goal

- bottom 영역을 2 row 로 분할 (status bar + key hint bar). 모든 모드 / pane 에서 항상 노출.
- ingest progress 의 dedicated row 를 status bar 의 dynamic slot 으로 흡수.
- Library doc list 위에 컬럼 헤더 row.
- Ask + Inspect 양쪽에 `PgUp` / `PgDn` (fixed `PAGE_STEP = 10`).

## Behavior contract

- Status bar 좌→우: `kebab v<version> │ <pane> │ <docs> docs │ [conv_<8hex>…  │ ]<dynamic_status>`.
- Dynamic state cascade: streaming (Ask) → searching (Search) → indexing (Ingest) → idle.
- conv_id (8-hex prefix + ellipsis) 는 Ask focused + (current_question 또는 turns) 일 때만.
- Library 헤더: `TITLE / TAGS / UPDATED / CHUNKS`, `Role::Heading`. `format_doc_row` 와 boundary 일치.
- Ask `PgUp/PgDn`: `j`/`k` 와 동일 follow_tail freeze. mode 무관.
- Inspect `PgUp/PgDn`: 기존 +/-10 그대로 (단 PAGE_STEP 상수 참조).

## Tests

- status_bar 통합 10 (version / pane / docs / idle / streaming / searching / ingest absorb / Ask conv_id present / Ask conv_id absent / outside Ask).
- library 통합 1 (헤더 row visible).
- Ask 통합 3 (PgDn / PgUp / PgUp saturating + freeze follow_tail / PgDn from Insert).
- Inspect 통합 2 (PAGE_STEP regression).
- format_doc_header 단위 1.
- 기존 695개 테스트 무수정 통과 (`cargo test --workspace -j 1` 기준 716 passed).

## Risks / notes

- `PAGE_STEP = 10` magic — viewport-aware 후속 task 가능.
- 60 컬럼 미만 터미널은 status bar wrap → 1 row 추가 차지.

Live deviations 반영 위치: `tasks/HOTFIXES.md` `2026-05-04 — p9-fb-24` 항목.
