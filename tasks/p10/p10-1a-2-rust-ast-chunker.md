# p10-1A-2 — Rust AST chunker

**Status:** 🟡 진행 중
**Contract sections:** §3.3 (chunker_version `code-rust-ast-v1`), §3.4 (symbol path — Rust convention), §3.4 frozen-design (`SourceSpan::Code` 신규 internal variant), §5 (code ingest 활성화), §6.1 (`kebab-parse-code/src/rust.rs` — tree-sitter-rust → CanonicalDocument), §6.2 (`kebab-chunk/src/code_rust_ast_v1.rs`), §9.1 (Tier 1 AST per-language + oversize fallback).
**Design:** [2026-05-15-kebab-code-ingest-design.md](../../docs/superpowers/specs/2026-05-15-kebab-code-ingest-design.md) §1A-2.
**Plan:** [2026-05-19-p10-1a-2-rust-ast-chunker.md](../../docs/superpowers/plans/2026-05-19-p10-1a-2-rust-ast-chunker.md).

## Goal

1A-1 의 프레임워크 위에 **Rust AST chunker 자체**를 올린다. `tree-sitter` + `tree-sitter-rust` 도입, `kebab-parse-code/src/rust.rs` (tree-sitter-rust → `CanonicalDocument`, AST 의미 단위마다 `Block::Code` + `SourceSpan::Code`), `kebab-chunk/src/code_rust_ast_v1.rs` (1 block → 1 chunk + oversize fallback split), `MediaType::Code` 신설, `kebab-app` dispatch. 머지 시점에 kebab 자기 자신 dogfooding 가능.

## 동결된 설계 결정 (이 task 로 확정)

- **tree-sitter 위치 = parser (`kebab-parse-code`)**, chunker 아님. design §6.3 의존성 그래프 (`kebab-parse-code → tree-sitter, tree-sitter-rust`) 가 authoritative. PDF 선례와 동형 — parser 가 구조화된 block 생성, chunker 가 매핑. §9.1 의 "chunker 가 AST" 서술은 *oversize fallback split* 만 chunker-side 라는 의미로 해석.
- **`SourceSpan::Code { line_start, line_end, symbol, lang }` 내부 variant 신설** (kebab-core). chunk 의 `source_spans_json` (chunks 테이블) 은 *내부 저장*이라 wire schema 아님 → wire major bump 불필요. `Citation::Code` (wire) 는 1A-1 에서 이미 추가됨. `citation_helper::citation_from_first_span` 에 `SourceSpan::Code → Citation::Code` arm 추가로 symbol/lang 이 자연스럽게 흐름.
- **`MediaType::Code(String)` 신설** — String = canonical code_lang (1A 는 `"rust"` 만 실제 처리, 그 외 인식된 code lang 은 `Skipped` — Tier 2/3 는 후속 phase).
- frozen design §3.4 의 `SourceSpan` enum 및 (해당 시) `MediaType` enum 목록을 같은 PR 에서 갱신. 본 task spec 은 머지 후 frozen.

## Acceptance criteria

- `cargo test --workspace --no-fail-fast -j 1` passes.
- 기존 markdown / PDF / image corpus regression test 무영향 (citation 5→6 variant: `Citation::Code` 는 1A-1 에 이미 존재; 기존 5 variant 직렬화 불변).
- `cargo clippy --workspace --all-targets -- -D warnings` passes.
- Rust fixture 한 개 (fn / impl method / struct / trait / top-level use + 200줄 초과 fn) ingest → chunk snapshot 안정 + `Citation::Code` 의 symbol/line 이 spec §3.4 Rust convention 과 일치.
- kebab 자기 crate 한 개를 isolated TempDir KB 에 ingest → `kebab search --json` 결과가 `citation.kind == "code"`, `repo`, `code_lang == "rust"` 반환 (SMOKE 절차).
- `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md` §3.4 (SourceSpan / MediaType) + §10.1 갱신.
- README + HANDOFF + ARCHITECTURE + SMOKE + tasks/INDEX.md + tasks/p10/INDEX.md 갱신.
- workspace `Cargo.toml` version minor bump (도그푸딩 가능 = bump 트리거, design §10.4) + release cut.

## Allowed dependencies

- `kebab-parse-code` 에 `tree-sitter` + `tree-sitter-rust` 추가 (workspace deps 경유). 기존 `kebab-core` / `anyhow` / `gix` 유지.
- `kebab-chunk` 는 `kebab-core` 만 (chunker 는 `CanonicalDocument` 만 소비 — tree-sitter import 금지).
- `kebab-app → kebab-parse-code` (facade 가 Extractor 호출).

## Forbidden dependencies

- `kebab-chunk` 가 `tree-sitter*` import 금지 (AST 는 parser-side).
- UI crate (cli / mcp / tui) 가 `kebab-parse-code` 직접 import 금지 — `kebab-app` facade 만.
- `kebab-parse-code` 가 store / embed / llm / rag import 금지 (design §8 inheritance).

## Risks / notes

- tree-sitter-rust 의 grammar 버전에 따라 node kind 명칭 차이 가능 — `function_item` / `impl_item` / `struct_item` / `enum_item` / `trait_item` / `mod_item` / `use_declaration` 는 도입 버전으로 pin 후 테스트로 고정.
- `SourceSpan::Code` 추가로 `SourceSpan` 의 모든 exhaustive match (citation_helper, store-sqlite serde, search) 가 영향 — 컴파일러가 non-exhaustive 를 잡아주므로 전수 대응.
- oversize fallback (단일 fn > `ast_chunk_max_lines`) 의 `symbol [part i/N]` 표기는 1A-2 chunker 내부 한정. 일반 Tier-3 `code-text-paragraph-v1` 은 Phase 3.
- 머지 후 동작 deviation 은 `tasks/HOTFIXES.md` 에 dated 로그 + 본 spec `Risks / notes` 에 one-line cross-link.
- AST_CHUNK_MAX_LINES deviation logged in HOTFIXES.md (2026-05-19): `Chunker` trait 이 per-medium config 미노출 — 상수 200 고정, default 와 동일하므로 user-visible 영향 없음.
- SourceType::Code deferred logged in HOTFIXES.md (2026-05-19): code 파일이 `SourceType::Note` 로 분류됨, `MediaType::Code` 기반 filter 는 정상 동작.
