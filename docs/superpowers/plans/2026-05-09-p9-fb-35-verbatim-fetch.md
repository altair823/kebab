# p9-fb-35 — Verbatim Fetch Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `kebab fetch chunk|doc|span` subcommand + `mcp__kebab__fetch` MCP tool. Returns verbatim text from `chunks.text` / `CanonicalDocument` (normalized markdown only — raw bytes not exposed). Wire = `fetch_result.v1` (kind discriminator). Reuses fb-32 staleness + fb-34 budget patterns.

**Architecture:** New `App::fetch(query: FetchQuery, opts: FetchOpts) -> Result<FetchResult>` facade. Three modes share one entry: `Chunk(ChunkId)` (with optional `--context N` ordinal-based ±N chunks), `Doc(DocumentId)` (markdown serialization of CanonicalDocument), `Span { doc_id, line_start, line_end }` (line-range slice; PDF/audio rejected as `span_not_supported`). fb-34 `StructuredError` wrapper preserves typed `error.v1.code` through anyhow. Single discriminated wire shape.

**Tech Stack:** Rust 2024, serde, JSON Schema (fetch_result.v1), no new deps.

**Spec:** `docs/superpowers/specs/2026-05-09-p9-fb-35-verbatim-fetch-design.md`

---

## File Structure

| File | Responsibility | Action |
|------|----------------|--------|
| `crates/kebab-core/src/fetch.rs` | NEW — `FetchQuery`, `FetchOpts`, `FetchResult`, `FetchKind` | create |
| `crates/kebab-core/src/lib.rs` | `mod fetch; pub use fetch::*;` | modify |
| `crates/kebab-app/src/fetch.rs` | NEW — `App::fetch` impl, mode dispatch, markdown serialization helper, error construction | create |
| `crates/kebab-app/src/app.rs` | Add `pub fn fetch(...)` method on `App` (delegates to `crate::fetch::run`) | modify |
| `crates/kebab-app/src/lib.rs` | `mod fetch;` + `pub use fetch::fetch_with_config;` | modify |
| `crates/kebab-app/src/error_wire.rs` | Add `chunk_not_found` / `doc_not_found` / `span_not_supported` / `invalid_input` to known codes (documentation comment only — codes already pass through `StructuredError`) | modify |
| `crates/kebab-cli/src/main.rs` | New `Cmd::Fetch` enum variant + `FetchWhat` subcommand + dispatch + plain renderer | modify |
| `crates/kebab-cli/src/wire.rs` | New `wire_fetch_result(&FetchResult) -> Value` | modify |
| `crates/kebab-mcp/src/tools/fetch.rs` | NEW — `FetchInput` + `handle()` | create |
| `crates/kebab-mcp/src/lib.rs` | Register `kebab__fetch` tool | modify |
| `docs/wire-schema/v1/fetch_result.schema.json` | NEW | create |
| `crates/kebab-app/tests/fetch_integration.rs` | NEW — chunk/doc/span unit + error paths | create |
| `crates/kebab-app/tests/common/mod.rs` | Possibly extend — ingest + lookup helpers | modify |
| `crates/kebab-cli/tests/wire_fetch.rs` | NEW — wire shape + plain output + exit codes | create |
| `crates/kebab-cli/tests/common/mod.rs` | Possibly extend — `run_fetch_with_args` helper | modify |
| `crates/kebab-mcp/tests/tools_call_fetch.rs` | NEW — 3 modes + invalid_input | create |
| `README.md` | `kebab fetch chunk|doc|span` row in 명령 table | modify |
| `docs/SMOKE.md` | Fetch walkthrough paragraph | modify |
| `tasks/p9/p9-fb-35-verbatim-fetch.md` | Status flip + design/plan links | modify |
| `tasks/INDEX.md` | fb-35 row → ✅ | modify |
| `integrations/claude-code/kebab/SKILL.md` | New `mcp__kebab__fetch` row + recipe | modify |

---

## Pre-flight

- [ ] **Step 0.1: Branch off main**

```bash
git checkout main
git pull
git checkout -b feat/fb-35-verbatim-fetch
```

- [ ] **Step 0.2: Confirm spec branch reachable**

```bash
git log --oneline spec/fb-35-verbatim-fetch -1
```

Expected: `4eda9c3 spec(fb-35): verbatim fetch — design`. If spec PR not yet merged into main, `git merge spec/fb-35-verbatim-fetch` so the spec doc lives on this branch.

---

## Task 1: Domain types in kebab-core

**Files:**
- Create: `crates/kebab-core/src/fetch.rs`
- Modify: `crates/kebab-core/src/lib.rs`

- [ ] **Step 1.1: Failing test**

Append to `crates/kebab-core/src/fetch.rs` (will create the file):

```rust
//! p9-fb-35 verbatim fetch domain types.
//!
//! Three modes (chunk / doc / span) carried by [`FetchQuery`]; one
//! response shape ([`FetchResult`]) discriminated by [`FetchKind`].
//! All types are `Serialize` so the CLI / MCP wire layers can hand
//! them straight through `serde_json::to_value`.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::asset::WorkspacePath;
use crate::ids::{ChunkId, DocumentId};
use crate::traits::Chunk;

#[derive(Clone, Debug)]
pub enum FetchQuery {
    Chunk(ChunkId),
    Doc(DocumentId),
    Span {
        doc_id: DocumentId,
        line_start: u32,
        line_end: u32,
    },
}

#[derive(Clone, Debug, Default)]
pub struct FetchOpts {
    /// chunk mode only: ±N chunks. None = no surrounding context.
    pub context: Option<u32>,
    /// doc / span mode only: chars/4 budget. None = no cap.
    pub max_tokens: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FetchKind {
    Chunk,
    Doc,
    Span,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FetchResult {
    pub kind: FetchKind,
    pub doc_id: DocumentId,
    pub doc_path: WorkspacePath,
    #[serde(with = "time::serde::rfc3339")]
    pub indexed_at: OffsetDateTime,
    pub stale: bool,
    // chunk mode payloads
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunk: Option<Chunk>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub context_before: Vec<Chunk>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub context_after: Vec<Chunk>,
    // doc / span payloads
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_start: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_end: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_end: Option<u32>,
    pub truncated: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fetch_opts_default_is_all_none() {
        let o = FetchOpts::default();
        assert!(o.context.is_none());
        assert!(o.max_tokens.is_none());
    }

    #[test]
    fn fetch_kind_serializes_snake_case() {
        let v = serde_json::to_value(FetchKind::Chunk).unwrap();
        assert_eq!(v, serde_json::json!("chunk"));
        let v = serde_json::to_value(FetchKind::Span).unwrap();
        assert_eq!(v, serde_json::json!("span"));
    }
}
```

- [ ] **Step 1.2: Wire module**

Edit `crates/kebab-core/src/lib.rs`. Find the existing `mod` declarations (search for `pub mod search;` or similar):

```bash
grep -n "^pub mod\|^mod " crates/kebab-core/src/lib.rs | head -10
```

Add at the same level:

```rust
pub mod fetch;
```

And re-export the public types — append to the `pub use` block near the top:

```rust
pub use fetch::{FetchKind, FetchOpts, FetchQuery, FetchResult};
```

- [ ] **Step 1.3: Run tests — verify pass**

```bash
cargo test -p kebab-core --lib fetch::tests
```

Expected: 2 PASS.

- [ ] **Step 1.4: Commit**

```bash
git add crates/kebab-core/src/fetch.rs crates/kebab-core/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(core): FetchQuery / FetchOpts / FetchResult / FetchKind (fb-35)

Domain types for `kebab fetch` 3 modes (chunk / doc / span). All
types Serialize so wire layers hand them through serde_json
directly. FetchKind is snake_case-renamed to match the wire
discriminator literal in fetch_result.v1.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: Wire schema — `fetch_result.v1`

**Files:**
- Create: `docs/wire-schema/v1/fetch_result.schema.json`

- [ ] **Step 2.1: Write the schema**

Create `docs/wire-schema/v1/fetch_result.schema.json`:

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://kb.local/wire/v1/fetch_result.schema.json",
  "title": "FetchResult v1",
  "description": "Verbatim text fetch from the indexed corpus. Discriminated by `kind`. All text is normalized markdown sourced from `CanonicalDocument` / `chunks.text` — original raw bytes are not exposed. PDF / audio span fetch returns `error.v1.code = span_not_supported`.",
  "type": "object",
  "required": ["schema_version", "kind", "doc_id", "doc_path", "indexed_at", "stale", "truncated"],
  "properties": {
    "schema_version": { "const": "fetch_result.v1" },
    "kind":           { "enum": ["chunk", "doc", "span"] },
    "doc_id":         { "type": "string" },
    "doc_path":       { "type": "string" },
    "indexed_at":     { "type": "string", "format": "date-time", "description": "fb-32 documents.updated_at" },
    "stale":          { "type": "boolean", "description": "fb-32 staleness flag against config.search.stale_threshold_days" },
    "chunk":          { "type": "object", "description": "kind=chunk: target chunk_inspection.v1 payload" },
    "context_before": { "type": "array", "description": "kind=chunk: --context N preceding chunks (ordinal-sorted)" },
    "context_after":  { "type": "array", "description": "kind=chunk: --context N following chunks (ordinal-sorted)" },
    "text":           { "type": "string", "description": "kind=doc/span: markdown text (truncated if budget tripped)" },
    "line_start":     { "type": ["integer", "null"], "minimum": 1, "description": "kind=span: requested start line (1-based)" },
    "line_end":       { "type": ["integer", "null"], "minimum": 1, "description": "kind=span: requested end line (1-based, inclusive)" },
    "effective_end":  { "type": ["integer", "null"], "minimum": 1, "description": "kind=span: actual emitted end line after budget truncation" },
    "truncated":      { "type": "boolean", "description": "kind=doc/span: budget forced text truncation. Always false for chunk." }
  }
}
```

- [ ] **Step 2.2: Validate JSON**

```bash
python3 -c "import json; json.load(open('docs/wire-schema/v1/fetch_result.schema.json'))"
```

Expected: silent success.

- [ ] **Step 2.3: Commit**

```bash
git add docs/wire-schema/v1/fetch_result.schema.json
git commit -m "$(cat <<'EOF'
feat(wire): fetch_result.v1 schema (fb-35)

Discriminated by kind (chunk / doc / span). Per-kind required
fields enforced by description prose at v1 stub stage; future
phase may add JSON Schema conditional validation.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: `App::fetch` chunk mode + markdown serializer

**Files:**
- Create: `crates/kebab-app/src/fetch.rs`
- Modify: `crates/kebab-app/src/app.rs`
- Modify: `crates/kebab-app/src/lib.rs`
- Create: `crates/kebab-app/tests/fetch_integration.rs`

- [ ] **Step 3.1: Write failing chunk-mode test**

Create `crates/kebab-app/tests/fetch_integration.rs`:

```rust
//! p9-fb-35 App::fetch integration tests.

mod common;

use kebab_app::App;
use kebab_core::{FetchKind, FetchOpts, FetchQuery};

fn open(env: &common::TestEnv) -> App {
    env.app()
}

#[test]
fn fetch_chunk_returns_target_only_when_no_context() {
    let env = common::TestEnv::new();
    common::ingest_md(&env, "a.md", "# Title\n\nFirst paragraph.\n\n## Section\n\nSecond.\n");
    let app = open(&env);

    // Find a chunk via search to obtain its id.
    let q = kebab_core::SearchQuery {
        text: "First".to_string(),
        mode: kebab_core::SearchMode::Lexical,
        k: 1,
        filters: kebab_core::SearchFilters::default(),
    };
    let hits = app.search(q).unwrap();
    let chunk_id = hits[0].chunk_id.clone();

    let result = app
        .fetch(FetchQuery::Chunk(chunk_id), FetchOpts::default())
        .unwrap();
    assert_eq!(result.kind, FetchKind::Chunk);
    assert!(result.chunk.is_some(), "target chunk populated");
    assert!(result.context_before.is_empty());
    assert!(result.context_after.is_empty());
    assert!(!result.truncated);
}
```

- [ ] **Step 3.2: Run test (verify failure)**

```bash
cargo test -p kebab-app --test fetch_integration fetch_chunk_returns_target_only_when_no_context
```

Expected: FAIL — `App::fetch` does not exist.

- [ ] **Step 3.3: Implement minimal `App::fetch` (chunk mode only)**

Create `crates/kebab-app/src/fetch.rs`:

```rust
//! p9-fb-35 verbatim fetch implementation.

use anyhow::Result;
use time::OffsetDateTime;

use kebab_core::{
    Chunk, ChunkId, DocumentId, DocumentStore, FetchKind, FetchOpts, FetchQuery, FetchResult,
    Block, CanonicalDocument,
};

use crate::App;
use crate::error_wire::{ErrorV1, StructuredError};
use crate::staleness::compute_stale;

impl App {
    /// p9-fb-35: verbatim fetch facade. Returns text from
    /// `chunks.text` / `CanonicalDocument` based on the requested
    /// mode. Errors surface as `StructuredError(ErrorV1)` with one
    /// of `chunk_not_found` / `doc_not_found` / `span_not_supported`.
    pub fn fetch(&self, query: FetchQuery, opts: FetchOpts) -> Result<FetchResult> {
        match query {
            FetchQuery::Chunk(id) => fetch_chunk(self, id, opts),
            FetchQuery::Doc(id) => fetch_doc(self, id, opts),
            FetchQuery::Span { doc_id, line_start, line_end } => {
                fetch_span(self, doc_id, line_start, line_end, opts)
            }
        }
    }
}

fn fetch_chunk(app: &App, id: ChunkId, opts: FetchOpts) -> Result<FetchResult> {
    let target = <kebab_store_sqlite::SqliteStore as DocumentStore>::get_chunk(&app.sqlite, &id)?
        .ok_or_else(|| {
            StructuredError(ErrorV1 {
                schema_version: "error.v1".to_string(),
                code: "chunk_not_found".to_string(),
                message: format!("chunk_id '{}' not found", id.0),
                details: serde_json::Value::Null,
                hint: None,
            })
        })?;

    let doc_id = target.doc_id.clone();
    let doc = <kebab_store_sqlite::SqliteStore as DocumentStore>::get_document(&app.sqlite, &doc_id)?
        .ok_or_else(|| {
            StructuredError(ErrorV1 {
                schema_version: "error.v1".to_string(),
                code: "doc_not_found".to_string(),
                message: format!(
                    "doc_id '{}' (parent of chunk '{}') not found",
                    doc_id.0, id.0
                ),
                details: serde_json::Value::Null,
                hint: None,
            })
        })?;

    let (context_before, context_after) = match opts.context {
        Some(n) if n > 0 => surrounding_chunks(app, &doc_id, &id, n)?,
        _ => (Vec::new(), Vec::new()),
    };

    let now = OffsetDateTime::now_utc();
    let stale = compute_stale(
        doc_metadata_updated_at(&doc),
        now,
        app.config.search.stale_threshold_days,
    );

    Ok(FetchResult {
        kind: FetchKind::Chunk,
        doc_id: doc.doc_id.clone(),
        doc_path: doc.workspace_path.clone(),
        indexed_at: doc_metadata_updated_at(&doc),
        stale,
        chunk: Some(target),
        context_before,
        context_after,
        text: None,
        line_start: None,
        line_end: None,
        effective_end: None,
        truncated: false,
    })
}

fn fetch_doc(_app: &App, _id: DocumentId, _opts: FetchOpts) -> Result<FetchResult> {
    // Implemented in Task 4.
    anyhow::bail!("fetch_doc not yet implemented")
}

fn fetch_span(
    _app: &App,
    _id: DocumentId,
    _line_start: u32,
    _line_end: u32,
    _opts: FetchOpts,
) -> Result<FetchResult> {
    // Implemented in Task 5.
    anyhow::bail!("fetch_span not yet implemented")
}

/// p9-fb-35: list chunks for a document in ordinal order, returning
/// the slice `[target_idx - n .. target_idx + n + 1]` (clamped).
/// Returns `(before, after)` with the target itself excluded.
fn surrounding_chunks(
    app: &App,
    doc_id: &DocumentId,
    target: &ChunkId,
    n: u32,
) -> Result<(Vec<Chunk>, Vec<Chunk>)> {
    let chunks = list_chunks_in_order(app, doc_id)?;
    let target_idx = chunks
        .iter()
        .position(|c| c.id == *target)
        .ok_or_else(|| anyhow::anyhow!("chunk not found in doc chunk list"))?;
    let n = n as usize;
    let lo = target_idx.saturating_sub(n);
    let hi = (target_idx + n + 1).min(chunks.len());
    let before: Vec<Chunk> = chunks[lo..target_idx].to_vec();
    let after: Vec<Chunk> = chunks[target_idx + 1..hi].to_vec();
    Ok((before, after))
}

/// p9-fb-35: ordinal-ordered chunk list for one document. Walks the
/// store, sorted by `chunks.created_at` (matches insertion order)
/// because the chunks table has no explicit ordinal column.
fn list_chunks_in_order(app: &App, doc_id: &DocumentId) -> Result<Vec<Chunk>> {
    use rusqlite::params;
    let conn = app.sqlite.lock_conn();
    let mut stmt = conn.prepare(
        "SELECT chunk_id FROM chunks WHERE doc_id = ? ORDER BY created_at ASC, chunk_id ASC",
    )?;
    let ids: Vec<String> = stmt
        .query_map(params![doc_id.0], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    drop(stmt);
    drop(conn);
    let mut out: Vec<Chunk> = Vec::with_capacity(ids.len());
    for id in ids {
        let cid = ChunkId(id);
        if let Some(chunk) = <kebab_store_sqlite::SqliteStore as DocumentStore>::get_chunk(
            &app.sqlite,
            &cid,
        )? {
            out.push(chunk);
        }
    }
    Ok(out)
}

fn doc_metadata_updated_at(doc: &CanonicalDocument) -> OffsetDateTime {
    doc.metadata.updated_at
}

/// p9-fb-35: serialize a `CanonicalDocument` back to markdown.
/// Loses round-trip fidelity for inline-styled spans (Strong /
/// Emph children become flat text); good enough for an agent
/// looking for verbatim context. Tests Task 4.
pub(crate) fn fmt_canonical_to_markdown(doc: &CanonicalDocument) -> String {
    let mut out = String::with_capacity(1024);
    for (i, block) in doc.blocks.iter().enumerate() {
        if i > 0 {
            out.push_str("\n\n");
        }
        match block {
            Block::Heading(h) => {
                for _ in 0..h.level.max(1).min(6) {
                    out.push('#');
                }
                out.push(' ');
                out.push_str(&h.text);
            }
            Block::Paragraph(t) | Block::Quote(t) => out.push_str(&t.text),
            Block::List(l) => {
                for (idx, item) in l.items.iter().enumerate() {
                    if idx > 0 {
                        out.push('\n');
                    }
                    if l.ordered {
                        out.push_str(&format!("{}. {}", idx + 1, item.text));
                    } else {
                        out.push_str(&format!("- {}", item.text));
                    }
                }
            }
            Block::Code(c) => {
                out.push_str("```");
                if let Some(lang) = &c.lang {
                    out.push_str(lang);
                }
                out.push('\n');
                out.push_str(&c.code);
                if !c.code.ends_with('\n') {
                    out.push('\n');
                }
                out.push_str("```");
            }
            Block::Table(t) => {
                let header = t.headers.join(" | ");
                out.push_str(&header);
                out.push('\n');
                out.push_str(&"---|".repeat(t.headers.len()));
                for row in &t.rows {
                    out.push('\n');
                    out.push_str(&row.join(" | "));
                }
            }
            Block::ImageRef(img) => {
                out.push_str(&format!("![{}]({})", img.alt, img.src));
            }
            Block::AudioRef(_a) => {
                out.push_str("(audio reference)");
            }
        }
    }
    out
}

/// p9-fb-35: free function entry for CLI / MCP.
pub fn fetch_with_config(
    config: kebab_config::Config,
    query: FetchQuery,
    opts: FetchOpts,
) -> Result<FetchResult> {
    App::open_with_config(config)?.fetch(query, opts)
}
```

The `app.sqlite` field access depends on visibility. Check:

```bash
grep -n "sqlite:\|pub(crate) sqlite\|sqlite.lock_conn\|pub fn lock_conn" crates/kebab-app/src/app.rs crates/kebab-store-sqlite/src/store.rs | head -10
```

If `app.sqlite` is private, expose `pub(crate)` or add an accessor. If `lock_conn` is private to `kebab-store-sqlite`, expose `pub` or use a public method. Adapt as needed.

If the `Chunk` struct's `id` field is named `chunk_id` instead of `id`, adapt the comparison `c.id == *target` accordingly. Verify:

```bash
grep -A 3 "^pub struct Chunk\b" crates/kebab-core/src/traits.rs
```

- [ ] **Step 3.4: Wire fetch module**

Edit `crates/kebab-app/src/lib.rs`. Add near other `mod` declarations:

```rust
pub mod fetch;
pub use fetch::fetch_with_config;
```

Edit `crates/kebab-app/src/app.rs` — no change needed; `App::fetch` is in the `impl App { ... }` block in `fetch.rs`.

- [ ] **Step 3.5: Run test (verify pass)**

```bash
cargo test -p kebab-app --test fetch_integration fetch_chunk_returns_target_only_when_no_context
```

Expected: PASS.

If `common::TestEnv::new()` / `ingest_md` don't exist with those exact names, look at what `tests/common/mod.rs` provides (fb-32 / fb-34 added several helpers):

```bash
grep -n "pub fn\|pub struct" crates/kebab-app/tests/common/mod.rs | head -10
```

Adapt the test to whatever scaffold exists.

- [ ] **Step 3.6: Add `--context` test**

Append to `crates/kebab-app/tests/fetch_integration.rs`:

```rust
#[test]
fn fetch_chunk_with_context_returns_neighbors() {
    let env = common::TestEnv::new();
    let body = "# H1\n\nA1\n\n# H2\n\nA2\n\n# H3\n\nA3\n\n# H4\n\nA4\n\n# H5\n\nA5\n";
    common::ingest_md(&env, "multi.md", body);
    let app = env.app();

    // Find the middle chunk (search for "A3").
    let q = kebab_core::SearchQuery {
        text: "A3".to_string(),
        mode: kebab_core::SearchMode::Lexical,
        k: 1,
        filters: kebab_core::SearchFilters::default(),
    };
    let hits = app.search(q).unwrap();
    let chunk_id = hits[0].chunk_id.clone();

    let result = app
        .fetch(
            FetchQuery::Chunk(chunk_id),
            FetchOpts {
                context: Some(2),
                max_tokens: None,
            },
        )
        .unwrap();
    assert_eq!(result.kind, FetchKind::Chunk);
    assert!(result.chunk.is_some());
    // We may not have exactly 2 each side depending on chunker, but
    // total context (before+after) should be at most 4 and at least 1.
    let total = result.context_before.len() + result.context_after.len();
    assert!(total >= 1, "at least one neighbor expected");
    assert!(total <= 4, "context capped at ±2 ⇒ max 4 neighbors");
}

#[test]
fn fetch_chunk_unknown_id_returns_chunk_not_found() {
    let env = common::TestEnv::new();
    let app = env.app();
    let err = app
        .fetch(
            FetchQuery::Chunk(kebab_core::ChunkId("nonexistent-id".to_string())),
            FetchOpts::default(),
        )
        .unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("chunk_not_found") || msg.contains("nonexistent-id"),
        "expected chunk_not_found error, got: {msg}"
    );
}
```

- [ ] **Step 3.7: Run tests (verify pass)**

```bash
cargo test -p kebab-app --test fetch_integration
```

Expected: 3 PASS.

- [ ] **Step 3.8: Commit**

```bash
git add crates/kebab-app/src/fetch.rs crates/kebab-app/src/lib.rs crates/kebab-app/tests/fetch_integration.rs
git commit -m "$(cat <<'EOF'
feat(app): App::fetch chunk mode + markdown serializer (fb-35)

Chunk mode + ±N context. doc / span modes return placeholder
errors (filled by subsequent tasks). fmt_canonical_to_markdown
helper introduced now since doc mode (Task 4) consumes it.
Errors are typed StructuredError so classify preserves
chunk_not_found / doc_not_found through the wire layer.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: `App::fetch` doc mode + budget

**Files:**
- Modify: `crates/kebab-app/src/fetch.rs`
- Modify: `crates/kebab-app/tests/fetch_integration.rs`

- [ ] **Step 4.1: Failing test**

Append to `crates/kebab-app/tests/fetch_integration.rs`:

```rust
#[test]
fn fetch_doc_returns_serialized_markdown() {
    let env = common::TestEnv::new();
    let body = "# Heading One\n\nFirst paragraph.\n\n## Sub\n\nSecond.\n";
    common::ingest_md(&env, "doc.md", body);
    let app = env.app();

    let doc_summary = common::list_docs(&app);
    let doc_id = doc_summary[0].doc_id.clone();

    let result = app
        .fetch(FetchQuery::Doc(doc_id), FetchOpts::default())
        .unwrap();
    assert_eq!(result.kind, FetchKind::Doc);
    let text = result.text.expect("doc text");
    assert!(text.contains("Heading One"), "doc text contains heading: {text:?}");
    assert!(text.contains("First paragraph"), "doc text contains body");
    assert!(!result.truncated);
}

#[test]
fn fetch_doc_unknown_id_returns_doc_not_found() {
    let env = common::TestEnv::new();
    let app = env.app();
    let err = app
        .fetch(
            FetchQuery::Doc(kebab_core::DocumentId("nonexistent-doc".to_string())),
            FetchOpts::default(),
        )
        .unwrap_err();
    assert!(err.to_string().contains("doc_not_found"));
}

#[test]
fn fetch_doc_with_max_tokens_truncates() {
    let env = common::TestEnv::new();
    // Long doc — repeated paragraphs.
    let p = "Lorem ipsum dolor sit amet consectetur adipiscing elit. ".repeat(20);
    let body = format!("# Big\n\n{p}\n");
    common::ingest_md(&env, "big.md", &body);
    let app = env.app();
    let doc_id = common::list_docs(&app)[0].doc_id.clone();

    let result = app
        .fetch(
            FetchQuery::Doc(doc_id),
            FetchOpts {
                context: None,
                max_tokens: Some(20), // ~80 chars
            },
        )
        .unwrap();
    assert!(result.truncated);
    let text = result.text.expect("doc text");
    assert!(text.len() <= 100, "trimmed text length {}", text.len());
}
```

If `common::list_docs` doesn't exist, look at how other tests find a doc id (e.g. via `app.list_docs(...)`):

```bash
grep -n "list_docs\|list_documents" crates/kebab-app/src/lib.rs crates/kebab-app/src/app.rs | head -5
```

Adapt — possibly inline the listing call.

- [ ] **Step 4.2: Run tests (verify failure)**

```bash
cargo test -p kebab-app --test fetch_integration fetch_doc
```

Expected: FAIL — `fetch_doc not yet implemented`.

- [ ] **Step 4.3: Implement doc mode**

Replace the placeholder `fetch_doc` in `crates/kebab-app/src/fetch.rs` with:

```rust
fn fetch_doc(app: &App, id: DocumentId, opts: FetchOpts) -> Result<FetchResult> {
    let doc = <kebab_store_sqlite::SqliteStore as DocumentStore>::get_document(&app.sqlite, &id)?
        .ok_or_else(|| {
            StructuredError(ErrorV1 {
                schema_version: "error.v1".to_string(),
                code: "doc_not_found".to_string(),
                message: format!("doc_id '{}' not found", id.0),
                details: serde_json::Value::Null,
                hint: None,
            })
        })?;

    let mut text = fmt_canonical_to_markdown(&doc);
    let mut truncated = false;
    if let Some(max_tokens) = opts.max_tokens {
        let max_chars = max_tokens.saturating_mul(4);
        if text.chars().count() > max_chars {
            text = trim_to_chars(&text, max_chars);
            truncated = true;
        }
    }

    let now = OffsetDateTime::now_utc();
    let stale = compute_stale(
        doc_metadata_updated_at(&doc),
        now,
        app.config.search.stale_threshold_days,
    );

    Ok(FetchResult {
        kind: FetchKind::Doc,
        doc_id: doc.doc_id.clone(),
        doc_path: doc.workspace_path.clone(),
        indexed_at: doc_metadata_updated_at(&doc),
        stale,
        chunk: None,
        context_before: Vec::new(),
        context_after: Vec::new(),
        text: Some(text),
        line_start: None,
        line_end: None,
        effective_end: None,
        truncated,
    })
}

/// p9-fb-35: trim string to N chars (Unicode-safe). Mirrors fb-34's
/// helper at `crates/kebab-app/src/app.rs` — kept local to avoid
/// re-exporting an internal helper.
fn trim_to_chars(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    let mut out = String::with_capacity(n * 4);
    for (i, c) in s.chars().enumerate() {
        if i >= n {
            break;
        }
        out.push(c);
    }
    out
}
```

- [ ] **Step 4.4: Run tests (verify pass)**

```bash
cargo test -p kebab-app --test fetch_integration fetch_doc
```

Expected: 3 PASS.

- [ ] **Step 4.5: Commit**

```bash
git add crates/kebab-app/src/fetch.rs crates/kebab-app/tests/fetch_integration.rs
git commit -m "$(cat <<'EOF'
feat(app): App::fetch doc mode with budget (fb-35)

Walks CanonicalDocument blocks, serializes to markdown, applies
chars/4 budget when opts.max_tokens is set. doc_not_found
preserved through StructuredError.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: `App::fetch` span mode + PDF/audio rejection

**Files:**
- Modify: `crates/kebab-app/src/fetch.rs`
- Modify: `crates/kebab-app/tests/fetch_integration.rs`

- [ ] **Step 5.1: Failing test**

Append to `crates/kebab-app/tests/fetch_integration.rs`:

```rust
#[test]
fn fetch_span_returns_line_range() {
    let env = common::TestEnv::new();
    let body = "Line one.\nLine two.\nLine three.\nLine four.\nLine five.\n";
    common::ingest_md(&env, "lines.md", body);
    let app = env.app();
    let doc_id = common::list_docs(&app)[0].doc_id.clone();

    let result = app
        .fetch(
            FetchQuery::Span {
                doc_id,
                line_start: 2,
                line_end: 4,
            },
            FetchOpts::default(),
        )
        .unwrap();
    assert_eq!(result.kind, FetchKind::Span);
    let text = result.text.expect("span text");
    // Lines 2-4 of the rendered markdown — exact content depends on
    // canonicalization, so check that we got a 3-line slice.
    assert_eq!(text.lines().count(), 3, "span should be 3 lines: {text:?}");
    assert_eq!(result.line_start, Some(2));
    assert_eq!(result.line_end, Some(4));
    assert_eq!(result.effective_end, Some(4));
    assert!(!result.truncated);
}

#[test]
fn fetch_span_clamps_line_end_when_out_of_range() {
    let env = common::TestEnv::new();
    common::ingest_md(&env, "short.md", "Line one.\nLine two.\n");
    let app = env.app();
    let doc_id = common::list_docs(&app)[0].doc_id.clone();

    let result = app
        .fetch(
            FetchQuery::Span {
                doc_id,
                line_start: 1,
                line_end: 999,
            },
            FetchOpts::default(),
        )
        .unwrap();
    let text = result.text.expect("span text");
    let actual_lines = text.lines().count();
    assert_eq!(result.effective_end, Some(actual_lines as u32));
    assert!(actual_lines < 999);
}
```

- [ ] **Step 5.2: Run tests (verify failure)**

```bash
cargo test -p kebab-app --test fetch_integration fetch_span
```

Expected: FAIL.

- [ ] **Step 5.3: Implement span mode**

Replace the placeholder `fetch_span` in `crates/kebab-app/src/fetch.rs`:

```rust
fn fetch_span(
    app: &App,
    id: DocumentId,
    line_start: u32,
    line_end: u32,
    opts: FetchOpts,
) -> Result<FetchResult> {
    let doc = <kebab_store_sqlite::SqliteStore as DocumentStore>::get_document(&app.sqlite, &id)?
        .ok_or_else(|| {
            StructuredError(ErrorV1 {
                schema_version: "error.v1".to_string(),
                code: "doc_not_found".to_string(),
                message: format!("doc_id '{}' not found", id.0),
                details: serde_json::Value::Null,
                hint: None,
            })
        })?;

    // Reject line-incompatible source types (PDF / audio).
    if matches!(
        doc.metadata.source_type,
        kebab_core::SourceType::Pdf | kebab_core::SourceType::Audio
    ) {
        return Err(StructuredError(ErrorV1 {
            schema_version: "error.v1".to_string(),
            code: "span_not_supported".to_string(),
            message: format!(
                "doc '{}' has source_type {:?}; line-based span fetch unsupported. \
                 Use `fetch chunk` or `fetch doc` instead.",
                id.0, doc.metadata.source_type
            ),
            details: serde_json::Value::Null,
            hint: Some("kind = chunk or kind = doc instead".to_string()),
        }))?;
    }

    if line_start == 0 || line_end == 0 || line_end < line_start {
        return Err(StructuredError(ErrorV1 {
            schema_version: "error.v1".to_string(),
            code: "invalid_input".to_string(),
            message: format!(
                "line_start ({line_start}) and line_end ({line_end}) must be 1-based with start <= end"
            ),
            details: serde_json::Value::Null,
            hint: None,
        }))?;
    }

    let full = fmt_canonical_to_markdown(&doc);
    let lines: Vec<&str> = full.lines().collect();
    let total = lines.len() as u32;
    let effective_end = line_end.min(total).max(line_start);
    let lo = (line_start - 1) as usize;
    let hi = effective_end as usize;
    let mut text = lines[lo..hi].join("\n");

    let mut truncated = effective_end != line_end;
    let mut effective_end_after_budget = effective_end;
    if let Some(max_tokens) = opts.max_tokens {
        let max_chars = max_tokens.saturating_mul(4);
        if text.chars().count() > max_chars {
            text = trim_to_chars(&text, max_chars);
            truncated = true;
            // Recount lines after char-trim — effective_end may shrink.
            let kept = text.lines().count() as u32;
            effective_end_after_budget = (line_start - 1) + kept;
        }
    }

    let now = OffsetDateTime::now_utc();
    let stale = compute_stale(
        doc_metadata_updated_at(&doc),
        now,
        app.config.search.stale_threshold_days,
    );

    Ok(FetchResult {
        kind: FetchKind::Span,
        doc_id: doc.doc_id.clone(),
        doc_path: doc.workspace_path.clone(),
        indexed_at: doc_metadata_updated_at(&doc),
        stale,
        chunk: None,
        context_before: Vec::new(),
        context_after: Vec::new(),
        text: Some(text),
        line_start: Some(line_start),
        line_end: Some(line_end),
        effective_end: Some(effective_end_after_budget),
        truncated,
    })
}
```

If `kebab_core::SourceType` doesn't have `Pdf` / `Audio` variants exactly (might be lowercase, might be different names), adapt:

```bash
grep -A 10 "^pub enum SourceType" crates/kebab-core/src/metadata.rs
```

- [ ] **Step 5.4: Run tests (verify pass)**

```bash
cargo test -p kebab-app --test fetch_integration fetch_span
```

Expected: 2 PASS.

- [ ] **Step 5.5: Run all kebab-app tests**

```bash
cargo test -p kebab-app
```

Expected: all PASS — existing search / staleness / cursor tests unaffected.

- [ ] **Step 5.6: Commit**

```bash
git add crates/kebab-app/src/fetch.rs crates/kebab-app/tests/fetch_integration.rs
git commit -m "$(cat <<'EOF'
feat(app): App::fetch span mode + PDF/audio rejection (fb-35)

Line-based slice over fmt_canonical_to_markdown output.
PDF / audio source_type → span_not_supported StructuredError.
Out-of-range line_end clamps to total; effective_end reflects
post-budget trim.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: CLI `kebab fetch` subcommand

**Files:**
- Modify: `crates/kebab-cli/src/main.rs`
- Modify: `crates/kebab-cli/src/wire.rs`

- [ ] **Step 6.1: Add wire helper**

Edit `crates/kebab-cli/src/wire.rs`. Append after `wire_search_response`:

```rust
/// p9-fb-35: tag a `FetchResult` as `fetch_result.v1`.
pub fn wire_fetch_result(r: &kebab_core::FetchResult) -> Value {
    let mut v = serde_json::to_value(r).expect("FetchResult serializes");
    if let Value::Object(ref mut map) = v {
        map.insert(
            "schema_version".to_string(),
            Value::String("fetch_result.v1".to_string()),
        );
    }
    v
}
```

(Use the same shape pattern as `wire_search_response` from fb-34. The key insertion is idempotent / inserts the discriminator on top of serde's default.)

- [ ] **Step 6.2: Add clap subcommand**

Edit `crates/kebab-cli/src/main.rs`. Find the existing `Cmd::Inspect` enum branch and add a `Fetch` variant nearby:

```bash
grep -n "Inspect {\|InspectWhat" crates/kebab-cli/src/main.rs | head -3
```

In the `enum Cmd { ... }` definition, add:

```rust
/// p9-fb-35: verbatim chunk / doc / span fetch.
Fetch {
    #[command(subcommand)]
    what: FetchWhat,
},
```

Add the `FetchWhat` enum near `InspectWhat`:

```rust
#[derive(Subcommand, Debug)]
enum FetchWhat {
    /// Fetch a single chunk verbatim, optionally with surrounding context.
    Chunk {
        id: String,
        /// p9-fb-35: include ±N chunks before and after the target.
        #[arg(long)]
        context: Option<u32>,
    },
    /// Fetch the entire normalized markdown text of a document.
    Doc {
        id: String,
        /// p9-fb-35: chars/4 budget cap.
        #[arg(long)]
        max_tokens: Option<usize>,
    },
    /// Fetch a 1-based line range of a document. PDF / audio source
    /// types are rejected (use `fetch chunk` instead).
    Span {
        doc_id: String,
        line_start: u32,
        line_end: u32,
        /// p9-fb-35: chars/4 budget cap.
        #[arg(long)]
        max_tokens: Option<usize>,
    },
}
```

In the match arm of `run()`, add `Cmd::Fetch { what } => { ... }`. Place it next to the `Cmd::Inspect` arm:

```rust
Cmd::Fetch { what } => {
    let cfg = kebab_config::Config::load(cli.config.as_deref())?;
    let (query, opts) = match what {
        FetchWhat::Chunk { id, context } => (
            kebab_core::FetchQuery::Chunk(kebab_core::ChunkId(id.clone())),
            kebab_core::FetchOpts {
                context: *context,
                max_tokens: None,
            },
        ),
        FetchWhat::Doc { id, max_tokens } => (
            kebab_core::FetchQuery::Doc(kebab_core::DocumentId(id.clone())),
            kebab_core::FetchOpts {
                context: None,
                max_tokens: *max_tokens,
            },
        ),
        FetchWhat::Span {
            doc_id,
            line_start,
            line_end,
            max_tokens,
        } => (
            kebab_core::FetchQuery::Span {
                doc_id: kebab_core::DocumentId(doc_id.clone()),
                line_start: *line_start,
                line_end: *line_end,
            },
            kebab_core::FetchOpts {
                context: None,
                max_tokens: *max_tokens,
            },
        ),
    };
    let result = kebab_app::fetch_with_config(cfg, query, opts)?;
    if cli.json {
        println!("{}", serde_json::to_string(&wire::wire_fetch_result(&result))?);
    } else {
        render_fetch_plain(&result);
    }
    Ok(())
}
```

Add the plain renderer function near the bottom of `main.rs`:

```rust
/// p9-fb-35: human-friendly plain output.
fn render_fetch_plain(r: &kebab_core::FetchResult) {
    println!("# {} ({})", r.doc_path.0, format_kind(r.kind));
    if r.stale {
        println!("[stale; indexed_at = {}]", r.indexed_at);
    }
    match r.kind {
        kebab_core::FetchKind::Chunk => {
            for (label, chunks) in [
                ("=== before ===", &r.context_before),
                ("=== target ===", &r.chunk.iter().cloned().collect::<Vec<_>>()),
                ("=== after ===", &r.context_after),
            ] {
                if !chunks.is_empty() {
                    println!("\n{label}");
                    for c in chunks {
                        println!("[{} § {}]\n{}\n", c.id.0, c.heading_path.last().map(|s| s.as_str()).unwrap_or(""), c.text);
                    }
                }
            }
        }
        kebab_core::FetchKind::Doc | kebab_core::FetchKind::Span => {
            if let Some(text) = &r.text {
                println!("\n{text}");
            }
            if r.truncated {
                eprintln!("[truncated; widen --max-tokens for fuller text]");
            }
        }
    }
}

fn format_kind(k: kebab_core::FetchKind) -> &'static str {
    match k {
        kebab_core::FetchKind::Chunk => "chunk",
        kebab_core::FetchKind::Doc => "doc",
        kebab_core::FetchKind::Span => "span",
    }
}
```

If `Chunk.id` is named `chunk_id` (verify), adjust accordingly. The format/visibility of the `Chunk` struct may require minor adaptation — inspect existing CLI plain renderers (`Cmd::Search` with hits) for the idiom.

- [ ] **Step 6.3: Build CLI**

```bash
cargo build -p kebab-cli
```

Expected: clean.

- [ ] **Step 6.4: Verify --help**

```bash
cargo run -q -p kebab-cli -- fetch --help 2>&1 | head -20
cargo run -q -p kebab-cli -- fetch chunk --help 2>&1 | head -10
cargo run -q -p kebab-cli -- fetch doc --help 2>&1 | head -10
cargo run -q -p kebab-cli -- fetch span --help 2>&1 | head -10
```

Expected: subcommand help shows `chunk` / `doc` / `span` and per-mode flags.

- [ ] **Step 6.5: Run kebab-cli tests**

```bash
cargo test -p kebab-cli
```

Expected: all PASS.

- [ ] **Step 6.6: Commit**

```bash
git add crates/kebab-cli/src/main.rs crates/kebab-cli/src/wire.rs
git commit -m "$(cat <<'EOF'
feat(cli): kebab fetch chunk / doc / span (fb-35)

JSON output is fetch_result.v1; plain output is human-friendly
labeled sections (chunk: before / target / after; doc/span: full
text + stderr truncated hint).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: CLI integration tests

**Files:**
- Create: `crates/kebab-cli/tests/wire_fetch.rs`
- Modify: `crates/kebab-cli/tests/common/mod.rs` (if helper missing)

- [ ] **Step 7.1: Add `run_fetch_with_args` helper**

Edit `crates/kebab-cli/tests/common/mod.rs`. After existing `run_search_with_args`:

```rust
/// p9-fb-35: invoke `kebab fetch` with arbitrary args.
pub fn run_fetch_with_args(cfg: &std::path::Path, args: &[&str]) -> (String, String) {
    let exe = env!("CARGO_BIN_EXE_kebab");
    let mut cmd_args: Vec<&str> = vec!["--config"];
    let cfg_str = cfg.to_str().expect("utf8");
    cmd_args.push(cfg_str);
    cmd_args.push("fetch");
    cmd_args.extend(args);
    let out = std::process::Command::new(exe)
        .args(&cmd_args)
        .output()
        .expect("kebab fetch");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}
```

If the existing `run_search_with_args` has a different signature (e.g. takes `&workspace`), mirror it.

- [ ] **Step 7.2: Write integration tests**

Create `crates/kebab-cli/tests/wire_fetch.rs`:

```rust
//! p9-fb-35: CLI fetch wire shape + plain output + exit codes.

mod common;

use serde_json::Value;

#[test]
fn fetch_chunk_json_emits_fetch_result_v1() {
    let (cfg, ws) = common::write_config();
    common::ingest(&cfg, &ws, "a.md", "# T\n\napples are red.\n");

    // First find a chunk_id via search.
    let (search_stdout, _) = common::run_search_with_args(
        &cfg,
        &["--mode", "lexical", "--json", "--k", "1", "apples"],
    );
    let search: Value = serde_json::from_str(search_stdout.trim()).expect("search json");
    let chunk_id = search["hits"][0]["chunk_id"]
        .as_str()
        .expect("chunk_id")
        .to_string();

    let (stdout, _) = common::run_fetch_with_args(
        &cfg,
        &["--json", "chunk", &chunk_id],
    );
    let v: Value = serde_json::from_str(stdout.trim()).expect("fetch json");
    assert_eq!(v["schema_version"], "fetch_result.v1");
    assert_eq!(v["kind"], "chunk");
    assert!(v["chunk"].is_object());
    assert_eq!(v["truncated"], false);
}

#[test]
fn fetch_doc_json_with_max_tokens_truncates() {
    let (cfg, ws) = common::write_config();
    let body: String = "Lorem ipsum dolor sit amet. ".repeat(20);
    common::ingest(&cfg, &ws, "big.md", &format!("# Big\n\n{body}\n"));

    // Discover doc_id via list-docs.
    let (list_stdout, _) = common::run_with_args(
        &cfg,
        &["list", "docs", "--json"],
    );
    // list docs --json prints `doc_summary.v1` ndjson; parse first line.
    let first = list_stdout.lines().next().expect("at least one doc");
    let summary: Value = serde_json::from_str(first).expect("doc_summary json");
    let doc_id = summary["doc_id"].as_str().expect("doc_id").to_string();

    let (stdout, _) = common::run_fetch_with_args(
        &cfg,
        &["--json", "doc", &doc_id, "--max-tokens", "20"],
    );
    let v: Value = serde_json::from_str(stdout.trim()).expect("fetch json");
    assert_eq!(v["kind"], "doc");
    assert_eq!(v["truncated"], true);
}

#[test]
fn fetch_chunk_unknown_id_exits_with_error_v1() {
    let (cfg, _ws) = common::write_config();
    let exe = env!("CARGO_BIN_EXE_kebab");
    let cfg_str = cfg.to_str().expect("utf8");
    let out = std::process::Command::new(exe)
        .args(["--config", cfg_str, "--json", "fetch", "chunk", "nonexistent"])
        .output()
        .expect("kebab fetch");
    assert_ne!(out.status.code(), Some(0), "must exit non-zero");
    let stderr = String::from_utf8_lossy(&out.stderr);
    let err_line = stderr
        .lines()
        .find(|l| {
            serde_json::from_str::<Value>(l)
                .ok()
                .and_then(|v| v.get("schema_version").and_then(|s| s.as_str()).map(String::from))
                .as_deref()
                == Some("error.v1")
        })
        .unwrap_or_else(|| panic!("no error.v1 on stderr: {stderr}"));
    let v: Value = serde_json::from_str(err_line).expect("error.v1 json");
    assert_eq!(v["code"], "chunk_not_found");
}
```

If `common::run_with_args` (generic dispatcher) doesn't exist, write a one-off `Command` invocation inline for the list-docs call. The goal is to obtain a doc_id; any working route is fine.

- [ ] **Step 7.3: Run tests**

```bash
cargo test -p kebab-cli --test wire_fetch
```

Expected: 3 PASS.

- [ ] **Step 7.4: Run full kebab-cli suite**

```bash
cargo test -p kebab-cli
```

Expected: all PASS, no regressions.

- [ ] **Step 7.5: Commit**

```bash
git add crates/kebab-cli/tests/
git commit -m "$(cat <<'EOF'
test(cli): wire_fetch — chunk/doc + chunk_not_found integration (fb-35)

3 integration tests: chunk JSON shape, doc truncated, unknown id
returns error.v1 with code = chunk_not_found.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 8: MCP `kebab__fetch` tool

**Files:**
- Create: `crates/kebab-mcp/src/tools/fetch.rs`
- Modify: `crates/kebab-mcp/src/lib.rs`
- Create: `crates/kebab-mcp/tests/tools_call_fetch.rs`

- [ ] **Step 8.1: Inspect MCP tool registration pattern**

```bash
sed -n '1,80p' crates/kebab-mcp/src/lib.rs
ls crates/kebab-mcp/src/tools/
```

Read `src/tools/search.rs` to mirror the input/handler pattern.

- [ ] **Step 8.2: Implement fetch tool**

Create `crates/kebab-mcp/src/tools/fetch.rs`:

```rust
//! `fetch` tool — wraps `kebab_app::fetch_with_config` for chunk /
//! doc / span mode. Input shape matches the spec / SKILL.md.

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{to_tool_error, to_tool_success};
use crate::state::KebabAppState;

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct FetchInput {
    /// "chunk" | "doc" | "span"
    pub kind: String,
    /// Required when kind = "chunk".
    pub chunk_id: Option<String>,
    /// Required when kind = "doc" or "span".
    pub doc_id: Option<String>,
    /// Required when kind = "span" (1-based, inclusive).
    pub line_start: Option<u32>,
    pub line_end: Option<u32>,
    /// chunk only: ±N surrounding chunks.
    pub context: Option<u32>,
    /// doc/span only: chars/4 budget.
    pub max_tokens: Option<usize>,
}

pub fn handle(state: &KebabAppState, input: FetchInput) -> CallToolResult {
    let query = match input.kind.as_str() {
        "chunk" => match input.chunk_id {
            Some(id) => kebab_core::FetchQuery::Chunk(kebab_core::ChunkId(id)),
            None => return invalid_input("kind=chunk requires chunk_id"),
        },
        "doc" => match input.doc_id {
            Some(id) => kebab_core::FetchQuery::Doc(kebab_core::DocumentId(id)),
            None => return invalid_input("kind=doc requires doc_id"),
        },
        "span" => match (input.doc_id, input.line_start, input.line_end) {
            (Some(id), Some(start), Some(end)) => kebab_core::FetchQuery::Span {
                doc_id: kebab_core::DocumentId(id),
                line_start: start,
                line_end: end,
            },
            _ => return invalid_input("kind=span requires doc_id, line_start, line_end"),
        },
        other => return invalid_input(&format!("unknown kind '{other}'; expected chunk|doc|span")),
    };

    let opts = kebab_core::FetchOpts {
        context: input.context,
        max_tokens: input.max_tokens,
    };

    let cfg_clone = (*state.config).clone();
    let result = kebab_app::fetch_with_config(cfg_clone, query, opts);
    match result {
        Ok(r) => {
            let mut v = match serde_json::to_value(&r) {
                Ok(v) => v,
                Err(e) => return to_tool_error(&anyhow::anyhow!("FetchResult serialize: {e}")),
            };
            if let serde_json::Value::Object(ref mut map) = v {
                map.insert(
                    "schema_version".to_string(),
                    serde_json::Value::String("fetch_result.v1".to_string()),
                );
            }
            match serde_json::to_string(&v) {
                Ok(json) => to_tool_success(json),
                Err(e) => to_tool_error(&anyhow::anyhow!(e)),
            }
        }
        Err(e) => to_tool_error(&e),
    }
}

fn invalid_input(msg: &str) -> CallToolResult {
    to_tool_error(&anyhow::anyhow!("invalid_input: {msg}"))
}
```

If `to_tool_error` already wraps in `error.v1.code = invalid_input` based on the message prefix, the `format!("invalid_input: ...")` is enough. Verify by reading `crates/kebab-mcp/src/error.rs` (or wherever `to_tool_error` lives). If not, construct an explicit `StructuredError`:

```rust
use kebab_app::{ErrorV1, StructuredError};

fn invalid_input(msg: &str) -> CallToolResult {
    let err = anyhow::Error::new(StructuredError(ErrorV1 {
        schema_version: "error.v1".to_string(),
        code: "invalid_input".to_string(),
        message: msg.to_string(),
        details: serde_json::Value::Null,
        hint: None,
    }));
    to_tool_error(&err)
}
```

This guarantees `error.v1.code = "invalid_input"` propagates through.

- [ ] **Step 8.3: Register tool**

Edit `crates/kebab-mcp/src/lib.rs`. Find where `search` / `ask` / `schema` / `doctor` / `ingest_*` tools are registered (search for `tools::search` or `register`):

```bash
grep -n "tools::search\|tools::ask\|register_tool\|mcp_tool" crates/kebab-mcp/src/lib.rs | head -10
```

Add `mod fetch;` to the `mod tools { ... }` block (or wherever) and register the tool the same way as `search`. Inspect existing registration boilerplate and clone it.

- [ ] **Step 8.4: Write integration test**

Create `crates/kebab-mcp/tests/tools_call_fetch.rs`. Mirror `tests/tools_call_search.rs`:

```rust
//! p9-fb-35: mcp__kebab__fetch tool — 3 modes + invalid_input.

mod common;

use serde_json::Value;

#[test]
fn fetch_tool_chunk_returns_fetch_result_v1() {
    let env = common::TestEnv::new();
    common::ingest_md(&env, "a.md", "# T\n\napples\n");

    // Discover a chunk_id via the search tool.
    let chunk_id = common::call_search(&env, "apples")
        .pointer("/hits/0/chunk_id")
        .expect("search returns at least one hit")
        .as_str()
        .expect("chunk_id string")
        .to_string();

    let body = common::call_fetch_chunk(&env, &chunk_id);
    let v: Value = serde_json::from_str(&body).expect("fetch json");
    assert_eq!(v["schema_version"], "fetch_result.v1");
    assert_eq!(v["kind"], "chunk");
}

#[test]
fn fetch_tool_invalid_kind_returns_invalid_input() {
    let env = common::TestEnv::new();
    let body = common::call_fetch_raw(
        &env,
        serde_json::json!({"kind": "garbage"}),
    );
    let v: Value = serde_json::from_str(&body).expect("error json");
    assert_eq!(v["schema_version"], "error.v1");
    assert_eq!(v["code"], "invalid_input");
}
```

The `common::call_fetch_chunk` / `call_search` helpers depend on the existing kebab-mcp test scaffold. Inspect `crates/kebab-mcp/tests/common/mod.rs` (or `tests/tools_call_search.rs`'s test setup) and clone the pattern. Add helpers as needed.

- [ ] **Step 8.5: Run MCP tests**

```bash
cargo test -p kebab-mcp
```

Expected: all PASS.

- [ ] **Step 8.6: Commit**

```bash
git add crates/kebab-mcp/
git commit -m "$(cat <<'EOF'
feat(mcp): kebab__fetch tool — chunk / doc / span (fb-35)

Mirrors CLI surface: same input shape, same fetch_result.v1
output. invalid_input error for missing kind-specific fields.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 9: Workspace test + clippy gate

- [ ] **Step 9.1: Workspace test**

```bash
cargo test --workspace --no-fail-fast -j 1 2>&1 | tail -20
```

Expected: all PASS.

- [ ] **Step 9.2: Clippy**

```bash
cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -10
```

Expected: clean. New patterns to watch:
- `clippy::result_large_err` on `App::fetch` if it returns `Result<FetchResult, ErrorV1>`. We use `Result<FetchResult, anyhow::Error>` so this should not fire — `StructuredError` wraps `ErrorV1` inside an `anyhow::Error`.
- `clippy::large_enum_variant` on `FetchQuery` (`Span` carries 3 fields). Smaller than `StreamEvent::Final`; should be fine.

- [ ] **Step 9.3: Commit clippy fixes if needed**

```bash
git add -A
git commit -m "chore: clippy fixes for fb-35"
```

(Skip if no fixes were necessary.)

---

## Task 10: Documentation updates

**Files:**
- Modify: `README.md`
- Modify: `docs/SMOKE.md`
- Modify: `tasks/p9/p9-fb-35-verbatim-fetch.md`
- Modify: `tasks/INDEX.md`
- Modify: `integrations/claude-code/kebab/SKILL.md`

- [ ] **Step 10.1: README — fetch row**

Find the 명령 table:

```bash
grep -n "| .kebab " README.md | head -10
```

Add a new row after `inspect`:

```markdown
| `kebab fetch chunk <id> [--context N]` / `kebab fetch doc <id> [--max-tokens N]` / `kebab fetch span <doc_id> <ls> <le> [--max-tokens N]` | (p9-fb-35) verbatim text fetch from indexed corpus. wire = `fetch_result.v1` (kind discriminator). chunk: target + ±N ordinal-context chunks. doc: full normalized markdown. span: 1-based line range (PDF/audio rejected as `error.v1.code = span_not_supported`). chars/4 budget on doc/span. |
```

- [ ] **Step 10.2: SMOKE.md — fetch walkthrough**

Append a section after the fb-34 pagination block:

```markdown
### Verbatim fetch (fb-35)

```bash
# Search to get a chunk_id.
CHUNK_ID=$(kebab search "rust ownership" --json --k 1 | jq -r '.hits[0].chunk_id')

# Fetch verbatim with surrounding context.
kebab fetch chunk "$CHUNK_ID" --context 2 --json | jq .

# Fetch the full doc as markdown.
DOC_ID=$(kebab list docs --json | head -1 | jq -r .doc_id)
kebab fetch doc "$DOC_ID" --max-tokens 1000 --json | jq '{kind, truncated, len: (.text | length)}'

# Fetch a line range (markdown / text only).
kebab fetch span "$DOC_ID" 1 5 --json | jq '{line_start, line_end, effective_end, text}'
```

PDF / audio docs reject `fetch span` with `error.v1.code = span_not_supported` — use `fetch chunk` (PDF chunks are page-aligned) or `fetch doc` instead.
```

- [ ] **Step 10.3: Spec status flip**

Edit `tasks/p9/p9-fb-35-verbatim-fetch.md`:

```diff
-status: open
+status: completed
```

Replace the `> ⏳ **백로그 only — 미구현.**` block with:

```markdown
> ✅ **구현 완료.** 본 spec 은 구현 시점의 frozen 상태. post-merge deviation 은 [HOTFIXES.md](../HOTFIXES.md) 참조.

상세 설계: `docs/superpowers/specs/2026-05-09-p9-fb-35-verbatim-fetch-design.md`.
구현 계획: `docs/superpowers/plans/2026-05-09-p9-fb-35-verbatim-fetch.md`.
```

- [ ] **Step 10.4: tasks/INDEX.md**

```diff
-    - [p9-fb-35 verbatim fetch](p9/p9-fb-35-verbatim-fetch.md) — ⏳ 미구현, brainstorm 필요
+    - [p9-fb-35 verbatim fetch](p9/p9-fb-35-verbatim-fetch.md) — ✅ 머지 + v0.5.0 cut 후보 (2026-05-09)
```

- [ ] **Step 10.5: SKILL.md**

Find the MCP tools table (around line 33-41) and add a row:

```markdown
| `mcp__kebab__fetch` | verbatim text → `fetch_result.v1` (chunk / doc / span) | no |
```

After the `mcp__kebab__ask` section, add a new section:

```markdown
### `mcp__kebab__fetch` — when you need raw text

Use after `search` to read the verbatim chunk text + surrounding context, or to pull a full doc / line range.

Input:
```json
{ "kind": "chunk", "chunk_id": "<id>", "context": 2 }
{ "kind": "doc", "doc_id": "<id>", "max_tokens": 1000 }
{ "kind": "span", "doc_id": "<id>", "line_start": 1, "line_end": 5 }
```

- `chunk` mode: `--context N` returns ordinal-adjacent chunks before/after for surrounding paragraphs.
- `doc` mode: full normalized markdown. `max_tokens` (chars/4) caps the response — `truncated: true` when applied.
- `span` mode: 1-based line range. PDF / audio docs reject as `error.v1.code = span_not_supported` (use `chunk` mode instead — PDF chunks are page-aligned).
- `error.v1.code = chunk_not_found` / `doc_not_found` are non-retryable from the same query — re-issue search to get a fresh id.
```

- [ ] **Step 10.6: Commit docs**

```bash
git add README.md docs/SMOKE.md tasks/p9/p9-fb-35-verbatim-fetch.md tasks/INDEX.md integrations/claude-code/kebab/SKILL.md
git commit -m "$(cat <<'EOF'
docs(fb-35): README + SMOKE + INDEX + skill notes

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 11: Smoke + push + PR

- [ ] **Step 11.1: Manual smoke**

```bash
cd /tmp/kebab-smoke
~/Workspace/projects/kebab/target/release/kebab --config /tmp/kebab-smoke/config.toml ingest
CHUNK_ID=$(~/Workspace/projects/kebab/target/release/kebab --config /tmp/kebab-smoke/config.toml search "test" --json --k 1 | jq -r '.hits[0].chunk_id')
~/Workspace/projects/kebab/target/release/kebab --config /tmp/kebab-smoke/config.toml fetch chunk "$CHUNK_ID" --context 1 --json | jq '{schema_version, kind, before: (.context_before | length), after: (.context_after | length)}'
```

Expected:
- `schema_version: "fetch_result.v1"`, `kind: "chunk"`, sane before/after counts.

- [ ] **Step 11.2: Final workspace test**

```bash
cd ~/Workspace/projects/kebab
cargo test --workspace --no-fail-fast -j 1
```

Expected: all green.

- [ ] **Step 11.3: Push branch**

```bash
git push -u origin feat/fb-35-verbatim-fetch
```

- [ ] **Step 11.4: Open PR via gitea-pr**

Build PR body at `/tmp/fb35-pr-body.md`:

```markdown
## Summary

- adds `kebab fetch chunk|doc|span` CLI subcommand + `mcp__kebab__fetch` MCP tool
- wire = `fetch_result.v1` (kind discriminator). chunk mode optional `--context N` returns ±N ordinal-adjacent chunks. doc mode serializes `CanonicalDocument` to markdown. span mode slices line range (1-based inclusive).
- PDF / audio source_type rejected on span fetch — `error.v1.code = span_not_supported`.
- chars/4 budget on doc/span (mirrors fb-34); chunk fetch unbounded.
- All errors typed via fb-34 `StructuredError` wrapper — `chunk_not_found` / `doc_not_found` / `span_not_supported` / `invalid_input` reach the wire.

## Test plan

- [x] `cargo test --workspace --no-fail-fast -j 1` — green
- [x] `cargo clippy --workspace --all-targets -- -D warnings` — clean
- [x] new tests:
  - `fetch_integration` (kebab-app): 7 tests covering chunk / chunk+context / doc / doc+budget / span / span clamp / unknown id
  - `wire_fetch` (kebab-cli): 3 lexical-only integration tests (wire shape, doc truncation, error path)
  - `tools_call_fetch` (kebab-mcp): 2 tests (chunk happy path + invalid_input)
- [x] manual smoke per `docs/SMOKE.md` "Verbatim fetch" walkthrough

## Architectural notes

- `App::fetch` is the new public method; `App::search` / `App::search_with_opts` / `App::ask` unchanged.
- `fmt_canonical_to_markdown` is a private helper in `kebab-app::fetch` — round-trip is best-effort (inline styling collapsed); good enough for an agent reading raw context.
- chunk_id stability: blake3(doc_id || chunker_version || ...) — stable across normal re-ingest, invalidated only by chunker_version cascade. Spec + SKILL.md note retry pattern.
- Source = `chunks.text` / `CanonicalDocument`. Raw bytes (`assets.storage_path`) NOT exposed by design — sucetable from sucetable from sucetable from sucetable user can read directly.

## Files of interest

- spec: `docs/superpowers/specs/2026-05-09-p9-fb-35-verbatim-fetch-design.md`
- plan: `docs/superpowers/plans/2026-05-09-p9-fb-35-verbatim-fetch.md`
- core: `crates/kebab-core/src/fetch.rs`
- app: `crates/kebab-app/src/fetch.rs`
- CLI: `crates/kebab-cli/src/main.rs` + `wire.rs::wire_fetch_result`
- MCP: `crates/kebab-mcp/src/tools/fetch.rs`
- wire: `docs/wire-schema/v1/fetch_result.schema.json`
```

(Fix the typo `sucetable from` — likely an artifact; replace with: "user can read directly via `cat $WORKSPACE_ROOT/<storage_path>`.")

Open the PR:

```bash
/Users/user/.claude/skills/gitea-ops/bin/gitea-pr \
  --title "feat(fb-35): verbatim fetch (chunk / doc / span)" \
  --body "$(cat /tmp/fb35-pr-body.md)" \
  --head feat/fb-35-verbatim-fetch \
  --base main
```

Capture the URL.

- [ ] **Step 11.5: Cleanup**

```bash
rm /tmp/fb35-pr-body.md
```

---

## Self-review

- **Spec coverage:**
  - §Behavior contract / 3 modes → Tasks 3, 4, 5
  - §CLI subcommand → Task 6
  - §Wire shape → Task 2 (schema) + Task 6 (CLI emit) + Task 8 (MCP emit)
  - §Mode 동작 (chunk / doc / span semantics, error codes) → Tasks 3, 4, 5
  - §Budget integration → Tasks 4, 5 (`max_tokens` chars/4 trim)
  - §Error codes → all tasks via `StructuredError`
  - §MCP tool → Task 8
  - §Public surface delta → Tasks 1, 3
  - §Test plan → Tasks 3, 4, 5 (App), 7 (CLI), 8 (MCP)
  - §Documentation → Task 10
  - §Risks (markdown round-trip, span line counting, chunk_id stability, PDF/audio rejection) → addressed in Task 4 (helper), Task 5 (line counting + reject), Task 10 (SKILL.md notes), Task 5 (rejection)

- **Placeholder scan:**
  - Task 6 / 7 / 8 have "If existing helper has different signature, mirror it" — concrete fallback paths spelled out.
  - Task 3's `app.sqlite` access note instructs visibility check + fallback. No "TODO" / "fill in" remaining.

- **Type consistency:**
  - `FetchQuery::{Chunk(ChunkId), Doc(DocumentId), Span { doc_id, line_start, line_end }}` consistent across Tasks 1, 3, 4, 5, 6, 8.
  - `FetchOpts { context: Option<u32>, max_tokens: Option<usize> }` consistent.
  - `FetchResult { kind, doc_id, doc_path, indexed_at, stale, chunk, context_before, context_after, text, line_start, line_end, effective_end, truncated }` consistent.
  - `FetchKind { Chunk, Doc, Span }` snake_case rename pinned in Task 1's serde test.
  - Error codes consistent: `chunk_not_found`, `doc_not_found`, `span_not_supported`, `invalid_input`.

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-05-09-p9-fb-35-verbatim-fetch.md`. Two execution options:

**1. Subagent-Driven (recommended)** — fresh subagent per task, review between tasks.

**2. Inline Execution** — execute tasks in this session.

Which approach?
