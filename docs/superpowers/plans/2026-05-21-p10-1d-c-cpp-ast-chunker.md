# p10-1D C + C++ AST Chunkers Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Activate C + C++ code ingest end-to-end. P10 Tier 1 chunker family final entry.

**Architecture:** Same shape as 1B (multi-language single PR) and 1C-JK (JVM family). 2 new tree-sitter grammars + 2 extractors + 2 chunkers + media routing (delegated via `code_lang_for_path`, no change) + app dispatch arms. C symbol = function name only; C++ symbol = `namespace::Class::method` via recursive class/namespace nesting (Java/Kotlin + Python hybrid).

**Tech Stack:** Rust 2024 workspace, `tree-sitter` 0.26 (already), `tree-sitter-c` + `tree-sitter-cpp` (NEW). 1A-2/1B/1C/p10-2/p10-3 infrastructure unchanged.

**Memory note:** Host has been OOM'd previously (재부팅 사례). Per-crate cargo only. ONE full-suite + clippy invocation in Task J. NO `cargo test --workspace` outside that gate.

---

## Pre-flight

Branch `feat/p10-1d-c-cpp` already exists (spec commit `8add684`).

- [ ] **Disk hygiene**: `df -h /` 점검. 80% 넘으면 `cargo clean`.

Reference files:
- 1C-JK extractor: `crates/kebab-parse-code/src/{java,kotlin}.rs` — closest template for source-side identifier prefix (package vs namespace).
- 1B Python extractor: `crates/kebab-parse-code/src/python.rs` — class-nesting recursion model (relevant for C++ class nesting).
- 1A-2 chunker: `crates/kebab-chunk/src/code_rust_ast_v1.rs` — duplicate-with-substitution pattern.
- 1B/1C/p10-2/p10-3 dispatch generalization: `crates/kebab-app/src/lib.rs::ingest_one_code_asset` (~L1796–2116). Current allowlist + 4-arm match.
- spec: `tasks/p10/p10-1d-c-cpp-ast-chunker.md`.

---

## Task A: Workspace deps (tree-sitter-c + tree-sitter-cpp)

**Files:**
- Modify: `Cargo.toml` (`[workspace.dependencies]`, after `tree-sitter-kotlin-ng`)
- Modify: `crates/kebab-parse-code/Cargo.toml`

- [ ] **Step 1**: `cargo add tree-sitter-c tree-sitter-cpp -p kebab-parse-code`. If either crate's actively-maintained name differs (e.g. `tree-sitter-cpp` vs `tree-sitter-cpp-ng`), verify on crates.io. The `tree-sitter-c` 0.24 / `tree-sitter-cpp` 0.23 line is the most common; verify compatibility with workspace `tree-sitter = "0.26"` (likely already supported via the `tree-sitter-language` shim).

- [ ] **Step 2**: Lift the two resolved versions into `[workspace.dependencies]` (after `tree-sitter-kotlin-ng`):

```toml
# C/C++ family grammars for code ingest (kebab-parse-code, p10-1D).
tree-sitter-c          = "<resolved>"
tree-sitter-cpp        = "<resolved>"
```

Switch crate's `Cargo.toml` entries to `{ workspace = true }`.

- [ ] **Step 3**: `cargo build -p kebab-parse-code` → clean. Unused dep warning is fine.

- [ ] **Step 4**: Commit:

```bash
git add Cargo.toml Cargo.lock crates/kebab-parse-code/Cargo.toml
git commit -m "$(cat <<'EOF'
build(p10-1d): add tree-sitter-c + tree-sitter-cpp workspace deps

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

If a crate's resolved name has a non-obvious fork suffix (e.g. `tree-sitter-cpp-ng`), document it in the commit body.

---

## Task B: C AST extractor (`kebab-parse-code/src/c.rs`)

**Files:**
- Create: `crates/kebab-parse-code/src/c.rs`
- Modify: `crates/kebab-parse-code/src/lib.rs` (pub mod + `C_PARSER_VERSION` const)

- [ ] **Step 1**: Create `crates/kebab-parse-code/src/c.rs`. Mirror `crates/kebab-parse-code/src/go.rs` (closest template — single-language, no namespace/package nesting, top-level units). Replace tree-sitter-go with tree-sitter-c:

```rust
//! p10-1D: C AST extractor.

use crate::traits::{Extractor, ExtractContext};
use anyhow::{Context, Result};
use kebab_core::{Block, BlockId, CanonicalDocument, CodeBlock, CommonBlock, /*..*/, SourceSpan, id_for_block, id_for_doc};
use tree_sitter::Parser;

pub const C_PARSER_VERSION: &str = concat!("tree-sitter-c-", env!("CARGO_PKG_VERSION"));
// Or use the tree-sitter-c crate version: better to hardcode for stability.
// Look at how go.rs / rust.rs / etc. set their PARSER_VERSION.

pub struct CAstExtractor {
    parser: Parser,
}

impl CAstExtractor {
    pub fn new() -> Self {
        let mut parser = Parser::new();
        parser.set_language(&tree_sitter_c::LANGUAGE.into()).expect("load tree-sitter-c");
        Self { parser }
    }
}

impl Extractor for CAstExtractor {
    fn extract(&mut self, ctx: &ExtractContext, bytes: &[u8]) -> Result<CanonicalDocument> {
        // ... mirror go.rs:
        //   1. parse the tree
        //   2. iterate source_file's named_children
        //   3. for each top-level node:
        //      - function_definition → emit unit (symbol = fn name)
        //      - struct_specifier (named) → emit unit (symbol = struct name)
        //      - enum_specifier (named) → emit unit (symbol = enum name)
        //      - union_specifier (named) → emit unit (symbol = union name)
        //      - declaration → glue
        //      - preproc_include / preproc_def / preproc_function_def / preproc_ifdef → glue
        //      - else → glue
        //   4. <top-level> glue chunk if any glue accumulated
        //   5. <module> post-pass if 0 units
        // ...
        todo!("mirror go.rs structure with C-specific node-kind names")
    }
}
```

**ACTION**: Read `crates/kebab-parse-code/src/go.rs` in full first. It's the closest template — single-language, no namespace prefix to thread through (C is even simpler than Go since there's no `package`). Port the structure: parse → iterate top-level → match on node-kind → emit units or accumulate glue.

Node-kind name reference (tree-sitter-c): `function_definition`, `struct_specifier`, `enum_specifier`, `union_specifier`, `declaration`, `preproc_*`. Confirm by checking the crate's `node-types.json` if uncertain.

**Function name extraction**: `function_definition` has a `declarator` field. The innermost `identifier` of that declarator is the function name. Mirror how go.rs extracts function names — it uses tree-sitter field traversal.

- [ ] **Step 2**: Register the module in `crates/kebab-parse-code/src/lib.rs`:

```rust
pub mod c;
pub use c::{CAstExtractor, C_PARSER_VERSION};
```

- [ ] **Step 3**: Build:

```bash
cargo build -p kebab-parse-code 2>&1 | tail -5
```

Expected: clean.

- [ ] **Step 4**: Commit (no test yet — Task D adds the snapshot test):

```bash
git add crates/kebab-parse-code/src/c.rs crates/kebab-parse-code/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(p10-1d): C AST extractor (tree-sitter-c)

Top-level units: function_definition (symbol = fn name), struct_specifier,
enum_specifier, union_specifier (each emits 1 unit with the symbol being
the named identifier). Preprocessor directives + top-level declarations
group into a <top-level> glue chunk. Empty file or zero units → <module>
post-pass.

C symbol = function name only — no namespace, no class nesting (design §3.4).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task C: C++ AST extractor (`kebab-parse-code/src/cpp.rs`)

**Files:**
- Create: `crates/kebab-parse-code/src/cpp.rs`
- Modify: `crates/kebab-parse-code/src/lib.rs`

- [ ] **Step 1**: Create `crates/kebab-parse-code/src/cpp.rs`. The closest template is `crates/kebab-parse-code/src/java.rs` (1C-JK) — it handles package prefix + class nesting via recursion. C++ adds namespace nesting (multiple levels possible).

Pseudocode:

```rust
//! p10-1D: C++ AST extractor.

use crate::traits::{Extractor, ExtractContext};
use anyhow::{Context, Result};
use kebab_core::{/* ... */};
use tree_sitter::{Node, Parser};

pub const CPP_PARSER_VERSION: &str = "tree-sitter-cpp-<resolved>";

pub struct CppAstExtractor { parser: Parser }

impl CppAstExtractor {
    pub fn new() -> Self {
        let mut parser = Parser::new();
        parser.set_language(&tree_sitter_cpp::LANGUAGE.into()).expect("load tree-sitter-cpp");
        Self { parser }
    }

    fn visit(&self, node: Node, source: &[u8], prefix: &[&str], units: &mut Vec<(String, Node)>, glue: &mut Vec<Node>) {
        // prefix is the namespace/class chain so far (e.g. ["kebab", "chunk", "MdHeadingV1Chunker"]).
        for child in node.named_children(&mut node.walk()) {
            match child.kind() {
                "namespace_definition" => {
                    let name = child.child_by_field_name("name")
                        .and_then(|n| n.utf8_text(source).ok())
                        .unwrap_or("<anonymous>");
                    let mut new_prefix = prefix.to_vec();
                    new_prefix.push(name);
                    let body = child.child_by_field_name("body").unwrap_or(child);
                    self.visit(body, source, &new_prefix, units, glue);
                }
                "class_specifier" | "struct_specifier" if child.child_by_field_name("name").is_some() => {
                    let name = child.child_by_field_name("name")
                        .and_then(|n| n.utf8_text(source).ok())
                        .unwrap_or("<anonymous>");
                    // Emit the class itself as a unit.
                    let symbol = build_symbol(prefix, &[], name);  // e.g. "kebab::chunk::Foo"
                    units.push((symbol, child));
                    // Recurse for nested classes / methods.
                    let mut new_prefix = prefix.to_vec();
                    new_prefix.push(name);
                    let body = child.child_by_field_name("body").unwrap_or(child);
                    self.visit(body, source, &new_prefix, units, glue);
                }
                "function_definition" => {
                    // declarator may be qualified_identifier (out-of-class def) or plain identifier.
                    let symbol = extract_fn_symbol(child, source, prefix);
                    units.push((symbol, child));
                    // Do NOT recurse into function body — inner classes/lambdas left to a future revision.
                }
                "template_declaration" => {
                    // Recurse: unwrap to inner declarator (function_definition or class_specifier)
                    // and treat it as if it were directly there. Template params NOT in symbol.
                    self.visit(child, source, prefix, units, glue);
                }
                "enum_specifier" if child.child_by_field_name("name").is_some() => {
                    let name = child.child_by_field_name("name").and_then(|n| n.utf8_text(source).ok()).unwrap_or("<anonymous>");
                    let symbol = build_symbol(prefix, &[], name);
                    units.push((symbol, child));
                }
                "concept_definition" => {
                    let name = /* extract */;
                    let symbol = build_symbol(prefix, &[], &name);
                    units.push((symbol, child));
                }
                _ => glue.push(child),
            }
        }
    }
}

fn build_symbol(prefix: &[&str], extras: &[&str], leaf: &str) -> String {
    // Join with ::
    let mut parts: Vec<&str> = prefix.iter().copied().collect();
    parts.extend_from_slice(extras);
    parts.push(leaf);
    parts.join("::")
}

fn extract_fn_symbol(node: Node, source: &[u8], prefix: &[&str]) -> String {
    // function_definition.declarator may be a function_declarator wrapping a
    // qualified_identifier (out-of-class def like `void Foo::bar(){}`) or a
    // plain identifier (free fn or in-namespace fn).
    // Need to walk down to the leaf identifier and any qualifier chain.
    // For qualified_identifier "Foo::bar::baz", break into ["Foo", "bar"] qualifier + "baz" leaf.
    // ...
    todo!("walk declarator → qualified_identifier → assemble symbol with prefix")
}

// Extractor impl: parse, visit(root, ...), emit chunks-of-blocks per (symbol, node) pair + <top-level> glue + <module> fallback.
```

This is the most intricate extractor in p10-1D. **Action**: read `crates/kebab-parse-code/src/java.rs` for the recursion pattern, then `crates/kebab-parse-code/src/python.rs` for the class-nesting pattern, and combine. tree-sitter-cpp's node-types.json (or a quick `tree-sitter parse` against a sample file) confirms exact node-kind names.

- [ ] **Step 2**: Register in `crates/kebab-parse-code/src/lib.rs`:

```rust
pub mod cpp;
pub use cpp::{CppAstExtractor, CPP_PARSER_VERSION};
```

- [ ] **Step 3**: Build:

```bash
cargo build -p kebab-parse-code 2>&1 | tail -5
```

Expected: clean.

- [ ] **Step 4**: Commit:

```bash
git add crates/kebab-parse-code/src/cpp.rs crates/kebab-parse-code/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(p10-1d): C++ AST extractor (tree-sitter-cpp)

Symbol = namespace::Class::method via recursive visit. namespace_definition
pushes namespace name (anonymous → <anonymous>). class_specifier / struct_specifier
(named) emit class unit + recurse with class name pushed. function_definition
emits method unit (symbol may include qualified_identifier prefix for
out-of-class definitions). template_declaration unwraps to inner declarator
(template params NOT in symbol). enum_specifier + concept_definition emit
type-level units. extern "C" block content + using/include/define → glue.

Constructor / destructor symbols use Class::Class / Class::~Class
convention. Operator overloads keep operator+ form.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task D: C chunker + snapshot test

**Files:**
- Create: `crates/kebab-chunk/src/code_c_ast_v1.rs`
- Create: `crates/kebab-chunk/tests/fixtures/sample.c`
- Create: `crates/kebab-chunk/tests/code_c_ast_snapshot.rs`
- Modify: `crates/kebab-chunk/src/lib.rs`

- [ ] **Step 1**: Create `crates/kebab-chunk/src/code_c_ast_v1.rs`. **Mirror `crates/kebab-chunk/src/code_go_ast_v1.rs`** (closest 1-extractor pattern, no nesting):

```rust
//! p10-1D: C AST chunker.

use crate::tier2_shared::build_chunk;
use crate::{Chunker, ChunkPolicy};
use anyhow::Result;
use kebab_core::{Block, Chunk, Document};

pub const VERSION_LABEL: &str = "code-c-ast-v1";

pub struct CodeCAstV1Chunker;

impl Chunker for CodeCAstV1Chunker {
    fn chunker_version(&self) -> &'static str { VERSION_LABEL }
    fn policy_hash(&self, policy: &ChunkPolicy) -> String {
        crate::tier2_shared::policy_hash(policy)
    }
    fn chunk(&self, doc: &Document, policy: &ChunkPolicy) -> Result<Vec<Chunk>> {
        // Mirror code_go_ast_v1.rs's body — iterate doc.blocks, each Block::Code
        // contributes 1 chunk via build_chunk. Apply oversize fallback per block
        // via tier2_shared::push_chunks_with_oversize.
        // ...
        todo!("mirror code_go_ast_v1.rs verbatim, substituting VERSION_LABEL")
    }
}
```

Read `code_go_ast_v1.rs` and port verbatim — the language-agnostic body iterates `doc.blocks` and emits chunks. Only the `VERSION_LABEL` and (potentially) symbol formatting helper change.

- [ ] **Step 2**: Create `tests/fixtures/sample.c` (~30 lines, includes top-level fn, struct, enum, preprocessor):

```c
#include <stdio.h>
#include <stdlib.h>

#define MAX_BUF 4096

typedef enum {
    OK = 0,
    ERR_PARSE,
    ERR_IO,
} status_t;

typedef struct {
    int id;
    char name[64];
    status_t status;
} record_t;

static int counter = 0;

int parse_record(const char *line, record_t *out) {
    if (line == NULL || out == NULL) return ERR_PARSE;
    return OK;
}

void print_record(const record_t *r) {
    printf("[%d] %s (status=%d)\n", r->id, r->name, r->status);
}

int main(void) {
    record_t r = { .id = 1, .name = "foo", .status = OK };
    print_record(&r);
    return 0;
}
```

Expected snapshot: 3 function units (`parse_record`, `print_record`, `main`) + 1 enum unit (`status_t`) + 1 struct unit (`record_t`) + 1 `<top-level>` glue (preproc + global var). Total ~6 chunks.

- [ ] **Step 3**: Create `tests/code_c_ast_snapshot.rs` mirroring `tests/code_go_ast_snapshot.rs`. Assertions:

```rust
// Pseudocode:
// 1. Load fixture sample.c
// 2. Run CAstExtractor → Document
// 3. Run CodeCAstV1Chunker.chunk(&doc, &policy)
// 4. Assert chunks.len() == expected (6).
// 5. Assert symbols (from chunks[i].source_spans[0]::SourceSpan::Code.symbol) match expected list:
//    ["status_t", "record_t", "parse_record", "print_record", "main", "<top-level>"]
//    (order matches AST traversal order — verify by running once.)
// 6. Assert all chunks have lang = Some("c").
```

- [ ] **Step 4**: Register module in `crates/kebab-chunk/src/lib.rs`:

```rust
pub mod code_c_ast_v1;
pub use code_c_ast_v1::CodeCAstV1Chunker;
```

- [ ] **Step 5**: Run test:

```bash
cargo test -p kebab-chunk --test code_c_ast_snapshot -- --nocapture 2>&1 | tail -25
```

Expected: PASS. If chunk count or symbol order differs from expectation, INSPECT the actual output and update the test's expected list to match (run once to learn, codify on second run).

- [ ] **Step 6**: Clippy + commit:

```bash
cargo clippy -p kebab-chunk --all-targets -- -D warnings
git add crates/kebab-chunk/src/code_c_ast_v1.rs \
        crates/kebab-chunk/src/lib.rs \
        crates/kebab-chunk/tests/fixtures/sample.c \
        crates/kebab-chunk/tests/code_c_ast_snapshot.rs
git commit -m "$(cat <<'EOF'
feat(p10-1d): code-c-ast-v1 chunker + snapshot test

Mirrors code-go-ast-v1's chunker pattern (1 chunk per AST unit + <top-level>
glue + oversize fallback). Snapshot test against tests/fixtures/sample.c
(function + struct + enum + preprocessor) verifies symbol order + lang=c
stamping.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task E: C++ chunker + snapshot test

**Files:**
- Create: `crates/kebab-chunk/src/code_cpp_ast_v1.rs`
- Create: `crates/kebab-chunk/tests/fixtures/sample.cpp`
- Create: `crates/kebab-chunk/tests/code_cpp_ast_snapshot.rs`
- Modify: `crates/kebab-chunk/src/lib.rs`

- [ ] **Step 1**: Create `code_cpp_ast_v1.rs`. **Mirror `code_c_ast_v1.rs`** verbatim, only VERSION_LABEL differs:

```rust
pub const VERSION_LABEL: &str = "code-cpp-ast-v1";

pub struct CodeCppAstV1Chunker;

impl Chunker for CodeCppAstV1Chunker {
    fn chunker_version(&self) -> &'static str { VERSION_LABEL }
    // ... identical body — both languages use the same Block::Code → Chunk emission ...
}
```

The actual symbol-formatting work happens in the EXTRACTOR (Task C). The chunker's job is to iterate blocks the extractor produced and emit Chunks. Both C and C++ chunkers are essentially identical bodies.

- [ ] **Step 2**: Create `tests/fixtures/sample.cpp` (~50 lines, includes namespace + nested class + method + free fn + template):

```cpp
#include <string>
#include <vector>

namespace kebab {
namespace chunk {

class MdHeadingV1Chunker {
public:
    MdHeadingV1Chunker() = default;
    ~MdHeadingV1Chunker() = default;

    std::string chunk_doc(const std::string& doc) {
        return doc;
    }

    int operator()(int x) const {
        return x * 2;
    }

private:
    int counter_ = 0;
};

template <typename T>
T identity(T value) {
    return value;
}

}  // namespace chunk

void global_helper() {
    // free function in kebab namespace
}

}  // namespace kebab

int main() {
    kebab::chunk::MdHeadingV1Chunker c;
    return 0;
}
```

Expected snapshot symbols (verify on first run, then codify):
- `kebab::chunk::MdHeadingV1Chunker` (class unit)
- `kebab::chunk::MdHeadingV1Chunker::MdHeadingV1Chunker` (constructor)
- `kebab::chunk::MdHeadingV1Chunker::~MdHeadingV1Chunker` (destructor)
- `kebab::chunk::MdHeadingV1Chunker::chunk_doc`
- `kebab::chunk::MdHeadingV1Chunker::operator()`
- `kebab::chunk::identity` (template fn)
- `kebab::global_helper`
- `main` (free fn, no namespace)
- `<top-level>` (include + using)

~9 chunks total.

- [ ] **Step 3**: Create `tests/code_cpp_ast_snapshot.rs` mirroring `code_c_ast_snapshot.rs`. Assert symbol list matches expected (run once to learn the actual order, codify).

- [ ] **Step 4**: Register module in `lib.rs`:

```rust
pub mod code_cpp_ast_v1;
pub use code_cpp_ast_v1::CodeCppAstV1Chunker;
```

- [ ] **Step 5**: Run test:

```bash
cargo test -p kebab-chunk --test code_cpp_ast_snapshot -- --nocapture 2>&1 | tail -30
```

Expected: PASS.

- [ ] **Step 6**: Clippy + commit:

```bash
cargo clippy -p kebab-chunk --all-targets -- -D warnings
git add crates/kebab-chunk/src/code_cpp_ast_v1.rs \
        crates/kebab-chunk/src/lib.rs \
        crates/kebab-chunk/tests/fixtures/sample.cpp \
        crates/kebab-chunk/tests/code_cpp_ast_snapshot.rs
git commit -m "$(cat <<'EOF'
feat(p10-1d): code-cpp-ast-v1 chunker + snapshot test

Identical chunker body to code-c-ast-v1; per-language work happens in the
CppAstExtractor (Task C). Snapshot fixture covers nested namespace +
class + ctor/dtor + method + operator overload + template fn + free fn +
top-level main, verifying namespace::Class::method symbol convention per
design §3.4.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task F: ingest_one_code_asset dispatch + tier3 fallback list extension

**Files:**
- Modify: `crates/kebab-app/src/lib.rs`

- [ ] **Step 1**: Top-of-file `use kebab_chunk::{...}` extend with `CodeCAstV1Chunker` + `CodeCppAstV1Chunker`:

```rust
use kebab_chunk::{
    /* existing items */,
    CodeCAstV1Chunker,
    CodeCppAstV1Chunker,
};
```

- [ ] **Step 2**: Allowlist (around line 953) extend:

```rust
if matches!(lang.as_str(),
    "rust" | "python" | "typescript" | "javascript" | "go" | "java" | "kotlin"
    | "yaml" | "dockerfile" | "toml" | "json" | "xml" | "groovy" | "go-mod"
    | "shell"
    | "c" | "cpp")
```

- [ ] **Step 3**: `parser_version` match — add C/C++ arms (Tier 1, so they DO get a real parser version):

```rust
let parser_version = match code_lang {
    // ... existing 7 Tier 1 + Tier 2 + shell arms ...
    "c"   => ParserVersion(kebab_parse_code::C_PARSER_VERSION.to_string()),
    "cpp" => ParserVersion(kebab_parse_code::CPP_PARSER_VERSION.to_string()),
    other => anyhow::bail!("unsupported code_lang: {other}"),
};
```

- [ ] **Step 4**: `chunker_version` match — add C/C++ arms:

```rust
let chunker_version = match code_lang {
    // ... existing arms ...
    "c"   => CodeCAstV1Chunker.chunker_version(),
    "cpp" => CodeCppAstV1Chunker.chunker_version(),
    other => anyhow::bail!("unreachable chunker_version: {other}"),
};
```

- [ ] **Step 5**: `canonical_result` extract match — add C/C++ arms:

```rust
let canonical_result: anyhow::Result<CanonicalDocument> = match code_lang {
    "rust"   => RustAstExtractor::new().extract(&ctx, &bytes).context("..."),
    // ... existing ...
    "c"      => CAstExtractor::new().extract(&ctx, &bytes)
                  .context("kb-parse-code::CAstExtractor::extract (code:c)"),
    "cpp"    => CppAstExtractor::new().extract(&ctx, &bytes)
                  .context("kb-parse-code::CppAstExtractor::extract (code:cpp)"),
    // ... Tier 2 + shell ...
    other => anyhow::bail!("unreachable (extract): {other}"),
};
```

(Add `use kebab_parse_code::{CAstExtractor, CppAstExtractor};` at the top if not already wildcard-imported.)

- [ ] **Step 6**: `chunks_result` match — add C/C++ arms:

```rust
let chunks_result: anyhow::Result<Vec<Chunk>> = if extract_fell_back {
    // ... existing ...
} else {
    match code_lang {
        "rust"   => CodeRustAstV1Chunker.chunk(&canonical, chunk_policy).context("..."),
        // ... existing ...
        "c"      => CodeCAstV1Chunker.chunk(&canonical, chunk_policy)
                      .context("kb-chunk::CodeCAstV1Chunker::chunk (code:c)"),
        "cpp"    => CodeCppAstV1Chunker.chunk(&canonical, chunk_policy)
                      .context("kb-chunk::CodeCppAstV1Chunker::chunk (code:cpp)"),
        // ... existing ...
        other => anyhow::bail!("unreachable (chunk): {other}"),
    }
};
```

- [ ] **Step 7**: `tier3_fallback_cv` (p10-3 Critical fix) — C/C++ are fallback-eligible (extract may fail on `.h` C++ headers or malformed code):

```rust
let tier3_fallback_cv = match code_lang {
    "rust" | "python" | "typescript" | "javascript"
    | "go" | "java" | "kotlin"
    | "yaml" | "dockerfile" | "toml" | "json" | "xml" | "groovy" | "go-mod"
    | "c" | "cpp"   // p10-1d:
        => Some(CodeTextParagraphV1Chunker.chunker_version()),
    _ => None,
};
```

(The exact location of this match is in `ingest_one_code_asset` between ~lines 1921-1927 per the p10-3 critical fix.)

- [ ] **Step 8**: Build:

```bash
cargo build -p kebab-app 2>&1 | tail -5
```

Expected: clean.

- [ ] **Step 9**: Per-crate test (no regression):

```bash
cargo test -p kebab-app --lib -- --nocapture 2>&1 | tail -10
```

Expected: 52 PASS (existing baseline).

- [ ] **Step 10**: Clippy + commit:

```bash
cargo clippy -p kebab-app --all-targets -- -D warnings
git add crates/kebab-app/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(p10-1d): activate C + C++ in ingest_one_code_asset dispatch

Extends 4-arm match (parser_version / chunker_version / extract / chunks)
+ allowlist + tier3_fallback_cv list with "c" + "cpp" arms. C uses
CAstExtractor + CodeCAstV1Chunker; C++ uses CppAstExtractor +
CodeCppAstV1Chunker. Both langs are Tier 3-fallback-eligible (e.g. .h
file with C++ syntax may fail tree-sitter-c parse → Tier 3 paragraph
fallback).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task G: code_ingest_smoke integration tests (C + C++)

**Files:**
- Modify: `crates/kebab-app/tests/code_ingest_smoke.rs`

- [ ] **Step 1**: Append 2 tests at the end of the file (mirror the existing tier1 tests `c_ast_v1_*` if present; if not, mirror `rust_ast_v1_*` or `go_ast_v1_*`):

```rust
#[test]
fn tier1_c_ingest_searchable() {
    let env = TestEnv::lexical_only();
    let workspace = env.workspace_root();
    std::fs::write(
        workspace.join("parser.c"),
        "#include <stdio.h>\n\nint parse_record(const char *line) {\n    if (line == NULL) return -1;\n    return 0;\n}\n",
    )
    .unwrap();

    let report = env.ingest().expect("ingest");
    assert!(report.new_docs >= 1, "expected at least 1 new doc");

    let hits = env.search_code_lang("c", "parse_record").expect("search");
    assert!(!hits.is_empty(), "expected at least 1 c hit");

    match &hits[0].citation {
        Citation::Code { symbol, lang, .. } => {
            assert_eq!(symbol.as_deref(), Some("parse_record"), "C symbol must be function name only");
            assert_eq!(lang.as_deref(), Some("c"));
        }
        other => panic!("expected Citation::Code, got {other:?}"),
    }
    assert_eq!(
        hits[0].chunker_version.as_ref().map(|c| c.0.as_str()),
        Some("code-c-ast-v1"),
    );
}

#[test]
fn tier1_cpp_ingest_searchable() {
    let env = TestEnv::lexical_only();
    let workspace = env.workspace_root();
    std::fs::write(
        workspace.join("chunker.cpp"),
        "namespace kebab {\nnamespace chunk {\nclass Foo {\npublic:\n    void bar() { /* impl */ }\n};\n}\n}\n",
    )
    .unwrap();

    let report = env.ingest().expect("ingest");
    assert!(report.new_docs >= 1);

    let hits = env.search_code_lang("cpp", "bar").expect("search");
    assert!(!hits.is_empty(), "expected at least 1 cpp hit");

    match &hits[0].citation {
        Citation::Code { symbol, lang, .. } => {
            // Symbol could be "kebab::chunk::Foo::bar" or "kebab::chunk::Foo" depending on which chunk hits first.
            assert!(
                symbol.as_deref().map_or(false, |s| s.starts_with("kebab::chunk::Foo")),
                "C++ symbol must start with namespace::Class prefix, got {:?}", symbol
            );
            assert_eq!(lang.as_deref(), Some("cpp"));
        }
        other => panic!("expected Citation::Code, got {other:?}"),
    }
    assert_eq!(
        hits[0].chunker_version.as_ref().map(|c| c.0.as_str()),
        Some("code-cpp-ast-v1"),
    );
}
```

- [ ] **Step 2**: Run tests:

```bash
cargo test -p kebab-app --test code_ingest_smoke tier1_c_ingest tier1_cpp_ingest -- --nocapture 2>&1 | tail -30
```

Expected: 2 PASS.

- [ ] **Step 3**: Full smoke regression:

```bash
cargo test -p kebab-app --test code_ingest_smoke -- --nocapture 2>&1 | tail -30
```

Expected: 18 PASS (16 existing + 2 new).

- [ ] **Step 4**: Clippy + commit:

```bash
cargo clippy -p kebab-app --tests -- -D warnings
git add crates/kebab-app/tests/code_ingest_smoke.rs
git commit -m "$(cat <<'EOF'
test(p10-1d): integration smoke tests for C + C++

Verifies end-to-end ingest + search + Citation::Code shape:
- tier1_c_ingest_searchable: .c file → --code-lang c search → symbol
  = function name (no nesting), lang = "c", chunker_version = "code-c-ast-v1".
- tier1_cpp_ingest_searchable: .cpp file → --code-lang cpp search →
  symbol starts with namespace::Class prefix, lang = "cpp",
  chunker_version = "code-cpp-ast-v1".

Brings code_ingest_smoke to 18 tests (Rust 3 + Python 1 + TS 1 + JS 1 +
Go 1 + Java 1 + Kotlin 1 + yaml 1 + dockerfile 1 + manifest 1 + shell 1 +
yaml-fallback 1 + 2 reingest-unchanged regression + c 1 + cpp 1).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task H: frozen design §10 activation log

**Files:**
- Modify: `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md`

- [ ] **Step 1**: Find §10 activation log. Add p10-1D entry right after the p10-3 entry:

```
**p10-1D 활성화 (C + C++) (2026-05-21)**: Tier 1 chunker family 완료 — C (`code-c-ast-v1`, `.c`/`.h`) + C++ (`code-cpp-ast-v1`, `.cpp`/`.cc`/`.cxx`/`.hpp`/`.hh`/`.hxx`) AST chunker 활성화. C symbol = function name only; C++ symbol = `namespace::Class::method` (recursive namespace + class nesting). `.h` 가 C++ syntax 만나면 tree-sitter-c parse 실패 → p10-3 Tier 3 fallback 으로 자동 picked up.
```

- [ ] **Step 2**: Commit:

```bash
git add docs/superpowers/specs/2026-05-15-kebab-code-ingest-design.md \
        docs/superpowers/specs/2026-04-27-kebab-final-form-design.md 2>/dev/null
git add docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
git commit -m "$(cat <<'EOF'
docs(p10-1d): activate C + C++ in frozen design §10

P10 Tier 1 chunker family complete.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task I: README + HANDOFF + ARCHITECTURE + SMOKE + tasks/INDEX + tasks/p10/INDEX

**Files:**
- Modify: `README.md` (Mermaid + ingest row), `HANDOFF.md`, `docs/ARCHITECTURE.md`, `docs/SMOKE.md`, `tasks/INDEX.md`, `tasks/p10/INDEX.md`

- [ ] **Step 1 — README.md**: Update the `kebab ingest` row's supported-langs list to include `.c` / `.h` → `code-c-ast-v1` and `.cpp`/`.cc`/`.cxx`/`.hpp`/`.hh`/`.hxx` → `code-cpp-ast-v1`. Extend `--code-lang c` / `--code-lang cpp` in the enumeration. Update the Mermaid `chunker[...]` node to include `code-c-ast-v1, code-cpp-ast-v1` in the brace.

- [ ] **Step 2 — HANDOFF.md**: P10 row append `, **1D ✅ (C + C++ AST chunkers, code-c-ast-v1 + code-cpp-ast-v1 — v0.16.0)**`. Update 한 줄 요약 to include C/C++. Update 다음 후보 (drop p10-1D; remaining: P9-5 desktop / P8 audio).

- [ ] **Step 3 — docs/ARCHITECTURE.md**: code parser table row: append C + C++ row mention. Flowchart `pcode` node: append `+ P10-1D`. Directory tree chunkers list: add `code_c_ast_v1.rs` + `code_cpp_ast_v1.rs`.

- [ ] **Step 4 — docs/SMOKE.md**: Add a "## P10-1D C + C++ AST chunker" section after the P10-3 section. Walkthrough with sample.c + sample.cpp ingest + `--code-lang c` / `--code-lang cpp` search assertions. Append verification checklist entry.

- [ ] **Step 5 — tasks/INDEX.md + tasks/p10/INDEX.md**: Flip p10-1D row ⏳ → ✅ (v0.16.0).

- [ ] **Step 6**: Commit:

```bash
git add README.md HANDOFF.md docs/ARCHITECTURE.md docs/SMOKE.md tasks/INDEX.md tasks/p10/INDEX.md
git commit -m "$(cat <<'EOF'
docs(p10-1d): README/HANDOFF/ARCHITECTURE/SMOKE/INDEX sync

P10 Tier 1 chunker family complete (Rust + Python + TS + JS + Go + Java +
Kotlin + C + C++). Tier 2 (k8s + dockerfile + manifest) and Tier 3
(paragraph fallback) already active. p10-1D 활성화 + ✅ flip.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task J: workspace test gate + clippy

- [ ] **Step 1**: Disk check (`df -h /`) + optional `cargo clean`.

- [ ] **Step 2**: `cargo test --workspace --no-fail-fast -j 1 2>&1 | tail -80`. Expected: all PASS.

- [ ] **Step 3**: `cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -30`. Expected: clean.

---

## Task K: version bump + gitea PR + release

**Files:**
- Modify: `Cargo.toml`

- [ ] **Step 1**: Workspace `version = "0.15.0"` → `"0.16.0"`.

- [ ] **Step 2**: `cargo build -p kebab-cli` to refresh Cargo.lock.

- [ ] **Step 3**: Commit:

```bash
git add Cargo.toml Cargo.lock
git commit -m "$(cat <<'EOF'
chore: bump version 0.15.0 → 0.16.0 (p10-1d C + C++ AST chunkers)

Minor bump — additive new chunker_versions code-c-ast-v1 + code-cpp-ast-v1
+ new routing langs c / cpp + new tree-sitter-c / tree-sitter-cpp workspace
deps. P10 Tier 1 chunker family complete. No DB migration, no wire schema
major bump.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

- [ ] **Step 4**: Push branch + open gitea PR via REST API. Title: `feat(p10-1d): C + C++ AST chunkers — P10 Tier 1 chunker family complete`.

- [ ] **Step 5**: Wait for code-reviewer APPROVE → merge via gitea REST API → cut `gitea-release v0.16.0`.

---

## Verification matrix

| 검증 | 명령 | 기대 |
|------|------|------|
| C symbol | `kebab search --code-lang c --json` | `Citation::Code.symbol = "<fn_name>"` |
| C++ symbol | `kebab search --code-lang cpp --json` | `Citation::Code.symbol = "namespace::Class::method"` |
| .h fallback | `.h` with C++ syntax → ingest | Tier 3 fallback: `chunker_version = "code-text-paragraph-v1"`, lang = c |
| code_lang_breakdown | `kebab schema --json` | `"c": N`, `"cpp": M` |

---

## Risks reminder (구현 중 주의)

- **tree-sitter grammar version resolution**: tree-sitter 0.26 호환 grammar. crates.io 최신 버전 default.
- **tree-sitter-cpp 의 node-kind 명**: spec 의 가정 (`namespace_definition`, `class_specifier`, `function_definition`, `template_declaration`, `concept_definition`, etc.) 이 실제 grammar 와 일치하는지 fixture parse 로 검증.
- **out-of-class method def 의 prefix 복원**: `void Foo::bar()` 의 declarator 가 `function_declarator > qualified_identifier > namespace_identifier "Foo" + identifier "bar"`. spec 의 `extract_fn_symbol` 이 이 chain 정확히 walk.
- **Operator overload**: tree-sitter-cpp 의 `operator_name` 또는 `field_identifier` "operator+" 형태. fixture 로 검증.
- **머지 후 deviation** 은 `tasks/HOTFIXES.md` dated 로그.
