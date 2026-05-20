# p10-1C-Go — Go AST chunker

**Status:** 🟡 진행 중
**Contract sections:** §3.3 (chunker_version `code-go-ast-v1`), §3.4 (symbol path — Go `package.Func` / `package.(*Receiver).Method`), §3.5 (code_lang `go`, ext `.go`), §6.1 (`kebab-parse-code/src/go.rs`), §6.2 (`kebab-chunk/src/code_go_ast_v1.rs`), §9.1 (Tier 1 AST per-language + oversize fallback).
**Design:** [2026-05-15-kebab-code-ingest-design.md](../../docs/superpowers/specs/2026-05-15-kebab-code-ingest-design.md) §1C (Go 부분 — Java + Kotlin 은 후속 PR).
**Plan:** [2026-05-20-p10-1c-go-ast-chunker.md](../../docs/superpowers/plans/2026-05-20-p10-1c-go-ast-chunker.md).

## Goal

1A-2 / 1B 인프라 위에 Go AST chunker 활성화. 사용자 결정으로 1C 의 3 언어 (Go + Java + Kotlin) 를 2 PR 로 분할 — Go 가 method receiver / package convention 면에서 Java/Kotlin (JVM family) 과 다르므로 별 PR. 본 PR 머지 시점부터 Go 프로젝트 dogfooding 가능.

## 동결된 설계 결정 (이 task 로 확정)

- **Symbol path 의 package prefix = 소스 코드의 `package` 선언에서 추출** (design §3.4 그대로). 1B 의 workspace-path 변환과 다름 — Go 는 언어 자체에 `package` declaration 이 있어 그게 canonical source. tree-sitter-go 의 `source_file` root 의 첫 named child `package_clause` 에서 추출. 빈 경우 (이론상 invalid Go, 실용엔 거의 없음) `<unknown>` 또는 fallback `<package>` (1A `<module>` 패턴과 유사).
- **Method receiver 표현** (design 예시 그대로): `package.(*Receiver).Method` (포인터 receiver), `package.(Receiver).Method` (value receiver). tree-sitter-go 의 `method_declaration` 의 `receiver` field 에서 type + pointer 여부 추출. 예: `func (m *MdHeadingV1Chunker) ChunkDoc(...)` → symbol `chunk.(*MdHeadingV1Chunker).ChunkDoc`.
- **Top-level unit 종류**:
  - `function_declaration` → 1 unit, symbol `package.Func`
  - `method_declaration` → 1 unit, symbol `package.(*Receiver).Method` / `package.(Receiver).Method`
  - `type_declaration` (struct / interface / type alias) → 1 unit each, symbol `package.TypeName`
  - `const_declaration`, `var_declaration`, `import_declaration` (블록 또는 단일) → glue, grouped → `package.<top-level>` (1A/1B 패턴)
- **Go 의 generic 처리**: `func Foo[T any](...)` 또는 `type Foo[T any] struct{}` 의 type parameter 는 symbol 에 미포함 (Go 자체도 보통 symbol 에 안 적음). 단순 `package.Foo` 만.
- **Test detection**: Go 의 `func TestXxx(t *testing.T)` 는 *일반 fn 으로 emit*. test 감지 boost/penalty 등 ranking 영향은 본 task 범위 밖 (ranking brainstorm 보류 메모리 따름).
- frozen design 자체는 변경 없음 (§3.4 의 Go 행이 이미 본 결정과 일치). §10.1 에 1C-Go 활성화 한 줄 추가.

## Acceptance criteria

- `cargo test --workspace --no-fail-fast -j 1` passes (memory-conscious: per-crate 위주, full-suite gate 는 docs task 직전 1회).
- `cargo clippy --workspace --all-targets -- -D warnings` passes.
- Go fixture (`tests/fixtures/sample.go`) ingest → chunk snapshot 안정 + `Citation::Code` 의 symbol 이 §3.4 컨벤션 일치 (`pkg.Func` / `pkg.(*Receiver).Method`).
- 격리 TempDir KB 에 Go 파일 두고 `kebab search --code-lang go --json` 가 `Citation::Code { lang: "go", symbol: "...", ... }` 반환.
- `kebab schema --json | jq .stats.code_lang_breakdown` 에 `"go"` 카운트.
- README + HANDOFF + ARCHITECTURE + SMOKE + tasks/INDEX + tasks/p10/INDEX 갱신.
- frozen design §10.1 한 줄 추가.
- workspace `Cargo.toml` minor bump (0.11.1 → 0.12.0).

## Allowed dependencies

- `kebab-parse-code` 에 `tree-sitter-go` 추가 (workspace deps). 기존 deps 유지.
- `kebab-chunk` 의 새 모듈 `code_go_ast_v1.rs` — kebab-core + serde_json_canonicalizer + blake3 + anyhow + tracing. tree-sitter 절대 import 금지.
- `kebab-app`, `kebab-source-fs` 변경 — 새 crate dep 없음.

## Forbidden dependencies

- `kebab-chunk` 가 `tree-sitter-go` 직접 import 금지.
- UI crate 가 `kebab-parse-code` 직접 import 금지.
- `kebab-parse-code` 가 store / embed / llm / rag 직접 import 금지.

## Risks / notes

- tree-sitter-go 의 `package_clause` node 가 root 의 첫 named child 인지 grammar 버전에 따라 다를 수 있음 — extractor 가 `source_file` 전체를 named_children iterate 하면서 첫 `package_clause` 잡는 방식이 안전.
- `method_declaration` 의 receiver pointer 여부: tree-sitter-go AST 에서 receiver type 이 `pointer_type` 노드면 `*Receiver`, 그냥 `type_identifier` 면 `Receiver`. 정확한 텍스트 추출 필요.
- Generic type parameter (`[T any]`) 가 method_declaration / function_declaration 의 name field 와 별도 child — name 만 추출하면 generic 부분 자동 제외.
- 1B Python/TS/JS 패턴 (helpers from lang.rs) 와 *다른* 모델 — 본 task 의 mod_prefix 는 source-side AST 에서 추출, helper fn 불필요.
- 머지 후 deviation 은 `tasks/HOTFIXES.md` 에 dated 로그 + 본 spec `Risks / notes` 에 one-line cross-link.
