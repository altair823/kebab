---
phase: P9
component: kb-tui (library view)
task_id: p9-1
title: "Ratatui library list view + tag filter"
status: planned
depends_on: [p1-6]
unblocks: [p9-2, p9-3, p9-4]
contract_source: ../../docs/superpowers/specs/2026-04-27-kb-final-form-design.md
contract_sections: [§16.2 TUI epic (tasks/phase-9-ui.md), §3.7 SearchHit, §1 UX scenes for shared key bindings]
---

# p9-1 — TUI library view

## Goal

Stand up a Ratatui app skeleton with a "Library" pane: list documents, filter by tag/lang, navigate. Establishes the global app loop, key dispatch, and `kb-app` integration point that the search/ask/inspect panes (p9-2..p9-4) extend.

## Why now / why this size

Library is the cheapest screen and the natural anchor for the TUI shell. Subsequent panes plug into the same dispatch / shared state.

## Allowed dependencies

- `kb-core`
- `kb-config`
- `kb-app` (facade — the only crate this binary touches besides `kb-core`/`kb-config`)
- `ratatui = "0.28"`
- `crossterm`
- `tracing`
- `thiserror`

## Forbidden dependencies

- `kb-source-fs`, `kb-parse-*`, `kb-normalize`, `kb-chunk`, `kb-store-*`, `kb-embed*`, `kb-search`, `kb-llm*`, `kb-rag` (UI must go through `kb-app` only — this is the design §8 boundary)

## Inputs

| input | type | source |
|-------|------|--------|
| `kb-app::list_docs(filter)` | facade call | runtime |
| keyboard events | `crossterm` | terminal |
| `kb-config::Config` | runtime | env / file |

## Outputs

| output | type | downstream |
|--------|------|------------|
| Ratatui frame | terminal render | user |
| App state (selected doc, filter, focus) | in-memory | next-pane handoff |

## Public surface (signatures only — no new types)

```rust
pub struct App { /* state: docs, filter, selection, focus pane */ }

impl App {
    pub fn new(config: kb_config::Config) -> anyhow::Result<Self>;
    pub fn run(&mut self) -> anyhow::Result<()>;     // blocking loop until quit
}

pub enum Pane { Library, Search, Ask, Inspect, Jobs }

pub fn render_library<B: ratatui::backend::Backend>(f: &mut ratatui::Frame, area: ratatui::layout::Rect, state: &App);

pub fn handle_key_library(state: &mut App, key: crossterm::event::KeyEvent) -> KeyOutcome;

pub enum KeyOutcome { Continue, Quit, SwitchPane(Pane), Refresh }
```

## Behavior contract

- Layout: header (1 line, breadcrumb / pane label) + body (full) + footer (key hints).
- Library body: scrollable list of `DocSummary` with columns `[title]  [tag list]  [updated_at]  [chunk_count]`.
- Filter bar (toggled by `f`): edit `tags_any` and `lang` fields; pressing `Enter` re-runs `list_docs`.
- Key bindings (Library pane only):
  - `j` / `k` or arrow keys → move selection down/up
  - `g g` → top, `G` → bottom
  - `f` → toggle filter
  - `/` → switch to Search pane (p9-2)
  - `?` → switch to Ask pane (p9-3)
  - `Enter` → switch to Inspect pane (p9-4) on selected doc
  - `q` or `Esc` → quit
- All facade calls run on the main thread (no async). For long calls, render a "loading…" state and call from a worker thread; bridge via `mpsc::channel` (this task may keep things synchronous and accept brief UI hangs for v1).
- Logging: `tracing` initialized to a file under `~/.local/state/kb/logs/`; never to stdout/stderr (so the TUI is not corrupted).
- Error rendering: a popup overlay shows `error: {msg}\nhint: {hint}` from `anyhow::Error` chain; press any key to dismiss.

## Storage / wire effects

- Reads: `kb-app::list_docs` only.
- Writes: none.

## Test plan

| kind | description | fixture / data |
|------|-------------|----------------|
| unit | `handle_key_library` arrow-down increments selection within bounds | inline state |
| unit | filter `f` opens edit overlay; `Enter` triggers refresh | inline |
| snapshot | rendered library with 3 docs + filter open produces stable frame buffer (use `ratatui::backend::TestBackend`) | inline |
| unit | error popup renders without panic on injected `anyhow::Error` | inline |
| integration | mocked `kb-app::list_docs` returning N docs renders all rows | inline |

All tests under `cargo test -p kb-tui library`.

## Definition of Done

- [ ] `cargo check -p kb-tui` passes
- [ ] `cargo test -p kb-tui library` passes
- [ ] No imports outside `kb-core`, `kb-config`, `kb-app`
- [ ] `kb tui` (or `kb` if TUI is the default) launches and shows Library on a real terminal (manual smoke)
- [ ] PR links design §8 module boundary, §16.2 epic

## Out of scope

- Search pane (p9-2), Ask pane (p9-3), Inspect pane (p9-4), Jobs pane.
- Mouse support (P+).
- Theme / color customization (P+).
- Cross-platform installation packaging (separate concern).

## Risks / notes

- Ratatui re-renders on every event; large doc lists can be slow. Use `ListState` and only render visible rows.
- crossterm raw-mode cleanup must run on panic (`color_eyre` or manual `disable_raw_mode` in `Drop`); a corrupted terminal after a crash is a UX disaster.
- Korean text rendering width: use `unicode-width` and account for wide characters when computing column widths.
