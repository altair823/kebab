# Track 1 Spec — candle e5-large 임베딩 provider (NUMA-안전)

- 날짜: 2026-06-01
- 우산: [meta-spec](./2026-06-01-embedding-numa-backends-meta-spec.md) / [meta-plan](./2026-06-01-embedding-numa-backends-meta-plan.md)
- 선행: Phase 0 스파이크 PASS+독립검증 (cosine 1.000000, 스레드 캡 가능, latency ~4×). 커밋 76841af.
- 브랜치: `feat/embed-candle`

## 1. 목표

fastembed(onnxruntime) 의 "intra-op 스레드 48 하드코딩 → NUMA 힙 손상" 을 회피하기 위해, 동일 모델 `multilingual-e5-large` 를 **candle(순수 Rust)** 로 돌리는 임베딩 provider 를 추가한다. opt-in, 품질 중립, NUMA 스레드 캡 가능.

## 2. 확정 결정 (사용자 승인 2026-06-01)

- **D-reindex**: `embedding_version` **유지(재색인 0)** 를 목표. 구현 중 candle vs onnxruntime 벡터의 **차원별 max 절대오차**를 측정해 사실상 동일(예: max abs diff < 1e-5)함을 확인하고, 골든 스위트로 회귀 0 을 실측해 확정. 유의미한 차이가 나오면만 version bump + 재색인.
- **D-default**: 글로벌 default provider 는 **onnxruntime 유지**, candle 은 **opt-in** (`models.embedding.provider = "candle"`).
- **조기 종료**: candle 이 골든 baseline 충족 시 ollama/A2 트랙 생략 (A1 stopgap 문서만 별도).

## 3. 아키텍처

- **신규 crate `kebab-embed-candle`** — `kebab_core::Embedder` 구현. candle 의 큰 의존성 트리를 이 crate 에 격리.
  - 허용 deps: `candle-core`/`candle-nn`/`candle-transformers` (0.10.x), `tokenizers`, `hf-hub`, `kebab-core`, `kebab-config`, `anyhow`, `tracing`. **다른 `kebab-*` 의존 금지**(core/config 외) — design §8 경계.
- **주입 분기**: `kebab-app/src/app.rs` 의 `embedder()` (현 :829-837, `FastembedEmbedder::new` 무조건 생성) 를 `config.models.embedding.provider` 로 분기:
  - `"fastembed"` | `"onnx"` | (빈값/기존) → `FastembedEmbedder` (default, 기존 동작 유지).
  - `"candle"` → `CandleEmbedder`.
  - 알 수 없는 값 → 명확한 에러.
- **facade 규칙 준수**: UI crate 는 `kebab-app` 만. `kebab-app` 이 `kebab-embed-candle` 의존 추가.

## 4. CandleEmbedder 동작 (스파이크에서 검증된 파이프라인)

- 모델: `intfloat/multilingual-e5-large` 의 `model.safetensors` + `config.json` + `tokenizer.json` 을 `hf-hub` 으로 `{model_dir}/candle/` (config `storage.model_dir`) 아래에 캐시.
- `candle_transformers::models::xlm_roberta::{Config, XLMRobertaModel}` 로 로드 (CPU `Device::Cpu`).
- `embed()`: e5 프리픽스(`query: `/`passage: `, `EmbeddingInput` kind 기준 — `kebab-embed-local` 의 `prefix_input` 규약과 동일) → 토크나이즈(max_len 512, batch-longest 패딩, special tokens) → forward → **attention-mask 가중 mean pooling** → **L2 정규화**.
- `dimensions()` = 1024, `model_id`/`model_version` = config 값(기존과 동일 식별자 유지).
- **스레드 캡**: config 신규 필드 `models.embedding.num_threads`(u32, 0=auto) + env `KEBAB_EMBED_THREADS`. `CandleEmbedder::new` 에서 `rayon::ThreadPoolBuilder::new().num_threads(n).build_global()` 1회 적용(이미 초기화 시 무시). 0/auto 면 미설정(rayon 기본). NUMA 노드 바인딩은 `numactl`(A1) 과 조합 — 문서화.
- `Mutex<XLMRobertaModel>` 또는 forward 가 `&self` 면 불필요 — candle forward 는 `&self` 가능, 단 내부 가변 없으면 `Send+Sync` 보장 확인.

## 5. config 변경

- `EmbeddingModelCfg` 에 `num_threads: u32`(default 0) 추가. env `KEBAB_EMBED_THREADS`.
- `provider` 허용값 문서화: `fastembed`(default)/`candle`.
- default toml + `Config::default()` 갱신, 기존 테스트 영향 확인.

## 6. 버전/캐스케이드

- D-reindex 에 따라 `embedding_version` 유지 (벡터 동일). cascade(design §9) 트리거 안 함 — 기존 색인 재사용. (max abs diff 확인 실패 시에만 bump.)
- wire schema 변경 없음.

## 7. 테스트 (산출물)

- **단위**(`kebab-embed-candle`): `dimensions()==1024`; `embed()` 출력 L2≈1; 빈 입력 빈 출력; 프리픽스 적용 확인.
- **패리티 테스트**(`#[ignore]`, 모델 2GB+네트워크 필요): candle vs `FastembedEmbedder` 동일 문장 cosine ≥ 0.9999 + max abs diff 보고. CI 기본 제외, 수동/도그푸딩에서 실행.
- **통합**(`kebab-cli` 또는 `kebab-app`): `provider="candle"` 로 소량 fixture ingest → 청크/임베딩 카운트 > 0, 검색 1건 성공. (모델 필요 → `#[ignore]` 또는 feature.)
- **스레드 캡**: `num_threads=4` 설정 시 `rayon::current_num_threads()==4` 확인.
- **회귀**: 기존 fastembed 경로 default 동작 불변(provider 미지정 시).
- clippy `-D warnings`, 빌드 직렬 `-j 4`.

## 8. 품질 게이트 (머지 전)

- `kebab-eval` 골든 스위트(`/build/dogfood/golden_queries.yaml`) 를 provider=candle 로 실행 → MRR/hit@k ≥ 현 baseline (회귀 0). [[feedback_search_quality_dogfood]]
- 패러프레이즈 robustness(#195/#196) 스폿 확인.

## 9. 문서/릴리스 (머지 시 동일 PR)

- README: Configuration 에 `provider=candle` + `num_threads`/`KEBAB_EMBED_THREADS` 추가. SMOKE config 예시 동기화. [[feedback_readme_sync_rule]]
- ARCHITECTURE: crate 그래프 + 디렉터리에 `kebab-embed-candle` 추가.
- HANDOFF: 머지 후 한 줄(임베딩 백엔드 다변화).
- HOTFIXES: 본 날짜 dated entry (NUMA double-free 진단 + candle provider 도입 + 스파이크 패리티 증거).
- 버전 bump: 신규 config surface(provider=candle, num_threads) = pre-1.0 minor bump (0.21.1 → 0.22.0), release notes.

## 10. 범위 밖 / 후속

- candle crate feature-gate 로 빌드 비용 격리 (후속).
- NUMA 노드 자동 바인딩(현재는 numactl 외부 조합).
- ollama/A2/A1 트랙 (candle 게이트 통과 시 생략).

## 11. 잔여 게이트 (사용자 실행, Claude 불가)

- 그 듀얼소켓 NUMA 서버에서 `provider=candle` 로 5150-doc ingest **double-free 없이 EXIT=0 완주**. PR 머지 전/후 검증 예약. (meta-spec §4.3)
