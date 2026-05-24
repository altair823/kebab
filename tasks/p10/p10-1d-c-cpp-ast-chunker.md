# p10-1D — C + C++ AST chunkers

**Status:** 🟡 진행 중
**Contract sections:** §3.3 (chunker_version `code-c-ast-v1` + `code-cpp-ast-v1`), §3.4 (symbol path — C `func_name`, C++ `namespace::Class::method`), §3.5 (code_lang `c` + `cpp`, ext `.c`/`.h` / `.cpp`/`.cc`/`.cxx`/`.hpp`/`.hh`/`.hxx`), §6.1 (`kebab-parse-code/src/{c,cpp}.rs`), §6.2 (`kebab-chunk/src/code_{c,cpp}_ast_v1.rs`), §9.1 (Tier 1 AST per-language + oversize fallback), §10 (activation log).
**Design:** [2026-05-15-kebab-code-ingest-design.md](../../docs/superpowers/specs/2026-05-15-kebab-code-ingest-design.md) §1D (C + C++ 부분).
**Plan:** [2026-05-21-p10-1d-c-cpp-ast-chunker.md](../../docs/superpowers/plans/2026-05-21-p10-1d-c-cpp-ast-chunker.md).

## Goal

p10-1A-2 / 1B / 1C / p10-2 / p10-3 인프라 위에 C + C++ AST chunker 2종을 단일 PR 로 활성화. P10 의 Tier 1 chunker family 마지막. 머지 시점부터 `.c` / `.h` / `.cpp` / `.cc` / `.cxx` / `.hpp` / `.hh` / `.hxx` 파일 dogfooding 가능.

`.h` 가 design 명시대로 C 매핑 — C++ 프로젝트의 `.h` 는 tree-sitter-c 의 parse 가 namespace / template 같은 C++ syntax 에 실패할 가능성. 실패 시 p10-3 의 Tier 3 fallback 으로 자동 picked up (이미 wired).

## 동결된 설계 결정 (이 task 로 확정)

### C extractor (`code-c-ast-v1`)

- **Symbol** = function name only. design §3.4 그대로 — no nesting, no namespace. 예: `parse_blocks`.
- **Top-level units**:
  - `function_definition` (named) → 1 unit, symbol = function name
  - `struct_specifier` (named, top-level) → 1 unit, symbol = struct name
  - `enum_specifier` (named, top-level) → 1 unit, symbol = enum name
  - `union_specifier` (named, top-level) → 1 unit, symbol = union name
  - `declaration` (top-level — typedef / global var / fn prototype) → glue `<top-level>`
  - `preproc_include` / `preproc_def` / `preproc_function_def` / `preproc_ifdef` 등 preprocessor → glue `<top-level>`
- **Static / extern / inline fn**: 일반 fn 과 동일 처리 (storage class qualifier 무시 — symbol 은 declarator 의 fn name 만).
- **Inner struct / enum 안의 nested declaration** (C 도 가능): 1B Python class-nesting 미적용 — C 의 inner type 은 흔치 않고 outer 가 typedef wrapper 인 패턴이라 top-level 만 emit.
- **Empty file 또는 unit 0개** → `<module>` post-pass (1A-2 패턴).

### C++ extractor (`code-cpp-ast-v1`)

- **Symbol** = `namespace::Class::method` (design §3.4 그대로). namespace 가 없으면 `Class::method` 또는 `func_name`. 예: `kebab::chunk::MdHeadingV1Chunker::chunk_doc`.
- **Top-level units + recursion**:
  - `namespace_definition` (named) → recurse with namespace name pushed (Python class-nesting + Java/Kotlin package-prefix hybrid).
  - **Anonymous namespace** (`namespace { ... }`) → namespace name = `<anonymous>` push (Python `<unnamed>` 패턴 일관).
  - `class_specifier` / `struct_specifier` (top-level or in namespace or nested in class, named) → recurse with class name pushed.
  - `function_definition` (top-level or in namespace or in class) → 1 unit, symbol per nesting (`namespace::Class::method` / `namespace::func` / `Class::method` / `func_name`).
  - `template_declaration` → 내부 declarator type 따라 recurse / emit (function template → method emit, class template → class recurse). template type params (`<T>`, `<typename T>`) 는 symbol 미포함 (Go generic 처리와 동일).
  - `enum_specifier` (named) → 1 unit, symbol per nesting.
  - `concept_definition` (C++20) → 1 unit, symbol per nesting (treat as type-level definition).
  - `using_declaration` / `using_directive` / `preproc_include` / `preproc_def` 등 → glue `<top-level>`.
  - `extern "C"` 블록 안의 정의 → 일반 fn 처리 (block 자체는 glue).
- **Method out-of-class definition** (`Class::method` 형태로 namespace 밖에서 정의): tree-sitter-cpp 의 `function_declarator` 의 `qualified_identifier` 따라 prefix 복원 — declarator 의 `Class::method` 자체에서 추출.
- **Operator overload** (`operator+`, `operator()` 등): symbol = `Class::operator+` 그대로.
- **Constructor / destructor**: symbol = `Class::Class` / `Class::~Class` (convention).
- **Empty file 또는 unit 0개** → `<module>` post-pass.

### 공통

- **`<top-level>` glue grouping**: preprocessor + global var + using 선언 등 의미 단위 외 → 1 glue chunk per file.
- **Oversize fallback**: 1A-2 의 `AST_CHUNK_MAX_LINES = 200` 동일.
- **`.h` 의 fallback 보장**: C parser 실패 시 p10-3 의 Tier 3 fallback wrapper (이미 wired) 가 picked up → `Citation::Code { symbol: None, lang: "c" }` + `code-text-paragraph-v1`.

### Module layout

```
crates/kebab-parse-code/src/
├── c.rs                            [신규] — C AST extractor (PARSER_VERSION `tree-sitter-c-<ver>`)
├── cpp.rs                          [신규] — C++ AST extractor (PARSER_VERSION `tree-sitter-cpp-<ver>`)
└── lib.rs                          [edit] — pub use + C_PARSER_VERSION / CPP_PARSER_VERSION 상수 노출

crates/kebab-chunk/src/
├── code_c_ast_v1.rs                [신규] — VERSION_LABEL `code-c-ast-v1`. 1A-2 패턴 (canonical Document → Vec<Chunk>).
├── code_cpp_ast_v1.rs              [신규] — VERSION_LABEL `code-cpp-ast-v1`. 동일 패턴.
└── lib.rs                          [edit] — pub use 2개

crates/kebab-source-fs/src/media.rs  [편집 불요] — code_lang_for_path 위임 패턴 그대로 (Task C of p10-2 이후 단일 source of truth).

crates/kebab-parse-code/src/lang.rs  [편집 불요] — `.c`/`.h`/`.cpp` 등 매핑은 1A-1 시점부터 이미 존재.

crates/kebab-app/src/lib.rs          [edit] — ingest_one_code_asset 의 allowlist + 4-arm match 에 "c" + "cpp" 추가. tier3 fallback list 에도 둘 추가.

crates/kebab-chunk/tests/             [신규]
├── fixtures/sample.c                — C fixture (top-level fn + struct)
├── fixtures/sample.cpp              — C++ fixture (namespace + class + method)
├── code_c_ast_snapshot.rs           — C snapshot test
└── code_cpp_ast_snapshot.rs         — C++ snapshot test

crates/kebab-app/tests/code_ingest_smoke.rs [edit] — 2 신규 integration test (c + cpp). 16 + 2 = 18.

Cargo.toml workspace.dependencies   [edit] — tree-sitter-c + tree-sitter-cpp.
crates/kebab-parse-code/Cargo.toml  [edit] — 위 2 dep 신규 entry.
```

## Acceptance criteria

- `cargo test --workspace --no-fail-fast -j 1` PASS (memory-conscious `-j 1`).
- `cargo clippy --workspace --all-targets -- -D warnings` clean.
- C fixture (`tests/fixtures/sample.c`) + C++ fixture (`tests/fixtures/sample.cpp`) ingest → chunk snapshot 안정. C snapshot 의 chunks 가 모두 `Citation::Code { lang: "c", symbol: Some(<fn|struct|enum name>), ... }`. C++ snapshot 의 chunks 가 namespace + class nesting 포함 (`kebab::chunk::Foo::bar`).
- 격리 TempDir KB 에 `.c` / `.cpp` 파일 두고 `kebab search --code-lang c --json` / `--code-lang cpp --json` 가 각각 `Citation::Code` 반환. integration test `tier1_c_ingest_searchable` + `tier1_cpp_ingest_searchable` (기존 16 + 2 = 18).
- `kebab schema --json | jq .stats.code_lang_breakdown` 에 `"c"` + `"cpp"` 카운트 등장 (.c/.cpp 파일 ingest 후).
- README + HANDOFF + docs/ARCHITECTURE + docs/SMOKE + tasks/INDEX + tasks/p10/INDEX 갱신.
- frozen design 2026-04-27 §10 activation log 한 줄.
- workspace `Cargo.toml` minor bump (0.15.0 → 0.16.0), gitea-release v0.16.0.

## Allowed dependencies

- `kebab-parse-code` 에 `tree-sitter-c` + `tree-sitter-cpp` workspace deps 추가. 기존 deps 유지.
- `kebab-chunk` 의 새 모듈 2개 (`code_c_ast_v1.rs`, `code_cpp_ast_v1.rs`) — language-agnostic body, tree-sitter import 금지. 기존 `tier2_shared::build_chunk` (pub(crate)) 재사용.
- `kebab-app`, `kebab-source-fs` — 새 crate dep 없음.

## Forbidden dependencies

- `kebab-chunk` 가 tree-sitter-c / tree-sitter-cpp 직접 import 금지 (boundary §6.3).
- `kebab-parse-code` 가 store / embed / llm / rag 직접 import 금지.
- UI crate (`kebab-cli` / `kebab-mcp` / `kebab-tui`) 가 `kebab-parse-code` / `kebab-chunk` 직접 import 금지 — `kebab-app` facade 만.

## Risks / notes

- **tree-sitter-c / tree-sitter-cpp 호환성**: tree-sitter 0.26 (현재 workspace) 과 호환 필요. resolve 시 `tree-sitter-language` shim 사용 fork (1C-JK 의 tree-sitter-kotlin-ng 패턴) 가능성 — crate.io 의 가장 활발한 maintainer 우선. 실패 시 별도 fork 검토.
- **`.h` parse 실패**: C++ 헤더 (`namespace`, `template`, `class`) 를 C parser 가 만나면 partial parse + error nodes. 1A-2 의 extractor 패턴이 error node 무시 + recoverable parse 진행 — emit 결과가 *불완전* 할 가능성. 그럴 때 chunks 가 0 으로 떨어지면 p10-3 Tier 3 fallback 으로 자동 picked up (이미 wired). 부분 emit 시 일부만 색인 — Tier 3 fallback 안 함. dogfood 후 HOTFIXES 검토.
- **Method out-of-class definition** (`Class::method` 형식): tree-sitter-cpp 의 `function_definition` 의 declarator 가 `qualified_identifier` 일 때 prefix 복원. fixture 로 검증.
- **Template specialization** (`template<> class Foo<int>`): tree-sitter-cpp 의 `template_declaration` 안의 `class_specifier` name 만 추출 — `Foo` 만 symbol 에 들어가고 `<int>` 미포함. design 의 generic 무시 룰 일관.
- **`extern "C"` block 안의 fn**: 일반 fn 처리. 외부 wrapping block 은 glue.
- **Anonymous union / struct** (`struct { int x; }` 변수 안에): 흔치 않음 + named 만 unit. anonymous 는 glue.
- **typedef-wrapped struct/enum idiom** (`typedef struct { ... } Foo;`) — ✅ v0.17.0 (2026-05-24) PR-B 에서 해소. extractor 의 `type_definition` 분기가 inner anonymous `struct_specifier` / `enum_specifier` / `union_specifier` 를 탐지해 declarator 의 typedef alias 이름으로 synthetic unit 방출. `PARSER_VERSION` `code-c-v1` → `code-c-v2` bump + same-workspace_path orphan purge cascade 동반. **잔여 미해결**: nested typedef (`typedef struct { struct {...} inner; } Outer;`) 의 inner 익명 struct 는 여전히 glue — v2 의 1차 범위는 top-level typedef alias 만. See [HOTFIXES.md 2026-05-21 entry](../HOTFIXES.md) (frozen 관찰) + 2026-05-24 closure entry.
- **Macro-heavy code** (Linux kernel 등): `#define FOO(x) ...` 매크로가 function-like 라도 parser 가 fn 으로 인식 안 함. preprocessor glue 로 처리 — symbol 안 잡힘. 의도된 동작 (parser 의 macro expansion 안 함).
- **`__attribute__((...))`** annotations: tree-sitter-c 의 attribute 노드는 declarator 옆 sibling. 무시 가능. function name 추출에 영향 없음.
- **fixture 크기**: sample.c 는 ~30 line (top-level fn + struct + enum + preprocessor), sample.cpp 는 ~50 line (nested namespace + class + method + template + free fn). oversize fallback 의 별도 검증은 1A-2 의 long_section_snapshot 패턴이 이미 cover (필요 시 별도 fixture).
- **머지 후 deviation** 은 `tasks/HOTFIXES.md` dated 로그 + 본 spec `Risks / notes` cross-link.
