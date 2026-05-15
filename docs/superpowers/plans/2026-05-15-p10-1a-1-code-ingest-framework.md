# p10-1A-1 Code Ingest Framework Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land the framework surface for code ingest — wire schema (`Citation::Code` variant, `SearchHit.repo` / `code_lang` fields, `IngestReport` skip counters), new CLI filter flags (`--media code` / `--code-lang` / `--repo`), `.gitignore` honor + built-in safety-net blacklist + generated-header sniff + size cap, `kebab-parse-code` crate skeleton (no per-language parsers), `[ingest.code]` config section — **without enabling any code chunker yet**. 1A-2 plugs in the Rust AST chunker on top of this framework.

**Architecture:** All changes are additive minor at the wire layer (no breaking change). Domain types in `kebab-core` get new variants / optional fields. The new `kebab-parse-code` crate ships with infrastructure modules (`lang.rs`, `repo.rs`, `skip.rs`) but no per-language parser modules — those land in 1A-2. The walker (`kebab-source-fs`) integrates `.gitignore` honor + built-in blacklist + generated header sniff + size cap, surfacing new skip counters in `IngestReport`. CLI filter flags wire through `SearchFilters` to the existing retriever stack. After 1A-1 merges, ingesting the existing markdown corpus produces byte-level identical wire output (verified by regression test).

**Tech Stack:** Rust 2024, serde, anyhow, `ignore` crate (already present), `gix` (new dep — for repo detect), JSON Schema 2020-12.

**Spec:** `docs/superpowers/specs/2026-05-15-kebab-code-ingest-design.md`

---

## File map

**Create:**
- `crates/kebab-parse-code/Cargo.toml` — new crate manifest.
- `crates/kebab-parse-code/src/lib.rs` — public surface (re-export `lang` / `repo` / `skip` items).
- `crates/kebab-parse-code/src/lang.rs` — `code_lang_for_path()` extension dispatcher.
- `crates/kebab-parse-code/src/repo.rs` — `detect_repo()` via `gix`.
- `crates/kebab-parse-code/src/skip.rs` — `is_generated_file()` + `is_oversized()` helpers + built-in blacklist patterns.
- `crates/kebab-parse-code/tests/lang.rs` — `code_lang_for_path` test fixture.
- `crates/kebab-parse-code/tests/repo.rs` — `detect_repo` test fixture (uses `gix::init` for temp repo).
- `crates/kebab-parse-code/tests/skip.rs` — `is_generated_file` + `is_oversized` test fixture.
- `tasks/p10/INDEX.md` — phase 10 task index.
- `tasks/p10/p10-1a-1-code-ingest-framework.md` — task spec for this PR.

**Modify:**
- `Cargo.toml` (workspace root) — register `crates/kebab-parse-code` in `members`, register `gix` in workspace dependencies.
- `crates/kebab-core/src/citation.rs` — add `Citation::Code { path, line_start, line_end, symbol, lang }` variant + `to_uri()` arm + `path()` arm.
- `crates/kebab-core/src/search.rs` — add `SearchHit.repo: Option<String>` + `SearchHit.code_lang: Option<String>` (both `#[serde(default, skip_serializing_if = "Option::is_none")]`) and extend `SearchFilters` with `repo: Vec<String>` + `code_lang: Vec<String>`.
- `crates/kebab-core/src/ingest.rs` — add `IngestReport.skipped_gitignore: u32` + `skipped_kebabignore: u32` + `skipped_builtin_blacklist: u32` + `skipped_generated: u32` + `skipped_size_exceeded: u32` + `skip_examples: SkipExamples` (new struct), and a `MediaKind::Code` arm hint (`metadata.code_lang` placeholder is on `Metadata`, not `IngestItem`, so no IngestItem field change needed).
- `crates/kebab-core/src/metadata.rs` — add `Metadata.repo: Option<String>` + `Metadata.git_branch: Option<String>` + `Metadata.git_commit: Option<String>` + `Metadata.code_lang: Option<String>`.
- `crates/kebab-core/src/lib.rs` — re-export new structs.
- `crates/kebab-source-fs/src/walker.rs` — extend `build_overrides()` to also walk repo-local `.gitignore` cascade and append built-in safety-net patterns (5 entries).
- `crates/kebab-source-fs/src/lib.rs` — surface new skip counters via the connector return.
- `crates/kebab-source-fs/src/connector.rs` — wire skip counters into the per-file decision (call `kebab_parse_code::skip` helpers when relevant).
- `crates/kebab-source-fs/Cargo.toml` — add `kebab-parse-code` dep (for `skip` + `repo` helpers).
- `crates/kebab-app/src/lib.rs` — register no new modules (1A-1 is infra only); thread new skip counters through the ingest reporter.
- `crates/kebab-app/src/schema.rs` — extend `SchemaStats` with `code_lang_breakdown: BTreeMap<String, u32>` and `repo_breakdown: BTreeMap<String, u32>` (default-empty until 1A-2 produces code chunks).
- `crates/kebab-config/src/lib.rs` — add `IngestCodeCfg` struct and embed it in `IngestCfg` (or in `Config` directly if `IngestCfg` doesn't exist yet — verify path).
- `crates/kebab-cli/src/main.rs` — add `--repo` (Vec) + `--code-lang` (Vec) to `Cmd::Search`. `--media code` is automatically accepted since `--media` is already free-form Vec<String>.
- `crates/kebab-cli/src/wire.rs` — propagate `repo` / `code_lang` fields into `wire_search_hit` output.
- `docs/wire-schema/v1/citation.schema.json` — add `code` to the `kind` enum + add `"code": { "type": "object" }` to top-level properties.
- `docs/wire-schema/v1/search_hit.schema.json` — add `repo` and `code_lang` to top-level properties (optional).
- `docs/wire-schema/v1/ingest_report.schema.json` — add five new skip counters + `skip_examples` to top-level properties.
- `docs/wire-schema/v1/schema.schema.json` — add `code_lang_breakdown` and `repo_breakdown` under `stats`.
- `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md` — apply §10.1 of the code ingest spec (Citation 6 variants, SearchHit fields, etc.).
- `README.md` — add `--media code` / `--code-lang` / `--repo` filter rows; mention `[ingest.code]` config block; note `.gitignore` honor.
- `HANDOFF.md` — add Phase 10 row (in-progress).
- `docs/SMOKE.md` — update example config to include `[ingest.code]` block with defaults.
- `tasks/INDEX.md` — add phase 10 entry.

**Test (regression):**
- `crates/kebab-cli/tests/wire_search_hit_no_code_fields.rs` — confirms markdown corpus hits omit `repo` / `code_lang` from JSON output (Option::None → absent).
- `crates/kebab-cli/tests/wire_citation_5_variants_unchanged.rs` — confirms existing 5 Citation variants serialize byte-identical (no spurious `code` key).
- `crates/kebab-app/tests/ingest_report_skip_counters_zero.rs` — confirms a markdown-only corpus reports `skipped_generated = 0` etc.

---

## Task 1: `Citation::Code` variant in `kebab-core`

**Files:**
- Modify: `crates/kebab-core/src/citation.rs`
- Modify: `crates/kebab-core/src/lib.rs` (re-export not needed — already `pub use`)

- [ ] **Step 1: Append failing test to `crates/kebab-core/src/citation.rs`'s `mod tests`**

```rust
#[test]
fn citation_code_variant_serializes_with_kind_tag() {
    let c = Citation::Code {
        path: WorkspacePath("crates/kebab-chunk/src/md_heading_v1.rs".into()),
        line_start: 142,
        line_end: 168,
        symbol: Some("MdHeadingV1Chunker::chunk_doc".into()),
        lang: Some("rust".into()),
    };
    let v = serde_json::to_value(&c).unwrap();
    assert_eq!(v["kind"], "code");
    assert_eq!(v["line_start"], 142);
    assert_eq!(v["line_end"], 168);
    assert_eq!(v["symbol"], "MdHeadingV1Chunker::chunk_doc");
    assert_eq!(v["lang"], "rust");
    // Existing 5 variants must NOT pick up these fields.
    let line = Citation::Line {
        path: WorkspacePath("notes/foo.md".into()),
        start: 1,
        end: 10,
        section: None,
    };
    let lv = serde_json::to_value(&line).unwrap();
    assert!(lv.get("line_start").is_none());
    assert!(lv.get("symbol").is_none());
}

#[test]
fn citation_code_uri_format() {
    let c = Citation::Code {
        path: WorkspacePath("a/b.rs".into()),
        line_start: 10,
        line_end: 20,
        symbol: None,
        lang: Some("rust".into()),
    };
    assert_eq!(c.to_uri(), "a/b.rs#L10-L20");
    // Single-line uses `#L10`.
    let single = Citation::Code {
        path: WorkspacePath("a/b.rs".into()),
        line_start: 5,
        line_end: 5,
        symbol: None,
        lang: None,
    };
    assert_eq!(single.to_uri(), "a/b.rs#L5");
}

#[test]
fn citation_code_path_accessor() {
    let c = Citation::Code {
        path: WorkspacePath("x.rs".into()),
        line_start: 1,
        line_end: 1,
        symbol: None,
        lang: None,
    };
    assert_eq!(c.path().0, "x.rs");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p kebab-core --lib citation_code -- --nocapture`
Expected: FAIL — `Citation::Code` variant does not exist.

- [ ] **Step 3: Add the `Code` variant to the `Citation` enum**

Insert after the `Time` variant in `crates/kebab-core/src/citation.rs`:

```rust
    Code {
        path: WorkspacePath,
        line_start: u32,
        line_end: u32,
        symbol: Option<String>,
        lang: Option<String>,
    },
```

- [ ] **Step 4: Extend the `path()` arm**

```rust
            Citation::Line { path, .. }
            | Citation::Page { path, .. }
            | Citation::Region { path, .. }
            | Citation::Caption { path, .. }
            | Citation::Time { path, .. }
            | Citation::Code { path, .. } => path,
```

- [ ] **Step 5: Extend the `to_uri()` arm**

```rust
            Citation::Code { path, line_start, line_end, .. } => {
                if line_start == line_end {
                    format!("{}#L{}", path.0, line_start)
                } else {
                    format!("{}#L{}-L{}", path.0, line_start, line_end)
                }
            }
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p kebab-core --lib citation_code -- --nocapture`
Expected: PASS (3 new tests).

- [ ] **Step 7: Run full `kebab-core` test suite to catch fall-out**

Run: `cargo test -p kebab-core --lib`
Expected: All tests pass. If a `match` somewhere errors with non-exhaustive, fix the missing arm (likely in `path()` / `to_uri()` already covered).

- [ ] **Step 8: Commit**

```bash
git add crates/kebab-core/src/citation.rs
git commit -m "feat(p10-1a-1): add Citation::Code variant"
```

---

## Task 2: `SearchHit.repo` / `code_lang` + `SearchFilters.repo` / `code_lang`

**Files:**
- Modify: `crates/kebab-core/src/search.rs`

- [ ] **Step 1: Append failing tests to `mod tests`**

```rust
#[test]
fn search_hit_repo_and_code_lang_are_optional_and_omit_when_none() {
    let hit = SearchHit {
        rank: 1,
        chunk_id: ChunkId("c1".into()),
        doc_id: DocumentId("d1".into()),
        doc_path: WorkspacePath("a.md".into()),
        heading_path: vec![],
        section_label: None,
        snippet: "".into(),
        citation: Citation::Line {
            path: WorkspacePath("a.md".into()),
            start: 1,
            end: 2,
            section: None,
        },
        retrieval: RetrievalDetail::default(),
        index_version: IndexVersion("v1".into()),
        embedding_model: None,
        chunker_version: ChunkerVersion("md-heading-v1".into()),
        indexed_at: time::OffsetDateTime::UNIX_EPOCH,
        stale: false,
        score_kind: ScoreKind::Rrf,
        repo: None,
        code_lang: None,
    };
    let v = serde_json::to_value(&hit).unwrap();
    assert!(v.get("repo").is_none(), "repo should be omitted when None");
    assert!(v.get("code_lang").is_none(), "code_lang should be omitted when None");
}

#[test]
fn search_hit_repo_and_code_lang_present_when_some() {
    let hit = SearchHit {
        rank: 1,
        chunk_id: ChunkId("c1".into()),
        doc_id: DocumentId("d1".into()),
        doc_path: WorkspacePath("a.rs".into()),
        heading_path: vec![],
        section_label: None,
        snippet: "".into(),
        citation: Citation::Code {
            path: WorkspacePath("a.rs".into()),
            line_start: 1,
            line_end: 2,
            symbol: None,
            lang: Some("rust".into()),
        },
        retrieval: RetrievalDetail::default(),
        index_version: IndexVersion("v1".into()),
        embedding_model: None,
        chunker_version: ChunkerVersion("code-rust-ast-v1".into()),
        indexed_at: time::OffsetDateTime::UNIX_EPOCH,
        stale: false,
        score_kind: ScoreKind::Rrf,
        repo: Some("kebab".into()),
        code_lang: Some("rust".into()),
    };
    let v = serde_json::to_value(&hit).unwrap();
    assert_eq!(v["repo"], "kebab");
    assert_eq!(v["code_lang"], "rust");
}

#[test]
fn search_filters_repo_and_code_lang_default_to_empty_vec() {
    let f = SearchFilters::default();
    assert!(f.repo.is_empty());
    assert!(f.code_lang.is_empty());
}
```

If `RetrievalDetail::default()` doesn't exist yet, derive it with `#[derive(Default)]` on the struct (it has only primitive Option / Vec fields — Default is trivially derivable).

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p kebab-core --lib search -- --nocapture`
Expected: FAIL with "no field `repo` on type `SearchHit`".

- [ ] **Step 3: Add the two fields to `SearchHit`**

In `crates/kebab-core/src/search.rs`, in the `SearchHit` struct, append after `score_kind`:

```rust
    /// p10-1A-1: optional. Filled when the source file lives in a git repo
    /// (`.git/` walk-up). null for markdown / pdf / image hits and for code
    /// hits ingested via `kebab ingest-file` outside a repo boundary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,

    /// p10-1A-1: optional. Programming language identifier (lowercase). Set for
    /// every code/manifest/k8s chunk; null for markdown / pdf / image hits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_lang: Option<String>,
```

- [ ] **Step 4: Extend `SearchFilters`**

Append after `doc_id`:

```rust
    /// p10-1A-1: filter by `metadata.repo`. Empty = no filter; multi-value = OR.
    #[serde(default)]
    pub repo: Vec<String>,

    /// p10-1A-1: filter by `metadata.code_lang`. Empty = no filter; multi-value = OR.
    /// Identifiers are lowercase canonical names (`rust`, `python`, `typescript`, ...).
    /// Unknown values produce empty hits (consistent with `media` policy).
    #[serde(default)]
    pub code_lang: Vec<String>,
```

- [ ] **Step 5: If `RetrievalDetail` doesn't derive Default, add it**

```rust
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RetrievalDetail {
    ...
}
```

- [ ] **Step 6: Run tests**

Run: `cargo test -p kebab-core --lib search -- --nocapture`
Expected: PASS (3 new tests).

- [ ] **Step 7: Build the whole workspace to find consumers that need to construct SearchHit**

Run: `cargo build --workspace`
Expected: A handful of test files and call sites need `repo: None, code_lang: None` appended. Patch each. Common sites:
- `crates/kebab-search/src/...` — wherever `SearchHit` is constructed by the retriever
- `crates/kebab-app/tests/...` — integration test fixtures

When patching, only add the two `None` lines; do not alter other field values.

- [ ] **Step 8: Run full workspace test (one crate at a time per CLAUDE.md)**

Run: `cargo test -p kebab-core && cargo test -p kebab-search && cargo test -p kebab-app && cargo test -p kebab-cli`
Expected: PASS across all four.

- [ ] **Step 9: Commit**

```bash
git add crates/kebab-core/src/search.rs
# include any consumer files that needed the two None fields
git commit -m "feat(p10-1a-1): add SearchHit.repo / code_lang + SearchFilters.repo / code_lang"
```

---

## Task 3: `IngestReport` skip counters + `SkipExamples`

**Files:**
- Modify: `crates/kebab-core/src/ingest.rs`

- [ ] **Step 1: Append failing test**

```rust
#[test]
fn skip_examples_default_is_empty() {
    let s = SkipExamples::default();
    assert!(s.generated.is_empty());
    assert!(s.size_exceeded.is_empty());
    assert!(s.builtin_blacklist.is_empty());
    assert!(s.gitignore.is_empty());
}

#[test]
fn ingest_report_skip_counters_serialize() {
    let r = IngestReport {
        scope: SourceScope::Workspace,
        scanned: 100,
        new: 50,
        updated: 0,
        skipped: 0,
        unchanged: 0,
        errors: 0,
        duration_ms: 1234,
        skipped_by_extension: Default::default(),
        skipped_gitignore: 30,
        skipped_kebabignore: 5,
        skipped_builtin_blacklist: 10,
        skipped_generated: 3,
        skipped_size_exceeded: 2,
        skip_examples: SkipExamples {
            generated: vec!["a/b.pb.rs".into()],
            size_exceeded: vec![],
            builtin_blacklist: vec!["node_modules/x.js".into()],
            gitignore: vec![],
        },
        items: None,
    };
    let v = serde_json::to_value(&r).unwrap();
    assert_eq!(v["skipped_gitignore"], 30);
    assert_eq!(v["skipped_builtin_blacklist"], 10);
    assert_eq!(v["skipped_generated"], 3);
    assert_eq!(v["skipped_size_exceeded"], 2);
    assert_eq!(v["skip_examples"]["generated"][0], "a/b.pb.rs");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p kebab-core --lib skip_examples -- --nocapture`
Expected: FAIL.

- [ ] **Step 3: Add `SkipExamples` struct**

In `crates/kebab-core/src/ingest.rs`, after `IngestReport`:

```rust
/// p10-1A-1: per-category sample of skipped file paths. Each category caps at
/// 5 entries (oldest-first). Used for debugging "why was X not indexed?"
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SkipExamples {
    #[serde(default)]
    pub generated: Vec<String>,
    #[serde(default)]
    pub size_exceeded: Vec<String>,
    #[serde(default)]
    pub builtin_blacklist: Vec<String>,
    #[serde(default)]
    pub gitignore: Vec<String>,
}
```

- [ ] **Step 4: Add the five new counters + `skip_examples` field to `IngestReport`**

After `skipped_by_extension`:

```rust
    /// p10-1A-1: files skipped because they matched a repo-local `.gitignore`.
    #[serde(default)]
    pub skipped_gitignore: u32,

    /// p10-1A-1: files skipped because they matched a `.kebabignore` entry.
    #[serde(default)]
    pub skipped_kebabignore: u32,

    /// p10-1A-1: files skipped because they matched the built-in safety-net
    /// blacklist (`node_modules/`, `target/`, `__pycache__/`, `.venv/`,
    /// `venv/`, `env/`).
    #[serde(default)]
    pub skipped_builtin_blacklist: u32,

    /// p10-1A-1: files skipped because their first ~512 bytes contained a
    /// generated-file marker (`@generated`, `do not edit`, …).
    #[serde(default)]
    pub skipped_generated: u32,

    /// p10-1A-1: files skipped because they exceeded `max_file_bytes` or
    /// `max_file_lines` in `[ingest.code]`.
    #[serde(default)]
    pub skipped_size_exceeded: u32,

    /// p10-1A-1: sample file paths per skip category (≤ 5 each).
    #[serde(default)]
    pub skip_examples: SkipExamples,
```

- [ ] **Step 5: Run test**

Run: `cargo test -p kebab-core --lib skip_examples -- --nocapture`
Expected: PASS.

- [ ] **Step 6: Build workspace to find consumers constructing IngestReport**

Run: `cargo build --workspace`
Expected: Patch sites that construct `IngestReport` to add the new fields (use `..Default::default()` style if a `Default` impl exists; otherwise spell out zeros). Typical consumers: `kebab-source-fs` connector, `kebab-app/src/lib.rs` ingest reporter.

- [ ] **Step 7: Run test suites**

Run: `cargo test -p kebab-core && cargo test -p kebab-source-fs && cargo test -p kebab-app`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add crates/kebab-core/src/ingest.rs
# include consumer files patched in step 6
git commit -m "feat(p10-1a-1): add IngestReport skip counters + SkipExamples"
```

---

## Task 4: `Metadata` extension — `repo` / `git_branch` / `git_commit` / `code_lang`

**Files:**
- Modify: `crates/kebab-core/src/metadata.rs`

- [ ] **Step 1: Append failing test**

```rust
#[test]
fn metadata_repo_fields_default_to_none_and_omit_when_serialized() {
    let m = Metadata {
        aliases: vec![],
        tags: vec![],
        created_at: time::OffsetDateTime::UNIX_EPOCH,
        updated_at: time::OffsetDateTime::UNIX_EPOCH,
        source_type: SourceType::Markdown,
        trust_level: TrustLevel::Primary,
        user_id_alias: None,
        user: Default::default(),
        repo: None,
        git_branch: None,
        git_commit: None,
        code_lang: None,
    };
    let v = serde_json::to_value(&m).unwrap();
    assert!(v.get("repo").is_none());
    assert!(v.get("git_branch").is_none());
    assert!(v.get("git_commit").is_none());
    assert!(v.get("code_lang").is_none());
}

#[test]
fn metadata_repo_fields_present_when_some() {
    let m = Metadata {
        aliases: vec![],
        tags: vec![],
        created_at: time::OffsetDateTime::UNIX_EPOCH,
        updated_at: time::OffsetDateTime::UNIX_EPOCH,
        source_type: SourceType::Markdown,
        trust_level: TrustLevel::Primary,
        user_id_alias: None,
        user: Default::default(),
        repo: Some("kebab".into()),
        git_branch: Some("main".into()),
        git_commit: Some("a".repeat(40)),
        code_lang: Some("rust".into()),
    };
    let v = serde_json::to_value(&m).unwrap();
    assert_eq!(v["repo"], "kebab");
    assert_eq!(v["git_branch"], "main");
    assert_eq!(v["git_commit"].as_str().unwrap().len(), 40);
    assert_eq!(v["code_lang"], "rust");
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p kebab-core --lib metadata_repo_fields -- --nocapture`
Expected: FAIL.

- [ ] **Step 3: Add four fields to `Metadata`**

After `user`:

```rust
    /// p10-1A-1: name of the source repo if the file lives inside a git
    /// working tree (`.git/` walk-up). null otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,

    /// p10-1A-1: HEAD branch at ingest time. null when no repo or detached HEAD.
    /// Informational only — current-state observability, not a partition key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_branch: Option<String>,

    /// p10-1A-1: HEAD commit (40-hex) at ingest time. null when no repo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_commit: Option<String>,

    /// p10-1A-1: programming language identifier (lowercase canonical). null
    /// for markdown / pdf / image. Set by `kebab_parse_code::lang::code_lang_for_path`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_lang: Option<String>,
```

- [ ] **Step 4: Run test + build workspace**

Run: `cargo test -p kebab-core --lib metadata_repo_fields && cargo build --workspace`
Expected: Test PASS. Build will reveal `Metadata` construction sites needing the four fields. Patch with `repo: None, git_branch: None, git_commit: None, code_lang: None` — additive, no behavioral change.

- [ ] **Step 5: Run full test suites for crates that touched**

Run: `cargo test -p kebab-core && cargo test -p kebab-parse-md && cargo test -p kebab-parse-pdf && cargo test -p kebab-parse-image && cargo test -p kebab-app`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/kebab-core/src/metadata.rs
# any consumer patches
git commit -m "feat(p10-1a-1): add Metadata.repo / git_branch / git_commit / code_lang"
```

---

## Task 5: New crate `kebab-parse-code` skeleton

**Files:**
- Create: `crates/kebab-parse-code/Cargo.toml`
- Create: `crates/kebab-parse-code/src/lib.rs`
- Modify: `Cargo.toml` (workspace root)

- [ ] **Step 1: Add `gix` to workspace dependencies**

Edit `Cargo.toml` (workspace root). In `[workspace.dependencies]`, add:

```toml
gix = { version = "0.66", default-features = false, features = ["worktree-mutation", "blocking-network-client"] }
```

(Verify the latest stable version on crates.io if 0.66 has shipped; this is approximate as of 2026-05.)

In `[workspace.members]`, append:

```toml
"crates/kebab-parse-code",
```

- [ ] **Step 2: Write `crates/kebab-parse-code/Cargo.toml`**

```toml
[package]
name = "kebab-parse-code"
version.workspace = true
edition.workspace = true
license.workspace = true

[dependencies]
anyhow.workspace = true
gix.workspace = true
kebab-core.path = "../kebab-core"

[dev-dependencies]
tempfile.workspace = true
```

(Verify `tempfile` is in workspace.dependencies. If not, add it there too.)

- [ ] **Step 3: Write `crates/kebab-parse-code/src/lib.rs`**

```rust
//! `kebab-parse-code` — language-aware parsing for code corpora.
//!
//! Phase 1A-1 ships infrastructure only:
//!
//! - [`lang::code_lang_for_path`] — extension → language identifier.
//! - [`repo::detect_repo`] — `.git/` walk-up → repo / branch / commit metadata.
//! - [`skip::is_generated_file`] / [`skip::is_oversized`] — pre-ingest skip
//!   helpers consulted by `kebab-source-fs`.
//! - [`skip::BUILTIN_BLACKLIST`] — 5-entry safety-net pattern list.
//!
//! Per-language parser modules (`rust`, `python`, `typescript`, …) land in
//! later phases (1A-2 onwards). The crate boundary is otherwise identical to
//! `kebab-parse-md` / `kebab-parse-pdf` per design §8: must NOT depend on
//! store / embed / llm / rag.

pub mod lang;
pub mod repo;
pub mod skip;

pub use lang::code_lang_for_path;
pub use repo::{RepoMeta, detect_repo};
pub use skip::{BUILTIN_BLACKLIST, is_generated_file, is_oversized};
```

- [ ] **Step 4: Run `cargo build -p kebab-parse-code` to confirm the empty crate compiles**

Run: `cargo build -p kebab-parse-code`
Expected: FAIL — `lang.rs` / `repo.rs` / `skip.rs` don't exist yet. That's fine; next task adds them.

- [ ] **Step 5: Commit (intentionally broken until Task 6/7/8 land — keep this commit atomic with the next three or squash later)**

```bash
git add Cargo.toml crates/kebab-parse-code/Cargo.toml crates/kebab-parse-code/src/lib.rs
git commit -m "feat(p10-1a-1): scaffold kebab-parse-code crate"
```

---

## Task 6: `kebab-parse-code::lang` — extension → language identifier

**Files:**
- Create: `crates/kebab-parse-code/src/lang.rs`
- Create: `crates/kebab-parse-code/tests/lang.rs`

- [ ] **Step 1: Write the test fixture**

`crates/kebab-parse-code/tests/lang.rs`:

```rust
use kebab_parse_code::code_lang_for_path;
use std::path::Path;

#[test]
fn known_extensions_map_to_canonical_identifiers() {
    let cases = [
        ("foo.rs", Some("rust")),
        ("foo.py", Some("python")),
        ("foo.pyi", Some("python")),
        ("foo.ts", Some("typescript")),
        ("foo.tsx", Some("typescript")),
        ("foo.js", Some("javascript")),
        ("foo.mjs", Some("javascript")),
        ("foo.cjs", Some("javascript")),
        ("foo.jsx", Some("javascript")),
        ("foo.go", Some("go")),
        ("foo.java", Some("java")),
        ("foo.kt", Some("kotlin")),
        ("foo.kts", Some("kotlin")),
        ("foo.c", Some("c")),
        ("foo.h", Some("c")),
        ("foo.cpp", Some("cpp")),
        ("foo.cc", Some("cpp")),
        ("foo.cxx", Some("cpp")),
        ("foo.hpp", Some("cpp")),
        ("foo.hh", Some("cpp")),
        ("foo.hxx", Some("cpp")),
        ("foo.yaml", Some("yaml")),
        ("foo.yml", Some("yaml")),
        ("foo.toml", Some("toml")),
        ("foo.json", Some("json")),
        ("foo.sh", Some("shell")),
        ("foo.bash", Some("shell")),
        ("foo.zsh", Some("shell")),
        ("foo.mk", Some("make")),
    ];
    for (path, expected) in cases {
        assert_eq!(
            code_lang_for_path(Path::new(path)),
            expected,
            "path = {path}"
        );
    }
}

#[test]
fn special_filenames_map_to_identifiers() {
    assert_eq!(code_lang_for_path(Path::new("Dockerfile")), Some("dockerfile"));
    assert_eq!(code_lang_for_path(Path::new("foo.dockerfile")), Some("dockerfile"));
    assert_eq!(code_lang_for_path(Path::new("Makefile")), Some("make"));
}

#[test]
fn unknown_extension_returns_none() {
    assert_eq!(code_lang_for_path(Path::new("foo.docx")), None);
    assert_eq!(code_lang_for_path(Path::new("foo")), None);
    assert_eq!(code_lang_for_path(Path::new("foo.unknown")), None);
}

#[test]
fn case_insensitive() {
    assert_eq!(code_lang_for_path(Path::new("Foo.RS")), Some("rust"));
    assert_eq!(code_lang_for_path(Path::new("FOO.YAML")), Some("yaml"));
}
```

- [ ] **Step 2: Run test to verify it fails (module doesn't exist)**

Run: `cargo test -p kebab-parse-code --test lang`
Expected: FAIL — `code_lang_for_path` not in scope.

- [ ] **Step 3: Write `crates/kebab-parse-code/src/lang.rs`**

```rust
//! Canonical extension → language identifier mapping (spec §3.5).
//!
//! Lowercase canonical identifiers, matching tree-sitter parser conventions:
//! `rust`, `python`, `typescript`, `javascript`, `go`, `java`, `kotlin`, `c`,
//! `cpp`, `yaml`, `toml`, `json`, `shell`, `make`, `dockerfile`.

use std::path::Path;

/// Returns the canonical language identifier for a given file path, or
/// `None` if the extension / filename is not recognized.
///
/// Matching priority:
///   1. exact filename match (e.g. `Dockerfile`, `Makefile`)
///   2. lowercase extension match
pub fn code_lang_for_path(path: &Path) -> Option<&'static str> {
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        match name {
            "Dockerfile" => return Some("dockerfile"),
            "Makefile" | "GNUmakefile" => return Some("make"),
            _ => {}
        }
    }
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "rs" => Some("rust"),
        "py" | "pyi" => Some("python"),
        "ts" | "tsx" => Some("typescript"),
        "js" | "mjs" | "cjs" | "jsx" => Some("javascript"),
        "go" => Some("go"),
        "java" => Some("java"),
        "kt" | "kts" => Some("kotlin"),
        "c" | "h" => Some("c"),
        "cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx" => Some("cpp"),
        "yaml" | "yml" => Some("yaml"),
        "toml" => Some("toml"),
        "json" => Some("json"),
        "sh" | "bash" | "zsh" => Some("shell"),
        "mk" => Some("make"),
        "dockerfile" => Some("dockerfile"),
        _ => None,
    }
}
```

- [ ] **Step 4: Run test**

Run: `cargo test -p kebab-parse-code --test lang`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/kebab-parse-code/src/lang.rs crates/kebab-parse-code/tests/lang.rs
git commit -m "feat(p10-1a-1): kebab-parse-code::lang — extension dispatcher"
```

---

## Task 7: `kebab-parse-code::repo` — `.git/` walk-up via `gix`

**Files:**
- Create: `crates/kebab-parse-code/src/repo.rs`
- Create: `crates/kebab-parse-code/tests/repo.rs`

- [ ] **Step 1: Write the test fixture**

`crates/kebab-parse-code/tests/repo.rs`:

```rust
use kebab_parse_code::repo::{RepoMeta, detect_repo};
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

fn init_git_repo(root: &std::path::Path) {
    // Use the `git` binary for fixture setup — the production code uses
    // `gix`. We don't care which library set up the fixture, we only verify
    // that the code reads it correctly.
    let run = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .current_dir(root)
            .status()
            .expect("git command failed");
    };
    run(&["init", "-q"]);
    run(&["config", "user.email", "test@test"]);
    run(&["config", "user.name", "test"]);
    fs::write(root.join("README.md"), "hi").unwrap();
    run(&["add", "README.md"]);
    run(&["commit", "-q", "-m", "init"]);
}

#[test]
fn detect_repo_returns_none_outside_git() {
    let tmp = TempDir::new().unwrap();
    let nested = tmp.path().join("a/b/c.txt");
    fs::create_dir_all(nested.parent().unwrap()).unwrap();
    fs::write(&nested, "x").unwrap();
    assert!(detect_repo(&nested).is_none());
}

#[test]
fn detect_repo_walks_up_to_git_dir() {
    let tmp = TempDir::new().unwrap();
    let repo_root = tmp.path().join("myrepo");
    fs::create_dir_all(&repo_root).unwrap();
    init_git_repo(&repo_root);
    let nested = repo_root.join("src/deep/file.rs");
    fs::create_dir_all(nested.parent().unwrap()).unwrap();
    fs::write(&nested, "x").unwrap();

    let meta = detect_repo(&nested).expect("should detect repo");
    assert_eq!(meta.name, "myrepo");
    assert!(meta.branch.is_some()); // could be "main" or "master" depending on git defaults
    assert!(meta.commit.is_some());
    assert_eq!(meta.commit.as_ref().unwrap().len(), 40);
}

#[test]
fn detect_repo_caches_per_path_call_for_repeated_files_in_same_repo() {
    // This is an observability check rather than a hard invariant —
    // detect_repo() may or may not cache internally, but it MUST be cheap
    // enough that calling it once per file in a repo doesn't blow up.
    // We just verify two calls in the same repo return the same name.
    let tmp = TempDir::new().unwrap();
    let repo_root = tmp.path().join("myrepo");
    fs::create_dir_all(&repo_root).unwrap();
    init_git_repo(&repo_root);
    let f1 = repo_root.join("a.rs");
    let f2 = repo_root.join("b.rs");
    fs::write(&f1, "x").unwrap();
    fs::write(&f2, "x").unwrap();
    let m1 = detect_repo(&f1).unwrap();
    let m2 = detect_repo(&f2).unwrap();
    assert_eq!(m1.name, m2.name);
    assert_eq!(m1.commit, m2.commit);
}
```

- [ ] **Step 2: Run test to verify failure**

Run: `cargo test -p kebab-parse-code --test repo`
Expected: FAIL — `detect_repo` not in scope.

- [ ] **Step 3: Write `crates/kebab-parse-code/src/repo.rs`**

```rust
//! Git repo auto-detection (spec §5.1).
//!
//! Walks up from `path` looking for a `.git/` directory. If found, reads
//! repo dir name, current branch, and HEAD commit using `gix` (pure Rust;
//! no `git` binary on PATH required).

use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoMeta {
    pub name: String,
    pub branch: Option<String>,
    pub commit: Option<String>,
}

/// Walk up from `path` until a `.git/` directory is found. Returns repo
/// metadata, or `None` if no repo boundary is reached before the filesystem
/// root.
///
/// - `name`: directory name containing `.git/`.
/// - `branch`: current HEAD branch, or `"detached"` if detached HEAD, or
///   `None` if branch can't be read.
/// - `commit`: 40-hex commit SHA at HEAD, or `None` if empty repo / read
///   failure.
///
/// `.git/` as a file (worktree marker / submodule) returns `None` for
/// `branch` and `commit` and falls back to the parent dir name for `name`.
pub fn detect_repo(path: &Path) -> Option<RepoMeta> {
    let mut cur = if path.is_dir() { path } else { path.parent()? };
    loop {
        let dotgit = cur.join(".git");
        if dotgit.is_dir() {
            let name = cur.file_name()?.to_string_lossy().into_owned();
            let (branch, commit) = read_head(cur);
            return Some(RepoMeta { name, branch, commit });
        } else if dotgit.is_file() {
            // worktree marker / submodule — name only.
            let name = cur.file_name()?.to_string_lossy().into_owned();
            return Some(RepoMeta { name, branch: None, commit: None });
        }
        cur = cur.parent()?;
    }
}

fn read_head(repo_dir: &Path) -> (Option<String>, Option<String>) {
    match gix::open(repo_dir) {
        Ok(repo) => {
            let branch = repo
                .head_name()
                .ok()
                .flatten()
                .map(|n| n.shorten().to_string())
                .or_else(|| Some("detached".to_string()));
            let commit = repo
                .head_id()
                .ok()
                .map(|id| id.to_string());
            (branch, commit)
        }
        Err(_) => (None, None),
    }
}
```

- [ ] **Step 4: Run test**

Run: `cargo test -p kebab-parse-code --test repo`
Expected: PASS (3 tests).

If the `gix` API differs in the available crate version (the surface around `head_name` / `head_id` evolves between minor versions), adjust the call sites — the test fixture is contract; the implementation can use whichever `gix` API achieves the same result.

- [ ] **Step 5: Commit**

```bash
git add crates/kebab-parse-code/src/repo.rs crates/kebab-parse-code/tests/repo.rs
git commit -m "feat(p10-1a-1): kebab-parse-code::repo — git walk-up via gix"
```

---

## Task 8: `kebab-parse-code::skip` — generated header + size cap + built-in blacklist

**Files:**
- Create: `crates/kebab-parse-code/src/skip.rs`
- Create: `crates/kebab-parse-code/tests/skip.rs`

- [ ] **Step 1: Write the test fixture**

`crates/kebab-parse-code/tests/skip.rs`:

```rust
use kebab_parse_code::skip::{BUILTIN_BLACKLIST, is_generated_file, is_oversized};
use std::fs;
use tempfile::NamedTempFile;

#[test]
fn generated_header_markers_trigger_skip() {
    let cases = [
        "// @generated\nfn foo() {}\n",
        "// Code generated by tonic-build. DO NOT EDIT.\nfn x() {}\n",
        "/* DO NOT EDIT */\nfn x() {}\n",
        "/* do not modify */\nfn x() {}\n",
        "// AUTOMATICALLY GENERATED\nfn x() {}\n",
        "# auto-generated\ndef x(): pass\n",
        "// autogenerated\nfn x() {}\n",
    ];
    for content in cases {
        let f = NamedTempFile::new().unwrap();
        fs::write(f.path(), content).unwrap();
        assert!(is_generated_file(f.path()).unwrap(), "content: {content:?}");
    }
}

#[test]
fn normal_code_is_not_flagged_generated() {
    let f = NamedTempFile::new().unwrap();
    fs::write(f.path(), "fn main() {\n    println!(\"hi\");\n}\n").unwrap();
    assert!(!is_generated_file(f.path()).unwrap());
}

#[test]
fn is_generated_returns_false_for_empty_file() {
    let f = NamedTempFile::new().unwrap();
    fs::write(f.path(), "").unwrap();
    assert!(!is_generated_file(f.path()).unwrap());
}

#[test]
fn oversized_by_bytes_returns_true() {
    let f = NamedTempFile::new().unwrap();
    let body: String = "x".repeat(300_000);
    fs::write(f.path(), &body).unwrap();
    assert!(is_oversized(f.path(), 262_144, 5_000).unwrap());
}

#[test]
fn oversized_by_lines_returns_true() {
    let f = NamedTempFile::new().unwrap();
    let body: String = "x\n".repeat(6_000); // 12_000 bytes, but 6_000 lines
    fs::write(f.path(), &body).unwrap();
    assert!(is_oversized(f.path(), 262_144, 5_000).unwrap());
}

#[test]
fn small_file_returns_false_for_oversize() {
    let f = NamedTempFile::new().unwrap();
    fs::write(f.path(), "fn foo() {}\n").unwrap();
    assert!(!is_oversized(f.path(), 262_144, 5_000).unwrap());
}

#[test]
fn builtin_blacklist_has_exactly_six_entries() {
    // node_modules/, target/, __pycache__/, .venv/, venv/, env/
    assert_eq!(BUILTIN_BLACKLIST.len(), 6);
    let expected = [
        "**/node_modules/**",
        "**/target/**",
        "**/__pycache__/**",
        "**/.venv/**",
        "**/venv/**",
        "**/env/**",
    ];
    for pat in expected {
        assert!(BUILTIN_BLACKLIST.contains(&pat), "missing pattern: {pat}");
    }
}
```

- [ ] **Step 2: Run test to verify failure**

Run: `cargo test -p kebab-parse-code --test skip`
Expected: FAIL — `skip` module not in scope.

- [ ] **Step 3: Write `crates/kebab-parse-code/src/skip.rs`**

```rust
//! Pre-ingest skip helpers (spec §5.3 + §5.4 + §5.2 built-in).
//!
//! - [`BUILTIN_BLACKLIST`] — 6 gitignore-style patterns universal across
//!   ecosystems. Source-of-truth list: see spec §5.2.
//! - [`is_generated_file`] — reads first ~512 bytes, checks for 7
//!   case-insensitive markers. False positives are *intentional* — we'd
//!   rather skip a hand-written file with "DO NOT EDIT" in a comment than
//!   index 50K lines of protobuf output.
//! - [`is_oversized`] — byte cap then line cap. Cascade is cheap because
//!   most code files are well under the byte cap.

use anyhow::Result;
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

/// 6 built-in gitignore-style patterns. These are applied *in addition to*
/// `.gitignore` + `.kebabignore`, and they have priority — user negation
/// (`!pattern` in `.kebabignore`) is the only way to override.
pub const BUILTIN_BLACKLIST: &[&str] = &[
    "**/node_modules/**",
    "**/target/**",
    "**/__pycache__/**",
    "**/.venv/**",
    "**/venv/**",
    "**/env/**",
];

/// Read the first 512 bytes of `path` and check for any of the 7
/// case-insensitive generated-file markers. Returns Ok(true) on match,
/// Ok(false) otherwise. IO errors propagate.
pub fn is_generated_file(path: &Path) -> Result<bool> {
    let mut buf = [0u8; 512];
    let mut f = File::open(path)?;
    let n = f.read(&mut buf)?;
    if n == 0 {
        return Ok(false);
    }
    // Only look at valid UTF-8 prefix; if the head is binary, we skip via
    // size cap / extension policy elsewhere.
    let head = std::str::from_utf8(&buf[..n]).unwrap_or("");
    let lower: String = head.lines().take(10).collect::<Vec<_>>().join("\n").to_ascii_lowercase();
    Ok(
        lower.contains("@generated")
            || lower.contains("code generated by")
            || lower.contains("do not edit")
            || lower.contains("do not modify")
            || lower.contains("automatically generated")
            || lower.contains("auto-generated")
            || lower.contains("autogenerated"),
    )
}

/// Check if `path` exceeds `max_bytes` or `max_lines`. Byte cap is checked
/// first (cheap stat call); line cap only if byte cap passes (streaming
/// read with early exit).
pub fn is_oversized(path: &Path, max_bytes: u64, max_lines: u32) -> Result<bool> {
    let meta = std::fs::metadata(path)?;
    if meta.len() > max_bytes {
        return Ok(true);
    }
    let reader = BufReader::new(File::open(path)?);
    let mut count: u32 = 0;
    for line in reader.lines() {
        let _ = line?;
        count = count.saturating_add(1);
        if count > max_lines {
            return Ok(true);
        }
    }
    Ok(false)
}
```

- [ ] **Step 4: Run test**

Run: `cargo test -p kebab-parse-code --test skip`
Expected: PASS (7 tests).

- [ ] **Step 5: Build the whole crate**

Run: `cargo build -p kebab-parse-code`
Expected: PASS (no warnings preferable but not blocking).

- [ ] **Step 6: Commit**

```bash
git add crates/kebab-parse-code/src/skip.rs crates/kebab-parse-code/tests/skip.rs
git commit -m "feat(p10-1a-1): kebab-parse-code::skip — generated / size / blacklist helpers"
```

---

## Task 9: `kebab-source-fs` — integrate `.gitignore` honor + built-in blacklist

**Files:**
- Modify: `crates/kebab-source-fs/src/walker.rs`
- Modify: `crates/kebab-source-fs/Cargo.toml` (add `kebab-parse-code` dep)

- [ ] **Step 1: Read the existing `build_overrides` to understand the integration point**

Run: `sed -n '50,100p' crates/kebab-source-fs/src/walker.rs`
Note the function signature and where it merges `.kebabignore` patterns. We'll extend it to also merge the built-in blacklist patterns and (per repo-root .gitignore) but for simplicity at v1, we use `ignore::WalkBuilder` indirectly via the existing path — the cleanest integration is to add a separate `OverrideBuilder` pass for built-ins and rely on per-directory `.gitignore` discovery via the `ignore` crate. **Decision for 1A-1:** simplest viable path is to add the 6 built-in patterns to the same `OverrideBuilder` that already holds `.kebabignore` patterns. `.gitignore` honor (per-repo cascade) is best done by letting `walkdir` ignore `.git/` and relying on the existing `ignore::Override` mechanism with the addition of `.gitignore` files merged at the workspace.root level only.

A full implementation reading nested `.gitignore` cascade is in **Task 10** (a follow-up step in this plan). Task 9 lands only the built-in blacklist piece.

- [ ] **Step 2: Add `kebab-parse-code` as a dep in `crates/kebab-source-fs/Cargo.toml`**

In the `[dependencies]` table:

```toml
kebab-parse-code.path = "../kebab-parse-code"
```

- [ ] **Step 3: Append failing test to `crates/kebab-source-fs/tests/`** (or to walker.rs's `mod tests`)

```rust
#[test]
fn built_in_blacklist_excludes_node_modules() {
    use tempfile::TempDir;
    use std::fs;

    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::create_dir_all(root.join("node_modules/foo")).unwrap();
    fs::write(root.join("src/main.rs"), "x").unwrap();
    fs::write(root.join("node_modules/foo/bar.js"), "x").unwrap();

    let overrides = build_overrides(root, &[], &[]).unwrap();
    let m_in = overrides.matched(root.join("src/main.rs"), false);
    let m_out = overrides.matched(root.join("node_modules/foo/bar.js"), false);

    assert!(!m_in.is_ignore(), "src/main.rs should NOT be ignored");
    assert!(m_out.is_ignore(), "node_modules/foo/bar.js SHOULD be ignored");
}

#[test]
fn built_in_blacklist_excludes_target_pycache_venv() {
    use tempfile::TempDir;
    use std::fs;

    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    for dir in ["target/x", "__pycache__/x", ".venv/x", "venv/x", "env/x"] {
        fs::create_dir_all(root.join(dir)).unwrap();
        fs::write(root.join(dir).join("y.txt"), "z").unwrap();
    }
    fs::create_dir_all(root.join("ok")).unwrap();
    fs::write(root.join("ok/z.txt"), "z").unwrap();

    let overrides = build_overrides(root, &[], &[]).unwrap();
    for blacklisted in [
        "target/x/y.txt",
        "__pycache__/x/y.txt",
        ".venv/x/y.txt",
        "venv/x/y.txt",
        "env/x/y.txt",
    ] {
        let m = overrides.matched(root.join(blacklisted), false);
        assert!(m.is_ignore(), "{blacklisted} should be ignored");
    }
    let m_ok = overrides.matched(root.join("ok/z.txt"), false);
    assert!(!m_ok.is_ignore(), "ok/z.txt should not be ignored");
}
```

- [ ] **Step 4: Run test**

Run: `cargo test -p kebab-source-fs --lib built_in_blacklist`
Expected: FAIL.

- [ ] **Step 5: Extend `build_overrides` to include built-in patterns**

Locate the function (around line 56 of `walker.rs`). Before the loop that adds `kbignore_patterns`, add:

```rust
    // p10-1A-1: built-in safety-net blacklist (spec §5.2). 6 patterns that
    // are universal across ecosystems. User can negate via `.kebabignore`.
    for pat in kebab_parse_code::BUILTIN_BLACKLIST {
        builder
            .add(pat)
            .with_context(|| format!("built-in blacklist pattern: {pat}"))?;
    }
```

If the existing patterns are stored with `!`-prefix to make `OverrideBuilder` treat them as excludes, match that convention; the `BUILTIN_BLACKLIST` should be applied via the same convention.

- [ ] **Step 6: Run test**

Run: `cargo test -p kebab-source-fs --lib built_in_blacklist`
Expected: PASS.

- [ ] **Step 7: Run the full `kebab-source-fs` suite**

Run: `cargo test -p kebab-source-fs`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add crates/kebab-source-fs/Cargo.toml crates/kebab-source-fs/src/walker.rs
git commit -m "feat(p10-1a-1): integrate built-in blacklist into walker overrides"
```

---

## Task 10: `kebab-source-fs` — `.gitignore` honor (per-repo cascade)

**Files:**
- Modify: `crates/kebab-source-fs/src/walker.rs` (or `connector.rs` — see below)

Decision: use the `ignore::WalkBuilder` to walk and discover `.gitignore` cascade automatically rather than implementing it manually. The current walker uses `walkdir::WalkDir` for tighter control. The simplest path: read each repo's root `.gitignore` (one per repo boundary) at walk start, add its patterns to the `Override`. This handles the 80% case (most repos use a single repo-root `.gitignore`). Nested `.gitignore` cascade is deferred to a follow-up (open question in spec §11).

- [ ] **Step 1: Append failing test**

```rust
#[test]
fn gitignore_at_repo_root_excludes_matching_files() {
    use tempfile::TempDir;
    use std::fs;

    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join(".gitignore"), "*.log\ndist/\n").unwrap();
    fs::write(root.join("a.log"), "x").unwrap();
    fs::write(root.join("src/main.rs"), "x").unwrap();
    fs::create_dir_all(root.join("dist")).unwrap();
    fs::write(root.join("dist/bundle.js"), "x").unwrap();

    let overrides = build_overrides_with_gitignore(root, &[], &[]).unwrap();
    assert!(overrides.matched(root.join("a.log"), false).is_ignore());
    assert!(overrides.matched(root.join("dist/bundle.js"), false).is_ignore());
    assert!(!overrides.matched(root.join("src/main.rs"), false).is_ignore());
}
```

(Or rename existing `build_overrides` once it includes `.gitignore` reading. For minimal disruption, introduce `build_overrides_with_gitignore` and have the old wrapper call into it with `.gitignore` enabled by default.)

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p kebab-source-fs --lib gitignore_at_repo_root`
Expected: FAIL.

- [ ] **Step 3: Add a `read_gitignore` helper + extend `build_overrides`**

```rust
/// Read `<root>/.gitignore` (single-file, root-only — nested cascade is P+).
/// Missing file → empty Vec. Comments / blanks stripped.
pub(crate) fn read_gitignore(root: &Path) -> Result<Vec<String>> {
    let p = root.join(".gitignore");
    if !p.exists() {
        return Ok(vec![]);
    }
    let s = std::fs::read_to_string(&p)
        .with_context(|| format!("read .gitignore at {}", p.display()))?;
    Ok(s.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect())
}
```

Modify `build_overrides` to also accept `.gitignore` patterns and add them with the existing convention (excludes prefix). Place them *after* built-in blacklist (so `.kebabignore` can negate both) and *before* `.kebabignore`.

- [ ] **Step 4: Run test + full suite**

Run: `cargo test -p kebab-source-fs`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/kebab-source-fs/src/walker.rs
git commit -m "feat(p10-1a-1): honor repo-root .gitignore in walker overrides"
```

---

## Task 11: wire IngestReport skip counters through `kebab-source-fs` connector

**Files:**
- Modify: `crates/kebab-source-fs/src/connector.rs`
- Modify: `crates/kebab-source-fs/src/walker.rs`

This task threads the new `IngestReport.skipped_gitignore` / `skipped_kebabignore` / `skipped_builtin_blacklist` counters through the connector. `skipped_generated` / `skipped_size_exceeded` come from `kebab-parse-code::skip` and are wired in Task 13 (the per-file decision point).

- [ ] **Step 1: Read `connector.rs` to find the IngestReport assembly site**

Run: `grep -n "IngestReport\|skipped" crates/kebab-source-fs/src/connector.rs | head -20`

- [ ] **Step 2: Append failing test (where the connector is tested)**

```rust
#[test]
fn ingest_report_counts_gitignored_files_under_skipped_gitignore() {
    use tempfile::TempDir;
    use std::fs;

    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::write(root.join(".gitignore"), "*.log\n").unwrap();
    fs::write(root.join("ok.md"), "# ok").unwrap();
    fs::write(root.join("skipme.log"), "x").unwrap();

    let report = run_scan(root);  // your connector entry point
    assert_eq!(report.skipped_gitignore, 1);
    assert!(report.skip_examples.gitignore.contains(&"skipme.log".to_string()));
}
```

(Adapt `run_scan` to whatever the connector's actual entry is.)

- [ ] **Step 3: Implement the per-category increment**

In `connector.rs`, where the walker iterator is consumed, distinguish *why* a file was excluded by checking the override matchers in order:

```rust
// pseudocode — match the existing code style
for entry in walker {
    let path = entry?.path();
    if matches_builtin_blacklist(&path) {
        report.skipped_builtin_blacklist += 1;
        push_sample(&mut report.skip_examples.builtin_blacklist, &path);
        continue;
    }
    if matches_gitignore(&path) {
        report.skipped_gitignore += 1;
        push_sample(&mut report.skip_examples.gitignore, &path);
        continue;
    }
    if matches_kebabignore(&path) {
        report.skipped_kebabignore += 1;
        // (skip_examples.kebabignore intentionally not in SkipExamples per spec)
        continue;
    }
    // ... proceed with ingest
}
```

Helper:

```rust
fn push_sample(samples: &mut Vec<String>, path: &Path) {
    if samples.len() < 5 {
        samples.push(path.to_string_lossy().into_owned());
    }
}
```

- [ ] **Step 4: Run test + full suite**

Run: `cargo test -p kebab-source-fs`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/kebab-source-fs/src/connector.rs crates/kebab-source-fs/src/walker.rs
git commit -m "feat(p10-1a-1): split skip counters by category in IngestReport"
```

---

## Task 12: wire generated / size cap skip checks per file

**Files:**
- Modify: `crates/kebab-source-fs/src/connector.rs`
- Modify: `crates/kebab-config/src/lib.rs` (need `IngestCodeCfg` first — Task 14)

Note: This task depends on Task 14's config struct. Reorder execution: do Task 14 first, then return to Task 12.

- [ ] **Step 1: Read connector to find the per-file decision point** (after walker yield, before parse dispatch)

- [ ] **Step 2: Append failing test**

```rust
#[test]
fn ingest_report_counts_generated_files() {
    use tempfile::TempDir;
    use std::fs;

    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::write(root.join("normal.md"), "# hi").unwrap();
    fs::write(root.join("autogen.rs"), "// @generated\nfn x() {}\n").unwrap();

    let report = run_scan_with_code_cfg(root, &IngestCodeCfg {
        skip_generated_header: true,
        ..Default::default()
    });
    assert_eq!(report.skipped_generated, 1);
    assert!(report.skip_examples.generated.contains(&"autogen.rs".to_string()));
}

#[test]
fn ingest_report_counts_oversized_files() {
    use tempfile::TempDir;
    use std::fs;

    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::write(root.join("normal.md"), "# hi").unwrap();
    let big: String = "x\n".repeat(100_000);
    fs::write(root.join("huge.rs"), &big).unwrap();

    let report = run_scan_with_code_cfg(root, &IngestCodeCfg {
        max_file_bytes: 1024,
        max_file_lines: 5_000,
        ..Default::default()
    });
    assert_eq!(report.skipped_size_exceeded, 1);
}
```

- [ ] **Step 3: Add the per-file check between walker yield and parse dispatch**

```rust
// After: file passed gitignore / kebabignore / built-in checks.
// Before: parse dispatch by media type.
if cfg.code.skip_generated_header
    && kebab_parse_code::is_generated_file(&path).unwrap_or(false)
{
    report.skipped_generated += 1;
    push_sample(&mut report.skip_examples.generated, &path);
    continue;
}
if kebab_parse_code::is_oversized(&path, cfg.code.max_file_bytes, cfg.code.max_file_lines)
    .unwrap_or(false)
{
    report.skipped_size_exceeded += 1;
    push_sample(&mut report.skip_examples.size_exceeded, &path);
    continue;
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p kebab-source-fs && cargo test -p kebab-app`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/kebab-source-fs/src/connector.rs
git commit -m "feat(p10-1a-1): apply generated-header + size-cap skip per file"
```

---

## Task 13: regression test — markdown corpus output unchanged

**Files:**
- Create: `crates/kebab-cli/tests/wire_search_hit_no_code_fields.rs`
- Create: `crates/kebab-cli/tests/wire_citation_5_variants_unchanged.rs`

These tests prove the wire is byte-identical for the existing markdown corpus after 1A-1 lands. They are the *gate* on the framework changes.

- [ ] **Step 1: Write `wire_search_hit_no_code_fields.rs`**

```rust
use kebab_core::{Citation, SearchHit, RetrievalDetail, ScoreKind};
use kebab_core::{ChunkId, ChunkerVersion, DocumentId, IndexVersion, WorkspacePath};

#[test]
fn markdown_hit_omits_repo_and_code_lang() {
    let hit = SearchHit {
        rank: 1,
        chunk_id: ChunkId("c1".into()),
        doc_id: DocumentId("d1".into()),
        doc_path: WorkspacePath("notes/foo.md".into()),
        heading_path: vec!["A".into(), "B".into()],
        section_label: Some("B".into()),
        snippet: "hi".into(),
        citation: Citation::Line {
            path: WorkspacePath("notes/foo.md".into()),
            start: 1,
            end: 2,
            section: None,
        },
        retrieval: RetrievalDetail::default(),
        index_version: IndexVersion("v1".into()),
        embedding_model: None,
        chunker_version: ChunkerVersion("md-heading-v1".into()),
        indexed_at: time::OffsetDateTime::UNIX_EPOCH,
        stale: false,
        score_kind: ScoreKind::Rrf,
        repo: None,
        code_lang: None,
    };
    let s = serde_json::to_string(&hit).unwrap();
    assert!(!s.contains("\"repo\""), "repo should be absent: {s}");
    assert!(!s.contains("\"code_lang\""), "code_lang should be absent: {s}");
}
```

- [ ] **Step 2: Write `wire_citation_5_variants_unchanged.rs`**

```rust
use kebab_core::{Citation, WorkspacePath};

#[test]
fn line_variant_serialization_unchanged() {
    let c = Citation::Line {
        path: WorkspacePath("a.md".into()),
        start: 1,
        end: 2,
        section: Some("§14".into()),
    };
    let v = serde_json::to_value(&c).unwrap();
    assert_eq!(v["kind"], "line");
    assert_eq!(v["start"], 1);
    assert_eq!(v["end"], 2);
    assert_eq!(v["section"], "§14");
    assert!(v.get("line_start").is_none());
    assert!(v.get("symbol").is_none());
    assert!(v.get("code").is_none());
}

#[test]
fn page_variant_serialization_unchanged() {
    let c = Citation::Page {
        path: WorkspacePath("a.pdf".into()),
        page: 13,
        section: None,
    };
    let v = serde_json::to_value(&c).unwrap();
    assert_eq!(v["kind"], "page");
    assert_eq!(v["page"], 13);
    assert!(v.get("line_start").is_none());
}

#[test]
fn caption_variant_serialization_unchanged() {
    let c = Citation::Caption {
        path: WorkspacePath("a.png".into()),
        model: "qwen2.5-vl:7b".into(),
    };
    let v = serde_json::to_value(&c).unwrap();
    assert_eq!(v["kind"], "caption");
    assert_eq!(v["model"], "qwen2.5-vl:7b");
}
```

- [ ] **Step 3: Run regression tests**

Run: `cargo test -p kebab-cli --test wire_search_hit_no_code_fields && cargo test -p kebab-cli --test wire_citation_5_variants_unchanged`
Expected: PASS. (If you forgot `#[serde(skip_serializing_if = "Option::is_none")]` in Task 2, the regression FAILS here — fix and commit.)

- [ ] **Step 4: Commit**

```bash
git add crates/kebab-cli/tests/wire_search_hit_no_code_fields.rs crates/kebab-cli/tests/wire_citation_5_variants_unchanged.rs
git commit -m "test(p10-1a-1): regression — markdown wire output unchanged"
```

---

## Task 14: `kebab-config` — `[ingest.code]` section

**Files:**
- Modify: `crates/kebab-config/src/lib.rs`

- [ ] **Step 1: Append failing test**

```rust
#[test]
fn ingest_code_cfg_defaults() {
    let cfg: IngestCodeCfg = toml::from_str("").unwrap();
    assert_eq!(cfg.max_file_bytes, 262_144);
    assert_eq!(cfg.max_file_lines, 5_000);
    assert!(cfg.skip_generated_header);
    assert!(cfg.extra_skip_globs.is_empty());
    assert_eq!(cfg.ast_chunk_max_lines, 200);
    assert_eq!(cfg.fallback_lines_per_chunk, 80);
    assert_eq!(cfg.fallback_lines_overlap, 20);
}

#[test]
fn ingest_code_cfg_user_override() {
    let toml = r#"
        max_file_bytes = 1048576
        max_file_lines = 20000
        skip_generated_header = false
        extra_skip_globs = ["**/fixtures/**", "**/snapshots/**"]
    "#;
    let cfg: IngestCodeCfg = toml::from_str(toml).unwrap();
    assert_eq!(cfg.max_file_bytes, 1_048_576);
    assert_eq!(cfg.max_file_lines, 20_000);
    assert!(!cfg.skip_generated_header);
    assert_eq!(cfg.extra_skip_globs.len(), 2);
}

#[test]
fn config_with_ingest_code_section() {
    let toml = r#"
        [workspace]
        root = "~/Notes"

        [ingest.code]
        max_file_bytes = 524288
    "#;
    let cfg: Config = toml::from_str(toml).unwrap();
    assert_eq!(cfg.ingest.code.max_file_bytes, 524_288);
}
```

- [ ] **Step 2: Run failing test**

Run: `cargo test -p kebab-config --lib ingest_code -- --nocapture`
Expected: FAIL.

- [ ] **Step 3: Add `IngestCodeCfg` struct and `IngestCfg` wrapper (if absent)**

In `crates/kebab-config/src/lib.rs`:

```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct IngestCfg {
    pub code: IngestCodeCfg,
}

impl Default for IngestCfg {
    fn default() -> Self {
        Self { code: IngestCodeCfg::default() }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct IngestCodeCfg {
    /// Generated header sniff. Reads first ~512 bytes, checks 7 markers.
    pub skip_generated_header: bool,
    /// Max byte size per file. Bigger files skipped.
    pub max_file_bytes: u64,
    /// Max line count per file. Bigger files skipped (byte cap checked first).
    pub max_file_lines: u32,
    /// User extra skip globs (gitignore syntax). Applied on top of built-in
    /// + `.gitignore` + `.kebabignore`.
    pub extra_skip_globs: Vec<String>,
    /// AST chunk size cap. Functions/classes longer than this fall back to
    /// paragraph-based split (1A-2 and later).
    pub ast_chunk_max_lines: u32,
    /// Tier 3 fallback chunker: lines per chunk.
    pub fallback_lines_per_chunk: u32,
    /// Tier 3 fallback chunker: line overlap between adjacent chunks.
    pub fallback_lines_overlap: u32,
}

impl Default for IngestCodeCfg {
    fn default() -> Self {
        Self {
            skip_generated_header: true,
            max_file_bytes: 262_144,
            max_file_lines: 5_000,
            extra_skip_globs: vec![],
            ast_chunk_max_lines: 200,
            fallback_lines_per_chunk: 80,
            fallback_lines_overlap: 20,
        }
    }
}
```

Then add `pub ingest: IngestCfg` to the `Config` struct with `#[serde(default)]`.

- [ ] **Step 4: Run test**

Run: `cargo test -p kebab-config --lib ingest_code -- --nocapture`
Expected: PASS.

- [ ] **Step 5: Build the workspace; expect config consumers to need updating**

Run: `cargo build --workspace`
Expected: PASS (the new field has Default → no breakage).

- [ ] **Step 6: Commit**

```bash
git add crates/kebab-config/src/lib.rs
git commit -m "feat(p10-1a-1): add [ingest.code] config section"
```

---

## Task 15: `kebab-cli` — `--repo` / `--code-lang` flags

**Files:**
- Modify: `crates/kebab-cli/src/main.rs`

Note: `--media code` works automatically because `--media` is already a free-form Vec<String>. We just document it.

- [ ] **Step 1: Append failing test (CLI integration)**

`crates/kebab-cli/tests/wire_search_filters_code.rs`:

```rust
use assert_cmd::Command;
use predicates::str::contains;

#[test]
fn cli_accepts_repo_flag_repeated() {
    let mut cmd = Command::cargo_bin("kebab").unwrap();
    let assert = cmd
        .args(["search", "--repo", "foo", "--repo", "bar", "--help"])
        .assert()
        .success();
    // --help short-circuits — we're just verifying the flag parses.
    let _ = assert;
}

#[test]
fn cli_accepts_code_lang_flag_repeated() {
    let mut cmd = Command::cargo_bin("kebab").unwrap();
    cmd.args(["search", "--code-lang", "rust", "--code-lang", "python", "--help"])
        .assert()
        .success();
}

#[test]
fn cli_accepts_media_code_value() {
    let mut cmd = Command::cargo_bin("kebab").unwrap();
    cmd.args(["search", "--media", "code", "--help"])
        .assert()
        .success();
}
```

- [ ] **Step 2: Run failing test**

Run: `cargo test -p kebab-cli --test wire_search_filters_code`
Expected: FAIL — `--repo` and `--code-lang` not recognized.

- [ ] **Step 3: Add the flags to `Cmd::Search` in `crates/kebab-cli/src/main.rs`**

After the existing `media: Vec<String>` field:

```rust
        /// p10-1A-1: filter by repo name (`metadata.repo`). Repeatable;
        /// multi-value = OR.
        #[arg(long = "repo", value_name = "NAME", num_args = 1)]
        repo: Vec<String>,

        /// p10-1A-1: filter by code language identifier (lowercase canonical).
        /// Repeatable or comma-separated. Examples: rust,python,typescript.
        /// Unknown values produce empty hits.
        #[arg(long = "code-lang", value_name = "LANG", num_args = 1, value_delimiter = ',')]
        code_lang: Vec<String>,
```

- [ ] **Step 4: Propagate to `SearchFilters` in the dispatch site**

In the `Cmd::Search` arm where `SearchFilters` is constructed (around `media: media_norm,`):

```rust
                SearchFilters {
                    // ... existing fields ...
                    media: media_norm,
                    ingested_after,
                    doc_id: doc_id_parsed,
                    repo,
                    code_lang,
                }
```

- [ ] **Step 5: Run test + full CLI suite**

Run: `cargo test -p kebab-cli --test wire_search_filters_code && cargo test -p kebab-cli`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/kebab-cli/src/main.rs crates/kebab-cli/tests/wire_search_filters_code.rs
git commit -m "feat(p10-1a-1): add --repo / --code-lang CLI flags"
```

---

## Task 16: `kebab-app::schema` — `code_lang_breakdown` + `repo_breakdown` stats

**Files:**
- Modify: `crates/kebab-app/src/schema.rs`

- [ ] **Step 1: Append failing test**

```rust
#[test]
fn schema_stats_includes_code_lang_and_repo_breakdown() {
    let stats = SchemaStats {
        // existing fields with sensible defaults
        ..Default::default()
    };
    let v = serde_json::to_value(&stats).unwrap();
    assert!(v.get("code_lang_breakdown").is_some(), "stats must include code_lang_breakdown");
    assert!(v.get("repo_breakdown").is_some(), "stats must include repo_breakdown");
}
```

- [ ] **Step 2: Run failing test**

Run: `cargo test -p kebab-app --lib schema_stats_includes -- --nocapture`
Expected: FAIL.

- [ ] **Step 3: Add the two BTreeMaps to `SchemaStats`**

```rust
    /// p10-1A-1: code language breakdown (chunk counts by canonical lowercase
    /// language identifier). Empty until 1A-2 produces code chunks.
    #[serde(default)]
    pub code_lang_breakdown: std::collections::BTreeMap<String, u32>,

    /// p10-1A-1: repo breakdown (chunk counts by `metadata.repo` value).
    /// Empty until 1A-2 produces code chunks.
    #[serde(default)]
    pub repo_breakdown: std::collections::BTreeMap<String, u32>,
```

Also add `code` to the `media_breakdown` if it's an enumerated set (verify the existing impl — it may be free-form `BTreeMap<String, u32>` already).

- [ ] **Step 4: Run test + full app suite**

Run: `cargo test -p kebab-app`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/kebab-app/src/schema.rs
git commit -m "feat(p10-1a-1): SchemaStats — code_lang_breakdown + repo_breakdown"
```

---

## Task 17: wire schema JSON files

**Files:**
- Modify: `docs/wire-schema/v1/citation.schema.json`
- Modify: `docs/wire-schema/v1/search_hit.schema.json`
- Modify: `docs/wire-schema/v1/ingest_report.schema.json`
- Modify: `docs/wire-schema/v1/schema.schema.json`

- [ ] **Step 1: Update `citation.schema.json`**

Add `"code"` to the `kind` enum and `"code": { "type": "object" }` to the properties:

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://kb.local/wire/v1/citation.schema.json",
  "title": "Citation v1",
  "description": "Stub schema — declares the schema_version label and the always-present fields. Variant-discriminated property validation lands in a later phase.",
  "type": "object",
  "required": ["schema_version", "kind", "path", "uri", "indexed_at", "stale"],
  "properties": {
    "schema_version": { "const": "citation.v1" },
    "kind": { "enum": ["line", "page", "region", "caption", "time", "code"] },
    "path": { "type": "string" },
    "uri":  { "type": "string" },
    "line":    { "type": "object" },
    "page":    { "type": "object" },
    "region":  { "type": "object" },
    "caption": { "type": "object" },
    "time":    { "type": "object" },
    "code":    { "type": "object" },
    "indexed_at": { "type": "string", "format": "date-time" },
    "stale":      { "type": "boolean" }
  }
}
```

- [ ] **Step 2: Update `search_hit.schema.json`**

Add to `properties`:

```json
    "repo":       { "type": ["string", "null"] },
    "code_lang":  { "type": ["string", "null"] }
```

(Verify the file's existing structure first via `cat docs/wire-schema/v1/search_hit.schema.json`.)

- [ ] **Step 3: Update `ingest_report.schema.json`**

Add to `properties`:

```json
    "skipped_gitignore":         { "type": "integer", "minimum": 0 },
    "skipped_kebabignore":       { "type": "integer", "minimum": 0 },
    "skipped_builtin_blacklist": { "type": "integer", "minimum": 0 },
    "skipped_generated":         { "type": "integer", "minimum": 0 },
    "skipped_size_exceeded":     { "type": "integer", "minimum": 0 },
    "skip_examples": {
      "type": "object",
      "properties": {
        "generated":         { "type": "array", "items": { "type": "string" }, "maxItems": 5 },
        "size_exceeded":     { "type": "array", "items": { "type": "string" }, "maxItems": 5 },
        "builtin_blacklist": { "type": "array", "items": { "type": "string" }, "maxItems": 5 },
        "gitignore":         { "type": "array", "items": { "type": "string" }, "maxItems": 5 }
      }
    }
```

- [ ] **Step 4: Update `schema.schema.json`**

Add to `stats.properties`:

```json
    "code_lang_breakdown": {
      "type": "object",
      "additionalProperties": { "type": "integer", "minimum": 0 }
    },
    "repo_breakdown": {
      "type": "object",
      "additionalProperties": { "type": "integer", "minimum": 0 }
    }
```

- [ ] **Step 5: Verify JSON validity**

Run: `for f in docs/wire-schema/v1/*.json; do python3 -m json.tool < "$f" > /dev/null && echo "$f OK" || echo "$f BAD"; done`
Expected: All files report OK.

- [ ] **Step 6: Commit**

```bash
git add docs/wire-schema/v1/
git commit -m "feat(p10-1a-1): wire schema v1 — code variant + repo/code_lang + skip counters"
```

---

## Task 18: Frozen design doc update

**Files:**
- Modify: `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md`

- [ ] **Step 1: Read §10.1 of the code ingest spec** for the exact list of frozen-design sections that need updating

Run: `sed -n '/^### 10.1/,/^### 10.2/p' docs/superpowers/specs/2026-05-15-kebab-code-ingest-design.md`

- [ ] **Step 2: Update §0 (동결된 결정 요약)** — add one row at the bottom

```
| C+ | code ingest 추가 | Tier 1/2/3 fan-out, e5-large 유지, 새 Citation `code` variant | 2026-05-15 spec cross-link |
```

- [ ] **Step 3: Update §2.1 Citation** — change "5-variant" to "6-variant" and add the `code` example block

Find the existing `Citation (5 variants — discriminated by `kind`)` heading and update.

- [ ] **Step 4: Update §2.2 SearchHit** — add `repo` / `code_lang` rows to the example JSON, with a one-line note "p10-1A-1: optional, omitted when null".

- [ ] **Step 5: Update §2.4 IngestReport** — add the 5 new skip counters and `skip_examples` to the example.

- [ ] **Step 6: Update §3.2 Versions / labels** — note "chunker_version family extended in phase 10 (per-language pattern). See 2026-05-15 spec §3.3 for canonical list."

- [ ] **Step 7: Update §3.6 Metadata** — add the four new fields with one-line notes.

- [ ] **Step 8: Update §8 모듈 경계** — add `kebab-parse-code` to the crate inventory and inheritance rules (same boundary as other `kebab-parse-*`).

- [ ] **Step 9: Update §11 동결 범위** — add one line: "코드 ingest 는 더 이상 비-스코프 아님 (2026-05-15 spec). 단 multi-workspace / watch mode / history aware 는 그대로 비-스코프."

- [ ] **Step 10: Commit**

```bash
git add docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
git commit -m "docs(p10-1a-1): apply code ingest framework to frozen design"
```

---

## Task 19: README / HANDOFF / SMOKE updates

**Files:**
- Modify: `README.md`
- Modify: `HANDOFF.md`
- Modify: `docs/SMOKE.md`

- [ ] **Step 1: Update README's `kebab search` command row**

Append to the existing flag list inside the `search` row table:

```
[--repo NAME ...] [--code-lang LIST] [--media code]
```

After the existing `--media md` / `--media markdown` alias paragraph (around the "filter flags" block), add:

```markdown
**code corpus filters (p10-1A-1):** `--repo` 는 반복 가능 (`--repo kebab --repo other`) OR 매칭. `--code-lang` 는 반복 또는 comma 다중 값 (`--code-lang rust,python`), 알 수 없는 값은 빈 hits. `--media code` 는 Tier 1/2/3 모든 code chunk 포함. 1A-1 시점에서는 indexed 된 code chunk 가 없어 filter 가 항상 빈 결과 — 1A-2 (Rust AST chunker) 머지 이후 실효.
```

- [ ] **Step 2: Add Configuration row about `[ingest.code]`**

Under the existing Configuration section, add:

```markdown
- `[ingest.code]` (p10-1A-1) — code ingest 의 skip 정책 + chunker 기본값.
  - `skip_generated_header = true` — 첫 ~512 byte 의 generated marker (`@generated` / `DO NOT EDIT` 등) 감지 시 skip.
  - `max_file_bytes = 262144` (256 KiB) / `max_file_lines = 5000` — 파일당 cap, 초과 시 skip.
  - `extra_skip_globs = []` — 사용자 추가 skip 패턴 (`.gitignore` 문법).
  - `.gitignore` honor: 자동 적용. `.kebabignore` 는 추가 layer. 우선순위: built-in safety net (`node_modules/` / `target/` / `__pycache__/` / `.venv/` / `venv/` / `env/`) > `.gitignore` > `.kebabignore`.
```

- [ ] **Step 3: Update HANDOFF.md**

Add a row to the phase status table:

```
| 10 | code ingest framework | 🟡 진행 중 (1A-1) | 1A-1 머지 시점 wire schema + 새 crate skeleton 동결, code chunker 는 1A-2 부터 |
```

- [ ] **Step 4: Update docs/SMOKE.md config example**

In the `/tmp/kebab-smoke/config.toml` block, append:

```toml
[ingest.code]
skip_generated_header = true
max_file_bytes = 262144
max_file_lines = 5000
```

(Default values — same as the in-code defaults. Smoke workflow doesn't need overrides; the block exists for discoverability.)

- [ ] **Step 5: Commit**

```bash
git add README.md HANDOFF.md docs/SMOKE.md
git commit -m "docs(p10-1a-1): README + HANDOFF + SMOKE — code ingest framework"
```

---

## Task 20: tasks index + p10 directory

**Files:**
- Create: `tasks/p10/INDEX.md`
- Create: `tasks/p10/p10-1a-1-code-ingest-framework.md`
- Modify: `tasks/INDEX.md`

- [ ] **Step 1: Create `tasks/p10/INDEX.md`**

```markdown
# Phase 10 — Code Ingest

| ID | Subject | Status |
|----|---------|--------|
| 1A-1 | code ingest framework (wire schema, parse-code crate skeleton, filter flags, skip policy, config 절) | 🟡 진행 중 |
| 1A-2 | Rust AST chunker | ⏳ |
| 1B | Python + TS/JS AST chunkers | ⏳ |
| 1C | Go + Java + Kotlin AST chunkers | ⏳ |
| 1D | C + C++ AST chunkers | ⏳ |
| 2 | Tier 2 resource-aware (k8s / Dockerfile / manifest) | ⏳ |
| 3 | Tier 3 paragraph + line-window fallback | ⏳ |

Design: [2026-05-15-kebab-code-ingest-design.md](../../docs/superpowers/specs/2026-05-15-kebab-code-ingest-design.md)
```

- [ ] **Step 2: Create `tasks/p10/p10-1a-1-code-ingest-framework.md`**

```markdown
# p10-1A-1 — code ingest framework

**Status:** 🟡 진행 중
**Contract sections:** §2.1 (Citation `code` variant), §2.2 (SearchHit repo/code_lang), §2.4 (IngestReport skip counters), §2 schema.v1 (code_lang_breakdown + repo_breakdown), §3.6 (Metadata fields), §8 (kebab-parse-code crate boundary), §11 (code ingest no longer 비-스코프).
**Design:** [2026-05-15-kebab-code-ingest-design.md](../../docs/superpowers/specs/2026-05-15-kebab-code-ingest-design.md) §1A-1.
**Plan:** [2026-05-15-p10-1a-1-code-ingest-framework.md](../../docs/superpowers/plans/2026-05-15-p10-1a-1-code-ingest-framework.md).

## Goal

Land the *framework surface* for code ingest — wire schema (additive minor), CLI filter flags, ignore policy, skip policy infrastructure, `kebab-parse-code` crate skeleton, `[ingest.code]` config section — without enabling any code chunker. 1A-2 plugs the Rust AST chunker on top.

## Acceptance criteria

- `cargo test --workspace --no-fail-fast -j 1` passes.
- Regression test (`wire_search_hit_no_code_fields`, `wire_citation_5_variants_unchanged`) passes — markdown corpus wire output unchanged.
- `cargo clippy --workspace --all-targets -- -D warnings` passes.
- `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md` updated per design §10.1.
- README + HANDOFF + SMOKE updated.

## Allowed dependencies

- `kebab-parse-code` may depend on `kebab-core`, `anyhow`, `gix`. NOT on store / embed / llm / rag / UI.
- Source-fs may depend on `kebab-parse-code`.

## Forbidden dependencies

- UI crates (cli / mcp / tui) must NOT import `kebab-parse-code` directly.

## Risks / notes

- `.gitignore` honor changes existing behavior for markdown corpora whose files live in gitignored areas. Regression test covers the standard case (no overlap). If a user reports missing docs after 1A-1 lands, log to HOTFIXES.
```

- [ ] **Step 3: Update `tasks/INDEX.md`** — add a phase 10 row

- [ ] **Step 4: Commit**

```bash
git add tasks/p10/ tasks/INDEX.md
git commit -m "docs(p10-1a-1): task index + framework task spec"
```

---

## Task 21: final clippy + full workspace test

- [ ] **Step 1: Run clippy across the workspace**

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: PASS. Fix any new warnings the framework code introduced.

- [ ] **Step 2: Run the full workspace test with -j 1 (per CLAUDE.md)**

Run: `cargo test --workspace --no-fail-fast -j 1`
Expected: PASS.

- [ ] **Step 3: Manually run a smoke ingest against the temp workspace**

```bash
mkdir -p /tmp/kebab-p10-smoke
cat > /tmp/kebab-p10-smoke/config.toml <<'EOF'
[workspace]
root = "/tmp/kebab-p10-smoke/notes"
EOF
mkdir -p /tmp/kebab-p10-smoke/notes
echo "# hello" > /tmp/kebab-p10-smoke/notes/a.md
cargo run --release -p kebab-cli -- --config /tmp/kebab-p10-smoke/config.toml init
cargo run --release -p kebab-cli -- --config /tmp/kebab-p10-smoke/config.toml ingest --json | jq '.skipped_gitignore, .skipped_generated, .skipped_size_exceeded'
```

Expected: All three values = `0`. Wire output includes the new fields (even when zero — verify via `--json`).

- [ ] **Step 4: `cargo clean` to recover disk** (per CLAUDE.md routine-after-merge rule, but do it now before opening the PR — keeps the work-tree light)

Optional but recommended after the test run.

- [ ] **Step 5: Final commit if any clippy fixes were needed; otherwise skip**

```bash
git commit -m "chore(p10-1a-1): final clippy pass"
```

---

## Self-Review

After all 21 tasks land, do a final sanity check before opening the PR:

**Spec coverage:**
- §2 Phase 1A-1 row → Tasks 1-21 cover every bullet in the table.
- §3.1 Citation::Code variant → Task 1 + 17.
- §3.2 SearchHit fields → Task 2 + 17.
- §3.5 Metadata extension → Task 4.
- §4 wire schema → Task 17.
- §5.1 repo detect → Task 7.
- §5.2 ignore integration → Task 9 + 10.
- §5.3 generated header → Task 8 + 12.
- §5.4 size cap → Task 8 + 12.
- §5.5 IngestReport skip counters → Task 3 + 11 + 12.
- §6 crate structure → Task 5.
- §7.1 CLI filter flags → Task 15.
- §7.2 schema stats → Task 16.
- §8 config section → Task 14.
- §10.1 frozen design update → Task 18.
- §10.4 no binary bump for 1A-1 → respected (no bump commit).

**Placeholder scan:** Search for `TBD` / `TODO` / `XXX` / `FIXME` in the plan body — none should remain. (`open question` references in the spec are intentional and don't carry forward into the plan.)

**Type consistency:**
- `Citation::Code` field names match across Task 1, 13, 17, 18.
- `SearchHit.repo` / `code_lang` match across Task 2, 13, 17.
- `Metadata` field names match Task 4 + Task 18.
- `IngestReport` skip counter names match Task 3, 11, 12, 17.
- `IngestCodeCfg` field names match Task 14, 12.

If any name drift, fix before opening the PR.
