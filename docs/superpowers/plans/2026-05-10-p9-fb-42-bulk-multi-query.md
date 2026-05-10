# fb-42 Bulk Multi-Query Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add bulk multi-query surface — `kebab search --bulk` (CLI stdin ndjson) + `mcp__kebab__bulk_search` tool — so agents can issue N queries per round-trip with the App instance / cache reused.

**Architecture:** kebab-core domain types (BulkSearchItem / Summary / Response). kebab-app `bulk_search_with_config` facade runs sequential for-loop, reusing one App. CLI parses stdin ndjson and emits per-query stdout ndjson + summary stderr. MCP tool wraps the same facade with a JSON envelope. Per-query failures embed `error.v1` in their item (continue, no abort). Caps at 100 queries per call.

**Tech Stack:** Rust 2024, serde, serde_json, anyhow, JSON Schema 2020-12.

**Spec:** `docs/superpowers/specs/2026-05-10-p9-fb-42-bulk-multi-query-design.md`

---

## File map

**Create:**
- `crates/kebab-app/src/bulk.rs` — `bulk_search_with_config` facade.
- `crates/kebab-cli/tests/wire_bulk_search.rs` — CLI integration tests.
- `crates/kebab-mcp/src/tools/bulk_search.rs` — MCP tool handler.
- `crates/kebab-mcp/tests/tools_call_bulk_search.rs` — MCP integration tests.
- `docs/wire-schema/v1/bulk_search_item.schema.json` — per-query result schema.
- `docs/wire-schema/v1/bulk_search_response.schema.json` — MCP envelope schema.

**Modify:**
- `crates/kebab-core/src/search.rs` — `BulkSearchItem` / `BulkSearchSummary` / `BulkSearchResponse` types.
- `crates/kebab-core/src/lib.rs` — re-export new types.
- `crates/kebab-app/src/lib.rs` — register `bulk` module + re-export facade.
- `crates/kebab-app/src/schema.rs` — add `bulk_search: bool` to `Capabilities` + snapshot value `true`.
- `crates/kebab-cli/src/main.rs` — add `--bulk` flag to `Cmd::Search` + dispatch branch + stdin reader + ndjson output.
- `crates/kebab-cli/src/wire.rs` — `wire_bulk_search_item` helper.
- `crates/kebab-mcp/src/tools/mod.rs` — `pub mod bulk_search;`.
- `crates/kebab-mcp/src/lib.rs` — `build_tools_vec` adds `bulk_search` entry; `call_tool` adds `"bulk_search" =>` arm.
- `crates/kebab-mcp/tests/tools_list.rs` — count 7 → 8.
- `README.md` — `kebab search --bulk` row + example line.
- `docs/SMOKE.md` — bulk walkthrough section.
- `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md` §4 — bulk subsection.
- `integrations/claude-code/kebab/SKILL.md` — `mcp__kebab__bulk_search` tool description.
- `tasks/p9/p9-fb-42-bulk-multi-query-rerank.md` — flip status, link design + plan, "rerank hint deferred" note.
- `tasks/INDEX.md` — fb-42 row ✅.

---

## Task 1: kebab-core domain types

**Files:**
- Modify: `crates/kebab-core/src/search.rs`
- Modify: `crates/kebab-core/src/lib.rs`

- [ ] **Step 1: Append failing tests to `mod tests`**

```rust
#[test]
fn bulk_search_summary_serde_roundtrip() {
    let s = BulkSearchSummary { total: 5, succeeded: 4, failed: 1 };
    let v = serde_json::to_value(s).unwrap();
    assert_eq!(v["total"], 5);
    assert_eq!(v["succeeded"], 4);
    assert_eq!(v["failed"], 1);
    let back: BulkSearchSummary = serde_json::from_value(v).unwrap();
    assert_eq!(back, s);
}

#[test]
fn bulk_search_summary_default_is_zeros() {
    let s = BulkSearchSummary::default();
    assert_eq!(s.total, 0);
    assert_eq!(s.succeeded, 0);
    assert_eq!(s.failed, 0);
}

#[test]
fn bulk_search_item_serde_response_variant() {
    let item = BulkSearchItem {
        query: serde_json::json!({"query": "rust"}),
        response: Some(serde_json::json!({"hits": []})),
        error: None,
    };
    let v = serde_json::to_value(&item).unwrap();
    assert!(v["response"].is_object());
    assert!(v["error"].is_null());
}

#[test]
fn bulk_search_item_serde_error_variant() {
    let item = BulkSearchItem {
        query: serde_json::json!({"query": "rust"}),
        response: None,
        error: Some(serde_json::json!({"code": "config_invalid", "message": "bad"})),
    };
    let v = serde_json::to_value(&item).unwrap();
    assert!(v["response"].is_null());
    assert_eq!(v["error"]["code"], "config_invalid");
}
```

- [ ] **Step 2: Run tests to verify compile errors**

```bash
cargo test -p kebab-core --lib bulk_search
```
Expected: errors — types undefined.

- [ ] **Step 3: Add domain types in `crates/kebab-core/src/search.rs`**

After existing `IndexBytes` (or wherever fb-37 fb-38 types live, near end of types):

```rust
/// p9-fb-42: per-query result in bulk search. `response` XOR `error` —
/// exactly one is `Some`. `query` is the input echo (raw JSON value)
/// so consumers can correlate input to output without index tracking.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BulkSearchItem {
    pub query: serde_json::Value,
    pub response: Option<serde_json::Value>,
    pub error: Option<serde_json::Value>,
}

/// p9-fb-42: bulk summary counts. Invariant: total == succeeded + failed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BulkSearchSummary {
    pub total: u32,
    pub succeeded: u32,
    pub failed: u32,
}

/// p9-fb-42: MCP-only envelope. CLI emits raw ndjson without envelope.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BulkSearchResponse {
    pub schema_version: String,
    pub results: Vec<BulkSearchItem>,
    pub summary: BulkSearchSummary,
}
```

`schema_version` is a runtime String (not const) so the constructor can stamp `"bulk_search_response.v1"` consistent with the existing `kebab-app::schema::SchemaV1` pattern.

- [ ] **Step 4: Re-export in `crates/kebab-core/src/lib.rs`**

Find the `search::` re-export block (the one fb-37 + fb-38 already extended with `SearchTrace`, `IndexBytes`, `MEDIA_KINDS`, `ScoreKind`). Add `BulkSearchItem`, `BulkSearchSummary`, `BulkSearchResponse` to the same export list.

```bash
grep -n "SearchTrace\|ScoreKind\|MEDIA_KINDS" crates/kebab-core/src/lib.rs
```

- [ ] **Step 5: Run tests + clippy**

```bash
cargo test -p kebab-core --lib
cargo clippy -p kebab-core --all-targets -- -D warnings
```
Expected: 4 new tests pass, existing tests untouched, clippy clean.

- [ ] **Step 6: Commit**

```bash
git add crates/kebab-core/src/search.rs crates/kebab-core/src/lib.rs
git commit -m "feat(core): BulkSearchItem / Summary / Response types (fb-42)"
```

---

## Task 2: kebab-app bulk facade

**Files:**
- Create: `crates/kebab-app/src/bulk.rs`
- Modify: `crates/kebab-app/src/lib.rs`

- [ ] **Step 1: Inspect existing facade pattern**

```bash
grep -n "search_with_opts_with_config\|App::open_with_config\|pub fn search_with" crates/kebab-app/src/lib.rs | head -10
```

Read `App::search_with_opts` body in `crates/kebab-app/src/app.rs` (~line 306) for the SearchOpts → SearchQuery → search flow.

- [ ] **Step 2: Create `crates/kebab-app/src/bulk.rs`**

```rust
//! p9-fb-42: bulk multi-query facade. Sequential for-loop reusing
//! one App instance so embedder cold-start + LRU cache amortize
//! across the N queries.

use anyhow::Context;
use kebab_core::{
    BulkSearchItem, BulkSearchSummary, ChunkId, DocumentId, Lang,
    SearchFilters, SearchMode, SearchOpts, SearchQuery, TrustLevel, WorkspacePath,
};
use serde_json::Value;

use crate::App;

/// Hard cap on items per bulk call. Documented in spec — agents that
/// hit this should batch-split.
pub const BULK_QUERIES_MAX: usize = 100;

/// p9-fb-42: bulk search facade. Returns `(items, summary)` always
/// — per-query failures embed `error.v1` JSON in the item rather
/// than aborting the bulk call. Returns `Err` only for input
/// validation failures (e.g. >100 queries).
#[doc(hidden)]
pub fn bulk_search_with_config(
    config: kebab_config::Config,
    raw_items: Vec<Value>,
) -> anyhow::Result<(Vec<BulkSearchItem>, BulkSearchSummary)> {
    if raw_items.len() > BULK_QUERIES_MAX {
        anyhow::bail!(
            "queries: max {} items, got {}",
            BULK_QUERIES_MAX,
            raw_items.len()
        );
    }

    let app = App::open_with_config(config).context("kebab-app: open for bulk_search")?;

    let mut results: Vec<BulkSearchItem> = Vec::with_capacity(raw_items.len());
    let mut succeeded: u32 = 0;
    let mut failed: u32 = 0;

    for raw in raw_items {
        let item = run_one(&app, raw);
        if item.error.is_some() {
            failed += 1;
        } else {
            succeeded += 1;
        }
        results.push(item);
    }

    let summary = BulkSearchSummary {
        total: succeeded + failed,
        succeeded,
        failed,
    };
    Ok((results, summary))
}

fn run_one(app: &App, raw: Value) -> BulkSearchItem {
    let echo = raw.clone();
    match parse_one(&raw) {
        Ok((query, opts)) => match app.search_with_opts(query, opts) {
            Ok(resp) => {
                let resp_v = serde_json::to_value(&resp).unwrap_or(Value::Null);
                BulkSearchItem {
                    query: echo,
                    response: Some(resp_v),
                    error: None,
                }
            }
            Err(e) => BulkSearchItem {
                query: echo,
                response: None,
                error: Some(error_v1_json("retrieval_error", &format!("{e:#}"), None)),
            },
        },
        Err(msg) => BulkSearchItem {
            query: echo,
            response: None,
            error: Some(error_v1_json("invalid_input", &msg, None)),
        },
    }
}

fn parse_one(raw: &Value) -> Result<(SearchQuery, SearchOpts), String> {
    let obj = raw.as_object().ok_or("expected JSON object")?;
    let text = obj
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or("missing required field: query")?
        .to_string();

    let mode = match obj.get("mode").and_then(|v| v.as_str()) {
        None => SearchMode::Hybrid,
        Some("hybrid") => SearchMode::Hybrid,
        Some("lexical") => SearchMode::Lexical,
        Some("vector") => SearchMode::Vector,
        Some(other) => return Err(format!("invalid mode: {other:?}")),
    };

    let k = obj
        .get("k")
        .and_then(|v| v.as_u64())
        .map(|n| n as usize)
        .unwrap_or(0); // 0 → use config default in app

    let trust_min = match obj.get("trust_min").and_then(|v| v.as_str()) {
        None => None,
        Some("primary") => Some(TrustLevel::Primary),
        Some("secondary") => Some(TrustLevel::Secondary),
        Some("generated") => Some(TrustLevel::Generated),
        Some(other) => return Err(format!("invalid trust_min: {other:?}")),
    };

    let ingested_after = match obj.get("ingested_after").and_then(|v| v.as_str()) {
        None => None,
        Some(s) => Some(
            time::OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339)
                .map_err(|e| format!("invalid ingested_after RFC3339 {s:?}: {e}"))?,
        ),
    };

    let media: Vec<String> = obj
        .get("media")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(normalize_media_alias))
                .collect()
        })
        .unwrap_or_default();

    let tags_any: Vec<String> = obj
        .get("tag")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|x| x.as_str().map(String::from)).collect())
        .unwrap_or_default();

    let lang = obj
        .get("lang")
        .and_then(|v| v.as_str())
        .map(|s| Lang(s.to_string()));

    let path_glob = obj
        .get("path_glob")
        .and_then(|v| v.as_str())
        .map(String::from);

    let doc_id = obj
        .get("doc_id")
        .and_then(|v| v.as_str())
        .map(|s| DocumentId(s.to_string()));

    let filters = SearchFilters {
        tags_any,
        lang,
        path_glob,
        trust_min,
        media,
        ingested_after,
        doc_id,
    };

    let opts = SearchOpts {
        max_tokens: obj.get("max_tokens").and_then(|v| v.as_u64()).map(|n| n as usize),
        snippet_chars: obj
            .get("snippet_chars")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize),
        cursor: obj.get("cursor").and_then(|v| v.as_str()).map(String::from),
        trace: obj.get("trace").and_then(|v| v.as_bool()).unwrap_or(false),
    };

    Ok((SearchQuery { text, mode, k, filters }, opts))
}

fn normalize_media_alias(s: &str) -> String {
    match s.to_ascii_lowercase().as_str() {
        "md" => "markdown".to_string(),
        other => other.to_string(),
    }
}

fn error_v1_json(code: &str, message: &str, hint: Option<&str>) -> Value {
    serde_json::json!({
        "schema_version": "error.v1",
        "code": code,
        "message": message,
        "hint": hint,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_temp() -> kebab_config::Config {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = kebab_config::Config::defaults();
        cfg.storage.data_dir = dir.path().to_string_lossy().into_owned();
        // Bring up migrations so SqliteStore::open_existing succeeds inside App::open.
        let store = kebab_store_sqlite::SqliteStore::open(&cfg).unwrap();
        store.run_migrations().unwrap();
        drop(store);
        // Leak the tempdir into a static — tests are short-lived; not worth threading.
        std::mem::forget(dir);
        cfg
    }

    #[test]
    fn empty_input_returns_empty_summary() {
        let cfg = open_temp();
        let (items, summary) = bulk_search_with_config(cfg, vec![]).unwrap();
        assert!(items.is_empty());
        assert_eq!(summary.total, 0);
        assert_eq!(summary.succeeded, 0);
        assert_eq!(summary.failed, 0);
    }

    #[test]
    fn over_cap_returns_err() {
        let cfg = open_temp();
        let raw: Vec<Value> = (0..101)
            .map(|_| serde_json::json!({"query": "x"}))
            .collect();
        let err = bulk_search_with_config(cfg, raw).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("max 100"));
    }

    #[test]
    fn invalid_item_emits_error_keeps_total_count() {
        let cfg = open_temp();
        let raw = vec![
            serde_json::json!({"query": "ok"}),
            serde_json::json!({"mode": "lexical"}),  // missing required `query`
        ];
        let (items, summary) = bulk_search_with_config(cfg, raw).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(summary.total, 2);
        // First item: lexical mode against empty corpus succeeds with empty hits.
        assert!(items[0].error.is_none());
        // Second item: missing required field.
        assert!(items[1].error.is_some());
        assert_eq!(items[1].error.as_ref().unwrap()["code"], "invalid_input");
    }
}
```

- [ ] **Step 3: Register module + re-export facade**

In `crates/kebab-app/src/lib.rs`, find the existing `mod app;` / `mod fetch;` etc. block. Add:

```rust
mod bulk;
```

In the `pub use` block (or near `search_with_opts_with_config` re-export), add:

```rust
#[doc(hidden)]
pub use bulk::{bulk_search_with_config, BULK_QUERIES_MAX};
```

- [ ] **Step 4: Run tests + clippy**

```bash
cargo test -p kebab-app --lib bulk
cargo clippy -p kebab-app --all-targets -- -D warnings
```
Expected: 3 new tests pass; clippy clean.

- [ ] **Step 5: Commit**

```bash
git add crates/kebab-app/src/bulk.rs crates/kebab-app/src/lib.rs
git commit -m "feat(app): bulk_search_with_config facade (fb-42)"
```

---

## Task 3: CLI --bulk flag + stdin ndjson + output stream

**Files:**
- Modify: `crates/kebab-cli/src/main.rs`
- Modify: `crates/kebab-cli/src/wire.rs`

- [ ] **Step 1: Add `wire_bulk_search_item` helper to `crates/kebab-cli/src/wire.rs`**

Append (after `wire_search_response`):

```rust
/// p9-fb-42: tag a `BulkSearchItem` (already serialized as a Value)
/// as `bulk_search_item.v1`. The inner `query` / `response` / `error`
/// fields stay verbatim — only the envelope gets the schema_version stamp.
pub fn wire_bulk_search_item(item: &kebab_core::BulkSearchItem) -> Value {
    let mut v = serde_json::to_value(item).expect("BulkSearchItem serializes");
    if let Value::Object(ref mut map) = v {
        map.insert(
            "schema_version".to_string(),
            Value::String("bulk_search_item.v1".to_string()),
        );
    }
    v
}
```

- [ ] **Step 2: Add `--bulk` flag to `Cmd::Search` in `crates/kebab-cli/src/main.rs`**

Find `Cmd::Search { ... }` field block (around line 95-180 — fb-37 added `trace`, fb-38 added `score_kind` though that's not a flag). Append after the last field:

```rust
        /// p9-fb-42: bulk multi-query mode. Reads ndjson from stdin —
        /// one JSON object per line, each item shape mirrors the
        /// single-query input. Output is per-query ndjson on stdout
        /// (one `bulk_search_item.v1` per line) plus a summary line on
        /// stderr. Single-query flags (`--mode`, `--k`, `--tag`, etc.)
        /// are ignored when `--bulk` is set; pass them per-item in the
        /// stdin JSON instead. Caps at 100 queries per call.
        #[arg(long)]
        bulk: bool,
```

- [ ] **Step 3: Wire bulk dispatch in the `Cmd::Search` arm**

Find the `Cmd::Search { ... } => { ... }` arm (~line 664). Add `bulk,` to the destructure pattern. Near the top of the arm body (before single-query SearchOpts construction), branch on `*bulk`:

```rust
            // p9-fb-42: bulk mode — stdin ndjson → bulk_search_with_config
            // → stdout ndjson per query + stderr summary. Single-query
            // flags are ignored (each item supplies its own).
            if *bulk {
                use std::io::{BufRead, Write};

                let cfg = kebab_config::Config::load(cli.config.as_deref())?;

                let stdin = std::io::stdin();
                let stdin_locked = stdin.lock();
                let mut raw_items: Vec<serde_json::Value> = Vec::new();
                for (lineno, line) in stdin_locked.lines().enumerate() {
                    let line = line?;
                    if line.trim().is_empty() {
                        continue;
                    }
                    let v: serde_json::Value =
                        serde_json::from_str(&line).map_err(|e| {
                            anyhow::Error::new(
                                kebab_app::error_wire::StructuredError(kebab_app::ErrorV1 {
                                    schema_version: kebab_app::ERROR_V1_ID.to_string(),
                                    code: "config_invalid".to_string(),
                                    message: format!(
                                        "stdin ndjson line {} parse error: {e}",
                                        lineno + 1
                                    ),
                                    details: serde_json::Value::Null,
                                    hint: Some(
                                        "each line must be a JSON object with at least `query`"
                                            .to_string(),
                                    ),
                                }),
                            )
                        })?;
                    raw_items.push(v);
                }

                let (items, summary) =
                    kebab_app::bulk_search_with_config(cfg, raw_items)?;

                if cli.json {
                    let mut stdout = std::io::stdout().lock();
                    for item in &items {
                        let v = wire::wire_bulk_search_item(item);
                        writeln!(stdout, "{}", serde_json::to_string(&v)?)?;
                    }
                    eprintln!(
                        "bulk_summary: total={} succeeded={} failed={}",
                        summary.total, summary.succeeded, summary.failed,
                    );
                } else {
                    let mut stdout = std::io::stdout().lock();
                    for (idx, item) in items.iter().enumerate() {
                        writeln!(stdout, "# Query {}: {}", idx + 1, item.query)?;
                        if let Some(err) = &item.error {
                            writeln!(stdout, "error: {}", err)?;
                        } else if let Some(resp) = &item.response {
                            writeln!(stdout, "{}", serde_json::to_string_pretty(resp)?)?;
                        }
                        writeln!(stdout)?;
                    }
                    eprintln!(
                        "bulk_summary: total={} succeeded={} failed={}",
                        summary.total, summary.succeeded, summary.failed,
                    );
                }
                return Ok(());
            }
```

The `kebab_app::ErrorV1` / `error_wire::StructuredError` / `ERROR_V1_ID` types should already be in scope from prior fb-27 / fb-34 wiring. Verify by reading the existing fb-36 error path in the same file (search for `StructuredError`).

- [ ] **Step 4: Run tests + clippy**

```bash
cargo build -p kebab-cli
cargo clippy -p kebab-cli --all-targets -- -D warnings
```
Expected: clean compile + clippy clean. (No new tests yet — those land in Task 4.)

- [ ] **Step 5: Commit**

```bash
git add crates/kebab-cli/src/main.rs crates/kebab-cli/src/wire.rs
git commit -m "feat(cli): kebab search --bulk flag + stdin ndjson + output stream (fb-42)"
```

---

## Task 4: CLI integration tests

**Files:**
- Create: `crates/kebab-cli/tests/wire_bulk_search.rs`

- [ ] **Step 1: Inspect common fixture pattern**

```bash
head -40 crates/kebab-cli/tests/common/mod.rs
```

`common::write_config(dir, threshold_days)` returns `(cfg, workspace, data)`. `common::ingest(&cfg, &workspace)` runs `kebab ingest`.

- [ ] **Step 2: Create test file with 5 integration tests**

```rust
//! p9-fb-42: integration tests for `kebab search --bulk`.

mod common;

use serde_json::Value;
use std::fs;
use std::io::Write;
use std::process::{Command, Stdio};

fn cargo_bin() -> &'static str {
    env!("CARGO_BIN_EXE_kebab")
}

fn run_bulk_with_stdin(cfg: &std::path::Path, stdin_body: &str, json: bool) -> std::process::Output {
    let mut cmd = Command::new(cargo_bin());
    cmd.arg("--config").arg(cfg).arg("search").arg("--bulk");
    if json {
        cmd.arg("--json");
    }
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn kebab");
    {
        let mut sin = child.stdin.take().expect("stdin");
        sin.write_all(stdin_body.as_bytes()).expect("write stdin");
    }
    child.wait_with_output().expect("wait")
}

fn seed_workspace(workspace: &std::path::Path) {
    fs::write(workspace.join("a.md"), "# Alpha\n\nrust async hello").unwrap();
    fs::write(workspace.join("b.md"), "# Bravo\n\nbread and kebab").unwrap();
}

#[test]
fn two_query_bulk_emits_per_query_ndjson() {
    let dir = tempfile::tempdir().unwrap();
    let (cfg, workspace, _data) = common::write_config(dir.path(), 0);
    seed_workspace(&workspace);
    common::ingest(&cfg, &workspace);

    let out = run_bulk_with_stdin(
        &cfg,
        "{\"query\":\"rust\",\"mode\":\"lexical\"}\n{\"query\":\"kebab\",\"mode\":\"lexical\"}\n",
        true,
    );
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(lines.len(), 2, "expected 2 ndjson lines, got {lines:?}");
    for line in &lines {
        let v: Value = serde_json::from_str(line).expect("valid JSON line");
        assert_eq!(v["schema_version"], "bulk_search_item.v1");
        assert!(v["response"].is_object());
        assert!(v["error"].is_null());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("bulk_summary: total=2 succeeded=2 failed=0"),
        "stderr summary missing: {stderr}"
    );
}

#[test]
fn empty_stdin_returns_empty_results_with_zero_summary() {
    let dir = tempfile::tempdir().unwrap();
    let (cfg, workspace, _data) = common::write_config(dir.path(), 0);
    seed_workspace(&workspace);
    common::ingest(&cfg, &workspace);

    let out = run_bulk_with_stdin(&cfg, "", true);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.trim().is_empty(), "expected empty stdout, got: {stdout}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("bulk_summary: total=0 succeeded=0 failed=0"));
}

#[test]
fn malformed_ndjson_line_emits_config_invalid_exit_2() {
    let dir = tempfile::tempdir().unwrap();
    let (cfg, workspace, _data) = common::write_config(dir.path(), 0);
    seed_workspace(&workspace);
    common::ingest(&cfg, &workspace);

    let out = run_bulk_with_stdin(&cfg, "not json\n", true);
    assert_eq!(out.status.code(), Some(2), "expected exit 2");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("config_invalid") || stderr.contains("parse error"),
        "expected config_invalid in stderr: {stderr}");
}

#[test]
fn over_cap_input_emits_error_exit_2() {
    let dir = tempfile::tempdir().unwrap();
    let (cfg, workspace, _data) = common::write_config(dir.path(), 0);
    seed_workspace(&workspace);
    common::ingest(&cfg, &workspace);

    let body: String = (0..101)
        .map(|_| "{\"query\":\"x\",\"mode\":\"lexical\"}\n")
        .collect();
    let out = run_bulk_with_stdin(&cfg, &body, true);
    // bulk_search_with_config returns Err(anyhow) — surfaces as exit 1 (anyhow chain)
    // or 2 if classified as config_invalid by error_wire. Accept either,
    // but message must mention `max 100`.
    assert!(out.status.code().is_some());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("max 100"), "expected 'max 100' in stderr: {stderr}");
}

#[test]
fn invalid_item_field_emits_per_item_error_continues() {
    let dir = tempfile::tempdir().unwrap();
    let (cfg, workspace, _data) = common::write_config(dir.path(), 0);
    seed_workspace(&workspace);
    common::ingest(&cfg, &workspace);

    let out = run_bulk_with_stdin(
        &cfg,
        "{\"query\":\"rust\",\"mode\":\"lexical\"}\n{\"query\":\"x\",\"mode\":\"bogus\"}\n",
        true,
    );
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(lines.len(), 2);
    let v0: Value = serde_json::from_str(lines[0]).unwrap();
    let v1: Value = serde_json::from_str(lines[1]).unwrap();
    assert!(v0["error"].is_null());
    assert!(v1["error"].is_object());
    assert_eq!(v1["error"]["code"], "invalid_input");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("succeeded=1 failed=1"));
}
```

- [ ] **Step 3: Run tests**

```bash
cargo test -p kebab-cli --test wire_bulk_search
```
Expected: 5 tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/kebab-cli/tests/wire_bulk_search.rs
git commit -m "test(cli): integration tests for kebab search --bulk (fb-42)"
```

---

## Task 5: MCP bulk_search tool

**Files:**
- Create: `crates/kebab-mcp/src/tools/bulk_search.rs`
- Modify: `crates/kebab-mcp/src/tools/mod.rs`
- Modify: `crates/kebab-mcp/src/lib.rs`

- [ ] **Step 1: Create `crates/kebab-mcp/src/tools/bulk_search.rs`**

```rust
//! `bulk_search` tool — wraps `kebab_app::bulk_search_with_config`.
//! Input: `{ queries: [<SearchInput shape>, ...] }`.
//! Output: `bulk_search_response.v1` envelope (results + summary).

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{to_tool_error, to_tool_success};
use crate::state::KebabAppState;

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct BulkSearchInput {
    /// Per-query inputs. Each item mirrors the single-query `search`
    /// tool's input shape — `query` is required, all other fields are
    /// optional and default to single-search defaults. Capped at 100
    /// items; exceeding returns an `invalid_input` tool error without
    /// running any query.
    pub queries: Vec<serde_json::Value>,
}

pub fn handle(state: &KebabAppState, input: BulkSearchInput) -> CallToolResult {
    let cfg_clone = (*state.config).clone();
    match kebab_app::bulk_search_with_config(cfg_clone, input.queries) {
        Ok((items, summary)) => {
            let tagged_items: Vec<serde_json::Value> = items
                .iter()
                .map(|it| {
                    let mut v = serde_json::to_value(it).unwrap_or(serde_json::Value::Null);
                    if let serde_json::Value::Object(ref mut map) = v {
                        map.insert(
                            "schema_version".to_string(),
                            serde_json::Value::String("bulk_search_item.v1".to_string()),
                        );
                    }
                    v
                })
                .collect();
            let envelope = serde_json::json!({
                "schema_version": "bulk_search_response.v1",
                "results": tagged_items,
                "summary": {
                    "total": summary.total,
                    "succeeded": summary.succeeded,
                    "failed": summary.failed,
                },
            });
            match serde_json::to_string(&envelope) {
                Ok(json) => to_tool_success(json),
                Err(e) => to_tool_error(&anyhow::anyhow!(e)),
            }
        }
        Err(e) => {
            // Cap-exceed and other validation failures surface here.
            // Map to invalid_input via to_tool_error chain.
            to_tool_error(&e)
        }
    }
}
```

- [ ] **Step 2: Register module**

In `crates/kebab-mcp/src/tools/mod.rs`, add:

```rust
pub mod bulk_search;
```

- [ ] **Step 3: Register tool in `build_tools_vec`**

In `crates/kebab-mcp/src/lib.rs`, find `build_tools_vec` (~line 33). Add a new `Tool::new` entry inside the `vec![...]` (place after `fetch`):

```rust
        Tool::new(
            "bulk_search",
            "Bulk multi-query search — N queries per call (cap 100). Each query mirrors the `search` input shape; returns `bulk_search_response.v1` with per-query results + summary. Sequential execution reuses one App instance so cache / embedder cold-start cost amortizes.",
            schema_for_type::<tools::bulk_search::BulkSearchInput>(),
        ),
```

- [ ] **Step 4: Add dispatch arm in `call_tool`**

Find the `match request.name.as_ref()` block (~line 129). Add new arm after `fetch`:

```rust
            "bulk_search" => {
                let args = request.arguments.unwrap_or_default();
                self.spawn_tool(args, |state, input| {
                    tools::bulk_search::handle(&state, input)
                })
                .await
            }
```

- [ ] **Step 5: Bump tool count assertion**

Modify `crates/kebab-mcp/tests/tools_list.rs`. Find `assert_eq!(tools.len(), 7, ...)` (line ~10). Change to:

```rust
    assert_eq!(tools.len(), 8, "expected exactly 8 tools, got {}", tools.len());
```

If the file also has assertions about specific tool names (e.g. a Vec containing `["schema", "doctor", ...]`), add `"bulk_search"` to that list.

- [ ] **Step 6: Run tests + clippy**

```bash
cargo test -p kebab-mcp --test tools_list
cargo clippy -p kebab-mcp --all-targets -- -D warnings
```
Expected: tools_list count test passes; clippy clean.

- [ ] **Step 7: Commit**

```bash
git add crates/kebab-mcp/src/tools/bulk_search.rs crates/kebab-mcp/src/tools/mod.rs crates/kebab-mcp/src/lib.rs crates/kebab-mcp/tests/tools_list.rs
git commit -m "feat(mcp): kebab__bulk_search tool (fb-42)"
```

---

## Task 6: MCP integration tests

**Files:**
- Create: `crates/kebab-mcp/tests/tools_call_bulk_search.rs`

- [ ] **Step 1: Inspect existing MCP integration test pattern**

```bash
head -80 crates/kebab-mcp/tests/tools_call_search.rs
```

Mirror `minimal_config + setup` pattern.

- [ ] **Step 2: Create `crates/kebab-mcp/tests/tools_call_bulk_search.rs`**

```rust
//! p9-fb-42: integration tests for `mcp__kebab__bulk_search`.

use std::fs;

use kebab_config::Config;
use kebab_core::SourceScope;
use kebab_mcp::{KebabAppState, KebabHandler};
use rmcp::model::RawContent;
use serde_json::json;

fn minimal_config(data_dir: &std::path::Path, workspace_root: &std::path::Path) -> Config {
    let mut cfg = Config::defaults();
    cfg.storage.data_dir = data_dir.to_string_lossy().into_owned();
    cfg.storage.model_dir = data_dir.join("models").to_string_lossy().into_owned();
    cfg.workspace.root = workspace_root.to_string_lossy().into_owned();
    cfg.workspace.exclude.clear();
    cfg.models.embedding.provider = "none".to_string();
    cfg.models.embedding.dimensions = 0;
    cfg
}

fn setup() -> (tempfile::TempDir, KebabHandler) {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let workspace_root = dir.path().join("notes");
    fs::create_dir_all(&data_dir).unwrap();
    fs::create_dir_all(&workspace_root).unwrap();
    let config = minimal_config(&data_dir, &workspace_root);
    fs::write(
        workspace_root.join("a.md"),
        "# Alpha\n\nThis document mentions kebab and bread.",
    )
    .unwrap();
    let scope = SourceScope { root: workspace_root.clone(), include: vec![], exclude: vec![] };
    let _ = kebab_app::ingest_with_config(config.clone(), scope, false).unwrap();
    let state = KebabAppState::new(config, None);
    let handler = KebabHandler::new(state);
    (dir, handler)
}

fn extract_json(result: &rmcp::model::CallToolResult) -> serde_json::Value {
    assert!(!result.is_error.unwrap_or(false), "expected isError=false, got {result:?}");
    let content = result.content.first().expect("at least one content item");
    let text = match &content.raw {
        RawContent::Text(t) => &t.text,
        other => panic!("expected Text content, got {other:?}"),
    };
    serde_json::from_str(text).expect("valid JSON")
}

#[tokio::test]
async fn bulk_search_two_queries_returns_envelope() {
    let (_dir, handler) = setup();
    let input = kebab_mcp::tools::bulk_search::BulkSearchInput {
        queries: vec![
            json!({"query": "kebab", "mode": "lexical", "k": 5}),
            json!({"query": "bread", "mode": "lexical", "k": 5}),
        ],
    };
    let result = kebab_mcp::tools::bulk_search::handle(handler.state(), input);
    let v = extract_json(&result);
    assert_eq!(v["schema_version"], "bulk_search_response.v1");
    let results = v["results"].as_array().expect("results array");
    assert_eq!(results.len(), 2);
    for r in results {
        assert_eq!(r["schema_version"], "bulk_search_item.v1");
        assert!(r["response"].is_object());
        assert!(r["error"].is_null());
    }
    assert_eq!(v["summary"]["total"], 2);
    assert_eq!(v["summary"]["succeeded"], 2);
    assert_eq!(v["summary"]["failed"], 0);
}

#[tokio::test]
async fn bulk_search_empty_queries_returns_empty_envelope() {
    let (_dir, handler) = setup();
    let input = kebab_mcp::tools::bulk_search::BulkSearchInput { queries: vec![] };
    let result = kebab_mcp::tools::bulk_search::handle(handler.state(), input);
    let v = extract_json(&result);
    assert_eq!(v["schema_version"], "bulk_search_response.v1");
    assert_eq!(v["results"].as_array().unwrap().len(), 0);
    assert_eq!(v["summary"]["total"], 0);
}

#[tokio::test]
async fn bulk_search_invalid_item_field_continues_with_per_item_error() {
    let (_dir, handler) = setup();
    let input = kebab_mcp::tools::bulk_search::BulkSearchInput {
        queries: vec![
            json!({"query": "kebab", "mode": "lexical"}),
            json!({"query": "bread", "mode": "bogus"}),  // invalid mode
        ],
    };
    let result = kebab_mcp::tools::bulk_search::handle(handler.state(), input);
    let v = extract_json(&result);
    let results = v["results"].as_array().unwrap();
    assert_eq!(results.len(), 2);
    assert!(results[0]["error"].is_null());
    assert!(results[1]["error"].is_object());
    assert_eq!(results[1]["error"]["code"], "invalid_input");
    assert_eq!(v["summary"]["succeeded"], 1);
    assert_eq!(v["summary"]["failed"], 1);
}

#[tokio::test]
async fn bulk_search_over_cap_returns_tool_error() {
    let (_dir, handler) = setup();
    let queries: Vec<serde_json::Value> = (0..101)
        .map(|_| json!({"query": "x", "mode": "lexical"}))
        .collect();
    let input = kebab_mcp::tools::bulk_search::BulkSearchInput { queries };
    let result = kebab_mcp::tools::bulk_search::handle(handler.state(), input);
    assert!(result.is_error.unwrap_or(false), "expected isError=true");
    let content = result.content.first().expect("error content");
    let text = match &content.raw {
        RawContent::Text(t) => &t.text,
        other => panic!("expected Text content, got {other:?}"),
    };
    assert!(text.contains("max 100"), "expected 'max 100' in error: {text}");
}
```

- [ ] **Step 3: Run tests**

```bash
cargo test -p kebab-mcp --test tools_call_bulk_search
```
Expected: 4 tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/kebab-mcp/tests/tools_call_bulk_search.rs
git commit -m "test(mcp): integration tests for bulk_search tool (fb-42)"
```

---

## Task 7: Capability flag bulk_search

**Files:**
- Modify: `crates/kebab-app/src/schema.rs`

- [ ] **Step 1: Add `bulk_search` field to `Capabilities` struct + snapshot value**

In `crates/kebab-app/src/schema.rs`, find `pub struct Capabilities { ... }` (~line 24). Append field:

```rust
    pub bulk_search: bool,
```

Find `fn capabilities_snapshot() -> Capabilities { ... }` (~line 114). Add field initializer inside:

```rust
        bulk_search: true,
```

- [ ] **Step 2: Update existing schema integration test (if any asserts capability count)**

```bash
grep -rn "capabilities\.\|Capabilities {" crates/kebab-app/ crates/kebab-cli/tests/ | head -10
```

If any test asserts `capabilities.json_mode` etc., extend with `bulk_search` assertion. If a test deserializes schema JSON and checks capability count, bump.

If `crates/kebab-cli/tests/cli_schema.rs` exists with capability assertions, update.

- [ ] **Step 3: Run tests**

```bash
cargo test -p kebab-app -p kebab-cli
cargo clippy -p kebab-app --all-targets -- -D warnings
```
Expected: all pass; clippy clean.

- [ ] **Step 4: Commit**

```bash
git add crates/kebab-app/src/schema.rs crates/kebab-cli/tests/
git commit -m "feat(schema): bulk_search capability flag (fb-42)"
```

---

## Task 8: Wire schema docs + README + SMOKE + design + SKILL + INDEX + status flip

**Files:**
- Create: `docs/wire-schema/v1/bulk_search_item.schema.json`
- Create: `docs/wire-schema/v1/bulk_search_response.schema.json`
- Modify: `crates/kebab-app/src/schema.rs` — add new schemas to `WIRE_SCHEMAS` const list.
- Modify: `README.md`
- Modify: `docs/SMOKE.md`
- Modify: `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md`
- Modify: `integrations/claude-code/kebab/SKILL.md`
- Modify: `tasks/p9/p9-fb-42-bulk-multi-query-rerank.md`
- Modify: `tasks/INDEX.md`

- [ ] **Step 1: Create `docs/wire-schema/v1/bulk_search_item.schema.json`**

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://kb.local/wire/v1/bulk_search_item.schema.json",
  "title": "BulkSearchItem v1",
  "description": "p9-fb-42: per-query result inside a bulk_search response. `response` XOR `error` — exactly one is non-null. `query` is the input echo so consumers can correlate without index tracking.",
  "type": "object",
  "required": ["schema_version", "query", "response", "error"],
  "properties": {
    "schema_version": { "const": "bulk_search_item.v1" },
    "query":   { "type": "object", "description": "Input echo (verbatim JSON object)." },
    "response":{
      "type": ["object", "null"],
      "description": "search_response.v1 payload on success; null when error is non-null."
    },
    "error":   {
      "type": ["object", "null"],
      "description": "error.v1 payload when this query failed; null on success."
    }
  }
}
```

- [ ] **Step 2: Create `docs/wire-schema/v1/bulk_search_response.schema.json`**

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://kb.local/wire/v1/bulk_search_response.schema.json",
  "title": "BulkSearchResponse v1",
  "description": "p9-fb-42: MCP envelope for bulk_search. CLI emits raw `bulk_search_item.v1` ndjson without this envelope (summary on stderr).",
  "type": "object",
  "required": ["schema_version", "results", "summary"],
  "properties": {
    "schema_version": { "const": "bulk_search_response.v1" },
    "results": {
      "type": "array",
      "items": { "type": "object", "description": "bulk_search_item.v1" }
    },
    "summary": {
      "type": "object",
      "required": ["total", "succeeded", "failed"],
      "properties": {
        "total":     { "type": "integer", "minimum": 0 },
        "succeeded": { "type": "integer", "minimum": 0 },
        "failed":    { "type": "integer", "minimum": 0 }
      }
    }
  }
}
```

- [ ] **Step 3: Register new schemas in `WIRE_SCHEMAS` const**

In `crates/kebab-app/src/schema.rs`, find `const WIRE_SCHEMAS: &[&str] = &[ ... ]` (~line 65). Add:

```rust
    "bulk_search_item.v1",
    "bulk_search_response.v1",
```

- [ ] **Step 4: Update `README.md`**

Find the search command row in the command table or flag list. Add a `--bulk` mention next to other search flags. If the README has a "검색" section, add a paragraph:

```markdown
- `--bulk` (fb-42) — stdin ndjson 으로 N query 한 번에 실행. `--json` 면 stdout per-query ndjson + stderr summary. Cap 100. agent 가 query decomposition 후 sub-query 일괄 실행 시 single round-trip.
```

Also add the `kebab__bulk_search` MCP tool to the "MCP 도구" list if such a list exists.

- [ ] **Step 5: Add SMOKE walkthrough**

Append a new section to `docs/SMOKE.md` after the fb-37/fb-38 walkthroughs:

```markdown
### Bulk multi-query (fb-42)

Stdin ndjson으로 N query 한 번에:

\`\`\`bash
printf '{"query":"rust","mode":"lexical"}\n{"query":"async","mode":"lexical"}\n' \
  | kebab --config /tmp/kebab-smoke/config.toml search --bulk --json
\`\`\`

stdout: per-query ndjson (`bulk_search_item.v1`). stderr: `bulk_summary: total=2 succeeded=2 failed=0`.

MCP tool 동등:

\`\`\`json
{"name":"kebab__bulk_search","arguments":{"queries":[{"query":"rust"},{"query":"async"}]}}
\`\`\`
```

- [ ] **Step 6: Update design §4 search**

Find §4 search in `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md`. Append a "Bulk multi-query (fb-42)" subsection with the input shape + output shape (per-query item + envelope) + cap 100 + per-query error policy.

```bash
grep -n "^## §4\|^### §4\|search.*bulk\|^## 4 검색" docs/superpowers/specs/2026-04-27-kebab-final-form-design.md | head -5
```

Insert after the existing search content. Mirror the fb-37 trace section's brevity.

- [ ] **Step 7: Update SKILL.md**

Find the `mcp__kebab__search` section in `integrations/claude-code/kebab/SKILL.md`. After it, add a sibling `mcp__kebab__bulk_search` section:

```markdown
### `mcp__kebab__bulk_search`

N개 query 한 번에 — agent loop 효율 개선. 각 query 는 `mcp__kebab__search` 와 동일 input shape (query 필수, 나머지 optional). Cap 100.

Input:
\`\`\`json
{"queries": [{"query": "..."}, {"query": "...", "mode": "lexical"}, ...]}
\`\`\`

Output: `bulk_search_response.v1` envelope — `results: [bulk_search_item.v1]` (각 item = `{query, response | null, error | null}`) + `summary: {total, succeeded, failed}`. Per-query 실패는 item 의 error 에 격리, 다른 query 계속 진행.
```

- [ ] **Step 8: Flip task spec status**

Edit `tasks/p9/p9-fb-42-bulk-multi-query-rerank.md`. Change frontmatter `status: open` → `status: completed`. Replace the skeleton banner with:

```markdown
> ✅ **Bulk multi-query 부분 구현 완료.** 본 spec 의 rerank hint lever 는 별도 task 로 분리 (fb-39 cross-encoder 설계 후).
>
> - Design: [`docs/superpowers/specs/2026-05-10-p9-fb-42-bulk-multi-query-design.md`](../../docs/superpowers/specs/2026-05-10-p9-fb-42-bulk-multi-query-design.md)
> - Plan: [`docs/superpowers/plans/2026-05-10-p9-fb-42-bulk-multi-query.md`](../../docs/superpowers/plans/2026-05-10-p9-fb-42-bulk-multi-query.md)
```

- [ ] **Step 9: Flip INDEX row**

In `tasks/INDEX.md`, find the fb-42 row. Mirror the format `✅ 머지 (2026-05-10) — bulk only, rerank hint deferred` (preserve the deferral note since fb-42's stub had two levers).

- [ ] **Step 10: Run full workspace tests + clippy**

```bash
cargo test --workspace --no-fail-fast -j 1 2>&1 | tail -10
cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -5
```
`-j 1` REQUIRED for workspace test.

Expected: all green.

- [ ] **Step 11: Commit**

```bash
git add docs/ README.md crates/kebab-app/src/schema.rs tasks/p9/p9-fb-42-bulk-multi-query-rerank.md tasks/INDEX.md integrations/claude-code/kebab/SKILL.md
git commit -m "docs(fb-42): wire schema + README + SMOKE + design + SKILL + INDEX"
```

---

## Final verification checklist

- [ ] `cargo test --workspace --no-fail-fast -j 1` green
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` clean
- [ ] Manual smoke against `/tmp/kebab-smoke`:
  - [ ] `printf '{"query":"a"}\n{"query":"b"}\n' | kebab search --bulk --json` returns 2 ndjson lines + stderr summary
  - [ ] `kebab schema --json | jq .capabilities.bulk_search` returns `true`
  - [ ] `kebab schema --json | jq .wire.schemas` includes `"bulk_search_item.v1"` and `"bulk_search_response.v1"`
- [ ] README, SMOKE, design §4, SKILL, INDEX, spec status all updated
