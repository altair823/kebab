---
phase: P9
component: kebab-tui (search pane)
task_id: p9-2
title: "TUI Search pane: input + result list + preview + editor jump"
status: completed
depends_on: [p2-2, p3-4, p9-1]
unblocks: []
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§1.5/1.6 search output, §3.7 SearchHit, §0 Q3 citation]
---

# p9-2 — TUI Search pane

## Goal

Add a Search pane to the TUI that drives `kebab-app::search`, renders dense results (rank+score / path#frag / heading / snippet), and supports `g` (editor jump to citation) for the selected hit.

## Why now / why this size

Search is the most-used surface. Confining it to one pane leverages the App skeleton from p9-1 without rebuilding key dispatch.

## Allowed dependencies

- `kebab-core`
- `kebab-config`
- `kebab-app`
- `kebab-tui` (extends p9-1)
- `ratatui`, `crossterm`
- `tracing`
- `thiserror`

## Forbidden dependencies

- `kebab-source-fs`, `kebab-parse-*`, `kebab-normalize`, `kebab-chunk`, `kebab-store-*`, `kebab-embed*`, `kebab-search`, `kebab-llm*`, `kebab-rag`, `kebab-desktop`

## Inputs

| input | type | source |
|-------|------|--------|
| `kebab-app::search(query)` | facade | runtime |
| keyboard events | `crossterm` | terminal |
| selected hit's citation | `kebab_core::Citation` | App state |

## Outputs

| output | type | downstream |
|--------|------|------------|
| Ratatui frame for Search pane | render | user |
| External editor process spawn | `std::process::Command` | OS |

## Public surface (signatures only — no new types)

```rust
pub fn render_search<B: ratatui::backend::Backend>(f: &mut ratatui::Frame, area: ratatui::layout::Rect, state: &App);
pub fn handle_key_search(state: &mut App, key: crossterm::event::KeyEvent) -> KeyOutcome;
pub fn jump_to_citation(citation: &kebab_core::Citation, editor_env: &str /* $EDITOR */) -> anyhow::Result<()>;
```

This task fills the body of `kebab_tui::SearchState` (forward-declared in p9-1). The `App` struct itself is NOT edited — only `SearchState` gets fields:

```rust
pub struct SearchState {
    pub input: String,
    pub mode: kebab_core::SearchMode,
    pub hits: Vec<kebab_core::SearchHit>,
    pub selected_hit: usize,
    pub last_query_at: Option<time::OffsetDateTime>,    // debounce timer
}
```

The Library pane's keypress handler (in p9-1) sets `app.search = Some(SearchState::default())` on pane switch; p9-2's `render_search`/`handle_key_search` read `app.search.as_mut()` exclusively. Parallel-safety contract from p9-1 holds.

## Behavior contract

- Layout: top input bar (search query + mode badge `[hybrid|lexical|vector]`), middle result list (one hit per 4 lines per design §1.5 dense format), bottom preview pane (full chunk text fetched lazily via `kebab-app::inspect_chunk`).
- Key bindings (Search pane):
  - typing → updates `search_input`; debounced (200 ms) re-search
  - `Tab` → cycles `search_mode` Lexical → Vector → Hybrid → Lexical
  - `Enter` → forces re-search immediately
  - `j` / `k` or arrow keys → move selected hit
  - `g` → call `jump_to_citation(&hits[selected].citation, &env::var("EDITOR").unwrap_or_else(|_| "vi".into()))`
  - `Esc` → switch back to Library pane
- `jump_to_citation`:
  - For `Citation::Line { path, start, .. }`: spawn `editor +<start> <workspace_root>/<path>`. Common editors `vim`/`nvim`/`vi`/`emacs`/`hx` accept `+N`. Fallback: `code -g <path>:<start>` if `$EDITOR` contains "code".
  - For other citation kinds: open the file in `$EDITOR` without line jump (best effort).
  - Use `std::process::Command::status()` blocking; suspend the TUI (`disable_raw_mode`) before launch and restore on return.
- The search call runs synchronously; for hybrid mode that may take seconds, render a centered "searching…" overlay until complete.
- All search results rendered must conform to design §1.5 dense format (4 lines: `<rank>. <score>  <path#frag>` / `<section_label>` / `<snippet line 1>` / `<snippet line 2>`).
- Errors → popup overlay (consistent with p9-1).
- Stable terminal restoration on panic and process exit.

## Storage / wire effects

- Reads only. No DB writes.
- Spawns external editor process; that process can mutate user files. The TUI does not interfere.

## Test plan

| kind | description | fixture / data |
|------|-------------|----------------|
| unit | typing into search_input triggers re-search after debounce | inline timer mock |
| unit | `Tab` cycles mode through 3 values back to Lexical | inline |
| unit | `j` / `k` move selection within bounds | inline |
| unit | `jump_to_citation` for `Line` builds `+<line> <path>` command (assert via mocked Command runner) | inline |
| snapshot | rendered Search pane with 3 hits + preview stable | TestBackend |
| integration | mocked `kebab-app::search` returning fixture hits drives render | inline |

All tests under `cargo test -p kebab-tui search`.

## Definition of Done

- [ ] `cargo check -p kebab-tui` passes
- [ ] `cargo test -p kebab-tui search` passes
- [ ] `g` keybinding launches `$EDITOR` with correct `+<line>` argument (manual smoke against vim)
- [ ] No imports outside Allowed dependencies
- [ ] PR links design §1.5/1.6, §3.7

## Out of scope

- Inline citation render of LLM answers (Ask pane = p9-3).
- Full `--explain` retrieval trace (mention but defer to a future toggle).
- Mouse selection.

## Risks / notes

- Suspending and restoring crossterm raw mode around the editor spawn is finicky; code defensively (RAII guard).
- Different editors take different jump syntaxes. Provide an env override `KEBAB_EDITOR_JUMP_FORMAT="vim"` for users on exotic editors.
- Long snippet text wrap: clamp to viewport width and ellipsize per design §1.5 (`…` already in dense template).
