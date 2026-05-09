# p9-fb-34 — Output Budget Controls Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `--max-tokens` / `--snippet-chars` / `--cursor` flags to `kebab search` so agents can cap result size and paginate. Wire output gains a top-level `search_response.v1` wrapper around the existing `search_hit.v1[]` array, with `next_cursor` and `truncated` metadata.

**Architecture:** Domain `SearchOpts` enters `App::search_with_opts(query, opts) -> SearchResponse`; existing `App::search(query) -> Vec<SearchHit>` becomes a thin wrapper. Token estimation uses `chars/4` (no new tokenizer dep). Truncate priority: snippet shorten → k pop → minimum 1 hit. Cursor is opaque base64 of `{offset, corpus_revision}` JSON; mismatch returns `error.v1.code = stale_cursor`. CLI plain output unchanged + truncated stderr hint; `--json` output is the new wrapper.

**Tech Stack:** Rust 2024, base64 (workspace dep — add to root if missing), serde, JSON Schema (search_response.v1).

**Spec:** `docs/superpowers/specs/2026-05-09-p9-fb-34-output-budget-controls-design.md`

---

## File Structure

| File | Responsibility | Action |
|------|----------------|--------|
| `crates/kebab-core/src/search.rs` | New `pub struct SearchOpts { max_tokens, snippet_chars, cursor }` with `Default` impl | modify |
| `crates/kebab-core/src/lib.rs` | Re-export `SearchOpts` | modify |
| `crates/kebab-app/src/cursor.rs` | New module — `encode_cursor(offset, revision) -> String`, `decode_cursor(s, expected) -> Result<usize, ErrorV1>` | create |
| `crates/kebab-app/src/app.rs` | New `pub struct SearchResponse`, `App::search_with_opts(...)`, budget loop, retain `App::search` thin wrapper | modify |
| `crates/kebab-app/src/lib.rs` | Re-export `SearchResponse`, `SearchOpts`, cursor module if needed | modify |
| `crates/kebab-app/src/error_wire.rs` | Add `stale_cursor` classify branch | modify |
| `crates/kebab-app/Cargo.toml` | Add `base64` dep (or workspace-managed) | modify |
| `Cargo.toml` (workspace root) | Add `base64 = "0.22"` to `[workspace.dependencies]` if not already managed | modify (conditional) |
| `crates/kebab-cli/src/main.rs` | `Cmd::Search` new flags + dispatch to `search_with_opts` + plain truncated hint | modify |
| `crates/kebab-cli/src/wire.rs` | New `wire_search_response(&SearchResponse) -> Value` helper | modify |
| `crates/kebab-mcp/src/tools/search.rs` | Extend `SearchInput` + emit `search_response.v1` | modify |
| `docs/wire-schema/v1/search_response.schema.json` | NEW wrapper schema | create |
| `crates/kebab-app/tests/cursor.rs` | Unit: encode/decode round-trip + StaleCursor | create |
| `crates/kebab-app/tests/search_budget_integration.rs` | Integration: budget None passthrough + snippet shorten + k pop + 1-hit minimum + snippet_chars override + cursor pagination + corpus_revision bump → StaleCursor | create |
| `crates/kebab-cli/tests/wire_search_response.rs` | Integration: `--json` shape + `--max-tokens` truncation + `--cursor` next page + plain truncated stderr hint | create |
| `crates/kebab-mcp/tests/tools_call_search.rs` | Augment existing test (or sibling) — verify `search_response.v1` returned | modify |
| `README.md` | `kebab search` row update + `--max-tokens` / `--cursor` mention | modify |
| `docs/SMOKE.md` | Pagination walkthrough paragraph | modify |
| `tasks/p9/p9-fb-34-output-budget-controls.md` | Status flip + design/plan links | modify |
| `tasks/INDEX.md` | fb-34 row → ✅ | modify |
| `tasks/HOTFIXES.md` | New entry — `2026-05-09 — p9-fb-34: search wire wrapped in search_response.v1` | modify |
| `integrations/claude-code/kebab/SKILL.md` | Recipe update — `response.hits[]` instead of bare array; cursor example | modify |

---

## Pre-flight

- [ ] **Step 0.1: Branch off main**

```bash
git checkout main
git pull
git checkout -b feat/fb-34-output-budget-controls
```

- [ ] **Step 0.2: Confirm spec branch reachable**

```bash
git log --oneline spec/fb-34-output-budget-controls -1
```

Expected: `a80f65c spec(fb-34): output budget controls — design`. If spec PR has not yet merged into main, `git merge spec/fb-34-output-budget-controls` so the spec doc lands on this branch.

---

## Task 1: Domain — `SearchOpts` in kebab-core

**Files:**
- Modify: `crates/kebab-core/src/search.rs`
- Modify: `crates/kebab-core/src/lib.rs`

- [ ] **Step 1.1: Write the failing test**

Append to `crates/kebab-core/src/search.rs` `#[cfg(test)] mod tests` block (one already exists from fb-32):

```rust
#[test]
fn search_opts_default_is_all_none() {
    let opts = SearchOpts::default();
    assert!(opts.max_tokens.is_none());
    assert!(opts.snippet_chars.is_none());
    assert!(opts.cursor.is_none());
}
```

- [ ] **Step 1.2: Run test — verify failure**

```bash
cargo test -p kebab-core search_opts_default_is_all_none
```

Expected: FAIL — `cannot find type SearchOpts in scope`.

- [ ] **Step 1.3: Define `SearchOpts`**

Append to `crates/kebab-core/src/search.rs` (after the existing `DocSummary` struct, before any `#[cfg(test)]`):

```rust
/// p9-fb-34: caller-supplied output budget knobs for `App::search_with_opts`.
/// All `None` = no enforcement (existing behavior).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SearchOpts {
    /// chars/4 approximation of wire JSON token cost. None = no cap.
    pub max_tokens: Option<usize>,
    /// Per-hit snippet character cap. None = use config default.
    pub snippet_chars: Option<usize>,
    /// Opaque base64 cursor from a previous response. None = first page.
    pub cursor: Option<String>,
}
```

- [ ] **Step 1.4: Re-export from `crates/kebab-core/src/lib.rs`**

Find the existing `pub use search::{...}` line:

```bash
grep -n "pub use search" crates/kebab-core/src/lib.rs
```

Add `SearchOpts` to the brace list. If the existing line is e.g. `pub use search::{SearchHit, SearchQuery, SearchFilters, SearchMode, RetrievalDetail, DocFilter, DocSummary};`, append `SearchOpts`.

- [ ] **Step 1.5: Run tests — verify pass**

```bash
cargo test -p kebab-core search_opts_default_is_all_none
cargo test -p kebab-core
```

Expected: PASS.

- [ ] **Step 1.6: Commit**

```bash
git add crates/kebab-core/src/search.rs crates/kebab-core/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(core): SearchOpts domain type for budget controls (fb-34)

3 optional knobs (max_tokens, snippet_chars, cursor); Default = all
None = no enforcement (backwards-compat existing search behavior).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: Cursor encode/decode helper

**Files:**
- Create: `crates/kebab-app/src/cursor.rs`
- Modify: `crates/kebab-app/src/lib.rs`
- Modify: `crates/kebab-app/Cargo.toml`
- Possibly modify: `Cargo.toml` (workspace root) — add `base64` to `[workspace.dependencies]` if absent

- [ ] **Step 2.1: Add base64 to kebab-app deps**

Check workspace root `Cargo.toml`:

```bash
grep -n "^base64" Cargo.toml
```

If absent, add to `[workspace.dependencies]`:

```toml
base64 = "0.22"
```

Then add to `crates/kebab-app/Cargo.toml` `[dependencies]`:

```toml
base64 = { workspace = true }
```

If `base64` is already directly in another crate (e.g. `kebab-parse-image`), promote it to workspace dep first then update both.

- [ ] **Step 2.2: Write the failing test**

Create `crates/kebab-app/tests/cursor.rs`:

```rust
//! p9-fb-34: cursor encode/decode round-trip + corpus_revision mismatch.

use kebab_app::cursor;

#[test]
fn cursor_roundtrip_preserves_offset() {
    let encoded = cursor::encode(5, "rev-abc");
    let offset = cursor::decode(&encoded, "rev-abc").unwrap();
    assert_eq!(offset, 5);
}

#[test]
fn cursor_decode_rejects_mismatched_revision() {
    let encoded = cursor::encode(7, "rev-old");
    let err = cursor::decode(&encoded, "rev-new").unwrap_err();
    assert_eq!(err.code, "stale_cursor");
    assert!(err.message.contains("rev-old") || err.message.contains("rev-new"));
}

#[test]
fn cursor_decode_rejects_garbage_input() {
    let err = cursor::decode("not-base64!!!", "any").unwrap_err();
    assert_eq!(err.code, "stale_cursor");
}
```

- [ ] **Step 2.3: Run test — verify failure**

```bash
cargo test -p kebab-app --test cursor
```

Expected: FAIL — `cannot find module cursor in kebab_app`.

- [ ] **Step 2.4: Implement cursor module**

Create `crates/kebab-app/src/cursor.rs`:

```rust
//! p9-fb-34 opaque pagination cursor.
//!
//! Format: base64(JSON({offset: usize, corpus_revision: string})).
//! Opaque to callers — they MUST NOT decode the contents themselves;
//! the schema is internal and may change without notice.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};

use crate::error_wire::ErrorV1;

#[derive(Serialize, Deserialize)]
struct Payload {
    offset: usize,
    corpus_revision: String,
}

/// Encode `(offset, corpus_revision)` as an opaque base64 string.
pub fn encode(offset: usize, corpus_revision: &str) -> String {
    let payload = Payload {
        offset,
        corpus_revision: corpus_revision.to_string(),
    };
    let json = serde_json::to_vec(&payload).expect("Payload serializes");
    URL_SAFE_NO_PAD.encode(&json)
}

/// Decode an opaque cursor against the expected `corpus_revision`.
/// Mismatch or malformed input returns an `ErrorV1` with
/// `code = "stale_cursor"`.
pub fn decode(s: &str, expected_revision: &str) -> Result<usize, ErrorV1> {
    let bytes = URL_SAFE_NO_PAD.decode(s.as_bytes()).map_err(|_| stale(
        "<malformed>",
        expected_revision,
    ))?;
    let payload: Payload = serde_json::from_slice(&bytes).map_err(|_| stale(
        "<malformed>",
        expected_revision,
    ))?;
    if payload.corpus_revision != expected_revision {
        return Err(stale(&payload.corpus_revision, expected_revision));
    }
    Ok(payload.offset)
}

fn stale(found: &str, expected: &str) -> ErrorV1 {
    ErrorV1 {
        schema_version: "error.v1".to_string(),
        code: "stale_cursor".to_string(),
        message: format!(
            "cursor was issued against corpus_revision '{found}'; current revision is \
             '{expected}'. Re-issue search to obtain a fresh cursor."
        ),
        cause: None,
    }
}
```

If `ErrorV1` field names differ (verify via `grep -A 10 "pub struct ErrorV1" crates/kebab-app/src/error_wire.rs`), adapt the struct literal accordingly.

- [ ] **Step 2.5: Wire the module into the crate**

Edit `crates/kebab-app/src/lib.rs`. Find the `mod` declarations near the top and add:

```rust
pub mod cursor;
```

(Use `pub mod` so `cursor::encode` / `cursor::decode` are reachable from the integration test.)

- [ ] **Step 2.6: Run tests — verify pass**

```bash
cargo test -p kebab-app --test cursor
```

Expected: 3 PASS.

- [ ] **Step 2.7: Commit**

```bash
git add crates/kebab-app/src/cursor.rs crates/kebab-app/src/lib.rs crates/kebab-app/Cargo.toml Cargo.toml Cargo.lock crates/kebab-app/tests/cursor.rs
git commit -m "$(cat <<'EOF'
feat(app): cursor encode/decode for paginated search (fb-34)

Opaque base64(JSON{offset, corpus_revision}). Mismatch or
malformed input returns ErrorV1 with code = stale_cursor.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: `error_wire` — `stale_cursor` classification

**Files:**
- Modify: `crates/kebab-app/src/error_wire.rs`

- [ ] **Step 3.1: Write the failing test**

Append to `crates/kebab-app/src/error_wire.rs` `#[cfg(test)] mod tests`:

```rust
#[test]
fn stale_cursor_classifies_correctly() {
    use anyhow::anyhow;
    let err: anyhow::Error = anyhow!("stale_cursor: rev mismatch");
    let v1 = classify(&err, false);
    // Without explicit downcast support, the generic anyhow path
    // will fall through to "unknown" — the actual stale_cursor
    // ErrorV1 is constructed directly by `cursor::decode`, not via
    // the classify path. This test pins that behavior so future
    // refactors of classify don't accidentally clobber the code.
    assert_ne!(v1.code, "stale_cursor", "classify is not the source for stale_cursor");
}
```

(If a richer classification is desired, add a downcast branch — but per the spec, `cursor::decode` returns `ErrorV1` directly so the classify path doesn't need to handle it. The test exists to lock that invariant.)

- [ ] **Step 3.2: Run test — verify it passes immediately**

```bash
cargo test -p kebab-app --lib stale_cursor_classifies_correctly
```

Expected: PASS (no implementation needed — classify already returns "unknown" for unrecognized errors).

- [ ] **Step 3.3: Document the convention**

Add a comment near the top of `crates/kebab-app/src/error_wire.rs`:

```rust
// p9-fb-34: `stale_cursor` is constructed directly by `cursor::decode`
// instead of routed through `classify`. Keep that contract — adding a
// classify branch would create two sources of truth for the same code.
```

- [ ] **Step 3.4: Commit**

```bash
git add crates/kebab-app/src/error_wire.rs
git commit -m "$(cat <<'EOF'
docs(error_wire): note stale_cursor convention (fb-34)

stale_cursor is built by cursor::decode, not classify. Test
locks the invariant.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: `App::search_with_opts` + `SearchResponse`

**Files:**
- Modify: `crates/kebab-app/src/app.rs`
- Modify: `crates/kebab-app/src/lib.rs`

- [ ] **Step 4.1: Write the failing integration test (passthrough)**

Create `crates/kebab-app/tests/search_budget_integration.rs`:

```rust
//! p9-fb-34: App::search_with_opts integration tests.

mod common;

use kebab_app::SearchResponse;
use kebab_core::{SearchFilters, SearchMode, SearchOpts, SearchQuery};

fn lex(text: &str, k: usize) -> SearchQuery {
    SearchQuery {
        text: text.to_string(),
        mode: SearchMode::Lexical,
        k,
        filters: SearchFilters::default(),
    }
}

#[test]
fn search_with_opts_no_budget_matches_search() {
    let env = common::TestEnv::new();
    common::ingest_md(&env, "a.md", "# T\n\napples are red\n");
    let app = env.app();

    let baseline = app.search(lex("apples", 5)).unwrap();
    let resp: SearchResponse = app
        .search_with_opts(lex("apples", 5), SearchOpts::default())
        .unwrap();

    assert_eq!(resp.hits.len(), baseline.len());
    assert!(!resp.truncated);
    assert!(resp.next_cursor.is_none(), "k=5 against 1 doc → no next page");
}
```

- [ ] **Step 4.2: Run — verify failure**

```bash
cargo test -p kebab-app --test search_budget_integration search_with_opts_no_budget_matches_search
```

Expected: FAIL — `cannot find type SearchResponse` / `method search_with_opts`.

- [ ] **Step 4.3: Define `SearchResponse` + skeleton `search_with_opts`**

In `crates/kebab-app/src/app.rs`, after the existing `pub use kebab_core::{...};` imports and before the `App` struct (or wherever public types belong), add:

```rust
/// p9-fb-34: top-level wrapper around a paginated, budget-limited
/// search result. Mirrors the wire `search_response.v1` shape.
#[derive(Clone, Debug)]
pub struct SearchResponse {
    pub hits: Vec<SearchHit>,
    pub next_cursor: Option<String>,
    pub truncated: bool,
}
```

Then in `impl App`, add:

```rust
/// p9-fb-34: budget-aware search facade. Returns hits trimmed to
/// `opts.max_tokens` (chars/4 approximation) plus pagination
/// metadata. `App::search` is now a thin wrapper that drops the
/// metadata for backwards compat.
pub fn search_with_opts(
    &self,
    query: SearchQuery,
    opts: SearchOpts,
) -> Result<SearchResponse> {
    use crate::cursor;

    let corpus_revision = self.sqlite.corpus_revision().to_string();
    let offset = match opts.cursor.as_ref() {
        Some(c) => cursor::decode(c, &corpus_revision)
            .map_err(|e| anyhow::anyhow!("stale_cursor: {}", e.message))?,
        None => 0,
    };

    let snippet_chars = opts
        .snippet_chars
        .unwrap_or(self.config.search.snippet_chars);

    // Fetch enough to satisfy offset + requested page.
    let k_effective = query.k.max(self.config.search.default_k);
    let fetch_k = offset.saturating_add(k_effective);
    let fetch_query = SearchQuery {
        k: fetch_k,
        ..query.clone()
    };
    let mut all_hits = self.search(fetch_query)?;

    // Skip offset.
    let drop_n = offset.min(all_hits.len());
    all_hits.drain(..drop_n);
    let mut hits: Vec<SearchHit> = all_hits.into_iter().take(k_effective).collect();

    // Apply snippet_chars override (production search already used
    // config snippet_chars; this re-trims if the override is shorter).
    if opts.snippet_chars.is_some() {
        for h in hits.iter_mut() {
            if h.snippet.chars().count() > snippet_chars {
                h.snippet = trim_to_chars(&h.snippet, snippet_chars);
            }
        }
    }

    // Budget loop.
    let mut truncated = false;
    if let Some(max_tokens) = opts.max_tokens {
        let max_chars = max_tokens.saturating_mul(4);
        // Step 1: shorten snippets progressively to a 60-char floor.
        const SNIPPET_FLOOR: usize = 60;
        let mut current_snippet_cap = snippet_chars;
        while estimate_chars(&hits) > max_chars && current_snippet_cap > SNIPPET_FLOOR {
            current_snippet_cap = (current_snippet_cap / 2).max(SNIPPET_FLOOR);
            for h in hits.iter_mut() {
                if h.snippet.chars().count() > current_snippet_cap {
                    h.snippet = trim_to_chars(&h.snippet, current_snippet_cap);
                    truncated = true;
                }
            }
        }
        // Step 2: pop hits from the end until we fit, but always keep ≥ 1.
        while estimate_chars(&hits) > max_chars && hits.len() > 1 {
            hits.pop();
            truncated = true;
        }
    }

    // Compute next_cursor: did we have more in the original fetch?
    let returned = hits.len();
    let next_cursor = if returned == k_effective && offset.saturating_add(returned) > 0 {
        // Speculative: the retriever returned exactly k_effective hits
        // after offset, so there *might* be more. Encoding the cursor
        // is cheap; the next call falls through to an empty page if
        // nothing remains.
        Some(cursor::encode(offset + returned, &corpus_revision))
    } else if truncated && returned > 0 {
        // Budget-truncated mid-page; let the caller resume from where
        // we stopped.
        Some(cursor::encode(offset + returned, &corpus_revision))
    } else {
        None
    };

    Ok(SearchResponse {
        hits,
        next_cursor,
        truncated,
    })
}
```

Add the helpers near the bottom of `app.rs` (or in `cursor.rs` if cleaner — keep them adjacent to where they're called):

```rust
/// p9-fb-34: trim to N chars (Unicode-safe).
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

/// p9-fb-34: estimate wire JSON char cost of the hit list. The wire
/// shape adds object/array boilerplate (~50 chars per hit), so we
/// approximate by serializing each hit and summing chars. Cheap
/// enough to call inside the budget loop on small k.
fn estimate_chars(hits: &[SearchHit]) -> usize {
    hits.iter()
        .map(|h| serde_json::to_string(h).map(|s| s.len()).unwrap_or(0))
        .sum()
}
```

- [ ] **Step 4.4: Run passthrough test — verify pass**

```bash
cargo test -p kebab-app --test search_budget_integration search_with_opts_no_budget_matches_search
```

Expected: PASS.

- [ ] **Step 4.5: Re-export `SearchResponse`**

Edit `crates/kebab-app/src/lib.rs`:

```rust
pub use app::{App, SearchResponse};
```

(The existing `pub use app::App;` line gains `SearchResponse`.)

- [ ] **Step 4.6: Add budget-shorten test**

Append to `crates/kebab-app/tests/search_budget_integration.rs`:

```rust
#[test]
fn budget_truncates_snippets_when_below_threshold() {
    let env = common::TestEnv::new();
    // Long body so snippet has room to shrink.
    let body: String = "rust ownership is a memory model. ".repeat(10);
    common::ingest_md(&env, "a.md", &format!("# T\n\n{body}\n"));
    let app = env.app();

    let unrestricted = app.search(lex("rust", 5)).unwrap();
    let unrestricted_chars: usize = unrestricted.iter().map(|h| h.snippet.chars().count()).sum();

    let resp = app
        .search_with_opts(
            lex("rust", 5),
            SearchOpts {
                max_tokens: Some(50), // ~200 chars total cap, well under unrestricted
                snippet_chars: None,
                cursor: None,
            },
        )
        .unwrap();
    let limited_chars: usize = resp.hits.iter().map(|h| h.snippet.chars().count()).sum();

    assert!(resp.truncated, "small budget must trip truncation");
    assert!(limited_chars < unrestricted_chars, "snippet should shrink");
    assert!(!resp.hits.is_empty(), "always retain ≥1 hit");
}
```

- [ ] **Step 4.7: Run + verify**

```bash
cargo test -p kebab-app --test search_budget_integration
```

Expected: 2 PASS.

- [ ] **Step 4.8: Add cursor-pagination + stale-cursor tests**

Append to `crates/kebab-app/tests/search_budget_integration.rs`:

```rust
#[test]
fn cursor_paginates_to_next_page() {
    let env = common::TestEnv::new();
    // Seed N docs so k=2 returns multiple pages.
    for i in 0..6 {
        common::ingest_md(&env, &format!("d{i}.md"), &format!("# T{i}\n\nrust topic {i}\n"));
    }
    let app = env.app();

    let page1 = app
        .search_with_opts(lex("rust", 2), SearchOpts::default())
        .unwrap();
    assert_eq!(page1.hits.len(), 2);
    let cursor = page1.next_cursor.expect("more hits available");

    let page2 = app
        .search_with_opts(
            lex("rust", 2),
            SearchOpts {
                max_tokens: None,
                snippet_chars: None,
                cursor: Some(cursor),
            },
        )
        .unwrap();
    assert_eq!(page2.hits.len(), 2);
    // Second page must contain different hits than first.
    let p1_ids: std::collections::HashSet<_> = page1.hits.iter().map(|h| h.chunk_id.0.clone()).collect();
    let p2_ids: std::collections::HashSet<_> = page2.hits.iter().map(|h| h.chunk_id.0.clone()).collect();
    assert!(p1_ids.is_disjoint(&p2_ids), "page 2 must not repeat page 1 hits");
}

#[test]
fn cursor_rejected_after_corpus_revision_bump() {
    let env = common::TestEnv::new();
    common::ingest_md(&env, "a.md", "# T\n\napples\n");
    let app = env.app();

    let page1 = app
        .search_with_opts(lex("apples", 1), SearchOpts::default())
        .unwrap();
    let cursor = page1.next_cursor;

    if let Some(c) = cursor {
        // Force a corpus_revision bump.
        common::ingest_md(&env, "b.md", "# B\n\nbananas\n");
        let app2 = env.app(); // re-open to pick up new revision

        let result = app2.search_with_opts(
            lex("apples", 1),
            SearchOpts {
                max_tokens: None,
                snippet_chars: None,
                cursor: Some(c),
            },
        );
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("stale_cursor"),
            "must surface stale_cursor: {err}"
        );
    }
    // If page1 had no next_cursor (k=1 and only 1 doc), this branch
    // is unreachable but the test still passes — exercises the
    // happy-no-cursor path.
}
```

- [ ] **Step 4.9: Run + verify**

```bash
cargo test -p kebab-app --test search_budget_integration
```

Expected: 4 PASS.

If `common::TestEnv::app()` returns a freshly-built `App` each call, the corpus_revision bump test works. If it caches, you may need a `env.reopen_app()` helper — extend `tests/common/mod.rs`.

- [ ] **Step 4.10: Verify existing `App::search` callers still work**

```bash
cargo test -p kebab-app
cargo build --workspace
```

Expected: green. `App::search` signature unchanged so TUI / kebab-rag callers compile.

- [ ] **Step 4.11: Commit**

```bash
git add crates/kebab-app/src/app.rs crates/kebab-app/src/lib.rs crates/kebab-app/tests/search_budget_integration.rs
git commit -m "$(cat <<'EOF'
feat(app): App::search_with_opts + SearchResponse (fb-34)

Budget loop: snippet shorten → k pop → ≥1 hit floor. Cursor
encode/decode threads corpus_revision; mismatch surfaces as
stale_cursor anyhow error. App::search retained as thin wrapper.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: Wire schema — `search_response.v1`

**Files:**
- Create: `docs/wire-schema/v1/search_response.schema.json`

- [ ] **Step 5.1: Write the schema**

Create `docs/wire-schema/v1/search_response.schema.json`:

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://kb.local/wire/v1/search_response.schema.json",
  "title": "SearchResponse v1",
  "description": "Top-level wrapper for `kebab search --json` output. Replaces the bare `search_hit.v1[]` array — wraps it with pagination + truncation metadata. Token counts are approximate (chars/4 estimate, no tokenizer dep).",
  "type": "object",
  "required": ["schema_version", "hits", "next_cursor", "truncated"],
  "properties": {
    "schema_version": { "const": "search_response.v1" },
    "hits":           { "type": "array", "description": "search_hit.v1[]" },
    "next_cursor":    { "type": ["string", "null"], "description": "Opaque base64 cursor for next page; null when no more hits." },
    "truncated":      { "type": "boolean", "description": "True when budget forced snippet shortening or k reduction. Caller can request next page via next_cursor or pass higher k." }
  }
}
```

- [ ] **Step 5.2: Validate**

```bash
python3 -c "import json; json.load(open('docs/wire-schema/v1/search_response.schema.json'))"
```

Expected: silent success.

- [ ] **Step 5.3: Commit**

```bash
git add docs/wire-schema/v1/search_response.schema.json
git commit -m "$(cat <<'EOF'
feat(wire): search_response.v1 schema (fb-34)

Wrapper around search_hit.v1[] with next_cursor + truncated.
Wire breaking — agent that parses bare array must adapt.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: CLI `--max-tokens` / `--snippet-chars` / `--cursor`

**Files:**
- Modify: `crates/kebab-cli/src/main.rs`
- Modify: `crates/kebab-cli/src/wire.rs`

- [ ] **Step 6.1: Add `wire_search_response` helper**

Locate `crates/kebab-cli/src/wire.rs`. After `wire_search_hits`, append:

```rust
/// p9-fb-34: tag a `SearchResponse` as `search_response.v1`. Wraps
/// the existing `search_hit.v1[]` array with pagination + truncation
/// metadata.
pub fn wire_search_response(r: &kebab_app::SearchResponse) -> Value {
    let v = serde_json::json!({
        "hits": r.hits.iter().map(wire_search_hit).collect::<Vec<_>>(),
        "next_cursor": r.next_cursor,
        "truncated": r.truncated,
    });
    tag_object(v, "search_response.v1")
}
```

- [ ] **Step 6.2: Add clap flags + dispatch**

Locate the `Cmd::Search` enum variant in `crates/kebab-cli/src/main.rs`:

```bash
grep -n "Cmd::Search" crates/kebab-cli/src/main.rs | head -3
```

Add three new fields to the variant definition (the `enum Cmd { ... Search { query, k, mode, explain, no_cache, ... } }` block):

```rust
/// p9-fb-34: cap result wire JSON size at approximately N tokens
/// (chars/4 estimate). When set, smaller snippets and fewer hits
/// may be returned; check `truncated` in the JSON wire.
#[arg(long)]
max_tokens: Option<usize>,
/// p9-fb-34: per-hit snippet character cap, overrides
/// `config.search.snippet_chars` for this call only.
#[arg(long)]
snippet_chars: Option<usize>,
/// p9-fb-34: opaque cursor from a previous response's
/// `next_cursor` to fetch the next page. Mismatched
/// `corpus_revision` returns `error.v1.code = stale_cursor`.
#[arg(long)]
cursor: Option<String>,
```

In the match arm, replace the existing dispatch (around `Cmd::Search { query, k, mode, explain: _, no_cache } =>`):

```rust
Cmd::Search {
    query,
    k,
    mode,
    explain: _,
    no_cache,
    max_tokens,
    snippet_chars,
    cursor,
} => {
    let cfg = kebab_config::Config::load(cli.config.as_deref())?;
    let q = kebab_core::SearchQuery {
        text: query.clone(),
        mode: (*mode).into(),
        k: *k,
        filters: kebab_core::SearchFilters::default(),
    };
    let opts = kebab_core::SearchOpts {
        max_tokens: *max_tokens,
        snippet_chars: *snippet_chars,
        cursor: cursor.clone(),
    };
    // p9-fb-34: budget-aware path. --no-cache still bypasses the
    // App-level LRU; wire wrapper applies regardless.
    let app = kebab_app::App::open_with_config(cfg)?;
    let resp = if *no_cache {
        // search_uncached_with_opts not exposed; degrade by
        // clearing cache then calling search_with_opts.
        app.clear_search_cache();
        app.search_with_opts(q, opts)?
    } else {
        app.search_with_opts(q, opts)?
    };

    if cli.json {
        println!("{}", serde_json::to_string(&wire::wire_search_response(&resp))?);
    } else {
        // Plain output unchanged — list hits with [stale] tag
        // (fb-32) per existing convention. Truncation hint goes
        // to stderr so it doesn't pollute stdout.
        use std::io::IsTerminal;
        let color = std::io::stdout().is_terminal();
        for h in &resp.hits {
            let heading = if h.heading_path.is_empty() {
                String::new()
            } else {
                format!("  >  {}", h.heading_path.join(" / "))
            };
            let stale_tag = if h.stale {
                if color { "\x1b[33m[stale]\x1b[0m " } else { "[stale] " }
            } else {
                ""
            };
            println!(
                "{:>2}. {:.4}  {}{}{}",
                h.rank, h.retrieval.fusion_score, stale_tag, h.doc_path.0, heading,
            );
        }
        if resp.truncated {
            let next = resp.next_cursor.as_deref().unwrap_or("(none)");
            eprintln!("[truncated; use --cursor {next} for the next page]");
        }
    }
    Ok(())
}
```

If the existing path uses `kebab_app::search_with_config` / `search_uncached_with_config` (free functions rather than `App::open_with_config`), grep for the actual idiom:

```bash
grep -n "kebab_app::search\|App::open_with_config" crates/kebab-cli/src/main.rs | head -5
```

Adapt the dispatch to match — the goal is `App::search_with_opts(query, opts)`. If a `*_with_opts_with_config` free function is preferred, add it to `crates/kebab-app/src/lib.rs` mirroring the existing `search_with_config` shape:

```rust
pub fn search_with_opts_with_config(
    config: kebab_config::Config,
    query: SearchQuery,
    opts: SearchOpts,
) -> anyhow::Result<SearchResponse> {
    App::open_with_config(config)?.search_with_opts(query, opts)
}
```

- [ ] **Step 6.3: Build the CLI**

```bash
cargo build -p kebab-cli
```

Expected: clean.

- [ ] **Step 6.4: Verify --help shows the new flags**

```bash
cargo run -q -p kebab-cli -- search --help 2>&1 | grep -E "max-tokens|snippet-chars|cursor"
```

Expected: 3 lines, one per flag.

- [ ] **Step 6.5: Run kebab-cli existing tests**

```bash
cargo test -p kebab-cli
```

Expected: existing tests pass. If a wire test asserts the OLD bare `search_hit.v1[]` shape, it will fail — update those tests now to expect `search_response.v1`. Search:

```bash
grep -rn "search_hit.v1\|wire_search_hits" crates/kebab-cli/tests/
```

For each match, decide:
- If the test verifies `kebab search --json` stdout → update to expect `search_response.v1` wrapper.
- If the test only verifies a single hit's wire shape (still part of the wrapper) → no change.

- [ ] **Step 6.6: Commit**

```bash
git add crates/kebab-cli/src/main.rs crates/kebab-cli/src/wire.rs crates/kebab-app/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(cli): kebab search --max-tokens / --snippet-chars / --cursor (fb-34)

JSON output wrapped in search_response.v1 (breaking — agent must
adapt). Plain output unchanged + [truncated; use --cursor X]
stderr hint when budget tripped.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: CLI integration tests

**Files:**
- Create: `crates/kebab-cli/tests/wire_search_response.rs`

- [ ] **Step 7.1: Inspect existing common helpers**

```bash
sed -n '1,50p' crates/kebab-cli/tests/common/mod.rs
```

Existing fb-32 / fb-33 helpers: `write_config(cfg, ws)`, `ingest`, `run_search_json`, etc. Mirror these.

- [ ] **Step 7.2: Add `run_search` helper for arbitrary args**

If a generic search runner doesn't exist, append to `crates/kebab-cli/tests/common/mod.rs`:

```rust
/// p9-fb-34: invoke `kebab search` with arbitrary flags, capture
/// stdout + stderr.
pub fn run_search_with_args(cfg: &std::path::Path, args: &[&str]) -> (String, String) {
    let exe = env!("CARGO_BIN_EXE_kebab");
    let mut cmd_args: Vec<&str> = vec!["--config"];
    let cfg_str = cfg.to_str().expect("utf8");
    cmd_args.push(cfg_str);
    cmd_args.push("search");
    cmd_args.extend(args);
    let out = std::process::Command::new(exe)
        .args(&cmd_args)
        .output()
        .expect("kebab search");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}
```

Adapt to whatever signature the existing helpers use.

- [ ] **Step 7.3: Write the integration tests**

Create `crates/kebab-cli/tests/wire_search_response.rs`:

```rust
//! p9-fb-34: CLI search wire wrapper + budget controls.

mod common;

use serde_json::Value;

#[test]
fn search_json_emits_search_response_v1_wrapper() {
    let (cfg, ws) = common::write_config();
    common::ingest(&cfg, &ws, "a.md", "# T\n\napples are red.\n");
    let (stdout, _stderr) = common::run_search_with_args(
        &cfg,
        &["--mode", "lexical", "--json", "apples"],
    );
    let v: Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("not JSON: {stdout:?}: {e}"));
    assert_eq!(v["schema_version"], "search_response.v1");
    assert!(v["hits"].is_array(), "hits must be array");
    assert!(v["next_cursor"].is_null() || v["next_cursor"].is_string());
    assert!(v["truncated"].is_boolean());
}

#[test]
fn search_json_truncates_with_max_tokens() {
    let (cfg, ws) = common::write_config();
    let body: String = "rust ownership is a memory model. ".repeat(10);
    common::ingest(&cfg, &ws, "a.md", &format!("# T\n\n{body}\n"));
    let (stdout, _stderr) = common::run_search_with_args(
        &cfg,
        &["--mode", "lexical", "--json", "--max-tokens", "30", "rust"],
    );
    let v: Value = serde_json::from_str(stdout.trim()).expect("json");
    assert_eq!(v["truncated"], true, "30 tokens cap must trip truncation");
}

#[test]
fn search_json_cursor_paginates() {
    let (cfg, ws) = common::write_config();
    for i in 0..6 {
        common::ingest(&cfg, &ws, &format!("d{i}.md"), &format!("# T{i}\n\nrust topic {i}\n"));
    }
    let (page1, _) = common::run_search_with_args(
        &cfg,
        &["--mode", "lexical", "--json", "-k", "2", "rust"],
    );
    let v1: Value = serde_json::from_str(page1.trim()).expect("json");
    let cursor = v1["next_cursor"].as_str().expect("next_cursor present");

    let (page2, _) = common::run_search_with_args(
        &cfg,
        &["--mode", "lexical", "--json", "-k", "2", "--cursor", cursor, "rust"],
    );
    let v2: Value = serde_json::from_str(page2.trim()).expect("json");
    let p1_ids: Vec<_> = v1["hits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["chunk_id"].as_str().unwrap().to_string())
        .collect();
    let p2_ids: Vec<_> = v2["hits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["chunk_id"].as_str().unwrap().to_string())
        .collect();
    assert!(p2_ids.iter().all(|id| !p1_ids.contains(id)),
        "page 2 must not repeat page 1");
}

#[test]
fn search_plain_emits_truncated_hint_to_stderr() {
    let (cfg, ws) = common::write_config();
    let body: String = "rust ownership is a memory model. ".repeat(10);
    common::ingest(&cfg, &ws, "a.md", &format!("# T\n\n{body}\n"));
    let (_stdout, stderr) = common::run_search_with_args(
        &cfg,
        &["--mode", "lexical", "--max-tokens", "30", "rust"],
    );
    assert!(
        stderr.contains("[truncated;"),
        "stderr must carry truncated hint: {stderr:?}"
    );
}
```

If `common::write_config()` doesn't exist with the exact signature, look at how `wire_search_stale.rs` calls it (fb-32) and mirror.

- [ ] **Step 7.4: Build + run**

```bash
cargo test -p kebab-cli --test wire_search_response 2>&1 | tail -20
```

Expected: 4 PASS. (Lexical-only, no Ollama gate needed.)

- [ ] **Step 7.5: Verify full kebab-cli suite**

```bash
cargo test -p kebab-cli
```

Expected: all PASS.

- [ ] **Step 7.6: Commit**

```bash
git add crates/kebab-cli/tests/
git commit -m "$(cat <<'EOF'
test(cli): wire_search_response + budget integration (fb-34)

4 lexical-only tests covering search_response.v1 wrapper shape,
--max-tokens truncation, --cursor pagination, plain stderr hint.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 8: MCP search tool — wrapper + new inputs

**Files:**
- Modify: `crates/kebab-mcp/src/tools/search.rs`
- Possibly modify: `crates/kebab-mcp/tests/tools_call_search.rs`

- [ ] **Step 8.1: Inspect current MCP search tool**

```bash
sed -n '1,80p' crates/kebab-mcp/src/tools/search.rs
```

Note the existing `SearchInput` shape and the wire-tag pattern used for the response.

- [ ] **Step 8.2: Extend `SearchInput`**

Add 3 optional fields:

```rust
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SearchInput {
    pub query: String,
    pub mode: Option<String>,
    pub k: Option<usize>,
    /// p9-fb-34: cap result wire size at ~N tokens (chars/4 estimate).
    pub max_tokens: Option<usize>,
    /// p9-fb-34: per-hit snippet character cap.
    pub snippet_chars: Option<usize>,
    /// p9-fb-34: opaque cursor from a previous response.
    pub cursor: Option<String>,
}
```

- [ ] **Step 8.3: Switch dispatch to `search_with_opts`**

In `handle(state, input)`, replace the existing `search_with_config(...)` call with:

```rust
let opts = kebab_core::SearchOpts {
    max_tokens: input.max_tokens,
    snippet_chars: input.snippet_chars,
    cursor: input.cursor,
};
let cfg_clone = (*state.config).clone();
let result = kebab_app::search_with_opts_with_config(cfg_clone, query, opts);
```

(Use whatever wrapper free function shape `kebab-app` provides per Task 6 Step 6.2.)

For the success branch, serialize `SearchResponse` and tag with `search_response.v1`:

```rust
match result {
    Ok(resp) => {
        let v = serde_json::json!({
            "schema_version": "search_response.v1",
            "hits": resp.hits.iter().map(serde_json::to_value).collect::<Result<Vec<_>, _>>()?,
            "next_cursor": resp.next_cursor,
            "truncated": resp.truncated,
        });
        match serde_json::to_string(&v) {
            Ok(json) => to_tool_success(json),
            Err(e) => to_tool_error(&anyhow::anyhow!(e)),
        }
    }
    Err(e) => to_tool_error(&e),
}
```

If the existing handler returns `Result<CallToolResult, ErrorData>` rather than `CallToolResult` directly, adapt.

- [ ] **Step 8.4: Update the MCP search test**

Open `crates/kebab-mcp/tests/tools_call_search.rs`. The existing test likely asserts `search_hit.v1` on the response array. Update to expect the new wrapper:

```rust
// (the existing assertions for individual hits stay; add wrapper assertions)
let v: serde_json::Value = serde_json::from_str(&body).expect("json");
assert_eq!(v["schema_version"], "search_response.v1");
assert!(v["hits"].is_array());
```

If the test asserted `arr.as_array().first()` on what was a top-level array, change to `v["hits"].as_array().unwrap().first()`.

- [ ] **Step 8.5: Run MCP tests**

```bash
cargo test -p kebab-mcp
```

Expected: all PASS.

- [ ] **Step 8.6: Commit**

```bash
git add crates/kebab-mcp/
git commit -m "$(cat <<'EOF'
feat(mcp): search tool emits search_response.v1 + budget inputs (fb-34)

SearchInput gains max_tokens / snippet_chars / cursor (all optional).
Output wrapped in search_response.v1 to match CLI; existing
tools_call_search test updated to read v["hits"] instead of the bare
array.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 9: Workspace test + clippy gate

- [ ] **Step 9.1: Workspace test**

```bash
cargo test --workspace --no-fail-fast -j 1 2>&1 | tail -30
```

Expected: all PASS.

If any other crate (kebab-tui, kebab-eval, etc.) hits compile errors due to the `App::search` API surface change, that signals the change wasn't backwards-compatible. Verify `App::search` signature is unchanged (still `Vec<SearchHit>`).

- [ ] **Step 9.2: Clippy**

```bash
cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -10
```

Expected: clean. Common new warnings to watch:
- `clippy::needless_pass_by_value` on cursor params — adjust as flagged.
- `clippy::large_struct_passed_by_value` if `SearchOpts` grows — currently 3 small Options.

- [ ] **Step 9.3: Commit clippy fixes if needed**

```bash
git add -A
git commit -m "chore: clippy fixes for fb-34"
```

(Skip if no fixes were necessary.)

---

## Task 10: Documentation updates

**Files:**
- Modify: `README.md`
- Modify: `docs/SMOKE.md`
- Modify: `tasks/p9/p9-fb-34-output-budget-controls.md`
- Modify: `tasks/INDEX.md`
- Modify: `tasks/HOTFIXES.md`
- Modify: `integrations/claude-code/kebab/SKILL.md`

- [ ] **Step 10.1: README — search row update**

Find the `kebab search` row in the 명령 table:

```bash
grep -n "kebab search" README.md | head -3
```

Append `--max-tokens`, `--snippet-chars`, `--cursor` to the flag list and add a one-liner about wire shape change. Example:

```markdown
| `kebab search "<query>" [--mode lexical|vector|hybrid] [--max-tokens N] [--snippet-chars N] [--cursor <opaque>]` | (existing description) **`--max-tokens` / `--snippet-chars` / `--cursor` (p9-fb-34)** — agent budget controls. `--json` 출력은 `search_response.v1` wrapper (`{hits, next_cursor, truncated}`) — pre-fb-34 의 bare array 와 호환 안 됨. |
```

- [ ] **Step 10.2: SMOKE.md — pagination walkthrough**

Append a section after the existing search section (and after the fb-32 / fb-33 sections):

```markdown
### Pagination + budget (fb-34)

```bash
# First page
kebab search "rust" --json -k 5 > page1.json
jq '.next_cursor' page1.json

# Next page using the returned cursor
NEXT=$(jq -r '.next_cursor' page1.json)
kebab search "rust" --json -k 5 --cursor "$NEXT" > page2.json

# Budget cap — returns smaller snippet / fewer hits + truncated=true
kebab search "rust" --json --max-tokens 200 | jq '.truncated, (.hits | length)'
```

`next_cursor` 는 corpus_revision 변경 (이후 ingest 등) 시 invalid — 다음 호출이 `error.v1.code = stale_cursor` 로 거절. agent 는 새 search 로 재발급 받기.
```

- [ ] **Step 10.3: Task spec status flip**

Edit `tasks/p9/p9-fb-34-output-budget-controls.md`:

```diff
-status: open
+status: completed
```

Replace the `> ⏳ **백로그 only — 미구현.**` block with:

```markdown
> ✅ **구현 완료.** 본 spec 은 구현 시점의 frozen 상태. post-merge deviation 은 [HOTFIXES.md](../HOTFIXES.md) 의 `2026-05-09 — p9-fb-34` 항목 참조 — live source of truth.

상세 설계: `docs/superpowers/specs/2026-05-09-p9-fb-34-output-budget-controls-design.md`.
구현 계획: `docs/superpowers/plans/2026-05-09-p9-fb-34-output-budget-controls.md`.
```

- [ ] **Step 10.4: tasks/INDEX.md**

```diff
-    - [p9-fb-34 output budget controls](p9/p9-fb-34-output-budget-controls.md) — ⏳ 미구현, brainstorm 필요
+    - [p9-fb-34 output budget controls](p9/p9-fb-34-output-budget-controls.md) — ✅ 머지 + v0.5.0 cut 후보 (2026-05-09)
```

- [ ] **Step 10.5: HOTFIXES — wire breaking decision log**

Add a new entry near the top of dated entries in `tasks/HOTFIXES.md`:

```markdown
## 2026-05-09 — p9-fb-34: search wire wrapped in search_response.v1

**무엇이 바뀌었나**: `kebab search --json` stdout 이 기존 `search_hit.v1[]` 배열에서 신규 `search_response.v1` object 로 교체. wrapper 가 `hits`, `next_cursor`, `truncated` 세 필드를 가짐.

**Spec contract 와의 관계**: 명시적 wire breaking change. spec `docs/superpowers/specs/2026-05-09-p9-fb-34-output-budget-controls-design.md` 의 §Wire shape 절에 단일 출처 결정.

**의식적 결정**:
- pagination + truncation metadata 를 `search_hit` 자체에 흡수하면 단일 hit 의 도메인 의미가 오염됨 (모든 hit 가 `next_cursor` 필드 보유 등). top-level wrapper 가 분리도 깨끗.
- 외부 consumer 영향: 단일 사용자 환경 + Claude Code skill 한 곳. skill 은 fb-34 와 동시 갱신.
- 이 변경은 search_hit.v1 자체 schema 는 손대지 않음 — 도메인 stable.

**영향 받는 consumer**: kebab-tui (Search 패널 — 변경 불필요, App::search 시그니처 보존), kebab-mcp (search tool — 같은 PR 에서 갱신), Claude Code skill (같은 PR 에서 갱신). 외부 producer/consumer 없음.
```

- [ ] **Step 10.6: SKILL.md — recipe + cursor example**

Edit `integrations/claude-code/kebab/SKILL.md`. Find the search recipes / parsing tips and update:
- Recipe A / B / C: `response.hits[]` instead of bare array. Example:
  ```jq
  jq '.hits[] | {rank, doc_path, heading: .heading_path[-1], snippet}'
  ```
- Add a "Pagination" subsection under Parsing tips:
  ```markdown
  - `search_response.v1.next_cursor` — opaque base64. Pass back as `--cursor` (CLI) or `cursor` (MCP `mcp__kebab__search` input) for the next page. `null` when no more hits. `corpus_revision` mismatch returns `error.v1.code = stale_cursor` — re-issue the search to obtain a fresh cursor.
  - `search_response.v1.truncated` — true when `--max-tokens` (CLI) / `max_tokens` (MCP) forced snippet shortening or k reduction. Either widen the budget or paginate via `next_cursor`.
  ```

- [ ] **Step 10.7: Commit docs**

```bash
git add README.md docs/SMOKE.md tasks/p9/p9-fb-34-output-budget-controls.md tasks/INDEX.md tasks/HOTFIXES.md integrations/claude-code/kebab/SKILL.md
git commit -m "$(cat <<'EOF'
docs(fb-34): README + SMOKE + INDEX + HOTFIXES + skill notes

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 11: Smoke + push + PR

- [ ] **Step 11.1: Manual smoke**

```bash
cd /tmp/kebab-smoke   # existing scratch dir from prior tasks
~/Workspace/projects/kebab/target/release/kebab --config /tmp/kebab-smoke/config.toml ingest
~/Workspace/projects/kebab/target/release/kebab --config /tmp/kebab-smoke/config.toml search "test" --json | jq '{schema_version, truncated, next_cursor, hit_count: (.hits | length)}'
~/Workspace/projects/kebab/target/release/kebab --config /tmp/kebab-smoke/config.toml search "test" --json --max-tokens 30 | jq '.truncated'
```

Expected:
- First call: `schema_version: "search_response.v1"`, `truncated: false`, `hit_count > 0`.
- Second call: `truncated: true`.

- [ ] **Step 11.2: Final workspace test**

```bash
cd ~/Workspace/projects/kebab
cargo test --workspace --no-fail-fast -j 1
```

Expected: all green.

- [ ] **Step 11.3: Push branch**

```bash
git push -u origin feat/fb-34-output-budget-controls
```

- [ ] **Step 11.4: Open PR via gitea-pr**

Build the PR body at `/tmp/fb34-pr-body.md`:

```markdown
## Summary

- adds `kebab search --max-tokens / --snippet-chars / --cursor` plus the equivalent inputs on `mcp__kebab__search`
- wraps `--json` output in `search_response.v1` (`{hits, next_cursor, truncated}`) — wire breaking; agent that parses bare `search_hit.v1[]` must adapt
- token estimation = `chars/4` (no tokenizer dep); truncate priority: snippet shorten → k pop → ≥1 hit floor
- cursor = opaque base64(`{offset, corpus_revision}`); mismatch returns `error.v1.code = stale_cursor`
- ask path scope out (rag.max_context_tokens already covers it)
- TUI Search pane unchanged — `App::search` signature preserved as thin wrapper

## Test plan

- [x] `cargo test --workspace --no-fail-fast -j 1` — green
- [x] `cargo clippy --workspace --all-targets -- -D warnings` — clean
- [x] new tests:
  - `cursor` (kebab-app): encode/decode round-trip + stale_cursor mismatch (3 tests)
  - `search_budget_integration` (kebab-app): passthrough + snippet shorten + cursor pagination + corpus_revision bump (4 tests)
  - `wire_search_response` (kebab-cli): wire wrapper + max-tokens truncation + cursor pagination + plain stderr hint (4 tests)
  - `tools_call_search` (kebab-mcp): updated to assert `search_response.v1` wrapper
- [x] manual smoke per `docs/SMOKE.md` "Pagination + budget" walkthrough

## Architectural notes

- `App::search` signature unchanged → TUI / kebab-rag callers unaffected.
- `App::search_with_opts` is the new public API; CLI / MCP go through it.
- `chars/4` token estimation matches `rag::pack_context` convention.
- Cursor is opaque on purpose — internal schema may change; agent must not parse.
- Wire breaking documented in HOTFIXES `2026-05-09 — p9-fb-34`.

## Files of interest

- spec: `docs/superpowers/specs/2026-05-09-p9-fb-34-output-budget-controls-design.md`
- plan: `docs/superpowers/plans/2026-05-09-p9-fb-34-output-budget-controls.md`
- core: `crates/kebab-core/src/search.rs` (SearchOpts)
- app: `crates/kebab-app/src/{cursor,app}.rs` (SearchResponse + budget loop)
- CLI: `crates/kebab-cli/src/main.rs` (Cmd::Search), `crates/kebab-cli/src/wire.rs`
- MCP: `crates/kebab-mcp/src/tools/search.rs`
- wire: `docs/wire-schema/v1/search_response.schema.json`
```

Open the PR:

```bash
/Users/user/.claude/skills/gitea-ops/bin/gitea-pr \
  --title "feat(fb-34): output budget controls" \
  --body "$(cat /tmp/fb34-pr-body.md)" \
  --head feat/fb-34-output-budget-controls \
  --base main
```

Capture the URL.

- [ ] **Step 11.5: Cleanup**

```bash
rm /tmp/fb34-pr-body.md
```

---

## Self-review

- **Spec coverage:**
  - §Behavior contract / CLI flags → Task 6
  - §Wire shape → Task 5 (schema) + Task 6 (CLI emit) + Task 8 (MCP emit)
  - §Token estimation → Task 4 (`estimate_chars` helper using serde_json size, chars/4 conceptually)
  - §Truncate priority → Task 4 budget loop (snippet shorten → k pop → ≥1)
  - §Pagination cursor → Task 2 (encode/decode) + Task 4 (next_cursor computation) + Task 6 (CLI flag) + Task 8 (MCP input)
  - §Stale cursor error → Task 2 + Task 3
  - §Domain API change → Tasks 1, 4 (SearchOpts + SearchResponse + App::search_with_opts)
  - §Components → Tasks 1-8
  - §Test plan → Tasks 2 (cursor), 4 (App), 7 (CLI), 8 (MCP)
  - §Documentation → Task 10
  - §Risks (wire breaking, App stability, chars/4 ±15%, cursor opacity) → addressed in Task 4 (App::search preserved), Task 5 (schema description mentions approximation), Task 10 (HOTFIXES)

- **Placeholder scan:**
  - Two "if/look at" instructions in Task 6 + Task 8 — those direct the engineer to mirror existing scaffold rather than invent. Concrete fallback paths spelled out.
  - No TODO / "fill in" / "later".

- **Type consistency:**
  - `SearchOpts { max_tokens: Option<usize>, snippet_chars: Option<usize>, cursor: Option<String> }` consistent across Tasks 1, 4, 6, 8.
  - `SearchResponse { hits: Vec<SearchHit>, next_cursor: Option<String>, truncated: bool }` consistent across Tasks 4, 5, 6, 8.
  - `cursor::encode(offset, revision) -> String`, `cursor::decode(s, expected) -> Result<usize, ErrorV1>` consistent across Tasks 2, 4.
  - `error.v1.code = "stale_cursor"` consistent across spec, Task 2, Task 3, Task 10.

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-05-09-p9-fb-34-output-budget-controls.md`. Two execution options:

**1. Subagent-Driven (recommended)** — fresh subagent per task, review between tasks.

**2. Inline Execution** — execute tasks in this session.

Which approach?
