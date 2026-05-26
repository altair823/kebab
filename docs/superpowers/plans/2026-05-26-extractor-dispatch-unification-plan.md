---
status: open
target_version: 0.18.0
spec: docs/superpowers/specs/2026-05-26-extractor-dispatch-unification-spec.md
contract_sections: []
related_specs:
  - docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
  - docs/superpowers/specs/2026-05-26-source-fs-dep-lightening-spec.md
  - docs/superpowers/specs/2026-05-26-normalize-absorption-spec.md
sibling_plans:
  - docs/superpowers/plans/2026-05-26-source-fs-dep-lightening-plan.md   # PR #185 merged
  - docs/superpowers/plans/2026-05-26-normalize-absorption-plan.md       # PR #186 merged
---

# Extractor dispatch unification — implementation plan (v3 — reflection round 2)

> plan round 1 의 16 finding (1 CRITICAL + 5 MAJOR + 6 MINOR + 1 NIT + 4 Missing + 1 Ambiguity) 흡수. spec §5.5 + §7 의 error path wire-scope risk acceptance 동반 갱신. scope = AST 9-arm extract dispatch + image/pdf extract callsite. 11 step, atomic block 1 (Step 4-5-6) + mutually independent (Step 6/7/8). v2 의 7 step plan rewrite 가 instruction 의 fine-grained sequencing 정합 — round 1 finding 의 actual codebase grep 결과로 다수 정정.

## §0 Pre-flight + branch state

- **Branch**: `refactor/extractor-dispatch-unification` (현재 위치).
- **Base SHA**: `9676640` (PR #186 sibling — normalize-absorption 머지 직후, v0.18.0 cut 시점).
- **Working dir**: `/home/altair823/kebab`.
- **Env 강제** (`~/.claude/CLAUDE.md` 의 "Disk Layout — 루트 디스크 보호가 최우선" 룰):
  - `export CARGO_TARGET_DIR=/build/out/cargo-target/target` — 본 plan 의 모든 cargo 명령 적용. repo root 의 `target/` 생성 방지 (16 GiB RAM 머신의 `/` 250 G 보호).
  - `export TMPDIR=/build/cache/tmp` — 대용량 임시 파일 발생 시 보호.
- **Cargo build 직렬화** (MEMORY.md `feedback_serial_build_only.md` — 사용자 결정 2026-05-26):
  - **per-crate cargo**: `-j 4` default (예: `cargo build -p kebab-app -j 4`).
  - **full workspace** (`cargo test --workspace`, `cargo clippy --workspace`): `-j 1` 강제. 18 integration-test binary 동시 link 시 OOM (linker SIGKILL).
  - cargo test / clippy / build 동시 background 실행 금지. 직렬 진행.
- **`target/` clean policy**: full workspace test 직전 `cargo clean` 1회 (Step 11). 중간 step (Step 2-10) 은 per-crate incremental build — `cargo clean` 불필요.
- **HOTFIXES.md / HANDOFF.md / README.md / docs/ARCHITECTURE.md 변경 0** (spec §7 + §1.9 verified).
- **21+ frozen task spec 변경 0** (spec §1.8 verified).
- **wire schema 변경 0 — success path** (spec §5.5). **error path = `error.v1.message` 의 internal context wording 변경 가능** — `error.v1.code` + `error.v1.schema_version` 보존 (spec §5.5 의 risk acceptance + §7 row 동반 갱신).
- **workspace `Cargo.toml` version bump 0** (`target_version: 0.18.0` 유지, CLAUDE.md §Release 룰 3 트리거 미충족).
- **design contract 변경 0** (`contract_sections: []`).
- **doc-test 포함 여부 (MINOR #2 fix)**: Step 1 + Step 11 의 baseline / after awk sum 이 doc-test (예: `running 0 tests` 의 doc-test result 라인) 도 cover. doc-test 가 0 이면 sum 영향 0; 가산 결과는 baseline + after 모두에 동일 가산되어 delta 보존.

## §1 Approach summary

Spec §3 의 결정을 단계별 atomic step 으로 decompose. destination = `App` field (Option A, spec §3.1). 핵심 sequencing:

1. **Pre-flight + 무변경 baseline** (Step 1) — 측정 only.
2. **Registry surface 부터 build-up** (Step 2-3) — `App.extractors` field + `App::extract_for` helper 신설 (Step 2: struct/method shape + placeholder init) → `App::open_with_config` 의 11-entry init (Step 3: replace placeholder + lib.rs:1235 explicit cleanup). 두 step 합쳐 helper 가 사용 가능 상태 — callsite migration 의 전제 조건.
3. **image dispatch migration** (Step 4-6, logical atomic block) — local 제거 (Step 4) → ImagePipeline 갱신 (Step 5) → dispatch callsite 교체 (Step 6). 세 step 의 intermediate state 에서 build red 가능, Step 6 후 build green.
4. **pdf dispatch migration** (Step 7) — 단일 callsite 교체. 가장 작은 atomic step.
5. **code AST 9-arm hoist** (Step 8) — 가장 risk 큰 step. **12 arm (11 explicit + 1 wildcard)** → **4 arm** (9-AST-group + manifest-group + shell + wildcard) [round 1 MINOR GAP #5 정정].
6. **dead code 정리** (Step 9) — Step 4-8 의 결과로 사용 안 되는 use statement / 임시 `#[allow(dead_code)]` 정리.
7. **unit tests 추가** (Step 10) — spec §5.1 의 3 test class. **in-crate `#[cfg(test)] mod tests` in app.rs** (round 1 CRITICAL #1 — `pub(crate)` access).
8. **workspace 회귀 + clean commit** (Step 11) — 7 cargo gate + 4 wire diff + 3 callsite-count verify + numeric delta gate + single commit.

ordering 의 핵심 invariant:

- **Step 2-3 < Step 4-8**: registry + helper 가 사용 가능한 후에 callsite 교체. Step 2 후 build green (additive — placeholder), Step 3 후 build green (real init + lib.rs:1235 cleanup).
- **Step 4-6 는 logical atomic — single commit 단위**: 중간 state 에서 build red 가능. team-lead 의 fine-grained split 은 review/closure granularity 위함이지 commit 단위 분리 의도 아님.
- **Step 7 + Step 8 mutually independent** — pdf 와 code 의 dispatch site 가 별 helper 함수.
- **Step 9 < Step 10**: dead code 정리 후 unit test 추가 (clippy clean 상태에서 test 작성).
- **Step 10 < Step 11**: unit test 가 먼저 + full workspace test 다음.

## §2 Steps (11 steps)

### Step 1: Pre-flight baseline 측정 + env 확인

- **Files affected**: 변경 0 (측정 only).
- **Action**:
  - `cd /home/altair823/kebab && git rev-parse HEAD` → `9676640` 또는 그 위 commit 확인.
  - env 확인: `echo $CARGO_TARGET_DIR` 가 `/build/out/cargo-target/target` 인지. 비어있으면 §0 의 export 적용.
  - workspace baseline crate count: `cargo metadata --no-deps --format-version 1 | jq '.workspace_members | length'` → **22** (PR #186 머지 후).
  - baseline test 함수 수 persist (spec §5.2 의 1313 baseline — Step 11 의 numeric compare gate):
    ```bash
    $ mkdir -p .omc/state
    $ cargo test --workspace --no-fail-fast -j 1 2>&1 \
        | awk '/^test result: ok\./ {for(i=1;i<=NF;i++) if($i=="passed;") sum += $(i-1)} END {print sum}' \
        > .omc/state/extractor-dispatch-baseline.txt
    $ cat .omc/state/extractor-dispatch-baseline.txt
    1313  # 예상. doc-test 의 `running 0 tests` 라인도 awk 의 `test result: ok.` 매칭에 합쳐짐 — delta 보존 (MINOR #2 fix).
    ```
  - hardcoded callsite count baseline 측정 (Step 11 의 callsite-count verify 비교 source). MINOR GAP #6 의 instance-method pattern 보강:
    ```bash
    $ grep -nE "ImageExtractor::new|PdfTextExtractor::new|(Rust|Python|Typescript|Javascript|Go|Java|Kotlin|C|Cpp)AstExtractor::new" crates/kebab-app/src/lib.rs
    # 예상: 11 hit (image 1 + pdf 1 + 9 AST — type-direct call).
    $ grep -nE "image_extractor\.extract|image_pipeline\.extractor\.extract" crates/kebab-app/src/lib.rs
    # 예상: 1 hit (lib.rs:1296 의 instance-method call).
    $ grep -c "image_extractor" crates/kebab-app/src/lib.rs
    # 예상: ≥ 3 hit (lib.rs:356 local + lib.rs:1235 alias + lib.rs:1296 dispatch).
    ```
  - **wire baseline snapshot — falsifiable cmd (round 1 MAJOR #4 fix)**:
    ```bash
    $ mkdir -p .omc/state/wire-baseline /tmp/kb-wire-baseline
    # config.toml 생성 — docs/SMOKE.md 의 isolated TempDir KB 절차 정합.
    $ cat > /tmp/kb-wire-baseline/config.toml <<'EOF'
    [workspace]
    root = "/tmp/kb-wire-baseline/ws"
    data_dir = "/tmp/kb-wire-baseline/data"
    exclude = []

    [search]
    cache_capacity = 0

    [rag]
    nli_threshold = 0.0
    EOF
    $ mkdir -p /tmp/kb-wire-baseline/ws /tmp/kb-wire-baseline/data
    # 4-medium fixture (markdown / pdf / png / rust) 의 ingest + search + ask:
    $ cp crates/kebab-app/src/lib.rs /tmp/kb-wire-baseline/ws/lib.rs              # rust code fixture
    $ cp README.md /tmp/kb-wire-baseline/ws/                                       # markdown fixture
    $ cargo run --release --bin kebab -- --config /tmp/kb-wire-baseline/config.toml ingest --json \
        > .omc/state/wire-baseline/ingest_report.json
    $ cargo run --release --bin kebab -- --config /tmp/kb-wire-baseline/config.toml search "polymorphic dispatch" --json \
        > .omc/state/wire-baseline/search.json
    $ cargo run --release --bin kebab -- --config /tmp/kb-wire-baseline/config.toml ask "what is extract_for" --json \
        > .omc/state/wire-baseline/answer.json
    ```
    PDF / PNG fixture 가 repo 에 없으면 markdown + rust 의 2-medium 만으로도 wire diff 검증 충분 (success path 의 4 medium 중 2 만 cover, 나머지 image/pdf 는 §4.3 의 callsite-count verify 로 covered). 본 plan 의 fixture path 명시 (Missing #4 fix).
- **Exit gate**:
  - `cargo metadata --no-deps --format-version 1 | jq '.workspace_members | length'` = **22**.
  - `cargo build --workspace -j 1 2>&1 | tail -3` 의 마지막 라인 = `Finished` (현 시점 baseline green).
  - `cat .omc/state/extractor-dispatch-baseline.txt` = 1313 (또는 실측치).
  - `ls -1 .omc/state/wire-baseline/*.json | wc -l` = **3** (ingest_report.json + search.json + answer.json).
- **Spec 참조**: §5.2 (baseline), §1.3 (callsite enumeration), §5.4 (SMOKE).

### Step 2: `App.extractors` field + `App::extract_for` helper method shape 신설 (placeholder init)

- **Files affected**:
  - `crates/kebab-app/src/app.rs` (단일 — struct + impl method + use statement).
- **Action**:
  - **(a) use statement 추가** — `app.rs` head 의 use 부에 다음 추가 (round 1 MAJOR #3 의 use 정책 정합 — short-name 사용):
    ```rust
    use kebab_parse_image::ImageExtractor;
    use kebab_parse_pdf::PdfTextExtractor;
    use kebab_parse_code::{
        CAstExtractor, CppAstExtractor, GoAstExtractor, JavaAstExtractor,
        JavascriptAstExtractor, KotlinAstExtractor, PythonAstExtractor,
        RustAstExtractor, TypescriptAstExtractor,
    };
    ```
    (이미 use 가 있으면 skip).
  - **(b) `App` struct 갱신** — `crates/kebab-app/src/app.rs:115` 의 struct 에 field 추가:
    ```rust
    pub struct App {
        pub(crate) config: kebab_config::Config,
        pub(crate) sqlite: Arc<SqliteStore>,
        /// post-v0.18.0: inner-AST 9-arm extract dispatch + image/pdf
        /// extract callsite 통합. App init 시 1회 등록. markdown 은 별 PR.
        pub(crate) extractors: Vec<Box<dyn kebab_core::Extractor + Send + Sync>>,
        embedder: OnceLock<Arc<dyn Embedder + Send + Sync>>,
        vector: OnceLock<Arc<LanceVectorStore>>,
        llm: OnceLock<Arc<dyn LanguageModel>>,
        search_cache: Option<Mutex<LruCache<SearchCacheKey, Vec<SearchHit>>>>,
        pipeline_verifier: Option<Arc<dyn kebab_nli::NliVerifier>>,
    }
    ```
  - **(c) `App::extract_for` helper method 추가** — `impl App { ... }` 안에 spec §3.6 의 코드 그대로:
    ```rust
    /// Polymorphic dispatcher for the Extractor trait. Looks up the first
    /// Extractor whose `supports(media)` returns true and invokes
    /// `extract(ctx, bytes)` on it.
    ///
    /// Errors with `anyhow!("no Extractor for media_type {media:?}")`
    /// when no matching Extractor is registered — caller (e.g.
    /// `ingest_one_*_asset`) should treat this as a programming error
    /// (unreachable in the post-outer-dispatch branches), NOT as a
    /// user-facing skip.
    pub(crate) fn extract_for(
        &self,
        media: &kebab_core::MediaType,
        ctx: &kebab_core::ExtractContext<'_>,
        bytes: &[u8],
    ) -> anyhow::Result<kebab_core::CanonicalDocument> {
        let extractor = self.extractors.iter()
            .find(|e| e.supports(media))
            .ok_or_else(|| anyhow::anyhow!(
                "no Extractor for media_type {:?}", media
            ))?;
        extractor.extract(ctx, bytes)
    }
    ```
  - **(d) `App::open_with_config` 의 constructor placeholder** — field missing 회피 위해 `extractors: Vec::new()` 임시 placeholder:
    ```rust
    Ok(Self {
        config,
        sqlite: Arc::new(sqlite),
        extractors: Vec::new(),  // Step 3 에서 real init 으로 replace
        embedder: OnceLock::new(),
        ...
    })
    ```
  - clippy 의 dead-code warn 발생 가능 (extract_for unused + extractors always-empty) — Step 3 머지 시 자동 해소. fail 시 `#[allow(dead_code)]` 임시 부착 (Step 9 의 cleanup checklist 항목).
- **Exit gate**:
  - `cargo build -p kebab-app -j 4 2>&1 | tail -3` 의 마지막 라인 = `Finished`.
  - `grep -c "pub(crate) extractors:" crates/kebab-app/src/app.rs` = **1** (struct field 등장).
  - `grep -c "fn extract_for" crates/kebab-app/src/app.rs` = **1** (method 정의 등장).
  - `grep -c "extractors: Vec::new()" crates/kebab-app/src/app.rs` = **1** (placeholder).
- **Spec 참조**: §3.5 (struct), §3.6 (helper).

### Step 3: `App::open_with_config` 의 registry init (11 Extractor) + lib.rs:1235 alias 제거

- **Files affected**:
  - `crates/kebab-app/src/app.rs` (단일 — open_with_config body).
  - `crates/kebab-app/src/lib.rs` (단일 — :1235 alias line 삭제, round 1 MAJOR #5 fix).
- **Action**:
  - **(a) `App::open_with_config` 의 placeholder 교체** — `pipeline_verifier` init 의 직전 (Missing #2 fix — init order 자연 위치, state-less + side-effect 0 추가) 에 real init:
    ```rust
    // pipeline_verifier init 직전:
    let extractors: Vec<Box<dyn kebab_core::Extractor + Send + Sync>> = vec![
        Box::new(ImageExtractor::new()),
        Box::new(PdfTextExtractor::new()),
        Box::new(RustAstExtractor::new()),
        Box::new(PythonAstExtractor::new()),
        Box::new(TypescriptAstExtractor::new()),
        Box::new(JavascriptAstExtractor::new()),
        Box::new(GoAstExtractor::new()),
        Box::new(JavaAstExtractor::new()),
        Box::new(KotlinAstExtractor::new()),
        Box::new(CAstExtractor::new()),
        Box::new(CppAstExtractor::new()),
    ];

    // (기존 pipeline_verifier init ...)

    Ok(Self {
        config,
        sqlite: Arc::new(sqlite),
        extractors,                       // placeholder Vec::new() → real init replace.
        embedder: OnceLock::new(),
        ...
    })
    ```
    init order rationale (Missing #2): sqlite (heavy I/O) → search_cache (light) → pipeline_verifier (가능한 fallible NLI build) → extractors (state-less, cheap, infallible) → `Ok(Self)`. extractors 의 자연 위치 = `pipeline_verifier` 직전 또는 직후 — `pipeline_verifier` 가 fallible (`?`) 이므로 그 직전 (init order 가 fail-fast 의 cost 와 정합) 또는 직후. **본 plan 은 `pipeline_verifier` 직전** 으로 정합.

    state-less + side-effect 0 (Missing #3): 모든 11 impl 의 `new()` = unit-struct 또는 zero-field. `pipeline_verifier` 의 `Err` 가 발생해도 `extractors` lifetime 가 `Ok(Self)` 까지만 — drop 시 cost 0. side-effect 0.

  - **(b) lib.rs:1235 의 alias line 삭제** (round 1 MAJOR #5 + MINOR GAP #4 fix):
    ```rust
    // BEFORE (lib.rs:1235-1237):
    let image_extractor = image_pipeline.extractor;     // ← 삭제 (struct field 가 Step 5 에서 제거됨)
    let ocr_engine = image_pipeline.ocr_engine;         // 유지
    let caption_llm = image_pipeline.caption_llm;       // 유지

    // AFTER:
    let ocr_engine = image_pipeline.ocr_engine;
    let caption_llm = image_pipeline.caption_llm;
    ```
    **타이밍**: 본 step (Step 3) 에서 alias line 1235 만 삭제. ImagePipeline.extractor field 는 Step 5 에서 제거. 본 step 후 lib.rs:1296 의 `image_extractor.extract(...)` 가 unresolved → build red. Step 4 + 5 + 6 atomic block 의 일부 — Step 6 후에야 build green.

    이 순서가 ImagePipeline.extractor field 삭제 (Step 5) 이전에 alias line 1235 삭제 (Step 3) — interpretation A vs B 의 명시 (Ambiguity #1 fix). **본 plan 의 interpretation = "lib.rs:1235 alias 를 Step 3 에서 먼저 삭제, ImagePipeline struct + init block 은 Step 5 에서 재작성"**.

  - **(c) pre-flight: `grep "kebab-parse-" crates/kebab-app/Cargo.toml`** → 4 dep (md/pdf/image/code) 모두 보유 verify. spec §1.8 의 dep graph 보존.
- **Exit gate**:
  - `grep -c "Box::new(.*Extractor::new())" crates/kebab-app/src/app.rs` = **11** (11 entry).
  - per-line breakdown verify:
    ```bash
    $ grep -c "Box::new(ImageExtractor::new" crates/kebab-app/src/app.rs
    1
    $ grep -c "Box::new(PdfTextExtractor::new" crates/kebab-app/src/app.rs
    1
    $ grep -cE "Box::new\((Rust|Python|Typescript|Javascript|Go|Java|Kotlin|C|Cpp)AstExtractor::new\(\)\)" crates/kebab-app/src/app.rs
    9
    ```
  - `grep -c "extractors: Vec::new()" crates/kebab-app/src/app.rs` = **0** (placeholder 제거).
  - `grep -n "let image_extractor = image_pipeline.extractor" crates/kebab-app/src/lib.rs` = **0 hit** (alias 삭제).
  - `cargo build -p kebab-app -j 4 2>&1 | tail -3` — **build red 예상** (lib.rs:1296 의 `image_extractor.extract` 가 unresolved). Step 6 후 종합 verify. **본 step 의 build verify 는 next exit gate 의 Step 6 에서**.
- **Spec 참조**: §3.5 (registry init 11 entry), §3.5.1 (lib.rs:1235 alias 정리).

### Step 4: lib.rs:356 의 local `image_extractor` 제거 (atomic block 1 시작)

- **Files affected**:
  - `crates/kebab-app/src/lib.rs` (단일 — :356 부근).
- **Action**:
  - lib.rs:356 의 `let image_extractor = ImageExtractor::new();` 1 줄 삭제.
  - lib.rs:357 부근 의 `ImagePipeline { extractor: &image_extractor, ... }` 의 `extractor: &image_extractor,` line 도 동시 삭제 (Step 5 의 struct field 제거와 정합 — interpretation A 의 단순화).
  - 본 step 단독으로는 ImagePipeline struct 의 `extractor` field 가 아직 존재 (Step 5 에서 제거) → init block 의 `extractor: ...` 없는 상태가 missing-field error → **build red**. Step 5 + 6 와 atomic block close.
- **Exit gate**:
  - `grep -c "let image_extractor = ImageExtractor::new" crates/kebab-app/src/lib.rs` = **0**.
  - `grep -A3 "let image_pipeline = ImagePipeline" crates/kebab-app/src/lib.rs | grep -c "extractor:"` = **0**.
  - `cargo build -p kebab-app -j 4 2>&1 | tail -3` — **red 가능** (Step 5 + 6 와 atomic). Step 6 의 exit gate 에서 종합 verify.
- **Spec 참조**: §3.5.1 (lib.rs:356 local 제거).

### Step 5: `ImagePipeline.extractor` field 제거 + struct/사용 site 갱신

- **Files affected**:
  - `crates/kebab-app/src/lib.rs` (단일 — :760-764 struct).
- **Action**:
  - lib.rs:760-764 의 struct 갱신 (spec §3.5.1 Option c):
    ```rust
    // BEFORE
    struct ImagePipeline<'a> {
        extractor: &'a ImageExtractor,
        ocr_engine: Option<&'a OllamaVisionOcr>,
        caption_llm: Option<&'a dyn LanguageModel>,
    }

    // AFTER
    struct ImagePipeline<'a> {
        ocr_engine: Option<&'a OllamaVisionOcr>,
        caption_llm: Option<&'a dyn LanguageModel>,
    }
    ```
  - Step 3 (b) 의 lib.rs:1235 alias 가 이미 삭제됨 + Step 4 의 init block `extractor: &image_extractor,` 가 이미 삭제됨 → 본 step 후 `image_pipeline.extractor` 의 모든 reference 제거 완료.
  - **본 step 단독** 으로는 lib.rs:1296 의 `image_extractor.extract(...)` 가 still unresolved (Step 4 의 local 삭제 + Step 3 의 alias 삭제 후) → **build red**. Step 6 후 close.
- **Exit gate**:
  - `grep -c "extractor: &" crates/kebab-app/src/lib.rs` = **0** (struct field + init block 모두 제거).
  - `grep -A3 "struct ImagePipeline" crates/kebab-app/src/lib.rs | grep -c "extractor:"` = **0**.
  - `cargo build -p kebab-app -j 4 2>&1 | tail -3` — **red 가능** (Step 6 의 exit gate 에서 종합 verify).
- **Spec 참조**: §3.5.1 (ImagePipeline Option c — field 제거).

### Step 6: lib.rs:1296 image extract callsite — `extract_for` 로 교체 (atomic block 1 close)

- **Files affected**:
  - `crates/kebab-app/src/lib.rs` (단일 — :1296 부근).
- **Action**:
  - lib.rs:1296 의 dispatch callsite 교체:
    ```rust
    // BEFORE
    let mut canonical = image_extractor
        .extract(&ctx, &bytes)
        .context("kb-parse-image::ImageExtractor::extract")?;

    // AFTER
    let mut canonical = app
        .extract_for(&asset.media_type, &ctx, &bytes)
        .context("kb-app::extract_for (image)")?;
    ```
  - 본 step 후 Step 3-4-5-6 의 4-step block close — build green 보장.
  - additional grep (혹시 다른 `image_extractor` ref 가 남아있는지):
    ```bash
    $ grep -n "image_extractor" crates/kebab-app/src/lib.rs
    # 예상: 0 hit (Step 3 alias + Step 4 local + Step 6 dispatch 모두 제거).
    ```
- **Exit gate (Step 3-4-5-6 atomic block 종합)**:
  - `grep -c "image_extractor" crates/kebab-app/src/lib.rs` = **0**.
  - `grep -c "image_pipeline.extractor" crates/kebab-app/src/lib.rs` = **0**.
  - `grep -c "app.extract_for" crates/kebab-app/src/lib.rs` ≥ **1** (image dispatch).
  - `cargo build -p kebab-app -j 4 2>&1 | tail -3` = `Finished` (atomic block close 후 build green).
  - `cargo test -p kebab-app -j 4 --no-fail-fast 2>&1 | tail -10` — 기존 image ingest test 가 모두 pass.
- **Spec 참조**: §3.6 (Pattern β image), §3.7 (image row).

### Step 7: lib.rs:1783 pdf extract callsite — `extract_for` 로 교체

- **Files affected**:
  - `crates/kebab-app/src/lib.rs` (단일 — :1783 부근).
- **Action**:
  - lib.rs:1783 의 dispatch callsite 교체:
    ```rust
    // BEFORE
    let mut canonical = PdfTextExtractor::new()
        .extract(&ctx, &bytes)
        .context("kb-parse-pdf::PdfTextExtractor::extract")?;

    // AFTER
    let mut canonical = app
        .extract_for(&asset.media_type, &ctx, &bytes)
        .context("kb-app::extract_for (pdf)")?;
    ```
  - use 선언 `use kebab_parse_pdf::PdfTextExtractor;` (lib.rs:53) — 본 step 후 `PdfTextExtractor` 의 short-name 참조가 lib.rs 안에 0 (registry init 은 app.rs 안에 short-name 사용). 따라서 lib.rs 의 use 가 unused → clippy warn. **Step 9 의 dead-code 정리에서 처리**.
  - **wire diff scope** (round 1 MAJOR verifier #2 + spec §5.5 갱신 정합): error path 의 `.context("kb-parse-pdf::PdfTextExtractor::extract")` → `"kb-app::extract_for (pdf)"` wording 변경. `error.v1.code` 보존 (downcast_ref 기반 exit code branching 영향 0). spec §5.5 risk acceptance 가 정합.
- **Exit gate**:
  - `grep -nE "PdfTextExtractor::new\(\)\.extract" crates/kebab-app/src/lib.rs` = **0 hit** (dispatch callsite 교체).
  - `grep -c "app.extract_for" crates/kebab-app/src/lib.rs` ≥ **2** (image + pdf).
  - `cargo build -p kebab-app -j 4 2>&1 | tail -3` = `Finished`. clippy 의 unused-import warn 발생 가능 — Step 9 에서 정리.
  - `cargo test -p kebab-app -j 4 --no-fail-fast 2>&1 | tail -10` — 기존 pdf ingest test 가 모두 pass.
- **Spec 참조**: §3.6 (Pattern β pdf), §3.7 (pdf row).

### Step 8: lib.rs:2012-2047 9 AST arm — `extract_for` 로 hoist (가장 큰 atomic edit)

- **Files affected**:
  - `crates/kebab-app/src/lib.rs` (단일 region — :2012-2047 부근).
- **Action**: spec §3.7 의 table — actual arm count = **12 (11 explicit + 1 wildcard)** (round 1 MINOR GAP #5 정정) → 본 step 후 **4 arm** (9 AST group + manifest group + shell + wildcard). 정확한 diff:
  - **BEFORE** (lib.rs:2012-2047, 12 arm):
    ```rust
    let canonical_result: anyhow::Result<kebab_core::CanonicalDocument> = match code_lang {
        "rust" => RustAstExtractor::new()
            .extract(&ctx, &bytes)
            .context("kb-parse-code::RustAstExtractor::extract (code:rust)"),
        "python" => PythonAstExtractor::new()
            .extract(&ctx, &bytes)
            .context("kb-parse-code::PythonAstExtractor::extract (code:python)"),
        "typescript" => TypescriptAstExtractor::new()
            .extract(&ctx, &bytes)
            .context("kb-parse-code::TypescriptAstExtractor::extract (code:typescript)"),
        "javascript" => JavascriptAstExtractor::new()
            .extract(&ctx, &bytes)
            .context("kb-parse-code::JavascriptAstExtractor::extract (code:javascript)"),
        "go" => GoAstExtractor::new()
            .extract(&ctx, &bytes)
            .context("kb-parse-code::GoAstExtractor::extract (code:go)"),
        "java" => JavaAstExtractor::new()
            .extract(&ctx, &bytes)
            .context("kb-parse-code::JavaAstExtractor::extract (code:java)"),
        "kotlin" => KotlinAstExtractor::new()
            .extract(&ctx, &bytes)
            .context("kb-parse-code::KotlinAstExtractor::extract (code:kotlin)"),
        "yaml" | "dockerfile" | "toml" | "json" | "xml" | "groovy" | "go-mod" => {
            synthesize_tier2_document(asset, &bytes, code_lang, &parser_version)
        }
        "shell" => synthesize_tier2_document(asset, &bytes, "shell", &parser_version),
        "c" => CAstExtractor::new()
            .extract(&ctx, &bytes)
            .context("kebab-parse-code::CAstExtractor::extract (code:c)"),
        "cpp" => CppAstExtractor::new()
            .extract(&ctx, &bytes)
            .context("kebab-parse-code::CppAstExtractor::extract (code:cpp)"),
        other => anyhow::bail!("unreachable (extract): {other}"),
    };
    ```
  - **AFTER** (4 arm):
    ```rust
    // p10-1b Task D/G/J/L + post-v0.18.0 extractor-dispatch-unification:
    // 9 AST lang 의 dispatch 가 polymorphic — App.extractors registry 의
    // `*AstExtractor` entry 가 lang string 으로 disjoint `supports()` 비교 후
    // 단일 hit. Tier 2 (manifest) + Tier 3 (shell) 은 free-function
    // `synthesize_tier2_document` 유지 (Extractor impl 아님, 별 PR future work).
    // p10-3: capture Result so Tier 1 extractor errors can fall back to Tier 3.
    let canonical_result: anyhow::Result<kebab_core::CanonicalDocument> = match code_lang {
        // 9 AST lang: rust / python / typescript / javascript / go / java / kotlin / c / cpp
        "rust" | "python" | "typescript" | "javascript"
        | "go" | "java" | "kotlin" | "c" | "cpp" => {
            app.extract_for(&asset.media_type, &ctx, &bytes)
                .with_context(|| format!("kb-app::extract_for (code:{code_lang})"))
        }
        // p10-2 Tier 2: no extractor — synthesize Document directly from raw bytes.
        "yaml" | "dockerfile" | "toml" | "json" | "xml" | "groovy" | "go-mod" => {
            synthesize_tier2_document(asset, &bytes, code_lang, &parser_version)
        }
        // p10-3: shell reuses the same synthesizer.
        "shell" => synthesize_tier2_document(asset, &bytes, "shell", &parser_version),
        other => anyhow::bail!("unreachable (extract): {other}"),
    };
    ```
  - net: **12 arm → 4 arm** (9 AST individual arm 통합 + manifest group + shell + wildcard). 9 callsite 의 `*Extractor::new().extract(…)` 가 1 callsite `app.extract_for(...)` 로 hoist.
  - **후속 control flow 보존 trace (Missing #1 fix)**:
    - lib.rs:2050 부근 `match canonical_result { Err(e) if code_lang == "shell" || matches!(...) => return Err(e).context(...) }` 가 Err 의 root cause 변별 — `code_lang` 의 lang string 으로 분기 (NOT anyhow chain 의 message 내용). `app.extract_for(...)` 의 Err 가 `*Extractor::extract(...)` 의 Err 와 동일 chain 구조 + `with_context(...)` 으로 outer wrap → Err variant matching 영향 0.
    - lib.rs:2050+ 의 Tier 1 → Tier 3 fallback 분기 `Err(e) => { tracing::warn!(...); chunker_version = CodeTextParagraphV1Chunker.chunker_version(); ...; synthesize_tier2_document(...) }` — anyhow chain 의 인용 (`error = %e`) 만 사용, 변별 의미 없음. **fallback control flow 보존 검증** = Step 11 의 `cargo test --workspace` 의 `p10_*` tier1-fallback test pass (예: `tests/p10_3_*.rs` 의 tier1 fail → tier3 recover test).
  - use 선언 (lib.rs:52 `use kebab_parse_code::{...}` 9 type) — 9 type 모두 lib.rs 안에 short-name 참조 0 (registry init 은 app.rs 의 short-name). lib.rs 의 use 가 unused → clippy warn. **Step 9 에서 정리**.
- **Exit gate**:
  - `grep -cE "(Rust|Python|Typescript|Javascript|Go|Java|Kotlin|C|Cpp)AstExtractor::new\(\)\.extract" crates/kebab-app/src/lib.rs` = **0 hit** (9 AST dispatch callsite 모두 제거).
  - `grep -c "app.extract_for" crates/kebab-app/src/lib.rs` ≥ **3** (image + pdf + code 의 3 dispatch site).
  - `grep -c "synthesize_tier2_document" crates/kebab-app/src/lib.rs` ≥ **3** (manifest arm + shell arm + 다른 callsite — Tier 2/3 유지).
  - arm count post-state verify (round 1 MINOR GAP #5 정정 — actual 12 → 4):
    ```bash
    $ awk '/let canonical_result.*= match code_lang/,/^    \};$/' crates/kebab-app/src/lib.rs \
        | grep -cE "^\s+\"[^\"]+\"[^=>]*=>|^\s+other\s*=>"
    # 예상: 4 (9-AST-group + manifest-group + shell + wildcard).
    ```
  - `cargo build -p kebab-app -j 4 2>&1 | tail -3` = `Finished`. clippy 의 unused-import warn 발생 가능 — Step 9 정리.
  - `cargo test -p kebab-app -j 4 --no-fail-fast 2>&1 | tail -20` — 기존 code ingest test (`tests/p10_*.rs` 등) 가 모두 pass — **fallback control flow 보존** 검증.
- **Spec 참조**: §3.6 (Pattern β code), §3.7 (9 AST arm row — 12 → 4 arm diff).

### Step 9: dead code 정리 (unused use statement + 임시 `#[allow(dead_code)]` cleanup checklist)

- **Files affected**:
  - `crates/kebab-app/src/lib.rs` (use statement — :51-53 부근).
  - `crates/kebab-app/src/app.rs` (Step 2 의 임시 `#[allow(dead_code)]` 가 있으면 제거).
- **Action**:
  - **(a) clippy 실행하여 unused-import warn 식별**:
    ```bash
    $ cargo clippy -p kebab-app --all-targets -j 4 -- -D warnings 2>&1 | grep -E "unused_imports|unused-imports|warning"
    ```
    예상 warn 후보:
    - `use kebab_parse_image::ImageExtractor` (Step 4 후 short-name 참조 0).
    - `use kebab_parse_pdf::PdfTextExtractor` (Step 7 후 short-name 참조 0).
    - `use kebab_parse_code::{CAstExtractor, ..., TypescriptAstExtractor}` 9 type (Step 8 후 short-name 참조 0).
  - **(b) lib.rs:51-53 의 use statement 갱신** — short-name 참조 없는 type 만 제거 (round 1 MAJOR #6 fix — destructure 의 비 AST type 보존):
    ```rust
    // BEFORE (lib.rs:51-53):
    use kebab_parse_image::{ImageExtractor, OllamaVisionOcr, apply_caption, apply_ocr};
    use kebab_parse_code::{CAstExtractor, CppAstExtractor, GoAstExtractor, JavaAstExtractor, JavascriptAstExtractor, KotlinAstExtractor, PythonAstExtractor, RustAstExtractor, TypescriptAstExtractor};
    use kebab_parse_pdf::PdfTextExtractor;

    // AFTER:
    use kebab_parse_image::{OllamaVisionOcr, apply_caption, apply_ocr};
    // kebab-parse-code 의 9 AST type 은 app.rs 의 registry init 에서만 사용 → lib.rs 의 use 제거.
    // kebab-parse-pdf::PdfTextExtractor 는 app.rs 의 registry init 에서만 사용 → lib.rs 의 use 제거.
    // 단 lib.rs 안에 kebab_parse_code 의 다른 type 호출 (e.g. `kebab_parse_code::detect_repo`) 이 있으면 보존:
    //   - lib.rs:2334 의 `kebab_parse_code::detect_repo(...)` 는 fully-qualified — use 갱신 영향 0.
    ```
    추가 grep 검증:
    ```bash
    $ grep -cE "^use kebab_parse_(image|pdf|code)::" crates/kebab-app/src/lib.rs
    # 예상: 1 (image 의 OllamaVisionOcr + apply_* 만 보존).
    ```
  - **(c) cleanup checklist (round 1 MINOR #1 fix)** — Step 2 (d) 또는 Step 3 (a) 의 임시 attribute 제거:
    | 위치 | 부착됐는가 | 제거 여부 |
    |---|---|---|
    | `app.rs` 의 `#[allow(dead_code)]` (extract_for 또는 extractors field) | Step 2 placeholder 시 부착 가능 | Step 3 의 real init 후 모두 제거 |
    | `lib.rs` 의 임시 `#[allow(unused_imports)]` | Step 4/7/8 시 부착 가능 | Step 9 (b) 의 use 갱신 후 모두 제거 |

    `grep -n "#\[allow(dead_code)\]\|#\[allow(unused_imports)\]" crates/kebab-app/src/{app,lib}.rs` → 본 step 후 0 hit.
- **Exit gate**:
  - `cargo clippy -p kebab-app --all-targets -j 4 -- -D warnings 2>&1 | tail -5` clean (warn 0).
  - `grep -c "use kebab_parse_code::" crates/kebab-app/src/lib.rs` = **0** (or ≤ 1 if `detect_repo` 같은 other type 의 별 use line 이 있는 경우 — pre-flight grep 결과로 결정).
  - `grep -c "ImageExtractor\|PdfTextExtractor" crates/kebab-app/src/lib.rs` = **0** (short-name 참조 0).
  - `grep -cE "(Rust|Python|Typescript|Javascript|Go|Java|Kotlin|C|Cpp)AstExtractor" crates/kebab-app/src/lib.rs` = **0**.
  - `grep -cE "#\[allow\(dead_code\)\]|#\[allow\(unused_imports\)\]" crates/kebab-app/src/{app,lib}.rs` = **0** (임시 attribute 모두 제거).
  - `cargo build -p kebab-app -j 4 2>&1 | tail -3` = `Finished`.
- **Spec 참조**: §3.5 (registry init source-of-truth), §3.7 (use statement 갱신).

### Step 10: unit tests 추가 (in-crate `#[cfg(test)] mod tests` in app.rs)

- **Files affected**:
  - `crates/kebab-app/src/app.rs` (단일 — 기존 `impl App { ... }` 의 아래에 `#[cfg(test)] mod tests { ... }` 추가).
- **Action**: spec §5.1 의 3 test class 를 in-crate unit test 로 작성 (round 1 CRITICAL #1 fix — `pub(crate)` access 위해 integration test 가 아닌 in-crate test). `crates/kebab-app/src/app.rs` 의 마지막 (impl App 끝나는 지점 아래) 에 추가:
  ```rust
  #[cfg(test)]
  mod tests {
      use super::*;
      use kebab_core::{ExtractContext, MediaType, AudioType};

      /// helper: tempdir-isolated App for tests.
      fn open_test_app() -> App {
          let tmp = tempfile::tempdir().expect("tempdir");
          let mut cfg = kebab_config::Config::default();
          cfg.workspace.root = tmp.path().join("workspace");
          cfg.workspace.data_dir = tmp.path().join("data");
          std::fs::create_dir_all(&cfg.workspace.root).expect("mkdir workspace");
          std::fs::create_dir_all(&cfg.workspace.data_dir).expect("mkdir data");
          let app = App::open_with_config(cfg).expect("App::open_with_config");
          std::mem::forget(tmp); // tempdir 의 drop 후 KB 가 사라지면 안 됨 (App 이 sqlite 점유)
          app
      }

      #[test]
      fn registry_has_eleven_extractors() {
          let app = open_test_app();
          assert_eq!(app.extractors.len(), 11,
              "registry must hold 11 Extractors (image + pdf + 9 AST). \
               markdown 은 별 PR.");
      }

      /// 11 Extractor 의 `supports()` 가 16 sample MediaType 에 대해
      /// mutually exclusive (어떤 두 Extractor 도 동일 MediaType 에 대해 true 반환 0).
      #[test]
      fn supports_grid_is_mutually_exclusive() {
          let app = open_test_app();
          let samples = vec![
              MediaType::Markdown,
              MediaType::Pdf,
              MediaType::Image(kebab_core::ImageType::Png),
              MediaType::Image(kebab_core::ImageType::Jpeg),
              MediaType::Code("rust".into()),
              MediaType::Code("python".into()),
              MediaType::Code("typescript".into()),
              MediaType::Code("javascript".into()),
              MediaType::Code("go".into()),
              MediaType::Code("java".into()),
              MediaType::Code("kotlin".into()),
              MediaType::Code("c".into()),
              MediaType::Code("cpp".into()),
              MediaType::Code("yaml".into()),     // registry NOT cover
              MediaType::Code("shell".into()),    // registry NOT cover
              MediaType::Audio(AudioType::Wav),    // registry NOT cover
          ];
          for sample in &samples {
              let hits: Vec<_> = app.extractors.iter()
                  .filter(|e| e.supports(sample))
                  .collect();
              assert!(hits.len() <= 1,
                  "mutually exclusive violated for {sample:?}: {} hits", hits.len());
          }
      }

      /// `extract_for` 가 registry NOT cover MediaType 에 대해
      /// `Err("no Extractor for media_type ...")` 반환.
      /// MAJOR #2 simpler suggestion: Audio MediaType 사용으로 RawAsset 의존성 회피 —
      /// extract_for 는 dispatch loop 만 검증, RawAsset 의 actual content 는 무관.
      #[test]
      fn extract_for_unsupported_media_errors() {
          let app = open_test_app();

          // Minimal RawAsset — actual content 는 dispatch 까지 도달 안 함
          // (Audio MediaType → registry NOT cover → 즉시 Err).
          // RawAsset 의 actual field (asset.rs:63-73): asset_id / source_uri /
          // workspace_path / media_type / byte_len / checksum / discovered_at / stored.
          let asset = kebab_core::RawAsset {
              asset_id: kebab_core::AssetId("dummy-blake3-12".to_string()),
              source_uri: kebab_core::SourceUri::File("/tmp/dummy.wav".into()),
              workspace_path: kebab_core::WorkspacePath("dummy.wav".to_string()),
              media_type: MediaType::Audio(AudioType::Wav),
              byte_len: 0,
              checksum: kebab_core::Checksum("00".repeat(32)),
              discovered_at: time::OffsetDateTime::now_utc(),
              stored: kebab_core::AssetStorage::Inline,
          };

          // MAJOR #1 fix: workspace_root 를 owned PathBuf 로 binding 한 후 borrow.
          let workspace_root: std::path::PathBuf = std::path::PathBuf::from("/tmp");
          let cfg = kebab_core::ExtractConfig::default();
          let ctx = ExtractContext {
              asset: &asset,
              workspace_root: &workspace_root,
              config: &cfg,
          };
          let result = app.extract_for(&MediaType::Audio(AudioType::Wav), &ctx, &[]);
          assert!(result.is_err(), "Audio 는 registry 미포함 → Err 기대");
          let err_msg = format!("{:#}", result.unwrap_err());
          assert!(err_msg.contains("no Extractor"), "unexpected err: {err_msg}");
      }
  }
  ```
  주의: RawAsset 의 `Checksum` / `AssetStorage` field 의 actual type 이 위 sample 과 다를 수 있음 — executor 가 `crates/kebab-core/src/asset.rs:63-73` (Step 1 의 pre-flight grep 결과) + `checksum.rs` / `stored.rs` 의 actual struct 확인 후 정합화. plan 의 sample 은 의도 명시 — 정확한 field 값은 executor 가 정합.

  `tempfile` dev-dep 확인:
  ```bash
  $ grep -A20 "\[dev-dependencies\]" crates/kebab-app/Cargo.toml | grep -E "^tempfile\s*="
  ```
  없으면 추가:
  ```toml
  [dev-dependencies]
  tempfile = { workspace = true }
  ```
- **Exit gate**:
  - `cargo test -p kebab-app --lib -j 4 2>&1 | tail -10` 의 결과 — `mod tests` 의 3 test pass.
  - 3 test 함수 등장: `grep -cE "^\s+fn (registry_has_eleven_extractors|supports_grid_is_mutually_exclusive|extract_for_unsupported_media_errors)" crates/kebab-app/src/app.rs` = **3**.
- **Spec 참조**: §5.1 (3 test class), §4.2 (mutually-exclusive verified).

### Step 11: workspace 회귀 + 7 cargo gate + wire diff 0 verify + clean commit

- **Files affected**: production code 변경 0 (verification + commit).
- **Action**:
  - **(a) `cargo clean`** — full workspace test 직전 1회.
  - **(b) 7 cargo gate**:
    ```bash
    $ cargo build --workspace -j 1                                              # gate 1
    $ cargo clippy --workspace --all-targets -j 1 -- -D warnings                # gate 2
    $ cargo test --workspace --no-fail-fast -j 1 \
        2>&1 | tee .omc/state/extractor-dispatch-after.log                      # gate 3

    # gate 4 — numeric net-delta compare (round 1 MINOR GAP #9 fix):
    $ BASELINE=$(cat .omc/state/extractor-dispatch-baseline.txt)
    $ AFTER=$(awk '/^test result: ok\./ {for(i=1;i<=NF;i++) if($i=="passed;") sum += $(i-1)} END {print sum}' \
        .omc/state/extractor-dispatch-after.log)
    $ DELTA=$((AFTER - BASELINE))
    $ test "$DELTA" -eq 3 || { echo "test count delta $DELTA != +3"; exit 1; }
    $ echo "test delta = +$DELTA ✓"

    $ cargo tree -p kebab-app -e normal | grep "kebab-parse-" | wc -l           # gate 5 — 4
    $ cargo build --release                                                     # gate 6
    $ cargo metadata --no-deps --format-version 1 | jq '.workspace_members | length'   # gate 7 — 22
    ```
  - **(c) wire diff 0 verify** (success path — spec §5.5 의 risk acceptance 정합 — error path scope 외):
    ```bash
    # Step 1 의 baseline 과 동일 cmd sequence 로 after snapshot 생성:
    $ rm -rf /tmp/kb-wire-after && mkdir -p /tmp/kb-wire-after/ws /tmp/kb-wire-after/data
    $ cp /tmp/kb-wire-baseline/config.toml /tmp/kb-wire-after/config.toml
    $ sed -i 's|/tmp/kb-wire-baseline|/tmp/kb-wire-after|g' /tmp/kb-wire-after/config.toml
    $ cp crates/kebab-app/src/lib.rs /tmp/kb-wire-after/ws/lib.rs
    $ cp README.md /tmp/kb-wire-after/ws/
    $ mkdir -p .omc/state/wire-after
    $ cargo run --release --bin kebab -- --config /tmp/kb-wire-after/config.toml ingest --json \
        > .omc/state/wire-after/ingest_report.json
    $ cargo run --release --bin kebab -- --config /tmp/kb-wire-after/config.toml search "polymorphic dispatch" --json \
        > .omc/state/wire-after/search.json
    $ cargo run --release --bin kebab -- --config /tmp/kb-wire-after/config.toml ask "what is extract_for" --json \
        > .omc/state/wire-after/answer.json
    $ diff -u .omc/state/wire-baseline/search.json .omc/state/wire-after/search.json | head
    $ diff -u .omc/state/wire-baseline/answer.json .omc/state/wire-after/answer.json | head
    $ diff -u .omc/state/wire-baseline/ingest_report.json .omc/state/wire-after/ingest_report.json | head
    # 모두 빈 출력 (diff 0) 기대.
    ```
    error path 의 wire diff 는 본 plan 의 scope 외 (spec §5.5 risk acceptance + §7 row).
  - **(d) 3 callsite-count post-state verify** (spec §3.7 net effect):
    ```bash
    $ grep -c "app.extract_for" crates/kebab-app/src/lib.rs
    # 기대: ≥ 3.
    $ grep -cE "(Rust|Python|Typescript|Javascript|Go|Java|Kotlin|C|Cpp)AstExtractor::new\(\)\.extract" crates/kebab-app/src/lib.rs
    # 기대: 0.
    $ grep -c "image_extractor" crates/kebab-app/src/lib.rs
    # 기대: 0.
    ```
  - **(e) code dispatch arm count**:
    ```bash
    $ awk '/let canonical_result.*= match code_lang/,/^    \};$/' crates/kebab-app/src/lib.rs \
        | grep -cE "^\s+\"[^\"]+\"[^=>]*=>|^\s+other\s*=>"
    # 기대: 4 (9-AST-group + manifest-group + shell + wildcard).
    ```
  - **(f) clean commit**:
    ```
    modified:   crates/kebab-app/src/app.rs       # struct + extract_for + registry init + mod tests
    modified:   crates/kebab-app/src/lib.rs       # 5 변경 site + use 갱신
    modified:   crates/kebab-app/Cargo.toml       # (optional) tempfile dev-dep
    ```
    commit message:
    ```
    refactor(app): AST 9-arm extract dispatch → App.extract_for polymorphic

    9 AST + image + pdf 의 11 `*Extractor::new().extract(…)` callsite 가
    App.extractors registry + extract_for helper 의 3 dispatch site 로 통합
    (9 AST 는 1 callsite 로 hoist). lib.rs:2012-2047 의 12 arm (11 explicit +
    1 wildcard) → 4 arm (9-AST-group + manifest-group + shell + wildcard).
    wire schema success path 변경 0 + design contract 변경 0 + frozen task spec
    변경 0. workspace.version bump 0. error path 의 anyhow context wording
    diff 는 user-visible surface 외 (spec §5.5 risk acceptance).

    MarkdownExtractor 신설 + Tier 2/3 Extractor 화 + Chunker registry +
    inner 4-match 통합 + outer 4-arm helper 통합 + dual-source
    parser_version 정리 + ExtractorRegistry plugin system 의 7 follow-up
    은 별 PR (spec §11).

    Spec: docs/superpowers/specs/2026-05-26-extractor-dispatch-unification-spec.md
    Plan: docs/superpowers/plans/2026-05-26-extractor-dispatch-unification-plan.md

    Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>
    ```
- **Exit gate**:
  - 7 cargo gate 모두 clean.
  - gate 4 의 numeric delta = +3 (정확 매칭, 더도 덜도 아님).
  - 3 wire diff (success path: search / answer / ingest_report) = 0 line.
  - 3 callsite-count + 1 arm count = expected (위 (d) (e)).
  - git status clean (single commit 후).
- **Spec 참조**: §5.2 (회귀), §5.3 (cargo gate), §5.4 (SMOKE), §5.5 (wire diff 0 success + error path scope 외), §5.6 (integration delta 0), §3.7 (net effect).

## §3 Step dependency graph

각 step 의 ordering invariant + atomic block 정의:

```text
Step 1 (baseline 측정 + wire snapshot)
    │ (변경 0 — single observation)
    ▼
Step 2 (App struct + extract_for shape + placeholder Vec::new())
    │ (additive — build green, dead-code warn 가능)
    ▼
Step 3 (registry init 11 entry + lib.rs:1235 alias 삭제)
    │ (init close — app.rs build green, lib.rs build red 시작 — atomic block 1 enter)
    ▼
Step 4 ─┐
        │ (lib.rs:356 local 제거 — intermediate build red)
Step 5 ─┤  ATOMIC BLOCK 1 (Step 3-4-5-6 의 single commit)
        │ (ImagePipeline struct field 제거 — intermediate build red)
Step 6 ─┘ (lib.rs:1296 callsite 교체 — atomic close, build green)
    │
    ▼
Step 7 (pdf callsite 교체 — atomic single-edit, build green)
    │ (independent of Step 4-6; image와 mutually independent)
    ▼
Step 8 (9 AST hoist — atomic single-region, build green)
    │ (independent of Step 4-7; code dispatch site 가 별 helper 함수)
    ▼
Step 9 (dead code 정리 — clippy clean, build green)
    │ (Step 6/7/8 모두 완료 후 unused use 식별 가능)
    ▼
Step 10 (unit tests 추가 — in-crate, Step 9 clippy clean 위에 작성)
    │
    ▼
Step 11 (회귀 + 7 cargo gate + wire diff + commit — closure)
```

### §3.1 Atomic block 1 (Step 3-4-5-6) 의 invariant (round 1 Ambiguity #1 fix)

본 4 step 은 **single commit 단위로 묶임**. 중간 state 에서 build red 허용:

- Step 3 후: app.rs build green (registry init), lib.rs build red (lib.rs:1235 alias 삭제 → lib.rs:1296 의 `image_extractor` 가 unresolved).
- Step 4 후: lib.rs:356 local 도 삭제 → 동일 red.
- Step 5 후: ImagePipeline struct field 도 제거 → init block 의 missing-field error (Step 4 에서 이미 init line 제거되었지만 struct field 존재 시 동일).
- Step 6 후: lib.rs:1296 callsite 교체 → atomic close, build green 보장.

**executor 가 Step 3-4-5-6 을 한 working session 안에서 진행 + Step 6 후에야 첫 `cargo build` 실행**. Step 3-5 단독 build 시도 금지.

### §3.2 mutually independent step (Step 6 / 7 / 8)

본 3 dispatch migration 은 lib.rs 의 3 다른 helper 함수 의 head — 어느 순서로 진행하든 동등. plan 의 ordering (image → pdf → code) = risk gradient reverse.

### §3.3 Step 9 의 의존성

Step 9 의 dead-code 식별이 Step 6 + 7 + 8 모두 완료된 후에야 가능. Step 9 는 Step 6-8 모두에 의존.

## §4 Verification gate (acceptance)

verifier 가 다음 모두 verify 후에만 plan 을 `status: completed`:

### §4.1 Cargo gate (7) + numeric delta gate

| gate | 명령 | 기대 |
|---|---|---|
| 1 | `cargo build --workspace -j 1` | `Finished` |
| 2 | `cargo clippy --workspace --all-targets -j 1 -- -D warnings` | warn 0 |
| 3 | `cargo test --workspace --no-fail-fast -j 1` | 모든 test pass |
| 4 | numeric delta = `AFTER - BASELINE` | **= 3** (정확 매칭) |
| 5 | `cargo tree -p kebab-app -e normal \| grep "kebab-parse-" \| wc -l` | **4** |
| 6 | `cargo build --release` | `Finished` |
| 7 | `cargo metadata --no-deps --format-version 1 \| jq '.workspace_members \| length'` | **22** |

### §4.2 Wire diff 0 (success path only)

| diff | source vs target | 기대 |
|---|---|---|
| 1 | `.omc/state/wire-baseline/search.json` vs after | 0 line |
| 2 | `.omc/state/wire-baseline/answer.json` vs after | 0 line |
| 3 | `.omc/state/wire-baseline/ingest_report.json` vs after | 0 line |

`schema_version` field = `*.v1` 유지. error path 의 wire diff 는 spec §5.5 의 risk acceptance — scope 외.

### §4.3 Callsite-count post-state (3)

| metric | grep | 기대 |
|---|---|---|
| polymorphic dispatch site | `grep -c "app.extract_for" lib.rs` | ≥ **3** |
| 9 AST direct callsite | `grep -cE "(Rust\|...\|Cpp)AstExtractor::new\(\)\.extract" lib.rs` | **0** |
| local image_extractor 잔존 | `grep -c "image_extractor" lib.rs` | **0** |

### §4.4 Code dispatch arm count

```bash
$ awk '/let canonical_result.*= match code_lang/,/^    \};$/' lib.rs \
    | grep -cE "^\s+\"[^\"]+\"[^=>]*=>|^\s+other\s*=>"
```
기대: **4** (9-AST-group + manifest-group + shell + wildcard).

## §5 Commit strategy

### §5.1 Single clean commit

본 plan 전체 작업이 single commit 으로 closure. 이유 = (a) Step 3-4-5-6 의 atomic block 이 single commit 단위 강제, (b) wire schema 변경 0 + design contract 변경 0 → 하나의 logical change, (c) sub-item 1/2 (PR #185 / #186) 패턴 정합.

executor 가 Step 1-11 모두 완료 + verifier 가 §4 의 14 gate 통과 후 단일 commit. Step 별 partial commit 금지 (atomic block 1 의 build-red intermediate state 가 git history 에 들어가면 bisect 불가).

### §5.2 commit message

Step 11 의 (f) sub-action 의 template. CLAUDE.md commit style + Co-Authored-By 트레일러.

### §5.3 push + PR (out-of-plan)

`git push origin refactor/extractor-dispatch-unification` + gitea PR 생성은 team-lead 의 work — 본 plan 의 step 아님.

## §6 Risks + mitigation

### §6.1 Step 3-4-5-6 atomic block 의 intermediate build red

- **risk**: executor 가 Step 3 단독으로 `cargo build` 시도 시 build red.
- **mitigation**: §3.1 의 atomic block 명시 — Step 3 → 4 → 5 → 6 한 working session 안에서 진행 + Step 6 후에야 첫 build verify.

### §6.2 Step 8 의 lang string source-of-truth mismatch

- **risk**: `app.extract_for(&asset.media_type, ...)` 의 `asset.media_type` vs `code_lang: &str` 의 source mismatch.
- **mitigation**: `code_lang` 는 `ingest_one_code_asset` 의 8번째 arg (lib.rs:1903 signature). caller (`ingest_one_asset` lib.rs:961-1040) 가 `lang.as_str()` 으로 전달 → 동일 source. unit test (Step 10 grid-search) 가 disjoint 검증.

### §6.3 Step 8 의 Tier 1 → Tier 3 fallback control flow 단절 (Missing #1 trace)

- **risk**: lib.rs:2050 부근 `match canonical_result { Err(e) if ... => ... }` fallback 이 `app.extract_for(...)` 의 Err 와 동일 형태로 전파 안 됨.
- **mitigation**: `app.extract_for` 의 body 가 `.extract(ctx, bytes)` 결과 그대로 반환 — Err 의 anyhow chain 형태 동일. `with_context(|| format!("kb-app::extract_for (code:{code_lang})"))` 의 outer context 추가가 root cause variant matching 영향 0 (`downcast_ref` 패턴). fallback 분기 `Err(e) if code_lang == "shell" || matches!(...)` 가 lang string 기반 — Err message 미사용. **검증** = Step 11 `cargo test --workspace` 의 `tests/p10_3_*.rs` (tier1 fail → tier3 recover) pass.

### §6.4 Step 11 의 wire diff > 0 (success path)

- **risk**: trait dispatch 의 vtable lookup 차이가 silent regression.
- **mitigation**: `Box<dyn Extractor>` 의 `extract` 호출이 본질적으로 `*Extractor::extract` 와 동일 (Rust trait object dispatch semantic preservation). diff > 0 발생 시 §4.2 의 first mismatch line → `ExtractContext<'_>` lifetime 또는 `&MediaType` enum variant 비교 차이 식별.

### §6.5 Step 10 의 `App::open_with_config` SQLite migration cost

- **risk**: 3 unit test 가 각각 fresh tempdir + SQLite open + migration → test 무거움 (~수 초).
- **mitigation**: light-weight constructor 신설 = spec §11 future work. test 시간 < 30s 면 acceptable.

### §6.6 Step 9 의 use statement 갱신 시 reference 보존 (round 1 MAJOR #6)

- **risk**: lib.rs:51 `use kebab_parse_image::{ImageExtractor, OllamaVisionOcr, apply_caption, apply_ocr};` 의 4 type 중 `ImageExtractor` 만 제거 — `OllamaVisionOcr / apply_caption / apply_ocr` 은 lib.rs 안에서 계속 사용.
- **mitigation**: Step 9 (b) sub-action 의 명시 — `use kebab_parse_image::{OllamaVisionOcr, apply_caption, apply_ocr};` 갱신. 동일 패턴 for `kebab_parse_code` (`detect_repo` 같은 다른 type 의 fully-qualified call 은 use 갱신 영향 0).

### §6.7 error path wire scope (spec §5.5 risk acceptance)

- **risk**: `.context("...")` wording 변경이 `error.v1.message` 또는 `IngestReport.v1.items[].error` String 에 surface.
- **mitigation**: spec §5.5 의 risk acceptance — internal Rust error chain wording 변경, `error.v1.code` 보존, message chain detail 은 user-visible surface 외. claude-code-skill / mcp consumer 의 wire contract 가 `error.v1.code` finite enumeration 의존 — message chain wording 의존 0. error path 의 wire diff 는 본 plan 의 success-path scope 외 (§4.2 의 3 diff 만 verify).

## §7 Out of scope (plan-level)

본 plan 이 다루지 않는 work — spec §2.2 의 non-goal inherit + plan-level deferred:

1. **markdown ingest path 의 변경** — MarkdownExtractor defer.
2. **Chunker dispatch unification** — design §7.2 갱신 동반.
3. **Tier 2/3 free-function path 의 Extractor 화**.
4. **inner 4 위치 match 통합**.
5. **outer 4-arm helper 통합**.
6. **dual-source `parser_version` 정리**.
7. **ExtractorRegistry plugin system**.
8. **light-weight `App` constructor** (test 전용).
9. **HOTFIXES.md / HANDOFF.md / ARCHITECTURE.md 갱신** — sibling pattern 따라 본 PR 머지 후 optional.
10. **push + PR creation** — team-lead 의 work.
11. **error path wire diff verify** — spec §5.5 risk acceptance.

## §8 Open questions

**없음**. round 1-2 의 모든 OQ 가 resolved (round 1 의 3 OQ + round 2 의 16 finding 모두 §9 closure table 에).

## §9 Round 1 finding closure status table

| round 1 finding | severity | source | closure 위치 |
|---|---|---|---|
| CRITICAL #1 (integration test → in-crate unit test) | CRITICAL | critic | Step 10 의 `crates/kebab-app/src/app.rs` 의 `#[cfg(test)] mod tests` 로 이동. `pub(crate)` access 보존. |
| MAJOR #1 (workspace_root 타입/lifetime) | MAJOR | critic | Step 10 의 `let workspace_root: PathBuf = PathBuf::from("/tmp"); ... workspace_root: &workspace_root` — owned binding 후 borrow. |
| MAJOR #2 (test_fixtures helper 부재) | MAJOR | critic | Step 10 의 `extract_for_unsupported_media_errors` 가 inline `kebab_core::RawAsset { ... }` 생성. Audio MediaType 사용으로 fixture 의존성 회피 (MAJOR #2 의 simpler suggestion 채택). |
| MAJOR #3 (Step 4 retroactive 수정) | MAJOR | critic | Step 2 (a) 의 use statement 추가 + Step 2 (d) / Step 3 (a) 의 vec![] 부터 short-name 으로 작성. Step 4 의 option α/β 토론 삭제. |
| MAJOR #4 (wire baseline cmd) | MAJOR | critic | Step 1 의 wire baseline snapshot section — `cargo run --release --bin kebab -- --config /tmp/kb-wire-baseline/config.toml ingest --json > ...` 의 falsifiable cmd 명시. Step 11 의 after snapshot 도 동일 cmd sequence. |
| MAJOR #5 (Step 3 lib.rs:1235 alias 삭제) | MAJOR | critic | Step 3 (b) — `lib.rs:1235 의 alias line 을 본 step 에서 명시적 삭제`. ImagePipeline struct field 제거 (Step 5) 이전. |
| MAJOR verifier #2 (error.v1 wire scope) | MAJOR | verifier | spec §5.5 갱신 — internal error context wording risk acceptance + plan §6.7 의 risk 명시. error path wire diff 는 success-path verify scope 외. |
| MINOR #1 (`#[allow(dead_code)]` cleanup) | MINOR | critic | Step 9 (c) — 임시 attribute 의 cleanup checklist table. |
| MINOR #2 (Step 1 awk doc-test 포함) | MINOR | critic | §0 의 "doc-test 포함 여부" 보강 — awk 의 `test result: ok.` 매칭이 doc-test 도 cover. baseline + after delta 보존. |
| MINOR GAP #4 (Step 3 lib.rs:1235 alias edit) | MINOR | verifier | MAJOR #5 와 중복 — Step 3 (b). |
| MINOR GAP #5 (arm count 13 → **12**) | MINOR | verifier | Step 8 + §4.4 + §1 approach summary — "12 (11 explicit + 1 wildcard) → 4 arm" 일관 명시. |
| MINOR GAP #6 (instance-method pattern) | MINOR | verifier | Step 1 의 baseline grep 추가 — `image_extractor\.extract\|image_pipeline\.extractor\.extract`. |
| MINOR GAP #7 (use-prefix policy) | MINOR | verifier | MAJOR #3 와 중복 — Step 2 (a) + Step 9 (b). |
| MINOR GAP #8 (pub(crate) test access) | MINOR | verifier | CRITICAL #1 와 중복 — Step 10 의 in-crate test. |
| MINOR GAP #9 (numeric net-delta gate) | MINOR | verifier | Step 11 (b) gate 4 — `BASELINE=$(cat ...); AFTER=$(awk ...); DELTA=$((...)); test "$DELTA" -eq 3 || exit 1`. |
| NIT #1 (visibility wording 일관) | NIT | critic | CRITICAL #1 와 동시 — Step 10 의 in-crate test 가 spec §3.5 + §3.6 의 `pub(crate)` 와 정합. |
| Missing #1 (Tier1→Tier3 fallback trace) | Missing | critic | Step 8 의 "후속 control flow 보존 trace" + §6.3 risk + Step 11 의 `tests/p10_3_*.rs` pass 검증. |
| Missing #2 (open_with_config init order) | Missing | critic | Step 3 (a) — extractors init 위치 = `pipeline_verifier` 직전 (sqlite → search_cache → pipeline_verifier → extractors → Ok(Self)). |
| Missing #3 (pipeline_verifier Err 시 extractors lifetime) | Missing | critic | Step 3 (a) 의 rationale — state-less, side-effect 0. drop cost 0. |
| Missing #4 (Step 6 happy-path fixture path) | Missing | critic | Step 1 의 wire baseline section — `cp crates/kebab-app/src/lib.rs ...` + `cp README.md ...` 의 2-medium fixture 명시. PDF/PNG fixture 부재 시 §4.3 callsite-count 로 covered. |
| Ambiguity #1 (ImagePipeline 제거 interpretation A vs B) | Ambiguity | critic | Step 3 (b) — "lib.rs:1235 alias 를 Step 3 에서 먼저 삭제, ImagePipeline struct + init block 은 Step 5 에서 재작성" 명시. Step 4 가 lib.rs:357 의 init block `extractor: &image_extractor,` 도 동시 삭제. |

### §9.1 Round closure status

| round | reviewer | mode | status | notes |
|---|---|---|---|---|
| 0 (drafting) | planner (self) | full | drafted | 11 step decompose. |
| 1 | critic-plan (opus) | full | REQUEST_CHANGES | 1 CRITICAL + 5 MAJOR + 2 MINOR + 1 NIT + 4 Missing + 1 Ambiguity. |
| 1 | verifier-plan (opus) | full | ACCEPT_WITH_RESERVATIONS | 3 MAJOR + 6 MINOR (overlap 일부). |
| 2 (reflection) | planner (self) | full rewrite | reflected | 16 finding closure (위 status table). spec §5.5 + §7 갱신 동반 (MAJOR verifier #2). plan v2 → v3. |
| 2 | critic-plan + verifier-plan (opus) | full | REQUEST_CHANGES (수렴 실패 보고) | 보고 = CRITICAL #1 NOT CLOSED + MAJOR #5 NOT CLOSED + MAJOR #6 PARTIAL. 단 round 3 의 grep cross-check 결과 = **CRITICAL #1 / MAJOR #5 모두 v3 에서 closure 완료** (Step 10 line 498-501 이 in-crate `mod tests`, RawAsset field 가 line 580-582 의 `checksum + stored` 정합, Step 3 (b) line 233-236 이 lib.rs:1235 alias 명시적 삭제). **round 2 critic/verifier report 가 v2 baseline 으로 misread** 한 false negative. MAJOR #6 만 실제 잔존 — spec §3.7 line 110/398/407 의 "13 arm" 잔존. |
| 3 (reflection) | planner (self) | spec micro-patch | reflected | spec §3.7 의 "13 arm" 3 location → "12 arm" + "13 → 5" → "12 → 4" 정정 (round 3 의 유일한 actual 정정). plan §9 의 closure table 에 round 2 의 false-negative cross-check 결과 추가. |
| 4 | critic-plan (sonnet) | **closure verify only** | pending | round 3 의 spec micro-patch + round 2 의 v3 plan content 가 round 1 finding 모두 closure 검증. grep cross-check 가 효율적. |
| 4 | verifier-plan (sonnet) | closure verify only | pending | 동일. |
| 5+ | as needed | — | pending | — |

### §9.2 Round 2 의 false-negative finding 의 grep cross-check

round 2 의 critic + verifier 양쪽이 100% 일치 finding 보고 — "CRITICAL #1 NOT CLOSED + MAJOR #5 NOT CLOSED" — 단 v3 plan 의 actual content 와 mismatch. round 3 의 grep evidence:

| round 2 finding | v3 plan actual | verdict |
|---|---|---|
| CRITICAL #1 — "test 가 여전히 tests/extract_for_dispatch.rs (integration)" | line 498 = `### Step 10: unit tests 추가 (in-crate #[cfg(test)] mod tests in app.rs)` + line 501 = `crates/kebab-app/src/app.rs (단일 — 기존 impl App { ... } 의 아래에 #[cfg(test)] mod tests { ... } 추가)` | **false-negative** — v3 가 이미 in-crate. v2 baseline 으로 misread. |
| CRITICAL #1 — "RawAsset 의 content_hash 잔존" | line 580 = `checksum: kebab_core::Checksum("00".repeat(32))` + line 582 = `stored: kebab_core::AssetStorage::Inline` | **false-negative** — v3 가 이미 actual asset.rs:63-73 의 field name 정합 (checksum + stored). v2 의 content_hash 오기는 round 2 reflection 에서 정정 완료. |
| MAJOR #5 — "lib.rs:1235 alias 삭제 의무 부재" | line 233-236 = `(b) lib.rs:1235 의 alias line 삭제 ... `let image_extractor = image_pipeline.extractor;` ← 삭제` + Ambiguity #1 closure (line 246) 이 atomic block ordering 명시 | **false-negative** — v3 의 Step 3 (b) 가 이미 명시. team-lead 가 가정한 "Step 5 (b)" position 은 본 plan 의 sequencing 과 다름 (atomic block 1 의 Step 3 = alias 삭제, Step 5 = struct field 제거 — interpretation A). |
| MAJOR #6 — "spec §3.7 + plan §3.7 narrative 의 13/5 잔존" | spec line 110, 398, 407 = "13 arm cover 17 lang" / "13 → 5 arm" 잔존 (round 3 정정 대상) + plan §3.7 narrative = grep 결과 0 hit (v3 이미 12/4 일관 — Step 8 + §4.4 + summary) | **partial** — spec 만 정정 필요. round 3 의 spec 3 site edit 으로 closure. |

cross-check 결론: round 2 의 critic + verifier 의 100% 일치 finding 의 2 항목 (CRITICAL #1, MAJOR #5) 이 v2 baseline mis-read. plan v3 의 actual content 가 round 1 finding 의 closure 를 이미 정합. round 3 의 단일 정정 = spec §3.7 의 "13 arm" 3 site → "12 arm (11 explicit + 1 wildcard)" + "12 → 4 arm" 정정.

→ Phase C (executor opus) 진입 준비됨.
