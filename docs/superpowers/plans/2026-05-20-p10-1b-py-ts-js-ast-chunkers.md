# p10-1B Python + TS/JS AST Chunkers Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task.

**Goal:** Activate Python / TypeScript / JavaScript code ingest end-to-end on top of 1A-2's infrastructure — 3 new tree-sitter grammars, 3 new Extractors, 3 new chunkers (`code-{python,ts,js}-ast-v1`), a `module_path_for_*` helper for workspace-path → module-path conversion, and a small app-dispatch generalization. Wire `code_lang` filter / breakdown / Citation::Code surface activate automatically.

**Architecture:** Mirror 1A-2 exactly per language. Each Extractor in `kebab-parse-code/src/{python,typescript,javascript}.rs` calls its tree-sitter grammar and emits one `Block::Code` per top-level AST semantic unit with `SourceSpan::Code { line_start, line_end, symbol, lang }`. Symbol = `module_path` (from workspace_path) `+` per-language join (`.` for Python, `/.../basename.symbol` for TS/JS). Each chunker is a near-duplicate of `code-rust-ast-v1` (1:1 + oversize split). App dispatch becomes `match lang { "rust" | "python" | "typescript" | "javascript" }`.

**Tech Stack:** Rust 2024 workspace, `tree-sitter` 0.26, `tree-sitter-python` / `tree-sitter-typescript` / `tree-sitter-javascript`, existing 1A-2 infrastructure (citation_helper Code arm, backfill, schema breakdown).

**Memory note:** Host was OOM-killed earlier in this branch's history. Prefer `cargo test -p <crate>` and `cargo check -p <crate>`; the only `cargo test --workspace -j 1` call is the Task L full-suite gate. Never run cargo invocations in parallel.

---

## Pre-flight

Branch `feat/p10-1b-py-ts-js` already exists on main (`git checkout feat/p10-1b-py-ts-js`).

- [ ] **Disk hygiene**: `cargo clean`.

Reference files (read before touching the corresponding 1B file):
- 1A-2 Rust extractor: `crates/kebab-parse-code/src/rust.rs` — the scaffold every per-lang extractor mirrors.
- 1A-2 Rust chunker: `crates/kebab-chunk/src/code_rust_ast_v1.rs` — the scaffold every per-lang chunker mirrors.
- 1A-2 app dispatch: `crates/kebab-app/src/lib.rs` `ingest_one_code_asset` (~line 1645).
- 1A-2 source-fs routing: `crates/kebab-source-fs/src/media.rs:39` (the `"rs" =>` arm).
- 1A-2 lang dispatch: `crates/kebab-parse-code/src/lang.rs::code_lang_for_path`.

---

## Task A: Workspace deps

**Files:**
- Modify: `Cargo.toml` (workspace `[workspace.dependencies]`, after the existing `tree-sitter-rust` entry)
- Modify: `crates/kebab-parse-code/Cargo.toml` (`[dependencies]`)

- [ ] **Step 1**: Resolve versions: `cargo add tree-sitter-python tree-sitter-typescript tree-sitter-javascript -p kebab-parse-code`.

- [ ] **Step 2**: Lift the three resolved versions into `[workspace.dependencies]` in the root `Cargo.toml`, immediately after the `tree-sitter-rust` line. Single-line comment first:

```toml
# Python / TS / JS grammars for code ingest (kebab-parse-code, p10-1B).
tree-sitter-python     = "<resolved>"
tree-sitter-typescript = "<resolved>"
tree-sitter-javascript = "<resolved>"
```

Then change the crate's `[dependencies]` entries to `{ workspace = true }` matching the existing `tree-sitter` / `tree-sitter-rust` style.

- [ ] **Step 3**: `cargo build -p kebab-parse-code` → clean (unused deps OK; warnings appear when actually imported in later tasks).

- [ ] **Step 4**: Commit.

```bash
git add Cargo.toml Cargo.lock crates/kebab-parse-code/Cargo.toml
git commit -m "build(p10-1b): add tree-sitter-python/-typescript/-javascript workspace deps

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task B: source-fs media routing for `.py`/`.pyi`/`.ts`/`.tsx`/`.js`/`.mjs`/`.cjs`/`.jsx`

**Files:**
- Modify: `crates/kebab-source-fs/src/media.rs` (add 3 arms next to the existing `"rs"` arm at L39)
- Test: same file's test module

- [ ] **Step 1 (failing test)**:

```rust
#[test]
fn py_ts_js_files_map_to_media_code() {
    assert_eq!(media_type_for(Path::new("a/b.py")),    MediaType::Code("python".into()));
    assert_eq!(media_type_for(Path::new("a/b.pyi")),   MediaType::Code("python".into()));
    assert_eq!(media_type_for(Path::new("a/b.ts")),    MediaType::Code("typescript".into()));
    assert_eq!(media_type_for(Path::new("a/b.tsx")),   MediaType::Code("typescript".into()));
    assert_eq!(media_type_for(Path::new("a/b.js")),    MediaType::Code("javascript".into()));
    assert_eq!(media_type_for(Path::new("a/b.mjs")),   MediaType::Code("javascript".into()));
    assert_eq!(media_type_for(Path::new("a/b.cjs")),   MediaType::Code("javascript".into()));
    assert_eq!(media_type_for(Path::new("a/b.jsx")),   MediaType::Code("javascript".into()));
    // Rust 1A-2 arm still works
    assert_eq!(media_type_for(Path::new("a/b.rs")),    MediaType::Code("rust".into()));
}
```

- [ ] **Step 2**: Run → FAIL.

- [ ] **Step 3**: Add the three arms before the `_ => MediaType::Other(ext)` fallback. Match existing style and order extensions logically (most common first within each language):

```rust
        // p10-1B: Python / TS / JS AST chunkers active.
        "py" | "pyi"               => MediaType::Code("python".into()),
        "ts" | "tsx"               => MediaType::Code("typescript".into()),
        "js" | "mjs" | "cjs" | "jsx" => MediaType::Code("javascript".into()),
```

- [ ] **Step 4**: Run → PASS. Then `cargo test -p kebab-source-fs` → no regression.

- [ ] **Step 5**: `cargo clippy -p kebab-source-fs --all-targets -- -D warnings` clean. Commit.

```bash
git add crates/kebab-source-fs/
git commit -m "feat(p10-1b): route .py/.pyi/.ts/.tsx/.js/.mjs/.cjs/.jsx to MediaType::Code

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task C: `module_path_for_python` + `module_path_for_tsjs` helpers

**Files:**
- Modify: `crates/kebab-parse-code/src/lang.rs` (add 2 pub fns + tests)
- Modify: `crates/kebab-parse-code/src/lib.rs` (re-export the 2 fns)

These convert a `WorkspacePath` into a module-path prefix for symbol formatting. Single source of truth — used by all per-language extractors.

### Rules

**`module_path_for_python(workspace_path: &str) -> String`**:
1. Strip a leading well-known "source root" prefix from a small allowlist if present (in order): `crates/<name>/src/`, `src/`, `lib/`. (Use a single small `for` loop over the allowlist; stop at first prefix match.) Rationale: avoid noisy `crates.x.src.foo.bar` symbols when the user has a conventional layout, while leaving non-conventional paths untouched.
2. Strip trailing `.py` or `.pyi` extension. If the basename (after extension strip) is `__init__`, drop it (and the preceding `/`) so `pkg/__init__.py` → `pkg`.
3. Replace `/` with `.`.
4. Result is the dotted module prefix. Symbols are joined with `.` (e.g. `module_path + "." + sym`). Empty result (file is at workspace root without prefix) → use empty string → symbol is the unit name alone.

**`module_path_for_tsjs(workspace_path: &str) -> String`**:
1. Strip extension if it's one of `.ts` / `.tsx` / `.js` / `.jsx` / `.mjs` / `.cjs`.
2. Do NOT replace `/` (TS/JS convention is path-like). Do NOT strip any source root (TS/JS layouts vary too widely).
3. Result is the path-style prefix (e.g. `src/search/retriever/Retriever`). Symbols join with `.` (`prefix + "." + sym`, e.g. `src/search/retriever/Retriever.search`).

- [ ] **Step 1 (failing tests)** — add to existing `mod tests` (or create one) in `lang.rs`:

```rust
#[test]
fn module_path_for_python_strips_src_roots_and_extensions() {
    assert_eq!(module_path_for_python("kebab_eval/metrics.py"),       "kebab_eval.metrics");
    assert_eq!(module_path_for_python("kebab_eval/__init__.py"),      "kebab_eval");
    assert_eq!(module_path_for_python("src/foo/bar.py"),              "foo.bar");
    assert_eq!(module_path_for_python("crates/x/src/foo/bar.py"),     "foo.bar");
    assert_eq!(module_path_for_python("a/b/c.pyi"),                   "a.b.c");
    assert_eq!(module_path_for_python("standalone.py"),               "standalone");
    assert_eq!(module_path_for_python("src/__init__.py"),             "");
}

#[test]
fn module_path_for_tsjs_keeps_slashes_and_strips_ext() {
    for ext in ["ts", "tsx", "js", "jsx", "mjs", "cjs"] {
        let p = format!("src/search/retriever/Retriever.{ext}");
        assert_eq!(module_path_for_tsjs(&p), "src/search/retriever/Retriever");
    }
    assert_eq!(module_path_for_tsjs("foo.ts"),                  "foo");
    assert_eq!(module_path_for_tsjs("a/b/c.ts"),                "a/b/c");
    // No `src/` strip — TS layouts vary.
    assert_eq!(module_path_for_tsjs("packages/x/src/Foo.ts"),  "packages/x/src/Foo");
}
```

- [ ] **Step 2**: Run → FAIL (helpers not defined).

- [ ] **Step 3**: Implement both in `lang.rs`. Suggested implementation (refine if a test points out a missed edge case):

```rust
/// p10-1B: workspace-relative Python file path → dotted module-path prefix.
/// See plan §Task C for the exact rules.
pub fn module_path_for_python(workspace_path: &str) -> String {
    let mut p: &str = workspace_path;
    // Strip a known source-root prefix. Allowlist + `starts_with` over a
    // pattern with a glob in the middle would be a pain; treat
    // `crates/*/src/` by string-walking.
    if let Some(rest) = p.strip_prefix("crates/") {
        if let Some(slash) = rest.find('/') {
            let after = &rest[slash + 1..];
            if let Some(stripped) = after.strip_prefix("src/") {
                p = stripped;
            }
        }
    } else if let Some(stripped) = p.strip_prefix("src/") {
        p = stripped;
    } else if let Some(stripped) = p.strip_prefix("lib/") {
        p = stripped;
    }
    // Strip extension.
    let p = p
        .strip_suffix(".py")
        .or_else(|| p.strip_suffix(".pyi"))
        .unwrap_or(p);
    // __init__ → drop it (and the preceding `/`).
    let p = if let Some(parent) = p.strip_suffix("/__init__") {
        parent
    } else if p == "__init__" {
        ""
    } else {
        p
    };
    p.replace('/', ".")
}

/// p10-1B: workspace-relative TS/JS file path → path-style prefix
/// (no slash replacement). See plan §Task C.
pub fn module_path_for_tsjs(workspace_path: &str) -> String {
    let p = workspace_path;
    for ext in [".tsx", ".ts", ".jsx", ".mjs", ".cjs", ".js"] {
        if let Some(stripped) = p.strip_suffix(ext) {
            return stripped.to_string();
        }
    }
    p.to_string()
}
```

- [ ] **Step 4**: Re-export both from `lib.rs` (next to the existing `pub use lang::code_lang_for_path`):

```rust
pub use lang::{code_lang_for_path, module_path_for_python, module_path_for_tsjs};
```

- [ ] **Step 5**: Run → PASS. clippy clean.

- [ ] **Step 6**: Commit.

```bash
git add crates/kebab-parse-code/
git commit -m "feat(p10-1b): module_path_for_python / _tsjs helpers (workspace path → module prefix)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task D: App dispatch generalization

**Files:**
- Modify: `crates/kebab-app/src/lib.rs`

Today's `ingest_one_code_asset` (~L1645) hardcodes `RustAstExtractor` + `CodeRustAstV1Chunker`. 1B needs to dispatch by `lang`. Cleanest minimal change: keep the same function signature but take `code_lang: &str` and `match` it internally onto an `Extractor` + `Chunker` pair. Rust path keeps the same observable behavior.

Two equivalent dispatch shapes — pick the one with the smallest diff:

**Shape 1 (recommended — fewest lines changed):** factor extractor invocation + chunker invocation into a small `match code_lang` *inside* `ingest_one_code_asset`. The `parser_version` constant lookup also branches. Everything else (read bytes, ExtractContext, put_*, embed, IngestItem) stays a single non-branched flow.

**Shape 2:** introduce a tiny enum `CodeLangKind { Rust, Python, Typescript, Javascript }` + an `impl CodeLangKind { fn extract(...) -> CanonicalDocument; fn chunk(...) -> Vec<Chunk>; fn parser_version() -> ParserVersion; fn chunker_version() -> ChunkerVersion; }`. More structure, but better insulates the function body.

Use Shape 1 for this task (less risk). A future C/D phase can refactor to Shape 2 if the dispatch grows.

- [ ] **Step 1 (failing test)** — add a Python smoke as the failing test (TS/JS land later in this PR; one failing-then-passing TDD cycle is enough to lock the dispatch contract):

In `crates/kebab-app/tests/code_ingest_smoke.rs` add:

```rust
#[test]
fn python_file_ingests_and_searches_as_code_citation() {
    // Mirror rust_file_ingests_and_searches_as_code_citation exactly,
    // but write `kebab_eval/metrics.py` (in the temp workspace root) with:
    //     def compute_mrr(): return 1.0
    // and assert h.code_lang == Some("python"), citation.lang == Some("python"),
    // citation.symbol == Some("kebab_eval.metrics.compute_mrr"), parser_version "code-python-v1",
    // chunker_version "code-python-ast-v1".
    // ...
}
```

(Spec shape ONLY — the actual extractor + chunker land in Tasks F + G. This test compiles but FAILS at runtime until those land. Mark it `#[ignore]` if it would otherwise break TDD ordering — un-`#[ignore]` it in Task G's commit. Alternative: skip this step here and rely on the per-extractor unit tests in Task F + Task G; that is the cleaner TDD ordering. Choose either; document the choice in the commit message.)

- [ ] **Step 2**: Update `ingest_one_asset` dispatch match arm to accept all four code languages with a `lang` capture passed through:

```rust
        // p10-1A-2 / 1B: code ingest dispatch.
        MediaType::Code(lang)
            if matches!(lang.as_str(), "rust" | "python" | "typescript" | "javascript") =>
        {
            return ingest_one_code_asset(
                app, asset, chunk_policy, embedder, vector_store,
                existing_doc_ids, force_reingest, lang.as_str(),
            );
        }
```

(Keep the trailing `MediaType::Code(_) | MediaType::Audio(_) | MediaType::Other(_)` or-pattern as the Skipped fallback — non-allowlisted code langs route there.)

- [ ] **Step 3**: Update `ingest_one_code_asset` signature to take `code_lang: &str` and dispatch internally. Keep all I/O / persistence / embed code unchanged. Per the Shape-1 recipe:
  - `let parser_version = match code_lang { "rust" => ParserVersion(kebab_parse_code::RUST_PARSER_VERSION.into()), "python" => ParserVersion(kebab_parse_code::PYTHON_PARSER_VERSION.into()), "typescript" => ParserVersion(kebab_parse_code::TS_PARSER_VERSION.into()), "javascript" => ParserVersion(kebab_parse_code::JS_PARSER_VERSION.into()), _ => unreachable!(), };`
  - The `try_skip_unchanged` call's chunker_version arg branches the same way (different chunker per lang).
  - The extract call branches: `match code_lang { "rust" => RustAstExtractor::new().extract(...), "python" => PythonAstExtractor::new().extract(...), ... }`.
  - The chunk call branches: `match code_lang { "rust" => CodeRustAstV1Chunker.chunk(...), "python" => CodePythonAstV1Chunker.chunk(...), ... }`.
  - All other lines (purge_vector_orphans / put_asset_with_bytes / put_document / put_blocks / put_chunks / embed branch / IngestItem) unchanged.

At this point Python/TS/JS extractors + chunkers don't exist yet → compile FAILS on the references. Acceptable — Task E/F/G/H/I add them. To stage compile-cleanly: gate the Python/TS/JS arms behind `unimplemented!()` for now (returns an error path) and let Tasks F/G/H/I/J/K replace them. Recommended: leave the dispatch fully written but use `anyhow::bail!("not yet activated in this commit")` for the three non-Rust arms, with a `TODO(p10-1b Task X)` comment per arm. They flip to real calls when each language's extractor + chunker land.

- [ ] **Step 4**: `cargo test -p kebab-app --lib` (lib-only is enough — integration tests for the non-Rust paths land later). Existing Rust path tests must stay green.

- [ ] **Step 5**: clippy clean, commit.

```bash
git add crates/kebab-app/
git commit -m "refactor(p10-1b): generalize ingest_one_code_asset for multi-language dispatch

Rust path unchanged (verified by existing code_ingest_smoke tests). Python/TS/JS arms
bail with TODO; per-lang extractor + chunker land in subsequent tasks.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task E: Python Extractor (`kebab-parse-code/src/python.rs`)

**Files:**
- Create: `crates/kebab-parse-code/src/python.rs`
- Modify: `crates/kebab-parse-code/src/lib.rs` (`pub mod python` + re-exports `PYTHON_PARSER_VERSION` and `PythonAstExtractor`)
- Create: `crates/kebab-parse-code/tests/fixtures/sample.py`

Scaffold MIRRORS `crates/kebab-parse-code/src/rust.rs` line-for-line (read it first). Only the AST walk + the symbol prefix differ.

### Python AST mapping

tree-sitter-python language: `tree_sitter_python::LANGUAGE` (LanguageFn). Set via `parser.set_language(&tree_sitter_python::LANGUAGE.into())`.

Walk `module` (root) named children. Maintain `mod_path: Vec<String>` — but for Python we DO NOT push class names onto `mod_path` (class members get `Class.method` form via the class arm directly; nested classes recurse with the class name appended).

| node kind | unit | symbol (joined with `.`) |
|-----------|------|--------------------------|
| `function_definition` (name field) | 1 | `<module_prefix>.<fn_name>` (or `<fn_name>` if module_prefix empty) |
| `class_definition` (name) — emit ONE unit for the class definition itself (symbol `<module_prefix>.<ClassName>`), then recurse into its `block` body: each inner `function_definition` → unit with symbol `<module_prefix>.<ClassName>.<method_name>`; nested `class_definition` recurses with parent class prepended. | 1 per class + 1 per method (etc.) | as above |
| `decorated_definition` | unwrap — process its inner `definition` (either function_definition or class_definition) as if at the same level. `unit_start`'s backward extension over `decorator` siblings folds them into the unit. | n/a | n/a |
| `import_statement`, `import_from_statement`, `expression_statement`, `assignment`, `global_statement`, `future_import_statement` at module level | glue | `<top-level>` (with `module_prefix` prefix if non-empty: `<module_prefix>.<top-level>`) |

`unit_start` (backward extension) covers `comment` siblings + `decorator` siblings (decorators in tree-sitter-python appear as children of `decorated_definition`, NOT as siblings — so the `unwrap decorated_definition` arm above is what brings them in; `comment` siblings still need backward extension). Adapt `unit_start` for the Python flavor: extend over `comment` siblings only (decorators are already covered by unwrapping `decorated_definition`).

Module-prefix application: at extract time, compute `let mod_prefix = kebab_parse_code::module_path_for_python(&asset.workspace_path.0);`. The walk builds symbols using `mod_prefix` (joined with `.` if non-empty; the bare name if empty). Glue group: if `mod_prefix` non-empty, symbol = `format!("{mod_prefix}.<top-level>")`; else `<top-level>`. `<module>` glue label (file contains only `import`s and no real unit) follows the same prefix rule.

### Scaffold differences from rust.rs

- `pub const PARSER_VERSION: &str = "code-python-v1";`
- `pub struct PythonAstExtractor;` + `new()`/`Default`.
- `fn supports(&self, m: &MediaType) -> bool { matches!(m, MediaType::Code(l) if l == "python") }`
- Agent string `"kb-parse-code"` (unchanged).
- `metadata.code_lang = Some("python".to_string())`.
- `repo` / `git_branch` / `git_commit` from `crate::repo::detect_repo` (same as Rust).
- The AST walk is its own `build_blocks` function — DO NOT generalize across languages in this task (each grammar's node names differ enough that polymorphism hurts more than helps; a future refactor task can extract common helpers if patterns converge).

### Step list (TDD)

- [ ] **Step 1**: Create `tests/fixtures/sample.py`:

```python
"""sample fixture."""
import os

ANSWER = 42

@staticmethod
def free(x):
    """free fn."""
    return x + 1

class Foo:
    """doc."""
    def double(self, n):
        return n * 2

    @classmethod
    def name(cls):
        return "foo"

class Outer:
    class Inner:
        def helper(self):
            return True

def with_decorator():
    pass
```

- [ ] **Step 2 (failing test)** in `python.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use kebab_core::{Block, MediaType, SourceSpan};
    fn extract_fixture() -> kebab_core::CanonicalDocument {
        let bytes = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/sample.py")).unwrap();
        // Reuse the test-support helper added in Task 6 of 1A-2 (rust.rs tests):
        // adjust `fixed_rust_asset` to a generic `fixed_code_asset(workspace_path, code_lang)`
        // OR inline a per-test asset constructor that matches its kebab-core types.
        let asset = crate::rust::tests_support::fixed_code_asset(
            "kebab_eval/metrics.py", "python");
        let cfg = kebab_core::ExtractConfig::default();
        let root = std::path::PathBuf::from("/tmp");
        let ctx = kebab_core::ExtractContext { asset: &asset, workspace_root: &root, config: &cfg };
        PythonAstExtractor::new().extract(&ctx, &bytes).unwrap()
    }
    #[test]
    fn extractor_supports_only_media_code_python() {
        let e = PythonAstExtractor::new();
        assert!(e.supports(&MediaType::Code("python".into())));
        assert!(!e.supports(&MediaType::Code("rust".into())));
        assert!(!e.supports(&MediaType::Markdown));
    }
    #[test]
    fn python_units_carry_module_prefixed_symbols() {
        let doc = extract_fixture();
        let mut syms: Vec<String> = doc.blocks.iter().map(|b| match b {
            Block::Code(c) => match &c.common.source_span {
                SourceSpan::Code { symbol, lang, .. } => {
                    assert_eq!(lang.as_deref(), Some("python"));
                    symbol.clone().unwrap()
                }
                _ => panic!("expected SourceSpan::Code"),
            },
            other => panic!("expected Block::Code, got {other:?}"),
        }).collect();
        syms.sort();
        // workspace_path `kebab_eval/metrics.py` → mod_prefix `kebab_eval.metrics`
        assert!(syms.iter().any(|s| s == "kebab_eval.metrics.free"));
        assert!(syms.iter().any(|s| s == "kebab_eval.metrics.Foo"));
        assert!(syms.iter().any(|s| s == "kebab_eval.metrics.Foo.double"));
        assert!(syms.iter().any(|s| s == "kebab_eval.metrics.Foo.name"));
        assert!(syms.iter().any(|s| s == "kebab_eval.metrics.Outer"));
        assert!(syms.iter().any(|s| s == "kebab_eval.metrics.Outer.Inner"));
        assert!(syms.iter().any(|s| s == "kebab_eval.metrics.Outer.Inner.helper"));
        assert!(syms.iter().any(|s| s == "kebab_eval.metrics.with_decorator"));
        assert!(syms.iter().any(|s| s == "kebab_eval.metrics.<top-level>"));  // import + assignment
    }
    #[test]
    fn deterministic_across_runs() {
        let a = extract_fixture();
        for _ in 0..50 { assert_eq!(extract_fixture().blocks, a.blocks); }
    }
}
```

(`tests_support::fixed_code_asset` — promote 1A-2's `fixed_rust_asset` to a generic helper that takes the lang string and sets `media_type: MediaType::Code(lang.to_string())`. Move it to a new `pub(crate) mod tests_support` in `rust.rs` so it's reachable from `python.rs::tests`, OR duplicate it inline — pick the smaller diff. Keep the helper `#[cfg(test)]`.)

- [ ] **Step 3**: Run → FAIL (`PythonAstExtractor` undefined).

- [ ] **Step 4**: Implement `python.rs`. Scaffold mirrors `rust.rs`; the AST walk follows the table above. The `mod_path: Vec<String>` for Python tracks **class nesting** (so methods get `Class.method`, nested classes get `Outer.Inner`). `Vec` empty at function-level. Glue grouping mirrors Rust's. Apply `mod_prefix` from `module_path_for_python(&asset.workspace_path.0)` to all unit symbols: `if mod_prefix.is_empty() { sym } else { format!("{mod_prefix}.{sym}") }`. The `<top-level>` / `<module>` label inherits the same prefixing.

- [ ] **Step 5**: Wire into `lib.rs`:

```rust
pub mod python;
pub use python::{PARSER_VERSION as PYTHON_PARSER_VERSION, PythonAstExtractor};
```

- [ ] **Step 6**: `cargo test -p kebab-parse-code python` → all pass.

- [ ] **Step 7**: clippy clean, commit.

```bash
git add crates/kebab-parse-code/
git commit -m "feat(p10-1b): tree-sitter-python AST extractor (PythonAstExtractor)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task F: Python chunker (`code-python-ast-v1`)

**Files:**
- Create: `crates/kebab-chunk/src/code_python_ast_v1.rs`
- Modify: `crates/kebab-chunk/src/lib.rs` (`mod` + `pub use`)

NEAR-DUPLICATE of `crates/kebab-chunk/src/code_rust_ast_v1.rs`. ONLY differences:
- `const VERSION_LABEL: &str = "code-python-ast-v1";`
- struct name `CodePythonAstV1Chunker`
- The validation message says "code-python-ast-v1 only handles..."

`split_oversize` + `make_chunk` + `AST_CHUNK_MAX_LINES` + `BYTES_PER_TOKEN` + `POLICY_HASH_HEX_LEN` IDENTICAL (these are language-agnostic).

- [ ] **Step 1 (failing tests)**: Copy the entire `#[cfg(test)] mod tests` from `code_rust_ast_v1.rs` and substitute `Rust` → `Python` / `code-rust-ast-v1` → `code-python-ast-v1`. Use the same in-memory `code_doc` helper — it doesn't care about the actual language. Add one extra test specifically asserting the `policy_hash` equals the Rust chunker's (cross-chunker fingerprint identity is a 1A-2 invariant — must hold for new chunkers too).

- [ ] **Step 2**: Run → FAIL.

- [ ] **Step 3**: Copy `code_rust_ast_v1.rs` to `code_python_ast_v1.rs` and apply the substitutions above. Keep the `tree-sitter is intentionally NOT a dependency here` comment (still true).

- [ ] **Step 4**: Wire into `lib.rs`:

```rust
mod code_python_ast_v1;
pub use code_python_ast_v1::CodePythonAstV1Chunker;
```

- [ ] **Step 5**: `cargo test -p kebab-chunk code_python_ast` → pass. Full per-crate suite stays green.

- [ ] **Step 6**: clippy clean, commit.

```bash
git add crates/kebab-chunk/
git commit -m "feat(p10-1b): code-python-ast-v1 chunker (1:1 + oversize split)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task G: Activate Python in app dispatch

**Files:**
- Modify: `crates/kebab-app/src/lib.rs` (replace the Python `bail!` arm with real calls)
- Modify: `crates/kebab-app/tests/code_ingest_smoke.rs` (un-`#[ignore]` the Python test, OR add it now if you deferred in Task D)

- [ ] **Step 1**: Replace the Python arm's `bail!` with `PythonAstExtractor::new().extract(...)` + `CodePythonAstV1Chunker.chunk(...)` calls (mirror the Rust arm exactly). Set parser_version / chunker_version per Python.

- [ ] **Step 2**: Un-ignore / add `python_file_ingests_and_searches_as_code_citation`. Test asserts the full pipeline produces a `Citation::Code { lang: Some("python"), symbol: Some("kebab_eval.metrics.compute_mrr"), .. }` for a `kebab_eval/metrics.py` written into the temp workspace.

- [ ] **Step 3**: `cargo test -p kebab-app code_ingest_smoke python_file_ingests` → pass. Existing Rust test stays green.

- [ ] **Step 4**: clippy clean, commit.

```bash
git add crates/kebab-app/
git commit -m "feat(p10-1b): activate Python in ingest_one_code_asset dispatch

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task H: TypeScript Extractor (`kebab-parse-code/src/typescript.rs`)

**Files:**
- Create: `crates/kebab-parse-code/src/typescript.rs`
- Modify: `crates/kebab-parse-code/src/lib.rs`
- Create: `crates/kebab-parse-code/tests/fixtures/sample.ts` + `sample.tsx`

Scaffold mirrors `rust.rs`/`python.rs`. Grammar selection: `tree_sitter_typescript::LANGUAGE_TYPESCRIPT` for `.ts`, `LANGUAGE_TSX` for `.tsx`. Decide inside `extract` by inspecting `asset.workspace_path.0` extension (a tiny helper local to this module is fine).

### TypeScript AST mapping

| node kind | unit | symbol (joined with `.`) |
|-----------|------|--------------------------|
| `function_declaration` (name) | 1 | `<mod>.<fn>` |
| `class_declaration` (name) — recurse into `class_body`: each `method_definition` (name) → unit `<mod>.<Class>.<method>` | 1 + 1 per method | as above |
| `interface_declaration` (name), `type_alias_declaration` (name), `enum_declaration` (name) | 1 | `<mod>.<Name>` |
| `export_statement` wrapping any of the above | unwrap to inner declaration; if the inner is `class_declaration` / `function_declaration` / `interface_declaration` / `type_alias_declaration` / `enum_declaration`, treat as that arm. If `export_statement` itself contains a default (i.e., `export default function () {...}` with no name field), emit unit symbol `<mod>.default`. | unwrapped as above, OR `<mod>.default` for nameless default |
| `lexical_declaration` / `variable_declaration` at top level (`const`/`let`/`var`) | glue | `<top-level>` (prefixed) |
| `import_statement`, `export_statement` of bare values | glue | as above |

`mod_path` for TS is empty (TS modules are file-level, not nested class/namespace at the symbol level — interfaces/types DO live in module scope but their names are unit-level, not parent context). Skip TS `namespace` / `module` declarations: emit them as glue for 1B (the explicit-namespace case is rare in modern TS; documented in 1B Risks).

Module prefix: `mod_prefix = module_path_for_tsjs(&asset.workspace_path.0)`. Join with `.` for symbol.

### Steps

- [ ] **Step 1 (fixtures)**:

```typescript
// sample.ts
import { x } from "./other";
const ANSWER = 42;
export interface Greet { hello(): string; }
export type Maybe<T> = T | null;
export function add(a: number, b: number): number { return a + b; }
export class Retriever {
    search(q: string): string[] { return []; }
    static create(): Retriever { return new Retriever(); }
}
export default function () { return 1; }
```

```tsx
// sample.tsx
import React from "react";
export function Hello({ name }: { name: string }) { return <span>{name}</span>; }
export const App = () => <Hello name="x" />;  // arrow fn assigned → glue in 1B
```

- [ ] **Step 2 (failing tests)**: 2 fixture-based tests asserting per-fixture symbols. Asserted symbols (sample.ts):
  - `src/sample.add` (if workspace_path is `src/sample.ts`)
  - `src/sample.Greet`, `src/sample.Maybe`, `src/sample.Retriever`, `src/sample.Retriever.search`, `src/sample.Retriever.create`, `src/sample.default`, `src/sample.<top-level>`.
- For sample.tsx (workspace_path `src/sample.tsx`): `src/sample.Hello`, `src/sample.<top-level>` (App arrow fn rolled into glue).
- Also: `extractor_supports_only_media_code_typescript`, `deterministic_across_runs`.

- [ ] **Step 3**: Run → FAIL.

- [ ] **Step 4**: Implement `typescript.rs` mirroring `rust.rs` scaffold. Grammar selection by file extension. AST walk per the table above. Module prefix application same shape as Python (prefix joined with `.`).

- [ ] **Step 5**: Wire into `lib.rs`:

```rust
pub mod typescript;
pub use typescript::{PARSER_VERSION as TS_PARSER_VERSION, TypescriptAstExtractor};
```

- [ ] **Step 6**: Tests pass, clippy clean, commit.

---

## Task I: TS chunker (`code-ts-ast-v1`)

Pattern identical to Task F — duplicate `code_rust_ast_v1.rs` with substitutions (`VERSION_LABEL = "code-ts-ast-v1"`, struct `CodeTsAstV1Chunker`, error message). Test module copies the Rust chunker tests with name substitutions + adds `policy_hash_matches_md_heading_v1`.

Commit:

```
feat(p10-1b): code-ts-ast-v1 chunker (1:1 + oversize split)
```

---

## Task J: Activate TypeScript in app dispatch

Mirror Task G. Replace TS `bail!` arm with real calls. Add `typescript_file_ingests_and_searches_as_code_citation` integration test using a `src/Foo.ts` fixture.

Commit:

```
feat(p10-1b): activate TypeScript in ingest_one_code_asset dispatch
```

---

## Task K: JavaScript Extractor (`javascript.rs`)

Mirror Task H. tree-sitter-javascript single LanguageFn. AST mapping similar to TS but without `interface_declaration` / `type_alias_declaration` / `enum_declaration`. Module prefix via `module_path_for_tsjs`.

Test fixture `sample.js`:

```javascript
// sample.js
import { x } from "./other";
const ANSWER = 42;
export function add(a, b) { return a + b; }
export class Retriever {
    search(q) { return []; }
    static create() { return new Retriever(); }
}
export default function () { return 1; }
```

Asserted symbols: `src/sample.add`, `src/sample.Retriever`, `src/sample.Retriever.search`, `src/sample.Retriever.create`, `src/sample.default`, `src/sample.<top-level>`.

Wire into `lib.rs`:

```rust
pub mod javascript;
pub use javascript::{PARSER_VERSION as JS_PARSER_VERSION, JavascriptAstExtractor};
```

Commits:

```
feat(p10-1b): tree-sitter-javascript AST extractor (JavascriptAstExtractor)
```

---

## Task L: JS chunker (`code-js-ast-v1`) + Activate JS in app dispatch

Combine Task F + Task G shape for JS in a single commit (less ceremony than splitting since the diffs are tiny):

- Chunker: duplicate-with-substitution from `code_rust_ast_v1.rs`. `VERSION_LABEL = "code-js-ast-v1"`, struct `CodeJsAstV1Chunker`.
- App dispatch: replace JS `bail!` with real calls.
- Integration test: `javascript_file_ingests_and_searches_as_code_citation`.

Commit:

```
feat(p10-1b): code-js-ast-v1 chunker + activate JS in app dispatch
```

---

## Task M: Snapshots + full-suite gate + manual SMOKE

**Files:**
- Create: `crates/kebab-chunk/tests/code_python_ast_snapshot.rs` + fixture `tests/fixtures/code-sample.py` + baseline `code-sample.chunks.snapshot.json`
- Create: same for TS (`code_ts_ast_snapshot.rs` + fixture `.ts` + baseline)
- Create: same for JS (`code_js_ast_snapshot.rs` + fixture `.js` + baseline)

Mirror `crates/kebab-chunk/tests/code_rust_ast_snapshot.rs` exactly for each language. Build the `CanonicalDocument` IN-MEMORY (no `kebab-parse-code` dep crossing the chunk boundary).

- [ ] **Step 1**: Add the 3 snapshot tests. Generate baselines: `UPDATE_SNAPSHOTS=1 cargo test -p kebab-chunk code_{python,ts,js}_ast_snapshot`. Re-run without env var → PASS.

- [ ] **Step 2**: Full-suite gate (memory-conscious):
  - `cargo clippy --workspace --all-targets -- -D warnings` (one invocation, no parallel).
  - `cargo test --workspace --no-fail-fast -j 1` (the `-j 1` is mandatory). If the pre-existing `runner_lexical_is_deterministic_per_query_payload` flake reappears (unlikely — was fixed in PR #141 on main and merged before 1B branch was cut), re-run that single test once.

- [ ] **Step 3**: Manual SMOKE (mirror `docs/SMOKE.md` P10-1A-2 flow for each language):

```bash
cargo build --release
rm -rf /tmp/kebab-1bsmoke && mkdir -p /tmp/kebab-1bsmoke/ws/{kebab_eval,src}
echo 'def compute_mrr(): return 1.0' > /tmp/kebab-1bsmoke/ws/kebab_eval/metrics.py
echo 'export function add(a,b){return a+b;}' > /tmp/kebab-1bsmoke/ws/src/foo.ts
echo 'export function sub(a,b){return a-b;}' > /tmp/kebab-1bsmoke/ws/src/bar.js
# (match isolated config block format from docs/SMOKE.md)
./target/release/kebab --config /tmp/kebab-1bsmoke/config.toml ingest --json | jq '.items[].parser_version' | sort -u
./target/release/kebab --config /tmp/kebab-1bsmoke/config.toml search "compute_mrr" --code-lang python --json | jq '.hits[0]'
./target/release/kebab --config /tmp/kebab-1bsmoke/config.toml schema --json | jq '.stats.code_lang_breakdown'
```

Expected: parser_versions include `code-python-v1`, `code-ts-v1`, `code-js-v1`. Search returns `Citation::Code { lang: "python", symbol: "kebab_eval.metrics.compute_mrr" }`. `code_lang_breakdown` includes all four langs (rust may be 0 unless you also added a .rs).

- [ ] **Step 4**: Commit (snapshot files + any harness tweaks).

```bash
git add crates/kebab-chunk/tests/
git commit -m "test(p10-1b): per-language chunker snapshots + full-suite gate"
```

---

## Task N: Docs + HOTFIXES + version bump

- README: 지원 형식 / 명령 table row adds Python / TypeScript / JavaScript next to Rust. Mermaid stays unchanged (no new external surface crosses the diagram).
- HANDOFF: P10 row notes 1B merged (3 langs active). Add a one-line entry under 머지 후 결정 cross-linking the HOTFIXES entries.
- ARCHITECTURE: dependency-graph edge `pcode → core` already present. The new tree-sitter-{python,typescript,javascript} edges to `pcode` add to the description text. Locked-in decisions table: add "1B symbol path: workspace path → module path (Python dotted, TS/JS slash-style); Rust 1A keeps file-scope nesting only — HOTFIXES 2026-05-20".
- SMOKE: add 1B section mirroring the 1A-2 P10 section structure (config block, ingest / search / schema verification commands) for Python and TS/JS. Compact — one shared section for all three.
- tasks/INDEX + tasks/p10/INDEX: flip 1B row 🟡→🟢 (on PR open; ✅ on merge).
- tasks/HOTFIXES.md: TWO dated 2026-05-20 entries:
  1. **Rust 1A-2 symbol path is file-scope-only; 1B+ uses workspace path → module prefix**. Cross-link to design §3.4. Acceptable inconsistency for now (cost of 1A retrofit = chunker_version bump + reindex for every existing Rust corpus). User-requested retrofit triggers a separate task.
  2. **Expression-level functions (arrow fn / function expression assigned to const) NOT emitted as separate units in 1B 1차**. They fold into the `<top-level>` glue. Documented limit; future phase may add `lexical_declaration` → inner-expression unwrap.
  Cross-link both in `tasks/p10/p10-1b-py-ts-js-ast-chunkers.md` Risks/notes.
- `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md` §10.1: add a one-liner — "p10-1B 활성화 (Python / TypeScript / JavaScript)".
- `Cargo.toml`: workspace version `0.7.0 → 0.8.0`. `cargo build --release` refreshes Cargo.lock.
- One commit:

```bash
git add -A
git commit -m "docs(p10-1b): README/HANDOFF/ARCHITECTURE/SMOKE/INDEX + HOTFIXES; chore: bump version 0.7.0 → 0.8.0

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Finalize

- `gitea-pr` open the PR (gitea-ops skill) — title `feat(p10-1B): Python + TS/JS AST chunkers — tree-sitter-{python,typescript,javascript} 코드 색인 활성화`.
- **Review loop mode** (fixed per workflow memory) until APPROVE → merge → main pull → branch cleanup → `cargo clean` → `gitea-release v0.8.0`.

---

## Self-review checklist (filled by plan author)

- **Spec coverage**: every row of design §1B has a task; §3.4 symbol path covered by Task C + per-language extractors + integration tests; §6.1/§6.2 module structure covered by Tasks E/F/H/I/K/L; §9.1 Tier-1 + oversize fallback inherited from 1A-2 chunker pattern (Tasks F/I/L); §3.5 code_lang already in 1A-2 helper, extended in Task B routing; §5 dispatch covered by Task D; cascade rule (versioning §9) — chunker versions are per-language, fixture snapshots lock behavior.
- **No placeholders**: all novel logic (module_path helpers, app dispatch generalization, Python AST walk rules) given concretely with full code or exact deltas vs 1A-2. The per-language chunkers are explicit "duplicate code_rust_ast_v1.rs with substitution X/Y/Z" — concrete and verifiable, not vague.
- **Type consistency**: parser_version constants (`code-{rust,python,ts,js}-v1`) and chunker_version labels (`code-{rust,python,ts,js}-ast-v1`) used consistently across Tasks D/E/F/G/H/I/J/K/L. `module_path_for_python` / `module_path_for_tsjs` referenced consistently as the source of truth for prefixing.
