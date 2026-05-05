---
phase: P9
component: kebab-app
task_id: p9-fb-23
title: "Incremental ingest — skip unchanged docs (post-merge dogfooding)"
status: completed
depends_on: [p9-fb-03, p9-fb-07]
unblocks: []
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§9 Versioning cascade, §2.4a IngestEvent, §3.x IngestReport]
source_feedback: 사용자 도그푸딩 2026-05-04 — 변하지 않은 문서 재처리 회피 요청.
---

# p9-fb-23 — Incremental ingest

상세 설계: `docs/superpowers/specs/2026-05-04-p9-fb-23-incremental-ingest-design.md`.
구현 계획: `docs/superpowers/plans/2026-05-04-p9-fb-23-incremental-ingest.md`.

## Goal

`kebab ingest` 가 변경/신규 doc 만 처리. 변하지 않은 doc 은 parse/chunk/embed/vector upsert 모두 회피.

## Behavior contract

Skip 조건 4 모두 만족:
1. 신규 blake3 == `assets.checksum`.
2. `documents.parser_version` == 현 active.
3. `documents.last_chunker_version` == 현 active.
4. `documents.last_embedding_version` == 현 active (None == None 도 match).

위 중 하나라도 mismatch → 정상 path. parse/chunk/embed/vector upsert 모두.

`IngestOpts.force_reingest=true` → skip 무시 강제 재처리.

## Tests

- 통합: 두 번째 ingest 가 unchanged 1 / new 0 / updated 0.
- 통합: `--force-reingest` 가 skip 우회.
- 단위: V006 migration, SQLite put/get_document roundtrip 신규 컬럼, get_asset_by_workspace_path roundtrip.
- 통합: 첫 ingest 가 chunker/embedding version stamp.

## Risks / notes

- mtime pre-hash skip 미구현 (YAGNI, 후속 가능).
- 외부 embedder model swap 후 config 갱신 안 하면 silently skip — doctor 명령이 mismatch 감지하는 후속 task 가능.

Live deviations 반영 위치: `tasks/HOTFIXES.md` `2026-05-04 — p9-fb-23` 항목.
