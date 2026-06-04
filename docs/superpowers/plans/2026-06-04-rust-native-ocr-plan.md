# Plan: Rust 네이티브 OCR 엔진 (PP-OCRv5 ONNX) 구현

spec: `docs/superpowers/specs/2026-06-04-rust-native-ocr-spec.md`. 브랜치 `feat/rust-native-ocr`.
빌드 `CARGO_TARGET_DIR=/build/out/cargo-target`, 테스트 **`-j 8`**(절대 `-j 1` 금지), touched 크레이트 위주(`-p kebab-parse-image -p kebab-app -p kebab-config`).
참조 구현: `oar-ocr`(Apache-2.0) 소스 + Python PaddleOCR + 검증된 PoC `/build/cache/ocr-bench/{rust-poc,onnx,rc9-spike}/`(변환 ONNX + rc.9 동작 확인).

## Task 0a — 레퍼런스 골든 하네스 (C1 — 최우선 선행, executor 차단 제거)
**T3/T5 골든은 oar-ocr 로 못 만든다**(중간 텐서 미노출, PoC 는 최종텍스트만). 먼저 Python `onnxruntime` 직접(oar-ocr X)으로 변환 모델을 돌려 fixture 별 중간 산출을 골든으로 덤프:
- 입력: `/build/dogfood/corpus/images/synthetic-ocr-bench/` fixtures + 변환 ONNX(`/build/cache/ocr-bench/onnx/`).
- 덤프(JSON/npy, repo `crates/kebab-parse-image/tests/golden/`): (a) det 확률맵 슬라이스, (b) threshold 후 박스 폴리곤, (c) **rec 원시 logits `[T,C]`**, (d) 디코드 문자열, (e) 전처리 텐서 일부값.
- **M2 해결**: 알려진 텍스트라인 crop 의 logits + argmax 로 **blank 인덱스 + dict 11,945→클래스 11,947 매핑(+2 정체)을 경험적으로 도출**해 plan/주석에 사실로 기록(추정 금지). 경계문자(dict 첫/끝) 포함 골든.
- 도구: 기존 venv `/build/cache/ocr-bench/venv`(onnxruntime 직접 설치) 또는 paddleocr API 의 raw 단계. 하네스 스크립트는 `/build/cache/ocr-bench/` 에 보관(런타임 의존 아님, 골든 생성 전용).
- 수용: 각 fixture 골든 파일 생성 + blank 인덱스 문서화. 이후 T3~T5 가 이 골든에 핀.

## Task 0 — 모델 번들 (결정 C-1: include_bytes, release feature 게이트)
- 변환 ONNX(이미 존재: `/build/cache/ocr-bench/onnx/{ppocrv5_mobile_det.onnx, korean_ppocrv5_mobile_rec.onnx, korean_dict.txt}`)를 repo `crates/kebab-parse-image/assets/paddleocr-onnx/` 에 배치(+NOTICE, Apache-2.0).
- `bundled-ocr-models` cargo feature: on 이면 `include_bytes!` 로 임베드, off(dev 기본)면 config override 경로 필수. release 빌드는 feature on.
- 대안 C-2/C-3 는 빌드/링크 부담 측정 후 폴백(spec §모델 배포). 17MB 임베드의 dev 링크 영향 먼저 측정 — 과하면 C-2(repo 벤더 + OUT_DIR) 전환.
- **assets 17MB 커밋 방식 결정(M4/packaging)**: git-LFS 권장(clone/`cargo package` 비대 회피). `.gitattributes` 에 `*.onnx filter=lfs`. NOTICE(Apache-2.0) 동반.
- **테스트 모델 출처(M4)**: OCR 단위/e2e 테스트는 `bundled-ocr-models` feature 무관하게 `KEBAB_TEST_OCR_MODEL_DIR`(기본 `assets/paddleocr-onnx/`)에서 로드. 모델 없으면 `#[ignore]` 가 아니라 명확 skip+경고(CI 는 assets 존재 가정). dev 빌드 OCR 테스트가 모델 못 찾아 실패하는 모호함 제거.
- 수용: feature on 빌드 임베드 확인, off 빌드 정상, 테스트가 assets 에서 모델 로드.

## Task 1 — 의존성 (kebab-parse-image/Cargo.toml)
- `ort = { workspace = true, features = ["ndarray", "download-binaries"] }`(C1: 단독빌드 링크, nli 선례 주석). `ndarray = { workspace = true }`. `imageproc`(연결요소/윤곽).
- `ort-sys` caret 으로 rc.12 끌려가지 않게 Cargo.lock 정합 확인(rc.9 고정). unclip 다각형 offset 은 **pure-Rust 직접 구현**(clipper2 C++ FFI 회피 — spec).
- 수용: `cargo build -p kebab-parse-image -j 8` 링크 성공(onnxruntime), `cargo tree` 에 ort 단일 rc.9.

## Task 2 — OnnxPaddleOcr 골격 + 전처리 (kebab-parse-image)
- **선행 사실 확인**: rc.9 `ort::Session` 이 `Send+Sync` 인지 먼저 확인(아니면 Mutex 래핑). 결과를 주석에 기록.
- 신규 모듈 `paddle_onnx.rs`. `OcrEngine` 구현. **`engine_version`=생성 시 모델+dict blake3 1회 계산해 String 캐시**(m3: per-asset 재해시 금지 — `ingest_config_signature` 가 자산마다 호출). format 고정(후일 변경 시 mass 재색인 주의).
- det/rec `ort::Session` 2개 1회 로드 후 보관. **max_pixels 자체 bounds 적용**(spec 의 ocr.rs MIN/MAX clamp 은 Ollama private — paddle 은 자기 clamp 명시).
- 전처리: 디코드(image)→긴변 max_pixels 축소→BGR mean/std 정규화→`Array4<f32>`.
- 수용: 단위테스트 — 알려진 이미지→입력텐서 일부 값 골든(T0a).

## Task 3 — det 후처리 (단계 단위, 골든벡터)
- det Session 추론(`[1,1,H,W]` 확률맵, rc.9 `try_extract_tensor`→`ArrayViewD`) → threshold 0.3 이진화 → imageproc 연결요소/윤곽 → **min-area rotated-rect(rotating calipers 직접 구현)** → **unclip(pure-Rust 다각형 offset, ratio 1.5)** → 박스 Vec.
- 수용: 합성 fixture 기대 박스 개수/대략 좌표 골든. min-area rect·unclip 각각 단위테스트.

## Task 4 — crop + rectify
- 회전 박스 → perspective/affine warp 로 수평 정렬(oar-ocr 가 제공하던 부분 이식).
- 수용: 회전 텍스트 fixture → 정렬 crop 골든.

## Task 5 — rec + CTC decode
- crop→48×W 정규화→rec Session(`[1,T,C]`) → CTC greedy(argmax/timestep→연속중복 제거→blank 제거).
- **blank 인덱스 + 11,945→11,947 매핑은 T0a 하네스에서 도출한 사실을 사용**(추정 금지). bounds-check(dict 길이≠클래스 시 생성 에러).
- 수용: T0a 골든 logit→문자열 일치(blank/중복/**경계문자 dict 첫·끝** 포함).

## Task 6 — 조립 + OcrText
- 박스 reading-order(상→하,좌→우) → `OcrText{joined, regions:[OcrRegion{bbox,text,confidence}], engine, engine_version}`. per-region 실제 confidence(Ollama 상수1.0 대비 값 변화 — release note).
- 수용: e2e — 합성 한/영 fixture **CER ≤ 0.05**, bbox>0. PoC 0.976 baseline 대비 회귀 없음.
- **CER 게이트 실패 시 폴백 사다리(M3)**: ① T0a 단계 골든과 diff 해 어느 단계 divergence 인지 국소화 → ② det postproc(unclip/min-area rect)가 원인이면 **oar-ocr 의 해당 함수를 verbatim 이식**(Apache-2.0, NOTICE+파일별 출처 표기 — 코드 파생물) → ③ time-box(예 반나절) 초과 시 리더 escalate. 손수 재유도에 매몰 금지.

## Task 7 — config (kebab-config)
- `OcrCfg`: `engine` 값에 "paddle-onnx" 문서화(기본 "ollama-vision" 유지). 신규 override `det_model`/`rec_model`/`dict`(Option), `score_thresh`(0.3)/`unclip_ratio`(1.5)/`max_boxes`(1000). `KEBAB_IMAGE_OCR_*` env. serde default(forward-compat) + init 템플릿 노출.
- 수용: override 미지정→번들 모델, 지정→그 경로 사용 테스트. config migrate(#198) 무수정 로드 회귀.

## Task 8 — 엔진 팩토리 (kebab-app/lib.rs) — **4개 site 전부(M1)**
구체타입 `OllamaVisionOcr` 가 박힌 곳이 4군데 — 누락 시 타입에러로 막힘:
- `:360` image 엔진 생성 → `Box<dyn OcrEngine>` 팩토리(`match engine`: ollama-vision|paddle-onnx|err).
- `:379` pdf 엔진 생성 → 동일 팩토리.
- `:839` `ImagePipeline.ocr_engine` 필드 → `Option<&dyn OcrEngine>`.
- `:1113`, `:2096` `pdf_ocr_engine: Option<&OllamaVisionOcr>` 함수 시그니처 2곳 → `Option<&dyn OcrEngine>`.
- `apply_ocr_to_pdf_pages`(`pdf_ocr_apply.rs:93`)는 이미 `&dyn OcrEngine` — 스레딩만 변경, 헬퍼 불변. `--config` facade 스레딩(`OnnxPaddleOcr::new(cfg,…)`).
- 수용: 팩토리 단위테스트(선택/미지값 에러). **ollama-vision 경로 출력 동일** 회귀 테스트(구체→dyn 전환 무영향).

## Task 9 — 서명 cascade (C3, kebab-app)
- `ingest_config_signature` image/pdf 브랜치 `|ocr:1:{model}` → `|ocr:1:{engine}:{engine_version}`(engine + 모델/dict blake3). 
- 수용: (a)ollama↔paddle 동일model→서명다름 (b)engine_version 다름→다름 (c)search 등 무관→불변. → 엔진/모델 변경 시 v0.26.2 자동 재색인.

## Task 10 — 에러 매트릭스 (spec §에러 처리)
- 다운로드/blake3 실패→fail-fast, 디코드불가→skip+provenance, det 0박스→`OcrText{"",[]}` 성공, rec 빈→박스skip, 박스폭증→max_boxes 절단+로그, dict 불일치→생성에러.
- 수용: 각 케이스 단위/통합 테스트.

## Task 11 — 검증 게이트
- `cargo clippy --workspace --all-targets -j 8 -- -D warnings` 0.
- `cargo test -p kebab-parse-image -p kebab-app -p kebab-config -j 8` 통과(+ `-p kebab-parse-image` 단독 링크 확인).
- 스모크: `engine="paddle-onnx"` 이미지 ingest→FTS5 hit, 큰 페이지 CPU <5초.

## Task 12 — 문서 + 버전 + 도그푸딩
- README(Configuration: `image.ocr.engine`+모델 번들), docs/SMOKE(config 예시), HANDOFF 1줄, docs/ARCHITECTURE(OCR 백엔드/그래프), HOTFIXES dated entry.
- Cargo.toml workspace version **minor bump**(+Cargo.lock). release notes(엔진 추가/per-region confidence/오프라인).
- 도그푸딩: 사용자 실제 이미지·책 스캔 정확도·속도 → HOTFIXES + release notes evidence.
- 결과 요약 `/tmp/rust-ocr-result.md`(게이트 + 스모크 + 도그푸딩 캡처).

## 리뷰 루프
완료 → 리더 clippy/타깃테스트(-j8) 독립 재확인 + paddle-onnx 스모크 → `gitea-pr`(title `feat(ocr): PP-OCRv5 ONNX Rust 네이티브 OCR 엔진`) → 리뷰 루프 → 사용자 머지. 모델 ONNX 는 release feature/asset 로 동반.

## 단계 의존
**T0a(레퍼런스 골든+blank 도출) 최우선 선행** → T0(번들),T1(deps) → T2→T3→T4→T5→T6(파이프라인 순차, 각 T0a 골든에 핀) ∥ T7(config) → T8(팩토리 4site)→T9(서명)→T10(에러) → T11 게이트 → T12 문서. T3~T5 가 핵심 난도(직접 이식), T0a 골든+T6 폴백사다리로 회귀·매몰 차단. T8 의 정확한 라인(:1113/:2096 등)은 구현 시점 grep 으로 재확인(코드 이동 가능).
