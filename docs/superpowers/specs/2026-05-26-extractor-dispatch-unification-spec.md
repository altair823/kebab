---
status: drafting
target_version: 0.18.0    # 0.18.0 release 의 후속 internal-refactor PR — workspace.version bump 없음 (CLAUDE.md §Release 룰 3 트리거 미충족: frozen design contract 변경 0, wire schema 변경 0, V00X migration 0).
contract_sections: []     # design §7.2 의 Extractor trait 정의가 이미 `supports(&MediaType)` 포함 — trait surface 변경 0. §8 dep graph 변경 0. 갱신 필요한 frozen section 없음.
related_specs:
  - docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
  - docs/superpowers/specs/2026-05-26-source-fs-dep-lightening-spec.md   # sibling sub-item 1 (PR #185 merged)
  - docs/superpowers/specs/2026-05-26-normalize-absorption-spec.md       # sibling sub-item 2 (PR #186 merged)
related_plans: []
hotfix_links: []
---

# kebab-app 의 AST 9-arm extract dispatch 통합 — `*Extractor::new().extract(…)` → `app.extract_for(...)` polymorphic dispatch

## §1 Background + evidence chain

### §1.1 현재 `Extractor` trait + impl 위치 (investigation step 1, 3)

`crates/kebab-core/src/traits.rs:115-122` 의 trait 정의 인용 (round 1 CRITICAL #2 보강: design `:1416-1420` 의 `Result<>` + elided lifetime 약식 표기와 semantically identical 이지만 syntactic byte-identical 은 아님 — 어느 쪽도 본 refactor 가 변경하지 않음):

```rust
pub trait Extractor: Send + Sync {
    fn supports(&self, media_type: &MediaType) -> bool;
    fn parser_version(&self) -> ParserVersion;
    fn extract(
        &self,
        ctx: &ExtractContext<'_>,
        bytes: &[u8],
    ) -> anyhow::Result<CanonicalDocument>;
}
```

핵심 사실: `supports(&MediaType) -> bool` 가 **이미 trait method 로 존재**한다. 본 refactor 가 새 method 를 추가하는 것이 아니라, 이미 존재하는 polymorphic surface 를 활용하지 못하고 있는 dead polymorphism 상태를 부분 해소한다.

production `impl Extractor for ...` 11곳 (`grep -rn "impl.*Extractor for\|impl Extractor for" crates/ --include="*.rs"` 결과):

| crate | type | 위치 | `supports()` 조건 |
|---|---|---|---|
| `kebab-parse-image` | `ImageExtractor` | `src/lib.rs:69` | `matches!(m, MediaType::Image(_))` |
| `kebab-parse-pdf` | `PdfTextExtractor` | `src/lib.rs:51` | `matches!(m, MediaType::Pdf)` |
| `kebab-parse-code` | `RustAstExtractor` | `src/rust.rs:53` | `matches!(m, MediaType::Code(l) if l == "rust")` |
| `kebab-parse-code` | `PythonAstExtractor` | `src/python.rs:49` | `matches!(m, MediaType::Code(l) if l == "python")` |
| `kebab-parse-code` | `TypescriptAstExtractor` | `src/typescript.rs:59` | `… "typescript"` |
| `kebab-parse-code` | `JavascriptAstExtractor` | `src/javascript.rs:66` | `… "javascript"` |
| `kebab-parse-code` | `GoAstExtractor` | `src/go.rs:51` | `… "go"` |
| `kebab-parse-code` | `JavaAstExtractor` | `src/java.rs:61` | `… "java"` |
| `kebab-parse-code` | `KotlinAstExtractor` | `src/kotlin.rs:66` | `… "kotlin"` |
| `kebab-parse-code` | `CAstExtractor` | `src/c.rs:52` | `… "c"` |
| `kebab-parse-code` | `CppAstExtractor` | `src/cpp.rs:76` | `… "cpp"` |

**누락**: `kebab-parse-md` 는 `impl Extractor` 가 **없다**. Markdown 의 ingest path 는 `parse_frontmatter` / `parse_blocks` / `build_canonical_document` 세 자유 함수의 직접 호출로 처리된다 (lib.rs:1085-1118). 본 refactor 는 round 1 reflection 의 MAJOR #2 Option (ii) 채택에 따라 **`MarkdownExtractor` 신설을 별 PR 로 defer** — 본 PR scope 는 AST 9-arm extract dispatch only. §2 + §3.4 + §11 참조.

### §1.2 현재 `Chunker` trait + impl 위치 (investigation step 2, 4)

`crates/kebab-core/src/traits.rs:125-132` 의 trait 정의 인용:

```rust
pub trait Chunker: Send + Sync {
    fn chunker_version(&self) -> ChunkerVersion;
    fn policy_hash(&self, policy: &ChunkPolicy) -> String;
    fn chunk(
        &self,
        doc: &CanonicalDocument,
        policy: &ChunkPolicy,
    ) -> anyhow::Result<Vec<Chunk>>;
}
```

핵심 사실: `Chunker` trait 은 **`supports()` 또는 그에 준하는 dispatch discriminator method 가 없다**. Extractor 와 비대칭. 본 refactor 가 Chunker 까지 polymorphic dispatch 로 통합하려면 trait 에 새 method 신설이 필요하고, design §7.2 의 trait 정의 갱신 (= frozen contract 갱신) 도 필요해진다. → 별 PR scope (§8 / §11).

production `impl Chunker for ...` 15곳 — `kebab-chunk` 한 crate 안에서 다음 15 type:

| 위치 | type | 적용 lang/media |
|---|---|---|
| `src/md_heading_v1.rs:77` | `MdHeadingV1Chunker` | Markdown |
| `src/pdf_page_v1.rs:76` | `PdfPageV1Chunker` | PDF |
| `src/code_rust_ast_v1.rs:30` | `CodeRustAstV1Chunker` | code:rust |
| `src/code_python_ast_v1.rs:30` | `CodePythonAstV1Chunker` | code:python |
| `src/code_ts_ast_v1.rs:30` | `CodeTsAstV1Chunker` | code:typescript |
| `src/code_js_ast_v1.rs:30` | `CodeJsAstV1Chunker` | code:javascript |
| `src/code_go_ast_v1.rs:30` | `CodeGoAstV1Chunker` | code:go |
| `src/code_java_ast_v1.rs:30` | `CodeJavaAstV1Chunker` | code:java |
| `src/code_kotlin_ast_v1.rs:30` | `CodeKotlinAstV1Chunker` | code:kotlin |
| `src/code_c_ast_v1.rs:30` | `CodeCAstV1Chunker` | code:c |
| `src/code_cpp_ast_v1.rs:30` | `CodeCppAstV1Chunker` | code:cpp |
| `src/code_text_paragraph_v1.rs:25` | `CodeTextParagraphV1Chunker` | code:shell + Tier 3 fallback |
| `src/manifest_file_v1.rs:18` | `ManifestFileV1Chunker` | toml/json/xml/groovy/go-mod |
| `src/dockerfile_file_v1.rs:17` | `DockerfileFileV1Chunker` | dockerfile |
| `src/k8s_manifest_resource_v1.rs:18` | `K8sManifestResourceV1Chunker` | yaml |

### §1.3 kebab-app hardcoded callsite enumeration (investigation step 5)

`grep -nE "match.*code_lang|ImageExtractor::|PdfTextExtractor::|MarkdownParser::|kebab_parse_(md|pdf|image|code)::" crates/kebab-app/src/lib.rs` 결과 중 본 refactor 가 건드릴 site (use statement / version constant 인용 제외):

| line | site | 종류 | 본 PR 변경 여부 |
|---|---|---|---|
| `51` | `use kebab_parse_image::{ImageExtractor, OllamaVisionOcr, apply_caption, apply_ocr};` | use 선언 | **유지** (registry 가 동일 type 을 Box 로 감싸므로 use 필요) |
| `52` | `use kebab_parse_code::{CAstExtractor, …, TypescriptAstExtractor};` (9 type) | use 선언 | **유지** (registry init 에서 9 type 모두 instantiate) |
| `53` | `use kebab_parse_pdf::PdfTextExtractor;` | use 선언 | **유지** |
| `54` | `use kebab_parse_md::{BodyHints, build_canonical_document, parse_blocks, parse_frontmatter};` | use 선언 | **유지** (MarkdownExtractor defer — 자유 함수 그대로) |
| `356` | `let image_extractor = ImageExtractor::new();` | App init (local var) | **제거** (MAJOR #4 의 Option c) |
| `1089` | `parse_frontmatter(&bytes, &body_hints)` | Markdown ingest path | **변경 0** (MarkdownExtractor defer) |
| `1097` | `parse_blocks(&bytes[fm_span_end(fm_span)..], body_offset_lines)` | Markdown ingest path | **변경 0** |
| `1111` | `build_canonical_document(asset, …)` | Markdown ingest path | **변경 0** |
| `1296` | `image_extractor.extract(&ctx, &bytes)` | Image ingest path (instance call) | **교체** → `app.extract_for(&asset.media_type, &ctx, &bytes)?` |
| `1783` | `let mut canonical = PdfTextExtractor::new().extract(&ctx, &bytes)` | PDF ingest path (typed) | **교체** → `app.extract_for(&asset.media_type, &ctx, &bytes)?` |
| `1935-1953` | `let parser_version = match code_lang { … }` (11 explicit arms cover 17 lang) | code parser_version 결정 | **변경 0** (Chunker registry + Tier 2/3 통합과 묶임 — 별 PR) |
| `1955-1974` | `let mut chunker_version = match code_lang { … }` (11 explicit arms cover 17 lang) | code chunker_version 결정 | **변경 0** |
| `1979-1988` | `let tier3_fallback_cv: Option<ChunkerVersion> = match code_lang { … }` (2 arm: positive 16-lang sum + `_ => None`) | tier3 fallback CV 결정 | **변경 0** |
| `2012-2049` | `let canonical_result = match code_lang { "rust" => RustAstExtractor::new().extract(…), …, "yaml" \| … => synthesize_tier2_document(…), "shell" => synthesize_tier2_document(…), "c" \| "cpp" => *AstExtractor::new().extract(…) }` (12 arm = 11 explicit + 1 wildcard, cover 17 lang) | code extract dispatch | **부분 교체** — 9 AST arm 의 `*Extractor::new().extract(…)` → `app.extract_for(&asset.media_type, &ctx, &bytes)?` 단일 호출 (위로 hoist). 7 manifest + 1 shell arm 의 `synthesize_tier2_document(…)` 는 유지 (Extractor 아님). |
| `2087-2128` | `match code_lang { "rust" => CodeRustAstV1Chunker.chunk(…), …, "shell" => CodeTextParagraphV1Chunker.chunk(…), "c" \| "cpp" => Code*AstV1Chunker.chunk(…) }` (14 explicit arms) | code chunk dispatch | **변경 0** (Chunker registry 별 PR) |

### §1.4 lang 분기 정확한 count + 17 code_lang 값 (investigation step 6, 12)

round 1 MAJOR #5 보강: `crates/kebab-app/src/lib.rs:1009-1013` 의 outer guard 가 다음 17 lang 을 enumerate (인용):

```rust
MediaType::Code(lang)
    if matches!(lang.as_str(),
        "rust" | "python" | "typescript" | "javascript" | "go" | "java" | "kotlin"
        | "yaml" | "dockerfile" | "toml" | "json" | "xml" | "groovy" | "go-mod"
        | "shell" | "c" | "cpp")
    => return ingest_one_code_asset(…),
```

- **AST lang 9개**: rust / python / typescript / javascript / go / java / kotlin / c / cpp — 각 lang 의 `*AstExtractor` + `Code*AstV1Chunker` 호출.
- **Tier 2 (manifest) lang 7개**: yaml / dockerfile / toml / json / xml / groovy / go-mod — `synthesize_tier2_document` (free function) + chunker (K8sManifestResourceV1 / DockerfileFileV1 / ManifestFileV1) 호출.
- **Tier 3 lang 1개**: shell — `synthesize_tier2_document(..., "shell", ...)` + `CodeTextParagraphV1Chunker` 호출.

총 **17 code_lang**. AST lang 9개만 Extractor impl 이 있고, Tier 2/3 의 8 lang 은 Extractor impl 없이 `synthesize_tier2_document` 라는 자유 함수가 대신 emit 한다. 본 PR 의 scope = **9 AST arm only**.

### §1.5 ingest entry point + first dispatch (investigation step 8)

`kebab-app::ingest_with_config*` 의 entry chain (lib.rs:219 / :234 / :250 / :281 / :720) 모두 동일한 inner loop (`ingest_one_asset` per asset) 로 수렴. `ingest_one_asset` 의 lib.rs:961-1040 head 가 **first dispatch** — `match &asset.media_type` 의 4-arm (Markdown 자체 fall-through / Image / Pdf / Code(lang) + 1-arm catch-all skip).

본 dispatch 는 **2-layer** 구조:

1. **outer dispatch** — `ingest_one_asset` 의 `match &asset.media_type` (4-arm + 1-skip). **본 PR 에서 그대로 유지** — helper 함수 분기 (`ingest_one_image_asset` / `ingest_one_pdf_asset` / `ingest_one_code_asset`) 가 each medium 의 post-extract pipeline (OCR / page-chunker / tier3-fallback / try-skip-unchanged) 을 들고 있어서 통합 비용이 큼. 별 PR scope.
2. **inner dispatch** — `ingest_one_code_asset` 안의 5 위치 `match code_lang` (위 §1.3 의 5 site). **본 PR 에서 lib.rs:2012-2049 의 9 AST arm 만 polymorphic 교체**. 나머지 4 위치 (parser_version / chunker_version / tier3_fallback_cv / chunk dispatch) 는 Chunker registry + Tier 2/3 통합과 묶여 별 PR.

### §1.6 App struct 현재 state (investigation step 9)

`crates/kebab-app/src/app.rs:115` 의 struct 인용. App 의 lifecycle 은 `App::open_with_config(config) -> Result<Self>` 에서 시작, SQLite store 를 open + migrate 한 뒤 embedder/vector/llm 은 lazy `OnceLock` 으로 deferred init (round 1 MINOR #1 보강 — App 의 lifecycle 의 lazy/eager 라인을 명시):

```rust
pub struct App {
    pub(crate) config: kebab_config::Config,
    pub(crate) sqlite: Arc<SqliteStore>,
    embedder: OnceLock<Arc<dyn Embedder + Send + Sync>>,  // lazy
    vector: OnceLock<Arc<LanceVectorStore>>,              // lazy
    llm: OnceLock<Arc<dyn LanguageModel>>,                // lazy
    search_cache: Option<Mutex<LruCache<SearchCacheKey, Vec<SearchHit>>>>,
    pipeline_verifier: Option<Arc<dyn kebab_nli::NliVerifier>>,  // eager
}
```

기존 trait-object pattern 은 **단일 `Arc<dyn Trait>`** (`embedder` / `llm` / `pipeline_verifier`). `Vec<Box<dyn Extractor>>` 는 새 패턴 — 다중 trait-object collection. `OnceLock` 으로 lazy init 할 필요 없음 — 모든 11 Extractor impl 이 state-less (§1.7).

`ingest_with_config*` 는 `App::open_with_config` 를 통해 인스턴스를 얻은 뒤 inner ingest loop 를 돈다. App field 에 registry 가 들어가면 `ingest_one_*_asset` 의 `app: &App` 인자를 통해 자동으로 접근 가능 — 추가 wiring 0.

### §1.7 state-less Extractor 확인 (investigation step 7 보강)

모든 11개 Extractor impl 의 `new()` signature 가 `pub fn new() -> Self` 이고 struct body 는 unit-struct 또는 zero-field. 인용 예 (`crates/kebab-parse-pdf/src/lib.rs:51-58`):

```rust
pub struct PdfTextExtractor;
impl PdfTextExtractor {
    pub fn new() -> Self { Self }
}
impl Default for PdfTextExtractor { fn default() -> Self { Self::new() } }
```

`ImageExtractor` / `RustAstExtractor` / … 동일 패턴. `OllamaVisionOcr` 는 LLM client 를 들고 있는 state-ful type 이지만 **Extractor 가 아니다** — OCR adapter (별 trait). 본 refactor 의 registry 는 Extractor 만 담는다 — 모든 entry 가 state-less + zero-cost `new()` → init 비용 사실상 0.

### §1.8 design contract 영향 (investigation step 10) + referencing task spec (investigation step 11)

design `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md` 의 영향 분석:

- **§7.2 (트레잇)**: `:1416-1420` 의 `Extractor` trait 정의가 이미 `supports(&MediaType)` 포함 — design 의 약식 표기 (`Result<>` + elided lifetime) 가 actual `crates/kebab-core/src/traits.rs:115-122` 의 `anyhow::Result<>` + `ExtractContext<'_>` 와 semantically identical. 어느 쪽도 본 refactor 가 변경하지 않음. **갱신 불필요**.
- **§8 (모듈 경계)**: dep graph 의 `kebab-app -> kebab-parse-md / kebab-parse-pdf / kebab-parse-image / kebab-parse-code` 4 line 그대로. 본 refactor 가 새 crate 를 추가하거나 기존 crate dep 를 끊지 않음. **갱신 불필요**.
- **§6 (filesystem layout)**: 본 refactor 와 관련 0 — workspace path / config / XDG 영향 0. **갱신 불필요**.

frozen task spec 의 영향 (`grep -rln "Extractor\|Chunker" tasks/`):

- `tasks/phase-{0,1,6,7,8}-*.md`, `tasks/p0/p0-1-skeleton.md`, `tasks/p1/p1-5-chunk.md`, `tasks/p6/p6-{1,4}*.md`, `tasks/p7/p7-{1,2,3}*.md`, `tasks/p8/p8-{1,2}*.md`, `tasks/p10/*.md`, `tasks/p3/p3-5-app-wiring.md`, `tasks/p5/p5-2-metrics-compare.md` — 모두 frozen historical contract. 본 refactor 가 trait signature 변경 0 + impl 추가 0 (MarkdownExtractor defer) → frozen task spec 의 contract 침범 0.

**결론**: design contract 변경 0, frozen task spec 변경 0 → `contract_sections: []` + `target_version: 0.18.0` (workspace.version bump 불필요).

### §1.9 ARCHITECTURE.md 의 dispatch flow 묘사 부재 (round 1 Missing 1)

`grep -n "dispatch\|registry\|polymorphic\|ingest flow" docs/ARCHITECTURE.md` 결과 = `line 25` 의 "code parser" table 의 chunker / parser version 묘사만 존재. **"ingest dispatch flow" section 없음**. → 본 PR 이 ARCHITECTURE.md 의 dispatch 묘사를 신설하지 않는 것이 정합 (변경 0). 단 line 25 의 code parser table 의 wording 은 그대로 — 본 refactor 가 parser/chunker family 를 건드리지 않으므로.

---

## §2 Goals + non-goals

### §2.1 Goals

1. **inner AST 9-arm extract dispatch 통합** — `ingest_one_code_asset` 의 lib.rs:2012-2049 9 AST arm 의 `*Extractor::new().extract(…)` 호출을 `app.extract_for(&asset.media_type, &ctx, &bytes)?` 단일 polymorphic call 로 교체. outer 4-arm match (helper 분기) + Tier 2/3 의 `synthesize_tier2_document` free-function path + Chunker dispatch 는 §2.2 / §2.3 의 non-goal.
2. **image / pdf path 의 hardcoded Extractor 호출 교체** — lib.rs:1296 (`image_extractor.extract(…)`) + lib.rs:1783 (`PdfTextExtractor::new().extract(…)`) 두 callsite 를 동일 `app.extract_for(&asset.media_type, &ctx, &bytes)?` 단일 호출로 교체. lib.rs:356 의 local `image_extractor` 변수 + `ImagePipeline.extractor` field 모두 제거 (§3.5.1 의 Option c).
3. **`Vec<Box<dyn Extractor>>` registry 도입** — `App` 에 새 field `extractors: Vec<Box<dyn Extractor + Send + Sync>>` 추가. `App::open_with_config` 에서 11 Extractor impl (ImageExtractor + PdfTextExtractor + 9 AST) 등록. `App::extract_for(&MediaType, &ExtractContext, &[u8]) -> Result<CanonicalDocument>` helper method 추가.
4. **wire schema 변경 0** — `CanonicalDocument` / `IngestReport.v1` / `error.v1` 출력 byte-identical. `IngestItem.warnings` (round 1 MAJOR #2 의 risk) 의 channel 보존 — markdown path 가 본 PR 에서 변경되지 않으므로 risk 자동 해소. `--json` smoke 의 diff = 0.
5. **workspace.version bump 불필요** — frozen design contract 변경 0, wire schema 변경 0, V00X migration 0 → CLAUDE.md §Release 룰 3 트리거 미충족.
6. **workspace test net delta = small positive** — 현재 baseline 1313 test 가 본 refactor 후 1313 + N (registry init 의 mutually-exclusive `supports()` grid + `App::extract_for` 의 4-medium happy-path unit test 만큼). 기존 ingest happy path test 가 byte-identical pass.

### §2.2 Non-goals

1. **MarkdownExtractor 신설** — round 1 MAJOR #2 Option (ii) 채택. `IngestItem.warnings` channel 의 `parse_frontmatter` + `parse_blocks` warning sink 가 `MarkdownExtractor::extract(&ExtractContext, &[u8]) -> Result<CanonicalDocument>` signature 에는 흐를 수 없는 구조 — `CanonicalDocument.provenance` 의 ProvenanceEvent 는 WarningKind enum 의 Debug 형식을 보존 안 함. wire schema diff 0 보장 위해 markdown path 를 그대로 유지하고 MarkdownExtractor 는 별 PR 에서 처리 (§11). **즉 `kebab-parse-md` 는 본 PR 에서 변경 0**.
2. **ExtractorRegistry 별 type / plugin system** — `App` field 가 아닌 별도 `ExtractorRegistry` struct + dynamic-loading hook 의 도입은 본 PR 의 scope 가 아니다 (Option B in §3.1). future defer.
3. **enum-based dispatch** — `enum AnyExtractor { Md, Pdf, … }` 의 zero-cost static dispatch (Option C in §3.1) 는 trait polymorphism 의 의도와 conflict — 본 PR 의 scope 가 아니다.
4. **Tier 2/3 free-function path 의 Extractor 화** — `synthesize_tier2_document` 의 7 manifest + 1 shell lang 의 Extractor impl 승격은 별 PR.
5. **Chunker dispatch unification** — `Chunker` trait 에 `supports()` 신설 + `App.chunkers` registry 도입은 design §7.2 갱신 동반. 별 spec + 별 PR (`2026-05-?? -chunker-dispatch-unification-spec.md` follow-up — §11).
6. **inner 4 위치 match 의 polymorphic 통합** — parser_version (lib.rs:1935-1953) / chunker_version (1955-1974) / tier3_fallback_cv (1979-1988) / chunk dispatch (2087-2128) 의 통합. parser_version 은 Extractor::parser_version() method 로 가져올 수 있지만 Tier 2/3 의 sentinel `"none-v1"` 가 hardcoded → free-function path 의 Extractor 화 (#4) 와 묶여야 함.

### §2.3 Scope 축소 이유 (round 1 MAJOR #2)

본 refactor 의 mission 은 "dead polymorphism 해소". 본 PR scope = "AST 9-arm extract dispatch + image + pdf extract callsite" 의 polymorphic 교체. 이것만으로:

- Extractor trait 의 `supports()` 가 실제 호출되어 polymorphism 이 살아난다 (lib.rs:1296 / :1783 / :2012-2049 의 9 AST arm 의 11 callsite 가 단일 `app.extract_for(...)` 로 수렴).
- `App.extractors` registry 가 도입되어 향후 (a) MarkdownExtractor 추가, (b) Chunker registry, (c) Tier 2/3 Extractor 화 등의 follow-up 시 확장 지점이 명확해진다.
- wire schema diff 0 (markdown warning channel 미손) + design contract 변경 0 (Chunker / MarkdownExtractor defer) → release cycle 영향 0.

markdown / inner-4-match / Chunker / Tier 2/3 통합은 모두 별 PR 에서 처리하는 편이 (a) risk 분리, (b) review surface 축소, (c) design §7.2 갱신 동반 시 별 release cycle 정합. §11 future work 에 명시.

---

## §3 Design

### §3.1 Destination = Option A (`App` 의 field)

3 option 비교:

| option | 설명 | trade-off |
|---|---|---|
| **A. `App.extractors: Vec<Box<dyn Extractor>>`** | App field 로 11 impl 등록. dispatch = `app.extractors.iter().find(\|e\| e.supports(media)).ok_or_else(...)?.extract(&ctx, &bytes)`. | + 가장 단순 + 변경 surface 최소.<br>− App field 증가 (1 line).<br>− registry 의 ownership 이 App 에 강결합. |
| **B. 별 `ExtractorRegistry` struct** | App 의 field 가 아닌 별도 type. App 이 owner 인 점은 동일하지만 type 이 분리. | + 미래 plugin 가능성 (defer).<br>− 본 PR 의 변경 surface 증가 (새 type + 새 file).<br>− 현재 caller 가 App 단일 — 분리 가치 0. |
| **C. enum-based dispatch** | `enum AnyExtractor { Md, Pdf, … }` + static match. | + zero-cost dispatch.<br>− trait polymorphism 의 의도와 conflict.<br>− 신 Extractor 추가 = enum variant 변경 (API 표면 확대). |

**결정: Option A**. 근거 = §1.6 의 App single-owner pattern + §1.7 의 state-less Extractor 사실. `Vec<Box<dyn Extractor + Send + Sync>>` 가 정합.

trait object vtable overhead 의 performance 측면: dispatch 가 per-asset 1회 (extract 안의 hot loop 0 회) → 측정 불가 수준. ingest throughput 영향 0.

### §3.2 `Extractor` trait surface — 변경 0

`crates/kebab-core/src/traits.rs` 의 `Extractor` trait 정의 변경 **불필요**. 이미 `supports(&MediaType)` + `parser_version()` + `extract(&ExtractContext, &[u8])` 의 3 method 가 충분.

**critical invariant**: trait byte-identical 보존. trait file (`crates/kebab-core/src/traits.rs`) 의 변경 0 — 만일 trait 갱신이 발생하면 sub-item 2 의 CRITICAL #1 (trait signature drift) 재발 risk. 본 PR 의 diff 에서 `crates/kebab-core/src/traits.rs` 가 변경되면 안 됨 (verifier 검증 지점).

### §3.3 기존 11 Extractor impl — 변경 0

`ImageExtractor` / `PdfTextExtractor` / 9 `*AstExtractor` 의 `impl Extractor for ...` block 전체 변경 **불필요**. `supports()` / `parser_version()` / `extract()` 의 3 method 가 이미 구현되어 있고 (§1.1 의 grep 결과), 본 refactor 가 호출 site 만 변경.

**critical invariant**: 11 impl 의 method body byte-identical 보존. lib.rs 의 callsite 가 변하더라도 Extractor impl 의 결과 (`CanonicalDocument` 의 모든 field) 가 동일해야 wire schema diff 0 보장.

### §3.4 `MarkdownExtractor` 신설 — 본 PR 에서 defer (round 1 MAJOR #2 Option (ii))

본 PR 에서 `MarkdownExtractor` 를 **신설하지 않는다**. 근거:

`lib.rs:1085-1118` 의 markdown ingest path 가 `parse_frontmatter` + `parse_blocks` 의 `Vec<Warning>` 두 stream 을 합쳐 다음 두 sink 로 분배:

1. **`warning_notes: Vec<String>`** (lib.rs:1100-1109) → `IngestItem.warnings` (wire `ingest_report.v1.IngestItem.warnings`).
2. **`all_warnings: Vec<Warning>`** → `build_canonical_document(asset, metadata, blocks, parser_version, warnings)` (crates/kebab-parse-md/src/normalize.rs:60-65 의 signature 인용 — `warnings: Vec<Warning>` 의 5번째 arg).

Pattern β 의 `MarkdownExtractor::extract(&ExtractContext, &[u8]) -> Result<CanonicalDocument>` signature 는 `Vec<Warning>` 의 caller-visible channel 이 없음. `CanonicalDocument.provenance` 의 ProvenanceEvent 로 만들어도 wire schema 의 `IngestItem.warnings` 필드와 다른 형태 + WarningKind enum 의 Debug 출력 (`format!("{:?}: {}", w.kind, w.note)`) 보존 mechanic 미흡 → wire diff > 0 risk.

본 PR 의 §2.1 #4 (wire schema 변경 0) 와 모순 → MarkdownExtractor 신설은 **별 PR 로 defer**. 별 PR 에서 처리될 work:

- `kebab-parse-md/src/extractor.rs` 신규 + `impl Extractor for MarkdownExtractor`.
- `kebab-parse-md/src/lib.rs` 의 `pub mod extractor;` + `pub use crate::extractor::MarkdownExtractor;` 추가 (re-export).
- `Vec<Warning>` channel 의 새 surface 설계 — `CanonicalDocument.provenance` 의 Warning event 로 lift 하거나 wire schema 의 `IngestItem.warnings` 필드 추가 surface (additive minor bump).
- `build_body_hints` (kebab-app/src/lib.rs:2422-2429) 의 MarkdownExtractor 안으로 이동 — `&RawAsset` 단독 input + first_h1/fallback_lang None 하드코딩 + fs_ctime/fs_mtime ← asset.discovered_at 의 mechanic 보존.

본 PR 의 §11 (Future work) 에 명시.

### §3.5 `App::open_with_config` 의 registry 초기화 — 11 Extractor

`crates/kebab-app/src/app.rs` 의 `App` struct 갱신:

```rust
pub struct App {
    pub(crate) config: kebab_config::Config,
    pub(crate) sqlite: Arc<SqliteStore>,
    /// post-v0.18.0: inner-AST 9-arm extract dispatch + image/pdf extract
    /// callsite 통합. App init 시 1회 등록 — markdown 은 별 PR 에서 추가.
    pub(crate) extractors: Vec<Box<dyn Extractor + Send + Sync>>,
    embedder: OnceLock<Arc<dyn Embedder + Send + Sync>>,
    vector: OnceLock<Arc<LanceVectorStore>>,
    llm: OnceLock<Arc<dyn LanguageModel>>,
    search_cache: Option<Mutex<LruCache<SearchCacheKey, Vec<SearchHit>>>>,
    pipeline_verifier: Option<Arc<dyn kebab_nli::NliVerifier>>,
}
```

`App::open_with_config` 안의 init 코드 추가 (round 1 NIT #2: trailing comma 정리):

```rust
let extractors: Vec<Box<dyn Extractor + Send + Sync>> = vec![
    Box::new(kebab_parse_image::ImageExtractor::new()),
    Box::new(kebab_parse_pdf::PdfTextExtractor::new()),
    Box::new(kebab_parse_code::RustAstExtractor::new()),
    Box::new(kebab_parse_code::PythonAstExtractor::new()),
    Box::new(kebab_parse_code::TypescriptAstExtractor::new()),
    Box::new(kebab_parse_code::JavascriptAstExtractor::new()),
    Box::new(kebab_parse_code::GoAstExtractor::new()),
    Box::new(kebab_parse_code::JavaAstExtractor::new()),
    Box::new(kebab_parse_code::KotlinAstExtractor::new()),
    Box::new(kebab_parse_code::CAstExtractor::new()),
    Box::new(kebab_parse_code::CppAstExtractor::new()),
];
```

**ordering invariant** (round 1 Ambiguity 1 해소 — (A) safety guard 의미만):

11 Extractor 의 `supports()` 가 mutually exclusive 한 한 ordering 무관. registry 의 ordering 은 wire contract 가 아니며 (외부에 노출되지 않음 + serialize 되지 않음), `find()` 의 first-match optimization 의 안정성을 위한 **safety guard** 일 뿐 — verifier 의 unit test (§5.1) 가 mutually-exclusive grid 로 검증.

현재 11 + 1 (markdown 별 PR 후) impl 의 `supports()` 가 disjoint:
- `MediaType::Markdown` / `MediaType::Pdf` / `MediaType::Image(_)` 는 enum variant 단위 disjoint.
- `MediaType::Code(l)` 의 9 AST lang 의 `supports()` 도 lang string equality 비교로 disjoint (rust ≠ python ≠ … ≠ cpp).

#### §3.5.1 `ImagePipeline.extractor` lifecycle (round 1 MAJOR #4 Option c)

actual `crates/kebab-app/src/lib.rs:760-764` 의 `ImagePipeline` struct 인용:

```rust
struct ImagePipeline<'a> {
    extractor: &'a ImageExtractor,
    ocr_engine: Option<&'a OllamaVisionOcr>,
    caption_llm: Option<&'a dyn LanguageModel>,
}
```

3 option:

| option | 방법 | trade-off |
|---|---|---|
| (a) parallel state | local `image_extractor` (lib.rs:356) 유지 + `App.extractors` 도 별도 보유 | + 변경 surface 최소.<br>− 두 source-of-truth — silent drift risk. |
| (b) trait object 로 변경 | `extractor: &'a (dyn Extractor + Send + Sync)` 로 field type 변경 | + registry 의 entry 를 `&dyn` 로 borrow.<br>− concrete `image_extractor.extract(...)` callsite 의 type 추론 깨질 risk + lifetime gymnastics. |
| **(c) field 제거** | `ImagePipeline.extractor` field 자체 제거. `ingest_one_image_asset` (lib.rs:1296) 이 직접 `app.extract_for(&asset.media_type, &ctx, &bytes)?` 호출. | + single source-of-truth.<br>+ lib.rs:356 의 local 도 제거.<br>− ImagePipeline 의 의미가 OCR + caption 만 남음 (의도와 정합 — image-specific post-extract adapter 만 carry). |

**결정: Option c**. 근거 = sub-item 2 의 CRITICAL #5 (sole-source-of-truth) 원칙 + ImagePipeline 의 의미가 OCR + caption pipeline (post-extract) 로 정확히 한정 + lib.rs:356 의 local 제거.

본 PR 의 ImagePipeline 갱신 후 모습:

```rust
struct ImagePipeline<'a> {
    ocr_engine: Option<&'a OllamaVisionOcr>,
    caption_llm: Option<&'a dyn LanguageModel>,
}
```

callsite `ingest_one_image_asset` (lib.rs:1232 의 head — image_pipeline arg 가 들어오는 곳) 에서 `image_pipeline.extractor.extract(...)` 의 호출이 `app.extract_for(&asset.media_type, &ctx, &bytes)?` 로 교체.

### §3.6 ingest entry dispatch loop 패턴 (Pattern β — extract-only polymorphism)

**Pattern β** 채택. 즉 outer 4-arm match (helper 함수 분기) 는 본 PR 에서 그대로 유지하고, **medium-internal `*Extractor::new().extract(…)` 호출만** polymorphic dispatch 로 교체. 영향 callsite 3 군:

1. lib.rs:1296 — `image_extractor.extract(&ctx, &bytes)` → `app.extract_for(&asset.media_type, &ctx, &bytes)?`.
2. lib.rs:1783 — `PdfTextExtractor::new().extract(&ctx, &bytes)` → 동일 helper.
3. lib.rs:2012-2049 — 9 AST arm 의 `*AstExtractor::new().extract(&ctx, &bytes)` → 9 callsite 가 1 callsite 로 hoist + 단일 `app.extract_for(&asset.media_type, &ctx, &bytes)?` 호출.

마크다운 path (lib.rs:1085-1118 의 `parse_frontmatter` / `parse_blocks` / `build_canonical_document` 3-step) + Tier 2/3 path (lib.rs:2012-2049 의 7 manifest + 1 shell arm 의 `synthesize_tier2_document(...)`) 은 **변경 0**.

**round 1 의 design tension 재인용**: Pattern β 가 outer 4-arm match 의 helper 분기는 유지 → "dead polymorphism 해소" 의 의미가 부분적. 본 PR 의 net scope = `*Extractor::new().extract(…)` callsite (12 callsite — image 1 + pdf 1 + AST 9) 를 1 callsite 로 통합. outer 4-arm helper 분기는 별 PR (markdown extractor 신설 + Chunker registry + Tier 2/3 통합) 의 work — 그 때 전체 dispatch flow 가 통합 가능.

helper signature:

```rust
impl App {
    /// Polymorphic dispatcher for the Extractor trait. Looks up the
    /// first Extractor whose `supports(media)` returns true and invokes
    /// `extract(ctx, bytes)` on it.
    ///
    /// Errors with `anyhow!("no Extractor for media_type {media:?}")`
    /// when no matching Extractor is registered — caller (e.g.
    /// `ingest_one_*_asset`) should treat this as a programming error
    /// (unreachable in the post-outer-dispatch branches), NOT as a
    /// user-facing skip.
    pub(crate) fn extract_for(
        &self,
        media: &MediaType,
        ctx: &ExtractContext<'_>,
        bytes: &[u8],
    ) -> anyhow::Result<CanonicalDocument> {
        let extractor = self.extractors.iter()
            .find(|e| e.supports(media))
            .ok_or_else(|| anyhow::anyhow!(
                "no Extractor for media_type {:?}", media
            ))?;
        extractor.extract(ctx, bytes)
    }
}
```

### §3.7 5 위치 match 의 본 PR 정리 scope (round 1 MAJOR #6 — arm count 정확 표기)

| 위치 | explicit arm 수 (lang cover) | 본 PR 정리 | 이유 |
|---|---|---|---|
| `lib.rs:2012-2049` 의 9 AST arm | 12 arm = 11 explicit + 1 wildcard, cover 17 lang (그 중 9 AST arm) | **정리** | Pattern β 로 `app.extract_for(...)` 단일 호출로 교체. 12 arm 중 9 AST arm 의 `*Extractor::new().extract(…)` 가 사라지고, 7 manifest arm + 1 shell arm + 1 other-bail 만 남는다 (post-state = 4 arm). |
| `lib.rs:2012-2049` 의 7 Tier-2 + 1 Tier-3 arm | (위와 동일 region 의 나머지 4 arm) | **유지** | `synthesize_tier2_document` 가 Extractor 아닌 free function. Extractor 화는 별 PR. |
| `lib.rs:1935-1953` 의 parser_version | 11 explicit arm cover 17 lang | **유지** | `Extractor::parser_version()` method 로 가져올 수 있지만 Tier 2/3 의 sentinel `"none-v1"` 가 hardcoded. inner 통합과 묶임. |
| `lib.rs:1955-1974` 의 chunker_version | 11 explicit arm cover 17 lang | **유지** | Chunker registry 도입 = 별 PR (§2.2 non-goal). |
| `lib.rs:1979-1988` 의 tier3_fallback_cv | 2 arm (positive 16-lang sum + `_ => None`) | **유지** | 동일. |
| `lib.rs:2087-2128` 의 chunk dispatch | 14 explicit arm cover 17 lang | **유지** | 동일. |

본 PR 의 net 효과:

- **lib.rs:2012-2049 region**: **12 arm (11 explicit + 1 wildcard) → 4 arm** [round 3 정정 — plan critic round 2 verifier GAP #5 의 actual count]. 9 AST arm 의 사라짐 → 그 자리에 dispatch loop entry 의 단일 9-AST-group arm 1 줄; 7 manifest arm + 1 shell arm + 1 other-bail wildcard 유지.
- **lib.rs:1296 region (image)**: 1 callsite → 1 callsite (`image_extractor.extract` → `app.extract_for`).
- **lib.rs:1783 region (pdf)**: 1 callsite → 1 callsite (`PdfTextExtractor::new().extract` → `app.extract_for`).
- **lib.rs:356**: 1 local 제거 (`let image_extractor = …`).
- **lib.rs:760 region (ImagePipeline)**: 1 field 제거 (`extractor: &'a ImageExtractor`).
- **lib.rs:1232 region (ingest_one_image_asset signature)**: image_pipeline arg 의 destructure 갱신.

총 변경 site: 5 위치 (image / pdf / 9-AST extract / image_extractor local / ImagePipeline field).

### §3.8 inner 4 위치 match + Chunker dispatch — 본 PR 의 명시적 defer

§2.2 + §2.3 + §3.7 의 종합. 본 PR 은 9 AST extract callsite + image + pdf extract callsite 만 정리. inner 4 위치 match (parser_version / chunker_version / tier3_fallback_cv / chunk dispatch) + Chunker registry 도입 + Tier 2/3 Extractor 화 + MarkdownExtractor 신설은 모두 별 spec 의 work. §11 future work 에 follow-up 후보 명시.

---

## §4 Open questions (closure status — round 2 inline 해소)

round 1 reflection 의 3 OQ + drafting 단계 5 OQ 를 inline 해소.

### §4.1 `build_body_hints` 의 input dependency — **resolved**

`crates/kebab-app/src/lib.rs:2422-2429` 의 actual signature + body 인용:

```rust
fn build_body_hints(asset: &RawAsset) -> BodyHints {
    BodyHints {
        first_h1: None,
        fs_ctime: asset.discovered_at,
        fs_mtime: asset.discovered_at,
        fallback_lang: None,
    }
}
```

input = `&RawAsset` 단독. App-side state (config / sqlite / embedder / …) 침투 0. → 미래 MarkdownExtractor 신설 시 `extract(&ctx, &bytes)` 안에서 `ctx.asset` 으로부터 동일 derive 가능. 본 PR 의 scope 외 (MarkdownExtractor defer) — 별 PR 에서 활용.

### §4.2 `supports()` 의 mutually exclusive 보장 — **resolved**

11 Extractor impl 의 `supports()` 가 mutually exclusive. `MediaType::Markdown` / `MediaType::Pdf` / `MediaType::Image(_)` / `MediaType::Code(l)` 의 4 variant 중 첫 3개는 명백히 disjoint. `MediaType::Code(l)` 의 9 AST lang 의 `supports()` 도 lang string 비교 (rust ≠ python ≠ … ≠ cpp) 로 disjoint. → mutually exclusive. unit test 로 검증 (§5.1 의 grid-search).

### §4.3 markdown warning channel — **resolved (MarkdownExtractor defer 로 자동 해소)**

`lib.rs:1100-1109` 의 `warning_notes: Vec<String>` snapshot + `all_warnings: Vec<Warning>` 의 `build_canonical_document(..., warnings)` 마지막 arg 의 dual sink 가 본 PR 에서 변경 0. → `IngestItem.warnings` 의 wire 형태 변경 0. risk 0.

`crates/kebab-parse-md/src/normalize.rs:60-65` 의 actual signature (round 1 OQ-2):

```rust
pub fn build_canonical_document(
    asset: &RawAsset,
    metadata: Metadata,
    blocks: Vec<ParsedBlock>,
    parser_version: &ParserVersion,
    warnings: Vec<Warning>,
) -> Result<CanonicalDocument> { ... }
```

`warnings: Vec<Warning>` arg 가 함수 body 안에서 `CanonicalDocument.provenance` 의 Warning event 로 lift — 본 PR 에서 변경 0.

### §4.4 Pattern β 의 markdown helper signature 변경 폭 — **resolved (MarkdownExtractor defer)**

본 PR 에서 markdown path 변경 0 → diff line count 0. 미래 MarkdownExtractor 신설 PR 의 work.

### §4.5 verifier 의 wire-identity 검증 방법 — **resolved**

§5.4 에서 명시. `docs/SMOKE.md` 의 isolated TempDir KB ingest + `kebab search/ask --json` output diff = baseline (main HEAD = 9676640) 과 byte-identical. 4 medium fixture table 은 §5.4.1.

### §4.6 `parser_version` source-of-truth dual drift (round 1 MAJOR #3 → OQ-1) — **resolved**

`ingest_with_config_opts` (lib.rs:281-360) 의 chain 추적:

- lib.rs:331 — `let parser_version = ParserVersion(kebab_parse_md::PARSER_VERSION.to_string());` — **markdown 전용**.
- lib.rs:380 (`ingest_with_config_cancellable`) → `ingest_one_asset(app, asset, parser_version: &ParserVersion, ...)` — 이 `parser_version` 이 markdown path 의 lib.rs:1111 `build_canonical_document(asset, metadata, parsed_blocks, parser_version, all_warnings)` 의 4번째 arg 로 흐름.
- `ingest_one_image_asset` (lib.rs:1264) — caller-arg `parser_version` 무시, 자체 `let image_parser_version = ParserVersion(kebab_parse_image::PARSER_VERSION.to_string());` 으로 재build.
- `ingest_one_pdf_asset` (lib.rs:1758) — caller-arg `parser_version` 무시, 자체 `let pdf_parser_version = ParserVersion(kebab_parse_pdf::PARSER_VERSION.to_string());` 재build.
- `ingest_one_code_asset` (lib.rs:1935) — caller-arg 무시, 자체 9-arm match 로 per-lang `RUST_PARSER_VERSION` / `PYTHON_PARSER_VERSION` / ... 재build.

**결론**: `parser_version` caller-arg 의 source-of-truth 는 **markdown path 전용**. image / pdf / code path 모두 자체 const 로 재build → `Extractor::parser_version()` method 와의 dual-drift risk 가 있다. 본 PR 은 `extract_for` 가 `Extractor::extract` 만 호출하고 `parser_version()` 은 호출 안 함 → `CanonicalDocument.parser_version` 의 wire form 은 Extractor 의 `extract` body 안에서 결정 (e.g. ImageExtractor body 의 `let parser_version = self.parser_version();`) — 본 PR 에서 변경 0. dual-source 의 정리는 별 PR (MarkdownExtractor 신설 + Tier 2/3 Extractor 화 + inner-match 통합) 의 work.

### §4.7 ARCHITECTURE.md dispatch flow section (round 1 OQ-3, Missing 1) — **resolved**

`grep -n "dispatch\|registry\|polymorphic\|ingest flow" docs/ARCHITECTURE.md` 결과 = `line 25` 의 "code parser" table 의 chunker / parser version 묘사만 존재. "ingest dispatch flow" section **없음**. → 본 PR 이 ARCHITECTURE.md 갱신 0. §7 의 docs/ARCHITECTURE.md row = "변경 0".

---

## §5 Verification plan

### §5.1 Unit tests (per crate)

- **`kebab-app`** (registry coverage):
  - `App::open_with_config` 호출 후 `app.extractors.len() == 11`.
  - grid-search: 11 Extractor 의 `supports()` 가 `MediaType::Markdown` / `Pdf` / `Image(_)` / `Code("rust"|"python"|...|"cpp")` / `Code("yaml")` / `Code("shell")` / `Audio(_)` / `Other(_)` 의 16 sample MediaType 에 대해 mutually exclusive (어떤 두 Extractor 도 동일 MediaType 에 대해 true 반환 0).
  - `Code("yaml")` / `Code("shell")` / `Code("ruby")` 처럼 registry 가 cover 안 하는 MediaType → `app.extract_for(...)` 가 `Err("no Extractor for media_type ...")` 반환.
- **`kebab-app`** (smoke):
  - `app.extract_for(&MediaType::Markdown, ...)` → markdown path 본 PR 에서 미사용 (별 PR 추가). `Err` 가 정상.
  - `app.extract_for(&MediaType::Image(ImageType::Png), &ctx, &bytes)` → existing ImageExtractor 와 byte-identical result.
  - `app.extract_for(&MediaType::Pdf, &ctx, &bytes)` → existing PdfTextExtractor result.
  - `app.extract_for(&MediaType::Code("rust".into()), &ctx, &bytes)` → existing RustAstExtractor result.

### §5.2 Workspace 회귀 (1313 baseline)

`cargo test --workspace --no-fail-fast -j 1` 의 net delta = +N (registry coverage + grid-search + 4-medium happy-path smoke test 만큼). 기존 ingest happy path test (특히 `kebab-app/tests/ingest_*.rs`, `kebab-app/tests/p10_*.rs`) 전수 pass.

### §5.3 Clippy + build + cargo tree

- `cargo clippy --workspace --all-targets -- -D warnings` clean.
- `cargo build --release` clean.
- `cargo tree -p kebab-app -e normal` 의 결과가 본 refactor 전후로 동일 (4 parser crate 그대로) — `kebab-parse-md / kebab-parse-pdf / kebab-parse-image / kebab-parse-code` 4 line 보존.

### §5.4 ingest happy path manual smoke

`docs/SMOKE.md` 의 isolated TempDir KB 절차 실행.

#### §5.4.1 SMOKE fixture table (round 1 MINOR #2)

| medium | fixture path 후보 | 기대 `--json` schema_version | baseline snapshot |
|---|---|---|---|
| Markdown | `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md` (self-ingest) | `ingest_report.v1` + `search_hit.v1` + `answer.v1` | main HEAD (9676640) 동일 input 결과 |
| PDF | lopdf-decodable fixture (예: `tests/fixtures/sample.pdf` 가 있으면 사용; 없으면 verifier 가 생성) | 동일 | 동일 |
| Image | PNG fixture (`tests/fixtures/sample.png` 또는 verifier 생성) | 동일 (OCR / caption 옵션 default off) | 동일 |
| Code:rust | `crates/kebab-app/src/lib.rs` self-ingest 또는 verifier 가 작은 fixture rust 파일 생성 | 동일 | 동일 |

별 fixture path 가 repo 에 없으면 verifier 가 `_external/` 의 `kebab ingest-file` flow 로 생성 — round 1 Missing 2 의 관심사 (§5.4.2).

#### §5.4.2 `_external/` single-file ingest path (round 1 Missing 2)

`crates/kebab-app/src/lib.rs:2689-2753` 의 `ingest_file_with_config` 가 외부 파일을 `_external/<blake3-12>.<ext>` 로 copy 한 뒤 `ingest_with_config_opts` 로 재진입. → 본 PR 의 polymorphic dispatch 가 동일하게 적용된다 (entry point 통일 → outer 4-arm match → 본 PR 의 `app.extract_for(...)` 단일 호출). `_external/` path 영향 0 — wire schema diff 0 보장에 포함.

### §5.5 wire schema diff = 0 (success path) + error path 의 internal context 예외

§5.4 의 `--json` output 의 `schema_version` field 가 모두 `*.v1` 유지. `IngestReport.v1` / `IngestItem` (특히 `warnings`) / `search_hit.v1` / `answer.v1` 의 field 추가 / 삭제 / 의미 변경 0.

**Exception (round 2 verifier MAJOR #2 of plan): error context string 변경**

본 PR 의 callsite migration 이 `.context("kb-parse-image::ImageExtractor::extract")` → `.context("kb-app::extract_for (image)")` 등의 anyhow context string 을 변경. 이는 `error.v1.message` 의 surface 에 영향 가능 (현재 stderr ndjson 또는 `--json` mode 의 fatal err 출력 surface).

본 변경은 **internal Rust error chain wording 의 변경** — `error.v1.code` (exit code branching 의 source) + `error.v1.schema_version` 보존. message chain 의 internal detail (어느 Rust function 이 anyhow context 를 chained 했는가 의 trace) 변경은 **user-visible surface 정의 외**. claude-code-skill / mcp consumer 의 wire contract 가 `error.v1.code` 의 finite enumeration 에 의존 (e.g. `RefusalSignal` / `NoHitSignal` / `DoctorUnhealthy`) — message chain 의 wording diff 에 의존 0.

risk acceptance: 본 PR 의 error context wording diff 가 `IngestReport.v1.items[].error` field 의 String 표현에 surface 시 diff > 0 surface 가능하나, 본 PR 은 error path 의 분기 의미를 바꾸지 않음 (success path 만 polymorphic dispatch 로 통합 — error 종류 + code + branch 모두 보존). plan 의 verifier 가 success path 의 wire diff 만 verify + error path 의 schema diff 는 manual 검증 (`error.v1.code` 보존 확인).

### §5.6 Integration 통합 영향 (round 1 Missing 3)

`integrations/claude-code/kebab/` (Claude Code skill — `kebab search/ask --json` consumer) 의 wire schema delta 0 → **integration 갱신 0**. CLAUDE.md "Wire schema v1" rule 의 v1→v2 major bump 시 cascade 갱신 의무에 본 PR 미해당 (additive 변경조차 없음).

---

## §6 Risks

### §6.1 ingest happy path 의 runtime regression — Medium mitigation

본 refactor 의 risk = `app.extract_for(...)` 가 `ImageExtractor::extract` / `PdfTextExtractor::extract` / 9 `*AstExtractor::extract` 의 호출 결과를 byte-identical 로 재현하지 못하는 경우. trait dispatch 의 self-method 의 결과는 본질적으로 동일하지만, `Box<dyn Extractor>` 의 vtable lookup 또는 `ExtractContext<'_>` 의 lifetime 처리 차이가 silent regression 으로 surface 할 risk.

**mitigation**: §5.4 의 4-medium SMOKE manual diff + §5.2 의 1313 + N test pass + §5.1 의 grid-search.

본 risk 는 round 1 의 §6.1 risk (markdown warning channel) 와 **다르다**. round 1 risk 는 MarkdownExtractor defer 로 자동 해소 — markdown path 가 본 PR 에서 변경 0 이므로 `Vec<Warning>` channel + `IngestItem.warnings` wire form 영향 0.

### §6.2 registry 의 wrong dispatch — Low mitigation

§4.2 의 mutually-exclusive 검증 + §5.1 의 grid-search. risk 가 깨지면 `find()` 의 first-match 가 ordering-dependent → wire result 가 ordering 의존이 됨 — verifier 가 unit test 로 fail-fast.

### §6.3 state-ful Extractor 의 미래 추가 — Low impact (round 1 MINOR #3 보강)

본 PR 에서는 모든 11 Extractor 가 state-less → init cost 0. 미래에 state-ful Extractor (e.g. LLM-backed image OCR 이 Extractor trait 으로 합쳐질 경우) 가 추가되면 두 migration 패턴:

- **Pattern α**: `OnceLock<Box<dyn Extractor>>` 같은 lazy init wrapper. App init 시 lazy slot 만 등록, first dispatch 시 build.
- **Pattern β**: eager init 시 `Result<Box<dyn Extractor>>` 의 fallible — config 의 enable flag 가 off 면 `None` 으로 skip + dispatch 시 `Err`.

본 PR 의 scope 아님 — 미래 PR 의 design 결정. 본 PR 의 `Vec<Box<dyn Extractor>>` field 는 두 패턴 모두 수용 가능 (Vec entry type 만 swap).

### §6.4 trait object vtable overhead — Negligible

dispatch 가 per-asset 1회 (extract 의 hot loop 0회) → 측정 불가. ingest throughput 영향 0.

### §6.5 partial 정리의 인지 부담 — Medium

본 PR 이 9 AST extract callsite + image + pdf extract callsite 만 정리 → 코드 reader 가 markdown path (자유 함수) + Tier 2/3 path (free-function `synthesize_tier2_document`) + AST extract path (`app.extract_for`) 의 3 비대칭에 confusion. **mitigation**: §3.7 의 table 을 코드 comment 또는 PR description 에 인용 + §11 의 follow-up 명시.

### §6.6 frozen task spec / design contract 침범 — None (verified)

§1.8 의 분석으로 design §7.2 + §8 + §6 모두 변경 0 + frozen task spec 21 file 모두 변경 0 검증. risk 0.

### §6.7 dual-source parser_version drift (round 1 MAJOR #3) — Low impact (정리는 별 PR)

§4.6 의 결론 — `Extractor::parser_version()` method 의 결과 vs caller-arg / 자체 `let *_parser_version` 의 dual-source 가 본 PR 에서 정리되지 않음. 단 본 PR 이 새로운 dual-source 를 도입하지도 않음 → silent regression risk 0 (기존 wire form 그대로). 정리는 별 PR (MarkdownExtractor + Tier 2/3 + inner-match 통합) 의 work — §11 명시.

### §6.8 cargo features 영향 (round 1 Missing 4) — None

`crates/kebab-app/Cargo.toml` + 11 Extractor 의 source crate `Cargo.toml` 의 `[features]` section 검사 — 현재 no feature gate 가 Extractor impl 의 visibility 를 토글하지 않음 (future `vision-ocr` 또는 `audio` feature gate 도입 시 본 PR 의 registry init 이 `#[cfg(feature = "...")]` 의 `vec![]` push 분기로 자연스럽게 적응 가능). 본 PR 에서 feature 신설 0.

---

## §7 Wire / surface impact

| 항목 | 변경 |
|---|---|
| wire schema (`*.v1`) | **success path = 변경 0** — `IngestReport.v1` / `IngestItem.warnings` / `search_hit.v1` / `answer.v1` / `chunk_inspection.v1` / `citation.v1` / `doc_summary.v1` 모두 byte-identical. **error path = `error.v1.message` 의 internal context string wording 변경 가능 (예: `"kb-parse-image::ImageExtractor::extract"` → `"kb-app::extract_for (image)"`)** — `error.v1.code` + `error.v1.schema_version` 보존, message chain 의 wording diff 는 user-visible surface 정의 외 (§5.5 risk acceptance 참조). |
| CLI / TUI / MCP surface | **변경 0** — `kebab ingest` / `search` / `ask` / `doctor` / `reset` / `inspect-chunk` 의 argv + `--json` field 그대로. |
| Cargo `workspace.version` | **bump 불필요** — frozen design contract 변경 0, wire schema 변경 0, V00X migration 0 → CLAUDE.md §Release 룰 3 트리거 미충족. |
| Internal Rust crate-API | `kebab-app::App.extractors` field 추가 (pub(crate), 외부 영향 0) + `App::extract_for(...)` method 추가 (pub(crate)). 기존 `kebab-parse-md / kebab-parse-pdf / kebab-parse-image / kebab-parse-code` 의 `pub` surface 모두 보존. **`kebab-parse-md` 변경 0**. |
| README | **변경 0** — dispatch 통합은 사용자 가시 surface 가 아님. |
| HANDOFF.md | **변경 0** — phase epic 완료 아님 (sub-item 3 의 internal refactor). |
| docs/ARCHITECTURE.md | **변경 0** — §1.9 의 grep 결과 "ingest dispatch flow" section 부재 → 본 PR 이 신설하지 않는 것이 정합. line 25 의 code parser table 은 lang / version family 묘사 — 본 refactor 가 family 를 건드리지 않음. |
| docs/superpowers/specs/2026-04-27-kebab-final-form-design.md | **변경 0** — §7.2 Extractor trait 정의 semantically identical, §8 dep graph 그대로. |
| `integrations/claude-code/kebab/` | **변경 0** — §5.6 의 wire delta 0. |
| tasks/HOTFIXES.md | **append 가능** — refactor 머지 후 한 줄 dated entry (sub-item 1 / 2 와 동일 pattern). |

---

## §8 Out of scope (별 PR / future defer)

1. **MarkdownExtractor 신설** — `kebab-parse-md::MarkdownExtractor` 의 `impl Extractor` + `build_body_hints` 이동 + `Vec<Warning>` channel 의 새 surface 설계. 별 spec.
2. **Tier 2/3 free-function path 의 Extractor 화** — `synthesize_tier2_document` 의 7 manifest + 1 shell lang 을 `*Extractor` impl 로 승격.
3. **Chunker dispatch unification** — `Chunker` trait 에 `supports()` 신설 + `App.chunkers: Vec<Box<dyn Chunker>>` registry + `lib.rs:2087-2128` 의 chunk dispatch 통합. design §7.2 갱신 동반 → 별 spec.
4. **inner 4 위치 match (parser_version / chunker_version / tier3_fallback_cv / chunk dispatch) 의 polymorphic 통합** — Chunker registry + Tier 2/3 Extractor 화와 묶임.
5. **ExtractorRegistry plugin system** — App field 가 아닌 별 type + dynamic-loading.
6. **dual-source `parser_version` 정리** — §6.7 의 risk. 별 PR.

---

## §9 References

- `crates/kebab-core/src/traits.rs:115-132` — Extractor + Chunker trait 정의 (§1.1, §1.2).
- `crates/kebab-app/src/lib.rs:961-1040` — `ingest_one_asset` outer dispatch (§1.5).
- `crates/kebab-app/src/lib.rs:281-360` — `ingest_with_config_opts` (§4.6).
- `crates/kebab-app/src/lib.rs:760-764` — ImagePipeline struct (§3.5.1).
- `crates/kebab-app/src/lib.rs:1232` — `ingest_one_image_asset` signature head.
- `crates/kebab-app/src/lib.rs:1296` — image extract callsite (§3.7).
- `crates/kebab-app/src/lib.rs:1783` — pdf extract callsite (§3.7).
- `crates/kebab-app/src/lib.rs:1935-2128` — code dispatch 5 위치 match (§1.3, §1.4, §3.7).
- `crates/kebab-app/src/lib.rs:2422-2429` — `build_body_hints` (§4.1).
- `crates/kebab-app/src/lib.rs:2689-2753` — `ingest_file_with_config` (§5.4.2).
- `crates/kebab-app/src/app.rs:115` — App struct (§1.6).
- `crates/kebab-parse-md/src/normalize.rs:60-65` — `build_canonical_document` signature (§4.3).
- `crates/kebab-parse-md/src/frontmatter.rs:34-44` — `BodyHints` struct (§4.1).
- 11 Extractor impl: §1.1 의 table.
- 15 Chunker impl: §1.2 의 table.
- design `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md` §7.2 (`:1416-1420`) / §8 (`:1475+`) (§1.8).
- sibling spec: `2026-05-26-source-fs-dep-lightening-spec.md` (sub-item 1, PR #185 merged).
- sibling spec: `2026-05-26-normalize-absorption-spec.md` (sub-item 2, PR #186 merged).

---

## §10 Round closure status table

| round 1 finding | severity | closure | reflection 위치 |
|---|---|---|---|
| CRITICAL #1 (BodyHints field 부정확) | CRITICAL | resolved | §3.4 (defer note) + §4.1 (resolved 의 actual signature 인용) — `first_h1 / fs_ctime / fs_mtime / fallback_lang` 4 field 명시 + `&RawAsset` 단독 derive. |
| CRITICAL #2 (byte-identical 인용 과장) | CRITICAL | resolved | §1.1 (인용 표현 → "trait 정의 인용") + §1.8 ("semantically identical" 로 약화) + §3.2 (trait 갱신 0 보존 invariant 만 유지). |
| MAJOR #1 (§2.1 Goal #1 vs §3.6/§3.7 모순) | MAJOR | resolved | §2.1 Goal #1 재작성 — "inner AST 9-arm extract dispatch 통합" 으로 명확화. spec title 도 "AST 9-arm extract dispatch" 로 변경. |
| MAJOR #2 (Pattern β warning channel 미해결) | MAJOR | resolved (Option (ii) 채택) | §2.2 #1 + §3.4 + §6.1 + §11 — MarkdownExtractor defer. wire risk 0. |
| MAJOR #3 (parser_version dual-source) | MAJOR | resolved | §4.6 (OQ-1 inline 해소) + §6.7 (risk 명시 + 정리는 별 PR). |
| MAJOR #4 (ImagePipeline.extractor lifecycle) | MAJOR | resolved (Option c 채택) | §3.5.1 + §2.1 Goal #2 — field 제거 + local 제거. |
| MAJOR #5 (code_lang count off-by-one) | MAJOR | resolved | §1.4 ("16" → "17") + §1.3 (lib.rs:1009-1013 outer guard 인용). |
| MAJOR #6 (5 위치 arm count 부정확) | MAJOR | resolved | §3.7 table — explicit arm 수 (lang cover) 형식 통일 + 1935/1955/1979/2012/2087 실 lib.rs 인용 검증. |
| MAJOR #7 (kebab-parse-md re-export 누락) | MAJOR | resolved | §3.4 defer note 안에 future PR work 로 `pub mod extractor;` + `pub use ...` 명시. |
| MINOR #1 (App struct lifecycle 보강) | MINOR | resolved | §1.6 — embedder/vector/llm lazy + pipeline_verifier eager 주석. |
| MINOR #2 (SMOKE fixture 3-column table) | MINOR | resolved | §5.4.1 의 4-medium table. |
| MINOR #3 (state-ful Extractor migration 보강) | MINOR | resolved | §6.3 의 Pattern α/β 두 wrapper 패턴 명시. |
| MINOR #4 (round 2 sonnet closure verify only) | MINOR | resolved | §10 status table (이 표) 의 round 2 row 의 mode 명시. |
| NIT #1 (spec title "9-arm") | NIT | resolved | spec title 재작성 (위). |
| NIT #2 (sample code trailing comma) | NIT | resolved | §3.5 의 vec![] block 의 trailing comma 정리. |
| Missing 1 (ARCHITECTURE.md dispatch flow grep) | Missing | resolved | §1.9 + §4.7 — grep 결과 = section 부재 → 변경 0. |
| Missing 2 (`_external/` ingest path 영향) | Missing | resolved | §5.4.2 — `ingest_file_with_config` 가 `ingest_with_config_opts` 재진입 → 영향 0. |
| Missing 3 (integration update 0 명시) | Missing | resolved | §5.6 — wire delta 0 → integration 갱신 0. |
| Missing 4 (cargo features 영향) | Missing | resolved | §6.8 — 현재 no feature gate. |
| Ambiguity 1 (§3.5 ordering invariant 의미) | Ambiguity | resolved | §3.5 의 ordering invariant 보강 — (A) safety guard only, (B) wire contract NOT. |
| Ambiguity 2 (ingest_one_image_asset polymorphic dispatch) | Ambiguity | resolved | §3.5.1 + §3.7 + §3.6 의 Pattern β 명시. |
| OQ-1 (`parser_version` source-of-truth grep) | OQ | resolved | §4.6. |
| OQ-2 (`build_canonical_document` signature grep) | OQ | resolved | §4.3. |
| OQ-3 (ARCHITECTURE.md dispatch grep) | OQ | resolved | §4.7 + §1.9. |

| round | reviewer | mode | status | notes |
|---|---|---|---|---|
| 0 (drafting) | planner (self) | full | drafted | spec body 작성 완료. |
| 1 | critic (opus) | full | REQUEST_CHANGES | 2 CRITICAL + 7 MAJOR + 4 MINOR + 2 NIT + 4 Missing + 2 Ambiguity + 3 OQ. |
| 2 (reflection) | planner (self) | full rewrite | reflected | 위 status table 의 모든 finding closure. MarkdownExtractor defer (Option (ii)) 핵심 결정 — scope 가 "AST 9-arm extract dispatch + image + pdf extract" 로 축소. |
| 3 | critic (sonnet) | **closure verify only** | pending | round 2 의 reflection 이 round 1 finding 을 모두 closure 했는지 검증. |
| 4+ | as needed | — | pending | — |

---

## §11 Future work (별 PR / sibling spec 후보)

본 PR 머지 후의 follow-up. 우선순위 + 의존성 순서:

1. **MarkdownExtractor 신설** — `kebab-parse-md` 에 `impl Extractor for MarkdownExtractor` 추가. `build_body_hints` 의 이동. `Vec<Warning>` channel 의 새 surface 설계 — 두 option:
   - **(α) wire schema additive minor bump** — `IngestItem.warnings` 의 source 가 `CanonicalDocument.provenance.warnings` 가 되도록 lift. wire form `additive` 변경 (workspace.version minor bump 트리거).
   - **(β) `extract()` 의 별 channel** — `Extractor::extract` signature 갱신 → design §7.2 갱신 → frozen contract 변경. release cycle 영향 큼.
   spec 작성 시 option 선정.
2. **Tier 2/3 free-function path 의 Extractor 화** — `synthesize_tier2_document` 를 `K8sManifestExtractor` / `DockerfileExtractor` / `ManifestFileExtractor` / `ShellExtractor` 의 4 impl 로 분리. App.extractors 에 4 entry 추가.
3. **Chunker dispatch unification** — `Chunker::supports(&CanonicalDocument or &ChunkerVersion)` 신설 (design §7.2 갱신) + `App.chunkers: Vec<Box<dyn Chunker>>` registry + `lib.rs:2087-2128` chunk dispatch 통합 + `chunker_version` 결정도 chunker.chunker_version() polymorphic.
4. **inner 4 위치 match 전체 통합** — parser_version / chunker_version / tier3_fallback_cv / chunk dispatch — Tier 2/3 Extractor 화 + Chunker registry 완료 후 자연스럽게 단일 dispatch loop 으로 통합 가능.
5. **outer 4-arm helper 통합** — `ingest_one_image_asset` / `ingest_one_pdf_asset` / `ingest_one_code_asset` 의 helper 분기를 단일 dispatch loop 으로 흡수. post-extract pipeline (OCR / page-chunker / tier3-fallback / try-skip-unchanged) 의 trait 화 동반.
6. **dual-source `parser_version` 정리** — `Extractor::parser_version()` method 의 결과를 single source-of-truth 로 강제. caller-arg 의 markdown 전용 hardcoded 제거.
7. **ExtractorRegistry plugin system** — App field 가 아닌 별 type + dynamic-loading. (low priority, design-only)

#1 + #2 + #3 이 본 PR 머지 후 다음 milestone (v0.19.0 minor bump 동반 가능).
