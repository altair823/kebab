---
phase: P9
component: kb-desktop (Tauri)
task_id: p9-5
title: "Tauri desktop app: backend commands wrapping kb-app + multimodal source viewer"
status: planned
depends_on: [p9-1, p9-2, p9-3, p9-4]
unblocks: []
contract_source: ../../docs/superpowers/specs/2026-04-27-kb-final-form-design.md
contract_sections: [§16.3 desktop epic (tasks/phase-9-ui.md), §1 ask/search scenes, §2 wire schemas v1, §8 module boundaries]
---

# p9-5 — Tauri desktop app

## Goal

Stand up a Tauri 2.x app (`kb-desktop` crate as backend, `kb-desktop-frontend/` as web assets) whose Tauri commands wrap `kb-app` 1:1. The frontend renders multimodal source viewers (Markdown render, PDF page viewer, image viewer with region overlay, audio player with seek). Citation clicks route to the appropriate viewer.

## Why now / why this size

Last task. Combines all backend phases into a single user-facing surface. Strict policy: backend commands are thin wrappers over `kb-app`; no new business logic.

## Allowed dependencies

- backend (`kb-desktop`):
  - `kb-core`
  - `kb-config`
  - `kb-app`
  - `tauri = "2"` + `tauri-build`
  - `serde`, `serde_json`
  - `tracing`
  - `thiserror`
- frontend (`kb-desktop-frontend/`): vanilla TypeScript + Vite (default; user may swap to Svelte/Solid in a follow-up).
  - PDF rendering: `pdfjs-dist`
  - Markdown rendering: `marked` + `dompurify`
  - Audio: HTML `<audio>` with custom segment overlay
  - Image: HTML `<img>` with absolute-positioned bounding box overlay

## Forbidden dependencies

- `kb-source-fs`, `kb-parse-*`, `kb-normalize`, `kb-chunk`, `kb-store-*`, `kb-embed*`, `kb-search`, `kb-llm*`, `kb-rag` (UI must go through `kb-app` only — design §8).
- **No native PDF render backend** (no `pdfium`, no `mupdf`, no `poppler`). PDF rendering lives entirely in the frontend (`pdfjs-dist`). Adding any of these would (a) bloat the bundle 100+ MB, (b) require frozen-design amendment, and (c) double the path-containment surface.

## Inputs

| input | type | source |
|-------|------|--------|
| Tauri commands | invoked from frontend | user clicks |
| `kb-config::Config` | runtime | env / file |
| user file system (read-only) | for source viewers | OS |

## Outputs

| output | type | downstream |
|--------|------|------------|
| Tauri app bundle (macOS dmg, Linux AppImage, Windows msi) | distribution | user |
| Tauri commands return wire-schema-v1 JSON | IPC | frontend |

## Public surface (signatures only — no new types)

```rust
// Tauri command surface (one per kb-app facade method, plus source viewers)
#[tauri::command] fn cmd_init(force: bool) -> Result<()>;
#[tauri::command] fn cmd_ingest(scope_json: serde_json::Value, summary_only: bool) -> Result<serde_json::Value /* IngestReportWireV1 */>;
#[tauri::command] fn cmd_list_docs(filter_json: serde_json::Value) -> Result<Vec<serde_json::Value /* DocSummaryWireV1 */>>;
#[tauri::command] fn cmd_inspect_doc(id: String) -> Result<serde_json::Value /* CanonicalDocument as wire */>;
#[tauri::command] fn cmd_inspect_chunk(id: String) -> Result<serde_json::Value /* ChunkInspectionWireV1 */>;
#[tauri::command] fn cmd_search(query_json: serde_json::Value) -> Result<Vec<serde_json::Value /* SearchHitWireV1 */>>;
#[tauri::command] fn cmd_ask(query: String, opts_json: serde_json::Value) -> Result<serde_json::Value /* AnswerWireV1 */>;
#[tauri::command] fn cmd_doctor() -> Result<serde_json::Value /* DoctorReportWireV1 */>;

// Source viewers — file IO restricted to workspace_root, raw-bytes only.
// Rendering happens 100% in the frontend (pdfjs / <img> / <audio>); backend has NO native render dependency.
#[tauri::command] fn cmd_read_markdown(path: String) -> Result<String>;       // utf-8 Markdown source
#[tauri::command] fn cmd_read_file_bytes(path: String) -> Result<Vec<u8>>;    // raw bytes for PDF / image / audio
```

(All commands convert internal `kb-core` types to wire-schema-v1 JSON before returning.)

## Behavior contract

- Backend bootstraps `tracing` to a file under `~/.local/state/kb/logs/` and a Tauri plugin loads/saves window state.
- Every Tauri command performs **path containment** for source viewers: resolves `path` against `config.workspace.root`, rejects (`anyhow::Error`) any path outside.
- Layout (frontend): left = Library + Search + Ask tabs; right = Source viewer keyed by current citation.
- Citation routing in the frontend (clicks on `[#N]` markers or hit rows). All rendering is frontend-side; backend serves raw bytes only.
  - `Citation::Line { path, start, end }` → `cmd_read_markdown(path)`, render with `marked`, scroll + highlight lines `[start, end]`.
  - `Citation::Page { path, page }` → `cmd_read_file_bytes(path)` → pass `Uint8Array` to `pdfjs-dist` (`getDocument({ data })`), navigate to `page`. No backend PDF render; no `pdfium` native dep.
  - `Citation::Region { path, x, y, w, h }` → `cmd_read_file_bytes(path)` → blob URL → `<img>` + absolute-positioned overlay at `(x, y, w, h)`.
  - `Citation::Caption { path, model }` → same as Region but no overlay; caption banner shows `model`.
  - `Citation::Time { path, start_ms, end_ms }` → `cmd_read_file_bytes(path)` → blob URL → `<audio src=...>` seeked to `start_ms / 1000`, with a timeline marker spanning `[start_ms, end_ms]`.
- Streaming `kb ask`: backend command `cmd_ask` returns the buffered Answer (per §0 Q5: pipe/JSON mode buffers). For real-time streaming in the desktop, expose a separate `cmd_ask_stream` event channel via Tauri's `Window::emit("kb://ask-token", payload)`. (Implementation can be deferred to a follow-up; v1 of the desktop accepts buffered.)
- All backend errors mapped to a `String` message with structure `{ "error": msg, "hint": Option<msg> }`.
- Frontend respects light/dark per OS theme (Tauri supplies the API).
- No telemetry. No automatic update channel for v1 (manual download).

## Storage / wire effects

- Reads via `kb-app` (which reads/writes via SQLite + LanceDB).
- Reads workspace files directly for source viewers (path-contained).
- Writes nothing outside what `kb-app` writes.
- Wire JSON between backend and frontend uses schema v1 strictly. The frontend MUST validate `schema_version` strings on every IPC return and warn (or upgrade-gate) when `v1 != current`.

## Test plan

| kind | description | fixture / data |
|------|-------------|----------------|
| unit (backend) | each command wraps the corresponding `kb-app` function and serializes via wire schema | inline mocks |
| unit (backend) | `cmd_read_markdown` rejects paths outside workspace | tmp config |
| unit (backend) | `cmd_read_file_bytes` rejects paths outside workspace incl. `..`, absolute path, symlink-out | tmp config + traversal vectors |
| unit (backend) | `cmd_read_file_bytes` returns identical bytes to `std::fs::read` for an in-workspace file | tmp config |
| unit (backend) | citation route in deserialized wire JSON resolves to expected viewer kind (string match) | inline |
| smoke (frontend, optional in this task) | Vitest test that mounts the Library tab, calls a mocked `cmd_list_docs`, renders 1 row | minimal |
| manual | full-stack smoke against a real ingested workspace (Markdown + 1 PDF + 1 image + 1 audio); each citation jumps correctly | manual checklist |

Backend tests under `cargo test -p kb-desktop`. Frontend tests are bonus and not gated by this task's DoD.

## Definition of Done

- [ ] `cargo check -p kb-desktop` passes
- [ ] `cargo test -p kb-desktop` passes
- [ ] `pnpm --filter kb-desktop-frontend build` produces a static asset bundle Tauri can package
- [ ] `tauri build` produces an unsigned dmg on macOS in CI (signed/notarized are out of scope)
- [ ] Each Tauri command returns wire-schema-v1 JSON; frontend asserts `schema_version`
- [ ] No imports outside Allowed dependencies (backend)
- [ ] PR links design §16.3 epic, §1, §2 wire schemas, §8

## Out of scope

- Code signing & notarization.
- Auto-update channel.
- Multi-window UI.
- Drag-and-drop ingestion (P+).
- Workspace selection UI for multi-workspace (multi-workspace itself is out of scope per design §0).
- Streaming `ask` event channel (deferred; buffered v1 acceptable).

## Risks / notes

- Tauri 2 frontend stack churn: lock pinned versions in `package.json` and `tauri.conf.json` to avoid CI drift.
- Path containment is the desktop's most security-sensitive surface; tests must include path traversal vectors (`..`, symlinks, absolute paths).
- PDF rendering via `pdfjs-dist` is heavy (~2 MB worker); lazy-load on first PDF citation. The trade-off vs a native render backend (e.g., `pdfium` ~150 MB binary, code-signing pain) is heavily one-sided; v1 stays on `pdfjs-dist`.
- Audio formats vary; rely on the browser engine's HTML audio decoder (WebKit on macOS supports `.m4a`, `.mp3`; mileage varies on `.flac`/`.ogg`).
- Wide Tauri command surface tempts business-logic creep; CI must enforce that no `kb-rag` / `kb-search` / store crate appears in `kb-desktop`'s `cargo tree`.
