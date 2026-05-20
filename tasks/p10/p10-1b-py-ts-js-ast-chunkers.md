# p10-1B — Python + TS/JS AST chunkers

**Status:** 🟡 진행 중
**Contract sections:** §3.3 (chunker_version `code-python-ast-v1` / `code-ts-ast-v1` / `code-js-ast-v1`), §3.4 (symbol path — Python `pkg.module.Class.method`, TS/JS `module/Class.method` / `module/default`), §3.5 (code_lang `python` / `typescript` / `javascript`), §5 (확장자 라우팅 활성화), §6.1 (`kebab-parse-code/src/{python,typescript,javascript}.rs`), §6.2 (`kebab-chunk/src/code_{python,ts,js}_ast_v1.rs`), §9.1 (Tier 1 AST per-language + oversize fallback).
**Design:** [2026-05-15-kebab-code-ingest-design.md](../../docs/superpowers/specs/2026-05-15-kebab-code-ingest-design.md) §1B.
**Plan:** [2026-05-20-p10-1b-py-ts-js-ast-chunkers.md](../../docs/superpowers/plans/2026-05-20-p10-1b-py-ts-js-ast-chunkers.md).

## Goal

1A-2 가 깐 인프라 (`SourceSpan::Code`, `MediaType::Code(String)`, `Citation::Code` 매핑, `citation_helper` arm, `backfill_code_lang` + `backfill_repo`, `schema.v1.code_lang_breakdown`, `[ingest.code]` 절, HOTFIXES) 위에 **Python + TypeScript + JavaScript** 3 언어의 extractor + chunker 를 활성화. design §1B 묶음과 일치하는 단일 PR. 머지 시점부터 Python / TS / JS 프로젝트도 dogfooding 가능.

## 동결된 설계 결정 (이 task 로 확정)

- **Symbol path 의 module prefix = workspace 경로 → module path 변환** (design §3.4 예시 충실, 사용자 명시 결정):
  - **Python**: `crates/x/src/foo/bar.py` 같은 workspace_path 를 `/`/`__init__.py` 처리 + `.py`·`.pyi` strip + `/` → `.` 변환 후 dotted prefix 로 사용. 예시: `kebab_eval/metrics.py` 의 `def compute_mrr()` → symbol `kebab_eval.metrics.compute_mrr`. `pkg/__init__.py` 는 module `pkg` 자체. 변환은 `kebab-parse-code::lang::module_path_for_python(workspace_path)` 단일 함수 (source of truth).
  - **TS/JS**: `src/search/retriever/Retriever.ts` → `src/search/retriever/Retriever` prefix + `/` 구분자 보존 + `.ts`/`.tsx`/`.js`/`.jsx`/`.mjs`/`.cjs` strip. 예시: `src/search/retriever/Retriever.ts` 의 method `search` → `src/search/retriever/Retriever.search`. `module/default` 는 `export default function/class` 경우. 변환은 `module_path_for_tsjs(workspace_path)`.
  - **Rust 1A-2 는 retrofit 하지 않음** — 1A 는 file-scope nesting 만 사용 (workspace prefix 없음). 비일관 수용; HOTFIXES 2026-05-20 에 기록 + 사용자가 명시 요청 시 retrofit (chunker_version bump + re-ingest cascade 필요).
- **TypeScript grammar selection**: `tree-sitter-typescript` crate 의 `LANGUAGE_TYPESCRIPT` 는 `.ts`, `LANGUAGE_TSX` 는 `.tsx` 에 사용. 파일 확장자로 선택. `code-ts-ast-v1` 하나의 chunker 가 둘 다 처리 (parser_version `code-ts-v1`).
- **JavaScript grammar**: `tree-sitter-javascript` 단일 LanguageFn 가 `.js` / `.mjs` / `.cjs` / `.jsx` 모두 처리. 별도 분기 불필요.
- **Expression-level 함수 (arrow fn / function expression assigned to const)**: 1B 1차에서는 *declaration-level 만* unit (function_declaration / class_declaration / method_definition / interface_declaration / type_alias_declaration / decorated_definition 등). `const foo = () => {...}` 같은 expression-level 은 glue 로 잡힘. HOTFIXES 2026-05-20 기록; 후속 phase 에서 lexical_declaration 안의 함수 표현식 unwrap 추가 검토.
- **App dispatch 일반화**: 현재 `ingest_one_code_asset` 은 RustAstExtractor + CodeRustAstV1Chunker 하드코딩. 1B 에서 `lang: &str` 받아 dispatch (Rust 도 동일 함수로 흡수) — Extractor 와 Chunker 를 trait object 가 아니라 enum/match 로 선택 (kebab-app 만 변경, kebab-core/Chunker trait 불변). frozen design 영향 없음.
- frozen design 자체는 변경 없음 (§3.4 의 symbol path 예시는 이미 본 결정과 일치). §10.1 (post-merge surface) 에 1B 활성화 한 줄 추가.

## Acceptance criteria

- `cargo test --workspace --no-fail-fast -j 1` passes (메모리 의식적으로는 per-crate; full-suite gate 는 Task K 직전 1회).
- `cargo clippy --workspace --all-targets -- -D warnings` passes.
- 3 언어 각각의 fixture (`tests/fixtures/sample.{py,ts,js}`) ingest → chunk snapshot 안정 + `Citation::Code` 의 symbol/line 이 §3.4 컨벤션 (workspace path → module path) 과 일치.
- 격리 TempDir KB 에 Python/TS/JS 파일 하나씩 두고 `kebab search --code-lang {python|typescript|javascript} --json` 가 정상 결과 반환.
- `kebab schema --json | jq .stats.code_lang_breakdown` 에 `python`, `typescript`, `javascript` 카운트 등장.
- README + HANDOFF + ARCHITECTURE + SMOKE + tasks/INDEX + tasks/p10/INDEX 갱신.
- frozen design §10.1 한 줄 추가 (1B 활성화).
- HOTFIXES 2026-05-20 에 (a) Rust 1A-2 symbol path 비일관 (1B 와 다름), (b) expression-level 함수 단위 제외 — 두 편차 기록.
- workspace `Cargo.toml` minor bump (0.7.0 → 0.8.0) — 도그푸딩 가능 surface 확장.

## Allowed dependencies

- `kebab-parse-code` 에 `tree-sitter-python`, `tree-sitter-typescript`, `tree-sitter-javascript` 추가 (workspace deps 경유). 기존 `kebab-core` / `anyhow` / `gix` / `tree-sitter` / `tree-sitter-rust` / `serde_json` / `time` / `tracing` 유지.
- `kebab-chunk` 의 새 모듈 3개 (`code_python_ast_v1.rs` / `code_ts_ast_v1.rs` / `code_js_ast_v1.rs`) — 1A-2 chunker 와 동일 dep (kebab-core + serde_json_canonicalizer + blake3 + anyhow + tracing). tree-sitter 절대 import 금지.
- `kebab-app` 변경 — 새 crate dep 없음.
- `kebab-source-fs` — 확장자 추가만, 새 dep 없음.

## Forbidden dependencies

- `kebab-chunk` 가 `tree-sitter-*` 직접 import 금지 (AST 는 parser-side).
- UI crate (cli / mcp / tui) 가 `kebab-parse-code` 직접 import 금지.
- `kebab-parse-code` 가 store / embed / llm / rag 직접 import 금지 (design §8 inheritance).

## Risks / notes

- tree-sitter-typescript 의 `LANGUAGE_TYPESCRIPT` 와 `LANGUAGE_TSX` 가 별도 LanguageFn — 잘못 선택하면 TSX JSX 가 parse 실패. 파일 확장자 기반 선택을 단일 함수에서 결정 (테스트로 고정).
- tree-sitter-python 의 `decorated_definition` 노드 처리 — 데코레이터가 wrap 하는 형태라 `function_definition` / `class_definition` 가 child. unwrap 필요 (decorator 라인은 unit_start backward extension 으로 자연스럽게 포함됨).
- Python `pkg/__init__.py` 의 module path = `pkg` 자체 (basename 제거). `module_path_for_python` 가 이걸 처리.
- TS/JS 의 `export default function/class` — name 이 없을 수 있음 (`export default function () {...}`). symbol `module/default` 로 표기 (design §3.4).
- `module_path_for_python` / `module_path_for_tsjs` 가 workspace_path 의 비-ASCII / 공백 / 특수문자 처리 필요. 1B 1차에서는 그대로 전달 (sanitize 없음); HOTFIXES 에 path-sanitize 부재 기록.
- 1A-2 `ingest_one_code_asset` 일반화로 인한 dispatch 코드 변경 — Rust 기존 동작 byte-identical 유지를 통합 테스트로 확인.
- 머지 후 deviation 은 `tasks/HOTFIXES.md` 에 dated 로그 + 본 spec `Risks / notes` 에 one-line cross-link.
- **[HOTFIXES 2026-05-20]** Rust 1A-2 symbol 은 file-scope nesting 만 (workspace prefix 없음); 1B 의 Python/TypeScript/JavaScript 와 비일관 — retrofit 은 사용자 명시 요청 시. 자세한 내용: `tasks/HOTFIXES.md` (2026-05-20, "Rust 1A-2 symbol path").
- **[HOTFIXES 2026-05-20]** TypeScript/JavaScript 의 expression-level 함수 (`const foo = () => {}` 등) 는 `<top-level>` glue 로 처리됨, 독립 unit 미방출 — 후속 phase 에서 `lexical_declaration` unwrap 검토. 자세한 내용: `tasks/HOTFIXES.md` (2026-05-20, "expression-level functions").
- **[HOTFIXES 2026-05-20]** `module_path_for_python` / `module_path_for_tsjs` 가 path-sanitize 안 함 (특수문자/공백 그대로 prefix 에 들어감) — 후속 phase 에서 NFKC + 사용금지 문자 변환 검토. 자세한 내용: `tasks/HOTFIXES.md` (2026-05-20, "module_path_for_python / _tsjs do not sanitize").
