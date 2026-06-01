# Track 1 / Phase 0 — candle e5-large 타당성 스파이크 (BRIEF)

너는 이 worktree(`/build/out/kebab-worktrees/embed-candle`, 브랜치 `feat/embed-candle`)에서 작업하는 executor 다.
이건 **타당성 검증 스파이크**다 — 프로덕션 코드가 아니라, candle 트랙을 본격 구현해도 되는지 판단할 증거를 모으는 게 목적이다. 깔끔함보다 **정확한 증거**가 우선.

## 배경 (왜)

CPU-only 듀얼소켓 NUMA 서버에서 `kebab ingest` 가 매번 `double free or corruption (!prev)` 로 죽는다.
근본 원인: fastembed 4.9.1 이 onnxruntime intra-op 스레드를 전체 CPU(48)로 하드코딩하고 override 불가 → NUMA 에서 힙 손상.
해법 후보 1순위 = **candle(순수 Rust)로 동일 모델 multilingual-e5-large 를 돌리기**. candle-transformers 에 `xlm_roberta` 모듈이 있고 e5-large 는 XLM-RoBERTa-large 구조라 가능성 확인됨. 이 스파이크가 그 가능성을 **수치로 입증**해야 한다.

전체 맥락: `/home/altair823/kebab/docs/superpowers/specs/2026-06-01-embedding-numa-backends-meta-spec.md` 및 `-meta-plan.md`.

## 검증해야 할 caveat 3 + 성능 1

1. **수치 패리티**: candle 출력 벡터가 기존 onnxruntime(fastembed) e5-large 와 사실상 동일한가 (같은 가중치니 cosine ≥ 0.99 이어야 정상; 낮으면 padding/pooling 버그).
2. **padding_idx 위치 임베딩**: XLM-R 은 position id 가 `padding_idx(=1)+1` 부터 시작. candle `xlm_roberta` 가 이를 맞게 처리하는지 (패리티가 높으면 간접 입증).
3. **스레드 제어**: candle CPU 스레드를 캡할 수 있는가 (`RAYON_NUM_THREADS` 또는 candle API). NUMA 안전의 전제.
4. **CPU 성능**: 배치 임베딩 latency 를 측정. onnxruntime 대비 대략 비교.

## 구체 작업

이 worktree 안에서 **격리된 스파이크 바이너리**를 만들어라 (프로덕션 crate 의 기본 동작 변경 금지). 예: 새 example 또는 작은 `xtask`/bin. candle 의존성(candle-core, candle-nn, candle-transformers, tokenizers, hf-hub, safetensors)은 스파이크 대상에만 추가.

스파이크가 할 일:
1. **모델 로드 (candle, CPU)**: `intfloat/multilingual-e5-large` 의 safetensors + config.json + tokenizer.json 을 hf-hub 으로 받아 `candle_transformers::models::xlm_roberta::XLMRobertaModel` 로 로드. (참고: 이 머신의 fastembed 캐시는 ONNX 라 candle 이 못 읽는다. tokenizer.json/config.json 은 `/build/dogfood/kb/models/fastembed/models--Qdrant--multilingual-e5-large-onnx/snapshots/*/` 에서 재사용 가능.)
2. **임베딩 파이프라인 재현**: 입력에 e5 프리픽스(`query: ` / `passage: `) 적용 → 토크나이즈 → forward → **attention-mask 가중 mean pooling** → **L2 정규화**. (kebab 의 `crates/kebab-embed-local/src/lib.rs` 의 prefix/정규화 규약 참고.)
3. **패리티 비교**: 동일 문장 집합(한국어/영어 혼합, 최소 8개)을 (a) 위 candle 경로, (b) 기존 `kebab_embed_local::FastembedEmbedder`(워크스페이스에 이미 있음) 양쪽으로 임베딩 → 문장별 cosine 유사도. min/mean 보고. FastembedEmbedder 는 `/build/dogfood/config.toml` 또는 적절한 Config 로 생성(모델 캐시 `/build/dogfood/kb/models`).
4. **스레드 제어 확인**: `RAYON_NUM_THREADS=4` 등으로 실제 스레드 수가 제한되는지 확인(예: 실행 중 thread 수 또는 latency 변화).
5. **latency 측정**: 배치(예: 32문장) 임베딩 wall-clock.

## 제약 (반드시 준수)

- `CARGO_TARGET_DIR=/build/out/cargo-target/target` (루트 디스크 보호). 빌드 직렬, `-j 4`. candle 첫 빌드는 무거우니 `cargo build` 는 `run_in_background` 로.
- 프로덕션 crate(`kebab-embed-local` 등)의 기존 동작/기본값 변경 금지. 스파이크는 추가만.
- 네트워크: HuggingFace 접근 가능(이 머신은 됨). safetensors 다운로드는 `/build/cache/` 하위로.
- RAM 30GB, OOM 주의. 배치 작게.

## 산출물 (필수)

`/build/out/kebab-worktrees/embed-candle/SPIKE_REPORT.md` 에 다음을 적어라:
- **VERDICT**: PASS / FAIL (candle 본 구현 진행 권고 여부).
- 패리티: 문장별 cosine min/mean (표).
- padding_idx: 정상 여부 + 근거.
- 스레드 제어: 가능 여부 + 방법.
- latency: 배치 측정값 + onnxruntime 대략 대비.
- 막힌 점 / 리스크 / 다음 단계 권고.
- 재현 명령(스파이크 빌드+실행 커맨드).

작업 로그는 수시로 `SPIKE_REPORT.md` 에 누적. 완료되면 변경을 `feat/embed-candle` 에 커밋(스파이크 코드 + 리포트). 커밋 메시지 끝에 `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`.

## 합격 기준

- cosine 패리티 mean ≥ 0.99 (동일 가중치) → padding/pooling 정확, candle 트랙 GREEN.
- 0.95~0.99 → 경미한 차이(pooling 옵션 등), 진단 후 판단.
- < 0.95 → 구조/패딩 불일치 → 원인 규명 후 FAIL 또는 수정.
- 스레드 캡 불가 시 NUMA 안전성 위협 → 리포트에 명시.
