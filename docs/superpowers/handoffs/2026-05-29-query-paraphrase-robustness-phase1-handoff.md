---
title: Query-paraphrase robustness — Phase 1 (변형 일관성 평가) 완료 + (A)/(B) 진단
date: 2026-05-29
branch: feat/crossscript-rerank
status: Phase 1 구현·측정 완료 — Phase 2(처방) 결정 대기
related:
  - docs/superpowers/specs/2026-05-29-query-paraphrase-robustness-eval-design.md
  - docs/superpowers/plans/2026-05-29-query-paraphrase-robustness-eval.md
  - docs/superpowers/handoffs/2026-05-29-crossscript-rerank-progress-handoff.md (선행 rerank 실험)
  - memory: project_paraphrase_robustness, project_rerank_experiment, project_crossscript_diagnosis
---

# Query-paraphrase robustness — Phase 1 완료

## TL;DR

같은 의미를 다른 표현(한/영·동의어·풀어쓴 문장)으로 물어도 일관된 품질이 나오는지 **직접 측정**하는
프레임워크를 `kebab-eval` 에 구축하고(Phase 1), dogfood KB 에 8개 변형 그룹(32 변형)을 큐레이션해
측정했다. 결과: **문제는 한/영이 아니라 "어휘 거리"** 이고, **(B) 어휘격차가 (A) 순위출렁보다 우세**
(B_dominant=4 vs A_dominant=2). 즉 선행 rerank 실험(A형 처방)은 소수만 커버 — "측정 먼저" 논제가
정량 검증됨. Phase 2 처방(쿼리 확장/번역 vs near-tie 흡수)은 사용자 결정 대기.

## Phase 1 구현 (branch `feat/crossscript-rerank`, 머지 전)

| Task | 커밋 | 내용 | 리뷰 |
|---|---|---|---|
| 1 | `e491a7b`+`48c94de` | `GoldenQuery.group` + loader 그룹 정합성 검증 | sonnet APPROVE-WITH-NITS (반영) |
| 2 | `0ff38581`+`67e104f` | `kebab-eval::variant` 메트릭 + (A)/(B) 분류 | opus CHANGES-REQUESTED → H1/M1 수정 |
| 3 | `895dcea` | `kebab eval variants <run_id>` CLI | 직접 검증 |

- **메트릭**: 그룹 내 `recall@narrow(10)` vs `recall@pool(50)` 대비 →
  `Ok`(top-10 안) / `MisRanked`(A: pool엔 있고 top-10 밖) / `Missing`(B: pool에도 없음).
  그룹 롤업: recall_spread@10, worst@10, A/B dominant, fully_consistent, `pool_possibly_truncated`.
- **리뷰 H1 (실제 버그, 측정 전 차단)**: `POOL_K=50` 인데 `eval run --k` 기본=10 →
  pool==narrow 항상 → A 영원히 안 나옴, 전부 B 오분류. 수정: `config_snapshot_json` 에 `eval_k`
  추가 + `eval_k < 50` 이면 `bail` + `pool_possibly_truncated` 플래그. 회귀 테스트 고정.
- 전 task `cargo test`+`clippy -D warnings` green. 기존 `AggregateMetrics` 경로 불변(회귀 가드 통과).

## 측정 (Task 4 큐레이션 + Task 5)

- golden: `/build/dogfood/golden_queries.yaml` 에 8그룹×4변형(ko/en/동의어/풀어쓴문장) append.
  정답 문서는 **corpus 의미로 판정**(검색 상위 자동채택 X — ownership 의 rank1 이 garbage-collection.md
  의 대조 언급이라 정답 아님을 실증). `topics/` 군(1파일=1주제)이라 판정 명확.
- run: `kebab eval run --mode hybrid --k 50` (run_id `run_019e74dcae2778f3984df49ee79b725a`).
- 리포트: `kebab eval variants <run_id>` (⚠️ `KEBAB_EVAL_GOLDEN=/build/dogfood/golden_queries.yaml`
  설정 필수 — 미설정 시 default golden → groups=0). 전체:
  `/build/dogfood/logs/2026-05-29-paraphrase-robustness-variants-hybrid.txt`.

### 결과 (hybrid, k=50, err=0)

```
groups=8 fully_consistent=2 A_dominant=2 B_dominant=4 mean_spread@10=0.750 pool=top-50
```

| group | A(MisRanked) | B(Missing) | 분류 | 핵심 |
|---|---|---|---|---|
| ownership | 0 | 0 | 완전 일관 ✅ | 4변형 모두 recall 1.0 |
| isolation_levels | 0 | 0 | 완전 일관 ✅ | 4변형 모두 1.0 (한국어 "트랜잭션 격리 수준" 포함) |
| cap_theorem | 2 | 0 | (A) near-tie | 풀어쓴 문장(영/한) recall@50=1·@10=0 |
| vector_database | 2 | 0 | (A) near-tie | "벡터 데이터베이스"·"근사 최근접…" @50=1·@10=0 |
| raft | 0 | 1 | (B) 어휘격차 | 영어 풀어쓴 "how nodes agree…" @50=0 |
| mvcc | 0 | 1 | (B) 어휘격차 | 영어 풀어쓴 "how databases serve reads…" @50=0 |
| backprop | 0 | 2 | (B) 어휘격차 | 한국어 "역전파 알고리즘"·"연쇄 법칙…" @50=0 |
| gradient_descent | 0 | 2 | (B) 어휘격차 | 한국어 "경사 하강법"·"손실 함수…" @50=0 |

raw search 독립 검증: `kebab search "역전파 알고리즘" --k 50` → backprop doc(54e0ac…) **top-50 부재**
(top은 무관한 algorithm.md). eval 파이프라인 artifact 아님 확인.

### 진단 (Read 검증된 숫자 기반)

1. **문제는 실재하고 크다**: mean_spread@10=0.750 — 같은 의도의 표현 간 recall 이 평균 0.75 출렁.
2. **한/영 문제가 아니라 어휘 거리 문제**: 영어 풀어쓴 문장도 miss(raft/mvcc), 일부 한국어는 잘 됨
   (러스트 소유권, 트랜잭션 격리 수준, MVCC 동작 원리, 래프트 합의 알고리즘). 사용자 재정의 목표
   ("정확한 단어가 아닌 같은 의미의 다른 단어")와 정확히 일치.
3. **(B) 어휘격차 우세 (4 vs 2)**: 못 찾은 정답이 top-50 pool 에도 없음 → 재정렬(rerank)로 해결 불가.
   특히 ml-training(backprop/gradient_descent) 한국어는 영어 본문 문서를 의미·표층 둘 다 못 매칭.
   → **쿼리 확장/번역**(또는 더 나은 다국어 임베딩) 처방 신호.
4. **(A) 순위출렁은 소수 (cap_theorem/vector_database)**: 정답이 pool엔 있고 top-10 밖 →
   near-tie 흡수 / rerank 후보. 선행 rerank 실험이 도움 됐을 그룹.
5. **"측정 먼저" 논제 검증**: rerank(A형) 단독은 6개 문제 그룹 중 2개만 커버. 선행 실험이 overlap
   프록시로 헛돈 이유가 데이터로 드러남.

## Phase 2 (처방) — 결정 대기

본 spec §2 의 조건부 게이트대로:
- **(B) 우세이므로 쿼리 확장/번역이 1차 후보** (로컬 LLM gemma). cap_theorem/vector_database 의
  (A) 성분엔 near-tie 흡수가 보조.
- 처방 효과는 본 Phase 1 평가셋(`kebab eval variants`)으로 재측정해 검증 (또 프록시 금지).
- 미결: 확장/번역의 형태(쿼리→영어 번역 후 retrieve, 양쪽 retrieve 합집합, HyDE 류 등),
  latency·품질 trade-off, default on/off. → Phase 2 brainstorm/spec 에서.

## Phase 2 방향 — 딥리서치 + PoC (2026-05-30)

- **딥리서치** (`docs/superpowers/research/2026-05-30-vocabulary-gap-recall-fix-research.md`, 104 agent,
  22 confirmed/3 killed): 어휘격차 pool-miss 최선책 = **색인시 doc-side expansion(doc2query)**.
  pool 자체를 키우고(rerank 아님), per-query 지연 ~0(색인시 1회 → 사용자가 거부한 per-query LLM 아님),
  정확매칭 보존(별도 필드 append). 단 vanilla mt5 doc2query 는 같은언어라 한/영 갭은 색인시 KO↔EN
  대체 query 생성 필요. query-side(HyDE=거부된 per-query LLM, Vector-PRF=recall 주장 0-3 기각) 부적합.
  learned-sparse(SPLADE/MILCO)는 CPU/Rust 경로 없거나 교차언어 약함.
- **PoC 확인** (`/build/dogfood/logs/2026-05-30-docexpansion-poc-result.md`): dogfood KB(3940 doc)에
  backprop/raft 별칭추가판 ingest → recall@50=0 이던 3쿼리 전부 **rank 1~2 로 부활**(hybrid+vector),
  별칭은 골든쿼리 verbatim 아님(일반화 확인). **딥리서치의 핵심 미검증 고리를 실 corpus 로 정량 확인.**
  - ⚠️ dogfood KB 현재 3942 doc (PoC 2개 잔존, corpus/_poc 는 삭제). variant 골든은 원본 doc_id
    타겟이라 baseline eval 무영향. pristine 필요 시 `kebab reset` + reingest.
- **Phase 2 권고**: 색인시 doc-side expansion(같은언어 + KO↔EN 번역 별칭, 로컬 gemma 색인시 1회) →
  별도 FTS5 필드 → RRF. flag off 기본. 효과는 `kebab eval variants` 로 재측정. brainstorm→spec→plan.

## 다음 세션 첫 작업

1. 사용자와 Phase 2 방향 확정 (쿼리 확장/번역 설계 brainstorm).
2. 또는 Phase 1 코드(group + variant + CLI)를 main 머지할지 결정 (default off, eval 전용·additive,
   기존 동작 무영향 → 머지 안전. PR 은 gitea-pr + 리뷰 루프).
3. `--with-rag` 변형 일관성(답변 품질 직접 측정)은 미실행 — recall 진단으로 충분했음. 필요 시 후속.
