---
phase: P9
component: kebab-tui
task_id: p9-fb-03
title: "TUI ingest as background worker + status bar"
status: completed
depends_on: [p9-fb-01]
unblocks: []
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§7 ingest, §10 UX]
source_feedback: p9-dogfooding-feedback.md item 1
---

# p9-fb-03 — TUI ingest background + status bar

## Goal

TUI 에서 `:ingest` (또는 `r` 키) 누르면 ingest 가 background thread 에서 돌고, status bar 가 progress 그리기. blocking 하지 않음.

## Allowed dependencies

- 기존 kebab-tui deps (ratatui, crossterm, kebab-app, kebab-config).
- 신규 X.

## Public surface

`kebab-tui::App` 에 `ingest_state: Option<IngestState>` slot. p9-3/4 와 동일 parallel-safe pattern.

```rust
pub(crate) struct IngestState {
    rx: Receiver<IngestEvent>,
    counts: AggregateCounts,
    current_path: Option<String>,
    started_at: Instant,
    cancel_tx: Sender<()>, // p9-fb-04 와 wiring
}
```

## Behavior contract

- ingest worker thread 는 `kebab_app::ingest_with_config_progress(cfg, scope, false, Some(tx))` 호출.
- main loop 가 매 frame 마다 `rx.try_recv()` drain → counts 갱신 + status bar 라인 갱신.
- status bar 위치: 화면 하단 1 줄. 형식: `ingest: 142/1024 (14%) parsing notes/foo.md  [0:42]`.
- 완료/abort 시 status bar 가 final line (`✓ ingest: 1024 docs, 4521 chunks, 12.3s` 또는 `✗ aborted at 142/1024`) 잠시 유지 후 자동 hide.
- ingest 중 다른 pane 이동 자유 — Library / Search 등은 그대로 동작 (DB 는 read 가능, partial 결과 surface).

## Test plan

| kind | description |
|------|-------------|
| unit | IngestState event drain 이 counts 누적 |
| integration | TUI run-loop 모의 + IngestEvent stream → status line 텍스트 snapshot |

## DoD

- [ ] `cargo test -p kebab-tui` 통과
- [ ] README TUI 절에 background ingest + status bar 동작 명시
- [ ] 키 cheatsheet 에 `r` (refresh/ingest) 추가

## Out of scope

- desktop (P9-5) progress 표시
- ingest cancel UI (p9-fb-04)
