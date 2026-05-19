# p10-1A-2 Rust AST Chunker Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Activate Rust code ingest end-to-end — `.rs` files parse via tree-sitter into one `Block::Code` per AST semantic unit, chunk 1:1 (with oversize fallback split), and surface as `Citation::Code { symbol, lang, line_start, line_end }` in search.

**Architecture:** tree-sitter lives in the **parser** (`kebab-parse-code`, per design §6.3 dependency graph; mirrors the proven PDF pattern where the parser emits structured blocks and the chunker maps them). `kebab-parse-code/src/rust.rs` is an `Extractor` producing a `CanonicalDocument` whose blocks are AST units carrying a new internal `SourceSpan::Code` variant. `kebab-chunk/src/code_rust_ast_v1.rs` maps each block to a chunk and splits oversize units. `citation_helper` gains one arm so the span flows to the existing (1A-1) `Citation::Code` wire shape. A new `MediaType::Code(String)` routes `.rs` files; non-Rust code langs stay `MediaType::Other` in 1A.

**Tech Stack:** Rust 2024 workspace, `tree-sitter` + `tree-sitter-rust`, existing `kebab-core` domain model, `serde_json_canonicalizer` + `blake3` (chunk id / policy hash), `gix` (1A-1 repo detect).

---

## Pre-flight

- [ ] **Branch.** From clean `main`:

```bash
cd /home/altair823/kebab
git checkout main && git pull
git checkout -b feat/p10-1a-2-rust-ast-chunker
```

- [ ] **Disk hygiene** (CLAUDE.md — routine after each merged PR; #139 just merged):

```bash
cargo clean
```

Notes that apply throughout:
- Full suite is always `cargo test --workspace --no-fail-fast -j 1` (CLAUDE.md — parallel link OOMs). Per-crate runs (`cargo test -p <crate>`) may run normally.
- `cargo clippy --workspace --all-targets -- -D warnings` is the CI gate; run before every commit that touches code.
- Frozen task spec for this work: `tasks/p10/p10-1a-2-rust-ast-chunker.md` (already written; do not edit retroactively).

---

## Task 1: Add tree-sitter dependencies (workspace-deps pattern)

**Files:**
- Modify: `Cargo.toml` (workspace `[workspace.dependencies]`, ends ~line 90 after the `gix` entry)
- Modify: `crates/kebab-parse-code/Cargo.toml` (`[dependencies]`)

- [ ] **Step 1: Resolve + pin versions via cargo add**

Run (this picks the latest compatible versions and writes them into the crate):

```bash
cargo add tree-sitter tree-sitter-rust -p kebab-parse-code
```

- [ ] **Step 2: Move the resolved versions into workspace deps**

Read the two version strings `cargo add` wrote into `crates/kebab-parse-code/Cargo.toml`. Then in the workspace `Cargo.toml`, append to `[workspace.dependencies]` directly after the `gix = { ... }` line (keep the existing comment style — one explanatory comment line):

```toml
# Rust source parsing for code ingest (kebab-parse-code, p10-1A-2). The
# chunker stays tree-sitter-free — AST work is parser-side per design §6.3.
tree-sitter      = "<resolved major.minor>"
tree-sitter-rust = "<resolved major.minor>"
```

Then rewrite the crate's `[dependencies]` to use the workspace table (matching the existing `anyhow`/`gix` style):

```toml
[dependencies]
anyhow           = { workspace = true }
gix              = { workspace = true }
tree-sitter      = { workspace = true }
tree-sitter-rust = { workspace = true }

[dev-dependencies]
tempfile = { workspace = true }
```

- [ ] **Step 3: Verify it builds and the lock updated**

Run: `cargo build -p kebab-parse-code`
Expected: compiles clean (skeleton still has no tree-sitter use yet — deps unused is fine, no `-D warnings` on a plain build).

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml Cargo.lock crates/kebab-parse-code/Cargo.toml
git commit -m "build(p10-1a-2): add tree-sitter + tree-sitter-rust workspace deps

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: `SourceSpan::Code` internal variant

**Files:**
- Modify: `crates/kebab-core/src/document.rs` (`SourceSpan` enum, after the `Time { start_ms, end_ms }` variant ~line 144)
- Modify: `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md` (§3.4 `SourceSpan` enum listing, ~line 562)
- Test: `crates/kebab-core/src/document.rs` (`#[cfg(test)] mod tests`, or the file's existing test module)

`SourceSpan` is `#[serde(rename_all = "lowercase", tag = "kind")]` — the new variant serializes as `{"kind":"code", ...}`. This is the chunks-table `source_spans_json` internal shape, NOT a wire schema (wire `Citation::Code` already shipped in 1A-1), so no wire `.v2` bump.

- [ ] **Step 1: Write the failing test**

Add to the `kebab-core` test module that covers `SourceSpan` (search `mod tests` in `document.rs`; if none, add one at end of file):

```rust
#[test]
fn source_span_code_round_trips_and_tags_lowercase() {
    let s = SourceSpan::Code {
        line_start: 10,
        line_end: 42,
        symbol: Some("foo::Bar::baz".to_string()),
        lang: Some("rust".to_string()),
    };
    let v = serde_json::to_value(&s).unwrap();
    assert_eq!(v["kind"], "code");
    assert_eq!(v["line_start"], 10);
    assert_eq!(v["line_end"], 42);
    assert_eq!(v["symbol"], "foo::Bar::baz");
    assert_eq!(v["lang"], "rust");
    let back: SourceSpan = serde_json::from_value(v).unwrap();
    assert_eq!(back, s);
}
```

- [ ] **Step 2: Run it — expect compile failure**

Run: `cargo test -p kebab-core source_span_code_round_trips`
Expected: FAIL — `no variant named Code`.

- [ ] **Step 3: Add the variant**

In `crates/kebab-core/src/document.rs`, add as the last variant of `pub enum SourceSpan` (after `Time { ... }`):

```rust
    /// p10-1A-2: AST-unit span for code ingest. Internal storage shape
    /// (chunks.source_spans_json) — `citation_helper` maps this to the
    /// wire `Citation::Code` (added 1A-1). `symbol` is the per-language
    /// self-reference path (design §3.4); `<top-level>` / `<module>` for
    /// glue regions, never null for an identified unit. `lang` is the
    /// canonical code_lang.
    Code {
        line_start: u32,
        line_end: u32,
        symbol: Option<String>,
        lang: Option<String>,
    },
```

- [ ] **Step 4: Compile — fix every non-exhaustive match**

Run: `cargo build --workspace 2>&1 | grep -A2 "non-exhaustive\|E0004"`

The compiler will flag every exhaustive `match` on `SourceSpan`. Known sites (handle each minimally — a `Code` arm that does the type-correct thing, NOT a catch-all `_`):
- `crates/kebab-search/src/citation_helper.rs` — handled fully in Task 3; for now add a temporary `SourceSpan::Code { .. } => Citation::Line { path, start: 1, end: 1, section }` arm with a `// TODO(Task 3)` and replace it in Task 3.
- Any `store-sqlite` / `search` / `id` site: add a faithful arm (e.g. id recipe already serializes `SourceSpan` via serde — likely no match there; only fix real `match` statements the compiler points at).

Run: `cargo build --workspace`
Expected: clean.

- [ ] **Step 5: Run the test**

Run: `cargo test -p kebab-core source_span_code_round_trips`
Expected: PASS.

- [ ] **Step 6: Sync frozen design §3.4**

In `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md`, find `pub enum SourceSpan` (~line 562) and add the `Code { line_start, line_end, symbol, lang }` variant to the listing, with a one-line comment `// p10-1A-2: internal code-unit span (see tasks/p10/p10-1a-2)`. Do not alter other variants.

- [ ] **Step 7: clippy + commit**

```bash
cargo clippy --workspace --all-targets -- -D warnings
git add crates/ docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
git commit -m "feat(p10-1a-2): add internal SourceSpan::Code variant + design §3.4 sync

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: `citation_helper` Code arm

**Files:**
- Modify: `crates/kebab-search/src/citation_helper.rs:21-74`
- Test: `crates/kebab-search/src/citation_helper.rs` (add `#[cfg(test)] mod tests` if absent; `lexical.rs` has `build_citation_*` tests — mirror style there if a test module already imports the helper)

- [ ] **Step 1: Write the failing test**

Add a test (in `citation_helper.rs` add a test module, or extend the existing helper tests in `crates/kebab-search/src/lexical.rs` near `build_citation_page_forwards_section`):

```rust
#[test]
fn build_citation_code_maps_symbol_and_lang() {
    use kebab_core::{Citation, SourceSpan, WorkspacePath};
    let span = SourceSpan::Code {
        line_start: 5,
        line_end: 30,
        symbol: Some("chunk::md_heading_v1::MdHeadingV1Chunker::chunk".into()),
        lang: Some("rust".into()),
    };
    let c = super::citation_from_first_span(
        "c1",
        WorkspacePath("crates/kebab-chunk/src/md_heading_v1.rs".into()),
        None,
        Some(&span),
    );
    match c {
        Citation::Code { path, line_start, line_end, symbol, lang } => {
            assert_eq!(path.0, "crates/kebab-chunk/src/md_heading_v1.rs");
            assert_eq!(line_start, 5);
            assert_eq!(line_end, 30);
            assert_eq!(symbol.as_deref(), Some("chunk::md_heading_v1::MdHeadingV1Chunker::chunk"));
            assert_eq!(lang.as_deref(), Some("rust"));
        }
        other => panic!("expected Citation::Code, got {other:?}"),
    }
}
```

- [ ] **Step 2: Run — expect fail**

Run: `cargo test -p kebab-search build_citation_code_maps_symbol_and_lang`
Expected: FAIL (currently the Task-2 temporary arm produces `Citation::Line`).

- [ ] **Step 3: Replace the temporary arm**

In `citation_from_first_span`, replace the Task-2 placeholder arm with the real mapping (place it directly after the `SourceSpan::Time` arm, before the `Byte | None` fallback):

```rust
        Some(SourceSpan::Code { line_start, line_end, symbol, lang }) => Citation::Code {
            path,
            line_start: *line_start,
            line_end: *line_end,
            symbol: symbol.clone(),
            lang: lang.clone(),
        },
```

(`section` is unused for code — `Citation::Code` has no section field; this matches the spec's code citation shape.)

- [ ] **Step 4: Run — expect pass**

Run: `cargo test -p kebab-search build_citation_code_maps_symbol_and_lang`
Expected: PASS.

- [ ] **Step 5: clippy + commit**

```bash
cargo clippy -p kebab-search --all-targets -- -D warnings
git add crates/kebab-search/
git commit -m "feat(p10-1a-2): map SourceSpan::Code -> Citation::Code in citation_helper

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: `MediaType::Code(String)` variant

**Files:**
- Modify: `crates/kebab-core/src/media.rs:38-44` (`MediaType` enum)
- Modify: `crates/kebab-app/src/ingest_progress.rs:99` (`media_label`)
- Modify: `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md` (only if `enum MediaType` is enumerated there — `grep -n "enum MediaType" docs/superpowers/specs/2026-04-27-kebab-final-form-design.md`; if present, add the `Code(String)` variant + one-line comment, else skip)
- Test: `crates/kebab-core/src/media.rs` test module

`MediaType` is `#[serde(rename_all = "lowercase")]`; `Code(String)` serializes as `{"code":"rust"}`.

- [ ] **Step 1: Write the failing test**

Add to `media.rs` tests:

```rust
#[test]
fn media_type_code_serializes_lowercase_tagged() {
    let m = MediaType::Code("rust".to_string());
    let v = serde_json::to_value(&m).unwrap();
    assert_eq!(v, serde_json::json!({ "code": "rust" }));
    let back: MediaType = serde_json::from_value(v).unwrap();
    assert_eq!(back, m);
}
```

- [ ] **Step 2: Run — expect fail**

Run: `cargo test -p kebab-core media_type_code_serializes`
Expected: FAIL — `no variant named Code`.

- [ ] **Step 3: Add the variant + media_label arm**

In `crates/kebab-core/src/media.rs`, add to `pub enum MediaType` immediately before `Other(String)`:

```rust
    /// p10-1A-2: a source-code file. Inner string is the canonical
    /// code_lang (design §3.5). 1A activates `"rust"` only; other
    /// recognized code langs are still routed `Other` until their phase.
    Code(String),
```

In `crates/kebab-app/src/ingest_progress.rs`, add a match arm next to the `MediaType::Other(_) => "other"` arm (~line 99):

```rust
        kebab_core::MediaType::Code(_) => "code",
```

Then `cargo build --workspace 2>&1 | grep non-exhaustive` and add a faithful arm to every other `MediaType` match the compiler flags (e.g. any UI/store display) — `Code(lang)` should render analogously to `Other`.

- [ ] **Step 4: Run — expect pass + suite green**

Run: `cargo test -p kebab-core media_type_code_serializes`
Expected: PASS.
Run: `cargo test --workspace --no-fail-fast -j 1`
Expected: PASS (catches any golden/asset serialization that enumerates MediaType variants; fix faithfully if any fixture counts variants).

- [ ] **Step 5: design sync (conditional) + clippy + commit**

```bash
grep -n "enum MediaType" docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
# if present, add Code(String) to that listing with a p10-1A-2 comment
cargo clippy --workspace --all-targets -- -D warnings
git add crates/ docs/
git commit -m "feat(p10-1a-2): add MediaType::Code(lang) variant

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 5: Route `.rs` → `MediaType::Code("rust")`

**Files:**
- Modify: `crates/kebab-source-fs/src/media.rs:39` (the `_ => MediaType::Other(ext)` fallthrough)
- Test: `crates/kebab-source-fs/src/media.rs` test module (`media_type_for` tests, ~line 49)

Scope to Rust only (1A-2 = Rust). Non-Rust extensions keep their current `MediaType::Other` mapping — minimal blast radius, regression-safe.

- [ ] **Step 1: Write the failing test**

Add near the existing `media_type_for` asserts:

```rust
#[test]
fn rust_files_map_to_media_code_rust() {
    assert_eq!(
        media_type_for(Path::new("crates/kebab-core/src/lib.rs")),
        MediaType::Code("rust".to_string())
    );
    // non-Rust code extensions stay Other in 1A
    assert_eq!(media_type_for(Path::new("a/b.py")), MediaType::Other("py".to_string()));
    assert_eq!(media_type_for(Path::new("Cargo.toml")), MediaType::Other("toml".to_string()));
}
```

- [ ] **Step 2: Run — expect fail**

Run: `cargo test -p kebab-source-fs rust_files_map_to_media_code_rust`
Expected: FAIL — `.rs` currently → `MediaType::Other("rs")`.

- [ ] **Step 3: Add the routing arm**

In `crates/kebab-source-fs/src/media.rs`, add an `"rs"` arm before the final `_ => MediaType::Other(ext)`:

```rust
        // p10-1A-2: Rust is the only code lang activated in 1A. Other
        // recognized code langs stay Other until their phase (1B+).
        "rs" => MediaType::Code("rust".to_string()),
```

- [ ] **Step 4: Run — expect pass**

Run: `cargo test -p kebab-source-fs rust_files_map_to_media_code_rust`
Expected: PASS.

- [ ] **Step 5: clippy + commit**

```bash
cargo clippy -p kebab-source-fs --all-targets -- -D warnings
git add crates/kebab-source-fs/
git commit -m "feat(p10-1a-2): route .rs files to MediaType::Code(rust)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 6: `kebab-parse-code` Rust AST extractor

**Files:**
- Create: `crates/kebab-parse-code/src/rust.rs`
- Modify: `crates/kebab-parse-code/src/lib.rs` (add `pub mod rust;` + re-export `PARSER_VERSION`, `RustAstExtractor`)
- Create: `crates/kebab-parse-code/tests/fixtures/sample.rs` (Rust fixture)
- Test: inline `#[cfg(test)] mod tests` in `rust.rs`

This is the core. The extractor walks the tree-sitter parse tree and emits **one `Block::Code` per top-level AST semantic unit**, each with `SourceSpan::Code`. The `CanonicalDocument` scaffold (doc_id, provenance, metadata, return struct) **mirrors `crates/kebab-parse-pdf/src/lib.rs:51-225` exactly** — same `Extractor` trait impl shape, same `id_for_doc` / `ProvenanceEvent` / `CanonicalDocument` construction. Only the differences below change.

### tree-sitter API note

Use the modern API: `parser.set_language(&tree_sitter_rust::LANGUAGE.into())`. If the resolved `tree-sitter-rust` predates the `LANGUAGE: LanguageFn` const (no `LANGUAGE` symbol), use `parser.set_language(&tree_sitter_rust::language())` instead. Verify which by `cargo doc -p tree-sitter-rust --no-deps` or reading its docs; pick the one that compiles.

### Semantic-unit rules (design §9.1 + §3.4)

Walk `root_node()` (kind `source_file`) named children. Maintain a `mod_path: Vec<String>` (module nesting), starting empty.

| node kind | unit | symbol |
|-----------|------|--------|
| `function_item` | 1 | `mod_path::fn_name` |
| `struct_item` / `enum_item` / `union_item` / `trait_item` / `type_item` | 1 | `mod_path::TypeName` |
| `macro_definition` | 1 | `mod_path::name!` |
| `impl_item` | 1 per inner `function_item` | `mod_path::ImplType::method` (ImplType = text of impl `type` field; for `impl Trait for T`, use `Trait::method` per §3.4) |
| `mod_item` **with** `declaration_list` body | recurse with `mod_path` + mod name pushed | — |
| `use_declaration`, `extern_crate_declaration`, `const_item`, `static_item`, `mod_item` **without** body, top-level `attribute_item`/`macro_invocation` | accumulated into ONE grouped unit | `<top-level>` (or `<module>` if the whole file produced no fn/type/impl unit and the group is only `mod_item` declarations) |

- **Line range:** `node.start_position().row + 1 ..= node.end_position().row + 1` (1-based inclusive). Extend `line_start` upward over contiguous immediately-preceding sibling `line_comment` / `block_comment` / `attribute_item` nodes (doc comments + attributes belong to their item — design §9.1 "선언 + doc comment").
- **Grouped unit line range:** min start_line over the group .. max end_line over the group.
- **`code` field:** the exact source substring for those lines (split `source` by `\n`, take `[line_start-1 ..= line_end-1]`, rejoin with `\n`).
- Each unit → `Block::Code(CodeBlock { common: CommonBlock { block_id, heading_path: vec![], source_span }, lang: Some("rust".into()), code })` where `source_span = SourceSpan::Code { line_start, line_end, symbol: Some(sym), lang: Some("rust".into()) }`. `block_id = id_for_block(&doc_id, "code", &[], ordinal, &source_span)` with `ordinal` = 0-based unit index.

- [ ] **Step 1: Create the fixture**

Create `crates/kebab-parse-code/tests/fixtures/sample.rs`:

```rust
//! sample fixture

use std::fmt;

const ANSWER: u32 = 42;

/// Doc comment on a free fn.
pub fn parse(input: &str) -> usize {
    input.len()
}

pub struct Foo {
    pub n: u32,
}

impl Foo {
    /// method doc
    pub fn double(&self) -> u32 {
        self.n * 2
    }

    fn name() -> &'static str {
        "foo"
    }
}

pub trait Greet {
    fn hello(&self) -> String;
}

mod inner {
    pub fn helper() -> bool {
        true
    }
}
```

- [ ] **Step 2: Write the failing test**

In `rust.rs` add:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use kebab_core::{Block, MediaType, SourceSpan};

    fn extract_fixture() -> kebab_core::CanonicalDocument {
        let bytes = std::fs::read(
            concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/sample.rs"),
        )
        .unwrap();
        let asset = kebab_parse_code_test_support::fixed_rust_asset("crates/x/src/sample.rs");
        let cfg = kebab_core::ExtractConfig::default();
        let root = std::path::PathBuf::from("/tmp");
        let ctx = kebab_core::ExtractContext { asset: &asset, workspace_root: &root, config: &cfg };
        RustAstExtractor::new().extract(&ctx, &bytes).unwrap()
    }

    #[test]
    fn extractor_supports_only_media_code_rust() {
        let e = RustAstExtractor::new();
        assert!(e.supports(&MediaType::Code("rust".into())));
        assert!(!e.supports(&MediaType::Code("python".into())));
        assert!(!e.supports(&MediaType::Markdown));
    }

    #[test]
    fn emits_one_block_per_semantic_unit_with_symbols() {
        let doc = extract_fixture();
        let mut syms: Vec<(String, u32, u32)> = doc
            .blocks
            .iter()
            .map(|b| match b {
                Block::Code(c) => match &c.common.source_span {
                    SourceSpan::Code { symbol, line_start, line_end, lang } => {
                        assert_eq!(lang.as_deref(), Some("rust"));
                        (symbol.clone().unwrap(), *line_start, *line_end)
                    }
                    _ => panic!("code block must carry SourceSpan::Code"),
                },
                other => panic!("expected Block::Code, got {other:?}"),
            })
            .collect();
        syms.sort();
        let names: Vec<&str> = syms.iter().map(|(s, _, _)| s.as_str()).collect();
        assert!(names.contains(&"parse"));
        assert!(names.contains(&"Foo"));
        assert!(names.contains(&"Foo::double"));
        assert!(names.contains(&"Foo::name"));
        assert!(names.contains(&"Greet"));
        assert!(names.contains(&"inner::helper"));
        assert!(names.contains(&"<top-level>")); // use + const grouped
        // doc-comment line is folded into the unit it documents:
        let parse_unit = syms.iter().find(|(s, _, _)| s == "parse").unwrap();
        let parse_src = doc.blocks.iter().find_map(|b| match b {
            Block::Code(c) if matches!(&c.common.source_span, SourceSpan::Code{symbol,..} if symbol.as_deref()==Some("parse")) => Some(c.code.clone()),
            _ => None,
        }).unwrap();
        assert!(parse_src.contains("/// Doc comment on a free fn."), "doc comment folded in: {parse_src}");
        let _ = parse_unit;
    }

    #[test]
    fn deterministic_across_runs() {
        let a = extract_fixture();
        for _ in 0..50 {
            assert_eq!(extract_fixture().blocks, a.blocks);
        }
    }
}

#[cfg(test)]
mod kebab_parse_code_test_support {
    use kebab_core::*;
    use time::OffsetDateTime;
    pub fn fixed_rust_asset(path: &str) -> RawAsset {
        RawAsset {
            asset_id: AssetId("a".repeat(64)),
            source_uri: SourceUri::File(std::path::PathBuf::from(path)),
            workspace_path: WorkspacePath(path.to_string()),
            media_type: MediaType::Code("rust".to_string()),
            byte_len: 0,
            checksum: Checksum("b".repeat(64)),
            discovered_at: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
            stored: AssetStorage::InPlace,
        }
    }
}
```

> Before relying on it, verify `Checksum`/`SourceUri`/`AssetStorage` field shapes by reading `crates/kebab-core/src/asset.rs:63-95`; adjust the test-support constructor to the actual variants (e.g. `AssetStorage` may be `External`/`InPlace` — use whatever exists). This is a fixture-construction detail, not a design choice.

- [ ] **Step 3: Run — expect fail**

Run: `cargo test -p kebab-parse-code emits_one_block_per_semantic_unit`
Expected: FAIL — `RustAstExtractor` undefined.

- [ ] **Step 4: Implement `rust.rs`**

Create `crates/kebab-parse-code/src/rust.rs`. Scaffold (Extractor impl, doc_id, provenance events, final `CanonicalDocument {…}`) is a **direct adaptation of `crates/kebab-parse-pdf/src/lib.rs:51-225`** with these concrete differences:

- `pub const PARSER_VERSION: &str = "code-rust-v1";`
- `pub struct RustAstExtractor;` + `new()`/`Default` like `PdfTextExtractor`.
- `fn supports(&self, m: &MediaType) -> bool { matches!(m, MediaType::Code(l) if l == "rust") }`
- agent strings: `"kb-parse-code"` instead of `"kb-parse-pdf"`.
- `title`: filename stem of `asset.workspace_path` (reuse the same `strip_extension(filename_from_workspace_path(...))` helpers — copy those two small fns from `kebab-parse-pdf/src/lib.rs:229+` into `rust.rs`, or inline equivalent).
- `lang: Lang("und".into())` (natural-language detection out of scope, design §3.5).
- **metadata**: same `Metadata { … }` literal as PDF but set:
  - `source_type`: use `SourceType::Code` if the enum has it (`grep -n "enum SourceType" crates/kebab-core/src/metadata.rs`), else `SourceType::Note`.
  - `code_lang: Some("rust".to_string())`
  - `repo` / `git_branch` / `git_commit`: from `crate::repo::detect_repo`. Resolve the file's absolute path: if `asset.source_uri` is `SourceUri::File(p)` use `p`; join with `ctx.workspace_root` if relative. `match detect_repo(&abs) { Some(r) => (Some(r.name), r.branch, r.commit), None => (None, None, None) }`.
- **blocks**: replace the PDF per-page loop with the AST walk. Implementation:

```rust
fn build_blocks(
    source: &str,
    doc_id: &kebab_core::DocumentId,
) -> anyhow::Result<Vec<kebab_core::Block>> {
    use kebab_core::{Block, CodeBlock, CommonBlock, SourceSpan, id_for_block};

    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .map_err(|e| anyhow::anyhow!("set tree-sitter-rust language: {e}"))?;
    let tree = parser
        .parse(source.as_bytes(), None)
        .ok_or_else(|| anyhow::anyhow!("tree-sitter failed to parse Rust source"))?;
    let lines: Vec<&str> = source.split('\n').collect();

    // (symbol, start_line_1based, end_line_1based) in document order.
    let mut units: Vec<(String, u32, u32)> = Vec::new();
    // Pending glue (use/const/static/mod-decl/attr) accumulated into one
    // <top-level> (or <module>) unit, flushed when a real unit appears or
    // at end of a scope.
    let mut glue: Vec<(usize, u32, u32)> = Vec::new(); // (is_mod_decl as 0/1 via usize, s, e)

    fn node_name<'a>(n: &tree_sitter::Node, src: &'a str) -> Option<&'a str> {
        n.child_by_field_name("name")
            .map(|c| &src[c.start_byte()..c.end_byte()])
    }
    // Extend start upward over leading doc-comments / attributes.
    fn unit_start(n: &tree_sitter::Node) -> u32 {
        let mut start = n.start_position().row as u32 + 1;
        let mut prev = n.prev_sibling();
        while let Some(p) = prev {
            let k = p.kind();
            if k == "line_comment" || k == "block_comment" || k == "attribute_item" {
                start = p.start_position().row as u32 + 1;
                prev = p.prev_sibling();
            } else {
                break;
            }
        }
        start
    }

    fn walk(
        node: tree_sitter::Node,
        src: &str,
        mod_path: &[String],
        units: &mut Vec<(String, u32, u32)>,
        glue: &mut Vec<(usize, u32, u32)>,
    ) {
        let mut cur = node.walk();
        for child in node.named_children(&mut cur) {
            let s = unit_start(&child);
            let e = child.end_position().row as u32 + 1;
            let prefix = if mod_path.is_empty() {
                String::new()
            } else {
                format!("{}::", mod_path.join("::"))
            };
            match child.kind() {
                "function_item" | "struct_item" | "enum_item" | "union_item"
                | "trait_item" | "type_item" => {
                    if let Some(name) = node_name(&child, src) {
                        flush_glue(glue, units);
                        units.push((format!("{prefix}{name}"), s, e));
                    }
                }
                "macro_definition" => {
                    if let Some(name) = node_name(&child, src) {
                        flush_glue(glue, units);
                        units.push((format!("{prefix}{name}!"), s, e));
                    }
                }
                "impl_item" => {
                    flush_glue(glue, units);
                    let ty = child
                        .child_by_field_name("type")
                        .map(|c| src[c.start_byte()..c.end_byte()].trim().to_string());
                    let tr = child
                        .child_by_field_name("trait")
                        .map(|c| src[c.start_byte()..c.end_byte()].trim().to_string());
                    let owner = tr.or(ty).unwrap_or_else(|| "<impl>".to_string());
                    if let Some(body) = child.child_by_field_name("body") {
                        let mut bc = body.walk();
                        for m in body.named_children(&mut bc) {
                            if m.kind() == "function_item" {
                                if let Some(mn) = node_name(&m, src) {
                                    let ms = unit_start(&m);
                                    let me = m.end_position().row as u32 + 1;
                                    units.push((format!("{prefix}{owner}::{mn}"), ms, me));
                                }
                            }
                        }
                    }
                }
                "mod_item" => {
                    if let Some(body) = child.child_by_field_name("body") {
                        flush_glue(glue, units);
                        let name = node_name(&child, src).unwrap_or("mod").to_string();
                        let mut np = mod_path.to_vec();
                        np.push(name);
                        walk(body, src, &np, units, glue);
                    } else {
                        glue.push((1, s, e)); // bare `mod foo;` declaration
                    }
                }
                "use_declaration" | "extern_crate_declaration" | "const_item"
                | "static_item" | "attribute_item" | "macro_invocation" => {
                    glue.push((0, s, e));
                }
                _ => { /* line_comment / block_comment / unknown: ignore (folded via unit_start) */ }
            }
        }
        flush_glue(glue, units);
    }

    fn flush_glue(glue: &mut Vec<(usize, u32, u32)>, units: &mut Vec<(String, u32, u32)>) {
        if glue.is_empty() {
            return;
        }
        let s = glue.iter().map(|(_, a, _)| *a).min().unwrap();
        let e = glue.iter().map(|(_, _, b)| *b).max().unwrap();
        let only_mod_decls = glue.iter().all(|(is_mod, _, _)| *is_mod == 1);
        let sym = if only_mod_decls { "<module>" } else { "<top-level>" };
        units.push((sym.to_string(), s, e));
        glue.clear();
    }

    walk(tree.root_node(), source, &[], &mut units, &mut glue);

    let total_lines = lines.len() as u32;
    let mut blocks = Vec::with_capacity(units.len());
    for (ordinal, (symbol, ls, le)) in units.into_iter().enumerate() {
        let line_start = ls.max(1);
        let line_end = le.min(total_lines.max(1));
        let span = SourceSpan::Code {
            line_start,
            line_end,
            symbol: Some(symbol),
            lang: Some("rust".to_string()),
        };
        let block_id = id_for_block(doc_id, "code", &[], ordinal as u32, &span);
        let code = lines[(line_start as usize - 1)..=(line_end as usize - 1)].join("\n");
        blocks.push(Block::Code(CodeBlock {
            common: CommonBlock {
                block_id,
                heading_path: Vec::new(),
                source_span: span,
            },
            lang: Some("rust".to_string()),
            code,
        }));
    }
    Ok(blocks)
}
```

Notes for the implementer:
- `flush_glue` ordering: glue flushed *before* pushing a real unit so document order is preserved (glue that precedes the first fn becomes the `<top-level>` chunk spanning the `use`/`const` region; the `unit_start` doc-comment extension keeps the fn's own doc comment with the fn, not the glue).
- A `glue` flushed after a real unit between two fns is still a valid `<top-level>` unit (rare; acceptable).
- If `units` is empty (e.g. an empty file) → emit zero blocks (consistent with empty-PDF-page behavior).
- The `e` of a fixture's last `mod inner { … }` etc. is end-of-block; line slicing uses inclusive 1-based.

- [ ] **Step 5: Wire into lib.rs**

In `crates/kebab-parse-code/src/lib.rs`:

```rust
pub mod rust;
pub use rust::{PARSER_VERSION as RUST_PARSER_VERSION, RustAstExtractor};
```

- [ ] **Step 6: Run the tests — expect pass**

Run: `cargo test -p kebab-parse-code`
Expected: PASS (`extractor_supports_*`, `emits_one_block_per_semantic_unit_with_symbols`, `deterministic_across_runs`).
If symbol names mismatch (tree-sitter-rust grammar field-name drift, e.g. `impl_item` `type` vs `type_arguments`), inspect with a scratch `node.kind()`/`field` dump and adjust the field names; pin behavior with the test.

- [ ] **Step 7: clippy + commit**

```bash
cargo clippy -p kebab-parse-code --all-targets -- -D warnings
git add crates/kebab-parse-code/
git commit -m "feat(p10-1a-2): tree-sitter-rust AST extractor (parser-side)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 7: `code-rust-ast-v1` chunker

**Files:**
- Create: `crates/kebab-chunk/src/code_rust_ast_v1.rs`
- Modify: `crates/kebab-chunk/src/lib.rs` (`mod` + `pub use`)
- Test: inline `#[cfg(test)] mod tests` in the new file

The chunker consumes the AST `CanonicalDocument` and maps **1 `Block::Code` → 1 `Chunk`**, except a unit longer than `AST_CHUNK_MAX_LINES` is split into `[part i/N]` sub-chunks. tree-sitter is NOT imported here (forbidden — AST is parser-side). Mirror `crates/kebab-chunk/src/pdf_page_v1.rs` for: `VERSION_LABEL` const, `BYTES_PER_TOKEN = 3`, `POLICY_HASH_HEX_LEN = 16`, `policy_hash` impl (identical blake3 recipe — cross-chunker fingerprint identity is required), per-chunk `policy_hash` variant to avoid `chunk_id` collision on split units, the upfront block-shape validation that `bail!`s on a non-code doc.

`AST_CHUNK_MAX_LINES` is a module constant (`= 200`) matching `IngestCodeCfg::default().ast_chunk_max_lines`. Threading the config value through the fixed `Chunker` trait needs a per-medium chunker registry — a P+ task; this mirrors the existing `pdf-page-v1` "chunker_version hard-coded" deviation. Record it (Task 11 HOTFIXES).

- [ ] **Step 1: Write failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use kebab_core::{
        Block, CanonicalDocument, ChunkPolicy, Chunker, ChunkerVersion, CodeBlock, CommonBlock,
        SourceSpan, id_for_block, id_for_doc, AssetId, Lang, Metadata, ParserVersion, Provenance,
        SourceType, TrustLevel, WorkspacePath,
    };
    use time::OffsetDateTime;

    fn code_doc(units: &[(&str, u32, u32, &str)]) -> CanonicalDocument {
        let wp = WorkspacePath("crates/x/src/a.rs".into());
        let aid = AssetId("a".repeat(64));
        let pv = ParserVersion("code-rust-v1".into());
        let doc_id = id_for_doc(&wp, &aid, &pv);
        let blocks = units
            .iter()
            .enumerate()
            .map(|(i, (sym, ls, le, code))| {
                let span = SourceSpan::Code {
                    line_start: *ls,
                    line_end: *le,
                    symbol: Some((*sym).to_string()),
                    lang: Some("rust".into()),
                };
                let bid = id_for_block(&doc_id, "code", &[], i as u32, &span);
                Block::Code(CodeBlock {
                    common: CommonBlock { block_id: bid, heading_path: vec![], source_span: span },
                    lang: Some("rust".into()),
                    code: (*code).to_string(),
                })
            })
            .collect();
        CanonicalDocument {
            doc_id, source_asset_id: aid, workspace_path: wp, title: "a".into(),
            lang: Lang("und".into()), blocks,
            metadata: Metadata {
                aliases: vec![], tags: vec![],
                created_at: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
                updated_at: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
                source_type: SourceType::Note, trust_level: TrustLevel::Primary,
                user_id_alias: None, user: Default::default(),
                repo: Some("kebab".into()), git_branch: Some("main".into()),
                git_commit: Some("0".repeat(40)), code_lang: Some("rust".into()),
            },
            provenance: Provenance { events: vec![] },
            parser_version: pv, schema_version: 1, doc_version: 1,
            last_chunker_version: None, last_embedding_version: None,
        }
    }
    fn policy() -> ChunkPolicy {
        ChunkPolicy { target_tokens: 500, overlap_tokens: 80,
            respect_markdown_headings: false,
            chunker_version: ChunkerVersion(VERSION_LABEL.into()) }
    }

    #[test]
    fn chunker_version_is_code_rust_ast_v1() {
        assert_eq!(CodeRustAstV1Chunker.chunker_version(),
            ChunkerVersion("code-rust-ast-v1".into()));
    }

    #[test]
    fn one_chunk_per_unit_preserves_code_span() {
        let doc = code_doc(&[
            ("parse", 1, 3, "pub fn parse() {}\n// x\n}"),
            ("Foo::double", 5, 7, "fn double() {}\n//\n}"),
        ]);
        let chunks = CodeRustAstV1Chunker.chunk(&doc, &policy()).unwrap();
        assert_eq!(chunks.len(), 2);
        for c in &chunks {
            assert_eq!(c.source_spans.len(), 1);
            assert!(matches!(c.source_spans[0], SourceSpan::Code { .. }));
            assert_eq!(c.heading_path, Vec::<String>::new());
            assert_eq!(c.chunker_version.0, "code-rust-ast-v1");
        }
        match &chunks[0].source_spans[0] {
            SourceSpan::Code { symbol, line_start, line_end, .. } => {
                assert_eq!(symbol.as_deref(), Some("parse"));
                assert_eq!((*line_start, *line_end), (1, 3));
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn oversize_unit_splits_into_parts_with_unique_ids() {
        // 500-line fn → must split (AST_CHUNK_MAX_LINES = 200).
        let body = (0..500).map(|i| format!("    let x{i} = {i};")).collect::<Vec<_>>().join("\n");
        let code = format!("pub fn big() {{\n{body}\n}}");
        let doc = code_doc(&[("big", 1, 502, &code)]);
        let chunks = CodeRustAstV1Chunker.chunk(&doc, &policy()).unwrap();
        assert!(chunks.len() >= 2, "oversize unit must split, got {}", chunks.len());
        for c in &chunks {
            match &c.source_spans[0] {
                SourceSpan::Code { symbol, .. } => {
                    assert!(symbol.as_deref().unwrap().starts_with("big [part "),
                        "part-numbered symbol, got {symbol:?}");
                }
                _ => unreachable!(),
            }
        }
        let mut ids: Vec<&str> = chunks.iter().map(|c| c.chunk_id.0.as_str()).collect();
        let n = ids.len(); ids.sort(); ids.dedup();
        assert_eq!(ids.len(), n, "chunk_ids unique across split parts");
    }

    #[test]
    fn non_code_doc_errors() {
        use kebab_core::TextBlock;
        let mut doc = code_doc(&[("parse", 1, 1, "fn parse(){}")]);
        doc.blocks = vec![Block::Paragraph(TextBlock {
            common: CommonBlock {
                block_id: kebab_core::BlockId("b".into()),
                heading_path: vec![],
                source_span: SourceSpan::Line { start: 1, end: 1 },
            },
            text: "x".into(), inlines: vec![],
        })];
        let err = CodeRustAstV1Chunker.chunk(&doc, &policy()).unwrap_err();
        assert!(err.to_string().contains("CodeRustAstV1Chunker"));
    }

    #[test]
    fn deterministic_chunk_ids_1000() {
        let doc = code_doc(&[("parse", 1, 2, "fn parse(){}\n}")]);
        let base: Vec<String> = CodeRustAstV1Chunker.chunk(&doc, &policy())
            .unwrap().into_iter().map(|c| c.chunk_id.0).collect();
        for _ in 0..1000 {
            let again: Vec<String> = CodeRustAstV1Chunker.chunk(&doc, &policy())
                .unwrap().into_iter().map(|c| c.chunk_id.0).collect();
            assert_eq!(again, base);
        }
    }

    #[test]
    fn policy_hash_matches_md_heading_v1() {
        let p = policy();
        assert_eq!(CodeRustAstV1Chunker.policy_hash(&p),
            crate::MdHeadingV1Chunker.policy_hash(&p));
    }
}
```

- [ ] **Step 2: Run — expect fail**

Run: `cargo test -p kebab-chunk code_rust_ast`
Expected: FAIL — `CodeRustAstV1Chunker` undefined.

- [ ] **Step 3: Implement the chunker**

Create `crates/kebab-chunk/src/code_rust_ast_v1.rs`:

```rust
//! `code-rust-ast-v1` — maps a tree-sitter-derived Rust AST
//! `CanonicalDocument` (one `Block::Code` per semantic unit, each with
//! `SourceSpan::Code`) to chunks 1:1. A unit longer than
//! `AST_CHUNK_MAX_LINES` is split into `<symbol> [part i/N]` sub-chunks
//! at blank-line paragraph boundaries (design §9.1 oversize fallback).
//!
//! tree-sitter is intentionally NOT a dependency here: AST work is
//! parser-side (`kebab-parse-code`, design §6.3). This chunker only
//! consumes the `CanonicalDocument`.
//!
//! `AST_CHUNK_MAX_LINES` is a constant matching
//! `IngestCodeCfg::default().ast_chunk_max_lines` (200). Per-medium
//! config threading needs a chunker registry (P+); same deviation
//! pattern as `pdf-page-v1`'s pinned `chunker_version`
//! (`tasks/HOTFIXES.md`).

use kebab_core::{
    Block, BlockId, CanonicalDocument, Chunk, ChunkPolicy, Chunker, ChunkerVersion, DocumentId,
    SourceSpan, id_for_chunk,
};

const VERSION_LABEL: &str = "code-rust-ast-v1";
const BYTES_PER_TOKEN: usize = 3;
const POLICY_HASH_HEX_LEN: usize = 16;
const AST_CHUNK_MAX_LINES: u32 = 200;

#[derive(Clone, Copy, Debug, Default)]
pub struct CodeRustAstV1Chunker;

impl Chunker for CodeRustAstV1Chunker {
    fn chunker_version(&self) -> ChunkerVersion {
        ChunkerVersion(VERSION_LABEL.to_string())
    }

    fn policy_hash(&self, policy: &ChunkPolicy) -> String {
        let bytes = serde_json_canonicalizer::to_vec(policy)
            .expect("canonical JSON serialization of ChunkPolicy must not fail");
        let hex = blake3::hash(&bytes).to_hex().to_string();
        hex[..POLICY_HASH_HEX_LEN].to_string()
    }

    fn chunk(
        &self,
        doc: &CanonicalDocument,
        policy: &ChunkPolicy,
    ) -> anyhow::Result<Vec<Chunk>> {
        for b in &doc.blocks {
            let c = match b {
                Block::Code(c) => c,
                _ => anyhow::bail!(
                    "CodeRustAstV1Chunker only handles code docs (got non-Code block)"
                ),
            };
            if !matches!(c.common.source_span, SourceSpan::Code { .. }) {
                anyhow::bail!(
                    "CodeRustAstV1Chunker only handles code docs (got non-Code source_span)"
                );
            }
        }

        let base_policy_hash = self.policy_hash(policy);
        let chunker_version = self.chunker_version();
        let mut out: Vec<Chunk> = Vec::new();

        for b in &doc.blocks {
            let cb = match b {
                Block::Code(c) => c,
                _ => unreachable!("validated above"),
            };
            let (ls, le, symbol, lang) = match &cb.common.source_span {
                SourceSpan::Code { line_start, line_end, symbol, lang } => {
                    (*line_start, *line_end, symbol.clone(), lang.clone())
                }
                _ => unreachable!("validated above"),
            };
            let block_ids: Vec<BlockId> = vec![cb.common.block_id.clone()];
            let span_lines = le.saturating_sub(ls) + 1;

            if span_lines <= AST_CHUNK_MAX_LINES {
                let span = SourceSpan::Code {
                    line_start: ls,
                    line_end: le,
                    symbol: symbol.clone(),
                    lang: lang.clone(),
                };
                out.push(make_chunk(
                    doc, &chunker_version, &block_ids, &base_policy_hash,
                    None, span, cb.code.clone(),
                ));
            } else {
                let parts = split_oversize(&cb.code);
                let n = parts.len();
                for (i, (off_start, off_end, text)) in parts.into_iter().enumerate() {
                    let part_ls = ls + off_start;
                    let part_le = ls + off_end;
                    let part_sym = symbol
                        .as_ref()
                        .map(|s| format!("{s} [part {}/{n}]", i + 1));
                    let span = SourceSpan::Code {
                        line_start: part_ls,
                        line_end: part_le,
                        symbol: part_sym,
                        lang: lang.clone(),
                    };
                    out.push(make_chunk(
                        doc, &chunker_version, &block_ids, &base_policy_hash,
                        Some(part_ls), span, text,
                    ));
                }
            }
        }

        tracing::debug!(
            target: "kebab-chunk",
            doc_id = %doc.doc_id,
            chunks = out.len(),
            "code-rust-ast-v1 chunked",
        );
        Ok(out)
    }
}

#[allow(clippy::too_many_arguments)]
fn make_chunk(
    doc: &CanonicalDocument,
    chunker_version: &ChunkerVersion,
    block_ids: &[BlockId],
    base_policy_hash: &str,
    split_key: Option<u32>,
    span: SourceSpan,
    text: String,
) -> Chunk {
    // Per-chunk policy_hash variant prevents chunk_id collision when one
    // block splits into multiple parts (same block_ids). Mirrors
    // pdf-page-v1. Single-chunk units use the base hash unchanged.
    let id_hash = match split_key {
        Some(k) => format!("{base_policy_hash}#L{k}"),
        None => base_policy_hash.to_string(),
    };
    let chunk_id = id_for_chunk(&doc.doc_id, chunker_version, block_ids, &id_hash);
    let token_estimate = text.len().div_ceil(BYTES_PER_TOKEN);
    Chunk {
        chunk_id,
        doc_id: DocumentId(doc.doc_id.0.clone()),
        block_ids: block_ids.to_vec(),
        text,
        heading_path: Vec::new(),
        source_spans: vec![span],
        token_estimate,
        chunker_version: chunker_version.clone(),
        policy_hash: base_policy_hash.to_string(),
    }
}

/// Split an oversize unit at blank-line paragraph boundaries, greedily
/// gluing paragraphs until ~`AST_CHUNK_MAX_LINES` lines accumulate.
/// Returns `(line_offset_start, line_offset_end, text)` where offsets are
/// 0-based within the unit (caller adds the unit's absolute `line_start`).
fn split_oversize(code: &str) -> Vec<(u32, u32, String)> {
    let lines: Vec<&str> = code.split('\n').collect();
    let total = lines.len() as u32;
    let mut out: Vec<(u32, u32, String)> = Vec::new();
    let mut start: u32 = 0;
    while start < total {
        let mut end = (start + AST_CHUNK_MAX_LINES).min(total);
        // Prefer ending on a blank line within the last 20% of the window
        // so we don't cut mid-paragraph when a boundary is nearby.
        let floor = start + (AST_CHUNK_MAX_LINES * 4 / 5);
        if end < total {
            if let Some(b) = (floor.min(end)..end)
                .rev()
                .find(|&i| lines[i as usize].trim().is_empty())
            {
                end = b + 1;
            }
        }
        let text = lines[start as usize..end as usize].join("\n");
        out.push((start, end.saturating_sub(1), text));
        start = end;
    }
    if out.is_empty() {
        out.push((0, total.saturating_sub(1), code.to_string()));
    }
    out
}
```

- [ ] **Step 4: Wire into lib.rs**

In `crates/kebab-chunk/src/lib.rs` add (matching the existing `mod`/`pub use` style):

```rust
mod code_rust_ast_v1;
pub use code_rust_ast_v1::CodeRustAstV1Chunker;
```

- [ ] **Step 5: Run — expect pass**

Run: `cargo test -p kebab-chunk code_rust_ast`
Expected: PASS (all 6 tests).

- [ ] **Step 6: clippy + commit**

```bash
cargo clippy -p kebab-chunk --all-targets -- -D warnings
git add crates/kebab-chunk/
git commit -m "feat(p10-1a-2): code-rust-ast-v1 chunker (1:1 + oversize split)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 8: `kebab-app` dispatch — `ingest_one_code_asset`

**Files:**
- Modify: `crates/kebab-app/Cargo.toml` (add `kebab-parse-code = { path = "../kebab-parse-code" }` if not already a dep)
- Modify: `crates/kebab-app/src/lib.rs` (`ingest_one_asset` match ~line 896; new `ingest_one_code_asset` fn modeled on `ingest_one_pdf_asset` at 1455-end)
- Test: `crates/kebab-app/tests/` — add `code_ingest_smoke.rs` (mirror an existing app integration test's harness)

- [ ] **Step 1: Check/Add the crate dep**

```bash
grep -n "kebab-parse-code" crates/kebab-app/Cargo.toml || \
  echo 'kebab-parse-code = { path = "../kebab-parse-code" }  # p10-1A-2' \
  >> /dev/stderr
```

If absent, add `kebab-parse-code = { path = "../kebab-parse-code" }` under `[dependencies]` in `crates/kebab-app/Cargo.toml` (1A-1 may have added it for the skip helpers — verify).

- [ ] **Step 2: Write the failing integration test**

Create `crates/kebab-app/tests/code_ingest_smoke.rs`. Model the harness on an existing app integration test (read one under `crates/kebab-app/tests/` for the `App`/`Config`/TempDir setup pattern). Core assertions:

```rust
// Build an isolated TempDir KB, drop a tiny .rs file in the workspace,
// run ingest, then assert a search returns a Citation::Code.
#[test]
fn rust_file_ingests_and_searches_as_code_citation() {
    // ... TempDir + Config pointing workspace.root at it (copy the
    // harness from the sibling integration test verbatim) ...
    std::fs::write(workspace_root.join("demo.rs"),
        "/// adds\npub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n").unwrap();

    let report = kebab_app::ingest_with_config(&cfg, /* args per existing test */).unwrap();
    assert!(report.ingested >= 1, "rust file ingested: {report:?}");

    let hits = kebab_app::search_with_config(&cfg, "add", /* args */).unwrap();
    let code_hit = hits.iter().find(|h| matches!(
        h.citation, kebab_core::Citation::Code { .. }));
    let h = code_hit.expect("a Citation::Code hit");
    match &h.citation {
        kebab_core::Citation::Code { lang, symbol, line_start, .. } => {
            assert_eq!(lang.as_deref(), Some("rust"));
            assert_eq!(symbol.as_deref(), Some("add"));
            assert!(*line_start >= 1);
        }
        _ => unreachable!(),
    }
    assert_eq!(h.code_lang.as_deref(), Some("rust"));
}
```

> Use the exact `*_with_config` facade signatures from a sibling test (the facade rule — CLAUDE.md — requires the `_with_config` form). Read one existing `crates/kebab-app/tests/*.rs` to copy the harness; do not invent the Config builder.

- [ ] **Step 3: Run — expect fail**

Run: `cargo test -p kebab-app rust_file_ingests_and_searches_as_code_citation`
Expected: FAIL — code asset currently hits the `_ => Skipped` arm in `ingest_one_asset`.

- [ ] **Step 4: Add the dispatch + `ingest_one_code_asset`**

In `crates/kebab-app/src/lib.rs`, in the `ingest_one_asset` match (~line 896, where `MediaType::Pdf => { return ingest_one_pdf_asset(...) }`), add before the catch-all `_`:

```rust
        MediaType::Code(lang) if lang == "rust" => {
            return ingest_one_code_asset(
                app, asset, chunk_policy, embedder, vector_store,
                existing_doc_ids, force_reingest,
            );
        }
        // Non-Rust code langs activate in later phases (1B+); skip for now.
```

Add `fn ingest_one_code_asset(...)` modeled **line-for-line on `ingest_one_pdf_asset` (lib.rs:1455-end)** with these substitutions:
- parser_version: `ParserVersion(kebab_parse_code::RUST_PARSER_VERSION.to_string())`
- extractor: `kebab_parse_code::RustAstExtractor::new().extract(&ctx, &bytes).context("kb-parse-code::RustAstExtractor::extract")?`
- chunker: `let chunker = CodeRustAstV1Chunker;` + `chunker.chunk(&canonical, chunk_policy).context("kb-chunk::CodeRustAstV1Chunker::chunk")?`
- `try_skip_unchanged(... &CodeRustAstV1Chunker.chunker_version() ...)`
- `.context("... (code)")` strings instead of `(pdf)`
- import `CodeRustAstV1Chunker` (it's re-exported from `kebab_chunk`) at the top of `lib.rs` alongside the existing `PdfPageV1Chunker` import.

Everything else (read bytes, ExtractContext, put_asset/document/blocks/chunks, embed branch, IngestItem construction, `kb://` skip) is identical.

- [ ] **Step 5: Run — expect pass**

Run: `cargo test -p kebab-app rust_file_ingests_and_searches_as_code_citation`
Expected: PASS.

- [ ] **Step 6: clippy + commit**

```bash
cargo clippy -p kebab-app --all-targets -- -D warnings
git add crates/kebab-app/
git commit -m "feat(p10-1a-2): wire code ingest dispatch (ingest_one_code_asset)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 9: `code_lang_breakdown` in `kebab schema`

**Files:**
- Modify: `crates/kebab-store-sqlite/src/store.rs` (add a `code_lang_breakdown()` query next to whatever computes `media_breakdown` ~line 608)
- Modify: `crates/kebab-app/src/schema.rs:170` (currently `code_lang_breakdown: BTreeMap::new()`)
- Test: extend the existing `schema.rs` test (`crates/kebab-app/src/schema.rs:196+`) to assert a real count after a code ingest, or a store-level unit test in `kebab-store-sqlite`

- [ ] **Step 1: Write the failing test**

In `crates/kebab-store-sqlite` add a unit test that inserts a document with `metadata.code_lang = Some("rust")` and asserts:

```rust
#[test]
fn code_lang_breakdown_counts_by_code_lang() {
    // ... open in-memory/temp store, put_document with code_lang=Some("rust") ...
    let bd = store.code_lang_breakdown().unwrap();
    assert_eq!(bd.get("rust"), Some(&1));
}
```

> Read an existing `kebab-store-sqlite` test for the store-setup harness and how documents/metadata are persisted (so the `code_lang` column / json path is correct — `metadata` is stored as JSON; the query likely uses `json_extract(metadata_json, '$.code_lang')`).

- [ ] **Step 2: Run — expect fail**

Run: `cargo test -p kebab-store-sqlite code_lang_breakdown_counts`
Expected: FAIL — `code_lang_breakdown` undefined.

- [ ] **Step 3: Implement the store query**

In `crates/kebab-store-sqlite/src/store.rs`, mirror the `media_breakdown` query. The exact column depends on how `Metadata` is stored — inspect the `put_document` insert and the existing `media_breakdown` SQL, then add:

```rust
pub fn code_lang_breakdown(&self) -> anyhow::Result<std::collections::BTreeMap<String, u32>> {
    // documents whose metadata JSON has a non-null code_lang
    let mut stmt = self.conn.prepare(
        "SELECT json_extract(metadata_json, '$.code_lang') AS cl, COUNT(*) \
         FROM documents \
         WHERE cl IS NOT NULL \
         GROUP BY cl",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u32))
    })?;
    let mut out = std::collections::BTreeMap::new();
    for row in rows {
        let (k, v) = row?;
        out.insert(k, v);
    }
    Ok(out)
}
```

> Adjust table/column names (`documents`, `metadata_json`) to the actual schema — grep the `media_breakdown` impl and copy its `FROM`/column conventions exactly.

- [ ] **Step 4: Populate it in schema.rs**

In `crates/kebab-app/src/schema.rs`, replace the `code_lang_breakdown: std::collections::BTreeMap::new(),` placeholder (line ~170) with a call to the new store method (mirror how `media_breakdown: counts.media_breakdown` is sourced — likely `app.sqlite.code_lang_breakdown()?`).

- [ ] **Step 5: Run — expect pass + suite**

Run: `cargo test -p kebab-store-sqlite code_lang_breakdown_counts`
Expected: PASS.
Run: `cargo test -p kebab-app schema`
Expected: PASS (existing `schema.v1` serialization tests still green; `code_lang_breakdown` now populated).

- [ ] **Step 6: clippy + commit**

```bash
cargo clippy -p kebab-store-sqlite -p kebab-app --all-targets -- -D warnings
git add crates/kebab-store-sqlite/ crates/kebab-app/
git commit -m "feat(p10-1a-2): populate schema.v1 code_lang_breakdown

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 10: Full-suite gate + self-ingest snapshot

**Files:**
- Create: `crates/kebab-chunk/tests/code_rust_ast_snapshot.rs` + fixture `crates/kebab-chunk/tests/fixtures/code-sample.rs` + baseline `code-sample.chunks.snapshot.json` (mirror `crates/kebab-chunk/tests/long_section_snapshot.rs`)

- [ ] **Step 1: Add a snapshot integration test**

Mirror `long_section_snapshot.rs` exactly (same `UPDATE_SNAPSHOTS=1` regen mechanism). Build a `CanonicalDocument` by running `kebab_parse_code::RustAstExtractor` on `tests/fixtures/code-sample.rs` (a representative file with a free fn, an impl with 2 methods, a struct, a trait, a top-level `use`/`const` block, and one >200-line fn to exercise the split), chunk with `CodeRustAstV1Chunker`, serialize, compare to baseline.

> `kebab-chunk` may not depend on `kebab-parse-code` even as a dev-dep if that crosses a boundary — check `tasks/p10/p10-1a-2-rust-ast-chunker.md` Allowed deps. It does not list kebab-chunk→kebab-parse-code. So instead, build the `CanonicalDocument` **by hand** in the test (construct `Block::Code` units directly, like Task 7's `code_doc` helper) rather than invoking the extractor. The snapshot then locks the *chunker's* mapping/splitting, which is the unit under test here. (Extractor behavior is already locked by Task 6's tests.)

- [ ] **Step 2: Generate the baseline**

Run: `UPDATE_SNAPSHOTS=1 cargo test -p kebab-chunk code_rust_ast_snapshot`
Then run without the env var:
Run: `cargo test -p kebab-chunk code_rust_ast_snapshot`
Expected: PASS.

- [ ] **Step 3: Full workspace suite (the real gate)**

```bash
cargo test --workspace --no-fail-fast -j 1
```

Expected: PASS. Pay attention to: citation/wire round-trip tests (Citation still 6 variants — `Code` from 1A-1 unchanged), any golden eval fixtures, asset/MediaType serialization tests. Fix faithfully; a regression here means an earlier task's match arm was wrong.

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

Expected: clean.

- [ ] **Step 4: Manual smoke (SMOKE.md flow, isolated TempDir)**

```bash
cargo build --release
rm -rf /tmp/kebab-p10smoke && mkdir -p /tmp/kebab-p10smoke/ws
cp crates/kebab-chunk/src/code_rust_ast_v1.rs /tmp/kebab-p10smoke/ws/
cat > /tmp/kebab-p10smoke/config.toml <<'EOF'
[workspace]
root = "/tmp/kebab-p10smoke/ws"
[paths]
data_dir = "/tmp/kebab-p10smoke/data"
EOF
# (match the SMOKE.md config skeleton if these keys differ — read docs/SMOKE.md)
./target/release/kebab --config /tmp/kebab-p10smoke/config.toml ingest --json
./target/release/kebab --config /tmp/kebab-p10smoke/config.toml search "chunk" --json | head
./target/release/kebab --config /tmp/kebab-p10smoke/config.toml schema --json | grep code_lang
```

Expected: ingest report shows ≥1 ingested; a search hit with `"citation":{"kind":"code",...,"lang":"rust"}` and top-level `"code_lang":"rust"`, `"repo":...`; `schema --json` `code_lang_breakdown` has `"rust"`.

- [ ] **Step 5: Commit**

```bash
git add crates/kebab-chunk/tests/
git commit -m "test(p10-1a-2): code-rust-ast-v1 chunker snapshot + full-suite gate

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 11: Docs, version bump, status flips

**Files:**
- `README.md` — 지원 형식 / 명령 table: note Rust code ingest is now active (per CLAUDE.md README rule — a new media surface). Mermaid: add `code` source crossing the boundary only if a media type newly crosses it (it does — add a code-ingest edge to the existing diagram).
- `HANDOFF.md` — P10 phase row: note 1A-2 merged (Rust code ingest active, kebab self-dogfooding possible); add a one-line entry under 머지 후 결정 if a HOTFIXES item lands.
- `docs/ARCHITECTURE.md` — add the `kebab-app → kebab-parse-code` edge + `kebab-parse-code → tree-sitter/tree-sitter-rust` to the dependency graph; add the locked-in decision "tree-sitter lives parser-side, not chunker-side (design §6.3)" to the decisions table.
- `docs/SMOKE.md` — add the Rust code-ingest smoke steps (the Task 10 Step 4 flow), and the `[ingest.code]` config keys if not already documented.
- `tasks/INDEX.md` line ~143 + `tasks/p10/INDEX.md` row 1A-2 — flip 1A-1 to ✅ and 1A-2 to ✅ (on merge).
- `tasks/HOTFIXES.md` — add a dated entry for the `AST_CHUNK_MAX_LINES` constant-vs-config deviation (chunker can't see `IngestCodeCfg.ast_chunk_max_lines` through the fixed `Chunker` trait; pinned to the default 200; per-medium chunker registry is P+). Cross-link one line in `tasks/p10/p10-1a-2-rust-ast-chunker.md` Risks/notes.
- `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md` §10.1 — record the 1A-2 surface per design §10.1 (already partly done for SourceSpan/MediaType in Tasks 2/4; ensure §10.1 mentions code ingest activation).
- `Cargo.toml` — workspace `version` minor bump (design §10.4: 1A-2 merge = 도그푸딩 가능 = bump trigger). e.g. `0.6.x` → `0.7.0` (check current value first).

- [ ] **Step 1: Make all doc edits above.** Keep README narrow (usage only, one Mermaid). HANDOFF gets phase status. ARCHITECTURE gets the graph/decision.

- [ ] **Step 2: Version bump**

```bash
grep -m1 '^version' Cargo.toml   # read current workspace version
# bump minor in Cargo.toml [workspace.package].version
cargo build --release            # refresh Cargo.lock + binary identity
```

- [ ] **Step 3: Full suite once more (docs/version shouldn't break it, but the lock changed)**

```bash
cargo test --workspace --no-fail-fast -j 1
cargo clippy --workspace --all-targets -- -D warnings
```

Expected: PASS / clean.

- [ ] **Step 4: Commit (bump = release commit, same commit per CLAUDE.md)**

```bash
git add -A
git commit -m "docs(p10-1a-2): README/HANDOFF/ARCHITECTURE/SMOKE + HOTFIXES + status; chore: bump version

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Finalize: PR + review loop + release

Per `tasks/HOTFIXES.md` workflow memory and CLAUDE.md Remote section (Gitea, not gh):

- [ ] Use the **gitea-ops** skill: `gitea-pr` to open the PR (feature branch → main). Title: `feat(p10-1a-2): Rust AST chunker — tree-sitter-rust code ingest active`. Body summarizes: SourceSpan::Code internal variant, parser-side tree-sitter (design §6.3), code-rust-ast-v1 chunker + oversize split, MediaType::Code, schema code_lang_breakdown, frozen design §3.4/§10.1 sync, version bump.
- [ ] **Review loop mode** (do not ask single-shot): `gitea-pr-status --wait-ci` → `gitea-pr-diff` → analyze → `gitea-pr-review` (REQUEST_CHANGES/APPROVE) each round; actionable comments → follow-up commits; converge to APPROVE.
- [ ] On APPROVE: merge immediately (no asking), sync local `main`, delete `feat/p10-1a-2-rust-ast-chunker`.
- [ ] After merge: `cargo clean` (CLAUDE.md routine). Cut release via gitea-ops `gitea-release v<new version>` (release notes: Rust code ingest active, `Citation::Code` now populated, `MediaType::Code`, `schema.v1 code_lang_breakdown`, internal `SourceSpan::Code`).
- [ ] Flip `tasks/INDEX.md` / `tasks/p10/INDEX.md` 1A-2 → ✅ if not already in the merged commit; update memory phase-priorities note if the next-task priority shifts (P10-1B vs other).

---

## Self-Review (completed by plan author)

- **Spec coverage:** design §1A-2 (Rust chunker + tree-sitter-rust + activation) → Tasks 6/7/8; §3.3 (`code-rust-ast-v1`) → Task 7; §3.4 symbol path → Task 6 walk rules + Task 2 SourceSpan; §6.1 (rust.rs parser) → Task 6; §6.2 (kebab-chunk module) → Task 7; §6.3 dep graph (tree-sitter parser-side) → Task 1 + Task 7 forbidden-dep note; §9.1 Tier-1 + oversize fallback → Task 7 `split_oversize`; §10.4 version bump → Task 11; wire (no v2 — Citation::Code from 1A-1) → Task 10 Step 3 gate. Citation routing gap (1A-1 left unwired) → Tasks 2+3. `MediaType::Code` + routing → Tasks 4+5. schema breakdown → Task 9. Docs cascade → Task 11.
- **Placeholder scan:** novel logic (SourceSpan variant, citation arm, AST walk, chunker, split) is given in full. Mechanical mirrors (extractor scaffold, `ingest_one_code_asset`, store breakdown query, integration-test harness) are pinned to an exact existing file:line to copy with enumerated deltas — the established-pattern path the writing-plans skill endorses, not "TBD".
- **Type consistency:** `RustAstExtractor` / `RUST_PARSER_VERSION` / `CodeRustAstV1Chunker` / `VERSION_LABEL="code-rust-ast-v1"` / `SourceSpan::Code{line_start,line_end,symbol,lang}` / `Citation::Code` (1A-1 shape) used consistently across Tasks 2/3/6/7/8. `id_for_block(doc,"code",&[],ordinal,&span)` and `id_for_chunk(doc,cv,block_ids,hash)` match `crates/kebab-core/src/ids.rs:146,163`.
