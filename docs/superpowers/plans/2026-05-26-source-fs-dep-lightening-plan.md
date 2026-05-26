---
status: open
target_version: 0.18.0
spec: docs/superpowers/specs/2026-05-26-source-fs-dep-lightening-spec.md
contract_sections: ["§3.5", "§3.7b", "§5.2", "§8"]
---

# kebab-source-fs dep lightening — implementation plan

> **Round 2 reflection**: 12 step → 10 step. parse-code cleanup 을 atomic clippy gate 로 통합 (구 Step 9+10 합침), 그리고 kebab-core doc / ARCHITECTURE / design §8 / workspace 회귀 / commit 을 single closure step 으로 통합 (구 Step 11+12 합침). 모든 BLOCKER + MAJOR + MINOR + NIT 의 reflection 위치는 §9 closure table 참조.

## §0 Pre-flight + branch state

- **Branch**: `refactor/source-fs-dep-lightening` (현재 위치, main 위에서 분기 완료).
- **Base SHA**: `b02ac82` (HOTFIX #15 + S3 NLI 머지 직후, v0.18.0 cut 완료 시점 — spec §1.6 / §5.2 baseline 과 동일).
- **Working dir**: `/home/altair823/kebab`.
- **Env 강제** (CLAUDE.md disk-protection):
  - `export CARGO_TARGET_DIR=/build/out/kebab/target` — 본 plan 의 모든 cargo 명령에 적용. `target/` 가 repo root 아래에 생성되지 않게.
  - `export TMPDIR=/build/cache/tmp` — 대용량 임시 파일 발생 시 보호.
- **Cargo build 직렬화**: 모든 cargo 명령 `-j 1` 강제 (CLAUDE.md "Build / test / lint" — 18 integration-test binary 동시 link 시 OOM). per-crate `-p` 명령은 `-j 1` 없어도 OK 지만, workspace 단위 `--workspace` 만 `-j 1` 필수. 본 plan 은 일관성을 위해 모든 cargo 호출에 `-j 1` 명시.
- **Memory persistence** (`~/.claude/projects/-home-altair823-kebab/memory/MEMORY.md` 의 `feedback_serial_build_only.md` 참조): cargo test/clippy/build 동시 bg 실행 금지. 하나 끝난 후 다음.
- **HOTFIXES.md / HANDOFF.md / README.md / tasks/INDEX.md / 4 frozen task spec 변경 0** (spec §7 명시).
- **workspace `Cargo.toml` version bump 0** (spec NG5).
- **wire schema / Config / V00X migration 영향 0** (spec §7).

## §1 Approach summary

Spec §3 의 핵심 sequencing:

1. **신규 module 부터 작성** — `kebab-source-fs/src/code_meta.rs` 에 4 surface (`BUILTIN_BLACKLIST` `pub`, 3 helper fn `pub(crate)`) 본문 byte-identical 이동 + 12 unit test 이전.
2. **lib.rs 에 module + pub use 등록** — 이 시점부터 양쪽 surface (`kebab_parse_code::...` 와 `crate::code_meta::...`) 공존. cargo check 통과.
3. **5 callsite migration** — `media.rs` (1) → `walker.rs` (2 + 주석 3) → `connector.rs` (2).
4. **`Cargo.toml` 의 `kebab-parse-code` dep 제거** — 본 plan 의 anchor step. G1 + G5 (`cargo tree | grep tree-sitter` 0 줄) 달성.
5. **integration test 신설** — `tests/code_meta.rs::builtin_blacklist_has_exactly_six_entries` 가 design §5.2 frozen contract 의 외부 검증 surface.
6. **parse-code 측 atomic cleanup** — skip.rs 삭제 + lib.rs (skip 줄 + lang 줄) + lang.rs narrow edit (`code_lang_for_path` 함수 + 관련 2 unit test + `use std::path::Path;` import 제거) + tests/{lang,skip}.rs 삭제 + 헤더 doc rewrite. atomic clippy gate 통과.
7. **doc 갱신 + workspace 회귀 + commit** — `kebab-core/src/metadata.rs:36` docstring + `docs/ARCHITECTURE.md` 산문 + frozen design §8 graph 두 줄 + workspace 회귀 + 1 clean commit. design §8 + ARCHITECTURE 갱신은 검증 cli 로 falsifiable acceptance.

핵심 ordering 보장:
- callsite migration (Step 4-6) 완료 전에 Cargo.toml dep 제거 (Step 7) 금지.
- source-fs callsite + Cargo.toml + integration test (Step 4-8) 완료 전에 parse-code 측 surface 삭제 (Step 9) 금지.
- parse-code 측 surface 삭제 (Step 9) 완료 전에 kebab-core docstring 정리 (Step 10 의 첫 action) 의미 없음.

## §2 Steps (10 steps)

### Step 1: Pre-flight baseline 측정 + env 확인

- **Files affected**: 변경 0 (측정 only).
- **Action**:
  - `cd /home/altair823/kebab && git rev-parse HEAD` → `b02ac82` 또는 그 위 commit 확인 (refactor branch 의 base).
  - env 확인: `echo $CARGO_TARGET_DIR` 가 `/build/out/...` 인지. 비어있으면 §0 의 export 적용.
  - baseline workspace test 수 측정 (PR description 의 "before N passing" 용). **awk 합산 cli 명시** (MINOR #1):
    ```sh
    cargo test --workspace --no-fail-fast -j 1 2>&1 \
      | awk '/^test result: ok\./ {for(i=1;i<=NF;i++) if($i=="passed;") sum += $(i-1)} END {print sum}' \
      > .omc/state/baseline_N.txt
    ```
    `.omc/state/baseline_N.txt` 에 N 값 한 줄 기록 (working dir 의 `.omc/` 는 git untracked — repo 에 들어가지 않음). PR description 에도 inline 인용 (primary record, local file 은 optional convenience).
  - 검증: `cargo tree -p kebab-source-fs | grep tree-sitter` → **non-zero 줄** (현재 9 grammar drag 존재 확인 = before-state baseline).
- **Spec reference**: §1.1, §5.2.
- **Exit gate**:
  - `git rev-parse HEAD` ≥ `b02ac82` (또는 동일).
  - baseline N 기록됨 (`.omc/state/baseline_N.txt` + PR description inline).
  - `cargo tree -p kebab-source-fs | grep tree-sitter | wc -l` ≥ 9 (before-state 확인).

### Step 2: 신규 `crates/kebab-source-fs/src/code_meta.rs` 생성

- **Files affected**: `crates/kebab-source-fs/src/code_meta.rs` (신규).
- **Action**:
  - 신규 file 작성. module-level doc 은 spec §3.3 의 cross-link wording 그대로 (5-line `//!` 블록 — "Pre-ingest classification ... `BUILTIN_BLACKLIST` is `pub` because ... `tests/code_meta.rs` ... 3 helper fns are `pub(crate)`").
  - `use std::fs::File; use std::io::{BufRead, BufReader, Read}; use std::path::Path; use anyhow::Result;` (kebab-parse-code/src/skip.rs:9-12 + lang.rs:7 의 use 절 합집합).
  - `pub const BUILTIN_BLACKLIST: &[&str] = &[...6 entry...];` — kebab-parse-code/src/skip.rs **line 17-24** 본문 byte-identical (MINOR #2 정정 — 17-24 가 실제 const 본문 line range), 단 doc 주석에 spec §3.4 의 "Source of truth: design §5.2 (frozen)" 줄 추가.
  - `pub(crate) fn code_lang_for_path(path: &Path) -> Option<&'static str>` — kebab-parse-code/src/lang.rs:17-66 본문 byte-identical (visibility 만 `pub` → `pub(crate)`).
  - `pub(crate) fn is_generated_file(path: &Path) -> Result<bool>` — kebab-parse-code/src/skip.rs:28-46 본문 byte-identical (visibility `pub` → `pub(crate)`).
  - `pub(crate) fn is_oversized(path: &Path, max_bytes: u64, max_lines: u32) -> Result<bool>` — kebab-parse-code/src/skip.rs:50-65 본문 byte-identical (visibility `pub` → `pub(crate)`).
  - `#[cfg(test)] mod tests { ... }` 블록 — 12 unit test 본문 이전 (spec §3.9 table).
  - **consolidated imports** (MINOR #4) — `#[cfg(test)] mod tests` 블록 최상단:
    ```rust
    #[cfg(test)] mod tests {
        use super::{is_generated_file, is_oversized, code_lang_for_path};
        use super::BUILTIN_BLACKLIST;          // unit tests 에는 미사용이나 import resolver 단순화 — 단, BLACKLIST 6-entry 검증은 integration test (§3.7) 로 분리되므로 본 import 는 실제로는 생략 가능. 본 plan 은 **포함 안 함** (false unused-import warn 회피).
        use std::fs;
        use std::path::Path;
        use tempfile::NamedTempFile;
        // ... 12 test fn ...
    }
    ```
    실제 적용: `use super::{is_generated_file, is_oversized, code_lang_for_path}; use std::fs; use std::path::Path; use tempfile::NamedTempFile;` — 4 줄. `BUILTIN_BLACKLIST` import 는 unit 측에서 미사용이므로 생략 (사용은 integration test 에서만).
  - 12 unit test mapping (spec §3.9):
    - `tests/lang.rs` 의 4 test (`known_extensions_map_to_canonical_identifiers`, `special_filenames_map_to_identifiers`, `unknown_extension_returns_none`, `case_insensitive`) — 본문 byte-identical, `use kebab_parse_code::code_lang_for_path;` 줄 제거 (consolidated import 가 대체).
    - `src/lang.rs::tests` 의 2 test (`tier2_basename_takes_precedence_over_extension`, `tier2_extension_fallback`) — 본문 byte-identical.
    - `tests/skip.rs` 의 6 test (`generated_header_markers_trigger_skip`, `normal_code_is_not_flagged_generated`, `is_generated_returns_false_for_empty_file`, `oversized_by_bytes_returns_true`, `oversized_by_lines_returns_true`, `small_file_returns_false_for_oversize`) — 본문 byte-identical, `use kebab_parse_code::skip::{...};` 줄 제거 + `use tempfile::NamedTempFile; use std::fs;` 는 consolidated import 로 대체.
    - **이전 안 함**: `builtin_blacklist_has_exactly_six_entries` (= Step 8 의 integration test 로 분리).
- **Spec reference**: §3.2, §3.3, §3.4, §3.7, §3.9.
- **Exit gate**:
  - file 신설됨. 이 step 만으로는 `lib.rs` 미등록 → 컴파일러 무시 (`cargo check -p kebab-source-fs -j 1` 변화 없음, untouched dead file). 검증 항목은 Step 3 에 위임.

### Step 3: `crates/kebab-source-fs/src/lib.rs` 에 module + pub use 등록

- **Files affected**: `crates/kebab-source-fs/src/lib.rs`.
- **Action** (spec §3.2):
  - 기존 `mod walker;` 다음 줄에 `mod code_meta;` 한 줄 추가.
  - 기존 `pub use connector::{FsScanSkips, FsSourceConnector};` 다음 줄에 `pub use code_meta::BUILTIN_BLACKLIST;` 한 줄 추가 (인라인 주석: `// design §5.2 frozen contract — integration test (§5.1) 의 접근 surface.`).
  - **변경 안 함**: 기존 `mod connector; mod hash; mod media; mod walker;` (NEW MAJOR #2 의 surface 무근거 확장 회피).
- **Spec reference**: §3.2.
- **Exit gate**:
  - `cargo check -p kebab-source-fs -j 1` 통과.
  - 이 시점: `crate::code_meta::*` + `kebab_parse_code::*` 양쪽 surface 공존.
  - `cargo test -p kebab-source-fs -j 1 code_meta::tests` → 12 passing (12 unit test 모두 통과).

### Step 4: `crates/kebab-source-fs/src/media.rs` callsite migration

- **Files affected**: `crates/kebab-source-fs/src/media.rs`.
- **Action** (spec §3.5 row 1):
  - line 17: `if let Some(lang) = kebab_parse_code::code_lang_for_path(path) {` → `if let Some(lang) = crate::code_meta::code_lang_for_path(path) {`
- **Spec reference**: §3.5.
- **Exit gate**:
  - `cargo check -p kebab-source-fs -j 1` clean (warn-free).
  - `cargo test -p kebab-source-fs -j 1 media` 통과.

### Step 5: `crates/kebab-source-fs/src/walker.rs` callsite migration + 주석 갱신

- **Files affected**: `crates/kebab-source-fs/src/walker.rs`.
- **Action** (spec §3.5 row 2-3 + comment row):
  - line 131: `for pat in kebab_parse_code::BUILTIN_BLACKLIST {` → `for pat in crate::code_meta::BUILTIN_BLACKLIST {`
  - line 211: 동일 패턴.
  - line 9 (module-level `//!` 주석): `kebab_parse_code::BUILTIN_BLACKLIST` → `crate::code_meta::BUILTIN_BLACKLIST`
  - line 85, 161 (function-level `///` 주석): 동일 패턴.
- **Spec reference**: §3.5.
- **Exit gate**:
  - `cargo check -p kebab-source-fs -j 1` clean.
  - `cargo test -p kebab-source-fs -j 1 walker` 통과.

### Step 6: `crates/kebab-source-fs/src/connector.rs` callsite migration

- **Files affected**: `crates/kebab-source-fs/src/connector.rs`.
- **Action** (spec §3.5 row 4-5):
  - line 152: `&& kebab_parse_code::is_generated_file(&abs_path).unwrap_or(false)` → `&& crate::code_meta::is_generated_file(&abs_path).unwrap_or(false)`
  - line 169: `if kebab_parse_code::is_oversized(` → `if crate::code_meta::is_oversized(`
- **Spec reference**: §3.5.
- **Exit gate**:
  - `cargo check -p kebab-source-fs -j 1` clean.
  - **추가 가드** (spec §6.2): `grep -rn "kebab_parse_code\|kebab-parse-code" crates/kebab-source-fs/src/ crates/kebab-source-fs/tests/` → 0 줄. (Cargo.toml 은 제외 — Step 7).

### Step 7: `crates/kebab-source-fs/Cargo.toml` 에서 `kebab-parse-code` dep 제거 — **anchor**

- **Files affected**: `crates/kebab-source-fs/Cargo.toml`.
- **Action** (spec §3.8 diff):
  - line 13 `kebab-parse-code = { path = "../kebab-parse-code" }` 한 줄 삭제.
  - **변경 안 함**: `kebab-core`, `kebab-config`, 기타 모든 dep + `[dev-dependencies]`.
- **Spec reference**: §3.8, G1, G5.
- **Exit gate** — 본 plan 의 **anchor step**, 4 검증 모두 통과 필수:
  - `cargo build -p kebab-source-fs -j 1` clean.
  - `cargo clippy -p kebab-source-fs --all-targets -j 1 -- -D warnings` clean (workspace pedantic 그대로).
  - `cargo test -p kebab-source-fs -j 1` 통과 (기존 integration test 3개 + Step 2 의 12 unit test).
  - `cargo tree -p kebab-source-fs | grep tree-sitter | wc -l` → **0 줄** (G5 + spec §5.3).
  - 이 step 통과 = **G1 (source-fs dep lightening) 달성 시점**.

### Step 8: 신규 `crates/kebab-source-fs/tests/code_meta.rs` integration test 생성

- **Files affected**: `crates/kebab-source-fs/tests/code_meta.rs` (신규).
- **Action** (spec §3.7, §3.9 의 integration row):
  - 신규 file:
    ```rust
    use kebab_source_fs::BUILTIN_BLACKLIST;

    #[test]
    fn builtin_blacklist_has_exactly_six_entries() {
        assert_eq!(BUILTIN_BLACKLIST.len(), 6);
        let expected = [
            "**/node_modules/**",
            "**/target/**",
            "**/__pycache__/**",
            "**/.venv/**",
            "**/venv/**",
            "**/env/**",
        ];
        for pat in expected {
            assert!(BUILTIN_BLACKLIST.contains(&pat), "missing pattern: {pat}");
        }
    }
    ```
  - `kebab-parse-code/tests/skip.rs:60-74` 의 본문을 import 만 갈아끼우고 byte-identical 이전.
- **Spec reference**: §3.7, §3.9, §5.1.
- **Exit gate**:
  - `cargo test -p kebab-source-fs -j 1 code_meta` → 13 passing (12 unit + 1 integration).
  - `cargo test -p kebab-source-fs --test code_meta -j 1` → 1 passing (`--test` flag 로 integration binary 만 선택, false-positive 회피).

### Step 9: `kebab-parse-code` 측 atomic cleanup — skip.rs 삭제 + lang.rs narrow edit + lib.rs (skip + lang) 재구성 + tests/{lang,skip}.rs 삭제 + 헤더 doc rewrite

- **Files affected**:
  - `crates/kebab-parse-code/src/skip.rs` (삭제).
  - `crates/kebab-parse-code/src/lang.rs` (narrow edit).
  - `crates/kebab-parse-code/src/lib.rs` (edit — skip 줄 + lang 줄 + 헤더 doc).
  - `crates/kebab-parse-code/tests/lang.rs` (삭제).
  - `crates/kebab-parse-code/tests/skip.rs` (삭제).
- **Action**:
  - **(a) `crates/kebab-parse-code/src/skip.rs` 파일 삭제** (spec §3.6 skip.rs 행).
  - **(b) `crates/kebab-parse-code/src/lang.rs` narrow edit** (spec §3.6 lang.rs 행 + BLOCKER #2):
    - **line 7 의 `use std::path::Path;` 삭제** (BLOCKER #2 — `code_lang_for_path` 가 유일한 consumer 였음. 보존되는 `module_path_for_python` / `module_path_for_tsjs` 둘 다 `workspace_path: &str` 인자, 보존되는 2 unit test 도 `Path::new(...)` 부재. 미삭제 시 `cargo clippy -- -D warnings` 의 `unused_imports` lint fail).
    - 함수 본문 `pub fn code_lang_for_path(path: &Path) -> Option<&'static str> { ... }` (line 17-66) 전체 삭제.
    - `#[cfg(test)] mod tests` 안의 `tier2_basename_takes_precedence_over_extension` (line 147-158) + `tier2_extension_fallback` (line 161-168) unit test 삭제.
    - **보존**: `pub fn module_path_for_python(...)` (line 77-103), `pub fn module_path_for_tsjs(...)` (line 107-115), `#[cfg(test)] mod tests` 안의 `module_path_for_python_strips_src_roots_and_extensions` (line 122-133), `module_path_for_tsjs_keeps_slashes_and_strips_ext` (line 136-144) — caller 는 본 crate 자체 (`python.rs:78`, `typescript.rs:88`, `javascript.rs:95`). (round 3 MINOR #1 — off-by-one 정정.)
    - 헤더 doc (line 1-5) 한 단락 rewrite (spec §3.6 MINOR #2; round 3 MINOR #2 — line 7 `use std::path::Path;` 는 별도 sub-bullet 의 삭제 대상이라 doc range 에서 제외):
      - 기존: `//! Canonical extension → language identifier mapping (spec §3.5).\n//!\n//! Lowercase canonical identifiers, matching tree-sitter parser conventions:\n//! \`rust\`, \`python\`, ...\n`
      - 신규: `//! Workspace-relative path → module-path conversion for P10-1B AST extractors (Python dotted form / TS+JS slash form). 본 module 의 \`code_lang_for_path\` 는 v0.18.0+ 부터 \`kebab-source-fs::code_meta\` 로 이동.`
  - **(c) `crates/kebab-parse-code/src/lib.rs` edit** (spec §3.6 lib.rs 행 전체):
    - `pub mod skip;` (line 27) 삭제.
    - `pub use lang::{code_lang_for_path, module_path_for_python, module_path_for_tsjs};` → `pub use lang::{module_path_for_python, module_path_for_tsjs};` (`code_lang_for_path` 제거).
    - `pub use skip::{BUILTIN_BLACKLIST, is_generated_file, is_oversized};` (line 40) 삭제.
    - 헤더 doc `//!` 단락 (line 1-14) rewrite (spec §3.6 MINOR #3):
      - 기존: `//! \`kebab-parse-code\` — language-aware parsing for code corpora.\n//!\n//! Phase 1A-1 ships infrastructure only:\n//! ... 4 bullet ... //!\n//! Per-language parser modules ...`
      - 신규: `//! \`kebab-parse-code\` — language-aware parsing for code corpora.\n//!\n//! Repo metadata (\`detect_repo\`) + per-language AST extractors (Rust = P10-1A-2, Python/TS/JS = P10-1B, Go = P10-1C-Go, Java+Kotlin = P10-1C-JK, C+C++ = P10-1D).\n//!\n//! lang detect (\`code_lang_for_path\`) + pre-ingest skip helpers (\`is_generated_file\`, \`is_oversized\`, \`BUILTIN_BLACKLIST\`) 는 v0.18.0+ 부터 \`kebab-source-fs::code_meta\` 로 이동 — refactor 2026-05-26.\n//!\n//! 본 crate 의 boundary 는 design §8 — store / embed / llm / rag / UI 의존 금지.`
  - **(d) `crates/kebab-parse-code/tests/lang.rs` 삭제** (4 test case 가 §3.9 매핑대로 Step 2 의 source-fs unit 으로 이미 이전됨).
  - **(e) `crates/kebab-parse-code/tests/skip.rs` 삭제** (7 test case 가 §3.9 매핑대로 Step 2 (6) + Step 8 (1) 으로 이미 이전됨).
  - **변경 안 함** (spec §3.6 + §3.8): `crates/kebab-parse-code/Cargo.toml` (CRITICAL #1 — `[dev-dependencies] tempfile` 는 `tests/repo.rs:4` 가 계속 소비), 9 grammar AST extractor file (`c.rs ~ rust.rs`), `repo.rs`, `scaffold.rs`, `tests/repo.rs`.
- **Spec reference**: §3.6 (전체), §6.5.
- **Exit gate** — atomic clippy gate (모든 sub-action 적용 후 단발 검증):
  - `cargo check -p kebab-parse-code -j 1` clean.
  - `cargo clippy -p kebab-parse-code --all-targets -j 1 -- -D warnings` clean (Path import + 함수 + unit test + tests/{lang,skip}.rs + lib.rs export 모두 동시 정리되어 unused-import / dead-code lint 0).
  - `cargo test -p kebab-parse-code -j 1` 통과 (module_path_for_* + AST extractor + repo + 9 grammar 보존).
  - **§6.5 sibling 안전망**: `cargo test -p kebab-parse-code -j 1 module_path_for_` → 2 passing (`module_path_for_python_strips_src_roots_and_extensions` + `module_path_for_tsjs_keeps_slashes_and_strips_ext`).
  - **추가 가드 — lib.rs 의 skip module 등록 + re-export 0 건** (MAJOR #2 명료화):
    ```sh
    grep -nE '^pub mod skip|^pub use skip' crates/kebab-parse-code/src/lib.rs | wc -l
    ```
    → **0**. 헤더 doc 산문 내 단어 "skip" 은 미터치 (의미 보존). 본 정규식은 declaration / re-export 만 잡고 산문 미스매치.
  - **추가 가드 — lang.rs 의 code_lang_for_path 부재**:
    ```sh
    grep -nE '^pub fn code_lang_for_path|^use std::path::Path' crates/kebab-parse-code/src/lang.rs | wc -l
    ```
    → **0**. 함수 정의 + Path import 둘 다 사라졌는지 확인.
  - **sub-action 가시성 가드** (round 3 optional GAP #2 — 5 sub-action atomic 의 partial-apply 시 어느 sub-action 빠졌는지 clippy 결과 추적 전에 즉시 가시):
    ```sh
    test ! -f crates/kebab-parse-code/src/skip.rs                                       # (a) skip.rs 삭제
    ! grep -qE '^pub fn code_lang_for_path' crates/kebab-parse-code/src/lang.rs         # (b1) 함수 본문 제거
    ! grep -qE '^use std::path::Path' crates/kebab-parse-code/src/lang.rs               # (b2) Path import 제거 (BLOCKER #2)
    ! grep -qE '^pub mod skip|^pub use skip|^pub use lang::.*code_lang_for_path' crates/kebab-parse-code/src/lib.rs   # (c) skip/lang code_lang_for_path 줄 부재 — round 4 CRITICAL #1: 세 번째 alternative `^pub use lang::.*` anchor 로 한정, 새 헤더 doc 의 backtick 산문 `code_lang_for_path` 산문 매치 회피
    test ! -f crates/kebab-parse-code/tests/lang.rs                                     # (d) tests/lang.rs 삭제
    test ! -f crates/kebab-parse-code/tests/skip.rs                                     # (e) tests/skip.rs 삭제
    ```
    여섯 줄 모두 exit 0. 한 줄이라도 fail → partial-apply 진단 (clippy 결과 분석 전에 어느 sub-action 빠졌는지 즉시 식별).

### Step 10: `kebab-core` docstring + `ARCHITECTURE.md` + 설계 §8 graph 갱신 + workspace 회귀 + 1 clean commit — **closure**

- **Files affected**:
  - `crates/kebab-core/src/metadata.rs` (line 36 doc 한 줄).
  - `docs/ARCHITECTURE.md` (산문 한 줄 추가).
  - `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md` (§8 graph 두 줄 — edge 제거 (a) + inline note 추가 (b)).
- **Action 1 — `kebab-core/src/metadata.rs:36` docstring 정리** (spec §3.5 comment-only row, MINOR #5 honest wording):
  - 기존: `Set by kebab_parse_code::lang::code_lang_for_path.`
  - 신규: `Set by the local-filesystem source connector during ingest.`
  - **Rationale (honest wording, MINOR #5)**: backtick inline code 는 rustdoc 자동 intra-doc-link 처리 대상 아님 (대괄호 부재). 또한 `kebab-core` 는 `kebab-parse-code` 의존 0 (design §8 — kebab-core 는 도메인 타입만, 다른 kebab-* crate 미참조) — cross-crate resolution 시도조차 안 됨. 따라서 본 edit 의 목적은 "broken intra-doc link 회피" 가 아니라 **stale dependency reference 제거** (design §8 cross-crate forbidden 룰 정합 — 변경 후의 surface 위치를 정확히 반영).
- **Action 2 — `docs/ARCHITECTURE.md` 산문 갱신** (spec §7 row "docs/ARCHITECTURE.md"):
  - line 134 단락 (현재 wording: `'kebab-parse-code' 의 외부 tree-sitter grammar crate 의존: P10-1A-2 에서 'tree-sitter-rust' 추가, ...`) 끝에 한 줄 추가:
    - `v0.18.0+ 부터 'kebab-source-fs' 는 자체 'code_meta' 모듈 (lang detect + skip helpers + BUILTIN_BLACKLIST) 을 보유, 'kebab-parse-code' 와 분리 (refactor 2026-05-26).`
  - Mermaid 변경 0 (`srcfs → pcode` arrow 부재).
- **Action 3 — frozen design §8 graph 갱신** (spec §7 row "design §8 graph" + MAJOR #4):
  - frozen design 의 line 1460-1461 block:
    ```text
    ├─> kebab-source-fs
    │     └─> kebab-parse-code (p10-1A-1: lang detect / repo detect / skip policy)
    ```
    →
    ```text
    ├─> kebab-source-fs
    │     (p10-2 이후: lang detect + skip policy 내장; kebab-parse-code 와 분리)
    ```
  - 즉: (a) edge 한 줄 제거, (b) inline note 한 줄 추가. Sibling row `├─> kebab-parse-code\n│     └─> kebab-core ...` (line 1464-1465) 는 그대로 — `kebab-parse-code` 가 워크스페이스 의 별도 crate 로 계속 존재.
- **Action 4 — 변경 0 명시** (spec §7 cross-check):
  - `README.md` / `HANDOFF.md` / `tasks/HOTFIXES.md` / `tasks/INDEX.md` 변경 0.
  - `tasks/p10/p10-1a-1-code-ingest-framework.md`, `tasks/p10/p10-2-tier2-resource-aware.md`, `docs/superpowers/plans/2026-05-15-p10-1a-1-code-ingest-framework.md`, `docs/superpowers/plans/2026-05-20-p10-2-tier2-resource-aware.md` — frozen 보존 (spec §1.6 + §6.6 — "may" reference 는 contract violation 0).
  - workspace `Cargo.toml` `version` bump 0. 양 crate `[features]` 변경 0.
- **Action 5 — workspace 회귀** (acceptance):
  ```sh
  cargo clippy --workspace --all-targets -j 1 -- -D warnings
  cargo test --workspace --no-fail-fast -j 1 2>&1 \
    | awk '/^test result: ok\./ {for(i=1;i<=NF;i++) if($i=="passed;") sum += $(i-1)} END {print sum}'
  cargo test -p kebab-app --test code_ingest_smoke -j 1   # BLOCKER #1 정정 — `--test` 강제
  RUSTDOCFLAGS="-D rustdoc::broken-intra-doc-links" cargo doc -p kebab-core --no-deps -j 1   # MINOR #3 — flag 강제
  cargo build --release -j 1
  cargo tree -p kebab-source-fs | grep tree-sitter | wc -l   # → 0
  ```
- **Action 6 — design §8 + ARCHITECTURE 갱신 검증** (MAJOR #3 — falsifiable acceptance, 3 idempotent grep):
  ```sh
  # (i) 옛 tree-edge 부재 확인 — 'kebab-source-fs └─> kebab-parse-code (p10-1A-1: ...)' 구문이 사라졌는지
  #     (round 3 CRITICAL #1 — 산문 inline note 의 'kebab-parse-code 와 분리' 와 syntactic 구분 위해 tree-edge format 까지 anchor)
  ! grep -qE '└─>\s*kebab-parse-code\s*\(p10-1A-1' \
      docs/superpowers/specs/2026-04-27-kebab-final-form-design.md

  # (ii) inline note 추가 확인
  test "$(grep -c 'p10-2 이후: lang detect + skip policy 내장' \
            docs/superpowers/specs/2026-04-27-kebab-final-form-design.md)" -ge 1

  # (iii) ARCHITECTURE.md 산문 한 줄 확인
  test "$(grep -c 'kebab-source-fs.*code_meta.*kebab-parse-code 와 분리' docs/ARCHITECTURE.md)" -ge 1
  ```
  세 줄 모두 통과해야 acceptance 충족.
- **Action 7 — 1 clean commit** (spec §5 + plan §5):
  - commit message draft (한국어, spec §5.2 의 baseline N 인용):
    ```text
    refactor(source-fs): drop kebab-parse-code dep — extract code_meta module

    Move 4 surface (BUILTIN_BLACKLIST + 3 helper fn) from kebab-parse-code
    into kebab-source-fs::code_meta. Drops 9 tree-sitter grammar drag from
    source-fs's dep tree (cargo tree -p kebab-source-fs | grep tree-sitter
    → 0 lines).

    Visibility 정책 (mixed):
      - BUILTIN_BLACKLIST: pub (design §5.2 frozen contract — integration
        test 의 외부 검증 surface)
      - 3 helper fn: pub(crate) (source-fs 내부 호출만)

    Test 이전: 12 unit (src/code_meta.rs::tests) + 1 integration
    (tests/code_meta.rs::builtin_blacklist_has_exactly_six_entries).
    kebab-parse-code 의 module_path_for_python / module_path_for_tsjs 와
    그 2 unit test 는 보존 (sibling caller = python.rs / typescript.rs /
    javascript.rs).

    Spec: docs/superpowers/specs/2026-05-26-source-fs-dep-lightening-spec.md
    Design §8 graph: edge 'kebab-source-fs → kebab-parse-code' 제거 +
    inline note 추가.

    Verification:
      - cargo test --workspace --no-fail-fast -j 1 → baseline N maintained
      - cargo test -p kebab-app --test code_ingest_smoke -j 1 → pass
      - cargo clippy --workspace --all-targets -j 1 -- -D warnings → clean
      - cargo tree -p kebab-source-fs | grep tree-sitter → 0 lines
      - workspace.version bump 0, wire schema impact 0, V00X 0.
    ```
- **Spec reference**: §3.5 (comment-only), §5.1, §5.2, §5.3, §6.5, §7.
- **Exit gate** — plan exit gate 와 동일 (acceptance):
  - Action 5 의 6 cli 모두 통과.
  - Action 6 의 3 idempotent grep 모두 통과.
  - 1 commit on `refactor/source-fs-dep-lightening` branch.

## §3 Step dependency graph

```text
Step 1  (baseline + env)
  ↓
Step 2  (code_meta.rs 신설 — dead file 까지)
  ↓
Step 3  (lib.rs mod + pub use — 양쪽 surface 공존)
  ↓
Step 4  (media.rs callsite — 1 곳)
  ↓
Step 5  (walker.rs callsite — 2 곳 + 주석 3)
  ↓
Step 6  (connector.rs callsite — 2 곳)
  ↓
Step 7  (Cargo.toml dep 제거)  ← **anchor: G1 + G5 달성, source-fs 측 완료**
  ↓
Step 8  (integration test 신설)
  ↓
Step 9  (parse-code atomic cleanup — skip.rs 삭제 + lang.rs narrow + lib.rs + tests 삭제 + 헤더 doc)
  ↓
Step 10 (kebab-core doc + ARCHITECTURE + design §8 + workspace 회귀 + commit)  ← **acceptance: G2/G3/G4 달성, plan complete** (NIT #1)
```

**Linear chain — 모든 step 직렬, parallelism 0.** 근거:

- Step 3 가 Step 2 의 file 존재 전제.
- Step 4-6 의 각 callsite migration 은 Step 3 의 surface 등록 전제. 순서는 무관하지만 (file 별 독립) plan checklist 의 추적 단순성을 위해 linear.
- Step 7 (Cargo.toml dep 제거) 는 Step 4-6 의 모든 callsite migration 완료 전제. 그렇지 않으면 cargo build fail.
- Step 8 (integration test) 가 Step 7 의 `pub use code_meta::BUILTIN_BLACKLIST;` (= Step 3 에서 등록) + source-fs 의 parse-code 무의존 (= Step 7) 양쪽 전제.
- Step 9 (parse-code 측 surface 삭제) 는 Step 7 + Step 8 의 source-fs 측 완료 후만 안전. atomic clippy gate — skip + lang + Path import + tests 모두 동시 정리되어 `cargo clippy -p kebab-parse-code -- -D warnings` 가 한 번에 통과.
- Step 10 (closure) 가 Step 9 의 `pub use lang::code_lang_for_path` 제거 후. design §8 graph + ARCHITECTURE 갱신 + workspace 회귀 + commit 의 단일 closure step.

## §4 Verification gate (acceptance)

Plan exit gate = spec §5 + Step 9 의 atomic gate + Step 10 의 acceptance gate.

### §4.1 Source-fs 측 (spec §5.1) — Step 8 시점 통과 확인

```sh
cargo test -p kebab-source-fs -j 1 code_meta
cargo test -p kebab-source-fs --test code_meta -j 1   # integration binary 단독 검증
```

기대: 13 passing (12 unit + 1 integration) + 1 passing (integration 단독).

### §4.2 Workspace 회귀 (spec §5.2) — Step 10 시점 통과 확인

```sh
cargo test --workspace --no-fail-fast -j 1 2>&1 \
  | awk '/^test result: ok\./ {for(i=1;i<=NF;i++) if($i=="passed;") sum += $(i-1)} END {print sum}'
cargo test -p kebab-app --test code_ingest_smoke -j 1   # BLOCKER #1 — --test flag 강제
```

기대:
- workspace test sum: Step 1 의 baseline N 과 동일 (회귀 0).
- code_ingest_smoke: `--test code_ingest_smoke` 가 16+ fn 모두 실행 (substring filter 가 아니라 binary 선택). `test result: ok. N passed; 0 failed` 의 N ≥ 16 확인.

### §4.3 Clippy + build + dep tree (spec §5.3) — Step 10 시점 통과 확인

```sh
cargo clippy --workspace --all-targets -j 1 -- -D warnings
cargo build --release -j 1
cargo tree -p kebab-source-fs | grep tree-sitter | wc -l
RUSTDOCFLAGS="-D rustdoc::broken-intra-doc-links" cargo doc -p kebab-core --no-deps -j 1   # MINOR #3
```

기대:
- clippy: clean.
- build: clean release binary.
- `cargo tree` grep: **0 줄** (G5 final acceptance).
- `cargo doc`: 0 broken intra-doc link.

### §4.4 Design §8 + ARCHITECTURE 갱신 acceptance (MAJOR #3) — Step 10 시점 통과 확인

```sh
# (i) 옛 tree-edge 부재 확인 — round 3 CRITICAL #1 정정 (산문 inline note 와 syntactic 구분)
! grep -qE '└─>\s*kebab-parse-code\s*\(p10-1A-1' \
    docs/superpowers/specs/2026-04-27-kebab-final-form-design.md

# (ii) inline note 추가 확인
test "$(grep -c 'p10-2 이후: lang detect + skip policy 내장' \
          docs/superpowers/specs/2026-04-27-kebab-final-form-design.md)" -ge 1

# (iii) ARCHITECTURE.md 산문 한 줄 확인
test "$(grep -c 'kebab-source-fs.*code_meta.*kebab-parse-code 와 분리' docs/ARCHITECTURE.md)" -ge 1
```

세 줄 모두 통과해야 G4 acceptance 충족.

### §4.5 Optional informational only (spec §5.4) — acceptance 가 아님

PR description 에 부기 가능. plan exit gate 에 포함 안 함 (MINOR #4 — `informational only`).

## §5 Commit strategy

**1 clean commit** 권장 — 본 refactor 는 internal-only + 10 step 이 모두 Step 10 의 verification gate 한 묶음으로 묶임. step-별 atomic commit 으로 쪼개면 중간 commit 이 cargo build 깨진 상태 (예: Step 5 후 Step 6 전) 거나 step 별 의미 단편이라 review 가치 낮음.

Commit message draft = §2 Step 10 Action 7 의 draft 그대로 유지 (~30 줄, substantive surface — round 2 open-question 답변 4 의 권장).

**push / PR 생성 0** — team-lead 책임.

## §6 Risks + mitigation

### §6.1 중간 단계 cargo build 깨짐 (step ordering 깨짐)

- **Risk**: Step 4-6 의 callsite migration 중 한 file 만 migrate 하고 Step 7 (Cargo.toml dep 제거) 로 점프하면 다른 file 의 `kebab_parse_code::*` 가 unresolved → cargo build fail.
- **Mitigation**: 각 step 의 exit gate 가 `cargo check -p kebab-source-fs -j 1` 통과 강제. Step 6 의 exit gate 의 보조 grep — `kebab_parse_code` 잔여 0 확인.

### §6.2 Step 9 의 atomic clippy gate — partial-apply 위험 (BLOCKER #2 + MAJOR #2)

- **Risk**: Step 9 의 5 sub-action 중 일부만 적용하면:
  - skip.rs 파일 남기고 lib.rs 만 제거 → `unused file` (warn 0, but stale).
  - lang.rs 의 `code_lang_for_path` 함수만 지우고 `use std::path::Path;` 보존 → `unused_imports` warn → clippy `-D warnings` fail.
  - lib.rs 의 `pub use lang::code_lang_for_path` 보존 채로 lang.rs 함수 본문만 삭제 → cargo check 단계 unresolved-name fail.
  - 헤더 doc 의 산문 내 "skip" 단어 미터치 (= 의도) 인데, exit gate 의 grep 패턴이 산문까지 매치하면 (= `grep -n "skip"`) self-contradiction.
- **Mitigation**:
  - Step 9 가 **atomic step** — 5 sub-action 모두 적용 후만 exit gate 검증.
  - Step 9 exit gate 의 grep 패턴이 **declaration / re-export 전용 정규식**: `grep -nE '^pub mod skip|^pub use skip'`. 산문 미스매치 (MAJOR #2 명료화).
  - BLOCKER #2 의 Path import 삭제는 Step 9 (b) 의 첫 줄로 명시.

### §6.3 Sibling `module_path_for_*` 의 accidental drop (CRITICAL #2)

- **Risk**: Step 9 의 lang.rs narrow edit 시 `module_path_for_python` / `module_path_for_tsjs` 함수 또는 그 unit test 를 같이 지움 → P10-1B AST extractor (`python.rs:78`, `typescript.rs:88`, `javascript.rs:95`) 가 compile fail 또는 e2e fixture 가 runtime fail.
- **Mitigation**:
  - Step 9 의 exit gate: `cargo test -p kebab-parse-code -j 1 module_path_for_` → 2 passing 명시.
  - Step 10 Action 5 의 workspace 회귀: `cargo test -p kebab-app --test code_ingest_smoke -j 1` 명시 (가장 강한 안전망, BLOCKER #1 정정 후).
  - 2 단 cover.

### §6.4 `kebab-core::metadata.rs` stale reference

- **Risk**: Step 9 가 `pub use lang::code_lang_for_path` 를 제거하면 `kebab-core/src/metadata.rs:36` 의 backtick inline code `` `kebab_parse_code::lang::code_lang_for_path` `` 가 stale (실제 path 미존재).
- **Rationale 정정 (MINOR #5)**: backtick inline code 는 rustdoc 자동 intra-doc-link 처리 대상 아님 (대괄호 부재) + `kebab-core` 가 `kebab-parse-code` 의존 0 → cross-crate resolution 시도 0. 따라서 "broken intra-doc link 회피" 가 아니라 **stale dependency reference 제거** (design §8 cross-crate forbidden 룰 정합).
- **Mitigation**: Step 10 Action 1 의 doc rewrite — abstract wording ("Set by the local-filesystem source connector during ingest").
- **추가 가드**: Step 10 Action 5 의 `RUSTDOCFLAGS="-D rustdoc::broken-intra-doc-links" cargo doc -p kebab-core --no-deps -j 1` — 만약 향후 누군가 대괄호 link 로 다시 wrapping 하더라도 catch (MINOR #3 — flag 강제).

### §6.5 4 surface 외 hidden callsite (spec §6.2)

- **Risk**: 어떤 file 이 `kebab_parse_code::skip::BUILTIN_BLACKLIST` 같은 풀 path 또는 alias / re-export 로 4 surface 를 우회 호출.
- **Mitigation**:
  - spec §1.5 의 grep 결과 (NIT #1 보강) — 외부 명시 path consumer 0 확정.
  - Step 6 의 추가 가드 grep — source-fs 측 잔여 0 확인.
  - Step 9 후 추가 가드: `grep -rn "kebab_parse_code::skip\|kebab_parse_code::lang::code_lang" crates/ --include="*.rs"` → 0 줄 (parse-code 자체 test file 도 Step 9 에서 삭제됨).

### §6.6 cargo `-j 1` 미준수 시 OOM

- **Risk**: workspace test (Step 10 의 `cargo test --workspace`) 시 18 integration-test binary 동시 link → linker SIGKILL (CLAUDE.md "Build / test / lint" 문서화된 패턴). 본 plan 의 Step 8 이후 19 integration-test binary 가 됨 (`kebab-source-fs/tests/code_meta.rs` 추가).
- **Note (NIT #2)**: source-fs 는 lance / datafusion 무링크 → 추가 1 개 binary 의 link cost 증분 단발적 + RAM peak 영향 0. 본 plan 의 `-j 1` 룰 자체와 무관 (lance / datafusion 합산 link 폭주 vs 단일 lightweight binary).
- **Mitigation**: 모든 cargo workspace 명령 `-j 1` 명시. plan §0 의 env 룰 강조.

### §6.7 design §8 graph 갱신 형식 misread

- **Risk**: Step 10 의 frozen design §8 graph 두 줄 갱신 시 다른 row 영향을 줄 수 있음 (예: `kebab-app` 의 sibling row).
- **Mitigation**:
  - Step 10 Action 3 의 정확한 before/after block 인용. line range 1460-1461 만 변경, 다른 row 변경 0.
  - Step 10 Action 6 의 3 idempotent grep (MAJOR #3) — falsifiable acceptance.

## §7 Out of scope (plan-level)

Spec §8 (out of scope) 전부 + plan-level 추가:

- `kebab-parse-code` 의 9 tree-sitter grammar feature gating / dynamic loading — v0.19+ candidate.
- `kebab-parse-code/src/repo.rs` ownership 검토.
- `kebab-core::media.rs` 와 `kebab-source-fs::code_meta` 의 medium-vs-lang detection 통합 (Lens 3).
- `kebab-chunk` Tier 2 helper 정리 (Lens 2).
- `kebab-normalize` 흡수, `kebab-parse-types` 추가 정리 (Lens 1 다른 묶음).
- `deny.toml` 신설 / cargo-deny CI 도입 (spec §4.6 + §6.7 — design §8 의 미래 state).
- `tasks/INDEX.md` doc-sync (spec §1.6 — INDEX.md 가 stale 한 P10 phase status 표시. 별도 PR).

**HOTFIXES.md 갱신 0** (spec §7 명시 — design §8 자체를 same-PR 로 갱신하므로 frozen vs ship deviation 0, CLAUDE.md HOTFIXES rule 미트리거).

## §8 References

- Spec: `docs/superpowers/specs/2026-05-26-source-fs-dep-lightening-spec.md` (v3, 623 lines, Round 1+2+3 critic APPROVE + round 2 reflection 시 §5.2/§6.5 cli micro-patch 동반).
- Frozen design: `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md` §3.5, §3.7b, §5.2, §8 (graph block line 1455-1478).
- `docs/ARCHITECTURE.md` line 134 단락 — same-PR 갱신 target.
- Frozen task spec 보존 (spec §1.6, §6.6 분석 결과):
  - `tasks/p10/p10-1a-1-code-ingest-framework.md` line 23 ("may" reference, contract violation 0).
  - `tasks/p10/p10-2-tier2-resource-aware.md`.
  - `docs/superpowers/plans/2026-05-15-p10-1a-1-code-ingest-framework.md`.
  - `docs/superpowers/plans/2026-05-20-p10-2-tier2-resource-aware.md`.
- Sibling caller evidence (CRITICAL #2 안전망):
  - `crates/kebab-parse-code/src/python.rs:78`.
  - `crates/kebab-parse-code/src/typescript.rs:88`.
  - `crates/kebab-parse-code/src/javascript.rs:95`.
  - `crates/kebab-app/tests/code_ingest_smoke.rs:165, 242, 319`.
- Baseline evidence (CRITICAL #1):
  - `crates/kebab-parse-code/tests/repo.rs:4` — `tempfile::TempDir` 사용 (dev-dep 보존 정당화).
- BLOCKER #1 evidence (verifier-plan round 1):
  - `crates/kebab-app/tests/code_ingest_smoke.rs` 의 16+ `#[test]` fn 중 substring `code_ingest_smoke` 0건 — bare `cargo test -p kebab-app code_ingest_smoke` 는 false-positive PASS (0 tests run + exit 0). `--test code_ingest_smoke` 강제.

## §9 Round 2 closure status

| Finding | Severity | 반영 | 위치 |
|---------|----------|------|------|
| **BLOCKER #1** `cargo test code_ingest_smoke` substring filter false-positive | BLOCKER | **reflected** | spec §5.2 + §6.5 cli 정정 (`--test code_ingest_smoke`), plan Step 10 Action 5, plan §4.2, plan §6.3 mitigation. |
| **BLOCKER #2** lang.rs `use std::path::Path;` dead import → clippy fail | BLOCKER | **reflected** | plan Step 9 (b) 의 첫 줄 명시, plan §6.2 mitigation, plan §8 references (BLOCKER #2 evidence). |
| **MAJOR #2** Step 9 grep "skip" 가드 자가 모순 | MAJOR | **reflected** (option a — declaration-only regex) | plan Step 9 exit gate 의 `grep -nE '^pub mod skip\|^pub use skip'` 패턴 명시, plan §6.2 mitigation. |
| **MAJOR #3** design §8 갱신 검증 acceptance 누락 | MAJOR | **reflected** | plan Step 10 Action 6 의 3 idempotent grep, plan §4.4 falsifiable acceptance, plan §6.7 mitigation. |
| **MINOR #1** baseline N awk cli 미명시 | MINOR | **reflected** | plan Step 1 Action 의 awk one-liner, plan §4.2 동일 cli 재인용. |
| **MINOR #2** skip.rs line range 14-24 → 17-24 | MINOR | **reflected** | plan Step 2 Action — `skip.rs **line 17-24**` 정정. |
| **MINOR #3** rustdoc broken-intra-doc-links flag 강제 | MINOR | **reflected** | plan Step 10 Action 5 의 `RUSTDOCFLAGS="-D rustdoc::broken-intra-doc-links"`, plan §4.3 동일. |
| **MINOR #4** Step 2 consolidated imports 불명확 | MINOR | **reflected** | plan Step 2 Action 의 4 줄 import 블록 명시. |
| **MINOR #5** Step 11 rustdoc broken-link rationale 의심 | MINOR | **reflected** (honest wording) | plan Step 10 Action 1 Rationale 단락 — "stale dependency reference 제거 (design §8 cross-crate forbidden)", plan §6.4 동일 rationale. |
| **MINOR #6** Step 9+10 합치기 + Step 11→12 흡수 (10 step) | MINOR | **reflected** | plan 전체 — 12 → 10 step. Step 9 atomic clippy gate, Step 10 closure step. |
| **NIT #1** §3 dep graph 에 Step 10 acceptance annotation | NIT | **reflected** | plan §3 — `Step 10 ... ← **acceptance: G2/G3/G4 달성, plan complete**` annotation. |
| **NIT #2** §6.6 link cost rationale 보강 | NIT | **reflected** | plan §6.6 — "source-fs 는 lance / datafusion 무링크 → 추가 1 개 binary 의 link cost 증분 단발적 + RAM peak 영향 0" 한 줄 inline. |

**Round 2 closure summary**: 2 BLOCKER + 1 MAJOR (BLOCKER #2 = critic MAJOR #1 dedup) + 2 MAJOR (verifier-plan Gap #2/#3) + 6 MINOR + 2 NIT = **13 finding 모두 reflected**, rejection 0. Spec micro-patch 2 곳 동반 (spec §5.2 + §6.5, cli 1 줄씩, round 1-3 closure 영향 0).

### §9.1 Spec micro-patch summary (round 2)

| Spec section | Edit | Rationale |
|--------------|------|-----------|
| §5.2 (line 467) | `cargo test -p kebab-app code_ingest_smoke -j 1` → `cargo test -p kebab-app --test code_ingest_smoke -j 1` + 산문 한 단락 (false-positive 회피 근거) | BLOCKER #1 |
| §6.5 (정정된 안전망 두 번째 cli) | `cargo test -p kebab-app code_ingest_smoke -j 1` → `cargo test -p kebab-app --test code_ingest_smoke -j 1` + 인라인 cross-link "(`--test` flag 강제 — verifier-plan round 1 Gap #1)" | BLOCKER #1 |

두 edit 모두 wording-only — spec round 1-3 closure status table (§10, §10.1) 의 finding-to-edit 매핑에 영향 0. spec round 4 critic 진입 불요 (verifier-plan round 1 의 Gap #1 가 cli precision 정정이므로 plan reflection 의 부수 작업).

### §9.2 Round 3 closure status

| Finding | Severity | 반영 | 위치 |
|---------|----------|------|------|
| **NEW CRITICAL #1** Step 10 Action 6 (i) grep self-contradictory — 'kebab-source-fs' 줄 다음 prose 의 'kebab-parse-code 와 분리' substring 이 grep 에 매치 → 올바른 edit 적용 후에도 영구 FAIL | CRITICAL | **reflected** | plan Step 10 Action 6 의 (i) 와 plan §4.4 의 (i) 두 곳 모두 `! grep -qE '└─>\s*kebab-parse-code\s*\(p10-1A-1' ...` 로 교체 — tree-edge format anchor 가 산문과 syntactic 구분. plan §6.7 mitigation cross-link 도 본 정정이 자동 cover (mitigation 본문이 "정확한 before/after block + 3 idempotent grep" wording 만 사용, 본 정정 후에도 의미 보존). |
| **NEW MINOR #1** lang.rs 보존 unit test line range off-by-one | MINOR | **reflected** | plan Step 9 (b) 보존 sub-bullet — `(line 121-133)` → `(line 122-133)`, `(line 135-144)` → `(line 136-144)` + 정정 근거 inline. |
| **NEW MINOR #2** lang.rs 헤더 doc rewrite "line 1-7" 표기 | MINOR | **reflected** | plan Step 9 (b) 의 헤더 doc sub-bullet — `(line 1-7)` → `(line 1-5)` + line 7 의 Path import 는 별도 sub-bullet 소관임을 명시. |
| **NEW NIT #1** double-space cosmetic | NIT | **reflected** | plan 본문 4 위치 (line 266 `cargo test  --workspace`, line 268 `cargo test  -p kebab-app`, line 271 `cargo tree  -p kebab-source-fs`, line 388 동일) — `replace_all` 로 single-space normalize. bash 무영향. |
| **NEW (Optional) GAP #2** Step 9 sub-action 가시성 보강 | NIT (optional, low-severity) | **reflected** (적용) | plan Step 9 exit gate 끝에 "sub-action 가시성 가드" 6 줄 추가 — `test ! -f skip.rs`, lang.rs Path import + 함수 부재, lib.rs 의 skip + code_lang_for_path 부재 (third alternative 를 `^pub use lang::.*` anchor 로 한정 — round 4 critic-plan 보강, 새 헤더 doc breadcrumb 산문 매치 회피), tests/{lang,skip}.rs 부재. partial-apply 시 clippy 결과 분석 전에 어느 sub-action 빠졌는지 직접 식별. |

**Round 3 closure summary**: 1 NEW CRITICAL + 2 NEW MINOR + 1 NEW NIT + 1 NEW OPTIONAL = **5 finding 모두 reflected**, rejection 0. spec edit 0 (round 3 의 모든 정정은 plan 본문 단독). round 2 closure (13 row) 영향 0 — round 3 정정은 round 2 가 확립한 atomic structure 위에 cli precision 만 보강.

### §9.3 Round 4 closure status

| Finding | Severity | 반영 | 위치 |
|---------|----------|------|------|
| **NEW CRITICAL #1** Step 9 sub-action 가드 (c) 줄의 세 번째 alternative `code_lang_for_path` 가 anchor 부재 → 새 헤더 doc 의 backtick 산문 `` `code_lang_for_path` `` 매치 → gate 영구 false-FAIL (round 2 MAJOR #2 와 동일 class — 산문 substring × unanchored regex) | CRITICAL | **reflected** | plan Step 9 exit gate 의 sub-action 가시성 가드 (c) 줄 — `code_lang_for_path` → `^pub use lang::.*code_lang_for_path` (re-export 라인만 매치, doc comment 산문 미터치). §9.2 GAP #2 row 의 wording 도 round 4 보강 inline cross-link. |

**Round 4 closure summary**: 1 NEW CRITICAL = **1 finding reflected**, rejection 0. spec edit 0, plan 변경 line ≤ 2 (regex 1 줄 + §9.2 wording 한 alternative 추가). round 1-3 closure 영향 0 (anchored alternative 의 추가는 기존 sub-action 의 부재 검증 의미 동일, 새 산문 매치 회피만 보강).
