---
title: "p9-fb-42 — Bulk multi-query design"
phase: P9
component: kebab-core + kebab-app + kebab-cli + kebab-mcp + wire-schema
task_id: p9-fb-42
status: design
target_version: 0.7.0
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§4 search]
date: 2026-05-10
---

# p9-fb-42 — Bulk multi-query

## Goal

agent 가 N 개 sub-query 를 단일 호출로 검색 — fb-41 multi-hop 또는 일반 query decomposition 의 surface efficiency 개선. fb-29 daemon 거부 후 stdio MCP (fb-30) 가 session-warm cache 제공해 subprocess overhead 일부 해소했지만, agent 가 한 turn 안에서 여러 query 를 병렬적으로 검색하려면 N 회 round-trip 필요. fb-42 는 단일 round-trip / 단일 process 안에서 N query 처리.

**Scope**: bulk multi-query 만 — rerank hint 는 별도 task (fb-39 cross-encoder 와 통합).

## Behavior contract

### CLI surface

```
kebab search --bulk [--json]
```

stdin 에서 ndjson 읽음. 한 줄 = 한 query input JSON. exit:
- 0: 모든 query 처리 완료 (개별 실패 포함).
- 2: stdin parse 실패 또는 N > 100 또는 기타 input validation 실패.

각 input item shape (single search SearchOpts/SearchFilters 와 동일 surface):

```jsonc
{
  "query": "rust async",            // 필수
  "mode": "lexical",                // optional, default hybrid
  "k": 5,                           // optional
  "max_tokens": 1000,               // optional (fb-34)
  "snippet_chars": 200,             // optional (fb-34)
  "cursor": "...",                  // optional (fb-34)
  "trace": false,                   // optional (fb-37)
  "tag": ["rust"],                  // optional (fb-36) — repeated -> Vec
  "lang": "en",                     // optional (fb-36)
  "path_glob": "src/**",            // optional (fb-36)
  "trust_min": "primary",           // optional (fb-36)
  "media": ["markdown"],            // optional (fb-36)
  "ingested_after": "2026-01-01T00:00:00Z",  // optional (fb-36)
  "doc_id": "..."                   // optional (fb-36)
}
```

`--json` 모드:
- stdout: per-query result ndjson — 한 줄 = `bulk_search_item.v1`.
- stderr: 마지막에 summary 한 줄 ndjson (`bulk_search_summary.v1` 또는 plain text — 구현 시 결정, 본 spec 은 stderr 로 분리하기로 명시).

non-`--json` 모드:
- stdout: 각 query 의 hits 가 human-readable block (single search plain renderer 재사용) + 빈 줄로 구분.
- stderr: query header (`# Query 1: <query text>`) + summary.

### MCP surface

신규 tool `kebab__bulk_search`. tools/list count 7 → 8.

input:
```jsonc
{
  "queries": [
    {"query": "...", "mode": "lexical", "k": 5, ...},
    {"query": "...", ...}
  ]
}
```

output (`bulk_search_response.v1` envelope):
```jsonc
{
  "schema_version": "bulk_search_response.v1",
  "results": [/* bulk_search_item.v1 */],
  "summary": {"total": N, "succeeded": M, "failed": K}
}
```

### Per-query result shape

`bulk_search_item.v1`:

```jsonc
{
  "schema_version": "bulk_search_item.v1",
  "query": {                                  // input echo (전체 fields)
    "query": "rust async",
    "mode": "lexical",
    "k": 5
    // ... 기타 input 필드 (None 이면 omit)
  },
  "response": {                               // success path
    "schema_version": "search_response.v1",
    "hits": [...],
    "next_cursor": null,
    "truncated": false,
    "trace": null
  },
  "error": null                               // error path 시 response: null + error: error.v1
}
```

`response` XOR `error`. 둘 중 하나 항상 non-null, 다른 하나 null.

### Limits

- `queries.len() > 100`:
  - CLI: exit 2 + error.v1 stderr (`code = config_invalid`, message: "queries: max 100 items").
  - MCP: tool error.v1 (`code = invalid_input`).
- `queries.len() == 0`:
  - CLI: exit 0, summary `0/0/0`, results: empty stream.
  - MCP: response envelope with `results: []`, summary `0/0/0`.

### Per-query error policy

- 한 query 의 처리 실패 (invalid filter, retrieval error, embedding 실패 등) → 해당 item 의 `error: error.v1` 채움 + 나머지 query 계속 진행.
- summary `failed` 카운트 증가.
- exit code 0 유지 (전체 처리 완료).
- bulk-level abort 트리거 없음 (개별 query 실패 격리).

### Execution

- Sequential for-loop. App instance 재사용 — embedder cold-start / cache 비용 한 번만.
- 같은 process / 같은 session — fb-30 MCP 의 hot cache 효과 N query 동안 누적.
- Parallel execution 보류 (out of scope — SQLite read pool 경쟁 + fastembed CPU thread 경쟁 부담).

## Allowed / forbidden dependencies

- `kebab-core`: 신규 dep 없음. 도메인 type 추가만.
- `kebab-app`: 신규 dep 없음. 기존 `App::search_with_opts` 재사용.
- `kebab-cli`: 신규 dep 없음. clap flag + stdin ndjson parse.
- `kebab-mcp`: 신규 dep 없음. 신규 tool module.

`kebab-core` 다른 `kebab-*` 의존 금지 + UI → facade only 룰 그대로.

## Public surface delta

### kebab-core (`search.rs`)

```rust
/// p9-fb-42: per-query result in bulk search.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BulkSearchItem {
    pub query: BulkQueryInput,            // input echo
    pub response: Option<SearchResponseMirror>,  // 또는 직접 wire shape
    pub error: Option<ErrorV1>,
}

/// p9-fb-42: bulk summary counts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BulkSearchSummary {
    pub total: u32,
    pub succeeded: u32,
    pub failed: u32,
}

/// p9-fb-42: bulk envelope (MCP only — CLI emits ndjson without envelope).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BulkSearchResponse {
    pub schema_version: String,           // "bulk_search_response.v1"
    pub results: Vec<BulkSearchItem>,
    pub summary: BulkSearchSummary,
}

/// p9-fb-42: per-query input echo (subset of full SearchInput, omits null).
pub type BulkQueryInput = serde_json::Value;  // 단순화 — 그대로 echo
```

`BulkQueryInput` 는 `serde_json::Value` 로 단순화 — 입력 그대로 echo. 도메인 type 으로 strict 하면 maintenance 부담만 늘고 backwards-compat 깨짐.

`SearchResponseMirror` 는 wire의 search_response.v1 shape — 기존 `kebab_app::SearchResponse` 직접 재사용 또는 별도 mirror struct. 구현 시 결정.

### kebab-app (`bulk.rs` 신규 또는 `app.rs` 확장)

```rust
#[doc(hidden)]
pub fn bulk_search_with_config(
    config: kebab_config::Config,
    items: Vec<serde_json::Value>,        // raw input items, validated inside
) -> anyhow::Result<(Vec<BulkSearchItem>, BulkSearchSummary)>;
```

내부:
1. `items.len() > 100` → early Err (config_invalid).
2. App instance 한 번 open.
3. for-loop: 각 item parse → SearchQuery + SearchOpts → app.search_with_opts → 성공/실패 분기.
4. summary 누적.

### kebab-cli (`Cmd::Search`)

```rust
Cmd::Search {
    // ... existing fields ...
    /// p9-fb-42: bulk multi-query mode. stdin 에서 ndjson 읽음 (한 줄 = 한 query JSON).
    /// `--json` 면 stdout per-query ndjson + stderr summary.
    /// non-`--json` 면 stdout human-readable per-query block + stderr summary.
    /// 기존 single-query flag (`query`, `--mode`, `--k`, etc) 와 mutual-exclusive — `--bulk` 일 때 single-query flag 무시.
    #[arg(long)]
    bulk: bool,
}
```

dispatch 분기:
- `bulk == true` → stdin read ndjson → bulk_search → output stream.
- `bulk == false` → 기존 single-query 경로 (변경 없음).

stdin ndjson parse 실패 (한 줄이라도) → exit 2 + error.v1 stderr.

### kebab-mcp (`tools/bulk_search.rs` 신규)

```rust
#[derive(Debug, Deserialize, JsonSchema)]
pub struct BulkSearchInput {
    pub queries: Vec<serde_json::Value>,  // 각 item = SearchInput shape
}

pub fn handle(state: &KebabAppState, input: BulkSearchInput) -> CallToolResult {
    // 1. queries.len() > 100 → invalid_input error
    // 2. for each query: parse → search → bulk_item
    // 3. envelope 빌드 + tool_success
}
```

`tools/mod.rs` 의 tool list 에 `bulk_search` 추가. capability `kebab schema --json` `capabilities.bulk_search: true` 신규.

## Test plan

| kind | description |
|------|-------------|
| unit (kebab-core) | `BulkSearchItem` serde — response variant + error variant |
| unit (kebab-core) | `BulkSearchSummary` total = succeeded + failed invariant |
| unit (kebab-app) | `bulk_search_with_config` empty input → empty result + 0/0/0 summary |
| unit (kebab-app) | `bulk_search_with_config` 3 query (lexical, 1건 invalid filter) → 2 success + 1 error |
| unit (kebab-app) | `bulk_search_with_config` 101 items → early Err (config_invalid) |
| 통합 (kebab-cli) | `echo '{"query":"a"}\n{"query":"b"}' \| kebab search --bulk --json` → 2 ndjson 줄 (response 채움) |
| 통합 (kebab-cli) | empty stdin → exit 0 + empty ndjson + summary 0/0/0 |
| 통합 (kebab-cli) | `echo 'not json' \| kebab search --bulk --json` → exit 2 + error.v1 stderr (config_invalid) |
| 통합 (kebab-cli) | 101 줄 ndjson → exit 2 + error.v1 |
| 통합 (kebab-cli) | non-`--json` mode bulk → human-readable per-query block, summary stderr |
| 통합 (kebab-cli) | 1건 invalid filter (`media: ["foo"]` 와 같은 unknown — fb-36 lenient 라 hits=0 success, 또는 다른 invalid case) → success 또는 error item 명확 |
| 통합 (kebab-mcp) | `kebab__bulk_search` queries=[2건] → response envelope, results 2 items, summary `2/2/0` |
| 통합 (kebab-mcp) | `kebab__bulk_search` queries=[] → envelope, results: [], summary `0/0/0` |
| 통합 (kebab-mcp) | `kebab__bulk_search` queries=[101건] → tool error invalid_input |
| 통합 (kebab-mcp) | tools/list count 7 → 8, `bulk_search` 등록 |
| 통합 (kebab-cli) | `kebab schema --json` capabilities.bulk_search == true |

invalid filter test 의 구체 case 는 구현 시 결정 — fb-36 의 invalid filter 가 명확한 error 를 emit 하는 path 를 택한다 (예: invalid trust_min value).

## Implementation steps (high-level)

1. `kebab-core`: BulkSearchItem / BulkSearchSummary / BulkSearchResponse types + 단위 테스트.
2. `kebab-app::bulk` (또는 app.rs): `bulk_search_with_config` 구현 + 단위 테스트.
3. `kebab-cli::Cmd::Search`: `--bulk` flag + dispatch + stdin ndjson parse + output stream + 통합 테스트.
4. `kebab-mcp::tools::bulk_search`: 신규 tool module + tools/list 등록 + 통합 테스트.
5. `kebab-app::schema`: capabilities.bulk_search = true + 단위 테스트.
6. wire schema docs (bulk_search_item / bulk_search_response).
7. README + SMOKE walkthrough.
8. design §4 search — bulk subsection.
9. SKILL.md `mcp__kebab__bulk_search` 안내.
10. tasks/INDEX.md / spec status flip.

## Risks / notes

- **JSON-RPC payload size**: MCP 가 N=100 + per-query trace 활성 시 payload 폭증. agent 가 cap 받으면 batch 분할 — agent 측 책임.
- **stdin 한 줄 parse 실패**: 한 줄 lexer error 면 전체 abort (atomic 입력 단위로 봄). 부분 입력 / 부분 처리 의미 모호.
- **summary stderr 위치**: `--json` 모드에서 stdout 은 순수 result stream — agent 가 line count 로 total 계산 가능. summary 는 stderr 인 게 stream 무결.
- **App instance 재사용**: kebab-app 의 cache (search LRU, embedder OnceLock) 가 N query 동안 hot. 첫 query 가 cold-start 비용, 나머지 amortize.
- **non-`--json` mode 가독성**: query 가 많으면 human reading 어려움. agent 는 항상 `--json` 사용 가정. non-JSON 은 사용자 디버그용 best-effort.
- **fb-30 MCP 와의 관계**: MCP session 이 이미 long-lived → bulk 가 줄여주는 비용은 N round-trip → 1 round-trip. 큰 N 에서 의미 있음. 작은 N (2-3) 은 MCP 호출 N 회와 큰 차이 없음 — agent decision.
- **rerank hint deferral**: stub 의 두 번째 lever (`--rerank-hint`) 는 본 PR scope 외. fb-39 (cross-encoder) 설계 후 별도 task 로 분리. tasks/p9/p9-fb-42 spec 의 status flip 시 "rerank hint deferred to fb-42b" note 추가.

## Out of scope

- LLM rerank hint (`--rerank-hint`).
- Cross-encoder reranker.
- Parallel execution (sequential for-loop 만).
- Inter-query result fusion / dedup.
- Bulk progress events (stream output 자체가 progress 역할).
- Per-query timeout (single search 도 timeout 없음 — 동일 정책).
- bulk session caching (App instance 재사용은 within-call 만).
- bulk cursor (전체 bulk 의 next-page) — 각 query 가 자체 cursor 가짐.

## Documentation updates (implementation PR 동시)

- `README.md`: `kebab search --bulk` row + 사용 예 한 줄.
- `docs/SMOKE.md`: bulk walkthrough — `echo '{"query":"a"}\n{"query":"b"}' | kebab search --bulk --json | jq`.
- `docs/wire-schema/v1/bulk_search_item.schema.json` 신규.
- `docs/wire-schema/v1/bulk_search_response.schema.json` 신규.
- `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md` §4 — bulk subsection.
- `integrations/claude-code/kebab/SKILL.md`: `mcp__kebab__bulk_search` tool 설명 + input/output shape.
- `tasks/p9/p9-fb-42-bulk-multi-query-rerank.md`: status flip + design/plan 링크 + "rerank hint deferred" note.
- `tasks/INDEX.md`: fb-42 ✅ (rerank hint 분리 명시).
