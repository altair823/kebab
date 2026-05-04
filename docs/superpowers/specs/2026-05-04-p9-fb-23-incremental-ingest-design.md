# p9-fb-23 — Incremental ingest (skip unchanged docs)

**Date**: 2026-05-04
**Status**: planned
**Audience**: kebab-app / kebab-store-sqlite implementer / reviewer.
**Source feedback**: 사용자 도그푸딩 2026-05-04 — "새 문서들이 폴더에 추가되면 ingest 시 변하지 않은 문서는 다시 ingest 하지 않고 변하거나 새로 추가된 문서만 처리하고 싶어."

## Goal

`kebab ingest` 가 변경되지 않은 (그리고 모든 version cascade input 도 동일한) document 의 parse / chunk / embed / vector upsert 를 스킵. 비용 dominator (fastembed embedding 호출) 가 변경된 / 새 file 에만 발생.

## Non-goals

- Mtime 기반 pre-hash skip (파일 읽기 자체를 회피). YAGNI — blake3 streaming 은 이미 scan 에서 무조건 발생, 본 spec 은 parse/chunk/embed 만 회피해도 90%+ 비용 절감.
- Watch-mode (실시간 file change detection). 후속 task.
- 부분 변경 (single chunk re-embedding). 항상 doc 단위 all-or-nothing.

## Allowed dependencies

- 기존 crate 만. 신규 crate 없음.
- SQLite migration 추가 (V006).

## Scope

본 spec 은 *file-system 소스* (`kebab-source-fs`) + 메인 ingest 파이프라인 (`kebab-app::ingest_with_config*`) 에만 적용. 다른 source connector (현재 없음, 후속 phase) 도 같은 skip 계약을 따름 — `IngestReport.unchanged` 카운트는 connector 무관.

## Skip 조건

문서가 다음 4개 모두 만족할 때 `Unchanged` 로 분류:

1. `assets.checksum` (저장된 blake3) == 신규 blake3 (스캔 중 재계산).
2. `documents.parser_version` == 현재 active parser_version.
3. `documents.last_chunker_version` == 현재 active chunker_version.
4. `documents.last_embedding_version` == 현재 active embedding_version (또는 양쪽 모두 NULL — embedder 미설정).

위 4개 중 하나라도 다르면 정상 ingest path. parse / chunk / embed / vector upsert 모두 발생.

## Storage 변경

**Migration V006** (`crates/kebab-store-sqlite/migrations/V006__incremental_ingest.sql`):

`documents` 테이블에 두 column 추가:

```sql
ALTER TABLE documents ADD COLUMN last_chunker_version TEXT;
ALTER TABLE documents ADD COLUMN last_embedding_version TEXT;
```

기존 row 는 NULL — 첫 ingest 시 항상 mismatch → 강제 재처리 (안전 default). 이후 매 ingest 가 row 의 두 column 을 현 active version 으로 stamp.

`parser_version` 은 이미 `documents` 테이블에 존재 (v005 이전). 활용.

V006 migration 은 idempotent (`ALTER TABLE` + `ADD COLUMN` 이 두 번 실행돼도 sqlite 가 column-exists 체크). Refinery framework 가 single-shot 보장.

## Pipeline 흐름

`kebab-app::ingest_with_config_progress_cancellable` (현 메인 ingest fn) 의 asset 루프 안에서:

1. Source connector 가 file scan + blake3 streaming → `asset_blake3` 생성 (현재와 동일).
2. **신규 early-skip 체크**:
   - `store.get_asset_by_workspace_path(path)` 로 기존 asset row 조회.
   - 존재 + `existing.checksum == new asset_blake3` → asset 동일.
     - `store.get_document_by_doc_id(id_for_doc(path, asset_id, current_parser_version))` 로 기존 doc 조회.
     - 존재 + `existing.last_chunker_version == current_chunker_version` + `existing.last_embedding_version == current_embedding_version` → **skip**.
       - `IngestReport.unchanged += 1`.
       - `IngestEvent::Item { kind: Unchanged, .. }` emit (progress consumer 가 표시).
       - 다음 asset 로 continue.
3. Skip 미충족 → 정상 path: `put_asset_with_bytes` → parse → `put_document` → chunk → `put_chunks` → embed → `vec_store.upsert`.
4. 정상 path 끝에서 `documents.last_chunker_version` + `documents.last_embedding_version` 을 현 active version 으로 stamp (`put_document` 가 받는 `Document` struct 에 두 field 추가, refinery 마이그레이션 자동 column 채움).

## API 변경

### `kebab-core::Document` struct

필드 두 개 추가:

```rust
pub struct Document {
    // ... existing ...
    pub last_chunker_version: Option<ChunkerVersion>,
    pub last_embedding_version: Option<EmbeddingVersion>,
}
```

`Option` — embedder 미설정 (config.models.embedding.enabled = false) 시 `last_embedding_version = None`.

### `kebab-core::IngestReport` + `kebab-app::AggregateCounts`

`unchanged: u32` 필드 추가. wire schema 변경:

`docs/wire-schema/v1/ingest_report.schema.json` 에 `unchanged` (integer, minimum 0) 필드 추가. **additive — v1 호환 유지** (기존 client 가 모르는 필드 무시). v2 bump 불필요.

`AggregateCounts::default()` 가 `unchanged: 0` 자동 처리.

### `kebab-core::IngestItemKind`

```rust
pub enum IngestItemKind {
    New,
    Updated,
    Skipped,    // 기존: media-type 필터 / kb:// URI
    Unchanged,  // 신규: skip 조건 4개 모두 만족
    Error,
}
```

`Skipped` (media-type 필터) 와 `Unchanged` (모든 versions match) 의미적 분리. `IngestEvent::Item.kind` 도 같이 확장.

### `kebab-store-sqlite` 신규 메서드

```rust
fn get_asset_by_workspace_path(&self, path: &WorkspacePath) -> Result<Option<Asset>>;
fn get_document_by_doc_id(&self, doc_id: &DocumentId) -> Result<Option<Document>>;
```

기존 `put_*` / `purge_*` 메서드는 변경 없음. 새 read 경로만 추가.

## TUI 노출

`kebab-tui::ingest_progress::status_line` 의 final line 포맷에 `unchanged` 추가:

```
✓ ingest: 100 docs (5 new, 3 updated, 92 unchanged, 0 skipped), 142 chunks indexed in 12s
```

진행 중 (in-flight) status 는 그대로 (per-asset granularity 이므로 unchanged 별 카운트 불필요).

p9-fb-24 의 status bar dynamic slot 도 같은 텍스트 표시 (cascade 의 `indexing N/M` final line).

## CLI 노출

`kebab ingest` 의 `--json` 모드는 wire schema 의 `unchanged` 필드 자동 출력. human 모드 final line 은 위 status_line 과 동일 포맷.

`--force-reingest` flag 신규 추가 — skip 조건 무시하고 모든 doc 강제 재처리. 사용자가 "이상한 결과 → 일단 모두 재처리" 케이스 대응. CLI 의 `kebab_app::AskOpts` 같은 패턴으로 `IngestOpts.force_reingest: bool` 추가, 기본 false.

## Tests

### 신규 단위

- V006 migration smoke (sqlite store): apply → `documents` 에 두 컬럼 존재 + NULL default.
- `get_asset_by_workspace_path` / `get_document_by_doc_id` 단위 (kebab-store-sqlite).
- `id_for_doc` 변경 없음 (parser_version 만 input — 그대로).

### 신규 통합 (kebab-app)

- **Unchanged path**: 한 번 ingest → 두 번째 ingest 시 `IngestReport.unchanged == 1`, embed 호출 0회.
- **Checksum mismatch**: 첫 ingest 후 파일 수정 → 두 번째 ingest 가 `updated == 1`.
- **Parser version bump**: 첫 ingest 후 `KEBAB_PARSE_MD_VERSION` 상수 변경 simulate → 두 번째 ingest 가 `updated == 1` (doc_id 변경됨).
- **Chunker version bump**: 첫 ingest 후 chunker_version 변경 simulate → `updated == 1`.
- **Embedder version bump**: 첫 ingest 후 embedder_version 변경 simulate → `updated == 1`.
- **`--force-reingest`**: 두 번째 ingest 가 skip 조건 만족하지만 강제로 `updated == 1` (또는 별도 카테고리?).

### 기존 영향

- 기존 ingest 통합 테스트 (kebab-app/tests/) 는 빈 KB 에서 시작하므로 모두 첫 번째 ingest path → `unchanged` 가 0 인 채로 그대로 통과.
- `IngestReport` JSON 출력 테스트가 `unchanged` 필드 추가됐을 때 호환되는지 검증. additive 라 통과해야 함.

## Spec contract impact

- **Design §9 versioning cascade**: 명시적 동작 추가. parser/chunker/embedder version bump 시 다음 ingest 가 자동으로 모든 doc 을 `updated` 로 처리. 기존엔 silently 새 version 으로 overwrite (idempotent UPSERT) 였으나 본 spec 으로 explicit refresh 보장.
- **Design §3.x IngestReport**: `unchanged` 필드 추가 (additive). v1 wire schema bump 없음.
- **Design §2.4a IngestEvent**: `IngestItemKind::Unchanged` variant 추가. line-delimited JSON consumer 는 unknown variant 무시 (현 default behavior).

## Risks / notes

- **Stale skip risk**: 사용자가 외부 도구 (Ollama 모델 swap 등) 로 embedder 바꾸고도 config 의 `models.embedding.id` 갱신 안 하면 `last_embedding_version` 매치 → silently skip. 완화: model_id 도 stamp 에 포함? 또는 doctor 명령이 mismatch 감지 → 권고. 본 spec 은 `embedding_version` (model 명+버전 fingerprint) 만 신뢰 — model 자체 무결성은 별 영역.
- **Force-reingest UX**: `--force-reingest` 는 모든 doc 재처리. 큰 corpus 에서 비싸므로 confirm prompt? 일단 flag 만 — 사용자가 명시적으로 입력하니 confirmation 불필요.
- **V006 migration 호환**: refinery 가 down-migration 미지원 (one-way). 이전 commit 으로 rollback 시 column 그대로 남음 (sqlite ALTER 의 한계). 무해 — 미사용 column.
- **doc_version 와의 관계**: 기존 `doc_version` (ingest 마다 +1) 는 그대로. Unchanged path 에서는 `doc_version` bump 안 함 — "이번 ingest 에서 처리 안 됨" 의미 보존.

## Live deviations

추후 발견되는 deviation 은 `tasks/HOTFIXES.md` `2026-05-04 — p9-fb-23` 항목에 dated 로그로 추가. spec 자체는 frozen.
