---
phase: P9
component: kebab-cli + kebab-app + wire-schema
task_id: p9-fb-35
title: "Verbatim fetch (`kebab fetch <chunk_id|doc_id>`) — citation deep-link"
status: completed
target_version: 0.5.0
depends_on: []
unblocks: []
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§4 search, §5 storage, §10 UX]
source_feedback: 사용자 도그푸딩 2026-05-06 — agent 가 search hit / citation 보고 더 깊이 파볼 때 raw chunk text + 주변 context 필요.
---

# p9-fb-35 — Verbatim fetch

> ✅ **구현 완료.** 본 spec 은 구현 시점의 frozen 상태. post-merge deviation 은 [HOTFIXES.md](../HOTFIXES.md) 참조.

상세 설계: `docs/superpowers/specs/2026-05-09-p9-fb-35-verbatim-fetch-design.md`.
구현 계획: `docs/superpowers/plans/2026-05-09-p9-fb-35-verbatim-fetch.md`.

## 증상 / 동기

- search 결과의 snippet 은 highlight 중심 — agent 가 "이 chunk 의 전체 raw text" 또는 "이 chunk 앞뒤 context" 원함.
- 현재 inspect 는 TUI 전용 — CLI / `--json` 으로 chunk 가져오는 명시 surface 없음.
- citation 의 doc_id 만 받고 doc 전체 다시 ingest / read 하는 비효율.

## Goal (skeleton)

- `kebab fetch chunk <chunk_id> [--context N]` — chunk verbatim + 앞뒤 N 개 chunk.
- `kebab fetch doc <doc_id>` — doc 전체 raw text.
- `kebab fetch span <doc_id> <line_start> <line_end>` — 특정 라인 범위.
- response wire schema `fetch_result.v1` 추가.

## 후속 작업 — brainstorm 필요 항목

- chunk_id / doc_id 노출 — 현재 search_hit.v1 에 있는지 확인 + 안정성.
- context window — N 개 chunk vs N tokens.
- doc 전체 fetch 의 size 제한 (fb-34 budget 과 통합).
- pdf / image 의 fetch — 텍스트 추출본 vs 원본 path.

## Risks / notes

- wire schema 신규 — `fetch_result.v1` JSON Schema 추가.
- 큰 doc fetch 시 budget control 필수 — fb-34 와 통합.
- chunk_id 안정성 — re-ingest 후 chunk_id 변경되면 agent 의 citation stale.
