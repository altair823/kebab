---
title: v0.20.0 sub-item 1 — PDF scanned OCR via Ollama vision LLM — implementation plan
created: 2026-05-27
status: draft (round 1c rewrite — critic round 1 + verifier round 1 통합)
target_version: 0.20.0
spec: docs/superpowers/specs/2026-05-27-pdf-scanned-ocr-spec.md
contract_sections: ["§9"]
related_specs:
  - docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
  - docs/superpowers/handoffs/2026-05-26-v0.20-image-pdf-normalize-handoff.md
  - docs/superpowers/poc/2026-05-27-pdf-ocr-engine-comparison.md
  - docs/superpowers/specs/2026-05-26-extractor-dispatch-unification-spec.md
sibling_plans:
  - docs/superpowers/plans/2026-05-26-normalize-absorption-plan.md   # PR #186 머지
  - docs/superpowers/plans/2026-05-26-extractor-dispatch-unification-plan.md  # PR #187 머지
review_history:
  - 2026-05-27 spec round 1 critic (opus, thorough) — NEEDS_DISCUSSION, HIGH 5 + MEDIUM 14
  - 2026-05-27 spec round 1c rewrite (opus, drafter) — HIGH 5 + MEDIUM 14 applied
  - 2026-05-27 spec round 2 critic (sonnet, closure) — ACCEPT, LOW 1 (cosmetic)
  - 2026-05-27 plan round 0 (opus, drafter) — 890 lines, 11 step group A-K + 31 sub-action
  - 2026-05-27 plan round 1 critic (opus, thorough) — NEEDS_DISCUSSION, MEDIUM 4 + LOW 5 + NIT 1
  - 2026-05-27 plan round 1 verifier (opus, thorough) — NEEDS_DISCUSSION, HIGH 5 + MEDIUM 10 + LOW 3 + NIT 2
  - 2026-05-27 plan round 1c rewrite (opus, drafter) — HIGH 5 + MEDIUM 14 + LOW/NIT applied (본 round)
---

# v0.20.0 sub-item 1 — PDF scanned OCR plan

> ACCEPT 된 spec (`docs/superpowers/specs/2026-05-27-pdf-scanned-ocr-spec.md`, 1720 lines) 의 step decomposition. spec § Acceptance §9 의 15 row 가 step 단위로 분산. **11 step (Group A-K), 34 sub-action** (round 1c 후 +3: A3 baseline capture / B2 fixture relocation / E4 cancel wiring / I5 alnum ocr_e2e — Step 9 I2 의 fixture 합성은 Step 2 B2 로 이전됨). TDD pattern: 각 group 의 module 추가 step 은 RED (failing test) → GREEN (impl) → Refactor. spec 의 L-1 cosmetic fix (round 2 critic) 가 Step 1 의 first sub-action.

## §0 Pre-flight + branch state

- **Branch**: `feat/pdf-scanned-ocr` (현재 위치 — 사용자 설정 branch).
- **Base SHA**: `bcd1e37` (main HEAD — PR #188 docs handoff backfill 머지 직후, v0.19.0 cut 시점).
- **Working dir**: `/home/altair823/kebab`.
- **Env 강제** (`~/.claude/CLAUDE.md` 의 "Disk Layout — 루트 디스크 보호가 최우선" 룰):
  - `export CARGO_TARGET_DIR=/build/out/cargo-target/target` — 본 plan 의 모든 cargo 명령 적용. repo root 의 `target/` 생성 방지 (16 GiB RAM 머신의 `/` 250 G 보호).
  - **`export RELEASE_BIN="${CARGO_TARGET_DIR:-target}/release/kebab"`** — release binary 경로 alias (verifier H-5 resolution). K2 + dogfood smoke 의 모든 acceptance command 가 `$RELEASE_BIN` 사용 → `CARGO_TARGET_DIR` override 충돌 0.
  - `export TMPDIR=/build/cache/tmp` — 대용량 임시 파일 발생 시 보호.
- **Cargo build 직렬화** (MEMORY.md `feedback_serial_build_only.md` — 사용자 결정 2026-05-26):
  - **per-crate cargo**: `-j 4` default (예: `cargo build -p kebab-parse-pdf -j 4`).
  - **full workspace** (`cargo test --workspace`, `cargo clippy --workspace`): `-j 1` 강제. 18 integration-test binary 동시 link 시 OOM (linker SIGKILL).
  - **K3 / K4 sequential** (verifier M-7 resolution): cargo test / clippy / build 동시 background 실행 금지. R-9 mitigation 의 "background + 다른 작업 mutually independent" 문구 삭제 — K3 완료 후 K4 → K5 순차 진행.
- **`target/` clean policy** (retro 2026-05-27 — disk-threshold conditional, see memory `feedback-cargo-clean-policy`): `CARGO_TARGET_DIR=/build/out/cargo-target/target` 가 XFS 4TB 전용 디스크에 분리되어 있어 root disk 압박 0. `cargo clean` 은 임계 도달 시에만 — `df -h /build` 의 `Avail < 500G` OR `du -sh /build/out/cargo-target` > 500G OR `du -sh $CARGO_TARGET_DIR` > 200G. 본 plan 의 모든 cargo clean 명시 step (§0 + §1 + Step 11 K2) 가 conditional. 임계 미달 시 skip + result file/commit body 안 "skipped cargo clean — /build avail X TB" 1줄 record. CLAUDE.md "fb-* batch 후 90+ GB → cargo clean routinely" 룰은 root disk 가정으로, /build 분리 환경에는 부적합.
- **HOTFIXES.md / HANDOFF.md / README.md / docs/ARCHITECTURE.md / docs/SMOKE.md 변경**: 본 plan 의 Step 10 에서 갱신 (spec §6.4 의 Docs split 룰 따름).
- **frozen task spec 변경 0** — `tasks/p7/p7-1-pdf-text-extractor.md` 의 historical scope (OCR explicit non-scope) 가 본 sub-item 으로 해소되지만 task spec 자체는 frozen 유지 (CLAUDE.md "Task specs themselves stay frozen as the historical contract").
- **wire schema additive minor only** — `ingest_progress.v1` 의 `kind` enum extension + `ingest_report.v1.items[].pdf_ocr_*` optional field. JSON Schema 갱신 동반.
- **workspace `Cargo.toml` version bump `0.19.0` → `0.20.0`** (Step 11.1, CLAUDE.md §Release "사용자 도그푸딩에 영향이 가는 surface 변경" 트리거).
- **design contract 변경 0** (`contract_sections: ["§9"]` 이지만 §9 versioning rules table 자체 갱신 0 — parser_version `"pdf-text-v1"` 보존, H-4 결정).

## §1 Plan overview + spec linkage

Spec §3-§5 의 결정 + §3.1 H-1 (post-extract enrichment) + §3.2 H-3 (DCTDecode-only v1) + §4.7 H-4 (parser_version 유지) + §4.4 H-5 (eager init) 을 atomic step 으로 decompose. destination = `kebab-app::pdf_ocr_apply` (Option d, spec §3.1) + `kebab-parse-pdf::page_image` + `kebab-parse-pdf::text_quality`. 핵심 sequencing:

1. **Foundation + dep + baseline** (Step 1) — spec L-1 cosmetic fix + `kebab-parse-pdf` 의 변경 surface 명문화 (Cargo.toml dep 변경 0 — H-3 갈래 A 의 image crate 미도입 invariant) + **cargo tree baseline 캡처** (verifier H-3 resolution).
2. **lopdf probe + 모든 fixture 합성** (Step 2) — PoC fixture F1/F2 의 PDF wrap 의 image XObject `/Filter` 가 DCTDecode 인지 측정. **F1/F2 + F4 mojibake + F6 FlateDecode + F7 CCITTFax 5 fixture 모두 본 step 에서 합성·commit** (verifier H-4 resolution — Step 4 D1 의 fixture-dependent test 가 Step 9 commit 에 의존하던 sequencing gap 제거). FlateDecode 이면 fixture 재합성도 본 step 에서.
3. **page_image.rs + text_quality.rs** (Step 3) — `kebab-parse-pdf` 안에 두 module 추가. spec §4.1 의 body 그대로. RED (failing test) → GREEN (impl). **page_image 가 happy + negative 두 test** (verifier M-2 resolution).
4. **pdf_ocr_apply helper** (Step 4) — `kebab-app::pdf_ocr_apply` 신규. MockOcrEngine 기반 isomorphic test. spec §3.1 의 H-1 resolution 의 핵심 deliverable. **F7 CCITTFax skip test split** (verifier M-4 resolution).
5. **Config schema** (Step 5) — `kebab-config::PdfCfg + PdfOcrCfg`. image OCR pattern mirror. defaults + env override.
6. **Ingest wiring + cancel propagation** (Step 6) — `kebab-app::ingest_with_config_opts` 의 eager init (H-5) + `ingest_one_pdf_asset` signature 확장 + post-extract enrichment 호출 (spec §4.4 의 diff). **E4 cancel handle propagation 새 sub-action** (critic M-2 resolution).
7. **Wire schema additive** (Step 7) — `ingest_progress.v1` kind enum extension + `ingest_report.v1.items[].pdf_ocr_*` field. JSON Schema 동기. **enum 추가 대상 = `crates/kebab-app/src/ingest_progress.rs` 의 `IngestEvent`** (verifier M-1 resolution — `kebab-core::ingest.rs` 가 아님).
8. **In-tree consumer** (Step 8) — `kebab-cli` stdout printer + ndjson snapshot regenerate.
9. **Integration smoke + regression + alnum e2e** (Step 9) — integration smoke (`app.search` step 포함) + vector PDF regression + **alnum accuracy `#[ignore]` test 신규** (verifier M-5 / M-6 resolution). 모든 fixture 합성은 Step 2 로 이전됨.
10. **Docs sync** (Step 10) — README + HANDOFF + ARCHITECTURE + SMOKE + v0.20.0 release notes. H-4 force-reingest UX wording. **release notes path 본 step 의 first sub-action 에서 pre-flight 결정** (verifier M-10 resolution).
11. **Version bump + final verify** (Step 11) — `0.19.0` → `0.20.0` + (conditional) `cargo clean` (disk threshold 미달 시 skip, §0 retro) + 5 cargo gate + § Acceptance §9 #1-#15 row 검증 + step-별 commit + PR open.

ordering invariant:

- **Step 1 A3 (baseline 캡처) < 그 외 모든 step**: cargo tree baseline 이 K5 row #9 + #10 의 diff verifier 의 ground-truth.
- **Step 2 < Step 3**: probe 결과로 F1/F2 fixture 재합성 필요성 결정. **모든 fixture commit 이 Step 2 안** → Step 3-4 의 fixture-dependent test 는 commit 시점 부터 GREEN-able (verifier H-4 resolution).
- **Step 3 < Step 4**: `page_image::extract_dctdecode_page_image` + `text_quality::compute_valid_char_ratio` 가 `pdf_ocr_apply::apply_ocr_to_pdf_pages` 의 prerequisite.
- **Step 4 < Step 5/6**: helper 가 사용 가능한 후에 config + wiring. Step 5/6 는 mutually independent — 동시 가능하지만 정합성 위해 sequential.
- **Step 5 < Step 6**: config field 가 init code 의 input.
- **Step 6 < Step 7**: ingest path 가 wire event emit 의 source — wire schema 갱신은 emit code 의 동작 검증 가능 시점.
- **Step 7 < Step 8**: in-tree consumer update 는 wire schema 갱신의 dependent.
- **Step 8 < Step 9**: integration smoke + regression test 가 production code 완성 후 가능 (smoke test 가 production code 호출).
- **Step 9 < Step 10**: docs sync 는 production code + test 완성 후 (release notes 가 실측 result 인용).
- **Step 10 < Step 11**: version bump + 최종 verify 는 모든 surface 완성 후.

각 step 의 commit 단위는 **logical group 1 commit** (atomic) — §7 sequencing summary 의 11-commit table 따름. 사용자 memory `feedback_pr_workflow` (gitea-pr + 리뷰 루프) 따라 final PR 은 단일 logical change (= "feat(pdf): scanned PDF OCR via qwen2.5vl:3b vision LLM (v0.20.0)") + Co-Authored-By line.

---

## §2 Step group structure (Group A-K)

| Step | Group | 분류 | sub-action |
|---:|---|---|---|
| 1 | A | Foundation + dep + baseline | A1 spec L-1 cosmetic fix + A2 module skeleton + **A3 cargo tree baseline 캡처 (NEW, H-3)** |
| 2 | B | lopdf probe + 모든 fixture 합성 | B1 F1/F2 PDF /Filter 측정 + **B2 F1/F2/F4/F6/F7 fixture commit (NEW, H-4 relocation)** |
| 3 | C | page_image + text_quality | C1 page_image.rs (RED→GREEN, **2 test**), C2 text_quality.rs (RED→GREEN) |
| 4 | D | pdf_ocr_apply helper | D1 helper body + MockOcrEngine (**9 test, F7 split**), D2 dual-block ordinal test, D3 bridge integration test |
| 5 | F | Config schema | F1 PdfCfg + PdfOcrCfg, F2 serde + env override test |
| 6 | E | Ingest wiring | E1 eager init, E2 signature update, E3 enrichment 호출, **E4 cancel handle propagation (NEW, critic M-2)** |
| 7 | G | Wire schema additive | G1 IngestEvent kind enum (**file = `kebab-app/src/ingest_progress.rs`**), G2 IngestItem field, G3 JSON Schema doc |
| 8 | H | In-tree consumer | H1 CLI printer, H2 ndjson snapshot regenerate |
| 9 | I | Smoke + regression + alnum e2e | I3 integration smoke (**w/ `app.search` step**), I4 vector PDF regression, **I5 ocr_e2e.rs alnum #[ignore] (NEW, M-6)** |
| 10 | J | Docs sync | **J0 release notes path 결정 (NEW pre-flight, M-10)**, J1 README, J2 HANDOFF, J3 ARCHITECTURE/SMOKE, J4 release notes |
| 11 | K | Version bump + final verify | K1 version bump, K2-K4 cargo gate, K5 § Acceptance row-by-row, K6 Step 11 commit + PR open |

---

## §3 Per-step detail

### Step 1 (Group A): Foundation — spec L-1 fix + module skeleton + cargo tree baseline

#### Sub-action A1 — spec L-1 cosmetic fix (round 2 critic deliverable)

- **Files affected**: `docs/superpowers/specs/2026-05-27-pdf-scanned-ocr-spec.md`.
- **Action**: spec §4.2 line 740 의 prose pseudo-code 갱신:
  ```diff
  -     - pdf_ocr_engine_opt = app.pdf_ocr_engine.as_ref() (eager init at ingest entry, §4.4 — H-5 resolution).
  +     - pdf_ocr_engine_opt = local `pdf_ocr_engine: Option<OllamaVisionOcr>` built in `ingest_with_config_opts` (§4.4 eager init, fall-fast on build failure).
  ```
  spec §4.4 line 791 의 "App field `pdf_ocr_engine` 도입 0" 결정과 정합.
- **Acceptance**:
  - `grep -c "app.pdf_ocr_engine.as_ref" docs/superpowers/specs/2026-05-27-pdf-scanned-ocr-spec.md` = **0**.
  - `grep -c "local \`pdf_ocr_engine: Option<OllamaVisionOcr>\` built in" docs/superpowers/specs/2026-05-27-pdf-scanned-ocr-spec.md` ≥ **1**.

#### Sub-action A2 — Cargo.toml dep invariant + module skeleton 사전 verify

- **Files affected**: `crates/kebab-parse-pdf/Cargo.toml` (변경 0 — invariant verify), `crates/kebab-app/Cargo.toml` (변경 0 — 두 parser crate 이미 dep).
- **Action** (사전 확인, edit 없음):
  - `crates/kebab-parse-pdf/Cargo.toml` 의 deps = `kebab-core + anyhow + serde_json + time + tracing + lopdf` 그대로. **`image` crate 도입 0** 보장 (H-3 DCTDecode-only v1 invariant).
  - `crates/kebab-app/Cargo.toml` 가 `kebab-parse-image` + `kebab-parse-pdf` 둘 다 이미 dep (확인): `grep -c "kebab-parse-image\|kebab-parse-pdf" crates/kebab-app/Cargo.toml` ≥ 2.
  - `crates/kebab-parse-pdf/Cargo.toml` 의 description 갱신 1줄 — Step 3 의 module 추가 후:
    ```diff
    -description   = "Text PDF extractor (per-page text + page citation) for the kebab pipeline (P7-1)"
    +description   = "Text PDF extractor + scanned-page image extract helpers for the kebab pipeline (P7-1 + v0.20.0 sub-item 1)"
    ```
- **Acceptance**:
  - `grep -c "image\s*=" crates/kebab-parse-pdf/Cargo.toml` = **0** (image crate 미도입 invariant).
  - `grep -c "lopdf" crates/kebab-parse-pdf/Cargo.toml` ≥ **1** (lopdf 그대로).

#### Sub-action A3 — cargo tree baseline 캡처 (NEW — verifier H-3 resolution)

- **Files affected**: `.omc/state/pdf-ocr-app-parse-deps.baseline.txt` (신규), `.omc/state/pdf-ocr-parse-pdf-deps.baseline.txt` (신규).
- **Action**: K5 row #9 + #10 의 ground-truth baseline 사전 캡처. plan 본 step 의 first cargo 호출 (다른 step 의 cargo run 이 dep graph 변경 0 invariant 의 baseline).
  ```bash
  mkdir -p .omc/state
  cargo tree -p kebab-app -e normal | grep "kebab-parse" \
    > .omc/state/pdf-ocr-app-parse-deps.baseline.txt
  cargo tree -p kebab-parse-pdf -e normal \
    > .omc/state/pdf-ocr-parse-pdf-deps.baseline.txt
  ```
  - sub-item 3 의 baseline 패턴 (`.omc/state/extractor-dispatch-baseline.txt`) mirror.
- **Acceptance**:
  - `test -s .omc/state/pdf-ocr-app-parse-deps.baseline.txt` (non-empty).
  - `test -s .omc/state/pdf-ocr-parse-pdf-deps.baseline.txt` (non-empty).
  - `grep -c "kebab-parse-image\|kebab-parse-pdf\|kebab-parse-md\|kebab-parse-code" .omc/state/pdf-ocr-app-parse-deps.baseline.txt` ≥ **4**.
- **Commit message draft** (Step 1 전체):
  ```
  docs+chore(plan-bootstrap): apply spec L-1 cosmetic fix + capture cargo tree baselines for v0.20 sub-item 1 verifier gates
  ```

### Step 2 (Group B): lopdf prototype probe + 모든 fixture 합성 (H-4 relocation)

verifier H-4 resolution — Step 4/9 의 fixture-dependent test 가 사이 commit 에서 RED 상태가 되지 않도록 **5 fixture 모두 본 step 의 deliverable**. fixture commit + probe result + PoC doc append 가 logical commit 단위.

#### Sub-action B1 — lopdf `/Filter` probe (F1/F2 의 image XObject encoding 측정)

- **Files affected**: `docs/superpowers/poc/2026-05-27-pdf-ocr-engine-comparison.md` (probe result append), `tests/fixtures/_synth/lopdf_filter_probe.rs` (또는 .sh, 신규 — disposable).
- **Action**:
  - **(a) probe script** — PoC 의 F1/F2 PDF wrap 의 첫 image XObject `/Filter` + `decompressed_content` length + 첫 8 byte magic 측정. disposable test binary 또는 standalone bin (`cargo run --bin pdf_filter_probe`).
  - probe pseudo-code:
    ```rust
    let bytes = std::fs::read("docs/superpowers/poc/F1.pdf")?;
    let doc = lopdf::Document::load_mem(&bytes)?;
    let pages = doc.get_pages();
    for (page_num, &oid) in &pages {
        let page = doc.get_dictionary(oid)?;
        // Resources → XObject → first /Subtype /Image 의 /Filter + magic
        // (Step 3 의 extract_dctdecode_page_image 와 동일 traversal)
        // print: page_num, filter_name, content_len, first_8_bytes_hex
    }
    ```
  - **(b) 결과 분기 + 즉시 B2 fixture commit 으로 carry**:
    - **Case ✓** (F1/F2 의 `/Filter == DCTDecode` + JPEG magic `\xFF\xD8`): B2 에서 그대로 copy commit.
    - **Case ✗** (F1/F2 가 FlateDecode 등 raw pixel — Pillow 의 PNG → PDF wrap 의 default): B2 에서 `img2pdf` 또는 ImageMagick 의 JPEG-stream PDF wrap (`magick page1.png page1.pdf` with `-compress jpeg`). 결과를 `crates/kebab-parse-pdf/tests/fixtures/scanned_page1.pdf` 로 commit.
  - **(c) result record**: PoC doc 의 끝에 1-단락 append — "lopdf probe (2026-05-27): F1/F2 `/Filter == <측정 결과>`, content_len=N bytes, magic=<hex>. Step 3 의 extract_dctdecode_page_image body 의 baseline.".
- **Acceptance**:
  - probe 의 stdout 라인 ≥ 2 (F1 + F2).
  - PoC doc 의 line count delta ≥ 3 (append 단락).

#### Sub-action B2 — 5 fixture commit (F1/F2/F4/F6/F7, H-4 relocation)

- **Files affected**:
  - `crates/kebab-parse-pdf/tests/fixtures/scanned_page1.pdf` (F1 — DCTDecode JPEG).
  - `crates/kebab-parse-pdf/tests/fixtures/scanned_page2.pdf` (F2 — DCTDecode JPEG).
  - `crates/kebab-parse-pdf/tests/fixtures/mojibake.pdf` (F4 — Type 0 font + ToUnicode CMap disable, M-9 best-effort).
  - `crates/kebab-parse-pdf/tests/fixtures/flate_raw.pdf` (F6 — FlateDecode raw pixel).
  - `crates/kebab-parse-pdf/tests/fixtures/ccitt.pdf` (F7 — CCITTFaxDecode bilevel).
  - `tests/fixtures/_synth/mojibake.py` (신규 — F4 합성 script, reportlab).
- **Action**:
  - **F1/F2**: B1 의 결과 분기에 따라 copy (Case ✓) 또는 `img2pdf` / ImageMagick `-compress jpeg` 재합성 (Case ✗).
  - **F4 mojibake** (spec §5.1 line 1190-1206, M-9 best-effort fallback chain):
    ```bash
    # 시도 순서:
    # 1) reportlab Type 0 font + ToUnicode CMap disable.
    # 2) 실패 시 fpdf2 의 CID font + ToUnicode 직접 stripping.
    # 3) 최후 fallback: lopdf 수작업 Type 0 dict (M-9).
    python tests/fixtures/_synth/mojibake.py \
        crates/kebab-parse-pdf/tests/fixtures/mojibake.pdf
    ```
  - **F6 FlateDecode** (spec §5.1 line 1211):
    ```bash
    python -c "from PIL import Image; im = Image.new('RGB', (300,200), 'white'); im.save('crates/kebab-parse-pdf/tests/fixtures/flate_raw.pdf', 'PDF')"
    ```
  - **F7 CCITTFax** (spec §5.1 line 1213):
    ```bash
    magick -size 600x800 xc:white -fill black -draw "text 50,50 'test'" -compress Group4 /tmp/ccitt.tif
    magick /tmp/ccitt.tif crates/kebab-parse-pdf/tests/fixtures/ccitt.pdf
    rm /tmp/ccitt.tif
    ```
  - 합성 후 lopdf 로 열어 `/Filter` 가 각각 DCTDecode / FlateDecode / CCITTFaxDecode 인지 quick-verify (B1 probe 의 reuse).
- **Acceptance**:
  - 5 fixture file 모두 존재:
    ```bash
    ls -1 crates/kebab-parse-pdf/tests/fixtures/{scanned_page1,scanned_page2,mojibake,flate_raw,ccitt}.pdf | wc -l
    # = 5 (단 F4 absent fallback 시 = 4 + plan retro record — M-9 conditional)
    ```
  - F4 합성 실패 시 **conditional retro record**: plan 의 본 sub-action 끝에 "F4 fixture absent — plan executor 의 best-effort 후 row skip" 1줄 명문 (M-9 resolution). K3 의 expected delta 도 -1 (= +21 instead of +22) 자동 조정. **F4 absent 의 결정 boundary = B2 의 deliverable 자체** — Step 3 C2 / Step 4 D1 의 F4-dependent test 가 `#[ignore = "F4 fixture absent — plan retro 참조"]` 로 자동 gating.
- **Commit message draft** (Step 2 전체):
  ```
  poc+test(pdf-ocr): lopdf /Filter probe + 5 fixture commit (F1/F2/F4/F6/F7) for v0.20 sub-item 1
  ```

### Step 3 (Group C): page_image.rs + text_quality.rs 신규 (RED → GREEN)

#### Sub-action C1 — page_image.rs (DCTDecode passthrough, **2 test**, verifier M-2)

- **Files affected**:
  - `crates/kebab-parse-pdf/src/page_image.rs` (신규, spec §4.1 line 604-680 의 body 그대로).
  - `crates/kebab-parse-pdf/src/lib.rs` (`mod page_image; pub use page_image::extract_dctdecode_page_image;` 추가).
  - `crates/kebab-parse-pdf/tests/page_image.rs` (신규 — RED→GREEN integration test, **2 test**).
- **Action**:
  - **(a) RED step**: integration test 먼저 작성:
    ```rust
    // crates/kebab-parse-pdf/tests/page_image.rs
    use lopdf::Document;
    use kebab_parse_pdf::extract_dctdecode_page_image;

    // happy path
    #[test]
    fn f1_fixture_yields_dctdecode_jpeg_bytes() {
        let bytes = include_bytes!("fixtures/scanned_page1.pdf");
        let doc = Document::load_mem(bytes).unwrap();
        let result = extract_dctdecode_page_image(&doc, 1).unwrap();
        let jpeg = result.expect("F1 의 page 1 이 DCTDecode image 보유");
        assert!(jpeg.starts_with(b"\xFF\xD8"), "JPEG magic missing");
        assert!(jpeg.len() > 1000, "JPEG bytes too small");
    }

    // negative path (verifier M-2 — page_image test count +2 와 정합)
    #[test]
    fn flate_raw_fixture_yields_none() {
        let bytes = include_bytes!("fixtures/flate_raw.pdf");
        let doc = Document::load_mem(bytes).unwrap();
        let result = extract_dctdecode_page_image(&doc, 1).unwrap();
        assert!(result.is_none(), "FlateDecode page 가 Ok(None) 반환 — DCTDecode-only v1 invariant");
    }
    ```
    Step 2 B2 가 두 fixture commit — 본 sub-action 의 test 가 commit 시점부터 GREEN-able (verifier H-4 resolution).
  - **(b) GREEN step**: `crates/kebab-parse-pdf/src/page_image.rs` 작성. spec §4.1 의 body (line 604-680) 그대로:
    - `pub fn extract_dctdecode_page_image(pdf_doc: &Document, page_num: u32) -> Result<Option<Vec<u8>>>`.
    - `pdf_doc.get_pages()` → `page_oid` lookup → `get_dictionary(page_oid)?` → `Resources` → `XObject` traverse → 첫 `/Subtype /Image` + `/Filter == DCTDecode` (single Name 또는 `Array([Name])`) + JPEG magic `\xFF\xD8` 검증 → `Ok(Some(stream.content.clone()))`. 그 외 `Ok(None)`.
    - lopdf 0.32 API 사용.
- **Acceptance**:
  - `cargo test -p kebab-parse-pdf --test page_image -j 4` green (2 test, RED→GREEN 완료).
  - `grep -c "extract_dctdecode_page_image" crates/kebab-parse-pdf/src/lib.rs` ≥ **1** (pub use).
  - `grep -c "DCTDecode" crates/kebab-parse-pdf/src/page_image.rs` ≥ **2** (filter name match + 주석).
  - `grep -c "image\s*=" crates/kebab-parse-pdf/Cargo.toml` = **0** (image crate 미도입 invariant 보존).

#### Sub-action C2 — text_quality.rs (valid char ratio)

- **Files affected**:
  - `crates/kebab-parse-pdf/src/text_quality.rs` (신규, spec §4.1 line 686-723 의 body 그대로).
  - `crates/kebab-parse-pdf/src/lib.rs` (`mod text_quality; pub use text_quality::compute_valid_char_ratio;` 추가).
- **Action**:
  - **(a) RED step**: spec §5.6 의 7 unit test 가 `mod tests` 안에 — empty / pure ASCII / pure Hangul / mojibake PUA / mixed 50/50 / CJK ideograph / Hangul Jamo. **F4 fixture-dependent test** (`f4_fixture_ratio_under_threshold`) 는 Step 2 B2 의 F4 fixture 가 합성 성공 시 같이 활성, 합성 실패 (B2 retro record) 시 `#[ignore]`.
    - LOW L-4 resolution: F4 test 의 `#[ignore = "F4 fixture absent — Step 2 B2 retro record 참조"]` 명문 annotation pattern. 합성 성공 시 plan executor 가 annotation 제거.
  - **(b) GREEN step**: `crates/kebab-parse-pdf/src/text_quality.rs` 작성. spec §4.1 line 692-722 의 body:
    - `pub fn compute_valid_char_ratio(s: &str) -> f32` — empty → 0.0, 그 외 = valid count / total count.
    - `fn is_valid_text_char(c: char) -> bool` — codepoint range match (ASCII printable + Latin Extended + Hangul Jamo + Compatibility Jamo + CJK + Hangul Syllables + 한국 punctuation subset).
  - **(c) test name alignment** (LOW L-2 — verifier 가 plan/spec 의 test name pin 요청):
    - spec line 1207 = `mojibake_fixture_ratio_under_0_3`, plan = `f4_fixture_ratio_under_threshold` — **plan 의 `f4_fixture_ratio_under_threshold` 로 pin** (plan executor 의 cargo invoke 명령과 정합). spec L-2 round 의 cosmetic fix 후행 옵션.
- **Acceptance**:
  - `cargo test -p kebab-parse-pdf text_quality -j 4` green — 6 unit test (F4 fixture test 가 ignore 또는 활성, B2 결과 따라).
  - F4 합성 성공 시 추가로 `cargo test -p kebab-parse-pdf text_quality::f4_fixture_ratio_under_threshold -j 4` green.
  - `grep -c "compute_valid_char_ratio" crates/kebab-parse-pdf/src/lib.rs` ≥ **1** (pub use).
  - `grep -c "AC00\|D7A3\|Hangul Syllables" crates/kebab-parse-pdf/src/text_quality.rs` ≥ **1** (한글 syllables 범위 보장).
- **MEDIUM-1 cross-reference** (critic M-1): Step 3 의 두 RED test (page_image 2 + text_quality 6-7) 가 Step 6/7/8 의 wiring/wire/printer RED→GREEN coverage 의 prerequisite. Step 4 D1 의 9 integration test + Step 8 H2 의 새 test 가 wiring + wire + printer 의 effective RED→GREEN coverage.
- **Commit message draft** (Step 3 전체):
  ```
  feat(parse-pdf): add page_image (DCTDecode passthrough, 2 test) + text_quality (valid char ratio, 6-7 test) modules
  ```

### Step 4 (Group D): pdf_ocr_apply helper (kebab-app) — H-1 resolution 의 핵심

#### Sub-action D1 — helper body + MockOcrEngine fixture (**9 test, F7 split**)

- **Files affected**:
  - `crates/kebab-app/src/pdf_ocr_apply.rs` (신규, spec §4.1 line 381-599 의 body 그대로).
  - `crates/kebab-app/src/lib.rs` (`mod pdf_ocr_apply;` + `use crate::pdf_ocr_apply::*` for ingest path).
  - `crates/kebab-app/tests/pdf_ocr_apply.rs` (신규 — bridge integration test, spec §5.5).
- **Action**:
  - **(a) RED step** — **9 integration test** (verifier M-4 — F7 split):
    1. `f1_input_with_ocr_enabled_replaces_empty_block` — F1 + enabled=true → in-place mutate.
    2. `f3_input_with_ocr_enabled_keeps_text_detect_blocks` — F3 + enabled=true → mock 호출 0.
    3. `f1_input_with_ocr_disabled_keeps_empty_block` — F1 + enabled=false → no-op.
    4. `f4_input_with_ocr_enabled_replaces_mojibake_block` — F4 + enabled=true → in-place mutate via valid_ratio. **B2 F4 absent retro 시 `#[ignore]`** (M-9 conditional).
    5. `f3_input_with_always_on_pushes_dual_blocks` — F3 + always_on=true → block_count = page_count*2.
    6. `f6_flatedecode_skipped_with_warning` — F6 + enabled=true → extract_dctdecode_page_image=None → warning event.
    7. **`f7_ccittfax_skipped_with_warning`** — F7 + enabled=true → extract_dctdecode_page_image=None → warning event (verifier M-4 split).
    8. `ocr_engine_failure_surfaces_as_warning` — Mock 가 Err → warning event push.
    9. `dual_block_ordinals_are_deterministic_and_unique` — text-detect block ordinal [0,page_count), OCR block [page_count,page_count*2).
  - **(b) MockOcrEngine** — spec §5.5 line 1284-1299 의 struct + impl:
    ```rust
    struct MockOcrEngine { expected_text: String, fail: bool }
    impl OcrEngine for MockOcrEngine {
        fn engine_name(&self) -> &'static str { "mock-ocr" }
        fn engine_version(&self) -> String { "mock-v1".to_string() }
        fn recognize(&self, _img: &[u8], _hint: Option<&Lang>) -> Result<OcrText> {
            if self.fail { anyhow::bail!("mock failure"); }
            Ok(OcrText { joined: self.expected_text.clone(), regions: vec![], engine: ..., engine_version: ... })
        }
    }
    ```
    - OQ-E4 deferral — `OcrText.engine` field 의 actual type (String vs &'static) 은 plan executor 의 first sub-action 으로 grep 확인.
  - **(c) GREEN step** — `crates/kebab-app/src/pdf_ocr_apply.rs` body 작성. spec §4.1 line 381-599 의 body 그대로:
    - `pub struct PdfOcrOpts { enabled, always_on, valid_ratio_threshold, min_char_count, lang_hint, cancel }` — **cancel field 포함** (spec §4.1 + §4.8 의 합집합, verifier LOW L-1 — plan 이 correct).
    - `pub struct PdfOcrSummary { pages_ocrd: u32, ms_total: u64 }`.
    - `pub enum PdfOcrProgress { Started { page }, Finished { page, ms, chars, skipped } }`.
    - `pub fn apply_ocr_to_pdf_pages<F>(canonical: &mut CanonicalDocument, engine: &dyn OcrEngine, pdf_bytes: &[u8], opts: &PdfOcrOpts, emit_progress: F) -> Result<PdfOcrSummary>` — body 그대로 (per-page loop + needs_ocr decision matrix + in-place vs dual-block + provenance event push + per-page cancel check).
    - `fn find_paragraph_block_idx(blocks: &[Block], page_num: u32) -> usize` — invariant helper.
- **Acceptance**:
  - `cargo test -p kebab-app --test pdf_ocr_apply -j 4` green — **9 test pass** (F4 ignore 시 = 8 green + 1 ignored).
  - `grep -c "pub fn apply_ocr_to_pdf_pages" crates/kebab-app/src/pdf_ocr_apply.rs` = **1**.
  - `grep -c "use kebab_parse_image::OcrEngine" crates/kebab-app/src/pdf_ocr_apply.rs` ≥ **1** — parser cross-import 가 facade 안 (H-1).
  - `grep -c "use kebab_parse_pdf::" crates/kebab-app/src/pdf_ocr_apply.rs` ≥ **1** — page_image + text_quality import.
  - `diff <(cargo tree -p kebab-parse-pdf -e normal | grep -E "kebab-parse-image|^image v") .omc/state/pdf-ocr-parse-pdf-deps.baseline.txt` empty diff (A3 baseline reuse — parser isolation 보존).

#### Sub-action D2 — dual-block ordinal test (M-3 invariant)

이미 D1 의 test 9 (`dual_block_ordinals_are_deterministic_and_unique`) 에서 cover. 별 step 아님 — D1 의 sub-task.

- **Acceptance** (D1 의 부분):
  - test 9 assertion:
    ```rust
    let text_detect_ords: Vec<u32> = collect_ordinals(&canonical.blocks, "text-detect range");
    let ocr_ords: Vec<u32> = collect_ordinals(&canonical.blocks, "ocr range");
    assert!(text_detect_ords.iter().all(|&o| o < page_count));
    assert!(ocr_ords.iter().all(|&o| o >= page_count && o < page_count * 2));
    ```

#### Sub-action D3 — bridge integration smoke (cancel test)

- **Files affected**: `crates/kebab-app/tests/pdf_ocr_apply.rs` (D1 의 file 에 추가 test).
- **Action**:
  - cancel handle test:
    ```rust
    #[test]
    fn cancel_handle_aborts_mid_pdf() {
        let cancel = Arc::new(AtomicBool::new(false));
        let opts = PdfOcrOpts { cancel: Some(cancel.clone()), ... };
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            cancel.store(true, Ordering::Relaxed);
        });
        let result = apply_ocr_to_pdf_pages(&mut canonical, &mock_slow, &bytes, &opts, |_| {});
        let err = result.unwrap_err();
        assert!(format!("{err}").contains("cancelled mid-PDF"));
    }
    ```
  - spec §4.8 의 per-page cancel check 명문 정합. **production cancel wiring 은 Step 6 E4 의 deliverable** (critic M-2 resolution).
- **Acceptance**:
  - `cargo test -p kebab-app --test pdf_ocr_apply cancel_handle_aborts -j 4` green.
- **Commit message draft** (Step 4 전체):
  ```
  feat(app): add pdf_ocr_apply helper (9 test, F7 split) — post-extract OCR enrichment for PDF (H-1 resolution)
  ```

### Step 5 (Group F): Config schema — PdfCfg + PdfOcrCfg

#### Sub-action F1 — PdfCfg + PdfOcrCfg struct + defaults

- **Files affected**: `crates/kebab-config/src/lib.rs`.
- **Action**:
  - spec §4.5 line 920-1003 의 `PdfCfg` + `PdfOcrCfg` struct 추가. image OCR pattern mirror.
  - `Config` struct 에 `#[serde(default = "PdfCfg::defaults")] pub pdf: PdfCfg` field 추가.
  - 11 env var override (spec §4.5 line 1018-1029):
    - `KEBAB_PDF_OCR_ENABLED` / `_ALWAYS_ON` / `_ENGINE` / `_MODEL` / `_ENDPOINT` / `_LANGUAGES` / `_MAX_PIXELS` / `_REQUEST_TIMEOUT_SECS` / `_VALID_RATIO_THRESHOLD` / `_MIN_CHAR_COUNT` / `_LANG_HINT`.
  - OQ-E3 / OQ-E5 / OQ-E9 deferral — `request_timeout_secs=0` semantics + `KEBAB_PDF_OCR_LANGUAGES` array parsing + `models.llm.endpoint` actual field 모두 plan executor 의 first sub-action grep 으로 image OCR pattern 확인 후 정합.
- **Acceptance**:
  - `grep -c "pub struct PdfOcrCfg" crates/kebab-config/src/lib.rs` = **1**.
  - `grep -c "KEBAB_PDF_OCR_" crates/kebab-config/src/lib.rs` ≥ **11**.
  - `cargo build -p kebab-config -j 4` green.

#### Sub-action F2 — serde roundtrip + env override test

- **Files affected**: `crates/kebab-config/tests/pdf_ocr.rs` (신규).
- **Action**:
  - **(a)** roundtrip test — toml example block (spec §4.5 line 1034-1047) → `toml::from_str::<Config>` → `serde_json::to_string` → 모든 field 보존.
  - **(b)** default test — `Config::default().pdf.ocr.enabled` == false, `model` == "qwen2.5vl:3b", `valid_ratio_threshold` == 0.5, `min_char_count` == 20.
  - **(c)** env override test — `KEBAB_PDF_OCR_ENABLED=true` + `KEBAB_PDF_OCR_MODEL=qwen2.5vl:7b` → config 의 두 field 반영.
- **Acceptance**:
  - `cargo test -p kebab-config --test pdf_ocr -j 4` green.
- **Commit message draft** (Step 5 전체):
  ```
  feat(config): add [pdf.ocr] section — qwen2.5vl:3b default, opt-in, valid_ratio threshold + env overrides
  ```

### Step 6 (Group E): Ingest wiring — eager init + signature + enrichment + cancel propagation

#### Sub-action E1 — `ingest_with_config_opts` 의 eager init (H-5)

- **Files affected**: `crates/kebab-app/src/lib.rs` (line ~338-347 의 image OCR build 직후).
- **Action**: spec §4.4 line 796-826 의 diff 그대로. image OCR build pattern (lib.rs:338-347) 의 mirror:
  ```rust
  // p10 / v0.20 sub-item 1: PDF OCR engine eager init (H-5).
  // image OCR pattern mirror — per-ingest 1회 build, fallible → fail-fast.
  let pdf_ocr_engine: Option<OllamaVisionOcr> =
      if app.config.pdf.ocr.enabled || app.config.pdf.ocr.always_on {
          let cfg = &app.config.pdf.ocr;
          let endpoint = match cfg.endpoint.as_deref() {
              Some(s) if !s.is_empty() => s.to_string(),
              _ => app.config.models.llm.endpoint.clone(),
          };
          Some(OllamaVisionOcr::from_parts(
              endpoint, cfg.model.clone(), cfg.languages.clone(),
              cfg.max_pixels, cfg.request_timeout_secs,
          ).context("kb-app::ingest: build OllamaVisionOcr (pdf)")?)
      } else { None };
  ```
- **Acceptance**:
  - `grep -c "OllamaVisionOcr::from_parts" crates/kebab-app/src/lib.rs` ≥ **1**.
  - `grep -c "build OllamaVisionOcr (pdf)" crates/kebab-app/src/lib.rs` = **1**.
  - `cargo build -p kebab-app -j 4` green.

#### Sub-action E2 — `ingest_one_pdf_asset` signature 확장

- **Files affected**: `crates/kebab-app/src/lib.rs` (line ~1720, signature + caller).
- **Action**: spec §4.4 line 829-845 의 diff 그대로:
  ```diff
   fn ingest_one_pdf_asset(
       app: &App,
       asset: &RawAsset,
       chunk_policy: &ChunkPolicy,
       embedder: Option<&Arc<dyn Embedder + Send + Sync>>,
       vector_store: Option<&Arc<kebab_store_vector::LanceVectorStore>>,
       existing_doc_ids: &std::collections::HashSet<String>,
       force_reingest: bool,
  +    pdf_ocr_engine: Option<&OllamaVisionOcr>,
  +    progress: Option<&IngestEventSender>,
  +    cancel: Option<&Arc<AtomicBool>>,
   ) -> anyhow::Result<kebab_core::IngestItem> {
  ```
  caller (ingest dispatch loop) update — `&pdf_ocr_engine.as_ref()` + `progress` + `cancel` carry. **E4 (cancel propagation) 와 paired**.
- **Acceptance**:
  - `grep -A14 "fn ingest_one_pdf_asset" crates/kebab-app/src/lib.rs | grep -c "pdf_ocr_engine: Option<&OllamaVisionOcr>"` = **1**.
  - `grep -A14 "fn ingest_one_pdf_asset" crates/kebab-app/src/lib.rs | grep -c "cancel: Option<&Arc<AtomicBool>>"` = **1**.
  - `grep -c "ingest_one_pdf_asset(" crates/kebab-app/src/lib.rs` ≥ **2** (정의 + caller).
  - `cargo build -p kebab-app -j 4` green.

#### Sub-action E3 — post-extract enrichment 호출 + `IngestItem.pdf_ocr_*` 채움

- **Files affected**: `crates/kebab-app/src/lib.rs` (line ~1779 `extract_for` 직후 + return 부근).
- **Action**: spec §4.4 line 850-911 의 두 hunk 그대로:
  1. extract_for 직후 enrichment block (Hunk 1, 41 lines):
     ```rust
     let mut canonical = app.extract_for(&asset.media_type, &ctx, &bytes)?;
     let (pdf_ocr_pages, pdf_ocr_ms_total): (Option<u32>, Option<u64>) =
         if app.config.pdf.ocr.enabled || app.config.pdf.ocr.always_on {
             match pdf_ocr_engine {
                 Some(engine) => {
                     let opts = PdfOcrOpts {
                         enabled: app.config.pdf.ocr.enabled,
                         always_on: app.config.pdf.ocr.always_on,
                         valid_ratio_threshold: app.config.pdf.ocr.valid_ratio_threshold,
                         min_char_count: app.config.pdf.ocr.min_char_count,
                         lang_hint: app.config.pdf.ocr.lang_hint.clone().map(Lang),
                         cancel: cancel.cloned(),  // E4 — production cancel wiring (critic M-2)
                     };
                     let summary = crate::pdf_ocr_apply::apply_ocr_to_pdf_pages(
                         &mut canonical, engine, &bytes, &opts, |p| match p { ... }
                     )?;
                     (Some(summary.pages_ocrd), Some(summary.ms_total))
                 }
                 None => (Some(0), Some(0)),
             }
         } else { (None, None) };
     ```
  2. IngestItem 반환 시 두 field 채우기.
  - PR #187 invariant 보존 — `extract_for` 가 normal entry. registry 우회 0.
- **Acceptance** (verifier H-2 — function-scope grep + case-sensitive 정합):
  - `grep -c "apply_ocr_to_pdf_pages" crates/kebab-app/src/lib.rs` ≥ **1** (호출).
  - **`awk '/^fn ingest_one_pdf_asset/,/^}/' crates/kebab-app/src/lib.rs | grep -c "extract_for(&asset.media_type"` ≥ **1**** (verifier H-2 — function-scope grep, actual literal 매치).
  - `cargo build -p kebab-app -j 4` green.
  - `cargo test -p kebab-app -j 4 --no-fail-fast` — 기존 PDF ingest test 전수 pass (vector PDF regression).

#### Sub-action E4 — cancel handle propagation (NEW — critic M-2 resolution)

- **Files affected**: `crates/kebab-app/src/lib.rs` (ingest_with_config_cancellable + ingest_one_pdf_asset caller).
- **Action**: cancel handle 의 ingest entry → `PdfOcrOpts.cancel` chain 완전 wiring:
  1. `ingest_with_config_cancellable` (lib.rs:716 부근) 의 cancel handle 을 `ingest_one_pdf_asset` 의 새 `cancel: Option<&Arc<AtomicBool>>` parameter (E2) 로 carry.
  2. `ingest_one_pdf_asset` body 의 `PdfOcrOpts` 생성 시 `cancel: cancel.cloned()` (E3 의 hunk).
  3. dispatch loop 의 cancel handle source = `Option<&Arc<AtomicBool>>` (ingest entry signature 와 매치).
  4. spec §4.8 line 1159 의 "PdfOcrOpts 에 optional `cancel: Option<Arc<AtomicBool>>` 추가" 명문 정합.
- **Acceptance**:
  - `grep -c "PdfOcrOpts {" crates/kebab-app/src/lib.rs` ≥ **1**.
  - `awk '/^fn ingest_one_pdf_asset/,/^}/' crates/kebab-app/src/lib.rs | grep -c "cancel:" ` ≥ **2** (parameter + PdfOcrOpts field carry).
  - **production cancel smoke 추가 test** (LOW L-2 strict — Step 9 I3 와 paired):
    ```rust
    // crates/kebab-app/tests/ingest_pdf_ocr_smoke.rs 안
    #[test]
    fn ingest_with_cancel_aborts_mid_pdf() {
        let cancel = Arc::new(AtomicBool::new(false));
        let handle = std::thread::spawn({
            let cancel = cancel.clone();
            move || ingest_with_config_cancellable(/* cfg + cancel */)
        });
        std::thread::sleep(Duration::from_millis(100));
        cancel.store(true, Ordering::Relaxed);
        let result = handle.join().unwrap();
        // partial result OR Err — production cancel 작동 확인
        assert!(result.is_err() || /* IngestReport.aborted == true */);
    }
    ```
- **Commit message draft** (Step 6 전체):
  ```
  feat(app): wire PDF OCR enrichment + cancel propagation into ingest_one_pdf_asset (H-5 eager init + post-extract hook + per-page cancel)
  ```

### Step 7 (Group G): Wire schema additive — IngestEvent + IngestItem + JSON Schema

#### Sub-action G1 — `IngestEvent::kind` enum 확장 (verifier M-1 — file pinned)

- **Files affected**: **`crates/kebab-app/src/ingest_progress.rs`** (verifier M-1 resolution — `crates/kebab-core/src/ingest.rs` 가 아님. actual definition = line 58 의 `pub enum IngestEvent { ... }` 6 variant).
- **Action**: 2 variant 추가:
  - `PdfOcrStarted { page: u32 }`.
  - `PdfOcrFinished { page: u32, ms: u64, chars: u32, ocr_engine: String }`.
  - serde discriminant = `"pdf_ocr_started"` / `"pdf_ocr_finished"` (enum 의 `#[serde(tag = "kind", rename_all = "snake_case")]` attribute 의 자동 매핑). JSON Schema 의 enum value 와 일관 (Step 7.3).
  - **wire enum drift note** (verifier M-1): JSON Schema 의 enum value 10 entry (existing 8 + 신규 2) vs Rust `IngestEvent` 의 8 variant (existing 6 + 신규 2). `embed_batch_started` / `embed_batch_finished` 가 wire schema 에는 reserved 로 등재되어 있고 Rust enum 에는 미emit (line 14 의 "reserved for a future iteration"). spec §4.6.1 의 wording 과 정합.
- **Acceptance**:
  - `grep -c "PdfOcrStarted\|PdfOcrFinished" crates/kebab-app/src/ingest_progress.rs` ≥ **2**.
  - `cargo build -p kebab-app -j 4` green.

#### Sub-action G2 — `IngestItem` field 추가 (M-9 wire pattern)

- **Files affected**: `crates/kebab-core/src/ingest.rs` (line 75-87 의 IngestItem struct — actual position 은 OQ deferral, plan executor 의 first sub-action grep).
- **Action**: spec §4.6.2 line 1097-1119 의 diff 그대로 — `pdf_ocr_pages: Option<u32>` + `pdf_ocr_ms_total: Option<u64>`. **`skip_serializing_if` 없음**.
  ```diff
   pub struct IngestItem {
       ...
       pub warnings: Vec<String>,
  +    /// v0.20.0: scanned PDF OCR — page count for which vision OCR ran.
  +    pub pdf_ocr_pages: Option<u32>,
  +    pub pdf_ocr_ms_total: Option<u64>,
       pub error: Option<String>,
   }
  ```
  caller (`ingest_one_*_asset`) 가 모든 non-PDF asset 의 반환 시 `pdf_ocr_pages: None, pdf_ocr_ms_total: None` 채움.
- **Acceptance**:
  - `grep -c "pdf_ocr_pages" crates/kebab-core/src/ingest.rs` = **1** (struct field).
  - `grep -rc "pdf_ocr_pages: None" crates/kebab-app/src/` ≥ **3** (non-PDF caller 의 default).
  - `cargo test -p kebab-core -j 4` green.

#### Sub-action G3 — JSON Schema 동기 갱신

- **Files affected**:
  - `docs/wire-schema/v1/ingest_progress.schema.json` (kind enum + 4 optional field).
  - `docs/wire-schema/v1/ingest_report.schema.json` (items[].pdf_ocr_pages + pdf_ocr_ms_total nullable integer).
  - `docs/wire-schema/v1/ingest_progress.v1.md` + `ingest_report.v1.md` (markdown doc 갱신).
- **Action**: spec §4.6.1 line 1056-1078 + §4.6.2 line 1097-1122 의 diff 그대로.
- **Acceptance**:
  - `jq '.properties.kind.enum | length' docs/wire-schema/v1/ingest_progress.schema.json` ≥ **10** (기존 8 + 신규 2).
  - `jq '.properties.items.items.properties.pdf_ocr_pages.type' docs/wire-schema/v1/ingest_report.schema.json` ≠ null (field 추가 확인).
  - `grep -c "pdf_ocr_started\|pdf_ocr_finished" docs/wire-schema/v1/ingest_progress.v1.md` ≥ **2**.
- **MEDIUM-1 cross-reference** (critic M-1): Step 7 의 wiring step 의 RED→GREEN coverage = Step 8 H2 의 새 test (`pdf_ocr_progress_emits_started_finished_events`) + Step 9 I3 (search hit step) 가 serde roundtrip + wire 의 effective coverage 제공.
- **Commit message draft** (Step 7 전체):
  ```
  feat(wire): additive minor — IngestEvent kind 의 pdf_ocr_*  + ingest_report.items[].pdf_ocr_pages/ms_total (v1)
  ```

### Step 8 (Group H): In-tree consumer — CLI printer + snapshot regenerate

#### Sub-action H1 — `kebab-cli` ingest stdout printer

- **Files affected**: `crates/kebab-cli/src/main.rs` (또는 ingest event handler — OQ-E5 가 plan executor 의 first grep).
- **Action**: 2 새 kind 의 사람-친화 라인 mapping (spec §4.6.1 line 1085-1086):
  ```rust
  IngestEvent::PdfOcrStarted { page } => format!("  📷 OCR page {page}..."),
  IngestEvent::PdfOcrFinished { page, ms, chars, ocr_engine } =>
      format!("  ✓ OCR page {page} ({chars} chars, {ms}ms via {ocr_engine})"),
  ```
  CLI stdout 의 line-by-line ndjson 또는 사람-친화 mode 양쪽 대응.
- **Acceptance**:
  - `grep -c "PdfOcrStarted\|PdfOcrFinished" crates/kebab-cli/src/main.rs` ≥ **2**.
  - `cargo build -p kebab-cli -j 4` green.

#### Sub-action H2 — `kebab-app/tests/ingest_progress_*.rs` snapshot regenerate

- **Files affected**: `crates/kebab-app/tests/ingest_progress_*.rs` (existing) + 신규 PDF OCR test.
- **Action**:
  - 기존 snapshot 의 ndjson baseline 가 `pdf_ocr_started` / `pdf_ocr_finished` event 가 PDF asset 의 OCR-enabled run 시 등장하도록 새 test 추가 (`pdf_ocr_progress_emits_started_finished_events`, mock OcrEngine 사용 + F1 fixture).
  - 기존 PDF (OCR off) snapshot 의 변경 = `pdf_ocr_pages: null` + `pdf_ocr_ms_total: null` 두 field 추가만 (M-9 wire convention). 다른 field 변경 0.
  - OQ-E8 deferral — `cargo insta` 사용 여부는 plan executor 의 first sub-action 의 grep:
    ```bash
    grep -rn "insta\|assert_snapshot" crates/kebab-app/tests/
    ```
- **Acceptance**:
  - `cargo test -p kebab-app --test ingest_progress -j 4` green.
  - 새 test `pdf_ocr_progress_emits_started_finished_events` 존재.
  - 기존 PDF snapshot 의 diff 가 `pdf_ocr_pages: null` + `pdf_ocr_ms_total: null` 두 line 추가만:
    ```bash
    git diff main -- crates/kebab-app/tests/ingest_progress*.snap \
        | awk '/^-/ && !/^---/' | grep -cv "pdf_ocr_"
    # = 0 (existing line removal 0)
    ```
- **Commit message draft** (Step 8 전체):
  ```
  feat(cli): humanize pdf_ocr_started/finished events in ingest stdout printer + snapshot baseline
  ```

### Step 9 (Group I): Integration smoke + regression + alnum e2e

verifier H-4 resolution — fixture 합성은 Step 2 B2 로 이전. Step 9 는 integration smoke + regression test + alnum e2e 만.

#### Sub-action I3 — Integration smoke test (`tests/ingest_pdf_ocr_smoke.rs`) — **w/ search hit step** (M-5)

- **Files affected**: `crates/kebab-app/tests/ingest_pdf_ocr_smoke.rs` (신규).
- **Action**:
  - **step 1**: `KEBAB_PDF_OCR_ENABLED=true` + MockOcrEngine + F1 fixture ingest → `IngestItem.pdf_ocr_pages >= 1` + `pdf_ocr_ms_total > 0`. § Acceptance §9 #1 cover.
  - **step 2** (verifier M-5 — row #2 자동 cover): `app.search(...)` (facade API) — MockOcrEngine 의 `expected_text` substring 검색 → `≥ 1 hit`. isolated TempDir KB 에서 deterministic embedder (text-hash) 사용.
    ```rust
    let mock = MockOcrEngine { expected_text: "MOCK_OCR_UNIQUE_TOKEN_42".into(), fail: false };
    // ... ingest with mock ...
    let hits = app.search("MOCK_OCR_UNIQUE_TOKEN_42")?;
    assert!(hits.iter().any(|h| h.text.contains("MOCK_OCR_UNIQUE_TOKEN_42")));
    ```
  - **step 3** (Step 6 E4 의 production cancel test — paired): `ingest_with_cancel_aborts_mid_pdf` (D3 의 unit test 의 production-path mirror).
- **Acceptance**:
  - `cargo test -p kebab-app --test ingest_pdf_ocr_smoke -j 4` green — **3 test pass** (ingest + search + cancel).

#### Sub-action I4 — Vector PDF regression (`text_extractor_regression.rs`)

- **Files affected**: `crates/kebab-parse-pdf/tests/text_extractor_regression.rs` (신규, spec §5.4), `crates/kebab-parse-pdf/tests/snapshots/vector_pdf_canonical.json` (신규 baseline).
- **Action**:
  - **baseline generation point** (verifier LOW L-2): plan executor 의 first sub-action 으로 baseline generate. 시점 = Step 9 진입 시점 — Step 1-8 의 모든 변경이 vector PDF path 의 결과를 byte-identical 보존하는 invariant. (Step 6 wiring 적용 후 동일 cmd 결과 = baseline byte-identical = M-14 invariant.)
  - F3 (vector PDF, 기존 fixture 가능) → `PdfTextExtractor::new().extract(...)` → `normalize_provenance_timestamps(&mut doc)` → JSON serialize → baseline snapshot `tests/snapshots/vector_pdf_canonical.json` 와 byte-identical.
  - timestamp normalize helper (sub-item 2 의 existing helper reuse — R-3 mitigation):
    ```bash
    grep -rn "normalize_provenance_timestamps\|OffsetDateTime::UNIX_EPOCH" crates/
    # actual helper 위치 record + reuse.
    ```
  - § Acceptance §9 #4 의 verifier evidence.
- **Acceptance**:
  - `cargo test -p kebab-parse-pdf --test text_extractor_regression -j 4` green.
  - vector PDF canonical JSON 의 byte-identical: `diff <(cargo test -p kebab-parse-pdf --test text_extractor_regression -- --nocapture 2>&1 | grep -A100 "BEGIN_SNAPSHOT") tests/snapshots/vector_pdf_canonical.json` empty.

#### Sub-action I5 — alnum accuracy `#[ignore]` test (NEW — verifier M-6 resolution)

- **Files affected**: `crates/kebab-parse-pdf/tests/ocr_e2e.rs` (신규), `crates/kebab-parse-pdf/Cargo.toml` (dev-dep `strsim`).
- **Action**:
  - § Acceptance §9 #3 의 alnum ≥85% (F1) / ≥70% (F2) 의 implementation step.
  - real Ollama dependency — `#[ignore]` default, manual invoke `cargo test -p kebab-parse-pdf --ignored -- ocr_e2e`.
  - alnum metric helper:
    - **option A** (권장): `strsim = "0.11"` dev-dep 추가 — Levenshtein distance. `image` crate 도입 0 invariant 와 무관 (dev-dep, runtime dep 아님).
    - **option B**: PoC python-Levenshtein 의 Rust port (직접 구현 ~50 LOC).
  - test body:
    ```rust
    #[test]
    #[ignore]
    fn f1_alnum_accuracy_ge_85() {
        let pdf = include_bytes!("fixtures/scanned_page1.pdf");
        let ocr = run_real_ollama_ocr(pdf, 1).unwrap();
        let expected = include_str!("fixtures/scanned_page1_truth.txt");
        let accuracy = alnum_accuracy(&ocr, expected);
        assert!(accuracy >= 0.85, "F1 alnum accuracy {} < 0.85", accuracy);
    }

    #[test]
    #[ignore]
    fn f2_alnum_accuracy_ge_70() {
        let pdf = include_bytes!("fixtures/scanned_page2.pdf");
        let ocr = run_real_ollama_ocr(pdf, 1).unwrap();
        let expected = include_str!("fixtures/scanned_page2_truth.txt");
        let accuracy = alnum_accuracy(&ocr, expected);
        assert!(accuracy >= 0.70, "F2 alnum accuracy {} < 0.70", accuracy);
    }

    fn alnum_accuracy(actual: &str, expected: &str) -> f32 {
        let a: String = actual.chars().filter(|c| c.is_alphanumeric()).collect();
        let e: String = expected.chars().filter(|c| c.is_alphanumeric()).collect();
        if e.is_empty() { return 0.0; }
        let dist = strsim::levenshtein(&a, &e) as f32;
        ((e.chars().count() as f32 - dist) / e.chars().count() as f32).max(0.0)
    }
    ```
  - truth file `scanned_page1_truth.txt` / `scanned_page2_truth.txt` = PoC 의 ground-truth (PoC doc 의 §3 region 1 의 expected transcription).
- **Acceptance**:
  - `cargo test -p kebab-parse-pdf --test ocr_e2e -j 4` green (default — `#[ignore]` 가 자동 skip).
  - Manual invoke (사용자 도그푸딩 시): `KEBAB_PDF_OCR_ENABLED=true cargo test -p kebab-parse-pdf --test ocr_e2e --ignored -j 4` — real Ollama (`192.168.0.47:11434` 의 `qwen2.5vl:3b`) 사용, F1 ≥85% + F2 ≥70%.
  - `grep -c "f1_alnum_accuracy_ge_85\|f2_alnum_accuracy_ge_70" crates/kebab-parse-pdf/tests/ocr_e2e.rs` ≥ **2**.
- **Commit message draft** (Step 9 전체):
  ```
  test(pdf): integration smoke (w/ search hit) + vector regression + alnum e2e (#[ignore]) for v0.20 sub-item 1
  ```

### Step 10 (Group J): Docs sync — README + HANDOFF + ARCHITECTURE + SMOKE + release notes

#### Sub-action J0 — release notes path 결정 (NEW pre-flight — verifier M-10 resolution)

- **Files affected**: plan 의 본 sub-action (record only).
- **Action**: v0.19.0 cut 시점의 release notes pattern 확인:
  ```bash
  git log --grep="bump version" --format="%H %s" | head -5
  git log -1 --format="%B" $(git log --grep="bump version 0.18.*0.19" --format="%H" | head -1)
  ```
  - 결과의 release notes path = 다음 셋 중 하나:
    - **(a)** repo root `RELEASE_NOTES.md` (정통적 한 파일).
    - **(b)** gitea-release commit body (gitea-tag 의 message body 가 release notes).
    - **(c)** `docs/RELEASE_NOTES_v<X.Y.Z>.md` (버전 별 파일).
  - record path → J4 의 actual file path 결정.
- **Acceptance**:
  - plan J0 의 record block (본 plan 문서의 §6 retro 또는 J4 sub-action 의 head 1줄) 에 결정된 path + 근거 commit SHA 명시.

#### Sub-action J1 — README.md — `[pdf.ocr]` config section + force-reingest UX

- **Files affected**: `README.md`.
- **Action** (spec §6.4 line 1494):
  - **Configuration** section 에 `[pdf.ocr]` block + 11 field 설명. Off-by-default + opt-in 명시.
  - **force-reingest UX 1줄** (H-4): "**v0.20 upgrade after**: scanned PDF that were ingested in v0.19 (empty block + warning) do NOT auto-pick OCR. Run `kebab ingest --force` to re-process."
  - Mermaid 다이어그램 변경 0 (외부 boundary 변경 0, Ollama 가 이미 boundary 안).
- **Acceptance**:
  - `grep -c "\[pdf.ocr\]" README.md` ≥ **1**.
  - `grep -c "kebab ingest --force" README.md` ≥ **1** (force-reingest wording presence).

#### Sub-action J2 — HANDOFF.md — phase status + 결정 entry

- **Files affected**: `HANDOFF.md`.
- **Action** (spec §6.4 line 1495):
  - phase status table 의 v0.20.0 sub-item 1 (scanned PDF OCR) row 의 status ⏳ → ✅ flip.
  - "머지 후 발견된 버그 / 결정 (요약)" 의 새 1줄 — "v0.20 sub-item 1 (scanned PDF OCR via qwen2.5vl:3b): post-extract enrichment pattern, DCTDecode-only v1 scope, parser_version 유지 + force-reingest UX 명문 (H-4)".
- **Acceptance**:
  - `grep -c "scanned PDF OCR" HANDOFF.md` ≥ **1**.
  - `grep -c "force-reingest" HANDOFF.md` ≥ **1**.

#### Sub-action J3 — docs/ARCHITECTURE.md + docs/SMOKE.md

- **Files affected**: `docs/ARCHITECTURE.md` + `docs/SMOKE.md`.
- **Action**:
  - **ARCHITECTURE**: PDF parser row 의 "locked-in decisions" 에 "qwen2.5vl:3b OCR fallback (PoC 2026-05-27) — DCTDecode passthrough only v1, post-extract enrichment via kebab-app::pdf_ocr_apply" 1줄 추가. crate dep graph 변경 0.
  - **SMOKE**: `[pdf.ocr]` example block 추가 + dogfood §5.10 step 6 (force-reingest 시나리오 — v0.19 binary → v0.20 binary + `kebab ingest --force` 동작 확인) 추가.
- **Acceptance**:
  - `grep -c "qwen2.5vl:3b" docs/ARCHITECTURE.md` ≥ **1**.
  - `grep -c "force-reingest" docs/SMOKE.md` ≥ **1**.
  - `grep -c "pdf.ocr" docs/SMOKE.md` ≥ **1**.

#### Sub-action J4 — v0.20.0 release notes draft

- **Files affected**: J0 의 결정 path 따라 — `RELEASE_NOTES.md` OR commit message body OR `docs/RELEASE_NOTES_v0.20.0.md`.
- **Action** (CLAUDE.md §Release 절차 #2 의 "user 가 이해할 수 있도록 친절하고 자세하게 풀어서 설명" rule):
  - full paragraph 으로 다음 4 topic:
    1. **OCR opt-in 사용법** — `[pdf.ocr] enabled = true` + qwen2.5vl:3b pull 가이드 + remote Ollama endpoint 설정.
    2. **force-reingest 가이드** — v0.19 indexed scanned PDF 의 OCR 미적용 + `kebab ingest --force` 절차.
    3. **DCTDecode-only v1 scope** — FlateDecode / CCITTFax / JPXDecode PDF 의 warning event + qpdf 정규화 가이드.
    4. **family asymmetry deferral** — image OCR 기본 `gemma4:e4b` 유지, PDF OCR 만 `qwen2.5vl:3b`. 향후 별 sub-item.
  - dogfood / test 결과 (PoC alnum 94.79% page1 / 81.56% 받침, integration smoke green) 포함.
- **Acceptance**:
  - (J0 결정 path 따라) `wc -l <release-notes-file>` ≥ 40 OR `git show HEAD --format=%B | wc -l` ≥ 40 (full paragraph 의 baseline).
  - `grep -c "OCR opt-in\|force-reingest\|DCTDecode-only\|family asymmetry" <release-notes-file>` ≥ **4**.
- **Commit message draft** (Step 10 전체):
  ```
  docs(v0.20): sync README + HANDOFF + ARCHITECTURE + SMOKE + release notes for scanned PDF OCR
  ```

### Step 11 (Group K): Version bump + final verify

#### Sub-action K1 — workspace `Cargo.toml` version bump

- **Files affected**: `Cargo.toml` (workspace root) + `Cargo.lock` (auto cascade).
- **Action**:
  ```diff
   [workspace.package]
  -version       = "0.19.0"
  +version       = "0.20.0"   # v0.20.0 sub-item 1 (scanned PDF OCR via qwen2.5vl:3b) — CLAUDE.md §Release 사용자 도그푸딩 트리거
  ```
  cascade 자동 (모든 kebab-* crate 가 `version = { workspace = true }`).
- **Acceptance**:
  - `grep '^version' Cargo.toml | head -1` = `version       = "0.20.0"`.
  - `cargo metadata --no-deps --format-version 1 | jq -r '.packages[0].version'` = `"0.20.0"`.
  - **Cargo.lock cascade** (verifier LOW L-3):
    ```bash
    grep -c '^version = "0.20.0"' Cargo.lock
    # ≥ 20 (workspace 의 모든 kebab-* crate cascade).
    ```

#### Sub-action K2 — (conditional) `cargo clean` + workspace build (release) — **`$RELEASE_BIN`** (H-5)

- **Action** (disk-threshold conditional — §0 retro 2026-05-27, memory `feedback-cargo-clean-policy`):
  ```bash
  # 1. Disk threshold check
  BUILD_AVAIL_KB=$(df -k /build | awk 'NR==2 {print $4}')
  TARGET_SIZE_GB=$(du -s --block-size=1G "$CARGO_TARGET_DIR" 2>/dev/null | awk '{print $1}')
  CARGO_CACHE_GB=$(du -s --block-size=1G /build/out/cargo-target 2>/dev/null | awk '{print $1}')
  # threshold: avail < 500G OR target > 200G OR full cache > 500G
  if [ "$BUILD_AVAIL_KB" -lt $((500*1024*1024)) ] || [ "${TARGET_SIZE_GB:-0}" -gt 200 ] || [ "${CARGO_CACHE_GB:-0}" -gt 500 ]; then
      echo "Threshold reached — running cargo clean"
      cargo clean
  else
      echo "Skipped cargo clean — /build avail=$((BUILD_AVAIL_KB/1024/1024))G, target=${TARGET_SIZE_GB:-0}G, full cache=${CARGO_CACHE_GB:-0}G"
  fi

  # 2. Release build
  cargo build --release -p kebab-cli -j 4
  ```
  CLAUDE.md §Build 의 "target/ 가 90+ GB balloon → cargo clean routinely" 룰은 root disk 가정. `CARGO_TARGET_DIR=/build/out/cargo-target/target` (XFS 4TB) 분리 환경에서는 임계 미달 시 incremental build 가 시간/캐시 효율 ↑.
- **Acceptance** (verifier H-5 resolution — `CARGO_TARGET_DIR` override 충돌 해소):
  - `test -x "${CARGO_TARGET_DIR:-target}/release/kebab"` 또는 plan §0 의 `$RELEASE_BIN` alias 사용:
    ```bash
    test -x "$RELEASE_BIN"  # plan §0 의 alias 사용
    # 또는: test -x "${CARGO_TARGET_DIR:-target}/release/kebab"
    ```
  - 빌드 시간 측정 (5-10 min 예상).

#### Sub-action K3 — `cargo test --workspace --no-fail-fast -j 1` — **precise delta + awk-sum** (M-4, N-1)

- **Action**:
  ```bash
  # baseline (Step 0 진입 시점 — Step 1 A3 의 baseline 캡처와 같이 미리)
  cargo test --workspace --no-fail-fast -j 1 2>&1 \
    | awk '/^test result: ok\./ {for(i=1;i<=NF;i++) if($i=="passed;") sum += $(i-1)} END {print sum}' \
    > .omc/state/pdf-ocr-test-count.baseline.txt

  # after Step 1-10 적용
  cargo test --workspace --no-fail-fast -j 1 2>&1 \
    | awk '/^test result: ok\./ {for(i=1;i<=NF;i++) if($i=="passed;") sum += $(i-1)} END {print sum}' \
    > .omc/state/pdf-ocr-test-count.after.txt

  # delta 확인
  POST=$(cat .omc/state/pdf-ocr-test-count.after.txt)
  PRE=$(cat .omc/state/pdf-ocr-test-count.baseline.txt)
  echo "delta = $((POST - PRE))"
  ```
- **Acceptance** — **precise test 수 delta breakdown** (critic M-4 + verifier M-2/M-3 resolution):
  - kebab-parse-pdf: text_quality 6-7 (F4 활성/ignore conditional) + page_image **2** (verifier M-2) + text_extractor_regression 1 + ocr_e2e 2 (`#[ignore]` 라 baseline 가산 시 0 — `cargo test` 가 ignored 도 count 한다면 +2, default skip 시 +0) = **+11~+12**.
  - kebab-app: pdf_ocr_apply **9** (F7 split, verifier M-4) + ingest_pdf_ocr_smoke **3** (M-5 search step + cancel step) + ingest_progress new 1 = **+13**.
  - kebab-config: pdf_ocr 3 = **+3**.
  - **expected total: +27 ~ +28** (F4 ignore + ocr_e2e ignore 의 conditional 따라).
  - K3 acceptance: `POST - PRE` 가 **+27 또는 +28** (또는 plan executor 의 실측 first sub-action 후 expected delta 확정 — sub-action B2 의 F4 retro 결과로 pin 가능).
  - 전수 pass (모든 non-ignored test).

#### Sub-action K4 — `cargo clippy --workspace --all-targets -- -D warnings`

- **Action** (verifier M-7 — K3 완료 후 sequential): clippy clean. unused-import warn 0.
  ```bash
  cargo clippy --workspace --all-targets -j 1 -- -D warnings
  ```
- **Acceptance**:
  - `cargo clippy --workspace --all-targets -j 1 -- -D warnings` exit 0.

#### Sub-action K5 — § Acceptance §9 #1-#15 row-by-row verifier (**모든 row scriptable**)

- **Action** (spec § Acceptance §9 의 15 row 의 명시적 verify — verifier H-1/H-3/H-5/M-5/M-6/M-8/M-10 resolution 모두 반영):

| # | row | verifier cmd (scriptable) |
|---:|---|---|
| 1 | `pdf_ocr_pages ≥ 1` + `pdf_ocr_ms_total > 0` for scanned PDF | `cargo test -p kebab-app --test ingest_pdf_ocr_smoke -j 4` green |
| 2 | `kebab search` ≥ 1 hit for OCR-only content | **`cargo test -p kebab-app --test ingest_pdf_ocr_smoke ::search -j 4` green** (verifier M-5 — I3 step 2) |
| 3 | alnum accuracy ≥ 85% (F1) / ≥ 70% (F2) | **`cargo test -p kebab-parse-pdf --test ocr_e2e --ignored -j 4` green** — manual invoke, real Ollama (verifier M-6 — I5 new) |
| 4 | F3 byte-identical (regression) | `cargo test -p kebab-parse-pdf --test text_extractor_regression -j 4` green |
| 5 | `Extractor::extract` trait byte-identical | `git diff main -- crates/kebab-core/src/traits.rs \| wc -l` = 0 + `git diff main -- crates/kebab-parse-pdf/src/lib.rs \| grep -E '^[-+]\\s+fn extract' \| wc -l` = 0 |
| 6 | wire schema additive only | **(verifier M-8 — concrete jq + diff)** `jq -r '.properties.kind.enum[]' docs/wire-schema/v1/ingest_progress.schema.json \| sort \| diff - <(echo -e "aborted\\nasset_finished\\nasset_started\\ncompleted\\nembed_batch_finished\\nembed_batch_started\\npdf_ocr_finished\\npdf_ocr_started\\nscan_completed\\nscan_started" \| sort)` exit 0, **그리고** `git diff main -- docs/wire-schema/v1/ingest_report.schema.json \| awk '/^-/ && !/^---/' \| grep -cv "pdf_ocr_"` = 0 |
| 7 | clippy clean | K4 |
| 8 | workspace test clean | K3 |
| 9 | `cargo tree -p kebab-parse-pdf -e normal` 변경 0 (image crate 도입 0) | `diff <(cargo tree -p kebab-parse-pdf -e normal) .omc/state/pdf-ocr-parse-pdf-deps.baseline.txt` empty diff (A3 baseline reuse) |
| 10 | `cargo tree -p kebab-app -e normal \| grep kebab-parse` 변경 0 | **(verifier H-3 — baseline 명시)** `diff <(cargo tree -p kebab-app -e normal \| grep "kebab-parse") .omc/state/pdf-ocr-app-parse-deps.baseline.txt` empty diff |
| 11 | docs sync — README + HANDOFF + ARCHITECTURE + SMOKE + release notes | J1-J4 acceptance grep 전수 green (J0 의 release notes path 결정 반영) |
| 12 | version bump (+Cargo.lock cascade, LOW L-3) | K1 acceptance + `grep -c '^version = "0.20.0"' Cargo.lock` ≥ 20 |
| 13 | dogfood smoke 6 step | manual run (executor) — `$RELEASE_BIN` 사용 |
| 14 | PR #187 invariant — `app.extract_for(&MediaType::Pdf, ...)` 유지 | **(verifier H-1 — function-scope grep)** `awk '/^fn ingest_one_pdf_asset/,/^}/' crates/kebab-app/src/lib.rs \| grep -c "extract_for(&asset.media_type"` ≥ **1** (실측 actual code line 1778 의 literal 매치) |
| 15 | DCTDecode-only v1 — F6/F7 skip path test | **(verifier M-4)** `cargo test -p kebab-app --test pdf_ocr_apply f6_flatedecode_skipped -j 4` green + `cargo test -p kebab-app --test pdf_ocr_apply f7_ccittfax_skipped -j 4` green |

- **Acceptance**:
  - 15 row 모두 green (row #3 의 alnum 은 real Ollama 환경 의존 — `#[ignore]` default 라 manual invoke).

#### Sub-action K6 — Step 11 commit + PR open (**critic M-3 resolution**)

- **Action** (critic M-3 resolution — Step 11 의 commit 만 K6, Step 1-10 의 commit 은 각 step 의 commit message draft 따라 §7 11-commit pattern 으로 happens):
  - Step 1-10 의 각 step 마다 §7 commit table 의 logical commit 가 happens (per-step commit during execution). K6 는 **Step 11 의 version bump + final verify 의 마지막 commit + PR open 만**.
  - `git add Cargo.toml Cargo.lock .omc/state/pdf-ocr-*.txt` (Step 11 K1 의 변경 + K3 의 baseline/after record).
  - `git commit -m "chore: bump version 0.19.0 → 0.20.0 + final verifier evidence (v0.20.0 sub-item 1)"` with HEREDOC body — spec linkage + design contract section + ACCEPT verdict + § Acceptance §9 row-by-row green evidence + Co-Authored-By line:
    ```
    chore: bump version 0.19.0 → 0.20.0 + final verifier evidence (v0.20.0 sub-item 1)

    spec: docs/superpowers/specs/2026-05-27-pdf-scanned-ocr-spec.md (§9 contract)
    plan: docs/superpowers/plans/2026-05-27-pdf-scanned-ocr-plan.md (round 1c)
    verdict: ACCEPT (round 2 critic + verifier closure sonnet)

    § Acceptance §9 row-by-row green:
    - row #1-15: all scriptable verifier cmd green (K5 K4 K3 K2 K1 evidence)
    - test delta: +27~+28 (precise breakdown in K3)
    - cargo tree baseline diff: empty (A3 baseline reuse)

    Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
    ```
  - `gitea-pr --title "feat(pdf): scanned PDF OCR via qwen2.5vl:3b vision LLM (v0.20.0 sub-item 1)" --head feat/pdf-scanned-ocr --base main` (사용자 memory `feedback_pr_workflow` 따라 gitea-pr + 리뷰 루프 모드).
- **Acceptance**:
  - PR open + URL 사용자에게 보고.
  - PR 의 commit count = 11 (§7 table 의 per-step commit) — single squash 가 아닌 logical history 보존.

---

## §4 Verifier checklist (final Step 11 K5 의 acceptance commands)

executor 의 closure step 의 명시적 checklist:

```bash
# 0. RELEASE_BIN alias 활성 (H-5)
export RELEASE_BIN="${CARGO_TARGET_DIR:-target}/release/kebab"

# 1. spec L-1 cosmetic fix applied (A1)
grep -c "app.pdf_ocr_engine.as_ref" docs/superpowers/specs/2026-05-27-pdf-scanned-ocr-spec.md
# 기대: 0

# 2. parser-pdf 의 image crate 미도입 invariant (A2 + K5 row #9)
diff <(cargo tree -p kebab-parse-pdf -e normal) .omc/state/pdf-ocr-parse-pdf-deps.baseline.txt
# 기대: empty diff (A3 baseline reuse)

# 3. parser isolation 보존 (K5 row #10, verifier H-3)
diff <(cargo tree -p kebab-app -e normal | grep "kebab-parse") .omc/state/pdf-ocr-app-parse-deps.baseline.txt
# 기대: empty diff (A3 baseline reuse)

# 4. PR #187 registry invariant (K5 row #14, verifier H-1 function-scope grep)
awk '/^fn ingest_one_pdf_asset/,/^}/' crates/kebab-app/src/lib.rs | grep -c "extract_for(&asset.media_type"
# 기대: ≥ 1 (실측 line 1778 의 actual literal)

# 5. Extractor::extract trait byte-identical (K5 row #5)
git diff main -- crates/kebab-core/src/traits.rs | wc -l
# 기대: 0
git diff main -- crates/kebab-parse-pdf/src/lib.rs | grep -E "^[-+]\s+fn extract" | wc -l
# 기대: 0

# 6. parser_version 보존 (H-4)
grep -c '"pdf-text-v1"' crates/kebab-parse-pdf/src/lib.rs
# 기대: ≥ 1

# 7. force-reingest UX wording presence (H-4)
grep -c "kebab ingest --force" README.md
# 기대: ≥ 1
grep -c "force-reingest" HANDOFF.md docs/SMOKE.md | awk -F: '{sum+=$2} END {print sum}'
# 기대: ≥ 2 (release notes path 는 J0 결정 결과 추가)

# 8. wire schema additive only (K5 row #6, verifier M-8 concrete)
jq -r '.properties.kind.enum[]' docs/wire-schema/v1/ingest_progress.schema.json | sort \
  | diff - <(echo -e "aborted\nasset_finished\nasset_started\ncompleted\nembed_batch_finished\nembed_batch_started\npdf_ocr_finished\npdf_ocr_started\nscan_completed\nscan_started" | sort)
# 기대: exit 0 (additive only — existing 8 entry 보존 + 신규 2 entry)
git diff main -- docs/wire-schema/v1/ingest_report.schema.json | awk '/^-/ && !/^---/' | grep -cv "pdf_ocr_"
# 기대: 0 (existing line removal 0)

# 9. IngestEvent enum (verifier M-1 — file pinned)
grep -c "PdfOcrStarted\|PdfOcrFinished" crates/kebab-app/src/ingest_progress.rs
# 기대: ≥ 2

# 10. IngestItem 새 field
grep -c "pdf_ocr_pages\|pdf_ocr_ms_total" crates/kebab-core/src/ingest.rs
# 기대: ≥ 2

# 11. version bump + Cargo.lock cascade (K5 row #12 + LOW L-3)
grep '^version' Cargo.toml | head -1
# 기대: version       = "0.20.0"
grep -c '^version = "0.20.0"' Cargo.lock
# 기대: ≥ 20 (workspace 의 모든 kebab-* crate)

# 12. workspace test + clippy clean (K5 row #7 + #8, K3 sequential K4)
cargo test --workspace --no-fail-fast -j 1
cargo clippy --workspace --all-targets -j 1 -- -D warnings

# 13. release binary check (K2, verifier H-5)
test -x "$RELEASE_BIN"

# 14. dogfood §5.10 6 step (manual)
KEBAB_PDF_OCR_ENABLED=true "$RELEASE_BIN" ingest --json
# 기대: pdf_ocr_started / pdf_ocr_finished ndjson event stream

# 15. F6 / F7 skip path (K5 row #15, verifier M-4 split)
cargo test -p kebab-app --test pdf_ocr_apply f6_flatedecode_skipped -j 4
cargo test -p kebab-app --test pdf_ocr_apply f7_ccittfax_skipped -j 4
# 기대: 둘 다 green

# 16. alnum e2e (K5 row #3, verifier M-6 — real Ollama 의존 manual invoke)
KEBAB_PDF_OCR_ENABLED=true cargo test -p kebab-parse-pdf --test ocr_e2e --ignored -j 4
# 기대: f1 ≥ 0.85, f2 ≥ 0.70 (real Ollama 환경 의존)
```

---

## §5 Risks (plan 단계)

### R-1 — F1/F2 fixture 가 DCTDecode 가 아닌 경우 (Step 2 probe 의 분기)

PoC fixture (Pillow PNG → PDF) 의 default encoding 이 FlateDecode 일 가능성. 본 시점 미측정 — Step 2 B1 의 probe 가 first deliverable.

- **mitigation**: Step 2 B1 의 결과 negative 시 B2 의 fixture 재합성 deliverable 추가 (`img2pdf` 또는 ImageMagick `-compress jpeg`). probe 자체는 disposable, 10분 미만.
- **fallback**: 재합성 도 실패 시 (라이브러리 부재) — manual JPEG-stream PDF 합성 수작업 (lopdf 의 raw Stream write).

### R-2 — F4 mojibake fixture 합성 reliability (verifier M-9)

reportlab Type 0 font + ToUnicode CMap disable 의 합성이 library version 의존성으로 불안정 가능.

- **mitigation**: spec §5.1 line 1198-1206 의 fallback chain — reportlab 실패 → fpdf2 시도 → lopdf 수작업. 최후 fallback = F4 row 의 acceptance "best-effort, F4 absent → row skip" 으로 downgrade + plan retro 에 record (Step 2 B2 의 retro record 명문).
- **conditional acceptance** (verifier M-9): I1 의 acceptance 가 `ls crates/kebab-parse-pdf/tests/fixtures/mojibake.pdf` 존재 OR plan 의 retro 단락에 "F4 fixture absent" 1줄 명문. K3 의 expected delta 도 F4 ignored 시 자동 -1 (= +26 instead of +27).

### R-3 — sub-item 2 의 normalize_provenance_timestamps helper 위치

vector PDF regression test (Step 9 I4) 의 timestamp normalize helper 의 sub-item 2 existing helper reuse 가능성. helper 위치가 `kebab-normalize` 흡수 후 (PR #186 머지) `kebab-parse-md::tests::common` 또는 별 location 일 수 있음.

- **mitigation**: Step 9 I4 의 first sub-action — `grep -rn "normalize_provenance_timestamps\|OffsetDateTime::UNIX_EPOCH" crates/` 으로 existing helper 위치 확인 후 reuse. 부재 시 신규 helper 작성 (12-line, 30분 미만).

### R-4 — `IngestEvent` 의 actual location + serde discriminant 정합

(verifier M-1 후) `IngestEvent` 의 location pin 완료 = `crates/kebab-app/src/ingest_progress.rs:58`. serde attribute 도 확인 완료 (`#[serde(tag = "kind", rename_all = "snake_case")]`).

- **mitigation**: Step 7 G1 의 first sub-action 으로 actual file content `sed -n '50,80p' crates/kebab-app/src/ingest_progress.rs` 재확인 (plan rewrite 후 codebase 변경 없음 확신).

### R-5 — `kebab-cli` 의 ingest event handler 위치 (H1)

`kebab-cli/src/main.rs` 의 ndjson event mapping 코드 위치 + structure 가 spec 의 예시와 다를 수 있음.

- **mitigation**: Step 8.1 의 first sub-action — `grep -rn "IngestEvent\|scan_started\|asset_started" crates/kebab-cli/src/` 으로 handler 위치 확인 후 mapping 추가.

### R-6 — F1 fixture path 정합 (`docs/superpowers/poc/F1.pdf` vs `crates/kebab-parse-pdf/tests/fixtures/scanned_page1.pdf`)

PoC 의 F1 fixture 가 PoC doc 안에 raw 보관 또는 별 fixture path. 본 plan 의 page_image test 가 `crates/kebab-parse-pdf/tests/fixtures/scanned_page1.pdf` 를 path 로 명시 (spec §5.1, B2 commit target).

- **mitigation**: Step 2 B2 의 일부 — PoC 의 F1/F2 fixture actual path 확인 + `crates/kebab-parse-pdf/tests/fixtures/` 로 copy commit. 두 location 의 dual 보관도 가능.

### R-7 — qwen2.5vl 의 Ollama host availability (§7.6)

dogfood smoke (§5.10) + alnum e2e (Step 9 I5) 가 실제 Ollama 호출 — host (192.168.0.47:11434) 의 `qwen2.5vl:3b` pull 필요. 미pull 시 503 또는 pull-in-progress.

- **mitigation**: Step 9 I3 의 integration smoke 는 MockOcrEngine 사용 — Ollama dependency 0. I5 ocr_e2e 와 dogfood smoke 만 real Ollama. dogfood / e2e 전에 `ollama pull qwen2.5vl:3b` 사용자 사전 실행. I5 의 `#[ignore]` default 가 CI 자동 skip 보장.

### R-8 — snapshot regenerate 의 cascade 영향 (Step 8 H2)

`ingest_progress_*.rs` snapshot 의 baseline 갱신이 다른 PDF ingest test (e.g. text-only PDF) 의 snapshot 에도 cascade 가능. `pdf_ocr_pages: null` 추가가 ndjson 전수 변경 트리거.

- **mitigation**: Step 8 H2 의 acceptance — 기존 PDF (OCR off) snapshot 의 변경 = `pdf_ocr_pages: null` + `pdf_ocr_ms_total: null` 두 field 추가만 (M-9 wire convention). 다른 field 변경 0. `git diff` awk 의 `pdf_ocr_` 제외 grep 으로 확인.

### R-9 — workspace test 의 `-j 1` 시간 (Step 11 K3) — **K3 / K4 sequential 진행** (verifier M-7)

기존 baseline ~1316 test + new ~27 test = ~1340 test, 18 integration binary serial link + run. CPU bound (~15-30 min) — 시간 risk 가 cost 아님.

- **mitigation** (verifier M-7 resolution): K3 / K4 sequential 진행. plan §0 의 "직렬 진행" rule 정합. K3 background + K4 mutually independent 문구 삭제. K3 의 측정이 cargo invoke 의 incremental compilation lock 의 single-owner — K4 는 K3 종료 후 시작.

### R-10 — `IngestEventSender` 의 actual type 정합

spec §4.4 의 diff 가 `progress: Option<&IngestEventSender>` (또는 spec 의 `IngestProgressSender` wording) 를 carry — 본 type 의 actual 위치 + signature 확인 필요.

- **mitigation**: Step 6 E2 의 first sub-action — `grep -rn "struct IngestEventSender\|IngestProgressSender" crates/` 으로 actual 정의 확인 후 import path 정합.

---

## §6 Open questions deferred to executor

executor 가 plan 진입 후 첫 step 들에서 결정 + spec/plan 의 record (M-N resolution 의 plan executor deliverable list 의 sub-item):

### OQ-E1 — F1/F2 fixture path 의 dual 보관 여부

PoC 의 F1/F2 가 `docs/superpowers/poc/` 또는 PoC 내부의 임시 path 일 가능성. test fixture 로 commit 시 `crates/kebab-parse-pdf/tests/fixtures/` 로 copy. dual 보관 (poc + test fixture) 가능. executor 가 Step 2 B2 sub-action 에서 결정.

### OQ-E2 — `mod page_image; pub use` 의 export surface

`kebab-parse-pdf/src/lib.rs` 의 export = `extract_dctdecode_page_image` + `compute_valid_char_ratio` 두 fn 만 vs internal mod 의 모든 pub 항목. spec §4.1 의 module skeleton 은 두 fn export 명시. 단 `OllamaVisionOcr` 가 facade 에서만 import 되도록 — `page_image` / `text_quality` 의 internal helper 는 mod private 유지.

executor 가 Step 3 의 첫 sub-action 에서 explicit 결정.

### OQ-E3 — `OllamaVisionOcr` 의 `request_timeout_secs = 0` semantic

spec §4.5 line 963-965 — `request_timeout_secs = 0` 의 "fail immediately" semantics. image OCR (`crates/kebab-parse-image/src/ocr.rs`) 의 actual behavior 확인 필요 (spec doc 의 인용 vs actual code).

executor 가 Step 5 first sub-action — image OCR ocr.rs grep 으로 actual behavior 확인 + plan/config doc 의 정합.

### OQ-E4 — `MockOcrEngine::recognize` 의 `OcrText.engine` field 의 owned vs &'static

spec §5.5 line 1293-1298 의 MockOcrEngine 의 `OcrText { ..., engine: self.engine_name().to_string(), engine_version: self.engine_version() }`. `OcrText.engine` field 의 actual type (String vs &'static str) 확인 — actual `kebab-parse-image::ocr::OcrText` 의 field type 따라 mock 정합.

executor 가 Step 4 first sub-action — `kebab-parse-image::ocr::OcrText` 의 field type 확인 후 mock impl 정합.

### OQ-E5 — `KEBAB_PDF_OCR_LANGUAGES` env 의 array parsing (comma-separated)

spec §4.5 line 1024 — `KEBAB_PDF_OCR_LANGUAGES="eng,kor"` 의 comma split + trim + filter empty pattern. image OCR 의 env override 가 동일 pattern 이지만 actual impl 확인 필요.

executor 가 Step 5 first sub-action.

### OQ-E6 — `IngestEvent::kind` 의 `snake_case` discriminant tag

(verifier M-1 후 확인 완료) actual = `#[serde(tag = "kind", rename_all = "snake_case")]` (`crates/kebab-app/src/ingest_progress.rs:57`). 즉 `PdfOcrStarted` → `"pdf_ocr_started"` 자동 매핑. JSON Schema enum value 와 일관.

executor 가 Step 7 G1 first sub-action 으로 attribute 재확인.

### OQ-E7 — release notes path (`RELEASE_NOTES.md` vs gitea-release commit message body) — **J0 pre-flight 명문 deliverable** (verifier M-10)

CLAUDE.md §Release 절차 #2 의 "release notes" 가 별 file 인지 gitea-release commit body 인지 모호. **본 OQ 는 Step 10 J0 (NEW) 의 명시적 deliverable — deferred 아님**. 기존 v0.19.0 cut 시점의 patterns 확인 명령:

```bash
git log --grep="bump version" --format="%H %s" | head -5
git log -1 --format="%B" <bump-version-commit-sha>
```

J0 의 결과 path = `RELEASE_NOTES.md` OR `docs/RELEASE_NOTES_v0.20.0.md` OR commit body 의 셋 중 하나로 record.

### OQ-E8 — `cargo insta` 사용 여부 (snapshot regenerate, Step 8 H2)

`crates/kebab-app/tests/ingest_progress_*.rs` 의 snapshot library 가 `cargo insta` 인지 수작업 baseline 인지 확인. `cargo insta` 시 `cargo insta accept` 가 baseline 갱신.

executor 가 Step 8 H2 first sub-action — `grep -rn "insta\|assert_snapshot" crates/kebab-app/tests/` 으로 actual snapshot library 확인.

### OQ-E9 — `kebab-config` 의 endpoint fallback 의 actual field (`models.llm.endpoint`)

spec §4.5 line 956 + spec §4.4 line 811 의 `app.config.models.llm.endpoint` — `kebab-config::Config` 의 actual field 명칭 (`models.llm.endpoint` vs `llm.endpoint` 등) 확인 필요.

executor 가 Step 5 first sub-action — `grep -rn "endpoint" crates/kebab-config/src/lib.rs` 으로 actual field 확인.

### OQ-E10 — pdf_ocr_engine + cancel handle 의 ingest dispatch loop wiring — **E4 의 명시적 deliverable** (critic M-2)

(critic M-2 후) `ingest_one_pdf_asset` 가 caller (ingest dispatch loop) 에서 `pdf_ocr_engine: Option<&OllamaVisionOcr>` + `progress: Option<&IngestEventSender>` + `cancel: Option<&Arc<AtomicBool>>` carry — caller (dispatch loop) 의 actual location + 변경 필요한 caller 수 확인. **본 OQ 는 Step 6 E4 의 명시적 deliverable — deferred 아님**.

executor 가 Step 6 E4 sub-action — `grep -rn "ingest_one_pdf_asset(" crates/kebab-app/src/` 으로 caller 위치 확인 후 update.

---

## §7 Sequencing summary (logical commit boundaries — critic M-3 resolution)

본 plan 의 34 sub-action 의 logical commit grouping (K6 는 §7 의 commit table 우선 — single squash 아님):

| commit # | step range | logical scope |
|---:|---|---|
| 1 | Step 1 (A1+A2+A3) | docs(spec)+chore(plan-bootstrap): L-1 cosmetic + module skeleton + cargo tree baselines |
| 2 | Step 2 (B1+B2) | poc+test(pdf-ocr): lopdf /Filter probe + 5 fixture commit (F1/F2/F4/F6/F7) |
| 3 | Step 3 (C1+C2) | feat(parse-pdf): page_image (2 test) + text_quality module |
| 4 | Step 4 (D1+D2+D3) | feat(app): pdf_ocr_apply helper (9 test, F7 split) |
| 5 | Step 5 (F1+F2) | feat(config): [pdf.ocr] section |
| 6 | Step 6 (E1+E2+E3+E4) | feat(app): wire PDF OCR enrichment + cancel propagation |
| 7 | Step 7 (G1+G2+G3) | feat(wire): additive minor — IngestEvent + IngestItem + JSON Schema |
| 8 | Step 8 (H1+H2) | feat(cli): humanize pdf_ocr events + snapshot baseline |
| 9 | Step 9 (I3+I4+I5) | test(pdf): integration smoke (w/ search) + vector regression + alnum e2e |
| 10 | Step 10 (J0+J1-J4) | docs(v0.20): sync README + HANDOFF + ARCHITECTURE + SMOKE + release notes (path pinned) |
| 11 | Step 11 (K1+K2-K5+K6) | chore: bump version 0.19 → 0.20 + final verifier evidence |

**11 commit**. PR open 시 `gitea-pr --title "feat(pdf): scanned PDF OCR via qwen2.5vl:3b vision LLM (v0.20.0 sub-item 1)" --head feat/pdf-scanned-ocr --base main` (사용자 memory `feedback_pr_workflow` 의 gitea-pr + 리뷰 루프).

executor 가 review feedback 따라 micro-patch round (sonnet) — 사용자 memory `feedback_teammate_model_routing` 의 routing 정합.

---

## §8 Round 1c rewrite changelog (drafter trace)

본 round 의 plan 변경 summary (critic round 1 의 MEDIUM 4 + verifier round 1 의 HIGH 5 + MEDIUM 10 + LOW 5 + NIT 3 의 resolution):

### critic MEDIUM 4 적용
- **M-1** (Step 6/7/8 RED test missing): Step 3 C2 / Step 7 G3 / Step 8 H2 의 MEDIUM-1 cross-reference block 추가 — "Step 4 D1 의 9 test + Step 8 H2 의 새 test 가 wiring + wire + printer 의 effective RED→GREEN coverage" 명문.
- **M-2** (cancel wiring missing step): **Step 6 E4 new sub-action** — cancel handle propagation (ingest entry → PdfOcrOpts.cancel). production cancel smoke test 가 Step 9 I3 step 3 으로 추가.
- **M-3** (K6 commit grouping vs §7 11-commit): K6 wording 정정 — Step 11 의 version bump + final verify 의 마지막 commit + PR open 만. Step 1-10 의 commit 은 §7 table 의 per-step commit.
- **M-4** (K3 test delta precision): K3 acceptance 의 delta breakdown 정확화 — kebab-parse-pdf +11~+12 + kebab-app +13 + kebab-config +3 = **+27~+28** (F4 / ocr_e2e ignore conditional).

### verifier HIGH 5 적용
- **H-1** (K5 row #14 grep regex case-mismatch): function-scope grep — `awk '/^fn ingest_one_pdf_asset/,/^}/' ... | grep -c "extract_for(&asset.media_type"` ≥ 1.
- **H-2** (Step 6 E3 acceptance grep 동일 case-mismatch): H-1 와 동일 pattern.
- **H-3** (cargo tree baseline file 미캡처): **Step 1 A3 new sub-action** — baseline `.omc/state/pdf-ocr-app-parse-deps.baseline.txt` + `pdf-ocr-parse-pdf-deps.baseline.txt` 캡처.
- **H-4** (Step 4 D1 fixture-dependent test sequencing): **모든 fixture commit (F1/F2/F4/F6/F7) 을 Step 2 B2 로 끌어옴**. Step 9 는 integration smoke + regression + alnum e2e 만.
- **H-5** (K2 path override 충돌): plan §0 의 `RELEASE_BIN` alias 정의 + K2 acceptance 가 `test -x "$RELEASE_BIN"` 사용.

### verifier MEDIUM 10 적용
- **M-1** (IngestEvent location + naming): G1 "Files affected" 를 `crates/kebab-app/src/ingest_progress.rs` 로 pin + acceptance grep target 변경. OQ-E6 도 actual confirm 후 wording 갱신.
- **M-2/M-3** (page_image test count + K3 arithmetic): **C1 의 test list 2 test 로 확장** (`f1_fixture_yields_dctdecode_jpeg_bytes` + `flate_raw_fixture_yields_none`). K3 delta breakdown 정정 (+27~+28).
- **M-4** (F7 CCITTFax test name 누락): D1 의 test 6 split — `f6_flatedecode_skipped_with_warning` + `f7_ccittfax_skipped_with_warning`. K5 row 15 의 verifier 도 두 test name 모두 명시.
- **M-5** (§9 row #2 search hit automated coverage 0): **I3 acceptance step 2 추가** — `app.search(...)` 호출 + MockOcrEngine expected_text substring 검색 ≥ 1 hit.
- **M-6** (§9 row #3 alnum accuracy implementation step 부재): **Step 9 I5 new sub-action** — `crates/kebab-parse-pdf/tests/ocr_e2e.rs` 신규 + `#[ignore]` test 2 (`f1_alnum_accuracy_ge_85` / `f2_alnum_accuracy_ge_70`) + `strsim` dev-dep.
- **M-7** (R-9 background vs §0 직렬 rule 충돌): K3 / K4 sequential 진행. R-9 mitigation 의 "background + 다른 작업 mutually independent" 문구 삭제.
- **M-8** (K5 row #6 wire schema diff command vague): concrete jq + diff command — additive only verifier script 명시 (K5 + §4).
- **M-9** (F4 fixture 합성 fallback 미반영): B2 + I1 acceptance 가 conditional — fixture 존재 OR retro 단락의 "F4 absent → row skip" 1줄 명문. K3 expected delta 자동 -1 fallback.
- **M-10** (RELEASE_NOTES.md path 결정 deferral): **Step 10 J0 new sub-action** — `git log --grep="bump version"` 으로 path 결정 + record.

### LOW + NIT 적용 (best-effort)
- **critic LOW-1 / NIT-1** (Group letter order F before E): §2 table header 의 disclaimer 추가 — "Group letter 는 spec §3 design section 이름 mirror".
- **critic LOW-2** (snapshot baseline generation point): I4 acceptance 에 baseline generation 시점 명문 (Step 9 진입 시점 = Step 1-8 변경 후).
- **critic LOW-3** (Step 2 fixture commit Files affected 누락): B2 의 "Files affected" 가 5 fixture file 전수 명시.
- **critic LOW-4** (F4 fixture timing — Step 3 C2 tests gated): C2 의 F4 test `#[ignore = "F4 fixture absent — Step 2 B2 retro record 참조"]` annotation pattern 명시.
- **critic LOW-5** (line number references approx): R-4 / R-5 / R-10 mitigation 으로 plan executor 의 first sub-action grep — accept 그대로.
- **verifier LOW-1** (PdfOcrOpts.cancel spec inconsistency): D1 의 PdfOcrOpts body 가 spec §4.1 + §4.8 의 합집합 — plan 이 correct. spec L-2 cosmetic fix 후행 옵션.
- **verifier LOW-2** (F4 test name plan vs spec): plan 의 `f4_fixture_ratio_under_threshold` 로 pin (C2 acceptance 의 명령과 정합). spec L-2 cosmetic fix 후행 옵션.
- **verifier LOW-3** (Cargo.lock cascade 검증): K1 acceptance 에 `grep -c '^version = "0.20.0"' Cargo.lock` ≥ 20 추가.
- **verifier NIT-1** (awk-sum delta 측정): K3 의 baseline / after awk-sum cmd 명시 (`.omc/state/pdf-ocr-test-count.{baseline,after}.txt`).
- **verifier NIT-2** (K6 HEREDOC fixture): K6 의 commit body HEREDOC actual template 명시 (Step 11 의 body block).

### Sub-action count 변경
- 본 round 의 plan 변경 summary: 31 sub-action → **34 sub-action** (+3 = A3 baseline / B2 fixture relocation / E4 cancel / I5 ocr_e2e; -1 = I1/I2 fixture 합성 step 이 B2 로 흡수). Step 9 는 I3 + I4 + I5 의 3 sub-action.

### Trace 출처 file
- `/home/altair823/kebab/.omc/reviews/2026-05-27-pdf-ocr-plan-critic-r1-result.md` (327 lines) — critic round 1 thorough opus.
- `/home/altair823/kebab/.omc/reviews/2026-05-27-pdf-ocr-plan-verifier-r1-result.md` (~395 lines) — verifier round 1 thorough opus.
- 본 round 의 report: `/home/altair823/kebab/.omc/reviews/2026-05-27-pdf-ocr-plan-rewrite-report.md` (drafter 1c traceability matrix).
