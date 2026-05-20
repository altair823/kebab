# p10-1C-JavaKotlin — Java + Kotlin AST chunkers

**Status:** 🟡 진행 중
**Contract sections:** §3.3 (chunker_version `code-java-ast-v1` + `code-kotlin-ast-v1`), §3.4 (symbol path — Java/Kotlin `package.Class.method`), §3.5 (code_lang `java` + `kotlin`, ext `.java` / `.kt` / `.kts`), §6.1 (`kebab-parse-code/src/{java,kotlin}.rs`), §6.2 (`kebab-chunk/src/code_{java,kotlin}_ast_v1.rs`), §9.1 (Tier 1 AST per-language + oversize fallback).
**Design:** [2026-05-15-kebab-code-ingest-design.md](../../docs/superpowers/specs/2026-05-15-kebab-code-ingest-design.md) §1C (Java + Kotlin 부분 — Go 는 PR #151 / v0.12.0 별 PR 완료).
**Plan:** [2026-05-20-p10-1c-jk-ast-chunker.md](../../docs/superpowers/plans/2026-05-20-p10-1c-jk-ast-chunker.md).

## Goal

1C-Go (PR #151 / v0.12.0) 의 자매 PR. 같은 1C phase 의 JVM family (Java + Kotlin) 묶음. 머지 시점부터 `.java` / `.kt` / `.kts` 파일 dogfooding 가능.

## 동결된 설계 결정 (이 task 로 확정)

- **Symbol prefix = 소스 코드의 `package` 선언에서 추출** (design §3.4 그대로, 1C-Go 모델과 동일). 1B 의 workspace-path 변환과 다름.
  - **Java**: tree-sitter-java 의 `package_declaration` → 안의 `scoped_identifier` 또는 `identifier` 텍스트 (e.g. `com.kebab.chunk`). 없으면 `<unknown>`.
  - **Kotlin**: tree-sitter-kotlin 의 `package_header` → `identifier` 텍스트. 없으면 (default package) `<unknown>`.
- **Symbol 형식** (design §3.4): `package.Class.method`. 예시: `com.kebab.chunk.MdHeadingV1Chunker.chunkDoc`.
- **Java AST mapping**:
  - `class_declaration` (name) → 1 unit + recurse body
  - `interface_declaration` (name) → 1 unit + recurse
  - `enum_declaration` (name) → 1 unit
  - `record_declaration` (Java 14+) (name) → 1 unit
  - `annotation_type_declaration` → 1 unit
  - Inside class/interface/enum: `method_declaration` (name) → unit `package.Class.method` (class nesting like 1B Python)
  - `import_declaration`, `package_declaration` 자체 → glue `<top-level>` 
  - Top-level fn 없음 (Java 자체에 없음)
- **Kotlin AST mapping**:
  - `class_declaration` (name) → 1 unit + recurse class_body. `data class` / `sealed class` / `enum class` 도 같은 노드.
  - `object_declaration` (name) → 1 unit + recurse class_body (singleton)
  - `function_declaration` (name) — **top-level 가능** → unit `package.fnName`. Class 내부면 `package.Class.method`.
  - `property_declaration` at top-level → glue
  - `interface` (in tree-sitter-kotlin 보통 `class_declaration` with `interface` modifier 또는 별 노드) → 1 unit
  - `import_header`, `package_header` 자체 → glue `<top-level>`
- **Glue grouping**: 1B Python / 1C-Go 패턴 동일 — imports + 기타 → 하나의 `<top-level>` (또는 `<module>` post-pass if file has zero real units).
- **Tree-sitter Kotlin crate 선택**: tree-sitter-kotlin 의 가장 잘 유지되는 crate 사용 (`tree-sitter-kotlin` 또는 fork). resolve 시 active maintainer 확인.
- frozen design 자체 변경 없음 — §10.1 에 1C-JK 활성화 한 줄.

## Acceptance criteria

- `cargo test --workspace --no-fail-fast -j 1` passes.
- `cargo clippy --workspace --all-targets -- -D warnings` passes.
- Java/Kotlin fixture 각각 (`tests/fixtures/sample.java`, `tests/fixtures/sample.kt`) ingest → chunk snapshot 안정 + symbol 이 §3.4 컨벤션 일치.
- 격리 TempDir KB 에 `.java` / `.kt` 파일 두고 `kebab search --code-lang java --json` / `--code-lang kotlin --json` 가 `Citation::Code` 반환.
- `kebab schema --json | jq .stats.code_lang_breakdown` 에 `"java"` + `"kotlin"` 카운트.
- README + HANDOFF + ARCHITECTURE + SMOKE + tasks/INDEX + tasks/p10/INDEX 갱신.
- frozen design §10.1 한 줄.
- workspace `Cargo.toml` minor bump (0.12.0 → 0.13.0).

## Allowed dependencies

- `kebab-parse-code` 에 `tree-sitter-java` + `tree-sitter-kotlin` 추가. 기존 deps 유지.
- `kebab-chunk` 의 새 모듈 2개 (`code_java_ast_v1.rs`, `code_kotlin_ast_v1.rs`) — language-agnostic body. tree-sitter import 금지.
- `kebab-app`, `kebab-source-fs` — 새 crate dep 없음.

## Forbidden dependencies

- `kebab-chunk` 가 tree-sitter-java / tree-sitter-kotlin import 금지 (boundary §6.3).
- UI crate 가 `kebab-parse-code` 직접 import 금지.
- `kebab-parse-code` 가 store / embed / llm / rag 직접 import 금지.

## Risks / notes

- tree-sitter-kotlin: 공식 또는 가장 활발히 유지되는 crate (`tree-sitter-kotlin` 또는 fork) 선택 필요. resolve 시 metadata 확인.
- Kotlin 의 grammar 가 다른 tree-sitter-* 보다 update 빈도 낮을 수 있어 grammar field 명 변동 가능 — 테스트 fixture 로 contract 고정.
- Java record (Java 14+) — tree-sitter-java 에서 `record_declaration` 노드 (확인 필요).
- Kotlin sealed class / data class / object declaration 등 변종 노드 — tree-sitter-kotlin 의 정확한 node kind 명 확인 필요 (grammar.json / node-types.json).
- Java class 안의 inner class — Python 패턴 (recursion with class name pushed) 동일 처리.
- Kotlin top-level fn 은 1B Python 의 top-level fn 패턴 + 1C-Go 의 package-prefix 패턴 hybrid — `package.fnName`.
- 머지 후 deviation 은 `tasks/HOTFIXES.md` dated 로그 + 본 spec `Risks / notes` cross-link.
