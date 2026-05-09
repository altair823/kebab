---
title: "p9-fb-35 — Verbatim fetch design"
phase: P9
component: kebab-core + kebab-app + kebab-cli + kebab-mcp + wire-schema
task_id: p9-fb-35
status: design
target_version: 0.5.0
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§4 search, §5 storage, §10 UX]
date: 2026-05-09
---

# p9-fb-35 — Verbatim fetch

## Goal

agent 가 search hit / RAG citation 의 `chunk_id` / `doc_id` 로 raw verbatim text 를 deep-link fetch 할 수 있는 surface. CLI 와 MCP 동시 노출. 3가지 mode (chunk / doc / span). PDF / audio 의 line-based span 은 명시적 거절 (`error.v1.code = span_not_supported`). image OCR text 는 line-addressable 이라 span 허용.

## Behavior contract

### Source of truth

모든 text 는 `CanonicalDocument` / `chunks.text` 에서 가져온 정규화된 markdown. 원본 raw bytes (`assets.storage_path` 의 파일) 는 노출 안 함 — 사용자가 필요하면 직접 read. PDF / audio / image 도 동일 surface (page-text / transcript / OCR text). 단 line-based span 은 PDF / audio 거절 — image OCR 은 line-addressable 이라 허용.

### CLI subcommand

`kebab fetch` 신규 subcommand, 3 mode:

| mode | flags |
|------|-------|
| `kebab fetch chunk <chunk_id> [--context N] [--json]` | `--context` 시 동일 doc 의 ordinal ±N 범위 chunks 도 포함 |
| `kebab fetch doc <doc_id> [--max-tokens N] [--json]` | doc 정규화된 markdown text. budget 트립 시 truncated. |
| `kebab fetch span <doc_id> <line_start> <line_end> [--max-tokens N] [--json]` | doc text 의 line range (1-based, inclusive). PDF/audio 면 거절. |

`--context` 는 chunk mode 에만. `--max-tokens` 는 doc/span 에만 (chunk 는 bounded size).

### Wire shape — `fetch_result.v1`

discriminated by `kind`:

```json
{
  "schema_version": "fetch_result.v1",
  "kind": "chunk" | "doc" | "span",
  "doc_id": "<id>",
  "doc_path": "<workspace_path>",
  "indexed_at": "<RFC3339>",
  "stale": <bool>,
  "chunk":          {/* chunk_inspection.v1, kind=chunk */},
  "context_before": [/* chunk_inspection.v1[], kind=chunk */],
  "context_after":  [/* chunk_inspection.v1[], kind=chunk */],
  "text":           "<markdown>",
  "line_start":     <int>,
  "line_end":       <int>,
  "effective_end":  <int>,
  "truncated":      <bool>
}
```

Per-kind 필수 필드 — schema description 으로 명시 (JSON Schema 의 conditional validation 은 v1 stub 단계에서 미구현, agent 책임).

`indexed_at` / `stale` — fb-32 와 동일 stamping. `documents.updated_at` 기준.

### Mode 동작

**chunk mode**:
1. `DocumentStore::get_chunk(chunk_id)` — 없으면 `error.v1.code = chunk_not_found`.
2. `--context N` 시 doc 안 chunks 의 ordinal 정렬 → target ordinal ±N 의 chunks 추출. doc 경계 넘기지 않음 (clamp).
3. wire: `kind: "chunk"`, `chunk: <target>`, `context_before: [...]`, `context_after: [...]`, `truncated: false`.

**doc mode**:
1. `DocumentStore::get_document(doc_id)` — 없으면 `error.v1.code = doc_not_found`.
2. `CanonicalDocument` 의 blocks → markdown 직렬화 (`fmt_canonical_to_markdown` 헬퍼 신규).
3. `--max-tokens N` 시 chars/4 추정 budget 적용 — 초과 시 끝에서 끊고 truncated=true. (line 단위 trim 은 별도 task — 단순 char-trim.)
4. wire: `kind: "doc"`, `text: <md>`, `truncated: <bool>`.

**span mode**:
1. doc lookup 동일.
2. media_type 검사 — PDF (`Page` citation) / audio (`Time` citation) 는 line-incompatible → `error.v1.code = span_not_supported`.
3. doc text → `text.lines()` slice `[line_start..=line_end]`. line_end 가 total 초과 시 clamp.
4. `--max-tokens` 적용 시 끝에서 추가 truncate, `effective_end` 갱신.
5. wire: `kind: "span"`, `text`, `line_start`, `line_end` (요청), `effective_end` (실제 emit), `truncated`.

### Budget integration

fb-34 의 chars/4 추정 + truncate 패턴 재사용. `FetchOpts.max_tokens` 가 `Some(N)` 일 때만 동작. chunk mode 는 무관 (chunk 는 chunker 단위 bounded).

### Error codes

`error.v1.code` enum 추가:
- `chunk_not_found` — chunk_id lookup miss.
- `doc_not_found` — doc_id lookup miss.
- `span_not_supported` — line-incompatible media (PDF / audio).
- `invalid_input` — MCP tool 의 mode 별 필수 필드 누락 (e.g. `kind: "chunk"` + `chunk_id: null`).

`StructuredError` wrapper (fb-34) 재사용 — `App::fetch` 의 typed `ErrorV1` 가 `classify` downcast 거쳐 wire 까지 보존.

### MCP tool

`mcp__kebab__fetch` 신규. Input:

```rust
pub struct FetchInput {
    /// "chunk" | "doc" | "span"
    pub kind: String,
    pub chunk_id: Option<String>,
    pub doc_id: Option<String>,
    pub line_start: Option<u32>,
    pub line_end: Option<u32>,
    pub context: Option<u32>,
    pub max_tokens: Option<usize>,
}
```

Validation: `kind` 별 필수 필드 검증 후 `App::fetch` 호출. 출력 = `fetch_result.v1`.

## Allowed / forbidden dependencies

- `kebab-core`: 신규 도메인 type. 신규 dep 없음.
- `kebab-app`: 기존 deps 충분. fb-32 staleness + fb-34 budget 헬퍼 재사용. markdown 직렬화는 단순 fmt 함수 (별도 dep 불필요).
- `kebab-cli`: clap subcommand 추가, wire helper.
- `kebab-mcp`: tool 추가.
- `kebab-tui`: 변경 없음.
- `kebab-search` / `kebab-rag`: 변경 없음.

## Public surface delta

### kebab-core

```rust
#[derive(Clone, Debug)]
pub enum FetchQuery {
    Chunk(ChunkId),
    Doc(DocumentId),
    Span {
        doc_id: DocumentId,
        line_start: u32,
        line_end: u32,
    },
}

#[derive(Clone, Debug, Default)]
pub struct FetchOpts {
    /// chunk mode 만: ±N chunks. None = no context.
    pub context: Option<u32>,
    /// doc/span 만: chars/4 budget. None = no cap.
    pub max_tokens: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct FetchResult {
    pub kind: FetchKind,
    pub doc_id: DocumentId,
    pub doc_path: WorkspacePath,
    pub indexed_at: OffsetDateTime,
    pub stale: bool,
    // chunk
    pub chunk: Option<Chunk>,
    pub context_before: Vec<Chunk>,
    pub context_after: Vec<Chunk>,
    // doc / span
    pub text: Option<String>,
    pub line_start: Option<u32>,
    pub line_end: Option<u32>,
    pub effective_end: Option<u32>,
    pub truncated: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FetchKind { Chunk, Doc, Span }
```

`Serialize` impl for `FetchResult` flattens to `fetch_result.v1` shape (or wire helper does the projection).

### kebab-app

```rust
impl App {
    pub fn fetch(&self, query: FetchQuery, opts: FetchOpts) -> Result<FetchResult>;
}

pub fn fetch_with_config(
    config: kebab_config::Config,
    query: FetchQuery,
    opts: FetchOpts,
) -> Result<FetchResult>;

// markdown 직렬화 헬퍼 (private)
fn fmt_canonical_to_markdown(doc: &CanonicalDocument) -> String;
```

### kebab-cli

```rust
// Cmd::Fetch 신규 enum variant
Fetch {
    #[command(subcommand)]
    what: FetchWhat,
}

#[derive(Subcommand)]
enum FetchWhat {
    Chunk { id: String, #[arg(long)] context: Option<u32> },
    Doc { id: String, #[arg(long)] max_tokens: Option<usize> },
    Span {
        doc_id: String,
        line_start: u32,
        line_end: u32,
        #[arg(long)] max_tokens: Option<usize>,
    },
}
```

```rust
// wire helper
pub fn wire_fetch_result(r: &FetchResult) -> Value;
```

### kebab-mcp

`FetchInput` + `mcp__kebab__fetch` tool registration.

## Test plan

| kind | description |
|------|-------------|
| unit (kebab-app) | chunk fetch (no context) — chunk + 빈 context |
| unit (kebab-app) | chunk fetch `--context 2` 다중 chunk doc 에서 ±2 ordinal 정확 |
| unit (kebab-app) | chunk fetch `--context 99` doc 경계 clamp |
| unit (kebab-app) | doc fetch — markdown 직렬화 결과 |
| unit (kebab-app) | doc fetch `--max-tokens N` budget 트립 → truncated=true + text 잘림 |
| unit (kebab-app) | span fetch line range slice 정확 |
| unit (kebab-app) | span line_end > total → effective_end clamped |
| unit (kebab-app) | span PDF doc → StructuredError(span_not_supported) |
| unit (kebab-app) | span audio doc → StructuredError(span_not_supported) |
| unit (kebab-app) | unknown chunk_id → StructuredError(chunk_not_found) |
| unit (kebab-app) | unknown doc_id → StructuredError(doc_not_found) |
| unit (kebab-app) | indexed_at + stale fb-32 stamping 정확 |
| 통합 (kebab-cli) | `kebab fetch chunk <id> --json --context 1` wire 검증 |
| 통합 (kebab-cli) | `kebab fetch doc <id> --max-tokens 100 --json` truncated=true |
| 통합 (kebab-cli) | `kebab fetch span <doc_id> 1 5 --json` line range |
| 통합 (kebab-cli) | `kebab fetch chunk <unknown>` → exit 2 + error.v1.code = chunk_not_found |
| 통합 (kebab-cli) | plain mode chunk — `[doc_path § heading]\n<text>` 형태 |
| 통합 (kebab-mcp) | `mcp__kebab__fetch` 3 mode 정상 응답 |
| 통합 (kebab-mcp) | `kind: "chunk"` + `chunk_id: null` → invalid_input |
| 통합 (wire-schema) | `fetch_result.schema.json` 3 mode 샘플 validate |

## Implementation steps (high-level)

1. wire schema 신규 `fetch_result.schema.json` + `error.v1` enum 4 codes 추가.
2. `kebab-core` 신규 types (`FetchQuery`, `FetchOpts`, `FetchResult`, `FetchKind`).
3. `kebab-app::fetch` impl + `fmt_canonical_to_markdown` 헬퍼.
4. `kebab-cli::Cmd::Fetch` clap subcommand + wire helper + plain renderer.
5. `kebab-mcp` `kebab__fetch` tool + input validation.
6. 단위 + 통합 테스트.
7. README + SMOKE — fetch 예시.
8. tasks/INDEX.md / spec status flip.
9. `tasks/HOTFIXES.md` — 신규 surface 라 deviation 없을 가능성 (skip).
10. `integrations/claude-code/kebab/SKILL.md` — Recipe 추가 ("agent fetched a chunk_id from search, wants surrounding context").

## Risks / notes

- **Markdown 직렬화 round-trip** — `CanonicalDocument.blocks` 가 round-trip 손실 적은지 확인. 손실 발견 시 ingest 시점에 raw markdown 도 store 에 보존하는 후속 task 가능 (fb-3X).
- **chunk_id stability** — chunker_version cascade 시 invalidate. spec 에 명시 + skill notes 의 retry pattern 안내.
- **`Chunk` 가 `chunk_inspection.v1` 와 동일** — `wire_chunk_inspection` 재사용 가능. 새 헬퍼 불필요.
- **doc/span budget — line trim 안 함** — char-level trim 만. agent 가 끊긴 line 받을 가능성 있음. 충분히 작은 한도 (e.g. 2000 chars) 면 큰 영향 없음. 후속에서 line-aware trim 가능.
- **media_type 판정** — `documents.source_type` 또는 첫 chunk 의 citation kind (Line/Page/Time) 로 분기. PDF/audio 는 Page/Time citation. line range 의미 없음.

## Documentation updates (implementation PR 동시)

- `README.md` — 명령 표에 `kebab fetch chunk|doc|span` row.
- `docs/SMOKE.md` — fetch walkthrough (search → fetch chunk --context flow).
- `tasks/p9/p9-fb-35-verbatim-fetch.md` — `status: open → completed`, design/plan 링크.
- `tasks/INDEX.md` — fb-35 행 ✅.
- `integrations/claude-code/kebab/SKILL.md` — 신규 `mcp__kebab__fetch` row + recipe.
