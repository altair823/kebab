# Plan: arctic-embed-l-v2.0 임베더 통합 구현

spec: `docs/superpowers/specs/2026-06-03-arctic-embedder-spec.md`. 브랜치 `feat/arctic-embedder`. 빌드 `CARGO_TARGET_DIR=/build/out/cargo-target`, `-j 4`(전체 test `-j 1`). cli 통합테스트용 `target` 심링크 필요 후 정리.

## Task 1 — kebab-embed-candle 모델 레지스트리
- e5 하드코딩(`HF_MODEL`/`SUPPORTED_MODEL`/mean pool/`query:`+`passage:`) → 레지스트리 구조체 `EmbedModelSpec { name, hf_repo, pooling: Pooling, query_prefix, doc_prefix, dim }`.
- 등록: e5(`intfloat/multilingual-e5-large`, Mean, `query: `/`passage: `, 1024) + arctic(`Snowflake/snowflake-arctic-embed-l-v2.0`, Cls, `query: `/``, 1024). **arctic pooling 은 모델 `1_Pooling/config.json` 로 확인 후 확정(CLS 추정).**
- `embed_batch` pooling 분기: Mean=기존 attention-mask-weighted, Cls=hidden_state[:,0,:]. tokenize/forward/L2 공유.
- `CandleEmbedder::new` 가 config model 로 spec 조회, 없으면 에러. `model_id`/`model_version` 에 모델명 반영.
- 단위테스트: 레지스트리 조회, prefix 적용, (가능하면) CLS vs mean pooling shape.

## Task 2 — kebab-embed-ollama 신규 크레이트
- `Cargo.toml`(workspace member), `Embedder` 구현. `reqwest::blocking` POST `/api/embed`.
- 배치(48) + fail-soft 재시도(3). query/doc prefix 모델별. L2 normalize. dim 검증(config 와 일치).
- endpoint = config.models.embedding.endpoint ?? models.llm.endpoint. model_version=`ollama:{model}`.
- 단위테스트: wiremock 으로 /api/embed mock → dim·정규화·prefix 검증.

## Task 3 — config + app 배선
- `kebab-config`: `EmbeddingCfg.provider` 문서/검증에 `ollama` 추가, `endpoint: Option<String>` 필드(serde default None). migrate.rs 주석.
- `kebab-app` `embedder()`(또는 해당 선택부, lib.rs ~836): provider match → fastembed | candle(레지스트리) | ollama. facade 통해 cfg 주입.
- config 직렬화/round-trip 테스트 갱신.

## Task 4 — correctness 검증 테스트 (핵심)
- candle arctic vs Ollama arctic 코사인>0.99 테스트: 테스트 문장 임베딩을 candle(arctic spec)로 1개 + Ollama(`snowflake-arctic-embed2` @192.168.0.47)로 1개 → cos>0.99 assert. live Ollama 의존이라 `#[ignore]`(이유: 외부 Ollama), 수동 실행 절차를 테스트 doc + HOTFIXES 에 기록. (CI 무인 환경 회피.)
- 단, **리더가 머지 전 이 테스트를 수동 실행해 통과 확인**(pooling/prefix 정확성 게이트).

## Task 5 — 검증 + 문서
- clippy 0 / 전체 test 통과(기존 e5 회귀 0).
- provider=candle+arctic, ollama+arctic, fastembed+e5(기본) 각 로드 스모크.
- 문서: README Configuration(provider candle/ollama + arctic + endpoint + metal), ARCHITECTURE(백엔드 그래프 + kebab-embed-ollama 크레이트 + 결정표), HANDOFF 1줄, HOTFIXES dated, Cargo.toml members + version minor bump(+Cargo.lock).

## 리뷰 루프
구현 완료 → 리더가 (a) clippy/test 독립 재확인 (b) **candle≈Ollama 코사인>0.99 수동 검증** → `gitea-pr`(title `feat(embed): arctic-embed-l-v2.0 임베더(candle+ollama)`) → 리뷰 루프 → 사용자 머지. 머지 후 Mac Metal 도그푸딩(recall 130 재현).
