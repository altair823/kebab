---
title: "p9-fb-36 — Search filter args design"
phase: P9
component: kebab-core + kebab-search + kebab-cli + kebab-mcp
task_id: p9-fb-36
status: design
target_version: 0.5.0
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§4 search]
date: 2026-05-10
---

# p9-fb-36 — Search filter args

## Goal

agent / 사용자가 검색 범위를 좁힐 수 있도록 CLI / MCP 에 filter flag 추가. 기존 `SearchFilters` 도메인 type 의 4 필드 (tags_any / lang / path_glob / trust_min) 를 CLI 표면에 노출하고, 신규 3 필드 (media / ingested_after / doc_id) 추가. wire schema 변경 없음 (input-only). filter 적용 layer = SQLite WHERE (lexical) + over-fetch + post-filter (vector). AND 조합 의미 고정.

## Behavior contract

### CLI flags on `kebab search`

7 flags 추가, 모두 optional. 비어있으면 미적용 (기존 동작 보존):

| flag | 의미 | repeat? |
|------|------|---------|
| `--tag <name>` | doc 의 `metadata.tags` 안에 매칭 (OR-within) | yes (`--tag rust --tag async` = `tag IN (rust,async)`) |
| `--lang <iso>` | `documents.lang` 정확 매칭 | no |
| `--path-glob <pattern>` | `documents.workspace_path` glob 매칭 | no |
| `--trust-min <level>` | `documents.trust_level >= level` (enum 순서) | no |
| `--media <csv>` | `assets.media_type.kind` IN 리스트 (예: `--media md,pdf`) | csv |
| `--ingested-after <RFC3339>` | `documents.updated_at >= timestamp` | no |
| `--doc-id <id>` | `documents.doc_id = id` | no |

다중 flag 조합 = AND 결합. 각 flag 안 다중 값 (--tag, --media) = OR.

### Filter validation

- `--ingested-after` RFC3339 파싱 실패 → CLI 진입 시 `error.v1.code = config_invalid`, exit 2.
- `--media` 의 unknown value (예: `--media foo`) → 매칭 0건 (filter unmatch). 명시적 거절 안 함 (lenient).
- `--trust-min` clap value_enum 검증 (enum 외 거절).
- `--doc-id` 형식 검증 안 함 (DocumentId 는 단순 string wrapper). 존재하지 않으면 매칭 0건.

### Filter layer

**Lexical (lexical.rs)**:
- 기존 SQL builder 의 WHERE 절 확장. `media` / `ingested_after` / `doc_id` 모두 SQL 구문 가능.
- `media`: `JOIN assets a ON a.asset_id = d.asset_id` + `json_extract(a.media_type, '$.kind') IN (?, ?)` (다중 값).
- `ingested_after`: `d.updated_at >= ?` (RFC3339 lexicographic compare; UTC `Z` 가정).
- `doc_id`: `d.doc_id = ?`.
- path_glob 은 기존 post-filter 그대로.

**Vector (vector.rs)**:
- 기존 over-fetch (k * 2) + `filter_chunks` 헬퍼에서 SQLite chunks JOIN documents JOIN assets.
- 같은 WHERE 조건 적용. k 부족 시 truncated.

### Wire shape

기존 wire schema 변경 없음.

- `search_response.v1` (output) — 그대로.
- `search_hit.v1` (개별 hit) — 그대로.
- 입력 측 (CLI args / MCP `SearchInput`) 만 확장.

MCP `SearchInput` schema 는 `schemars` derive 로 자동 갱신. 수동 schema 파일 X.

### MCP `SearchInput` 확장

```rust
pub struct SearchInput {
    pub query: String,
    pub mode: Option<String>,
    pub k: Option<usize>,
    pub max_tokens: Option<usize>,    // fb-34
    pub snippet_chars: Option<usize>, // fb-34
    pub cursor: Option<String>,       // fb-34
    // p9-fb-36 신규 (모두 optional)
    pub tags: Option<Vec<String>>,
    pub lang: Option<String>,
    pub path_glob: Option<String>,
    pub trust_min: Option<String>,    // "low" | "medium" | "high"
    pub media: Option<Vec<String>>,
    pub ingested_after: Option<String>,  // RFC3339
    pub doc_id: Option<String>,
}
```

input → `SearchFilters` 변환 시 위와 동일 검증 (RFC3339 파싱, trust_level enum). 실패 시 `invalid_input` ErrorV1.

## Allowed / forbidden dependencies

- `kebab-core`: 신규 dep 없음. 기존 type 확장만.
- `kebab-search`: 변경 없음 (SQL builder 안 WHERE 추가만).
- `kebab-cli`: clap flag 추가, dispatch 변환.
- `kebab-mcp`: SearchInput 확장.
- `kebab-tui`: 변경 없음.

`kebab-core` 의 다른 `kebab-*` crate 의존 금지 룰 그대로.

## Public surface delta

### kebab-core

```rust
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SearchFilters {
    pub tags_any: Vec<String>,
    pub lang: Option<Lang>,
    pub path_glob: Option<String>,
    pub trust_min: Option<TrustLevel>,
    /// p9-fb-36: media_type filter — IN-list of `MediaType.kind` strings
    /// (e.g. `["markdown", "pdf"]`). Empty Vec = no filter.
    #[serde(default)]
    pub media: Vec<String>,
    /// p9-fb-36: hits whose source doc's `documents.updated_at` is at
    /// or after this timestamp. None = no filter. RFC3339 / UTC.
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub ingested_after: Option<OffsetDateTime>,
    /// p9-fb-36: restrict hits to a single document. None = no filter.
    #[serde(default)]
    pub doc_id: Option<DocumentId>,
}
```

`#[serde(default)]` on each new field = backwards-compat (older JSON without these keys deserializes as defaults).

### kebab-search (lexical + vector)

내부 SQL builder 확장만. public API 변경 없음.

### kebab-cli (`Cmd::Search`)

```rust
Cmd::Search {
    // 기존
    query, k, mode, explain, no_cache,
    max_tokens, snippet_chars, cursor,   // fb-34
    // p9-fb-36 신규
    #[arg(long)] tag: Vec<String>,
    #[arg(long)] lang: Option<String>,
    #[arg(long)] path_glob: Option<String>,
    #[arg(long, value_enum)] trust_min: Option<TrustLevelFlag>,
    #[arg(long, value_delimiter = ',')] media: Vec<String>,
    #[arg(long)] ingested_after: Option<String>,
    #[arg(long)] doc_id: Option<String>,
}
```

`TrustLevelFlag` 신규 clap value_enum (CLI-internal, kebab-core 의 `TrustLevel` 로 변환).

### kebab-mcp::tools::search

`SearchInput` 7 optional 필드 추가 (위 §MCP `SearchInput` 확장). dispatch 에서 `SearchFilters` 빌드 + 검증.

## Test plan

| kind | description |
|------|-------------|
| unit (kebab-core) | `SearchFilters::default()` — 7 필드 모두 비어있음 |
| unit (kebab-search/lexical) | `media: ["pdf"]` — markdown doc 안 잡힘 |
| unit (kebab-search/lexical) | `media: ["markdown", "pdf"]` — IN-list 동작 |
| unit (kebab-search/lexical) | `ingested_after: <어제>` — 어제 이전 doc 안 잡힘 |
| unit (kebab-search/lexical) | `doc_id: <X>` — 다른 doc 의 chunk 안 잡힘 |
| unit (kebab-search/lexical) | 다중 filter AND — 모두 만족하는 hit 만 |
| unit (kebab-search/lexical) | 빈 filter (default) — 기존 동작과 동일 |
| unit (kebab-search/vector) | 동일 패턴 — `filter_chunks` post-filter |
| unit (kebab-search) | 알 수 없는 media 값 (`["foo"]`) — empty result, no error |
| 통합 (kebab-cli) | `kebab search Q --media md --json` wire shape (search_response.v1 그대로) |
| 통합 (kebab-cli) | `kebab search Q --ingested-after 2020-01-01 --json` 모든 hit 통과 |
| 통합 (kebab-cli) | `kebab search Q --ingested-after garbage --json` → `error.v1.code = config_invalid` exit 2 |
| 통합 (kebab-cli) | `kebab search Q --doc-id <id> --json` 단일 doc 만 |
| 통합 (kebab-cli) | `kebab search Q --tag rust --tag async --json` IN-list 동작 |
| 통합 (kebab-mcp) | `mcp__kebab__search` 7 optional 필드 모두 정상 응답 |
| 통합 (kebab-mcp) | `mcp__kebab__search` invalid `ingested_after` → invalid_input |

## Implementation steps (high-level)

1. `kebab-core::SearchFilters` 3 필드 추가 + 단위 테스트.
2. `kebab-search/lexical.rs` SQL builder 확장 + 단위 테스트.
3. `kebab-search/vector.rs` `filter_chunks` 헬퍼 동일 확장 + 단위 테스트.
4. `kebab-cli::Cmd::Search` 7 flag 추가 + dispatch + RFC3339 파싱.
5. `kebab-cli` 통합 테스트 (lexical-only, no Ollama).
6. `kebab-mcp::tools::search::SearchInput` 7 필드 + dispatch + invalid_input 검증.
7. `kebab-mcp` 통합 테스트.
8. README + SMOKE — filter 예시.
9. tasks/INDEX.md / spec status flip.
10. SKILL.md — `mcp__kebab__search` input shape 갱신.

## Risks / notes

- **`assets.media_type` JSON shape**: `MediaType` enum 의 serde 직렬화 형태가 `{"kind": "markdown"}` 인지, 다른 형태인지 SQLite 저장 형식 확인 필요. `Markdown` 같은 unit variant 는 `"markdown"` 문자열, `Image(...)` / `Audio(...)` 같은 tuple variant 는 `{"image": {...}}` 형태일 가능성. `json_extract` 경로를 그에 맞춰 조정 (e.g. `case when typeof(...) = 'text' then ... else json_extract($.kind) end`).
- **RFC3339 lexicographic compare**: ingest 시 항상 UTC `Z` 로 저장 (fb-32 ingest path 확인됨). 외부 도구가 다른 offset 으로 강제 update 시 비교 부정확. spec 에 "UTC `Z` 가정" 명시.
- **path_glob 과 다른 filter 의 ordering**: path_glob 은 post-filter (lexical), 신규 3 개는 SQL — fetch_limit 도달 후 path_glob 으로 추가 cut → final hit 수가 줄 수 있음. 기존 동작과 동일 (path_glob 패턴 유지).
- **clap `Vec<String>` 의 default**: clap 0.4 에서 미지정 = `Vec::new()`. 자동.
- **trust_min enum 매핑**: clap value_enum 으로 안전. `TrustLevelFlag` → `TrustLevel` 변환 헬퍼.
- **SearchFilters serde backwards-compat**: `#[serde(default)]` 로 옛 JSON 무영향. SQLite 안 SearchFilters 직렬 저장 안 함 (request-time only).

## Out of scope

- `--exclude-doc-id` / `--exclude-tag` (exclusion filter).
- 다중 doc_id (`--doc-id a --doc-id b`) — 단일만.
- TUI Search 패널 filter UI.
- Lance metadata pre-filter.
- tag 시스템 신규 도입 (이미 존재).
- `--search.default-filter` config (default 값 지정) — agent 가 매번 명시.

## Documentation updates (implementation PR 동시)

- `README.md` — `kebab search` row 의 flag 표기에 7 flag 추가.
- `docs/SMOKE.md` — filter walkthrough (`--media md --ingested-after 2026-04-01` 예시).
- `tasks/p9/p9-fb-36-search-filters.md` — `status: open → completed`, design/plan 링크.
- `tasks/INDEX.md` — fb-36 행 ✅.
- `integrations/claude-code/kebab/SKILL.md` — `mcp__kebab__search` input shape 갱신 (7 필드 명시 + AND 의미 + lenient unknown media).
