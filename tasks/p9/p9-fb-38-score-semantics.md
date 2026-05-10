---
phase: P9
component: kebab-search + kebab-app + wire-schema
task_id: p9-fb-38
title: "Score semantics 노출 + 문서화 (RRF score 천장 / 채널별 score 분리)"
status: completed
target_version: 0.5.0
depends_on: []
unblocks: []
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§4 search, §10 UX, wire-schema search_hit.v1]
source_feedback: 사용자 도그푸딩 2026-05-06 — Claude Code 가 kebab CLI 사용 후 "top score ~0.5 천장" 지적. RRF 의 rank-only fusion 특성상 absolute relevance 가 아닌데 외부 도구가 score 를 confidence 로 오해.
---

# p9-fb-38 — Score semantics 노출 + 문서화

> ✅ **구현 완료.** 본 spec 은 구현 시점의 frozen 상태.
>
> - Design: [`docs/superpowers/specs/2026-05-10-p9-fb-38-score-semantics-design.md`](../../docs/superpowers/specs/2026-05-10-p9-fb-38-score-semantics-design.md)
> - Plan: [`docs/superpowers/plans/2026-05-10-p9-fb-38-score-semantics.md`](../../docs/superpowers/plans/2026-05-10-p9-fb-38-score-semantics.md)

## 증상 / 동기

- hybrid 검색의 RRF score 가 일정 ceiling 에 머무름. RRF 수식 (`2/(k+rank)`, post-merge hotfix) 상 max = `2/(k+1)`.
- 외부 도구 (Claude Code skill, MCP) 가 `score` 를 0~1 confidence 로 해석 → "0.5 면 50% 확신" 오용.
- 단일 channel score (raw BM25 / cosine sim) 가 wire 에 노출 안 됨 — 디버깅도 어려움.

## Goal (skeleton — brainstorm 단계에서 확정)

- score 의 의미를 wire 와 README 에 명시.
- 채널별 raw score (lexical BM25, vector cosine) 를 search_hit 에 옵션 필드로 노출.
- RRF score 와 channel score 의 관계 / scale 문서화.

## 후속 작업 — brainstorm 필요 항목

- score field 를 그대로 둘지 (legacy), `rrf_score` / `lexical_score` / `vector_score` 분리할지.
- wire schema 변경이 additive (minor) 인지 breaking (major) 인지 결정.
- README / docs/wire-schema 갱신 범위.

## Risks / notes

- wire schema breaking 시 외부 통합 (claude-code skill 등) 영향 — 버전 cascade 필요.
- spec PR 우선 — design §4 search score scale 정의 추가.
