---
title: dogfood 검색 품질 검증 — golden suite + 정성 검증 인프라
created: 2026-05-29
status: accepted
contract_sections: [§5.7 eval runs + metrics, §6 filesystem+config layout]
---

# dogfood 검색 품질 검증 — golden suite + 정성 검증 인프라

## 1. Summary

dogfood corpus(`/build/dogfood/corpus`, ~3940 docs / ~34896 chunks)에 대한 **재사용 가능한 검색 품질 검증 인프라**를 정의한다. 평가 엔진(`kebab-eval`: metrics / runner / compare)은 **이미 존재**하므로, 새로 만들 것은 두 가지다.

1. dogfood KB 전용 **golden query suite** (`/build/dogfood/golden_queries.yaml`, ~15–20 query) — 순환 의존을 피하는 큐레이션 절차 포함.
2. golden suite 실행 + baseline 저장 + 정성 sanity 체크로 이루어진 **검증 절차**, 그리고 이를 `docs/DOGFOOD.md` 시나리오로 편입.

v0.20.2가 첫 적용 대상이며, 이 run을 **baseline**으로 `eval_runs`에 영속화한다. 이후 release는 `kebab eval compare`로 baseline 대비 회귀를 감지한다.

본 spec 작성 중 코드 재확인에서 두 가지 사실을 확정했고, 설계에 반영했다(§4, §9 참조):
- `kebab-eval`의 aggregate metric에는 **NDCG가 존재하지 않는다**. 측정 가능한 metric은 §4.2 목록이 전부다.
- `kebab eval run` CLI 경로는 원래 **`--config`를 thread하지 않았으나**, **Task A(facade-rule 정합 패치)**로 `run_eval_with_config` / `compute_aggregate_with_config` 경로가 적용됐다. dogfood KB 평가는 §4.4 명령 블록(`--config /build/dogfood/config.toml`)으로 실행한다(§5.1).

---

## 2. Background

### 2.1 이미 존재하는 평가 인프라 (`crates/kebab-eval/`)

| 모듈 | 역할 |
|------|------|
| `types.rs` | `GoldenQuery`, `EvalRunOpts`, `EvalRun`, `QueryResult` 공개 타입 |
| `loader.rs` | golden YAML 로드 + expected_* 실재 검증 (stale ref면 runner bail) |
| `runner.rs` | query별 `kebab_app` facade 호출 → `eval_runs` + `eval_query_results` + `runs_dir/<run_id>/per_query.jsonl` 영속화 |
| `metrics.rs` | `eval_query_results` row → `AggregateMetrics` 집계 |
| `compare.rs` | 두 run의 metric delta 리포트 (Markdown / JSON) |

**`GoldenQuery`** (`types.rs:14`) — required: `id`, `query`. optional(default empty/None): `lang`, `expected_doc_ids: Vec<DocumentId>`, `expected_chunk_ids: Vec<ChunkId>`, `must_contain: Vec<String>`, `forbidden: Vec<String>`, `difficulty: Option<String>`.

**`EvalRunOpts`** (`types.rs:40`) — `suite: String`, `mode: SearchMode`, `with_rag: bool`, `k: usize`, `temperature: Option<f32>`, `seed: Option<u64>`.

**golden 경로 해석** — `KEBAB_EVAL_GOLDEN` env가 비어있지 않으면 그 경로, 아니면 CWD 기준 `fixtures/golden_queries.yaml` (`runner.rs:142`, `metrics.rs:161`; runner와 metrics가 동일 상수를 공유). `~` / `${...}` 확장은 하지 않는다 — 직접 경로만.

### 2.2 metric별 ground-truth 요구사항

`aggregate_from_rows` (`metrics.rs:184`) 동작을 코드에서 확인한 결과:

- `expected_chunk_ids`가 **비어있지 않은** query만 `hit_at_k` / `mrr` / `precision_at_k_chunk` denominator에 들어간다.
- `expected_doc_ids`가 **비어있지 않은** query만 `recall_at_k_doc`에 들어간다. `expected_doc_ids`가 빈 query는 "should refuse" class로 간주되어 `refusal_correctness`(RAG run 한정)로만 평가된다.
- `must_contain` / `forbidden`은 expected_* 없이도 동작하는 **rule-based groundedness** 입력이다. 둘 다 비면 해당 query는 groundedness denominator에서 제외(무설정 golden이 공짜 1.0/0.0을 얻지 않도록).

즉 ranking metric(hit@k/MRR/precision@k)을 측정하려면 query마다 `expected_chunk_ids`를, recall을 측정하려면 `expected_doc_ids`를 큐레이션해야 한다.

### 2.3 기존 golden과의 관계

repo fixture `./fixtures/golden_queries.yaml`(현재 5 query 템플릿, expected_* 비어있음)는 fresh workspace에서 loadable하도록 의도된 **템플릿**이다. dogfood corpus용 golden은 존재하지 않는다. 본 spec의 dogfood golden은 repo fixture를 **변경하지 않고** 별도 파일(`/build/dogfood/golden_queries.yaml`)로 둔다.

---

## 3. Goals + Non-Goals

### Goals

- dogfood corpus를 대표하는 ~15–20 query golden suite를 `/build/dogfood/golden_queries.yaml`에 작성.
- 순환 의존(검색 결과가 정답을 정의) 없이 expected_chunk_ids / expected_doc_ids를 큐레이션하는 절차 확정.
- golden suite를 dogfood KB에 대해 실행 → v0.20.2 baseline을 `eval_runs`에 영속화.
- baseline 대비 회귀 감지 기준 + 정성 sanity 체크리스트 확정.
- 위 절차를 `docs/DOGFOOD.md`의 검색 품질 시나리오로 편입.

### Non-Goals

- ranking 파라미터 자동 튜닝 / heuristic 조정 — [[project_ranking_deferred]] (실사용 1주+ 후 별도 brainstorm).
- 기존 repo `fixtures/golden_queries.yaml`(템플릿) 변경.
- 새 metric 구현 (NDCG 포함). 기존 `kebab-eval` metric만 재사용.
- RAG 품질 자체 튜닝(prompt / NLI threshold). 본 spec은 retrieval(검색) 품질 검증이 주, RAG groundedness/refusal은 보조.
- **lang/media/code_lang 등 `SearchFilters` 동작 검증은 본 golden 하네스 범위 밖.** 러너가 `SearchFilters::default()` 고정(`runner.rs:151`)이므로 golden의 `lang` 필드는 큐레이션·리포트 라벨일 뿐 retrieval에 영향을 주지 않는다.

---

## 4. 설계

### 4.1 golden suite — `/build/dogfood/golden_queries.yaml` (신규)

~15–20 query. v0.20.x 검색 변경(V007 trigram → V009 한국어 형태소 + N-gram)을 집중 커버하고 corpus 대표성을 확보한다. 각 query 카테고리와 의도:

| 카테고리 | 의도 | 예시 query | lang | difficulty |
|----------|------|-----------|------|-----------|
| 한국어 2자/구 (V009 형태소) | 형태소 분석이 짧은 한국어 토큰을 잡는지 | "한국", "서울", "한국어 형태소 분석" | ko | easy~medium |
| 한국어 N-gram fallback | 형태소 분석이 분해 못 하는 복합어/신조어를 N-gram supplement가 잡는지 (V007 trigram 회귀 검증) | 형태소 미분리 복합어/신조어 query | ko | medium |
| 영어 whole-token exact | whole-token 정확 매칭과 substring 부산물 분리 | 단독 complete token query | en | easy |
| 영어 substring (V007→V009 회귀) | trigram substring 매칭이 V009 이후에도 살아있는지 | "tokenizer" | en | medium |
| hybrid 개념 query | 개념적 query에서 lexical+vector fusion이 정답을 끌어올리는지 | "hybrid search", "RAG architecture" | en | medium~hard |
| code 검색 | 코드 corpus(rust)에서 식별자/개념 검색 | "rust ownership", "dispatch 함수" | en/ko | medium |

요구사항:
- 난이도(easy/medium/hard)와 언어(ko/en)를 섞는다.
- 각 query: `id`, `query`, `lang`, `difficulty`는 필수 채움.
- ranking metric 측정 대상 query는 `expected_chunk_ids`(+ 가능하면 `expected_doc_ids`)를 §4.3 절차로 큐레이션.
- 일부 query에는 `must_contain`을 두어 groundedness를 함께 측정(예: "한국" → `must_contain: ["한국"]`은 지나치게 약하므로, RAG groundedness용 query는 답변에 반드시 등장할 핵심어를 신중히 고른다).

YAML 형식은 repo fixture와 동일(top-level query 리스트). 파일 상단 주석에 큐레이션 규칙(§4.3)과 "stale ref면 runner가 시작 시 bail"을 명시한다. 또한 golden YAML 헤더 주석에 `# curated against corpus_revision=<rev>` 를 기록해, 미래에 stale bail이 발생할 때 어느 DB 상태에 묶인 golden인지 즉시 판별할 수 있게 한다.

### 4.2 측정되는 metric (코드 확정 — `AggregateMetrics`, `metrics.rs:56`)

`kebab eval aggregate`가 산출하는 metric은 다음이 **전부**다. NDCG는 구현되어 있지 않다.

| metric | 타입 | ground-truth 요구 | 비고 |
|--------|------|------------------|------|
| `hit_at_k` | `{1,3,5,10}→f32` | `expected_chunk_ids` | 정답 chunk가 top-k 안에 있으면 hit |
| `mrr` | f32 | `expected_chunk_ids` | top-10 밖이면 0 기여 |
| `precision_at_k_chunk` | `{1,3,5,10}→f32` | `expected_chunk_ids` | denominator는 k 고정 (shortfall=precision loss) |
| `recall_at_k_doc` | `{1,3,5,10}→f32` | `expected_doc_ids` | doc-level recall |
| `groundedness` | f32 | `must_contain`/`forbidden` (RAG) | rule-based; with_rag 한정 |
| `citation_coverage` | f32 (null 가능) | RAG answer | 모든 citation path 非빈 + 최소 1개 |
| `refusal_correctness` | f32 (null 가능) | `expected_doc_ids` 빈 query (RAG) | "should refuse" + 실제 refuse |
| `empty_result_rate` | f32 | 없음 | 0건 hit query 비율 |
| `total_queries` / `failed_queries` | u32 | 없음 | run 카운트 |

`citation_coverage` / `refusal_correctness`는 denominator가 0이면 JSON `null`로 직렬화된다(예: lexical-only run은 Answer가 없으므로 둘 다 null). 검색 품질 검증의 **1차 지표는 `hit_at_k` / `mrr` / `precision_at_k_chunk` / `recall_at_k_doc`** 이고, groundedness/refusal/citation은 `--with-rag` run의 보조 지표다.

### 4.3 큐레이션 절차 (순환 회피 — 핵심)

검색 결과가 정답을 정의하면 평가가 무의미해진다. 따라서 정답은 **도메인 판정**으로, chunk_id는 **조회**로 얻는다.

1. **정답 문서를 corpus 의미로 먼저 판정.** 예: "한국" → `markdown/korean/korea-overview.md`. query 작성자가 corpus를 알고 어떤 문서가 정답인지 사람이 결정한다.
2. 그 문서의 실제 `doc_id`를 조회: `kebab --config <dogfood> list docs --json`에서 doc_path로 매칭하거나 `kebab inspect doc <id>`로 확인.
3. 그 문서 내 핵심 `chunk_id`를 조회: **`kebab inspect doc <id>`로 문서의 chunk 목록을 나열해 의미상 관련 chunk를 선택한다(chunk-level 순환 회피 — 랭커에 올라오지 않은 관련 chunk도 포함 가능).** `kebab --config <dogfood> search "<query>" --json` hit는 보조 확인용으로만 사용한다.
4. `expected_chunk_ids` / `expected_doc_ids`에 기입.

**불변식**: `expected_*`는 큐레이션 시점의 dogfood KB에 실재하는 row여야 한다. loader 계약상 stale reference면 runner가 시작 시 bail한다(`loader.rs`). dogfood KB를 reset/re-ingest하면 chunk_id가 바뀔 수 있으므로(§9 재현성), golden은 특정 corpus_revision에 묶인다.

### 4.4 실행

전제: §5.1의 `--config` thread 패치(Task A)가 적용된 binary를 사용한다.

```bash
# 1차: hybrid (lexical + vector fusion)
KEBAB_EVAL_GOLDEN=/build/dogfood/golden_queries.yaml \
  /build/out/cargo-target/target/release/kebab \
  --config /build/dogfood/config.toml \
  eval run --mode hybrid --k 10
#    → run_id 출력. 이어서 aggregate:
KEBAB_EVAL_GOLDEN=/build/dogfood/golden_queries.yaml \
  /build/out/cargo-target/target/release/kebab \
  --config /build/dogfood/config.toml \
  eval aggregate <run_id> --json \
  | tee /build/dogfood/logs/eval-hybrid-v0.20.2.json

# 2차: lexical (순수 FTS5 — V009 형태소 동작 격리 측정)
KEBAB_EVAL_GOLDEN=/build/dogfood/golden_queries.yaml \
  /build/out/cargo-target/target/release/kebab \
  --config /build/dogfood/config.toml \
  eval run --mode lexical --k 10
KEBAB_EVAL_GOLDEN=/build/dogfood/golden_queries.yaml \
  /build/out/cargo-target/target/release/kebab \
  --config /build/dogfood/config.toml \
  eval aggregate <run_id> --json \
  | tee /build/dogfood/logs/eval-lexical-v0.20.2.json
```

CLI flag (코드 확정 — `EvalWhat::Run`, `main.rs:406`):

| flag | 기본값 | 비고 |
|------|--------|------|
| `--suite <s>` | `golden` | `eval_runs.suite` 라벨 |
| `--mode <lexical\|vector\|hybrid>` | `lexical` | `ModeFlag` enum (`main.rs:441`) |
| `--k <n>` | `10` | retrieval top-k (with_rag 시 `AskOpts.k`) |
| `--with-rag` | off | query마다 `kebab_app::ask`도 호출, groundedness/citation/refusal 측정 |
| `--temperature <f>` | none | with_rag 결정성: `0.0` 권장 |
| `--seed <u64>` | none | with_rag 결정성: 고정 seed 권장 |

중요: `eval run`은 metric을 **집계하지 않는다** — `run_id`만 출력하고 `eval_runs`/`eval_query_results`에 영속화한다. metric은 `eval aggregate <run_id>`가 산출하고 `eval_runs.aggregate_json`에 write한다. 즉 검색 품질 측정은 **run → aggregate 2단계**다.

산출물:
- `eval_runs` row (config_snapshot_json: chunker/embedding/llm/prompt/index version + fusion params) + `eval_query_results` rows.
- `runs_dir/<run_id>/per_query.jsonl`.
- `/build/dogfood/logs/eval-{hybrid,lexical}-v0.20.2.json` (aggregate JSON, dogfood 보관소 정책).

### 4.5 baseline + 회귀 기준

- 첫 v0.20.2 (run + aggregate) = **baseline**. 절대 임계값보다 **baseline 대비 회귀 감지**가 1차 기준이다(어떤 절대값이 "좋은지"는 corpus 의존적이라 단정 불가).
- 정성 sanity (golden 큐레이션 후 즉시 눈으로):
  - 한국어 2자 query("한국" 등)의 정답 chunk가 top-3 근처에 오는지.
  - 명백히 무관한 문서가 top에 올라오지 않는지.
  - lexical run에서 V009 형태소가 짧은 한국어 토큰을 0건으로 떨구지 않는지(`empty_result_rate` 확인).
- 미래 release: `kebab eval compare <baseline_run> <new_run>` (`compare.rs`)로 hit@k / MRR / recall delta. chunker_version이 다르면 기본은 doc-id fallback, `--strict-chunker-version`로 거부 가능.

### 4.6 정성 보조 검증

golden 외 ad-hoc query 몇 개의 top-k를 mode별(lexical / vector / hybrid)로 눈으로 확인해 ordering이 자연스러운지 본다. 이 결과를 `docs/DOGFOOD.md`의 검색 품질 시나리오 섹션에 추가한다:
- golden suite 실행 명령(§4.4)
- 정성 체크리스트(§4.5)
- mode별 ad-hoc top-k 관찰 기록 위치(`/build/dogfood/logs/`)

---

## 5. 운영 전제 / 알려진 제약

### 5.1 `kebab eval --config` thread — **Task A로 적용됨 (권장 운영 경로)**

`Cmd::Eval` dispatch(`main.rs:1340~`)는 패치 전 `cli.config`를 전혀 thread하지 않았다:
- `EvalWhat::Run` → `kebab_eval::run_eval(&opts)` 내부 `Config::load(None)`(`runner.rs:31`).
- `EvalWhat::Aggregate` → `compute_aggregate` / `store_aggregate` 내부 `Config::load(None)`(`metrics.rs:110, 138`).
- `EvalWhat::Compare` → `Config::load(None)` 직접 사용 (`main.rs:1399`). run / aggregate와 동일한 XDG 기본 config 한계.

**Task A (fix(cli): thread --config through kebab eval run/aggregate/compare)** 로 세 arm 모두 `run_eval_with_config` / `compute_aggregate_with_config` / `store_aggregate_with_config` / (기존) `compare_runs_with_config`로 교체됐다. 이는 CLAUDE.md facade-rule 정합 수정(P3-5 / P4-3와 동형)이다. 패치 적용 후 **권장 운영 경로는 §4.4 명령 블록 그대로** (`kebab --config /build/dogfood/config.toml eval run ...`).

패치 전 XDG 우회 경로(A: `XDG_CONFIG_HOME` + symlink, B: `KEBAB_STORAGE_DATA_DIR` override)는 **패치 전 fallback**으로만 의미가 있다. baseline run evidence(어느 경로로 dogfood KB를 평가했는지)는 HOTFIXES + release notes에 명시한다.

### 5.2 dogfood 보관소 정책 (CLAUDE.md)

- golden: `/build/dogfood/golden_queries.yaml` (신규).
- 결과 로그: `/build/dogfood/logs/eval-*.json`.
- config: `/build/dogfood/config.toml` (canonical dogfood config).
- `eval_runs` / `eval_query_results` / `runs_dir`: dogfood KB(`/build/dogfood/kb/`) 내부.

---

## 6. Scope

본 spec = **재사용 가능한 검색 품질 검증 인프라**(golden suite + 큐레이션 절차 + 실행/회귀 기준 + DOGFOOD 시나리오 편입). v0.20.2 baseline run이 첫 적용이다. golden suite는 이후 release마다 §4.5 절차로 회귀 감지에 재사용된다.

---

## 7. Testing

- **golden 로드 검증**: `KEBAB_EVAL_GOLDEN=/build/dogfood/golden_queries.yaml kebab eval run --mode lexical`이 bail 없이 시작 → expected_* 가 dogfood KB에 실재함을 증명(loader 계약).
- **2단계 동작 검증**: `eval run`이 `run_id` 출력 + `eval_runs` row 생성 → `eval aggregate <run_id>`가 `aggregate_json` 채움. `eval aggregate <run_id> --json`으로 metric 확인.
- **mode 분리 검증**: lexical run의 `citation_coverage` / `refusal_correctness`가 null(Answer 없음)임을 확인 → with_rag 없는 run의 의도된 동작.
- **정성 sanity**: §4.5 체크리스트를 baseline run 직후 수행, 결과를 HOTFIXES + DOGFOOD에 기록.
- 본 spec은 인프라/절차 문서이므로 새 단위 테스트를 추가하지 않는다 — 기존 `kebab-eval` 테스트(`metrics.rs` 18개 unit test)가 metric 계산을 커버한다.

---

## 8. 결과 기록 (cascade)

도그푸딩 evidence는 두 곳에:
1. `tasks/HOTFIXES.md` dated entry — golden suite 적용 + baseline metric 표(hit@k/MRR/recall mode별) + §5.1 config 경로 결정 + 정성 sanity 결과.
2. `docs/release-notes/v0.20.2-draft.md`(또는 gitea release body) — 검색 품질 검증 인프라 도입을 사용자 영향 관점으로 기술.

또한 `docs/DOGFOOD.md`에 검색 품질 시나리오 섹션 추가(§4.6).

---

## 9. References

- `crates/kebab-eval/src/types.rs` — `GoldenQuery` / `EvalRunOpts` 공개 surface.
- `crates/kebab-eval/src/metrics.rs` — `AggregateMetrics`(측정 metric 전체), `aggregate_from_rows`(`metrics.rs:184`), golden 경로 해석. `compute_aggregate_with_config`(`metrics.rs:116`), `store_aggregate_with_config`(`metrics.rs:144`).
- `crates/kebab-eval/src/runner.rs` — `run_eval`(`runner.rs:30`) / `run_eval_with_config`(`runner.rs:39`), query 실행, `SearchFilters::default()` 고정(`runner.rs:151`).
- `crates/kebab-cli/src/main.rs:402` — `EvalWhat`(run/aggregate/compare flag), `main.rs:441` `ModeFlag`, `main.rs:1340` Eval dispatch(Task A 패치로 `--config` thread 적용).
- `fixtures/golden_queries.yaml` — repo 템플릿(변경 대상 아님).
- CLAUDE.md — facade rule(`*_with_config`), 버전 cascade(§9), dogfood trigger / 보관소 정책.
- design §5.7 — `eval_runs` / `eval_query_results` 영속화 + config_snapshot.
- [[project_ranking_deferred]] — ranking 자동 튜닝 deferral.
- 형식 참조: `docs/superpowers/specs/2026-05-28-v0.20.2-dogfood-findings-design.md`.
