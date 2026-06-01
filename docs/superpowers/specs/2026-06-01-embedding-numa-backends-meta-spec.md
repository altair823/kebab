# Meta-Spec — NUMA-안전 임베딩 백엔드 (다중 트랙)

- 날짜: 2026-06-01
- 상태: DRAFT (umbrella)
- 범위: `kebab-embed-local` 및 임베더 주입 경로. 4개 트랙의 우산 스펙.
- 하위 산출물: 각 트랙은 본 메타스펙을 참조하는 자체 spec(`tasks/` 또는 `docs/superpowers/specs/`)과 plan을 가진다.

## 1. 문제

CPU-only Ollama 서버(Intel Xeon Silver 4214 ×2 소켓 = 48 logical, NUMA 2노드)에서 `kebab ingest` 가 매 실행 힙 손상으로 죽는다:

```
ingest [>     ] 3/5150  double free or corruption (!prev)
중지됨 (core dumped)
```

근본 원인(코드로 확정): fastembed 4.9.1 (`text_embedding/impl.rs:52,80`) 이 ONNX intra-op 스레드를 `available_parallelism()`(=48) 로 **하드코딩**하고 `InitOptions` 에 이를 덮어쓸 API 가 없다. 듀얼소켓 NUMA 에서 onnxruntime(`ort 2.0.0-rc.9`) 스레드풀이 힙을 손상시킨다. 진단 근거: `tasks/HOTFIXES.md` 의 본 날짜 entry + 대화 로그.

- 모델/디스크/AVX/데이터 문제 아님 (모델 2.08GB 정상, AVX-512 완비). 순수 스레드/NUMA × 네이티브 런타임 버그.
- onnxruntime 공식 문서도 듀얼소켓 NUMA 는 intra-op 스레드를 한 노드로 묶으라고 권고.

## 2. 목표 / 비목표

목표:
- 그 NUMA 서버에서 5150-doc 코퍼스를 **double-free 없이 완주**하는 임베딩 경로 확보.
- 검색 품질을 골든 스위트(MRR/hit@k) baseline 이상으로 유지.
- `models.embedding.provider` 로 선택 가능한 백엔드들로 구현 (기존 provider 필드 활용).

비목표:
- 랭킹 자동 조정 (별도 보류 결정, `[[project_ranking_deferred]]`).
- 임베딩 모델 품질 개선 자체 (NUMA 안정성이 본 과제의 초점).
- GPU 경로.

## 3. 공유 아키텍처

- 교체 지점은 **단일**: `crates/kebab-app/src/app.rs:836` 의 `FastembedEmbedder::new(&config)`.
- 트레이트 표면이 작다: `kebab_core::Embedder` (`traits.rs:127`) — `model_id / model_version / dimensions / embed`. 새 백엔드는 이 4개만 구현.
- 설정: `models.embedding.provider` (이미 존재), `model`, `version`, `dimensions`, `batch_size`. 신규로 트랙별 스레드/affinity 노브 추가 가능.

## 4. 횡단 정책 (모든 트랙 공통)

### 4.1 embedding_version & 재색인
- 벡터가 바뀌면(=candle, ollama) **`embedding_version` bump → 전체 재색인** (design §9 cascade). A2/A1 은 동일 onnxruntime e5-large 라 벡터 불변 → 재색인 불필요.
- 재색인 비용/절차를 각 트랙 spec 에 명시.

### 4.2 품질 검증 (필수 게이트)
- 벡터가 바뀌는 트랙은 머지 전 `kebab-eval` 골든 스위트(`/build/dogfood/golden_queries.yaml`) 로 MRR/hit@k 측정, **baseline 이상**이어야 default 승격. baseline 미달이면 opt-in provider 로만 유지.
- 패러프레이즈 robustness(#195/#196) 회귀 확인.

### 4.3 NUMA 서버 검증 (필수 게이트, 사용자 실행)
- **결정적 증거는 그 서버에서만 난다 (Claude 접근 불가).** 각 트랙은 사용자가 그 서버에서 5150-doc 코퍼스 ingest 를 **double-free 없이 완주(EXIT=0)** 함을 확인해야 "검증 완료".
- 각 트랙 spec 에 사용자-실행 검증 절차(명령 + 기대 출력)를 문서화.

### 4.4 스레드/NUMA 제어
- 각 백엔드가 intra-op/worker 스레드를 캡하고 한 NUMA 노드로 묶을 수 있어야 함. 캡 못 하면 트랙 실패.

## 5. 트랙

선호/구현 순서: **candle → ollama → A2 → A1**. (단 A1 은 무코드 stopgap 이라 즉시 문서화해 당장의 불통을 해소; 구현 순서와 별개.)

| # | 트랙 | 백엔드 | 벡터 변경(재색인) | 핵심 리스크 | 격리 브랜치 |
|---|------|--------|----|------|------|
| 1 | candle | 순수 Rust (candle `xlm_roberta`) | 예 | XLM-R padding_idx/패리티/CPU 성능 | `feat/embed-candle` |
| 2 | ollama | 별 프로세스 (Ollama `/api/embed`) | 예 | 모델이 e5 아님→품질, ingest 가 Ollama 의존 | `feat/embed-ollama` |
| 3 | A2 | onnxruntime 직접(`ort` 세션) | 아니오 | fastembed 우회 후 토크나이즈/풀링 재현 정확도 | `feat/embed-ort-direct` |
| 4 | A1 | onnxruntime + 실행 래핑(taskset/numactl) | 아니오 | 코드 변경 거의 없음, 문서/런처만 | `docs/embed-numa-affinity` |

### 5.1 트랙별 테스트 매트릭스 (각 트랙 spec 에서 구체화)

모든 트랙:
- 단위: `embed()` 가 올바른 dim/정규화(L2≈1) 벡터 반환.
- 통합: `kebab ingest` 소량 fixture → 청크/임베딩 카운트.
- **NUMA 서버 검증**(§4.3): 5150-doc 완주.

벡터-변경 트랙(candle/ollama) 추가:
- 패리티: onnxruntime e5-large 대비 동일 입력 cosine 유사도(가능 시) 또는 골든 스위트 동등성.
- 골든: MRR/hit@k ≥ baseline (§4.2).
- 재색인 절차 검증.

벡터-불변 트랙(A2/A1) 추가:
- 회귀: 기존 e5-large 벡터와 cosine ≈ 1.0 (A2 는 같은 런타임이라 사실상 동일해야).

## 6. 결정사항 (확정 2026-06-01)

- **D1 조기 종료 (사용자 확정)**: 트랙을 선호 순서로 진행하되, candle 또는 ollama 가 **허용 품질 기준 + NUMA 안전**을 만족하면 **거기서 멈춘다** (이후 트랙 미진행). 둘 다 품질이 너무 낮으면 A2 → A1 까지 계속.
  - **허용 품질 기준**: 골든 스위트 MRR/hit@k 가 현 e5-large(onnxruntime) baseline 대비 유의미한 회귀 없음. candle 은 동일 e5-large 가중치라 패리티 통과 시 이 기준을 거의 자동 충족 → candle 이 종착 가능성 높음. ollama 는 모델이 달라 경계선이면 사용자 판단.
  - A2/A1 은 candle·ollama 둘 다 실패 시의 **fallback** (A2 는 재색인 0 품질-중립).
- **D2 즉시 완화**: A1(taskset/numactl) 은 무코드라 본 작업과 무관하게 지금 바로 사용자에게 워크어라운드로 제공.
- **D3 메타 산출물 위치**: 본 메타스펙 + 메타플랜은 `docs/superpowers/specs/`. 트랙별 spec 은 도달 시 작성.
- **D4 frozen design 영향**: 임베딩 백엔드 다변화는 design §(임베딩) 갱신 가능 — 트랙 머지 시 동기화.

## 7. 성공 기준

- 그 NUMA 서버에서 최소 1개 트랙이 5150-doc 완주(EXIT=0).
- default 로 승격되는 백엔드는 골든 baseline 이상.
- 각 트랙이 자체 브랜치/워크트리 + 문서화된 테스트로 독립 검증.

## 8. 시퀀싱 게이트

1. candle **스파이크**(Phase 0) 가 패리티+CPU 성능+스레드 제어를 입증해야 candle 본 구현 진행. 실패 시 candle 트랙 강등/스킵 후 ollama 로.
2. 각 트랙은 PR open + NUMA 서버 검증 예약 후 다음 트랙 시작 (omc-teams sequential single-team 제약).
3. 벡터-변경 트랙은 골든 게이트 통과 전 default 승격 금지.
