---
phase: P1
title: "Markdown ingestion 파이프라인"
status: completed
depends_on: [P0]
source: kebab_local_rust_report.md §8, §14, §17 Phase 1
---

# P1 — Markdown ingestion 파이프라인

## 목표

`Markdown 파일 -> RawAsset -> CanonicalDocument -> Chunk -> SQLite` 흐름 완성. LLM/embedding 없이도 `kebab ingest` / `kebab list docs` / `kebab inspect doc <id>` 동작.

## 산출 crate

| crate | 역할 |
|-------|------|
| `kebab-source-fs` | local folder scan, checksum, 변경 감지. `SourceConnector` 구현 |
| `kebab-parse-md` | Markdown bytes → structured document. `Extractor` 구현 |
| `kebab-normalize` | parser output → `CanonicalDocument` |
| `kebab-chunk` | block-aware chunking. `Chunker` 구현 (`md-heading-v1`) |
| `kebab-store-sqlite` | metadata, document, chunk, job table. FTS table 은 P2 에서 활성화 |

## kebab-source-fs

- 입력: `SourceScope { root: PathBuf, include: Vec<Glob>, exclude: Vec<Glob> }`
- 동작: 재귀 walk → 각 파일 `blake3` → `RawAsset` 목록.
- 변경 감지: `(source_uri, checksum)` 기준 신/구 비교. 동일 checksum 은 skip.
- watch 모드는 P1 범위 밖 (config 만 정의, 구현 후순위).

## kebab-parse-md

- parser 후보: `pulldown-cmark` 1차. GFM table/task list 필요해지면 `comrak` 검토 (§8).
- 보존 대상: YAML/TOML frontmatter, heading tree, paragraph, list, code block + lang tag, table, blockquote, link, image ref, **line range**.
- 출력: 중간 표현 (parser 고유). `kebab-normalize` 가 canonical 로 변환.
- malformed markdown: panic 금지. 가능한 부분만 보존하고 `Provenance` 에 warning 기록.

## kebab-normalize

- 책임: parser 중간 표현 → `CanonicalDocument`.
- frontmatter → `Metadata` (id, title, aliases, tags, created_at, updated_at, source_type, trust_level, lang).
- block 트리 평탄화 + `BlockId` 부여 (heading path + 순번 기반 deterministic).
- `SourceSpan` 은 `LineRange { start, end }` 또는 `ByteRange` 둘 다 허용. Markdown 은 line range 1차.

## kebab-chunk (`md-heading-v1`)

우선순위 (§14):
1. heading boundary 우선
2. code block 중간 분할 금지
3. table 가능한 한 단일 chunk
4. 긴 section 은 paragraph 단위
5. `heading_path` 보존
6. `source_spans` 보존
7. `chunker_version = "md-heading-v1"` 기록

policy 기본값: `target_tokens = 500`, `overlap_tokens = 80`, `respect_markdown_headings = true`.

token 추정: tokenizer 미도입 단계라 byte / 문자 기반 근사 OK. 실제 tokenizer 는 P3 embedding 도입 시 교체.

## kebab-store-sqlite

스키마 (1차):

```sql
CREATE TABLE assets (
  asset_id TEXT PRIMARY KEY,
  source_uri TEXT NOT NULL,
  media_type TEXT NOT NULL,
  byte_len INTEGER NOT NULL,
  checksum TEXT NOT NULL,
  discovered_at TEXT NOT NULL
);

CREATE TABLE documents (
  doc_id TEXT PRIMARY KEY,
  asset_id TEXT NOT NULL REFERENCES assets(asset_id),
  title TEXT,
  lang TEXT,
  parser_version TEXT NOT NULL,
  doc_version INTEGER NOT NULL,
  metadata_json TEXT NOT NULL,
  provenance_json TEXT NOT NULL
);

CREATE TABLE blocks (
  block_id TEXT PRIMARY KEY,
  doc_id TEXT NOT NULL REFERENCES documents(doc_id),
  kind TEXT NOT NULL,
  heading_path TEXT NOT NULL,
  source_span_json TEXT NOT NULL,
  payload_json TEXT NOT NULL
);

CREATE TABLE chunks (
  chunk_id TEXT PRIMARY KEY,
  doc_id TEXT NOT NULL REFERENCES documents(doc_id),
  text TEXT NOT NULL,
  heading_path TEXT NOT NULL,
  source_spans_json TEXT NOT NULL,
  token_estimate INTEGER NOT NULL,
  chunker_version TEXT NOT NULL,
  block_ids_json TEXT NOT NULL
);

CREATE TABLE jobs (
  job_id TEXT PRIMARY KEY,
  kind TEXT NOT NULL,
  status TEXT NOT NULL,
  payload_json TEXT NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
```

- migration: `refinery` 또는 수동 SQL. 단순함이 우선.
- transaction: ingest 1건 = 1 transaction. 부분 실패 시 rollback.
- idempotent: 동일 `doc_id` 재수집은 UPSERT, version bump.

## kebab-app facade 확장

```rust
pub fn ingest(scope: SourceScope) -> anyhow::Result<IngestReport>;
pub fn list_docs(filter: DocFilter) -> anyhow::Result<Vec<DocSummary>>;
pub fn inspect_doc(id: &DocumentId) -> anyhow::Result<CanonicalDocument>;
pub fn inspect_chunk(id: &ChunkId) -> anyhow::Result<Chunk>;
```

`IngestReport`: `{ scanned, new, updated, skipped, errors }`.

## CLI

```text
kebab ingest <path> [--include <glob>] [--exclude <glob>]
kebab list docs [--tag <t>]
kebab inspect doc <doc_id>
kebab inspect chunk <chunk_id>
```

## 테스트

- snapshot: `fixtures/markdown/*` → `CanonicalDocument` JSON 동결.
- snapshot: chunk 출력 (heading path / source span 포함) 동결.
- contract: 동일 입력 두 번 ingest → DB row 수 변화 없음 (idempotency).
- edge case: frontmatter only / nested headings / long paragraph / code block / table / image ref / relative link / malformed / 한영 혼합 (§18).

## 의존성 경계

`kebab-parse-md` 금지: `kebab-store-*`, `kebab-llm*`, `kebab-rag`, `kebab-tui`, `kebab-desktop`, embedding 호출. parser 는 순수 함수.

## 완료 조건

- [ ] `kebab ingest <path>` 실행 후 SQLite 에 documents/blocks/chunks 채워짐
- [ ] `kebab list docs` 정상 출력
- [ ] `kebab inspect doc <id>` JSON 출력
- [ ] `kebab inspect chunk <id>` JSON 출력 (heading path + source span 포함)
- [ ] 같은 폴더 재수집 시 중복 row 없음
- [ ] parser/chunker version 변경 시 재처리 대상 식별 가능
- [ ] fixture snapshot test 통과

## 리스크 / 주의

- chunker version 바꾸면 chunk_id 모두 변경. embedding 재생성 필요. version 막 올리지 말 것.
- frontmatter 파싱 실패 시 문서 전체 reject 금지. provenance 에 warning 만.
- line range 정확도가 P2 citation 품질을 좌우.
