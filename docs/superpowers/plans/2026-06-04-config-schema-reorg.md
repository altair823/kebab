# config 스키마 재편 (v2 → v3) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** config.toml 의 미디어 형식 설정을 `[ingest.*]` 우산 아래로 통합하고, 기존 v2 파일을 무손실 자동 마이그레이션하며, 각 옵션에 설명 주석을 부착한다.

**Architecture:** `kebab-config` 의 `Config` 구조체를 재배치(`indexing`/`chunking`/`image`/`pdf` → `ingest` 하위)하고, leaf 구조체는 불변으로 둔 채 부모 경로만 이동. `migrate::step_2_to_3` 가 toml_edit 으로 기존 테이블을 새 위치로 relocation(값·주석 보존). `Config::from_file` 이 load 시 메모리 내 자동 변환을 수행해 미변환 v2 파일도 설정 유실 없이 로드. env override 이름·`ingest_config_signature` 출력 문자열은 바이트 단위로 보존해 스크립트·재색인 회귀 0.

**Tech Stack:** Rust 2024, serde, toml, toml_edit, kebab workspace (kebab-config / kebab-app / kebab-eval).

**설계 문서:** `docs/superpowers/specs/2026-06-04-config-schema-reorg-design.md`

---

## File Structure

| 파일 | 책임 | 작업 |
|------|------|------|
| `crates/kebab-config/src/lib.rs` | `Config`/`IngestCfg` 재배치, `PdfOcrCfg` paddle 대칭 키, `ser_f32_clean`, `apply_env` RHS 갱신, `from_file` 자동변환 | Modify |
| `crates/kebab-config/src/migrate.rs` | `step_2_to_3` relocation, `key_comment`, `CURRENT_SCHEMA_VERSION=3`, annotate 확장 | Modify |
| `crates/kebab-config/tests/migrate_v3.rs` | v3 마이그레이션 골든·무손실·멱등 테스트 | Create |
| `crates/kebab-app/src/lib.rs` | call-site sweep + `ocr_engine_version_for_sig` 미디어화 + signature 불변 | Modify |
| `crates/kebab-app/src/app.rs` `schema.rs` | `config.chunking.*` → `config.ingest.chunking.*` | Modify |
| `crates/kebab-eval/src/runner.rs` | config_snapshot chunker_version 경로 | Modify |
| `crates/kebab-app/tests/config_invalidation.rs` | signature 골든 회귀 | Modify/확인 |
| `README.md` `docs/SMOKE.md` `tasks/HOTFIXES.md` | 문서 cascade | Modify |
| `Cargo.toml` (workspace) | minor version bump | Modify |

**Task 의존성:** T1(구조체) → T2(sweep, 컴파일 복구) → T3(signature) → T4(key_comment) → T5(step_2_to_3) → T6(from_file 자동변환) → T7(무손실 골든) → T8(env) → T9(문서+bump) → T10(도그푸딩).

---

## Task 1: `Config` 구조체 재배치 + paddle 대칭 + float 직렬화

**Files:**
- Modify: `crates/kebab-config/src/lib.rs`

핵심: leaf 구조체(`OcrCfg`/`CaptionCfg`/`ChunkingCfg`/`PdfOcrCfg`/`IngestCodeCfg`/`ImageCfg`/`PdfCfg`)는 내부 필드 유지. `IndexingCfg` 는 해체해 `IngestCfg` 스칼라로 흡수. `Config` 에서 `indexing`/`chunking`/`image`/`pdf` top-level 필드 제거, `ingest: IngestCfg` 하나로.

- [ ] **Step 1: `defaults_are_serde_roundtrip_stable` 가 새 경로를 검증하도록 실패 테스트 추가**

`crates/kebab-config/src/lib.rs` 의 `mod tests` 에 추가:

```rust
#[test]
fn v3_layout_nests_media_under_ingest() {
    let c = Config::defaults();
    // 새 경로가 컴파일·접근 가능해야 한다.
    assert_eq!(c.ingest.max_parallel_extractors, 2);
    assert_eq!(c.ingest.chunking.target_tokens, 500);
    assert_eq!(c.ingest.code.max_file_bytes, 262_144);
    assert_eq!(c.ingest.image.ocr.engine, "ollama-vision");
    assert_eq!(c.ingest.image.caption.max_pixels, 768);
    assert_eq!(c.ingest.pdf.ocr.model, "qwen2.5vl:3b");
    // pdf paddle 대칭 키 존재 + 기본값.
    assert_eq!(c.ingest.pdf.ocr.score_thresh, 0.3);
    assert_eq!(c.ingest.pdf.ocr.max_boxes, 1000);
    assert!(c.ingest.pdf.ocr.det_model.is_none());
}
```

- [ ] **Step 2: 컴파일 실패 확인**

Run: `cargo build -p kebab-config 2>&1 | head`
Expected: `no field 'ingest' on type 'Config'` 류 에러.

- [ ] **Step 3: 구조체 재배치 구현**

`Config` 정의를 다음으로 교체(필드 순서 = 직렬화 순서: 스칼라/단순 테이블 먼저):

```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Config {
    pub schema_version: u32,
    pub workspace: WorkspaceCfg,
    pub storage: StorageCfg,
    pub models: ModelsCfg,
    pub ingest: IngestCfg,
    pub search: SearchCfg,
    pub rag: RagCfg,
    #[serde(default = "UiCfg::defaults")]
    pub ui: UiCfg,
    #[serde(default)]
    pub logging: LoggingCfg,
    #[serde(skip)]
    pub(crate) source_dir: Option<PathBuf>,
}
```

`IngestCfg` 를 교체(기존 `IngestCfg { code }` 확장):

```rust
/// v3: 모든 미디어 형식 ingest 설정의 우산. 스칼라(병렬도)는 ← 기존 [indexing].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IngestCfg {
    #[serde(default = "default_max_parallel_extractors")]
    pub max_parallel_extractors: u32,
    #[serde(default = "default_max_parallel_embeddings")]
    pub max_parallel_embeddings: u32,
    #[serde(default)]
    pub watch_filesystem: bool,
    #[serde(default = "ChunkingCfg::defaults")]
    pub chunking: ChunkingCfg,
    #[serde(default)]
    pub code: IngestCodeCfg,
    #[serde(default = "ImageCfg::defaults")]
    pub image: ImageCfg,
    #[serde(default = "PdfCfg::defaults")]
    pub pdf: PdfCfg,
}

impl Default for IngestCfg {
    fn default() -> Self {
        Self {
            max_parallel_extractors: default_max_parallel_extractors(),
            max_parallel_embeddings: default_max_parallel_embeddings(),
            watch_filesystem: false,
            chunking: ChunkingCfg::defaults(),
            code: IngestCodeCfg::default(),
            image: ImageCfg::defaults(),
            pdf: PdfCfg::defaults(),
        }
    }
}

fn default_max_parallel_extractors() -> u32 { 2 }
fn default_max_parallel_embeddings() -> u32 { 1 }
```

`ChunkingCfg` 에 `defaults()` 연관함수 추가(없으면):

```rust
impl ChunkingCfg {
    pub fn defaults() -> Self {
        Self {
            target_tokens: 500,
            overlap_tokens: 80,
            respect_markdown_headings: true,
            chunker_version: "md-heading-v1".to_string(),
        }
    }
}
```

`IndexingCfg` struct 정의 + `apply_env` 의 `KEBAB_INDEXING_*` 대입을 임시로 둔 채(T1 에선 RHS 만 `self.ingest.*` 로) 삭제. 구체적으로 `IndexingCfg` 선언을 제거하고, `Config::defaults()` 의 `indexing:`/`chunking:`/`image:`/`pdf:` 필드를 `ingest: IngestCfg { ... }` 하나로 교체:

```rust
            ingest: IngestCfg {
                max_parallel_extractors: 2,
                max_parallel_embeddings: 1,
                watch_filesystem: false,
                chunking: ChunkingCfg::defaults(),
                code: IngestCodeCfg::default(),
                image: ImageCfg::defaults(),
                pdf: PdfCfg::defaults(),
            },
```

`PdfOcrCfg` 에 paddle 대칭 6키 추가(`OcrCfg` 와 동일 패턴, 전부 `#[serde(default)]`):

```rust
    #[serde(default)]
    pub det_model: Option<String>,
    #[serde(default)]
    pub rec_model: Option<String>,
    #[serde(default)]
    pub dict: Option<String>,
    #[serde(default = "default_ocr_score_thresh")]
    pub score_thresh: f32,
    #[serde(default = "default_ocr_unclip_ratio")]
    pub unclip_ratio: f32,
    #[serde(default = "default_ocr_max_boxes")]
    pub max_boxes: usize,
```

`PdfOcrCfg::defaults()` 에도 동일 필드 채움:

```rust
            det_model: None,
            rec_model: None,
            dict: None,
            score_thresh: default_ocr_score_thresh(),
            unclip_ratio: default_ocr_unclip_ratio(),
            max_boxes: default_ocr_max_boxes(),
```

- [ ] **Step 4: float 직렬화 헬퍼 추가 + 적용**

`crates/kebab-config/src/lib.rs` 상단 함수 영역에:

```rust
/// f32 의 shortest round-trip(Display)을 f64 로 재파싱해 직렬화한다.
/// `0.3_f32` 가 `0.30000001192092896` 으로 새지 않고 `0.3` 으로 출력되게 한다.
fn ser_f32_clean<S>(v: &f32, s: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let clean: f64 = format!("{v}").parse().unwrap_or(f64::from(*v));
    s.serialize_f64(clean)
}
```

다음 f32 필드에 `#[serde(serialize_with = "ser_f32_clean")]` 부착: `RagCfg::score_gate`, `RagCfg::nli_threshold`, `LlmCfg::temperature`, `OcrCfg::score_thresh`, `OcrCfg::unclip_ratio`, `PdfOcrCfg::score_thresh`, `PdfOcrCfg::unclip_ratio`, `PdfOcrCfg::valid_ratio_threshold`. (기존 `#[serde(default = ...)]` 와 병기.)

- [ ] **Step 5: kebab-config 자체 테스트 갱신**

같은 파일 `mod tests` / `mod fb27_tests` 에서 옛 경로를 새 경로로 치환:
- `c.image.ocr` → `c.ingest.image.ocr`, `c.image.caption` → `c.ingest.image.caption`
- `bumped.chunking.*` → `bumped.ingest.chunking.*`, `base.chunking` → `base.ingest.chunking`
- `ImageCfg::defaults()` 비교 테스트(`c.image` → `c.ingest.image`)
- `LEGACY_PRE_TIMEOUT_TOML` / `pre_p6_config...` fixture 는 **v2 형태 그대로 두되**, 이들이 `from_str::<Config>` 로 직접 파싱되므로 T6 자동변환 전까지는 실패한다 → 이 두 테스트는 T6 에서 "migrate_document 경유" 로 고친다. T1 단계에선 `#[ignore = "v3: T6 자동변환 후 복구"]` 부착.
- `workspace_cfg_has_only_root_and_exclude_fields` 등 무관 테스트는 유지.

- [ ] **Step 6: 빌드 + kebab-config 테스트**

Run: `cargo test -p kebab-config -j 8 2>&1 | tail -20`
Expected: `v3_layout_nests_media_under_ingest` PASS, ignore 2건 외 green.

- [ ] **Step 7: 커밋**

```bash
git add crates/kebab-config/src/lib.rs
git commit -m "refactor(config): v3 레이아웃 — 미디어 ingest 통합 + pdf paddle 대칭 + float 직렬화"
```

---

## Task 2: call-site sweep (컴파일 복구)

**Files:**
- Modify: `crates/kebab-app/src/lib.rs`, `crates/kebab-app/src/app.rs`, `crates/kebab-app/src/schema.rs`, `crates/kebab-eval/src/runner.rs`

leaf 구조체는 그대로라 변경은 전부 "부모 경로에 `.ingest` 삽입". 컴파일러가 누락을 강제한다.

- [ ] **Step 1: 빌드해 에러 목록 확보**

Run: `cargo build -p kebab-app -p kebab-eval -j 8 2>&1 | rg "no field" | head -40`
Expected: `config.chunking` / `config.image` / `config.pdf` 류 다수.

- [ ] **Step 2: 기계적 치환 (비테스트 src)**

다음 치환을 `kebab-app/src/lib.rs`, `app.rs`, `schema.rs`, `kebab-eval/src/runner.rs` 에 적용(문자열 단위, 정규식 아님):
- `config.chunking.` → `config.ingest.chunking.`
- `config.image.ocr` → `config.ingest.image.ocr`
- `config.image.caption` → `config.ingest.image.caption`
- `config.pdf.ocr` → `config.ingest.pdf.ocr`
- `app.config.image.ocr` → `app.config.ingest.image.ocr`
- `app.config.image.caption` → `app.config.ingest.image.caption`
- `app.config.pdf.ocr` → `app.config.ingest.pdf.ocr`
- `self.config.chunking.` → `self.config.ingest.chunking.`
- `cfg.chunking.` → `cfg.ingest.chunking.`
- `config.indexing.` → `config.ingest.` (해당 site 있으면)

기지 site: lib.rs 363/368/383/838/872/874/2232/2236-2240/3059-3062/3143/3156/3166, app.rs 927/1028, schema.rs 208, runner.rs 223. (3102 는 T3 에서 별도 처리.)

- [ ] **Step 3: 빌드 통과 확인 (3102 제외 일시 에러 가능)**

Run: `cargo build -p kebab-app -p kebab-eval -j 8 2>&1 | tail -15`
Expected: 남은 에러가 있으면 `config.image.ocr` (line ~3102, `ocr_engine_version_for_sig`) 한 곳 — T3 에서 처리. 그 외 0.

- [ ] **Step 4: 커밋**

```bash
git add crates/kebab-app/src crates/kebab-eval/src
git commit -m "refactor(config): v3 경로 call-site sweep (kebab-app/kebab-eval)"
```

---

## Task 3: `ingest_config_signature` 불변 + `ocr_engine_version_for_sig` 미디어화

**Files:**
- Modify: `crates/kebab-app/src/lib.rs`

signature 출력 문자열은 v2 와 **바이트 동일**해야 한다(값 기반). pdf paddle 경로 소스를 image 가 아닌 **pdf 자신**으로 바꾸되, 동작 보존은 T5 의 값 복사로 성립.

- [ ] **Step 1: signature 골든 실패 테스트 추가**

`crates/kebab-app/tests/config_invalidation.rs` 에 추가(없으면 신규 `signature_golden` 테스트):

```rust
#[test]
fn ingest_signature_image_paddle_byte_stable() {
    let mut cfg = kebab_config::Config::defaults();
    cfg.ingest.image.ocr.enabled = true;
    cfg.ingest.image.ocr.engine = "paddle-onnx".into();
    let sig = kebab_app::test_ingest_config_signature(&cfg, &kebab_core::MediaType::Image("png".into()));
    // 골든: 형식 보존(chunk:... |ocr:1:paddle-onnx:<engine_version> |cap:0)
    assert!(sig.starts_with("chunk:500:80:true:md-heading-v1"), "got: {sig}");
    assert!(sig.contains("|ocr:1:paddle-onnx:"), "got: {sig}");
    assert!(sig.ends_with("|cap:0"), "got: {sig}");
}
```

`kebab-app/src/lib.rs` 에 테스트 seam 추가(이미 있으면 생략):

```rust
#[doc(hidden)]
pub fn test_ingest_config_signature(c: &kebab_config::Config, m: &MediaType) -> String {
    ingest_config_signature(c, m)
}
```

- [ ] **Step 2: 테스트 실패/컴파일 에러 확인**

Run: `cargo test -p kebab-app ingest_signature_image_paddle_byte_stable -j 8 2>&1 | tail`
Expected: 컴파일 에러(3102 미수정) 또는 FAIL.

- [ ] **Step 3: `ocr_engine_version_for_sig` 를 paddle 경로 인자화**

현재 시그니처(내부에서 `config.image.ocr` 읽음)를 다음으로 교체:

```rust
/// T9/v3: OCR engine_version. paddle 경로(det/rec/dict)는 호출자가 미디어별로
/// 넘긴다(image 는 image.ocr, pdf 는 pdf.ocr) — v2 의 "pdf 가 image paddle 을
/// 빌려쓰던" 비대칭 제거. 마이그레이션(T5)이 pdf 대칭 키를 image 값으로 채워
/// 기존 signature 와 바이트 동일 유지.
fn ocr_engine_version_for_sig(
    config: &kebab_config::Config,
    engine: &str,
    model: &str,
    det: Option<&str>,
    rec: Option<&str>,
    dict: Option<&str>,
) -> String {
    if engine != PADDLE_ONNX_ENGINE {
        return format!("ollama/{model}");
    }
    let key = format!(
        "{}|{}|{}",
        det.unwrap_or("<bundled>"),
        rec.unwrap_or("<bundled>"),
        dict.unwrap_or("<bundled>"),
    );
    let memo = PADDLE_OCR_VERSION_MEMO
        .get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    if let Some(v) = memo.lock().unwrap().get(&key) {
        return v.clone();
    }
    let version = engine_version_for_config(config).unwrap_or_else(|e| {
        tracing::warn!(
            target: "kebab-app::ingest",
            error = %e,
            "paddle-onnx engine_version hash failed; using path-derived identity for signature"
        );
        format!("ppocrv5-mobile-kor-paths:{key}")
    });
    memo.lock().unwrap().insert(key, version.clone());
    version
}
```

> 주: `engine_version_for_config` 가 내부에서 `config.image.ocr` 의 경로로 hash 한다면, 그것도 호출자가 넘긴 det/rec/dict 를 쓰도록 인자화하거나, image/pdf 가 같은 값일 때(마이그레이션 보장)만 정확. T5 의 값 복사로 image==pdf paddle 이 보장되므로, 최소 변경으로 `engine_version_for_config` 는 image.ocr 기준 유지 가능. 단 pdf 가 image 와 다른 override 를 **사용자가 명시**한 경우를 위해, 가능하면 `engine_version_for_config(config, det, rec, dict)` 로 인자화(권장). 구현 시 `engine_version_for_config` 시그니처 확인 후 결정.

`ingest_config_signature` 의 두 호출부 갱신:

```rust
        // image 분기
            let ocr = &config.ingest.image.ocr;
            // ...
                ocr_engine_version_for_sig(
                    config, &ocr.engine, &ocr.model,
                    ocr.det_model.as_deref(), ocr.rec_model.as_deref(), ocr.dict.as_deref(),
                )
        // pdf 분기
            let ocr = &config.ingest.pdf.ocr;
            // ...
                ocr_engine_version_for_sig(
                    config, &ocr.engine, &ocr.model,
                    ocr.det_model.as_deref(), ocr.rec_model.as_deref(), ocr.dict.as_deref(),
                )
```

- [ ] **Step 4: 테스트 통과**

Run: `cargo test -p kebab-app ingest_signature -j 8 2>&1 | tail`
Expected: PASS.

- [ ] **Step 5: 전체 kebab-app 빌드/테스트(회귀 확인)**

Run: `cargo test -p kebab-app -j 8 2>&1 | rg "test result|error\[|FAILED" | tail -30`
Expected: green (기존 signature 의존 테스트 포함).

- [ ] **Step 6: 커밋**

```bash
git add crates/kebab-app/src/lib.rs crates/kebab-app/tests/config_invalidation.rs
git commit -m "refactor(config): signature paddle 경로 미디어화 + 바이트 불변 골든"
```

---

## Task 4: per-option 주석 인프라 (`key_comment`)

**Files:**
- Modify: `crates/kebab-config/src/migrate.rs`

- [ ] **Step 1: 실패 테스트 추가**

`migrate.rs` `mod tests` 에:

```rust
#[test]
fn annotated_default_has_per_key_comments() {
    let text = annotated_default_document().to_string();
    // 대표 키 주석 존재.
    assert!(text.contains("# 색인 루트"), "workspace.root 주석 누락:\n{text}");
    assert!(text.contains("0=즉시실패"), "request_timeout 주석 누락");
    assert!(text.contains("paddle-onnx 는 번들 모델"), "ocr.model 주석 누락");
}
```

- [ ] **Step 2: 실패 확인**

Run: `cargo test -p kebab-config annotated_default_has_per_key_comments -j 8 2>&1 | tail`
Expected: FAIL.

- [ ] **Step 3: `key_comment` + annotate 확장 구현**

`migrate.rs` 에 `section_comment` 자매로 추가(dotted leaf 경로 → 인라인 주석 텍스트, `#` 없이):

```rust
/// leaf 키 인라인 주석. dotted path(예: `ingest.chunking.target_tokens`) → 한 줄.
fn key_comment(path: &str) -> Option<&'static str> {
    Some(match path {
        "workspace.root" => "색인 루트. 절대/~/${VAR}/상대(=이 파일 기준).",
        "workspace.exclude" => "denylist glob.",
        "storage.copy_threshold_mb" => "이 크기 초과 파일은 사본 대신 참조.",
        "models.embedding.provider" => "fastembed | candle | ollama | none.",
        "models.embedding.dimensions" => "모델 출력 차원. 틀리면 검색 0건.",
        "models.embedding.num_threads" => "candle 전용 CPU 스레드 cap(0=auto).",
        "models.embedding.endpoint" => "ollama provider 시 HTTP. 비우면 llm.endpoint fallback.",
        "models.llm.request_timeout_secs" => "단일 HTTP 상한. 0=즉시실패(비활성화 아님).",
        "ingest.max_parallel_extractors" => "동시 extractor 수.",
        "ingest.max_parallel_embeddings" => "동시 임베딩 수.",
        "ingest.chunking.target_tokens" => "청크 목표 토큰(전 형식 공통).",
        "ingest.chunking.respect_markdown_headings" => "markdown heading 경계 존중.",
        "ingest.image.ocr.enabled" => "이미지 OCR(기본 off, asset 당 비용).",
        "ingest.image.ocr.engine" => "ollama-vision | paddle-onnx.",
        "ingest.image.ocr.model" => "ollama-vision 전용. paddle-onnx 는 번들 모델 사용(이 값 무시).",
        "ingest.image.ocr.request_timeout_secs" => "0=즉시실패(비활성화 아님).",
        "ingest.image.ocr.score_thresh" => "DBNet box 점수 하한(paddle).",
        "ingest.image.ocr.unclip_ratio" => "box 패딩 비율(paddle).",
        "ingest.image.ocr.max_boxes" => "이미지당 box cap(paddle).",
        "ingest.image.caption.enabled" => "이미지 캡션(기본 off).",
        "ingest.pdf.ocr.enabled" => "scanned PDF OCR(기본 off, page 당 비용).",
        "ingest.pdf.ocr.always_on" => "true=모든 page vision 호출(dual-text).",
        "ingest.pdf.ocr.engine" => "ollama-vision | paddle-onnx.",
        "ingest.pdf.ocr.model" => "ollama-vision 전용. paddle-onnx 는 번들 모델 사용.",
        "ingest.pdf.ocr.valid_ratio_threshold" => "유효문자 비율 < 이면 scanned 판정.",
        "ingest.pdf.ocr.min_char_count" => "page 문자수 < 이면 auto-scanned.",
        "ingest.pdf.ocr.request_timeout_secs" => "0=즉시실패(비활성화 아님).",
        "rag.score_gate" => "검색 점수 게이트.",
        "rag.nli_threshold" => "0=NLI 게이트 off.",
        "search.default_k" => "기본 검색 결과 수.",
        "ui.theme" => "dark | light.",
        "logging.ingest_log_enabled" => "ingest 로그(기본 on).",
        _ => return None,
    })
}
```

`annotate_table` 의 leaf 처리 분기 추가(현재는 sub-table 만 주석). 키가 테이블이 아니고 `key_comment` 가 있으면 인라인 suffix 주석 부착:

```rust
        if let Some(item) = table.get_mut(&key) {
            if let Some(sub) = item.as_table_mut() {
                if let Some(c) = section_comment(&path) {
                    sub.decor_mut().set_prefix(format!("\n{c}\n"));
                }
                annotate_table(sub, &path);
            } else if let Some(kc) = key_comment(&path) {
                // 스칼라 leaf: 값 뒤 인라인 주석.
                if let Some(kv) = table.key_mut(&key) {
                    // toml_edit: value 의 decor suffix 로 " # ..." 부착.
                }
                if let Some(v) = table.get_mut(&key).and_then(Item::as_value_mut) {
                    v.decor_mut().set_suffix(format!("  # {kc}"));
                }
            }
        }
```

> 구현 주의: toml_edit 의 value suffix 에 주석을 넣으면 그 줄 끝에 `# ...` 가 붙는다. 배열(`exclude`, `languages`)은 멀티라인 직렬화될 수 있어 suffix 가 닫는 `]` 뒤로 가도 유효. 컴파일 후 `annotated_default_document().to_string()` 출력으로 육안 확인.

- [ ] **Step 4: 테스트 통과 + 라운드트립 유지**

Run: `cargo test -p kebab-config -j 8 2>&1 | rg "test result|FAILED|annotated"`
Expected: `annotated_default_has_per_key_comments` PASS, `annotated_default_has_all_sections_and_parses_back_to_defaults` 여전히 PASS(주석 추가가 파싱을 깨지 않음).

- [ ] **Step 5: 커밋**

```bash
git add crates/kebab-config/src/migrate.rs
git commit -m "feat(config): per-option 인라인 주석(key_comment) — init/reconcile 부착"
```

---

## Task 5: `step_2_to_3` 마이그레이션 (테이블 relocation)

**Files:**
- Modify: `crates/kebab-config/src/migrate.rs`

- [ ] **Step 1: 실패 테스트 추가**

`migrate.rs` `mod tests` 에:

```rust
#[test]
fn step_2_to_3_relocates_media_tables() {
    let v2 = "\
schema_version = 2

[indexing]
max_parallel_extractors = 4
watch_filesystem = true

[chunking]
target_tokens = 700

[image.ocr]
enabled = true
engine = \"paddle-onnx\"
det_model = \"/custom/det.onnx\"

[image.caption]
enabled = true

[pdf.ocr]
enabled = false
engine = \"paddle-onnx\"
";
    let mut doc: toml_edit::DocumentMut = v2.parse().unwrap();
    let mut changes = Vec::new();
    step_2_to_3(&mut doc, &mut changes);
    let out = doc.to_string();
    // 새 위치 존재.
    assert!(out.contains("[ingest]"), "{out}");
    assert!(out.contains("max_parallel_extractors = 4"));
    assert!(out.contains("watch_filesystem = true"));
    assert!(out.contains("[ingest.chunking]"));
    assert!(out.contains("target_tokens = 700"));
    assert!(out.contains("[ingest.image.ocr]"));
    assert!(out.contains("det_model = \"/custom/det.onnx\""));
    assert!(out.contains("[ingest.image.caption]"));
    assert!(out.contains("[ingest.pdf.ocr]"));
    // 옛 위치 제거.
    assert!(!out.contains("[indexing]"));
    assert!(!out.contains("[chunking]"));
    assert!(!out.contains("\n[image]") && !out.contains("[image.ocr]"));
    assert!(!out.contains("[pdf.ocr]") || out.contains("[ingest.pdf.ocr]"));
    // pdf paddle 동작 보존: image paddle det_model 이 pdf 대칭 키로 복사.
    // (pdf.ocr 가 paddle 이고 자체 det_model 없으므로 image 값 복사)
    let reparsed: toml_edit::DocumentMut = out.parse().unwrap();
    let pdf_det = reparsed["ingest"]["pdf"]["ocr"].get("det_model");
    assert_eq!(pdf_det.and_then(|v| v.as_str()), Some("/custom/det.onnx"));
    // 멱등.
    let mut again = changes_after_second_pass(&out);
    assert!(again.is_empty(), "not idempotent: {again:?}");
}

fn changes_after_second_pass(text: &str) -> Vec<MigrationChange> {
    let mut doc: toml_edit::DocumentMut = text.parse().unwrap();
    let mut ch = Vec::new();
    step_2_to_3(&mut doc, &mut ch);
    ch
}
```

- [ ] **Step 2: 실패 확인**

Run: `cargo test -p kebab-config step_2_to_3_relocates_media_tables -j 8 2>&1 | tail`
Expected: FAIL (`step_2_to_3` 미정의).

- [ ] **Step 3: `step_2_to_3` 구현**

`migrate.rs` 에 헬퍼 + step 추가:

```rust
/// `from.remove(key)` 한 Item 을 `to` 의 dotted 경로에 삽입(중간 테이블 자동 생성).
/// 대상 키가 이미 있으면 덮어쓰지 않음(사용자 명시 우선).
fn move_table(
    doc: &mut DocumentMut,
    from_path: &[&str],
    to_path: &[&str],
    changes: &mut Vec<MigrationChange>,
) {
    // from 의 부모까지 내려가 마지막 키를 remove.
    let (from_parent, from_key) = from_path.split_at(from_path.len() - 1);
    let mut cur = doc.as_table_mut();
    for k in from_parent {
        match cur.get_mut(k).and_then(Item::as_table_mut) {
            Some(t) => cur = t,
            None => return, // 원본 없음 → no-op (멱등).
        }
    }
    let Some(item) = cur.remove(from_key[0]) else { return };

    // to 경로의 부모 테이블 확보(없으면 생성), 마지막 키에 삽입.
    let (to_parent, to_key) = to_path.split_at(to_path.len() - 1);
    let mut cur = doc.as_table_mut();
    for k in to_parent {
        if cur.get(k).is_none() {
            cur.insert(k, Item::Table(toml_edit::Table::new()));
        }
        cur = cur.get_mut(k).and_then(Item::as_table_mut).expect("just inserted");
    }
    if cur.get(to_key[0]).is_none() {
        cur.insert(to_key[0], item);
        changes.push(MigrationChange {
            kind: ChangeKind::AddedSection,
            path: to_path.join("."),
            detail: format!("{} → {}", from_path.join("."), to_path.join(".")),
        });
    }
}

/// `[indexing]` 의 bare 스칼라 키들을 `[ingest]` 로 옮긴다(테이블 자체가 아니라 키).
fn move_indexing_keys(doc: &mut DocumentMut, changes: &mut Vec<MigrationChange>) {
    let Some(idx) = doc.as_table_mut().remove("indexing") else { return };
    let Some(idx_tbl) = idx.as_table().cloned() else { return };
    // ingest 테이블 확보.
    if doc.get("ingest").is_none() {
        doc["ingest"] = Item::Table(toml_edit::Table::new());
    }
    let ingest = doc["ingest"].as_table_mut().expect("ingest table");
    for (k, v) in idx_tbl.iter() {
        if ingest.get(k).is_none() {
            ingest.insert(k, v.clone());
        }
    }
    changes.push(MigrationChange {
        kind: ChangeKind::AddedKey,
        path: "ingest".to_string(),
        detail: "indexing → ingest (병렬도 키)".to_string(),
    });
}

/// v2 → v3: 미디어 테이블을 [ingest.*] 로 relocation.
pub fn step_2_to_3(doc: &mut DocumentMut, changes: &mut Vec<MigrationChange>) {
    move_indexing_keys(doc, changes);
    move_table(doc, &["chunking"], &["ingest", "chunking"], changes);
    move_table(doc, &["image", "ocr"], &["ingest", "image", "ocr"], changes);
    move_table(doc, &["image", "caption"], &["ingest", "image", "caption"], changes);
    move_table(doc, &["pdf", "ocr"], &["ingest", "pdf", "ocr"], changes);

    // 빈 껍데기 [image] / [pdf] 제거.
    for empty in ["image", "pdf"] {
        if let Some(t) = doc.get(empty).and_then(Item::as_table) {
            if t.is_empty() {
                doc.as_table_mut().remove(empty);
            }
        }
    }

    // pdf paddle 동작 보존: v2 는 pdf paddle 이 image.ocr 의 경로를 빌려썼다.
    // 이동 후 image.ocr 의 paddle 6키 실제 값을 pdf.ocr 대칭 키로 복사
    // (pdf 가 해당 키를 이미 명시한 경우 덮어쓰지 않음).
    copy_image_paddle_to_pdf(doc);
}

fn copy_image_paddle_to_pdf(doc: &mut DocumentMut) {
    const PADDLE_KEYS: [&str; 6] =
        ["det_model", "rec_model", "dict", "score_thresh", "unclip_ratio", "max_boxes"];
    // image.ocr 값 스냅샷.
    let img = doc
        .get("ingest").and_then(|i| i.get("image")).and_then(|i| i.get("ocr"))
        .and_then(Item::as_table).cloned();
    let Some(img) = img else { return };
    // pdf.ocr 가 paddle 일 때만(없으면 skip).
    let pdf_is_paddle = doc
        .get("ingest").and_then(|i| i.get("pdf")).and_then(|i| i.get("ocr"))
        .and_then(|o| o.get("engine")).and_then(Item::as_str) == Some("paddle-onnx");
    if !pdf_is_paddle { return; }
    let Some(pdf) = doc["ingest"]["pdf"]["ocr"].as_table_mut() else { return };
    for k in PADDLE_KEYS {
        if pdf.get(k).is_none() {
            if let Some(v) = img.get(k) {
                pdf.insert(k, v.clone());
            }
        }
    }
}
```

`run_steps` 에 step 등록:

```rust
fn run_steps(doc: &mut DocumentMut, from: u32, changes: &mut Vec<MigrationChange>) {
    if from < 2 {
        step_1_to_2(doc, changes);
    }
    if from < 3 {
        step_2_to_3(doc, changes);
    }
}
```

`CURRENT_SCHEMA_VERSION` 을 3 으로 bump:

```rust
pub const CURRENT_SCHEMA_VERSION: u32 = 3;
```

- [ ] **Step 4: 테스트 통과 + 기존 migrate 테스트 갱신**

기존 `migrate_document_stamps_version_and_is_idempotent` 등은 `to_schema_version == 3` 기대로 자동 갱신됨(상수 참조). `annotated_default_has_all_sections_and_parses_back_to_defaults` 의 섹션 목록을 v3 로: `[ingest.chunking]`, `[ingest.image.ocr]`, `[ingest.image.caption]`, `[ingest.pdf.ocr]`, `[ingest.code]` 포함되도록 수정.

Run: `cargo test -p kebab-config -j 8 2>&1 | rg "test result|FAILED"`
Expected: green.

- [ ] **Step 5: 커밋**

```bash
git add crates/kebab-config/src/migrate.rs
git commit -m "feat(config): step_2_to_3 — 미디어 테이블 [ingest.*] relocation + pdf paddle 값 보존"
```

---

## Task 6: load 시 메모리 내 자동 변환 (`from_file`)

**Files:**
- Modify: `crates/kebab-config/src/lib.rs`

- [ ] **Step 1: 실패 테스트 추가**

`mod tests` 에:

```rust
#[test]
fn from_file_auto_migrates_v2_in_memory() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("config.toml");
    std::fs::write(&p, "\
schema_version = 2

[workspace]
root = \"/my/notes\"
exclude = []

[chunking]
target_tokens = 777

[image.ocr]
enabled = true
engine = \"ollama-vision\"
model = \"gemma4:e4b\"
languages = [\"kor\"]
max_pixels = 1600
").unwrap();
    let c = Config::from_file(&p).expect("v2 auto-migrate load");
    // 사용자 v2 값이 새 경로로 살아있어야(기본값 유실 X).
    assert_eq!(c.ingest.chunking.target_tokens, 777);
    assert!(c.ingest.image.ocr.enabled);
    assert_eq!(c.ingest.image.ocr.languages, vec!["kor"]);
    // 디스크 파일은 안 바뀜(여전히 schema_version = 2).
    let on_disk = std::fs::read_to_string(&p).unwrap();
    assert!(on_disk.contains("schema_version = 2"), "파일이 변경됨:\n{on_disk}");
    assert!(on_disk.contains("[chunking]"), "파일이 변경됨");
}
```

T1 에서 `#[ignore]` 단 `legacy_config_without_request_timeout_secs_uses_default` 등 2건을 "migrate 경유" 로 복구: `toml::from_str::<Config>(LEGACY...)` → `Config::from_file` 경유가 아니므로, 그 테스트들은 v2 raw 를 직접 파싱한다. 해결: 그 테스트들을 `migrate::migrate_document(LEGACY).new_text` 를 파싱하도록 변경하고 `#[ignore]` 제거.

```rust
#[test]
fn legacy_config_without_request_timeout_secs_uses_default() {
    let migrated = crate::migrate::migrate_document(LEGACY_PRE_TIMEOUT_TOML).new_text;
    let c: Config = toml::from_str(&migrated).expect("parse migrated legacy config");
    assert_eq!(c.models.llm.request_timeout_secs, 300);
}
```

(OCR 측·multi_hop·nli·pre_p6 테스트 동일 패턴으로 `migrate_document` 경유.)

- [ ] **Step 2: 실패 확인**

Run: `cargo test -p kebab-config from_file_auto_migrates_v2_in_memory -j 8 2>&1 | tail`
Expected: FAIL.

- [ ] **Step 3: `from_file` 자동 변환 구현**

기존 `from_file` 의 파싱부를 다음으로 교체(legacy include 경고 블록 유지):

```rust
    pub fn from_file(path: &Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path).map_err(|e| {
            anyhow::Error::new(ConfigInvalid {
                path: path.to_path_buf(),
                cause: format!("read_failed: {e}"),
            })
        })?;

        // (기존 workspace.include deprecation 경고 블록 유지)

        // v3: 파일의 schema_version < CURRENT 면 메모리에서 마이그레이션
        // (디스크 미변경 — 파일 갱신은 `kebab config migrate`).
        let parse_text = {
            let from = toml::from_str::<toml::Value>(&text)
                .ok()
                .and_then(|v| v.get("schema_version").and_then(toml::Value::as_integer))
                .unwrap_or(1) as u32;
            if from < crate::migrate::CURRENT_SCHEMA_VERSION {
                static MIGRATE_WARNED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
                MIGRATE_WARNED.get_or_init(|| {
                    tracing::warn!(
                        target: "kebab-config",
                        config = %path.display(),
                        from,
                        to = crate::migrate::CURRENT_SCHEMA_VERSION,
                        "config 가 옛 스키마입니다 — 이번 실행은 메모리에서 변환됨. 파일 갱신: `kebab config migrate`."
                    );
                });
                crate::migrate::migrate_document(&text).new_text
            } else {
                text.clone()
            }
        };

        let mut cfg: Self = toml::from_str(&parse_text).map_err(|e| {
            anyhow::Error::new(ConfigInvalid {
                path: path.to_path_buf(),
                cause: format!("parse_failed: {e}"),
            })
        })?;
        cfg.source_dir = path.parent().map(Path::to_path_buf);
        Ok(cfg)
    }
```

- [ ] **Step 4: 테스트 통과**

Run: `cargo test -p kebab-config -j 8 2>&1 | rg "test result|FAILED"`
Expected: green (ignore 0건 — T1 의 2건 복구됨).

- [ ] **Step 5: 커밋**

```bash
git add crates/kebab-config/src/lib.rs
git commit -m "feat(config): from_file load 시 v2→v3 메모리 내 자동 변환(디스크 미변경)"
```

---

## Task 7: 무손실 골든 (사용자 실제 v2 config)

**Files:**
- Create: `crates/kebab-config/tests/migrate_v3.rs`
- Create: `crates/kebab-config/tests/fixtures/user_v2_config.toml`

- [ ] **Step 1: 사용자 실제 config 를 fixture 로 저장**

`crates/kebab-config/tests/fixtures/user_v2_config.toml` 생성 — brainstorming 에서 사용자가 첨부한 실제 v2 파일 전문(주석·대안 줄·`score_gate = 0.30000001192092896` 포함) 그대로. (값 보존 검증의 기준.)

- [ ] **Step 2: 무손실 통합 테스트 작성**

```rust
//! v3 마이그레이션 무손실 골든 — 사용자 실제 v2 config.
use kebab_config::migrate::migrate_document;

const USER_V2: &str = include_str!("fixtures/user_v2_config.toml");

#[test]
fn user_v2_migrates_losslessly() {
    let out = migrate_document(USER_V2);
    assert_eq!(out.from_schema_version, 2);
    assert_eq!(out.to_schema_version, 3);
    let t = &out.new_text;

    // 사용자 값 보존.
    assert!(t.contains("root = \"/Users/user/Obsidian/Default\""), "{t}");
    assert!(t.contains("model = \"snowflake-arctic-embed2\""));
    assert!(t.contains("endpoint = \"http://192.168.0.2:11943\""));
    // 사용자 주석/대안 줄 보존.
    assert!(t.contains("# engine = \"ollama-vision\""), "대안 주석 유실:\n{t}");
    assert!(t.contains("# provider = \"candle\""));
    // 새 위치.
    assert!(t.contains("[ingest.image.ocr]"));
    assert!(t.contains("[ingest.pdf.ocr]"));
    assert!(t.contains("[ingest.chunking]"));
    assert!(!t.contains("\n[chunking]"));
    assert!(!t.contains("\n[image.ocr]"));

    // v3 Config 로 parse + 값 동일.
    let cfg: kebab_config::Config = toml::from_str(t).expect("v3 parse");
    assert!(cfg.ingest.image.ocr.enabled);
    assert_eq!(cfg.ingest.image.ocr.engine, "paddle-onnx");
    assert_eq!(cfg.models.embedding.model, "snowflake-arctic-embed2");
    assert_eq!(cfg.models.llm.endpoint, "http://192.168.0.2:11943");

    // 멱등.
    let again = migrate_document(t);
    assert!(!again.changed(), "재실행 변경: {:?}", again.changes);
    assert_eq!(again.new_text, *t);
}
```

`migrate` 모듈이 `pub` 인지 확인(`kebab-config/src/lib.rs` 에 `pub mod migrate;` 이미 존재). `MigrationOutcome`/`migrate_document` 가 crate-public 인지 확인.

- [ ] **Step 3: 테스트 실행**

Run: `cargo test -p kebab-config --test migrate_v3 -j 8 2>&1 | tail -20`
Expected: PASS. 실패 시 toml_edit relocation 의 주석 보존 결함 → `step_2_to_3` 의 `move_table` 가 `Item` 통째 이동(decor 포함)인지 점검.

- [ ] **Step 4: 커밋**

```bash
git add crates/kebab-config/tests/
git commit -m "test(config): v3 무손실 골든 — 사용자 실제 v2 config relocation+멱등"
```

---

## Task 8: env override 이름 보존 + 신규 pdf paddle env

**Files:**
- Modify: `crates/kebab-config/src/lib.rs`

`apply_env` whitelist 의 **키 문자열(LHS)은 전부 그대로**, 대입 대상(RHS)만 새 경로. T2 와 달리 이건 kebab-config 내부.

- [ ] **Step 1: 기존 env 이름 보존 테스트 추가**

```rust
#[test]
fn env_names_preserved_target_new_paths() {
    let mut env = HashMap::new();
    env.insert("KEBAB_CHUNKING_TARGET_TOKENS".into(), "640".into());
    env.insert("KEBAB_INDEXING_MAX_PARALLEL_EXTRACTORS".into(), "6".into());
    env.insert("KEBAB_IMAGE_OCR_ENABLED".into(), "true".into());
    env.insert("KEBAB_PDF_OCR_ENGINE".into(), "paddle-onnx".into());
    let c = Config::defaults().apply_env(&env);
    assert_eq!(c.ingest.chunking.target_tokens, 640);
    assert_eq!(c.ingest.max_parallel_extractors, 6);
    assert!(c.ingest.image.ocr.enabled);
    assert_eq!(c.ingest.pdf.ocr.engine, "paddle-onnx");
}

#[test]
fn env_pdf_paddle_symmetric_overrides() {
    let mut env = HashMap::new();
    env.insert("KEBAB_PDF_OCR_DET_MODEL".into(), "/d.onnx".into());
    env.insert("KEBAB_PDF_OCR_SCORE_THRESH".into(), "0.4".into());
    env.insert("KEBAB_PDF_OCR_MAX_BOXES".into(), "500".into());
    let c = Config::defaults().apply_env(&env);
    assert_eq!(c.ingest.pdf.ocr.det_model.as_deref(), Some("/d.onnx"));
    assert!((c.ingest.pdf.ocr.score_thresh - 0.4).abs() < 1e-6);
    assert_eq!(c.ingest.pdf.ocr.max_boxes, 500);
}
```

- [ ] **Step 2: 실패 확인**

Run: `cargo test -p kebab-config env_names_preserved -j 8 2>&1 | tail`
Expected: 컴파일 에러(RHS 옛 경로) 또는 FAIL.

- [ ] **Step 3: `apply_env` RHS 갱신 + 신규 키**

기존 모든 `self.chunking.*` → `self.ingest.chunking.*`, `self.indexing.*` → `self.ingest.*`, `self.image.ocr.*` → `self.ingest.image.ocr.*`, `self.image.caption.*` → `self.ingest.image.caption.*`, `self.pdf.ocr.*` → `self.ingest.pdf.ocr.*`. (키 문자열은 불변.)

`KEBAB_PDF_OCR_*` 그룹에 신규 6키 추가(`KEBAB_IMAGE_OCR_*` paddle 패턴 복제):

```rust
                "KEBAB_PDF_OCR_DET_MODEL" => {
                    self.ingest.pdf.ocr.det_model = if v.is_empty() { None } else { Some(v.clone()) };
                }
                "KEBAB_PDF_OCR_REC_MODEL" => {
                    self.ingest.pdf.ocr.rec_model = if v.is_empty() { None } else { Some(v.clone()) };
                }
                "KEBAB_PDF_OCR_DICT" => {
                    self.ingest.pdf.ocr.dict = if v.is_empty() { None } else { Some(v.clone()) };
                }
                "KEBAB_PDF_OCR_SCORE_THRESH" => {
                    if let Ok(f) = v.parse::<f32>() { self.ingest.pdf.ocr.score_thresh = f; }
                }
                "KEBAB_PDF_OCR_UNCLIP_RATIO" => {
                    if let Ok(f) = v.parse::<f32>() { self.ingest.pdf.ocr.unclip_ratio = f; }
                }
                "KEBAB_PDF_OCR_MAX_BOXES" => {
                    if let Ok(n) = v.parse::<usize>() { self.ingest.pdf.ocr.max_boxes = n; }
                }
```

- [ ] **Step 4: 테스트 통과(전 env 테스트 포함)**

Run: `cargo test -p kebab-config -j 8 2>&1 | rg "test result|FAILED"`
Expected: green (기존 `env_override_*` 전부 — 이름 보존이므로 통과).

- [ ] **Step 5: 워크스페이스 clippy + 전체 테스트**

Run:
```bash
export CARGO_TARGET_DIR=/build/out/cargo-target
cargo clippy --workspace --all-targets -j 8 -- -D warnings 2>&1 | tail -3
cargo test -p kebab-config -p kebab-app -p kebab-eval -j 8 2>&1 | rg "test result|FAILED|error\[" | tail -30
```
Expected: clippy 0, 테스트 green.

- [ ] **Step 6: 커밋**

```bash
git add crates/kebab-config/src/lib.rs
git commit -m "feat(config): env 이름 보존 RHS 갱신 + pdf paddle 신규 env 6키"
```

---

## Task 9: 문서 cascade + 버전 bump

**Files:**
- Modify: `README.md`, `docs/SMOKE.md`, `tasks/HOTFIXES.md`, `docs/superpowers/specs/2026-05-31-config-migration-design.md`, `Cargo.toml`

- [ ] **Step 1: README Configuration 섹션 — 새 레이아웃 반영**

`README.md` 의 config 예시/섹션 표를 v3 `[ingest.*]` 구조로 갱신. `[image.ocr]`/`[pdf.ocr]`/`[chunking]` 언급을 `[ingest.image.ocr]`/`[ingest.pdf.ocr]`/`[ingest.chunking]` 로. README:10 의 "OCR/caption family" 문구가 paddle 기준으로 stale 하면 정리. `kebab config migrate` 로 기존 파일 갱신 안내 1줄.

- [ ] **Step 2: `docs/SMOKE.md` config 예시 블록 — v3 로 교체**

SMOKE 의 `/tmp/kebab-smoke/config.toml` 예시를 spec §3 의 v3 레이아웃으로. (CLAUDE.md README sync 규칙: config 예시는 SMOKE 와 동기화.)

- [ ] **Step 3: HOTFIXES dated entry 추가**

`tasks/HOTFIXES.md` 에 `2026-06-04 config v3 재편` 항목: rename 매핑 표, 자동 변환(메모리)·env 이름 보존·signature 불변 보장, `kebab config migrate` 안내. 도그푸딩 evidence 는 T10 후 채움.

- [ ] **Step 4: 선행 spec 교차링크**

`docs/superpowers/specs/2026-05-31-config-migration-design.md` 의 Risks/notes 에 1줄: "2026-06-04 v3 재편(첫 non-additive rename)에서 step_2_to_3 + load 시 메모리 자동변환 추가 — `2026-06-04-config-schema-reorg-design.md`."

- [ ] **Step 5: workspace version minor bump**

`Cargo.toml` workspace `version` 을 현재값에서 **minor** bump(예 `0.27.x` → `0.28.0`). `cargo build` 로 `Cargo.lock` 자동 갱신. (CLAUDE.md: 인터페이스 변경 = minor.)

Run: `cargo build -p kebab-cli -j 8 2>&1 | tail -3`
Expected: 성공, `Cargo.lock` 갱신.

- [ ] **Step 6: 커밋**

```bash
git add README.md docs/SMOKE.md tasks/HOTFIXES.md docs/superpowers/specs/2026-05-31-config-migration-design.md Cargo.toml Cargo.lock
git commit -m "docs(config): v3 재편 surface 동기화 + minor version bump"
```

---

## Task 10: 도그푸딩 (실제 v2 파일 변환 검증)

**Files:**
- Modify: `tasks/HOTFIXES.md`, `docs/release-notes/v<X.Y.Z>-draft.md` (또는 release body)

- [ ] **Step 1: release 바이너리 빌드**

Run:
```bash
export CARGO_TARGET_DIR=/build/out/cargo-target
cargo build --release -j 8 2>&1 | tail -3
```
Expected: `target` → `/build/out/cargo-target/release/kebab` 생성.

- [ ] **Step 2: 실제 v2 config 복사본에 `config migrate` 실행**

```bash
mkdir -p /build/dogfood/config-v3-test
cp crates/kebab-config/tests/fixtures/user_v2_config.toml /build/dogfood/config-v3-test/config.toml
/build/out/cargo-target/release/kebab config migrate --config /build/dogfood/config-v3-test/config.toml
cat /build/dogfood/config-v3-test/config.toml
```
Expected: `[ingest.*]` 구조 + 사용자 값·주석 보존 + `schema_version = 3`. (출력에 변경 요약.)

- [ ] **Step 3: 변환된 config 로 재색인 → 재색인 0 확인**

도그푸딩 KB(`/build/dogfood/kb`) 에 v2 config 로 1회 ingest 후, v3 변환 config 로 재실행하여 `derivation cache: embedding hit=N miss=0` (또는 미미한 miss) 확인 — signature 불변 보장 실증. (CLAUDE.md Dogfood: search/RAG behavior 불변.)

```bash
/build/out/cargo-target/release/kebab ingest --config /build/dogfood/config-v3-test/config.toml 2>&1 | rg "derivation cache|re-index|unchanged" | tail
```
Expected: 두 번째 실행이 대부분 unchanged/hit.

- [ ] **Step 4: evidence 기록**

`tasks/HOTFIXES.md` 2026-06-04 항목에 도그푸딩 결과(변환 전후 diff 요약 + 재색인 0 evidence) 추가. release notes draft 에 4단락(변경/trade-off/mitigation/upgrade 절차) 작성.

- [ ] **Step 5: 커밋**

```bash
git add tasks/HOTFIXES.md docs/release-notes/
git commit -m "docs(config): v3 재편 도그푸딩 evidence + release notes"
```

---

## Self-Review 결과

- **Spec coverage:** §3 새 스키마=T1+T4, §4 매핑=T1+T5, §5.1 구조체=T1, §5.2 sweep=T2, §5.3 자동변환=T6, §6 step_2_to_3=T5, §7 key_comment=T4, §8.1 signature=T3, §8.2 env=T8, §8.3 무손실 골든=T7, §8.4 load=T6, §8.5 float=T1, §9 cascade=T9, §10 리스크=각 task 테스트. 누락 없음.
- **Placeholder scan:** 코드 step 전부 실제 코드 포함. `engine_version_for_config` 인자화는 T3 Step3 주석에서 "구현 시 시그니처 확인 후 결정"으로 명시(구현부가 실재 함수).
- **Type consistency:** `ingest.image.ocr`/`ingest.pdf.ocr`/`ingest.chunking` 경로 전 task 일관. `ocr_engine_version_for_sig`(det/rec/dict 인자) T3 정의 ↔ 호출 일치. `step_2_to_3`/`move_table`/`copy_image_paddle_to_pdf`/`move_indexing_keys` T5 정의 ↔ T6 `run_steps` 호출 일치.
