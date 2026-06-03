# Spec: arctic-embed-l-v2.0 임베더 통합 (candle 우선 + Ollama provider)

**날짜**: 2026-06-03
**유형**: feature (신규 임베딩 백엔드/모델)
**근거**: `docs/superpowers/research/2026-06-03-expansion-cost-rethink-research.md` + `/build/dogfood/logs/2026-06-03-method-measurements.md`. 별칭 제거(v0.25.0) 후 설명형 recall 보강의 최선책. 측정: arctic-embed2 = recall@10 **130/132**, recall@50 **132/132**, **용어 무손실**(bge-m3 와 달리 syn/abbr/en 유지). e5 대비 +7, 색인 1회·per-query 0·LLM 0 = 살아있는 KB 최적합.
**사용자 결정**: candle 우선 + Ollama embed provider 폴백 둘 다.

## 목표
`models.embedding` 에서 arctic-embed-l-v2.0 을 선택 가능하게 한다. 두 백엔드:
1. **candle** (주): `kebab-embed-candle` 를 e5 전용 → 다중 모델로 일반화, arctic 추가. in-process pure-Rust, NUMA 안전.
2. **ollama** (폴백): 신규 Ollama embedding provider. 측정에 쓴 경로(`/api/embed`) 그대로 → 130 보장.
기본 동작 불변(기본 provider=fastembed e5). arctic 은 opt-in.

## 모델 사실 (구현 기준)
- 아키텍처: **XLM-RoBERTa-large** (candle `XLMRobertaModel` 로드 가능, e5 와 동일 계열).
- dim: **1024** (e5 와 동일 → 벡터스토어/lancedb 테이블 차원 불변, 단 테이블명은 모델명 포함).
- pooling: arctic-embed-l-v2.0 의 sentence-transformers `1_Pooling/config.json` 기준(**CLS 토큰 추정 — 반드시 config 로 확인**). e5 는 mean pooling → pooling 을 모델별 분기.
- prefix: **query 에 `query: ` 접두어, 문서는 무접두어**(e5 의 `query:`/`passage:` 와 다름). 모델별 분기.
- 정규화: L2 normalize (코사인 일관성, 기존 e5 경로와 동일).
- HF repo: `Snowflake/snowflake-arctic-embed-l-v2.0` (candle 다운로드). Ollama: `snowflake-arctic-embed2`.

## 작업 A — kebab-embed-candle 다중 모델화
- 현재 `HF_MODEL`/`SUPPORTED_MODEL` 상수(e5 하드코딩) → **모델 레지스트리**로: `{ name, hf_repo, pooling: Mean|Cls, query_prefix, doc_prefix, dim }`. e5(mean, `query: `/`passage: `) + arctic(cls, `query: `/``).
- `embed_batch` 의 pooling 단계를 모델별 분기(mean=attention-mask-weighted mean / cls=first token). 나머지(tokenize→forward→L2)는 공유.
- `model_id()` / `model_version()` 가 모델명+pooling 반영(전환 시 embedding_version cascade 트리거).
- config `models.embedding.model` 이 레지스트리에 없으면 기존처럼 명확한 에러.
- `[features] metal`/`mkl` 유지(arctic 도 동일 XLM-R 경로라 그대로 동작).

## 작업 B — Ollama embedding provider (신규)
- 신규 크레이트 `kebab-embed-ollama` (또는 kebab-embed-local 내 모듈 — **새 크레이트 권장**, 의존 분리). `Embedder` trait 구현.
- `reqwest::blocking` 으로 `POST {endpoint}/api/embed` `{model, input:[...]}` → `embeddings`. 배치(예: 48/req), fail-soft 재시도.
- query/doc prefix 모델별(arctic: query 에 `query: `). 결과 **L2 normalize**(Ollama raw 반환 → 일관성 위해 정규화).
- endpoint: `models.embedding.endpoint`(신규, 미설정 시 models.llm.endpoint fallback). model_version = `ollama:{model}`.

## 작업 C — config + app 배선
- `kebab-config`: `EmbeddingCfg.provider` 에 `"ollama"` 허용. 신규 `endpoint: Option<String>`(ollama 용). serde forward-compat 유지.
- `kebab-app`: embedder 선택 분기(`embedder()`)에 candle 다중모델 + ollama provider 추가. facade(`*_with_config`) 통해 config 주입(facade rule 준수).
- UI 크레이트는 kebab-app 만 touch(불변).

## 결정 사항
- **차원 1024 동일** → lancedb 테이블은 모델명 포함(`chunk_embeddings_{model}_{dim}`)이라 모델 전환 시 새 테이블, 충돌 없음.
- **embedding_version cascade**: arctic 으로 전환 = embedding_version 변경 → 전체 재임베딩 필요(breaking). 기존 e5 KB 와 혼용 불가(명확). 기본값 e5 유지라 기존 사용자 무영향.
- arctic **ko 파인튠(dragonkue)** 은 base(130) 로 충분 → 본 작업은 base. ko 는 후속 옵션(레지스트리에 추가만 하면 됨).
- A(heading enrichment) 는 측정상 arctic 에서 악화 → **미적용**.

## 검증 기준 (Acceptance)
- `cargo clippy --workspace --all-targets -j 4 -- -D warnings` 통과.
- `cargo test --workspace --no-fail-fast -j 1` 통과 — 기존 e5-candle/fastembed 테스트 회귀 0.
- **correctness 핵심**: candle arctic 으로 임베딩한 테스트 문장(예: `query: 스택 자료구조` + 문서 `후입선출 자료구조`)이 **Ollama `snowflake-arctic-embed2` 임베딩과 코사인 > 0.99 일치**(Ollama 192.168.0.47 도달 가능 — pooling/prefix 정확성 정밀 검증, 130 재현 위험 차단). live Ollama 없으면 `#[ignore]` + 수동 절차 문서화.
- ollama provider: mock 또는 live 로 dim 1024 정규화 벡터 반환 smoke.
- config provider=`candle`+arctic / `ollama`+arctic 각각 올바른 embedder 로드.
- 기본 provider=fastembed e5 동작 불변(스모크).

## 도그푸딩 (별도, Mac Metal — 본 PR acceptance 아님)
arctic 으로 namu 재임베딩 → `namu_golden_expanded.yaml` 로 recall@10 ≈ 130 재현 확인. CLAUDE.md §Dogfood trigger(embedder 모델 변경) 충족. 결과 HOTFIXES + release notes.

## 문서 동기화 (같은 PR)
- README Configuration: provider=candle/ollama + arctic 모델 + endpoint + Apple Silicon(metal) 안내.
- docs/ARCHITECTURE: 임베딩 백엔드 그래프 + 신규 크레이트(kebab-embed-ollama) + 결정 표(arctic 채택 근거 측정 링크).
- HANDOFF 1줄. tasks/HOTFIXES dated entry(측정 근거 + cascade).
- Cargo.toml workspace members += kebab-embed-ollama, version minor bump.

## 비범위
- e5 KB 자동 마이그레이션(전환 = 수동 재임베딩, cascade 규칙대로).
- dragonkue ko 파인튠(후속).
- D(query-side)·C(reranker) 통합(별도 후속, 본 PR 은 임베더만).
