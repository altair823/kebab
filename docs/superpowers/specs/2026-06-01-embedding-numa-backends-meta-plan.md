# Meta-Plan — NUMA-안전 임베딩 백엔드 실행 계획

- 날짜: 2026-06-01
- 우산 스펙: [2026-06-01-embedding-numa-backends-meta-spec.md](./2026-06-01-embedding-numa-backends-meta-spec.md)
- 실행 모델: 트랙별 worktree 격리 + omc teammate (omc-teams, sequential single-team). 트랙 내 단계는 spec → plan → 구현 → 테스트 → PR.

## 0. 즉시 (본 계획과 병행, 무코드)

- **A1 stopgap 문서화 + 사용자 제공**: `numactl --cpunodebind=0 --membind=0 kebab ingest` (또는 `taskset -c 0-11`). 현재 불통 해소용. 이건 트랙 4의 산출물 일부지만 지금 바로 안내.
- 사용자 NUMA 서버에서 A1 로 5150-doc 완주되는지 1회 확인 → "스레드/NUMA 가 원인" 인과 확정(메타스펙 §1 보강).

## 1. 트랙 실행 순서 & 게이트

`candle → ollama → A2 → A1(정식 문서화)`. 한 트랙의 PR open + NUMA 검증 예약 전까지 다음 트랙 미착수.

**조기 종료 (D1 확정)**: candle 또는 ollama 가 허용 품질(골든 ≥ baseline 무회귀) + NUMA 안전을 만족하면 **거기서 종료**, 이후 트랙 미진행. 둘 다 품질 미달 시에만 A2 → A1 진행. candle 은 동일 e5-large 라 패리티 통과 시 종착 유력.

### 트랙 1 — candle (`feat/embed-candle`)

- **Phase 0 — 타당성 스파이크 (게이트, 최우선)**
  - worktree 에서 candle + candle-transformers 의존성 추가, `xlm_roberta::XLMRobertaModel` 로 `intfloat/multilingual-e5-large` safetensors 로드 (CPU).
  - 몇 개 문장 임베딩 → (a) onnxruntime e5-large 벡터와 cosine 패리티, (b) CPU latency, (c) `RAYON_NUM_THREADS` 로 스레드 캡 동작, (d) padding_idx 위치 임베딩 정확성.
  - 산출: 스파이크 리포트(패리티 수치 + latency + 스레드 제어 확인). **통과해야 Phase 1 진행.**
- **Phase 1 — spec**: 트랙 spec 작성 (Embedder 구현, config provider="candle", embedding_version, 재색인 절차, 테스트 매트릭스).
- **Phase 2 — plan**: 구현 plan.
- **Phase 3 — 구현**: `kebab-embed-candle`(신규 crate) 또는 `kebab-embed-local` 내 provider 분기. Embedder 구현 + app.rs 주입 분기 + config.
- **Phase 4 — 테스트**: 단위/통합 + 패리티 + 골든. 빌드는 직렬 `-j 4`.
- **Phase 5 — PR + 검증**: gitea PR. 사용자 NUMA 서버 5150-doc 완주 + 골든 baseline 확인.

### 트랙 2 — ollama (`feat/embed-ollama`)

- spec → plan → 구현(`OllamaEmbedder`: `/api/embed` 호출, provider="ollama", 모델 선택[e5 GGUF 또는 bge-m3]) → 테스트(패리티/골든, 프로세스 격리로 double-free 부재) → PR + NUMA 검증.

### 트랙 3 — A2 (`feat/embed-ort-direct`)

- spec → plan → 구현(fastembed 우회, `ort` 세션 직접 + `with_intra_threads(N)` + NUMA affinity, 토크나이즈/mean-pool/L2 재현, provider="onnx" 기본 유지) → 테스트(기존 e5 벡터와 cosine≈1.0, 재색인 0) → PR + NUMA 검증.
- **품질-중립 안전망**: 재색인 없이 즉시 default 가능.

### 트랙 4 — A1 정식화 (`docs/embed-numa-affinity`)

- 런처 래핑/문서 + (선택) config 노브로 affinity 힌트. README/SMOKE/HOTFIXES 동기화.

## 2. omc teammate 운용 (메모리 규약 준수)

- spawn: omc-teams tmux pane + brief 파일. **sequential single-team** (multi-team 동시 spawn 금지).
- 모델 라우팅: executor + initial draft + round-1 review = **opus**; closure verify / micro-patch round = **sonnet**. (`OMC_TEAM_ROLE_OVERRIDES` env)
- worker spawn 직후 completion polling shell `run_in_background=true` (phase=completed/failed 감지 → main session 자동 알림).
- 빌드/테스트 직렬, `-j 4` 기본. `CARGO_TARGET_DIR=/build` 사용 (routinely clean 금지).

## 3. 워크트리 / 브랜치

| 트랙 | 브랜치 | worktree |
|---|---|---|
| 1 candle | `feat/embed-candle` | 신규 |
| 2 ollama | `feat/embed-ollama` | 신규 |
| 3 A2 | `feat/embed-ort-direct` | 신규 |
| 4 A1 | `docs/embed-numa-affinity` | 신규 |

각 트랙 머지 후 다음 트랙 rebase. 트랙 간 공유 상태 없음(독립 provider).

## 4. 리스크 레지스터

- candle Phase 0 패리티 실패 → 트랙 1 강등, ollama 우선.
- candle CPU latency 가 onnxruntime 대비 과도 → opt-in provider 로만.
- ollama 모델이 e5 아님 → 골든 회귀 가능 → default 승격 보류.
- NUMA 검증이 사용자 가용성에 의존 → 각 PR 은 검증 전까지 "merge-pending".
- ort rc.9 자체 버그가 A2 에서도 재현 가능성 → A2 스레드 캡으로도 안 죽는지 NUMA 검증 필수.

## 5. 진행 상태 (라이브)

- [x] candle 타당성 desk-research (xlm_roberta 모듈 존재 + cembedd 선례) — 2026-06-01
- [ ] A1 stopgap 사용자 NUMA 서버 확인
- [x] 트랙 1 Phase 0 스파이크 — **VERDICT=PASS** (2026-06-01). cosine min=mean=1.000000(onnxruntime 동일), RAYON 스레드 캡 가능, latency ~4×(67.5 vs 16.8 ms/문장, 4 vs 12 스레드). 커밋 76841af. → **조기 종료 유력**: candle 이 품질 baseline 자동 충족 → ollama/A2/A1 불필요 전망. 잔여 게이트=골든 실측 + NUMA 서버 5150-doc 완주.
- [ ] 트랙 1 spec/plan/impl/test/PR (진행)
- [ ] 트랙 2 …
- [ ] 트랙 3 …
- [ ] 트랙 4 …
