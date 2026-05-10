# p9-fb-36 — Search Filter Args Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Expose 7 filter flags on `kebab search` (`--tag`, `--lang`, `--path-glob`, `--trust-min` for existing `SearchFilters` fields plus `--media`, `--ingested-after`, `--doc-id` as new fields). Filter layer = SQLite WHERE for lexical, over-fetch + post-filter for vector. AND combinator. Wire-shape input-only. MCP `kebab__search` SearchInput gains the 7 fields.

**Architecture:** Domain `SearchFilters` gets 3 new optional fields. Lexical retriever's SQL builder extends WHERE clause; vector retriever's `filter_chunks` helper mirrors. CLI dispatch translates clap flags into `SearchFilters`, parsing `--ingested-after` as RFC3339 (config_invalid on failure). MCP `SearchInput` gains 7 optional fields with the same translation. `media_type` JSON column has two shapes (text for unit variants, object for tuple variants) — use `CASE WHEN json_type(media_type) = 'text' THEN json_extract(media_type, '$') ELSE (SELECT key FROM json_each(media_type) LIMIT 1) END` to extract a unified `kind` string.

**Tech Stack:** Rust 2024, clap (value_enum, value_delimiter), serde, time crate (RFC3339), rusqlite (json_extract / json_each / json_type), no new deps.

**Spec:** `docs/superpowers/specs/2026-05-10-p9-fb-36-search-filters-design.md`

---

## File Structure

| File | Responsibility | Action |
|------|----------------|--------|
| `crates/kebab-core/src/search.rs` | `SearchFilters` 3 new fields + `MEDIA_KINDS` const | modify |
| `crates/kebab-search/src/lexical.rs` | SQL builder WHERE clause extension (media JOIN assets, ingested_after, doc_id) | modify |
| `crates/kebab-search/src/vector.rs` | `filter_chunks` helper extension to match | modify |
| `crates/kebab-cli/src/main.rs` | `Cmd::Search` 7 new flags + dispatch + RFC3339 parsing + `TrustLevelFlag` enum | modify |
| `crates/kebab-mcp/src/tools/search.rs` | `SearchInput` 7 optional fields + dispatch + invalid_input on bad RFC3339 | modify |
| `crates/kebab-search/tests/lexical.rs` | filter unit tests (media / ingested_after / doc_id / AND combo) | modify |
| `crates/kebab-search/tests/hybrid.rs` | vector filter mirror tests | modify |
| `crates/kebab-cli/tests/wire_search_filters.rs` | NEW — CLI integration tests for 7 flags | create |
| `crates/kebab-mcp/tests/tools_call_search.rs` | extend with filter input cases | modify |
| `README.md` | `kebab search` row update | modify |
| `docs/SMOKE.md` | filter walkthrough | modify |
| `tasks/p9/p9-fb-36-search-filters.md` | status flip + design/plan links | modify |
| `tasks/INDEX.md` | fb-36 row → ✅ | modify |
| `integrations/claude-code/kebab/SKILL.md` | `mcp__kebab__search` input shape doc + filter examples | modify |

---

## Pre-flight

- [ ] **Step 0.1: Branch off main**

```bash
git checkout main
git pull
git checkout -b feat/fb-36-search-filters
```

- [ ] **Step 0.2: Confirm spec branch reachable**

```bash
git log --oneline spec/fb-36-search-filters -1
```

Expected: `7210386 spec(fb-36): search filter args — design`. If spec PR not yet merged, `git merge spec/fb-36-search-filters`.

---

## Task 1: Domain — `SearchFilters` 3 new fields

**Files:**
- Modify: `crates/kebab-core/src/search.rs`

- [ ] **Step 1.1: Failing test**

Append to `crates/kebab-core/src/search.rs` `#[cfg(test)] mod tests`:

```rust
#[test]
fn search_filters_default_includes_new_fb36_fields() {
    let f = SearchFilters::default();
    assert!(f.media.is_empty(), "media default empty");
    assert!(f.ingested_after.is_none(), "ingested_after default None");
    assert!(f.doc_id.is_none(), "doc_id default None");
    // existing fields still default
    assert!(f.tags_any.is_empty());
    assert!(f.lang.is_none());
    assert!(f.path_glob.is_none());
    assert!(f.trust_min.is_none());
}

#[test]
fn search_filters_serialize_with_serde_default_compat() {
    // Old JSON without the new fields must still deserialize.
    let old: SearchFilters = serde_json::from_str(r#"{"tags_any":[],"lang":null,"path_glob":null,"trust_min":null}"#).unwrap();
    assert!(old.media.is_empty());
    assert!(old.ingested_after.is_none());
    assert!(old.doc_id.is_none());
}
```

- [ ] **Step 1.2: Run test (verify failure)**

```bash
cargo test -p kebab-core search_filters_default_includes_new_fb36_fields
```

Expected: FAIL — fields don't exist.

- [ ] **Step 1.3: Add the fields**

Edit `SearchFilters` struct in `crates/kebab-core/src/search.rs`:

```rust
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SearchFilters {
    pub tags_any: Vec<String>,
    pub lang: Option<Lang>,
    pub path_glob: Option<String>,
    pub trust_min: Option<TrustLevel>,
    /// p9-fb-36: media_type filter — IN-list of `MediaType.kind`
    /// strings (`"markdown"`, `"pdf"`, `"image"`, `"audio"`, `"other"`).
    /// Empty Vec = no filter. Match is on the variant tag only;
    /// e.g. `["image"]` matches `Image(Png)` and `Image(Jpeg)`.
    #[serde(default)]
    pub media: Vec<String>,
    /// p9-fb-36: hits whose source doc's `documents.updated_at` is at
    /// or after this timestamp. None = no filter. RFC3339 / UTC.
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub ingested_after: Option<OffsetDateTime>,
    /// p9-fb-36: restrict hits to a single document. None = no filter.
    #[serde(default)]
    pub doc_id: Option<DocumentId>,
}
```

`OffsetDateTime` is already imported (other fields use it). `DocumentId` is already in scope. If neither is, add:

```rust
use time::OffsetDateTime;
use crate::ids::DocumentId;
```

Also expose a `MEDIA_KINDS` const that downstream code can use for validation / aliases:

```rust
/// p9-fb-36: canonical kind labels for `SearchFilters.media`. Mirrors
/// `MediaType` variant tags; CLI / MCP normalize aliases (`md` → `markdown`)
/// before populating this Vec.
pub const MEDIA_KINDS: &[&str] = &["markdown", "pdf", "image", "audio", "other"];
```

- [ ] **Step 1.4: Run tests (verify pass)**

```bash
cargo test -p kebab-core
```

Expected: 33+ tests pass (2 new + existing).

Other crates may break (lexical / vector retrievers reference `SearchFilters`). That's expected — Tasks 2/3 fix.

- [ ] **Step 1.5: Commit**

```bash
git add crates/kebab-core/src/search.rs
git commit -m "$(cat <<'EOF'
feat(core): SearchFilters gains media / ingested_after / doc_id (fb-36)

3 additive optional fields. #[serde(default)] preserves
backwards compat for older JSON without the new keys.
MEDIA_KINDS const exposes canonical "markdown"/"pdf"/"image"/
"audio"/"other" labels for downstream alias normalization.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: Lexical retriever — SQL WHERE extension

**Files:**
- Modify: `crates/kebab-search/src/lexical.rs`
- Modify: `crates/kebab-search/tests/lexical.rs`

- [ ] **Step 2.1: Failing tests**

Append to `crates/kebab-search/tests/lexical.rs`:

```rust
#[test]
fn lexical_filter_by_media() {
    let env = TestEnv::new();
    env.insert_doc_with_media("md1.md", "rust ownership", kebab_core::MediaType::Markdown);
    env.insert_doc_with_media("doc.pdf", "rust pdf body", kebab_core::MediaType::Pdf);
    let filters = kebab_core::SearchFilters {
        media: vec!["pdf".to_string()],
        ..Default::default()
    };
    let hits = env.run_search("rust", &filters);
    assert_eq!(hits.len(), 1, "only pdf doc should match");
    assert!(hits[0].doc_path.0.ends_with(".pdf"), "got: {}", hits[0].doc_path.0);
}

#[test]
fn lexical_filter_by_ingested_after() {
    let env = TestEnv::new();
    let old_doc = env.insert_doc_with_updated_at(
        "old.md",
        "ingest test",
        time::macros::datetime!(2020-01-01 00:00:00 UTC),
    );
    let new_doc = env.insert_doc_with_updated_at(
        "new.md",
        "ingest test",
        time::macros::datetime!(2026-01-01 00:00:00 UTC),
    );
    let filters = kebab_core::SearchFilters {
        ingested_after: Some(time::macros::datetime!(2025-01-01 00:00:00 UTC)),
        ..Default::default()
    };
    let hits = env.run_search("ingest", &filters);
    let _ = (old_doc, new_doc);
    assert_eq!(hits.len(), 1, "only post-2025 doc matches");
}

#[test]
fn lexical_filter_by_doc_id() {
    let env = TestEnv::new();
    let target = env.insert_doc("a.md", "shared term");
    env.insert_doc("b.md", "shared term");
    let filters = kebab_core::SearchFilters {
        doc_id: Some(target.clone()),
        ..Default::default()
    };
    let hits = env.run_search("shared", &filters);
    for h in &hits {
        assert_eq!(h.doc_id, target, "all hits must be from target doc");
    }
}

#[test]
fn lexical_filter_combinator_is_and() {
    let env = TestEnv::new();
    let target = env.insert_doc_with_media("a.md", "rust", kebab_core::MediaType::Markdown);
    env.insert_doc_with_media("b.pdf", "rust", kebab_core::MediaType::Pdf);
    let filters = kebab_core::SearchFilters {
        media: vec!["markdown".to_string()],
        doc_id: Some(target.clone()),
        ..Default::default()
    };
    let hits = env.run_search("rust", &filters);
    assert!(hits.iter().all(|h| h.doc_id == target));
}

#[test]
fn lexical_filter_unknown_media_returns_empty() {
    let env = TestEnv::new();
    env.insert_doc("a.md", "rust");
    let filters = kebab_core::SearchFilters {
        media: vec!["nonexistent_kind".to_string()],
        ..Default::default()
    };
    let hits = env.run_search("rust", &filters);
    assert!(hits.is_empty(), "unknown media → no hits, no error");
}

#[test]
fn lexical_empty_filters_match_default_behavior() {
    let env = TestEnv::new();
    env.insert_doc("a.md", "rust");
    let with_default = env.run_search("rust", &kebab_core::SearchFilters::default());
    assert!(!with_default.is_empty());
}
```

The `TestEnv` helper functions (`insert_doc`, `insert_doc_with_media`, `insert_doc_with_updated_at`, `run_search`) need to exist in the test scaffold. Check what's there:

```bash
grep -n "pub fn insert_doc\|pub fn run_search\|TestEnv" crates/kebab-search/tests/common/mod.rs 2>/dev/null
ls crates/kebab-search/tests/
```

If missing, add minimal helpers to `crates/kebab-search/tests/common/mod.rs` (create the file if needed):

```rust
//! Lexical-test helpers shared across kebab-search integration tests.

use std::sync::Arc;

use kebab_core::{
    DocumentId, MediaType, SearchFilters, SearchHit, SearchMode, SearchQuery,
};
use kebab_search::LexicalRetriever;
use kebab_store_sqlite::SqliteStore;
use time::OffsetDateTime;

pub struct TestEnv {
    pub store: Arc<SqliteStore>,
    pub retriever: LexicalRetriever,
    next: std::cell::Cell<usize>,
}

impl TestEnv {
    pub fn new() -> Self {
        // ... use whatever the existing tests do for store init.
        // Mirror the pattern in crates/kebab-search/tests/lexical.rs that
        // sets up an in-memory or tempdir SqliteStore + LexicalRetriever.
        unimplemented!("copy the existing test scaffold's setup")
    }

    pub fn insert_doc(&self, path: &str, body: &str) -> DocumentId {
        self.insert_doc_with_media(path, body, MediaType::Markdown)
    }

    pub fn insert_doc_with_media(
        &self,
        path: &str,
        body: &str,
        media: MediaType,
    ) -> DocumentId {
        self.insert_doc_with_updated_at(path, body, OffsetDateTime::now_utc())
            // (set the media via a separate write or threading through
            // whatever fixture helper the existing tests use)
    }

    pub fn insert_doc_with_updated_at(
        &self,
        path: &str,
        body: &str,
        updated_at: OffsetDateTime,
    ) -> DocumentId {
        // Insert a synthetic document + asset row + chunks + FTS row.
        // Match the pattern used in the existing lexical / hybrid tests
        // (which already use TestEnv-like helpers — adapt their signatures).
        unimplemented!("see existing test scaffold")
    }

    pub fn run_search(&self, query: &str, filters: &SearchFilters) -> Vec<SearchHit> {
        let q = SearchQuery {
            text: query.to_string(),
            mode: SearchMode::Lexical,
            k: 10,
            filters: filters.clone(),
        };
        kebab_core::Retriever::search(&self.retriever, &q).expect("search")
    }
}
```

The "unimplemented" placeholders must be replaced with concrete code — see `crates/kebab-search/tests/lexical.rs`'s existing test setup for the right pattern (likely something like `init_store_with_doc_and_chunk(...)`). Take the time to study what's there and mirror it. The plan can't enumerate the full scaffold here because it depends on the codebase's existing fixtures.

If the existing tests already have similar helpers under different names, REUSE them — don't add a new TestEnv. The new fixture-needing helpers (`insert_doc_with_media`, `insert_doc_with_updated_at`) are the only genuinely new pieces.

- [ ] **Step 2.2: Run tests (verify failure)**

```bash
cargo test -p kebab-search --test lexical lexical_filter_by_media
```

Expected: FAIL — `lexical.rs` doesn't yet handle `media` filter; the test would either compile fail (helpers missing) or assertion fail.

- [ ] **Step 2.3: Implement SQL WHERE extension**

Edit `crates/kebab-search/src/lexical.rs::run_query`. Find the existing WHERE clause builder block (after `tags_any` / `lang` / `trust_min` arms — see line ~280-320). Add the 3 new arms BEFORE the `path_glob` post-filter (path_glob stays in Rust):

```rust
// p9-fb-36: media_type filter (IN-list).
// `assets.media_type` JSON has two shapes:
//   - unit variant (Markdown / Pdf): JSON text, e.g. `"markdown"`
//   - tuple variant (Image(Png) / Audio(Mp3) / Other(s)): JSON object,
//     e.g. `{"image": "png"}`
// Extract a unified "kind" string for both shapes via:
//   CASE WHEN json_type = 'text' THEN json_extract($)
//        ELSE (first object key)
//   END IN (?, ...)
if !filters.media.is_empty() {
    let placeholders: Vec<&str> = std::iter::repeat_n("?", filters.media.len()).collect();
    let placeholders = placeholders.join(",");
    sql.push_str(&format!(
        " AND f.doc_id IN (SELECT doc_id FROM documents d2 \
           JOIN assets a ON a.asset_id = d2.asset_id \
           WHERE CASE \
             WHEN json_type(a.media_type) = 'text' THEN json_extract(a.media_type, '$') \
             ELSE (SELECT key FROM json_each(a.media_type) LIMIT 1) \
           END IN ({placeholders}))"
    ));
    for kind in &filters.media {
        params.push(Box::new(kind.clone()));
    }
}

// p9-fb-36: ingested_after filter.
// `documents.updated_at` is RFC3339 stored as TEXT (always UTC `Z` per
// fb-32 ingest path), so lexicographic >= compare is correct.
if let Some(after) = &filters.ingested_after {
    let formatted = after
        .format(&time::format_description::well_known::Rfc3339)
        .expect("OffsetDateTime formats to RFC3339");
    sql.push_str(" AND d.updated_at >= ?");
    params.push(Box::new(formatted));
}

// p9-fb-36: doc_id filter — single-doc scoping.
if let Some(id) = &filters.doc_id {
    sql.push_str(" AND d.doc_id = ?");
    params.push(Box::new(id.0.clone()));
}
```

The exact `params` API depends on the existing builder pattern in `lexical.rs`. The current code uses something like `let mut params: Vec<Box<dyn ToSql>> = vec![...];`. Match that exactly. Don't introduce a new pattern.

If the existing SQL has joins on `documents d` already (via `chunks → documents`), the `media` subquery uses `documents d2` to avoid alias collision. Read the existing SQL string to verify.

- [ ] **Step 2.4: Run tests (verify pass)**

```bash
cargo test -p kebab-search --test lexical
```

Expected: all PASS, including 6 new fb-36 tests.

If the helpers in Step 2.1 weren't fleshed out, this is the moment to fill them in — they're the bridge between the test text above and the actual store setup. The store crate's `tests/contract_roundtrip.rs` is a good model for inserting an asset + document + chunks fixture.

- [ ] **Step 2.5: Commit**

```bash
git add crates/kebab-search/src/lexical.rs crates/kebab-search/tests/
git commit -m "$(cat <<'EOF'
feat(search/lexical): media / ingested_after / doc_id filters (fb-36)

SQL WHERE clause extension. media uses CASE WHEN json_type='text'
to handle both unit (`"markdown"`) and tuple (`{"image":"png"}`)
MediaType serde shapes. ingested_after relies on RFC3339 lexicographic
ordering with UTC Z (per fb-32 ingest invariant). doc_id is a simple
equality. AND combinator with existing tags / lang / trust filters.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: Vector retriever — `filter_chunks` mirror

**Files:**
- Modify: `crates/kebab-search/src/vector.rs`
- Modify: `crates/kebab-search/tests/hybrid.rs`

- [ ] **Step 3.1: Failing test**

Append to `crates/kebab-search/tests/hybrid.rs`:

```rust
#[test]
fn vector_filter_by_media() {
    let env = HybridTestEnv::new();
    env.insert_doc_with_media("md1.md", "rust ownership", kebab_core::MediaType::Markdown);
    env.insert_doc_with_media("doc.pdf", "rust pdf body", kebab_core::MediaType::Pdf);

    let filters = kebab_core::SearchFilters {
        media: vec!["pdf".to_string()],
        ..Default::default()
    };
    let hits = env.run_vector_search("rust", &filters);
    assert_eq!(hits.len(), 1);
    assert!(hits[0].doc_path.0.ends_with(".pdf"));
}

#[test]
fn vector_filter_by_doc_id() {
    let env = HybridTestEnv::new();
    let target = env.insert_doc("a.md", "shared");
    env.insert_doc("b.md", "shared");
    let filters = kebab_core::SearchFilters {
        doc_id: Some(target.clone()),
        ..Default::default()
    };
    let hits = env.run_vector_search("shared", &filters);
    assert!(hits.iter().all(|h| h.doc_id == target));
}
```

Mirror the helpers needed in `crates/kebab-search/tests/common/mod.rs` (add `HybridTestEnv` if it doesn't exist; copy the pattern from existing hybrid tests).

- [ ] **Step 3.2: Run tests (verify failure)**

```bash
cargo test -p kebab-search --test hybrid vector_filter_by_media
```

Expected: FAIL.

- [ ] **Step 3.3: Implement filter_chunks extension**

Edit `crates/kebab-search/src/vector.rs::filter_chunks` (or whatever helper the vector retriever uses to post-filter SQLite-side after Lance returns chunks). Add the same 3 SQL fragments as Task 2.

If `filter_chunks` builds its own SQL inline, match the lexical pattern verbatim. If it delegates to a shared SQL helper in `kebab-store-sqlite`, refactor: extract the "filter WHERE clause builder" into a small helper used by both. Inspect first:

```bash
grep -n "filter_chunks\|tags_any\|trust_min\|lang" crates/kebab-search/src/vector.rs | head -10
```

Decide: in-place duplication vs shared helper. Shared helper is cleaner if the SQL is identical. If the contexts differ (lexical SQL is a single statement, vector SQL is a follow-up `SELECT ... WHERE chunk_id IN (...) AND <filters>`), keep them separate but mirror the new filter pattern exactly.

- [ ] **Step 3.4: Run tests (verify pass)**

```bash
cargo test -p kebab-search --test hybrid
cargo test -p kebab-search
```

Expected: all PASS.

- [ ] **Step 3.5: Commit**

```bash
git add crates/kebab-search/src/vector.rs crates/kebab-search/tests/
git commit -m "$(cat <<'EOF'
feat(search/vector): media / ingested_after / doc_id filters (fb-36)

filter_chunks helper extended with the same 3 WHERE clauses as
lexical. Vector still over-fetches k * 2 then post-filters; small
k can return < k hits when filters drop a lot — agent is expected
to widen k or paginate. AND combinator with existing filters.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: CLI flags + dispatch

**Files:**
- Modify: `crates/kebab-cli/src/main.rs`

- [ ] **Step 4.1: Add `TrustLevelFlag` clap enum**

Locate the existing `enum Cmd` and `enum ModeFlag` (or similar) declarations. Add near them:

```rust
#[derive(clap::ValueEnum, Clone, Debug)]
enum TrustLevelFlag {
    Trusted,
    Reviewed,
    Hearsay,
    Untrusted,
}

impl From<TrustLevelFlag> for kebab_core::TrustLevel {
    fn from(f: TrustLevelFlag) -> Self {
        match f {
            TrustLevelFlag::Trusted => kebab_core::TrustLevel::Trusted,
            TrustLevelFlag::Reviewed => kebab_core::TrustLevel::Reviewed,
            TrustLevelFlag::Hearsay => kebab_core::TrustLevel::Hearsay,
            TrustLevelFlag::Untrusted => kebab_core::TrustLevel::Untrusted,
        }
    }
}
```

If `TrustLevel` variants are different (verify):

```bash
grep -A 8 "^pub enum TrustLevel" crates/kebab-core/src/metadata.rs
```

Adapt names accordingly.

- [ ] **Step 4.2: Add 7 flags to `Cmd::Search`**

In the `enum Cmd { ... Search { ... } }` definition, add 7 fields:

```rust
/// p9-fb-36: filter by `metadata.tags`. Repeatable; OR-within (any tag).
#[arg(long)]
tag: Vec<String>,

/// p9-fb-36: filter by `documents.lang` (ISO code).
#[arg(long)]
lang: Option<String>,

/// p9-fb-36: filter by `documents.workspace_path` glob.
#[arg(long)]
path_glob: Option<String>,

/// p9-fb-36: filter by minimum `documents.trust_level`.
#[arg(long, value_enum)]
trust_min: Option<TrustLevelFlag>,

/// p9-fb-36: filter by `assets.media_type` kind. Comma-separated.
/// Aliases: `md` → `markdown`. Other accepted: `markdown`, `pdf`,
/// `image`, `audio`, `other`. Unknown values match nothing.
#[arg(long, value_delimiter = ',')]
media: Vec<String>,

/// p9-fb-36: filter to docs whose `updated_at` is >= this RFC3339
/// timestamp (UTC). Invalid format → exit 2 with error.v1
/// code = config_invalid.
#[arg(long)]
ingested_after: Option<String>,

/// p9-fb-36: filter to a single doc by id.
#[arg(long)]
doc_id: Option<String>,
```

- [ ] **Step 4.3: Build SearchFilters in dispatch arm**

In the `Cmd::Search { ... } =>` match arm body, before the `let q = kebab_core::SearchQuery { ... }` line, replace the hardcoded `filters: kebab_core::SearchFilters::default()` with a constructed `SearchFilters`. Also normalize `--media` aliases:

```rust
fn normalize_media_alias(s: &str) -> String {
    match s.to_ascii_lowercase().as_str() {
        "md" => "markdown".to_string(),
        other => other.to_string(),
    }
}

let media_norm: Vec<String> = media.iter().map(|s| normalize_media_alias(s)).collect();

let ingested_after_parsed: Option<time::OffsetDateTime> = match ingested_after.as_deref() {
    Some(s) => {
        let parsed = time::OffsetDateTime::parse(
            s,
            &time::format_description::well_known::Rfc3339,
        );
        match parsed {
            Ok(ts) => Some(ts),
            Err(e) => {
                let err = anyhow::Error::new(kebab_app::StructuredError(kebab_app::ErrorV1 {
                    schema_version: "error.v1".to_string(),
                    code: "config_invalid".to_string(),
                    message: format!("--ingested-after: invalid RFC3339 timestamp '{s}': {e}"),
                    details: serde_json::Value::Null,
                    hint: Some("expected format like 2026-04-01T00:00:00Z".to_string()),
                }));
                return Err(err);
            }
        }
    }
    None => None,
};

let filters = kebab_core::SearchFilters {
    tags_any: tag.clone(),
    lang: lang.as_ref().map(|s| kebab_core::Lang(s.clone())),
    path_glob: path_glob.clone(),
    trust_min: trust_min.clone().map(Into::into),
    media: media_norm,
    ingested_after: ingested_after_parsed,
    doc_id: doc_id.as_ref().map(|s| kebab_core::DocumentId(s.clone())),
};

let q = kebab_core::SearchQuery {
    text: query.clone(),
    mode: (*mode).into(),
    k: *k,
    filters,
};
```

If `Lang` constructor differs (e.g. `Lang::new(...)` vs `Lang(s)`), check:

```bash
grep -A 3 "^pub struct Lang\b" crates/kebab-core/src/media.rs
```

If the existing `Cmd::Search` arm doesn't currently `return Err(...)` for failures, the dispatch's outer `Result<()>` should catch the anyhow propagation through `?`. Verify the existing pattern.

- [ ] **Step 4.4: Build CLI**

```bash
cargo build -p kebab-cli
```

Expected: clean.

- [ ] **Step 4.5: Verify --help**

```bash
cargo run -q -p kebab-cli -- search --help 2>&1 | grep -E "tag|lang|path-glob|trust-min|media|ingested-after|doc-id"
```

Expected: 7 new flags appear.

- [ ] **Step 4.6: Run kebab-cli tests**

```bash
cargo test -p kebab-cli
```

Expected: all PASS, no regressions.

- [ ] **Step 4.7: Commit**

```bash
git add crates/kebab-cli/src/main.rs
git commit -m "$(cat <<'EOF'
feat(cli): kebab search filter flags (fb-36)

7 new flags: --tag (repeatable), --lang, --path-glob,
--trust-min (value_enum), --media (csv with `md` alias),
--ingested-after (RFC3339; config_invalid on parse fail),
--doc-id. Dispatch translates clap values into SearchFilters
and propagates structured errors through the existing
StructuredError wrapper from fb-34.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: CLI integration tests

**Files:**
- Create: `crates/kebab-cli/tests/wire_search_filters.rs`
- Modify: `crates/kebab-cli/tests/common/mod.rs` (if helper missing)

- [ ] **Step 5.1: Write integration tests**

Create `crates/kebab-cli/tests/wire_search_filters.rs`:

```rust
//! p9-fb-36: CLI search filter flags.

mod common;

use serde_json::Value;

#[test]
fn search_with_doc_id_filter_returns_only_target_doc() {
    let (cfg, ws) = common::write_config();
    common::ingest(&cfg, &ws, "a.md", "# A\n\nshared term apple\n");
    common::ingest(&cfg, &ws, "b.md", "# B\n\nshared term banana\n");

    // Find any doc_id via search.
    let (probe_stdout, _) = common::run_search_with_args(
        &cfg,
        &["--mode", "lexical", "--json", "--k", "5", "shared"],
    );
    let probe: Value = serde_json::from_str(probe_stdout.trim()).expect("probe json");
    let target_doc_id = probe["hits"][0]["doc_id"]
        .as_str()
        .expect("doc_id in first hit")
        .to_string();

    let (stdout, _) = common::run_search_with_args(
        &cfg,
        &["--mode", "lexical", "--json", "--doc-id", &target_doc_id, "shared"],
    );
    let v: Value = serde_json::from_str(stdout.trim()).expect("filtered json");
    let hits = v["hits"].as_array().expect("hits array");
    assert!(!hits.is_empty(), "filter should still match the target doc");
    for h in hits {
        assert_eq!(h["doc_id"], target_doc_id);
    }
}

#[test]
fn search_with_invalid_ingested_after_emits_config_invalid() {
    let (cfg, _ws) = common::write_config();

    let exe = env!("CARGO_BIN_EXE_kebab");
    let cfg_str = cfg.to_str().expect("utf8");
    let out = std::process::Command::new(exe)
        .args([
            "--config", cfg_str, "--json",
            "search", "--mode", "lexical",
            "--ingested-after", "not-a-timestamp",
            "test",
        ])
        .output()
        .expect("kebab search");
    assert_ne!(out.status.code(), Some(0));
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
    assert_eq!(v["code"], "config_invalid");
    assert!(
        v["message"].as_str().unwrap_or("").contains("ingested-after"),
        "message should mention the flag: {v:?}"
    );
}

#[test]
fn search_with_media_filter_md_alias_normalizes_to_markdown() {
    let (cfg, ws) = common::write_config();
    common::ingest(&cfg, &ws, "a.md", "# A\n\nrust ownership body\n");

    let (stdout, _) = common::run_search_with_args(
        &cfg,
        &["--mode", "lexical", "--json", "--media", "md", "rust"],
    );
    let v: Value = serde_json::from_str(stdout.trim()).expect("json");
    let hits = v["hits"].as_array().expect("hits");
    assert!(!hits.is_empty(), "md alias should match markdown doc");
}

#[test]
fn search_with_tag_filter_repeats_or_within() {
    let (cfg, ws) = common::write_config();
    // Tag-aware ingest: write a doc with frontmatter tags. The
    // markdown parser captures them into `metadata.tags`.
    common::ingest(
        &cfg,
        &ws,
        "tagged.md",
        "---\ntags: [rust, async]\n---\n\n# Tagged\n\nbody about rust\n",
    );
    common::ingest(&cfg, &ws, "untagged.md", "# Plain\n\nbody about rust\n");

    // --tag rust → tagged doc only.
    let (stdout, _) = common::run_search_with_args(
        &cfg,
        &["--mode", "lexical", "--json", "--tag", "rust", "--k", "10", "rust"],
    );
    let v: Value = serde_json::from_str(stdout.trim()).expect("json");
    let hits = v["hits"].as_array().expect("hits");
    assert!(!hits.is_empty(), "tagged doc should match");
    for h in hits {
        let path = h["doc_path"].as_str().unwrap_or("");
        assert_eq!(path, "tagged.md", "untagged doc should be filtered out");
    }
}
```

If `common::write_config` / `common::ingest` / `common::run_search_with_args` already exist (they do from fb-32 / fb-34), reuse. The test file imports them via `mod common;`.

- [ ] **Step 5.2: Run tests**

```bash
cargo test -p kebab-cli --test wire_search_filters 2>&1 | tail -10
```

Expected: 4 PASS.

If the tag-frontmatter test fails because parser doesn't capture tags from this exact format, simplify the test or check what frontmatter shape the codebase expects:

```bash
grep -rn "metadata.tags\|frontmatter.*tags" crates/kebab-parse-md/src/ 2>/dev/null | head -5
```

Adapt the fixture frontmatter to the parser's expected shape.

- [ ] **Step 5.3: Run full kebab-cli suite**

```bash
cargo test -p kebab-cli
```

Expected: all PASS.

- [ ] **Step 5.4: Commit**

```bash
git add crates/kebab-cli/tests/
git commit -m "$(cat <<'EOF'
test(cli): wire_search_filters — 4 lexical-only integration tests (fb-36)

Cover: --doc-id scoping, --ingested-after validation error,
--media md alias, --tag repeatable + frontmatter parsing.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: MCP `SearchInput` extension

**Files:**
- Modify: `crates/kebab-mcp/src/tools/search.rs`
- Modify: `crates/kebab-mcp/tests/tools_call_search.rs`

- [ ] **Step 6.1: Inspect current `SearchInput`**

```bash
sed -n '1,80p' crates/kebab-mcp/src/tools/search.rs
```

Note where `mode` / `k` / `max_tokens` / `cursor` are wired.

- [ ] **Step 6.2: Add 7 fields to `SearchInput`**

Edit the struct:

```rust
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SearchInput {
    pub query: String,
    pub mode: Option<String>,
    pub k: Option<usize>,
    pub max_tokens: Option<usize>,
    pub snippet_chars: Option<usize>,
    pub cursor: Option<String>,
    /// p9-fb-36: filter by `metadata.tags` (OR-within).
    pub tags: Option<Vec<String>>,
    /// p9-fb-36: filter by `documents.lang` (ISO code).
    pub lang: Option<String>,
    /// p9-fb-36: filter by `documents.workspace_path` glob.
    pub path_glob: Option<String>,
    /// p9-fb-36: filter by minimum `documents.trust_level`.
    /// Accepts: `"trusted"`, `"reviewed"`, `"hearsay"`, `"untrusted"`.
    pub trust_min: Option<String>,
    /// p9-fb-36: filter by `assets.media_type` kind. IN-list. Accepts:
    /// `"markdown"`, `"pdf"`, `"image"`, `"audio"`, `"other"`.
    pub media: Option<Vec<String>>,
    /// p9-fb-36: RFC3339 UTC timestamp. Invalid format → invalid_input.
    pub ingested_after: Option<String>,
    /// p9-fb-36: filter to a single doc.
    pub doc_id: Option<String>,
}
```

- [ ] **Step 6.3: Update dispatch**

In `handle(state, input)`, before constructing `SearchOpts`, build `SearchFilters` from the new inputs:

```rust
let trust_min = match input.trust_min.as_deref() {
    Some("trusted") => Some(kebab_core::TrustLevel::Trusted),
    Some("reviewed") => Some(kebab_core::TrustLevel::Reviewed),
    Some("hearsay") => Some(kebab_core::TrustLevel::Hearsay),
    Some("untrusted") => Some(kebab_core::TrustLevel::Untrusted),
    Some(other) => {
        return invalid_input(&format!(
            "trust_min: unknown level '{other}'; expected trusted|reviewed|hearsay|untrusted"
        ));
    }
    None => None,
};

let ingested_after = match input.ingested_after.as_deref() {
    Some(s) => {
        match time::OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339) {
            Ok(ts) => Some(ts),
            Err(e) => return invalid_input(&format!("ingested_after: invalid RFC3339 '{s}': {e}")),
        }
    }
    None => None,
};

let filters = kebab_core::SearchFilters {
    tags_any: input.tags.unwrap_or_default(),
    lang: input.lang.map(kebab_core::Lang),
    path_glob: input.path_glob,
    trust_min,
    media: input.media.unwrap_or_default(),
    ingested_after,
    doc_id: input.doc_id.map(kebab_core::DocumentId),
};

let query = kebab_core::SearchQuery {
    text: input.query,
    mode,
    k: input.k.unwrap_or(10).clamp(1, 100),
    filters,
};
```

If `invalid_input` helper doesn't exist in this file (per fb-35 `tools/fetch.rs` pattern), add one:

```rust
fn invalid_input(msg: &str) -> CallToolResult {
    use kebab_app::{ErrorV1, StructuredError};
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

If the existing dispatch hardcodes `SearchFilters::default()`, replace with the new `filters` value above.

- [ ] **Step 6.4: Add MCP test cases**

Edit `crates/kebab-mcp/tests/tools_call_search.rs`. Add tests:

```rust
#[test]
fn search_with_doc_id_filter_returns_only_target() {
    // Mirror the existing tools_call_search.rs setup pattern.
    // After ingesting 2 docs and discovering target doc_id from a
    // baseline search, call mcp__kebab__search with doc_id set and
    // assert v["hits"] all have doc_id == target.
    // (Concrete test code mirrors what fb-34 / fb-35 added; see them
    // for the helper pattern this crate uses.)
}

#[test]
fn search_with_invalid_ingested_after_returns_invalid_input() {
    // Same MCP scaffold. Call with ingested_after = "garbage", assert
    // the response carries error.v1 with code = "invalid_input" and
    // message containing "ingested_after".
}
```

Implement against whatever the existing tools_call_search.rs scaffold uses. The fb-34/35 tests are good templates.

- [ ] **Step 6.5: Run MCP tests**

```bash
cargo test -p kebab-mcp
```

Expected: all PASS.

- [ ] **Step 6.6: Commit**

```bash
git add crates/kebab-mcp/
git commit -m "$(cat <<'EOF'
feat(mcp): kebab__search filter inputs (fb-36)

7 new optional inputs on SearchInput: tags, lang, path_glob,
trust_min, media, ingested_after, doc_id. Validation surfaces as
error.v1 code = invalid_input via StructuredError. Dispatch builds
SearchFilters from the inputs and forwards through the existing
search_with_opts_with_config facade.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: Workspace test + clippy

- [ ] **Step 7.1: Workspace test**

```bash
cargo test --workspace --no-fail-fast -j 1 2>&1 | tail -15
```

Expected: all PASS.

- [ ] **Step 7.2: Clippy**

```bash
cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -10
```

Expected: clean.

- [ ] **Step 7.3: Commit any clippy fixes**

```bash
git add -A
git commit -m "chore: clippy fixes for fb-36"
```

(Skip if no fixes needed.)

---

## Task 8: Documentation updates

**Files:**
- Modify: `README.md`
- Modify: `docs/SMOKE.md`
- Modify: `tasks/p9/p9-fb-36-search-filters.md`
- Modify: `tasks/INDEX.md`
- Modify: `integrations/claude-code/kebab/SKILL.md`

- [ ] **Step 8.1: README — search row update**

Find the `kebab search` row in 명령 table:

```bash
grep -n "kebab search" README.md | head -5
```

Append filter flags. The row gets long — keep concise:

> `... [--tag <tag>] [--lang <iso>] [--path-glob <glob>] [--trust-min <level>] [--media md,pdf,...] [--ingested-after <RFC3339>] [--doc-id <id>]` (p9-fb-36 — filter args. AND combinator across flags; OR within --tag/--media. Invalid `--ingested-after` RFC3339 → `error.v1.code = config_invalid`.)

- [ ] **Step 8.2: SMOKE.md — filter walkthrough**

After the existing fb-35 verbatim fetch section, append:

```markdown
### Filter args (fb-36)

```bash
# Filter by media kind (md alias normalizes to markdown).
kebab search "rust" --media md --json | jq '.hits | length'

# Filter by ingest timestamp (RFC3339).
kebab search "rust" --ingested-after 2026-04-01T00:00:00Z --json

# Combine: doc-id scope + tag (AND across flags).
kebab search "rust" --doc-id "<doc-id>" --tag rust --json
```

Bad `--ingested-after` → `error.v1.code = config_invalid`, exit 2.
Unknown `--media` value → silently empty (no error).
```

- [ ] **Step 8.3: Spec status flip**

Edit `tasks/p9/p9-fb-36-search-filters.md`:

```diff
-status: open
+status: completed
```

Replace the `> ⏳ **백로그 only — 미구현.**` block with:

```markdown
> ✅ **구현 완료.** 본 spec 은 구현 시점의 frozen 상태. post-merge deviation 은 [HOTFIXES.md](../HOTFIXES.md) 참조.

상세 설계: `docs/superpowers/specs/2026-05-10-p9-fb-36-search-filters-design.md`.
구현 계획: `docs/superpowers/plans/2026-05-10-p9-fb-36-search-filters.md`.
```

- [ ] **Step 8.4: tasks/INDEX.md**

```diff
-    - [p9-fb-36 search filter args](p9/p9-fb-36-search-filters.md) — ⏳ 미구현, brainstorm 필요 (depends_on 27)
+    - [p9-fb-36 search filter args](p9/p9-fb-36-search-filters.md) — ✅ 머지 + v0.5.0 cut 후보 (2026-05-10)
```

(The `depends_on 27` annotation in the original was carried over from the spec stub; drop it.)

- [ ] **Step 8.5: SKILL.md — search input shape**

Find the existing `mcp__kebab__search` Input section:

```bash
grep -n "mcp__kebab__search\|max_tokens.*null" integrations/claude-code/kebab/SKILL.md | head -5
```

Update the example input + bullets to mention the 7 new fields:

```markdown
Input:
```json
{
  "query": "<query>",
  "mode": "hybrid",
  "k": 10,
  "max_tokens": null,
  "snippet_chars": null,
  "cursor": null,
  "tags": null,
  "lang": null,
  "path_glob": null,
  "trust_min": null,
  "media": null,
  "ingested_after": null,
  "doc_id": null
}
```

- p9-fb-36 filter inputs: `tags` (OR-within), `lang`, `path_glob`, `trust_min`, `media` (IN-list of `markdown|pdf|image|audio|other`), `ingested_after` (RFC3339 UTC), `doc_id`. AND combinator across keys. Invalid `ingested_after` / unknown `trust_min` → `error.v1.code = invalid_input`. Unknown `media` value → empty hits, no error.
```

- [ ] **Step 8.6: Commit docs**

```bash
git add README.md docs/SMOKE.md tasks/p9/p9-fb-36-search-filters.md tasks/INDEX.md integrations/claude-code/kebab/SKILL.md
git commit -m "$(cat <<'EOF'
docs(fb-36): README + SMOKE + INDEX + skill notes

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 9: Smoke + push + PR

- [ ] **Step 9.1: Manual smoke**

```bash
cd /tmp/kebab-smoke
~/Workspace/projects/kebab/target/release/kebab --config /tmp/kebab-smoke/config.toml ingest
~/Workspace/projects/kebab/target/release/kebab --config /tmp/kebab-smoke/config.toml search "test" --json --media md | jq '{hits: (.hits | length)}'
~/Workspace/projects/kebab/target/release/kebab --config /tmp/kebab-smoke/config.toml search "test" --json --ingested-after garbage 2>&1 | tail -5
```

Expected:
- `--media md` returns sane hit count.
- garbage `--ingested-after` exits non-zero with `error.v1.code = config_invalid` on stderr.

- [ ] **Step 9.2: Final workspace test**

```bash
cd ~/Workspace/projects/kebab
cargo test --workspace --no-fail-fast -j 1
```

Expected: all green.

- [ ] **Step 9.3: Push branch**

```bash
git push -u origin feat/fb-36-search-filters
```

- [ ] **Step 9.4: Open PR**

Build PR body at `/tmp/fb36-pr-body.md`:

```markdown
## Summary

- adds 7 filter flags on `kebab search` and the equivalent inputs on `mcp__kebab__search`:
  - existing `SearchFilters` fields exposed: `--tag` (repeatable, OR-within), `--lang`, `--path-glob`, `--trust-min`
  - new fields: `--media` (csv, `md` alias), `--ingested-after` (RFC3339 UTC), `--doc-id`
- AND combinator across flags; OR within `--tag` and `--media`
- filter layer: SQLite WHERE for lexical (incl. media via `CASE WHEN json_type='text'` to handle both unit and tuple `MediaType` serde shapes), over-fetch + `filter_chunks` post-filter for vector
- wire shape unchanged — input-only feature; `search_response.v1` and `search_hit.v1` untouched
- invalid `--ingested-after` / unknown `trust_min` → `error.v1.code = config_invalid` (CLI) / `invalid_input` (MCP); unknown `--media` value → empty hits, no error

## Test plan

- [x] `cargo test --workspace --no-fail-fast -j 1` — green
- [x] `cargo clippy --workspace --all-targets -- -D warnings` — clean
- [x] new tests: 6 lexical (media / ingested_after / doc_id / AND / unknown / default), 2 vector mirror, 4 CLI integration, 2 MCP
- [x] manual smoke per `docs/SMOKE.md` "Filter args" walkthrough

## Architectural notes

- `SearchFilters` 3 fields are additive with `#[serde(default)]` — old JSON without the new keys deserializes cleanly.
- `MediaType` JSON has two shapes (`"markdown"` for unit variants, `{"image":"png"}` for tuple variants); the SQL `CASE WHEN json_type='text' THEN json_extract($) ELSE (first object key) END` extracts a unified kind string.
- Vector retriever mirrors the lexical SQL exactly (same WHERE clauses, same params binding pattern). path_glob remains a Rust post-filter — unchanged from before fb-36.
- No new HOTFIXES entry — additive minor, no contract drift.

## Files of interest

- spec: `docs/superpowers/specs/2026-05-10-p9-fb-36-search-filters-design.md`
- plan: `docs/superpowers/plans/2026-05-10-p9-fb-36-search-filters.md`
- core: `crates/kebab-core/src/search.rs` (SearchFilters)
- search: `crates/kebab-search/src/lexical.rs` + `vector.rs`
- CLI: `crates/kebab-cli/src/main.rs` (Cmd::Search)
- MCP: `crates/kebab-mcp/src/tools/search.rs` (SearchInput)
```

Open PR:

```bash
/Users/user/.claude/skills/gitea-ops/bin/gitea-pr \
  --title "feat(fb-36): search filter args (--media / --ingested-after / --doc-id + 4 existing)" \
  --body "$(cat /tmp/fb36-pr-body.md)" \
  --head feat/fb-36-search-filters \
  --base main
```

- [ ] **Step 9.5: Cleanup**

```bash
rm /tmp/fb36-pr-body.md
```

---

## Self-review

- **Spec coverage:**
  - §Behavior contract / 7 flags → Tasks 1, 4 (CLI), 6 (MCP)
  - §Filter validation (RFC3339, trust_min) → Task 4 (CLI dispatch), Task 6 (MCP dispatch)
  - §Filter layer (SQLite WHERE for lexical, over-fetch + post-filter for vector) → Tasks 2, 3
  - §Wire shape (input-only, no schema change) → no task needed; covered by absence of changes
  - §MCP `SearchInput` extension → Task 6
  - §Public surface delta (SearchFilters / TrustLevelFlag / SearchInput) → Tasks 1, 4, 6
  - §Test plan → Tasks 2 (6 lexical), 3 (2 vector), 5 (4 CLI), 6 (2 MCP)
  - §Documentation → Task 8
  - §Risks (MediaType JSON shape, RFC3339 UTC, path_glob ordering) → Task 2 explicitly handles the shape; Task 4 / 6 mention UTC; path_glob position unchanged

- **Placeholder scan:**
  - Task 2 / 3 / 6 contain "mirror the existing scaffold" instructions — concrete fallback paths spelled out (look at file X, copy pattern Y).
  - No "TODO" / "fill in" / "later" remaining.

- **Type consistency:**
  - `SearchFilters { tags_any, lang, path_glob, trust_min, media, ingested_after, doc_id }` consistent across Tasks 1, 2, 3, 4, 6.
  - `media: Vec<String>`, `ingested_after: Option<OffsetDateTime>`, `doc_id: Option<DocumentId>` consistent.
  - `MEDIA_KINDS` const used as documentation reference, not at runtime.
  - `TrustLevelFlag` clap enum → `kebab_core::TrustLevel` mapping defined in Task 4 step 4.1, used in Task 4 step 4.3.
  - Error codes consistent: `config_invalid` (CLI), `invalid_input` (MCP) — both via StructuredError.

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-05-10-p9-fb-36-search-filters.md`. Two execution options:

**1. Subagent-Driven (recommended)** — fresh subagent per task, review between tasks.

**2. Inline Execution** — execute tasks in this session.

Which approach?
