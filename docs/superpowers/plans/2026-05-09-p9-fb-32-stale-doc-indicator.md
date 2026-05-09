# p9-fb-32 — Stale Doc Indicator Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Surface "indexed-at timestamp + stale boolean" on every search hit and RAG citation so users / agents see when a doc was last re-processed and whether it crossed a configurable freshness threshold.

**Architecture:** Reuse `documents.updated_at` (already RFC3339, already excluded from fb-23 skip path → natural source-of-truth for "last re-processed"). Two retrievers (lexical, vector) JOIN documents to extract it. App facade computes `stale = now - indexed_at > threshold * 86400s`. `now` is threaded as an explicit `OffsetDateTime` parameter (no Clock trait — codebase has no precedent and the explicit-arg pattern is enough for test determinism). Wire serialization is automatic via `serde_json::to_value` on the domain types. CLI plain output gains `[stale]` tag; TUI gains `[STALE]` Warning-styled badge; agent JSON gains `indexed_at` + `stale` fields.

**Tech Stack:** Rust 2024, time crate (RFC3339), serde, rusqlite, refinery (no new migration), insta (snapshot redaction), JSON Schema (search_hit/citation v1).

**Spec:** `docs/superpowers/specs/2026-05-08-p9-fb-32-stale-doc-indicator-design.md`

---

## File Structure

| File | Responsibility | Action |
|------|----------------|--------|
| `crates/kebab-core/src/search.rs` | Domain `SearchHit` — add `indexed_at` + `stale` fields | modify |
| `crates/kebab-core/src/answer.rs` | Domain `AnswerCitation` — add `indexed_at` + `stale` fields | modify |
| `crates/kebab-config/src/lib.rs` | `SearchCfg.stale_threshold_days` field + default + env override + load-time validation | modify |
| `crates/kebab-search/src/lexical.rs` | JOIN `documents.updated_at`, parse RFC3339, populate `SearchHit.indexed_at` | modify |
| `crates/kebab-search/src/vector.rs` | Same JOIN extension in `hydrate_chunks` + populate | modify |
| `crates/kebab-app/src/staleness.rs` | New module — `compute_stale(indexed_at, now, threshold_days) -> bool` + `mark_stale_in_place(&mut [SearchHit], now, threshold_days)` | create |
| `crates/kebab-app/src/app.rs` | Call `mark_stale_in_place` after `search_uncached` AND after cache hits in `App::search`. Compute `now` once per call. | modify |
| `crates/kebab-app/src/lib.rs` (RAG path) | Compute `stale` for `AnswerCitation` items returned by `App::ask` | modify |
| `docs/wire-schema/v1/search_hit.schema.json` | Add `indexed_at` + `stale` to required + properties | modify |
| `docs/wire-schema/v1/citation.schema.json` | Add `indexed_at` + `stale` to required + properties | modify |
| `crates/kebab-cli/src/render.rs` (or equivalent plain renderer) | `[stale]` tag on hit / citation lines (TTY color when capable) | modify |
| `crates/kebab-tui/src/<search/inspect/ask panes>` | `[STALE]` Span via `Theme::style(Role::Warning)` | modify |
| `crates/kebab-app/tests/staleness.rs` | Unit tests for `compute_stale` boundary + threshold=0 | create |
| `crates/kebab-app/tests/search_stale_integration.rs` | Integration: ingest doc → fast-forward `now` → verify `stale=true` | create |
| `crates/kebab-config/src/lib.rs` (tests) | Unit: default 30, env override 7, negative → error | modify |
| `crates/kebab-cli/tests/wire_search_stale.rs` | Wire JSON contains `indexed_at` + `stale` on hits | create |
| `crates/kebab-cli/tests/wire_ask_stale.rs` | Wire JSON contains `indexed_at` + `stale` on `answer.citations[]` | create |
| `crates/kebab-tui/tests/snapshots/*` | Insta redaction filter for `indexed_at` (pattern `[indexed_at]`) | modify (existing) |
| `README.md` | Configuration section — `stale_threshold_days` line | modify |
| `docs/SMOKE.md` | Config example block + walkthrough paragraph | modify |
| `tasks/p9/p9-fb-32-stale-doc-indicator.md` | Status flip + design/plan links | modify |
| `tasks/INDEX.md` | fb-32 row → ✅ + 0.4.0 trigger note | modify |
| `integrations/claude-code/kebab/SKILL.md` | Parsing tip line about `indexed_at` / `stale` | modify |

---

## Pre-flight

- [ ] **Step 0.1: Branch off main**

```bash
git checkout main
git pull
git checkout -b feat/fb-32-stale-doc-indicator
```

- [ ] **Step 0.2: Confirm spec branch is reachable**

```bash
git log --oneline spec/fb-32-stale-doc-indicator -1
```

Expected: shows `401a47f spec(fb-32): stale doc indicator — design`. Spec lives on its own branch; the implementation branch does NOT need to merge spec since the spec file is on `main` once the spec PR lands. If spec PR not yet merged, `git merge spec/fb-32-stale-doc-indicator` first.

---

## Task 1: Domain — `SearchHit` gains `indexed_at` + `stale`

**Files:**
- Modify: `crates/kebab-core/src/search.rs`

- [ ] **Step 1.1: Write the failing test**

Append to `crates/kebab-core/src/search.rs` `#[cfg(test)]` block (create one if absent):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use time::OffsetDateTime;
    use time::macros::datetime;

    #[test]
    fn search_hit_serializes_indexed_at_and_stale() {
        let hit = SearchHit {
            rank: 1,
            chunk_id: ChunkId("c".to_string()),
            doc_id: DocumentId("d".to_string()),
            doc_path: WorkspacePath::new("a/b.md".to_string()).unwrap(),
            heading_path: vec!["H".to_string()],
            section_label: None,
            snippet: "s".to_string(),
            citation: Citation::Line {
                path: WorkspacePath::new("a/b.md".to_string()).unwrap(),
                start: 1,
                end: 1,
                section: None,
            },
            retrieval: RetrievalDetail {
                method: SearchMode::Lexical,
                fusion_score: 0.5,
                lexical_score: Some(0.5),
                vector_score: None,
                lexical_rank: Some(1),
                vector_rank: None,
            },
            index_version: IndexVersion("v1".to_string()),
            embedding_model: None,
            chunker_version: ChunkerVersion("c1".to_string()),
            indexed_at: datetime!(2026-05-09 12:00:00 UTC),
            stale: true,
        };
        let v = serde_json::to_value(&hit).unwrap();
        assert_eq!(v["indexed_at"], "2026-05-09T12:00:00Z");
        assert_eq!(v["stale"], true);
    }
}
```

- [ ] **Step 1.2: Run test — verify it fails**

```bash
cargo test -p kebab-core search_hit_serializes_indexed_at_and_stale
```

Expected: FAIL — "missing field `indexed_at`" or "no field `indexed_at` on type `SearchHit`".

- [ ] **Step 1.3: Implement — add fields to `SearchHit`**

Modify `crates/kebab-core/src/search.rs` `SearchHit` struct (the existing `pub struct SearchHit { ... }` block):

```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SearchHit {
    pub rank: u32,
    pub chunk_id: ChunkId,
    pub doc_id: DocumentId,
    pub doc_path: WorkspacePath,
    pub heading_path: Vec<String>,
    pub section_label: Option<String>,
    pub snippet: String,
    pub citation: Citation,
    pub retrieval: RetrievalDetail,
    pub index_version: IndexVersion,
    pub embedding_model: Option<EmbeddingModelId>,
    pub chunker_version: ChunkerVersion,
    /// p9-fb-32: source doc's `documents.updated_at` (last actual re-process).
    /// fb-23 incremental ingest skip path leaves this unchanged.
    #[serde(with = "time::serde::rfc3339")]
    pub indexed_at: OffsetDateTime,
    /// p9-fb-32: server-computed `now - indexed_at > threshold` per
    /// `config.search.stale_threshold_days`. `false` when threshold = 0.
    pub stale: bool,
}
```

- [ ] **Step 1.4: Run test — verify it passes**

```bash
cargo test -p kebab-core search_hit_serializes_indexed_at_and_stale
```

Expected: PASS. Other tests in the workspace will now fail to compile (every site building `SearchHit` is missing the two fields). That's expected — Tasks 4 / 5 / 7 plug them in. Do **not** add `..Default::default()` workarounds; let the compiler errors guide the next tasks.

- [ ] **Step 1.5: Commit**

```bash
git add crates/kebab-core/src/search.rs
git commit -m "$(cat <<'EOF'
feat(core): SearchHit gains indexed_at + stale (fb-32)

Domain field additions for p9-fb-32. Wire serialization is
automatic via serde rfc3339. Other crates fail to compile until
they populate the new fields — fixed in subsequent tasks.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: Domain — `AnswerCitation` gains `indexed_at` + `stale`

**Files:**
- Modify: `crates/kebab-core/src/answer.rs`

- [ ] **Step 2.1: Write the failing test**

Append to `crates/kebab-core/src/answer.rs` (create `#[cfg(test)] mod tests` if absent):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::asset::WorkspacePath;
    use crate::citation::Citation;
    use time::macros::datetime;

    #[test]
    fn answer_citation_serializes_indexed_at_and_stale() {
        let ac = AnswerCitation {
            marker: Some("[1]".to_string()),
            citation: Citation::Line {
                path: WorkspacePath::new("a.md".to_string()).unwrap(),
                start: 1,
                end: 1,
                section: None,
            },
            indexed_at: datetime!(2026-05-09 12:00:00 UTC),
            stale: false,
        };
        let v = serde_json::to_value(&ac).unwrap();
        assert_eq!(v["indexed_at"], "2026-05-09T12:00:00Z");
        assert_eq!(v["stale"], false);
    }
}
```

- [ ] **Step 2.2: Run test — verify it fails**

```bash
cargo test -p kebab-core answer_citation_serializes_indexed_at_and_stale
```

Expected: FAIL — missing fields on `AnswerCitation`.

- [ ] **Step 2.3: Implement — add fields**

Modify `crates/kebab-core/src/answer.rs` `AnswerCitation`:

```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnswerCitation {
    pub marker: Option<String>,
    pub citation: Citation,
    /// p9-fb-32: cited doc's `documents.updated_at`.
    #[serde(with = "time::serde::rfc3339")]
    pub indexed_at: OffsetDateTime,
    /// p9-fb-32: server-computed staleness flag per config threshold.
    pub stale: bool,
}
```

`OffsetDateTime` is already imported at the top of the file.

`Turn.citations` is also `Vec<AnswerCitation>` — automatically picks up the new fields.

- [ ] **Step 2.4: Run test**

```bash
cargo test -p kebab-core answer_citation_serializes_indexed_at_and_stale
```

Expected: PASS.

- [ ] **Step 2.5: Commit**

```bash
git add crates/kebab-core/src/answer.rs
git commit -m "$(cat <<'EOF'
feat(core): AnswerCitation gains indexed_at + stale (fb-32)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: Config — `SearchCfg.stale_threshold_days`

**Files:**
- Modify: `crates/kebab-config/src/lib.rs`

- [ ] **Step 3.1: Write the failing test (default + env override)**

Find the existing tests module in `crates/kebab-config/src/lib.rs` (search for `mod tests` or `#[test]`). Append:

```rust
#[test]
fn default_stale_threshold_is_30() {
    let c = Config::defaults();
    assert_eq!(c.search.stale_threshold_days, 30);
}

#[test]
fn env_override_stale_threshold() {
    let mut c = Config::defaults();
    let env: HashMap<String, String> = [
        ("KEBAB_SEARCH_STALE_THRESHOLD_DAYS".to_string(), "7".to_string()),
    ]
    .into_iter()
    .collect();
    c.apply_env(&env);
    assert_eq!(c.search.stale_threshold_days, 7);
}

#[test]
fn negative_stale_threshold_rejected_at_validation() {
    let mut c = Config::defaults();
    // u32 cannot hold a negative — represent the failure path through
    // `apply_env` parse-failure: malformed values are silently ignored
    // (existing pattern, see KEBAB_SEARCH_DEFAULT_K). For TOML-level
    // negative rejection we rely on serde's u32 type; assert that the
    // env path leaves the default in place when given garbage.
    let env: HashMap<String, String> = [
        ("KEBAB_SEARCH_STALE_THRESHOLD_DAYS".to_string(), "-5".to_string()),
    ]
    .into_iter()
    .collect();
    c.apply_env(&env);
    assert_eq!(c.search.stale_threshold_days, 30, "garbage env value must not corrupt the default");
}
```

(`HashMap` import — verify it's in scope in the existing tests module; if not, add `use std::collections::HashMap;` to the tests module.)

- [ ] **Step 3.2: Run tests — verify they fail**

```bash
cargo test -p kebab-config default_stale_threshold_is_30 env_override_stale_threshold negative_stale_threshold_rejected_at_validation
```

Expected: FAIL — no field `stale_threshold_days` on `SearchCfg`.

- [ ] **Step 3.3: Implement — add field, default, env mapping**

Modify `crates/kebab-config/src/lib.rs` `SearchCfg`:

```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SearchCfg {
    pub default_k: usize,
    pub hybrid_fusion: String,
    pub rrf_k: u32,
    pub snippet_chars: usize,
    /// p9-fb-19: in-memory LRU cache capacity for `App::search`.
    /// One entry ≈ 5 KB → default 256 caps memory at ~1.3 MB. Set
    /// to `0` to disable the cache entirely. Stale entries
    /// (corpus_revision mismatch) are evicted on next access.
    #[serde(default = "default_cache_capacity")]
    pub cache_capacity: usize,
    /// p9-fb-32: hits and citations whose source doc was last
    /// re-processed more than this many days ago are marked
    /// `stale: true` in wire / TUI / CLI surfaces. `0` disables.
    #[serde(default = "default_stale_threshold_days")]
    pub stale_threshold_days: u32,
}

fn default_stale_threshold_days() -> u32 {
    30
}
```

Also update the `Config::defaults()` literal — add `stale_threshold_days: 30,` to the `SearchCfg { ... }` block (around line 314-320).

Add the env mapping. Locate the existing `// search` comment near line 563 in `apply_env`. Append a new arm after `KEBAB_SEARCH_SNIPPET_CHARS`:

```rust
"KEBAB_SEARCH_STALE_THRESHOLD_DAYS" => {
    if let Ok(n) = v.parse::<u32>() {
        self.search.stale_threshold_days = n;
    }
}
```

(Garbage values fail `parse::<u32>()` and silently leave the default in place — matches the existing pattern documented at line 471-473.)

- [ ] **Step 3.4: Run tests — verify they pass**

```bash
cargo test -p kebab-config default_stale_threshold_is_30 env_override_stale_threshold negative_stale_threshold_rejected_at_validation
```

Expected: PASS.

- [ ] **Step 3.5: Update the test fixture TOML literal**

`crates/kebab-config/src/lib.rs` line 943-946 has the `[search]` section embedded in a fixture string. Append:

```diff
 default_k = 10
 hybrid_fusion = "rrf"
 rrf_k = 60
 snippet_chars = 220
+stale_threshold_days = 30
```

(Search the file for `default_k = 10` to find the exact spot. Verify the surrounding test still passes.)

- [ ] **Step 3.6: Run full config crate tests**

```bash
cargo test -p kebab-config
```

Expected: PASS — all tests including pre-existing ones.

- [ ] **Step 3.7: Commit**

```bash
git add crates/kebab-config/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(config): search.stale_threshold_days (fb-32)

default 30 days. env override KEBAB_SEARCH_STALE_THRESHOLD_DAYS.
Malformed env values are silently ignored, matching the existing
apply_env pattern.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: Lexical retriever — JOIN `documents.updated_at`

**Files:**
- Modify: `crates/kebab-search/src/lexical.rs`

- [ ] **Step 4.1: Write the failing test**

Append to `crates/kebab-search/tests/lexical.rs` (the integration test file):

```rust
#[test]
fn search_hit_carries_indexed_at_from_documents_updated_at() {
    let env = TestEnv::new(); // adapt to existing test scaffold in this file
    env.ingest_doc("a.md", "# T\n\nbody about apples\n");
    let hits = env.lexical_search("apples", 5);
    let hit = hits.first().expect("at least one hit");
    // updated_at is RFC3339; OffsetDateTime equality on a freshly-ingested
    // doc should be within the last 60 seconds of `now_utc()`.
    let now = time::OffsetDateTime::now_utc();
    let delta = (now - hit.indexed_at).whole_seconds().abs();
    assert!(delta < 60, "indexed_at within ±60s of now, got {delta}s");
}
```

If `tests/lexical.rs` does not have a `TestEnv` helper, examine the existing tests in that file and copy the pattern they use (likely a builder that creates a `LexicalRetriever` against a temp SQLite). The exact scaffold is dictated by what's there — adapt accordingly. Do not invent a new framework.

- [ ] **Step 4.2: Run test — verify it fails to compile**

```bash
cargo test -p kebab-search --test lexical search_hit_carries_indexed_at
```

Expected: FAIL — RawRow has no `updated_at`, hit construction missing `indexed_at` field.

- [ ] **Step 4.3: Implement — extend RawRow + SQL**

Modify `crates/kebab-search/src/lexical.rs`:

In the `RawRow` struct (line ~237), add:

```rust
struct RawRow {
    chunk_id: String,
    doc_id: String,
    bm25_raw: f64,
    snippet: String,
    heading_path_json: String,
    section_label: Option<String>,
    source_spans_json: String,
    chunker_version: String,
    workspace_path: String,
    /// p9-fb-32: documents.updated_at (RFC3339).
    updated_at: String,
}
```

In `run_query` (line ~251), extend the SELECT clause:

```rust
let mut sql = String::from(
    "SELECT \
        f.chunk_id, f.doc_id, \
        bm25(chunks_fts) AS score, \
        snippet(chunks_fts, 3, '', '', '…', ?) AS snippet, \
        c.heading_path_json, c.section_label, c.source_spans_json, \
        c.chunker_version, \
        d.workspace_path, \
        d.updated_at \
     FROM chunks_fts f \
     JOIN chunks c    ON c.chunk_id = f.chunk_id \
     JOIN documents d ON d.doc_id = f.doc_id",
);
```

In `row_from_sql` (line ~341), pull index 9:

```rust
fn row_from_sql(row: &Row<'_>) -> rusqlite::Result<RawRow> {
    Ok(RawRow {
        chunk_id: row.get(0)?,
        doc_id: row.get(1)?,
        bm25_raw: row.get(2)?,
        snippet: row.get(3)?,
        heading_path_json: row.get(4)?,
        section_label: row.get(5)?,
        source_spans_json: row.get(6)?,
        chunker_version: row.get(7)?,
        workspace_path: row.get(8)?,
        updated_at: row.get(9)?,
    })
}
```

In `build_hit` (line ~357), parse RFC3339 + populate:

```rust
let indexed_at = time::OffsetDateTime::parse(
    &raw.updated_at,
    &time::format_description::well_known::Rfc3339,
)
.context("kb-search lexical: parse documents.updated_at as RFC3339")?;

Ok(SearchHit {
    rank,
    chunk_id: ChunkId(raw.chunk_id),
    // ... existing fields ...
    chunker_version: ChunkerVersion(raw.chunker_version),
    indexed_at,
    stale: false, // placeholder — App layer overwrites
})
```

(`stale: false` is the placeholder. Task 6 owns the post-process pass that sets the real value.)

- [ ] **Step 4.4: Run test — verify it passes**

```bash
cargo test -p kebab-search --test lexical
```

Expected: PASS for new test. Existing lexical tests should also still pass.

- [ ] **Step 4.5: Commit**

```bash
git add crates/kebab-search/src/lexical.rs crates/kebab-search/tests/lexical.rs
git commit -m "$(cat <<'EOF'
feat(search/lexical): populate SearchHit.indexed_at (fb-32)

JOIN documents.updated_at. stale defaults to false; App facade
post-processes against config threshold.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: Vector retriever — extend `hydrate_chunks`

**Files:**
- Modify: `crates/kebab-search/src/vector.rs`

- [ ] **Step 5.1: Write the failing test**

Append to `crates/kebab-search/tests/hybrid.rs` (or `crates/kebab-search/tests/vector.rs` if separate — check what's there):

```rust
#[test]
fn vector_hit_carries_indexed_at() {
    let env = HybridTestEnv::new(); // adapt to existing scaffold
    env.ingest_doc("a.md", "# T\n\napples are fruit\n");
    let hits = env.vector_search("apples", 5);
    let hit = hits.first().expect("at least one vector hit");
    let now = time::OffsetDateTime::now_utc();
    let delta = (now - hit.indexed_at).whole_seconds().abs();
    assert!(delta < 60, "indexed_at within ±60s of now, got {delta}s");
}
```

- [ ] **Step 5.2: Run test — verify it fails**

```bash
cargo test -p kebab-search vector_hit_carries_indexed_at
```

Expected: FAIL — `ChunkMeta` has no `updated_at`, missing `indexed_at` on built hit.

- [ ] **Step 5.3: Implement — extend ChunkMeta + SQL + build path**

Modify `crates/kebab-search/src/vector.rs`:

`ChunkMeta` (line ~192):

```rust
struct ChunkMeta {
    text: String,
    heading_path_json: String,
    section_label: Option<String>,
    source_spans_json: String,
    chunker_version: String,
    doc_id: String,
    workspace_path: String,
    /// p9-fb-32: documents.updated_at (RFC3339).
    updated_at: String,
}
```

`hydrate_chunks` SELECT (line ~221):

```rust
let sql = format!(
    "SELECT \
        c.chunk_id, c.text, c.heading_path_json, c.section_label, \
        c.source_spans_json, c.chunker_version, \
        c.doc_id, d.workspace_path, d.updated_at \
     FROM chunks c \
     JOIN documents d ON d.doc_id = c.doc_id \
     WHERE c.chunk_id IN ({placeholders})"
);
```

`query_map` row builder (line ~244):

```rust
ChunkMeta {
    text: row.get(1)?,
    heading_path_json: row.get(2)?,
    section_label: row.get(3)?,
    source_spans_json: row.get(4)?,
    chunker_version: row.get(5)?,
    doc_id: row.get(6)?,
    workspace_path: row.get(7)?,
    updated_at: row.get(8)?,
}
```

The hit-construction site (line ~270-310 — `build SearchHit { ... }` block) — add:

```rust
let indexed_at = time::OffsetDateTime::parse(
    &meta.updated_at,
    &time::format_description::well_known::Rfc3339,
)
.context("kb-search vector: parse documents.updated_at as RFC3339")?;

SearchHit {
    // ... existing fields ...
    chunker_version: ChunkerVersion(meta.chunker_version.clone()),
    indexed_at,
    stale: false,
}
```

- [ ] **Step 5.4: Run test — verify it passes**

```bash
cargo test -p kebab-search vector_hit_carries_indexed_at
```

Expected: PASS.

- [ ] **Step 5.5: Run full crate tests**

```bash
cargo test -p kebab-search
```

Expected: all tests pass. The fusion logic in `hybrid.rs` consumes `Vec<SearchHit>` and just merges by `chunk_id` — `indexed_at` is preserved automatically by passing the hit struct through.

- [ ] **Step 5.6: Commit**

```bash
git add crates/kebab-search/src/vector.rs crates/kebab-search/tests/
git commit -m "$(cat <<'EOF'
feat(search/vector): populate SearchHit.indexed_at (fb-32)

hydrate_chunks now JOINs d.updated_at. Hybrid fusion path is
unchanged (passes SearchHit through, fields preserved).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: App facade — staleness module + post-process

**Files:**
- Create: `crates/kebab-app/src/staleness.rs`
- Modify: `crates/kebab-app/src/app.rs`
- Modify: `crates/kebab-app/src/lib.rs` (module declaration)

- [ ] **Step 6.1: Write the failing unit test**

Create `crates/kebab-app/src/staleness.rs`:

```rust
//! p9-fb-32 staleness helpers.

use time::{Duration, OffsetDateTime};

use kebab_core::SearchHit;

/// Returns `true` iff `now - indexed_at > threshold_days * 24h`.
/// `threshold_days = 0` always returns `false` (feature disabled).
/// Strict `>` so that exactly `threshold_days` old returns `false`.
pub fn compute_stale(
    indexed_at: OffsetDateTime,
    now: OffsetDateTime,
    threshold_days: u32,
) -> bool {
    if threshold_days == 0 {
        return false;
    }
    let threshold = Duration::days(i64::from(threshold_days));
    (now - indexed_at) > threshold
}

/// Sets `stale` on each hit in place using `compute_stale`.
pub fn mark_stale_in_place(
    hits: &mut [SearchHit],
    now: OffsetDateTime,
    threshold_days: u32,
) {
    for h in hits {
        h.stale = compute_stale(h.indexed_at, now, threshold_days);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    fn now() -> OffsetDateTime {
        datetime!(2026-05-09 12:00:00 UTC)
    }

    #[test]
    fn threshold_zero_always_fresh() {
        let very_old = datetime!(2020-01-01 00:00:00 UTC);
        assert!(!compute_stale(very_old, now(), 0));
    }

    #[test]
    fn just_under_threshold_is_fresh() {
        // 29 days, 23h, 59m old — under 30d.
        let indexed = now() - Duration::days(29) - Duration::hours(23) - Duration::minutes(59);
        assert!(!compute_stale(indexed, now(), 30));
    }

    #[test]
    fn exactly_threshold_is_fresh() {
        // strict `>` boundary: exactly 30d old is still fresh.
        let indexed = now() - Duration::days(30);
        assert!(!compute_stale(indexed, now(), 30));
    }

    #[test]
    fn one_minute_past_threshold_is_stale() {
        let indexed = now() - Duration::days(30) - Duration::minutes(1);
        assert!(compute_stale(indexed, now(), 30));
    }

    #[test]
    fn future_indexed_at_is_fresh() {
        // clock skew safety: future timestamps must not be stale.
        let future = now() + Duration::hours(1);
        assert!(!compute_stale(future, now(), 30));
    }
}
```

- [ ] **Step 6.2: Wire the module into the crate**

Edit `crates/kebab-app/src/lib.rs` — add `mod staleness;` and a `pub use staleness::{compute_stale, mark_stale_in_place};` near the other module declarations / re-exports. (Search for `mod app;` to find the existing module declaration block.)

Verify `kebab_core` is already a dependency of `kebab-app` (it is — `App` itself uses `SearchHit`).

- [ ] **Step 6.3: Run tests — verify they pass**

```bash
cargo test -p kebab-app --lib staleness
```

Expected: 5 tests PASS.

- [ ] **Step 6.4: Wire into `App::search` + `App::search_uncached`**

Modify `crates/kebab-app/src/app.rs`:

In `App::search_uncached`, after the retriever call returns hits and before `Ok(...)`:

```rust
pub fn search_uncached(&self, query: SearchQuery) -> Result<Vec<SearchHit>> {
    let mut hits = match query.mode {
        SearchMode::Lexical => { /* ... existing ... */ }
        SearchMode::Vector  => { /* ... existing ... */ }
        SearchMode::Hybrid  => { /* ... existing ... */ }
    };
    // p9-fb-32: stamp staleness against the freshest possible `now`
    // and the current threshold. Cheap (per-hit comparison).
    let now = OffsetDateTime::now_utc();
    crate::staleness::mark_stale_in_place(
        &mut hits,
        now,
        self.config.search.stale_threshold_days,
    );
    Ok(hits)
}
```

In `App::search` (the cache wrapper), the cached `Vec<SearchHit>` was stamped at write time but threshold may have changed and time has moved on. Re-stamp on every cache hit:

```rust
if let Some(hits) = guard.get(&key) {
    let mut hits = hits.clone();
    drop(guard);
    let now = OffsetDateTime::now_utc();
    crate::staleness::mark_stale_in_place(
        &mut hits,
        now,
        self.config.search.stale_threshold_days,
    );
    return Ok(hits);
}
```

(The cache miss path already calls `search_uncached` which stamps, so no extra work needed there.)

- [ ] **Step 6.5: Add integration test for `App::search` end-to-end**

Create `crates/kebab-app/tests/search_stale_integration.rs`:

```rust
//! p9-fb-32: App::search wires staleness onto every hit per
//! the configured threshold.

mod common; // adapt — use whatever test scaffold the crate has

use kebab_app::App;
use kebab_core::{SearchMode, SearchQuery};

#[test]
fn fresh_doc_is_not_stale_with_default_threshold() {
    let env = common::TestEnv::new(); // existing scaffold
    env.ingest_md("a.md", "# T\n\napples\n");
    let app = env.app();
    let hits = app.search(SearchQuery {
        text: "apples".to_string(),
        mode: SearchMode::Lexical,
        k: 5,
        filters: Default::default(),
    }).unwrap();
    assert!(!hits.is_empty());
    assert!(hits.iter().all(|h| !h.stale), "freshly-ingested doc must not be stale at default 30d threshold");
}

#[test]
fn threshold_zero_disables_staleness() {
    let env = common::TestEnv::new_with_threshold_days(0);
    env.ingest_md_with_backdated_updated_at("a.md", "# T\n\napples\n", 365);
    let app = env.app();
    let hits = app.search(SearchQuery {
        text: "apples".to_string(),
        mode: SearchMode::Lexical,
        k: 5,
        filters: Default::default(),
    }).unwrap();
    assert!(!hits.is_empty());
    assert!(hits.iter().all(|h| !h.stale), "threshold=0 disables staleness even for year-old docs");
}

#[test]
fn old_doc_marked_stale() {
    let env = common::TestEnv::new_with_threshold_days(30);
    env.ingest_md_with_backdated_updated_at("a.md", "# T\n\napples\n", 60);
    let app = env.app();
    let hits = app.search(SearchQuery {
        text: "apples".to_string(),
        mode: SearchMode::Lexical,
        k: 5,
        filters: Default::default(),
    }).unwrap();
    assert!(hits.iter().any(|h| h.stale), "60-day-old doc must be stale at 30d threshold");
}
```

The `ingest_md_with_backdated_updated_at` helper writes a doc through normal ingest then SQL-rewrites `documents.updated_at` to `now - days`. Implementation in `tests/common/mod.rs` (extend existing common helpers):

```rust
pub fn ingest_md_with_backdated_updated_at(&self, path: &str, body: &str, days_ago: i64) {
    self.ingest_md(path, body);
    let backdated = (time::OffsetDateTime::now_utc() - time::Duration::days(days_ago))
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap();
    let conn = rusqlite::Connection::open(self.sqlite_path()).unwrap();
    conn.execute(
        "UPDATE documents SET updated_at = ?1 WHERE workspace_path = ?2",
        rusqlite::params![backdated, path],
    ).unwrap();
}
```

If `TestEnv::new_with_threshold_days` doesn't exist, add it as a thin wrapper that builds a `Config` with the override applied before `App::open_with_config`.

- [ ] **Step 6.6: Run integration tests**

```bash
cargo test -p kebab-app --test search_stale_integration
```

Expected: 3 tests PASS.

- [ ] **Step 6.7: Commit**

```bash
git add crates/kebab-app/src/staleness.rs crates/kebab-app/src/lib.rs crates/kebab-app/src/app.rs crates/kebab-app/tests/
git commit -m "$(cat <<'EOF'
feat(app): staleness module + post-process search hits (fb-32)

compute_stale: strict > boundary, threshold=0 disables, future
timestamps treated as fresh (clock skew safety). App::search
re-stamps on cache hit so config threshold changes take effect
without flushing the cache.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: App facade — `AnswerCitation` staleness in `App::ask`

**Files:**
- Modify: `crates/kebab-app/src/app.rs` (or wherever `App::ask` lives)

- [ ] **Step 7.1: Locate `App::ask` AnswerCitation construction**

```bash
grep -n "AnswerCitation\|fn ask\b" crates/kebab-app/src/app.rs crates/kebab-app/src/lib.rs
```

Identify the spot where `Answer` is built (likely in `App::ask` around line 256+ in `app.rs`, or in a helper called from there). The `Vec<AnswerCitation>` is constructed by mapping over the underlying retrieval hits.

- [ ] **Step 7.2: Write the failing test**

Append to `crates/kebab-app/tests/search_stale_integration.rs`:

```rust
#[test]
fn ask_citation_carries_indexed_at_and_stale() {
    let env = common::TestEnv::new_with_threshold_days(30);
    env.ingest_md_with_backdated_updated_at("a.md", "# T\n\napples are fruit\n", 60);
    let app = env.app();
    let answer = app.ask("apples", Default::default()).unwrap();
    assert!(!answer.citations.is_empty());
    assert!(
        answer.citations.iter().any(|c| c.stale),
        "60d-old cited doc must surface stale=true"
    );
    let now = time::OffsetDateTime::now_utc();
    for c in &answer.citations {
        // indexed_at populated, not the zero-time default
        assert!((now - c.indexed_at).whole_seconds() > 0);
    }
}
```

If `App::ask` requires a real LLM, gate this test behind the same feature / env var existing ask integration tests use (search for an existing ask integration test in `kebab-app/tests/` for the pattern). If no LLM is available in CI, add the test under `#[cfg(test)]` with the same skip guard the existing tests use.

- [ ] **Step 7.3: Run test — verify it fails**

```bash
cargo test -p kebab-app --test search_stale_integration ask_citation_carries
```

Expected: FAIL — `AnswerCitation.indexed_at` is zero-time (default), `stale` is false.

- [ ] **Step 7.4: Implement — populate from retrieval hits**

In `App::ask`, the retrieval step produces `Vec<SearchHit>` (already stamped with `indexed_at` + `stale` by Task 6's post-processing). When constructing `AnswerCitation` from each hit, copy both fields:

```rust
let citations: Vec<AnswerCitation> = hits
    .iter()
    .map(|h| AnswerCitation {
        marker: build_marker(h),  // existing logic
        citation: h.citation.clone(),
        indexed_at: h.indexed_at,
        stale: h.stale,
    })
    .collect();
```

If the construction site uses a different builder pattern, adapt to match — the principle is the citation pulls both fields from the source `SearchHit`.

- [ ] **Step 7.5: Run test — verify it passes**

```bash
cargo test -p kebab-app --test search_stale_integration ask_citation_carries
```

Expected: PASS.

- [ ] **Step 7.6: Commit**

```bash
git add crates/kebab-app/
git commit -m "$(cat <<'EOF'
feat(app): AnswerCitation inherits indexed_at + stale from hit (fb-32)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 8: Wire schema — required fields

**Files:**
- Modify: `docs/wire-schema/v1/search_hit.schema.json`
- Modify: `docs/wire-schema/v1/citation.schema.json`

- [ ] **Step 8.1: Update search_hit.schema.json**

Edit `docs/wire-schema/v1/search_hit.schema.json`. Add to `required`:

```json
"required": [
  "schema_version",
  "rank",
  "score",
  "chunk_id",
  "doc_id",
  "doc_path",
  "heading_path",
  "snippet",
  "citation",
  "retrieval",
  "index_version",
  "chunker_version",
  "indexed_at",
  "stale"
]
```

Add to `properties`:

```json
"indexed_at": { "type": "string", "format": "date-time" },
"stale":      { "type": "boolean" }
```

- [ ] **Step 8.2: Update citation.schema.json**

Edit `docs/wire-schema/v1/citation.schema.json`. Add to `required`:

```json
"required": ["schema_version", "kind", "path", "uri", "indexed_at", "stale"]
```

Add to `properties`:

```json
"indexed_at": { "type": "string", "format": "date-time" },
"stale":      { "type": "boolean" }
```

- [ ] **Step 8.3: Find and update any wire schema test**

```bash
grep -rln "search_hit.schema.json\|citation.schema.json" crates/ tests/ 2>/dev/null
```

For each file using JSON Schema validation against these schemas, run its tests:

```bash
cargo test --workspace wire_schema 2>&1 | head -40
```

If any test fails because it generates a hit without `indexed_at`/`stale` for validation, the test fixture needs a regen — this is expected churn and the test will fix itself once Task 9's CLI emit path is in place. If a test asserts the absence of these fields, that's a failing assertion that needs the fields added to the expected fixture.

- [ ] **Step 8.4: Commit**

```bash
git add docs/wire-schema/v1/
git commit -m "$(cat <<'EOF'
feat(wire): search_hit.v1 + citation.v1 require indexed_at + stale (fb-32)

Additive minor — schema_version unchanged. Existing v1 consumers
that ignore unknown fields stay compatible; consumers that validate
strictly will reject pre-fb-32 payloads, which matches the wire
contract escape hatch (recipient version >= producer required).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 9: CLI plain renderer — `[stale]` tag

**Files:**
- Modify: the CLI plain output renderer (locate via grep below)

- [ ] **Step 9.1: Locate the plain renderer**

```bash
grep -rn "fn render\|render_hit\|render_search\|fn print_hit\|fn fmt_hit" crates/kebab-cli/src/ 2>/dev/null | head -20
```

The non-`--json` search output renderer prints rank, score, doc_path, snippet for each hit. Identify the function (likely in `crates/kebab-cli/src/main.rs` or a `render.rs` / `format.rs` sibling).

- [ ] **Step 9.2: Write the failing CLI integration test**

Create `crates/kebab-cli/tests/wire_search_stale.rs`:

```rust
//! p9-fb-32: CLI emits indexed_at + stale on JSON; plain output
//! gains [stale] tag.

mod common; // adapt to existing scaffold

#[test]
fn search_json_includes_indexed_at_and_stale() {
    let out = common::run_cli_search_json(&["apples"]);
    let arr: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    let first = arr.as_array().unwrap().first().unwrap();
    assert!(first.get("indexed_at").is_some());
    assert!(first.get("stale").is_some());
    assert_eq!(first["stale"], false);
}

#[test]
fn search_plain_marks_stale_doc() {
    let env = common::CliEnv::new_with_threshold_days(30);
    env.ingest_md_backdated("a.md", "apples", 60);
    let out = env.run_search_plain("apples");
    assert!(out.stdout.contains("[stale]"), "stale tag missing in plain output:\n{}", out.stdout);
}
```

- [ ] **Step 9.3: Run tests — verify they fail**

```bash
cargo test -p kebab-cli --test wire_search_stale
```

Expected: FAIL — plain output has no `[stale]` (JSON should already pass thanks to Task 1's serde derive).

- [ ] **Step 9.4: Implement plain renderer**

In the located plain renderer function, prepend `[stale] ` to the doc_path line when `hit.stale == true`. Apply ANSI yellow color when `is_terminal::is_terminal(&io::stderr())` (or whatever TTY-detect helper the crate already uses — search for `is_terminal` to find the convention):

```rust
fn render_hit_plain(out: &mut impl Write, hit: &SearchHit, color: bool) -> io::Result<()> {
    let stale_tag = if hit.stale {
        if color {
            "\x1b[33m[stale]\x1b[0m " // yellow
        } else {
            "[stale] "
        }
    } else {
        ""
    };
    writeln!(
        out,
        "{rank}. {stale}{path} § {heading}",
        rank = hit.rank,
        stale = stale_tag,
        path = hit.doc_path.0,
        heading = hit.heading_path.last().map(String::as_str).unwrap_or(""),
    )?;
    // ... existing score / snippet lines unchanged ...
    Ok(())
}
```

The exact format string must match what the existing renderer emits — DO NOT reinvent the layout. The change is just the `[stale] ` prefix when applicable. Match whatever format `render_hit_plain` (or its actual name) currently produces; only prepend the tag.

- [ ] **Step 9.5: Run tests — verify they pass**

```bash
cargo test -p kebab-cli --test wire_search_stale
```

Expected: PASS.

- [ ] **Step 9.6: Commit**

```bash
git add crates/kebab-cli/
git commit -m "$(cat <<'EOF'
feat(cli): [stale] tag on plain output (fb-32)

Yellow when TTY, plain when not. JSON path inherits via serde
on the domain type; no CLI-side wire change needed there.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 10: CLI ask renderer — citation `[stale]` tag

**Files:**
- Modify: CLI plain answer/citation renderer
- Create: `crates/kebab-cli/tests/wire_ask_stale.rs`

- [ ] **Step 10.1: Locate the ask plain renderer**

```bash
grep -rn "render_answer\|print_answer\|render_citation\|print_citation\|fn fmt_answer" crates/kebab-cli/src/ 2>/dev/null | head -10
```

- [ ] **Step 10.2: Write the failing test**

Create `crates/kebab-cli/tests/wire_ask_stale.rs`:

```rust
mod common;

#[test]
fn ask_json_citations_include_indexed_at_and_stale() {
    let env = common::CliEnv::new_with_threshold_days(30);
    env.ingest_md_backdated("a.md", "apples are fruit", 60);
    let out = env.run_ask_json("what about apples");
    let answer: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    let cit = answer["citations"].as_array().unwrap().first().unwrap();
    assert!(cit.get("indexed_at").is_some());
    assert_eq!(cit["stale"], true);
}

#[test]
fn ask_plain_marks_stale_citation() {
    let env = common::CliEnv::new_with_threshold_days(30);
    env.ingest_md_backdated("a.md", "apples are fruit", 60);
    let out = env.run_ask_plain("what about apples");
    assert!(out.stdout.contains("[stale]"));
}
```

Same LLM-availability gating as Task 7's ask test if the CLI test scaffold doesn't already cover it.

- [ ] **Step 10.3: Run tests — verify they fail**

```bash
cargo test -p kebab-cli --test wire_ask_stale
```

Expected: PASS for JSON (serde auto), FAIL for plain output.

- [ ] **Step 10.4: Implement plain citation renderer**

Same pattern as Task 9 but applied to the citation render function. The citation line in plain ask output gains `[stale] ` prefix when `citation.stale == true`.

- [ ] **Step 10.5: Run tests — verify they pass**

```bash
cargo test -p kebab-cli --test wire_ask_stale
```

Expected: PASS.

- [ ] **Step 10.6: Commit**

```bash
git add crates/kebab-cli/
git commit -m "$(cat <<'EOF'
feat(cli): [stale] tag on plain ask citations (fb-32)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 11: TUI — `[STALE]` Span on search/inspect/ask panes

**Files:**
- Modify: TUI search pane render (locate via grep)
- Modify: TUI inspect pane render
- Modify: TUI ask citations render

- [ ] **Step 11.1: Locate TUI render sites**

```bash
grep -rn "doc_path\|workspace_path\.0\|hit\.doc_path\|render_hit\|render_search" crates/kebab-tui/src/ 2>/dev/null | head -30
grep -rn "Role::Warning\|Theme::style" crates/kebab-tui/src/ 2>/dev/null | head -10
```

Identify the spots where `hit.doc_path` and `citation.path` get spanned for display in search / inspect / ask panes.

- [ ] **Step 11.2: Write the failing snapshot test**

Find the existing snapshot test for the search pane (likely `crates/kebab-tui/tests/search_pane.rs` or similar). Add or modify a test that ingests a doc, backdates `documents.updated_at`, runs a search, and snapshots the pane. The snapshot must include the `[STALE]` text.

```rust
#[test]
fn search_pane_shows_stale_badge_for_old_doc() {
    let mut env = TuiTestEnv::new_with_threshold_days(30);
    env.ingest_md_backdated("a.md", "apples", 60);
    let pane = env.run_search_pane("apples");
    insta::with_settings!({
        filters => vec![
            // p9-fb-32: indexed_at is time-dependent — mask in snapshots.
            (r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z", "[indexed_at]"),
        ],
    }, {
        insta::assert_snapshot!(pane);
    });
}
```

- [ ] **Step 11.3: Run snapshot test — verify it fails or pending**

```bash
cargo test -p kebab-tui search_pane_shows_stale_badge
```

Expected: pending (no snapshot yet) or fail (existing snapshot lacks `[STALE]`).

- [ ] **Step 11.4: Implement the badge**

In each render site (search hit row, inspect header, ask citation), wrap a `[STALE]` Span with the Warning style when `hit.stale == true`:

```rust
let mut spans: Vec<Span> = vec![
    Span::raw(format!("{}. ", hit.rank)),
];
if hit.stale {
    spans.push(Span::styled("[STALE] ", theme.style(Role::Warning)));
}
spans.push(Span::raw(hit.doc_path.0.clone()));
// ... rest of the row
```

- [ ] **Step 11.5: Accept the new snapshot**

```bash
cargo test -p kebab-tui search_pane_shows_stale_badge -- --nocapture
cargo insta review
```

Inspect the snapshot — the `[STALE]` text must appear before the doc_path on the stale row. Accept.

- [ ] **Step 11.6: Update insta filter for existing snapshots**

Existing TUI snapshots may now contain `indexed_at` or other timestamp-bearing diffs. Run the broader TUI test:

```bash
cargo test -p kebab-tui
```

For each insta failure, inspect with `cargo insta review`. If the only diff is a serialized `indexed_at`, add the filter pattern from Step 11.2 to the test in question. If the diff is the new `[STALE]` text on a row that should now be marked stale, accept. Reject anything else and investigate.

- [ ] **Step 11.7: Commit**

```bash
git add crates/kebab-tui/
git commit -m "$(cat <<'EOF'
feat(tui): [STALE] Warning-styled badge on search/inspect/ask (fb-32)

insta filter pattern '[indexed_at]' applied where snapshots
otherwise capture time-dependent RFC3339 strings.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 12: Snapshot fan-out — workspace-wide insta sweep

**Files:**
- Modify: any insta snapshot under `crates/*/tests/snapshots/` that now contains `indexed_at`

- [ ] **Step 12.1: Run full workspace test sequentially**

```bash
cargo test --workspace --no-fail-fast -j 1 2>&1 | tail -120
```

(`-j 1` per CLAUDE.md to avoid linker OOM.)

- [ ] **Step 12.2: For each snapshot diff: classify**

```bash
cargo insta pending-snapshots
```

For each pending:

- **Diff is only `indexed_at`** (new RFC3339 field): add the filter pattern `(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z", "[indexed_at]")` to the test's `with_settings!` block, OR if the snapshot already has many time fields, add the filter at module level. Re-run + accept.
- **Diff is `stale: false` field appearing**: accept (additive, expected).
- **Diff is `[STALE]` text on a stale doc row**: accept (expected from Task 11).
- **Diff is anything else**: reject and investigate — that's a regression.

- [ ] **Step 12.3: Accept reviewed snapshots**

```bash
cargo insta accept
```

Verify with:

```bash
git diff --stat crates/*/tests/snapshots/
```

The diff should be confined to insta `.snap` files plus filter additions in test files.

- [ ] **Step 12.4: Run workspace tests again — must be all-green**

```bash
cargo test --workspace --no-fail-fast -j 1
```

Expected: all PASS.

- [ ] **Step 12.5: Commit**

```bash
git add crates/*/tests/ crates/*/src/
git commit -m "$(cat <<'EOF'
test(snapshots): regen for indexed_at + stale fields (fb-32)

insta filter '[indexed_at]' applied where time-dependent.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 13: Clippy + workspace check

- [ ] **Step 13.1: Run clippy**

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

Expected: 0 warnings. Fix any introduced warnings inline.

- [ ] **Step 13.2: Commit if clippy required fixes**

```bash
git add -A
git commit -m "chore: clippy fixes for fb-32"
```

(Skip this commit if no fixes were needed.)

---

## Task 14: Documentation updates

**Files:**
- Modify: `README.md`
- Modify: `docs/SMOKE.md`
- Modify: `tasks/p9/p9-fb-32-stale-doc-indicator.md`
- Modify: `tasks/INDEX.md`
- Modify: `integrations/claude-code/kebab/SKILL.md`

- [ ] **Step 14.1: README — Configuration section**

Find the Configuration section (search for `## Configuration` or the `config.toml` example block):

```bash
grep -n "stale_threshold_days\|\\[search\\]" README.md
```

Add to the example `[search]` block:

```toml
[search]
default_k = 10
hybrid_fusion = "rrf"
rrf_k = 60
snippet_chars = 220
stale_threshold_days = 30  # 0 = disable. Marks hits/citations whose source doc was last reindexed > N days ago.
```

- [ ] **Step 14.2: docs/SMOKE.md — config example + walkthrough**

Add the same line to the SMOKE config example. After the existing search walkthrough, append a short paragraph:

```markdown
### Stale doc indicator

Each search hit and RAG citation carries `indexed_at` (RFC3339 of the doc's last
re-process) and `stale` (computed against `[search] stale_threshold_days`).
A 30-day default flags docs that haven't been touched in a month — the
intent is to nudge a reingest before relying on the snapshot. Set to `0`
to disable.
```

- [ ] **Step 14.3: Task spec status flip**

Edit `tasks/p9/p9-fb-32-stale-doc-indicator.md`:

```diff
 ---
 phase: P9
 component: kebab-app + kebab-tui + kebab-cli
 task_id: p9-fb-32
 title: "Stale doc indicator (ingest 시점 대비 X 일 임계 알림)"
-status: open
+status: completed
 target_version: 0.4.0
```

Replace the body's `> ⏳ **백로그 only — 미구현.**` block with:

```markdown
상세 설계: `docs/superpowers/specs/2026-05-08-p9-fb-32-stale-doc-indicator-design.md`.
구현 계획: `docs/superpowers/plans/2026-05-09-p9-fb-32-stale-doc-indicator.md`.
```

(Keep the rest of the spec body — it's the historical contract per CLAUDE.md.)

- [ ] **Step 14.4: tasks/INDEX.md — fb-32 row**

Edit `tasks/INDEX.md`:

```diff
-    - [p9-fb-32 stale doc indicator](p9/p9-fb-32-stale-doc-indicator.md) — ⏳ 미구현, brainstorm 필요
+    - [p9-fb-32 stale doc indicator](p9/p9-fb-32-stale-doc-indicator.md) — ✅ 머지 + v0.4.0 cut 후보 (2026-05-09)
```

- [ ] **Step 14.5: Skill — parsing tip**

Edit `integrations/claude-code/kebab/SKILL.md` — locate the "Parsing tips" section and append a bullet:

```markdown
- `search_hit.v1` and `answer.v1.citations[]` carry `indexed_at` (RFC3339) + `stale` (bool). When `stale == true`, the source doc hasn't been re-processed since `config.search.stale_threshold_days`. Surface this caveat to the user when summarizing — the cited snapshot may not reflect current reality.
```

- [ ] **Step 14.6: Commit docs**

```bash
git add README.md docs/SMOKE.md tasks/p9/p9-fb-32-stale-doc-indicator.md tasks/INDEX.md integrations/claude-code/kebab/SKILL.md
git commit -m "$(cat <<'EOF'
docs(fb-32): README + SMOKE + INDEX + skill parsing tip

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 15: Smoke + final verification

- [ ] **Step 15.1: Manual smoke against `docs/SMOKE.md`**

Follow the SMOKE walkthrough end-to-end:

```bash
mkdir -p /tmp/kebab-smoke && cd /tmp/kebab-smoke
# (build a minimal config.toml + workspace per docs/SMOKE.md)
~/Workspace/projects/kebab/target/release/kebab --config /tmp/kebab-smoke/config.toml init
~/Workspace/projects/kebab/target/release/kebab --config /tmp/kebab-smoke/config.toml ingest
~/Workspace/projects/kebab/target/release/kebab --config /tmp/kebab-smoke/config.toml search "test" --json | jq '.[0] | {indexed_at, stale}'
```

Expected output:

```json
{"indexed_at": "2026-05-09T...Z", "stale": false}
```

- [ ] **Step 15.2: Backdate + re-verify stale path**

```bash
sqlite3 /tmp/kebab-smoke/data/kebab.sqlite "UPDATE documents SET updated_at = '2025-01-01T00:00:00Z' WHERE workspace_path LIKE '%test%';"
~/Workspace/projects/kebab/target/release/kebab --config /tmp/kebab-smoke/config.toml search "test" --json | jq '.[0].stale'
```

Expected: `true`.

- [ ] **Step 15.3: Plain output check**

```bash
~/Workspace/projects/kebab/target/release/kebab --config /tmp/kebab-smoke/config.toml search "test"
```

Expected: `[stale]` tag present on the matched hit.

- [ ] **Step 15.4: Final workspace test**

```bash
cd ~/Workspace/projects/kebab
cargo test --workspace --no-fail-fast -j 1
```

Expected: all green.

- [ ] **Step 15.5: Push + open PR**

```bash
git push -u origin feat/fb-32-stale-doc-indicator
```

Open PR via Gitea API (per CLAUDE.md — `gh` does not work):

```bash
curl -s --netrc-file ~/.netrc \
    -X POST \
    -H "Content-Type: application/json" \
    https://gitea.altair823.xyz/api/v1/repos/altair823-org/kebab/pulls \
    -d '{
        "title": "feat(fb-32): stale doc indicator",
        "body": "## Summary\n- adds `indexed_at` + `stale` to `search_hit.v1` / `citation.v1`\n- reuses `documents.updated_at` (no migration)\n- config `search.stale_threshold_days` default 30; 0 disables\n- TUI `[STALE]` Warning badge, CLI `[stale]` tag, agent JSON fields\n\n## Test plan\n- [x] cargo test --workspace -j 1 green\n- [x] cargo clippy --workspace --all-targets -- -D warnings\n- [x] manual smoke: ingest → search shows fresh; backdate → search shows stale\n- [x] insta snapshots reviewed and accepted\n\nSpec: `docs/superpowers/specs/2026-05-08-p9-fb-32-stale-doc-indicator-design.md`\nPlan: `docs/superpowers/plans/2026-05-09-p9-fb-32-stale-doc-indicator.md`",
        "head": "feat/fb-32-stale-doc-indicator",
        "base": "main"
    }'
```

Capture the returned PR URL.

---

## Self-review checklist (post-plan, pre-execution)

- **Spec coverage:**
  - §Behavior contract → Tasks 1, 2, 6 (domain + compute_stale)
  - §Wire schema delta → Task 8
  - §Config → Task 3
  - §CLI plain output → Tasks 9, 10
  - §TUI → Task 11
  - §Components → Tasks 4 (lexical), 5 (vector), 6/7 (app), 9/10 (cli), 11 (tui)
  - §Test plan → unit (Tasks 3, 6), integration (Tasks 6, 7, 9, 10, 11)
  - §Documentation → Task 14
  - §Risks/Clock → Task 6 (explicit `now: OffsetDateTime` arg, no Clock trait)
  - §Risks/Snapshot churn → Task 12
  - §Risks/Off-by-one → Task 6 unit tests `exactly_threshold_is_fresh` + `one_minute_past_threshold_is_stale`

- **Placeholder scan:**
  - "adapt to existing scaffold" appears in Tasks 4, 5, 6, 7, 9 — these instruct copying from existing test infrastructure rather than inventing new helpers. The intent is concrete (find `TestEnv` / `common`, mirror the pattern). Acceptable since fully spelling out an existing scaffold would inflate the plan and the code is in the repo.
  - No "TODO", "later", or "fill in" remaining.

- **Type consistency:**
  - `indexed_at: OffsetDateTime` and `stale: bool` consistent across `SearchHit`, `AnswerCitation`, `compute_stale`, `mark_stale_in_place`.
  - `threshold_days: u32` consistent in `SearchCfg` + helpers.
  - Function `mark_stale_in_place(&mut [SearchHit], OffsetDateTime, u32)` — same signature in Tasks 6.4, 6.5, 6.6.

- **Spec deviation noted:**
  - Spec §Components says "kebab-core 변경 없음". Plan correctly identifies this as inaccurate (domain `SearchHit` IS the wire source) and updates kebab-core. The spec body should be amended in Task 14 if strict alignment matters; currently the spec § Public surface delta block already shows the kebab-core changes implicitly.

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-05-09-p9-fb-32-stale-doc-indicator.md`. Two execution options:

**1. Subagent-Driven (recommended)** — fresh subagent per task, review between tasks, fast iteration.

**2. Inline Execution** — execute tasks in this session using executing-plans, batch execution with checkpoints.

Which approach?
