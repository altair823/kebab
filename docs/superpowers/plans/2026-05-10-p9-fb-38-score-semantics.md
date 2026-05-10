# fb-38 Score Semantics Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `score_kind` field on `search_hit.v1` (`"rrf"` / `"bm25"` / `"cosine"`) and document RRF formula + score interpretation so agents stop misreading the top-level `score` as confidence.

**Architecture:** New `ScoreKind` enum on `kebab-core`. Each retriever (lexical / vector / hybrid) labels hits with the appropriate kind at construction. Wire serialization is automatic via existing `serde_json::to_value(&hit)`. Documentation in README + design + SKILL explains the RRF formula and the ranking-vs-confidence distinction.

**Tech Stack:** Rust 2024, serde, JSON Schema 2020-12.

**Spec:** `docs/superpowers/specs/2026-05-10-p9-fb-38-score-semantics-design.md`

---

## File map

**Create:** none.

**Modify:**
- `crates/kebab-core/src/search.rs` — add `ScoreKind` enum + `SearchHit.score_kind` field; update existing `SearchHit` test fixture.
- `crates/kebab-search/src/lexical.rs` — set `score_kind: Bm25` at hit construction.
- `crates/kebab-search/src/vector.rs` — set `score_kind: Cosine` at hit construction.
- `crates/kebab-search/src/hybrid.rs` — set `score_kind: Rrf` after RRF base.retrieval overwrite; update `mk_hit` test helper.
- `crates/kebab-rag/src/pipeline.rs` — update `mk_hit` test helper with `score_kind`.
- `crates/kebab-cli/tests/wire_search_response.rs` (or new) — integration test asserting `score_kind` on lexical / hybrid wire output.
- `docs/wire-schema/v1/search_hit.schema.json` — add optional `score_kind` enum field.
- `README.md` — new "Score interpretation (fb-38)" section.
- `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md` §4 — RRF formula + score_kind field block.
- `integrations/claude-code/kebab/SKILL.md` — `score_kind` mention + ranking-vs-confidence guidance.
- `tasks/p9/p9-fb-38-score-semantics.md` — flip status, add design + plan links.
- `tasks/INDEX.md` — flip fb-38 to ✅.

---

## Task 1: Add ScoreKind enum + SearchHit.score_kind field

**Files:**
- Modify: `crates/kebab-core/src/search.rs`

- [ ] **Step 1: Append failing tests to `mod tests`**

```rust
#[test]
fn score_kind_serde_roundtrip() {
    use ScoreKind::*;
    for (kind, expected) in [(Rrf, "rrf"), (Bm25, "bm25"), (Cosine, "cosine")] {
        let v = serde_json::to_value(kind).unwrap();
        assert_eq!(v.as_str(), Some(expected));
        let back: ScoreKind = serde_json::from_value(v).unwrap();
        assert_eq!(back, kind);
    }
}

#[test]
fn score_kind_default_is_rrf() {
    assert_eq!(ScoreKind::default(), ScoreKind::Rrf);
}

#[test]
fn search_hit_deserialize_without_score_kind_defaults_to_rrf() {
    // Old wire (pre-fb-38) shape — no `score_kind` field. Must
    // deserialize cleanly with `Rrf` default.
    let json = serde_json::json!({
        "rank": 1,
        "chunk_id": "c1",
        "doc_id": "d1",
        "doc_path": "a.md",
        "heading_path": [],
        "section_label": null,
        "snippet": "x",
        "citation": { "Line": { "path": "a.md", "start": 1, "end": 1, "section": null } },
        "retrieval": {
            "method": "Lexical",
            "fusion_score": 0.5,
            "lexical_score": 0.5,
            "vector_score": null,
            "lexical_rank": 1,
            "vector_rank": null
        },
        "index_version": "v1",
        "embedding_model": null,
        "chunker_version": "c1",
        "indexed_at": "2026-05-10T12:00:00Z",
        "stale": false
    });
    let hit: SearchHit = serde_json::from_value(json).unwrap();
    assert_eq!(hit.score_kind, ScoreKind::Rrf);
}
```

- [ ] **Step 2: Run tests to verify compile failures**

```bash
cargo test -p kebab-core --lib score_kind
```
Expected: errors — `ScoreKind` undefined; `SearchHit.score_kind` missing.

- [ ] **Step 3: Add `ScoreKind` enum + extend `SearchHit`**

In `crates/kebab-core/src/search.rs`, add the enum (place after `MEDIA_KINDS` constant, before `SearchQuery`):

```rust
/// p9-fb-38: top-level `SearchHit.score` declaration.
/// `Rrf` (hybrid) / `Bm25` (lexical-only) / `Cosine` (vector-only).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScoreKind {
    Rrf,
    Bm25,
    Cosine,
}

impl Default for ScoreKind {
    fn default() -> Self {
        ScoreKind::Rrf
    }
}
```

Extend `SearchHit` (add field after `stale`):

```rust
pub struct SearchHit {
    // ... existing fields ...
    pub stale: bool,
    /// p9-fb-38: declares the meaning of the top-level `score`.
    /// `Rrf` (hybrid mode), `Bm25` (lexical-only), `Cosine` (vector-only).
    /// Older wire (fb-38 미만) 부재 시 `Rrf` default — hybrid 가 기본 mode.
    #[serde(default)]
    pub score_kind: ScoreKind,
}
```

Update existing test fixture `search_hit_serializes_indexed_at_and_stale` (~line 190): add `score_kind: ScoreKind::Rrf,` to the struct literal.

- [ ] **Step 4: Run tests**

```bash
cargo test -p kebab-core --lib
```
Expected: all 3 new tests + existing tests pass.

- [ ] **Step 5: Re-export at crate root**

Edit `crates/kebab-core/src/lib.rs` re-export block — add `ScoreKind` to the `search::` re-export list.

```bash
grep -n "SearchHit\|SearchTrace\|TraceCandidate" crates/kebab-core/src/lib.rs
```

The fb-37 task added `SearchTrace`/`TraceCandidate`/`TraceFusionInput`/`TraceTiming`/`IndexBytes`/`MEDIA_KINDS` to the same export block — add `ScoreKind` next to them.

- [ ] **Step 6: Commit**

```bash
git add crates/kebab-core/src/search.rs crates/kebab-core/src/lib.rs
git commit -m "feat(core): ScoreKind enum + SearchHit.score_kind (fb-38)"
```

---

## Task 2: Label LexicalRetriever hits as Bm25

**Files:**
- Modify: `crates/kebab-search/src/lexical.rs`

- [ ] **Step 1: Add unit test in `crates/kebab-search/src/lexical.rs`**

Append to existing `mod tests` (find via `grep -n "mod tests" crates/kebab-search/src/lexical.rs`). If no tests module exists in that file, the integration tests in `tests/` cover behavior — add a unit test asserting via the public surface. Inspect first:

```bash
grep -n "mod tests\|#\[test\]" crates/kebab-search/src/lexical.rs | head -5
```

If no `mod tests` in lexical.rs, add a unit test in the existing integration test file (find via `ls crates/kebab-search/tests/`). Otherwise prepare an integration test that builds a lexical retriever against a real fixture and asserts on the hit's `score_kind`.

The simplest path: assert via the existing `lexical_*` integration tests. Pick the smallest one and add an assertion. Or, more cleanly, add a new integration test:

Append to `crates/kebab-search/tests/lexical_basic.rs` (or whichever existing lexical test file the workspace has — check `ls crates/kebab-search/tests/`):

```rust
#[test]
fn lexical_retriever_hits_carry_bm25_score_kind() {
    // Use the existing fixture-builder pattern from this file.
    // The intent: any hit returned by LexicalRetriever has
    // `score_kind == ScoreKind::Bm25`.
    let (_dir, retriever) = setup_lexical_with_corpus(&[
        ("a.md", "rust async tokens"),
    ]);
    let hits = retriever
        .search(&kebab_core::SearchQuery {
            text: "rust".into(),
            mode: kebab_core::SearchMode::Lexical,
            k: 5,
            filters: Default::default(),
        })
        .unwrap();
    assert!(!hits.is_empty());
    for h in &hits {
        assert_eq!(h.score_kind, kebab_core::ScoreKind::Bm25);
    }
}
```

`setup_lexical_with_corpus` is the existing fixture name — adjust to whatever the file's helper is called. If the file uses inline `tempfile::tempdir() + SqliteStore::open + ingest_with_config + LexicalRetriever::with_settings`, mirror that pattern.

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p kebab-search lexical_retriever_hits_carry_bm25_score_kind
```
Expected: compile error (struct literal needs new field) OR assertion failure (score_kind defaults to Rrf, not Bm25).

- [ ] **Step 3: Update `LexicalRetriever` hit construction**

In `crates/kebab-search/src/lexical.rs:447-471`, find the `Ok(SearchHit { ... })` block and add `score_kind: kebab_core::ScoreKind::Bm25,` (anywhere in the field list — placement doesn't matter for serde). Place it next to the `stale: false` line for visual grouping:

```rust
    Ok(SearchHit {
        rank,
        chunk_id: ChunkId(raw.chunk_id),
        // ... existing fields ...
        indexed_at,
        stale: false,
        score_kind: kebab_core::ScoreKind::Bm25,
    })
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p kebab-search
```
Expected: new test passes + all existing kebab-search tests still pass.

- [ ] **Step 5: Clippy**

```bash
cargo clippy -p kebab-search --all-targets -- -D warnings
```

- [ ] **Step 6: Commit**

```bash
git add crates/kebab-search/src/lexical.rs crates/kebab-search/tests/
git commit -m "feat(search/lexical): label hits with ScoreKind::Bm25 (fb-38)"
```

---

## Task 3: Label VectorRetriever hits as Cosine

**Files:**
- Modify: `crates/kebab-search/src/vector.rs`

- [ ] **Step 1: Add unit test**

VectorRetriever requires embeddings, so a real-corpus integration test isn't possible without a model. Add a unit test that constructs a `SearchHit` directly through whichever helper the file uses, OR adjust an existing vector test that already builds a retriever.

Inspect existing tests:
```bash
ls crates/kebab-search/tests/ | grep vector
grep -n "fn build_hit\|VectorRetriever" crates/kebab-search/src/vector.rs | head -5
```

If there's a private `build_hit` helper, write a unit test around it. Otherwise mirror the lexical test pattern but stub the embedder. Worst case: skip the unit test for VectorRetriever and rely on the hybrid test (Task 4) which exercises the vector path indirectly. Document in the commit message.

For simplicity, the recommended approach: add the score_kind line in Step 2 below first, then add a unit test using a simple hit-construction helper if accessible. If not accessible, the hybrid task (Task 4) covers behavior via the search_with_trace mode=Vector branch.

- [ ] **Step 2: Update `VectorRetriever` hit construction**

In `crates/kebab-search/src/vector.rs:304-330`, find `Ok(SearchHit { ... })` and add:

```rust
    Ok(SearchHit {
        rank,
        // ... existing fields ...
        indexed_at,
        stale: false,
        score_kind: kebab_core::ScoreKind::Cosine,
    })
```

- [ ] **Step 3: Run tests**

```bash
cargo test -p kebab-search
cargo clippy -p kebab-search --all-targets -- -D warnings
```
Expected: existing tests still pass; clippy clean.

- [ ] **Step 4: Commit**

```bash
git add crates/kebab-search/src/vector.rs
git commit -m "feat(search/vector): label hits with ScoreKind::Cosine (fb-38)"
```

---

## Task 4: Label HybridRetriever fuse hits as Rrf + update test helpers

**Files:**
- Modify: `crates/kebab-search/src/hybrid.rs`
- Modify: `crates/kebab-rag/src/pipeline.rs` (test helper)

- [ ] **Step 1: Add unit test in `crates/kebab-search/src/hybrid.rs` `mod tests`**

Append:

```rust
#[test]
fn hybrid_fuse_labels_hits_as_rrf() {
    // Reuse mk_hit / Stub from the existing tests in this file.
    use kebab_core::{ScoreKind, SearchMode, SearchQuery};
    use std::sync::Arc;

    struct Stub { hits: Vec<kebab_core::SearchHit> }
    impl Retriever for Stub {
        fn search(&self, _q: &SearchQuery) -> anyhow::Result<Vec<kebab_core::SearchHit>> {
            Ok(self.hits.clone())
        }
        fn index_version(&self) -> kebab_core::IndexVersion {
            kebab_core::IndexVersion("v1".into())
        }
    }

    let lex = Arc::new(Stub {
        hits: vec![mk_hit(1, "c1", 0.9, SearchMode::Lexical)],
    });
    let vec_r = Arc::new(Stub {
        hits: vec![mk_hit(1, "c1", 0.8, SearchMode::Vector)],
    });
    let hybrid = HybridRetriever::with_policy(
        lex,
        vec_r,
        FusionPolicy::Rrf { k_rrf: 60 },
        2,
    );
    let q = SearchQuery {
        text: "x".into(),
        mode: SearchMode::Hybrid,
        k: 1,
        filters: Default::default(),
    };
    let hits = hybrid.search(&q).unwrap();
    assert!(!hits.is_empty());
    assert_eq!(hits[0].score_kind, ScoreKind::Rrf);
}

#[test]
fn hybrid_search_with_trace_lexical_mode_passes_through_bm25() {
    use kebab_core::{ScoreKind, SearchMode, SearchQuery};
    use std::sync::Arc;

    struct Stub { hits: Vec<kebab_core::SearchHit> }
    impl Retriever for Stub {
        fn search(&self, _q: &SearchQuery) -> anyhow::Result<Vec<kebab_core::SearchHit>> {
            Ok(self.hits.clone())
        }
        fn index_version(&self) -> kebab_core::IndexVersion {
            kebab_core::IndexVersion("v1".into())
        }
    }

    let mut lex_hit = mk_hit(1, "c1", 0.5, SearchMode::Lexical);
    lex_hit.score_kind = ScoreKind::Bm25;
    let lex = Arc::new(Stub { hits: vec![lex_hit] });
    let vec_r = Arc::new(Stub { hits: vec![] });
    let hybrid = HybridRetriever::with_policy(
        lex,
        vec_r,
        FusionPolicy::Rrf { k_rrf: 60 },
        2,
    );
    let q = SearchQuery {
        text: "x".into(),
        mode: SearchMode::Lexical,
        k: 1,
        filters: Default::default(),
    };
    let (hits, _trace) = hybrid.search_with_trace(&q).unwrap();
    assert!(!hits.is_empty());
    // search_with_trace mode=Lexical passes through underlying hits.
    assert_eq!(hits[0].score_kind, ScoreKind::Bm25);
}
```

The existing `mk_hit` helper at `hybrid.rs:730` is in the same `mod tests` block — reachable.

- [ ] **Step 2: Run tests to verify failures**

```bash
cargo test -p kebab-search hybrid
```
Expected: compile errors (mk_hit doesn't set score_kind so the struct literal is incomplete; new tests assert wrong value).

- [ ] **Step 3: Update `mk_hit` test helper at `hybrid.rs:730`**

Find `fn mk_hit(rank: u32, chunk: &str, score: f32, mode: SearchMode) -> SearchHit` and add `score_kind` to the returned literal:

```rust
fn mk_hit(rank: u32, chunk: &str, score: f32, mode: SearchMode) -> SearchHit {
    SearchHit {
        // ... existing fields ...
        indexed_at: time::OffsetDateTime::UNIX_EPOCH,
        stale: false,
        score_kind: kebab_core::ScoreKind::Rrf,  // tests override per-mode
    }
}
```

- [ ] **Step 4: Update `hybrid.rs` fuse to set Rrf after retrieval overwrite**

Find `base.retrieval = RetrievalDetail { ... }` block (~line 302-314). Immediately AFTER that block (before `hits.push(base)`), add:

```rust
            base.score_kind = kebab_core::ScoreKind::Rrf;
            hits.push(base);
```

(`base` was cloned from a lex/vec hit that had `Bm25`/`Cosine`; the fuse output is RRF-scored so override.)

- [ ] **Step 5: Update `pipeline.rs` mk_hit test helper**

```bash
grep -n "fn mk_hit" crates/kebab-rag/src/pipeline.rs
```

At ~line 1092, the test helper builds a SearchHit. Add `score_kind: kebab_core::ScoreKind::Rrf,` to the literal (place after `stale`).

- [ ] **Step 6: Update `kebab-core` test fixture if any other SearchHit literal exists**

```bash
grep -rn "SearchHit {" crates/ --include="*.rs"
```

For each location, ensure the literal includes `score_kind`. The Task 1 update on `crates/kebab-core/src/search.rs:190` should already be done. Tasks 2/3 cover the lexical/vector retriever construction. Tasks 4 covers `mk_hit` helpers. If any other SearchHit literal turns up (e.g. fb-37 added some in tests), add `score_kind` there too.

- [ ] **Step 7: Run tests + clippy**

```bash
cargo test -p kebab-core -p kebab-search -p kebab-rag
cargo clippy -p kebab-core -p kebab-search -p kebab-rag --all-targets -- -D warnings
```
Expected: all green.

- [ ] **Step 8: Commit**

```bash
git add crates/kebab-search/src/hybrid.rs crates/kebab-rag/src/pipeline.rs
git commit -m "feat(search/hybrid): label fused hits with ScoreKind::Rrf (fb-38)"
```

---

## Task 5: Workspace tests + cross-crate cleanup for SearchHit literals

**Files:**
- Modify: any other crate file with `SearchHit {` literal that broke (e.g., `kebab-app`, `kebab-cli`, `kebab-mcp`, `kebab-tui` test fixtures).

- [ ] **Step 1: Find all broken sites**

```bash
cargo build --workspace 2>&1 | grep "missing field \`score_kind\`" | head -20
```

This reveals every spot. Common patterns:
- Test fixtures in `crates/kebab-cli/tests/wire_*.rs` that hand-build hits.
- Test helpers in `crates/kebab-app/tests/`.
- TUI test data in `crates/kebab-tui/tests/`.

For each: open the file, find the `SearchHit {` literal, add `score_kind: kebab_core::ScoreKind::Rrf,` (default for test fixtures unless the test specifically exercises lex/vec mode).

- [ ] **Step 2: Verify workspace builds**

```bash
cargo build --workspace 2>&1 | tail -5
```
Expected: clean.

- [ ] **Step 3: Run full workspace tests**

```bash
cargo test --workspace --no-fail-fast -j 1
cargo clippy --workspace --all-targets -- -D warnings
```
Expected: all green.

- [ ] **Step 4: Commit**

```bash
git add crates/
git commit -m "fix(fb-38): add score_kind to remaining SearchHit literals"
```

---

## Task 6: CLI integration test for score_kind

**Files:**
- Modify: `crates/kebab-cli/tests/wire_search_response.rs` (or new file `wire_search_score_kind.rs` if appending feels cluttered)

- [ ] **Step 1: Inspect existing wire test pattern**

```bash
ls crates/kebab-cli/tests/
head -50 crates/kebab-cli/tests/wire_search_response.rs
```

Use the same fixture pattern from fb-37's `wire_search_trace.rs` (`common::write_config + ingest + run_search_with_args`).

- [ ] **Step 2: Add integration tests**

Create `crates/kebab-cli/tests/wire_search_score_kind.rs`:

```rust
//! p9-fb-38: integration tests for `search_hit.v1.score_kind`.

mod common;

use serde_json::Value;
use std::fs;

fn doc_with_term(workspace: &std::path::Path) {
    fs::write(workspace.join("doc1.md"), "# Title\n\nrust async hello\n").unwrap();
}

#[test]
fn lexical_mode_hits_carry_bm25_score_kind() {
    let dir = tempfile::tempdir().unwrap();
    let (cfg, workspace, _data) = common::write_config(dir.path(), 0);
    doc_with_term(&workspace);
    common::ingest(&cfg, &workspace);

    let (stdout, _stderr) = common::run_search_with_args(
        &cfg,
        &["--mode", "lexical", "--json", "rust"],
    );
    let v: Value = serde_json::from_str(stdout.trim()).expect("valid JSON");
    let hits = v["hits"].as_array().expect("hits array");
    assert!(!hits.is_empty(), "expected at least 1 hit");
    for h in hits {
        assert_eq!(h["score_kind"], "bm25");
    }
}

#[test]
fn old_wire_reader_compat_score_kind_optional_field() {
    // The wire schema marks `score_kind` as additive (not required).
    // We can't easily simulate an old reader from inside Rust, but we
    // can confirm the JSON includes the field — old readers that
    // ignore unknown fields are unaffected. This test just ensures
    // the field is always present in fb-38+ output.
    let dir = tempfile::tempdir().unwrap();
    let (cfg, workspace, _data) = common::write_config(dir.path(), 0);
    doc_with_term(&workspace);
    common::ingest(&cfg, &workspace);

    let (stdout, _stderr) = common::run_search_with_args(
        &cfg,
        &["--mode", "lexical", "--json", "rust"],
    );
    let v: Value = serde_json::from_str(stdout.trim()).unwrap();
    let hit = &v["hits"][0];
    assert!(hit.get("score_kind").is_some(), "score_kind always emitted");
}
```

- [ ] **Step 3: Run integration tests**

```bash
cargo test -p kebab-cli --test wire_search_score_kind
```
Expected: 2 tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/kebab-cli/tests/wire_search_score_kind.rs
git commit -m "test(cli): integration tests for score_kind on lexical mode (fb-38)"
```

---

## Task 7: Wire schema + docs + status flip

**Files:**
- Modify: `docs/wire-schema/v1/search_hit.schema.json`
- Modify: `README.md`
- Modify: `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md`
- Modify: `integrations/claude-code/kebab/SKILL.md`
- Modify: `tasks/p9/p9-fb-38-score-semantics.md`
- Modify: `tasks/INDEX.md`

- [ ] **Step 1: Update `docs/wire-schema/v1/search_hit.schema.json`**

Add `score_kind` to `properties` (not to `required`). Insert next to `score`:

```json
    "score_kind": {
      "type": "string",
      "enum": ["rrf", "bm25", "cosine"],
      "description": "p9-fb-38: kind of `score` value. `rrf` = RRF normalized [0,1] (hybrid mode); `bm25` = raw BM25 score (lexical-only); `cosine` = raw cosine similarity (vector-only). Older clients that omit this field can treat absence as `rrf` (the historical default)."
    }
```

- [ ] **Step 2: Update `README.md`**

Find the `kebab search` section (or wherever flag descriptions live). Add a new "Score interpretation (fb-38)" subsection:

````markdown
### Score 해석 (fb-38)

`search_hit.v1.score` 는 **ranking signal** 이지 confidence 가 아니다. `score_kind` 필드로 의미 선언:

| `score_kind` | 의미 | 범위 |
|--------------|------|------|
| `rrf` (hybrid) | RRF normalized | `[0, 1]`, ceiling = 1.0 (양 채널 rank=1) |
| `bm25` (lexical) | raw BM25 | unbounded (≥ 0) |
| `cosine` (vector) | cosine sim | `[-1, 1]` |

#### RRF 수식 (hybrid mode)

```
chunk c 의 raw RRF = Σ_m  1 / (k_rrf + rank_m(c))

여기서 m ∈ {lexical, vector}, k_rrf = config.search.rrf_k (default 60).
양 채널 모두 rank=1 일 때 raw RRF = 2 / (k_rrf + 1) ≈ 0.0328.

normalize: rrf_score = raw_rrf / (2 / (k_rrf + 1))
       → rrf_score ∈ [0, 1]. 양쪽 rank=1 → 1.0, 한 쪽만 등장 → ≈ 0.5 천장.
```

`rrf_score = 0.5` 의 의미: chunk 가 한 채널 (lexical 또는 vector) 에서만 rank 1 로 등장. confidence 50% 가 아님 — RRF 수식의 산술적 천장.

agent 가 trust threshold 가 필요하면 top-level `score` 가 아닌 nested `retrieval.lexical_score` (BM25 raw) / `retrieval.vector_score` (cosine raw) 사용.
````

Place after the `kebab search` flag table or wherever similar reference content lives. If the README has existing `kebab search` row in a command table, add a `--trace` neighbor cross-reference here.

- [ ] **Step 3: Update `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md` §4 search**

Add a new "Score scale (fb-38)" subsection under §4 with the same RRF formula block + `score_kind` field definition. The frozen design doc gets the contract; README is the user-facing copy.

```bash
grep -n "^## §4\|^### §4\|RRF\|hybrid_fusion" docs/superpowers/specs/2026-04-27-kebab-final-form-design.md | head -10
```

Locate the §4 search section and append the score scale block.

- [ ] **Step 4: Update `integrations/claude-code/kebab/SKILL.md`**

Find the `mcp__kebab__search` response shape block. Add a sentence:

> `hits[].score_kind`: `"rrf"` (hybrid) / `"bm25"` (lexical) / `"cosine"` (vector). top-level `score` 의 의미 선언 — confidence 아님. trust threshold 가 필요하면 `retrieval.lexical_score` / `retrieval.vector_score` (raw) 사용.

- [ ] **Step 5: Update `tasks/p9/p9-fb-38-score-semantics.md`**

Flip frontmatter `status: open` → `status: completed`. Replace the skeleton banner with:

```markdown
> ✅ **구현 완료.** 본 spec 은 구현 시점의 frozen 상태.
>
> - Design: [`docs/superpowers/specs/2026-05-10-p9-fb-38-score-semantics-design.md`](../../docs/superpowers/specs/2026-05-10-p9-fb-38-score-semantics-design.md)
> - Plan: [`docs/superpowers/plans/2026-05-10-p9-fb-38-score-semantics.md`](../../docs/superpowers/plans/2026-05-10-p9-fb-38-score-semantics.md)
```

- [ ] **Step 6: Update `tasks/INDEX.md`**

Find the fb-38 row. Flip status to ✅, mirror format of fb-32..37 rows.

- [ ] **Step 7: Run full workspace tests + clippy**

```bash
cargo test --workspace --no-fail-fast -j 1
cargo clippy --workspace --all-targets -- -D warnings
```
Expected: all green.

- [ ] **Step 8: Commit**

```bash
git add docs/ README.md tasks/p9/p9-fb-38-score-semantics.md tasks/INDEX.md integrations/claude-code/kebab/SKILL.md
git commit -m "docs(fb-38): wire schema + README + design + SKILL + INDEX"
```

---

## Final verification checklist

- [ ] `cargo test --workspace --no-fail-fast -j 1` green
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` clean
- [ ] Manual smoke against `/tmp/kebab-smoke`:
  - [ ] `kebab search Q --mode lexical --json | jq '.hits[0].score_kind'` returns `"bm25"`
  - [ ] `kebab search Q --json | jq '.hits[0].score_kind'` returns `"rrf"` (hybrid default)
- [ ] README, design §4, SKILL, INDEX all reflect score_kind + RRF formula
