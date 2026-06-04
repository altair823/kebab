# config 스키마 재편 (v2 → v3): 미디어별 `[ingest]` 통합 + per-option 주석

- 상태: 설계 확정 (brainstorming 완료)
- 작성일: 2026-06-04
- 선행: `docs/superpowers/specs/2026-05-31-config-migration-design.md` (마이그레이션 엔진), `#197`(엔진), `#198`(`kebab config migrate` surface)
- 영향 crate: `kebab-config`(스키마+마이그레이션), `kebab-app`(call-site sweep + signature), `kebab-eval`(config_snapshot), `kebab-cli`(`config migrate`/`init` 출력)
- contract_sections: design §6 (Config schema / XDG), §9 (versioning cascade — signature 불변 보장)

## 1. 동기

옵션이 누적되며 `config.toml`(13 섹션 / ~60 필드)이 다음 군더더기를 갖게 됨:

1. **OCR 중복·비대칭** — `[image.ocr]` 와 `[pdf.ocr]` 가 `enabled/engine/model/endpoint/languages/max_pixels/request_timeout_secs` 를 거의 그대로 중복. 게다가 paddle-onnx 모델 경로(`det_model`/`rec_model`/`dict`/`score_thresh`/`unclip_ratio`/`max_boxes`)는 `[image.ocr]` 에만 존재하고 PDF paddle 경로가 거기를 참조(`kebab-app/src/lib.rs:3102` `ocr_engine_version_for_sig` 가 `config.image.ocr` 를 읽음) — "pdf 설정인데 image 밑을 봐야 하는" 숨은 비대칭.
2. **미디어별 설정 산재** — 이미지 `[image]`, PDF `[pdf]`, 코드 `[ingest.code]`, 청킹 `[chunking]`. "형식 X 설정이 어디 있나"의 규칙이 없음.
3. **`endpoint` 4중복** — `models.llm`/`models.embedding`/`image.ocr`/`pdf.ocr`. "비우면 `models.llm.endpoint` fallback" 규칙이 코드에만 있고 파일엔 안 보임. (단, **컴포넌트별 endpoint 는 실사용 중** — embedding 로컬 + llm 원격 — 이므로 통합 금지.)
4. **`request_timeout_secs` 3중복** + 각각 "`0` 은 비활성화 아님" 함정.
5. **`kebab init` 이 60+ 필드 일괄 방출** — 실제 사용자가 만지는 건 `workspace.root`/endpoint/모델명 정도.
6. 사용자 실파일에서 추가 관찰: `score_gate = 0.30000001192092896`(f32→f64 직렬화 찌꺼기), `engine="paddle-onnx"` 인데 `model="gemma4:e4b"` 가 남는 죽은 필드.

## 2. 목표 / 비목표

**목표**

- 미디어 형식 설정을 `[ingest.*]` 한 우산 아래로 일관 배치 (향후 새 형식 = `[ingest.<형식>]` 한 곳 추가).
- OCR 비대칭 제거: image·pdf 가 **각자 OCR 전체(paddle 경로 포함)를 독립 보유**(완전 대칭).
- **무손실 변환**: 기존 v2 파일의 모든 값·주석·순서·사용자 대안 주석 줄을 보존.
- **per-option 주석**: 각 키 옆 한 줄 설명을 `kebab init` 출력과 신규 추가 키에 부착.
- 업그레이드 시 **불필요한 재색인 0** (parser_version signature 불변).
- env override 이름 **무파손**.

**비목표 (YAGNI)**

- config 값 의미 검증(범위 체크 등) — 별개.
- 다운그레이드(v3→v2).
- 노브 숨기기/축소 — 명시적으로 제외(사용자가 "온전한 변환" 선택). 전 옵션을 잘 문서화한 완전체 유지.
- endpoint 통합 — 컴포넌트별 override 유지(실사용).
- **load 시 파일 자동 쓰기** — 여전히 비목표(2026-05-31 spec 계승). 단 §5.3 의 *메모리 내* 변환은 쓰기가 아니므로 별개로 허용.

## 3. 새 스키마 (v3)

per-option 주석을 부착한 `kebab init` 출력 형태(값은 기본값):

```toml
# kebab config — `~/.config/kebab/config.toml`.
# (헤더: workspace.root 경로 규칙 / 지원 형식 / KEBAB_* override — 기존 헤더 계승)
schema_version = 3

# 색인 대상 워크스페이스.
[workspace]
root = "~/KnowledgeBase"      # 색인 루트. 절대/~/${VAR}/상대(=이 파일 기준).
exclude = [".git/**", "node_modules/**", ".obsidian/**"]  # denylist glob.

# XDG 저장 경로(데이터/sqlite/벡터/에셋/모델).
[storage]
data_dir = "${XDG_DATA_HOME:-~/.local/share}/kebab"  # 모든 산출물 루트.
sqlite = "{data_dir}/kebab.sqlite"   # 메타·FTS5 DB.
vector_dir = "{data_dir}/lancedb"    # 임베딩 벡터 스토어.
asset_dir = "{data_dir}/assets"      # 원본 사본(_external 등).
artifact_dir = "{data_dir}/artifacts"
model_dir = "{data_dir}/models"      # fastembed/candle/nli 모델 캐시.
runs_dir = "{data_dir}/runs"         # eval run 산출.
copy_threshold_mb = 100              # 이 크기 초과 파일은 사본 대신 참조.

# 다국어 sentence embedding. dim 불일치 시 검색 0건.
[models.embedding]
provider = "fastembed"   # fastembed | candle | ollama | none.
model = "multilingual-e5-large"
version = "v1"           # 모델 정체성 일부(캐시 키). 모델 바꾸면 함께 갱신.
dimensions = 1024        # 모델 출력 차원. 틀리면 검색 0건.
batch_size = 64
num_threads = 0          # candle 전용 CPU 스레드 cap(0=auto). NUMA 회피 레버.
# endpoint = "..."       # ollama provider 시 HTTP. 비우면 models.llm.endpoint fallback.

# Ollama host:port + 모델.
[models.llm]
provider = "ollama"
model = "gemma4:e4b"
context_tokens = 32768
endpoint = "http://127.0.0.1:11434"
temperature = 0.0
seed = 0
request_timeout_secs = 300   # 단일 HTTP 상한. 0=즉시실패(비활성화 아님). 대형모델 CPU면 ↑.

# NLI(groundedness) 모델.
[models.nli]
model = "Xenova/mDeBERTa-v3-base-xnli-multilingual-nli-2mil7"
provider = "onnx"

# 색인 공통(병렬도 + 파일시스템 watch).   ← 기존 [indexing]
[ingest]
max_parallel_extractors = 2
max_parallel_embeddings = 1
watch_filesystem = false

# 청크 크기·오버랩·heading 존중 (markdown/pdf/code/image 모든 형식 공통).  ← 기존 [chunking]
[ingest.chunking]
target_tokens = 500
overlap_tokens = 80
respect_markdown_headings = true
chunker_version = "md-heading-v1"

# code ingest skip 정책(.gitignore 자동 honor).
[ingest.code]
skip_generated_header = true
max_file_bytes = 262144
max_file_lines = 5000
extra_skip_globs = []
ast_chunk_max_lines = 200
fallback_lines_per_chunk = 80
fallback_lines_overlap = 20

# 이미지 OCR(기본 off, asset 당 비용).   ← 기존 [image.ocr]
[ingest.image.ocr]
enabled = false
engine = "ollama-vision"     # ollama-vision | paddle-onnx.
model = "gemma4:e4b"         # ollama-vision 전용. paddle-onnx 는 번들 모델 사용(이 값 무시).
languages = ["eng", "kor"]
max_pixels = 1600
request_timeout_secs = 300   # 0=즉시실패(비활성화 아님).
# --- paddle-onnx 전용(engine=paddle-onnx 일 때만) ---
# det_model = "..."          # 비우면 번들 ppocrv5_mobile_det.onnx.
# rec_model = "..."          # 비우면 번들 korean rec.
# dict = "..."               # 비우면 번들 korean_dict.txt.
score_thresh = 0.3           # DBNet box 점수 하한.
unclip_ratio = 1.5           # box 패딩 비율.
max_boxes = 1000             # 이미지당 box cap(runaway guard).

# 이미지 캡션(기본 off).   ← 기존 [image.caption]
[ingest.image.caption]
enabled = false
max_pixels = 768
prompt_template_version = "caption-v1"

# scanned PDF page-단위 OCR(기본 off, page 당 비용).   ← 기존 [pdf.ocr]
[ingest.pdf.ocr]
enabled = false
always_on = false            # true=모든 page vision 호출(vector PDF dual-text).
engine = "ollama-vision"     # ollama-vision | paddle-onnx.
model = "qwen2.5vl:3b"       # ollama-vision 전용. paddle-onnx 는 번들 모델 사용.
languages = ["eng", "kor"]
max_pixels = 2048
request_timeout_secs = 180   # 0=즉시실패(비활성화 아님).
valid_ratio_threshold = 0.5  # 유효문자 비율 < 이면 scanned 로 판정→OCR fallback.
min_char_count = 20          # page 문자수 < 이면 auto-scanned.
lang_hint = "kor"            # 단일 page lang hint(비우면 없음).
# --- paddle-onnx 전용(대칭 신규) ---
# det_model / rec_model / dict = "..."   # 비우면 번들.
score_thresh = 0.3
unclip_ratio = 1.5
max_boxes = 1000

# 검색 기본 k·stale 기준·fusion.
[search]
default_k = 10
hybrid_fusion = "rrf"
rrf_k = 60
snippet_chars = 220
cache_capacity = 256
stale_threshold_days = 30

# 답변 생성: prompt 템플릿·score gate·multi-hop·NLI.
[rag]
prompt_template_version = "rag-v3"
score_gate = 0.3             # serialize_with 헬퍼로 직렬화 깔끔(기존 f32 찌꺼기 제거).
explain_default = false
max_context_tokens = 8000
multi_hop_max_depth = 3
multi_hop_max_sub_queries_per_iter = 5
multi_hop_max_pool_chunks = 15
nli_threshold = 0.0          # 0=NLI 게이트 off.

# TUI 팔레트.
[ui]
theme = "dark"

# ingest 로그(기본 on).
[logging]
ingest_log_enabled = true
ingest_log_dir = "{state_dir}/logs"
keep_recent_runs = 100
retention_days = 30
```

## 4. 필드 매핑 (v2 → v3)

| v2 위치 | v3 위치 | 비고 |
|---------|---------|------|
| `[workspace]` `[storage]` `[search]` `[rag]` `[ui]` `[logging]` `[models.*]` | 동일 | 변경 없음 |
| `[indexing].*` (3키) | `[ingest].*` (bare 키) | `IndexingCfg` 해체 → `IngestCfg` 스칼라 |
| `[chunking]` | `[ingest.chunking]` | 이름 의도적으로 `markdown` 아님(전 형식 공통) |
| `[ingest.code]` | `[ingest.code]` | 이미 nested — 무이동 |
| `[image.ocr]` | `[ingest.image.ocr]` | 키 동일 |
| `[image.caption]` | `[ingest.image.caption]` | 키 동일 |
| `[pdf.ocr]` | `[ingest.pdf.ocr]` | 키 동일 + **paddle 6키 대칭 신규** (`det_model`/`rec_model`/`dict`/`score_thresh`/`unclip_ratio`/`max_boxes`) |

신규 키(pdf paddle 대칭)는 모두 `#[serde(default)]` + `Option`/기본값 → v2 파일에 없어도 무해.

## 5. Rust 구조 변경 (`kebab-config/src/lib.rs`)

### 5.1 구조체

```rust
pub struct Config {
    pub schema_version: u32,
    pub workspace: WorkspaceCfg,
    pub storage: StorageCfg,
    pub models: ModelsCfg,
    pub ingest: IngestCfg,   // ← indexing/chunking/image/pdf 흡수
    pub search: SearchCfg,
    pub rag: RagCfg,
    pub ui: UiCfg,
    pub logging: LoggingCfg,
    #[serde(skip)] source_dir: Option<PathBuf>,
}

pub struct IngestCfg {
    // ← 기존 IndexingCfg (스칼라 먼저: toml 직렬화는 스칼라가 테이블보다 앞)
    pub max_parallel_extractors: u32,
    pub max_parallel_embeddings: u32,
    pub watch_filesystem: bool,
    // 하위 테이블
    pub chunking: ChunkingCfg,
    pub code: IngestCodeCfg,
    pub image: ImageCfg,     // { ocr: OcrCfg, caption: CaptionCfg }
    pub pdf: PdfCfg,         // { ocr: PdfOcrCfg }
}
```

- `IndexingCfg` 구조체 삭제(스칼라로 흡수). `ChunkingCfg`/`ImageCfg`/`OcrCfg`/`CaptionCfg`/`PdfCfg`/`IngestCodeCfg` **내부 필드 불변**(부모 경로만 이동).
- `PdfOcrCfg` 에 paddle 6키 대칭 추가.
- 제거된 top-level 필드: `indexing`/`chunking`/`image`/`pdf`.
- 스칼라-우선 필드 순서로 `defaults_are_serde_roundtrip_stable` 유지.

### 5.2 call-site sweep (~65곳, 7 src 파일)

기계적 치환: `config.chunking.X`→`config.ingest.chunking.X`, `config.image.ocr`→`config.ingest.image.ocr`, `config.pdf.ocr`→`config.ingest.pdf.ocr`, `config.indexing.X`→`config.ingest.X`. 대상: `kebab-app/src/{lib.rs,app.rs,schema.rs}`, `kebab-eval/src/runner.rs`. `kebab-parse-image` 는 leaf 구조체(`&OcrCfg` 등) 직접 수령 → 무영향(확인됨).

### 5.3 load 시 메모리 내 자동 변환 (정합성 필수)

v3 는 최초의 **non-additive rename** 이라, 미변환 v2 파일을 v3 struct 로 deserialize 하면 `[chunking]`/`[image]`/`[pdf]`/`[indexing]` 을 못 찾아 **사용자 설정이 조용히 기본값으로 유실**. (이전 마이그레이션은 전부 additive 라 serde default 로 load 호환됐음 — 이 가정이 v3 에서 처음 깨짐.)

→ `Config::from_file` 변경: 텍스트의 `schema_version < CURRENT` (또는 legacy 테이블 탐지) 시 `migrate::migrate_document(text)` 를 **메모리에서** 적용한 `new_text` 를 deserialize. **디스크 쓰기 없음**(파일 갱신은 여전히 `kebab config migrate` 전용 — 2026-05-31 spec 의 "자동 쓰기 비목표" 계승; 메모리 변환은 쓰기가 아니므로 무충돌). 1회성 `tracing::warn!`: "config 가 schema vN 입니다 — 이번 실행은 메모리에서 v3 로 변환됨. 파일 갱신은 `kebab config migrate`."

- parse 실패 시 `migrate_document` 는 입력 그대로 반환 → 기존 `ConfigInvalid` 경로 유지.
- `source_dir` stamp 는 변환 후 동일하게 `path.parent()`.

## 6. 마이그레이션 `step_2_to_3` (`kebab-config/src/migrate.rs`)

`run_steps` 에 `if from < 3 { step_2_to_3(doc, changes) }` 추가. `step_2_to_3` 는 **테이블 relocation**(toml_edit, 값·키주석·순서 보존):

1. `[indexing]` 의 3키 → `[ingest]` bare 키로 이동. 원 `[indexing]` 제거.
2. `[chunking]` 테이블 → `[ingest.chunking]` 로 이동(통째). 원 제거.
3. `[image.ocr]`→`[ingest.image.ocr]`, `[image.caption]`→`[ingest.image.caption]`. 원 `[image]` 제거.
4. `[pdf.ocr]`→`[ingest.pdf.ocr]`. 원 `[pdf]` 제거.
5. **pdf paddle 동작 보존(중요)** — v2 는 pdf 가 paddle 일 때 `image.ocr` 의 paddle 값(`det_model`/`rec_model`/`dict`/`score_thresh`/`unclip_ratio`/`max_boxes`)을 빌려 썼다(§1 비대칭). 따라서 이동 직후 **`[image.ocr]` 의 이 6키 실제 값을 `[ingest.pdf.ocr]` 의 대칭 키로 복사**한다(사용자가 image paddle 을 튜닝한 경우까지 동작 동일 보장). 사용자가 둘 다 기본이면 복사값=기본값이라 무차. 복사는 사용자가 `[pdf.ocr]` 에 해당 키를 이미 명시한 경우엔 덮어쓰지 않음.
6. 기존 `[ingest.code]` 는 그대로(이미 올바른 위치). 단 `[ingest]` 가 새로 bare 키를 받으므로 직렬화 순서 정합 확인.

이동은 **user item 의 decor(값 뒤 인라인 주석 + 사용자 대안 주석 줄)를 동반**해야 함 — toml_edit 에서 `Table::remove` 로 떼어낸 `Item` 을 새 부모에 `insert`. 멱등(이미 v3 형태면 no-op).

이동 후 기존 `reconcile(annotated_default, doc)` 가:
- 빠진 키(특히 pdf paddle 대칭 6키) 를 주석과 함께 추가.
- `schema_version` stamp → 3.

`CURRENT_SCHEMA_VERSION: u32 = 3` 으로 bump.

## 7. per-option 주석 인프라

- `key_comment(path: &str) -> Option<&'static str>` 신설 (`section_comment` 자매). dotted leaf 경로(`ingest.chunking.target_tokens` 등) → 한 줄.
- `annotate_table` 확장: 스칼라 leaf 에도 `key_comment` 가 있으면 인라인/prefix 주석 부착.
- **부착 범위**: `annotated_default_document`(=`kebab init` + reconcile 참조원) 의 모든 키. reconcile 가 **새로 추가하는** 키만 주석 동반(기존 사용자 키는 값 불가침 → 주석 미주입, 사용자 대안 주석 보존).
- §3 의 모든 키 주석 텍스트를 `key_comment` 에 등재(구현 시 일괄).

## 8. 불변식 / 회귀 가드

1. **signature 불변** — `ingest_config_signature`(lib.rs:3129) 출력 문자열이 v2 바이너리와 **바이트 동일**. 값 기반이라 struct 경로 변경과 무관해야 함. `ocr_engine_version_for_sig` 가 읽는 paddle 경로 소스를 image signature 는 `config.ingest.image.ocr` 로, **pdf signature 는 `config.ingest.pdf.ocr` 의 신규 대칭 키**로 갱신. 동작 보존은 §6.5 의 값 복사(image paddle 값 → pdf 대칭 키)로 성립 — 마이그레이션된 파일에서 pdf 대칭 키 = v2 시절 image 값이므로 signature 동일. 골든 문자열 회귀 테스트 필수.
2. **env 이름 보존** — `apply_env` whitelist 의 LHS(키 문자열) 전부 그대로, RHS(대입 대상)만 새 struct 경로. 신규 pdf paddle 키만 `KEBAB_PDF_OCR_{DET_MODEL,REC_MODEL,DICT,SCORE_THRESH,UNCLIP_RATIO,MAX_BOXES}` 추가. 기존 env 테스트 전부 green 유지.
3. **무손실 골든** — 사용자 실제 v2 config(첨부본; `score_gate` 찌꺼기·주석 대안 줄 포함)를 fixture 로: `migrate_document` → (a) 모든 사용자 값 보존, (b) 사용자 주석/대안 줄 보존, (c) `[ingest.image.ocr]` 등 신 위치 존재, (d) 결과가 v3 `Config` 로 parse 되고 값이 원 의미와 동일, (e) 재실행 멱등.
4. **load 자동변환** — v2 텍스트를 `Config::from_file` 로 읽으면(디스크 미변경) `config.ingest.chunking.target_tokens` 등이 사용자 값으로 채워짐(기본값 유실 없음) 테스트.
5. **float 직렬화 정리** — `Config::defaults()` 직렬화에 `0.30000001192092896` 부재, `score_gate = 0.3`. 구현: f32 필드에 `#[serde(serialize_with = "ser_f32_clean")]`(f32 Display 의 shortest round-trip 을 f64 로 재파싱해 직렬화) — struct 타입·호출부 무변경, kebab-config 국소. 사용자 기존 파일의 찌꺼기 값은 toml_edit 보존(값 불가침)이라 그대로 — 재생성 시에만 정리됨(비목표 §2 정합).

## 9. 버전 / 문서 cascade

- **minor bump** (인터페이스 변경: config 섹션 rename + 신규 키). `Cargo.toml` workspace version.
- **schema_version 2→3** (위).
- **도그푸딩 필수**(CLAUDE.md Dogfood trigger: CLI/config surface) — `kebab config migrate` 를 실제 v2 파일(첨부본)에 돌려 무손실 + 자동변환 + 재색인 0 확인. evidence → HOTFIXES + release notes.
- **문서 동기화(같은 PR)**: README Configuration 섹션 + `docs/SMOKE.md` config 예시 블록(새 레이아웃) + HOTFIXES dated entry + `2026-05-31-config-migration-design.md` 의 Risks/notes 에 v3 rename 교차링크.

## 10. 리스크

| 리스크 | 완화 |
|--------|------|
| 테이블 이동 시 주석 유실 | toml_edit `remove`→`insert` 로 `Item` 통째 이동, 골든 테스트(§8.3) |
| signature 변동→전체 재색인 | 골든 문자열 회귀 테스트(§8.1), 값 포맷 보존 |
| pdf paddle 대칭 추가가 기존 pdf paddle 동작 변경 | §6.5 마이그레이션이 image paddle 6키 실제 값을 pdf 대칭 키로 복사 → 동작·signature 동일(§8.1) |
| call-site 누락 | 컴파일러가 강제(필드 제거→ 미수정 site 컴파일 에러), clippy gate |
| 메모리 자동변환 매 load 비용 | toml_edit parse 1회/실행, 무시 가능 |
```
