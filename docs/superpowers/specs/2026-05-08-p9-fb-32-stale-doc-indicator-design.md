---
title: "p9-fb-32 — Stale doc indicator design"
phase: P9
component: kebab-app + kebab-store-sqlite + kebab-search + kebab-cli + kebab-tui
task_id: p9-fb-32
status: design
target_version: 0.4.0
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§3 ingest, §10 UX]
date: 2026-05-08
---

# p9-fb-32 — Stale doc indicator

## Goal

검색 hit / RAG citation 에 "마지막으로 색인된 시점" 과 "임계 초과 stale 여부" 두 신호를 노출. 사용자 / agent 가 답변 근거의 최신성 약점을 즉시 인지할 수 있게 한다. 자동 재 ingest 는 하지 않는다 — 표시까지가 본 task 범위.

## Behavior contract

### Stale 정의

`stale = (now - documents.updated_at) > stale_threshold_days * 86400s`

- `documents.updated_at` 은 V001 부터 존재. 마지막 실제 re-process 시점 (RFC3339).
- fb-23 incremental ingest 의 skip path 는 `put_document` 를 호출하지 않으므로 `updated_at` 이 자연스럽게 stale 의 source-of-truth 역할.
- `stale_threshold_days = 0` → 모든 hit `stale = false` (기능 비활성).
- 음수 threshold → config load error (`error.v1`, exit code 2).

### Wire schema delta

**`search_hit.v1`** — 두 필드 additive (required):

| 필드 | 타입 | 의미 |
|------|------|------|
| `indexed_at` | `string` (RFC3339 date-time) | hit 의 source doc `documents.updated_at` |
| `stale` | `boolean` | server-computed `now - indexed_at > threshold` |

**`citation.v1`** — 동일 두 필드 additive (required). `answer.v1.citations[]` 가 본 schema 사용.

**`answer.v1`** — 변경 없음. citation 단위로 stale 정보 운반. aggregate flag (`any_citation_stale`) 는 out of scope — consumer 가 citations[] 순회로 충분.

`schema_version` 은 `search_hit.v1` / `citation.v1` 그대로. additive minor 라 breaking 아님 — 바이너리 version cascade 의 wire 트리거에 해당하지 않음. 단, frozen wire schema 정책상 필드 추가 자체는 spec 갱신 필요 → CLAUDE.md §wire-schema 의 v1 명세 반영.

### Config

`config.toml` `[search]` 섹션 — 신규 필드:

```toml
[search]
stale_threshold_days = 30  # 0 = 비활성. 양의 정수.
```

- env override: `KEBAB_SEARCH_STALE_THRESHOLD_DAYS`.
- default: 30 (코드 `Default` impl).
- validation: 음수 → `Config::load` error (`error.v1.code` = config_invalid).
- 변경 즉시 반영 (compute 가 query path 에 위치, ingest 불필요).

### CLI plain output

`--json` 미설정 시 (사람 읽기용 plain stderr/stdout) — 각 hit / citation 의 `doc_path` 옆에 `[stale]` tag 추가:

```
1. [stale] notes/dmq-quota.md § 운영 절차
   score=0.812  indexed_at=2025-12-03
```

색상은 terminal capability 있을 때만 노란색. 없으면 plain 텍스트. 비활성 (threshold=0) 또는 fresh hit 은 tag 미표시.

### TUI

- `Theme::Role::Warning` 재사용 (fb-14 에 정의됨).
- search pane / inspect pane / ask citation pane: stale doc 의 `doc_path` 우측에 `[STALE]` 배지.
- 색상 단독 의미 전달 금지 정책 (fb-14) 준수 — 텍스트 `[STALE]` + Warning 색.
- `T` 토글 (fb-14) 영향 없음 (Warning role 은 dark/light 양쪽 정의됨).

## Allowed dependencies

각 crate 기존 deps 유지. 신규 dep 없음. `time` 크레이트 (이미 `kebab-store-sqlite` / `kebab-app` 에서 사용).

## Forbidden dependencies

- `kebab-core` 는 wire 변환 / threshold 계산 안 함 (도메인 타입만).
- UI crate (`kebab-cli` / `kebab-tui` / `kebab-mcp`) 가 직접 SQL 호출 X — `kebab-app` facade 통해서만.

## Public surface delta

### kebab-core (도메인)

```rust
// SearchHit / Citation 도메인 struct 에 indexed_at: time::OffsetDateTime 필드 추가.
// stale 은 도메인이 아니라 wire 계산 결과 — facade 에서만 채움.
```

### kebab-store-sqlite

```rust
// 기존 search query JOIN documents 확장 — updated_at SELECT.
// retriever 가 chunk hit 과 함께 indexed_at 받음.
```

### kebab-search (lexical/vector/hybrid)

```rust
// 내부 Hit struct 에 indexed_at: OffsetDateTime 운반 필드 추가.
// fusion / scoring 로직 영향 없음.
```

### kebab-app (facade)

```rust
pub fn compute_stale(indexed_at: OffsetDateTime, now: OffsetDateTime, threshold_days: u32) -> bool {
    if threshold_days == 0 { return false; }
    let delta = now - indexed_at;
    delta.whole_seconds() as u64 > u64::from(threshold_days) * 86400
}

// search / ask wire DTO 변환 시 indexed_at + stale 두 필드 채움.
// now 는 Clock 추상화로 주입 (테스트 결정성 위함).
```

### kebab-config

```rust
#[derive(Deserialize, ...)]
pub struct SearchConfig {
    #[serde(default = "default_stale_threshold_days")]
    pub stale_threshold_days: u32,
    // ... 기존 필드
}
fn default_stale_threshold_days() -> u32 { 30 }
```

env: `KEBAB_SEARCH_STALE_THRESHOLD_DAYS`.

### kebab-cli

`render_hit_plain` / `render_citation_plain` 에 `[stale]` tag 추가. 색상 헬퍼는 기존 `is_tty` 검사 패턴 재사용.

### kebab-tui

search/inspect/ask pane 의 doc_path 라인 렌더에 `Theme::style(Role::Warning)` 으로 `[STALE]` Span 추가. snapshot 테스트 갱신.

## Test plan

| kind | description |
|------|-------------|
| unit (kebab-config) | `default().search.stale_threshold_days == 30` |
| unit (kebab-config) | 음수 threshold load → error.v1 (config_invalid) |
| unit (kebab-config) | `KEBAB_SEARCH_STALE_THRESHOLD_DAYS=7` env override |
| unit (kebab-app) | `compute_stale(now-31d, now, 30) == true` |
| unit (kebab-app) | `compute_stale(now-29d, now, 30) == false` |
| unit (kebab-app) | `compute_stale(_, _, 0) == false` (모든 입력) |
| unit (kebab-app) | `compute_stale(now-30d, now, 30) == false` (boundary, 정확히 동일 = false) |
| 통합 (kebab-app/cli) | search wire JSON 의 각 hit 에 `indexed_at` (RFC3339) + `stale` (bool) 존재 |
| 통합 (kebab-app/cli) | ask wire JSON `citations[]` 각 항목에 동일 두 필드 |
| 통합 (kebab-tui) | stale doc 포함 search pane snapshot 에 `[STALE]` 배지 (insta `[time]` redaction 적용) |
| 통합 (kebab-cli) | plain output 에 `[stale]` tag 정확한 위치 |
| 통합 (wire-schema) | `search_hit.schema.json` / `citation.schema.json` 갱신 + JSON Schema validation 통과 |
| 통합 (smoke) | `docs/SMOKE.md` 시나리오에 stale 시나리오 추가 — 30일 전 ingest 시뮬레이션 (Clock 주입) |

snapshot 갱신: 기존 search / ask 관련 fixture (`crates/kebab-cli/tests/fixtures/`, `crates/kebab-tui/tests/snapshots/` 등) — `indexed_at` 은 time-dependent 라 insta filter 로 `[indexed_at]` 마스킹.

## Implementation steps (high-level)

1. wire schema 파일 갱신 (`docs/wire-schema/v1/search_hit.schema.json`, `citation.schema.json`) — required 두 필드 추가.
2. `kebab-config` `SearchConfig.stale_threshold_days` 신설 + env + 검증.
3. `kebab-store-sqlite` retriever query 의 documents JOIN 확장 — `updated_at` SELECT.
4. `kebab-search` 내부 Hit struct 에 `indexed_at` 필드 운반.
5. `kebab-app` facade 의 wire DTO 변환에 `compute_stale` 호출. Clock 주입 인프라 신설 (없을 시).
6. `kebab-cli` plain renderer + `[stale]` tag.
7. `kebab-tui` Warning Span 추가 + snapshot 갱신.
8. 모든 단위/통합 테스트 추가 + 기존 snapshot redaction 갱신.
9. README 의 Configuration 섹션에 `stale_threshold_days` 명시. `docs/SMOKE.md` 시나리오 추가.
10. HOTFIXES.md 영향 없음 (frozen spec 변경 X).

## Risks / notes

- **Snapshot churn**: 기존 search / ask snapshot 약 ~5개 재 record. `indexed_at` 마스킹 필터 표준화 필요.
- **Clock 주입**: 코드베이스에 `Clock` trait 가 이미 있는지 확인 — 없으면 facade-local trait 신설 (테스트 결정성 위함). Production path 는 `OffsetDateTime::now_utc` 단순 wrapper.
- **Off-by-one**: boundary 가 `>` (초과) 인지 `>=` (이상) 인지 — `>` 채택 (정확히 30일째는 fresh).
- **citation.v1 stub**: 현재 schema 가 stub 상태 (variant-discriminated validation 미구현). 두 필드 추가는 stub 수준에서도 required 명시 가능.
- **Scope discipline**: 자동 재 ingest / TUI 'r' 키 / file mtime 기반 stale 모두 후속 task. 본 spec 은 표시까지.

## Documentation updates (implementation PR 동시)

- `README.md` — Configuration 섹션의 config 예시 + `stale_threshold_days` 한줄.
- `docs/SMOKE.md` — config 예시 갱신 + stale 시나리오 walkthrough 한 단락.
- `tasks/p9/p9-fb-32-stale-doc-indicator.md` — `status: open → completed`, design/plan 링크 추가.
- `tasks/INDEX.md` — fb-32 행 ✅ 표시 + 0.4.0 release 트리거 노트.
- `integrations/claude-code/kebab/SKILL.md` — wire 필드 추가 멘션 (parsing tip 한 줄).
