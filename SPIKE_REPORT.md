# SPIKE REPORT — Track 1 / Phase 0 — candle multilingual-e5-large 타당성

- 날짜: 2026-06-01
- 워크트리: `/build/out/kebab-worktrees/embed-candle` (브랜치 `feat/embed-candle`)
- 목적: candle(순수 Rust)로 `intfloat/multilingual-e5-large` 를 돌려 기존 onnxruntime(`FastembedEmbedder`) 와 **수치 패리티**·**스레드 제어**·**CPU 성능**을 입증, candle 본 구현 진행 여부 판단.
- 머신: 12 logical CPU, 단일 소켓(비-NUMA). **결정적 NUMA 검증은 그 듀얼소켓 서버에서만 가능**(meta-spec §4.3) — 본 스파이크는 패리티·스레드캡·성능의 사전 입증.

> # VERDICT: **PASS** — candle 본 구현 진행 권고 (GREEN)
>
> 동일 e5-large 가중치로 onnxruntime 대비 **cosine min=mean=1.000000** (완전 일치). padding_idx/pooling 정확. `RAYON_NUM_THREADS` 로 CPU 스레드 캡 가능(NUMA 안전 전제 충족). latency 는 onnxruntime 대비 약 4배(67.5 vs 16.8 ms/문장, candle 4스레드 vs fastembed 12스레드) — 느리지만 ingest 배치에 허용 가능, 스레드 상향으로 개선 여지.

---

## 1. 접근 방식 (구현 사실)

격리 스파이크 바이너리 `crates/spike-embed-candle` 신설 (워크스페이스 멤버로 추가, candle 의존성은 이 crate 에만 — `candle-core/-nn/-transformers` 0.10.2, `hf-hub` 0.4, `tokenizers` 0.21). 프로덕션 crate(`kebab-embed-local` 등) 동작 변경 0.

- 모델 로드: `candle_transformers::models::xlm_roberta::{Config, XLMRobertaModel}`.
- 가중치: `intfloat/multilingual-e5-large` 의 `model.safetensors`(2.2GB) + `config.json` + `tokenizer.json` 을 `hf-hub` sync API 로 다운로드(`HF_HOME=/build/cache/huggingface`). fastembed 캐시는 ONNX 라 candle 이 못 읽으므로 safetensors 별도 수령. config.json 은 candle `Config`(serde) 로 직접 역직렬화 — hidden=1024, layers=24, heads=16, pad_token_id=1, max_pos=514, pos_emb=absolute (config 의 실제 로드 로그로 확인).
- 파이프라인 재현 (`kebab-embed-local` 규약과 동일): e5 프리픽스(`passage: `) → 토크나이즈(batch-longest 패딩, max_len=512, special tokens) → forward → **attention-mask 가중 mean pooling** → **L2 정규화**. 출력 ‖v‖=1.000000 확인.
- 패리티 비교: 동일 문장 10개(한/영 혼합)를 (a) candle 경로, (b) `kebab_embed_local::FastembedEmbedder`(`/build/dogfood/config.toml`, 모델 캐시 `/build/dogfood/kb/models`) 양쪽으로 임베딩. 양쪽 모두 `EmbeddingKind::Document`(`passage: ` 프리픽스).

## 2. 패리티 (caveat #1) — ✅ PASS (mean=1.000000)

| # | cosine | 문장(앞 40자) |
|---|--------|---------------|
| 0 | 1.000000 | The quick brown fox jumps over the lazy |
| 1 | 1.000000 | 오늘 날씨가 정말 좋아서 산책을 나가고 싶다. |
| 2 | 1.000000 | Rust is a systems programming language f |
| 3 | 1.000000 | 벡터 검색은 임베딩 사이의 코사인 유사도를 이용한다. |
| 4 | 1.000000 | Machine learning models require large am |
| 5 | 1.000000 | 한국어와 영어가 섞인 문장도 멀티링구얼 모델은 잘 처리한다. |
| 6 | 1.000000 | The capital of France is Paris, a city k |
| 7 | 1.000000 | 이 프로젝트는 로컬 우선 지식 베이스와 검색 증강 생성을 목표로 한다. |
| 8 | 1.000000 | Database indexing dramatically speeds up |
| 9 | 1.000000 | 임베딩 모델을 candle 로 옮기면 NUMA 서버에서 안전하게 돌릴 수 |

- **cosine min = 1.000000, mean = 1.000000** (합격선 mean≥0.99 GREEN 을 압도적 충족).
- 의미: candle 의 XLM-R forward + mean pooling + L2 가 onnxruntime e5-large 경로와 사실상 비트 단위로 동등. 본 구현으로 전환해도 **검색 품질(골든 MRR/hit@k) 회귀 없음**이 거의 보장됨 (meta-spec §6 D1 "candle 은 동일 가중치라 패리티 통과 시 품질 기준 자동 충족"과 일치). 단, meta-spec §4.2 골든 게이트는 본 구현 머지 전 별도 실측 권고.

## 3. padding_idx (caveat #2) — ✅ 정상 (소스 + 패리티 이중 확인)

candle-transformers 0.10.2 `xlm_roberta.rs` 의 `XLMRobertaEmbeddings::forward` 가 XLM-R 규약을 정확히 구현 (소스 확인):

```rust
let mask = input_ids.ne(self.padding_idx)?...;        // pad 아닌 위치 = 1
let cumsum = mask.cumsum(1)?;
let position_ids = (cumsum * mask)? + padding_idx;     // 위치 id 가 pad_token_id+1 부터
```

HF `create_position_ids_from_input_ids` 와 동일 (position id 가 `padding_idx(=1)` 다음부터 시작). config.json 의 `pad_token_id=1` 이 `Config.pad_token_id` 로 주입됨. **패리티가 1.000000 으로 나온 것이 padding_idx·pooling 의 정확성을 결정적으로 재확인** — 위치 임베딩이 한 칸이라도 어긋나면 cosine 이 1.0 이 될 수 없음.

## 4. 스레드 제어 (caveat #3) — ✅ 가능 (RAYON_NUM_THREADS)

| 항목 | 값 |
|---|---|
| `RAYON_NUM_THREADS` env | 4 |
| `rayon::current_num_threads()` | **4** |
| `available_parallelism()` | 12 |
| peak OS threads (`/proc/self/status`) | 16 |

- candle CPU 행렬연산(`gemm`)이 rayon 글로벌 풀을 사용 → `RAYON_NUM_THREADS=4` 로 **컴퓨트 스레드가 12→4 로 확실히 캡됨**. NUMA 안전(한 노드로 묶기)의 전제인 "스레드 수 제어 가능" 충족.
- 주의: peak 16 OS 스레드는 **패리티 비교를 위해 같은 프로세스에서 띄운 fastembed/onnxruntime 세션 스레드 + hf-hub 다운로드용 tokio 스레드**가 포함된 수치다. 실제 candle 전용 ingest 경로에는 fastembed 가 로드되지 않으며, candle 컴퓨트는 rayon 풀(=4)로 한정된다. 즉 **candle 백엔드는 fastembed 4.9.1 의 "48 하드코딩 + override 불가" 문제가 구조적으로 없다** (rayon 은 env/`ThreadPoolBuilder` 로 캡 가능).
- 다음 단계: 본 구현에서 `models.embedding` 에 스레드 노브(예: `KEBAB_EMBED_THREADS`→`RAYON_NUM_THREADS`/`ThreadPoolBuilder`)를 노출하고, NUMA 노드 바인딩은 `numactl`(A1 트랙)과 조합.

## 5. CPU latency (성능) — 허용 가능 (onnxruntime 대비 ~4×)

| 백엔드 | batch=32 wall-clock | ms/문장 | 스레드 |
|---|---|---|---|
| candle (release) | 2.161 s | 67.5 | 4 (RAYON cap) |
| fastembed (onnxruntime) | 0.536 s | 16.8 | 12 (이 머신) |

- candle 가 문장당 약 4배 느림. 단 **스레드가 1/3(4 vs 12)** 이고 fastembed 는 ORT 의 고도 최적화(MKL/AVX-512 커널)를 쓰는 반면 candle 은 순수 `gemm`. 스레드 상향·배치 튜닝 여지 있음.
- ingest 는 배치/백그라운드 작업이라 이 정도 latency 는 허용 가능. **NUMA 서버에서 "느리지만 완주" 가 "빠르지만 double-free 크래시" 보다 압도적으로 낫다** (본 과제의 핵심 동기).
- fastembed 모델 콜드 로드 86.9s (ORT 세션 init) 는 일회성. candle 모델 로드는 mmap 이라 즉시.

## 6. 막힌 점 / 리스크 / 다음 단계 권고

- **막힌 점**: 없음. 첫 빌드(candle+gemm) 2m24s, safetensors 2.2GB 다운로드 외 장애 없음.
- **리스크**:
  1. latency ~4×. 대용량(5150-doc) ingest 전체 시간이 늘어남 — 본 구현 시 wall-clock 실측 + release-notes 명시 필요.
  2. 본 스파이크는 비-NUMA 머신. **결정적 증거(5150-doc double-free 없이 EXIT=0)는 그 서버에서만**(meta-spec §4.3) — 본 구현 PR 후 사용자 실행 검증 예약.
  3. 벡터는 onnxruntime 와 1.0 일치하지만, 본 구현 시 `embedding_version` cascade 정책(재색인 여부) 명시 필요. 패리티 1.0 이면 **재색인 불필요 가능성**도 있으나(벡터 불변), 토크나이저/패딩 미세차 리스크로 보수적으로는 bump+재색인 권고 — 본 구현 spec 에서 결정.
- **다음 단계 권고 (candle 트랙 GREEN)**:
  1. `crates/kebab-embed-local` 에 `CandleEmbedder`(또는 신규 `kebab-embed-candle`) 추가, `Embedder` 4메서드 구현, `models.embedding.provider = "candle"` 분기.
  2. 스레드 노브 노출(`ThreadPoolBuilder`/`RAYON_NUM_THREADS`) + numactl 조합 문서화.
  3. `kebab-eval` 골든 스위트로 MRR/hit@k ≥ baseline 확인(§4.2) 후 default 승격 판단.
  4. 그 NUMA 서버에서 5150-doc 완주 검증(§4.3).

## 7. 재현 명령

```bash
cd /build/out/kebab-worktrees/embed-candle
# 빌드 (release, candle+gemm 첫 빌드 ~2.5분)
CARGO_TARGET_DIR=/build/out/cargo-target/target cargo build -j 4 --release -p spike-embed-candle
# 실행 (safetensors 2.2GB 첫 다운로드 + onnxruntime baseline 로드)
HF_HOME=/build/cache/huggingface RAYON_NUM_THREADS=4 \
  CARGO_TARGET_DIR=/build/out/cargo-target/target \
  /build/out/cargo-target/target/release/spike-embed-candle
```

## 8. 작업 로그

- 14:1x — worktree/모델캐시/config 확인. config.json: XLMRobertaModel, pad=1, vocab 250002, hidden 1024, 24 layers, max_pos 514.
- 14:1x — candle-transformers 0.10.2 `xlm_roberta` API 소스 확인 (Config serde, `XLMRobertaModel::{new,forward}`, `prepare_4d_attention_mask`, padding_idx 처리). 스파이크 crate 작성 + 워크스페이스 멤버 추가.
- 14:16 — release 빌드 백그라운드 시작.
- 14:18 — 빌드 완료 (2m24s, EXIT=0). 바이너리 실행 (RAYON_NUM_THREADS=4).
- 14:2x — 실행 완료 (EXIT=0). cosine min=mean=1.000000, rayon 캡=4, candle 2.161s vs fastembed 0.536s (batch=32). **VERDICT=PASS**.
