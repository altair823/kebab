---
phase: P9
component: kb-tui (inspect pane)
task_id: p9-4
title: "TUI Inspect pane: document & chunk detail render"
status: planned
depends_on: [p1-6, p9-1]
unblocks: []
contract_source: ../../docs/superpowers/specs/2026-04-27-kb-final-form-design.md
contract_sections: [§1 inspect output, §3.5 Chunk, §2.5 DocSummary, §2.6 ChunkInspection]
---

# p9-4 — TUI Inspect pane

## Goal

Render document and chunk inspection views (matching the wire schemas `doc_summary.v1` and `chunk_inspection.v1`) with collapsible sections for `metadata`, `provenance`, `blocks` (doc) and `embeddings` (chunk).

## Why now / why this size

Inspect is read-only and has no external interactions; smallest possible pane. Useful for debugging chunker output and citation provenance during P5+ tuning.

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
| `kb-app::inspect_doc(id)` | facade | runtime |
| `kb-app::inspect_chunk(id)` | facade | runtime |
| keyboard events | `crossterm` | terminal |

## Outputs

| output | type | downstream |
|--------|------|------------|
| Ratatui Inspect pane render | terminal | user |

## Public surface (signatures only — no new types)

```rust
pub enum InspectTarget { Doc(kb_core::DocumentId), Chunk(kb_core::ChunkId) }

pub fn render_inspect<B: ratatui::backend::Backend>(f: &mut ratatui::Frame, area: ratatui::layout::Rect, state: &App);
pub fn handle_key_inspect(state: &mut App, key: crossterm::event::KeyEvent) -> KeyOutcome;
```

This task fills the body of `kb_tui::InspectState` (forward-declared in p9-1). `App` is NOT edited.

```rust
pub struct InspectState {
    pub target: Option<InspectTarget>,
    pub doc: Option<kb_core::CanonicalDocument>,
    pub chunk: Option<kb_core::Chunk>,
    pub collapsed: std::collections::HashSet<&'static str>,
    pub scroll: u16,
}
```

`render_inspect`/`handle_key_inspect` read `app.inspect.as_mut()` exclusively. Parallel-safety contract from p9-1 holds.

## Behavior contract

- Switching to Inspect from Library passes `Doc(selected.doc_id)`. From Search pressing `i` (new key on Search pane) passes `Chunk(selected_hit.chunk_id)`.
- Doc view layout (top to bottom):
  1. Header (title, doc_path, doc_id, lang, source_type, trust_level)
  2. Metadata (aliases / tags / timestamps / `metadata.user` JSON pretty-printed)
  3. Provenance (events list)
  4. Blocks (count + first-N preview; on `b` toggle to full list paginated)
- Chunk view layout:
  1. Header (chunk_id, doc_id, doc_path, heading_path, chunker_version)
  2. Source spans (rendered as W3C fragment URIs per design §0 Q3)
  3. Text (chunk full text in a scrollable area)
  4. Embeddings (model_id, dims, embedding_id list — empty if none yet)
- Key bindings:
  - `j` / `k` → scroll
  - `c` → collapse / expand currently focused section (focus is implicit by current scroll position; v1 may simplify by toggling all sections)
  - `Esc` → return to previous pane (Library or Search)
  - `Enter` → no-op (Inspect is terminal — no editor jump here; users use Search pane for jump)
- Loading: while `kb-app::inspect_doc` or `inspect_chunk` runs, show "loading…". On error, popup with hint.
- Renders must conform to wire schemas `doc_summary.v1` (subset for header) and `chunk_inspection.v1`.

## Storage / wire effects

- Reads only.

## Test plan

| kind | description | fixture / data |
|------|-------------|----------------|
| unit | switching to InspectTarget::Doc triggers `kb-app::inspect_doc` once | inline mock |
| unit | scroll bounded by content height | inline |
| unit | collapse toggle via `c` flips state | inline |
| snapshot | doc-view rendered for fixture stable | TestBackend + fixture |
| snapshot | chunk-view rendered for fixture stable | TestBackend + fixture |

All tests under `cargo test -p kb-tui inspect`.

## Definition of Done

- [ ] `cargo check -p kb-tui` passes
- [ ] `cargo test -p kb-tui inspect` passes
- [ ] No imports outside Allowed dependencies
- [ ] Manual smoke: inspect a doc with multiple chunks, scroll, return to library
- [ ] PR links design §3.5, §2.5, §2.6

## Out of scope

- Editing documents.
- Re-ingestion buttons.
- Embedding inspection beyond listing model identity.
- Side-by-side diff with previous doc version.

## Risks / notes

- Long chunk text (~10 KB) rendering can be slow if re-rendered every frame; cache wrapped lines and re-wrap only on resize.
- Pretty-printing `metadata.user` as JSON: prefer `serde_json::to_string_pretty`. Indentation = 2 spaces.
- Korean text in metadata: ensure `unicode-width`-aware wrapping.
