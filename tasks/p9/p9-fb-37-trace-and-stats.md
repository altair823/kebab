---
phase: P9
component: kebab-cli + kebab-search + kebab-rag
task_id: p9-fb-37
title: "Trace (--trace) + stats — pipeline 가시성"
status: open
target_version: 0.4.0
depends_on: [p9-fb-27]
unblocks: []
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§4 search, §7 RAG, §10 UX]
source_feedback: 사용자 도그푸딩 2026-05-06 — agent / 사용자가 "왜 이 결과가 나왔는지" debug 필요. retrieval pipeline 의 각 stage 결과 + KB 건강 점검 surface 부재.
---

# p9-fb-37 — Trace + stats

> ⏳ **백로그 only — 미구현 (Nice-to-have).** 본 spec 은 도그푸딩 피드백 skeleton. 구현 착수 전 [superpowers:brainstorming](../../docs/superpowers/) 으로 설계 단계 선행 필요. trace 의 verbosity level / wire shape / stats 의 별도 명령 vs schema 통합 brainstorm 후 확정.

## 증상 / 동기

- search 결과 의문 — lexical / vector / RRF / rerank 각 stage 가 무엇 반환했는지 모름.
- KB 건강 — doc count / chunk count / last ingest / index size / model versions — 단일 surface 없음.
- agent 가 stale 판단 / 사용자가 디버깅 시 둘 다 필요.

## Goal (skeleton)

- `kebab search Q --trace` 또는 `--explain` — 응답에 `trace` 필드:
  - `lexical_hits: [{doc_id, score, …}]`
  - `vector_hits: [...]`
  - `rrf_combined: [...]`
  - `reranked: [...]` (reranker 도입 시)
  - `timing: {lexical_ms, vector_ms, fusion_ms, total_ms}`
- `kebab stats --json` — KB 통계 (fb-27 의 schema 와 별도 명령 또는 통합).
- TUI inspect 에 trace view — 1 hit 클릭 시 stage breakdown.

## 후속 작업 — brainstorm 필요 항목

- trace 의 verbosity — 모든 stage default vs flag opt-in (응답 size 우려).
- stats 명령의 위치 — `kebab stats` 또는 `kebab schema --include-stats`.
- timing 정확도 — async stage 는 wall-clock 부정확.

## Risks / notes

- trace 응답 size 큼 — agent budget (fb-34) 와 충돌 가능, 기본 OFF 권장.
- fb-27 introspection 의 stats 와 중복 — brainstorm 단계 통합 결정.
- 우선순위 낮음 — 핵심 기능 (fb-26 ~ 36) 후순위.
