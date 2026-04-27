---
phase: P9
component: kb-tui (ask pane)
task_id: p9-3
title: "TUI Ask pane: streaming answer + citation links + --explain toggle"
status: planned
depends_on: [p4-3, p9-1]
unblocks: []
contract_source: ../../docs/superpowers/specs/2026-04-27-kb-final-form-design.md
contract_sections: [§1.1–1.4 ask scenes, §2.3 Answer wire, §3.8 Answer]
---

# p9-3 — TUI Ask pane

## Goal

Add an Ask pane that calls `kb-app::ask`, streams tokens into the answer area in real time, renders citation footnotes (default mode A), and toggles to `--explain` (mode B + retrieval trace) with a key.

## Why now / why this size

Streaming UI is the only TUI piece that meaningfully differs from search/inspect. Confining it here keeps the change set focused.

## Allowed dependencies

- `kb-core`
- `kb-config`
- `kb-app`
- `kb-tui` (extends p9-1)
- `ratatui`, `crossterm`
- `tracing`
- `thiserror`

## Forbidden dependencies

- `kb-source-fs`, `kb-parse-*`, `kb-normalize`, `kb-chunk`, `kb-store-*`, `kb-embed*`, `kb-search`, `kb-llm*`, `kb-rag` (only via `kb-app`), `kb-desktop`

## Inputs

| input | type | source |
|-------|------|--------|
| `kb-app::ask(query, AskOpts)` | facade | runtime |
| keyboard events | `crossterm` | terminal |

## Outputs

| output | type | downstream |
|--------|------|------------|
| Ratatui Ask pane render | terminal | user |
| `kb-app::ask` invocation with streaming closure | facade | RAG pipeline |

## Public surface (signatures only — no new types)

```rust
pub fn render_ask<B: ratatui::backend::Backend>(f: &mut ratatui::Frame, area: ratatui::layout::Rect, state: &App);
pub fn handle_key_ask(state: &mut App, key: crossterm::event::KeyEvent) -> KeyOutcome;
```

`App` extended with: `ask_input: String`, `ask_explain: bool`, `ask_streaming: bool`, `ask_partial: String`, `ask_answer: Option<kb_core::Answer>`, `ask_thread: Option<std::thread::JoinHandle<anyhow::Result<kb_core::Answer>>>`, `ask_rx: Option<std::sync::mpsc::Receiver<String>>`.

## Behavior contract

- Layout: top input bar (`?` prompt, query text), middle answer area (rendered Markdown-light: paragraphs + inline `[N]` markers), bottom-right citations panel (numbered list of citations with `path#fragment` and section label), bottom-left status (`grounded ✓/✗  model  prompt_v  k chunks`).
- Submission: `Enter` triggers a worker thread that calls `kb-app::ask`. The thread receives a `mpsc::Sender<String>` it forwards each token through (closure plugged into `AskOpts.print_stream`). The TUI reads from the receiver and appends to `ask_partial`.
- Streaming: while `ask_streaming = true`, the Answer area shows `ask_partial` and a small "▍" cursor. When the worker finishes, `ask_answer` is populated and the citations panel switches to the final list.
- Refusal rendering:
  - `grounded = false` and `refusal_reason = ScoreGate` → render the answer (which is the human-friendly "근거 부족…" message), citations show "가까운 후보".
  - `grounded = false` and `refusal_reason = LlmSelfJudge` → same layout but status shows `grounded ✗  …  3 chunks searched, 0 grounded`.
- Key bindings (Ask pane):
  - typing → updates `ask_input`
  - `Enter` → submit (only when not currently streaming)
  - `e` → toggle `ask_explain`; resubmit on next `Enter`. While explain ON, citations panel is replaced by the per-claim breakdown (mode B in design §1.2) and a footer shows the retrieval trace summary.
  - `Esc` → switch back to Library pane (cancellation of an in-flight ask is best-effort: the worker thread continues but its final answer is dropped).
  - `j` / `k` → scroll the answer area when oversized.
- All facade calls stay within `kb-app::ask` — never reach into `kb-rag` directly.
- Errors render as a popup overlay; do not crash the pane.

## Storage / wire effects

- Reads/writes via `kb-app::ask` which itself writes the `answers` row in `kb.sqlite`. The pane has no direct DB access.

## Test plan

| kind | description | fixture / data |
|------|-------------|----------------|
| unit | submission spawns worker exactly once per `Enter` | inline mock |
| unit | streaming receiver accumulates tokens into `ask_partial` | inline mock with 5 tokens |
| unit | toggle `e` flips `ask_explain` and re-submits on `Enter` | inline |
| unit | refusal answer renders without citations panel index errors | inline |
| snapshot | rendered Ask pane mid-stream is stable | TestBackend |
| snapshot | rendered Ask pane after finished grounded answer is stable | TestBackend |
| integration | mocked `kb-app::ask` returning a canned `Answer` populates final state correctly | inline |

All tests under `cargo test -p kb-tui ask`.

## Definition of Done

- [ ] `cargo check -p kb-tui` passes
- [ ] `cargo test -p kb-tui ask` passes
- [ ] No imports outside Allowed dependencies
- [ ] Manual smoke: stream tokens visible character-by-character against a real Ollama (or `MockLanguageModel`)
- [ ] PR links design §1.1–1.4, §2.3

## Out of scope

- Persistent multi-turn chat memory.
- Conversational follow-ups.
- Voice input.
- Token-by-token highlighting per claim (the per-claim mode renders after completion).

## Risks / notes

- `mpsc::Receiver::try_recv` polled in the render loop; missing polls = stuttery streaming. Throttle the render at 30 fps and drain the channel each frame.
- Worker thread join on quit must not block forever; use `join_timeout` or detach if quit signaled.
- Cancellation: real cancellation of the LLM stream is provider-specific and out of scope. We accept "fire and forget" with discarded result on `Esc`.
