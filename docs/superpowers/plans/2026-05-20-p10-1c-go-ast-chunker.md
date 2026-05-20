# p10-1C-Go Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task.

**Goal:** Activate Go code ingest end-to-end on top of 1A-2 (Rust) + 1B (Python/TS/JS) infrastructure. Add `tree-sitter-go` grammar + `GoAstExtractor` + `code-go-ast-v1` chunker + media routing + app dispatch arm.

**Architecture:** Mirror 1A-2 / 1B exactly. `kebab-parse-code/src/go.rs` walks tree-sitter-go parse tree; emits one `Block::Code` per top-level AST semantic unit with `SourceSpan::Code { symbol, lang: Some("go") }`. Symbol prefix = **source-extracted package name** (from `package_clause` AST node — design §3.4 Go row). `kebab-chunk/src/code_go_ast_v1.rs` is a near-duplicate of `code-rust-ast-v1`. App dispatch's `ingest_one_code_asset` (PR #142 generalized 4-arm match) gets a 5th arm.

**Tech Stack:** Rust 2024 workspace, `tree-sitter` 0.26 (already in workspace), `tree-sitter-go` (NEW), 1A-2/1B infrastructure unchanged.

**Memory note:** Host has been OOM-killed previously. Use `cargo test -p <crate>` and `cargo check -p <crate>` only. ONE full-suite invocation reserved for Task G gate.

---

## Pre-flight

Branch `feat/p10-1c-go` already exists.

- [ ] **Disk hygiene**: `cargo clean` if previous artifacts are bloated. Skip if disk is comfortable (`df -h /`).

Reference files:
- 1A-2 Rust extractor: `crates/kebab-parse-code/src/rust.rs` — closest single-language scaffold template.
- 1B Python extractor (closest analog for "class-nesting recursion" — Go doesn't have classes but has package as the single prefix): `crates/kebab-parse-code/src/python.rs`.
- 1A-2 chunker scaffold: `crates/kebab-chunk/src/code_rust_ast_v1.rs`.
- 1B dispatch generalization: `crates/kebab-app/src/lib.rs::ingest_one_code_asset` (~L1645, 4-arm match).
- 1A-2 source-fs routing: `crates/kebab-source-fs/src/media.rs` `"rs" =>` arm.

---

## Task A: Workspace dep `tree-sitter-go`

**Files:**
- Modify: `Cargo.toml` (workspace `[workspace.dependencies]`, after `tree-sitter-javascript` line)
- Modify: `crates/kebab-parse-code/Cargo.toml`

- [ ] **Step 1**: `cargo add tree-sitter-go -p kebab-parse-code` to resolve version.

- [ ] **Step 2**: Lift the resolved version into `[workspace.dependencies]` after `tree-sitter-javascript`:

```toml
# Go grammar for code ingest (kebab-parse-code, p10-1C).
tree-sitter-go         = "<resolved>"
```

Switch the crate's entry to `{ workspace = true }` matching existing tree-sitter-* style.

- [ ] **Step 3**: `cargo build -p kebab-parse-code` → clean. Unused dep warning is fine.

- [ ] **Step 4**: Commit:

```bash
git add Cargo.toml Cargo.lock crates/kebab-parse-code/Cargo.toml
git commit -m "build(p10-1c-go): add tree-sitter-go workspace dep

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task B: source-fs media routing `.go` → `MediaType::Code("go")`

**Files:**
- Modify: `crates/kebab-source-fs/src/media.rs` (add arm after the existing JS arm at ~L44)
- Test: same file's test module

- [ ] **Step 1 (failing test)** — add to existing tests near `py_ts_js_files_map_to_media_code`:

```rust
#[test]
fn go_files_map_to_media_code_go() {
    assert_eq!(media_type_for(Path::new("a/b.go")), MediaType::Code("go".into()));
}
```

- [ ] **Step 2**: Run → FAIL.

- [ ] **Step 3**: Add the arm before the catch-all `_ => MediaType::Other(ext)`:

```rust
        // p10-1C-Go: Go ingest activated.
        "go" => MediaType::Code("go".into()),
```

- [ ] **Step 4**: Run → PASS. `cargo test -p kebab-source-fs` → no regression.

- [ ] **Step 5**: clippy clean, commit:

```bash
git add crates/kebab-source-fs/
git commit -m "feat(p10-1c-go): route .go to MediaType::Code(go)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task C: App dispatch allowlist + bail arm for "go"

**Files:**
- Modify: `crates/kebab-app/src/lib.rs` (dispatch match guard + 4 internal match arms in `ingest_one_code_asset`)

- [ ] **Step 1**: Find the `MediaType::Code(lang) if matches!(lang.as_str(), "rust" | "python" | "typescript" | "javascript")` arm (~L953). Add `"go"` to the allowlist:

```rust
        MediaType::Code(lang)
            if matches!(lang.as_str(), "rust" | "python" | "typescript" | "javascript" | "go") =>
        {
```

- [ ] **Step 2**: In `ingest_one_code_asset`'s 4 `match code_lang` blocks (parser_version, chunker_version, extract, chunk), add a "go" arm that `bail!()`s for now (extractor + chunker land in Task D/E). Mirror the Python/TS/JS bail-then-activate pattern:

```rust
let parser_version = match code_lang {
    // ... existing arms ...
    "go" => anyhow::bail!("go ingest not yet wired (p10-1c-go Task F)"),
    other => anyhow::bail!("unsupported code_lang: {other}"),
};
// similar for chunker_version / extract / chunk matches
```

- [ ] **Step 3**: `cargo test -p kebab-app --lib` → existing 52 lib tests stay green. `cargo test -p kebab-app --test code_ingest_smoke` → 6 stay green (Rust path unaffected).

- [ ] **Step 4**: clippy clean, commit:

```bash
git add crates/kebab-app/
git commit -m "refactor(p10-1c-go): add go to ingest dispatch allowlist (bail until Task F)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task D: `GoAstExtractor` (`kebab-parse-code/src/go.rs`)

**Files:**
- Create: `crates/kebab-parse-code/src/go.rs`
- Modify: `crates/kebab-parse-code/src/lib.rs` (`pub mod go;` + re-exports `GO_PARSER_VERSION`, `GoAstExtractor`)
- Create: `crates/kebab-parse-code/tests/fixtures/sample.go`

Scaffold mirrors `crates/kebab-parse-code/src/rust.rs` line-for-line for the `CanonicalDocument` skeleton (Extractor trait impl, `id_for_doc`, ProvenanceEvent, final `CanonicalDocument` literal). The novel parts:

### Constants

```rust
pub const PARSER_VERSION: &str = "code-go-v1";

pub struct GoAstExtractor;
// new() + Default
// supports: matches!(m, MediaType::Code(l) if l == "go")
// agent = "kb-parse-code"
// metadata.code_lang = Some("go")
// SourceType::Note (no SourceType::Code variant)
// repo/git_branch/git_commit via detect_repo
```

### Package extraction

Unlike 1B's path-based `module_path_for_python` / `_for_tsjs`, the Go package prefix comes from the **source code's `package` declaration** (design §3.4). tree-sitter-go's grammar:

- Root: `source_file`
- First named child is typically `package_clause` → contains `package_identifier` child whose text is the package name.

Helper (local to `go.rs`):

```rust
/// Returns the package name from a tree-sitter-go `source_file`, or
/// `None` if the file has no `package_clause` (invalid Go in practice,
/// but be defensive).
fn extract_package(root: tree_sitter::Node, src: &str) -> Option<String> {
    let mut cur = root.walk();
    for child in root.named_children(&mut cur) {
        if child.kind() == "package_clause" {
            // `package_clause` has a `package_identifier` named child.
            let mut c2 = child.walk();
            for sub in child.named_children(&mut c2) {
                if sub.kind() == "package_identifier" {
                    return Some(src[sub.start_byte()..sub.end_byte()].to_string());
                }
            }
        }
    }
    None
}
```

### Semantic-unit rules

| node kind | unit | symbol |
|-----------|------|--------|
| `function_declaration` (name field) | 1 | `<pkg>.<fn_name>` |
| `method_declaration` | 1 | `<pkg>.(<TypeText>).<MethodName>` where `<TypeText>` includes a leading `*` if the receiver is `pointer_type`. Examples: `chunk.(*MdHeadingV1Chunker).ChunkDoc`, `chunk.(Foo).Bar`. |
| `type_declaration` (struct / interface / type alias) | 1 per inner `type_spec` | `<pkg>.<TypeName>` |
| `const_declaration`, `var_declaration`, `import_declaration` (single or block) | glue | `<pkg>.<top-level>` (or `<pkg>.<package>` if file has ZERO real units AND glue is import-only — same `<module>` post-pass pattern as 1B Python, renamed to `<package>` to avoid colliding with Go's `package` keyword? — actually use `<module>` per design §3.4 — see "module / namespace 만 있고 symbol 없는 경우" line) |

`unit_start` walks `comment` siblings (same as 1B). Go doesn't have separate attribute / decorator nodes.

Method receiver pointer detection:

```rust
// In the method_declaration arm:
let receiver = child.child_by_field_name("receiver");  // parameter_list
let receiver_type_text = receiver.and_then(|r| {
    let mut cw = r.walk();
    for p in r.named_children(&mut cw) {
        if p.kind() == "parameter_declaration" {
            // type field is either type_identifier (value) or pointer_type (ptr)
            if let Some(ty) = p.child_by_field_name("type") {
                let s = &src[ty.start_byte()..ty.end_byte()];
                return Some(s.to_string());  // includes leading "*" if pointer_type
            }
        }
    }
    None
});
// Format: "(*Foo)" or "(Foo)" — wrap in parens, preserve leading "*" if any.
let owner = receiver_type_text
    .map(|t| format!("({t})"))
    .unwrap_or_else(|| "()".to_string());
let method_name = name_text(&child, src);
// symbol = format!("{pkg}.{owner}.{method_name}")
```

Read tree-sitter-go's grammar.json or node-types.json (in the registry source) if any field name above differs in the resolved crate version.

### Fixture `tests/fixtures/sample.go`:

```go
// sample.go
package chunk

import (
	"fmt"
	"strings"
)

const Version = "v1"

type MdHeadingV1Chunker struct {
	Name string
}

// ChunkDoc returns a stub list of strings.
func (m *MdHeadingV1Chunker) ChunkDoc(input string) []string {
	return []string{m.Name}
}

func (m MdHeadingV1Chunker) Name2() string {
	return m.Name
}

type Stringer interface {
	String() string
}

func Free(x int) int {
	return x + 1
}

func init() {
	fmt.Println(strings.ToUpper("init"))
}
```

### Test module

Mirror Python's test shape (use `crate::rust::tests_support::fixed_code_asset` from 1B):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use kebab_core::{Block, MediaType, SourceSpan};

    fn extract_fixture() -> kebab_core::CanonicalDocument {
        let bytes = std::fs::read(
            concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/sample.go"),
        ).unwrap();
        let asset = crate::rust::tests_support::fixed_code_asset(
            "crates/x/src/sample.go", "go",
        );
        let cfg = kebab_core::ExtractConfig::default();
        let root = std::path::PathBuf::from("/tmp");
        let ctx = kebab_core::ExtractContext { asset: &asset, workspace_root: &root, config: &cfg };
        GoAstExtractor::new().extract(&ctx, &bytes).unwrap()
    }

    #[test]
    fn extractor_supports_only_media_code_go() {
        let e = GoAstExtractor::new();
        assert!(e.supports(&MediaType::Code("go".into())));
        assert!(!e.supports(&MediaType::Code("rust".into())));
        assert!(!e.supports(&MediaType::Markdown));
    }

    #[test]
    fn go_units_match_design_3_4_symbols() {
        let doc = extract_fixture();
        let mut syms: Vec<String> = doc.blocks.iter().filter_map(|b| match b {
            Block::Code(c) => match &c.common.source_span {
                SourceSpan::Code { symbol, lang, .. } => {
                    assert_eq!(lang.as_deref(), Some("go"));
                    symbol.clone()
                }
                _ => None,
            },
            _ => None,
        }).collect();
        syms.sort();
        assert!(syms.iter().any(|s| s == "chunk.Free"), "got {syms:?}");
        assert!(syms.iter().any(|s| s == "chunk.init"));
        assert!(syms.iter().any(|s| s == "chunk.MdHeadingV1Chunker"));
        assert!(syms.iter().any(|s| s == "chunk.(*MdHeadingV1Chunker).ChunkDoc"));
        assert!(syms.iter().any(|s| s == "chunk.(MdHeadingV1Chunker).Name2"));
        assert!(syms.iter().any(|s| s == "chunk.Stringer"));
        assert!(syms.iter().any(|s| s == "chunk.<top-level>"));  // import + const grouped
    }

    #[test]
    fn deterministic_across_runs() {
        let a = extract_fixture();
        for _ in 0..50 { assert_eq!(extract_fixture().blocks, a.blocks); }
    }
}
```

### Step list

- [ ] Step 1: create fixture + test module.
- [ ] Step 2: run → FAIL (`GoAstExtractor` undefined).
- [ ] Step 3: implement `go.rs`. Scaffold mirrors `python.rs` (Extractor impl + extract scaffold + `build_blocks` returning blocks). `build_blocks` does: extract_package → walk root's named children → branch per node kind per the table above → emit `Block::Code` with `SourceSpan::Code { symbol, lang: Some("go") }`. Use the same `flush_glue` / glue grouping / `<top-level>` vs `<module>` post-pass as Python (rename to `<package>` if user prefers, but spec §3.4 says `<module>` so keep that name for cross-language consistency).
- [ ] Step 4: wire into `lib.rs`:

```rust
pub mod go;
pub use go::{PARSER_VERSION as GO_PARSER_VERSION, GoAstExtractor};
```

- [ ] Step 5: `cargo test -p kebab-parse-code` → all pass (Rust/Python/TS/JS + new Go). `cargo clippy -p kebab-parse-code --all-targets -- -D warnings` clean.
- [ ] Step 6: commit:

```bash
git add crates/kebab-parse-code/
git commit -m "feat(p10-1c-go): tree-sitter-go AST extractor (GoAstExtractor)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task E: `code-go-ast-v1` chunker

**Files:**
- Create: `crates/kebab-chunk/src/code_go_ast_v1.rs`
- Modify: `crates/kebab-chunk/src/lib.rs`

Identical pattern to PR #142 Task I (TS) / Task L (JS) — near-duplicate of `code_rust_ast_v1.rs` with substitutions:
- `const VERSION_LABEL: &str = "code-go-ast-v1";`
- struct name `CodeGoAstV1Chunker`
- error message says `"CodeGoAstV1Chunker only handles..."`
- module doc-comment prose `Rust` → `Go`, `code-rust-ast-v1` → `code-go-ast-v1`

`split_oversize` / `make_chunk` / `AST_CHUNK_MAX_LINES = 200` / `BYTES_PER_TOKEN = 3` / `POLICY_HASH_HEX_LEN = 16` IDENTICAL (language-agnostic).

Test module: copy from `code_ts_ast_v1.rs` and substitute names. KEEP cross-chunker `policy_hash_matches_md_heading_v1`.

Wire into `crates/kebab-chunk/src/lib.rs`:

```rust
mod code_go_ast_v1;
pub use code_go_ast_v1::CodeGoAstV1Chunker;
```

(Alphabetical placement.)

Verify + commit:
- `cargo test -p kebab-chunk code_go_ast` PASS (~6 tests)
- `cargo test -p kebab-chunk` full per-crate green
- `cargo clippy -p kebab-chunk --all-targets -- -D warnings` clean

```bash
git add crates/kebab-chunk/
git commit -m "feat(p10-1c-go): code-go-ast-v1 chunker (1:1 + oversize split)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task F: Activate Go in app dispatch

**Files:**
- Modify: `crates/kebab-app/src/lib.rs` (replace 4 "go" bail! arms with real calls)
- Modify: `crates/kebab-app/tests/code_ingest_smoke.rs` (add Go integration test)

Replace the 4 `"go" => anyhow::bail!(...)` arms in `ingest_one_code_asset` (added in Task C) with real:

```rust
"go" => ParserVersion(kebab_parse_code::GO_PARSER_VERSION.to_string()),
// ...
"go" => CodeGoAstV1Chunker.chunker_version(),
// ...
"go" => kebab_parse_code::GoAstExtractor::new()
    .extract(&ctx, &bytes)
    .context("kb-parse-code::GoAstExtractor::extract (code:go)")?,
// ...
"go" => CodeGoAstV1Chunker
    .chunk(&canonical, chunk_policy)
    .context("kb-chunk::CodeGoAstV1Chunker::chunk (code:go)")?,
```

Add imports at top of lib.rs:
- `kebab_chunk::CodeGoAstV1Chunker`
- `kebab_parse_code::GoAstExtractor`

Integration test (mirror PR #142's `python_file_ingests_and_searches_as_code_citation`):

```rust
#[test]
fn go_file_ingests_and_searches_as_code_citation() {
    // ... TempDir + Config harness same as Python/TS test ...
    let pkg_dir = env.workspace_root.join("chunk");
    std::fs::create_dir_all(&pkg_dir).unwrap();
    std::fs::write(
        pkg_dir.join("ast.go"),
        "package chunk\n\nfunc ParseDoc(input string) string {\n    return input\n}\n",
    ).unwrap();

    let report = kebab_app::ingest_with_config(/* ... */).unwrap();
    assert!(report.new >= 1);
    let go_item = report.items.as_ref().unwrap().iter()
        .find(|i| i.doc_path.0.ends_with("ast.go")).expect("ast.go item");
    assert_eq!(go_item.parser_version.as_ref().unwrap().0, "code-go-v1");
    assert_eq!(go_item.chunker_version.as_ref().unwrap().0, "code-go-ast-v1");

    let hits = kebab_app::search_with_config(/* search "ParseDoc" */).unwrap();
    let h = hits.iter().find(|h| matches!(h.citation, kebab_core::Citation::Code { .. }))
        .expect("Citation::Code hit");
    match &h.citation {
        kebab_core::Citation::Code { lang, symbol, line_start, .. } => {
            assert_eq!(lang.as_deref(), Some("go"));
            assert_eq!(symbol.as_deref(), Some("chunk.ParseDoc"));
            assert!(*line_start >= 1);
        }
        _ => unreachable!(),
    }
    assert_eq!(h.code_lang.as_deref(), Some("go"));
}
```

Verify:
- `cargo test -p kebab-app --test code_ingest_smoke` → 7/7 (6 existing + 1 new go)
- `cargo test -p kebab-app --lib` → 52/52 (no regression)
- clippy clean

```bash
git add crates/kebab-app/
git commit -m "feat(p10-1c-go): activate Go in ingest_one_code_asset dispatch

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task G: Snapshot + full-suite gate + manual SMOKE

**Files:**
- Create: `crates/kebab-chunk/tests/code_go_ast_snapshot.rs` + fixture + baseline (mirror `code_python_ast_snapshot.rs` from PR #142)

- [ ] **Step 1**: Add snapshot integration test. In-memory `CanonicalDocument` (no kebab-parse-code dep — boundary §6.3). Generate baseline: `UPDATE_SNAPSHOTS=1 cargo test -p kebab-chunk code_go_ast_snapshot` → re-run without env → PASS.

- [ ] **Step 2**: Full-suite gate (the ONE invocation allowed this PR):

```bash
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --no-fail-fast -j 1
```

Both must be CLEAN/GREEN.

- [ ] **Step 3**: Manual SMOKE (optional but recommended — mirror PR #142 SMOKE):

```bash
cargo build --release   # OR debug if RAM-tight
rm -rf /tmp/kebab-go-smoke && mkdir -p /tmp/kebab-go-smoke/ws/chunk
echo 'package chunk

func ParseDoc(input string) string { return input }
' > /tmp/kebab-go-smoke/ws/chunk/ast.go
# adapt isolated config from docs/SMOKE.md
./target/release/kebab --config /tmp/kebab-go-smoke/config.toml ingest --json | jq '.items[].parser_version' | sort -u
./target/release/kebab --config /tmp/kebab-go-smoke/config.toml search "ParseDoc" --code-lang go --json | jq '.hits[0]'
```

Expected: `code-go-v1` in parser_versions; Citation::Code with symbol `chunk.ParseDoc`.

- [ ] **Step 4**: Commit snapshot only (full-suite + SMOKE are gates, not commit content):

```bash
git add crates/kebab-chunk/tests/
git commit -m "test(p10-1c-go): code-go-ast-v1 chunker snapshot + full-suite gate

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task H: Docs + version bump

- README: 지원 형식 row — add Go (`.go`, `code-go-ast-v1`).
- HANDOFF: P10 phase row note 1C-Go merged (Go active). Java/Kotlin remain pending.
- ARCHITECTURE: directory tree note for kebab-parse-code includes `go.rs` (Java/Kotlin coming in next PR). Decisions table — no new row (1C-Go follows the 1A-2/1B convention).
- SMOKE: extend the P10 section with a 1-line note for Go (or compact Go example).
- tasks/INDEX + tasks/p10/INDEX: flip the row for 1C-Go to 🟡 (PR open) → ✅ on merge. The 1C row in p10/INDEX may need a split — `p10-1C-Go ⏳ → 🟡` and `p10-1C-JavaKotlin ⏳ unchanged` (since user split into 2 PRs).
- frozen design §10.1: add a one-liner — "p10-1C-Go 활성화 (Go)" (Java/Kotlin will get its own line in the next PR).
- `Cargo.toml`: workspace version `0.11.1 → 0.12.0` (minor — dogfooding surface 확장, 새 chunker + extractor 활성화).

```bash
git add -A
git commit -m "docs(p10-1c-go): README/HANDOFF/ARCHITECTURE/SMOKE/INDEX + chore: bump version 0.11.1 → 0.12.0

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Finalize: PR + review loop + release

Per workflow memory (gitea-pr + review loop, no single-shot):

- [ ] `gitea-pr` → PR title `feat(p10-1C-Go): tree-sitter-go AST extractor + chunker — Go 코드 색인 활성화`
- [ ] Review loop until APPROVE → merge → main pull → branch cleanup → `cargo clean` → `gitea-release v0.12.0`.

---

## Self-Review (filled by plan author)

- **Spec coverage**: design §1C Go (extractor + chunker + activation) → Tasks D/E/F; §3.3 (`code-go-ast-v1`) → Task E; §3.4 symbol path → Task D (extract_package + method receiver pointer detection); §6.1 (`kebab-parse-code/src/go.rs`) → Task D; §6.2 (`kebab-chunk/src/code_go_ast_v1.rs`) → Task E; §6.3 dep graph (`tree-sitter-go` parser-side) → Task A; §9.1 Tier-1 + oversize fallback → Task E (1A-2 split_oversize reused identically).
- **No placeholders**: novel logic (`extract_package`, method receiver pointer detection, fixture, test assertions, dispatch arm additions) given concretely. Mechanical mirrors (chunker, integration test, snapshot test) pinned to exact existing files with substitutions.
- **Type consistency**: `GoAstExtractor` / `GO_PARSER_VERSION = "code-go-v1"` / `CodeGoAstV1Chunker` / `VERSION_LABEL = "code-go-ast-v1"` used consistently across Tasks A-H. `MediaType::Code("go")` in routing + dispatch. `Citation::Code` with `lang: Some("go")` in integration test.
