---
phase: P9
component: kebab-eval + docs
task_id: p9-fb-39
title: "Retrieval precision 튜닝 (rank 5+ 노이즈 완화)"
status: completed
target_version: 0.7.0
depends_on: []
unblocks: []
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§3 chunking, §4 search, §7 RAG, §10.3 eval metrics]
source_feedback: 사용자 도그푸딩 2026-05-06 — Claude Code 가 kebab CLI 사용 후 "rank 5+ 부터 노이즈 섞임" 지적. precision-at-k 가 k=5 이후 떨어짐.
---

# p9-fb-39 — Retrieval precision 튜닝

> ✅ **Eval foundation 부분 구현 완료.** P@k metric (P@5, P@10) 추가. 본 spec 의 lever 적용 (chunk policy / RRF / cross-encoder / embedding 업그레이드) 은 별도 task 로 분리 (fb-39b 이후).
>
> - Design: [`docs/superpowers/specs/2026-05-10-p9-fb-39-eval-foundation-design.md`](../../docs/superpowers/specs/2026-05-10-p9-fb-39-eval-foundation-design.md)
> - Plan: [`docs/superpowers/plans/2026-05-10-p9-fb-39-eval-foundation.md`](../../docs/superpowers/plans/2026-05-10-p9-fb-39-eval-foundation.md)

## 증상 / 동기

- top-1~4 chunk 는 관련 있으나 5번째부터 무관 chunk 섞임. recall OK, precision-at-k 저하.
- LLM 이 noise chunk 를 context 에 포함하면 답변 품질 저하 / hallucinate 위험.

## Goal (skeleton — brainstorm 단계에서 확정)

- top-k 결과의 precision 향상. 후보:
  1. chunk policy 재검토 (size / overlap / boundary).
  2. RRF k 파라미터 (현재 default 60) 재튜닝 또는 score gate threshold default ON.
  3. cross-encoder reranker PoC — top-N retrieve → rerank → top-k.
  4. embedding model 업그레이드 (fastembed default → 더 큰 / 한글 강한 모델).
- 평가 지표: P@5, P@10, MRR, NDCG. P5 eval runner 활용.

## 후속 작업 — brainstorm 필요 항목

- 어느 lever 부터 손볼지 — 비용 / 효과 trade-off.
- cross-encoder 도입 시 local-only 유지 가능한지 (fastembed cross-encoder 지원?).
- embedding 변경이면 `embedding_version` cascade — 전체 재처리 필요.

## Risks / notes

- embedding_version bump = 전체 vector index 재구축. p9-fb-23 incremental ingest 와 충돌 가능.
- cross-encoder 는 latency 증가 — 단일 사용자 local 환경에서 받아들일 수 있는지 확인.
- eval golden set 부족하면 튜닝 불가 — golden set 확장 선행 필요할 수 있음.
