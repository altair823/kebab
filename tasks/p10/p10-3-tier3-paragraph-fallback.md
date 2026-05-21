# p10-3 — Tier 3 paragraph + line-window fallback chunker

**Status:** 🟡 진행 중
**Contract sections:** §3.3 (chunker_version `code-text-paragraph-v1`), §3.5 (code_lang routing — `shell` 활성화 + "미지원 / Tier 3 fallback" 명확화), §6.2 (`kebab-chunk/src/code_text_paragraph_v1.rs`), §6.3 (`tier2_shared::build_chunk` 의 `pub(crate)` 노출), §9.3 (Tier 3 정의), §10.1 (deactivation log 한 줄).
**Design:** [2026-05-15-kebab-code-ingest-design.md](../../docs/superpowers/specs/2026-05-15-kebab-code-ingest-design.md) §1.3 (Phase 3) + §9.3.
**Plan:** [2026-05-20-p10-3-tier3-paragraph-fallback.md](../../docs/superpowers/plans/2026-05-20-p10-3-tier3-paragraph-fallback.md).

## Goal

p10-1A-2 / 1B / 1C / 1A-1 의 framework + p10-2 Tier 2 인프라 위에 Tier 3 paragraph fallback chunker 활성화. 단일 PR. 머지 시점부터:

- `.sh` / `.bash` / `.zsh` 파일이 paragraph 단위로 색인.
- p10-2 의 비-k8s YAML / invalid YAML / Tier 1 AST extractor 실패 등 0-chunk 결과가 자동으로 Tier 3 로 fallback 되어 색인 — 이전에 skip 되던 파일이 search 가능.

## 동결된 설계 결정 (이 task 로 확정)

### chunker (`code-text-paragraph-v1`)

- **Input**: `Document` with single `Block::Code { text, lang, ... }`. Tier 2 의 `synthesize_tier2_document` 와 동일한 모양 — fallback wrapper 가 같은 doc 재사용.
- **VERSION_LABEL**: `"code-text-paragraph-v1"`.
- **Paragraph 분할**: `text.lines()` 순회. 빈 줄 (정확히 빈 줄 또는 only-whitespace) 을 paragraph boundary 로. 빈 줄 자체는 어느 paragraph 에도 포함되지 않음 (chunk 의 line range 에 미포함). 빈 paragraph (전부 whitespace) skip.
- **Paragraph 크기 룰** (design §9.3 default 그대로, hardcoded):
  - paragraph line count ≤ 80 → 1 chunk emit.
  - paragraph line count > 80 → line-window split with window size 80 / overlap 20 (stride 60). 즉 line 1-80, 61-140, 121-200, … 마지막 window 는 EOF 까지 (≤ 80 lines).
  - `FALLBACK_LINES_PER_CHUNK = 80`, `FALLBACK_LINES_OVERLAP = 20` 둘 다 hardcoded constants (1A-2 의 `AST_CHUNK_MAX_LINES = 200` 패턴 그대로 — 사용자 config 노출 안 함, 미래 HOTFIXES 시 노출 검토).
- **Citation**: `SourceSpan::Code { line_start, line_end, symbol: None, lang: <input lang> }`. `symbol = None` 통일 (Tier 3 는 의미 단위 식별 안 함). `lang` 은 입력 Document 의 `Block::Code.lang` 그대로 보존 — shell → `"shell"`, k8s skip → `"yaml"`, Rust extractor 실패 → `"rust"` 등.
- **chunk_id 충돌 방지**: 동일 paragraph 의 line-window split 시 `id_for_chunk` 의 `split_key` 에 `window_start` 전달 (Tier 2 `#L{k}` 패턴 동일).
- **Edge cases**:
  - 전체 파일이 빈 줄만 → 0 chunk emit (fallback 의 fallback 없음). `tracing::warn!`.
  - 단일 paragraph + ≤ 80 lines → 1 chunk, line range 1..N.
  - 빈 줄 없는 거대 파일 (한 paragraph 전체) → line-window split.

### Routing / fallback wrapper

- **`code_lang_for_path`** 변경 없음 (shell 매핑은 1A-1 시점부터 이미 존재).
- **`ingest_one_code_asset` allowlist** (`crates/kebab-app/src/lib.rs:953`) 에 `"shell"` 추가.
- **4-arm match (parser_version / chunker_version / extract / chunks)** 에 `"shell"` arm 추가:
  - parser_version = `"none-v1"` (Tier 2 sentinel 재사용).
  - chunker_version = `CodeTextParagraphV1Chunker.chunker_version()`.
  - extract = `synthesize_tier2_document(asset, &bytes, "shell", &parser_version)?` (재사용).
  - chunks = `CodeTextParagraphV1Chunker.chunk(&canonical, chunk_policy)?`.
- **Fallback wrapper** (핵심 신규 로직) — chunks match 직후 후처리:
  - Tier 1/2 lang 의 결과가 `Err(_)` 또는 `Ok(empty_vec)` 이면 Tier 3 retry.
  - retry 시:
    - `chunker_version` 를 `code-text-paragraph-v1` 로 swap (downstream stamping 정확성).
    - `canonical.parser_version` 도 `"none-v1"` 로 swap (Tier 1 의 `RUST_PARSER_VERSION` 등이 misleading 하므로).
    - `CodeTextParagraphV1Chunker.chunk(&canonical, chunk_policy)` 실행.
  - 실패 사유는 `tracing::warn!("tier1/2 emitted 0 chunks or errored for {workspace_path} ({code_lang}); falling back to tier 3")`.
- **Tier 3 자체가 0 chunk 또는 Err** 인 경우는 그대로 fail/skip (fallback 의 fallback 없음).

### `tier2_shared::build_chunk` 노출

- 현재 module-private `fn build_chunk`. Tier 3 가 동일 Chunk 생성 (hash / token / policy_hash 일관) 을 위해 호출 — `pub(crate) fn build_chunk(...)` 으로 visibility 만 변경. signature 동일.

### Lang 보존 정책

- Tier 3 chunk 의 `Citation::Code.lang` = 입력 Document 의 `Block::Code.lang` 그대로. 명시적으로 표:
  | Source | input lang | Tier 3 output lang |
  |--------|-----------|----------|
  | shell direct | `"shell"` | `"shell"` |
  | k8s 0-chunk fallback | `"yaml"` | `"yaml"` |
  | Rust AST 실패 fallback | `"rust"` | `"rust"` |
  | manifest 0-chunk (이론상, 거의 발생 안 함) | `"toml"` 등 | 유지 |
- 검색 시 `--code-lang shell` / `--code-lang yaml` 등이 fallback chunk 도 매칭 — search filter 동작 자연.

### Non-scope

- **미지원 확장자 wiring**: `.txt` / `.log` / `.scala` / `.rb` 등은 본 PR scope 밖. `code_lang_for_path` 의 매핑은 unchanged. Tier 3 chunker 자체는 만들어두고, 미래에 `code_lang_for_path` 에 새 lang 추가 시 자동 picked up (1A-2 패턴).
- **config 노출**: `FALLBACK_LINES_PER_CHUNK` / `FALLBACK_LINES_OVERLAP` hardcoded. config.toml 노출 없음.

### Frozen design 갱신

- `docs/superpowers/specs/2026-05-15-kebab-code-ingest-design.md` §10.1 활성화 로그 한 줄.
- `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md` §10 activation log 한 줄.
- §3.5 의 "미지원 / Tier 3 fallback → null" 표현은 그대로 유지 (해당 표현이 본 phase 의 정확한 의미 — Tier 3 chunk 의 lang 은 입력 lang 보존이므로 "null" 은 미지원 확장자 wire 시 적용).

## Acceptance criteria

- `cargo test --workspace --no-fail-fast -j 1` PASS (memory-conscious `-j 1`).
- `cargo clippy --workspace --all-targets -- -D warnings` clean.
- 4 신규 unit test in `crates/kebab-chunk/tests/code_text_paragraph_v1.rs`:
  - `shell_multi_paragraph_splits_on_blank_lines` — 3-paragraph fixture → 3 chunk, symbol=None, lang=shell, contiguous (exclusive of blank lines).
  - `single_long_paragraph_line_window_split` — 200+ line single paragraph → window split, distinct chunk_ids, expected line ranges (1-80, 61-140, 121-200, …).
  - `empty_file_emits_zero_chunks` — 빈 텍스트 → `Ok(vec![])`.
  - `lang_field_preserved_from_input_doc` — lang=yaml 입력 → emit chunk lang=yaml.
- 2 신규 integration test in `crates/kebab-app/tests/code_ingest_smoke.rs`:
  - `tier3_shell_ingest_searchable` — `.sh` 파일 ingest → `--code-lang shell` 검색 → `Citation::Code { symbol: None, lang: "shell" }`, `chunker_version: "code-text-paragraph-v1"`.
  - `tier3_yaml_fallback_picks_up_non_k8s_yaml` — apiVersion+kind 없는 yaml ingest → fallback 발동 → `Citation::Code { symbol: None, lang: "yaml" }`, chunker_version `code-text-paragraph-v1`.
- 기존 12 smoke test + 2 신규 = 14 testing surface. (Tier 1 9 + Tier 2 3 + Tier 3 2.)
- `kebab schema --json | jq .stats.code_lang_breakdown` 에 `"shell"` 카운트 등장 (.sh 파일 ingest 후). 비-k8s YAML 도 `"yaml"` 카운트에 누적 (Tier 2 와 Tier 3 가 같은 lang).
- README + HANDOFF + docs/ARCHITECTURE + docs/SMOKE + tasks/INDEX + tasks/p10/INDEX 갱신.
- frozen design §10.1 + §10 activation log 한 줄씩.
- workspace `Cargo.toml` minor bump (0.14.0 → 0.15.0), gitea-release v0.15.0.

## Allowed dependencies

- `kebab-chunk` 의 새 모듈 `code_text_paragraph_v1.rs` — kebab-core + anyhow + tracing. tier2_shared 의 `build_chunk` 호출 (visibility `pub(crate)` 로 노출). tree-sitter / serde_yaml 비사용.
- `kebab-app::ingest_one_code_asset` — 4-arm match + allowlist + fallback wrapper 확장. 새 crate dep 없음.
- `kebab-parse-code` — 변경 없음 (lang.rs 의 shell 매핑은 1A-1 부터 존재).
- `kebab-source-fs` — 변경 없음 (media.rs 이미 `code_lang_for_path` 위임).

## Forbidden dependencies

- `kebab-chunk` 가 store / embed / llm / rag / tree-sitter 직접 import 금지 (boundary §6.3 유지).
- UI crate (`kebab-cli` / `kebab-mcp` / `kebab-tui` / `kebab-desktop`) 가 `kebab-parse-code` / `kebab-chunk` 직접 import 금지 — `kebab-app` facade 만.

## Risks / notes

- **Fallback infinite loop 방지**: Tier 3 자체가 0 chunk 또는 Err 인 경우는 그대로 fail/skip — fallback 의 fallback 없음. 명시 spec.
- **chunker_version swap 시 `try_skip_unchanged` 일관성**: fallback 발동 후 stored chunker_version = `code-text-paragraph-v1`. 다음 ingest 에 동일 파일 → 동일 chunker_version 으로 lookup 매칭 (skip 동작 OK). Tier 1 chunker 가 미래에 작동하기 시작하면 (예: tree-sitter grammar fix) cascade rule 로 incremental cache miss → 자동 reprocess 가 정상 동작.
- **lang 보존 vs fallback 의미**: fallback chunk 의 lang 이 원본 lang 유지라 search filter `--code-lang yaml` 가 Tier 2 와 Tier 3 chunk 둘 다 매칭. 의도된 동작 — 사용자가 "yaml 파일 검색" 했을 때 모든 yaml 결과 표시.
- **line-window overlap 의미**: 80/20 (stride 60) 은 design §9.3 default. 거대 paragraph (예: minified JSON 한 줄) 의 경우에도 동일 알고리즘 — 단 한 줄 = 한 line 이라 split 발생 안 함 (length 80 lines 기준). minified 의 경우 chunk 한 개에 매우 긴 텍스트가 들어가는데 이는 paragraph 분할 정책의 inherent limitation. 미래 HOTFIXES 검토.
- **빈 줄 처리**: `^\s*$` 매칭 (whitespace-only) 줄을 paragraph boundary 로. 탭만 있는 줄 / CR-only 줄 등 edge case fixture 로 검증.
- **shell line-comment 처리**: shell script 의 `# comment` 줄은 일반 line. paragraph 분할에 영향 없음 (빈 줄 아님). chunk 안에 그대로 보존.
- **fallback wrapper 의 `canonical.parser_version` mutation**: Document 의 parser_version 을 Tier 3 fallback 시 `"none-v1"` 로 swap. CanonicalDocument 가 `mut` 로 받아져야 함. 이미 `let mut canonical = match ...` 이라 mut 가능. plan 단계 검증.
- **머지 후 deviation** 은 `tasks/HOTFIXES.md` dated 로그 + 본 spec `Risks / notes` cross-link.
