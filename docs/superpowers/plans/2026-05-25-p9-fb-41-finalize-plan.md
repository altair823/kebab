---
title: "p9-fb-41 finalize implementation plan v4 — NLI verification + v0.18.0 cut"
date: 2026-05-25
task_id: p9-fb-41-finalize
phase: P9
status: completed
target_version: 0.18.0
design: ../specs/2026-05-25-p9-fb-41-finalize-spec.md
spec_review_round: 5
spec_status: completed
plan_review_round: 3
plan_review_outcome: |
  All 4 OMC team reviewers APPROVE (plan v4 round 3, FINAL convergence).
  - architect: APPROVE (round 1 plan v2)
  - planner: APPROVE (round 2 spec + plan v2 re-confirmed)
  - document-specialist: APPROVE (round 2 plan v3 — NIT-1 minor)
  - critic: APPROVE (round 3 plan v4 FINAL — 5 axes 95.4% production excellence baseline)
---

# p9-fb-41 finalize plan v4

spec: `docs/superpowers/specs/2026-05-25-p9-fb-41-finalize-spec.md` (review_round=5, APPROVE by all 4 OMC team reviewers).

## 0. 작업 개요

PR-1 ~ PR-8 머지 후 v0.18 pre-cut 도그푸딩 (`/build/cache/dogfood-v018/results/SUMMARY.md`) 에서 발견된 S7 hallucination 의 진짜 fix (NLI post-synthesis verification) + v0.18.0 cut.

총 5 sub-PR (9a / 9b / 9c-1 / 9c-2 / 9d) + 1 cut PR. **총 추정 시간**: 작업 **21-31h** / wall time **28-44h** (§8 cumulative trace + plan v4 round-2 critic M2 분리 참조).

PR sequence 는 *순차* (각 PR 머지 후 다음 시작) — sub-PR 별 surface 가 다음 sub-PR 의 기반:

```
PR-9a (skeleton)
  ↓ 머지 후
PR-9b (ONNX inference)
  ↓ 머지 후
PR-9c-1 (core types + wire scaffolding)
  ↓ 머지 후
PR-9c-2 (pipeline integration + mock test)
  ↓ 머지 후
PR-9d (dogfood retest + HOTFIXES)
  ↓ 머지 후
cut PR (chore: bump version 0.17.2 → 0.18.0)
  ↓ 머지 + tag v0.18.0
```

본 plan 은 subagent-driven-development 의 task list — 각 sub-PR 의 *self-contained* description.

## 1. 머지된 PR-1 ~ PR-8 의 carry-over

각 PR 의 회차 리뷰 carry-over 항목은 본인 PR 안 또는 후속 PR 에서 해소됨. 본 plan 의 PR-9 sub-PRs 에는 추가 carry-over 없음 — clean baseline.

## 2. PR-9a — kebab-nli crate skeleton

**Goal**: trait surface + scaffolding + workspace dep chain 도입. implementation 없이도 build 가능.

**Pre-flight (PR-9a 시작 전, manual)** — spec §2.1 + §3 PR-9a:

1. **Model + tokenizer file 존재 검증**:
   ```sh
   curl -I https://huggingface.co/Xenova/mDeBERTa-v3-base-xnli-multilingual-nli-2mil7/resolve/main/onnx/model.onnx
   curl -I https://huggingface.co/Xenova/mDeBERTa-v3-base-xnli-multilingual-nli-2mil7/resolve/main/tokenizer.json
   ```
   둘 다 `200 OK` 확인. 실패 시 PR-9 design re-evaluation.

2. **`tokenizers` features 검증** (standalone repro):
   ```sh
   cargo new --bin /tmp/nli-tok-probe
   cd /tmp/nli-tok-probe
   cargo add tokenizers --no-default-features -F onig
   wget https://huggingface.co/Xenova/mDeBERTa-v3-base-xnli-multilingual-nli-2mil7/resolve/main/tokenizer.json
   # main.rs: tokenizers::Tokenizer::from_file("tokenizer.json").expect("load");
   cargo run --release
   ```
   성공 시 PR-9a features lock. 실패 시 `default-features = true` fallback. 결과 + 최종 features set 을 PR-9a PR description 의 `## Cargo features 결정 trace` 절에 첨부.

**Files**:

- `Cargo.toml` (workspace):
  - `members` 에 `"crates/kebab-nli"` 추가.
  - `workspace.dependencies` 에 추가 (fastembed transitive 와 정확히 일치):
    - `ort = { version = "=2.0.0-rc.9", default-features = false, features = ["ndarray"] }`
    - `tokenizers = { version = "0.21", default-features = false, features = ["onig"] }` (pre-flight 결과에 따라 features 갱신 가능)
    - `hf-hub = { version = "0.4", default-features = false, features = ["ureq", "rustls-tls"] }`
    - `ndarray = "0.16"`
- `crates/kebab-nli/Cargo.toml` (skeleton 의존만):
  - `dependencies`: `kebab-config`, `anyhow`, `serde`.
  - `dev-dependencies`: `tempfile`.
- `crates/kebab-nli/src/lib.rs`:
  - `NliScores` struct + `faithfulness()` + `from_xnli_logits()`.
  - `NliVerifier` trait.
  - private `softmax3` helper.
- `crates/kebab-nli/src/onnx.rs`:
  - `OnnxNliVerifier` placeholder struct.
  - `OnnxNliVerifier::new(&Config) -> Result<Self>` placeholder.
  - `impl NliVerifier::score → bail!("PR-9a stub")`.

**Tests** (6 unit):
- `softmax3_normalises_to_unit`, `softmax3_is_invariant_to_constant_shift`.
- `nli_scores_from_xnli_logits_orders_correctly`, `faithfulness_returns_entailment_channel`.
- `new_succeeds_on_default_config`, `score_returns_err_in_skeleton`.

**검증**:
- `cargo test -p kebab-nli -j 1` — 6 통과.
- `cargo clippy -p kebab-nli --all-targets -j 1 -- -D warnings` clean.

**시간**: 2-3h.

## 3. PR-9b — OnnxNliVerifier 의 ONNX inference + model download

**Goal**: `OnnxNliVerifier::score` 의 진짜 implementation.

**Dependency**: PR-9a 머지 완료.

**Files**:
- `crates/kebab-nli/Cargo.toml`:
  - `ort`, `tokenizers`, `hf-hub`, `ndarray`, `tracing` 추가 (workspace.dependencies).
- `crates/kebab-nli/src/onnx.rs`:
  - `OnnxNliVerifier` fields: `model_id`, `cache_dir` (= `config.storage.model_dir.join("nli").join(sanitize(model_id))`), `session: OnceLock<ort::Session>`, `tokenizer: OnceLock<tokenizers::Tokenizer>`.
  - `OnnxNliVerifier::new(&Config) -> Result<Self>` — model_id / cache_dir stamp + lazy load deferred.
  - `ensure_loaded(&self) -> Result<(&Session, &Tokenizer)>` — hf-hub download + `Tokenizer::from_file` + `Session::commit_from_file` + truncation params 설정.
  - `score(premise, hypothesis)` — encode pair (with OnlyFirst truncation) → ort run → softmax → NliScores.
  - `sanitize_model_id(s: &str) -> String` helper.
- `crates/kebab-nli/tests/inference.rs` 신규:
  - `#[ignore]` integration tests (5 cases):
    1. EN entailment (`"Caffeine is a stimulant."` → `"Caffeine is a stimulant."`) — entailment > 0.8.
    2. EN no-entailment (caffeine → C8H10N4O2) — entailment < 0.3.
    3. KR entailment (`"사과는 빨갛다."` → `"사과는 색이 있다."`) — entailment 높음.
    4. Long premise (10000 char) → truncation 적용 + 정상 score (panic 없음).
    5. Empty hypothesis → graceful error.

**Manual smoke protocol** (PR description 강제 첨부):
```sh
cargo test -p kebab-nli -j 1 --test inference -- --ignored 2>&1 | tail -20
```
- 5 test 모두 PASS 확인.
- case 1 의 `NliScores` dump (예: `entailment=0.92, neutral=0.05, contradiction=0.03`) 를 PR body 의 `## 검증` 절에 inline.

**검증**:
- unit test 통과 + clippy clean.
- `--ignored` integration test 의 manual run (PR 작업자 책임).

**시간**: 8-12h (round-2 planner P2 갱신).

**Risks**:
- ort 2.0-rc.9 API stability — workspace pin `"=2.0.0-rc.9"` (fastembed transitive 일치).
- mDeBERTa ONNX 존재 — PR-9a pre-flight 가 검증.
- tokenizers SentencePiece 호환성 — PR-9a pre-flight 가 검증.
- hf-hub `ureq + rustls-tls` vs fastembed `native-tls` features union — PR-9a 의 첫 build 가 검증.

## 4. PR-9c-1 — Core types + wire scaffolding

**Goal**: `RefusalReason` + `VerificationSummary` + `RagPipeline.verifier` field + Config + wire schema.

**Dependency**: PR-9b 머지 완료.

**Files**:
- `crates/kebab-core/src/answer.rs`:
  - `RefusalReason::NliVerificationFailed` + `RefusalReason::NliModelUnavailable` 신규.
  - `Answer.verification: Option<VerificationSummary>` field.
  - `VerificationSummary { nli_score: f32, nli_threshold: f32, nli_passed: bool }` 신규 struct.
- `crates/kebab-config/src/lib.rs`:
  - `NliCfg` 신규 struct + `[models.nli]`:
    - `model: String` (default `"Xenova/mDeBERTa-v3-base-xnli-multilingual-nli-2mil7"`).
    - `provider: String` (default `"onnx"`).
  - `RagCfg.nli_threshold: f32` (default `0.0` — disabled).
  - env override: `KEBAB_MODELS_NLI_MODEL`, `KEBAB_RAG_NLI_THRESHOLD`.
- `crates/kebab-rag/src/pipeline.rs`:
  - `RagPipeline` 의 새 field: `verifier: Option<Arc<dyn NliVerifier>>` (None = verify off).
  - **시그니처 widening = Option B (builder)**: 기존 `RagPipeline::new(config, retriever, llm, sqlite)` 시그니처 유지 + 신규 `pub fn with_verifier(self, v: Arc<dyn NliVerifier>) -> Self` builder.
  - `kebab-rag` 의 Cargo.toml 에 `kebab-nli` 의존 추가.
  - **`#[allow(dead_code)]` 처리** (round-2 critic M1 closure): PR-9c-1 의 `verifier` field 와 `with_verifier` builder 는 PR-9c-2 의 `ask_multi_hop` step 8.5 hook 가 활성화될 때까지 *unused — `cargo clippy -- -D warnings` 의 `dead_code` lint fail risk*. PR-9c-1 의 `verifier` field 에 `#[allow(dead_code)]` 임시 attribute (Cargo.toml 의 `kebab-nli` 의존 자체는 active path). 또는 placeholder smoke test (`pipeline.with_verifier(MockVerifier::default()).verifier.is_some()` 한 줄). PR-9c-2 가 hook 추가 시 attribute 제거.
- `docs/wire-schema/v1/answer.schema.json`:
  - `verification` field 추가 (`anyOf [object, null]`) + `$defs.VerificationSummary` 인라인:
    ```json
    "VerificationSummary": {
      "type": "object",
      "required": ["nli_score", "nli_threshold", "nli_passed"],
      "properties": {
        "nli_score":     { "type": "number" },
        "nli_threshold": { "type": "number" },
        "nli_passed":    { "type": "boolean" }
      }
    }
    ```
  - `refusal_reason.enum` 에 `"nli_verification_failed"`, `"nli_model_unavailable"` 추가.
- `docs/wire-schema/v1/error.schema.json`:
  - `code` enum 에 `nli_verification_failed`, `nli_model_unavailable` 추가.
  - `details.description` 에 두 항목 추가 (`multi_hop_decompose_failed` 패턴):
    - `nli_verification_failed: { score, threshold }` (reserved — currently emitted as Answer.refusal_reason on stdout, NOT as error.v1; forward-looking for future RefusalReason → error_wire promotion).
    - `nli_model_unavailable: { source }` (reserved — same pattern as nli_verification_failed).
- `docs/ARCHITECTURE.md` (round-1 document-specialist ISSUE-1 — CLAUDE.md "A new crate is added — extend the graph + directory tree" rule):
  - Mermaid Adapters subgraph 에 `nli["kebab-nli<br/>(NLI verifier)"]` 노드 추가.
  - **Edges** (round-2 critic R2-NIT-3 — *forward-looking final state* 명시):
    - PR-9c-1 시점 *직접 의존 추가* = `rag --> nli` (kebab-rag/Cargo.toml `kebab-nli` 추가) + `nli --> config` (kebab-nli/Cargo.toml `kebab-config` 추가).
    - `app --> nli` edge 는 *forward-looking* (PR-9c-2 에서 kebab-app/Cargo.toml 의 `kebab-nli` 의존 추가됨) — PR-9c-1 의 ARCHITECTURE.md 가 *최종 graph 상태* 반영 결정 (single update, 9c-2 에서 재변경 회피). 결정 trade-off: *forward-looking* 가 reader 의 `final state` 가시 ↑, *current state* 가 PR-9c-1 시점 정확도 ↑. **권장 forward-looking** (graph 가 surface 명세, 한 번 갱신).
    - `nli --> core` edge 는 PR-9a 머지 후 `crates/kebab-nli/Cargo.toml` 의 final `[dependencies]` 확인 결정 (round-2 document-specialist NIT-1) — `kebab-core` 직접 의존 시 edge 포함, `config` 경유 transitive 만이면 edge 생략. ARCHITECTURE.md graph 관례 = *직접 Cargo.toml 의존* 기준.
  - 디렉토리 트리에 `crates/kebab-nli/` 항목 추가.

**Tests**:
- `crates/kebab-config/src/lib.rs::tests`:
  - `default_nli_threshold_is_zero`.
  - `default_nli_model_is_xenova_mdeberta`.
  - `legacy_config_without_nli_uses_defaults`.
  - `env_override_nli_threshold`.
- `crates/kebab-cli/tests/wire_ask_multi_hop.rs`:
  - `answer_schema_declares_verification_field_and_defs`.
  - `answer_schema_refusal_reason_enum_includes_nli_verification_failed`.
  - `answer_schema_refusal_reason_enum_includes_nli_model_unavailable`.
  - `error_schema_code_enum_includes_nli_verification_failed`.
  - `error_schema_code_enum_includes_nli_model_unavailable`.

**검증**:
- `cargo test --workspace -j 1` — 회귀 0 (기존 multi-hop tests pass, RagPipeline::new 시그니처 unchanged).
- `cargo clippy --workspace --all-targets -j 1 -- -D warnings` clean.

**시간**: 2-3h.

## 5. PR-9c-2 — Pipeline integration + mock test

**Goal**: `ask_multi_hop` 의 NLI verify wiring + mock test + SKILL.md 갱신.

**Dependency**: PR-9c-1 머지 완료.

**Files**:
- `crates/kebab-rag/src/pipeline.rs`:
  - `ask_multi_hop` 의 step 8.5 NLI hook (spec §2.3 코드):
    - empty answer guard: `if !acc.trim().is_empty() { /* step 8.5 */ }`.
    - `if self.config.rag.nli_threshold > 0.0 { /* verify */ }` outer guard.
    - inner verify: `truncate_for_nli` → `verifier.score` → score 검사 → refuse 또는 진행.
  - `refuse_nli_verification` helper (`refuse_*` 패턴) — `verification: Some(...)` 채움.
  - `refuse_nli_model_unavailable` helper — `verification: None`.
  - `pub fn truncate_for_nli(premise: &str, hypothesis: &str) -> (String, bool)` helper:
    - max premise char count = `MAX_NLI_PREMISE_CHARS = 4 * 400` ≈ 1600 chars.
    - hypothesis 길이 + special tokens 32 char budget 적용 후 자연 보존.
    - 둘째 return = was_truncated boolean.
    - **token ratio 가정**: 4 char ≈ 1 token (영어 BPE). 한국어 SentencePiece 는 1-2 char/token — tokenizer OnlyFirst backup. v0.18.1 의 token-count 기반 budget 갱신 candidate.
- `crates/kebab-app`:
  - **실제 constructor 이름 = `App::open_with_config`** (round-2 critic R2-NIT-4 verification — `crates/kebab-app/src/app.rs:187`. spec §3 PR-9c-2 의 `App::new` 는 *논리적 이름* — 실제 code 의 함수명으로 mapping). 시그니처 *이미 `Result<Self, anyhow::Error>`* (현재 line 187 `pub fn open_with_config(config: kebab_config::Config) -> Result<Self>`) — **caller cascading 없음** (kebab-cli/tui/mcp 의 `App::open_with_config(...)` 호출 site 의 `?` 또는 `.context(...)` 그대로). round-2 NEW-M2 의 *시그니처 widening* = body 추가만 (`OnnxNliVerifier::new(config)?` integration).
  - `config.rag.nli_threshold > 0.0` → `OnnxNliVerifier::new(config)?` 호출 + `Arc::new` wrap + `pipeline.with_verifier(v)`.
  - `config.rag.nli_threshold == 0.0` → verifier = None, 기존 path.
  - `OnnxNliVerifier::new` 실패 시 `bail!()` — user-facing crash 회피.
- `crates/kebab-rag/tests/multi_hop.rs`:
  - `common/mod.rs` 에 `MockNliVerifier { scores: NliScores }` helper.
  - `multi_hop_nli_pass_keeps_grounded` — entailment 0.9 → grounded=true, verification.nli_passed=true.
  - `multi_hop_nli_fail_refuses` — entailment 0.1 → refusal=NliVerificationFailed.
  - `multi_hop_nli_disabled_skip_verify` — threshold = 0.0 → verify skip, verification=None.
  - `multi_hop_nli_model_unavailable_refuses` — verifier Err → refusal=NliModelUnavailable.
  - `multi_hop_truncate_for_nli_preserves_hypothesis` — long premise + 짧은 hypothesis → hypothesis 그대로.
- `integrations/claude-code/kebab/SKILL.md`:
  - `mcp__kebab__ask` 절에 NLI 안내 한 줄:
    > `answer.v1.verification.nli_passed` 의미 (true = NLI 통과, false = `refusal_reason = nli_verification_failed`). threshold tuning 권장 (0.5 production, 0.9 strict). `nli_model_unavailable` refusal 시 user 의 `[rag] nli_threshold = 0.0` 임시 disable + network/disk 복구 후 재시도.

**Tests**: 5 신규 multi-hop tests + 기존 tests 회귀 0.

**검증**:
- `cargo test --workspace -j 1` — 모든 test 통과 + 신규 5 multi-hop pass.
- `cargo clippy --workspace --all-targets -j 1 -- -D warnings` clean.

**시간**: 3-4h.

## 6. PR-9d — Dogfood retest + HOTFIXES closure

**Goal**: PR-9c 머지 후 dogfood corpus 에서 S7 + S1 + S3 + S10 retest.

**Dependency**: PR-9c-2 머지 완료.

**Pre-run prereq (manual + subagent 양쪽 적용)** — spec §3 PR-9d:
- Ollama service running (`curl -s 127.0.0.1:11434/api/tags`).
- dogfood corpus 디렉토리 존재 (`/build/cache/dogfood-v018/queries/*.txt`).
- network reachable (hf-hub 280 MB NLI model first-run download 가능).
- free RAM ≥ 6 GB.
- release binary path: `/build/out/cargo-target/release/kebab` (CARGO_TARGET_DIR) 또는 `./target/release/kebab` (in-tree). 권장: `/build/out/cargo-target/release/kebab` (HOTFIXES 2026-05-25 fb-41 dogfood entry).

prereq 실패 시 *조기 abort* + 사용자 보고. partial dogfood 결과 commit 회피.

**Tests** (자동화 없음, manual run):
- `[rag] nli_threshold = 0.5` config (production 권장).
- S7 / S1 / S3 / S10 multi-hop ask → 각각 NLI score 측정 + grounded/refuse 확인.
- single-pass S7 (verification 없음) baseline 도 같이 측정.

**RAM peak protocol**:
```sh
# 시작 전 baseline
ps -o rss=,vsz=,comm= -p $(pgrep -f 'ollama|kebab')

# multi-hop ask 진행 중 1초 sampling (5분 cap)
while sleep 1; do ps -o rss=,comm= -p $(pgrep -f 'ollama|kebab') ; done > /tmp/ram-S<N>.log &
RAM_PID=$!

# kebab ask 실행
/build/out/cargo-target/release/kebab ask --multi-hop "<query>" --json

# sampling 종료 + peak 추출
kill $RAM_PID
awk '{sum+=$1} END {print sum/NR " avg KB"}' /tmp/ram-S<N>.log
awk '{ if ($1>max) max=$1 } END { print max " peak KB" }' /tmp/ram-S<N>.log
```
peak < 10 GB (16 GB 환경 OOM 없음) 확인.

**Files**:
- `tasks/HOTFIXES.md`:
  - "PR-9 closure (post-v0.18 dogfood retest)" sub-section 추가 — pre/post 결과 비교 표.
- `docs/dogfood/v0.18.0/` 신규 디렉토리 (round-2 P5 의 보존 path):
  - `SUMMARY.md` — sanitized dogfood 보고서 (원본 `/build/cache/dogfood-v018/results/SUMMARY.md` 의 repo 포함 가능 부분).
  - `s7-multihop-post-pr9.json` — S7 multi-hop NLI 결과 sample (refuse + nli_score).
  - `s1-multihop-post-pr9.json` — S1 multi-hop NLI 결과 sample (grounded + nli_score).
- `/build/cache/dogfood-v018/results/post-pr9/` (작업 디렉토리, repo 외):
  - 시나리오별 JSON dump + findings.md + RAM log.

**검증** — spec §7 PASS criteria 표 따름:
- S7: grounded=false, refusal=`nli_verification_failed`, nli_score < 0.3.
- S1: grounded=true, refusal=None, nli_score ≥ 0.6.
- S3 (EN): primary grounded=true 또는 acceptable degraded LlmSelfJudge.
- S10 (KR): primary refusal=`nli_verification_failed` 또는 acceptable degraded LlmSelfJudge.
- range 밖 시 threshold / model 재검토 (spec §6 iteration trigger).

**시간**: 4-6h (RAM 측정 + corpus 보존 + HOTFIXES + manual retest).

**Scope**: PR default. 작업자 선택 가능 (별 commit 가능, round-1 P3).

## 7. v0.18.0 cut PR (PR-9d 머지 후)

**Goal**: version bump + cascading docs + frozen design contract 갱신 + release tag.

**Dependency**: PR-9d 머지 완료.

**Same-commit / Same-PR** (CLAUDE.md "Release / binary version bump" rule):
- Cargo.toml version bump + tag = 같은 commit.
- frozen design §3.8 갱신 = 본 cut PR 안.
- gitea-release tag v0.18.0 = 본 PR 머지 commit 위 즉시.

**Merge strategy** (round-1 critic P5-NEW-M2): kebab 의 default merge commit 패턴은 `Merge pull request '...' (#N)` 형태 — bump commit 이 PR branch 안에 있고 main 의 HEAD = merge commit (별 SHA). CLAUDE.md "bump commit = release commit" rule strict 해석:
- **Option A 권장 — gitea-pr 의 squash merge** 사용 (`gitea-pr --merge-method squash` 또는 머지 UI 의 squash 옵션). 결과: main HEAD = bump 의 squash commit (single SHA). `gitea-release v0.18.0` tag 가 그 commit 위.
- Option B (대안): bump 의 *PR branch commit* 에 직접 tag (main 의 merge commit 과 다른 SHA, 그러나 release tag 는 PR branch SHA reference — gitea 에서 가능). audit trail 약간 약함.
- Option C: merge commit 자체에 *bump 내용 포함* (PR description = bump + cascading docs). gitea-pr 의 default merge commit message 가 bump 의 commit message 와 다른 자체 message — *bump 의 의도* 가 merge commit 에 inline 되지 않음. 권장 안 함.

본 cut PR 작업자가 **Option A (squash merge)** 채택. main HEAD = bump commit, tag = same SHA. CLAUDE.md rule strict 정합.

**R5-NEW-NIT-1 carry-over** (round-1 critic P5-NEW-M1): release notes draft (spec §5 line 681) 의 `9B+ 모델` 표현이 spec §5 step 8 line 651 의 `8B+ Q4 모델 (gemma4:e4b 8B / gemma2:9b 등)` 와 inconsistency 잔존 (cut PR 시점 final 작성 시 정정). cut PR 작업자가 spec §5 step 8 wording 일관 적용. spec round-5 NIT 자체는 spec 안에서 closure (R5-NEW-NIT-1 row of §9), 본 plan §7 가 *implementation reminder*.

**Files** (모두 한 PR, commit msg `chore(release): bump version 0.17.2 → 0.18.0 + cut fb-41 multi-hop` — round-2 critic R2-NIT-2 scope label):
1. `Cargo.toml` (workspace): `version = "0.17.2"` → `"0.18.0"`. `Cargo.lock` 자동 cascade.
2. `HANDOFF.md`:
   - 한 줄 요약 (P0~P9 + P10 + v0.18.0 fb-41 multi-hop ship).
   - 머지 후 결정 절에 fb-41 entry 단락 (PR-1~PR-9 + dogfood + NLI 한 문단).
3. `tasks/HOTFIXES.md`: 기존 fb-41 entry 들 `post-v0.18` anchor.
4. `tasks/INDEX.md`: fb-41 status `open` → `completed`. v0.18.0 release subheader.
5. `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md`:
   - §3.8 RAG 의 multi-hop sub-section 추가 (본 finalize spec §1-§3 요약 verbatim).
   - §9 versioning cascade 표에 (선택) `nli_model_version` row.
6. `docs/superpowers/specs/2026-05-25-p9-fb-41-finalize-spec.md`:
   - `status: approved-by-team` → `completed`.
7. `docs/superpowers/plans/2026-05-25-p9-fb-41-finalize-plan.md`:
   - `status: open` → `completed`.
8. `integrations/claude-code/kebab/SKILL.md`:
   - v0.18.0 release notes link 한 줄.
9. `README.md`:
   - `kebab ask --multi-hop` + NLI 옵션 안내 한 단락 (model first-run download cost, RAM 권장).
   - binary path confusion 한 줄 (`/build/out/cargo-target/release/kebab` 명시).
10. `docs/SMOKE.md`:
    - NLI 옵션 활성화 절차 (`[rag] nli_threshold = 0.5`).
    - first-run model download 안내 (~280 MB).
    - RAM 권장 (gemma3:4b 기준 ~5-6 GB; 8B+ Q4 모델 추정 ~10 GB / 16 GB 경계).

**gitea-release** (cut PR 작업자 결정):
```sh
# Option A: --auto-notes 만 (gitea-ops skill 가 PR 시리즈 자동 list 생성)
gitea-release v0.18.0 --auto-notes

# Option B: --notes 만 (spec §5 release notes draft inline)
gitea-release v0.18.0 --notes-file release-notes-v0.18.0.md

# Option C: 둘 다 (gitea-ops 가 동시 명시 시 동작 — 사전 확인 필요)
gitea-release v0.18.0 --auto-notes --notes "fb-41 multi-hop RAG + NLI ship..."
```

cut PR 작업자가 `gitea-release --help` 또는 `~/.claude/skills/gitea-ops/SKILL.md` 확인 후 Option 선택 (round-1 critic P5-NIT-1). spec §5 의 release notes draft 가 *content* — 어느 path 로 input 할지가 결정.

권장: **Option A (--auto-notes)** + PR description 에 spec §5 release notes draft inline — gitea-release 가 PR description 의 release notes 절을 carry. 단순 + audit trail.

**검증**:
- `cargo build --release` 통과.
- `gitea-pr-status` gate passed.
- 머지 후 binary smoke test (cargo run --release).

**시간**: 2-3h (round-1 architect N4 + critic P5-NIT-4 반영 — frozen design §3.8 verbatim + 10 files cascading + release notes final + review iteration 가능성 반영).

## 8. 시간 추정 합산

| Sub-PR | 작업 (h) | + review iteration (h) | wall time (h) |
|---|---|---|---|
| PR-9a | 2-3 | 1-2 | 3-5 |
| PR-9b | 8-12 | 2-3 | 10-15 |
| PR-9c-1 | 2-3 | 1-2 | 3-5 |
| PR-9c-2 | 3-4 | 1-2 | 4-6 |
| PR-9d | 4-6 | 1-2 | 5-8 |
| cut PR | 2-3 | 1-2 | 3-5 |
| **Total** | **21-31h** | **+7-13h** | **28-44h** |

**round-2 plan critic M2 closure** — 작업 시간 vs wall time 명시적 분리:

- **작업 (h)** = 순수 implementation / dogfood / docs cascade 시간 (review feedback 반영 작업 별도).
- **review iteration (h)** = `gitea-pr-review` 회차당 1-2h × 평균 1-1.5 회차 추정 (HOTFIXES 평균 의거). 회차 0 (즉시 APPROVE) 도 가능 — 작업자 quality 의존.
- **wall time (h)** = 작업 + review iteration. 사용자 / stakeholder 의 *ship expectation* baseline.

cumulative 정정 trace: round-1 14-20h → round-2 14-22h → plan v2 20-30h → plan v3 21-31h (작업) / 28-44h (wall time, plan v4 round-2 critic M2 신설).

## 9. /subagent-driven-development 의 task list

`plan` 통과 + OMC team APPROVE 후 다음 task list 로 subagent dispatch:

1. **Task PR-9a**: kebab-nli crate skeleton — **plan §2 + spec §3 PR-9a + spec §2.1~§2.2.4 참조**. branch `feat/fb-41-pr-9a-kebab-nli-crate`. pre-flight (`curl -I` + tokenizers probe) 결과 PR description 첨부. 검증 + PR + 회차 리뷰 루프 + 머지.
2. **Task PR-9b**: OnnxNliVerifier inference — **plan §3 + spec §3 PR-9b + spec §2.2.2~§2.2.4 참조**. branch `feat/fb-41-pr-9b-onnx-nli-inference`. manual `--ignored` smoke 결과 PR description 첨부.
3. **Task PR-9c-1**: core types + wire — **plan §4 + spec §3 PR-9c-1 + spec §2.4~§2.6 참조**. branch `feat/fb-41-pr-9c-1-core-types-wire`. `docs/ARCHITECTURE.md` 갱신 포함.
4. **Task PR-9c-2**: pipeline integration + mock test + SKILL.md — **plan §5 + spec §3 PR-9c-2 + spec §2.3 참조**. branch `feat/fb-41-pr-9c-2-pipeline-integration`.
5. **Task PR-9d**: dogfood retest + HOTFIXES + dogfood corpus 보존 — **plan §6 + spec §3 PR-9d + spec §7 PASS criteria 표 참조**. branch `feat/fb-41-pr-9d-dogfood-retest`. pre-run prereq 검증 후 시작. **Environment (round-2 critic M3)**: *user machine 에서만 dispatch 가능* — Ollama service running + dogfood corpus 디렉토리 존재 + network reachable + free RAM ≥ 6 GB + release binary path 의존. isolated docker / ephemeral CI container 환경은 모두 부재 → dispatch 시 즉시 abort. autonomous subagent provisioning (sudo Ollama install + corpus mirror) 은 v0.19+ candidate.
6. **Task cut PR**: version bump + cascading docs — **plan §7 + spec §5 + spec R5-NEW-NIT-1 carry-over 참조**. branch `chore/v0.18.0-cut`. *gitea-pr squash merge* + `gitea-release v0.18.0` tag 머지 commit 위.

각 task 는 *self-contained* — 별 subagent dispatch 가능. dependency 는 *이전 task 의 main 머지* — subagent 가 다음 task 시작 전 `git pull` 로 sync. **순차 only** — speculative pre-work 권장 안 함 (review 부담 + rebase 위험). 특히 **PR-9c-2 는 PR-9c-1 의 review iteration 완료 + 머지 후 시작** (round-2 critic N4) — 중간 schema change 시 9c-2 의 mock test 의 schema validation expectation 변경 위험. `TaskUpdate` 의 `addBlockedBy` chain 으로 race 회피 (round-1 planner informational). **active subagent ≤ 1 임의 timestamp** — RAM 압박 회피 + user memory `feedback_serial_build_only` policy 정합.

각 subagent 는 다음을 책임:
- branch 생성 + 구현 + tests + cargo test/clippy 검증 (16 GB RAM 직렬 only, user memory `feedback_serial_build_only` 적용).
- gitea-pr 생성 + 리뷰 루프 (gitea-pr-review 회차) + APPROVE 후 머지.
- 머지 후 main checkout + pull + branch cleanup (`git branch -d` + worktree 사용 시 `git worktree remove`).
- `cargo clean` 권장 (CLAUDE.md routinely after merged PR rule, 92GB→0GB 복구).
- `TaskUpdate(status='completed')` 호출 + team-lead 에게 `SendMessage` 으로 다음 task 시작 신호 (또는 사용자 manual dispatch).

## 10. Self-review notes

- **PR-9 의 ONNX integration** 가 *새 dep chain* (ort + tokenizers + hf-hub) 도입 — 첫 사용 안정화 필요. PR-9a 의 pre-flight 가 *모든 위험 검증*. PR-9b 의 `#[ignore]` test manual smoke 가 *production binary 실제 동작* 검증.
- **multi-hop NLI 의 latency 추가** — current multi-hop synthesize 158s + NLI ~50ms ≈ 158s. negligible.
- **Model first-run download (~280 MB)** — 사용자 도그푸딩 환경 (CPU only) 의 disk + download bandwidth 1회 비용. README + SMOKE 안내. fail-closed download failure 정책.
- **`RagPipeline::new` 시그니처 widening — Option B (builder) 결정** — 18+ existing call sites 무영향.
- **frozen design contract §3.8 갱신 timing — v0.18.0 cut PR 안** — PR-9c 가 contract 변경 안 함.
- **kebab-nli 의 trait + impl 동일 crate** — v0.18 scope = adapter 1개. v0.19+ 에 multi-adapter 등장 시 `kebab-nli-onnx` 분리.
- **dogfood corpus 보존** — `docs/dogfood/v0.18.0/` 신규 dir + sanitized SUMMARY + sample JSON.
- **RAM cold-start 측정** — PR-9d 의 PASS criteria 에 포함, release notes 의 권장 RAM 한 줄.
- **ort version pin** — `workspace.dependencies.ort = "=2.0.0-rc.9"`.

### Plan-specific self-review (round-1 critic P5-NIT-3 반영)

execution / coordination 측면의 추가 self-review notes:

- **Subagent 간 race 회피**: `TaskUpdate.addBlockedBy` chain 필수 적용 — PR-9b 의 task 가 PR-9a task 의 머지 완료에 blockedBy. PR-9c-1 → 9c-2 → 9d → cut PR 동일 chain.
- **PR-9c-1 wire schema baseline for 9c-2**: PR-9c-1 의 `answer.schema.json` / `error.schema.json` 변경이 9c-2 의 mock test 의 schema validation baseline. PR-9c-1 의 review iteration 결과 schema 변경 시 9c-2 시작 전 *main pull* + spec/plan re-check 필수.
- **`#[allow(dead_code)]` for verifier field in PR-9c-1** (round-1 architect N1): PR-9c-1 의 `RagPipeline.verifier` field 가 *declared 되었지만 read by nothing* 인 interim 시기 (9c-2 머지 전) — `cargo clippy -- -D warnings` fail 위험. PR-9c-1 의 field 에 임시 `#[allow(dead_code)]` 또는 `Debug` derive 의 trivial field access. PR-9c-2 가 attribute 제거 + builder 의 `with_verifier` 의 사용 path 활성화.
- **OnnxNliVerifier::new 의 lazy stamp semantics** (round-1 architect N2): spec §2.2.2 의 OnceLock pattern — `new()` 자체는 cache_dir create 같은 *early error* 만 잡음. download / inference 실패는 *runtime path* 의 `refuse_nli_model_unavailable` 가 처리. 작업자가 *eager download 시도* (lazy 위반) 회피.
- **`truncate_for_nli` placement** (round-1 architect N3): module-level `pub fn` in `kebab_rag::pipeline`. 회귀 핀 test = `crates/kebab-rag/tests/multi_hop.rs` 의 `multi_hop_truncate_for_nli_preserves_hypothesis` (§5).
- **First-run download progress indicator 검증** (round-1 architect N5): PR-9d 의 first-run NLI model download 시 stderr 에 `kebab-nli: downloading model.onnx (280 MB)...` progress emit 확인. non-`--json` mode 만 progress emit. PR-9d 의 검증 절에 명시 안 됐지만 작업자가 stderr 출력 확인 + HOTFIXES PR-9 closure 절에 *progress 확인 결과* 한 줄 명시 권장.
- **Parallel execution opportunity (round-1 critic Open Question)**: PR-9b (8-12h, kebab-nli crate-internal) 동안 PR-9c-1 의 *kebab-nli 의존 없는 부분* (RefusalReason variant + wire schema) preparation 가능 — 시간 단축 4-6h. **권장 안 함** (review iteration 비용 ↑ + branch rebase risk). plan v3 는 *sequential only* 명시. 단, 작업자가 *speculative pre-work* 결정 시 `kebab-rag` 의 `kebab-nli` 의존 추가는 9b 머지 후 lock.
- **PR-9d binary path 일관성** (round-1 critic Open Question + dogfood SUMMARY §부수 발견 closure): subagent task 가 `cargo build --release` 후 `/build/out/cargo-target/release/kebab` 사용 (CARGO_TARGET_DIR env 설정 환경). cut PR 의 README 갱신 (§7 step 9) 가 *user-facing* path confusion closure.
- **Rollback path** (round-1 critic What's missing): PR-9d dogfood retest PASS criteria *catastrophic fail* (NLI library bug 등) 시 PR-9 revert path — `git revert` PR-9c-2 → PR-9c-1 → PR-9b → PR-9a 의 *reverse sequential*. `[rag] nli_threshold = 0.0` config knob 으로 graceful disable 가 더 가벼운 first-response. spec §6 의 threshold iteration trigger 와 분리 (혼동 회피).

## 11. Spec-driven 변경 trace

본 plan v4 는 spec v5 (review_round=5, approved-by-team) 의 모든 결정 반영 + plan-review round-1/round-2 의 issues closure. spec 의 §9 closure matrix (round 1-4) + plan v1/v2/v3 의 점진적 갱신 baseline. plan v2 → v4 갱신 사항 (round-2 critic R2-NIT-1 wording 정정):

- PR-9c 분할 = 9c-1 + 9c-2 별 PR (round-2 P1).
- 시간 추정 14-20h → 20-30h cumulative 정정 (round-2 P2 + round-3 R3-NEW-N1).
- PR-9d 의 RAM 측정 protocol + pre-run prereq + dogfood corpus 보존 (round-2 P5/P6 + round-3 R3-NEW-N3).
- cut PR step 명시 + same-commit rule (round-2 M7 + round-3 R3-NEW-N2).
- 시그니처 widening = Option B (round-2 NEW-M2).
- truncate_for_nli signature `(String, bool)` (round-2 NEW-N1).
- `RefusalReason::NliVerificationFailed` + `NliModelUnavailable` wire 통일 (round-2 ISSUE-1 + R3-NEW-N3).
- model ID Xenova/... config default 확정 (round-1 A1 / D5).
- `nli_threshold = 0.0` single gate (round-1 A3 / D3).
- pre-flight tokenizers features 검증 (round-2 NEW-M1).
- §7 cross-ref single source of truth (round-4 R4-NEW-M1 + R4-NEW-N1).

### Plan-review round-1 closure (post-spec-v5 plan-level review)

| reviewer | round-1 plan issue | plan v3 resolution |
|---|---|---|
| document-specialist | ISSUE-1 ARCHITECTURE.md missing | §4 PR-9c-1 Files 에 `docs/ARCHITECTURE.md` 추가 — Mermaid `nli` 노드 + 4 edges + 디렉토리 트리. |
| critic | P5-NEW-M1 R5-NEW-NIT-1 carry-over | §7 cut PR 에 "R5-NEW-NIT-1 carry-over" 절 — release notes draft 의 `9B+ 모델` → `8B+ Q4 모델` cut PR 작업자 reminder. |
| critic | P5-NEW-M2 merge strategy | §7 — Option A (gitea-pr squash merge) 권장 명시. bump commit = main HEAD = release tag SHA. CLAUDE.md same-commit rule strict 정합. |
| critic | P5-NIT-1 gitea-release flag combo | §7 — `--auto-notes` / `--notes-file` / 둘 다 의 Option A/B/C 명시 + 사전 `gitea-release --help` 확인 + Option A 권장. |
| critic | P5-NIT-2 §9 spec § reference | §9 — 각 task description 에 `plan §X + spec §Y` cross-ref inline. |
| critic | P5-NIT-3 plan-specific self-review | §10 — Plan-specific self-review notes 절 추가 (race avoidance + dead_code attr + lazy semantics + parallel opportunity + rollback path 등 8 항목). |
| critic | P5-NIT-4 cut PR 시간 | §7 + §8 — 시간 1-2h → 2-3h. 합산 20-30h → 21-31h. |
| architect | N1 dead_code attr | §10 plan-specific self-review notes 의 `#[allow(dead_code)]` 한 줄 명시. |
| architect | N2 OnnxNliVerifier::new lazy semantics | §10 — early error vs runtime error 명시. |
| architect | N3 truncate_for_nli placement | §10 — module-level `pub fn` + test 위치 cross-ref. |
| architect | N4 cut PR 시간 (frozen design §3.8 cost) | §7 시간 1-2h → 2-3h + §8 합산 정정 (critic P5-NIT-4 와 동일 fix). |
| architect | N5 download progress 검증 | §10 self-review — PR-9d 작업자가 stderr progress 확인 + HOTFIXES 한 줄 명시 권장. |
| planner | round-2 nit (4개) | spec v5 와 plan v2 가 closure (round-2 closure matrix). round-1 plan-level 신규 nit 0개 (re-confirm APPROVE). |

### Plan-review round-2 closure (post-plan-v3 deep ADVERSARIAL review)

| reviewer | round-2 plan issue | severity | plan v4 resolution |
|---|---|---|---|
| critic | M1 PR-9c-1 dead code clippy fail risk | MINOR | §4 PR-9c-1 의 `RagPipeline.verifier` field 절에 `#[allow(dead_code)]` 명시 + PR-9c-2 hook 추가 시 제거 trace. |
| critic | M2 시간 추정 review iteration 미포함 | MINOR | §8 시간 표 *작업 vs review iteration vs wall time* 분리 — 합산 21-31h (작업) / 28-44h (wall time). |
| critic | M3 PR-9d subagent dispatch environment | MINOR | §9 PR-9d 항목 Environment 명시 — user machine only (Ollama / corpus / network / RAM / binary path 의존), isolated container 부적합. |
| critic | N4 9c-1/9c-2 sequential strictness | NIT | §9 — "PR-9c-2 는 PR-9c-1 의 review iteration 완료 + 머지 후 시작" 명시. |
| document-specialist | NIT-1 nli→core edge 확인 권장 | NIT | §4 ARCHITECTURE.md 절 — `nli --> core` edge 는 PR-9a 머지 후 final `kebab-nli/Cargo.toml` deps 확인 결정. |
| critic | N1 spec §2.1.1 alternative models cross-ref | NIT (옵션) | 미반영 — PR-9d dogfood retest iteration trigger 발동 시 작업자가 spec 직접 cross-ref 권장. |
| critic | N2 PR-9b fallback Optimum self-export | NIT (옵션) | 미반영 — spec §3 PR-9b 의 fallback 명시 충분. |
| critic | N3 PR description template | NIT (옵션) | 미반영 — 각 plan 절이 *de facto* template 역할. |

### Plan-review round-2 (light pass on plan v3) closure

plan v3 SendMessage 후 critic round-2 light pass 가 1 actionable MINOR (R2-NEW-M1 §0 vs §8 시간 mismatch) + 1 verification MINOR (R2-NEW-M4 spec status — *invalid finding*: spec frontmatter 실제 `status: approved-by-team`) + 4 NIT (R2-NIT-1~4) 발견.

| reviewer | round-2 light issue | severity | plan v4 resolution |
|---|---|---|---|
| critic | R2-NEW-M1 §0 vs §8 시간 cumulative mismatch | MINOR | §0 line 22 "총 추정 시간 19-28h" → "작업 21-31h / wall time 28-44h (§8 cumulative trace + plan v4 round-2 critic M2 분리 참조)". |
| critic | R2-NEW-M4 spec status transition | MINOR (invalid) | spec frontmatter 확인 — `status: approved-by-team` (line 6). plan §7 step 6 `approved-by-team → completed` transition 정확. *no edit*. |
| critic | R2-NIT-1 "본 plan v2" wording | NIT | §11 의 plan-version 헤더 — "본 plan v2 는" → "본 plan v4 는 ... + plan-review round-1/round-2 의 issues closure". |
| critic | R2-NIT-2 cut PR commit msg scope | NIT | §7 Files header — `chore:` → `chore(release):` scope label. |
| critic | R2-NIT-3 ARCHITECTURE.md `app --> nli` edge timing | NIT | §4 ARCHITECTURE.md 절 — *forward-looking final state* 명시 + PR-9c-1 시점 직접 의존 (`rag --> nli` + `nli --> config`) 분리 + 권장 forward-looking. |
| critic | R2-NIT-4 App::new caller cascading | NIT | §5 PR-9c-2 의 kebab-app 항목 — **실제 constructor `App::open_with_config` (kebab-app/src/app.rs:187, 이미 Result return)**. PR-9c-2 = *body 추가만*, caller cascading 0. |
