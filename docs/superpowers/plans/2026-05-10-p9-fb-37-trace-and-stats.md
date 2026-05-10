# fb-37 Trace + Stats Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Surface retrieval pipeline trace (`kebab search Q --trace`) and richer KB stats (`kebab schema --json`) for agent / user debugging.

**Architecture:** Two additive surfaces. Trace = optional `trace` field on `search_response.v1` populated when `SearchOpts.trace = true`; HybridRetriever exposes a parallel `search_with_trace` method capturing pre-fusion lex/vec lists + per-stage timing. Stats = four new fields (`media_breakdown` / `lang_breakdown` / `index_bytes` / `stale_doc_count`) on existing `schema.v1.stats` populated unconditionally by new SQLite GROUP BY + fs::metadata helpers. TUI search pane gains `t` keystroke that re-runs the query with trace and opens a popup.

**Tech Stack:** Rust 2024, rusqlite (SQLite WHERE / GROUP BY / json_type / json_extract / json_each), std::time::Instant, std::fs, serde, ratatui.

**Spec:** `docs/superpowers/specs/2026-05-10-p9-fb-37-trace-and-stats-design.md`

---

## File map

**Create:**
- `crates/kebab-search/src/trace.rs` — trace timing + capture helpers (kept separate from `hybrid.rs` so `hybrid.rs` stays focused)
- `crates/kebab-store-sqlite/src/stats_ext.rs` — `breakdowns()` + `index_bytes()` helpers
- `crates/kebab-tui/src/trace_popup.rs` — TUI popup widget + state
- `crates/kebab-cli/tests/wire_search_trace.rs` — `--trace` integration tests
- `crates/kebab-cli/tests/wire_schema_breakdowns.rs` — `kebab schema` extended stats integration tests
- `crates/kebab-mcp/tests/tools_call_search_trace.rs` — MCP search trace integration test

**Modify:**
- `crates/kebab-core/src/search.rs` — add `SearchTrace` / `TraceCandidate` / `TraceFusionInput` / `TraceTiming` + `IndexBytes` types; extend `SearchOpts` with `trace: bool`
- `crates/kebab-store-sqlite/src/store.rs` — extend `CountSummary` with new fields, populate via new helpers
- `crates/kebab-app/src/schema.rs` — extend `Stats` mirror with new fields, wire collect_stats
- `crates/kebab-app/src/app.rs` — extend `SearchResponse` with `trace: Option<SearchTrace>`, thread trace through `App::search_with_opts`
- `crates/kebab-search/src/hybrid.rs` — add `HybridRetriever::search_with_trace`
- `crates/kebab-cli/src/main.rs` — add `--trace` flag to `Cmd::Search`, dispatch + non-JSON pretty-print
- `crates/kebab-cli/src/wire.rs` — extend `wire_search_response` to serialize `trace` field when present
- `crates/kebab-mcp/src/tools/search.rs` — add `trace: Option<bool>` to `SearchInput`, dispatch through
- `crates/kebab-tui/src/search.rs` — add `t` keystroke handler invoking trace + opening popup
- `crates/kebab-tui/src/app.rs` — store `trace_popup: Option<TracePopupState>`
- `crates/kebab-tui/src/cheatsheet.rs` — add `t = trace` line
- `crates/kebab-tui/src/lib.rs` — register `trace_popup` module
- `docs/wire-schema/v1/search_response.schema.json` — declare optional `trace` field
- `docs/wire-schema/v1/schema.schema.json` — declare new stats fields
- `README.md`, `docs/SMOKE.md`, `tasks/p9/p9-fb-37-trace-and-stats.md`, `tasks/INDEX.md`, `integrations/claude-code/kebab/SKILL.md`

---

## Task 1: Trace + IndexBytes domain types in kebab-core

**Files:**
- Modify: `crates/kebab-core/src/search.rs`

- [ ] **Step 1: Write failing test for SearchTrace serde roundtrip**

Append to `crates/kebab-core/src/search.rs` `mod tests`:
```rust
#[test]
fn search_trace_serde_roundtrip() {
    let t = SearchTrace {
        lexical: vec![TraceCandidate {
            chunk_id: ChunkId("c1".into()),
            doc_id: DocumentId("d1".into()),
            doc_path: WorkspacePath::new("a.md".into()).unwrap(),
            rank: 1,
            score: 0.42,
        }],
        vector: vec![],
        rrf_inputs: vec![TraceFusionInput {
            chunk_id: ChunkId("c1".into()),
            lexical_rank: Some(1),
            vector_rank: None,
            fusion_score: 0.0234,
        }],
        timing: TraceTiming {
            lexical_ms: 12,
            vector_ms: 0,
            fusion_ms: 1,
            total_ms: 14,
        },
    };
    let v = serde_json::to_value(&t).unwrap();
    assert_eq!(v["timing"]["lexical_ms"], 12);
    assert_eq!(v["lexical"][0]["score"], 0.42);
    let back: SearchTrace = serde_json::from_value(v).unwrap();
    assert_eq!(back, t);
}

#[test]
fn index_bytes_default_is_zero() {
    let b = IndexBytes::default();
    assert_eq!(b.sqlite, 0);
    assert_eq!(b.lancedb, 0);
}

#[test]
fn search_opts_trace_default_false() {
    let opts = SearchOpts::default();
    assert!(!opts.trace);
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -p kebab-core --lib
```
Expected: compile errors — `SearchTrace`, `TraceCandidate`, `TraceFusionInput`, `TraceTiming`, `IndexBytes` not defined; `SearchOpts.trace` field missing.

- [ ] **Step 3: Add types**

Append to `crates/kebab-core/src/search.rs` (after existing `SearchOpts`):

```rust
/// p9-fb-37: search retrieval pipeline trace. Populated only when
/// `SearchOpts.trace = true`; `None` on the wrapping `SearchResponse`
/// otherwise. `lexical` / `vector` are pre-fusion candidate lists
/// (each retriever's full output for the fanout query). `rrf_inputs`
/// is the union (chunk_id) used by RRF, with each side's rank
/// captured. `timing` is wall-clock per stage.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SearchTrace {
    pub lexical: Vec<TraceCandidate>,
    pub vector: Vec<TraceCandidate>,
    pub rrf_inputs: Vec<TraceFusionInput>,
    pub timing: TraceTiming,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TraceCandidate {
    pub chunk_id: ChunkId,
    pub doc_id: DocumentId,
    pub doc_path: WorkspacePath,
    pub rank: u32,
    pub score: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TraceFusionInput {
    pub chunk_id: ChunkId,
    pub lexical_rank: Option<u32>,
    pub vector_rank: Option<u32>,
    pub fusion_score: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceTiming {
    pub lexical_ms: u64,
    pub vector_ms: u64,
    pub fusion_ms: u64,
    pub total_ms: u64,
}

/// p9-fb-37: on-disk index size breakdown. Mirrored on the
/// wire `schema.v1.stats.index_bytes` block.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexBytes {
    pub sqlite: u64,
    pub lancedb: u64,
}
```

Extend `SearchOpts` (replace the existing struct definition):

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
    /// p9-fb-37: when true, capture pipeline trace (cache bypassed,
    /// lex / vec pre-fusion lists + timing populated on the response).
    #[serde(default)]
    pub trace: bool,
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test -p kebab-core --lib
```
Expected: all 3 new tests pass; existing tests unaffected.

- [ ] **Step 5: Commit**

```bash
git add crates/kebab-core/src/search.rs
git commit -m "feat(core): SearchTrace + IndexBytes types + SearchOpts.trace (fb-37)"
```

---

## Task 2: SQLite breakdowns helper

**Files:**
- Create: `crates/kebab-store-sqlite/src/stats_ext.rs`
- Modify: `crates/kebab-store-sqlite/src/lib.rs` (register module)

- [ ] **Step 1: Write failing tests**

Create `crates/kebab-store-sqlite/src/stats_ext.rs`:

```rust
//! p9-fb-37: extended stats helpers — per-media / per-lang doc counts,
//! stale doc count, on-disk index byte sums.

use std::collections::BTreeMap;
use std::path::Path;

use kebab_core::{IndexBytes, MEDIA_KINDS};
use rusqlite::Connection;

/// Returns `(media_breakdown, lang_breakdown, stale_doc_count)`.
///
/// `media_breakdown` always contains all 5 `MEDIA_KINDS` (zero-padded).
/// `lang_breakdown` only contains observed languages; NULL lang is
/// keyed as the literal string `"null"`. `stale_doc_count` is 0 when
/// `threshold_days == 0` (mirrors fb-32 staleness disable semantics).
pub fn breakdowns(
    conn: &Connection,
    threshold_days: u64,
) -> rusqlite::Result<(BTreeMap<String, u64>, BTreeMap<String, u64>, u64)> {
    // media: dual JSON shape — text variant ("markdown") vs object
    // variant ({"image":{"format":"png"}}). Same CASE WHEN as fb-36.
    let mut media: BTreeMap<String, u64> = MEDIA_KINDS
        .iter()
        .map(|k| ((*k).to_string(), 0u64))
        .collect();
    let mut stmt = conn.prepare(
        "SELECT \
           CASE \
             WHEN json_type(a.media_type) = 'text' \
               THEN json_extract(a.media_type, '$') \
             ELSE (SELECT key FROM json_each(a.media_type) LIMIT 1) \
           END AS kind, \
           COUNT(DISTINCT d.doc_id) \
         FROM documents d JOIN assets a ON a.asset_id = d.asset_id \
         GROUP BY kind",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?))
    })?;
    for row in rows {
        let (kind, n) = row?;
        media.insert(kind, n);
    }

    let mut lang: BTreeMap<String, u64> = BTreeMap::new();
    let mut stmt = conn.prepare(
        "SELECT COALESCE(lang, 'null') AS l, COUNT(*) \
         FROM documents GROUP BY l",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?))
    })?;
    for row in rows {
        let (l, n) = row?;
        lang.insert(l, n);
    }

    let stale: u64 = if threshold_days == 0 {
        0
    } else {
        let secs = (threshold_days as i64) * 86_400;
        let cutoff = time::OffsetDateTime::now_utc()
            - time::Duration::seconds(secs);
        let cutoff_str = cutoff
            .format(&time::format_description::well_known::Rfc3339)
            .expect("RFC3339 format");
        conn.query_row(
            "SELECT COUNT(*) FROM documents WHERE updated_at < ?",
            [cutoff_str],
            |r| r.get(0),
        )?
    };

    Ok((media, lang, stale))
}

/// Sum on-disk bytes of the SQLite database (main + WAL + SHM) and
/// the LanceDB directory tree. Missing files / dir = 0.
pub fn index_bytes(data_dir: &Path) -> std::io::Result<IndexBytes> {
    fn file_size_or_zero(p: &Path) -> u64 {
        std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
    }
    fn dir_walk_sum(p: &Path) -> std::io::Result<u64> {
        if !p.exists() {
            return Ok(0);
        }
        let mut total = 0u64;
        for entry in std::fs::read_dir(p)? {
            let entry = entry?;
            let ty = entry.file_type()?;
            if ty.is_dir() {
                total += dir_walk_sum(&entry.path())?;
            } else if ty.is_file() {
                total += entry.metadata()?.len();
            }
        }
        Ok(total)
    }

    let sqlite_main = data_dir.join("kebab.sqlite");
    let sqlite_wal = data_dir.join("kebab.sqlite-wal");
    let sqlite_shm = data_dir.join("kebab.sqlite-shm");
    let sqlite = file_size_or_zero(&sqlite_main)
        + file_size_or_zero(&sqlite_wal)
        + file_size_or_zero(&sqlite_shm);
    let lancedb = dir_walk_sum(&data_dir.join("lancedb"))?;
    Ok(IndexBytes { sqlite, lancedb })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_fresh() -> (tempfile::TempDir, crate::SqliteStore) {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = kebab_config::Config::defaults();
        cfg.storage.data_dir = dir.path().to_string_lossy().into_owned();
        let store = crate::SqliteStore::open(&cfg).unwrap();
        store.run_migrations().unwrap();
        (dir, store)
    }

    #[test]
    fn breakdowns_empty_corpus() {
        let (_dir, store) = open_fresh();
        let conn = store.read_conn();
        let (media, lang, stale) = breakdowns(&conn, 0).unwrap();
        // 5 keys all zero, lang map empty, stale 0.
        assert_eq!(media.len(), 5);
        for k in MEDIA_KINDS {
            assert_eq!(media.get(*k), Some(&0u64));
        }
        assert!(lang.is_empty());
        assert_eq!(stale, 0);
    }

    #[test]
    fn index_bytes_includes_sqlite_main() {
        let (dir, _store) = open_fresh();
        let b = index_bytes(dir.path()).unwrap();
        assert!(b.sqlite > 0, "main sqlite file should exist after migrations");
        assert_eq!(b.lancedb, 0);
    }

    #[test]
    fn index_bytes_lancedb_dir_walk() {
        let dir = tempfile::tempdir().unwrap();
        let lance = dir.path().join("lancedb");
        std::fs::create_dir_all(lance.join("vectors.lance")).unwrap();
        std::fs::write(
            lance.join("vectors.lance").join("data.bin"),
            vec![0u8; 1024],
        )
        .unwrap();
        let b = index_bytes(dir.path()).unwrap();
        assert_eq!(b.lancedb, 1024);
    }
}
```

Modify `crates/kebab-store-sqlite/src/lib.rs`. Find the existing `pub mod` declarations and add:

```rust
pub mod stats_ext;
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -p kebab-store-sqlite stats_ext
```
Expected: build error initially (module exists but test imports `MEDIA_KINDS` from kebab-core); resolve any compile issue, then run again. Tests should pass with the implementation provided in Step 1 — this is a test-with-implementation step (verifying via cargo).

Actually since the implementation is already in stats_ext.rs in Step 1, run:
```bash
cargo test -p kebab-store-sqlite stats_ext
```
Expected: 3 new tests pass.

- [ ] **Step 3: Commit**

```bash
git add crates/kebab-store-sqlite/src/stats_ext.rs crates/kebab-store-sqlite/src/lib.rs
git commit -m "feat(store): breakdowns + index_bytes helpers (fb-37)"
```

---

## Task 3: Extend CountSummary + wire to schema.v1.stats

**Files:**
- Modify: `crates/kebab-store-sqlite/src/store.rs`
- Modify: `crates/kebab-app/src/schema.rs`

- [ ] **Step 1: Write failing test in kebab-app**

Append to `crates/kebab-app/src/schema.rs` `mod tests` section (or create one if absent — check around line 200+):

```rust
#[cfg(test)]
mod tests_stats_ext {
    use super::*;

    #[test]
    fn stats_includes_breakdowns_and_bytes_on_fresh_corpus() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = kebab_config::Config::defaults();
        cfg.storage.data_dir = dir.path().to_string_lossy().into_owned();
        // Bring up migrations so the sqlite file is created.
        let store = kebab_store_sqlite::SqliteStore::open(&cfg).unwrap();
        store.run_migrations().unwrap();
        drop(store);

        let s = schema_with_config(&cfg).unwrap();
        // 5 keys padded.
        assert_eq!(s.stats.media_breakdown.len(), 5);
        assert_eq!(s.stats.media_breakdown.get("markdown"), Some(&0));
        assert_eq!(s.stats.media_breakdown.get("pdf"), Some(&0));
        // lang map empty on empty corpus.
        assert!(s.stats.lang_breakdown.is_empty());
        // sqlite bytes positive after migrations, lancedb 0.
        assert!(s.stats.index_bytes.sqlite > 0);
        assert_eq!(s.stats.index_bytes.lancedb, 0);
        assert_eq!(s.stats.stale_doc_count, 0);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p kebab-app stats_includes_breakdowns_and_bytes_on_fresh_corpus
```
Expected: compile error — `Stats` lacks `media_breakdown`, `lang_breakdown`, `index_bytes`, `stale_doc_count`.

- [ ] **Step 3: Extend `CountSummary`**

Modify `crates/kebab-store-sqlite/src/store.rs`. Find `pub struct CountSummary` (~line 595-606) and replace with:

```rust
#[derive(Debug, Clone)]
pub struct CountSummary {
    pub doc_count: u64,
    pub chunk_count: u64,
    pub asset_count: u64,
    /// ISO-8601 timestamp of the most-recently updated document row, or
    /// `None` when the store is empty.
    pub last_ingest_at: Option<String>,
    /// p9-fb-37: per-media-kind doc count (5 keys, zero-padded).
    pub media_breakdown: std::collections::BTreeMap<String, u64>,
    /// p9-fb-37: per-language doc count, NULL keyed as `"null"`.
    pub lang_breakdown: std::collections::BTreeMap<String, u64>,
    /// p9-fb-37: docs whose `updated_at < now - threshold_days`. 0 when threshold=0.
    pub stale_doc_count: u64,
}
```

Modify `count_summary` body (around line 615-650) to populate new fields. Replace the body of `pub fn count_summary(&self) -> anyhow::Result<CountSummary>`:

```rust
pub fn count_summary(&self) -> anyhow::Result<CountSummary> {
    use anyhow::Context;
    use rusqlite::OptionalExtension;

    let conn = self.read_conn();

    let doc_count: u64 = conn
        .query_row("SELECT COUNT(*) FROM documents", [], |r| r.get(0))
        .context("count documents")?;
    let chunk_count: u64 = conn
        .query_row("SELECT COUNT(*) FROM chunks", [], |r| r.get(0))
        .context("count chunks")?;
    let asset_count: u64 = conn
        .query_row("SELECT COUNT(*) FROM assets", [], |r| r.get(0))
        .context("count assets")?;
    let last_ingest_at: Option<String> = conn
        .query_row("SELECT MAX(updated_at) FROM documents", [], |r| r.get(0))
        .optional()
        .context("max updated_at")?
        .flatten();

    // p9-fb-37: pull threshold from config-defaults via a sentinel —
    // CountSummary callers that want correct stale_doc_count must
    // pass through count_summary_with_threshold. Default path uses 0
    // (matches fb-32 disable semantics) for backwards compat.
    let (media_breakdown, lang_breakdown, stale_doc_count) =
        crate::stats_ext::breakdowns(&conn, 0).context("breakdowns")?;

    Ok(CountSummary {
        doc_count,
        chunk_count,
        asset_count,
        last_ingest_at,
        media_breakdown,
        lang_breakdown,
        stale_doc_count,
    })
}

/// p9-fb-37: variant that honors `config.search.stale_threshold_days`.
/// Callers who need a meaningful `stale_doc_count` (e.g. `kebab schema`)
/// pass the configured threshold; the older `count_summary` returns 0.
pub fn count_summary_with_threshold(
    &self,
    threshold_days: u64,
) -> anyhow::Result<CountSummary> {
    use anyhow::Context;
    let mut s = self.count_summary()?;
    let conn = self.read_conn();
    let (m, l, stale) = crate::stats_ext::breakdowns(&conn, threshold_days)
        .context("breakdowns_with_threshold")?;
    s.media_breakdown = m;
    s.lang_breakdown = l;
    s.stale_doc_count = stale;
    Ok(s)
}
```

Update existing `count_summary_zero_on_fresh_store` test (~line 678) to assert new fields:

```rust
#[test]
fn count_summary_zero_on_fresh_store() {
    let (_dir, store) = open_fresh_store();
    let s = store.count_summary().unwrap();
    assert_eq!(s.doc_count, 0);
    assert_eq!(s.chunk_count, 0);
    assert_eq!(s.asset_count, 0);
    assert!(s.last_ingest_at.is_none());
    assert_eq!(s.media_breakdown.len(), 5);
    assert!(s.lang_breakdown.is_empty());
    assert_eq!(s.stale_doc_count, 0);
}
```

- [ ] **Step 4: Extend `Stats` mirror in kebab-app::schema**

Modify `crates/kebab-app/src/schema.rs`. Replace `pub struct Stats`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stats {
    pub doc_count: u64,
    pub chunk_count: u64,
    pub asset_count: u64,
    pub last_ingest_at: Option<String>,
    /// p9-fb-37: per-media-kind doc count (5 keys, zero-padded).
    #[serde(default)]
    pub media_breakdown: std::collections::BTreeMap<String, u64>,
    /// p9-fb-37: per-language doc count, NULL keyed as `"null"`.
    #[serde(default)]
    pub lang_breakdown: std::collections::BTreeMap<String, u64>,
    /// p9-fb-37: on-disk byte sums.
    #[serde(default)]
    pub index_bytes: kebab_core::IndexBytes,
    /// p9-fb-37: docs whose `updated_at` exceeds the staleness threshold.
    #[serde(default)]
    pub stale_doc_count: u64,
}
```

Replace `collect_stats` body:

```rust
fn collect_stats(
    cfg: &Config,
    store: &kebab_store_sqlite::SqliteStore,
) -> anyhow::Result<Stats> {
    let counts = store
        .count_summary_with_threshold(cfg.search.stale_threshold_days as u64)?;
    let data_dir = kebab_config::expand_path(&cfg.storage.data_dir, "");
    let index_bytes = kebab_store_sqlite::stats_ext::index_bytes(&data_dir)
        .map_err(|e| anyhow::anyhow!("index_bytes: {e}"))?;
    Ok(Stats {
        doc_count: counts.doc_count,
        chunk_count: counts.chunk_count,
        asset_count: counts.asset_count,
        last_ingest_at: counts.last_ingest_at,
        media_breakdown: counts.media_breakdown,
        lang_breakdown: counts.lang_breakdown,
        index_bytes,
        stale_doc_count: counts.stale_doc_count,
    })
}
```

Update the call site `let stats = collect_stats(&store)?;` (~line 88) to:

```rust
let stats = collect_stats(cfg, &store)?;
```

- [ ] **Step 5: Run tests to verify they pass**

```bash
cargo test -p kebab-store-sqlite count_summary
cargo test -p kebab-app stats_includes_breakdowns_and_bytes_on_fresh_corpus
```
Expected: both pass.

- [ ] **Step 6: Verify config field type**

`cfg.search.stale_threshold_days` must exist as integer. Check `crates/kebab-config/src/lib.rs` for `Search.stale_threshold_days`. If type mismatch (e.g. it's `u32`), adjust `as u64` cast accordingly.

```bash
grep -n "stale_threshold_days" crates/kebab-config/src/lib.rs
```
Expected: line with the field type. If it's already `u64` drop the cast; if `u32` keep `as u64`.

- [ ] **Step 7: Run full clippy + workspace tests**

```bash
cargo clippy -p kebab-core -p kebab-store-sqlite -p kebab-app --all-targets -- -D warnings
cargo test -p kebab-core -p kebab-store-sqlite -p kebab-app
```
Expected: clippy clean, all tests pass.

- [ ] **Step 8: Commit**

```bash
git add crates/kebab-store-sqlite/src/store.rs crates/kebab-app/src/schema.rs
git commit -m "feat(stats): media/lang/bytes/stale fields on schema.v1.stats (fb-37)"
```

---

## Task 4: HybridRetriever search_with_trace

**Files:**
- Create: `crates/kebab-search/src/trace.rs`
- Modify: `crates/kebab-search/src/hybrid.rs`
- Modify: `crates/kebab-search/src/lib.rs`

- [ ] **Step 1: Write failing test in hybrid.rs**

Append to `crates/kebab-search/src/hybrid.rs` `mod tests`:

```rust
#[test]
fn search_with_trace_returns_lex_and_vec_lists() {
    use kebab_core::{ChunkId, DocumentId, IndexVersion, ChunkerVersion,
                     RetrievalDetail, SearchHit, SearchMode, SearchQuery,
                     WorkspacePath, Citation};
    use std::sync::Arc;

    fn mk_hit(rank: u32, chunk: &str, score: f32, mode: SearchMode) -> SearchHit {
        SearchHit {
            rank,
            chunk_id: ChunkId(chunk.into()),
            doc_id: DocumentId(format!("d-{chunk}")),
            doc_path: WorkspacePath::new(format!("{chunk}.md")).unwrap(),
            heading_path: vec![],
            section_label: None,
            snippet: chunk.into(),
            citation: Citation::Line {
                path: WorkspacePath::new(format!("{chunk}.md")).unwrap(),
                start: 1,
                end: 1,
                section: None,
            },
            retrieval: RetrievalDetail {
                method: mode,
                fusion_score: score,
                lexical_score: if mode == SearchMode::Lexical { Some(score) } else { None },
                vector_score: if mode == SearchMode::Vector { Some(score) } else { None },
                lexical_rank: if mode == SearchMode::Lexical { Some(rank) } else { None },
                vector_rank: if mode == SearchMode::Vector { Some(rank) } else { None },
            },
            index_version: IndexVersion("v1".into()),
            embedding_model: None,
            chunker_version: ChunkerVersion("c1".into()),
            indexed_at: time::OffsetDateTime::UNIX_EPOCH,
            stale: false,
        }
    }

    // Stub retrievers from existing test patterns in this file (see
    // `MockRetriever` near line 363 if present, otherwise inline).
    struct Stub { hits: Vec<SearchHit>, mode: SearchMode }
    impl Retriever for Stub {
        fn search(&self, _q: &SearchQuery) -> anyhow::Result<Vec<SearchHit>> {
            Ok(self.hits.clone())
        }
        fn index_version(&self) -> IndexVersion { IndexVersion("v1".into()) }
    }

    let lex = Arc::new(Stub {
        hits: vec![
            mk_hit(1, "c1", 0.9, SearchMode::Lexical),
            mk_hit(2, "c2", 0.5, SearchMode::Lexical),
        ],
        mode: SearchMode::Lexical,
    });
    let vec_r = Arc::new(Stub {
        hits: vec![
            mk_hit(1, "c2", 0.8, SearchMode::Vector),
            mk_hit(2, "c3", 0.6, SearchMode::Vector),
        ],
        mode: SearchMode::Vector,
    });
    let hybrid = HybridRetriever::with_policy(
        lex.clone(),
        vec_r.clone(),
        FusionPolicy::Rrf { k: 60 },
        2,
    );
    let q = SearchQuery {
        text: "x".into(),
        mode: SearchMode::Hybrid,
        k: 2,
        filters: Default::default(),
    };
    let (hits, trace) = hybrid.search_with_trace(&q).unwrap();
    assert!(!hits.is_empty());
    assert_eq!(trace.lexical.len(), 2);
    assert_eq!(trace.vector.len(), 2);
    // Union: c1, c2, c3 → 3 entries.
    assert_eq!(trace.rrf_inputs.len(), 3);
    // Sanity: timing populated (any field >= 0 trivially; just check
    // the type was set, not a Default::default()).
    let _ = trace.timing.lexical_ms;
}

#[test]
fn search_with_trace_lexical_mode_empty_vector() {
    use kebab_core::{ChunkId, DocumentId, IndexVersion, ChunkerVersion,
                     RetrievalDetail, SearchHit, SearchMode, SearchQuery,
                     WorkspacePath, Citation};
    use std::sync::Arc;
    struct EmptyR(SearchMode);
    impl Retriever for EmptyR {
        fn search(&self, _q: &SearchQuery) -> anyhow::Result<Vec<SearchHit>> {
            Ok(vec![])
        }
        fn index_version(&self) -> IndexVersion { IndexVersion("v1".into()) }
    }
    let lex = Arc::new(EmptyR(SearchMode::Lexical));
    let vec_r = Arc::new(EmptyR(SearchMode::Vector));
    let hybrid = HybridRetriever::with_policy(lex, vec_r, FusionPolicy::Rrf { k: 60 }, 2);
    let q = SearchQuery {
        text: "x".into(),
        mode: SearchMode::Lexical,
        k: 2,
        filters: Default::default(),
    };
    let (_hits, trace) = hybrid.search_with_trace(&q).unwrap();
    assert!(trace.vector.is_empty());
    assert_eq!(trace.timing.vector_ms, 0);
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -p kebab-search hybrid::tests::search_with_trace
```
Expected: compile error — `search_with_trace` undefined.

- [ ] **Step 3: Add `trace.rs` helper module**

Create `crates/kebab-search/src/trace.rs`:

```rust
//! p9-fb-37: trace capture helpers for `HybridRetriever::search_with_trace`.

use std::collections::BTreeMap;

use kebab_core::{
    SearchHit, SearchTrace, TraceCandidate, TraceFusionInput, TraceTiming,
};

/// Build a `TraceCandidate` from a `SearchHit`. The score field reflects
/// each side's score (lexical / vector / fusion) — caller selects which
/// retriever's hit list this is.
pub fn candidates_from_hits(hits: &[SearchHit], score_kind: ScoreKind) -> Vec<TraceCandidate> {
    hits.iter()
        .map(|h| TraceCandidate {
            chunk_id: h.chunk_id.clone(),
            doc_id: h.doc_id.clone(),
            doc_path: h.doc_path.clone(),
            rank: h.rank,
            score: match score_kind {
                ScoreKind::Lexical => h.retrieval.lexical_score.unwrap_or(0.0),
                ScoreKind::Vector => h.retrieval.vector_score.unwrap_or(0.0),
            },
        })
        .collect()
}

#[derive(Clone, Copy, Debug)]
pub enum ScoreKind {
    Lexical,
    Vector,
}

/// Build the union of (chunk_id) across lex and vec hit lists, with
/// each side's rank captured. `fusion_score` is filled by the caller
/// (RRF computes it during fusion, this helper just pre-builds the
/// rank table — caller overwrites fusion_score in a second pass).
pub fn build_fusion_input_skeleton(
    lex: &[SearchHit],
    vec: &[SearchHit],
) -> Vec<TraceFusionInput> {
    let mut by_chunk: BTreeMap<String, TraceFusionInput> = BTreeMap::new();
    for h in lex {
        by_chunk
            .entry(h.chunk_id.0.clone())
            .or_insert(TraceFusionInput {
                chunk_id: h.chunk_id.clone(),
                lexical_rank: None,
                vector_rank: None,
                fusion_score: 0.0,
            })
            .lexical_rank = Some(h.rank);
    }
    for h in vec {
        by_chunk
            .entry(h.chunk_id.0.clone())
            .or_insert(TraceFusionInput {
                chunk_id: h.chunk_id.clone(),
                lexical_rank: None,
                vector_rank: None,
                fusion_score: 0.0,
            })
            .vector_rank = Some(h.rank);
    }
    by_chunk.into_values().collect()
}

/// Container the hybrid retriever fills during a traced run.
#[derive(Default)]
pub struct TraceBuilder {
    pub lexical: Vec<TraceCandidate>,
    pub vector: Vec<TraceCandidate>,
    pub rrf_inputs: Vec<TraceFusionInput>,
    pub timing: TraceTiming,
}

impl TraceBuilder {
    pub fn into_trace(self) -> SearchTrace {
        SearchTrace {
            lexical: self.lexical,
            vector: self.vector,
            rrf_inputs: self.rrf_inputs,
            timing: self.timing,
        }
    }
}
```

Modify `crates/kebab-search/src/lib.rs`. Add module declaration:

```rust
mod trace;
```

- [ ] **Step 4: Add `search_with_trace` on HybridRetriever**

Modify `crates/kebab-search/src/hybrid.rs`. Add at the top (under existing `use` lines):

```rust
use crate::trace::{build_fusion_input_skeleton, candidates_from_hits, ScoreKind, TraceBuilder};
use kebab_core::SearchTrace;
use std::time::Instant;
```

Add a method to `impl HybridRetriever` (place after `fn fuse`):

```rust
/// p9-fb-37: parallel to `Retriever::search` but additionally returns
/// a trace of pre-fusion lex/vec lists, RRF inputs (union with each
/// side's rank), and per-stage timing. Same fan-out logic as `fuse`,
/// just instrumented.
pub fn search_with_trace(
    &self,
    query: &SearchQuery,
) -> anyhow::Result<(Vec<SearchHit>, SearchTrace)> {
    let start_total = Instant::now();
    let target_k = if query.k == 0 { self.default_k } else { query.k };
    let fanout_k = target_k.saturating_mul(HYBRID_FANOUT_MULTIPLIER);
    let fanout_query = SearchQuery {
        k: fanout_k,
        ..query.clone()
    };

    let mut tb = TraceBuilder::default();

    let (lex_hits, vec_hits): (Vec<SearchHit>, Vec<SearchHit>) = match query.mode {
        SearchMode::Lexical => {
            let t0 = Instant::now();
            let lh = self.lexical.search(&fanout_query)?;
            tb.timing.lexical_ms = t0.elapsed().as_millis() as u64;
            (lh, Vec::new())
        }
        SearchMode::Vector => {
            let t0 = Instant::now();
            let vh = self.vector.search(&fanout_query)?;
            tb.timing.vector_ms = t0.elapsed().as_millis() as u64;
            (Vec::new(), vh)
        }
        SearchMode::Hybrid => {
            let t0 = Instant::now();
            let lh = self.lexical.search(&fanout_query)?;
            tb.timing.lexical_ms = t0.elapsed().as_millis() as u64;
            let t1 = Instant::now();
            let vh = self.vector.search(&fanout_query)?;
            tb.timing.vector_ms = t1.elapsed().as_millis() as u64;
            (lh, vh)
        }
    };

    tb.lexical = candidates_from_hits(&lex_hits, ScoreKind::Lexical);
    tb.vector = candidates_from_hits(&vec_hits, ScoreKind::Vector);
    tb.rrf_inputs = build_fusion_input_skeleton(&lex_hits, &vec_hits);

    let t_fusion = Instant::now();
    let final_hits = match query.mode {
        SearchMode::Lexical => {
            let mut h = lex_hits.clone();
            h.truncate(target_k);
            h
        }
        SearchMode::Vector => {
            let mut h = vec_hits.clone();
            h.truncate(target_k);
            h
        }
        SearchMode::Hybrid => self.fuse_with_inputs(&lex_hits, &vec_hits, target_k)?,
    };
    tb.timing.fusion_ms = t_fusion.elapsed().as_millis() as u64;

    // Backfill fusion_score onto the rrf_inputs union for each chunk
    // present in the final fused list.
    let score_by_chunk: std::collections::HashMap<String, f32> = final_hits
        .iter()
        .map(|h| (h.chunk_id.0.clone(), h.retrieval.fusion_score))
        .collect();
    for entry in &mut tb.rrf_inputs {
        if let Some(s) = score_by_chunk.get(&entry.chunk_id.0) {
            entry.fusion_score = *s;
        }
    }

    tb.timing.total_ms = start_total.elapsed().as_millis() as u64;
    Ok((final_hits, tb.into_trace()))
}
```

`fuse_with_inputs` is needed — extract from existing `fuse` so both `Retriever::search` (hybrid mode) and `search_with_trace` reuse the same RRF body without re-querying retrievers.

Refactoring recipe:
1. Read existing `fn fuse` (at line ~145). Note the body issues two `.search()` calls then builds `lex_index` / `vec_index` via `.into_iter()`.
2. Split into two functions. `fn fuse` keeps the two `.search()` calls, then delegates the rest. `fn fuse_with_inputs` takes the already-resolved hit slices.
3. Inside `fuse_with_inputs`: replace `let lex_index: HashMap<...> = lex_hits.into_iter().map(...).collect();` with `let lex_index: HashMap<...> = lex_hits.iter().cloned().map(...).collect();` (mirror for vec_index). All other RRF logic stays identical.

```rust
fn fuse(&self, query: &SearchQuery) -> Result<Vec<SearchHit>> {
    let target_k = if query.k == 0 { self.default_k } else { query.k };
    let fanout_k = target_k.saturating_mul(HYBRID_FANOUT_MULTIPLIER);
    let fanout_query = SearchQuery {
        k: fanout_k,
        ..query.clone()
    };
    let lex_hits = self.lexical.search(&fanout_query)?;
    let vec_hits = self.vector.search(&fanout_query)?;
    self.fuse_with_inputs(&lex_hits, &vec_hits, target_k)
}

fn fuse_with_inputs(
    &self,
    lex_hits: &[SearchHit],
    vec_hits: &[SearchHit],
    target_k: usize,
) -> Result<Vec<SearchHit>> {
    tracing::debug!(
        lex = lex_hits.len(),
        vec = vec_hits.len(),
        target_k,
        "kb-search hybrid: pre-fusion candidate counts"
    );
    // PASTE the rest of the original `fn fuse` body here. Two changes:
    //   - replace `lex_hits.into_iter()` with `lex_hits.iter().cloned()`
    //   - replace `vec_hits.into_iter()` with `vec_hits.iter().cloned()`
    // Everything else (RRF score formula, sort, truncate to target_k,
    // tie-breaking, `Ok(...)` return) is verbatim preserved.
}
```

Verify with `cargo test -p kebab-search` — existing hybrid tests must still pass (they exercise the `Retriever::search` → `fuse` path).

- [ ] **Step 5: Run tests**

```bash
cargo test -p kebab-search
```
Expected: existing hybrid tests still pass + 2 new search_with_trace tests pass.

- [ ] **Step 6: Clippy gate**

```bash
cargo clippy -p kebab-search --all-targets -- -D warnings
```
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add crates/kebab-search/src/trace.rs crates/kebab-search/src/hybrid.rs crates/kebab-search/src/lib.rs
git commit -m "feat(search): HybridRetriever::search_with_trace (fb-37)"
```

---

## Task 5: SearchResponse trace field + App::search_with_opts threading

**Files:**
- Modify: `crates/kebab-app/src/app.rs`

- [ ] **Step 1: Write failing test**

Append to `crates/kebab-app/src/app.rs` tests module (find existing `#[cfg(test)] mod tests` near bottom; if absent, add one at file end):

```rust
#[cfg(test)]
mod tests_trace {
    use super::*;
    use kebab_core::{SearchOpts, SearchQuery, SearchMode};

    fn open_app_with_temp_dir() -> (tempfile::TempDir, App) {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = kebab_config::Config::defaults();
        cfg.storage.data_dir = dir.path().to_string_lossy().into_owned();
        // Ensure DB exists.
        let store = kebab_store_sqlite::SqliteStore::open(&cfg).unwrap();
        store.run_migrations().unwrap();
        drop(store);
        let app = App::open_with_config(cfg).unwrap();
        (dir, app)
    }

    #[test]
    fn search_response_trace_none_when_opts_trace_false() {
        let (_dir, app) = open_app_with_temp_dir();
        let q = SearchQuery {
            text: "x".into(),
            mode: SearchMode::Lexical,
            k: 1,
            filters: Default::default(),
        };
        let resp = app.search_with_opts(q, SearchOpts::default()).unwrap();
        assert!(resp.trace.is_none());
    }

    #[test]
    fn search_response_trace_some_when_opts_trace_true() {
        let (_dir, app) = open_app_with_temp_dir();
        let q = SearchQuery {
            text: "x".into(),
            mode: SearchMode::Lexical,
            k: 1,
            filters: Default::default(),
        };
        let opts = SearchOpts { trace: true, ..Default::default() };
        let resp = app.search_with_opts(q, opts).unwrap();
        assert!(resp.trace.is_some(), "trace populated when opts.trace=true");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -p kebab-app tests_trace
```
Expected: compile errors — `SearchResponse.trace` field absent.

- [ ] **Step 3: Extend `SearchResponse`**

In `crates/kebab-app/src/app.rs`, replace `pub struct SearchResponse` (~line 69):

```rust
#[derive(Clone, Debug)]
pub struct SearchResponse {
    pub hits: Vec<SearchHit>,
    pub next_cursor: Option<String>,
    pub truncated: bool,
    /// p9-fb-37: present when caller passed `SearchOpts.trace = true`.
    /// Consumers that ignore trace should leave this `None`.
    pub trace: Option<kebab_core::SearchTrace>,
}
```

- [ ] **Step 4: Thread through `App::search_with_opts`**

In `crates/kebab-app/src/app.rs`, modify `pub fn search_with_opts` (~line 306) to honor `opts.trace`. Find the current `let mut all_hits = self.search(fetch_query)?;` line and replace surrounding logic:

```rust
let trace = if opts.trace {
    // Build a trace-capable retriever directly. Re-use construction
    // from the cached search path but bypass cache (debug intent).
    let retriever = self.build_retriever()?;
    let traced = retriever
        .as_any()
        .downcast_ref::<kebab_search::HybridRetriever>()
        .map(|h| h.search_with_trace(&fetch_query));
    if let Some(Ok((hits, t))) = traced {
        let mut all_hits = hits;
        let drop_n = offset.min(all_hits.len());
        all_hits.drain(..drop_n);
        let final_hits: Vec<SearchHit> = all_hits.into_iter().take(k_effective).collect();
        return Ok(self.build_response(final_hits, k_effective, &opts, snippet_chars, Some(t)));
    }
    None
} else {
    None
};

let mut all_hits = self.search(fetch_query)?;
// ... existing code ...
```

Engineer note: this is a sketch — review actual `App::search_with_opts` body before editing; the `build_retriever` / `as_any` / `build_response` helpers may not exist verbatim. The minimal change required is:
1. When `opts.trace = true`, call `search_with_trace` on the hybrid retriever (constructed the same way `App::search_uncached` does).
2. Bypass the search cache entirely.
3. Plug the resulting `SearchTrace` into `SearchResponse.trace`.

Use the existing `App::search_uncached` (line ~243) as the model — duplicate that path with `search_with_trace` and wrap the result. Look for: `let retriever = ... HybridRetriever::new(&self.config, lex, vec);`. Call `retriever.search_with_trace(&query)` instead of `retriever.search(&query)` when tracing.

If the retriever is constructed only as `Arc<dyn Retriever>` (and `search_with_trace` is not on the trait), add a concrete-typed local construction in the `if opts.trace` branch. Example pattern:

```rust
// inside fn search_with_opts:
if opts.trace {
    use kebab_search::HybridRetriever;
    let lex = self.build_lexical_retriever()?;
    let vec = self.build_vector_retriever()?;
    let retriever = HybridRetriever::new(&self.config, lex, vec);
    let (hits, trace) = retriever.search_with_trace(&fetch_query)?;
    // skip cache, run budget loop on hits, attach trace to response
    return Ok(self.finalize_response(hits, k_effective, offset, &opts, snippet_chars, Some(trace)));
}
```

The exact helpers (`build_lexical_retriever`, `finalize_response`) are method names you'll either find or extract during implementation. Goal: trace path bypasses cache and returns `Some(trace)`; non-trace path unchanged returns `None`.

Also update every other `SearchResponse { ... }` constructor in `app.rs` and `lib.rs` to include `trace: None`. Search for `SearchResponse {` to find all sites.

```bash
grep -n "SearchResponse {" crates/kebab-app/src/app.rs crates/kebab-app/src/lib.rs
```

- [ ] **Step 5: Run tests**

```bash
cargo test -p kebab-app tests_trace
cargo test -p kebab-app
```
Expected: 2 new trace tests pass; existing app tests unaffected.

- [ ] **Step 6: Workspace clippy**

```bash
cargo clippy -p kebab-app --all-targets -- -D warnings
```
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add crates/kebab-app/src/app.rs
git commit -m "feat(app): SearchResponse.trace + opts.trace threading (fb-37)"
```

---

## Task 6: CLI --trace flag + JSON wire + non-JSON pretty print

**Files:**
- Modify: `crates/kebab-cli/src/main.rs`
- Modify: `crates/kebab-cli/src/wire.rs`

- [ ] **Step 1: Write failing test for wire serialization**

Append to `crates/kebab-cli/src/wire.rs` `mod tests`:

```rust
#[test]
fn search_response_with_trace_serializes_trace_field() {
    use kebab_core::{SearchTrace, TraceCandidate, TraceFusionInput,
                     TraceTiming, ChunkId, DocumentId, WorkspacePath};
    let r = kebab_app::SearchResponse {
        hits: vec![],
        next_cursor: None,
        truncated: false,
        trace: Some(SearchTrace {
            lexical: vec![TraceCandidate {
                chunk_id: ChunkId("c1".into()),
                doc_id: DocumentId("d1".into()),
                doc_path: WorkspacePath::new("a.md".into()).unwrap(),
                rank: 1,
                score: 0.42,
            }],
            vector: vec![],
            rrf_inputs: vec![TraceFusionInput {
                chunk_id: ChunkId("c1".into()),
                lexical_rank: Some(1),
                vector_rank: None,
                fusion_score: 0.0,
            }],
            timing: TraceTiming { lexical_ms: 5, vector_ms: 0, fusion_ms: 1, total_ms: 7 },
        }),
    };
    let v = wire_search_response(&r);
    assert_eq!(v["schema_version"], "search_response.v1");
    assert!(v["trace"].is_object());
    assert_eq!(v["trace"]["timing"]["lexical_ms"], 5);
    assert_eq!(v["trace"]["lexical"][0]["chunk_id"], "c1");
}

#[test]
fn search_response_without_trace_omits_field() {
    let r = kebab_app::SearchResponse {
        hits: vec![],
        next_cursor: None,
        truncated: false,
        trace: None,
    };
    let v = wire_search_response(&r);
    assert!(v.get("trace").is_none(), "trace field absent when None");
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p kebab-cli wire::tests::search_response_with_trace_serializes_trace_field
```
Expected: compile error — `SearchResponse.trace` not threaded into wire helper output.

- [ ] **Step 3: Update `wire_search_response`**

Modify `crates/kebab-cli/src/wire.rs` `wire_search_response`:

```rust
pub fn wire_search_response(r: &kebab_app::SearchResponse) -> Value {
    let mut v = serde_json::json!({
        "hits": r.hits.iter().map(wire_search_hit).collect::<Vec<_>>(),
        "next_cursor": r.next_cursor,
        "truncated": r.truncated,
    });
    if let Some(trace) = &r.trace {
        let trace_v = serde_json::to_value(trace).expect("SearchTrace serializes");
        if let Value::Object(ref mut map) = v {
            map.insert("trace".to_string(), trace_v);
        }
    }
    tag_object(v, "search_response.v1")
}
```

- [ ] **Step 4: Add `--trace` clap flag**

Modify `crates/kebab-cli/src/main.rs`. Find `Cmd::Search { ... }` definition (~line 95-150). Add at the end of its field list (after `doc_id`):

```rust
        /// p9-fb-37: emit pre-fusion lexical / vector / RRF candidate
        /// lists + per-stage timing in the response. Bypasses cache
        /// (debug intent — fresh run guaranteed).
        #[arg(long)]
        trace: bool,
```

Find the `Cmd::Search` dispatch arm (~line 656). Add `trace,` to the destructure pattern (after `doc_id,`). Find where `SearchOpts` is constructed (~look for `SearchOpts {` inside the search arm, ~line 745) and add `trace: *trace,`. Example:

```rust
let opts = kebab_core::SearchOpts {
    max_tokens: *max_tokens,
    snippet_chars: *snippet_chars,
    cursor: cursor.clone(),
    trace: *trace,
};
```

- [ ] **Step 5: Add non-JSON pretty-print**

Find the search dispatch's non-JSON branch (the `else` of `if cli.json`, ~line 750-780). After hits are printed, add:

```rust
if *trace {
    if let Some(t) = &resp.trace {
        eprintln!();
        eprintln!("Trace:");
        eprintln!("  lexical ({} hits, {}ms):", t.lexical.len(), t.timing.lexical_ms);
        for c in t.lexical.iter().take(3) {
            eprintln!("    rank={} score={:.4} chunk={}", c.rank, c.score, c.chunk_id.0);
        }
        eprintln!("  vector ({} hits, {}ms):", t.vector.len(), t.timing.vector_ms);
        for c in t.vector.iter().take(3) {
            eprintln!("    rank={} score={:.4} chunk={}", c.rank, c.score, c.chunk_id.0);
        }
        eprintln!("  fusion ({} inputs, {}ms)", t.rrf_inputs.len(), t.timing.fusion_ms);
        eprintln!("  total: {}ms", t.timing.total_ms);
    }
}
```

- [ ] **Step 6: Run tests**

```bash
cargo test -p kebab-cli wire::tests
cargo test -p kebab-cli
```
Expected: 2 new wire tests pass; existing cli tests unaffected.

- [ ] **Step 7: Clippy**

```bash
cargo clippy -p kebab-cli --all-targets -- -D warnings
```
Expected: clean.

- [ ] **Step 8: Commit**

```bash
git add crates/kebab-cli/src/main.rs crates/kebab-cli/src/wire.rs
git commit -m "feat(cli): kebab search --trace flag + wire trace + pretty print (fb-37)"
```

---

## Task 7: CLI integration tests for --trace and stats breakdowns

**Files:**
- Create: `crates/kebab-cli/tests/wire_search_trace.rs`
- Create: `crates/kebab-cli/tests/wire_schema_breakdowns.rs`

- [ ] **Step 1: Write failing integration tests for --trace**

Create `crates/kebab-cli/tests/wire_search_trace.rs`. Use the same fixture pattern as existing `crates/kebab-cli/tests/wire_search_filters.rs` (read it first to mirror temp-dir + ingest setup):

```rust
//! p9-fb-37: integration tests for `kebab search --trace --json`.

use std::process::Command;

mod common;
use common::{cargo_bin, ingest_fixture, temp_kebab_root};

#[test]
fn search_trace_json_includes_trace_block() {
    let (_root, cfg_path) = temp_kebab_root();
    ingest_fixture(&cfg_path, "doc1.md", "# Title\n\nrust async hello\n");

    let out = Command::new(cargo_bin())
        .args([
            "--config", cfg_path.to_str().unwrap(),
            "search", "rust", "--trace", "--json",
        ])
        .output()
        .expect("run");
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["schema_version"], "search_response.v1");
    assert!(v["trace"].is_object(), "trace block present");
    assert!(v["trace"]["timing"].is_object());
    assert!(v["trace"]["timing"]["total_ms"].is_number());
    assert!(v["trace"]["lexical"].is_array());
    assert!(v["trace"]["vector"].is_array());
    assert!(v["trace"]["rrf_inputs"].is_array());
}

#[test]
fn search_without_trace_omits_trace_field() {
    let (_root, cfg_path) = temp_kebab_root();
    ingest_fixture(&cfg_path, "doc1.md", "# Title\n\nrust async hello\n");

    let out = Command::new(cargo_bin())
        .args([
            "--config", cfg_path.to_str().unwrap(),
            "search", "rust", "--json",
        ])
        .output()
        .expect("run");
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(v.get("trace").is_none(), "trace field absent when --trace not passed");
}

#[test]
fn search_trace_lexical_mode_empty_vector_list() {
    let (_root, cfg_path) = temp_kebab_root();
    ingest_fixture(&cfg_path, "doc1.md", "# Title\n\nrust async hello\n");

    let out = Command::new(cargo_bin())
        .args([
            "--config", cfg_path.to_str().unwrap(),
            "search", "rust", "--trace", "--mode", "lexical", "--json",
        ])
        .output()
        .expect("run");
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["trace"]["vector"].as_array().unwrap().len(), 0);
    assert_eq!(v["trace"]["timing"]["vector_ms"], 0);
}
```

- [ ] **Step 2: Write failing integration tests for stats**

Create `crates/kebab-cli/tests/wire_schema_breakdowns.rs`:

```rust
//! p9-fb-37: integration tests for `kebab schema --json` extended stats.

use std::process::Command;

mod common;
use common::{cargo_bin, ingest_fixture, temp_kebab_root};

#[test]
fn schema_stats_includes_breakdowns_on_fresh_corpus() {
    let (_root, cfg_path) = temp_kebab_root();
    // Fresh init — no docs. We need migrations to have run; the
    // first search/ingest call brings them up. Run an empty schema
    // query on a freshly-init'd config:
    Command::new(cargo_bin())
        .args(["--config", cfg_path.to_str().unwrap(), "init"])
        .output()
        .expect("init");

    let out = Command::new(cargo_bin())
        .args(["--config", cfg_path.to_str().unwrap(), "schema", "--json"])
        .output()
        .expect("run");
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let stats = &v["stats"];
    // 5 keys padded.
    let m = stats["media_breakdown"].as_object().unwrap();
    assert_eq!(m.len(), 5);
    for k in &["markdown", "pdf", "image", "audio", "other"] {
        assert_eq!(m[*k], 0);
    }
    // lang_breakdown empty {}.
    assert_eq!(stats["lang_breakdown"].as_object().unwrap().len(), 0);
    // index_bytes shape.
    assert!(stats["index_bytes"]["sqlite"].is_number());
    assert!(stats["index_bytes"]["lancedb"].is_number());
    assert_eq!(stats["stale_doc_count"], 0);
}

#[test]
fn schema_stats_breakdowns_after_ingest() {
    let (_root, cfg_path) = temp_kebab_root();
    ingest_fixture(&cfg_path, "a.md", "---\nlang: en\n---\nhello\n");
    ingest_fixture(&cfg_path, "b.md", "---\nlang: ko\n---\n안녕\n");

    let out = Command::new(cargo_bin())
        .args(["--config", cfg_path.to_str().unwrap(), "schema", "--json"])
        .output()
        .expect("run");
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let stats = &v["stats"];
    assert_eq!(stats["media_breakdown"]["markdown"], 2);
    assert_eq!(stats["lang_breakdown"]["en"], 1);
    assert_eq!(stats["lang_breakdown"]["ko"], 1);
    assert!(stats["index_bytes"]["sqlite"].as_u64().unwrap() > 0);
}
```

- [ ] **Step 3: Verify or create `tests/common/mod.rs`**

Check existing tests for shared `common` module:
```bash
ls crates/kebab-cli/tests/
cat crates/kebab-cli/tests/common/mod.rs 2>/dev/null
```

If `common` module exists with `cargo_bin`, `ingest_fixture`, `temp_kebab_root`, reuse. If not, mirror functions from `wire_search_filters.rs` (the fb-36 integration test) — copy its fixture helpers to `crates/kebab-cli/tests/common/mod.rs` and reference via `mod common`.

- [ ] **Step 4: Run integration tests**

```bash
cargo test -p kebab-cli --test wire_search_trace
cargo test -p kebab-cli --test wire_schema_breakdowns
```
Expected: all tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/kebab-cli/tests/wire_search_trace.rs crates/kebab-cli/tests/wire_schema_breakdowns.rs crates/kebab-cli/tests/common/mod.rs
git commit -m "test(cli): integration tests for --trace + schema breakdowns (fb-37)"
```

---

## Task 8: MCP SearchInput trace + integration test

**Files:**
- Modify: `crates/kebab-mcp/src/tools/search.rs`
- Create: `crates/kebab-mcp/tests/tools_call_search_trace.rs`

- [ ] **Step 1: Write failing integration test**

Create `crates/kebab-mcp/tests/tools_call_search_trace.rs`. Mirror existing `tools_call_search.rs` fixture pattern (read it first):

```rust
//! p9-fb-37: MCP search trace input/output integration.

use serde_json::json;

mod common;
use common::call_tool_with_temp_corpus;

#[test]
fn search_with_trace_true_returns_trace_field() {
    let v = call_tool_with_temp_corpus(
        "kebab__search",
        json!({"query": "rust", "trace": true}),
    );
    assert!(v["trace"].is_object(), "trace field present when trace:true");
    assert!(v["trace"]["timing"]["total_ms"].is_number());
}

#[test]
fn search_without_trace_omits_field() {
    let v = call_tool_with_temp_corpus(
        "kebab__search",
        json!({"query": "rust"}),
    );
    assert!(v.get("trace").is_none(), "trace absent when not requested");
}

#[test]
fn search_with_trace_false_omits_field() {
    let v = call_tool_with_temp_corpus(
        "kebab__search",
        json!({"query": "rust", "trace": false}),
    );
    assert!(v.get("trace").is_none());
}
```

If `tests/common/mod.rs` lacks `call_tool_with_temp_corpus`, derive from existing test fixtures. Pattern: spin up `kebab_mcp::Server`, send tools/call request, return result `serde_json::Value`.

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -p kebab-mcp --test tools_call_search_trace
```
Expected: compile error — `SearchInput.trace` field absent.

- [ ] **Step 3: Add `trace` to `SearchInput`**

Modify `crates/kebab-mcp/src/tools/search.rs`. Find `pub struct SearchInput` (~line 30-50). Add at end:

```rust
    /// p9-fb-37: when true, capture pipeline trace and include in
    /// response. Bypasses cache. Default false.
    #[serde(default)]
    pub trace: Option<bool>,
```

- [ ] **Step 4: Wire `trace` into dispatch**

Find the dispatch body where `SearchOpts` is constructed (~line 90-130). Add:

```rust
let opts = kebab_core::SearchOpts {
    max_tokens: input.max_tokens,
    snippet_chars: input.snippet_chars,
    cursor: input.cursor.clone(),
    trace: input.trace.unwrap_or(false),
};
```

(The existing struct construction may not include `cursor` etc — adapt to what's actually present, just add `trace:` line.)

The output JSON should already pick up `trace` because the wire helper inherits from the same `SearchResponse` shape. Verify by searching for how the MCP tool serializes its response — check whether it uses `kebab_cli::wire::wire_search_response` or its own builder.

```bash
grep -n "wire_search_response\|search_response.v1\|SearchResponse" crates/kebab-mcp/src/tools/search.rs
```

If MCP uses its own builder, mirror the trace-injection pattern from Task 6 Step 3.

- [ ] **Step 5: Run tests**

```bash
cargo test -p kebab-mcp --test tools_call_search_trace
```
Expected: all 3 pass.

- [ ] **Step 6: Clippy**

```bash
cargo clippy -p kebab-mcp --all-targets -- -D warnings
```

- [ ] **Step 7: Commit**

```bash
git add crates/kebab-mcp/src/tools/search.rs crates/kebab-mcp/tests/tools_call_search_trace.rs
git commit -m "feat(mcp): kebab__search trace input + output mirror (fb-37)"
```

---

## Task 9: TUI search pane `t` keystroke + TracePopup

**Files:**
- Create: `crates/kebab-tui/src/trace_popup.rs`
- Modify: `crates/kebab-tui/src/lib.rs`
- Modify: `crates/kebab-tui/src/app.rs`
- Modify: `crates/kebab-tui/src/search.rs`
- Modify: `crates/kebab-tui/src/cheatsheet.rs`

- [ ] **Step 1: Create `trace_popup.rs`**

```rust
//! p9-fb-37: TUI trace popup. Opens from Search pane via `t` key
//! when results are visible. Re-runs the current query with
//! `SearchOpts.trace = true` and displays the lex / vec / rrf union
//! + per-stage timing as a single scroll list.

use crossterm::event::{KeyCode, KeyEvent};
use kebab_core::SearchTrace;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

#[derive(Debug, Clone)]
pub struct TracePopupState {
    pub trace: SearchTrace,
    pub scroll: u16,
}

impl TracePopupState {
    pub fn new(trace: SearchTrace) -> Self {
        Self { trace, scroll: 0 }
    }
}

pub fn render_trace_popup(f: &mut Frame, area: Rect, state: &TracePopupState) {
    let mut lines: Vec<Line> = Vec::new();
    let bold = Style::default().add_modifier(Modifier::BOLD);

    lines.push(Line::from(Span::styled(
        format!(
            "Lexical ({} hits, {} ms)",
            state.trace.lexical.len(),
            state.trace.timing.lexical_ms,
        ),
        bold,
    )));
    for c in &state.trace.lexical {
        lines.push(Line::from(format!(
            "  #{:>2} score={:.4} chunk={}",
            c.rank, c.score, c.chunk_id.0
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!(
            "Vector ({} hits, {} ms)",
            state.trace.vector.len(),
            state.trace.timing.vector_ms,
        ),
        bold,
    )));
    for c in &state.trace.vector {
        lines.push(Line::from(format!(
            "  #{:>2} score={:.4} chunk={}",
            c.rank, c.score, c.chunk_id.0
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!(
            "RRF inputs ({} entries, {} ms fusion)",
            state.trace.rrf_inputs.len(),
            state.trace.timing.fusion_ms,
        ),
        bold,
    )));
    for e in &state.trace.rrf_inputs {
        lines.push(Line::from(format!(
            "  chunk={} lex={:?} vec={:?} fusion={:.4}",
            e.chunk_id.0, e.lexical_rank, e.vector_rank, e.fusion_score
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("Total: {} ms", state.trace.timing.total_ms),
        bold,
    )));

    let block = Block::default()
        .title("Trace — Esc to close, j/k or ↑↓ to scroll")
        .borders(Borders::ALL);
    let p = Paragraph::new(lines)
        .block(block)
        .scroll((state.scroll, 0))
        .wrap(Wrap { trim: false });
    f.render_widget(p, area);
}

/// Handle keys while popup is open. Returns true if the popup should
/// close.
pub fn handle_key_trace_popup(state: &mut TracePopupState, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Esc => true,
        KeyCode::Char('j') | KeyCode::Down => {
            state.scroll = state.scroll.saturating_add(1);
            false
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.scroll = state.scroll.saturating_sub(1);
            false
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;
    use kebab_core::TraceTiming;

    fn dummy_state() -> TracePopupState {
        TracePopupState::new(SearchTrace {
            lexical: vec![],
            vector: vec![],
            rrf_inputs: vec![],
            timing: TraceTiming::default(),
        })
    }

    #[test]
    fn esc_closes() {
        let mut s = dummy_state();
        assert!(handle_key_trace_popup(
            &mut s,
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
        ));
    }

    #[test]
    fn j_scrolls_down() {
        let mut s = dummy_state();
        assert!(!handle_key_trace_popup(
            &mut s,
            KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
        ));
        assert_eq!(s.scroll, 1);
    }
}
```

- [ ] **Step 2: Register module + state**

Modify `crates/kebab-tui/src/lib.rs`:
```rust
pub mod trace_popup;
```

Modify `crates/kebab-tui/src/app.rs`. Find `pub struct App` (~line 1-100). Add field:
```rust
    /// p9-fb-37: trace popup state, `Some` while open.
    pub trace_popup: Option<crate::trace_popup::TracePopupState>,
```

Initialize in `App::new` / `App::default` to `None`.

- [ ] **Step 3: Wire `t` keystroke in search pane**

Modify `crates/kebab-tui/src/search.rs` `pub fn handle_key_search` (~line 196). Add a key arm in the match block before existing arms:

```rust
        (KeyCode::Char('t'), KeyModifiers::NONE)
            if !state.results.is_empty() && state.trace_popup.is_none() =>
        {
            // Re-run current query with trace enabled.
            let cfg = match kebab_config::Config::load(state.config_path.as_deref()) {
                Ok(c) => c,
                Err(_) => return KeyOutcome::Consumed,
            };
            let q = kebab_core::SearchQuery {
                text: state.query.clone(),
                mode: state.mode,
                k: state.k,
                filters: state.filters.clone(),
            };
            let opts = kebab_core::SearchOpts {
                trace: true,
                ..Default::default()
            };
            if let Ok(resp) = kebab_app::search_with_opts_with_config(cfg, q, opts) {
                if let Some(t) = resp.trace {
                    state.trace_popup = Some(crate::trace_popup::TracePopupState::new(t));
                }
            }
            KeyOutcome::Consumed
        }
```

Engineer note: field names (`state.results`, `state.query`, `state.mode`, `state.k`, `state.filters`, `state.config_path`) must match actual `App` struct. Inspect `kebab-tui/src/app.rs` and adapt — if some are absent (e.g. `config_path`), fall back to `kebab_config::Config::load(None)`.

- [ ] **Step 4: Render popup + handle popup keys in main loop**

Find the main render loop (in `crates/kebab-tui/src/run.rs` or `app.rs`) — wherever `render_search` / `render_inspect` are conditionally called. Add a render check: if `state.trace_popup.is_some()`, draw the popup overlay. Pattern:

```rust
if let Some(popup) = &state.trace_popup {
    let popup_area = centered_rect(80, 80, frame.area());
    crate::trace_popup::render_trace_popup(frame, popup_area, popup);
}
```

`centered_rect` helper may already exist (commonly in `app.rs` or `terminal.rs`). If not, define it inline:

```rust
fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
```

In key dispatch, intercept popup keys first:

```rust
if let Some(popup) = state.trace_popup.as_mut() {
    if crate::trace_popup::handle_key_trace_popup(popup, key) {
        state.trace_popup = None;
    }
    return KeyOutcome::Consumed;
}
```

Place before the per-pane key dispatch.

- [ ] **Step 5: Update cheatsheet**

Modify `crates/kebab-tui/src/cheatsheet.rs`. Find the search pane keybind list (search for "Search" header or `i = inspect`). Add:

```rust
    "t = trace",
```

(Exact insertion depends on cheatsheet's data structure — array of strings, struct rows, etc. Adapt.)

- [ ] **Step 6: Run TUI tests**

```bash
cargo test -p kebab-tui
```
Expected: 2 new trace_popup tests pass; existing TUI tests unaffected.

- [ ] **Step 7: Clippy**

```bash
cargo clippy -p kebab-tui --all-targets -- -D warnings
```

- [ ] **Step 8: Commit**

```bash
git add crates/kebab-tui/src/trace_popup.rs crates/kebab-tui/src/lib.rs \
        crates/kebab-tui/src/app.rs crates/kebab-tui/src/search.rs \
        crates/kebab-tui/src/cheatsheet.rs crates/kebab-tui/src/run.rs
git commit -m "feat(tui): search pane t-key opens TracePopup (fb-37)"
```

---

## Task 10: Wire schema docs + README + SMOKE + INDEX + SKILL + status flip

**Files:**
- Modify: `docs/wire-schema/v1/search_response.schema.json`
- Modify: `docs/wire-schema/v1/schema.schema.json`
- Modify: `README.md`
- Modify: `docs/SMOKE.md`
- Modify: `tasks/p9/p9-fb-37-trace-and-stats.md`
- Modify: `tasks/INDEX.md`
- Modify: `integrations/claude-code/kebab/SKILL.md`

- [ ] **Step 1: Update `search_response.schema.json`**

Add `trace` to `properties` (NOT to `required`):

```json
"trace": {
  "type": "object",
  "description": "p9-fb-37: present iff caller passed --trace / SearchOpts.trace=true. Lex/vec pre-fusion lists + RRF union + per-stage timing.",
  "required": ["lexical", "vector", "rrf_inputs", "timing"],
  "properties": {
    "lexical":   { "type": "array", "items": { "type": "object" } },
    "vector":    { "type": "array", "items": { "type": "object" } },
    "rrf_inputs":{ "type": "array", "items": { "type": "object" } },
    "timing": {
      "type": "object",
      "required": ["lexical_ms", "vector_ms", "fusion_ms", "total_ms"],
      "properties": {
        "lexical_ms": { "type": "integer", "minimum": 0 },
        "vector_ms":  { "type": "integer", "minimum": 0 },
        "fusion_ms":  { "type": "integer", "minimum": 0 },
        "total_ms":   { "type": "integer", "minimum": 0 }
      }
    }
  }
}
```

- [ ] **Step 2: Update `schema.schema.json`**

In `properties.stats.properties`, add the four new fields:

```json
"media_breakdown": {
  "type": "object",
  "description": "p9-fb-37: per-media-kind doc count. 5 keys (markdown/pdf/image/audio/other), zero-padded.",
  "additionalProperties": { "type": "integer", "minimum": 0 }
},
"lang_breakdown": {
  "type": "object",
  "description": "p9-fb-37: per-language doc count. NULL lang keyed as the literal string 'null'. Map may be empty on empty corpus.",
  "additionalProperties": { "type": "integer", "minimum": 0 }
},
"index_bytes": {
  "type": "object",
  "description": "p9-fb-37: on-disk byte sums.",
  "required": ["sqlite", "lancedb"],
  "properties": {
    "sqlite":  { "type": "integer", "minimum": 0 },
    "lancedb": { "type": "integer", "minimum": 0 }
  }
},
"stale_doc_count": {
  "type": "integer",
  "minimum": 0,
  "description": "p9-fb-37: docs whose updated_at exceeds config.search.stale_threshold_days. 0 when threshold=0."
}
```

- [ ] **Step 3: Update `README.md`**

Find the `kebab search` row in the command table. Add `--trace` to its flag list. Find the `kebab schema` row — extend its description with one phrase like "+ media/lang/bytes/stale breakdowns (fb-37)".

- [ ] **Step 4: Update `docs/SMOKE.md`**

Add a new section after the fb-36 walkthrough:

```markdown
### Trace + stats (fb-37)

Re-run a search with `--trace` to see per-stage candidate lists + timing:

```bash
kebab --config /tmp/kebab-smoke/config.toml search "rust async" --trace --json | jq .trace
```

Inspect the corpus health surface:

```bash
kebab --config /tmp/kebab-smoke/config.toml schema --json | jq .stats
```

Look for: `media_breakdown` (5 keys), `lang_breakdown`, `index_bytes`, `stale_doc_count`.
```

- [ ] **Step 5: Update `tasks/p9/p9-fb-37-trace-and-stats.md`**

Flip the frontmatter `status: open` → `status: completed`. Add at the top (after the existing skeleton banner) a "Design + plan" links block:

```markdown
- Design: [`docs/superpowers/specs/2026-05-10-p9-fb-37-trace-and-stats-design.md`](../../docs/superpowers/specs/2026-05-10-p9-fb-37-trace-and-stats-design.md)
- Plan: [`docs/superpowers/plans/2026-05-10-p9-fb-37-trace-and-stats.md`](../../docs/superpowers/plans/2026-05-10-p9-fb-37-trace-and-stats.md)
```

- [ ] **Step 6: Update `tasks/INDEX.md`**

Find the fb-37 row. Flip the status column to ✅.

- [ ] **Step 7: Update `integrations/claude-code/kebab/SKILL.md`**

Find the `mcp__kebab__search` input shape block. Append a `trace: null` field. Add a sentence under the search inputs bullet list noting that `trace: true` returns a `trace` block on the response with pre-fusion lex/vec lists + per-stage timing, and that trace bypasses the search cache. Also update the schema bullet list to mention the new stats sub-fields.

- [ ] **Step 8: Run full workspace tests + clippy**

```bash
cargo test --workspace --no-fail-fast -j 1
cargo clippy --workspace --all-targets -- -D warnings
```
Expected: all green.

- [ ] **Step 9: Commit**

```bash
git add docs/ README.md tasks/p9/p9-fb-37-trace-and-stats.md tasks/INDEX.md integrations/claude-code/kebab/SKILL.md
git commit -m "docs(fb-37): wire schema + README + SMOKE + INDEX + SKILL"
```

---

## Final verification checklist

- [ ] `cargo test --workspace --no-fail-fast -j 1` green
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` clean
- [ ] Manual smoke against `/tmp/kebab-smoke`:
  - [ ] `kebab search Q --trace --json | jq .trace` shows lex/vec/rrf/timing
  - [ ] `kebab search Q --json` does NOT include `trace`
  - [ ] `kebab schema --json | jq .stats` shows 4 new fields
- [ ] README, SMOKE, SKILL, INDEX, spec status all updated
