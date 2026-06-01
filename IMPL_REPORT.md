# Track 1 / Phase 1 — candle 임베딩 provider 구현 보고서

- 날짜: 2026-06-01
- 브랜치: `feat/embed-candle` (worktree `/build/out/kebab-worktrees/embed-candle`)
- 스펙: `docs/superpowers/specs/2026-06-01-embed-candle-track-spec.md`
- 버전: 0.21.1 → **0.22.0**

## 1. 변경 요약

| 영역 | 변경 |
|------|------|
| 신규 crate | `crates/kebab-embed-candle` — `CandleEmbedder` (`kebab_core::Embedder` impl). 스파이크 파이프라인 흡수: safetensors via hf-hub → `XLMRobertaModel` forward(`Device::Cpu`) → attention-mask mean pooling → L2 → e5 prefix(`passage:`/`query:`). 모델 캐시 `{model_dir}/candle/`. deps = candle-core/nn/transformers 0.10.2, tokenizers, hf-hub, serde_json, rayon, anyhow, tracing + `kebab-core`/`kebab-config` 만 (design §8 경계 준수). |
| 스레드 캡 | `[models.embedding].num_threads: u32`(default 0=auto) + env `KEBAB_EMBED_THREADS`(우선). `apply_thread_cap()` 가 글로벌 rayon 풀 1회 캡 (이미 init 시 no-op). |
| 주입 분기 | `kebab-app::App::embedder()` 가 `provider` 분기 — `fastembed`/`onnx`/`""` → 기존 `FastembedEmbedder`(불변), `candle` → `CandleEmbedder`, 미지값 → 에러. `none` 은 기존 lexical-only. `kebab-app/Cargo.toml` 에 dep 추가. |
| config | `EmbeddingModelCfg.num_threads`(`#[serde(default)]` — 옛 config 호환) + `KEBAB_MODELS_EMBEDDING_NUM_THREADS` env + `Config::defaults()`. |
| 스파이크 제거 | `crates/spike-embed-candle` 삭제 + 워크스페이스 멤버 제거 + `spike_build.log`/`spike_run.log` 정리. |
| 문서/버전 | README Configuration, `docs/SMOKE.md` config 예시, `docs/ARCHITECTURE.md`(crate 그래프+트리), HANDOFF 한 줄, `tasks/HOTFIXES.md` 2026-06-01 dated entry, workspace `version` 0.22.0, `docs/release-notes/v0.22.0-draft.md`. |

## 2. 검증 게이트 결과 (모두 파일 출력 + exit code 로 검증)

> ⚠️ 주의: background shell 의 notification "exit 0" 은 wrapper 의 종료코드라
> 신뢰 불가. 실제 결과는 각 로그의 `*_EXIT=` 라인 값으로 확정했다
> ([[project_rerank_experiment]] 교훈). 실제로 첫 빌드는 wrapper 가 exit 0 을
> 보고했지만 로그의 `BUILD_EXIT=101`(serde_json 미선언)이었고, dep 추가 후 통과.

| 게이트 | 명령 | 결과 | 로그 |
|--------|------|------|------|
| 빌드 + clippy | `cargo clippy --workspace --all-targets -j 4 -- -D warnings` | **`CLIPPY_EXIT=0`**, warning 0 | `clippy.log` |
| 단위/통합 테스트 | `cargo test -p kebab-embed-candle -p kebab-config -j 4` | **`TEST_EXIT=0`** — candle lib unit 5, `thread_cap` 1 passed(rayon current=4 검증), config 68 passed, parity 1 ignored | `test_units.log` |
| config 회귀 | (위 동일 run, `kebab-config` 68 tests) | 0 failed | `test_units.log` |
| 패리티 `#[ignore]` 수동 1회 | `cargo test -p kebab-embed-candle --release -- --ignored --nocapture` | **`PARITY_EXIT=0`**, 1 passed (32.53s) | `test_parity.log` |

## 3. 패리티 수치 (재색인 결정 근거 — 스펙 D-reindex)

10 문장(한/영 혼합) candle vs `FastembedEmbedder`(onnxruntime):

```
PARITY_SUMMARY cosine_min=1.000000 max_abs_diff=2.011657e-7
```

- 코사인 최소 **1.000000** (≥ 0.9999 게이트 통과).
- 차원별 **max 절대오차 = 2.01e-7** — 스펙이 정한 "사실상 동일" 기준
  (max abs diff < 1e-5) 보다 **약 50배 작다**.
- **결론: `embedding_version` 유지 = 재색인 0.** candle 과 onnxruntime 의
  벡터는 f32 반올림 수준에서만 다르며 (e-7), 기존 LanceDB 색인을 그대로
  재사용해도 검색 결과가 바뀌지 않는다. version bump / cascade 트리거 안 함.

## 4. 잔여 리스크

- **CPU latency**: candle 는 순수 Rust 라 onnxruntime 의 네이티브 커널보다
  느리다 (Phase 0 스파이크 ~4×). 그래서 default 는 fastembed 유지, candle 은
  NUMA 환경 opt-in. 단일 워크스테이션 사용자에게는 권하지 않음 (README 명시).
- **모델 다운로드**: candle 은 `{model_dir}/candle/` 에 safetensors(~2GB)를
  별도 캐시 (onnx 캐시와 공유 안 함). 첫 ingest 시 ~2GB 다운로드 발생.
- **잔여 게이트 (사용자 실행, Claude 불가, meta-spec §4.3)**: 그 듀얼소켓
  NUMA 서버에서 `provider=candle` 로 5150-doc ingest 가 double-free 없이
  EXIT=0 완주하는지 — 이 머신은 GPU/NUMA 없는 단일 VM 이라 재현 불가. PR
  머지 전/후 사용자 검증 예약.
- **골든 스위트 회귀 0 (스펙 §8)**: provider=candle 로 `kebab-eval` 골든
  스위트 실행은 본 worktree 범위 밖(사용자 도그푸딩 단계). 패리티 e-7 로
  벡터 동일성이 입증돼 회귀 위험은 낮음.

## 5. 재현 명령

```bash
cd /build/out/kebab-worktrees/embed-candle
export CARGO_TARGET_DIR=/build/out/cargo-target/target

# 빌드 + clippy (warning 0)
cargo clippy --workspace --all-targets -j 4 -- -D warnings

# 단위 + config 회귀
cargo test -p kebab-embed-candle -p kebab-config -j 4

# 패리티 (모델 ~2GB 다운로드 + 양쪽 추론, /build/dogfood/config.toml 필요)
cargo test -p kebab-embed-candle --release -j 4 -- --ignored --nocapture
# → PARITY_SUMMARY cosine_min=1.000000 max_abs_diff=2.011657e-7

# candle provider 로 ingest (사용자 NUMA 검증)
KEBAB_EMBED_THREADS=8 kebab ingest --config /path/to/candle-config.toml
```

## 6. 커밋

`feat/embed-candle` 에 커밋 완료. push / PR 은 메인 세션이 처리 (본 worker 는 하지 않음).
