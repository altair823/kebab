---
title: Query-paraphrase robustness — 변형 일관성 평가 프레임워크 (측정 먼저)
date: 2026-05-29
status: design (approved-to-plan)
related:
  - docs/superpowers/handoffs/2026-05-29-crossscript-rerank-progress-handoff.md
  - docs/superpowers/specs/2026-05-29-crossscript-rerank-experiment-design.md (선행 실험 — overlap 프록시의 한계)
  - memory: project_crossscript_diagnosis, project_rerank_experiment, project_ranking_deferred, feedback_search_quality_dogfood
goal_reframe: "한/영 cross-script overlap → 같은 의미의 다양한 표현(동의어·다른 어휘·풀어쓴 문장·한/영)에서 일관되게 좋은 답"
---

# Query-paraphrase robustness — 변형 일관성 평가 프레임워크

## 0. 한 문단 요약

같은 의미를 다른 표현(동의어, 다른 어휘, 풀어쓴 문장, 한국어/영어)으로 물어도 **답변 품질이
일관되게 좋아야 한다**는 것이 목표다. 지난 cross-encoder reranker 실험은 "한/영 top-k 겹침
(overlap)"이라는 **프록시 지표**를 최적화하다 헛돌았다 (full-chunk-text 까지 시도했으나 회귀가
1:1 재현 — 가설 반증, 핸드오프 참조). 이번엔 처방을 만들기 전에 **진짜 지표(변형 간 답변 품질
일관성)를 직접 재는 평가 프레임워크**를 먼저 만든다. 이 평가가 (A) "핵심 문서는 후보 풀에
들어왔는데 순위만 출렁" 인지 (B) "다른 단어로 물으니 핵심 문서가 아예 후보에서 빠짐" 인지를
숫자로 판별하고, 그 결과에 따라 처방(near-tie 흡수 vs 쿼리 확장)을 별도 spec 으로 확정한다.
**본 spec 의 구현 범위는 Phase 1 (평가 프레임워크) 까지.** Phase 2 (처방) 는 측정 결과 게이트
뒤의 조건부 설계다.

## 1. 진단 근거 (왜 측정 먼저인가)

- **확정된 근본 원인** ([[project_crossscript_diagnosis]]): vector near-tie 불안정 — 상위 후보들의
  cosine 점수가 Δ0.003~0.005 로 다닥다닥 붙어, "top-k" 라는 칼같은 cutoff 가 near-tie 뭉치
  한가운데를 지나면 표현 차이(동의어/한영)가 핵심 문서를 9등↔11등으로 흔든다.
- **사용자 실제 불편** (2026-05-29 brainstorm): "한쪽 답이 나쁘다" — 겹침이 아니라 **답변 품질
  비대칭**이 핵심. 어느 쪽이 나쁠지는 **쿼리마다 다름** → 특정 언어의 구조적 결함이 아니라
  near-tie 불안정이 표현마다 다르게 발현.
- **선행 실험의 교훈** ([[project_rerank_experiment]]): overlap 프록시 최적화 → cross-encoder 가
  한/영 query 로 pool 을 독립 재정렬해 토픽별 수렴/발산. full-chunk-text 로도 database −4 등
  회귀가 그대로. **원인(near-tie)을 모르고 프록시를 최적화하면 또 헛돈다.** → 측정 선결.
- **(A) vs (B) 미해결**: 표현이 바뀔 때 핵심 문서가 ① 후보 풀엔 있는데 순위만 밀린 건지(A,
  near-tie), ② 후보 풀에서 아예 빠진 건지(B, 어휘 격차) 모른다. 처방이 완전히 다르므로 먼저 측정.

## 2. 범위 (scope)

**Phase 1 (본 spec 구현 대상):**
- `kebab-eval` 의 golden suite 에 **변형 그룹(intent group)** 개념 추가 — 같은 의도의 여러
  표현이 같은 정답(expected_doc_ids / expected_chunk_ids / must_contain)을 공유.
- **변형 일관성 메트릭** 산출: 그룹 내 recall/답변정답 의 분산(spread)·최악값, 그리고
  recall@pool vs recall@k 대비로 (A)/(B) 자동 분류.
- dogfood KB 에 큐레이션된 변형 그룹 ~6–10 개 (각 3–5 표현). 정답 문서는 **corpus 의미로 판정**
  (순환 회피, [[feedback_search_quality_dogfood]]).
- 측정 실행 → (A)/(B) 진단 리포트 → Phase 2 결정 게이트.

**Phase 2 (조건부, 별도 spec — 본 구현 제외):**
- (A) 우세 → near-tie 밴드 흡수 (cutoff 를 near-tie band 까지 확장; 검색 순서 불변, 저위험).
- (B) 우세 → 쿼리 확장/번역 (로컬 LLM).
- Phase 1 평가셋으로 처방 효과를 진짜 지표로 검증.

**비범위 (YAGNI):**
- LLM-judge 기반 답변 채점. `must_contain`/`forbidden` substring groundedness 가 이미 있고
  Phase 1 진단엔 충분. 필요성은 Phase 1 결과가 정한다.
- 임베딩 모델 교체(③). 전체 재임베딩 cascade 비용 + 효과 불확실 → 측정 후 최후 옵션.
- ranking 파라미터 자동 조정 ([[project_ranking_deferred]] 와 충돌 — 처방은 명시적 flag/설정).

## 3. 구조 (크레이트 경계 — design §8 준수)

- **`kebab-eval` 단독 변경.** retrieval/embedding/LLM 크레이트 직접 import 금지 규칙 유지
  (runner 만 `kebab-app` facade 사용 — P5-1 상속).
- `types.rs`: `GoldenQuery` 에 `group: Option<String>` 추가 (backward-compat — 기존 쿼리는
  `None`, 단독 그룹 취급). yaml 역직렬화 optional.
- `metrics.rs`: 기존 per-query 집계는 불변. per_query 결과를 `group` 으로 묶는 **변형 일관성
  집계** 함수 신규 (`compute_variant_consistency` 류). 기존 `AggregateMetrics` 는 안 건드림.
- `loader.rs`: 그룹 필드 로드 + 그룹 내 expected 정합성 검증(같은 그룹은 같은 expected 공유
  권장 — 경고/허용 정책은 plan 에서 확정).
- 측정 실행/리포트: 기존 `eval` 경로 재사용 (`--with-rag` 로 답변까지). 신규 metric 은 JSON
  리포트 + 사람이 읽는 요약 (CLI surface 확정은 plan).

## 4. 데이터 흐름

```
golden_queries.yaml (변형 그룹 포함)
  └─ loader → Vec<GoldenQuery>{group}
       └─ runner (eval --with-rag, mode=hybrid/vector) → per_query: {recall@k, answer.must_contain pass}
            ├─ 기존 AggregateMetrics (전체 hit@k/MRR/recall) — 불변
            └─ NEW: group 으로 묶어 변형 일관성:
                 · recall_spread@k  = max−min recall@k (그룹 내)  → 0 이면 완전 일관
                 · worst_recall@k   = min recall@k (약한 표현)
                 · answer_consistency = 모든 변형이 must_contain 통과한 그룹 비율
                 · A/B 분류: 변형별 (recall@pool_k high & recall@k low) → A(순위), recall@pool_k low → B(어휘)
```

`pool_k` = 진단용 넓은 후보 폭 (예: 50). near-tie 가설 검증을 위해 좁은 k(=답변 context 폭)와
넓은 pool 을 둘 다 측정.

## 5. 측정 / 수용 기준

- 변형 그룹 ≥6 개 (한/영 쌍 + 동의어 + 다른 어휘 + 풀어쓴 문장 골고루), 각 ≥3 표현.
- 평가 실행이 **clean**(err=0) 하고 결과가 파일로 추출 후 Read 검증됨 ([[project_rerank_experiment]]
  교훈 — 측정값 추측 금지, grep clean 추출 후에만 기록).
- 산출물: 그룹별 recall_spread@k, worst_recall, answer_consistency + A/B 분류 표.
- **수용 기준은 "처방이 좋아지는지" 가 아니라 "진단이 나오는지"** — Phase 1 은 측정 프레임워크
  완성 + (A)/(B) 판별이 목표. baseline 숫자 자체가 deliverable.
- 회귀 가드: 기존 golden suite 21쿼리의 AggregateMetrics 가 변형 그룹 추가 후에도 동일하게
  계산되어야 함 (group=None 경로 불변 — 기존 테스트 green).

## 6. 롤백 / 버전

- `group` 필드는 additive — 기존 yaml/스키마 backward-compat, 버전 cascade 트리거 아님.
- 평가 전용 변경이라 wire schema (`search_hit.v1`/`answer.v1`) 불변. 바이너리 surface 변경 없음
  (eval CLI 리포트 항목 추가 가능 — additive).
- golden_queries.yaml 의 변형 그룹은 dogfood KB 스냅샷(docs=3940 / chunks=34896, 2026-05-28)에
  큐레이션 — reset/re-ingest 시 chunk_id stale → runner bail. 재큐레이션 정책은 기존과 동일.

## 7. 미결 (구현 계획에서 확정)

- `group` 정합성: 같은 그룹이 서로 다른 expected 를 가질 때 — 에러 bail vs 경고+합집합. (권장:
  같은 그룹 = 같은 expected 강제, 위반 시 loader bail.)
- A/B 분류 임계값: "recall@pool_k high" 의 high 기준, near-tie band Δ 정의 (진단 리포트용).
- 변형 일관성 metric 의 CLI/JSON surface 형태 (기존 `eval` 출력에 합칠지 별도 서브커맨드일지).
- 변형 그룹 큐레이션: 어떤 의도 ~6–10 개를 고를지 — dogfood corpus 에서 한/영 양쪽으로 명확한
  정답 문서가 있는 토픽 선정 (rust 류 + 일반 토픽 섞기, 선행 ablation 토픽 재사용 가능).
- 답변 정답 신호: `must_contain` 큐레이션 방식 (핵심 사실 substring) — 그룹 내 공유.

## 8. 실행 방식 (사용자 지정, 2026-05-29)

- spec → plan → subagent 구현. 각 작업·리뷰는 **OMC teammate** (tmux pane spawn,
  [[feedback_teammate_spawn_mode]] / [[feedback_omc_teams_usage]]).
- 작은 작업은 sonnet, 복잡 작업은 opus ([[feedback_teammate_model_routing]] 조정).
- 테스트용 데이터는 dogfood 데이터셋(`/build/dogfood/corpus`, `/build/dogfood/golden_queries.yaml`)
  에서 가져올 수 있음.
- 빌드/테스트는 파일 redirect + exit code 확인 후에만 커밋 ([[project_rerank_experiment]] 교훈).
