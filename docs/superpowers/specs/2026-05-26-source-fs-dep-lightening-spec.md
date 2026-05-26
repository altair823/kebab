---
status: drafting
target_version: 0.18.0    # 0.18.0 release 의 후속 internal-refactor PR — workspace.version bump 없음 (§7 NG5 + CLAUDE.md §Release 룰 3 트리거 미충족).
contract_sections: ["§3.5 (MediaType::Code dispatch)", "§3.7b (parser intermediate boundary)", "§5.2 (ingest skip policy)", "§8 (allowed deps)"]
related_specs:
  - docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
  - tasks/p10/p10-1a-1-code-ingest-framework.md   # frozen — "Source-fs *may* depend on kebab-parse-code" (line 23) 의 `may` 가 의무 아니므로 본 refactor 가 task spec contract 침범 0.
---

# kebab-source-fs dep lightening — 9 tree-sitter grammars drag 제거

## §1 Background + evidence chain

### §1.1 현재 의존 그래프

`crates/kebab-source-fs/Cargo.toml` (현재 HEAD, refactor/source-fs-dep-lightening branch base = b02ac82) — `[dependencies]` 인용:

```toml
[dependencies]
kebab-core = { path = "../kebab-core" }
kebab-config = { path = "../kebab-config" }
kebab-parse-code = { path = "../kebab-parse-code" }   # ← 본 spec 의 제거 대상
anyhow       = { workspace = true }
serde        = { workspace = true }
...
```

`crates/kebab-parse-code/Cargo.toml` — 본 의존이 transitively 끌어오는 무게:

```toml
[dependencies]
kebab-core             = { path = "../kebab-core" }
anyhow                 = { workspace = true }
gix                    = { workspace = true }
serde_json             = { workspace = true }
time                   = { workspace = true }
tracing                = { workspace = true }
tree-sitter            = { workspace = true }
tree-sitter-rust       = { workspace = true }
tree-sitter-python     = { workspace = true }
tree-sitter-typescript = { workspace = true }
tree-sitter-javascript = { workspace = true }
tree-sitter-go         = { workspace = true }
tree-sitter-java       = { workspace = true }
tree-sitter-kotlin-ng  = { workspace = true }
tree-sitter-c          = { workspace = true }
tree-sitter-cpp        = { workspace = true }
```

즉 `kebab-source-fs` build → `kebab-parse-code` build → tree-sitter core + 9 grammar crates (C-compiled grammars + libstdc++ on cpp) drag.

#### ASCII dep graph — before / after (NIT #3 반영)

```text
before:
  kebab-source-fs ──> kebab-parse-code ──> [tree-sitter + 9 grammar crates]
                  \─> kebab-core
                  \─> kebab-config

after:
  kebab-source-fs ──> kebab-core
                  \─> kebab-config
  (4 helper surface 가 kebab-source-fs::code_meta 내부로 이전)
```

### §1.2 Drag 의 cost (qualitative)

정량 benchmark 는 본 spec 의 acceptance 에 포함하지 않는다 (workspace.version touch 0 + clean-build 측정 = 비용 대비 noise 큼). 정성적으로:

- `target/` 의 incremental compile artifact 가 9 grammar 별 `.o` + crate 별 metadata 로 누적. CLAUDE.md "90+ GB after a few task cycles" 의 일부.
- `cargo test -p kebab-source-fs` 가 link 단계에서 9 grammar object 를 끌어들임.
- 미래 `kebab-cli` / `kebab-mcp` 가 `kebab-source-fs` 만 의존 (code ingest 비활성 사용자) 하는 시나리오에도 9 grammar drag 가 강제됨.

본 cost 의 정량 측정은 **§5.4 informational only** 로 두고 acceptance 에서 분리.

### §1.3 4 surface — callsite (step 2 결과)

`grep -rn "kebab_parse_code\|kebab-parse-code" crates/kebab-source-fs/` 결과:

```
Cargo.toml:13            kebab-parse-code = { path = "../kebab-parse-code" }
src/media.rs:17          if let Some(lang) = kebab_parse_code::code_lang_for_path(path) {
src/walker.rs:9          //!     spec §5.2, applied via `kebab_parse_code::BUILTIN_BLACKLIST`)
src/walker.rs:85         /// Matcher built from `kebab_parse_code::BUILTIN_BLACKLIST` only.
src/walker.rs:131        for pat in kebab_parse_code::BUILTIN_BLACKLIST {
src/walker.rs:161        /// built-in safety-net blacklist (`kebab_parse_code::BUILTIN_BLACKLIST`),
src/walker.rs:211        for pat in kebab_parse_code::BUILTIN_BLACKLIST {
src/connector.rs:152     && kebab_parse_code::is_generated_file(&abs_path).unwrap_or(false)
src/connector.rs:169     if kebab_parse_code::is_oversized(
```

→ **공식 surface = 4 개** (round 1 의 "3 leaf" 추정에서 누락된 `BUILTIN_BLACKLIST` 포함):

| # | Surface | Kind | Real callsite count |
|---|---------|------|--------------------|
| 1 | `code_lang_for_path(&Path) -> Option<&'static str>` | fn | 1 (media.rs:17) |
| 2 | `is_generated_file(&Path) -> Result<bool>` | fn | 1 (connector.rs:152) |
| 3 | `is_oversized(&Path, u64, u32) -> Result<bool>` | fn | 1 (connector.rs:169) |
| 4 | `BUILTIN_BLACKLIST: &[&str]` (6 patterns) | `pub const` | 2 (walker.rs:131, 211) |

### §1.4 tree-sitter 미사용 검증 (step 3 결과)

3 leaf + 1 const 의 정의 file (`kebab-parse-code/src/lang.rs` + `kebab-parse-code/src/skip.rs`) 양쪽에 `grep -n "tree_sitter\|tree-sitter"`:

```
lang.rs:3: //! Lowercase canonical identifiers, matching tree-sitter parser conventions:
```

→ `lang.rs` 의 단 한 줄 — **docstring**. 본문 use 절:

- `lang.rs::code_lang_for_path`: `use std::path::Path;` — pure pattern match on `path.file_name()` / `path.extension()`.
- `skip.rs::is_generated_file`: `use anyhow::Result; use std::fs::File; use std::io::Read;` — 첫 512 byte 읽고 marker string 검사.
- `skip.rs::is_oversized`: `use anyhow::Result; use std::fs::{File, metadata}; use std::io::{BufRead, BufReader};` — `metadata.len()` → line iter.
- `skip.rs::BUILTIN_BLACKLIST`: `pub const &[&str] = &[...]` (6 entries).

→ tree-sitter / grammar crate 의존 0. 이동 가능 확정.

### §1.5 Consumer 검증 (step 4 결과 — destination 결정 핵심)

`grep -rn "code_lang_for_path\|is_generated_file\|is_oversized\|BUILTIN_BLACKLIST" crates/ --include="*.rs"` 결과에서 *kebab-parse-code 외부* 호출자:

| Crate / file | Surface | Kind |
|--------------|---------|------|
| `kebab-source-fs/src/media.rs:17` | `code_lang_for_path` | **real call** |
| `kebab-source-fs/src/connector.rs:152` | `is_generated_file` | **real call** |
| `kebab-source-fs/src/connector.rs:169` | `is_oversized` | **real call** |
| `kebab-source-fs/src/walker.rs:131, 211` | `BUILTIN_BLACKLIST` | **real ref** |
| `kebab-core/src/metadata.rs:36` | `code_lang_for_path` | **docstring only** (no actual call) |

→ **실 호출 consumer = `kebab-source-fs` 단일.** 그 외 `kebab-parse-code` 자체 tests (`tests/lang.rs`, `tests/skip.rs`) 에 호출 — destination 이동 시 함께 옮긴다.

부가 verification (NIT #1 반영) — `kebab_parse_code::skip::*` / `kebab_parse_code::lang::*` 명시 path 의 외부 ref:

```
$ grep -rn "kebab_parse_code::skip\|kebab_parse_code::lang::code_lang" crates/ --include="*.rs"
kebab-parse-code/tests/lang.rs:1   use kebab_parse_code::code_lang_for_path;            (re-export path)
kebab-parse-code/tests/skip.rs:1   use kebab_parse_code::skip::{BUILTIN_BLACKLIST, ...};  (전체 path)
```

→ 모두 `kebab-parse-code` 자체 test, 외부 명시 path consumer 0.

### §1.6 설계 contract / phase status (step 5/6 결과)

- `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md` §8 graph (현 frozen):
  ```text
  kebab-source-fs
        └─> kebab-parse-code (p10-1A-1: lang detect / repo detect / skip policy)
  ```
  → 본 spec 가 §8 의 해당 한 줄을 **edge 제거 + inline note 추가** (§7 MAJOR #4 반영) 형태로 갱신.
- `docs/ARCHITECTURE.md` Mermaid `srcfs → pcode` arrow 부재 (현 Mermaid 에 미표시) → Mermaid 변경 0, 산문 한 단락만 갱신.
- `tasks/INDEX.md`: P10 phase status — INDEX.md 의 dashboard 가 `p10-1B` / `p10-1C-Go` / `p10-1C-JK` 를 "PR 오픈" 으로 표시하나, branch base `b02ac82` 의 코드 트리에는 `module_path_for_python` (`lang.rs:77`), `python.rs` / `typescript.rs` / `javascript.rs` AST extractor, `go.rs` extractor, `java.rs`, `kotlin.rs` 모두 **이미 머지된 상태** (v0.18.0 cut 포함). INDEX.md 가 stale 한 것으로 추정 — 별도 doc-sync 영역. 본 refactor 와 conflict 가능 영역: 3 sub-task 의 코드는 모두 `kebab-chunk` 의 chunker 추가 (kebab-source-fs 의 dep 변경과 path 분리) → conflict 0. v0.18.0 cut 완료 (2026-05-26), active code-ingest PR / fb-* 진행 0.
- **task spec frozen 보존** (CLAUDE.md "Task specs themselves stay frozen…"): `tasks/p10/p10-1a-1-code-ingest-framework.md` line 23 "Source-fs **may** depend on `kebab-parse-code`" — "may" (의무 아니라 허용) 이므로 본 refactor 가 frozen task spec 의 contract 침범 0. 동일 logic 으로 `tasks/p10/p10-2`, `docs/superpowers/plans/2026-05-15-p10-1a-1`, `docs/superpowers/plans/2026-05-20-p10-2` 모두 frozen 보존. design §8 본체만 same-PR 갱신.

---

## §2 Goals + non-goals

### Goals (G)

- **G1**: `kebab-source-fs/Cargo.toml` 에서 `kebab-parse-code` dep 제거.
- **G2**: 4 surface (`code_lang_for_path`, `is_generated_file`, `is_oversized`, `BUILTIN_BLACKLIST`) 의 callsite + 의미 (signature, return type, behavior, error variant) 보존.
- **G3**: 기존 unit test (`kebab-parse-code/tests/lang.rs`, `tests/skip.rs`) 의 cover 가 destination 으로 1:1 이동 + design §5.2 의 frozen contract (6 BUILTIN_BLACKLIST entry) 검증이 외부 시점에서 가능하게 유지. baseline 회귀 0.
- **G4**: design §8 의 allowed-deps graph 갱신 — `kebab-source-fs → kebab-parse-code` edge 제거 + `kebab-source-fs (lang detect + skip policy 내장)` inline note 추가.
- **G5**: `cargo tree -p kebab-source-fs` 결과의 dep tree 에서 `tree-sitter*` 부재 (objective acceptance — §5.3).

### Non-goals (NG)

- **NG1**: `kebab-parse-code` 의 9 tree-sitter grammar 자체 정리 / 동적 로딩 / feature gate.
- **NG2**: `kebab-source-fs::media.rs` 의 extension-match logic 재설계.
- **NG3**: `kebab-parse-code/src/lang.rs` 의 sibling `module_path_for_python` / `module_path_for_tsjs` 이동 — caller 는 본 crate 자체 (`python.rs:78`, `typescript.rs:88`, `javascript.rs:95`). concern 분리 (lang **detection** vs module-path **derivation**). §1.5 / §6.5 / §6.6 참조.
- **NG4**: `kebab-parse-code/src/repo.rs` 의 `detect_repo` / `RepoMeta` 이동 — kebab-source-fs 가 호출 안 함.
- **NG5**: workspace `Cargo.toml` 의 `version` bump — internal refactor (wire 변경 0). CLAUDE.md "Release / binary version bump" 3 트리거 (dogfooding 필요, schema/wire breaking, frozen design 변경) 모두 미충족. frontmatter `target_version: 0.18.0` 의 의미 = "본 PR 머지 시 워크스페이스 version 이 0.18.0 그대로 유지된다" (= NG5 와 정합).
- **NG6**: V00X SQLite migration / wire schema major bump.
- **NG7**: `kebab-core::media.rs` 와의 medium-detection 통합 (Lens 3 별도).

---

## §3 Design

### §3.1 Destination 선택 — Option B (`kebab-source-fs::code_meta`)

| 후보 | 호환 | 트레이드오프 | 결정 |
|------|------|------------|------|
| **A. `kebab-core::code`** | OK | "kebab-core: domain types only" 룰 약 stretch (`kebab-core::media.rs` 의 precedent 는 enum 정의지만 본 helper 는 IO + match). 미래 2nd consumer 우월. | ✗ |
| **B. `kebab-source-fs::code_meta`** | OK | core 룰 0 stretch. dep graph 단순화 최대. 미래 2nd consumer 등장 시 promote 필요. | **✓ 채택** |
| **C. 신규 crate `kebab-code-meta`** | OK | workspace member 추가 ceremony. 1 consumer 대비 과함. | ✗ |

**채택 근거** (consumer count = 1, leaf + const 가 pure logic):

- Investigation step 4 결과로 외부 consumer = `kebab-source-fs` 1 개 확정.
- §8 boundary rule stretch 0 (core 영역 미침범).
- 미래 2nd consumer 발생 시 cost = visibility 확장 (§3.3 의 mixed-visibility 정책 참조). 본 작업 cost 와 비교해 deferred decision cost 낮음.

### §3.2 Module placement

신규 module + 기존 source 의 변경:

```text
crates/kebab-source-fs/src/
├── lib.rs
├── code_meta.rs    ← 신규: 4 surface. lang detect + skip helpers + blacklist.
├── connector.rs    ← edit: kebab_parse_code:: prefix 4 곳 → crate::code_meta:: 로 교체.
├── hash.rs
├── media.rs        ← edit: kebab_parse_code:: prefix 1 곳 → crate::code_meta:: 로 교체.
└── walker.rs       ← edit: kebab_parse_code:: prefix 2 곳 (block import) + comment 3 곳 → crate::code_meta:: 로 교체.
```

기존 `lib.rs` 의 module 선언 (branch base `b02ac82` 인용):

```rust
// 현재 (branch base b02ac82) ──────────────────────────────────────
mod connector;
mod hash;
mod media;
mod walker;

pub use connector::{FsScanSkips, FsSourceConnector};
```

본 refactor 후:

```rust
// 본 spec 적용 후 ─────────────────────────────────────────────────
mod connector;
mod hash;
mod media;
mod walker;
mod code_meta;   // 신규 — visibility 정책은 §3.3 참조.

pub use connector::{FsScanSkips, FsSourceConnector};
pub use code_meta::BUILTIN_BLACKLIST;   // §3.3 frozen contract — integration test (§5.1) 의 접근 surface.
```

→ `BUILTIN_BLACKLIST` 의 `pub use` 한 줄이 신규 추가되는 **유일한 외부 surface 증가** (현재의 `kebab_parse_code::skip::BUILTIN_BLACKLIST` 외부 ref 가 `kebab_source_fs::BUILTIN_BLACKLIST` 로 대칭 이동, 사실상 net surface 변화 0). 3 helper fn 은 `pub(crate)` 라서 `pub use` 미발생. §7 의 "wire/surface 변경 0" claim 과 정합 — 본 한 줄은 **internal Rust crate-API surface** (wire/CLI/TUI/MCP 의 user-facing surface 아님) 의 minimal 이동.

### §3.3 Visibility 정책 — mixed `pub` / `pub(crate)` (MAJOR #1 반영)

회차 1 critic MAJOR #1 의 트레이드오프: **모두 `pub(crate)` 로 좁히면 design §5.2 frozen contract (6 BUILTIN_BLACKLIST entry) 의 검증이 같은 module 안으로 한정 → silent breakage 가능**. 해결책 = **per-surface 차등 visibility**:

| Surface | Visibility | 근거 |
|---------|-----------|------|
| `BUILTIN_BLACKLIST` | **`pub`** | design §5.2 의 frozen contract (6 entry, 정확 list). 외부 integration test (§5.1) 가 검증 surface 로 사용. |
| `code_lang_for_path` | `pub(crate)` | source-fs 내부 호출만 (media.rs). 미래 2nd consumer 시 promote. |
| `is_generated_file` | `pub(crate)` | source-fs 내부 호출만 (connector.rs). |
| `is_oversized` | `pub(crate)` | source-fs 내부 호출만 (connector.rs). |

`code_meta.rs` 의 module-level doc 첫 줄에 본 visibility 정책을 cross-link:

```rust
//! Pre-ingest classification + skip helpers for the local-filesystem
//! SourceConnector. Moved from `kebab-parse-code` (refactor 2026-05-26)
//! to drop the 9 tree-sitter grammar drag from this crate's dep tree.
//!
//! `BUILTIN_BLACKLIST` is `pub` because it implements the **frozen contract
//! in design §5.2** (the 6-pattern safety-net list). External integration
//! tests (`tests/code_meta.rs`) verify the contract from outside the module
//! to prevent silent breakage. The 3 helper fns are `pub(crate)` — no
//! external consumer today.
```

### §3.4 Function + const signatures (보존 — 1:1)

```rust
// crates/kebab-source-fs/src/code_meta.rs   (신규)

use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

use anyhow::Result;

/// 6 built-in gitignore-style patterns. Applied in addition to `.gitignore`
/// + `.kebabignore`. User can override via `.kebabignore` negation (`!pattern`).
///
/// Source of truth: design §5.2 (frozen).
pub const BUILTIN_BLACKLIST: &[&str] = &[
    "**/node_modules/**",
    "**/target/**",
    "**/__pycache__/**",
    "**/.venv/**",
    "**/venv/**",
    "**/env/**",
];

/// Returns the canonical language identifier for a given file path.
/// 본문은 [kebab-parse-code/src/lang.rs:17] 와 byte-identical 보존.
pub(crate) fn code_lang_for_path(path: &Path) -> Option<&'static str> { /* ... */ }

/// Read first 512 bytes; check 7 case-insensitive generated-file markers.
/// 본문은 [kebab-parse-code/src/skip.rs:28] 와 byte-identical 보존.
pub(crate) fn is_generated_file(path: &Path) -> Result<bool> { /* ... */ }

/// Check if `path` exceeds `max_bytes` or `max_lines` (byte cap then line cap).
/// 본문은 [kebab-parse-code/src/skip.rs:50] 와 byte-identical 보존.
pub(crate) fn is_oversized(path: &Path, max_bytes: u64, max_lines: u32) -> Result<bool> { /* ... */ }
```

→ 시그니처 / 본문 / error type / return type 변경 0. visibility 만 §3.3 의 정책 적용.

### §3.5 Callsite migration

| File | Line | Before | After |
|------|------|--------|-------|
| `kebab-source-fs/src/media.rs` | 17 | `if let Some(lang) = kebab_parse_code::code_lang_for_path(path) {` | `if let Some(lang) = crate::code_meta::code_lang_for_path(path) {` |
| `kebab-source-fs/src/walker.rs` | 131 | `for pat in kebab_parse_code::BUILTIN_BLACKLIST {` | `for pat in crate::code_meta::BUILTIN_BLACKLIST {` |
| `kebab-source-fs/src/walker.rs` | 211 | `for pat in kebab_parse_code::BUILTIN_BLACKLIST {` | `for pat in crate::code_meta::BUILTIN_BLACKLIST {` |
| `kebab-source-fs/src/connector.rs` | 152 | `&& kebab_parse_code::is_generated_file(&abs_path).unwrap_or(false)` | `&& crate::code_meta::is_generated_file(&abs_path).unwrap_or(false)` |
| `kebab-source-fs/src/connector.rs` | 169 | `if kebab_parse_code::is_oversized(` | `if crate::code_meta::is_oversized(` |

Comment-only update:

| File | Line | Action |
|------|------|--------|
| `kebab-source-fs/src/walker.rs` | 9, 85, 161 | `kebab_parse_code::BUILTIN_BLACKLIST` → `crate::code_meta::BUILTIN_BLACKLIST` |
| `kebab-core/src/metadata.rs` | 36 | doc 주석 — `pub(crate)` 함수에 대한 rustdoc broken link 회피 위해 abstract wording 으로 정리 (MINOR #5): `Set by kebab_parse_code::lang::code_lang_for_path.` → `Set by the local-filesystem source connector during ingest.` |

### §3.6 kebab-parse-code 측 cleanup

| Path | Action | 비고 |
|------|--------|------|
| `crates/kebab-parse-code/src/skip.rs` | **삭제** | 본 file 의 모든 surface (3개) 가 source-fs 로 이동, 자체 사용처 0. |
| `crates/kebab-parse-code/src/lang.rs` | edit (narrow — `code_lang_for_path` + 관련 unit test 만 제거) | `code_lang_for_path` 함수 + `#[cfg(test)] mod tests::tier2_basename_takes_precedence_over_extension` + `#[cfg(test)] mod tests::tier2_extension_fallback` 만 제거. **`module_path_for_python`, `module_path_for_tsjs` + 그 두 unit test (`module_path_for_python_strips_src_roots_and_extensions`, `module_path_for_tsjs_keeps_slashes_and_strips_ext`) 보존** — caller 는 본 crate 자체 (`src/{python,typescript,javascript}.rs`). 헤더 doc 한 단락 rewrite (MINOR #2): "Lowercase canonical identifiers, matching tree-sitter parser conventions:" → "Workspace-relative path → module-path conversion for P10-1B AST extractors (Python dotted form / TS+JS slash form)." |
| `crates/kebab-parse-code/src/lib.rs` | edit | (a) `pub mod skip;` line 삭제. (b) `pub use lang::{code_lang_for_path, module_path_for_python, module_path_for_tsjs};` 의 `code_lang_for_path` 만 제거 (sibling 2 개 보존). (c) `pub use skip::{BUILTIN_BLACKLIST, is_generated_file, is_oversized};` line 전체 삭제. (d) `//!` 헤더 doc "Phase 1A-1 ships infrastructure only" 단락 rewrite (MINOR #3): infrastructure-only wording → "Repo metadata (`detect_repo`) + per-language AST extractors (Rust = P10-1A-2, Python/TS/JS = P10-1B, Go = P10-1C-Go, Java+Kotlin = P10-1C-JK, C+C++ = P10-1D)." |
| `crates/kebab-parse-code/tests/lang.rs` | **이동** | 본 test 가 `code_lang_for_path` 만 검증. 본문 4 case (`known_extensions_map_to_canonical_identifiers`, `special_filenames_map_to_identifiers`, `unknown_extension_returns_none`, `case_insensitive`) → `kebab-source-fs/tests/code_meta.rs` 의 integration test 로 이전 (§3.7 참조). (NIT #2 — "삭제 또는 이동" OR 단일화) |
| `crates/kebab-parse-code/tests/skip.rs` | **이동** | 7 test case 모두 → `kebab-source-fs/tests/code_meta.rs` 로 이전 (§3.7 참조). |
| `crates/kebab-parse-code/Cargo.toml` | **변경 0** | `[dev-dependencies] tempfile` 는 `tests/repo.rs:4 use tempfile::TempDir;` 가 계속 소비하므로 **유지** (CRITICAL #1). |
| `crates/kebab-parse-code/src/c.rs ~ rust.rs` (9 grammar AST extractor file) | **변경 0** | tree-sitter 본진. surface 보존. |

### §3.7 Test placement — integration over unit (MAJOR #5 반영)

신규 `crates/kebab-source-fs/tests/code_meta.rs` integration test 로 cover (unit 이 아님).

**근거 (round 1 MAJOR #5 의 약한 unit-test 정당화 보강)**:

- (a) `BUILTIN_BLACKLIST` 가 `pub` (§3.3) — design §5.2 frozen contract 의 외부 검증 surface 로서 integration test 가 자연.
- (b) source-fs 가 이미 integration test 3 개 (`include_allowlist.rs`, `snapshot_tree1.rs`, `symlink_cycle.rs`) 보유 — 단일 패턴 일관.
- (c) `code_lang_for_path` / `is_generated_file` / `is_oversized` 는 `pub(crate)` 라서 integration test 가 직접 호출 불가. 따라서 **mixed placement**:
  - `BUILTIN_BLACKLIST` 6-entry contract → `tests/code_meta.rs` (integration).
  - 3 helper fn 의 detail behavior (lang detection / generated marker / size cap) → `src/code_meta.rs` 의 `#[cfg(test)] mod tests` (unit, `pub(crate)` 접근 가능).
- (d) link 단계 추가 binary 1 개 (`tests/code_meta.rs`) — kebab-source-fs 가 lance / datafusion 미링크. CLAUDE.md `-j 1` 강제 트리거 (= lance/datafusion 합산 link 폭주) 에 영향 0 — source-fs 의 build cost 증분은 단발적이며 `-j 1` 강제와 무관.

### §3.8 Cargo.toml diff

`crates/kebab-source-fs/Cargo.toml` — 13번 줄 한 줄 삭제:

```diff
 [dependencies]
 kebab-core = { path = "../kebab-core" }
 kebab-config = { path = "../kebab-config" }
-kebab-parse-code = { path = "../kebab-parse-code" }
 anyhow       = { workspace = true }
```

`crates/kebab-parse-code/Cargo.toml` — **변경 0** (CRITICAL #1: `tempfile` 은 `tests/repo.rs` 가 계속 소비).

Workspace `Cargo.toml` — 변경 0.

### §3.9 Test 이동 path

`kebab-source-fs/tests/code_meta.rs` (integration — `BUILTIN_BLACKLIST` 검증) + `kebab-source-fs/src/code_meta.rs` 의 `#[cfg(test)] mod tests` (unit — 3 helper fn 검증) 로 split:

| 원본 (kebab-parse-code) | 목적지 | 비고 |
|----------------------|---------|------|
| `tests/lang.rs::known_extensions_map_to_canonical_identifiers` | source-fs unit (`pub(crate)` → 내부 호출) | 32 case |
| `tests/lang.rs::special_filenames_map_to_identifiers` | 동일 | Dockerfile / Makefile / GNUmakefile |
| `tests/lang.rs::unknown_extension_returns_none` | 동일 | 3 case |
| `tests/lang.rs::case_insensitive` | 동일 | `Foo.RS`, `FOO.YAML` |
| `src/lang.rs::tests::tier2_basename_takes_precedence_over_extension` | 동일 (unit) | tier2 basename |
| `src/lang.rs::tests::tier2_extension_fallback` | 동일 (unit) | tier2 ext |
| `tests/skip.rs::generated_header_markers_trigger_skip` | source-fs unit | 7 marker |
| `tests/skip.rs::normal_code_is_not_flagged_generated` | 동일 | |
| `tests/skip.rs::is_generated_returns_false_for_empty_file` | 동일 | |
| `tests/skip.rs::oversized_by_bytes_returns_true` | 동일 | |
| `tests/skip.rs::oversized_by_lines_returns_true` | 동일 | |
| `tests/skip.rs::small_file_returns_false_for_oversize` | 동일 | |
| `tests/skip.rs::builtin_blacklist_has_exactly_six_entries` | **integration** (`tests/code_meta.rs`) | `BUILTIN_BLACKLIST` 가 `pub` → 외부 검증 |

Tempfile 의존: source-fs 의 `[dev-dependencies]` 에 이미 존재 (line 25: `tempfile = "3"`).

---

## §4 Open questions

### §4.1 `code_lang_for_path` 의 미래 second consumer

- 현재: `source-fs::media.rs` 만 호출. `MediaType::Code(lang)` 가 downstream chunker (kebab-chunk) 의 dispatch key 가 되어 chunker 가 lang 을 직접 query 할 필요 없음.
- 미래 risk: `kebab-chunk` Tier 1 dispatch 가 path → lang 재 derivate 필요해질 경우 → `pub(crate)` → `pub` promote 필요. cost = visibility 한 줄 변경 + chunk crate 가 source-fs 의존 추가. deferred.

### §4.2 `is_generated_file` 의 7-marker sniff logic 갱신 시 ownership

- 본 spec 머지 후 ownership = source-fs maintainer (의도). 명문화 = `code_meta.rs` 의 module-level doc (§3.3) 의 "Moved from `kebab-parse-code` (refactor 2026-05-26)" 한 줄.

### §4.3 `BUILTIN_BLACKLIST` 6 entry = design §5.2 frozen contract

- ownership 이전 (parse-code → source-fs) 가 §5.2 의 "frozen" 의미와 충돌 0 — frozen 은 6 entry 내용 자체. owner crate 위치 변경은 frozen 대상 아님.
- 외부 검증: integration test (`tests/code_meta.rs`) 가 6 entry 의 byte-identical 보존 검증 (`assert_eq!(BUILTIN_BLACKLIST.len(), 6)` + 6 string 의 `contains` 검증).

### §4.4 `cargo tree -p kebab-source-fs | grep tree-sitter` = 0 의 transitive scope

- 본 acceptance 는 **검증 시점 snapshot**. 미래에 `kebab-config` / `kebab-core` 가 tree-sitter 끌어오면 본 acceptance 가 자동 fail — 단 그 경우 별도 spec 가 책임.

### §4.5 build-time benchmark = optional informational only (§5.4)

- 정량 측정은 acceptance 에서 분리. PR description 에 부기 권장이나 강제 아님.

### §4.6 cargo-deny / workspace `deny.toml` (What's Missing #1)

- 현 시점 repo 에 `deny.toml` 부재 (`ls /home/altair823/kebab/deny.toml` 결과: No such file). design §8 의 "cargo deny + workspace deny.toml + CI 체크로 강제" 는 frozen 의 미래 상태, 본 spec 머지 시점 미적용. 본 spec 가 deny.toml 신설 / 갱신 강제 안 함. 미래 cargo-deny 도입 시 본 refactor 의 edge 제거가 enforcement 와 정합 (= source-fs 에서 parse-code dep ban rule 가능).

### §4.7 Future risk: parse-code 가 source-fs 를 reverse-import 욕구 추가 (What's Missing #4)

- 가설: parse-code 의 AST extractor 가 어떤 helper 를 위해 source-fs 의 `code_meta` 를 호출하고 싶어질 경우 → 의존 cycle (source-fs → parse-code 가 끊겼는데, parse-code → source-fs 가 생기면 본 refactor 의 의도 무효화) 또는 design §8 forbidden edge.
- mitigation: 본 spec 의 destination 결정 (Option B) 가 future-coupling risk 를 키움. 만약 parse-code 가 lang detect 가 필요해지면 Option A (kebab-core::code) 로 promote 가 올바른 방향 — 즉 본 spec 의 `pub(crate)` choice (§3.3) 가 사실상 reverse-import risk 의 신호기 역할 (외부 호출 0 보장).

---

## §5 Verification plan

§5.4 는 **informational only**, acceptance 에서 분리 (MINOR #4).

### §5.1 Unit + integration tests (source-fs)

신규 `crates/kebab-source-fs/src/code_meta.rs::tests` (unit) + `crates/kebab-source-fs/tests/code_meta.rs` (integration) 에 다음 test name 이 모두 존재:

**Unit (`src/code_meta.rs`)** — `pub(crate)` helper 검증:
```
known_extensions_map_to_canonical_identifiers
special_filenames_map_to_identifiers
unknown_extension_returns_none
case_insensitive
tier2_basename_takes_precedence_over_extension
tier2_extension_fallback
generated_header_markers_trigger_skip
normal_code_is_not_flagged_generated
is_generated_returns_false_for_empty_file
oversized_by_bytes_returns_true
oversized_by_lines_returns_true
small_file_returns_false_for_oversize
```

**Integration (`tests/code_meta.rs`)** — `pub const` 검증:
```
builtin_blacklist_has_exactly_six_entries
```

기대: `cargo test -p kebab-source-fs code_meta` → 13 passing.

기존 `kebab-source-fs/src/{connector,media,walker}.rs::tests` + `kebab-source-fs/tests/{include_allowlist,snapshot_tree1,symlink_cycle}.rs` **변경 0** — callsite prefix 만 갱신.

### §5.2 Workspace 회귀

```sh
cargo test --workspace --no-fail-fast -j 1
```

기대: branch base `b02ac82` 에서 실측한 N passing 을 유지. **N = implementation phase 의 PR description 에 baseline 측정 결과로 명시** (MAJOR #2 — round 1 의 "1313" 은 v0.18.0 cut 시점 추정, b02ac82 = HOTFIX #15 + S3 NLI 머지 후 시점). 측정 방법:

```sh
cargo test --workspace --no-fail-fast -j 1 2>&1 | tail -50  # baseline N 추출
```

`-j 1` 필수 (CLAUDE.md "Build / test / lint" — 18 integration-test binary 동시 link 시 OOM).

추가로 **가장 강한 안전망 (What's Missing #3 강조)**:

```sh
cargo test -p kebab-app --test code_ingest_smoke -j 1
```

`--test code_ingest_smoke` 는 integration test **binary** (file 이름 = binary 이름) 를 선택 — bare `cargo test -p kebab-app code_ingest_smoke` 는 substring test-name filter 로 해석돼 16+ fn 중 매치 0건 → "0 tests run" + exit 0 의 false-positive PASS 가 나므로 사용 금지 (verifier-plan round 1 Gap #1). 이 e2e fixture 가 `module_path_for_python` / `module_path_for_tsjs` 사용을 dogfooding KB 흐름으로 검증 (`code_ingest_smoke.rs:165, 242, 319` 의 doc-comment + fixture). 회귀 시 본 명령이 fail.

### §5.3 Clippy + build + dep tree

```sh
cargo clippy --workspace --all-targets -j 1 -- -D warnings
cargo build --release -j 1
cargo tree -p kebab-source-fs | grep tree-sitter
```

기대:
- clippy: clean (workspace pedantic + inline 30+ allow 그대로).
- build: clean release binary.
- `cargo tree` grep: **0 줄 출력**.

### §5.4 Optional: build time benchmark (informational only — NOT acceptance)

```sh
cargo clean
time cargo build -p kebab-source-fs --release -j 1   # baseline
# checkout refactor branch
cargo clean
time cargo build -p kebab-source-fs --release -j 1   # after refactor
```

PR description 에 부기.

---

## §6 Risks

### §6.1 Destination 의 §8 stretch (Option B 채택 시 0)

§3.1 의 채택 근거로 해소.

### §6.2 4 surface 외 hidden callsite

- Investigation step 2 + NIT #1 의 보조 grep 으로 검증 완료. alias / re-export 0.
- 추가 가드: refactor 머지 직전 `grep -rn "parse_code\|parse-code" crates/kebab-source-fs/` 재확인 (implementation phase 의 plan checklist).

### §6.3 `BUILTIN_BLACKLIST` 의 link-time / .rodata 영향

- const 가 source-fs 로 이동 시 binary 의 `.rodata` 위치 변경 only. 의미 변경 0.

### §6.4 `kebab_parse_code::skip` 의 외부 ref

- NIT #1 grep 결과 매치 2 곳 모두 parse-code 자체 test → §3.6 의 test 이동과 함께 해소. 안전.

### §6.5 `lang.rs` narrow edit 시 sibling `module_path_for_*` accidental drop (CRITICAL #2 반영)

- Round 1 의 잘못된 회귀 catch crate: `kebab-chunk` 가 `module_path_for_*` 호출 0. 정정된 caller:
  - `kebab-parse-code/src/python.rs:78`
  - `kebab-parse-code/src/typescript.rs:88`
  - `kebab-parse-code/src/javascript.rs:95`
  - `kebab-app/tests/code_ingest_smoke.rs:165, 242, 319` (e2e fixture, doc-comment + 동작 검증)
- 정정된 안전망:
  - `cargo test -p kebab-parse-code --no-fail-fast -j 1 module_path_for_` — sibling unit test (`module_path_for_python_strips_src_roots_and_extensions` + `module_path_for_tsjs_keeps_slashes_and_strips_ext`) 가 fail 하면 catch.
  - `cargo test -p kebab-app --test code_ingest_smoke -j 1` — P10-1B e2e fixture 가 fail 하면 catch (`--test` flag 강제 — verifier-plan round 1 Gap #1).
- 둘 다 §5.2 의 workspace 회귀 + 본 §6.5 의 명시 cli 양쪽으로 cover.

### §6.6 ARCHITECTURE.md / design §8 drift

- §7 의 ARCHITECTURE.md + design §8 갱신을 same-PR 로 진행. CLAUDE.md "Changing the design doc requires updating every referencing task spec in the same PR" 룰 — frozen task spec (`tasks/p10/p10-1a-1`, `tasks/p10/p10-2`, `plans/2026-05-15-p10-1a-1`, `plans/2026-05-20-p10-2`) 는 §1.6 의 분석 (모두 "may" 수준의 reference, contract violation 0) 으로 frozen 보존. design §8 의 graph block 만 갱신.

### §6.7 cargo-deny enforcement 의 의도-vs-현실 gap (§4.6 cross-link)

- design §8 의 "cargo deny + deny.toml + CI 체크" frozen wording 이 현 시점 미적용. 본 spec 머지가 enforcement gap 신설 아님 — 이미 존재하는 gap 의 영향 받지 않음.

---

## §7 Wire / surface impact

| Surface | 변경 | 비고 |
|---------|------|------|
| wire schema (`*.v1`) | 0 | 본 4 surface 는 wire 출력에 미surface. |
| CLI subcommand / flag / `--json` field / exit code | 0 | |
| TUI / desktop / MCP | 0 | |
| **Cargo workspace.version** | **0 bump** | CLAUDE.md "Release / binary version bump" 3 트리거 미충족. frontmatter `target_version: 0.18.0` = "본 PR 머지 시 0.18.0 그대로". |
| **Cargo features** | **0** (MINOR #6) | `kebab-source-fs` / `kebab-parse-code` 양 crate 의 `[features]` 변경 0. |
| **parser_version cascade** | **0** (MINOR #6) | design §9 의 cascade identifier (`parser_version`, `chunker_version`, `embedding_version`, `prompt_template_version`, `index_version`) 변경 0. 회귀 시 cascade 영향 0. |
| `Config` / `KEBAB_*` env | 0 | `ingest.code.skip_generated_header` / `max_file_bytes` / `max_file_lines` 는 의미 + 위치 그대로 (kebab-config 의 `IngestCodeCfg`). callsite 만 source-fs internal 로 정리. |
| SQLite migration (V00X) | 0 | DDL 미접촉. |
| README | 변경 0 | |
| HANDOFF.md | 변경 0 | phase 단위 변화 0 (single-crate internal refactor). |
| **docs/ARCHITECTURE.md** | **갱신 (same-PR)** | (a) "kebab-parse-code 의 외부 tree-sitter grammar crate 의존" 산문 끝에 한 줄 추가 — "v0.18.0+ 부터 `kebab-source-fs` 는 자체 `code_meta` 모듈 (lang detect + skip helpers + BUILTIN_BLACKLIST) 을 보유, `kebab-parse-code` 와 분리." (b) Mermaid 변경 0 (`srcfs → pcode` arrow 미포함). |
| **design §8 graph** | **갱신 (same-PR — MAJOR #4 반영, 두 줄)** | (a) **edge 제거**: 기존 `kebab-source-fs └─> kebab-parse-code (p10-1A-1: lang detect / repo detect / skip policy)` 라인 삭제. (b) **inline note 추가**: `kebab-source-fs` row 아래 `(p10-2 이후: lang detect + skip policy 내장; kebab-parse-code 와 분리)` 한 줄 보강. |
| tasks/HOTFIXES.md | **추가 불필요** | design §8 자체를 갱신하므로 frozen vs ship deviation 0. CLAUDE.md HOTFIXES rule 미트리거. |
| referencing task spec | **frozen 보존** | §1.6 분석: `tasks/p10/p10-1a-1` line 23 의 "may" reference 는 contract violation 0 → frozen. `tasks/p10/p10-2`, `plans/2026-05-15-p10-1a-1`, `plans/2026-05-20-p10-2` 동일. |
| tasks/INDEX.md | 변경 0 | phase 단위 신규 task 아님. |

---

## §8 Out of scope

- Lens 1 다른 묶음 (`kebab-normalize` 흡수, `kebab-parse-types` 추가 정리) — 별도 spec.
- Lens 2 (`kebab-chunk` Tier 2 helper 정리) — 별도.
- Lens 3 (Extractor dispatch unification) — system-architect post-refactor report 의 차기 candidate, 별도.
- `kebab-parse-code` 의 9 tree-sitter grammar feature gating / dynamic loading — v0.19+ candidate, 별도.
- `kebab-parse-code/src/repo.rs` ownership 검토 — 본 spec 범위 밖.
- `kebab-core::media.rs` 와 `kebab-source-fs::code_meta` 의 medium-vs-lang detection 통합.
- `deny.toml` 신설 / cargo-deny CI 도입 — frozen design §8 의 미래 state, 별도.

---

## §9 References

- `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md` §3.5, §3.7b, §5.2, §8.
- `tasks/p10/p10-1a-1-code-ingest-framework.md` (frozen — line 23 "may" reference 보존).
- `tasks/INDEX.md` — P10 phase status (모두 ✅ 머지, v0.18.0 cut 완료 2026-05-26).
- `tasks/HOTFIXES.md` — 본 spec 머지 시 항목 추가 불필요.
- Investigation grep evidence — §1.3 + §1.5.
- Workspace `Cargo.toml` workspace.version = `0.18.0` (target frontmatter 동일, bump 미실시).
- `kebab-parse-code/tests/repo.rs:4` — `tempfile::TempDir` 사용 (CRITICAL #1 의 baseline evidence).
- `kebab-parse-code/src/python.rs:78`, `typescript.rs:88`, `javascript.rs:95` — `module_path_for_*` 실 caller (CRITICAL #2 의 baseline evidence).
- `kebab-app/tests/code_ingest_smoke.rs` — P10-1B e2e fixture (§5.2 의 가장 강한 안전망).

---

## §10 Round 1 critic closure status

| Finding | Severity | 반영 | 위치 |
|---------|----------|------|------|
| **CRITICAL #1** tempfile dev-dep 삭제 claim 거짓 | CRITICAL | **reflected** | §3.6 (Cargo.toml row 변경 0), §3.8 (`tempfile` 줄 삭제 제거), §9 (`tests/repo.rs:4` evidence). |
| **CRITICAL #2** module_path_for_* 회귀 catch crate 오인 | CRITICAL | **reflected** | §6.5 (verify 명령 교체 — `kebab-chunk` → `kebab-parse-code` + `kebab-app code_ingest_smoke`), §5.2 (e2e fixture cli 명시), §9 (caller evidence 3 file). |
| **MAJOR #1** BUILTIN_BLACKLIST `pub(crate)` 가 frozen verifiability erode | MAJOR | **reflected** (Option A 채택) | §3.3 (mixed visibility 정책 — `BUILTIN_BLACKLIST` `pub`, 3 fn `pub(crate)`), §3.7 (integration test 로 6-entry contract 보존), §4.3, §5.1. |
| **MAJOR #2** 1313 baseline 시점 부정확 | MAJOR | **reflected** | §5.2 (wording → "branch base b02ac82 에서 실측 N passing", 측정 방법 cli 명시). |
| **MAJOR #3** frontmatter target_version vs NG5 충돌 | MAJOR | **reflected** | frontmatter (target_version: 0.18.0 + 주석으로 의미 명시), §2 NG5 (frontmatter cross-link), §7 Cargo workspace.version row. |
| **MAJOR #4** design §8 graph 갱신 scope 부족 | MAJOR | **reflected** | §7 design §8 row (edge 제거 (a) + inline note 추가 (b) 두 줄). |
| **MAJOR #5** §3.7 unit test 정당화 약함 | MAJOR | **reflected** | §3.7 (4 가지 근거 재작성 — (a) frozen contract integration surface, (b) source-fs 의 integration test 패턴 일관, (c) `pub(crate)` 접근 위한 unit 필수성, (d) link cost 분석). |
| **MINOR #1** "edit (split)" wording | MINOR | **reflected** | §3.6 ("edit (narrow — code_lang_for_path + 관련 unit test 만 제거)"). |
| **MINOR #2** lang.rs 헤더 doc 갱신 명시 | MINOR | **reflected** | §3.6 lang.rs 행 ("Workspace-relative path → module-path conversion for P10-1B AST extractors…"). |
| **MINOR #3** lib.rs 헤더 doc 단락 rewrite | MINOR | **reflected** | §3.6 lib.rs 행 ("Repo metadata + per-language AST extractors…"). |
| **MINOR #4** §5.4 informational only 명시 | MINOR | **reflected** | §5 시작부 한 줄 + §5.4 제목 "informational only — NOT acceptance". |
| **MINOR #5** metadata.rs:36 abstract wording | MINOR | **reflected** | §3.5 ("Set by the local-filesystem source connector during ingest"). |
| **MINOR #6** wire/surface table 에 features + cascade row | MINOR | **reflected** | §7 (2 row 추가 — Cargo features = 0, parser_version cascade = 0). |
| **NIT #1** §1.5 또는 §6.2 끝에 추가 grep | NIT | **reflected** | §1.5 끝부 ("부가 verification" 블록 — `kebab_parse_code::skip\|kebab_parse_code::lang::code_lang` grep 결과). |
| **NIT #2** §3.5 "삭제 또는 이동" OR 단일화 | NIT | **reflected** | §3.6 ("이동 — 본문은 §3.7 참조"). |
| **NIT #3** ASCII before/after dep graph | NIT | **reflected** | §1.1 끝부 ASCII block. |
| **What's Missing #1** cargo deny / deny.toml | — | **reflected** | §4.6 + §6.7 + §8 (현 미적용, 본 spec 가 신설 강제 아님). |
| **What's Missing #2** task spec frozen contract rule | — | **reflected** | §1.6 (4 referencing task/plan 의 "may" reference 분석 — frozen 보존), §6.6, §7 (referencing task spec row), frontmatter `related_specs` cross-link. |
| **What's Missing #3** kebab-app code_ingest_smoke 가 가장 강한 안전망 | — | **reflected** | §5.2 ("가장 강한 안전망" 블록 + e2e fixture 의 module_path_for_* 검증 인용), §6.5 verify 명령. |
| **What's Missing #4** future risk: parse-code reverse-import | — | **reflected** | §4.7 (가설 + mitigation — `pub(crate)` 가 reverse-import risk 신호기). |

**Round 1 closure summary**: 2 CRITICAL + 5 MAJOR + 6 MINOR + 3 NIT + 4 What's Missing = **20 finding 모두 reflected**, rejection 0.

### §10.1 Round 2 critic 후속 closure (v2 → v3)

| Finding | Severity | 반영 | 위치 |
|---------|----------|------|------|
| **NEW MAJOR #1** §1.6 P10 phase status 사실 오류 (INDEX.md stale 미언급) | MAJOR | **reflected** (option a wording — honest INDEX.md stale 알림 + conflict 0 진술) | §1.6 P10 status 단락 재작성. |
| **NEW MAJOR #2** §3.2 lib.rs 예시 의 surface 무근거 확장 (`pub mod connector` / `pub mod media`) | MAJOR | **reflected** (Option A 채택 — `mod` 보존 + `pub use code_meta::BUILTIN_BLACKLIST` 한 줄 신규) | §3.2 (before/after lib.rs 두 블록 + net surface 변화 0 분석 한 단락), §7 의 wire/surface 변경 0 claim 과 정합 확인 inline 명시. |
| **NEW MINOR #1** §3.7 (d) link cost 부정확 wording (18 → 19 비교) | MINOR | **reflected** | §3.7 (d) wording 정정 — "lance/datafusion 합산 link 폭주에 영향 0 — single binary 단발적 증분, `-j 1` 강제와 무관". |

**Round 2 closure summary**: 0 CRITICAL + 2 NEW MAJOR + 1 NEW MINOR = **3 finding 모두 reflected**, rejection 0. Round 3 critic 의 verify review 준비 완료.
