---
title: "p9-fb-34 — Output budget controls design"
phase: P9
component: kebab-core + kebab-app + kebab-cli + kebab-mcp + wire-schema
task_id: p9-fb-34
status: design
target_version: 0.5.0
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§4 search, §10 UX, wire-schema search_hit.v1]
date: 2026-05-09
---

# p9-fb-34 — Output budget controls

## Goal

`kebab search` agent UX 개선. context window 제약 있는 agent 가 검색 결과 size 와 페이지네이션을 명시적으로 제어할 수 있게 한다. CLI surface 우선, MCP tool 도 동일 인자로 동시 노출. ask path 는 scope out (별도 `rag.max_context_tokens` 가 이미 budget 담당).

## Behavior contract

### CLI flags

`kebab search "<query>"` 에 세 가지 flag 신규:

| flag | 의미 | default |
|------|------|---------|
| `--max-tokens N` | 결과 wire JSON 의 추정 token 수 cap (`chars/4` 근사). 초과 시 truncate priority 적용. | 미설정 = 비활성 (기존 동작) |
| `--snippet-chars N` | 각 hit snippet 최대 chars. config 의 `search.snippet_chars` 보다 우선. | 미설정 = config 값 |
| `--cursor <opaque>` | 이전 호출의 `next_cursor` 값. 다음 페이지 hits 만 반환. | 미설정 = 첫 페이지 |

### Wire shape

`kebab search --json` stdout 이 기존 `search_hit.v1[]` 배열에서 신규 `search_response.v1` wrapper object 로 교체:

```json
{
  "schema_version": "search_response.v1",
  "hits": [/* search_hit.v1[] */],
  "next_cursor": "<base64>" | null,
  "truncated": true | false
}
```

**Backwards-compat broken** — agent 가 `[0]` 직접 인덱싱하면 깨짐. CLI plain (`--json` 없이) 출력 무영향. HOTFIXES 에 결정 로그.

### Token estimation

`chars/4` 근사 (RAG `pack_context` 와 일관). tiktoken-rs 등 신규 dep 없음. 정확도 ±15% 수준 — agent budget 제어 목적상 충분. wire schema description 에 "approximation" 명시.

### Truncate priority

`opts.max_tokens` 가 Some 일 때만 동작. 단계별:

1. **Snippet 단축** — 각 hit snippet 을 `opts.snippet_chars.unwrap_or(config.search.snippet_chars)` 로 자른 뒤, 여전히 budget 초과면 60-char floor 까지 점진 단축.
2. **k 축소** — snippet 60 char 까지 줄여도 초과면 마지막 hit 부터 pop. 최소 1 hit 보장.
3. **truncated flag** — 위 어느 단계라도 동작 시 `truncated: true`. agent 는 `next_cursor` 로 다음 페이지 요청 가능.

metadata (rank/score/doc_path/citation) 는 끝까지 유지 — agent 가 hit 자체를 못 찾으면 무의미.

### Pagination cursor

cursor 는 opaque base64 — 내부적으로 `{offset: usize, corpus_revision: string}` JSON 의 base64 encode.

- 첫 호출: cursor 미설정 → offset 0.
- 응답: 남은 hit 있으면 `next_cursor = encode(offset + returned, current_revision)`. 없으면 `null`.
- 다음 호출: `--cursor <prev>` → decode → offset 만큼 skip.
- corpus_revision mismatch (이후 ingest 등으로 corpus 가 변경됨) → `error.v1.code = "stale_cursor"`, exit 2. agent 책임으로 재호출.

retriever 호출 시 k = `effective_k + offset` 만큼 fetch 후 offset 만큼 skip 해 응답.

### Stale cursor error

`error.v1.code` enum 에 `"stale_cursor"` 추가. message 예시: `"cursor was issued against corpus_revision 'abc'; current revision is 'xyz'. Re-issue search to obtain a fresh cursor."`

## Allowed / forbidden dependencies

- `kebab-core`: `SearchOpts` 신규 도메인 type 정의. 신규 dep 없음 (option / String 만).
- `kebab-app`: cursor encode/decode 헬퍼 (base64 + serde_json). `base64` workspace dep 가 이미 있을 가능성 높음 — 확인 후 필요 시 추가.
- `kebab-cli`: clap 인자 추가, wire wrapper 헬퍼.
- `kebab-mcp`: tool input schema 확장.
- `kebab-tui`: 변경 없음 (Search 패널 budget 미사용. fb-3X 후속).
- `kebab-search`: 변경 없음 — retriever signature 보존.

`kebab-core` 가 다른 `kebab-*` crate 의존 금지 룰 준수.

## Public surface delta

### kebab-core

```rust
#[derive(Clone, Debug, Default)]
pub struct SearchOpts {
    /// p9-fb-34: chars/4 approximation. None = no budget enforcement.
    pub max_tokens: Option<usize>,
    /// p9-fb-34: per-hit snippet character cap. None = use config default.
    pub snippet_chars: Option<usize>,
    /// p9-fb-34: opaque base64 cursor from a previous response.
    pub cursor: Option<String>,
}
```

### kebab-app

```rust
#[derive(Clone, Debug)]
pub struct SearchResponse {
    pub hits: Vec<SearchHit>,
    pub next_cursor: Option<String>,
    pub truncated: bool,
}

impl App {
    /// p9-fb-34: budget-aware search.
    pub fn search_with_opts(
        &self,
        query: SearchQuery,
        opts: SearchOpts,
    ) -> Result<SearchResponse>;

    // Existing — thin wrapper for backwards-compat.
    pub fn search(&self, query: SearchQuery) -> Result<Vec<SearchHit>> {
        let resp = self.search_with_opts(query, SearchOpts::default())?;
        Ok(resp.hits)
    }
}

// cursor helpers (private to app crate)
pub(crate) fn encode_cursor(offset: usize, corpus_revision: &str) -> String;
pub(crate) fn decode_cursor(
    s: &str,
    expected_revision: &str,
) -> Result<usize /* offset */, ErrorV1 /* stale_cursor */>;
```

### kebab-cli

```rust
// Cmd::Search 새 인자
#[arg(long)] max_tokens: Option<usize>,
#[arg(long)] snippet_chars: Option<usize>,
#[arg(long)] cursor: Option<String>,
```

```rust
// wire helper
pub fn wire_search_response(r: &SearchResponse) -> Value {
    let v = serde_json::json!({
        "hits": r.hits.iter().map(wire_search_hit).collect::<Vec<_>>(),
        "next_cursor": r.next_cursor,
        "truncated": r.truncated,
    });
    tag_object(v, "search_response.v1")
}
```

plain output: 기존 hit 줄들 + truncated 시 stderr 한 줄:

```
[truncated; use --cursor <next_cursor> for the next page]
```

### kebab-mcp

`SearchInput` 에 optional 필드 추가:

```rust
pub struct SearchInput {
    pub query: String,
    pub mode: Option<String>,
    pub k: Option<usize>,
    /// p9-fb-34
    pub max_tokens: Option<usize>,
    pub snippet_chars: Option<usize>,
    pub cursor: Option<String>,
}
```

출력: `search_response.v1` JSON tag 적용 (CLI 와 동일 wrapper).

## Test plan

| kind | description |
|------|-------------|
| unit (kebab-app) | `cursor::encode/decode` round-trip + corpus_revision mismatch → `StaleCursor` |
| unit (kebab-app) | `App::search_with_opts` budget=None → 기존 `App::search` 동일 (truncated=false, next_cursor 채움) |
| unit (kebab-app) | budget=200 tokens → snippet 60-char floor 까지 단축, truncated=true |
| unit (kebab-app) | budget < single-hit 최소 → k=1 + truncated=true (1 hit 보장) |
| unit (kebab-app) | snippet_chars override → 해당 길이로 truncate |
| 통합 (kebab-app) | cursor offset 5 호출 → 6번째 hit 부터 반환 |
| 통합 (kebab-app) | corpus_revision bump 후 cursor 재호출 → `StaleCursor` error.v1 |
| 통합 (kebab-cli) | `kebab search "x" --json` → `search_response.v1` shape |
| 통합 (kebab-cli) | `--max-tokens 200 --json` → truncated=true, hits 짧음 |
| 통합 (kebab-cli) | `--cursor <encoded>` → 다음 페이지 |
| 통합 (kebab-cli) | plain output: `[truncated; ...]` stderr 한 줄 |
| 통합 (kebab-mcp) | `mcp__kebab__search` tool 이 `search_response.v1` 반환 |
| 통합 (wire-schema) | `search_response.schema.json` validate 샘플 (with/without next_cursor) |
| 통합 (kebab-app) | 기존 `App::search` 호출자 (TUI 등) 무영향 — return type 동일 |

## Implementation steps (high-level)

1. wire schema 신규 `search_response.schema.json` + `error.v1` enum 에 `stale_cursor` 추가.
2. `kebab-core::SearchOpts` 도메인 type.
3. `kebab-app::SearchResponse` + `cursor` 모듈 (encode/decode).
4. `App::search_with_opts` impl (budget loop, cursor handling).
5. `App::search` thin wrapper 보존.
6. `kebab-cli::Cmd::Search` 새 flag + wire wrapper helper + plain truncated hint.
7. `kebab-mcp::SearchInput` 확장 + 출력 wrapper.
8. 단위 + 통합 테스트.
9. README + SMOKE — `--max-tokens` / `--cursor` 예시.
10. tasks/INDEX.md / spec status flip.
11. `tasks/HOTFIXES.md` — wire breaking 결정 로그.
12. `integrations/claude-code/kebab/SKILL.md` — search 결과 shape 변경 명시.

## Risks / notes

- **Wire breaking**: agent 가 기존 `search_hit.v1[]` 배열 직접 파싱 시 깨짐. HOTFIXES 결정 로그 + skill notes 반영 필수. 내부 single-user 환경이라 실용적 영향 적음.
- **`App::search` 시그니처 보존** 으로 TUI / 기존 caller 무영향.
- **chars/4 추정 정확도** ±15% — agent budget 보호 목적상 충분. tiktoken 도입은 별도 task.
- **cursor opaque** — agent 가 base64 decode 시도 막을 방법 없음. spec 에 "구조 변경 가능, 직접 파싱 금지" 명시.
- **corpus_revision 이 fb-19 LRU cache invalidation key 와 동일 source** — 별도 source-of-truth 추가 불필요.
- **TUI Search 패널 budget UI** — out of scope. 사용자가 원하면 fb-3X 후속.

## Documentation updates (implementation PR 동시)

- `README.md` — `kebab search` 명령 표 row 업데이트, `--max-tokens` / `--cursor` 한 줄.
- `docs/SMOKE.md` — pagination walkthrough 한 단락 (cursor 흐름 예시).
- `tasks/p9/p9-fb-34-output-budget-controls.md` — `status: open → completed`, design/plan 링크 추가.
- `tasks/INDEX.md` — fb-34 행 ✅.
- `tasks/HOTFIXES.md` — `2026-05-09 — p9-fb-34: search wire wrapped in search_response.v1` 결정 로그.
- `integrations/claude-code/kebab/SKILL.md` — Recipe 의 search 결과 파싱 패턴 (`response.hits[]`) + cursor 예시.
