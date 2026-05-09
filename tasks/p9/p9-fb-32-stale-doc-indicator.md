---
phase: P9
component: kebab-app + kebab-tui + kebab-cli
task_id: p9-fb-32
title: "Stale doc indicator (ingest 시점 대비 X 일 임계 알림)"
status: completed
target_version: 0.4.0
depends_on: []
unblocks: []
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§3 ingest, §10 UX]
source_feedback: 사용자 도그푸딩 2026-05-06 — Claude Code 가 kebab CLI 사용 후 "최신성 약함" 지적. ingest 시점 snapshot 이라 이후 변경 사실 미반영. local-first 단일 사용자라 web fetch 안 함이 의도지만 사용자 / 외부 도구가 stale 여부 인지 못 함.
---

# p9-fb-32 — Stale doc indicator

> ✅ **구현 완료.** 본 spec 은 구현 시점의 frozen 상태. post-merge deviation (특히 search_hit.v1 / citation.v1 의 required-field 확장) 은 [HOTFIXES.md](../HOTFIXES.md) 의 `2026-05-09 — p9-fb-32` 항목 참조 — live source of truth.

상세 설계: `docs/superpowers/specs/2026-05-08-p9-fb-32-stale-doc-indicator-design.md`.
구현 계획: `docs/superpowers/plans/2026-05-09-p9-fb-32-stale-doc-indicator.md`.

## 증상 / 동기

- 답변에 사용된 chunk 가 N 일 전 ingest snapshot — 사용자 / 외부 도구는 fresh 여부 모름.
- p9-fb-23 (incremental ingest) 가 mtime 변경 doc 만 재처리 — 사용자가 자주 ingest 하면 자연 해결, 단 사용자가 자주 안 돌리는 doc 도 있음.

## Goal (skeleton — brainstorm 단계에서 확정)

- 각 search hit / citation 에 `ingested_at` (또는 `age_days`) 필드 노출.
- TUI inspect / search 결과에 stale 표시 (예: 30 일 이상 = 노란색 경고).
- CLI `--json` 도 동일 필드 — 외부 도구가 stale 여부 판단.
- 옵션: stale doc 자동 재 ingest 제안.

## 후속 작업 — brainstorm 필요 항목

- threshold 정책 — 사용자 config 가능 여부 (`stale_threshold_days`).
- "stale" 의 정의 — ingest 시점 vs file mtime 시점 vs 둘 다.
- wire schema search_hit.v1 / answer.v1 의 citation 에 필드 추가 — additive minor.
- TUI 색상 / 표시 방식 — p9-fb-14 color theme 와 통합.

## Risks / notes

- p9-fb-23 incremental ingest 와 의존 — `ingested_at` 정확성 위해 incremental 의 timestamp 갱신 동작 확인.
- additive wire 변경이라 외부 통합 영향 적음.
- 사이즈 작음 (S) — 단순 필드 추가 + 표시 로직.
