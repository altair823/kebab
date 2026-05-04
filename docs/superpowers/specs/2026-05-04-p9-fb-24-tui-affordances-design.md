# p9-fb-24 — TUI status/key bar + Library header + page scroll

**Date**: 2026-05-04
**Status**: planned
**Audience**: kebab-tui implementer / reviewer.
**Source feedback**: 사용자 도그푸딩 2026-05-04 — (1) Library 컬럼이 무엇을 뜻하는지 헤더 부재, (2) Ask transcript / Inspect 둘 다 페이지 단위 스크롤 키 필요, (3) 모든 모드에서 항상 떠 있는 상태바 + 키 안내바 (버전 정보 포함) 가 있으면 좋겠다.

## Goal

- bottom 영역을 2 row 로 분할: 윗줄 = 항상 떠 있는 상태바 (`kebab v0.1.0 │ pane │ docs 수 │ 동적 상태`), 아랫줄 = 기존 mode-aware 키 안내 (현 `footer_hints` 그대로 이전).
- ingest progress 의 dedicated row 를 새 status bar 의 동적 상태 영역으로 흡수 — 시각적 source 단일화.
- Library 의 List 위에 컬럼 헤더 row 추가 (TITLE / TAGS / UPDATED / CHUNKS, display-width 정렬, `Role::Heading` 색).
- Ask + Inspect 양쪽에 `PgUp` / `PgDn` 페이지 스크롤 (fixed step 10). Ask 의 PgDn / PgUp 은 `j` / `k` 와 동일하게 `follow_tail = false` 로 freeze.

## Non-goals

- viewport-aware 페이지 스텝 (fixed step 10 으로 시작, 후속 task 에서 viewport-relative 업그레이드 가능).
- Library `List` → `Table` 위젯 마이그레이션 (별 header row + 기존 List 유지 — sort/정렬 인디케이터 필요할 때 후속).
- 키 안내바 콘텐츠 확장 — 현 `footer_hints` 출력 그대로 이전, 키 추가/제거 없음.
- conversation id 풀 표시 — Ask 진입 시 8 자 prefix 만.

## Allowed dependencies

- `kebab-tui` 자체 (ratatui 0.28, crossterm 0.28). 신규 crate 없음.
- `env!("CARGO_PKG_VERSION")` (compile-time, std).

## Public surface

- `kebab_tui::run::render_status_bar(f, area, app)` 신규 (pub(crate)).
- `kebab_tui::run::render_key_hints(f, area, app)` — 기존 `render_footer` rename. 동작 동일.
- `kebab_tui::library::format_doc_header(area_width: u16) -> ratatui::text::Line<'static>` 신규.
- 기존 `IngestState` 의 dedicated render path 제거 — status bar 가 흡수.

## Behavior contract

### 상태바 (line 1)

좌→우 fragment, `│` separator (단순 ASCII pipe + 양쪽 공백):

```
kebab v0.1.0  │  <pane>  │  <doc_count> docs  │  [conv_<id_8>…  │  ]<dynamic_status>
```

- **버전**: `env!("CARGO_PKG_VERSION")` — workspace pinned 단일 값.
- **pane 라벨** (영문): `Library` / `Search` / `Ask` / `Inspect` / `Jobs`.
- **doc_count**: `app.library.inner.docs.len()` 직접 읽음. Library 의 `needs_refresh` 사이클이 이미 갱신 보장.
- **conversation_id (Ask 전용)**: `current_question.is_some() || !turns.is_empty()` 일 때만. 표시 form: `conv_<8 hex chars>…` (전체 32 hex 의 head 8 자 + ellipsis).
- **dynamic_status**: 우선순위 cascade — 한 번에 하나만:
  1. `streaming…` — `app.ask.as_ref().map(|s| s.streaming).unwrap_or(false)`
  2. `searching…` — `app.search.as_ref().map(|s| s.is_searching()).unwrap_or(false)`
  3. `indexing N/M (P%)` — `app.ingest_state.is_some() && !ingest_state.is_terminal()`. terminal (Completed/Aborted) 후 final 메시지 (`indexed N+M (T)` / `aborted at N/M`) 3 초 hold 후 `idle`.
  4. `idle` — fallback.

스타일: 전체 `Role::Hint`, dynamic_status 만 우선순위별 색 (streaming/searching = `Role::Heading`, indexing = `Role::Warning`, idle = `Role::Hint`).

### 키 안내바 (line 2)

기존 `footer_hints(focus, mode, filter_open)` 출력 그대로 single-line `Paragraph`. `Role::Hint`. wrap 시 자연스럽게 다음 줄 (단, 권장 환경 80+ 컬럼에서 wrap 거의 발생 안 함).

### 레이아웃

`render_root` Constraint 변경:

```
이전: [Length(3) header, Min(0) main, Length(1) ingest_status_optional, Length(1) footer]
이후: [Length(3) header, Min(0) main, Length(1) status_bar, Length(1) key_hints]
```

- `ingest_status_optional` 제거. status bar 가 흡수.
- error overlay 는 modal layer (Layout 영향 없음) — 그대로.

콘텐츠 영역 손실: 0 ~ 1 row (이전엔 ingest 진행 시만 1 row 차지, 평소엔 0 — 평균 +0.x row 손실).

### Library 헤더

```
┌Library — 42 docs──────────────────────────────────────┐
│TITLE                              TAGS         UPDATED  CHUNKS│
│친애하는 미스터 최                  rust,prog    2025-04  12   │
│architecture-spec                   docs         2025-05  47   │
│...                                                    │
└──────────────────────────────────────────────────────┘
```

- Block `inner` 안 vertical Layout 두 단계: `Length(1)` 헤더 paragraph + `Min(0)` List.
- `format_doc_header(area_width)` 가 `format_doc_row` 와 동일 컬럼 폭 계산식 사용 (display-width 정렬, TAGS_COL_W=12, UPDATED 10, CHUNKS unpadded).
- 헤더 라벨: `TITLE` / `TAGS` / `UPDATED` / `CHUNKS` (영문 cap).
- 색: `theme.style(Role::Heading)` (Bold cyan/팔레트별).
- `docs.is_empty()` 상태에서도 헤더는 표시. List 영역에 "(no docs)" hint.

### PgUp / PgDn

`const PAGE_STEP: u16 = 10;` 모듈 상수 (kebab-tui::input 또는 별 `pager.rs`).

**Ask** (`crates/kebab-tui/src/ask.rs::handle_key_ask`):

- `KeyCode::PageDown`: `s.scroll = s.scroll.saturating_add(PAGE_STEP); s.follow_tail = false;`
- `KeyCode::PageUp`: `s.scroll = s.scroll.saturating_sub(PAGE_STEP); s.follow_tail = false;`
- mode 무관 (Insert / Normal 양쪽). 기존 `j`/`k` 와 동일 의미 (자동 tail freeze).

**Inspect** (`crates/kebab-tui/src/inspect.rs`):

- 기존 +/-10 hardcode 를 `PAGE_STEP` 상수 참조로 교체. 동작 동일 (10 → 10).

cheatsheet popup Ask section 에 `PgUp / PgDn` row 추가, Inspect 는 기존 row 유지 (이미 명시).

## Tests

### 신규 단위 / 통합

- `render_status_bar` snapshot — 5 pane × 4 dynamic state (idle / streaming / searching / indexing) ≈ 8~10 case. 각 case 에서 version + pane + doc_count + dynamic 텍스트 visible.
- `render_status_bar` Ask conv_id case — `current_question.is_some()` 시 `conv_<8hex>…` 형태 visible.
- `render_status_bar` ingest absorb — `IngestState::Indexing { current, total }` 일 때 `indexing 12/40 (30%)` 정확.
- `format_doc_header` 단위 — 라벨 + display-width 정렬이 `format_doc_row` 와 boundary 일치.
- `library` integration — TestBackend, docs 3 fixture, header row + data row 모두 visible. Hangul 제목 정렬 회귀 확인.
- Ask `PageDown` / `PageUp` 신규 통합 — fixed step 10, `follow_tail` `false` 변경.
- Inspect `PageDown` / `PageUp` 회귀 — `PAGE_STEP` 상수 path.

### 기존 영향

- `footer_hints` 8 단위 테스트 — rename 외 무수정 통과.
- 기존 ingest progress render 테스트 — status bar 통합 후 텍스트 visible 검증으로 재작성 (위치만 이동, 콘텐츠 동일).
- p9-fb-22 Ask follow-tail 통합 테스트 — `j`/`k` / `Shift-G` / Ctrl-L / submission 시 `follow_tail` 동작 그대로 통과 (PgUp/PgDn 만 추가).

## Spec contract impact

- **p9-fb-13 follow-up (footer 단행 row)** frozen 텍스트와 충돌. frozen 그대로 두고 본 spec + HOTFIXES `2026-05-04 — p9-fb-24` 항목이 live source of truth.
- **p9-fb-03 (TUI background ingest)** 의 dedicated status row 가 status bar 의 동적 영역으로 흡수 — 시각적 위치 변경, 콘텐츠 동등. HOTFIXES 항목 cross-link.
- **p9-fb-22 (cursor + follow-tail)** Ask 키 매핑 보존 + PgUp/PgDn 추가 (충돌 없음).
- **p9-fb-21 (cheatsheet)** popup 의 Ask section 에 `PgUp / PgDn` row 추가.

## Risks / notes

- **80 컬럼 wrap**: `kebab v0.1.0  │  Library  │  42 docs  │  idle` ≈ 50 자, Ask conv_id 추가 시 ≈ 60 자. 80 컬럼 안전. 60 컬럼 미만 환경은 status bar wrap → 임시 한 줄 추가 차지. kebab TUI 권장 환경 80+ 가정.
- **콘텐츠 영역 1 row 손실**: 24 row 작은 터미널에서 transcript 영역 1 row 짧아짐. 실사용 무시 수준.
- **dynamic status priority cascade**: 동시 active 상태 (streaming + indexing 등) 시 streaming 우선 표기. 사용자 인지 우선순위와 일치 (포커스 = Ask 면 streaming, ingest 는 background).
- **`PAGE_STEP = 10` magic**: viewport 와 무관 fixed. 24 row 작은 터미널에서 한 페이지 = 10 row 가 viewport 보다 큼 (overflow 무해). 80 row 큰 터미널에서는 한 페이지가 viewport 보다 작음 (느린 페이징). 후속 task 가 viewport-aware 로 업그레이드 시 본 spec 의 동작은 frozen.

## Live deviations

추후 발견되는 deviation 은 `tasks/HOTFIXES.md` 의 `2026-05-04 — p9-fb-24` 항목에 dated 로그로 추가. spec 자체는 frozen.
