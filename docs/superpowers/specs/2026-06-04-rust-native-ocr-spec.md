# Spec: Rust 네이티브 OCR 엔진 (PP-OCRv5 ONNX, in-process)

**날짜**: 2026-06-04
**유형**: feature (minor) — 신규 OCR 엔진 + config 키 + 동작 변화
**상태**: draft (self-review 대기)
**contract_sections**: design §6 (parse/extract), §8 (deps), §9 (versioning cascade)

## 동기

현재 이미지/PDF OCR 은 Ollama Vision LLM(`gemma4:e4b` 8B) 1콜(`crates/kebab-parse-image/src/ocr.rs`, `OllamaVisionOcr`). 사용자 실측 문제:

- 실제 이미지 한 장당 **~50초**(VLM 은 글자를 토큰 단위로 생성 → 조밀 페이지는 본질적으로 느림). 모델을 바꿔도(qwen2.5vl:3b GPU 20~28초) 사용자 허용치 미달.
- 사용자 결정: **배치 ingest 용도 + Python 의존 불가 + Rust 내장**.

### 근거 벤치 (2026-06-04, `/build/dogfood/logs/2026-06-04-ocr-model-bench.md`)

| 방식 | 작은 이미지 | 초대형 1757×2644 | 정확도 | 비고 |
|---|---|---|---|---|
| gemma4:e4b 8B VLM (GPU) | 11초 | 43초 | 0.65~0.82 | 현재 |
| qwen2.5vl:3b VLM (GPU) | 3.6초 | 20초 | 0.93 | 속도 미달 |
| **PP-OCRv5 mobile ONNX, Rust (CPU)** | **0.05초** | **2.75초** | **0.976** | **PoC 검증됨** |

VLM 은 생성 병목으로 탈락. **검출+인식형 전용 엔진(PP-OCRv5)을 ONNX 로 Rust in-process 실행**이 속도·정확도·한국어·단일바이너리 모두 만족. PoC: `oar-ocr` 0.6.3 + `ort` 로 위 수치 확인(오류는 띄어쓰기뿐, 한국어 오인식 0). PoC 코드/모델: `/build/cache/ocr-bench/{rust-poc,onnx}/`.

## 핵심 설계 결정: oar-ocr 미채택, 핀 ort 위 직접 구현

PoC 는 `oar-ocr` 0.6.3 으로 검증했으나 **프로덕션 의존성으로는 쓰지 않는다**. 이유(load-bearing):

- kebab 은 `ort = "=2.0.0-rc.9"` 를 **의도적 핀**(workspace `Cargo.toml:195-204`): fastembed 4.9 의 ONNX Runtime+tokenizer 스택을 워크스페이스 단일 버전으로 유지. `ndarray = "0.16"` 도 동일.
- `oar-ocr` 0.6.3 은 `ort 2.0.0-rc.12` + `ndarray 0.17` 요구. `ort` 는 `ort-sys` 가 onnxruntime 네이티브 라이브러리를 `links` 하므로 **두 버전 공존 불가** → oar-ocr 채택 시 ort/ndarray 를 bump 해야 하고, 이는 fastembed/kebab-nli/kebab-embed-candle 의 임베딩·NLI 스택을 흔든다(사용자 우선순위인 검색 품질 직결, [[search-quality-dogfood]]).

**→ PaddleOCR 의 전/후처리(검출 DBNet postproc + 인식 CTC decode)를 kebab 의 기존 핀 `ort`(rc.9) 위에 직접 구현.** oar-ocr(Apache-2.0) 소스 + Python PaddleOCR 을 레퍼런스로. 공유 ort 라 새 네이티브 의존성 0, 임베딩 스택 무영향.

### C2 검증 완료 (rc.9 스파이크, 2026-06-04)

PoC 는 oar-ocr 경유 ort **rc.12** 로 돌았으므로, 핀 **rc.9** 가 paddle2onnx 산출 모델을 실제 로드/추론하는지 별도 검증함(`/build/cache/ocr-bench/rc9-spike/`). 결과 **PASS**:
- `ort = "=2.0.0-rc.9"` + `ort-sys = "=2.0.0-rc.9"`(caret 으로 rc.12 끌려가는 것 방지 — kebab Cargo.lock 과 동일) + `ndarray 0.16` + feature `["ndarray","download-binaries"]` 로 빌드/링크/onnxruntime 다운로드 성공.
- det: 입력 `"x"` → 출력 `[1,1,640,640]`(DBNet 확률맵). rec: 출력 `[1,40,11947]`(timestep×클래스; dict 11,945 + blank/특수 = 11,947, CTC 정합 확인).
- `try_extract_tensor::<f32>()` 는 rc.9 에서 `ArrayViewD<f32>` 반환(rc.12 의 `(shape,&[T])` 와 다름) — 구현 시 유의.
- **함의**: 핀 ort 유지(ort/ndarray bump 불필요)로 임베딩 스택 무영향 확정. opset 호환 OK. 출력 형태가 후처리 설계(det threshold→박스 / rec CTC)와 일치.

### 추가 의존성

- `image`(이미 허용), `ndarray`(workspace `=0.16`), `ort`(workspace `=2.0.0-rc.9`, **features `["ndarray","download-binaries"]`**).
  - **download-binaries 필수**: `kebab-parse-image` 는 fastembed 빌드그래프에 없어, 단독 빌드(`cargo test -p kebab-parse-image`)시 onnxruntime 링크 위해 명시 필요. `kebab-nli/Cargo.toml:23` 의 동일 선례 주석 그대로 따름.
  - `ort-sys` 가 caret 으로 rc.12 로 끌려가지 않도록 workspace 핀과 Cargo.lock 정합 확인.
- `imageproc` — det 확률맵 연결요소/윤곽 추출. **단 min-area rotated-rect 는 imageproc 미제공 → rotating-calipers 직접 구현**.
- DBNet unclip(다각형 확장): **`clipper2` 는 C++ FFI 가능성 → single-binary/pure-Rust 위배 위험. 우선 pure-Rust 다각형 offset 직접 구현 또는 검증된 pure-Rust crate.** (plan 에서 clipper2 가 C++ 링크인지 확인 후 택일.)

## 파이프라인 (OnnxPaddleOcr)

`crates/kebab-parse-image/src/` 에 신규 모듈. `OcrEngine` trait(`ocr.rs:54`) 구현:

```rust
pub trait OcrEngine: Send + Sync {
    fn engine_name(&self) -> &'static str;       // "paddle-onnx"
    fn engine_version(&self) -> String;          // "ppocrv5-mobile-kor-v1" (+model hash)
    fn recognize(&self, image_bytes: &[u8], lang_hint: Option<&Lang>) -> Result<OcrText>;
}
```

`recognize` 단계 (PoC 와 동일 알고리즘):

1. **디코드+다운스케일**: `image` 로 디코드 → 긴변 `max_pixels`(기본 1600) 로 축소(기존 `OcrCfg.max_pixels` 재사용, qwen 과 달리 PP-OCRv5 는 원본도 안전하나 속도 위해 유지).
2. **검출(det)**: BGR 정규화 → det ONNX(`PP-OCRv5_mobile_det`) → 확률맵 → threshold(0.3) 이진화 → 윤곽(imageproc) → min-area rect → unclip(ratio 1.5) → 텍스트 박스 N개.
3. **인식(rec)**: 각 박스 crop+회전보정 → 48×W 리사이즈/정규화 → rec ONNX(`korean_PP-OCRv5_mobile_rec`) → CTC greedy decode(dict 11,945자, blank 처리) → 텍스트+score.
4. **조립**: 박스를 reading-order(상→하, 좌→우) 정렬 → `OcrText { joined, regions: Vec<OcrRegion{bbox,text,confidence}>, engine, engine_version }`. **Ollama 와 달리 per-line bbox/confidence 제공**(`OcrRegion` 풍부화).

배치: PoC 는 박스별 순차 rec. 성능 충분(초대형 2.75초)하나, rec 를 ort 배치 입력으로 묶으면 추가 향상 가능(plan 에서 측정 후 결정).

### 단계별 분해 (M1 — 각 단계 골든벡터 단위테스트)

후처리가 실제 난도. "쉽다"로 뭉뚱그리지 않고 **각 단계를 독립 테스트 가능 단위**로 쪼갠다. 각 단위는 oar-ocr/Python PaddleOCR 이 **같은 fixture** 에 내는 출력을 골든벡터로 박아 단계별 회귀(0.976 baseline 대비)를 잡는다:

1. **전처리**(resize/pad/normalize): det 입력 정규화(mean/std, /255). 골든: 알려진 이미지→텐서 일부 값.
2. **det 후처리**: 확률맵(`[1,1,H,W]`)→threshold(0.3)→연결요소(imageproc)→**min-area rotated-rect(rotating calipers 직접 구현)**→**unclip(다각형 offset, ratio 1.5)**→박스. 골든: 합성 이미지의 기대 박스 개수/대략 좌표.
3. **crop+rectify**: 회전 박스→perspective/affine warp 로 수평 정렬(oar-ocr 가 공짜 제공하던 부분; 직접 구현 필요). 골든: 회전 텍스트 fixture.
4. **rec 전처리+추론**: crop→48×W 정규화→rec ONNX→`[1,T,C]` logits.
5. **CTC greedy decode**: argmax per timestep→연속중복 제거→blank(인덱스 0 또는 dict 길이 위치, **PaddleOCR 규약 정확 매칭**) 제거→dict 인덱스→char. dict 길이(11,945) vs rec 출력 클래스(11,947) 정합 + **인덱스 bounds-check**(잘못된 dict 길이/빈 줄 방어). 골든: 알려진 logit→문자열.
6. **box reading-order**: 상→하, 좌→우 정렬(가로쓰기 전제; 세로/회전 페이지는 비범위).

각 단계 divergence 를 end-to-end 가 아니라 단위에서 잡는다(M1 권고).

## Config

`OcrCfg`(`kebab-config/src/lib.rs:343`)에 `engine` 필드 **이미 존재**(기본 `"ollama-vision"`). 변경:

- `engine` 값에 `"paddle-onnx"` 추가(문서화). 기본값은 **당장 바꾸지 않음**(default 변경은 별도 결정 — 아래 "기본 엔진" 참조).
- 신규(선택) 필드: `det_model` / `rec_model` / `dict` 경로 override(미지정 시 자동 다운로드 캐시 경로). `score_thresh`(기본 0.3), `unclip_ratio`(기본 1.5) 는 고급 튜닝용(기본값 고정, 노출 최소).
- `pdf.ocr` 도 동일 `engine` 분기 적용(같은 trait).

### 모델 배포 — 결정 C: kebab 와 함께 번들 (HF 미사용, 사용자 확정 2026-06-04)

제3자(HF) 호스팅 의존 제거. 변환본(det 4.7MB + korean rec 13MB + dict ≈ **17MB**)을 kebab 자체에 번들. **구체 기법은 plan 에서 택1**(모두 HF/외부 네트워크 0):

- **C-1 바이너리 임베드(`include_bytes!`)**: 모델을 바이너리에 박음. 진정한 single-binary·완전 오프라인·재현성 100%. 비용: 릴리스 바이너리 +17MB, 그리고 dev/test 빌드마다 17MB 링크 부담 → **release feature(`bundled-ocr-models`) 게이트**로 dev 빌드 제외 가능. 로컬-first 철학 최적합.
- **C-2 repo 벤더링**: `assets/paddleocr-onnx/`(git 또는 git-LFS) 에 두고 빌드 시 `OUT_DIR` 복사 또는 런타임 상대경로. 바이너리 비대 회피하나 배포 시 파일 동반 필요.
- **C-3 gitea 릴리스 에셋 + 첫 실행 다운로드**: `gitea-release --asset` 로 첨부, 첫 실행 시 릴리스 URL 에서 `model_dir/paddleocr-onnx/` 로 받음. 바이너리 lean 하나 첫 실행 시 gitea 네트워크 필요(에어갭 불가) — 로컬-first 와 약간 상충.

**권장 = C-1(release feature 게이트)**: 오프라인·재현성·single-binary 가 kebab 정체성과 가장 정합. plan 에서 빌드/링크 영향 측정 후 확정.

- **무결성**: 임베드(C-1)면 빌드 시점 고정이라 별도 해시 불요(바이너리=정본). C-2/C-3 면 blake3 pin 필수.
- **라이선스**: PP-OCRv5 가중치 Apache-2.0 — 재배포 가능. 번들에 NOTICE 동반.
- **오프라인**: C-1 완전 오프라인. config override(`det_model`/`rec_model`/`dict`)로 로컬 모델 교체 항상 가능.

## 엔진 선택 (kebab-app 팩토리)

현재 `OllamaVisionOcr` 하드코딩(`kebab-app/src/lib.rs:360`(image), `379`(pdf)). 변경:

```rust
let ocr_engine: Option<Box<dyn OcrEngine>> = if cfg.image.ocr.enabled {
    match cfg.image.ocr.engine.as_str() {
        "ollama-vision" => Some(Box::new(OllamaVisionOcr::new(cfg)?)),
        "paddle-onnx"   => Some(Box::new(OnnxPaddleOcr::new(cfg)?)),
        other => bail!("unknown image.ocr.engine: {other}"),
    }
} else { None };
```

- `ImagePipeline.ocr_engine` 를 `Option<&'a dyn OcrEngine>` 로(현재 구체타입 `&OllamaVisionOcr`).
- pdf 경로 동일. `apply_ocr`/`apply_ocr_to_pdf_pages` 는 이미 `&dyn OcrEngine` 받음 → 변경 불필요.
- `OnnxPaddleOcr` 는 한 번 생성(모델 1회 로드) 후 ingest 전체에서 재사용(PoC 모델로드 58ms, 무시 가능).

## 버전/재색인 cascade

OCR 엔진 변경 시 **영향 자산 자동 재색인**되어야 함(v0.26.2 메커니즘). 현재 `ingest_config_signature`(`kebab-app/src/lib.rs:3036` 부근)의 image/pdf 브랜치는 `|ocr:1:{ocr.model}` 만 서명.

**C3 (필수, 권장 아님)**: paddle-onnx 브랜치에서 `model`("gemma4:e4b" 기본) 은 **미사용** — 실제 모델 정체성은 det/rec/dict + engine_version 에 있음. 따라서:
- 서명을 `|ocr:1:{engine}:{engine_version}` 로(엔진 + 모델/dict 식별자). `engine_version()`(spec 의 model+dict blake3 해시 포함, 라인 47)을 **반드시** 서명에 사용.
- 이유: ① `engine="ollama-vision"→"paddle-onnx"` 전환 시 model 이 기본값 그대로면 `{model}` 만으론 서명 불변 → **재색인 안 됨**(silent stale index, v0.26.2 가 없애려던 바로 그 버그). ② 모델 재변환/dict 수정 시 engine_version 변화로 재색인 트리거.
- 단위테스트(필수): (a) `ollama-vision`↔`paddle-onnx` 동일 model → 서명 다름. (b) 동일 engine, engine_version 다름 → 서명 다름. (c) 무관 설정(search 등) → 서명 불변.

## 기본 엔진 (default) — 별도 결정

본 spec 은 `paddle-onnx` 를 **선택 가능**하게만 한다. kebab 의 `image.ocr.engine` **기본값을 `paddle-onnx` 로 바꿀지**는 후속 결정:
- 바꾸면: 신규 사용자/기본 동작 변화 + 모델 다운로드 기본화. 강력하나 영향 큼.
- v1 은 기본 `ollama-vision` 유지, opt-in `paddle-onnx`. 도그푸딩 후 기본 전환을 별 PR 로. (사용자 본인 config 는 즉시 `paddle-onnx`.)

## 에러 처리 (M3 — 명시 매트릭스)

배치 ingest 가 미지의 사용자 스캔을 돈다. 각 케이스 동작 확정:

| 케이스 | 동작 | 근거 |
|---|---|---|
| 모델 다운로드 실패 | 엔진 생성 시 **fail-fast**(Ollama 와 동일, `lib.rs:360`) | 색인 시작 전 차단 |
| blake3 불일치 | fail-fast + 사유 | 무결성 |
| 디코드 불가 이미지 | **자산 skip + provenance 노트**(ingest 중단 X) | 기존 `apply_ocr` "skip vs surface" 계약(`ocr.rs:75`) |
| det 0 박스(빈 이미지 등) | **성공, `OcrText{joined:"", regions:[]}`**(에러 아님) | Ollama 빈줄 동작(`ocr.rs:290`) 미러 |
| rec 빈 출력(한 박스) | 그 박스 skip, 나머지 진행 | |
| 박스 폭증(노이즈 스캔) | **`max_boxes` 상한**(기본 예: 1000) 초과분 절단 + 로그 | 메모리/지연 cliff 방지 |
| dict 길이 ≠ rec 클래스 | 생성 시 에러(정합 검증) | bounds-check |

ort `Session` 은 생성 후 1회 로드·재사용. ingest 는 현재 직렬(`lib.rs:460`, rayon 없음)이라 동시접근 없음 — 단 `OcrEngine: Send+Sync` 유지(미래 병렬화 대비, rc.9 Session Send/Sync 확인은 plan).

## 검증 기준

- `cargo clippy --workspace --all-targets -j 8 -- -D warnings` 0.
- `cargo test -p kebab-parse-image -p kebab-app -j 8` 통과(touched 크레이트; `kebab-parse-image` 단독 빌드가 download-binaries 로 링크되는지 포함).
- 신규 단위테스트:
  - 단계별 골든벡터(전처리/det후처리/CTC/박스정렬) — baseline 0.976 대비 단계 회귀 감지.
  - OnnxPaddleOcr e2e: 합성 한/영 fixture → **CER ≤ 0.05**(=문자정확도 ≥95%), bbox>0. (단 합성 fixture 는 실코퍼스 회귀 미보장 → 도그푸딩 병행.)
  - CTC decode: 알려진 logit→문자열(blank/중복 제거, bounds-check).
  - 엔진 팩토리: `engine="paddle-onnx"`→OnnxPaddleOcr, 미지 값 에러.
  - 서명(C3): 위 (a)(b)(c) 케이스.
  - config override(`det_model`/`rec_model`/`dict`) 가 실제 사용됨 + **`--config` facade 스레딩**(CLAUDE.md facade rule, P3-5/P4-3 회귀 전례) — `OnnxPaddleOcr::new(cfg, …)` 가 explicit Config 받음.
- 회귀 가드: `engine="ollama-vision"`(기본) 경로 — 팩토리 리팩터(구체타입→`&dyn`) 후에도 **출력 동일** 핀하는 테스트.
- 스모크: `engine="paddle-onnx"` 이미지 ingest → OCR 텍스트 FTS5 hit. 큰 페이지 CPU <5초.
- 도그푸딩: 사용자 실제 이미지/책 스캔 정확도·속도(HOTFIXES + release notes).

## 의존성 규칙 (design §8)

`kebab-parse-image` allowed: kebab-core, kebab-config, serde, image, tracing, thiserror(task p6-2). 추가: `ort`(workspace, features `["ndarray","download-binaries"]`), `ndarray`(workspace), `imageproc`. **clipper2 미추가**(C++ FFI 회피 — unclip pure-Rust 직접). **hf-hub 미추가**(결정 C: 모델 번들, 외부 다운로드 0). **금지 유지**: kebab-store-*/embed-*/llm-* 미import. UI 크레이트 영향 없음.

## 비범위

- **OCR 텍스트→임베딩 갭**(현재 OCR 은 FTS5 lexical 전용, 벡터 미포함). 사용자 "OCR 모델만 먼저" → 별도 작업.
- **caption** 은 gemma 유지([[project_llm_default]]).
- **GPU provider**(ort CUDA/CoreML): CPU 로 충분(2.75초). 후속 옵션.
- **기본 엔진 전환**(default `paddle-onnx`): 도그푸딩 후 별 PR.
- 다국어 dict 동적 전환(현재 korean dict = 한+영+숫자+기호 11,945자로 한/영 충분).

## 잔여 노트 (critic minors)

- **max_pixels(m1)**: 기존 `[256,4096]` clamp 은 VLM 프롬프트 비용 기준. det/rec 엔진은 비용이 latency 라 trade-off 다름. v1 은 기본 1600 **유지(의도적)** — PoC 에서 1600 대 원본 정확도 차 미미, 속도 이점. plan 에서 paddle-onnx 전용 기본 재검토 가능.
- **config 마이그레이션(m3)**: 신규 키(`det_model` 등)는 serde default 로 forward-compat(기존 파일 무수정 로드). `kebab config migrate`(#198) 가 주석/순서 보존하며 신규 키 추가 — migration 핸들링 불필요(serde default), 단 init 템플릿에 신규 키 노출.
- **per-region confidence(open q)**: Ollama 는 region confidence 상수 1.0, paddle-onnx 는 실제 score. `OcrRegion` 형태 불변이라 wire 호환(값만 의미있어짐) — release note 1줄.
- **세로/회전 페이지**: 비범위(가로쓰기 reading-order 전제). 회전 박스 rectify 는 지원하나 페이지 전체 세로조판은 미지원 명시.

## 버전/문서

- feature(신규 engine 값 + 동작) → **minor bump**.
- README(Configuration: `image.ocr.engine`, 모델 첫 다운로드 안내), docs/SMOKE(config 예시), HANDOFF 1줄, docs/ARCHITECTURE(새 OCR 백엔드 추가 시 그래프/결정), HOTFIXES dated entry(도그푸딩 evidence). wire schema 불변(OcrText 내부, `--json` 표면 동일).
