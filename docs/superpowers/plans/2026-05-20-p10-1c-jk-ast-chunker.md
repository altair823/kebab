# p10-1C-JavaKotlin Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task.

**Goal:** Activate Java + Kotlin code ingest end-to-end. Mirror 1C-Go (PR #151 / v0.12.0) for Java (single-language scaffold) and Kotlin (additional top-level fn variant). Both use source-side `package` extraction (design §3.4 JVM convention).

**Architecture:** Same shape as 1B (multi-language single PR). 2 new tree-sitter grammars + 2 extractors + 2 chunkers + media routing + app dispatch arms. 1C-Go pattern is the closest template for source-side `package` extraction.

**Tech Stack:** Rust 2024 workspace, `tree-sitter` 0.26 (already), `tree-sitter-java` + `tree-sitter-kotlin` (NEW). 1A-2/1B/1C-Go infrastructure unchanged.

**Memory note:** Host has been OOM'd previously. Per-crate cargo only. ONE full-suite + clippy invocation in Task J.

---

## Pre-flight

Branch `feat/p10-1c-jk` already exists.

- [ ] **Disk hygiene**: `cargo clean` if heavy (last cleanup recovered 34 GB).

Reference files:
- 1C-Go extractor: `crates/kebab-parse-code/src/go.rs` — closest template for source-side package extraction.
- 1B Python extractor: `crates/kebab-parse-code/src/python.rs` — class-nesting recursion model (relevant for Java/Kotlin).
- 1A-2 chunker: `crates/kebab-chunk/src/code_rust_ast_v1.rs` — duplicate-with-substitution.
- 1B dispatch generalization: `crates/kebab-app/src/lib.rs::ingest_one_code_asset` 4-arm match (~L1645). 1C-Go already added `"go"`; this PR adds `"java"` + `"kotlin"`.

---

## Task A: Workspace deps (tree-sitter-java + tree-sitter-kotlin)

**Files:**
- Modify: `Cargo.toml` (workspace `[workspace.dependencies]`, after `tree-sitter-go` line)
- Modify: `crates/kebab-parse-code/Cargo.toml`

- [ ] **Step 1**: `cargo add tree-sitter-java tree-sitter-kotlin -p kebab-parse-code`. If `tree-sitter-kotlin` resolves to a fork name, verify the actively-maintained crate (e.g. check crates.io page / GitHub stars / last update). Likely `tree-sitter-kotlin` (without fork suffix) is the default.

- [ ] **Step 2**: Lift the two resolved versions into `[workspace.dependencies]` after `tree-sitter-go`:

```toml
# JVM family grammars for code ingest (kebab-parse-code, p10-1C-JK).
tree-sitter-java       = "<resolved>"
tree-sitter-kotlin     = "<resolved>"
```

Switch crate's entries to `{ workspace = true }`.

- [ ] **Step 3**: `cargo build -p kebab-parse-code` → clean. Unused dep warning is fine.

- [ ] **Step 4**: Commit:

```bash
git add Cargo.toml Cargo.lock crates/kebab-parse-code/Cargo.toml
git commit -m "build(p10-1c-jk): add tree-sitter-java + tree-sitter-kotlin workspace deps

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

If the kotlin crate has a different actual name (e.g. `tree-sitter-kotlin-ng` or fork suffix), document the choice in the commit body briefly.

---

## Task B: source-fs routing `.java` / `.kt` / `.kts`

**Files:**
- Modify: `crates/kebab-source-fs/src/media.rs` (add arm after the existing `.go` arm)
- Test: same file's test module

- [ ] **Step 1 (failing test)** — add near `go_files_map_to_media_code_go`:

```rust
#[test]
fn java_kotlin_files_map_to_media_code() {
    assert_eq!(media_type_for(Path::new("a/b.java")), MediaType::Code("java".into()));
    assert_eq!(media_type_for(Path::new("a/b.kt")), MediaType::Code("kotlin".into()));
    assert_eq!(media_type_for(Path::new("a/b.kts")), MediaType::Code("kotlin".into()));
}
```

- [ ] **Step 2**: Run → FAIL.

- [ ] **Step 3**: Add the arms before the `_ => MediaType::Other(ext)` fallback (after `"go" => ...`):

```rust
        // p10-1C-JK: JVM family (Java + Kotlin) ingest activated.
        "java"             => MediaType::Code("java".into()),
        "kt" | "kts"       => MediaType::Code("kotlin".into()),
```

- [ ] **Step 4**: Run → PASS. `cargo test -p kebab-source-fs` → no regression.

- [ ] **Step 5**: clippy clean, commit.

```bash
cargo clippy -p kebab-source-fs --all-targets -- -D warnings
git add crates/kebab-source-fs/
git commit -m "feat(p10-1c-jk): route .java/.kt/.kts to MediaType::Code

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task C: App dispatch + bail arms for "java" + "kotlin"

**Files:**
- Modify: `crates/kebab-app/src/lib.rs`

- [ ] **Step 1**: Find the dispatch arm guard (currently `matches!(lang.as_str(), "rust" | "python" | "typescript" | "javascript" | "go")`). Add `"java"` + `"kotlin"`:

```rust
MediaType::Code(lang)
    if matches!(lang.as_str(),
        "rust" | "python" | "typescript" | "javascript" | "go" | "java" | "kotlin") =>
```

- [ ] **Step 2**: In `ingest_one_code_asset` the 4 `match code_lang` blocks add `"java"` and `"kotlin"` arms that `bail!()` for now:

```rust
"java" => anyhow::bail!("java ingest not yet wired (p10-1c-jk Task F)"),
"kotlin" => anyhow::bail!("kotlin ingest not yet wired (p10-1c-jk Task I)"),
```

(in each of the 4 blocks before the `other =>` catch-all).

- [ ] **Step 3**: Verify per-crate:
- `cargo test -p kebab-app --lib` → 52 stay green
- `cargo test -p kebab-app --test code_ingest_smoke` → 7 stay green
- `cargo clippy -p kebab-app --all-targets -- -D warnings` clean

- [ ] **Step 4**: Commit:

```bash
git add crates/kebab-app/
git commit -m "refactor(p10-1c-jk): add java + kotlin to ingest dispatch allowlist (bail until Tasks F/I)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task D: `JavaAstExtractor`

**Files:**
- Create: `crates/kebab-parse-code/src/java.rs`
- Modify: `crates/kebab-parse-code/src/lib.rs` (`pub mod java;` + re-exports `JAVA_PARSER_VERSION`, `JavaAstExtractor`)
- Create: `crates/kebab-parse-code/tests/fixtures/sample.java`

Scaffold mirrors `crates/kebab-parse-code/src/go.rs` (1C-Go) — single-language with source-side `package` extraction. Differences:

### Constants

```rust
pub const PARSER_VERSION: &str = "code-java-v1";
pub struct JavaAstExtractor;
// supports: matches!(m, MediaType::Code(l) if l == "java")
// code_lang = Some("java"), SourceType::Note, repo via detect_repo
```

### Package extraction (Java)

tree-sitter-java grammar:
- Root: `program`
- `package_declaration` (top-level child) → contains `scoped_identifier` (dotted) OR `identifier` (single-segment)

```rust
fn extract_package(root: tree_sitter::Node, src: &str) -> Option<String> {
    let mut cur = root.walk();
    for child in root.named_children(&mut cur) {
        if child.kind() == "package_declaration" {
            // package_declaration has scoped_identifier OR identifier as first named child
            let mut c2 = child.walk();
            for sub in child.named_children(&mut c2) {
                if sub.kind() == "scoped_identifier" || sub.kind() == "identifier" {
                    return Some(src[sub.start_byte()..sub.end_byte()].to_string());
                }
            }
        }
    }
    None
}
```

(Verify field names against tree-sitter-java's node-types.json if any field differs.)

### AST mapping

| node kind | unit | symbol |
|-----------|------|--------|
| `class_declaration` (name field) | 1 + recurse body | `<pkg>.<ClassName>` |
| `interface_declaration` (name) | 1 + recurse body | `<pkg>.<InterfaceName>` |
| `enum_declaration` (name) | 1 | `<pkg>.<EnumName>` |
| `record_declaration` (name, Java 14+) | 1 | `<pkg>.<RecordName>` |
| `annotation_type_declaration` (name) | 1 | `<pkg>.<AnnotationName>` |
| Inside class body: `method_declaration` (name) | 1 | `<pkg>.<Class>.<method>` |
| Inside class body: `constructor_declaration` (name = class name) | 1 | `<pkg>.<Class>.<ClassName>` (matches Java convention) |
| Nested classes recurse with class name pushed onto mod_path | as above | `<pkg>.<Outer>.<Inner>` etc. |
| `import_declaration`, `package_declaration` | glue | `<pkg>.<top-level>` |
| `field_declaration` at top of class | NOT a unit in 1C-JK (would explode unit count for value-only fields) | n/a |

`unit_start` walks `comment` siblings; Java has `@interface` annotations but those are part of `annotation_type_declaration` itself, not separate sibling nodes.

`mod_path` = class nesting (like 1B Python). Empty at file top level.

### Fixture `tests/fixtures/sample.java`:

```java
// sample.java
package com.kebab.chunk;

import java.util.List;
import java.util.stream.Collectors;

/**
 * Heading-aware Markdown chunker.
 */
public class MdHeadingV1Chunker {
    private final String name;

    public MdHeadingV1Chunker(String name) {
        this.name = name;
    }

    public List<String> chunkDoc(String input) {
        return List.of(name, input);
    }

    public String getName() {
        return name;
    }

    public static class Builder {
        private String name;
        public Builder withName(String n) { this.name = n; return this; }
        public MdHeadingV1Chunker build() { return new MdHeadingV1Chunker(name); }
    }
}

interface Stringer {
    String asString();
}

enum Mode { DEFAULT, FAST }
```

### Test module (inline `#[cfg(test)] mod tests`)

Mirror 1C-Go shape:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use kebab_core::{Block, MediaType, SourceSpan};

    fn extract_fixture() -> kebab_core::CanonicalDocument {
        let bytes = std::fs::read(
            concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/sample.java"),
        ).unwrap();
        let asset = crate::rust::tests_support::fixed_code_asset(
            "crates/x/src/sample.java", "java",
        );
        let cfg = kebab_core::ExtractConfig::default();
        let root = std::path::PathBuf::from("/tmp");
        let ctx = kebab_core::ExtractContext { asset: &asset, workspace_root: &root, config: &cfg };
        JavaAstExtractor::new().extract(&ctx, &bytes).unwrap()
    }

    #[test]
    fn extractor_supports_only_media_code_java() { /* ... */ }

    #[test]
    fn java_units_match_design_3_4_symbols() {
        let doc = extract_fixture();
        let mut syms: Vec<String> = doc.blocks.iter().filter_map(|b| match b {
            Block::Code(c) => match &c.common.source_span {
                SourceSpan::Code { symbol, lang, .. } => {
                    assert_eq!(lang.as_deref(), Some("java"));
                    symbol.clone()
                }
                _ => None,
            },
            _ => None,
        }).collect();
        syms.sort();
        // workspace path → package extracted from source = com.kebab.chunk
        assert!(syms.iter().any(|s| s == "com.kebab.chunk.MdHeadingV1Chunker"), "got {syms:?}");
        assert!(syms.iter().any(|s| s == "com.kebab.chunk.MdHeadingV1Chunker.MdHeadingV1Chunker"));  // constructor
        assert!(syms.iter().any(|s| s == "com.kebab.chunk.MdHeadingV1Chunker.chunkDoc"));
        assert!(syms.iter().any(|s| s == "com.kebab.chunk.MdHeadingV1Chunker.getName"));
        assert!(syms.iter().any(|s| s == "com.kebab.chunk.MdHeadingV1Chunker.Builder"));
        assert!(syms.iter().any(|s| s == "com.kebab.chunk.MdHeadingV1Chunker.Builder.withName"));
        assert!(syms.iter().any(|s| s == "com.kebab.chunk.MdHeadingV1Chunker.Builder.build"));
        assert!(syms.iter().any(|s| s == "com.kebab.chunk.Stringer"));
        assert!(syms.iter().any(|s| s == "com.kebab.chunk.Mode"));
        assert!(syms.iter().any(|s| s == "com.kebab.chunk.<top-level>"));
    }

    #[test]
    fn deterministic_across_runs() {
        let a = extract_fixture();
        for _ in 0..50 { assert_eq!(extract_fixture().blocks, a.blocks); }
    }
}
```

### Wire into lib.rs

```rust
pub mod java;
pub use java::{PARSER_VERSION as JAVA_PARSER_VERSION, JavaAstExtractor};
```

### Verify + commit

- `cargo test -p kebab-parse-code` → all pass
- `cargo clippy -p kebab-parse-code --all-targets -- -D warnings` clean
- commit `feat(p10-1c-jk): tree-sitter-java AST extractor (JavaAstExtractor)`

---

## Task E: `code-java-ast-v1` chunker

Identical pattern to 1C-Go Task E. Duplicate `code_rust_ast_v1.rs` with substitutions:
- `VERSION_LABEL = "code-java-ast-v1"`, struct `CodeJavaAstV1Chunker`
- error message + module doc-comment prose
- Test module: parser_version `"code-java-v1"`, code_lang `"java"`
- Keep cross-chunker `policy_hash_matches_md_heading_v1`

Wire into `crates/kebab-chunk/src/lib.rs` (alphabetical). Verify + commit.

---

## Task F: Activate Java in app dispatch

Replace the `"java"` `bail!()` arms in `ingest_one_code_asset` with real calls (`JavaAstExtractor` + `CodeJavaAstV1Chunker`). Add integration test `java_file_ingests_and_searches_as_code_citation` (mirror 1C-Go test, fixture `pkg_dir/Foo.java` with `package com.foo;` and `public class Foo { public String bar() { ... } }`, assert symbol `com.foo.Foo.bar`).

Verify + commit.

---

## Task G: `KotlinAstExtractor`

**Files:**
- Create: `crates/kebab-parse-code/src/kotlin.rs`
- Modify: `crates/kebab-parse-code/src/lib.rs`
- Create: `crates/kebab-parse-code/tests/fixtures/sample.kt`

Constants: `PARSER_VERSION = "code-kotlin-v1"`, `KotlinAstExtractor`, `code_lang = "kotlin"`.

### Package extraction (Kotlin)

tree-sitter-kotlin grammar:
- Root: `source_file`
- `package_header` (top-level) → contains `identifier` (dotted is single `identifier` node text; verify against node-types.json)

```rust
fn extract_package(root: tree_sitter::Node, src: &str) -> Option<String> {
    let mut cur = root.walk();
    for child in root.named_children(&mut cur) {
        if child.kind() == "package_header" {
            let mut c2 = child.walk();
            for sub in child.named_children(&mut c2) {
                if sub.kind() == "identifier" {
                    return Some(src[sub.start_byte()..sub.end_byte()].to_string());
                }
            }
        }
    }
    None
}
```

(Verify against tree-sitter-kotlin's node-types.json — Kotlin grammar varies more than Java's.)

### AST mapping (Kotlin)

| node kind | unit | symbol |
|-----------|------|--------|
| `class_declaration` (name field) — covers `class`, `data class`, `sealed class`, `enum class`, `interface` (Kotlin's interface is a class_declaration variant) | 1 + recurse body | `<pkg>.<ClassName>` |
| `object_declaration` (name) — singleton | 1 + recurse | `<pkg>.<ObjectName>` |
| `function_declaration` (name) | 1 | `<pkg>.<fn_name>` (top-level) or `<pkg>.<Class>.<method>` (inside class) |
| Inside class body: `function_declaration` → method | 1 | `<pkg>.<Class>.<method>` |
| `property_declaration` at top-level (`val` / `var`) | glue | `<top-level>` (Kotlin top-level properties are common — keep as glue not unit) |
| `import_header`, `package_header` | glue | `<top-level>` |

(Detect class-vs-interface via modifier; for 1C 1차 treat both as `class_declaration` arm — symbol differs only via name. If tree-sitter-kotlin exposes `interface` keyword via modifier list, mention in HOTFIXES if special handling needed.)

### Fixture `sample.kt`:

```kotlin
// sample.kt
package com.kebab.chunk

import java.util.List

/**
 * Heading-aware Markdown chunker.
 */
class MdHeadingV1Chunker(val name: String) {
    fun chunkDoc(input: String): List<String> = listOf(name, input)

    fun getName(): String = name

    companion object {
        fun withName(n: String): MdHeadingV1Chunker = MdHeadingV1Chunker(n)
    }
}

interface Stringer {
    fun asString(): String
}

enum class Mode { DEFAULT, FAST }

fun freeFunction(x: Int): Int = x + 1

object Singleton {
    fun ping(): String = "pong"
}
```

### Test module — assert symbols

```rust
// Asserted symbols:
"com.kebab.chunk.MdHeadingV1Chunker"
"com.kebab.chunk.MdHeadingV1Chunker.chunkDoc"
"com.kebab.chunk.MdHeadingV1Chunker.getName"
"com.kebab.chunk.MdHeadingV1Chunker.Companion"  // companion object (verify name)
"com.kebab.chunk.MdHeadingV1Chunker.Companion.withName"  // method on companion
"com.kebab.chunk.Stringer"
"com.kebab.chunk.Mode"
"com.kebab.chunk.freeFunction"  // top-level fn (Kotlin-specific!)
"com.kebab.chunk.Singleton"
"com.kebab.chunk.Singleton.ping"
"com.kebab.chunk.<top-level>"  // import + property glue
```

(Companion object: tree-sitter-kotlin may use `companion_object` or `object_declaration` with `companion` modifier — verify and adjust the symbol if `Companion` isn't the right name.)

### Wire into lib.rs

```rust
pub mod kotlin;
pub use kotlin::{PARSER_VERSION as KOTLIN_PARSER_VERSION, KotlinAstExtractor};
```

Verify + commit.

---

## Task H: `code-kotlin-ast-v1` chunker

Same pattern as Task E. Substitute kotlin labels. Verify + commit.

---

## Task I: Activate Kotlin in app dispatch

Replace `"kotlin"` bail arms with real calls. Add integration test `kotlin_file_ingests_and_searches_as_code_citation`. Verify + commit.

---

## Task J: Snapshots + full-suite + SMOKE

- Create 2 snapshot tests (`code_java_ast_snapshot.rs`, `code_kotlin_ast_snapshot.rs`) + baselines. Mirror 1C-Go Task G snapshot test.
- ONE workspace test + clippy invocation.
- Manual SMOKE: write a `.java` and `.kt` file in TempDir, ingest, search.

Verify + commit (snapshot only).

---

## Task K: Docs + version bump

- README + HANDOFF + ARCHITECTURE + SMOKE + 2 INDEX updates + design §10.1.
- `Cargo.toml` version `0.12.0 → 0.13.0` (minor, surface 확장).

Commit `docs(p10-1c-jk): ... + chore: bump 0.12.0 → 0.13.0`.

---

## Finalize

`gitea-pr` → review loop → merge → main pull → branch cleanup → `cargo clean` → `gitea-release v0.13.0`.

---

## Self-Review (filled by plan author)

- **Spec coverage**: design §1C Java + Kotlin → Tasks D-I; §3.4 symbol path → extractor (Java D, Kotlin G); §6.1/§6.2 module structure → Tasks D/E/G/H; §6.3 dep graph → Task A; §9.1 Tier-1 + oversize fallback → chunkers E/H.
- **No placeholders**: novel logic (Java `extract_package`, Kotlin `extract_package`, AST walk arm tables) given concretely. Chunkers (E, H) are explicit "duplicate code_rust_ast_v1.rs with substitution X/Y/Z".
- **Type consistency**: `JavaAstExtractor` / `JAVA_PARSER_VERSION` / `CodeJavaAstV1Chunker` + `KotlinAstExtractor` / `KOTLIN_PARSER_VERSION` / `CodeKotlinAstV1Chunker` used consistently. `MediaType::Code("java")` / `("kotlin")` in routing + dispatch.
- **Kotlin grammar risk**: noted — tree-sitter-kotlin's exact node kinds (`class_declaration` vs `object_declaration`, `companion_object` vs companion modifier, `package_header` vs `package_directive`) should be verified against the resolved crate's node-types.json. Pin contract via test fixture; HOTFIXES any deviation found during implementation.
