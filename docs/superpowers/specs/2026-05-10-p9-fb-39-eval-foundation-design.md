---
title: "p9-fb-39 — Eval foundation design (P@k metric)"
phase: P9
component: kebab-eval + docs
task_id: p9-fb-39
status: design
target_version: 0.7.0
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§3 chunking, §4 search, §7 RAG, §11 eval]
date: 2026-05-10
---

# p9-fb-39 — Eval foundation (P@k metric)

## Goal

도그푸딩 피드백 — agent / 사용자가 "rank 5+ 부터 노이즈 섞임" 지적 (precision-at-k 저하). lever (chunk policy / RRF / score_gate / cross-encoder / embedding) 선택 전, **measurement infrastructure 먼저** 정비. 본 PR scope:

- `AggregateMetrics` 에 `precision_at_k_chunk: BTreeMap<u32, f32>` 추가 (P@5, P@10).
- chunk-level binary relevance 기반 — `expected_chunk_ids` 안 chunk 가 top-k 안 등장한 비율.
- Golden set schema 무변경 — `expected_chunk_ids` 가 ground truth (curator 책임).
- 문서화 강화 — `fixtures/golden_queries.yaml` 헤더 주석.

Lever 적용 (chunk policy / RRF tune / cross-encoder / embedding upgrade) 은 **본 spec 범위 외** — fb-39b 이후 별도 task 로 분리. 측정 도구가 먼저 있어야 lever 효과 비교 가능.

## Behavior contract

### Metric definition

```
P@k_chunk(query) = |top-k hits ∩ expected_chunk_ids| / k
```

**Denominator = k 고정**. `hits.len() < k` 인 경우에도 분모는 k — top-k 부족도 precision 손실로 간주 (`hit_at_k` 와 동일 컨벤션).

`expected_chunk_ids` 빈 query 는 metric 계산에서 skip (`hit_at_k_chunk` 와 동일 정책).

**Aggregation**: 모든 valid query (expected_chunk_ids 비어있지 않음) 의 P@k_chunk 평균. valid query 0 건이면 NaN → JSON null.

### Wire shape

`AggregateMetrics` 신규 field:

```rust
pub struct AggregateMetrics {
    pub hit_at_k: BTreeMap<u32, f32>,
    pub mrr: f32,
    pub recall_at_k_doc: BTreeMap<u32, f32>,
    /// p9-fb-39: chunk-level precision at k. Binary relevance via
    /// `expected_chunk_ids`. Denominator = k (fixed). Skip queries
    /// with empty `expected_chunk_ids`.
    #[serde(default)]
    pub precision_at_k_chunk: BTreeMap<u32, f32>,
    // ... 기존 필드 ...
}
```

`#[serde(default)]` — 기존 eval_runs.metrics_json (옛 binary 가 기록한) 에 field 부재 시 empty BTreeMap 로 deserialize. backwards-compat 보장.

### k values

`compute_aggregate_metrics` 가 5, 10 두 값에 대해 계산. (기존 `hit_at_k` / `recall_at_k_doc` 가 이미 동일 k 사용 — 재사용.)

## Allowed / forbidden dependencies

- `kebab-eval`: 신규 dep 없음. metrics 모듈 확장만.
- 다른 crate 무수정.

`kebab-eval` 의 `metrics` / `compare` 모듈은 retrieval / embedding / LLM crate 직접 import 금지 룰 그대로 (P5 inheritance).

## Public surface delta

### kebab-eval::metrics

```rust
pub struct AggregateMetrics {
    // ... 기존 ...
    #[serde(default)]
    pub precision_at_k_chunk: BTreeMap<u32, f32>,
}
```

`compute_aggregate_metrics` body 안 새 누적 BTreeMap + 평균 계산 추가. NaN handling 은 기존 `serialize_f32_nan_as_null` 패턴 재사용 — 단, BTreeMap<u32, f32> 의 NaN 처리 패턴이 hit_at_k 와 동일하게 round_recall_map 같은 helper 통해.

## Test plan

| kind | description |
|------|-------------|
| unit (metrics) | `precision_at_k_chunk` empty expected → query skip → metric BTreeMap 안 entry 부재 또는 NaN |
| unit (metrics) | exact match: 5 hits, top-3 in expected → P@5 = 3/5 = 0.6 |
| unit (metrics) | partial top-k: hits.len() = 3 < k=5, all 3 in expected → P@5 = 3/5 = 0.6 (분모 k 고정) |
| unit (metrics) | top-k 안 expected 0건 → P@5 = 0.0 |
| unit (metrics) | 모든 query expected 비어있음 → P@k entry 부재 또는 NaN → JSON null |
| unit (metrics) | `AggregateMetrics` serde roundtrip — precision_at_k_chunk 신규 field 보존 |
| unit (metrics) | 옛 JSON (precision_at_k_chunk 부재) deserialize → empty BTreeMap default |
| 통합 (eval runner) | runner end-to-end → eval_runs.metrics_json 안 precision_at_k_chunk 채워짐 |

snapshot tests (기존 metrics 출력 fixture 가 있다면 갱신 — `cargo test -p kebab-eval` 수행 후 fixture diff 확인).

## Implementation steps (high-level)

1. `kebab-eval::metrics`: `AggregateMetrics.precision_at_k_chunk` field 추가 + 계산 로직 + 단위 테스트.
2. snapshot tests 갱신 (있다면).
3. `fixtures/golden_queries.yaml` 헤더 주석 강화 — `expected_chunk_ids` 채우기 가이드.
4. README `kebab eval` 섹션 또는 design §11 eval 에 P@k 정의 한 줄 추가.
5. tasks/INDEX.md / spec status flip.

3-5 step PR. 단일 세션 내 완료 가능.

## Risks / notes

- **분모 = k 고정 정책**: `hits.len() < k` 인 query 가 많으면 P@k 가 항상 < 1.0. 사용자 직관과 다를 수 있음 — README/design 에 명시.
- **frozen design vs new metric**: design §11 eval 의 metric 표 갱신 필요. frozen contract 변경 트리거 — `target_version: 0.7.0` bump 명시.
- **lever deferral**: 본 spec contract_sections 는 §3 chunking + §4 search + §7 RAG + §11 eval 인데, 실제 본 PR 은 §11 만 건드림. lever 적용 (chunk policy / RRF / cross-encoder / embedding) 은 fb-39b 이후 별도. spec status banner 에 명시.
- **expected_chunk_ids 비어있는 shipped golden**: 현재 `fixtures/golden_queries.yaml` 의 g001-g005 모두 expected_chunk_ids 비어있음. P@k 계산 시 모두 skip — out-of-the-box 측정값 0건. curator 가 자기 KB 로 채워야 metric 의미 가짐. 의도 — golden set 은 workspace 의존이라 shipped fixtures 는 template, 실제 측정은 user 가 채워서 한다.
- **fb-23 incremental ingest 와 충돌 없음**: 본 PR 은 metric 만 추가. chunker_version / embedding_version 무변경.

## Out of scope

- Lever 적용 (chunk policy retune / RRF k tune / score_gate default ON / cross-encoder PoC / embedding model 업그레이드).
- NDCG / MAP / 기타 ranking metric.
- precision_at_k_doc (doc-level — recall_at_k_doc 가 이미 있음, 본 spec 은 chunk-level 만).
- Golden set 콘텐츠 확장 (g006+ 추가) — curator 책임.
- Synthetic golden generator (`kebab eval golden-from-corpus` 등).
- Per-query relevance score (binary 0/1 만 — graded relevance 는 NDCG 도입 시 검토).

## Documentation updates (implementation PR 동시)

- `fixtures/golden_queries.yaml` — 헤더 주석에 `expected_chunk_ids` ground truth 의미 + P@k 측정 위해 채우기 권장 안내.
- `README.md` — `kebab eval` 섹션 (있다면) 에 P@k metric 한 줄. 없으면 skip.
- `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md` §11 eval — metric 표에 `precision_at_k_chunk` 한 줄 추가.
- `tasks/p9/p9-fb-39-retrieval-precision-tuning.md` — `status: open → completed`, 단 banner 에 "eval foundation only, lever 적용 deferred to fb-39b" 명시 + design/plan 링크.
- `tasks/INDEX.md` — fb-39 행 ✅ (eval foundation only).
