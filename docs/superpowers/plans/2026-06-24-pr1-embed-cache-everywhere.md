# PR1 — Embedding cache everywhere Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Route image/PDF/code chunk embedding through the existing `embed_with_cache` (content-hash derivation cache), exactly as markdown already does, with byte-identical output.

**Architecture:** `embed_with_cache` (`crates/kebab-app/src/ingest.rs:1010-1063`) already caches embeddings keyed on `derivation_cache_key("embedding", chunk_text, version_key)`. Markdown wires it at `ingest.rs:1378-1411`. The image/PDF/code handlers still call `emb.embed(&inputs)` directly. This PR replaces those three direct calls with the markdown pattern (verbatim), adding the deferred `derivation_cache_touch`. No change to `embed_with_cache`, no new namespace, no new config/flag/wire field.

**Tech Stack:** Rust 2024, `kebab-app` crate, `blake3` derivation keys, fastembed (deterministic ONNX) for the test embedder.

## Global Constraints (from spec §2 + CLAUDE.md)

- **Byte-identical ingest output (HARD GATE).** Fresh-dir ingest = every key misses = embeds exactly as today. Re-ingest = cache hit returns the same LE-f32 bytes. Vectors and `embedding_id` are unchanged. PR is byte-neutral by construction.
- **Single shared version_key, no per-media variation:** `format!("doc|{}|{}|{}", model_id.0, model_version.0, dimensions)`. Same chunk text → same key across all media (cross-media reuse). Do NOT introduce per-media variants.
- `embed_with_cache` and `derivation_payload` are **unchanged** by this PR.
- Build/clippy: `CARGO_TARGET_DIR=/home/user/large_data/out/kebab/target`, `cargo clippy --workspace --all-targets -- -D warnings` = 0.
- `kebab-parse-*` MUST NOT gain a `kebab-store-*` dep (not at risk here — all changes are in `kebab-app`).

## File Structure

- **Modify** `crates/kebab-app/src/ingest.rs` — three embed sites:
  - image `ingest_one_image_asset` (block `1734-1773`)
  - pdf `ingest_one_pdf_asset` (block `2373-2410`)
  - code `ingest_one_code_asset` (block `2703-2742`)
- **Create** `crates/kebab-app/tests/embed_cache_reingest.rs` — deterministic integration test (fastembed, `#[ignore]` AVX-gated lane) proving a code asset re-ingest is a cache hit and produces byte-identical vectors.

---

### Task 1: Wire `embed_with_cache` into image / PDF / code handlers

**Files:**
- Modify: `crates/kebab-app/src/ingest.rs` (image `1734-1773`, pdf `2373-2410`, code `2703-2742`)

**Interfaces:**
- Consumes: `embed_with_cache(emb: &dyn Embedder, sqlite: &SqliteStore, texts: &[&str], version_key: &str, hit: &mut usize, miss: &mut usize, touch_keys: &mut Vec<String>) -> anyhow::Result<Vec<Vec<f32>>>` (`ingest.rs:1010`); `app.sqlite.derivation_cache_touch(&[String]) -> Result<()>`.
- Produces: nothing new (internal wiring only).

The change is mechanically identical at all three sites: replace the `let inputs … emb.embed(&inputs)?` block with the cache pattern, keep the `records`/`upsert` block exactly as-is, and add a `derivation_cache_touch` immediately after `upsert`. The reference is markdown at `ingest.rs:1378-1411`.

- [ ] **Step 1: Image site (`ingest.rs:1737-1772`)** — replace:

```rust
        let inputs: Vec<EmbeddingInput<'_>> = chunks
            .iter()
            .map(|c| EmbeddingInput {
                text: c.text.as_str(),
                kind: EmbeddingKind::Document,
            })
            .collect();
        let vectors = emb
            .embed(&inputs)
            .context("Embedder::embed (image chunks)")?;
        let model_id = emb.model_id();
        let model_version = emb.model_version();
        let dimensions = emb.dimensions();
```

with (note `&**emb`, the shared `doc|…` version_key, and `body_texts`):

```rust
        let model_id = emb.model_id();
        let model_version = emb.model_version();
        let dimensions = emb.dimensions();
        // derivation cache(§3.4): same version_key formula + same code path as
        // the markdown handler (ingest.rs:1374). Media-agnostic — identical
        // chunk text shares one entry across media.
        let emb_version_key =
            format!("doc|{}|{}|{}", model_id.0, model_version.0, dimensions);
        let body_texts: Vec<&str> = chunks.iter().map(|c| c.text.as_str()).collect();
        let mut emb_cache_hit = 0_usize;
        let mut emb_cache_miss = 0_usize;
        let mut emb_touch_keys: Vec<String> = Vec::new();
        let vectors = embed_with_cache(
            &**emb,
            &app.sqlite,
            &body_texts,
            &emb_version_key,
            &mut emb_cache_hit,
            &mut emb_cache_miss,
            &mut emb_touch_keys,
        )
        .context("Embedder::embed (image chunks)")?;
```

Then, immediately AFTER the existing `vec_store.upsert(&records).context("VectorStore::upsert (image)")?;` (currently `1770-1772`), add:

```rust
        app.sqlite.derivation_cache_touch(&emb_touch_keys)?;
```

The `records` construction (`1750-1769`) and the `upsert` call are **unchanged**.

- [ ] **Step 2: PDF site (`ingest.rs:2376-2409`)** — same transform. Replace:

```rust
        let inputs: Vec<EmbeddingInput<'_>> = chunks
            .iter()
            .map(|c| EmbeddingInput {
                text: c.text.as_str(),
                kind: EmbeddingKind::Document,
            })
            .collect();
        let vectors = emb.embed(&inputs).context("Embedder::embed (pdf chunks)")?;
        let model_id = emb.model_id();
        let model_version = emb.model_version();
        let dimensions = emb.dimensions();
```

with:

```rust
        let model_id = emb.model_id();
        let model_version = emb.model_version();
        let dimensions = emb.dimensions();
        let emb_version_key =
            format!("doc|{}|{}|{}", model_id.0, model_version.0, dimensions);
        let body_texts: Vec<&str> = chunks.iter().map(|c| c.text.as_str()).collect();
        let mut emb_cache_hit = 0_usize;
        let mut emb_cache_miss = 0_usize;
        let mut emb_touch_keys: Vec<String> = Vec::new();
        let vectors = embed_with_cache(
            &**emb,
            &app.sqlite,
            &body_texts,
            &emb_version_key,
            &mut emb_cache_hit,
            &mut emb_cache_miss,
            &mut emb_touch_keys,
        )
        .context("Embedder::embed (pdf chunks)")?;
```

After the existing `vec_store.upsert(&records).context("VectorStore::upsert (pdf)")?;` (`2407-2409`), add:

```rust
        app.sqlite.derivation_cache_touch(&emb_touch_keys)?;
```

- [ ] **Step 3: Code site (`ingest.rs:2706-2741`)** — same transform. Replace:

```rust
        let inputs: Vec<EmbeddingInput<'_>> = chunks
            .iter()
            .map(|c| EmbeddingInput {
                text: c.text.as_str(),
                kind: EmbeddingKind::Document,
            })
            .collect();
        let vectors = emb
            .embed(&inputs)
            .context("Embedder::embed (code chunks)")?;
        let model_id = emb.model_id();
        let model_version = emb.model_version();
        let dimensions = emb.dimensions();
```

with:

```rust
        let model_id = emb.model_id();
        let model_version = emb.model_version();
        let dimensions = emb.dimensions();
        let emb_version_key =
            format!("doc|{}|{}|{}", model_id.0, model_version.0, dimensions);
        let body_texts: Vec<&str> = chunks.iter().map(|c| c.text.as_str()).collect();
        let mut emb_cache_hit = 0_usize;
        let mut emb_cache_miss = 0_usize;
        let mut emb_touch_keys: Vec<String> = Vec::new();
        let vectors = embed_with_cache(
            &**emb,
            &app.sqlite,
            &body_texts,
            &emb_version_key,
            &mut emb_cache_hit,
            &mut emb_cache_miss,
            &mut emb_touch_keys,
        )
        .context("Embedder::embed (code chunks)")?;
```

After the existing `vec_store.upsert(&records).context("VectorStore::upsert (code)")?;` (`2739-2741`), add:

```rust
        app.sqlite.derivation_cache_touch(&emb_touch_keys)?;
```

- [ ] **Step 4: Confirm `embed_with_cache` and `EmbeddingInput`/`EmbeddingKind` imports are in scope** — they are already used in this file (markdown path). No new `use` needed. `app` is the handler's `&App` param (already in scope at all three sites — `app.sqlite`, `app.config` are used nearby).

- [ ] **Step 5: Build + clippy**

Run: `CARGO_TARGET_DIR=/home/user/large_data/out/kebab/target cargo clippy -p kebab-app --all-targets -- -D warnings`
Expected: `Finished`, 0 warnings. (The `emb_cache_hit`/`emb_cache_miss` locals are written via `&mut` and not read afterward — same as the markdown path; this does not trip `unused_variables` or `unused_assignments`.)

- [ ] **Step 6: Existing tests still pass (no wiring regression)**

Run: `CARGO_TARGET_DIR=/home/user/large_data/out/kebab/target cargo test -p kebab-app`
Expected: same pass/fail set as before the change (default lane is `provider="none"`, so embedding is not exercised here — this step only proves nothing else broke).

- [ ] **Step 7: Commit**

```bash
git add crates/kebab-app/src/ingest.rs
git commit -m "feat(app): image/pdf/code 임베딩을 embed_with_cache 경유로 (markdown 패턴 통일)"
```

---

### Task 2: Deterministic re-ingest cache-hit test + verification

**Files:**
- Create: `crates/kebab-app/tests/embed_cache_reingest.rs`

**Interfaces:**
- Consumes: `common::TestEnv::with_embeddings()` (`crates/kebab-app/tests/common/mod.rs:39`, fastembed/deterministic, AVX-gated); `kebab_app::ingest_with_config(cfg, scope, opts)`; `SqliteStore` row count of the `derivation_cache` table; vector search via the app facade.
- Produces: nothing (test only).

This test proves the new wiring: a code asset embedded once populates the `"embedding"` derivation cache, and a forced re-ingest is a cache **hit** that yields byte-identical vectors. It is `#[ignore]` (AVX-gated, like the other `with_embeddings` tests) so the default no-AVX CI lane stays green; run it explicitly with `-- --ignored`.

- [ ] **Step 1: Write the test**

```rust
//! PR1: image/pdf/code embedding now flows through `embed_with_cache`.
//! Proves a code asset's second (forced) ingest is a derivation-cache HIT
//! with byte-identical vectors. AVX-gated (`#[ignore]`) like all embedding
//! tests — run with `cargo test -p kebab-app --test embed_cache_reingest -- --ignored`.

mod common;

use common::TestEnv;

/// Read the row count of the "embedding" derivation-cache namespace from the
/// test KB's SQLite file.
fn embedding_cache_rows(data_dir: &std::path::Path) -> i64 {
    let db = data_dir.join("kebab.sqlite");
    let conn = rusqlite::Connection::open(db).expect("open kebab.sqlite");
    conn.query_row(
        "SELECT COUNT(*) FROM derivation_cache WHERE kind = 'embedding'",
        [],
        |r| r.get(0),
    )
    .expect("count embedding cache rows")
}

#[test]
#[ignore = "requires AVX + fastembed model download"]
fn code_reingest_is_embedding_cache_hit_and_byte_identical() {
    let env = TestEnv::with_embeddings();
    let data_dir = std::path::PathBuf::from(&env.config.storage.data_dir);

    // Write a small Rust source file into the workspace so the code handler runs.
    let src = env.workspace_root.join("sample.rs");
    std::fs::write(
        &src,
        "pub fn add(a: i32, b: i32) -> i32 { a + b }\n\
         pub fn sub(a: i32, b: i32) -> i32 { a - b }\n",
    )
    .unwrap();

    // First ingest: cold cache → embeddings computed + cached.
    let opts1 = kebab_app::IngestOpts::default();
    kebab_app::ingest_with_config(env.config.clone(), env.scope(), opts1).expect("first ingest");
    let rows_after_first = embedding_cache_rows(&data_dir);
    assert!(
        rows_after_first > 0,
        "first ingest must populate the embedding derivation cache (got {rows_after_first})"
    );

    // Capture vector-search results after the first ingest. Vector mode
    // exercises the embeddings; identical vectors ⇒ identical scores.
    let hits_first = search_hits(&env.config);
    assert!(!hits_first.is_empty(), "first ingest produced no searchable vectors");

    // Second ingest with force_reingest: same source bytes + same versions →
    // every chunk text is a cache HIT, so no new cache rows, identical vectors.
    let opts2 = kebab_app::IngestOpts {
        force_reingest: true,
        ..Default::default()
    };
    kebab_app::ingest_with_config(env.config.clone(), env.scope(), opts2).expect("re-ingest");
    let rows_after_second = embedding_cache_rows(&data_dir);
    assert_eq!(
        rows_after_first, rows_after_second,
        "re-ingest must be a pure cache hit — no new embedding cache rows"
    );

    let hits_second = search_hits(&env.config);
    assert_eq!(
        hits_first, hits_second,
        "re-ingest must yield byte-identical vector-search results (cache hit ⇒ same vectors ⇒ same scores)"
    );
}

/// Vector-mode search results as `(chunk_id, score_bits)`, sorted, for a
/// byte-exact cross-ingest comparison. `score.to_bits()` makes the f32
/// comparison exact; identical embedding vectors produce identical scores.
/// SearchQuery construction mirrors `tests/search_vector.rs`.
fn search_hits(config: &kebab_config::Config) -> Vec<(String, u32)> {
    let q = kebab_core::SearchQuery {
        text: "add".to_string(),
        mode: kebab_core::SearchMode::Vector,
        k: 10,
        filters: kebab_core::SearchFilters::default(),
    };
    let mut hits = kebab_app::search_with_config(config.clone(), q)
        .expect("vector search")
        .into_iter()
        .map(|h| (h.chunk_id.0, h.score.to_bits()))
        .collect::<Vec<_>>();
    hits.sort();
    hits
}
```

- [ ] **Step 2: Run it to verify it fails on a deliberately broken wiring (sanity)**

Temporarily change the code-site `version_key` to `format!("BROKEN|{}|…")` in `ingest.rs`, then:
Run: `CARGO_TARGET_DIR=/home/user/large_data/out/kebab/target cargo test -p kebab-app --test embed_cache_reingest -- --ignored`
Expected: the `rows_after_first == rows_after_second` / identical-vectors assertion FAILS (broken key → re-ingest misses). Revert the deliberate break.

- [ ] **Step 3: Run it green on the real wiring**

Run: `CARGO_TARGET_DIR=/home/user/large_data/out/kebab/target cargo test -p kebab-app --test embed_cache_reingest -- --ignored`
Expected: PASS — `rows_after_first > 0`, `rows_after_first == rows_after_second`, vectors/search byte-identical.

- [ ] **Step 4: Full clippy gate**

Run: `CARGO_TARGET_DIR=/home/user/large_data/out/kebab/target cargo clippy --workspace --all-targets -- -D warnings`
Expected: `Finished`, 0 warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/kebab-app/tests/embed_cache_reingest.rs
git commit -m "test(app): code 재인덱싱 임베딩 캐시 히트 + byte-identical 검증 (AVX-gated)"
```

---

## Verification summary (PR-level HARD GATE)

- **Byte-neutral by construction** (spec §2): fresh-dir = all-miss = identical embeds; re-ingest = hit returns the same LE-f32 bytes. PR1 changes no first-ingest output.
- **Deterministic proof:** Task 2 (`#[ignore]`) shows code re-ingest is a cache hit with byte-identical vectors/search.
- **No markdown regression:** PR1 does not touch the markdown path, so the markdown parity gate (`gate-ingest.sh`, GPU-ollama) is trivially IDENTICAL — running it is optional belt-and-suspenders, not required for PR1.
- **clippy** `--workspace --all-targets` = 0.

## Notes for the PR

- Conventional-commit, trailer-free (repo convention). PR via gitea-ops; ask single-shot vs review-loop before creating.
- Version bump: PR1 is observability/perf only, no interface/output change → **patch** (defer the bump to the release/정리 step per CLAUDE.md; do not bump in this PR).
- PR2 (OCR/caption cache) gets its own plan after PR1 merges.
