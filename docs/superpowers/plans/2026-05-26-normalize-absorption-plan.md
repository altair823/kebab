---
status: open
target_version: 0.19.0
spec: docs/superpowers/specs/2026-05-26-normalize-absorption-spec.md
contract_sections: ["§3.7b", "§8"]
related_specs:
  - docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
  - docs/superpowers/specs/2026-05-26-source-fs-dep-lightening-spec.md
sibling_plan: docs/superpowers/plans/2026-05-26-source-fs-dep-lightening-plan.md
---

# kebab-normalize + kebab-parse-types 흡수 — implementation plan

> spec round 3 APPROVE 직후의 plan (revision 2 — 10 step → **15 step** decompose). spec §3.1-§3.10 의 결정 + §3.5/§3.6 의 design contract diff + §3.7 (a)-(g) callsite migration + §3.9 의 HOTFIXES wording + §5.1-§5.11 의 verification gate 가 step 단위로 분산. design + doc 갱신 (구 Step 9) 을 4 step (§3.7b 재작성 / §8 graph / ARCHITECTURE + INDEX + HOTFIXES + HANDOFF / version 사이드카 verify) 으로 split — critic + verifier 의 closure granularity 향상.

## §0 Pre-flight + branch state

- **Branch**: `refactor/normalize-absorption` (현재 위치, main `d4395a3` 위에서 분기 완료).
- **Base SHA**: `d4395a3` (PR #185 sibling — source-fs dep lightening 머지 직후, v0.18.0 cut 완료 시점).
- **Working dir**: `/home/altair823/kebab`.
- **Env 강제** (CLAUDE.md disk-protection — `~/.claude/CLAUDE.md` 의 "Disk Layout — 루트 디스크 보호가 최우선" 룰):
  - `export CARGO_TARGET_DIR=/build/out/cargo-target/target` — 본 plan 의 모든 cargo 명령에 적용. `target/` 가 repo root 아래에 생성되지 않게 (16 GB RAM 머신 의 `/` 250 G 보호).
  - `export TMPDIR=/build/cache/tmp` — 대용량 임시 파일 발생 시 보호.
- **Cargo build 직렬화** (CLAUDE.md "Build / test / lint" + MEMORY.md `feedback_serial_build_only.md`):
  - 모든 cargo 명령 `-j 1` 강제. 18 integration-test binary 동시 link 시 OOM (linker SIGKILL).
  - per-crate `cargo test -p <crate>` 는 `-j 1` 없어도 OK 이나 일관성을 위해 명시.
  - cargo test / clippy / build 동시 background 실행 금지. 하나 끝난 후 다음.
- **`target/` clean policy** (CLAUDE.md 룰 + spec §5.10): full workspace test 직전 `cargo clean` 1회 (Step 15). 중간 step (Step 2-14) 에서는 per-crate build 만 — `cargo clean` 불필요 (incremental cache 활용).
- **HOTFIXES.md / HANDOFF.md / README.md 변경 0** (spec §7 명시) — HOTFIXES.md 와 HANDOFF.md 는 본 plan 의 Step 14 에서 추가 (README 는 변경 0).
- **4 frozen task spec (p1-2, p1-3, p1-4, p9-fb-07) 변경 0** + ~25 referencing task spec mechanical update 0 (spec §2 + §5.7).
- **wire schema 변경 0** (spec §1.9 verified, 16 wire schema 0 hit).
- **workspace `Cargo.toml` version bump 0.18.0 → 0.19.0** (Step 10 Hunk (b)).

## §1 Approach summary

Spec §3 의 핵심 sequencing — destination = `kebab-parse-md` (spec §3.1, Option A):

1. **신규 module 부터 작성** (Step 2-3) — `kebab-parse-md/src/types.rs` (parse-types 98 LOC 1:1 이식) + `kebab-parse-md/src/normalize.rs` (normalize 의 production fn body 이식, 4 hard-coded agent literal + tracing target literal 모두 보존).
2. **`lib.rs` 에 module + pub explicit re-export 등록** (Step 4) — 5 사용 type + 3 forward-declared struct + `build_canonical_document` / `derive_title` 의 surface 보존 (spec §3.3). explicit (glob 아님) — Q3 closure.
3. **`blocks.rs` + `frontmatter.rs` use 갱신** (Step 5) — `use kebab_parse_types::*` → `use crate::types::*`. 동일 crate 내 in-source ref shift.
4. **`kebab-app` callsite + dep cleanup** (Step 6-7, atomic 2 step) — `lib.rs:51` use statement + `lib.rs:1119` context string (Step 6) → `Cargo.toml` 의 2 dep 제거 (Step 7, `kebab-normalize` regular + `kebab-parse-types` dead regular — spec §3.10 incidental cleanup).
5. **dev-dep migration** (Step 8) — `kebab-chunk` + `kebab-store-sqlite` 의 `kebab-normalize` dev-dep 제거 (이미 `kebab-parse-md` dev-dep 보유). 통합 test source 의 `use kebab_normalize::*;` → `use kebab_parse_md::*;`.
6. **test file 이동 + 자기 참조 verify** (Step 9) — `kebab-normalize/tests/normalize_snapshot.rs` → `kebab-parse-md/tests/normalize_snapshot.rs`. 자기 참조 dev-dep declare 없는 cargo standard behavior verify.
7. **workspace `Cargo.toml` 갱신 — anchor step** (Step 10) — Hunk (a) `members` 의 2 entry 삭제 + Hunk (b) `workspace.package.version` 0.18.0 → 0.19.0 (NIT #N6 closure).
8. **`kebab-normalize/` + `kebab-parse-types/` 디렉토리 삭제** (Step 11) — `git rm -r`. workspace 가 22 crate 로 collapse.
9. **design §3.7b 재작성** (Step 12) — spec §3.5 의 4-단락 wording 으로 design contract 의 line 703-764 replace.
10. **design §8 graph 갱신** (Step 13) — spec §3.6 의 diff (3 edge 제거 + 2 forbidden bullet 의미 갱신 + commentary) 적용.
11. **ARCHITECTURE + INDEX + HOTFIXES + HANDOFF 갱신** (Step 14) — 4 doc 의 mechanical update. INDEX.md 의 "Future work / deferred" 섹션 신설 (현재 부재 — §11.7).
12. **workspace 회귀 + clean commit — closure** (Step 15) — `cargo clean` + 7 cargo gate + wire diff verify + 1 clean commit.

핵심 ordering invariant:

- **Step 2-3 < Step 4-5**: destination module file 생성 후 lib.rs re-export — 둘 동시 commit 시 cargo build green 보장.
- **Step 4-5 < Step 6-7**: in-crate ref shift 후 kebab-app callsite + dep — 외부 caller redirect 완료 후 dep 제거.
- **Step 6-7 < Step 8**: kebab-app 정합 후 dev-dep migration — order independent 이나 sequential gate 분리.
- **Step 8 < Step 9**: dev-dep cleanup 후 test file 이동 — git mv 가 git 의 add/remove 동시 commit 시 file content 보존.
- **Step 6-9 < Step 10**: 모든 caller redirect 후 workspace.members 삭제 — 중간 build 깨짐 방지.
- **Step 10 < Step 11**: workspace.members 제거 후 디렉토리 삭제 — stale path 회피.
- **Step 11 < Step 12-14**: production code 갱신 완료 후 design contract + doc 갱신 — reality ≡ contract.
- **Step 12-14 < Step 15**: 모든 doc 갱신 후 회귀 + commit — single clean commit 의 일관성.

## §2 Steps (15 steps)

### Step 1: Pre-flight baseline 측정 + env 확인

- **Files affected**: 변경 0 (측정 only).
- **Action**:
  - `cd /home/altair823/kebab && git rev-parse HEAD` → `d4395a3` 또는 그 위 commit 확인 (refactor branch 의 base).
  - env 확인: `echo $CARGO_TARGET_DIR` 가 `/build/out/cargo-target/target` 인지. 비어있으면 §0 의 export 적용.
  - workspace baseline count 측정: `cargo metadata --no-deps --format-version 1 | jq '.workspace_members | length'` → **24** (현재 시점).
  - dead `kebab-parse-types` regular dep verify (spec §3.10): `grep -n "kebab-parse-types" crates/kebab-app/Cargo.toml` → 1 hit (line 16 부근), `grep -rn "kebab_parse_types" crates/kebab-app/src/` → 0 hit.
  - baseline test count 측정 + persist (MAJOR GAP3 closure — spec §5.1 의 1313 + α 의 unit 정합화: test *함수 수* sum, NOT *binary 수* 행수):
    ```bash
    $ mkdir -p .omc/state
    $ cargo test --workspace --no-fail-fast -j 1 2>&1 \
        | awk '/^test result: ok\./ {for(i=1;i<=NF;i++) if($i=="passed;") sum += $(i-1)} END {print sum}' \
        > .omc/state/normalize-absorption-baseline.txt
    $ cat .omc/state/normalize-absorption-baseline.txt
    1313  # 예상 (spec §5.1)
    ```
    본 file 은 Step 15 의 numeric compare gate 의 source-of-truth. `.omc/state/` 는 git untracked (gitignore default 또는 `.omc/.gitignore`).
- **Exit gate (cargo cli — falsifiable / observable / idempotent / scope-correct)**:
  - `cargo metadata --no-deps --format-version 1 | jq '.workspace_members | length'` = **24** (observable, idempotent).
  - `cargo build --workspace -j 1 2>&1 | tail -5` 의 마지막 라인 = `Finished` 또는 `Compiling` (현 시점 baseline green — falsifiable).
- **Spec 참조**: §5.1 (baseline), §3.10 (dead dep verify).

### Step 2: 신규 `crates/kebab-parse-md/src/types.rs` 생성 (98 LOC 1:1 이식)

- **Files affected**:
  - `crates/kebab-parse-md/src/types.rs` (신규, ≈ 98 LOC, byte-identical 이식).
- **Action**:
  - `cp crates/kebab-parse-types/src/lib.rs crates/kebab-parse-md/src/types.rs` 또는 Write tool 로 신규 생성.
  - 본문 = `kebab-parse-types/src/lib.rs` 의 98 LOC 의 1:1 이식:
    - 5 사용 type (`ParsedBlock` + `ParsedBlockKind` + `ParsedPayload` + `Warning` + `WarningKind`) 의 serde 표현 + variant 명 byte-identical 보존 (spec §2.2 의 non-goal — 의미 변경 0).
    - 3 forward-declared struct (`ParsedImageRegion` + `ParsedPdfPage` + `ParsedAudioSegment`) 보존 (spec §3.3 + §11.5 의 future surface).
    - module-level doc 의 §3.7b reference 보존 + 한 줄 추가: `//! v0.19.0 부터 kebab-parse-md 의 in-crate module (이전 별 crate kebab-parse-types — HOTFIXES.md 2026-05-26 참조).`
  - **본 step 은 *추가* only** — 기존 `crates/kebab-parse-types/` 는 아직 alive. lib.rs 의 mod declare 가 없으므로 cargo 가 `types.rs` 무시 → build 깨지지 않음.
- **Exit gate**:
  - `wc -l crates/kebab-parse-md/src/types.rs` ≈ 98 (≤ 105) — falsifiable.
  - `grep -c "ParsedBlock\|ParsedBlockKind\|ParsedPayload\|Warning\|WarningKind\|ParsedImageRegion\|ParsedPdfPage\|ParsedAudioSegment" crates/kebab-parse-md/src/types.rs` ≥ 8 — 8 type 모두 등장.
  - `cargo build -p kebab-parse-md -j 1 2>&1 | tail -3` 의 마지막 라인 = `Finished` (mod declare 없으므로 file 무시).
- **Spec 참조**: §3.2 (module placement), §3.3 (visibility), §11.5 (future surface 보존).

### Step 3: 신규 `crates/kebab-parse-md/src/normalize.rs` 생성 + `Cargo.toml` dep 갱신 (1097 LOC 이식 + literal 보존 + dep migration)

- **Files affected**:
  - `crates/kebab-parse-md/src/normalize.rs` (신규, ≈ 1097 LOC, byte-identical 이식 + literal 보존 정책).
  - `crates/kebab-parse-md/Cargo.toml` (dep 갱신 — CRITICAL #1 + GAP2 closure).
- **Action**:
  - **(a) normalize.rs 생성** — `cp crates/kebab-normalize/src/lib.rs crates/kebab-parse-md/src/normalize.rs` 또는 Write 로 신규 생성.
  - 본문 = `kebab-normalize/src/lib.rs` 의 1097 LOC 의 production fn + comment + cfg(test) unit tests 1:1 이식:
    - `build_canonical_document` (signature `(asset: &RawAsset, metadata: Metadata, blocks: Vec<ParsedBlock>, parser_version: &ParserVersion, warnings: Vec<Warning>) -> Result<CanonicalDocument>`) 의 production body — spec §1.5 의 actual source byte-identical.
    - `derive_title(frontmatter_title: &str, blocks: &[Block], file_stem: &str) -> String` — spec §1.5 의 `blocks: &[Block]` (lifted, NOT ParsedBlock) inline 주석 보존.
    - `warning_agent(kind: &WarningKind) -> &'static str` 의 4 variant `"kb-parse-md"` 단일 return body 보존 (spec §3.7e + NEW MAJOR #N2 closure).
  - **(b) literal 보존 (CRITICAL — spec §3.7 (e)(g) + §1.9 의 6-row trace table)**:
    | line (post-port) | string | 보존? |
    |---|---|---|
    | `:109` (approx) | `target: "kebab-normalize"` (tracing::debug! literal) | ★ 보존 |
    | `:122` (approx) | `"kb-source-fs"` (Discovered event agent) | 보존 |
    | `:128` (approx) | `"kb-parse-md"` (Parsed event agent) | 보존 |
    | `:134` (approx) | `"kb-normalize"` (Normalized event agent) | ★ 보존 |
    | `:143` (approx) | `warning_agent(&w.kind).to_string()` (Warning event agent — 동적, "kb-parse-md" 단일 return) | 보존 |
    | `:153` (approx) | `"kb-normalize"` (lift_warnings event agent) | ★ 보존 |
  - **(c) in-source ref shift — 3 갈래** (actual `crates/kebab-normalize/src/lib.rs` grep 결과 기반):
    ```diff
    -use kebab_parse_types::{ParsedBlock, ParsedPayload, Warning, WarningKind};
    +use crate::types::{ParsedBlock, ParsedPayload, Warning, WarningKind};
    ```
    추가로 cfg(test) mod tests 안의 **9 hit fully-qualified call** 도 갱신 (lib.rs:489, :498, :507, :516, :525, :818, :862, :1070 — 모두 `kebab_parse_types::ParsedBlockKind::*` 패턴):
    ```diff
    -kind: kebab_parse_types::ParsedBlockKind::Paragraph,
    +kind: crate::types::ParsedBlockKind::Paragraph,
    ```
    `sed -i 's/kebab_parse_types::/crate::types::/g' crates/kebab-parse-md/src/normalize.rs` (or Edit `replace_all`) — 9 hit 모두 mechanical 변경.
  - **(d) `pub use kebab_core::{id_for_block, id_for_doc}` 제거 + in-body call 의 unqualified import 보존** (CRITICAL #2 closure):
    actual `lib.rs:33` 의 `pub use` 는 *re-export + 동시 current module scope import*. line 67 (`id_for_doc(...)`) + line 241 (`id_for_block(...)`) 의 unqualified call 이 `pub use` 의 scope import 에 의존. `pub use` 만 제거 시 unqualified call unresolved → compile error.
    ```diff
    -pub use kebab_core::{id_for_block, id_for_doc};
    +// (re-export 제거 — spec §3.3 R10 decision; production caller 0 verified.
    +//  in-body unqualified call 은 아래 use block 의 import 로 대체.)
    ```
    그리고 같은 file 의 *기존* `use kebab_core::{...}` block 에 `id_for_block, id_for_doc` 추가 (실제 use block 의 정확한 위치는 actual `lib.rs` 의 `use kebab_core::{` block 검색 후 정정 — Block / Metadata / SourceSpan 등의 normal import 와 함께 묶음):
    ```diff
     use kebab_core::{
         Block, BlockId, ...,
    +    id_for_block, id_for_doc,   // ← pub use 제거 후 in-body unqualified call 보존 (line 67, 241)
         ...
     };
    ```
  - **(e) `crates/kebab-parse-md/Cargo.toml` dep 갱신** (CRITICAL #1 + GAP2 closure — spec §3.4):
    ```diff
     [package]
     name = "kebab-parse-md"
     ...
    -description = "Markdown frontmatter and block parsing into kb-core::Metadata / kb-parse-types intermediates"
    +description = "Markdown frontmatter + block parsing + canonical-document lift (absorbed kb-parse-types + kb-normalize, see HOTFIXES.md 2026-05-26)"

     [dependencies]
     kebab-core = { path = "../kebab-core" }
    -kebab-parse-types = { path = "../kebab-parse-types" }
     anyhow          = { workspace = true }
     serde           = { workspace = true }
     serde_json      = { workspace = true }
     time            = { workspace = true }
     tracing         = { workspace = true }
    +# 흡수된 kb-normalize 의 NFKC 의존 — actual kebab-normalize/src/lib.rs:31 의
    +# `use unicode_normalization::UnicodeNormalization;` 이 normalize.rs 이식 시 동반.
    +# 이미 kebab-app 도 사용 중인 0.1 major (version drift 0).
    +unicode-normalization = "0.1"
     pulldown-cmark  = { version = "0.13", default-features = false }
     ...
    ```
  - **본 step 도 *추가* only** — 기존 `crates/kebab-normalize/` 는 아직 alive. lib.rs 의 mod declare (Step 4) 가 없으므로 normalize.rs file 무시되어 build 영향 0. 단 (e) 의 Cargo.toml 갱신은 Step 5/6 의 use shift 직후 fully active.
- **Exit gate**:
  - `wc -l crates/kebab-parse-md/src/normalize.rs` ≈ 1097 (≤ 1110).
  - `grep -c 'target: "kebab-normalize"' crates/kebab-parse-md/src/normalize.rs` = 1 — tracing target literal 보존 (spec §3.7 (g) + R8 mitigation).
  - production agent literal 의 정확한 검증 (MINOR GAP6 — production body 의 `agent: "kb-..."` 패턴 grep): `grep -cE 'agent:\s*"kb-(source-fs|parse-md|normalize)"\.to_string\(\)' crates/kebab-parse-md/src/normalize.rs` ≥ 3 — 3 hard-coded literal (Discovered + Parsed + Normalized) 의 production body emission. warning_agent body 의 4 `"kb-parse-md"` return + lift_warnings 의 `"kb-normalize"` 합쳐 production agent literal 5 hit 보장.
  - `grep -c "use crate::types::" crates/kebab-parse-md/src/normalize.rs` ≥ 1 — in-source ref shift.
  - `grep -c "kebab_parse_types::" crates/kebab-parse-md/src/normalize.rs` = 0 — fully-qualified 9 hit 모두 갱신 (MAJOR #5 closure 의 normalize.rs 쪽).
  - `grep -c "pub use kebab_core::" crates/kebab-parse-md/src/normalize.rs` = 0 — re-export 제거 (spec §3.3 R10).
  - `grep -E "use kebab_core::\{" crates/kebab-parse-md/src/normalize.rs | grep -c "id_for_block\|id_for_doc"` ≥ 1 — id_for_* unqualified import 보존 (CRITICAL #2 closure).
  - `grep -c "kebab-parse-types" crates/kebab-parse-md/Cargo.toml` = 0 — Cargo.toml dep 제거.
  - `grep -c "unicode-normalization" crates/kebab-parse-md/Cargo.toml` ≥ 1 — Cargo.toml dep 추가 (CRITICAL #1 + GAP2 closure).
  - `cargo build -p kebab-parse-md -j 1` green (mod declare 없으므로 normalize.rs file 무시. Cargo.toml dep 만 active).
- **Spec 참조**: §3.2 (module placement), §3.3 (visibility), §3.4 (Cargo.toml dep diff), §3.7 (c)(d)(e)(g) (in-source ref + id_for_* + literal), §1.5 (signature), §1.9 (6-row trace), §3.3 R10 (id_for_*), CRITICAL #1 + #2 + NEW MAJOR #N1 + #N2 + MINOR GAP6 closure.

### Step 4: `crates/kebab-parse-md/src/lib.rs` 갱신 (module declare + pub explicit re-export)

- **Files affected**: `crates/kebab-parse-md/src/lib.rs`.
- **Action**:
  - module-level doc 의 마지막에 한 줄 추가:
    ```rust
    //! v0.19.0 부터 `types` + `normalize` module 은 in-crate 흡수
    //! (`kebab-parse-types` + `kebab-normalize` 의 historical crate 가 본 crate 로
    //! collapse — see HOTFIXES.md 2026-05-26).
    ```
  - module declare + pub explicit re-export 추가 (spec §3.3 + §4.2 Q3 — glob 아님):
    ```rust
    mod types;
    mod normalize;

    // Spec §3.3 의 surface 보존 정책 — explicit (NOT glob) 으로 future addition leak 방지.
    pub use crate::types::{
        // 5 사용 type
        ParsedBlock, ParsedBlockKind, ParsedPayload,
        Warning, WarningKind,
        // 3 forward-declared struct (보존 — spec §3.3 + §11.5 future surface)
        ParsedImageRegion, ParsedPdfPage, ParsedAudioSegment,
    };
    pub use crate::normalize::{build_canonical_document, derive_title};
    ```
- **Exit gate**:
  - `grep -c "^mod types;\|^mod normalize;" crates/kebab-parse-md/src/lib.rs` = 2.
  - `grep "pub use crate::types" crates/kebab-parse-md/src/lib.rs | grep -c "ParsedImageRegion\|ParsedPdfPage\|ParsedAudioSegment"` ≥ 1 — 3 forward-declared struct 의 explicit re-export 확인.
  - `grep "pub use crate::normalize" crates/kebab-parse-md/src/lib.rs | grep -c "build_canonical_document\|derive_title"` ≥ 1.
  - `cargo build -p kebab-parse-md -j 1` green — 모듈 활성화 후 cargo 가 types.rs + normalize.rs 컴파일.
  - `cargo test -p kebab-parse-md -j 1` green — 이식된 normalize unit test (~700 LOC) 도 in-crate 통과.
- **Spec 참조**: §3.3 (visibility 정책), §4.2 Q3 (explicit vs glob — explicit 선택), §11.5 (future surface 보존).

### Step 5: `crates/kebab-parse-md/src/{blocks,frontmatter}.rs` + `tests/{blocks_snapshots,frontmatter_snapshots}.rs` ref shift

- **Files affected**:
  - `crates/kebab-parse-md/src/blocks.rs` (4 hit — actual line 1, 25, 37, 1589 verified).
  - `crates/kebab-parse-md/src/frontmatter.rs` (1+ hit — actual line 22 verified).
  - `crates/kebab-parse-md/tests/blocks_snapshots.rs` (BLOCKER #1 — actual line 19 verified).
  - `crates/kebab-parse-md/tests/frontmatter_snapshots.rs` (BLOCKER #1 — actual line 23 verified).
- **Action**:
  - **(a) blocks.rs — file-wide replace (MAJOR #5 closure, 4 hit)**:
    - line 1 doc comment: `//! Markdown body → flat \`Vec<kebab_parse_types::ParsedBlock>\` (§3.4 / §3.7b).` → `//! Markdown body → flat \`Vec<crate::types::ParsedBlock>\` (§3.4 / §3.7b).`
    - line 25 doc-link: `[\`kebab_parse_types::ParsedPayload::ImageRef\`]` → `[\`crate::types::ParsedPayload::ImageRef\`]`
    - line 37 use: `use kebab_parse_types::*;` (또는 explicit list) → `use crate::types::*;` (또는 explicit list 갱신)
    - line 1589 production fully-qualified call: `kebab_parse_types::ParsedBlockKind::List` → `crate::types::ParsedBlockKind::List`
    - Edit tool 의 `replace_all: true` 로 `kebab_parse_types::` → `crate::types::` 단일 치환 가능 (4 hit + 가능한 hidden hit 모두 mechanical 갱신).
  - **(b) frontmatter.rs — 동일 패턴**:
    ```diff
    -use kebab_parse_types::{Warning, WarningKind};
    +use crate::types::{Warning, WarningKind};
    ```
    `MalformedFrontmatter` variant 의 fully-qualified call 등 추가 hit 검색 (`grep -n "kebab_parse_types" crates/kebab-parse-md/src/frontmatter.rs` 으로 확인 후 `replace_all`).
  - **(c) tests/blocks_snapshots.rs (BLOCKER #1 — integration test 는 `crate::` 사용 불가, `kebab_parse_md::*` 사용)**:
    ```diff
    -use kebab_parse_types::{ParsedBlock, Warning};
    +use kebab_parse_md::{ParsedBlock, Warning};
    ```
    line 19 의 use 갱신. integration test 는 자기 crate `lib` 자동 link → `kebab_parse_md::*` re-export 활성화 (Step 4 의 lib.rs explicit re-export 의 8 type 중 4 type 활용).
  - **(d) tests/frontmatter_snapshots.rs (BLOCKER #1)**:
    ```diff
    -warnings: Vec<kebab_parse_types::Warning>,
    +warnings: Vec<kebab_parse_md::Warning>,
    ```
    line 23 의 fully-qualified type 갱신 (function signature param). 추가 hit 검색 (`grep -n "kebab_parse_types" crates/kebab-parse-md/tests/frontmatter_snapshots.rs`) 후 replace_all.
- **Exit gate**:
  - `grep -rn "kebab_parse_types" crates/kebab-parse-md/` = **0 hit** (src/ + tests/ 모두 — verify cmd 의 src/ 한정 해제, BLOCKER #1 closure).
  - `grep -c "crate::types::" crates/kebab-parse-md/src/blocks.rs` ≥ 4 (line 1/25/37/1589 갱신).
  - `grep -c "kebab_parse_md::" crates/kebab-parse-md/tests/blocks_snapshots.rs crates/kebab-parse-md/tests/frontmatter_snapshots.rs` ≥ 2.
  - `cargo build -p kebab-parse-md -j 1` green.
  - `cargo test -p kebab-parse-md -j 1` green (integration test 의 fixture builder 가 self-link 통해 redirect).
- **Spec 참조**: §3.7 (c) callsite migration (in-source ref shift), BLOCKER #1 + MAJOR #5 closure.

### Step 6: `crates/kebab-app/src/lib.rs:51, :1119` callsite migration

- **Files affected**: `crates/kebab-app/src/lib.rs`.
- **Action**:
  - **(a) line 51 use statement** (spec §3.7 (a)):
    ```diff
    -use kebab_normalize::build_canonical_document;
    +use kebab_parse_md::build_canonical_document;
    ```
  - **(b) line 1119 context string** (spec §3.7 (b) + MAJOR #4 closure):
    ```diff
    -        .context("kb-normalize::build_canonical_document")?;
    +        .context("kb-parse-md::build_canonical_document")?;
    ```
    *주의*: `kb-parse-md::parse_frontmatter` (line 1091) + `kb-parse-md::parse_blocks` (line 1099) 의 context string 은 변경 없음 — byte-identical hunk 적용 금지 (MAJOR #4 의 closure 명시).
- **Exit gate**:
  - `grep -n "kebab_normalize" crates/kebab-app/src/lib.rs` = 0 hit (use 와 의 자취 모두 사라짐).
  - `grep -n "kb-normalize::" crates/kebab-app/src/lib.rs` = 0 hit (context string 갱신).
  - `cargo build -p kebab-app -j 1` green — kebab-parse-md 의 re-export 가 alive 인 상태에서 redirect.
  - `cargo test -p kebab-app -j 1` green.
- **Spec 참조**: §3.7 (a)(b), MAJOR #4 closure.

### Step 7: `crates/kebab-app/Cargo.toml` dep cleanup (2 dep 제거 — regular + dead incidental) — **closure pre-pivot 1**

- **Files affected**: `crates/kebab-app/Cargo.toml`.
- **Action** (spec §3.4 + §3.10):
  MINOR #3 closure — hunk 적용 전 actual context 확인 (sed 으로 idempotent):
  ```bash
  $ sed -n '11,20p' crates/kebab-app/Cargo.toml   # context 의 actual line 확인
  ```
  hunk:
  ```diff
   kebab-source-fs = { path = "../kebab-source-fs" }
   kebab-parse-md = { path = "../kebab-parse-md" }
  -kebab-parse-types = { path = "../kebab-parse-types" }
  -kebab-normalize = { path = "../kebab-normalize" }
   kebab-chunk = { path = "../kebab-chunk" }
  ```
  - `kebab-normalize` regular dep 제거 — Step 6 의 use shift 가 정합되어 더 이상 dep 불필요.
  - `kebab-parse-types` regular dep 제거 — *dead dep* (spec §3.10 incidental cleanup — Step 1 의 verify 에서 `kebab_parse_types` source import 0 hit 검증 완료).
- **Exit gate**:
  - `grep -E "kebab-normalize|kebab-parse-types" crates/kebab-app/Cargo.toml` = 0 hit.
  - `cargo build -p kebab-app -j 1` green.
  - `cargo tree -p kebab-app --depth 2 | grep -E "kebab_(parse_types|normalize)"` = 0 줄 — spec §5.2 의 anchor invariant.
- **Spec 참조**: §3.4 (Cargo.toml diff), §3.10 (incidental cleanup), §5.2 (anchor invariant).

### Step 8: `crates/kebab-chunk/Cargo.toml` + `crates/kebab-store-sqlite/Cargo.toml` dev-dep migration + 통합 test source `use` 갱신

- **Files affected**:
  - `crates/kebab-chunk/Cargo.toml`.
  - `crates/kebab-store-sqlite/Cargo.toml`.
  - `crates/kebab-chunk/tests/*.rs` (use statement shift).
  - `crates/kebab-store-sqlite/tests/*.rs` (use statement shift).
- **Action**:
  - **(a) `crates/kebab-chunk/Cargo.toml`** — spec §3.4:
    ```diff
     [dev-dependencies]
     # kb-parse-md / kb-normalize / kb-parse-code are dev-only — used by the
     # snapshot integration tests to build a CanonicalDocument from fixture files.
     # Forbidden as regular deps per design §8 (chunker consumes CanonicalDocument
     # from kb-core only); `cargo tree -p kb-chunk --depth 1` (default scope,
     # excludes dev-deps) confirms this.
     kebab-parse-md   = { path = "../kebab-parse-md" }
     kebab-parse-code = { path = "../kebab-parse-code" }
    -kebab-normalize  = { path = "../kebab-normalize" }
     serde_json       = { workspace = true }
     time             = { workspace = true }
    ```
    그리고 doc comment 의 `kb-parse-md / kb-normalize / kb-parse-code` mention 갱신 — `kb-parse-md / kb-parse-code` (kb-normalize 흡수 명시).
  - **(b) `crates/kebab-store-sqlite/Cargo.toml`** — spec §3.4:
    ```diff
     kebab-parse-md = { path = "../kebab-parse-md" }
    -kebab-normalize = { path = "../kebab-normalize" }
     kebab-chunk = { path = "../kebab-chunk" }
    ```
  - **(c) 통합 test source 의 use statement 갱신** — actual grep 결과 명시 (MAJOR #2 closure, 2 file:line):
    - `crates/kebab-chunk/tests/long_section_snapshot.rs:21` — `use kebab_normalize::build_canonical_document;` → `use kebab_parse_md::build_canonical_document;`
    - `crates/kebab-store-sqlite/tests/contract_roundtrip.rs:16` — `use kebab_normalize::build_canonical_document;` → `use kebab_parse_md::build_canonical_document;`
    추가 hit 검색 (`grep -rn "kebab_normalize" crates/kebab-chunk/tests/ crates/kebab-store-sqlite/tests/`) — 본 시점 의 verified hit = 2건. 새로 발견 시 동일 패턴으로 갱신.
- **Exit gate**:
  - `grep -l "kebab-normalize" crates/kebab-chunk/Cargo.toml crates/kebab-store-sqlite/Cargo.toml` = 0 line.
  - `grep -rn "kebab_normalize" crates/kebab-chunk/tests/ crates/kebab-store-sqlite/tests/` = 0 hit.
  - `cargo test -p kebab-chunk -j 1` green — snapshot integration test 의 fixture builder 가 destination 으로 redirect.
  - `cargo test -p kebab-store-sqlite -j 1` green — contract round-trip test.
- **Spec 참조**: §3.4 (Cargo.toml diff), MAJOR #11 closure.

### Step 9: test file 이동 (`kebab-normalize/tests/` → `kebab-parse-md/tests/`) + 자기 참조 verify

- **Files affected** (MAJOR #3 closure — actual `find` 결과 명시):
  - `crates/kebab-normalize/tests/normalize_snapshot.rs` → `crates/kebab-parse-md/tests/normalize_snapshot.rs`.
  - actual `find crates/kebab-normalize/tests/ -type f` 결과 = **`normalize_snapshot.rs` 단일 file** (sibling snapshots/ 또는 fixtures/ 서브디렉토리 부재). 본 step 의 mv 대상 = 1 file only.
- **Action**:
  - **(a) git mv 로 mechanical move**:
    ```bash
    # 실 실행 전 사전 verify (idempotent + observable):
    $ find crates/kebab-normalize/tests/ -type f
    crates/kebab-normalize/tests/normalize_snapshot.rs

    # 단일 file mv (sibling fixture 부재 확인됨):
    $ git mv crates/kebab-normalize/tests/normalize_snapshot.rs crates/kebab-parse-md/tests/normalize_snapshot.rs
    ```
  - **(b) test file 의 use statement 갱신**:
    - 기존 `use kebab_normalize::*;` 또는 `use kebab_normalize::{build_canonical_document, derive_title};` → `use kebab_parse_md::{build_canonical_document, derive_title};` 또는 in-crate `crate::*` 사용.
    - `use kebab_parse_md::*;` 가 이미 alive 이면 keep (자기 crate import 는 integration test 에서 cargo standard behavior 로 자동 link — spec §3.7 (f), R3 / Q4 closure).
    - `kebab_parse_types::*` 의 import 도 `kebab_parse_md::*` re-export 로 갈음.
  - **(c) 자기 참조 dev-dep declare 제거 verify**: 본 step 이후 `kebab-normalize/` 디렉토리 자체가 Step 11 에서 삭제되므로 그 안의 Cargo.toml 의 `kebab-parse-md = { path = "..." }` dev-dep 도 vanish. 새 destination `kebab-parse-md/Cargo.toml` 에 *자기 참조* dev-dep 을 add 하지 않음 — cargo standard behavior (integration test 가 lib 자동 link).
- **Exit gate**:
  - `ls crates/kebab-parse-md/tests/normalize_snapshot.rs` 존재.
  - `ls crates/kebab-normalize/tests/normalize_snapshot.rs 2>&1 | grep -c "No such"` = 1 (이동 완료).
  - `cargo test -p kebab-parse-md --test normalize_snapshot -j 1` green — spec §3.7 (f) 의 cargo standard behavior verified (R3/Q4 closure).
  - `grep "kebab-parse-md = " crates/kebab-parse-md/Cargo.toml` = 0 hit (자기 참조 add 안 했는지 verify).
- **Spec 참조**: §3.7 (f) (test file 이동 + cargo standard behavior), R3 (Q4 closure).

### Step 10: workspace `Cargo.toml` Hunk (a) members + Hunk (b) version — anchor step

- **Files affected**: `Cargo.toml` (workspace root).
- **Action** (spec §3.8 + NIT #N6 closure — 2 hunk 분리):
  - **Hunk (a) — `[workspace] members` 의 2 entry 삭제**:
    ```diff
     [workspace]
     resolver = "3"
     members = [
         "crates/kebab-core",
    -    "crates/kebab-parse-types",
         "crates/kebab-config",
         "crates/kebab-source-fs",
         "crates/kebab-parse-md",
    -    "crates/kebab-normalize",
         "crates/kebab-chunk",
         ...
     ]
    ```
  - **Hunk (b) — `[workspace.package] version` 1-line 변경**:
    ```diff
     [workspace.package]
     edition       = "2024"
     rust-version  = "1.85"
     license       = "MIT OR Apache-2.0"
     repository    = "https://github.com/altair823/org/kebab"
    -version       = "0.18.0"
    +version       = "0.19.0"   # frozen design contract (§3.7b 재작성) 변경 trigger — CLAUDE.md "Release / binary version bump"
    ```
  - 두 hunk 는 sequential 또는 parallel 적용 가능 — line context 가 충분히 분리되어 있음 (NIT #N6 의 의도).
- **Exit gate**:
  - `cargo metadata --no-deps --format-version 1 | jq '.workspace_members | length'` = **22** (spec §5.2 의 robust 명령).
  - `grep '^version' Cargo.toml | head -1` = `version       = "0.19.0"`.
  - `cargo build --workspace -j 1` **green** — Step 6-9 가 모든 dep path 참조 제거 완료한 상태이므로, members 에서 제외된 두 orphan 디렉토리 (`crates/kebab-normalize/` + `crates/kebab-parse-types/`) 는 cargo 가 silently ignore. Cargo.lock 의 `[[package]]` entry 는 Step 11 직후 first `cargo build` 에서 자동 cleanup (MAJOR #1 closure — self-contradictory wording 제거).
- **Spec 참조**: §3.8 (Hunk a + b), NIT #N6 closure, §5.2 (workspace count invariant).

### Step 11: `crates/kebab-normalize/` + `crates/kebab-parse-types/` 디렉토리 삭제

- **Files affected**:
  - `crates/kebab-normalize/` (전체 디렉토리).
  - `crates/kebab-parse-types/` (전체 디렉토리).
- **Action**:
  - `git rm -r crates/kebab-normalize/`
  - `git rm -r crates/kebab-parse-types/`
  - `cargo build --workspace -j 1` 의 자동 Cargo.lock cleanup 으로 두 crate 의 `[[package]]` entry 사라짐 (Step 15 의 verification).
- **Exit gate**:
  - `ls crates/kebab-normalize/ 2>&1 | grep -c "No such"` = 1.
  - `ls crates/kebab-parse-types/ 2>&1 | grep -c "No such"` = 1.
  - `ls -d crates/*/ | wc -l` = **22** (spec §5.2 의 secondary robust cmd).
  - `cargo build --workspace -j 1` green — 모든 dep 정합.
- **Spec 참조**: §3.8 (디렉토리 삭제).

### Step 12: design §3.7b 4-단락 재작성 (`docs/superpowers/specs/2026-04-27-kebab-final-form-design.md` line 703-764)

- **Files affected**: `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md` (§3.7b 부분).
- **Action**:
  - line 703-764 의 §3.7b 본문을 spec §3.5 의 wording 으로 **replace** (strike 아닌 재작성).
  - 4-단락 구조:
    1. **원래 의도** (v0.1~v0.18 머지 시점) — thin layer + medium-agnostic lift.
    2. **현재 상태 (v0.19.0~)** — 흡수 근거 (4 parser 중 1개만 lift 경유, fan-in/fan-out 모두 1).
    3. **보존된 surface** — 5 사용 type + 3 forward-declared struct → `kebab-parse-md` 의 `pub` re-export.
    4. **future re-extraction trigger** — 3 조건 명시 (fan-in ≥ 2 회복, ParsedBlock 변종 emit, medium-agnostic lift 일반화).
  - 의존 그래프 ascii 갱신 (post-absorb):
    ```text
    kebab-core (도메인 모델 — Block, Chunk, SourceSpan, IDs, …)
       ▲
       │
    kebab-parse-md (markdown 의 frontmatter + block + types + normalize, 모두 in-crate)
       ▲
       │
    kebab-parse-pdf, kebab-parse-image, kebab-parse-code (자체 CanonicalDocument emit)
    ```
- **Exit gate**:
  - `sed -n '703,770p' docs/superpowers/specs/2026-04-27-kebab-final-form-design.md | grep -c "원래 의도\|현재 상태\|보존된 surface\|future re-extraction"` ≥ 4 — 4 단락 모두 존재.
  - `sed -n '703,770p' docs/superpowers/specs/2026-04-27-kebab-final-form-design.md | grep -c "thin layer\|fan-in"` ≥ 1 — historical intent 의 wording 보존.
  - `git diff main..HEAD -- docs/superpowers/specs/2026-04-27-kebab-final-form-design.md | head -100` 의 hunk 가 line 703-764 만 touch (spec §5.6).
- **Spec 참조**: §3.5 (4-단락 wording), §5.6 (design doc 갱신 검증).

### Step 13: design §8 graph 갱신 (line 1457-1491) — 3 edge 제거 + 2 forbidden bullet 의미 갱신

- **Files affected**: `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md` (§8 부분).
- **Action** (spec §3.6):
  - **graph diff** (3 edge 제거):
    ```diff
    -         ├─> kebab-parse-md / kebab-parse-pdf / kebab-parse-image / kebab-parse-audio
    -         │     └─> kebab-parse-types (parser intermediate)
    +         ├─> kebab-parse-md
    +         │     (post-v0.19.0: absorbed kebab-parse-types + kebab-normalize — §3.7b)
    +         ├─> kebab-parse-pdf / kebab-parse-image (self-emit CanonicalDocument)
              ├─> kebab-parse-code
              │     └─> kebab-core (domain types only — NO store/embed/llm/rag/UI)
    -         ├─> kebab-normalize
    -         │     └─> kebab-parse-types
              ├─> kebab-chunk
    ```
  - **commentary 갱신** (§3.7b reference 의 thin layer wording 폐기):
    ```diff
    -`kebab-parse-types` 는 `kebab-core` 와 parsers/normalize 사이의 thin layer (§3.7b 참조).
    +`kebab-parse-md` 는 v0.19.0 부터 `kebab-parse-types` (parser intermediate types) 와 `kebab-normalize` (CanonicalDocument lift) 를 흡수한다 (§3.7b 참조). 4 parser 중 markdown 한 갈래만 lift 를 경유하므로 thin layer 의 가치가 의미를 잃었다. 보존된 5 사용 type + 3 forward-declared struct 의 surface 는 `kebab-parse-md` 의 `pub` re-export 로 backward-compat.
    ```
  - **forbidden bullet 갱신**:
    ```diff
     - UI → store/llm/parse 직접 의존 ✗
     - parse-* → store/llm/embed ✗
    -- parse-* → kebab-normalize ✗ (단방향: parsers → kebab-parse-types ← normalize)
    +- parse-* (pdf/image/code) → kebab-parse-md ✗ (parser 끼리 cross-import 금지 — markdown 의 lift 가 다른 parser 에 노출되면 안 됨)
     - chunk → llm/embed ✗
    -- normalize → store / parse-* ✗
    -- kebab-parse-types → 어떤 parser/normalize/store/llm/embed/search/rag/ui ✗ (`kebab-core` 만 의존)
     - 다른 store 와 cross-write ✗
    ```
    *주의*: `kebab-parse-md → store / llm / embed ✗` 룰은 *추가 안 함* (MAJOR #5 closure — 기존 `parse-* → store/llm/embed ✗` 가 흡수된 lift 까지 자동 포함). 중복 룰 회피.
- **Exit gate**:
  - `sed -n '1457,1495p' docs/superpowers/specs/2026-04-27-kebab-final-form-design.md | grep -c "kebab-parse-types"` ≤ 1 — 본문 commentary 의 historical reference 만 보존 (또는 0).
  - `sed -n '1457,1495p' docs/superpowers/specs/2026-04-27-kebab-final-form-design.md | grep -c "kebab-normalize"` ≤ 1 — 동상.
  - `sed -n '1457,1495p' docs/superpowers/specs/2026-04-27-kebab-final-form-design.md | grep -c "parse-\\* (pdf/image/code) → kebab-parse-md ✗"` = 1 — 신규 forbidden bullet 존재.
  - `git diff main..HEAD -- docs/superpowers/specs/2026-04-27-kebab-final-form-design.md` 의 hunk 가 §3.7b (Step 12) + §8 (Step 13) 의 2 section 만 touch — MINOR GAP7 closure 의 truncate-free numeric verify:
    - `git diff main..HEAD -- docs/superpowers/specs/2026-04-27-kebab-final-form-design.md | grep -c "^@@"` = **2** (정확히 2 hunk).
    - `git diff main..HEAD -- docs/superpowers/specs/2026-04-27-kebab-final-form-design.md | grep -cE "^\+\+\+|^---"` = **2** (file header — 1 file × 2 = 2).
- **Spec 참조**: §3.6 (§8 graph diff), MAJOR #5 closure (중복 룰 회피).

### Step 14: ARCHITECTURE.md + tasks/INDEX.md (L169 closure + Future work 신설) + HOTFIXES.md (4-block) + HANDOFF.md (cross-link)

- **Files affected**:
  - `docs/ARCHITECTURE.md` (crate graph + directory tree).
  - `tasks/INDEX.md` (L169 closure mention + Future work 섹션 신설).
  - `tasks/HOTFIXES.md` (신규 entry — 4-block).
  - `HANDOFF.md` (1 줄 cross-link).
- **Action**:
  - **(a) `docs/ARCHITECTURE.md` 갱신** — spec §5.8:
    - crate graph 의 entries 24 → 22 — `kebab-parse-types` + `kebab-normalize` 절 삭제.
    - directory tree 의 `crates/kebab-parse-types/` + `crates/kebab-normalize/` 줄 삭제.
    - 흡수 mention 한 줄 추가 (또는 §3.7b strike 의 cross-link).
  - **(b) `tasks/INDEX.md` L169 closure** — spec §5.8:
    ```diff
    -  - **PR #181 chore: ... system-architect 의 component-level review 결론 = pre-cut nothing, all v0.18.1+ defer (kebab-normalize 흡수, Extractor dispatch unification, kebab-source-fs dep lightening 등).
    +  - **PR #181 chore: ... system-architect 의 component-level review 결론 = pre-cut nothing, all v0.18.1+ defer (kebab-normalize 흡수 — v0.19.0 closure, see HOTFIXES.md 2026-05-26; Extractor dispatch unification; kebab-source-fs dep lightening 등).
    ```
  - **(c) `tasks/INDEX.md` "Future work / deferred" 섹션 신설** — spec §11.7 + verified INDEX.md 에 현재 부재:
    - 위치: "## Post-merge 핫픽스" 와 "## 모든 task 공통 규약" 사이 (신설).
    - 본문:
      ```markdown
      ## Future work / deferred

      - v0.20+ image/pdf normalize integration — design §3.7b intent 미구현 (3 dead struct 보존). PR #186 (normalize-absorption) 의 spec §11 참조.
      ```
  - **(d) `tasks/HOTFIXES.md` 신규 entry** — spec §3.9 의 4-block 변형 그대로 (Symptom + Root cause + Action + Amends with inline Wire/surface impact). MAJOR #4 closure — anchor 검증 generic 화:
    ```bash
    # insertion point 의 generic verify (hard-coded entry 인용 회피):
    $ head -50 tasks/HOTFIXES.md   # frontmatter + heading + first entry 위치 확인
    $ FIRST_ENTRY_LINE=$(grep -n "^## 20" tasks/HOTFIXES.md | head -1 | cut -d: -f1)
    $ echo "insertion point = line ${FIRST_ENTRY_LINE} 의 *위* (chronological reverse — newest top)"
    ```
    본 시점 측정 결과 = line 17 (`## 2026-05-26 — S3 NLI unavailable`). 본 PR 의 entry 는 그 *위* 에 insert — 같은 2026-05-26 date 이지만 본 entry 는 source-fs sub-item 1 (PR #185) 의 sibling 인 sub-item 2 closure 라 chronologically later. Action 라인에 spec §11 cross-link 한 줄 포함 ("3 dead struct... 보존 — v0.20+ image/pdf normalize integration 의 future surface (spec §11 참조)").
  - **(e) `HANDOFF.md` cross-link 한 줄** — `## 머지 후 발견된 버그 / 결정` 섹션 의 가장 최근 entry 위:
    ```markdown
    - 2026-05-26 kebab-normalize + kebab-parse-types 흡수 (24 → 22 crates, design §3.7b 재작성). v0.19.0 cut. [HOTFIXES.md](tasks/HOTFIXES.md#2026-05-26--design-deviation--kebab-normalize--kebab-parse-types-흡수-24--22-crates).
    ```
- **Exit gate**:
  - `grep -c "Future work\|Future Work" tasks/INDEX.md` ≥ 1 — 신설 섹션 존재.
  - `grep -c "2026-05-26 — design deviation — kebab-normalize" tasks/HOTFIXES.md` = 1 — entry 추가.
  - `grep -c "image/pdf normalize integration" tasks/INDEX.md tasks/HOTFIXES.md` ≥ 2 — 두 doc 모두 cross-link.
  - `grep -c "2026-05-26 kebab-normalize" HANDOFF.md` = 1.
  - `grep -c "kebab-parse-types\|kebab-normalize" docs/ARCHITECTURE.md` ≤ 1 — historical mention 만 보존 (또는 0).
  - 4 frozen task spec (p1-2, p1-3, p1-4, p9-fb-07) `git diff main..HEAD` 의 path 에 0 hit.
- **Spec 참조**: §3.9 (HOTFIXES 4-block), §5.8 (ARCHITECTURE + INDEX), §11.7 (Future work 섹션 신설), §5.7 (frozen task spec invariant).

### Step 15: workspace 회귀 + 7 cargo gate + clean commit — closure

- **Files affected**: commit 만 (production code touch 0).
- **Action**:
  - **(a) `cargo clean`** — spec §5.10 (16 GB RAM pressure 회피).
  - **(b) full workspace test + numeric net-delta verify** (GAP4 closure — Step 1 의 baseline 과 numeric compare):
    ```bash
    $ POST_COUNT=$(cargo test --workspace --no-fail-fast -j 1 2>&1 \
        | awk '/^test result: ok\./ {for(i=1;i<=NF;i++) if($i=="passed;") sum += $(i-1)} END {print sum}')
    $ echo "POST_COUNT=$POST_COUNT"
    $ diff <(echo "$POST_COUNT") .omc/state/normalize-absorption-baseline.txt \
        && echo "✓ net delta = 0 (spec §5.1)" \
        || { echo "✗ net delta != 0 — REQUIRES INVESTIGATION"; exit 1; }
    ```
    spec §5.1 의 expected net delta = 0 (1:1 lift, signature 무변, test 의미 무변 — Step 9 의 file 이동도 in-crate test 로 collapse 이므로 함수 수 보존). 만약 +N intentional addition 시 (예: NEW MAJOR #N1 + #N2 의 literal 보존 regression pin 신규) plan §1 approach summary 에 명시 + diff 의 numeric tolerance update.
  - **(c) clippy gate** — `cargo clippy --workspace --all-targets -- -D warnings` 0 warning (spec §5.2).
  - **(d) cargo deny** — `cargo deny check` 0 error 0 warning (spec §5.9).
  - **(e) 22 crate workspace verify** — `cargo metadata --no-deps --format-version 1 | jq '.workspace_members | length'` = 22 (spec §5.2 + §5.8).
  - **(f) Cargo.lock 검증** — spec §5.11:
    ```bash
    $ grep '^name = "kebab-normalize"\|^name = "kebab-parse-types"' Cargo.lock
    (0 hit)
    $ awk '/^\[\[package\]\]/,/^$/{if(/name = "kebab-parse-md"/)f=1; if(f) print; if(/^$/ && f){f=0; print "---"}}' Cargo.lock | grep "unicode-normalization"
    unicode-normalization
    ```
  - **(g) dep tree invariant** — `cargo tree -p kebab-app --depth 2 | grep -E "kebab_(parse_types|normalize)"` = 0 줄.
  - **(h) wire schema diff = 0** — `git diff main..HEAD -- docs/wire-schema/v1/ | wc -l` = 0 (spec §5.4).
  - **(i) clean commit**:
    ```bash
    git add -A
    git status   # verify
    git commit -m "$(cat <<'EOF'
    refactor(parse-md): absorb kebab-normalize + kebab-parse-types — 24 → 22 crates + §3.7b 재작성

    design §3.7b 의 thin layer (ParsedBlock 류) 가 4 parser 중 1개 (markdown) 만 lift 를
    경유하는 현실 — fan-in/fan-out 모두 1 → layer 의미 잃음. kebab-normalize (1097 LOC)
    + kebab-parse-types (98 LOC) 둘을 kebab-parse-md 로 흡수.

    설계: docs/superpowers/specs/2026-05-26-normalize-absorption-spec.md
    플랜: docs/superpowers/plans/2026-05-26-normalize-absorption-plan.md
    HOTFIXES: tasks/HOTFIXES.md 의 2026-05-26 entry (design deviation)

    - 5 사용 type + 3 forward-declared struct → kebab-parse-md::types module 의 pub explicit re-export.
    - build_canonical_document + derive_title + warning_agent → kebab-parse-md::normalize module.
    - 4 hard-coded agent literal (lib.rs:122/128/134/153) + warning_agent body return + tracing target literal 모두 보존 — stage label 일관성.
    - kebab-app callsite (lib.rs:51 use + :1119 context string) + Cargo.toml 의 2 dep (regular + dead) 제거.
    - kebab-chunk + kebab-store-sqlite 의 [dev-dependencies] kebab-normalize → 제거 (kebab-parse-md 로 갈음). 통합 test source의 use shift.
    - test file 이동 (kebab-normalize/tests/normalize_snapshot.rs → kebab-parse-md/tests/).
    - workspace Cargo.toml: Hunk (a) members 2 entry 삭제 + Hunk (b) version 0.18.0 → 0.19.0 (frozen contract 변경).
    - design §3.7b 4-단락 재작성 (원래 intent 보존 + 현재 상태 + 보존된 surface + future re-extraction trigger).
    - design §8 graph 갱신 (3 edge 제거 + 2 forbidden bullet 의미 갱신 + commentary).
    - ARCHITECTURE.md crate graph + directory tree mechanical 갱신.
    - tasks/INDEX.md L169 closure mention + "Future work / deferred" 섹션 신설 (image/pdf normalize integration entry).
    - tasks/HOTFIXES.md 신규 entry (4-block — design deviation Symptom).
    - HANDOFF.md cross-link 한 줄.
    - 3 dead struct (ParsedImageRegion / ParsedPdfPage / ParsedAudioSegment) 는 보존 — v0.20+ image/pdf normalize integration 의 future surface (spec §11).

    Wire / surface impact: 0건. CLI / TUI / MCP / --json 출력 / config / XDG path /
    parser_version 모두 unchanged. wire-invisible provenance.events[].agent + tracing target
    literal "kb-normalize" 도 보존 — old DB row 와 new DB row 의 audit log 일관성.

    Verification: cargo test --workspace --no-fail-fast -j 1 green / cargo clippy --workspace
    --all-targets -- -D warnings 0 warning / cargo deny check 0 error / cargo metadata ...
    workspace_members | length = 22 / cargo tree -p kebab-app | grep kebab_parse_types
    + kebab_normalize = 0 줄.
    EOF
    )"
    ```
- **Exit gate**:
  - 모든 cargo gate green (a-h).
  - `git log --oneline -1` = 위 commit message 의 first line.
  - `git status` = clean (untracked file 0).
- **Spec 참조**: §5.1-§5.11 (모든 verification gate).

## §3 Step dependency graph

```text
Step 1 (Pre-flight baseline)
   │
   ▼
Step 2 (types.rs 신설) ──┐
   │                     │ parallel OK (둘 다 in-crate, lib.rs mod declare 아직 없음)
   ▼                     │
Step 3 (normalize.rs 신설 + literal 보존) ◄─┘
   │
   ▼
Step 4 (lib.rs mod + pub explicit re-export)
   │
   ▼
Step 5 (blocks.rs + frontmatter.rs use shift)
   │
   ▼
Step 6 (kebab-app callsite lib.rs:51 + :1119)
   │
   ▼
Step 7 (kebab-app Cargo.toml dep cleanup) — anchor 1
   │
   ▼
Step 8 (kebab-chunk + kebab-store-sqlite dev-dep migration)
   │
   ▼
Step 9 (test file 이동 + 자기 참조 verify)
   │
   ▼
Step 10 (workspace Cargo.toml Hunk a + b) — anchor 2
   │
   ▼
Step 11 (kebab-normalize/ + kebab-parse-types/ 디렉토리 삭제)
   │
   ▼
Step 12 (design §3.7b 4-단락 재작성)
   │
   ▼
Step 13 (design §8 graph 3 edge 제거 + 2 forbidden bullet 갱신)
   │
   ▼
Step 14 (ARCHITECTURE + INDEX + HOTFIXES + HANDOFF)
   │
   ▼
Step 15 (회귀 + 7 cargo gate + clean commit) — closure
```

핵심 invariant:

- **Step 2 + 3 < Step 4**: file 생성 후 lib.rs mod declare — 동시 commit 시 cargo 자동 link.
- **Step 4-5 < Step 6**: 동일 crate 내 ref shift 후 외부 caller redirect.
- **Step 6 < Step 7**: callsite migration 후 dep 제거 — kebab-app build green 보장.
- **Step 7 < Step 8**: kebab-app 정합 후 dev-dep migration (sequential gate 분리, 자체 dep 영향 0).
- **Step 8 < Step 9**: dev-dep cleanup 후 test file 이동.
- **Step 6-9 < Step 10**: anchor 1 (kebab-app) + dev-dep + test file 모두 정합 후 anchor 2 (workspace.members).
- **Step 10 < Step 11**: workspace.members 제거 후 디렉토리 삭제 — stale path 회피.
- **Step 11 < Step 12-14**: production code 갱신 후 design + doc 갱신 — reality ≡ contract.
- **Step 12 < Step 13**: §3.7b 재작성 후 §8 graph 갱신 (§8 commentary 가 §3.7b 인용).
- **Step 13 < Step 14**: design 갱신 후 ARCHITECTURE + INDEX + HOTFIXES + HANDOFF 갱신 — design 이 source-of-truth, doc 들이 그 mirror.
- **Step 14 < Step 15**: 모든 file change 후 commit.

## §4 Verification gate (acceptance)

### §4.1 Step 별 verify (per-step exit gate)

각 Step 의 "Exit gate" 항목이 step-local gate. 핵심 anchor:

- **Step 4** (lib.rs re-export): destination surface alive — `cargo build -p kebab-parse-md` green.
- **Step 7** (kebab-app dep cleanup): anchor 1 — `cargo tree -p kebab-app | grep ...` = 0 줄.
- **Step 10** (workspace.members): anchor 2 — `cargo metadata ... | jq '.workspace_members | length'` = 22.
- **Step 11** (디렉토리 삭제): final invariant — `ls -d crates/*/ | wc -l` = 22 + `cargo build --workspace` green.
- **Step 15** (closure): 7 cargo gate (a-h) + wire diff 0 + clean commit.

### §4.2 Workspace 회귀 (spec §5.1, §5.2) — Step 15 시점

```bash
$ cd /home/altair823/kebab && export CARGO_TARGET_DIR=/build/out/cargo-target/target
$ cargo clean
$ cargo test --workspace --no-fail-fast -j 1   # baseline ± 작은 변동 (net delta = 0 또는 +N intentional)
$ cargo clippy --workspace --all-targets -- -D warnings   # 0 warning
$ cargo deny check   # 0 error 0 warning
$ cargo metadata --no-deps --format-version 1 | jq '.workspace_members | length'   # = 22
$ ls -d crates/*/ | wc -l   # = 22 (secondary)
$ cargo tree -p kebab-app --depth 2 | grep -E "kebab_(parse_types|normalize)"   # 0 line
$ grep -rn "kebab_normalize\|kebab_parse_types" crates/kebab-app/src/   # 0 hit
$ grep -l "kebab-normalize\|kebab-parse-types" crates/*/Cargo.toml   # 0 line
$ grep '^name = "kebab-normalize"\|^name = "kebab-parse-types"' Cargo.lock   # 0 hit
```

### §4.3 Wire schema 회귀 (spec §5.4)

```bash
$ git diff main..HEAD -- docs/wire-schema/v1/ | wc -l   # = 0
```

### §4.4 4 frozen task spec + ~25 referencing task spec frozen (spec §5.7)

```bash
$ git diff main..HEAD --name-only | grep -E "tasks/p1/p1-(2|3|4)|tasks/p9/p9-fb-07"   # 0 line
$ git diff main..HEAD --name-only | grep "^tasks/p" | wc -l   # 0 (모든 task spec mechanical update 0)
```

### §4.5 SMOKE 회귀 (spec §5.5) — informational only

`docs/SMOKE.md` 가 정의한 isolated TempDir KB pipeline 의 ingest + search + ask 가 흡수 전후 byte-identical wire 출력. *informational only* — acceptance gate 아님 (production code 의 lift 로직 byte-identical 이므로 SMOKE 결과 자동 일관).

### §4.6 Literal 보존 verify (spec §3.7 (e)(g) + NEW MAJOR #N1 + #N2)

Step 3 + Step 15 에서 두 번 verify:

```bash
$ grep -c 'target: "kebab-normalize"' crates/kebab-parse-md/src/normalize.rs   # = 1 (tracing target literal)
$ grep -E '"kb-(source-fs|parse-md|normalize)"' crates/kebab-parse-md/src/normalize.rs | wc -l   # >= 5 (4 hard-coded agent literal + warning_agent body return)
```

## §5 Commit strategy

**single clean commit** — sibling plan (PR #185) 의 패턴. 본 PR 의 모든 change 가 atomic refactor 이므로 split 의 의미 없음.

- Step 1-14 의 file edit 을 모두 staging 후 Step 15 의 commit message 로 single commit.
- 중간 step 의 *작업 progress* 는 git stash 없이 working tree 에 누적 (cargo build/test green 유지 시 — exit gate 가 step 별 정합 보장).
- Step 10-11 사이에 cargo build *fail* 또는 *workspace 무시* 발생 시 — expected behavior (§2 의 Step 10 exit gate 명시).
- 만약 Step 11 의 디렉토리 삭제 후 unexpected build error 발생 시 — `git reset --hard HEAD` 으로 모든 working change drop 하고 plan 재진입 (sibling plan §6.1 와 동일 패턴).

Commit message 의 구조 (Step 15 의 (i) 참조):
- first line: `refactor(parse-md): absorb kebab-normalize + kebab-parse-types — 24 → 22 crates + §3.7b 재작성`
- 본문: 14 bullet (deliverable + wire/surface impact + verification) + Co-Authored-By 제거 (사용자 commit pattern 와 일관).

## §6 Risks + mitigation

### §6.1 중간 단계 cargo build 깨짐 (step ordering 깨짐)

**위험**: Step 6 (kebab-app callsite) 가 Step 2-4 (destination 생성) 보다 먼저 진행되면 `use kebab_parse_md::build_canonical_document;` 의 destination surface 가 alive 안 됨 → cargo build fail.

**Mitigation**: §3 step dependency graph 의 ordering invariant 명시. Step 별 exit gate 가 cargo build green 보장 — gate 통과 후 다음 step.

### §6.2 Step 10 anchor 후 build fail (workspace.members 미정합)

**위험**: Step 10 의 Hunk (a) 만 적용하고 Step 11 디렉토리 삭제 안 하면 cargo 가 `crates/kebab-normalize/Cargo.toml` 의 path 를 lingering 으로 인식 가능 (단 members 에 없으므로 silently skip 예상).

**Mitigation**: Step 10 의 exit gate 가 "build *fail* 예상 또는 *workspace 무시*" 명시 — *expected* behavior. Step 11 즉시 진행으로 정합.

### §6.3 자기 참조 dev-dep 의 cargo behavior 미검증

**위험**: Step 9 의 `crates/kebab-parse-md/tests/normalize_snapshot.rs` 가 *자기 자신* 의 `lib` 를 link 하는 패턴. cargo 의 standard behavior 는 dev-dep declare 없이 integration test 가 자기 crate `lib` 자동 link — 그러나 misconfig 시 silently no-link 위험.

**Mitigation**: spec §6.3 R3 + §3.7 (f) 의 명시 — `cargo test -p kebab-parse-md --test normalize_snapshot -j 1` 가 green 인지 Step 9 의 exit gate 에서 확인. 만약 fail 시 *명시적* `kebab-parse-md = { path = "." }` dev-dep declare 추가 (cargo 의 어떤 edge case 일 수 있음).

### §6.4 hard-coded literal accidental drop

**위험**: Step 3 의 normalize.rs 이식 시 4 hard-coded agent literal (lib.rs:122/128/134/153) + tracing target literal (lib.rs:109) 중 일부가 cp/Write 과정에서 누락. spec §3.7e + §3.7 (g) + §1.9 의 6-row production flow trace 정합 깨짐.

**Mitigation**: Step 3 의 exit gate 가 `grep -c 'target: "kebab-normalize"' ... = 1` + `grep -E '"kb-(source-fs|parse-md|normalize)"' ... | wc -l >= 5` 명시. 5 literal (4 agent + 1 tracing target) 모두 grep 으로 verifiable. §4.6 의 spec-level verify gate 도 cross-check.

### §6.5 ~25 referencing task spec 의 accidental edit

**위험**: Step 12-14 의 design + doc 갱신 시 ~25 referencing task spec 도 같이 검색-치환 하면 frozen 룰 위반.

**Mitigation**: spec §5.7 의 invariant — `git diff main..HEAD --name-only | grep "^tasks/p"` = 0 line. Step 14 종료 후 verify 로 확인. 만약 stray edit 발생 시 `git checkout main -- tasks/p<N>/...` 으로 revert.

### §6.6 16 GB RAM 의 build pressure (full workspace test)

**위험**: Step 15 의 `cargo test --workspace --no-fail-fast -j 1` 가 lance / datafusion link step 에서 OOM 위험 (MEMORY.md `feedback_serial_build_only.md` + CLAUDE.md "Serial cargo builds only").

**Mitigation**:
- `cargo clean` 직전 후 (Step 15 (a)) — stale artifact 제거로 link memory 절약.
- `-j 1` 엄수 (sibling plan §6.6 패턴).
- per-crate 단위 검증 우선 (Step 2-9 의 각 exit gate 가 `-p <crate>` 단위) — full workspace 는 Step 15 의 1회.
- `cargo test/clippy/build` 동시 background 실행 금지.

### §6.7 design §3.7b 재작성 wording 의 부정확

**위험**: Step 12 의 §3.7b 재작성 시 spec §3.5 의 4-단락 wording 을 정확히 따르지 않으면 critic round 5 (만약 spec 이 다회 revision) 의 verification 가 fail 가능.

**Mitigation**: Step 12 의 source-of-truth = spec §3.5 의 code block (한국어 산문 + 의존 그래프 ascii). plan/executor 가 spec §3.5 를 copy-paste 후 markdown formatting (heading depth 등) 조정.

### §6.8 HOTFIXES.md entry 의 chronological reverse insert 실수

**위험**: Step 14 의 HOTFIXES entry 신규 추가 시 chronological reverse (newest top) 룰 위반 — 본 PR 의 entry 가 기존 entry *밑* 에 들어가면 reader 혼란.

**Mitigation**: Step 14 (d) 의 generic anchor verify cmd 활용 — `head -50 tasks/HOTFIXES.md` + `FIRST_ENTRY_LINE=$(grep -n "^## 20" tasks/HOTFIXES.md | head -1 | cut -d: -f1)` 으로 first entry line 위치 동적 측정 후 그 *위* 에 insert. hard-coded date / wording 인용 회피 — file 의 history 변경에도 robust.

### §6.9 kebab-parse-md 의 dep 폭증 (spec §6.9 R9)

**위험**: 흡수 후 `kebab-parse-md/Cargo.toml` 의 deps 가 기존 + `unicode-normalization` (흡수 후 추가) 로 1개 증가. lingua 의 build time + binary size 가 markdown parse + lift 두 책임을 모두 가지는 crate 에 concentrate.

**Mitigation**:
- 신규 deps = `unicode-normalization` 1 개만 (이미 `kebab-app` 도 사용 중인 `0.1` major). version drift 없음.
- Step 3 의 normalize.rs 이식 시 `Cargo.toml` 의 `[dependencies]` 에 `unicode-normalization = "0.1"` 추가 (sibling normalize 의 dep 와 동일 version).
- 본 위험의 실질 영향 ≈ +1 dep → 영향 minimal.

### §6.10 frozen p1-4 surface re-export 후퇴 (spec §6.10 R10)

**위험**: Step 3 의 normalize.rs 이식 시 `pub use kebab_core::{id_for_block, id_for_doc}` 제거 → p1-4 frozen public surface 의 후퇴.

**Mitigation**:
- spec §6.10 R10 의 production caller 0 verified — re-export 경유 caller = 0 (test mod imports 제외, R10 의 grep cmd).
- Step 3 의 exit gate 가 `grep -c "pub use kebab_core::" crates/kebab-parse-md/src/normalize.rs` = 0 verify — 명시적 제거.
- p1-4 frozen 룰은 historical contract 로 보존 — 본 PR 의 HOTFIXES entry (Step 14) 가 live source.

## §7 Out of scope (plan-level)

spec §8 의 모든 out-of-scope 그대로 + plan-level 추가:

- sibling spec (PR #185 source-fs dep lightening) 의 follow-up. 별 PR / 별 plan.
- kebab-parse-md 의 internal refactor (예: types.rs + normalize.rs 외 module 재배치). follow-up.
- kebab-app/src/lib.rs 의 다른 callsite 의 cosmetic 변경 — line 51, 1119 만 touch.
- Lens 3 (Extractor + Chunker dispatch unification) — 별도 작업 (spec §8 + §11 의 future direction sibling).
- v0.20+ image/pdf normalize integration — spec §11 의 영구 보존 entry (본 PR 의 scope 외).
- **kebab-app/src/lib.rs 의 comment 안 historical `kb-normalize` mention** (verified — line 1308 의 `mirroring \`kb-normalize::build_canonical_document\`` + line 1474-1475 의 `see kb-normalize's \`warning_agent\``) — comment 의 historical reference 로 **보존** (MINOR GAP8 closure). git blame 일관성 + reader 가 흡수 history 의 origin 추적 가능. context string (line 1119) 만 갱신 (Step 6) — production runtime behavior 의 wire-invisible 영향과 무관한 internal doc 형식의 historical reference 보존.

## §8 References

- spec: `docs/superpowers/specs/2026-05-26-normalize-absorption-spec.md` (1067 lines, round 3 APPROVE).
- sibling plan: `docs/superpowers/plans/2026-05-26-source-fs-dep-lightening-plan.md` (PR #185 머지 완료).
- design contract: `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md` §3.7b + §8.
- 4 frozen task spec: `tasks/p1/p1-2-parser-types.md`, `tasks/p1/p1-3-markdown-parser.md`, `tasks/p1/p1-4-normalize.md`, `tasks/p9/p9-fb-07-md-title-fallback.md`.
- audit log root: `tasks/INDEX.md` L169 (PR #181 의 system-architect review).
- CLAUDE.md (project) + CLAUDE.md (machine) + MEMORY.md — disk / cargo / commit 룰.

## §9 Round closure status

### §9.1 Round 1 status

| Round | Reviewer | Verdict | Issues | Closure |
|---|---|---|---|---|
| 1 | critic-plan + verifier-plan (round 1) | REQUEST_CHANGES (2 CRITICAL + 2 BLOCKER + 6 MAJOR + 5 MINOR + 1 NIT = 16) | 본 round 2 revision 에서 all-closed — §9.2 의 finding-by-finding closure 참조 | 본 plan 의 round 2 revision (2026-05-26, planner) |

### §9.2 round 1 finding-by-finding closure

| ID | Severity | Source | Finding | Closure (plan-level edit location) |
|---|---|---|---|---|
| #1 (critic) | CRITICAL | critic-plan | `kebab-parse-md/Cargo.toml` 의 dep 갱신 step 부재 (spec §3.4 의 `-kebab-parse-types` + `+unicode-normalization` silently drop) | **CLOSED** — Step 3 의 Files affected 에 `crates/kebab-parse-md/Cargo.toml` 추가 + Action (e) 신설 (description string 갱신 + `kebab-parse-types` 제거 + `unicode-normalization = "0.1"` 추가). Exit gate 의 `grep -c "kebab-parse-types" Cargo.toml = 0` + `grep -c "unicode-normalization" Cargo.toml ≥ 1`. |
| #2 (critic) | CRITICAL | critic-plan | `pub use kebab_core::{id_for_block, id_for_doc}` 제거 시 in-body unqualified call (line 67, 241) unresolved → compile error | **CLOSED** — Step 3 의 Action (d) 갱신: `pub use` 제거 + 기존 `use kebab_core::{...}` block 에 `id_for_block, id_for_doc` 추가 명시. Exit gate `grep -E "use kebab_core::\{" ... \| grep -c "id_for_block\|id_for_doc" ≥ 1`. (대안 b: spec §3.3 retract — 채택 안 함, plan-side fix only) |
| #1 (verifier) | BLOCKER | verifier-plan | `kebab-parse-md/tests/` 의 `kebab_parse_types` import (2 hit verified — blocks_snapshots.rs:19 + frontmatter_snapshots.rs:23) 갱신 step 부재 | **CLOSED** — Step 5 의 Files affected 에 2 test file 추가 + Action (c)(d) 신설 (`kebab_parse_md::*` 로 갈음, integration test 는 `crate::` 사용 불가). Exit gate 의 grep scope src/ 한정 해제 → `grep -rn "kebab_parse_types" crates/kebab-parse-md/` = 0 hit. |
| #5 (verifier GAP5) | MAJOR | verifier-plan | blocks.rs 의 4 hit (line 1 doc, 25 doc-link, 37 use, 1589 production fully-qualified) 갱신 | **CLOSED** — Step 5 의 Action (a) 의 file-wide replace 명시. 4 hit 모두 `kebab_parse_types::` → `crate::types::` (Edit `replace_all: true` 가능). |
| (extra finding) | (planner self-detect) | normalize.rs | `cfg(test) mod tests` 안 9 hit fully-qualified `kebab_parse_types::ParsedBlockKind::*` (line 489/498/507/516/525/818/862/1070) 갱신 필요 | **CLOSED** — Step 3 의 Action (c) 의 sed/replace_all 명시. Exit gate `grep -c "kebab_parse_types::" crates/kebab-parse-md/src/normalize.rs = 0`. |
| #1 (critic) | MAJOR | critic-plan | Step 7 verify gate self-contradictory ("build fail 예상" — actual cargo behavior = silently ignore) | **CLOSED** — Step 10 verify gate 의 wording 정정: "`cargo build --workspace -j 1` **green** — 두 orphan 디렉토리는 cargo silently ignore. Cargo.lock 의 `[[package]]` entry 는 Step 11 직후 first `cargo build` 에서 자동 cleanup." (sibling plan revision 1 의 Step 7 → revision 2 의 Step 10) |
| #2 (critic) | MAJOR | critic-plan | Step 5 의 generic glob → 2 file:line 명시 | **CLOSED** — Step 8 의 Action (c) 갱신: `chunk/tests/long_section_snapshot.rs:21` + `store-sqlite/tests/contract_roundtrip.rs:16` explicit. |
| #3 (critic) | MAJOR | critic-plan | Step 6 의 fixture file enumerate (find 결과 명시) | **CLOSED** — Step 9 의 Files affected 갱신: `find crates/kebab-normalize/tests/` 결과 = `normalize_snapshot.rs` 단일 file (sibling fixture 부재) 명시. |
| #4 (critic) | MAJOR | critic-plan | HOTFIXES insertion anchor stale (hard-coded "S3 NLI" entry 인용) → generic 화 | **CLOSED** — Step 14 의 Action (d) 갱신: `head -50 tasks/HOTFIXES.md` + `FIRST_ENTRY_LINE=$(grep -n "^## 20" tasks/HOTFIXES.md \| head -1)` 동적 측정. §6.8 risk wording 도 generic 화. |
| GAP3 + GAP4 (verifier) | MAJOR | verifier-plan | Step 1 의 baseline N + Step 10 의 net delta 0 numeric compare cmd 부정확 (binary 수 행수 ≠ test 함수 수) | **CLOSED** — Step 1 의 baseline cmd 갱신: `awk '/^test result: ok\./ {sum += "passed;" 직전 숫자} END {print sum}'`. `.omc/state/normalize-absorption-baseline.txt` dump. Step 15 의 (b) numeric compare gate 추가: `diff <(echo $POST_COUNT) .omc/state/normalize-absorption-baseline.txt`. |
| GAP6 | MINOR | verifier-plan | Step 2 (현 Step 3) 의 4-literal grep production-only 분리 | **CLOSED** — Step 3 의 exit gate 의 grep cmd 정확화: `grep -cE 'agent:\s*"kb-(source-fs\|parse-md\|normalize)"\.to_string\(\)' ≥ 3` (production body 의 hard-coded literal 만). warning_agent body return + lift_warnings literal 합쳐 5 hit 보장. |
| GAP7 | MINOR | verifier-plan | Step 9 verify `git diff ... \| head -100` 의 truncate 위험 | **CLOSED** — Step 13 의 exit gate 갱신: `head -200` 삭제 + `grep -c "^@@" = 2` + `grep -cE "^\+\+\+\|^---" = 2` numeric verify 추가. |
| GAP8 | MINOR | verifier-plan | lib.rs comment stale `kb-normalize` mention (line 1308, 1474) 보존 결정 명시 | **CLOSED** — §7 out-of-scope 에 한 줄 추가: "`kebab-app/src/lib.rs` 의 comment 안 historical `kb-normalize` mention 은 보존 — git blame 일관성 + reader 가 흡수 history 추적 가능. context string (line 1119) 만 갱신." |
| #3 (critic) | MINOR | critic-plan | Step 4 (c) Cargo.toml hunk context — `sed -n '11,20p'` actual context 확인 명시 | **CLOSED** — Step 7 의 Action 갱신: `sed -n '11,20p' crates/kebab-app/Cargo.toml` 사전 context 확인 cmd 추가. |
| #1 (critic) | NIT | critic-plan | Step 7 "anchor" 명명 의미 — "closure pre-pivot" 정도 | **CLOSED** — Step 7 의 heading 정정: "anchor" → "closure pre-pivot 1". Step 10 의 "anchor 2" 도 의미상 "closure pre-pivot 2" 로 변경 가능하나 plan 의 convention 일관성 유지 차원에서 "anchor 1" / "anchor 2" / "closure" 의 3-stage 명명 유지 (NIT 수준 — semantic only). |

### §9.3 Round 2 metrics

- Plan line count: 567 (revision 1) → 761 (revision 2 start) → **현재** (revision 2 end, post-16-finding-closure).
- Step count: 15 (revision 2 무변).
- 신규 추가 (revision 2 end):
  - Step 3 의 Action (e) — kebab-parse-md/Cargo.toml diff.
  - Step 3 의 Action (d) — id_for_* unqualified import 보존.
  - Step 3 의 Action (c) — 9 hit fully-qualified 갱신.
  - Step 5 의 Action (c)(d) — kebab-parse-md/tests/ 의 2 file ref shift.
  - Step 1 + Step 15 의 baseline N + numeric compare.
  - §7 의 lib.rs comment 보존 명시.
  - §9.2 의 16-row closure table.
- §3 Design 결정 무변 (Option A, dead struct 3 보존, §3.7b 4-단락 재작성, target_version 0.19.0, warning_agent + tracing target 보존 정책, id_for_* re-export 제거).

---

**Plan drafted by**: planner (team `normalize-absorption`, Phase B).
**Date**: 2026-05-26.
**Source spec**: 2026-05-26-normalize-absorption-spec.md (round 3 APPROVE, 1067 lines).
**Step count**: 15 (decompose from 10 step revision 1).
**Step ordering invariant**: Step 2 + 3 < Step 4-5 < Step 6 < Step 7 < Step 8 < Step 9 < Step 10 < Step 11 < Step 12 < Step 13 < Step 14 < Step 15.
**Anchor steps**: Step 7 (kebab-app dep cleanup) + Step 10 (workspace.members) + Step 15 (closure).
**Estimated complexity**: MEDIUM (15 step, 1:1 spec mapping, 단일 commit, ~3000 LOC code touch).
