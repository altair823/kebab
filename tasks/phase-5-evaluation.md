---
phase: P5
title: "Golden query / regression eval"
status: completed
depends_on: [P4]
source: kb_local_rust_report.md §17 Phase 5, §18
---

# P5 — Golden query / regression eval

## 목표

검색/RAG 품질을 회귀 테스트 가능한 지표로 측정. 모델/chunker/embedding 교체 의사결정의 근거.

## 산출 crate

- `kb-eval` — golden query 실행기, 지표 계산, report 생성.

## Golden set fixture

`fixtures/golden_queries.yaml`:

```yaml
- id: q-001
  query: "Markdown chunking 규칙"
  lang: ko
  expected_doc_ids:
    - doc:notes/rust/kb-architecture.md
  expected_chunk_ids:
    - chunk:notes/rust/kb-architecture.md#chunking-policy
  must_contain:
    - "heading"
    - "code block"
  forbidden:
    - "embedding"   # 잘못된 chunk 매칭 검출용
  difficulty: easy

- id: q-002
  query: "저장소 전략 요약"
  ...
```

규모: 시작 30~50개. 한영 혼합 포함.

## 지표

| 지표 | 의미 | 단계 |
|------|------|------|
| `hit@k` | 정답 chunk_id 가 top-k 안에 있는 비율 | 검색 |
| `MRR` | mean reciprocal rank | 검색 |
| `recall@k_doc` | 정답 doc_id 회수율 (chunk 수준 미스 허용) | 검색 |
| `citation_coverage` | 답변 citation 중 실제 chunk 일치 비율 | RAG |
| `groundedness` | `must_contain` 모두 포함 비율 | RAG |
| `empty_result_rate` | 0 hit query 비율 | 검색 |
| `refusal_correctness` | 근거 없는 query 거절 비율 | RAG |

## 실행 모드

```text
kb eval run --suite golden [--mode {lexical,vector,hybrid}] [--with-rag]
kb eval compare <run_id_a> <run_id_b>
kb eval report <run_id> --format {json,md,html}
```

run record:

```rust
pub struct EvalRun {
    pub run_id: String,
    pub created_at: OffsetDateTime,
    pub commit_hash: Option<String>,
    pub config_snapshot: ConfigSnapshot,   // chunker_version, embedding model, llm model, prompt template version, fusion params
    pub per_query: Vec<QueryResult>,
    pub aggregate: AggregateMetrics,
}
```

DB 저장 (`eval_runs`, `eval_query_results` table) 또는 JSON 파일. 재현성을 위해 config snapshot 동시 저장.

## Compare report

두 run 간 diff:

- query 단위 win/loss/draw
- aggregate 차이
- regression query (이전엔 hit, 이번엔 miss) 강조

## 비-목표

- 자동 hyperparameter 탐색 — 안 함.
- LLM judge ("LLM as a judge") — P5 범위 밖. groundedness 는 rule-based (`must_contain`) 만.

## kb-app facade 확장

```rust
pub fn eval_run(opts: EvalRunOpts) -> anyhow::Result<EvalRun>;
pub fn eval_compare(a: &str, b: &str) -> anyhow::Result<CompareReport>;
```

## 테스트

- golden fixture 자체의 정합성 검사 (referenced doc_id/chunk_id 가 corpus 에 존재).
- eval 실행 자체가 deterministic (temperature=0 + 동일 seed).
- snapshot test: aggregate 지표 출력 형식 동결.

## 의존성 경계

- `kb-eval` 은 `kb-app` 만 호출 (검색/ask 는 facade 통해서). 내부 store/LLM 직접 호출 금지.

## 완료 조건

- [ ] `fixtures/golden_queries.yaml` 30+ 개
- [ ] `kb eval run` 으로 hit@k, MRR, citation_coverage 산출
- [ ] `kb eval compare` 로 두 run 비교 가능
- [ ] config snapshot 이 run 에 저장됨 (chunker, embedding, llm, prompt 버전)
- [ ] CI 로 회귀 감지 가능 (예: hit@5 가 baseline 대비 -3% 이상 떨어지면 실패)

## 리스크 / 주의

- golden set bias = eval bias. 한 사람이 만든 set 은 그 사람 검색 패턴에 과적합. 확장 시 다양성 의식.
- LLM 답변 변동성: 모델 버전 / 시드 고정 안 하면 비교 무의미.
- 정답 chunk_id 는 chunker version 변경 시 깨짐. golden set 도 versioning 필요.
