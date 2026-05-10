---
phase: P9
component: kebab-cli + kebab-search + wire-schema
task_id: p9-fb-36
title: "Search filter args (--media / --ingested-after / --doc-id / --tag)"
status: completed
target_version: 0.4.0
depends_on: []
unblocks: []
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§4 search]
source_feedback: 사용자 도그푸딩 2026-05-06 — agent 가 검색 범위 좁힐 수단 필요. 현재 search 는 query string 만 받음.
---

# p9-fb-36 — Search filter args

> ✅ **구현 완료.** 본 spec 은 구현 시점의 frozen 상태. post-merge deviation 은 [HOTFIXES.md](../HOTFIXES.md) 참조.

상세 설계: `docs/superpowers/specs/2026-05-10-p9-fb-36-search-filters-design.md`.
구현 계획: `docs/superpowers/plans/2026-05-10-p9-fb-36-search-filters.md`.

## 증상 / 동기

- agent 가 "최근 1 주 내 doc 중에서만" / "pdf 만" / "tag=research 만" 검색 원함.
- 현재 query 만 받고 후처리 filter 도 없음.

## Goal (skeleton)

- `kebab search Q --media md,pdf` — media type 필터.
- `kebab search Q --ingested-after 2026-04-01` — ingest 시점 필터 (fb-32 stale 와 연계).
- `kebab search Q --doc-id <id>` — 특정 doc 의 chunk 만.
- `kebab search Q --tag <tag>` — tag 시스템 도입 시 (선행 brainstorm).
- `--exclude-doc-id`, `--exclude-tag` 도 검토.

## 후속 작업 — brainstorm 필요 항목

- tag 시스템 도입 여부 — 새 SQLite 테이블 / migration.
- filter 적용 layer — SQLite WHERE 절 + Lance vector pre-filter.
- AND vs OR 의미 — 다중 filter 조합 default.
- 기존 wire `SearchRequest` 에 추가 필드 (additive minor).

## Risks / notes

- Lance vector pre-filter 가 효율적인지 (post-filter 면 k 부족 가능).
- tag 시스템은 큰 추가 — 분리 spec 으로 갈 수도.
- fb-32 (stale) 의 `ingested_at` 필드와 통합 — 같은 batch.
