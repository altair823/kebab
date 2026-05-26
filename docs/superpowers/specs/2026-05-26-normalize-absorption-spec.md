---
status: drafting
target_version: 0.19.0
spec: docs/superpowers/specs/2026-04-27-kebab-final-form-design.md (§3.7b 재작성 + §8 graph 갱신)
contract_sections: ["§3.7b", "§8"]
related_specs:
  - docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
  - docs/superpowers/specs/2026-05-26-source-fs-dep-lightening-spec.md
related_plans: []
hotfix_links: []
---

# kebab-normalize + kebab-parse-types 흡수 — 24 → 22 crates + §3.7b 재작성

## §1 Background + evidence chain

### §1.1 현재 24 crate workspace

`Cargo.toml` 의 `[workspace] members` 가 24 crate 를 declare (확인: `head -30 Cargo.toml`). 이 중 둘이 본 refactor 의 대상이다.

- `crates/kebab-normalize` — 1097 LOC (lib.rs only, `wc -l` 측정), production caller 1개 (`kebab-app`).
- `crates/kebab-parse-types` — 98 LOC (lib.rs only), production caller 2개 (`kebab-parse-md`, `kebab-normalize`), kebab-app 에서 dep declare 했지만 import 0건 (dead dep).

PR #181 (post-PR9 refactor) 머지 직전 system-architect 의 component-level review 가 "pre-cut nothing, all v0.18.1+ defer (kebab-normalize 흡수, Extractor dispatch unification, kebab-source-fs dep lightening 등)" 결론 (tasks/INDEX.md L169). Sub-item 1 (`kebab-source-fs dep lightening`) 은 이미 PR #185 로 머지됨. 본 spec 은 sub-item 2 ("`kebab-normalize` 흡수").

### §1.2 `kebab-normalize` caller 실측

production source 와 dev-deps 둘 다 명시 (actual `Cargo.toml` 의 `[dev-dependencies]` block 인용 — `cat crates/{kebab-chunk,kebab-store-sqlite,kebab-normalize}/Cargo.toml | grep -A20 "dev-dependencies"`):

| crate | callsite | 종류 |
|---|---|---|
| `kebab-app` | `src/lib.rs:51` (`use kebab_normalize::build_canonical_document;`) | **production** |
| `kebab-chunk` | `Cargo.toml [dev-dependencies] kebab-normalize` (snapshot integration test 의 fixture builder) | **dev-only** |
| `kebab-store-sqlite` | `Cargo.toml [dev-dependencies] kebab-normalize` (contract round-trip test 의 fixture builder) | **dev-only** |
| `kebab-normalize` (자체) | `tests/normalize_snapshot.rs` 가 `Cargo.toml [dev-dependencies] kebab-parse-md` 로 reverse-direction dev-dep — `kebab-parse-md` 를 fixture parser 로 사용 | **reverse dev-dep** |

→ production caller = **`kebab-app` 단일**. `kebab-normalize` 흡수 시:

1. `kebab-app` 의 1 줄 use statement + call site (lib.rs:51, :1119) 갱신.
2. `kebab-chunk` + `kebab-store-sqlite` 의 dev-dep `kebab-normalize` → `kebab-parse-md` 로 갈음 + 두 crate 의 통합 test source (`tests/*.rs`) 의 `use kebab_normalize::*;` → `use kebab_parse_md::*;` 갱신.
3. `kebab-normalize/tests/normalize_snapshot.rs` 의 `kebab-parse-md` reverse dev-dep 은 흡수 후 자기 자신 참조 → declare 제거 (in-crate test 가 lib 를 자동 link, `Q4` 의 cargo standard behavior — §6.3 의 R3 verified).

이 4 surface 가 본 PR 의 callsite migration scope 전체.

### §1.3 `kebab-parse-types` caller 실측

`grep -rn "kebab_parse_types" --include="*.rs" crates/*/src/` (production source 만):

| crate | use 횟수 | 사용 type | 종류 |
|---|---|---|---|
| `kebab-parse-md` | 다수 (`blocks.rs`, `frontmatter.rs`) | `ParsedBlock`, `ParsedBlockKind`, `ParsedPayload`, `Warning`, `WarningKind` | **production** |
| `kebab-normalize` | 다수 (`lib.rs`) | 위 5 type 동일 | **production** |
| `kebab-app` | `Cargo.toml` declare, `.rs` use 0건 | (없음) | **dead dep** |

→ production caller 2개 (`kebab-parse-md`, `kebab-normalize`) + `kebab-app` 의 dead dep 1건. `kebab-normalize` 가 흡수되면 caller 가 `kebab-parse-md` 단일로 collapse — `kebab-parse-types` 의 raison d'être (parser 와 normalize 사이의 layer) 소멸.

### §1.4 4 parser 의 normalize / parse-types 의존 실측

| parser crate | `kebab-normalize` 의존? | `kebab-parse-types` 의존? | extract 결과 |
|---|---|---|---|
| `kebab-parse-md` | (자체 의존 없음 — `kebab-normalize` 는 *역방향* 으로 parse-md 를 dev-dep 으로 사용, §1.2 참조) | **production** (`Cargo.toml:12`) | `Vec<ParsedBlock>` (normalize 경유 → `CanonicalDocument`) |
| `kebab-parse-pdf` | 0 (production 0, dev-deps 0) | 0 | `CanonicalDocument` 직접 emit |
| `kebab-parse-image` | 0 | 0 | `CanonicalDocument` 직접 emit |
| `kebab-parse-code` | 0 | 0 | `CanonicalDocument` 직접 emit |

→ design §3.7b 의 의도 ("ParsedBlock 류는 모든 parser 가 emit → normalize 가 일괄 lift") 와 **현실 (markdown 만 normalize 통과, 나머지 3 parser 는 CanonicalDocument 직접 emit)** 가 divergent. `kebab-normalize` 의 production caller 가 1개 (`kebab-app` 단일) 인 이유도 동일 — 4 parser 중 1개만 normalize 를 경유.

**중요**: `kebab-parse-audio` crate 는 미존재 (P8 audio 가 사용자 결정으로 deferred — `tasks/INDEX.md` 의 Phase 8 row 참조). 본 spec 의 mission wording 도 5 parser 가 아닌 4 parser 기준.

### §1.5 `kebab-normalize` surface (1097 LOC, lib.rs only)

actual source 의 정확한 signature (`crates/kebab-normalize/src/lib.rs:60-66, :360` — `tasks/p1/p1-4-normalize.md:54-62` frozen 과 byte-identical):

```rust
pub fn build_canonical_document(
    asset: &RawAsset,                  // by-ref, kebab_core::RawAsset (NOT &AssetInfo)
    metadata: Metadata,                // by-value, kebab_core::Metadata (NOT &Metadata)
    blocks: Vec<ParsedBlock>,
    parser_version: &ParserVersion,
    warnings: Vec<Warning>,
) -> Result<CanonicalDocument>;

pub fn derive_title(
    frontmatter_title: &str,           // &str by-ref (NOT Option<&str>)
    blocks: &[Block],                  // lifted Block, NOT ParsedBlock — lift 이후 호출
    file_stem: &str,                   // &str by-ref (NOT Option<&str>)
) -> String;

pub use kebab_core::{id_for_block, id_for_doc};
```

- `build_canonical_document` — markdown 의 `Vec<ParsedBlock>` 을 받아 `CanonicalDocument` 로 lift. ID 생성 / Provenance event 누적 / Warning → ProvenanceEvent 변환 / `warning_agent` 분기 (md vs lift-stage attribution). `metadata` 는 by-value — 내부에서 `user` map 의 `title` / `lang` 을 `remove` 하면서 lift (mutating ownership, wire 중복 회피).
- `derive_title` — p9-fb-07 frozen API (markdown title fallback chain). `tasks/p9/p9-fb-07-md-title-fallback.md` 가 본 함수 의 정확한 contract 를 freeze. **중요**: `blocks: &[Block]` 는 **lift 된** `kebab_core::Block` 이며 (lift 이전 의 `ParsedBlock` 아님), 따라서 본 함수는 `build_canonical_document` 의 *내부* 에서 lift 후 호출됨 — caller 입장 에서 `derive_title` 의 direct call 시점 에는 이미 `Vec<Block>` 가 손에 있어야 함. 본 의미가 plan / executor 의 type-error misassumption 방지의 핵심.
- 1097 LOC 중 ~700 LOC = unit tests. production fn body 는 ~400 LOC.

**p1-4 frozen cross-link**: `tasks/p1/p1-4-normalize.md:54-62` 의 signature block 이 본 §1.5 의 정확한 source-of-truth. 본 spec 의 §3.7 callsite migration 은 signature 자체를 변경하지 않으므로 frozen 위반 0.

### §1.6 `kebab-parse-types` surface (98 LOC, lib.rs only)

**production 에서 사용 중 (5 type)**:
- `ParsedBlock`, `ParsedBlockKind`, `ParsedPayload` — markdown parser → normalize 의 lift input.
- `Warning`, `WarningKind` — markdown parser 의 panic-recovery + table malformed + frontmatter malformed event.

**forward-declared, production caller 0 (3 struct)**:
- `ParsedImageRegion` (line 85) — P6 image stage 의 intent 표현. 현 surface = `pub struct ParsedImageRegion;` (unit struct, payload 없음).
- `ParsedPdfPage` (line 88) — P7 pdf stage 의 intent. surface = `pub struct ParsedPdfPage { pub page: u32, pub text: String }`.
- `ParsedAudioSegment` (line 94) — P8 audio (deferred) 의 intent. surface = `pub struct ParsedAudioSegment { pub start_ms: u64, pub end_ms: u64, pub text: String }`.

→ 3 dead struct 의 design intent 자체 ("multi-region image, multi-block pdf, multi-segment audio 의 lift surface") 는 유효. 사용자 결정 (team-lead 메시지 의 #6): 보존 + future surface 명시.

### §1.7 design §3.7b 의 abstraction reality check

design §3.7b (`docs/superpowers/specs/2026-04-27-kebab-final-form-design.md:703-764`) 가 `kebab-parse-types` 의 raison d'être 를 다음 2 점으로 정당화:

1. **namespace 폭발 방지**: parser-별 ParsedBlock 변종이 `kebab-core` 의 namespace 를 폭발시키지 않도록 thin layer 분리.
2. **normalize 의 parser non-dependence**: normalize 가 어떤 parser 도 직접 import 하지 않아야 함.

**현실 (P1~P10 머지 후)**:
- 4 parser 중 3개 (pdf / image / code) 가 `CanonicalDocument` 직접 emit → normalize 경유 안 함 → "normalize 가 parser 직접 import 하면 안 됨" 의 constraint 가 markdown 한 갈래에서만 의미 존재.
- `kebab-core::Block` namespace 폭발 우려도 4 parser 중 3개가 우회 → `kebab-parse-types` 의 layer 가 *현 시점* 에 막아야 할 폭발이 markdown 외엔 없음.
- §3.7b 의 "thin layer 가 유일한 합류 지점" — 합류 지점에 들어오는 parser 가 1개 (md) → layer 무용. fan-in/fan-out 둘 다 1.

**그러나** design intent 자체 ("향후 image/pdf/audio 가 normalize 거치도록 바뀔 경우 layer 가 다시 살아남") 는 *valid future contingency*. 따라서 §3.7b 를 **완전 strike 가 아닌 재작성** 하는 것이 정확 — "thin layer 는 현재 markdown 외 parser 가 사용하지 않으므로 `kebab-parse-md` 흡수. 3 forward-declared struct 는 보존되며 future re-extraction trigger 명시".

### §1.8 사용자 결정 요약 (team-lead 메시지의 7 점)

| # | 결정 | 본 spec 반영 위치 |
|---|---|---|
| 1 | target_version = 0.19.0 (frozen contract 변경) | frontmatter + §7 |
| 2 | Destination = Option A (`kebab-parse-md` 흡수) | §3.1 |
| 3 | ~25 referencing task spec = frozen 유지 + HOTFIXES live source | §7 (mechanical update 0) |
| 4 | `kebab-app` 의 dead `kebab-parse-types` dep = 같은 PR incidental cleanup | §3.10 |
| 5 | HOTFIXES entry = 4-block 변형 (Symptom → "design deviation") | §3.9 |
| 6 | 3 dead struct 보존 + future surface 명시 | §3.3 (보존) + §6 (future trigger) |
| 7 | `warning_agent` wire visibility = planner 가 검증 → 결과 반영 | §1.9 + §7 |

### §1.9 wire visibility 검증 결과 (sub-item 2 의 detective verification)

**wire 의 정의** (본 spec 범위 내): "wire" = JSON-RPC payload (`kebab-mcp` 의 tool definitions) + CLI `--json` output (kebab-app facade) + 외부 통합 의 schema (Claude Code skill 등). SQLite BLOB persistence (`documents.provenance_json`) 는 wire **외** — 단일 사용자 의 local DB 의 internal storage, 외부 통합 없음.

**검증 방법**: `docs/wire-schema/v1/*.json` 의 16 file 모두 (`ls docs/wire-schema/v1/*.json | wc -l = 16`) 의 `agent` / `provenance` field 명시적 검색 + production code 의 `agent` field flow trace.

| wire schema | `agent` field? | `provenance` field? |
|---|---|---|
| `answer_event.v1` | 0 | 0 |
| `answer.v1` | 0 | 0 |
| `bulk_search_item.v1` | 0 | 0 |
| `bulk_search_response.v1` | 0 | 0 |
| `chunk_inspection.v1` | 0 | 0 |
| `citation.v1` | 0 | 0 |
| `doc_summary.v1` | 0 | 0 |
| `doctor.v1` | 0 | 0 |
| `error.v1` | 0 | 0 |
| `fetch_result.v1` | 0 | 0 |
| `ingest_progress.v1` | 0 | 0 |
| `ingest_report.v1` | 0 | 0 |
| `reset_report.v1` | 0 | 0 |
| `schema.v1` | 0 | 0 |
| `search_hit.v1` | 0 | 0 |
| `search_response.v1` | 0 | 0 (description 산문 의 "MCP / agent consumers" mention 은 line 35 의 `hint` field description 내 — field 아님, false-positive 명시) |

**False-positive 명시**: `docs/wire-schema/v1/search_response.schema.json:35` 의 `hint` field description 본문에 "MCP / agent consumers should surface this" 문구 1 hit (description prose, schema field 아님). 본 hit 는 `ProvenanceEvent.agent` 와 무관 — `hint` 는 short-query advisory string (v0.17.0 A5).

**production flow trace** (actual `crates/kebab-normalize/src/lib.rs` 의 emission point 별 분리):

| line | string | emitter | persist |
|---|---|---|---|
| `:109` | `"kebab-normalize"` | `tracing::debug! target` literal | (log file, NOT SQLite — `~/.local/state/kebab/logs/kb.log.YYYY-MM-DD`) |
| `:122` | `"kb-source-fs"` | `Discovered` event 의 hard-coded agent | SQLite `provenance_json` |
| `:128` | `"kb-parse-md"` | `Parsed` event 의 hard-coded agent | SQLite `provenance_json` |
| `:134` | `"kb-normalize"` | `Normalized` event 의 hard-coded agent | SQLite `provenance_json` |
| `:143` | `warning_agent(&w.kind).to_string()` → `"kb-parse-md"` (4 variant 모두) | `Warning` event 의 agent | SQLite `provenance_json` |
| `:153` | `"kb-normalize"` | `lift_warnings` loop 의 hard-coded agent (AudioRef-deferred) | SQLite `provenance_json` |

- 5 위치 모두 `ProvenanceEvent.agent: String` 필드로 누적.
- `Provenance` 는 `serde_json::to_string(&doc.provenance)` 로 직렬화되어 `documents.provenance_json` BLOB 컬럼에 persist (`crates/kebab-store-sqlite/src/documents.rs:726`).
- **wire export 경로 0**: 어떤 `--json` 출력에도 `provenance` field 가 노출되지 않음. SQLite-only storage = wire **외**. `tracing` 의 target literal 도 wire 외 (log file).

**결론**: `warning_agent` 의 return string ("`kb-normalize`" / "`kb-parse-md`") 은 wire-invisible (SQLite-internal). String 값을 변경해도 wire schema 영향 = 0. 단 *DB compat* 차원에서는 기존 row 의 string 값과 신규 row 의 값이 일치하면 audit log 가 일관 — § 3.7 의 callsite migration 에서 string 보존 정책 명시.

## §2 Goals + non-goals

### §2.1 Goals

- **G1**: 24 → 22 crate. `kebab-normalize` + `kebab-parse-types` 두 crate 흡수.
- **G2**: design §3.7b 재작성 — "thin layer" 의 *현재* 무용성 + 보존된 3 forward-declared struct 의 future re-extraction trigger 명시.
- **G3**: design §8 graph 갱신 — 3 edge 제거 + 2 forbidden bullet 의미 갱신.
- **G4**: HOTFIXES.md 신규 entry (4-block 변형, design deviation Symptom).
- **G5**: 모든 referencing task spec (~25개) frozen 유지. HOTFIXES.md 가 live source of truth (CLAUDE.md 의 "Task specs themselves stay frozen" 룰).
- **G6**: wire schema 변경 0건. CLI / TUI / MCP surface 변경 0건.
- **G7**: `kebab-app` 의 dead `kebab-parse-types` regular dep incidental cleanup.
- **G8**: target_version = **0.19.0** (frozen design contract 변경 trigger).
- **G9**: `cargo tree -p kebab-app | grep -E "kebab_parse_types|kebab_normalize"` = 0 줄 (post-absorb invariant).

### §2.2 Non-goals

- `kebab-normalize` 의 lift 로직 의미 변경 (1:1 lift — `build_canonical_document` body 와 `derive_title` body 가 byte-identical 하게 destination 으로 이동).
- 5 사용 type (`ParsedBlock` / `ParsedBlockKind` / `ParsedPayload` / `Warning` / `WarningKind`) 의 의미 변경 (1:1 이동, serde 표현 + variant 명 보존).
- 3 forward-declared struct (`ParsedImageRegion` / `ParsedPdfPage` / `ParsedAudioSegment`) 의 surface 변경 (보존, future surface 명시).
- 4 parser 중 다른 parser (pdf / image / code) 가 normalize 를 신규로 거치도록 변경.
- `kebab-core` 의 도메인 타입 (`Block`, `CanonicalDocument`, `Provenance`, …) 변경.
- ~25 referencing task spec 의 mechanical update (frozen 유지 — HOTFIXES live source 룰).
- `parser_version` cascade 변경 (lift 로직 보존이므로 cascade trigger 없음).
- p9-fb-07 의 `derive_title` API contract 변경 (call site 이동만, signature 보존).
- README / HANDOFF / ARCHITECTURE 의 user-visible surface 변경 (단 ARCHITECTURE.md 의 crate graph 는 mechanical 갱신).

## §3 Design

### §3.1 Destination = Option A (`kebab-parse-md` 흡수)

흡수 대상 destination 으로 **`kebab-parse-md` 단일 crate** 를 선택 (team-lead 메시지 #2).

**근거 (evidence-driven)**:
- `kebab-parse-types` 의 production caller = `kebab-parse-md` + `kebab-normalize` 둘. `kebab-normalize` 는 자신을 흡수당하는 입장 → 자연스러운 합류 지점 = `kebab-parse-md`.
- `kebab-normalize::warning_agent` 의 분기 4건 중 4건 모두 `WarningKind::*` 의 emit 위치가 `kebab-parse-md` (`blocks.rs`, `frontmatter.rs`). 즉 *현실* 의 emit-site 가 이미 `kebab-parse-md` 단일.
- 4 parser 중 markdown 만 lift 를 거치므로, lift 가 `kebab-parse-md` 안에 있는 것이 caller 일관성과 맞음.

**대안 (Option B: `kebab-app` 흡수) 의 거부 이유**:
- `kebab-app` 은 facade — store / embed / llm / parse 전반의 orchestration 책임. lift 같은 markdown-specific 도메인 코드가 들어가면 facade 의 단일책임 침해.
- `kebab-app` 으로 흡수 시 `Vec<ParsedBlock>` 타입을 facade 가 노출하게 됨 → UI crate (cli/tui/mcp) 가 lift 의 intermediate type 을 indirect 으로 보게 되는 가능성.

**대안 (Option C: `kebab-core` 흡수) 의 거부 이유**:
- `kebab-core` 는 도메인 모델 only (Cargo.toml description). 8 variant `ParsedPayload` enum + markdown-specific Warning 류가 core 에 들어가면 §3.7b 가 명시한 namespace 폭발 우려가 정확히 현실화.

### §3.2 Module placement

`crates/kebab-parse-md/src/` 의 현 구성: `lib.rs`, `blocks.rs`, `frontmatter.rs`.

흡수 후 구성:

```
crates/kebab-parse-md/src/
├── lib.rs              # 기존 + pub re-exports (5 사용 type + 3 보존 struct + 2 normalize fn)
├── blocks.rs           # 기존 (use kebab_parse_types::* → use crate::types::* 갱신)
├── frontmatter.rs      # 기존 (use kebab_parse_types::* → use crate::types::* 갱신)
├── types.rs            # 신규 — kebab-parse-types/src/lib.rs 의 98 LOC 1:1 이식
└── normalize.rs        # 신규 — kebab-normalize/src/lib.rs 의 production fn body 이식
```

**근거**:
- `types.rs` + `normalize.rs` 분리는 readability (lib.rs 가 thin re-export layer 로 유지).
- 1:1 이식 = git blame / cargo doc / test grep 모두 file 단위로 traceable.
- 흡수 후에도 grep `crate/file` 단위로 "이 코드가 *원래 어디서 왔는가*" 추적 가능.

**대안 거부**: `lib.rs` 1 file 에 모두 합치는 옵션은 (a) blocks.rs + frontmatter.rs 의 use statement 가 `use crate::*` 로 광범위해지고, (b) lib.rs LOC 가 1200+ 으로 부풀어 review 부담. 분리 유지.

### §3.3 Visibility 정책

destination = `kebab-parse-md` 의 `lib.rs` 가 다음을 re-export.

| symbol | 현 visibility | 흡수 후 visibility | 근거 |
|---|---|---|---|
| `ParsedBlock` | `pub` | `pub` (re-export from `types.rs`) | kebab-app 이 import 안 함이지만 future caller 대비 + `tasks/p1-2.md` (frozen) 가 `pub` API 로 freeze |
| `ParsedBlockKind` | `pub` | `pub` | 동상 |
| `ParsedPayload` | `pub` | `pub` | 동상 |
| `Warning` | `pub` | `pub` | 동상 |
| `WarningKind` | `pub` | `pub` | 동상 |
| `ParsedImageRegion` | `pub` | `pub` (re-export, 보존 — §3.7b 재작성 후에도 surface) | future re-extraction trigger 시 surface 가 stable 해야 함 |
| `ParsedPdfPage` | `pub` | `pub` | 동상 |
| `ParsedAudioSegment` | `pub` | `pub` | 동상 |
| `build_canonical_document` | `pub` | `pub` (re-export from `normalize.rs`) | `kebab-app::lib.rs:51` 의 single import |
| `derive_title` | `pub` | `pub` | `tasks/p9/p9-fb-07-md-title-fallback.md` (frozen) 의 API contract |
| `warning_agent` | `fn` (private) | `fn` (`pub(crate)` — 동일 module 내 사용) | wire-invisible 내부 helper |
| `kebab_core::{id_for_block, id_for_doc}` re-export | `pub use` | (제거 — direct `kebab_core::*` 사용으로 통일. *frozen p1-4 surface 의 re-export 후퇴이나 `kebab-normalize::id_for_*` 경유 production caller = 0 — 모든 caller 가 `kebab_core::*` 직접 import. 본 결정의 후퇴 위험 §6.10 R10 참조.*) | `kebab-normalize` 가 kebab-core re-export 한 패턴은 historical artifact. `kebab-parse-md` 가 직접 `kebab-core` import 하므로 re-export 필요 없음 |

**중요**: `kebab-app::lib.rs:51` 의 `use kebab_normalize::build_canonical_document;` 가 흡수 후 `use kebab_parse_md::build_canonical_document;` 로 갱신. signature byte-identical.

### §3.4 Cargo.toml diff per crate

영향받는 Cargo.toml = 4 file (kebab-parse-md / kebab-normalize 삭제 / kebab-parse-types 삭제 / kebab-app).

**`crates/kebab-parse-md/Cargo.toml`** (변경):
```diff
 [package]
 name = "kebab-parse-md"
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
+unicode-normalization     = "0.1"   # 흡수된 kb-normalize 의 NFKC 의존성
 pulldown-cmark  = { version = "0.13", default-features = false }
 serde_yaml_ng   = "0.10"
 toml            = "0.8"
 lingua          = { version = "1.8", default-features = false, features = [...] }

 [dev-dependencies]
 serde_json = { workspace = true }
+# 흡수된 kb-normalize 의 snapshot test 가 사용했던 dev-dep 들은 (kb-parse-md 가
+# 이미 자신을 통한 in-crate test 로 대체 가능하므로) 신규 추가 없음.
```

**`crates/kebab-normalize/Cargo.toml`** — **삭제 (디렉토리 전체 제거)**.

**`crates/kebab-parse-types/Cargo.toml`** — **삭제**.

**`crates/kebab-app/Cargo.toml`** (변경):
```diff
 [dependencies]
 kebab-core = { path = "../kebab-core" }
 kebab-config = { path = "../kebab-config" }
 kebab-source-fs = { path = "../kebab-source-fs" }
 kebab-parse-md = { path = "../kebab-parse-md" }
-kebab-parse-types = { path = "../kebab-parse-types" }
-kebab-normalize = { path = "../kebab-normalize" }
 kebab-chunk = { path = "../kebab-chunk" }
 ...
```

→ `kebab-parse-types` (dead dep) 와 `kebab-normalize` (live dep, 흡수됨) 둘 다 제거. `kebab-parse-md` 가 (re-export 통해) 두 crate 의 surface 를 모두 제공.

**`crates/kebab-chunk/Cargo.toml`** (변경):
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

→ snapshot integration test 의 fixture builder 가 `kebab-normalize::build_canonical_document` 를 호출했었음 → `kebab-parse-md::build_canonical_document` 로 갈음 (이미 dev-dep 으로 존재 — 신규 add 0). 통합 test source (`tests/*.rs`) 의 `use kebab_normalize::*;` → `use kebab_parse_md::*;` 갱신.

**`crates/kebab-store-sqlite/Cargo.toml`** (변경):
```diff
 [dev-dependencies]
 tempfile        = "3"
 serde_json      = { workspace = true }
 # kb-parse-md / kb-normalize / kb-chunk are dev-only — used to build a
 # CanonicalDocument + Vec<Chunk> from a fixture in the contract round-trip
 # test. Forbidden as regular deps per design §8 (store consumes domain
 # types from kb-core only); `cargo tree -p kb-store-sqlite --depth 1`
 # (default scope, excludes dev-deps) confirms this.
 kebab-parse-md = { path = "../kebab-parse-md" }
-kebab-normalize = { path = "../kebab-normalize" }
 kebab-chunk = { path = "../kebab-chunk" }
```

→ contract round-trip test 의 `use kebab_normalize::*;` → `use kebab_parse_md::*;` 갱신. 위와 동일 패턴.

**dev-dep migration 의 evidence 명령** (verification 시 사용):
```bash
# 흡수 전 (현 시점) — kebab-normalize regular + dev-dep 모두 hit:
$ grep -l "kebab-normalize" crates/*/Cargo.toml
crates/kebab-app/Cargo.toml             # regular, §3.4 의 kebab-app 변경에서 제거
crates/kebab-chunk/Cargo.toml           # dev-only, 위 diff 에서 제거
crates/kebab-normalize/Cargo.toml       # 본 PR 에서 디렉토리 자체 삭제
crates/kebab-store-sqlite/Cargo.toml    # dev-only, 위 diff 에서 제거

# 흡수 후 (post-PR head) — expected:
$ grep -l "kebab-normalize" crates/*/Cargo.toml
(0 line)
```

### §3.5 design §3.7b 재작성 — 정확한 wording

`docs/superpowers/specs/2026-04-27-kebab-final-form-design.md:703-764` 의 §3.7b 를 다음으로 **교체**.

**Before (line 703-764, 약 60 LOC)**: 본 spec §1.7 + §1.6 인용 — "thin layer 가 raison d'être" 의 정당화.

**After (제안)**:

```markdown
### 3.7b Parser intermediate types — `kebab-parse-md` 흡수 후 (post-v0.19.0)

**원래 의도**: parser 의 *중간* 표현 (`ParsedBlock` 류) 을 `kebab-core` 가 아닌 별도 thin crate `kebab-parse-types` 에 두고, `kebab-normalize` 가 medium-agnostic 한 ID/Provenance lift 책임을 가지는 layered 구조 (v0.1~v0.18 머지 시점의 초기 design).

**현재 상태 (v0.19.0~)**: `kebab-parse-types` 와 `kebab-normalize` 두 crate 가 `kebab-parse-md` 에 흡수됨. 근거:

- 4 parser (`kebab-parse-md` / `kebab-parse-pdf` / `kebab-parse-image` / `kebab-parse-code`) 중 `kebab-parse-md` 한 갈래만 `kebab-normalize` 를 경유. 나머지 3 parser 는 `CanonicalDocument` 를 직접 emit — thin layer 의 fan-in/fan-out 모두 1.
- production caller 가 1개로 collapse 되어 layer 가 의미 잃음.
- 본 흡수 의 audit log = `tasks/HOTFIXES.md` 의 dated entry (2026-05-26 — "design deviation").

**보존된 surface**: `ParsedBlock`, `ParsedBlockKind`, `ParsedPayload`, `Warning`, `WarningKind`, 그리고 3 forward-declared struct (`ParsedImageRegion`, `ParsedPdfPage`, `ParsedAudioSegment`) 는 `kebab-parse-md` 의 `pub` re-export 로 보존. 의미와 serde 표현 모두 byte-identical.

**future re-extraction trigger** (측정 시점 명시 — `build_canonical_document` 의 input variant 변경 지점): 다음 중 하나가 발생하면 layer 재추출 (별 spec + 별 PR). §11 의 trigger 조건과 일관 cross-link (본 §3.5 의 list 와 §11.6 의 list 는 동일 의미).

1. `kebab-parse-pdf` / `kebab-parse-image` / `kebab-parse-audio` (audio 는 **P8 도입 시** — 현재 deferred, `tasks/INDEX.md` 의 Phase 8 row 참조) 가 `ParsedBlock` 또는 그 변종 (`ParsedPdfPage`, `ParsedImageRegion`, `ParsedAudioSegment`) 를 emit 시작 + `kebab-normalize` 의 lift 를 경유하도록 변경. **측정**: `kebab_parse_md::build_canonical_document` 의 input variant 가 `Vec<ParsedBlock>` 외 medium 의 변종이 추가되는 시점.
2. 즉, fan-in ≥ 2 (parser caller 2개 이상) 가 회복.
3. 또는 lift 로직이 markdown-only specific 함수에서 medium-agnostic 함수로 일반화 필요.

위 trigger 발생 전까지는 `kebab-parse-md` 내부의 `types.rs` + `normalize.rs` module 로 유지.

**의존 그래프 (post-absorb)**:

```text
kebab-core (도메인 모델 — Block, Chunk, SourceSpan, IDs, …)
   ▲
   │
kebab-parse-md (markdown 의 frontmatter + block + types + normalize, 모두 in-crate)
   ▲
   │
kebab-parse-pdf, kebab-parse-image, kebab-parse-code (자체 CanonicalDocument emit)
```

`kebab-parse-md` 는:
- `kebab-core` 에만 의존 (`Block`, `SourceSpan`, `Lang` 등 도메인 타입 사용).
- 다른 어떤 `kebab-*` 에도 의존하지 않는다.
- parser 구체 라이브러리 (`pulldown-cmark`) 와 normalize helper (`unicode-normalization`) 에 의존.

**보존된 surface (계속)**: 5 사용 type 의 정의 (`ParsedBlock` 의 4 field + `ParsedBlockKind` 의 8 variant + `ParsedPayload` 의 8 variant + `Warning` + `WarningKind` 의 4 variant) 와 3 forward-declared struct 의 본문은 P1 spec 의 원본 보존 — wire 표현 (serde rename_all / tag) 변경 0.

**Tracing instrumentation policy**: actual `crates/kebab-normalize/src/lib.rs:109` 에 **explicit literal** `target: "kebab-normalize"` 가 hard-coded (자동 module-path derive 아님). 흡수 후 manual 1-line 갱신 필요. stage label 보존 정책 (warning_agent 보존과 일관) 시 `target: "kebab-normalize"` 유지 — 호환되는 log scraper grep 일관성. 명시적 갱신 원할 시 `target: "kebab-parse-md::normalize"`. **본 spec 의 권장 = 보존** (stage label = "kebab-normalize" — 흡수 후에도 lift stage 의 의미 보존 + R8 mitigation). §3.7 callsite migration 의 (g) 에 1-line touch site 명시. wire / surface impact 0 (§6.8 R8 cross-link).
```

### §3.6 design §8 graph diff

`docs/superpowers/specs/2026-04-27-kebab-final-form-design.md:1457-1491` 의 §8 graph 를 다음으로 **교체**.

**Before (line 1457-1491, 약 35 LOC)**: 본 spec §1.7 의 evidence chain 에서 인용.

**After (제안 — line 단위 diff)**:

```diff
 ```text
 kebab-cli, kebab-tui, kebab-desktop
    └─> kebab-app
          ├─> kebab-source-fs
          │     (p10-2 이후: lang detect + skip policy 내장; kebab-parse-code 와 분리)
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
          ├─> kebab-store-sqlite (DocumentStore, JobRepo, Retriever[lexical])
          ├─> kebab-store-vector (VectorStore)
          ├─> kebab-embed-local
          ├─> kebab-search (Retriever[hybrid])
          ├─> kebab-llm-local
          ├─> kebab-rag
          ├─> kebab-eval
          └─> kebab-config
               └─> kebab-core (모두 의존)
 ```

-`kebab-parse-types` 는 `kebab-core` 와 parsers/normalize 사이의 thin layer (§3.7b 참조). parser-별 중간 표현 (`ParsedBlock`, `ParsedImageRegion`, `ParsedPdfPage`, `ParsedAudioSegment`, `Inline`) 을 한 곳에 모아 (a) `kebab-core` 의 namespace 폭발을 막고 (b) `kebab-normalize` 가 parser 를 직접 import 하지 않게 한다.
+`kebab-parse-md` 는 v0.19.0 부터 `kebab-parse-types` (parser intermediate types) 와 `kebab-normalize` (CanonicalDocument lift) 를 흡수한다 (§3.7b 참조). 4 parser 중 markdown 한 갈래만 lift 를 경유하므로 thin layer 의 가치가 의미를 잃었다. 보존된 5 사용 type + 3 forward-declared struct 의 surface 는 `kebab-parse-md` 의 `pub` re-export 로 backward-compat.

 핵심 금지:
 - UI → store/llm/parse 직접 의존 ✗
 - parse-* → store/llm/embed ✗
-- parse-* → kebab-normalize ✗ (단방향: parsers → kebab-parse-types ← normalize)
+- parse-* (pdf/image/code) → kebab-parse-md ✗ (parser 끼리 cross-import 금지 — markdown 의 lift 가 다른 parser 에 노출되면 안 됨)
 - chunk → llm/embed ✗
-- normalize → store / parse-* ✗
-- kebab-parse-types → 어떤 parser/normalize/store/llm/embed/search/rag/ui ✗ (`kebab-core` 만 의존)
 - 다른 store 와 cross-write ✗
```

**중요**:
- 새로운 forbidden bullet "parse-* (pdf/image/code) → kebab-parse-md ✗" 는 흡수 부산물 — `kebab-parse-md` 가 normalize 까지 흡수했으므로, 만약 다른 parser 가 `kebab-parse-md` 를 import 하면 lift 를 indirect 으로 사용하게 됨 (그리고 §3.7b 의 future re-extraction trigger 가 정확히 발동). 본 forbidden bullet 이 그 invariant 를 명시.
- *MAJOR #5 의 critic 의견에 따라* "kebab-parse-md → store / llm / embed ✗" 명시 룰은 추가하지 않음 — 기존 `parse-* → store/llm/embed ✗` 룰이 흡수된 lift 까지 자동 포함 (parse-md 도 parse-* 의 한 갈래). design contract 중복 룰 방지.

### §3.7 callsite migration

**(a) `kebab-app/src/lib.rs:51`**:
```diff
-use kebab_normalize::build_canonical_document;
+use kebab_parse_md::build_canonical_document;
```

**(b) `kebab-app/src/lib.rs:1119` 의 context string** (byte-identical 한 hunk 는 모두 삭제 — `kb-parse-md::parse_frontmatter` 의 line 1091 / `kb-parse-md::parse_blocks` 의 line 1099 는 변경 0):

```diff
@@ crates/kebab-app/src/lib.rs:1119 @@
-        .context("kb-normalize::build_canonical_document")?;
+        .context("kb-parse-md::build_canonical_document")?;  // 흡수 후 callsite 명시
```

**(c) `kebab-parse-md/src/blocks.rs`**, **`crates/kebab-parse-md/src/frontmatter.rs`**:
```diff
-use kebab_parse_types::{ParsedBlock, ParsedBlockKind, ParsedPayload, Warning, WarningKind};
+use crate::types::{ParsedBlock, ParsedBlockKind, ParsedPayload, Warning, WarningKind};
```

**(d) `kebab-parse-md/src/normalize.rs` (신규 — 이식된 body)**:
```diff
-use kebab_parse_types::{ParsedBlock, Warning, WarningKind};
+use crate::types::{ParsedBlock, Warning, WarningKind};
-pub use kebab_core::{id_for_block, id_for_doc};
+// (re-export 제거 — kebab-parse-md 가 이미 kebab-core 의존, 호출자가 직접 import)
```

**(e) `warning_agent` + hard-coded agent string 정책** (§1.9 의 wire-invisibility 검증 결과 기반).

actual code 의 정확한 distinction:

1. **`warning_agent` 의 body**: 4 `WarningKind` variant 모두 `"kb-parse-md"` 단일 return — `warning_agent` 자체는 "kb-normalize" string 을 반환 안 함 (`crates/kebab-normalize/src/lib.rs:187-191`).

2. **별도 hard-coded `"kb-normalize"` literal 2 곳** (warning_agent 와 별개):
   - `lib.rs:134` — `Normalized` event 의 `agent` field (build_canonical_document body 의 lift 종료 시점 ProvenanceEvent push).
   - `lib.rs:153` — `lift_warnings` loop 의 agent field (lift-stage warning 의 attribution — 현재 AudioRef-deferred drops only).

3. **또 다른 hard-coded agent literal 2 곳** (보존 대상, 흡수 후 의미 변경 없음):
   - `lib.rs:122` — `"kb-source-fs"` (Discovered event — kebab-source-fs 의 stage label, 흡수와 무관).
   - `lib.rs:128` — `"kb-parse-md"` (Parsed event — markdown parse stage label).

```rust
// crates/kebab-parse-md/src/normalize.rs (이식 후 — 정확한 보존 정책)
//
// IMPORTANT: 다음 4 hard-coded agent literal 은 SQLite 의 documents.provenance_json
// BLOB 으로만 persist (wire-invisible). 흡수 후에도 모두 보존 — stage label
// 의미가 crate 흡수와 독립 (stage 자체는 변하지 않음).
//
//   line 122: "kb-source-fs"   (Discovered event)         — 변경 X
//   line 128: "kb-parse-md"    (Parsed event)             — 변경 X
//   line 134: "kb-normalize"   (Normalized event)         — 변경 X (★ 본 PR 보존 결정)
//   line 153: "kb-normalize"   (lift_warnings event)      — 변경 X (★ 본 PR 보존 결정)
//
// warning_agent 자체는 4 WarningKind variant 모두 "kb-parse-md" 단일 return
// (lib.rs:187-191). String 값을 변경하지 않음 — old DB row 의 audit log
// (예: "kb-normalize") 와 new DB row 의 값이 일치하여 history grep
// (`SELECT provenance_json FROM documents WHERE provenance_json LIKE '%kb-normalize%'`)
// 결과의 의미적 일관성이 보존된다.
fn warning_agent(kind: &WarningKind) -> &'static str {
    match kind {
        WarningKind::MalformedFrontmatter | WarningKind::EncodingFallback => "kb-parse-md",
        WarningKind::MalformedTable => "kb-parse-md",
        WarningKind::ExtractFailed => "kb-parse-md",
    }
}
```

**(g) `tracing::debug!` target literal** (`crates/kebab-normalize/src/lib.rs:109`):

```rust
// 본 PR 결정 = 보존 (stage label 일관성 — warning_agent 와 같은 정책).
tracing::debug!(
    target: "kebab-normalize",  // ← 흡수 후에도 변경 X
    "built canonical document doc_id={} blocks={}",
    doc_id.0,
    lifted_blocks.len()
);
```

명시적 갱신 원할 시 → `target: "kebab-parse-md::normalize"` 한 줄 변경. 본 spec 권장 = **보존** — `~/.local/state/kebab/logs/kb.log.YYYY-MM-DD` 의 기존 grep pattern 일관성. R8 mitigation 의 핵심.

**(f) test file 이동**:
```diff
-crates/kebab-normalize/tests/normalize_snapshot.rs
+crates/kebab-parse-md/tests/normalize_snapshot.rs
```

→ destination 이 `kebab-parse-md` 이므로 (이미 `kebab-parse-md` 를 dev-dep 로 사용했던) integration test 가 *자기 자신 crate* 의 test 로 collapse. `Cargo.toml` 의 `kebab-parse-md = { path = "..." }` dev-dep declare 가 자기 참조 → 제거 (cargo standard behavior 명시: `tests/*.rs` integration test 는 자기 crate 의 `lib` 를 자동 link, dev-dep declare 불필요 — `cargo test -p kebab-parse-md --test normalize_snapshot` 가 갱신 후에도 green. R3/Q4 closure).

### §3.8 Cargo workspace.members 갱신

본 변경 은 `Cargo.toml` 의 두 분리된 block 에 영향 — plan/executor 가 한 hunk 로 적용 시 line-context mismatch 가능. 두 hunk 로 분리 (NEW NIT #N6 closure):

**Hunk (a)** — `[workspace] members` 의 두 entry 삭제:

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

**Hunk (b)** — `[workspace.package] version` 의 1-line 변경:

```diff
 [workspace.package]
 edition       = "2024"
 rust-version  = "1.85"
 license       = "MIT OR Apache-2.0"
 repository    = "https://github.com/altair823/kebab"
-version       = "0.18.0"
+version       = "0.19.0"   # frozen design contract (§3.7b 재작성) 변경 trigger — CLAUDE.md "Release / binary version bump"
```

→ count: 24 → **22**. `Cargo.lock` auto-갱신 (§5.11 verification).

### §3.9 HOTFIXES.md entry — 정확한 wording

`tasks/HOTFIXES.md` 의 가장 최근 entry 위 (chronological reverse) 에 다음 4-block 변형을 추가.

```markdown
## 2026-05-26 — design deviation — kebab-normalize + kebab-parse-types 흡수 (24 → 22 crates)

**Symptom**: design deviation — post-PR9 audit (system-architect, `tasks/INDEX.md` L169) identified 두 crate (`kebab-normalize` + `kebab-parse-types`) 가 dead abstraction. design §3.7b 의 "thin layer" raison d'être ((a) `kebab-core` namespace 폭발 방지, (b) normalize 의 parser non-dependence) 가 4 parser 중 1개 (markdown) 만 lift 를 경유하는 현실에서 fan-in/fan-out 모두 1 → layer 의미 잃음. `kebab-parse-types` 의 production caller 가 2개 (`kebab-parse-md` + `kebab-normalize`) 이고 `kebab-normalize` 자체 caller 가 1개 (`kebab-app`) — 모두 markdown 의 lift 경로 안에서 단일 fan-in 경계 가능.

**Root cause**: P1~P10 머지를 거치며 `kebab-parse-pdf` (P7) / `kebab-parse-image` (P6) / `kebab-parse-code` (P10) 가 `CanonicalDocument` 직접 emit 패턴으로 정착. `kebab-normalize::build_canonical_document` 는 markdown-specific `Vec<ParsedBlock>` → `CanonicalDocument` lift 만 책임. design §3.7b 가 가정한 "ParsedBlock 류는 모든 parser 가 emit → normalize 가 일괄 lift" 의 fan-in ≥ 2 시나리오가 미도래 — 그러나 layer 비용 (24 crate workspace, 두 crate 의 lib.rs only structure) 은 계속 지불.

**Action**: `kebab-normalize` (1097 LOC) + `kebab-parse-types` (98 LOC) 를 `kebab-parse-md` 에 흡수 — 22 crate workspace.

- `crates/kebab-parse-md/src/types.rs` (신규): `kebab-parse-types/src/lib.rs` 의 98 LOC 1:1 이식 (5 사용 type + 3 forward-declared struct 보존).
- `crates/kebab-parse-md/src/normalize.rs` (신규): `kebab-normalize/src/lib.rs` 의 production fn body (`build_canonical_document`, `derive_title`, `warning_agent`) 이식. `warning_agent` 의 return string ("kb-normalize") 보존 — SQLite `documents.provenance_json` 의 audit log 일관성 (wire-invisible, see §1.9).
- 3 dead struct (`ParsedImageRegion` / `ParsedPdfPage` / `ParsedAudioSegment`) 는 보존 — v0.20+ image/pdf normalize integration 의 future surface (본 spec §11 참조).
- `crates/kebab-parse-md/src/lib.rs`: `pub use crate::types::*; pub use crate::normalize::{build_canonical_document, derive_title};` re-export 추가.
- `crates/kebab-parse-md/src/{blocks,frontmatter}.rs`: `use kebab_parse_types::*` → `use crate::types::*`.
- `crates/kebab-app/src/lib.rs:51`: `use kebab_normalize::build_canonical_document` → `use kebab_parse_md::build_canonical_document`.
- `crates/kebab-app/Cargo.toml`: `kebab-normalize` regular dep 제거 + `kebab-parse-types` regular dep 제거 (후자는 dead dep — `cargo tree -p kebab-app | grep kebab_parse_types` 0줄 검증으로 incidental cleanup).
- `Cargo.toml` workspace.members: `kebab-normalize` + `kebab-parse-types` entries 제거. `workspace.package.version` 0.18.0 → **0.19.0** (frozen design contract 변경 trigger — CLAUDE.md "Release / binary version bump").
- `crates/kebab-normalize/` + `crates/kebab-parse-types/` 디렉토리 전체 삭제 (`git rm -r`).
- `crates/kebab-normalize/tests/normalize_snapshot.rs` → `crates/kebab-parse-md/tests/normalize_snapshot.rs` (mechanical move, `Cargo.toml` 자기 참조 dev-dep 제거).
- `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md` §3.7b 재작성 (보존 + future re-extraction trigger 명시) + §8 graph 갱신 (3 edge 제거 + 2 forbidden bullet 의미 갱신, "parse-* → kebab-normalize ✗" 룰 의미 부분 폐기).
- `docs/ARCHITECTURE.md` crate graph + 디렉토리 tree mechanical 갱신.
- `tasks/INDEX.md` L169 의 "kebab-normalize 흡수" defer mention 해소.

**Amends**: spec `docs/superpowers/specs/2026-05-26-normalize-absorption-spec.md` cross-link. design `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md` §3.7b + §8 동시 갱신 (CLAUDE.md "Changing the design doc requires updating every referencing task spec in the same PR" — 본 PR 의 design 갱신은 ~25 referencing task spec 의 raison d'être 인용을 stale 화하지만, frozen 원칙에 따라 mechanical update 없음. live source of truth = 본 HOTFIXES entry). 영향받는 task spec 의 `Forbidden dependencies` 또는 `contract_sections: ["§3.7b"]` 인용은 historical contract 로 보존됨 — `tasks/p1/p1-2-parser-types.md`, `tasks/p1/p1-3-markdown-parser.md`, `tasks/p1/p1-4-normalize.md`, `tasks/p9/p9-fb-07-md-title-fallback.md` 등. (Wire / surface impact: 0건 — CLI / TUI / MCP / `--json` 출력 / config / XDG path / parser_version 모두 unchanged. wire-invisible `provenance.events[].agent` 의 stage label "kb-normalize" 도 보존 — old DB row 와 new DB row 의 audit log 일관성.)
```

### §3.10 `kebab-app` 의 dead `kebab-parse-types` regular dep incidental cleanup

본 PR 에 같이 묶이는 이유 (team-lead 결정 #4):

- `kebab-app/Cargo.toml:15` 의 `kebab-parse-types = { path = "../kebab-parse-types" }` regular dep declare.
- `grep -rn "kebab_parse_types" crates/kebab-app/src/`: 0 hit.
- 즉 `kebab-app` 은 `kebab-parse-types` 의 production caller 가 아님 (analyst evidence).
- 본 PR 이 `kebab-parse-types` 를 삭제하면 declare 도 어차피 제거 필요 → incidental cleanup. 별 PR 불필요.

Verification: `cargo build -p kebab-app` 가 dead dep 제거 후에도 green. `cargo tree -p kebab-app | grep kebab_parse_types` = 0줄.

## §4 Open questions

### §4.1 Resolved (사용자 결정 + analyst evidence)

- **target_version 0.19.0 vs 0.18.1**: → 0.19.0 (frozen design contract 변경, CLAUDE.md release rule). [resolved by user]
- **Destination = parse-md vs app vs core**: → parse-md (caller 일관성 + 단일책임). [resolved by user + §3.1 evidence]
- **3 forward-declared struct 처리**: → 보존 + future surface 명시 (re-extraction trigger). [resolved by user, §3.3 + §3.5]
- **~25 referencing task spec mechanical update**: → 0건 (frozen 룰). [resolved by user]
- **HOTFIXES entry format**: → 4-block 변형 ("design deviation" Symptom). [resolved by user + §3.9]
- **kebab-app 의 dead parse-types dep**: → 같은 PR incidental cleanup. [resolved by user + §3.10]
- **warning_agent wire visibility**: → wire-invisible (§1.9 verified, **16 wire schema 모두 0 hit** — search_response.schema.json:35 의 description 산문 false-positive 명시). String 값 보존 정책 (§3.7e). [resolved by planner verification]

### §4.2 Unresolved (critic round 에서 결정)

- **Q1** — `warning_agent` 의 return string 정책: 보존 (`"kb-normalize"` 유지) vs 갱신 (`"kb-parse-md"` 단일화)?
  - 보존 근거: SQLite 의 audit log 일관성 (old + new DB row 의 grep 의미 보존), stage label 의미 (lift stage ≠ crate name).
  - 갱신 근거: crate name 과 일치하여 newcomer 혼란 감소.
  - **본 spec 의 현재 권장 = 보존** (§3.7e). critic round 에서 재확정.

- **Q2** — `kebab-parse-md` 의 description string 의 정확한 wording:
  - 현재: `"Markdown frontmatter and block parsing into kb-core::Metadata / kb-parse-types intermediates"`.
  - 흡수 후 후보: `"Markdown frontmatter + block parsing + canonical-document lift (absorbed kb-parse-types + kb-normalize, see HOTFIXES.md 2026-05-26)"`.
  - HOTFIXES cross-link 을 description string 에 두는 것이 적절한지 critic 의견.

- **Q3** — `kebab-parse-md` 의 lib.rs re-export 가 `pub use crate::types::*; pub use crate::normalize::{build_canonical_document, derive_title};` glob + specific 혼용:
  - 5 type + 3 struct 의 glob 가 future addition 의 surface leak 위험.
  - 대안: 8 type 모두 explicit re-export (`pub use crate::types::{ParsedBlock, ParsedBlockKind, ..., ParsedAudioSegment};`).
  - critic 의견.

- **Q4** — `kebab-parse-md` 의 dev-dep 정리: 흡수된 `kebab-normalize/tests/normalize_snapshot.rs` 가 이전에는 `kebab-parse-md` 를 dev-dep 으로 사용해서 fixture 를 만들었음 (`crates/kebab-normalize/Cargo.toml [dev-dependencies] kebab-parse-md`). 흡수 후 자기 자신을 dev-dep 으로 declare 할 필요 없음 (cargo가 자기 crate test 자동 link). cargo 가 어떻게 처리하는지 별 verification 필요한지?
  - **본 spec 의 현재 권장 = 자기 참조 dev-dep declare 제거** (in-crate integration test 는 `tests/*.rs` 가 `use kebab_parse_md::*;` 로 직접 link). critic round verification 으로 cargo 동작 확인 필요.

- **Q5** — kebab-chunk + kebab-store-sqlite 의 `kebab-normalize` dev-dep:
  - `grep -l "kebab-normalize" crates/{kebab-chunk,kebab-store-sqlite}/Cargo.toml` — 본 spec 에서 정확히 검증 필요. dev-dep 가 있으면 `kebab-parse-md` 로 갈음.
  - **본 spec 의 현재 권장 = §5.3 verification step 에서 mechanical 갱신** + critic 검토.

### §4.3 Open questions log

본 spec 의 Q1~Q5 + critic round 에서 추가될 항목들은 critic round 종료 시 `tasks/HOTFIXES.md` 또는 followup spec 으로 closure.

## §5 Verification plan

### §5.1 Unit + integration test 회귀

`cargo test --workspace --no-fail-fast -j 1` (CLAUDE.md 의 "-j 1 for the full workspace test isn't optional" 룰).

- **Baseline**: PR-9d 머지 시점 1313 tests (analyst evidence chain 의 baseline).
- **Expected post-absorb**: 1313 - X (kebab-normalize + kebab-parse-types 의 test 수) + X (destination 으로 이동된 동일 수). **net delta = 0**.
  - 단, 자기 참조 dev-dep 제거로 *통합되는 test scope* 변경 가능 — 본 spec 작성 시점 baseline N 정확히 계측 → plan 단계에서 채움.
  - 본 spec 의 약속: net delta = 0 또는 +N(신규 검증 test 의 의도된 addition).

### §5.2 Workspace ground truth invariants

다음 invariant 가 PR head 에서 모두 green:

| invariant | 확인 명령 | expected |
|---|---|---|
| 22 crate workspace | `cargo metadata --no-deps --format-version 1 \| jq '.workspace_members \| length'` (또는 `ls -d crates/*/ \| wc -l`) — Cargo.toml 의 comment / whitespace 무관 robust | 22 |
| `kebab-normalize` 디렉토리 부재 | `ls crates/kebab-normalize/ 2>&1` | "No such" |
| `kebab-parse-types` 디렉토리 부재 | `ls crates/kebab-parse-types/ 2>&1` | "No such" |
| `kebab-app` 의 dep tree | `cargo tree -p kebab-app --depth 2 \| grep -E "kebab_(parse_types\|normalize)"` | 0 줄 |
| `kebab-app` 의 source import | `grep -rn "kebab_normalize\|kebab_parse_types" crates/kebab-app/src/` | 0 hit |
| 5 사용 type 의 surface 보존 | `cargo doc -p kebab-parse-md --no-deps` + `cat target/doc/kebab_parse_md/index.html \| grep -c "ParsedBlock\|ParsedPayload\|Warning"` | ≥ 5 |
| 3 forward-declared struct 의 surface 보존 | (위 doc grep) | ≥ 3 |
| clippy gate | `cargo clippy --workspace --all-targets -- -D warnings` | 0 warning |

### §5.3 Cargo.toml 의 dev-deps grep + mechanical migration

흡수 전:
```bash
$ grep -l "kebab-normalize" crates/*/Cargo.toml
crates/kebab-app/Cargo.toml
crates/kebab-chunk/Cargo.toml
crates/kebab-normalize/Cargo.toml
crates/kebab-store-sqlite/Cargo.toml
```

흡수 후 expected:
```bash
$ grep -l "kebab-normalize" crates/*/Cargo.toml
(0 line — kebab-app: 본 PR 에서 제거. kebab-chunk + kebab-store-sqlite: dev-dep 가 있다면 kebab-parse-md 로 갈음. kebab-normalize: 디렉토리 자체 삭제)
```

`kebab-parse-types`:
```bash
$ grep -l "kebab-parse-types" crates/*/Cargo.toml
(0 line — kebab-app: 본 PR 에서 dead dep 제거. kebab-parse-md + kebab-normalize: 본 PR 에서 흡수 / 삭제. kebab-parse-types: 디렉토리 자체 삭제)
```

### §5.4 Wire schema 회귀

- `docs/wire-schema/v1/*.json` 16 파일 모두 변경 0. `git diff main..HEAD -- docs/wire-schema/v1/ | wc -l` = 0.
- `provenance.events[].agent` 의 stage label "kb-normalize" 보존 (§3.7e) — old DB 의 audit log 와 일관성.

### §5.5 SMOKE 회귀

`docs/SMOKE.md` 가 정의하는 isolated TempDir KB pipeline (`--config /tmp/kebab-smoke/config.toml`) 의 ingest + search + ask 가 흡수 전후 byte-identical wire 출력. plan 단계에서 dogfood snapshot 비교.

### §5.6 design doc 갱신 검증

- `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md` §3.7b (line 703-764) 가 §3.5 의 wording 으로 교체.
- 동일 doc §8 (line 1457-1491) 이 §3.6 의 diff 로 갱신.
- `git diff main..HEAD -- docs/superpowers/specs/2026-04-27-kebab-final-form-design.md` 의 hunk 가 위 2 section 만 touch.

### §5.7 referencing task spec 의 frozen 검증

`grep -rln "kebab-parse-types\|kebab-normalize\|§3.7b" tasks/` 의 hit 가 모두 본 PR 의 diff 에서 **0 hit** (frozen 룰).

본 spec 의 §7 가 명시한 4 frozen task spec (p1-2, p1-3, p1-4, p9-fb-07) + 다른 ~25 referencing task spec 모두 mechanical update 없음.

### §5.8 ARCHITECTURE.md + INDEX.md 갱신

- `docs/ARCHITECTURE.md` 의 crate graph + 디렉토리 tree 갱신 — 24 → 22 crate, parse-types + normalize 절 삭제. mechanical.
- `tasks/INDEX.md` L169 의 "kebab-normalize 흡수, … v0.18.1+ defer" 의 mention 갱신 — "(v0.19.0 closure — see HOTFIXES.md 2026-05-26)" cross-link 추가.
- `tasks/INDEX.md` 의 "Future work / deferred" 섹션 (없으면 신설) 에 §11.7 의 한 줄 entry 추가.

### §5.9 `cargo deny` workspace dep validation

```bash
# workspace 의 dep 룰 (license + advisories + ban + sources) 검증.
# 흡수된 두 crate 의 directory 삭제 후에도 ban 룰 (예: duplicate crate 금지)
# 위반 없어야 함.
$ cargo deny check
ok 0 errors, 0 warnings
```

### §5.10 `target/` 산출물 cleanup (CLAUDE.md 룰)

흡수된 두 crate 의 `target/debug/deps/libkebab_normalize-*.rlib` / `libkebab_parse_types-*.rlib` 가 stale artifact 로 남으면 build cache pollution. PR head 의 verification 직전 `cargo clean` 한 번 실행 — CLAUDE.md 의 "Run `cargo clean` routinely after each merged PR" 룰. (16 GB RAM 머신 의 link step 압박 회피 — §6.7 R7).

```bash
$ cargo clean
$ cargo test --workspace --no-fail-fast -j 1   # full re-build + test
```

### §5.11 `Cargo.lock` 변경 검증

```bash
# kebab-normalize / kebab-parse-types 의 [[package]] entry 가 삭제.
$ grep '^name = "kebab-normalize"\|^name = "kebab-parse-types"' Cargo.lock
(0 hit)

# kebab-parse-md 의 dependencies 에 unicode-normalization 추가.
$ awk '/^\[\[package\]\]/,/^$/{if(/name = "kebab-parse-md"/)f=1; if(f) print; if(/^$/ && f){f=0; print "---"}}' Cargo.lock | grep "unicode-normalization"
unicode-normalization
```

## §6 Risks

### §6.1 R1 — §3.7b strike 의 stakeholder impact

**위험**: design contract 의 §3.7b 가 P1~P10 의 ~25 task spec 의 `contract_sections` 또는 `Forbidden dependencies` 에서 인용됨. frozen 룰에 따라 mechanical update 안 함 → reading these specs in isolation 시 stale 한 raison d'être 인용 노출.

**Mitigation**:
- HOTFIXES.md 의 dated entry 가 live source of truth (CLAUDE.md "Live deviations from the original contract go in `tasks/HOTFIXES.md`" 룰).
- design §3.7b 재작성 가 "원래 의도 / 현재 상태 / 보존된 surface / future re-extraction trigger" 4 단락 구조로, original intent + 현재 reality + future contingency 모두 명시.
- 본 spec 의 §3.5 wording 이 §3.7b 의 *원래* paragraph 를 인용한 task spec 도 의미적 backward-compat (보존된 surface + future trigger 가 명시되어 원래 intent 의 일부 보존).

### §6.2 R2 — `warning_agent` string 정책 (Q1) 의 audit log 일관성

**위험**: 흡수 후 `"kb-normalize"` 문자열을 그대로 emit 하면 newcomer 가 "이 crate 가 어디 갔지?" 혼란. 반대로 `"kb-parse-md"` 로 갈음하면 old DB 와 new DB 의 grep 결과 diverge.

**Mitigation**:
- §3.7e 의 권장 (보존) 가 *stage label vs crate label* 의 의미 분리 강조. comment 로 rationale inline.
- HOTFIXES entry 가 dated audit log — newcomer 의 첫 grep 결과가 `tasks/HOTFIXES.md` 의 2026-05-26 entry 로 land.

### §6.3 R3 — 자기 참조 dev-dep (Q4) 의 cargo 동작

**위험**: `kebab-normalize/tests/normalize_snapshot.rs` 가 `kebab-parse-md` 를 fixture builder 로 사용. 흡수되어 `kebab-parse-md` 안으로 이식되면 *자기 참조 dev-dep* 가 됨. cargo 가 이를 자동 link 하는 패턴 vs declare 필요 패턴의 분기.

**Mitigation**:
- plan 단계에서 cargo behavior 검증 (소규모 sandbox 또는 `cargo test -p kebab-parse-md --test normalize_snapshot` 의 dry run).
- 본 spec 의 §3.7f 가 "자기 참조 dev-dep declare 제거" 권장 — in-crate integration test 는 `tests/*.rs` 가 `use kebab_parse_md::*;` 로 직접 link. cargo 의 standard behavior.

### §6.4 R4 — future re-extraction trigger 의 비용 추정

**위험**: 흡수 후 `kebab-parse-pdf` 또는 `kebab-parse-image` 가 향후 `ParsedBlock` 을 emit 하도록 변경되면 (§3.5 의 trigger), layer 재추출 필요. 그 비용이 본 흡수 비용보다 클 위험.

**Mitigation**:
- 본 spec 의 §3.5 wording 이 trigger 1~3 을 명시 → future audit 시 명확.
- 1:1 이식 (types.rs + normalize.rs 분리 구조) 가 재추출 시 cherry-pick 용이.
- 그러나 ParsedBlock 의 emit-site 가 markdown 1개 → 2개로 확장될 조짐이 v0.19.0 시점에 0 (P8 audio 도 deferred). 본 위험의 발현 확률 = low.

### §6.5 R5 — 5 사용 type 의 visibility 후퇴

**위험**: 5 type (`ParsedBlock` 등) 이 `pub` re-export 로 destination 에 surface 보존되지만, future caller 가 `kebab-parse-types::*` direct import 가 익숙해진 상태라면 ergonomic 회귀.

**Mitigation**:
- `kebab-parse-md::*` 의 single-crate import 경로가 newcomer 에게 더 직관적 (markdown parsing 의 unified surface).
- 4 frozen task spec (p1-2, p1-3, p1-4, p9-fb-07) 이 explicit type-by-type 인용 (`use kebab_parse_types::ParsedBlock`) → 이들은 frozen 으로 historical contract, mechanical update 없음. 새 caller (있다면) 는 `kebab-parse-md::ParsedBlock` 사용.

### §6.6 R6 — DB schema 영향 (provenance_json BLOB)

**위험**: `provenance_json` BLOB 안의 `agent` string 값이 변경되면 (Q1 갱신 정책 선택 시) old DB 의 entry 와 new DB 의 entry 가 diverge — UI 가 그 차이를 surface 하지 않으나, future analytic query 가 stale 한 `WHERE agent = 'kb-normalize'` filter 를 적용하면 row miss.

**Mitigation**:
- §3.7e 의 보존 정책 (권장) 이 본 위험 0.
- 만약 critic round 에서 Q1 을 "갱신" 으로 결정 시, V00X migration 또는 lazy re-classification helper 추가 — 본 spec 의 scope 외 (별 PR).

### §6.7 R7 — 16 GB RAM 의 build pressure

**위험**: CLAUDE.md 의 "Serial cargo builds only" 룰 (MEMORY.md). 본 흡수 PR 의 verification 이 `cargo test --workspace --no-fail-fast -j 1` 1회 + `cargo clippy --workspace --all-targets -- -D warnings` 1회 = 2회 full build. lance/datafusion link step 의 RAM pressure 가 PR-9c-2 머지 시점에 확인된 적 있음.

**Mitigation**:
- per-crate 단위 검증 (`cargo test -p kebab-parse-md` + `cargo test -p kebab-app`) 을 plan 단계에서 우선 → full workspace 는 verifier round 1회.
- `cargo clean` 직전 후 (CLAUDE.md 룰 — "Run `cargo clean` routinely after each merged PR").

### §6.8 R8 — `tracing` instrumentation 의 회귀

**위험**: `kebab-normalize` 의 `tracing::*` calls 가 destination 으로 이식 후 module path 변경 (`kebab_normalize::lib::...` → `kebab_parse_md::normalize::...`). log scraper (있다면) 의 module-path filter 가 stale.

**Mitigation**:
- `~/.local/state/kebab/logs/kb.log.YYYY-MM-DD` 의 grep pattern (있다면) 갱신 안내 — README / SMOKE.md 변경 없음 (verified, internal 항목).
- log scraper 자체가 user-visible surface 아님 (developer-facing) → wire / surface impact 0 유지.

### §6.9 R9 — kebab-parse-md 의 dependency 폭증

**위험**: 흡수 후 `kebab-parse-md/Cargo.toml` 의 deps 가 기존 (`kebab-core`, `pulldown-cmark`, `serde_yaml_ng`, `toml`, `lingua`, `tracing` …) + 흡수 (`unicode-normalization`) 로 폭증. lingua 의 build time + binary size 가 markdown parse + lift 두 책임을 모두 가지는 crate 에 concentrate.

**Mitigation**:
- 신규 deps 추가 = `unicode-normalization` 1개만 (이미 `kebab-app` 도 사용 중인 `0.1` major). version drift 없음.
- 다른 deps 는 모두 흡수 전 `kebab-parse-md` 에 이미 존재.
- 본 위험의 실질 영향 ≈ +1 dep (`unicode-normalization`) → 영향 minimal.

### §6.10 R10 — frozen p1-4 surface (`pub use kebab_core::{id_for_block, id_for_doc}`) re-export 제거

**위험**: `tasks/p1/p1-4-normalize.md:60-62` 의 frozen public surface 가 `kebab-normalize::{id_for_block, id_for_doc}` re-export 를 명시. 본 spec §3.3 의 결정 (re-export 제거) 가 그 frozen surface 의 후퇴.

**Mitigation (production caller 0 검증)**:

```bash
# kebab-normalize::id_for_* re-export 의 production caller 검색 (목표 = 0 hit).
$ grep -rn "kebab_normalize::id_for_\|use kebab_normalize::{.*id_for" crates/*/src/
(0 hit)

# 비교: kebab_core::id_for_* 직접 import 의 caller (production + test mod 모두):
$ grep -rn "id_for_block\|id_for_doc" crates/*/src/
crates/kebab-chunk/src/code_*_ast_v1.rs:187     ← #[cfg(test)] mod tests 안의 import (production 아님)
crates/kebab-chunk/src/code_*_ast_v1.rs:196     ← #[cfg(test)] mod tests 안의 call
crates/kebab-chunk/src/code_*_ast_v1.rs:207     ← #[cfg(test)] mod tests 안의 call
crates/kebab-parse-md/src/frontmatter.rs:592    ← #[cfg(test)] mod tests 안의 import (production 아님)
crates/kebab-parse-md/src/frontmatter.rs:737-738 ← #[cfg(test)] mod tests 안의 call
crates/kebab-normalize/src/lib.rs               ← production (`build_canonical_document` body 의 `id_for_doc(&asset.workspace_path, &asset.asset_id, parser_version)` direct call, lib.rs:66 부근)
```

→ **production code** 의 `id_for_*` direct caller = `kebab-normalize::lib.rs` 자신만 (lift body 안의 single call). 다른 모든 `id_for_*` hit 은 `#[cfg(test)] mod tests` 안의 fixture builder. `kebab-normalize::id_for_*` re-export 경유 production caller = 0 verified.

→ frozen surface 의 후퇴이나 production caller 0 → real-world 영향 0. tasks/p1/p1-4 의 frozen 룰은 historical contract 로 보존 — 본 PR 의 HOTFIXES entry (§3.9) 가 live source. **본 R10 은 critic round 에서 Q3 (lib.rs re-export glob vs explicit) 와 함께 closure 요망**.

## §7 Wire / surface impact

| surface | impact | 근거 |
|---|---|---|
| wire schema (`docs/wire-schema/v1/*.json` 16 file) | **0** | §1.9 의 11 schema 0 hit + §5.4 의 git diff = 0 |
| `--json` 출력 (`ingest_report`, `search_hit`, `answer`, `chunk_inspection`, `doc_summary`, `error`, …) | **0** | provenance 가 어떤 schema 에도 export 되지 않음 |
| CLI (`kebab` 의 subcommand + flag + exit code) | **0** | facade 변경 0, CLI 의 surface 는 `kebab-app::*_with_config` 의 signature 보존 |
| TUI (`kebab tui` 의 키 binding + pane) | **0** | UI crate 영향 0 |
| MCP (`kebab-mcp` 의 tool definitions + JSON-RPC) | **0** | MCP 가 `kebab-app` 통해 호출 — facade signature 보존 |
| config (`config.toml` field, `KEBAB_*` env, XDG path) | **0** | 변경 0 |
| `Cargo.toml workspace.version` | **0.18.0 → 0.19.0** | frozen design contract (§3.7b) 변경 trigger — CLAUDE.md "Release / binary version bump" |
| `Cargo.lock` | auto-갱신 | `cargo build` 후 자동 |
| `Cargo.toml workspace.members` count | **24 → 22** | §3.8 |
| README.md | **0** | user-facing 변경 0 |
| HANDOFF.md | 1 줄 추가 | "머지 후 발견된 버그 / 결정" 의 cross-link (HOTFIXES 2026-05-26) |
| `docs/ARCHITECTURE.md` | 갱신 | crate graph + directory tree mechanical |
| `tasks/INDEX.md` | 2 갱신 | (a) L169 의 defer mention closure, (b) "Future work / deferred" 섹션 신설 (없으면) + image/pdf normalize integration 한 줄 entry — §11 cross-link |
| `tasks/HOTFIXES.md` | 신규 entry | §3.9 의 4-block 변형 (Action 라인에 §11 future surface cross-link 포함) |
| 4 frozen task spec (p1-2, p1-3, p1-4, p9-fb-07) | **0** | frozen 룰 |
| ~25 referencing task spec | **0** | frozen 룰 |
| `parser_version` cascade | **0** | lift 로직 의미 보존 |
| `chunker_version`, `embedding_version`, `prompt_template_version`, `index_version` | **0** | 영향 없음 |
| Cargo features | **0** | 변경 0 |
| SQLite schema (V00X migration) | **0** | `documents.provenance_json` 의 string 값 보존 정책 (§3.7e) |
| `--json` `error.v1` 의 `code` field | **0** | 영향 없음 |
| `~/.local/state/kebab/logs/kb.log.YYYY-MM-DD` 의 `tracing::span` module path | mechanical 변경 | log scraper 가 user-visible surface 아님, README 변경 0 |
| integration package (`integrations/claude-code/kebab/SKILL.md`) | **0** | wire schema 변경 0 → SKILL.md 갱신 trigger 없음 |
| binary release tag | **v0.19.0 cut 필수** | CLAUDE.md "Bump 시점 = release 시점 같은 commit" + gitea-ops `gitea-release v0.19.0` |

## §8 Out of scope

본 spec 의 scope 외:

- **Lens 1 (kebab-source-fs dep lightening)**: 이미 PR #185 머지 완료 (`d4395a3`). 본 spec 과 독립.
- **Lens 3 (Extractor + Chunker dispatch unification)**: post-PR9 audit 의 sibling defer item — 별 spec + 별 PR.
- **4 parser 의 normalize 의존성 신규 추가**: 현재 markdown 만 normalize 를 경유. pdf/image/code 의 normalize 경유는 future re-extraction trigger 의 발동 시점 (§3.5). image/pdf normalize integration 의 미구현 design intent 자체는 §11 (Future work) 에 영구 보존.
- **`kebab-core` 의 도메인 타입 변경**: 변경 없음. `Block`, `CanonicalDocument`, `Provenance` 모두 그대로.
- **mechanical referencing task spec update**: frozen 룰. live source = HOTFIXES.md.
- **V00X migration (DB schema 변경)**: 발생 안 함 (`provenance_json` BLOB 보존, string 값 정책 §3.7e).
- **wire schema v1 → v2 major bump**: 발생 안 함 (§7 verified).
- **`derive_title` 또는 `build_canonical_document` 의 signature 변경**: 변경 없음. callsite 이동만.
- **새 forward-declared struct 추가**: 보존된 3 struct 외 추가 없음.
- **post-absorb `kebab-parse-md` 의 internal refactor**: scope 외 — 흡수가 1:1 이식이므로 destination 의 module 구조 재정렬 등은 follow-up PR.

## §9 References

- design contract: `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md` §3.7b (line 703-764) + §8 (line 1457-1491).
- sub-item 1 (sibling defer): `docs/superpowers/specs/2026-05-26-source-fs-dep-lightening-spec.md` (PR #185 완료).
- audit log root: `tasks/INDEX.md` L169 (PR #181 의 system-architect review 결론).
- frozen task spec — `tasks/p1/p1-2-parser-types.md`, `tasks/p1/p1-3-markdown-parser.md`, `tasks/p1/p1-4-normalize.md`, `tasks/p9/p9-fb-07-md-title-fallback.md`.
- CLAUDE.md (project) — "Spec contract" / "Task specs themselves stay frozen" / "Live deviations from the original contract go in `tasks/HOTFIXES.md`" / "Release / binary version bump" / "Versioning cascade" / "Wire schema v1".
- CLAUDE.md (machine) — "Serial cargo builds only" / "Disk layout: /build/ 우선".
- MEMORY.md — "Phase priorities — P8 deferred, P9 first" (audio 미존재 근거).
- wire schema directory: `docs/wire-schema/v1/` (16 file 검증 — §1.9).
- ProvenanceEvent definition: `crates/kebab-core/src/metadata.rs:65-72` (agent field internal-only).
- Provenance persistence: `crates/kebab-store-sqlite/src/documents.rs:726` (`provenance_json` BLOB 직렬화).

## §10 Round 1+ closure table

| Round | Reviewer | Verdict | Issues | Closure |
|---|---|---|---|---|
| 1 | critic (round 1) | REQUEST_CHANGES — 3 CRITICAL + 8 MAJOR + 4 MINOR + 2 NIT | 본 round 2 revision 에서 all-closed 처리 — §10.1 의 finding-by-finding closure 참조 | 본 spec 의 round 2 revision (2026-05-26, planner) |
| 2 | critic (round 2) | REQUEST_CHANGES — 2 NEW MAJOR + 3 NEW MINOR + 1 NEW NIT (round 1 의 22 finding 중 18 fully closed + 1 partial + 1 stale-line-ref + 1 inverse-closure + 1 already-complete + 1 false-positive 재확정) | 본 round 3 revision 에서 closed — §10.2 의 finding-by-finding closure 참조 | 본 spec 의 round 3 revision (2026-05-26, planner) |

### §10.1 round 1 finding-by-finding closure

| ID | Severity | Finding | Closure |
|---|---|---|---|
| #1 | CRITICAL | §1.5 signature byte-mismatch (`asset: &AssetInfo` 등) | **CLOSED** — §1.5 의 signature 정정 (actual source `crates/kebab-normalize/src/lib.rs:60-66, :360` 와 byte-identical). `derive_title` 의 `&[Block]` (lifted, NOT ParsedBlock) plan-time type-error 회피 inline 명시. `tasks/p1/p1-4-normalize.md:54-62` cross-link 추가. |
| #2 | CRITICAL | §1.4 dev-dep claim 부정확 (kebab-parse-md 의 dev-dep 에 normalize / parse-types 없음) | **CLOSED** — §1.2 caller table 갱신 (reverse dev-dep: normalize → parse-md, store-sqlite → normalize, chunk → normalize). §1.4 의 잘못된 dev-dep row 텍스트 정정. §3.7 (f) 의 cargo behavior 명시. |
| #3 | CRITICAL | §11 미존재 (critic 오해) | **FALSE-POSITIVE** — §11 main header line 997 부터 §11.8 까지 실존 (`grep -n "^## §11" spec` verified post-round-3-end; round 1 시점에는 line 793 부터 시작했고 round 2 의 +136 line + round 3 의 +68 line revision 으로 shift). critic round 1 의 stale spec read 또는 section search 부정확 의심. action 불필요. **Line reference 정책**: §10.1 의 모든 line number 는 grep cross-check 후 명시 (NEW MINOR #N4 closure). |
| #4 | MAJOR | §3.7 (b) 의 byte-identical diff (의미 없음) | **CLOSED** — 첫 hunk 삭제 + `crates/kebab-app/src/lib.rs:1119` line number 명시. |
| #5 | MAJOR | §3.6 의 새 forbidden bullet 중복 | **CLOSED** — `kebab-parse-md → store / llm / embed ✗` 삭제 + commentary 한 줄로 대체 ("기존 `parse-* → store/llm/embed ✗` 룰이 흡수된 lift 까지 자동 포함"). |
| #6 | MAJOR | §3.5 future trigger 1 의 audio P8 ambiguity | **CLOSED** — trigger 1 의 audio 항목에 "P8 도입 시 (현재 deferred, `tasks/INDEX.md` Phase 8 row 참조)" timing 명시. `build_canonical_document` input variant 변경 의 measurement trigger 명시. §11.6 와 cross-link. |
| #7 | MAJOR | §1.9 의 11 schema explicit (16 누락) | **CLOSED** — 16 row 모두 explicit expand. §4.1 wording "11 schema" → "16 wire schema". search_response.schema.json:35 의 description prose false-positive 명시. |
| #8 | MAJOR | §3.9 HOTFIXES 5-block (4-block 위반) | **CLOSED** — 5번째 block 의 내용을 Amends block 의 마지막 문장 (괄호 inline) 으로 흡수. 4-block 회복. |
| #9 | MAJOR | §3.3 의 id_for_* 후퇴 명시 부족 | **CLOSED** — table 의 row note 갱신 (`crates/kebab-chunk/src/code_*_ast_v1.rs` 7+ 곳 + `kebab-parse-md/src/frontmatter.rs:592` 모두 `kebab_core` 직접 import — production caller 0 검증). §6.10 R10 추가 (grep mitigation cmd). |
| #10 | MAJOR | §5.2 의 22 crate 검증 명령 fragile | **CLOSED** — `cargo metadata --no-deps --format-version 1 \| jq '.workspace_members \| length'` 으로 robust 화 (또는 `ls -d crates/*/ \| wc -l`). |
| #11 | MAJOR | §3.4 chunk + store-sqlite dev-dep migration 누락 | **CLOSED** — 두 crate 의 dev-dep diff explicit + 통합 test source 의 `use kebab_normalize::*;` → `use kebab_parse_md::*;` migration 명시. |
| #12 | MINOR | §1.2 의 evidence cmd 인용 | **CLOSED** — §1.2 의 caller table heading 에 `cat crates/.../Cargo.toml \| grep -A20 "dev-dependencies"` 명시. |
| #13 | MINOR | §3.5 의 parenthesis wording | **CLOSED** — `**보존된 surface (계속)**` block + tracing instrumentation block 으로 통합. parenthesis 풀어 body 문장. |
| #14 | MINOR | §1.6 의 P8 deferred cross-link generic-phrasing | **CLOSED** — `MEMORY.md` reference 를 `tasks/INDEX.md` 의 Phase 8 row 참조로 generic 화. |
| #15-16 | NIT | §3.7 (e) SQL 예제 + §6.7 RAM mitigation — 둘 다 정확 | **NO-ACTION** (정확 — critic 가 명시). |
| (Missing) #1 | Missing | §11 신설 | **ALREADY-COMPLETE** — §11 line 793-862 실존 (round 1 revision 시 완료). |
| (Missing) #2 | Missing | chunk + store-sqlite test source `use` migration | **CLOSED** — §3.4 의 chunk + store-sqlite dev-dep diff 와 함께 명시. |
| (Missing) #3 | Missing | `cargo deny` workspace dep validation | **CLOSED** — §5.9 신설. |
| (Missing) #4 | Missing | `target/` clean policy | **CLOSED** — §5.10 신설 (CLAUDE.md 룰 cross-link). |
| (Missing) #5 | Missing | `Cargo.lock` 변경 검증 | **CLOSED** — §5.11 신설 (kebab-normalize / kebab-parse-types `[[package]]` 0 hit + unicode-normalization 추가 검증). |
| (Missing) #6 | Missing | `tracing` instrumentation target string 정책 | **CLOSED** — §3.5 의 "Tracing instrumentation policy" block 신설. |
| (Ambiguity) #1 | Ambiguity | §3.7 (f) 자기 참조 dev-dep cargo standard behavior | **CLOSED** — §3.7 (f) 의 wording 갱신 (cargo standard behavior 명시 + `cargo test -p kebab-parse-md --test normalize_snapshot` verification cmd). |
| (Ambiguity) #2 | Ambiguity | §1.9 의 wire 정의 | **CLOSED** — §1.9 의 "wire 의 정의 (본 spec 범위 내)" block 신설 (JSON-RPC + CLI `--json` + 외부 통합. SQLite BLOB 는 wire 외). |

### §10.2 round 2 finding-by-finding closure

| ID | Severity | Finding | Closure |
|---|---|---|---|
| #N1 | NEW MAJOR | §3.5 "Tracing instrumentation policy" 가 actual code 와 정반대 (자동 derive 가정 부정확 — actual `lib.rs:109` 의 `target: "kebab-normalize"` literal hard-coded) | **CLOSED** — §3.5 wording 정정 (explicit literal 명시 + manual 갱신 필요 명시). §3.7 (g) 신설 — `tracing::debug!` target literal 의 보존 정책 (stage label 일관성, log scraper grep 호환). §6.8 R8 mitigation 의 1-line touch site 명시 (실제로 §3.5 + §3.7 (g) 가 mitigation 본체). |
| #N2 | NEW MAJOR | §3.7e + §1.9 의 warning_agent return string 정책 wording 부정확 (warning_agent 자체는 "kb-parse-md" 단일 return; 별도 hard-coded "kb-normalize" literal 2 곳) | **CLOSED** — §3.7e 의 comment block 갱신 (warning_agent body = "kb-parse-md" 단일 + lib.rs:122/128/134/153 의 4 hard-coded literal 위치별 보존 정책 inline 명시). §1.9 의 production flow trace 를 6-row table (line / string / emitter / persist) 로 분리. |
| #N3 | NEW MINOR | §3.3 + §6.10 R10 의 evidence test-mod imports → production 으로 misclassify | **CLOSED** — §3.3 row note 정정 (production caller wording 일반화). §6.10 R10 의 grep cmd refresh — test mod import 위치는 `#[cfg(test)] mod tests 안의 ...` 명시. 진짜 production caller (`kebab-normalize::lib.rs` 자기 자신, lift body 의 single direct call) 명시. R10 결론 (production caller 0) 무변. |
| #N4 | NEW MINOR | §10.1 line reference stale (round 2 revision 의 +136 line shift 미반영) | **CLOSED** — §10.1 의 row #3 (CRITICAL #3 closure) 의 line reference 갱신 (793→978 + post-revision 의 shift 명시). 추가로 "Line reference 정책" inline 명시 — 향후 round 의 모든 line number 는 grep cross-check 후 명시. |
| #N5 | NEW MINOR | §6 의 R10 / R9 ordering 비정합 (R10 line 803 < R9 line 829) | **CLOSED** — 두 section 의 physical position 을 swap. 결과: §6.9 R9 (dep 폭증) → §6.10 R10 (frozen p1-4 surface) 의 natural ordering. label number = position number = risk introduction order. |
| #N6 | NEW NIT | §3.8 의 diff hunk 두 block 한 hunk 표현 (line-context mismatch 위험) | **CLOSED** — §3.8 의 diff 를 Hunk (a) `[workspace] members` 의 2 entry 삭제 + Hunk (b) `[workspace.package] version` 1-line 변경 의 두 hunk 로 분리. plan/executor 가 둘을 sequential 또는 parallel 로 적용 가능. |
| (round 1 의 18 fully closed + already-complete + false-positive 재확정) | — | round 1 의 §10.1 closure 가 critic round 2 의 evidence 와 정합 — actual byte-identical verify 통과 | **NO-ADDITIONAL-ACTION**. |

### §10.3 round 3 metrics

- Spec line count: 863 (round 2 start) → 999 (round 2 end) → **1067** (round 3 end, post-N1~N6 + §10.1 line reference refresh + §10.3 placeholder fill).
- Section headers: 60 (round 2 start) → 65 (round 2 end) → **67** (round 3 end, §3.7 (g) tracing target + §10.2 round 2 closure table).
- §6 의 R9 ↔ R10 physical swap (label rename + content swap) — 신설 없음, ordering 정합 only.
- §3 Design 결정 무변 (Option A, dead struct 3 보존, §3.7b 4-단락 재작성, target_version 0.19.0, warning_agent + tracing target 보존 정책).

## §11 Future work — image/pdf normalize integration (design §3.7b intent 의 미구현)

본 PR 은 `kebab-parse-types` 와 `kebab-normalize` 를 `kebab-parse-md` 로 흡수하지만, design §3.7b 의 *원래* intent — "4 parser (md/pdf/image/audio) 가 각자 ParsedBlock 변종 emit → normalize 가 medium-agnostic 통합 lift" — 는 미구현 상태로 영구 보존된다. 본 섹션이 그 영구 보존 entry.

### §11.1 배경

design §3.7b 의 원래 의도:

- 4 parser (md/pdf/image/audio) 가 각자 ParsedBlock 변종 (`ParsedBlock` / `ParsedImageRegion` / `ParsedPdfPage` / `ParsedAudioSegment`) emit.
- `kebab-normalize` 가 medium-agnostic ID/Provenance lift 수행.
- 결과 = 모든 parser 가 동일 `CanonicalDocument` shape 으로 합류.
- 즉 normalize 는 *multi-parser 통합 layer*.

### §11.2 현재 (v0.18.x) 상태

- **markdown 만** 의도된 path 사용 — `ParsedBlock` → normalize → `CanonicalDocument`.
- image / pdf / code parser 는 normalize 우회, 직접 `Extractor::extract() → CanonicalDocument` emit.
- 3 forward-declared struct (`ParsedImageRegion` / `ParsedPdfPage` / `ParsedAudioSegment`) caller = 0.
- audio parser 자체가 미존재 (P8 deferred — `MEMORY.md` "Phase priorities — P8 deferred, P9 first").

### §11.3 본 PR (v0.19.0) 결정

- `kebab-normalize` + `kebab-parse-types` 흡수 → `kebab-parse-md`.
- dead struct 3 **보존** (design intent 자체는 유효 — future surface).
- design §3.7b strike → §3.5 의 4-단락 재작성 ("원래 intent + 현재 상태 + 보존된 surface + future re-extraction trigger" — raison d'être 약화이지 폐기 아님).

### §11.4 Future direction (v0.20+ 후보)

다음 세 갈래가 image/pdf normalize integration 의 구체 시나리오. 본 PR 머지 후 followup spec / PR 의 trigger 후보:

1. **image parser normalize integration** — `ImageExtractor::extract` 가 `Vec<ParsedImageRegion>` emit → `kebab-parse-md` (흡수된 destination) 의 `build_canonical_document_from_image_regions(...)` 형태 lift fn 추가. multi-region image 활용 — text region (OCR) + caption region (LLM-generated description) + image region (raw bytes pointer) 의 region-별 provenance + chunk granularity 향상.
2. **pdf parser normalize integration** — `PdfTextExtractor::extract` 가 `Vec<ParsedPdfPage>` emit → page-별 metadata + block 통합 lift. multi-block pdf 활용 — per-page provenance (page-N 단위 citation), cross-page reference (forward-ref / back-ref 감지), 또는 page-별 doc-summary.
3. **audio parser introduction (P8 재개 또는 P+ phase)** — `ParsedAudioSegment` 의 첫 production caller. segment-level timestamp + speaker provenance. 현재 P8 audio 사용자 결정 deferred (`MEMORY.md`), 재개 시 본 §11.4.3 가 entry point.

### §11.5 본 PR 의 future-proofing

- 3 dead struct 보존 → 미래 도입 시 type 재정의 cost 0. `pub` re-export 유지 (§3.3) 로 caller add 시점에 surface 변경 0.
- design §3.7b strike wording (§3.5) 이 "abstraction dead by P+ usage gap" 으로 한정 — raison d'être 자체는 보존, 4-단락 구조 (원래 intent 보존 + future trigger 명시).
- 본 PR 의 흡수로 `kebab-parse-md` 가 multi-parser lift 의 **단일 destination** 가 됨 — future direction 도입 시 추가 caller (image / pdf / audio) 가 `kebab-parse-md` 의 lift fn 을 호출하는 패턴으로 자연 합류 (§3.7b 의 fan-in 회복).
- `warning_agent` 의 stage label "kb-normalize" 보존 (§3.7e) 로 future caller 가 자기 stage label 추가 시 (`"kb-parse-image-normalize"` 등) 의미 충돌 없이 확장 가능.

### §11.6 Trigger 조건

다음 중 하나 발생 시 v0.20+ scope 진입 (별 spec + 별 PR):

1. **image parser 에서 multi-region 분리 요구** — search granularity 향상 (region-별 chunk) 또는 LLM caption-only vs caption+text 비교 시.
2. **pdf parser 에서 page-level metadata 통합 필요** — cross-page reference 감지, 또는 page-별 doc-summary surface.
3. **audio parser 도입** — whisper.cpp local transcription 활성화 (P8 재개 trigger).
4. **fan-in ≥ 2 회복** — 위 1~3 중 2개 이상 동시 도입 시 §3.7b 의 layer 가치 회복 → `kebab-parse-md` 에서 `kebab-normalize` re-extract 검토.

### §11.7 본 PR 의 deliverable (§11 관련)

- spec §11 자체 (본 섹션) — 영구 보존 entry.
- `tasks/INDEX.md` 의 "Future work / deferred" 섹션 (없으면 신설) 에 한 줄 entry:
  ```markdown
  ## Future work / deferred

  - v0.20+ image/pdf normalize integration — design §3.7b intent 미구현 (3 dead struct 보존). PR #186 (normalize-absorption) 의 spec §11 참조.
  ```
- `tasks/HOTFIXES.md` 의 2026-05-26 entry Action 라인에 §11 cross-link (§3.9 에 반영).

### §11.8 §11 이 critic 검토에서 손대지 말아야 할 결정

본 PR 의 §3 Design 결정 (Option A destination, dead struct 3 보존, §3.7b 4-단락 재작성, `warning_agent` 보존 정책) 는 §11 도입과 정합 — critic 검토에서 §11 추가가 §3 결정을 흔들면 안 됨. 만약 흔드는 결과 도출 시 §11 자체의 재배치 (별 spec 으로 split, 또는 §6 Risk 의 R4 로 흡수) 를 권장하고 §3 는 유지.

---

**Spec drafted by**: planner (team `normalize-absorption`, Phase A).
**Date**: 2026-05-26.
**Status**: `drafting` → critic round 대기.
**Revision**: 본 spec 의 §11 추가 (image/pdf normalize integration 의 future-work 영구 보존) — team-lead 추가 요청 반영 (2026-05-26).
