---
phase: P9
component: kebab-tui (library view)
task_id: p9-1
title: "Ratatui library list view + tag filter"
status: completed
depends_on: [p1-6]
unblocks: [p9-2, p9-3, p9-4]
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [report §16.2 TUI (also tasks/phase-9-ui.md epic), design §3.7 SearchHit, design §1 UX scenes for shared key bindings]
---

# p9-1 — TUI library view

## Goal

Stand up a Ratatui app skeleton with a "Library" pane: list documents, filter by tag/lang, navigate. Establishes the global app loop, key dispatch, and `kebab-app` integration point that the search/ask/inspect panes (p9-2..p9-4) extend.

## Why now / why this size

Library is the cheapest screen and the natural anchor for the TUI shell. Subsequent panes plug into the same dispatch / shared state.

## Allowed dependencies

- `kebab-core`
- `kebab-config`
- `kebab-app` (facade — the only crate this binary touches besides `kebab-core`/`kebab-config`)
- `ratatui = "0.28"`
- `crossterm`
- `tracing`
- `thiserror`

## Forbidden dependencies

- `kebab-source-fs`, `kebab-parse-*`, `kebab-normalize`, `kebab-chunk`, `kebab-store-*`, `kebab-embed*`, `kebab-search`, `kebab-llm*`, `kebab-rag` (UI must go through `kebab-app` only — this is the design §8 boundary)

## Inputs

| input | type | source |
|-------|------|--------|
| `kebab-app::list_docs(filter)` | facade call | runtime |
| keyboard events | `crossterm` | terminal |
| `kebab-config::Config` | runtime | env / file |

## Outputs

| output | type | downstream |
|--------|------|------------|
| Ratatui frame | terminal render | user |
| App state (selected doc, filter, focus) | in-memory | next-pane handoff |

## Public surface (signatures only — no new types)

```rust
// `App` is the SHELL — its full set of fields is owned by p9-1, but the layout
// reserves one optional sub-state slot per pane so p9-2/3/4 can plug their own
// state in WITHOUT modifying the App struct definition. This avoids merge
// conflicts when p9-2/3/4 land in parallel; only p9-1 ever changes `App`.
pub struct App {
    pub config: kebab_config::Config,
    pub focus: Pane,
    pub library: LibraryState,             // owned by p9-1
    pub search:  Option<SearchState>,      // populated by p9-2 (None until that crate links in)
    pub ask:     Option<AskState>,         // populated by p9-3
    pub inspect: Option<InspectState>,     // populated by p9-4
}

// p9-1 defines LibraryState fully. The other 3 sub-states are forward-declared
// as opaque (zero-field) here; their authoring tasks fill them.
pub struct LibraryState { /* docs, filter, selection */ }
pub struct SearchState;       // body filled by p9-2
pub struct AskState;          // body filled by p9-3
pub struct InspectState;      // body filled by p9-4

impl App {
    pub fn new(config: kebab_config::Config) -> anyhow::Result<Self>;
    pub fn run(&mut self) -> anyhow::Result<()>;     // blocking loop until quit
}

pub enum Pane { Library, Search, Ask, Inspect, Jobs }

pub fn render_library<B: ratatui::backend::Backend>(f: &mut ratatui::Frame, area: ratatui::layout::Rect, state: &App);

pub fn handle_key_library(state: &mut App, key: crossterm::event::KeyEvent) -> KeyOutcome;

pub enum KeyOutcome { Continue, Quit, SwitchPane(Pane), Refresh }
```

**Parallel-safety contract:** p9-2 / p9-3 / p9-4 fill the bodies of `SearchState` / `AskState` / `InspectState` in their own crate's source — no edits to `App`, no edits to the other sub-state structs. Their `render_*` and `handle_key_*` functions take `&mut App` but read/write only their own `Option<...>` field. With this slot pattern, the four p9-* tasks can be authored in parallel and merged in any order without conflict on `App`.

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
- Logging: `tracing` initialized to a file under `~/.local/state/kebab/logs/`; never to stdout/stderr (so the TUI is not corrupted).
- Error rendering: a popup overlay shows `error: {msg}\nhint: {hint}` from `anyhow::Error` chain; press any key to dismiss.

## Storage / wire effects

- Reads: `kebab-app::list_docs` only.
- Writes: none.

## Test plan

| kind | description | fixture / data |
|------|-------------|----------------|
| unit | `handle_key_library` arrow-down increments selection within bounds | inline state |
| unit | filter `f` opens edit overlay; `Enter` triggers refresh | inline |
| snapshot | rendered library with 3 docs + filter open produces stable frame buffer (use `ratatui::backend::TestBackend`) | inline |
| unit | error popup renders without panic on injected `anyhow::Error` | inline |
| integration | mocked `kebab-app::list_docs` returning N docs renders all rows | inline |

All tests under `cargo test -p kebab-tui library`.

## Definition of Done

- [ ] `cargo check -p kebab-tui` passes
- [ ] `cargo test -p kebab-tui library` passes
- [ ] No imports outside `kebab-core`, `kebab-config`, `kebab-app`
- [ ] `kebab tui` (or `kebab` if TUI is the default) launches and shows Library on a real terminal (manual smoke)
- [ ] PR links design §8 module boundary, report §16.2 (TUI epic)

## Out of scope

- Search pane (p9-2), Ask pane (p9-3), Inspect pane (p9-4), Jobs pane.
- Mouse support (P+).
- Theme / color customization (P+).
- Cross-platform installation packaging (separate concern).

## Risks / notes

- Ratatui re-renders on every event; large doc lists can be slow. Use `ListState` and only render visible rows.
- crossterm raw-mode cleanup must run on panic (`color_eyre` or manual `disable_raw_mode` in `Drop`); a corrupted terminal after a crash is a UX disaster.
- Korean text rendering width: use `unicode-width` and account for wide characters when computing column widths.
