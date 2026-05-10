---
title: "p9-fb-37 — Trace + stats design"
phase: P9
component: kebab-core + kebab-search + kebab-store-sqlite + kebab-app + kebab-cli + kebab-mcp + kebab-tui
task_id: p9-fb-37
status: design
target_version: 0.5.0
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§4 search, §7 RAG, §10 UX]
date: 2026-05-10
---

# p9-fb-37 — Trace + stats

## Goal

retrieval pipeline 가시성 + KB 건강 surface. 두 axes:

- **Trace**: `kebab search Q --trace` — `search_response.v1` 에 optional `trace` 필드 (lexical/vector pre-fusion lists + RRF inputs + per-stage timing). agent / 사용자가 "왜 이 결과가 나왔는지" 진단.
- **Stats**: `kebab schema --json` 의 기존 `stats` 객체에 4 필드 추가 (media/lang breakdown + index disk bytes + stale doc count). KB 건강 한 눈에.

둘 다 wire schema additive minor — 기존 consumer 무영향. trace 는 opt-in (cost 0 when off), stats 는 항상 채움 (저렴한 GROUP BY).

## Behavior contract

### CLI flag

```
kebab search <query> [--trace] [--json] [기존 flags ...]
kebab schema [--json]
```

`--trace` boolean, default false. 활성 시:
- HybridRetriever 가 lexical / vector 각 단계 출력 + per-stage timing 캡처.
- search cache **bypass 강제** (debug intent — cache hit timing 무의미).
- `--json` 면 `search_response.v1.trace` 채움.
- non-`--json` 면 hits 출력 후 `Trace:` section pretty-print (lex/vec 카운트 + timing + top 3 hit per stage).

`kebab schema --json` 의 `stats` 4 필드 항상 출력 (no flag).

### Wire shape

**`search_response.v1`** (additive minor — schema bump 없음):

```jsonc
{
  "schema_version": "search_response.v1",
  "hits":           [/* search_hit.v1 */],
  "next_cursor":    null,
  "truncated":      false,
  "trace": {                                  // OPTIONAL — present iff --trace
    "lexical": [
      {"chunk_id":"c1","doc_id":"d1","doc_path":"a.md","rank":1,"score":0.42}, ...
    ],
    "vector": [
      {"chunk_id":"c2","doc_id":"d2","doc_path":"b.md","rank":1,"score":0.81}, ...
    ],
    "rrf_inputs": [
      {"chunk_id":"c1","lexical_rank":2,"vector_rank":3,"fusion_score":0.0234}, ...
    ],
    "timing": {"lexical_ms":12,"vector_ms":45,"fusion_ms":1,"total_ms":58}
  }
}
```

`#[serde(default, skip_serializing_if = "Option::is_none")]` — `--trace` 없으면 `trace` 키 자체 부재.

**`schema.v1.stats`** (additive minor — schema bump 없음):

```jsonc
"stats": {
  "doc_count": 50,
  "chunk_count": 200,
  "asset_count": 50,
  "last_ingest_at": "2026-05-10T12:34:56Z",
  // fb-37 신규
  "media_breakdown": {"markdown":12,"pdf":3,"image":5,"audio":0,"other":0},
  "lang_breakdown":  {"en":10,"ko":5,"null":5},
  "index_bytes":     {"sqlite":12345678,"lancedb":23456789},
  "stale_doc_count": 2
}
```

- `media_breakdown`: `MEDIA_KINDS` (markdown/pdf/image/audio/other) 5 키 항상 채움 (0 포함). `assets.media_type` JSON 의 dual shape (text vs object) 는 fb-36 과 동일한 CASE WHEN 패턴.
- `lang_breakdown`: 비어있을 수 있음 (corpus 비면 `{}`). NULL lang 은 `"null"` 문자열 키.
- `index_bytes.sqlite` = `*.sqlite` + `*.sqlite-wal` + `*.sqlite-shm` 합. `lancedb` = 디렉터리 recursive 합 (없으면 0).
- `stale_doc_count` = `documents.updated_at < (now - threshold_days)` count. `config.search.stale_threshold_days = 0` 이면 항상 0 (fb-32 의미).

### Edge cases

| 상황 | 동작 |
|------|------|
| `--trace --mode lexical` | `vector: []`, `vector_ms: 0`. rrf_inputs 모두 `vector_rank: null` |
| `--trace --mode vector` | 대칭 |
| `--trace` cache 가 hit 가능 query | cache bypass 강제, fresh run |
| 빈 corpus | hits=[], trace lex/vec=[], timing 정상 (모두 작은 값) |
| index_bytes lancedb 디렉터리 부재 | 0 |
| sqlite WAL/SHM aux 파일 부재 | 메인 `.sqlite` 만 합산 |
| stale_doc_count threshold=0 | 0 (fb-32) |
| cursor pagination + `--trace` | 첫 호출 trace, next_cursor 따라 재호출 trace 부재 (재요청 필요) |
| `--trace` non-`--json` mode | hits + trace 텍스트 출력 (lex/vec count, timing, top 3 per stage) |

### MCP `SearchInput` 확장

```rust
pub struct SearchInput {
    pub query: String,
    pub mode: Option<String>,
    pub k: Option<usize>,
    pub max_tokens: Option<usize>,    // fb-34
    pub snippet_chars: Option<usize>, // fb-34
    pub cursor: Option<String>,       // fb-34
    pub tags: Option<Vec<String>>,    // fb-36
    pub lang: Option<String>,         // fb-36
    pub path_glob: Option<String>,    // fb-36
    pub trust_min: Option<String>,    // fb-36
    pub media: Option<Vec<String>>,   // fb-36
    pub ingested_after: Option<String>, // fb-36
    pub doc_id: Option<String>,       // fb-36
    // fb-37
    pub trace: Option<bool>,
}
```

`Some(true)` = trace ON, `Some(false)` / `None` = OFF. 출력은 wire 와 동일 (trace 필드 mirror).

### TUI Search pane

- 결과 표시 중 (`SearchPane.results` 비어있지 않음) `t` keybind → `TracePopup` 모달.
- TUI 가 `kebab_app::search_with_trace_with_config` 재호출 (현재 query, k, mode, filters 전부).
- popup: 단일 scroll list (lex section / vec section / rrf section 헤더로 구분), `Esc` 닫기, `j/k` 또는 ↑↓ scroll.
- 기존 inspect pane 무수정.

## Allowed / forbidden dependencies

- `kebab-core`: 신규 dep 없음. domain types 추가만.
- `kebab-store-sqlite`: 신규 dep 없음. rusqlite + std::fs 만.
- `kebab-search`: 신규 dep 없음. std::time::Instant 사용.
- `kebab-app`: 신규 dep 없음. facade 확장.
- `kebab-cli`: 신규 dep 없음. clap flag 추가.
- `kebab-mcp`: 신규 dep 없음. SearchInput 확장.
- `kebab-tui`: 신규 dep 없음. ratatui popup widget.

`kebab-core` 의 다른 `kebab-*` 의존 금지 룰 그대로. UI 크레이트는 facade 만.

## Public surface delta

### kebab-core (`search.rs`)

```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SearchTrace {
    pub lexical:    Vec<TraceCandidate>,
    pub vector:     Vec<TraceCandidate>,
    pub rrf_inputs: Vec<TraceFusionInput>,
    pub timing:     TraceTiming,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TraceCandidate {
    pub chunk_id: ChunkId,
    pub doc_id:   DocumentId,
    pub doc_path: WorkspacePath,
    pub rank:     u32,
    pub score:    f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TraceFusionInput {
    pub chunk_id:     ChunkId,
    pub lexical_rank: Option<u32>,
    pub vector_rank:  Option<u32>,
    pub fusion_score: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceTiming {
    pub lexical_ms: u64,
    pub vector_ms:  u64,
    pub fusion_ms:  u64,
    pub total_ms:   u64,
}
```

`IndexStats` 확장 (`stats.rs` 또는 위치 동일):

```rust
pub struct IndexStats {
    // 기존
    pub doc_count:      u64,
    pub chunk_count:    u64,
    pub asset_count:    u64,
    pub last_ingest_at: Option<OffsetDateTime>,
    // fb-37
    #[serde(default)]
    pub media_breakdown: BTreeMap<String, u64>,
    #[serde(default)]
    pub lang_breakdown:  BTreeMap<String, u64>,
    #[serde(default)]
    pub index_bytes:     IndexBytes,
    #[serde(default)]
    pub stale_doc_count: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexBytes {
    pub sqlite:  u64,
    pub lancedb: u64,
}
```

`#[serde(default)]` — 옛 JSON 누락 시 zero-valued 으로 deserialize (backwards-compat).

### kebab-store-sqlite (`stats.rs`)

```rust
pub fn breakdowns(conn: &rusqlite::Connection, threshold_days: u64)
    -> rusqlite::Result<(BTreeMap<String,u64>, BTreeMap<String,u64>, u64)>;

pub fn index_bytes(data_dir: &Path) -> std::io::Result<IndexBytes>;
```

기존 stats helper 가 이 두 함수 호출해 `IndexStats` 채움. 신규 query:
- media: `SELECT CASE WHEN json_type(media_type)='text' THEN json_extract(media_type,'$') ELSE (SELECT key FROM json_each(media_type) LIMIT 1) END AS kind, COUNT(DISTINCT d.doc_id) FROM documents d JOIN assets a ON a.asset_id=d.asset_id GROUP BY kind`
- lang: `SELECT COALESCE(lang,'null') AS l, COUNT(*) FROM documents GROUP BY l`
- stale: `SELECT COUNT(*) FROM documents WHERE updated_at < ?` (threshold_days > 0 일 때만; 0 면 0 반환).

### kebab-search (`hybrid.rs`)

```rust
impl HybridRetriever {
    pub fn search_with_trace(&self, query: &SearchQuery)
        -> Result<(Vec<SearchHit>, SearchTrace)>;
}
```

기존 `Retriever::search` 무변경. `search_with_trace` 는 hybrid 전용 (lexical/vector mode 도 한 쪽만 채워 동일 type 반환). 내부:
1. `Instant::now()` 기록, lex retriever 호출, lex_ms 측정.
2. 같은 패턴 vec.
3. fuse — fusion_ms 측정.
4. trace 빌드: lex/vec 전체 list → TraceCandidate 매핑. rrf_inputs = lex ∪ vec union (chunk_id 기준), 각 entry 의 lexical_rank/vector_rank/fusion_score 캡처. fusion 결과 ranking 과 동일.
5. total_ms = 처음~끝.

### kebab-app (`app.rs`)

```rust
#[doc(hidden)]
pub fn search_with_trace_with_config(
    cfg: kebab_config::Config,
    query: &str,
    opts: SearchOpts,  // 기존 + trace: bool
) -> Result<(SearchResponse, Option<SearchTrace>)>;
```

`opts.trace = true` 시:
- cache bypass (`no_cache = true` 강제).
- `HybridRetriever::search_with_trace` 호출.
- `SearchResponse` 빌드 + trace 별도 반환 (caller 가 wire 합성).

기존 `search_with_config` 무변경 (zero-overhead path).

### kebab-cli (`Cmd::Search`)

```rust
Cmd::Search {
    // 기존 + fb-34 + fb-36
    query, k, mode, explain, no_cache,
    max_tokens, snippet_chars, cursor,
    tag, lang, path_glob, trust_min, media, ingested_after, doc_id,
    // fb-37
    #[arg(long)] trace: bool,
}
```

dispatch:
- `trace == false` → 기존 `search_with_config` 경로.
- `trace == true` → `search_with_trace_with_config` 호출, wire 합성 시 `search_response.v1` JSON 에 `trace` 필드 inject.

non-`--json` 출력:
- `--trace` 면 hits 후 `\nTrace:\n  lexical (N hits, Xms): top3...\n  vector (M hits, Yms): top3...\n  rrf (Zms): top3...\n  total: Wms`.

### kebab-mcp (`tools/search.rs`)

`SearchInput.trace: Option<bool>` 추가. dispatch 시 `Some(true)` 이면 위 `_with_trace` 호출. 출력 JSON 에 trace 합성 (wire 와 동일).

### kebab-tui (`search.rs` + `trace_popup.rs` 신규)

- `App` 에 `trace_popup: Option<TracePopupState>` 필드.
- search pane key handler `t` → `kebab_app::search_with_trace_with_config` (현재 query/opts) 호출 → popup state 채움.
- `trace_popup.rs`: ratatui Paragraph 또는 List 로 lex/vec/rrf 3 section, scroll, `Esc` 닫기.
- cheatsheet 에 `t = trace` 한 줄 추가.

## Test plan

| kind | description |
|------|-------------|
| unit (kebab-core) | `SearchTrace` serde roundtrip — 모든 필드 |
| unit (kebab-core) | `IndexStats` 신규 4 필드 default — 비어있는 map / 0 bytes / 0 stale |
| unit (kebab-store-sqlite) | `breakdowns`: 3 docs (md/md/pdf, en/en/null) → media `{markdown:2,pdf:1,image:0,audio:0,other:0}` (5키 패딩 적용), lang `{en:2,null:1}` |
| unit (kebab-store-sqlite) | `index_bytes`: temp dir 내 sqlite 파일 + 빈 lancedb dir → sqlite>0, lancedb=0 |
| unit (kebab-store-sqlite) | `breakdowns` stale_doc_count: threshold 7 day, 8일 전 doc 1 + 어제 doc 2 → 1 |
| unit (kebab-store-sqlite) | `breakdowns` threshold=0 → stale_doc_count=0 |
| unit (kebab-search/hybrid) | `search_with_trace`: lex/vec list 가 단일 retriever 호출 결과 == |
| unit (kebab-search/hybrid) | timing 모두 정의됨, total ≥ lex+vec+fusion 의 sum (sequential 가정) |
| unit (kebab-search/hybrid) | mode=lexical → vector=[], vector_ms=0, rrf_inputs.vector_rank 모두 None |
| 통합 (kebab-cli) | `kebab search Q --trace --json` → trace 키 존재, lexical/vector/rrf_inputs/timing 모두 valid shape |
| 통합 (kebab-cli) | `kebab search Q --json` (no --trace) → trace 키 부재 |
| 통합 (kebab-cli) | `kebab schema --json` → media_breakdown 5 키, lang_breakdown 가능 키, index_bytes 두 필드, stale_doc_count 모두 존재 |
| 통합 (kebab-cli) | 빈 corpus `kebab schema --json` → media_breakdown 5키 모두 0, lang_breakdown {} |
| 통합 (kebab-cli) | `kebab search Q --trace` (non-json) → stdout 에 `Trace:` section, lex/vec count + timing 표시 |
| 통합 (kebab-mcp) | search input `trace:true` → 응답 JSON 에 trace 필드 |
| 통합 (kebab-mcp) | search input `trace` 미지정 → 응답 trace 부재 |
| TUI (kebab-tui) | search pane 결과 있는 상태에서 `t` 키 → popup 열림 (state transitions) |
| TUI (kebab-tui) | popup 열린 상태 `Esc` → popup 닫힘 |

`media_breakdown` 5키 패딩 책임: `kebab-store-sqlite::breakdowns` 가 SQL GROUP BY 결과를 받아 `MEDIA_KINDS` 순회해 누락 키 0 으로 채움.

## Implementation steps (high-level)

1. `kebab-core`: SearchTrace + 3 sibling struct + IndexStats 4 필드 + 단위 테스트.
2. `kebab-store-sqlite::stats`: breakdowns + index_bytes 헬퍼 + 단위 테스트.
3. `kebab-store-sqlite::stats`: 기존 IndexStats 빌더가 신규 4 필드 채우도록.
4. `kebab-search::hybrid`: `search_with_trace` 구현 + 단위 테스트.
5. `kebab-app`: `search_with_trace_with_config` facade + cache bypass.
6. `kebab-cli::Cmd::Search`: `--trace` flag + dispatch + JSON wire 합성 + non-JSON pretty-print.
7. `kebab-cli` 통합 테스트.
8. `kebab-mcp::tools::search`: SearchInput.trace + dispatch + 통합 테스트.
9. `kebab-tui::search` + `trace_popup`: `t` keybind + popup widget + cheatsheet.
10. README + SMOKE + INDEX/spec status flip + SKILL.

## Risks / notes

- **timing 정확도**: 현재 hybrid sequential. 추후 병렬화 시 `total_ms = max(lex,vec) + fusion` 으로 재정의 — 그 시점 schema doc note 갱신.
- **lancedb dir walk cost**: 큰 corpus 에서 O(file count) IO. 도그푸딩 corpus 작아 무시. 큰 corpus 만나면 cache 또는 lazy 도입 검토.
- **`media_breakdown` JSON shape**: fb-36 과 동일한 CASE WHEN 패턴 재사용 — `MediaType` serde 의 dual shape (text variant vs tuple variant) 처리.
- **lang null 키**: ASCII string `"null"` 사용. ISO 639 어떤 코드와도 충돌 X (3자 미만).
- **cache bypass when --trace**: agent 가 인지해야 (SKILL/README 명시). 안 그러면 trace timing 이 cache hit 의 sub-ms 보고할 위험.
- **wire backwards-compat**: `trace` 필드 optional + skip_serializing_if. `IndexStats` 신규 필드 #[serde(default)] 로 옛 reader 가 새 응답 deserialize 가능.
- **TUI popup**: 별도 `t` 키. 충돌 검사 — 현재 search pane keybinds 확인 (i=inspect, /=focus, j/k=move, n=next, p=prev). `t` 미사용.

## Out of scope

- per-stage filter 적용 전/후 카운트 (filter-debug 별도 작업).
- search 단계 병렬화 (sequential 유지).
- lance 테이블 별 / column 별 index_bytes (단일 sum).
- stats 시계열 (corpus_revision history).
- `--trace-level` verbosity (single boolean).
- TUI inspect pane 안 trace 통합 (search popup 으로 격리).
- `kebab stats` 별도 명령 (schema 통합 결정).
- `--explain` flag deprecation 알림 (현재 search dead, 무영향 — 별도 cleanup task).

## Documentation updates (implementation PR 동시)

- `README.md`: `kebab search` row 의 flag 표기에 `--trace` 추가, `kebab schema` row 에 신규 stats 한 줄 언급.
- `docs/SMOKE.md`: `--trace` walkthrough + `kebab schema --json` 출력 sample.
- `tasks/p9/p9-fb-37-trace-and-stats.md`: `status: open → completed`, design/plan 링크 추가.
- `tasks/INDEX.md`: fb-37 행 ✅.
- `integrations/claude-code/kebab/SKILL.md`: `mcp__kebab__search` `trace` 입력 + 출력 trace shape 명시. `kebab schema` 신규 stats 필드 mention.
- `docs/wire-schema/v1/search_response.schema.json`: `trace` optional 필드 추가.
- `docs/wire-schema/v1/schema.schema.json`: `stats` 4 신규 필드 추가.
