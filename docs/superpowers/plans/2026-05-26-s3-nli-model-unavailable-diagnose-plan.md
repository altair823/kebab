---
title: "S3 NLI unavailable implementation plan v1 — hypothesis-side char budget + token-count fallback retry"
date: 2026-05-26
task_id: s3-nli-unavailable
status: open
target_version: 0.18.1
design: ../specs/2026-05-26-s3-nli-model-unavailable-diagnose-spec.md
---

# S3 NLI unavailable — implementation plan v1

## §0. 개요

S3 dogfood query (`"Why does kebab combine multilingual-e5, LanceDB, and RRF together?"`) 가 `nli_model_unavailable` 로 일관 fail 하는 root cause 는 mDeBERTa-v3 tokenizer 의 `TruncationStrategy::OnlyFirst` + LLM 의 949-token (4564-char) 장문 답변 — *hypothesis 단독이 max_length cap (512) 을 초과* 하여 premise 를 0 까지 잘라도 fit 시킬 수 없어 tokenizer 가 `SequenceTooShortToTruncate` raise. 본 plan 은 spec §3 의 **Option A** (KR safety + graceful fallback) 를 단일 PR 로 구현 — `kebab-nli::NliVerifier` trait 에 `hypothesis_token_count` probe-only API 1 개 추가 + `OnnxNliVerifier` 가 *trait impl 블록 안에서* real tokenizer 로 override (RC1-residual: inherent impl 은 vtable 미등록 → silent NO-OP) + `kebab-rag::pipeline` 에 char-budget retry helper (`truncate_hypothesis_for_nli_with_budget`) + step 8.5 hook 의 callsite 수정. 총 3 production files + 1 test scaffold + 2 test files + 1 doc entry (round-1 plan critic MAJOR #2 정정). Wire schema 변경 없음, `Cargo.toml` version bump 불필요 — test/diagnostic fix.

---

## §1. 단일 PR 변경 file list

| 파일 | 종류 | 변경 요지 |
|---|---|---|
| `crates/kebab-nli/src/lib.rs` | production | `NliVerifier` trait 에 `hypothesis_token_count(&self, &str) -> anyhow::Result<usize>` method 추가 (default impl `Ok(0)` — backward-compat). |
| `crates/kebab-nli/src/onnx.rs` | production | inherent `OnnxNliVerifier::HYPOTHESIS_TOKEN_BUDGET = 256` const + **trait impl block 안** 에서 `hypothesis_token_count` override (real `tokenizer.encode` probe). |
| `crates/kebab-rag/src/pipeline.rs` | production | `MAX_NLI_HYPOTHESIS_CHARS_INITIAL = 1200` + `MAX_NLI_HYPOTHESIS_CHARS_MIN = 150` const + `truncate_chars(s, budget)` pure-fn + `truncate_hypothesis_for_nli_with_budget(verifier, hypothesis)` retry helper + step 8.5 callsite (line 1041 근방) hypothesis-side hook 추가 + `tracing::debug` log. |
| `crates/kebab-rag/tests/common/mod.rs` | test scaffold | `SpyNliVerifier` helper (closure-based 2-arg constructor: `score_fn` + `token_count_fn` + capture `Mutex<Vec<String>>`). 기존 `MockNliVerifier` 와 sibling 으로 공존. |
| `crates/kebab-rag/tests/multi_hop_nli_truncate.rs` | 신규 test | spec §5.3 의 3 mock multi-hop tests (EN happy / KR retry / unrelenting fallback). |
| `crates/kebab-nli/tests/inference.rs` | 기존 test 확장 | spec §5.1 의 2 신규 `#[ignore]` tests (long EN err pin + token_count 의 bounded range pin). |
| `tasks/HOTFIXES.md` | doc | 신규 dated entry `## 2026-05-26 — S3 NLI unavailable — hypothesis truncate + token-count fallback` (Symptom / Root cause / Action / Amends 4-block, HOTFIX 번호 미부여 — sibling fb-41 layer follow-up). |

총 **7 files** (4 production + 2 test + 1 doc). `truncate_chars` pure-fn 의 4 boundary test 는 `pipeline.rs` 내부 `#[cfg(test)] mod tests` 또는 `multi_hop_nli_truncate.rs` 의 별 `mod` 로 흡수 — 신규 file 미추가.

---

## §2. 구현 step list

Subagent 가 *spec §3 + §5 의 코드 블록을 verbatim 으로* 따라 12 step. 각 step 의 acceptance check 도 명시.

1. **`kebab-nli/src/lib.rs` — trait 확장.**
   - `NliVerifier` trait 에 `hypothesis_token_count(&self, _hypothesis: &str) -> anyhow::Result<usize> { Ok(0) }` default impl 추가.
   - Doc comment: spec §3 의 "Default impl 반환 0 — backward-compat, OnnxNliVerifier 는 *trait impl 블록 안에서* override" 명시.
   - Acceptance: `cargo check -p kebab-nli` 통과. 기존 `MockNliVerifier` (rag tests/common/mod.rs) 가 default impl 상속 → 명시 override 안 해도 compile.

2. **`kebab-nli/src/onnx.rs` — inherent const 추가.**
   - `impl OnnxNliVerifier { pub const HYPOTHESIS_TOKEN_BUDGET: usize = 256; }` (기존 `MAX_TOKENS = 512` 와 sibling 위치).
   - Doc comment: "= MAX_TOKENS (512) - 3 special tokens reserved (CLS, SEP, SEP) - 253 premise room".
   - Acceptance: `cargo check -p kebab-nli` 통과.

3. **`kebab-nli/src/onnx.rs` — trait impl 안 override (RC1-residual 핵심).**
   - `impl NliVerifier for OnnxNliVerifier { ... fn score(...) ... fn hypothesis_token_count(&self, hypothesis: &str) -> anyhow::Result<usize> { let (_session, tokenizer) = self.ensure_loaded()?; let enc = tokenizer.encode(hypothesis, /*add_special_tokens=*/ false).map_err(|e| anyhow!("kebab-nli: tokenizer.encode (probe) failed: {e}"))?; Ok(enc.get_ids().len()) } }`
   - **반드시 trait impl 블록 안 — inherent `impl OnnxNliVerifier {}` 안에 두면 vtable 미등록 → trait dispatch 시 default `Ok(0)` 호출 → production silent NO-OP** (round-3 critic RC1-residual closure).
   - Acceptance: `cargo check -p kebab-nli` 통과. spec §5.1 Test 7 (`hypothesis_token_count_dispatches_correctly_via_dyn_trait`) 가 vtable 통해 호출 검증.

4. **`kebab-rag/src/pipeline.rs` — consts 추가.**
   - 기존 `MAX_NLI_PREMISE_CHARS` (line 1803 근방) 직후에 `pub const MAX_NLI_HYPOTHESIS_CHARS_INITIAL: usize = 1200;` + `pub const MAX_NLI_HYPOTHESIS_CHARS_MIN: usize = 150;`.
   - Doc comment: spec §3 의 KR safety + retry rationale 명시 + "round-1 critic H3 closure" cross-link.
   - Acceptance: `cargo check -p kebab-rag` 통과.

5. **`kebab-rag/src/pipeline.rs` — `truncate_chars(s, budget)` pure-fn sub-helper.**
   - `pub(crate) fn truncate_chars(s: &str, budget: usize) -> (String, bool) { if s.chars().count() <= budget { (s.to_string(), false) } else { let truncated: String = s.chars().take(budget).collect(); (truncated, true) } }`
   - 기존 `truncate_for_nli` (line 1810 근방) 와 sibling 위치.
   - Acceptance: `cargo check -p kebab-rag` 통과. spec §5.2 의 4 boundary tests 가 이 helper 호출.

6. **`kebab-rag/src/pipeline.rs` — `truncate_hypothesis_for_nli_with_budget` retry helper.**
   - spec §3 의 코드 블록 verbatim:
     ```rust
     pub fn truncate_hypothesis_for_nli_with_budget(
         verifier: &(dyn kebab_nli::NliVerifier + 'static),
         hypothesis: &str,
     ) -> anyhow::Result<(String, bool)> {
         let original_chars = hypothesis.chars().count();
         let mut budget = MAX_NLI_HYPOTHESIS_CHARS_INITIAL;
         let mut was_truncated = false;
         loop {
             let candidate: String = if original_chars <= budget {
                 hypothesis.to_string()
             } else {
                 was_truncated = true;
                 hypothesis.chars().take(budget).collect()
             };
             let token_count = verifier
                 .hypothesis_token_count(&candidate)
                 .with_context(|| "kebab-rag: hypothesis token-count probe failed")?;
             if token_count <= kebab_nli::OnnxNliVerifier::HYPOTHESIS_TOKEN_BUDGET {
                 return Ok((candidate, was_truncated));
             }
             budget = budget / 2;
             if budget < MAX_NLI_HYPOTHESIS_CHARS_MIN {
                 anyhow::bail!(
                     "kebab-rag: hypothesis remains over token budget after retry (original {original_chars} chars, last budget {} chars, tokens {token_count} > {})",
                     budget * 2,
                     kebab_nli::OnnxNliVerifier::HYPOTHESIS_TOKEN_BUDGET,
                 );
             }
         }
     }
     ```
   - Acceptance: `cargo check -p kebab-rag` 통과.

7. **`kebab-rag/src/pipeline.rs` — step 8.5 hook 의 callsite 수정** (round-1 plan critic CRITICAL #1 closure: `?` propagation 이 `ask_multi_hop` 의 `Err(anyhow::Error)` 반환 → wire `error.v1` 로 빠짐 → graceful fallback 약속 위반. 기존 `v.score()` Err 분기 (`return self.refuse_nli_model_unavailable`) 와 *대칭* 으로 explicit `match` + `return refuse_*` 패턴).

   기존 callsite (`if was_truncated { debug! }` block 직후, `match v.score(...)` 직전) 다음과 같이 갱신:

   ```rust
   let (truncated_premise, premise_was_truncated) = truncate_for_nli(&packed_text);
   if premise_was_truncated {
       tracing::debug!(target: "kebab-rag", "NLI premise truncated to MAX_NLI_PREMISE_CHARS");
   }
   let (truncated_hypothesis, hypothesis_was_truncated) =
       match truncate_hypothesis_for_nli_with_budget(v.as_ref(), &acc) {
           Ok(x) => x,
           Err(e) => {
               tracing::warn!(
                   target: "kebab-rag",
                   error = %e,
                   "NLI hypothesis budget retry exhausted; refusing with NliModelUnavailable"
               );
               return self.refuse_nli_model_unavailable(query, &opts, hops, started);
           }
       };
   if hypothesis_was_truncated {
       tracing::debug!(
           target: "kebab-rag",
           original_chars = acc.chars().count(),
           "NLI hypothesis truncated to MAX_NLI_HYPOTHESIS_CHARS"
       );
   }
   match v.score(&truncated_premise, &truncated_hypothesis) {
       // ... 기존 Ok/Err 분기 그대로 (Err 도 동일하게 refuse_nli_model_unavailable)
   }
   ```

   - `v.score(&truncated_premise, &acc)` → `v.score(&truncated_premise, &truncated_hypothesis)`.
   - **`?` 사용 금지** — wire `answer.v1 + NliModelUnavailable refusal` 유지 보장 (graceful fallback).
   - Acceptance: `cargo check -p kebab-rag` 통과. 기존 `multi_hop_nli_model_unavailable_refuses` test 등 여전히 PASS. **§5.3 test #3 (`unrelenting_token_overflow_falls_through_to_unavailable`) 의 `.unwrap()` 이 panic 안 함** (Ok(Answer{refusal}) unwrap).

8. **`kebab-rag/tests/common/mod.rs` — `SpyNliVerifier` helper 추가.**
   - spec §5.3 의 코드 블록 verbatim:
     ```rust
     use std::sync::{Arc, Mutex};
     pub struct SpyNliVerifier {
         pub score_fn: Arc<dyn Fn(&str, &str) -> anyhow::Result<NliScores> + Send + Sync>,
         pub hypothesis_token_count_fn:
             Arc<dyn Fn(&str) -> anyhow::Result<usize> + Send + Sync>,
         pub received_premises: Mutex<Vec<String>>,
         pub received_hypotheses: Mutex<Vec<String>>,
     }
     impl SpyNliVerifier {
         pub fn new<F, G>(score_fn: F, token_count_fn: G) -> Arc<Self>
         where
             F: Fn(&str, &str) -> anyhow::Result<NliScores> + Send + Sync + 'static,
             G: Fn(&str) -> anyhow::Result<usize> + Send + Sync + 'static,
         {
             Arc::new(Self {
                 score_fn: Arc::new(score_fn),
                 hypothesis_token_count_fn: Arc::new(token_count_fn),
                 received_premises: Mutex::new(Vec::new()),
                 received_hypotheses: Mutex::new(Vec::new()),
             })
         }
     }
     impl NliVerifier for SpyNliVerifier {
         fn score(&self, premise: &str, hypothesis: &str) -> anyhow::Result<NliScores> {
             self.received_premises.lock().unwrap().push(premise.to_string());
             self.received_hypotheses.lock().unwrap().push(hypothesis.to_string());
             (self.score_fn)(premise, hypothesis)
         }
         fn hypothesis_token_count(&self, hypothesis: &str) -> anyhow::Result<usize> {
             (self.hypothesis_token_count_fn)(hypothesis)
         }
     }
     ```
   - 기존 `MockNliVerifier` 와 sibling, 둘 다 살려둠 (서로 다른 test pattern).
   - **Option B (권장) — inline pattern, helper 작성 안 함** (round-1 plan critic MAJOR #3 closure: 기존 `build_test_pipeline_*` builder 부재 (empirical grep 확인), helper 1회용 + 매 test 마다 long-answer 길이/언어 다름 → inline 이 더 명확). 각 §5.3 의 3 test 안에서 직접 `RagEnv::new()` + `ScriptedRetriever::new(...)` + `ScriptedLm::new(vec!["[\"q1\"]", "[]", &"lorem ".repeat(1000)])` (또는 `&"한국어 ".repeat(N)`) + `RagPipeline::new(...).with_verifier(verifier)` 패턴 — 기존 `crates/kebab-rag/tests/multi_hop.rs` 의 happy-path test 의 inline pattern 그대로 따름. ScriptedLm 응답 시퀀스: decompose JSON → decide JSON → synthesize long-answer.
   - Acceptance: `cargo check -p kebab-rag --tests` 통과.

9. **`kebab-nli/tests/inference.rs` — §5.1 의 2 신규 `#[ignore]` tests.**
   - `score_long_en_hypothesis_returns_err_without_pipeline_truncation` — `"lorem ipsum ".repeat(500)` (~6000 chars) 가 OnlyFirst 에서 err 반환 + 메시지 `"Truncation error"` 또는 `"too short to respect"` contains assertion.
   - `hypothesis_token_count_dispatches_correctly_via_dyn_trait` — **vtable dispatch 검증** (`let v_dyn: &dyn NliVerifier = &v; v_dyn.hypothesis_token_count(...)`) — concrete type 호출이 아닌 dyn dispatch 로 RC1-residual silent NO-OP regression pin. inherent-only 배치 시 default `Ok(0)` 반환 → `assert!(en_count > 0)` 실패. trait impl block 배치 시 EN bounded range (`0 < count < 20`) + KR bounded range (`0 < count < 30`) 양쪽 통과.
   - 둘 다 `#[ignore]` (network 요구 — model download).
   - Acceptance: `cargo test -p kebab-nli -j 1` 통과 (ignored 는 default skip 으로 GREEN). `cargo test -p kebab-nli --test inference -- --ignored --test-threads=1` 로 manual 실행 시 PASS (사용자 smoke).

10. **`crates/kebab-rag/tests/multi_hop_nli_truncate.rs` (신규) — §5.3 의 3 mock tests.**
    - `long_en_synth_answer_truncated_before_nli_call` — EN 5000-char synth answer → `SpyNliVerifier` 가 token_count `Ok(100)` 반환 (budget 안) → retry 0회 → hypothesis 가 정확히 1200 chars 로 truncate + Right direction pin (`assert_eq!(hyp.as_str(), input_first_1200)`).
    - `long_kr_synth_answer_retries_with_smaller_budget` — `token_count_call_count` Arc<Mutex<usize>> spy, token_count_fn 이 chars > 1000 → 900 tokens, > 500 → 450, else → 220. 2500-char KR-sim answer → retry >= 3 회 + 최종 hypothesis <= 300 chars + `refusal_reason = None` pin.
    - `unrelenting_token_overflow_falls_through_to_unavailable` — token_count_fn 이 `Ok(9_999)` 무조건 반환 → retry 소진 → `refusal_reason = Some(RefusalReason::NliModelUnavailable)` pin (regression 0 guarantee). score_fn 은 `unreachable!()` 로 — 호출되면 안 됨.
    - Acceptance: `cargo test -p kebab-rag --test multi_hop_nli_truncate -j 1` GREEN.

11. **`crates/kebab-rag/src/pipeline.rs` 의 `#[cfg(test)] mod tests` block — §5.2 의 4 pure-fn boundary tests** (round-1 plan critic MAJOR #4 closure: `truncate_chars` 가 `pub(crate)` 라 integration test 파일에서 접근 불가 → 동일 crate 의 `#[cfg(test)] mod tests` 필수).
    - `truncate_chars` 의 4 boundary:
      - 입력 <= budget → identity, `was_truncated=false`.
      - 입력 > budget → 정확히 budget chars, `was_truncated=true`.
      - empty 입력 → identity (budget 무관).
      - KR 한글 입력 (codepoint, not byte) → `chars().count()` 정확 — 예: `"가나다라마"` 5 chars, budget 3 → `"가나다"` 3 chars (byte 9).
    - `pipeline.rs` 의 `#[cfg(test)] mod tests` block 안에 inline (`truncate_chars` 가 `pub(crate)` 라 같은 crate 에서 직접 호출 가능). 별 file 추가 없음.
    - Acceptance: `cargo test -p kebab-rag --lib -j 1` 또는 `cargo test -p kebab-rag -j 1` GREEN.

12. **`tasks/HOTFIXES.md` 신규 dated entry.**
    - line 17 (현 HOTFIX #15 entry) 직전 위치에 삽입. **HOTFIX 번호 미부여** — sibling fb-41 PR-9 closure layer 의 production behavior follow-up (round-1 critic M6 closure).
    - 형식: `## 2026-05-26 — S3 NLI unavailable — hypothesis truncate + token-count fallback`
      - `**Symptom**`: S3 dogfood query 가 NLI 활성 시 `nli_model_unavailable` 일관 fail + 5 회 WARN tokenizer truncation err in `kb.log.2026-05-26`.
      - `**Root cause**`: mDeBERTa-v3 tokenizer 의 `OnlyFirst` 가 hypothesis 단독으로 512-token cap 초과 시 truncate dead-end. premise-side `truncate_for_nli` 만 적용되고 hypothesis-side 무방비.
      - `**Action**`: `kebab-nli::NliVerifier::hypothesis_token_count` trait method 추가 (default `Ok(0)`) + `OnnxNliVerifier` 가 trait impl block 안에서 real tokenize override + `kebab-rag::pipeline::truncate_hypothesis_for_nli_with_budget` retry helper (1200 → 600 → 300 → 150 chars 반감 retry + min 150 floor 시 graceful unavailable fallback) + step 8.5 callsite 양쪽 truncate. 7 files.
      - `**Amends**`: spec `docs/superpowers/specs/2026-05-26-s3-nli-model-unavailable-diagnose-spec.md` cross-link. task spec `tasks/p9/p9-fb-41-multi-hop-finalize.md` 의 NLI 동작 추가 보강 (hypothesis-side budget 신규).
    - Acceptance: `git diff tasks/HOTFIXES.md` 로 entry 가 line 17 직전 위치에 삽입됐는지 확인.

---

## §3. 검증

spec §5.5 의 cargo command verbatim — 단일 사용자 환경 / 16 GB RAM 제약 → `-j 1` 필수.

```bash
# 1. kebab-nli 회귀 — ignored 2 신규 default skip 으로 GREEN
CARGO_TARGET_DIR=/build/out/cargo-target/target \
  cargo test -p kebab-nli -j 1

# 2. kebab-rag 회귀 — 3 mock + 4 boundary 신규 포함 전부 GREEN
CARGO_TARGET_DIR=/build/out/cargo-target/target \
  cargo test -p kebab-rag -j 1

# 3. workspace 전체 — baseline 1306 test 수 유지
CARGO_TARGET_DIR=/build/out/cargo-target/target \
  cargo test --workspace --no-fail-fast -j 1

# 4. lint — -D warnings clean
CARGO_TARGET_DIR=/build/out/cargo-target/target \
  cargo clippy --workspace --all-targets -j 1 -- -D warnings

# 5. Manual smoke — 2 신규 ignored test 가 real mDeBERTa 로 PASS
CARGO_TARGET_DIR=/build/out/cargo-target/target \
  cargo test -p kebab-nli --test inference -- --ignored --test-threads=1
```

### Dogfood retest (spec §5.4 verbatim — `2>&1` 제거)

```bash
CARGO_TARGET_DIR=/build/out/cargo-target/target \
  cargo build --release -p kebab-cli -j 1

# round-1 plan verifier Gap 2 closure: config 의 nli_threshold > 0 사전 확인 —
# default 0.0 이면 NLI off → retest 가 vacuous (fix 의 효과 미발현).
grep -E "^\s*nli_threshold\s*=" /build/cache/dogfood-v018/config/config.toml \
  || echo "WARNING: nli_threshold 미설정 — config 에 [rag] nli_threshold = 0.3 추가 후 진행"

# EN — S3 본래 케이스
RUST_LOG=info,kebab_rag=debug,kebab_nli=debug \
  /build/out/cargo-target/target/release/kebab ask \
    "Why does kebab combine multilingual-e5, LanceDB, and RRF together?" \
    --multi-hop --config /build/cache/dogfood-v018/config/config.toml \
    --json > /build/cache/dogfood-v018/results/post-s3-fix/s3-en-retest.json

# KR — long-answer 시뮬레이션
RUST_LOG=info,kebab_rag=debug,kebab_nli=debug \
  /build/out/cargo-target/target/release/kebab ask \
    "kebab 의 multi-hop RAG + NLI verification 의 동작 원리는 무엇인가?" \
    --multi-hop --config /build/cache/dogfood-v018/config/config.toml \
    --json > /build/cache/dogfood-v018/results/post-s3-fix/s3-kr-retest.json

# 결과 검증
jq -r '.refusal_reason, .verification.nli_score, .usage.completion_tokens' \
  /build/cache/dogfood-v018/results/post-s3-fix/s3-en-retest.json
jq -r '.refusal_reason, .verification.nli_score, .usage.completion_tokens' \
  /build/cache/dogfood-v018/results/post-s3-fix/s3-kr-retest.json

# Tracing log 의 truncate path 활성 확인 (file appender)
grep "NLI hypothesis truncated\|hypothesis_token_count\|hypothesis budget retry" \
  ~/.local/state/kebab/logs/kb.log.$(date -I)
```

### 성공 기준

| 검증 | GREEN 정의 |
|---|---|
| `cargo test -p kebab-nli -j 1` | 기존 test 전부 PASS, 2 ignored skip 으로 표시 |
| `cargo test -p kebab-rag -j 1` | 기존 test 전부 PASS + 3 mock multi-hop + 4 boundary 신규 PASS |
| `cargo test --workspace -j 1` | 1306 baseline 유지 (+ 신규 9 — 정확 카운트 PR 시점 측정) |
| `cargo clippy --workspace -- -D warnings` | warning 0 |
| Manual ignored smoke | 2 신규 test PASS (real mDeBERTa 호출) |
| Dogfood S3 EN retest | `refusal_reason` 가 `null` 또는 `"nli_verification_failed"` (NOT `"nli_model_unavailable"`) + `verification.nli_score` finite float |
| Dogfood S3 KR retest | 동일 — `nli_model_unavailable` 아님 + log 에 retry trace |

---

## §4. 시간 추정

| 단계 | wall time |
|---|---|
| 구현 (12 step) | ~1 hr |
| 검증 (cargo test + clippy + 2 dogfood retest) | ~30 min (`-j 1` workspace test ~12 min + dogfood 2 query ~2 min × 2 + log/jq inspection) |
| OMC reviewer dispatch + 0-1 review iteration amend | ~30 min - 1 hr |
| **합계** | **wall time 2-3 h** |

단일 PR, scope 작음 — round-1 review 만으로 ACCEPT 가능 확률 높음.

---

## §5. 위험 / 회피

spec §8 cross-ref + pre-mortem (a)-(d). 핵심 risk 와 mitigation 만 재서술:

| Risk | Mitigation |
|---|---|
| **RC1-residual: inherent vs trait impl** — step 3 의 `hypothesis_token_count` 가 inherent `impl OnnxNliVerifier {}` 안에 위치하면 vtable 미등록 → pipeline 의 `&dyn NliVerifier` dispatch 시 default `Ok(0)` 호출 → retry loop 즉시 통과 (token count 0 ≤ 256) → real tokenizer 무방비 → **production silent NO-OP** | step 3 의 code-block 위치를 *`impl NliVerifier for OnnxNliVerifier {}` block 안* 명시. spec §5.1 Test 7 (`hypothesis_token_count_dispatches_correctly_via_dyn_trait`) 가 vtable 통해 `> 0` count 검증 — silent NO-OP regression 시 fail. |
| **KR-extreme density (pre-mortem a)** — char-truncate 후에도 token count 초과 (한자/CJK 1 char = 2-3 tokens 가능) | retry 1200→600→300→150 + min floor 시 graceful `nli_model_unavailable` (regression 0). §5.3 mock test `unrelenting_token_overflow_falls_through_to_unavailable` 가 pin. |
| **Conclusion-bearing hypothesis (pre-mortem b)** — LLM 답변의 후반부 결론이 truncate 로 손실 → false negative entailment | fail-closed semantic 보존 (정상 reject). 사용자가 `nli_threshold = 0` 으로 임시 disable 가능. README 안내 별 task §6 #7. |
| **Mock test 의 `build_test_pipeline_with_long_answer` 부재** | round-2 plan critic MAJOR #3 closure: Option B (inline pattern) 채택 — helper 작성 안 함. 각 §5.3 test 안에서 `RagEnv::new()` + `ScriptedRetriever::new()` + `ScriptedLm::new(vec![decompose, decide, &"lorem ".repeat(N)])` + `RagPipeline::new(...).with_verifier(verifier)` 5-component inline (plan step 8 verbatim). |
| **`hypothesis_token_count` probe 자체 fail (pre-mortem d)** | `with_context` wrap → anyhow err → step 8.5 hook 의 **explicit `match` + `return self.refuse_nli_model_unavailable(...)`** (round-1 plan critic CRITICAL #1 closure: `?` propagation 금지, `v.score()` Err 분기와 *대칭*). empty hypothesis 는 진입 전 `acc.trim().is_empty()` guard. |

---

## §6. 자기-review (spec items → plan step mapping)

| spec item | plan step |
|---|---|
| §3 Option A: `NliVerifier::hypothesis_token_count` trait 확장 (default `Ok(0)`) | step 1 |
| §3 Option A: `OnnxNliVerifier::HYPOTHESIS_TOKEN_BUDGET = 256` inherent const | step 2 |
| §3 Option A: trait impl block 안 override (RC1-residual) | step 3 |
| §3 Option A: `MAX_NLI_HYPOTHESIS_CHARS_INITIAL = 1200` + `MIN = 150` consts | step 4 |
| §5.2 chars-only pure-fn `truncate_chars` | step 5 |
| §3 Option A: `truncate_hypothesis_for_nli_with_budget` retry helper | step 6 |
| §3 Option A: step 8.5 hook callsite + premise + hypothesis 양쪽 truncate + tracing debug | step 7 |
| §5.3 verifier Blocker 1: `SpyNliVerifier` helper (closure 2-arg constructor) | step 8 |
| §5.1 Test 6: `score_long_en_hypothesis_returns_err_without_pipeline_truncation` | step 9 |
| §5.1 Test 7 (renumbered): `hypothesis_token_count_dispatches_correctly_via_dyn_trait` (vtable dispatch + KR/EN bounded range — RC1-residual pin) | step 9 |
| §5.3 Test 1: `long_en_synth_answer_truncated_before_nli_call` (Right direction pin) | step 10 |
| §5.3 Test 2: `long_kr_synth_answer_retries_with_smaller_budget` (token_count call count + ≤ 300 chars 최종) | step 10 |
| §5.3 Test 3: `unrelenting_token_overflow_falls_through_to_unavailable` (graceful fallback pin) | step 10 |
| §5.2 4 boundary tests (identity / truncate / empty / KR codepoint) | step 11 |
| §7 합의 출구 #6: HOTFIXES.md dated entry (Symptom / Root cause / Action / Amends 4-block, HOTFIX 번호 미부여) | step 12 |
| §5.5 cargo test + clippy 4 command + manual ignored smoke | §3 |
| §5.4 dogfood EN + KR retest | §3 (dogfood retest 블록) |

모든 spec 의 9 test + 7 file + RC1-residual mitigation 이 plan step 으로 1-to-1 mapping. 누락 0.

---

## §7. Subagent dispatch task

`/superpowers:subagent-driven-development` 의 single task description — executor (sonnet/opus) 가 self-contained 로 따라할 수 있게 *spec §3 + §5 의 핵심 코드 skeleton inline* (round-2 critic M2 의 self-contained 패턴).

### Task name

`S3 NLI unavailable — hypothesis char budget + token-count fallback retry`

### Task description (subagent 에 직접 전달)

> S3 dogfood query (`"Why does kebab combine multilingual-e5, LanceDB, and RRF together?"`) 가 multi-hop 에서 `nli_model_unavailable` 로 일관 fail. Root cause: mDeBERTa-v3 tokenizer 의 `TruncationStrategy::OnlyFirst` + 949-token LLM 답변 = hypothesis 단독이 512-cap 초과 → truncate dead-end. Fix: char-budget 으로 자른 후 real tokenizer 로 token-count 재검증 → 초과 시 char budget 절반화 retry, min 150 chars floor 시 graceful unavailable fallback. KR safe + regression 0.
>
> Spec: `docs/superpowers/specs/2026-05-26-s3-nli-model-unavailable-diagnose-spec.md` (single source).
> Plan: `docs/superpowers/plans/2026-05-26-s3-nli-model-unavailable-diagnose-plan.md` (이 문서).
>
> 12 step (plan §2):
>
> 1. `crates/kebab-nli/src/lib.rs` — trait 에 `hypothesis_token_count(&self, &str) -> anyhow::Result<usize> { Ok(0) }` default 추가.
> 2. `crates/kebab-nli/src/onnx.rs` — `impl OnnxNliVerifier { pub const HYPOTHESIS_TOKEN_BUDGET: usize = 256; }`.
> 3. `crates/kebab-nli/src/onnx.rs` — `impl NliVerifier for OnnxNliVerifier` block **안** 에서 `hypothesis_token_count` override (real `tokenizer.encode` probe). **inherent 가 아닌 trait impl block — vtable 등록 보장**.
> 4. `crates/kebab-rag/src/pipeline.rs` — `MAX_NLI_HYPOTHESIS_CHARS_INITIAL = 1200` + `MAX_NLI_HYPOTHESIS_CHARS_MIN = 150` const.
> 5. `crates/kebab-rag/src/pipeline.rs` — `truncate_chars(s, budget) -> (String, bool)` pure-fn (codepoint-aware).
> 6. `crates/kebab-rag/src/pipeline.rs` — `truncate_hypothesis_for_nli_with_budget(verifier: &dyn NliVerifier, hypothesis: &str) -> anyhow::Result<(String, bool)>` retry loop (spec §3 코드 verbatim).
> 7. `crates/kebab-rag/src/pipeline.rs` — step 8.5 hook (line 1041 근방) 의 `v.score(&truncated_premise, &acc)` 직전에 hypothesis-side truncate hook 삽입 + `tracing::debug` log.
> 8. `crates/kebab-rag/tests/common/mod.rs` — `SpyNliVerifier` (closure score_fn + token_count_fn, 2-arg constructor, capture `Mutex<Vec<String>>`) helper 추가.
> 9. `crates/kebab-nli/tests/inference.rs` — `#[ignore]` 2 test (long EN err pin + **vtable dispatch test for RC1-residual pin** — `let v_dyn: &dyn NliVerifier = &v;` 통해 `hypothesis_token_count` 호출, inherent-only 배치 시 `Ok(0)` 받아 assertion fail).
> 10. `crates/kebab-rag/tests/multi_hop_nli_truncate.rs` (신규) — 3 mock test (EN happy / KR retry / unrelenting fallback).
> 11. `crates/kebab-rag/src/pipeline.rs` 의 `#[cfg(test)] mod tests` — `truncate_chars` 4 boundary test.
> 12. `tasks/HOTFIXES.md` — line 17 직전 신규 dated entry (Symptom / Root cause / Action / Amends 4-block, HOTFIX 번호 미부여).
>
> **검증** (`-j 1` 필수, 16 GB RAM):
>
> ```bash
> CARGO_TARGET_DIR=/build/out/cargo-target/target cargo test -p kebab-nli -j 1
> CARGO_TARGET_DIR=/build/out/cargo-target/target cargo test -p kebab-rag -j 1
> CARGO_TARGET_DIR=/build/out/cargo-target/target cargo test --workspace --no-fail-fast -j 1
> CARGO_TARGET_DIR=/build/out/cargo-target/target cargo clippy --workspace --all-targets -j 1 -- -D warnings
> CARGO_TARGET_DIR=/build/out/cargo-target/target cargo test -p kebab-nli --test inference -- --ignored --test-threads=1
> ```
>
> + dogfood S3 EN + KR retest (plan §3 의 코드 블록 verbatim, `2>&1` 제거 — jq parsing 충돌 회피).
>
> **단일 commit**:
>
> ```
> fix(rag,nli): S3 NLI unavailable — hypothesis char budget + token-count fallback retry
>
> mDeBERTa-v3 tokenizer 의 OnlyFirst strategy 가 hypothesis 단독으로
> 512-token cap 초과 시 truncate dead-end. char-budget retry + real
> tokenizer token-count probe 로 회피, KR safe (1200→600→300→150 retry +
> graceful unavailable fallback at min 150 chars).
>
> - kebab-nli::NliVerifier 에 hypothesis_token_count probe API 추가
>   (default Ok(0) backward-compat).
> - OnnxNliVerifier 가 trait impl block 안에서 real tokenize override
>   (RC1-residual: inherent impl 은 vtable 미등록 → silent NO-OP).
> - kebab-rag::pipeline::truncate_hypothesis_for_nli_with_budget retry
>   helper + step 8.5 hook 의 hypothesis-side hook.
> - SpyNliVerifier closure-based test helper + 5 신규 test (2 ignored
>   inference + 3 mock multi-hop + 4 pure-fn boundary).
> - HOTFIXES.md dated entry.
>
> Closes spec docs/superpowers/specs/2026-05-26-s3-nli-model-unavailable-diagnose-spec.md
>
> 🤖 Generated with [Claude Code](https://claude.com/claude-code)
>
> Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>
> ```
>
> **PR title**: `fix(rag,nli): S3 NLI unavailable — hypothesis char budget + token-count fallback retry`
>
> **PR body skeleton**:
>
> ```
> ## Summary
> - S3 dogfood query 의 `nli_model_unavailable` consistent fail root cause = mDeBERTa-v3 OnlyFirst tokenizer 의 hypothesis-side overflow. char-budget retry + real tokenizer token-count probe 로 회피, KR safe + graceful fallback (regression 0).
> - 4 production files (lib.rs trait, onnx.rs inherent+trait override, pipeline.rs consts+helpers+callsite) + 3 test files (common/mod.rs SpyNliVerifier, inference.rs 2 ignored, multi_hop_nli_truncate.rs 3 mock) + HOTFIXES.md entry.
> - Wire schema 변경 0, Cargo.toml version bump 불필요 (production behavior fix).
>
> ## Test plan
> - [ ] `cargo test -p kebab-nli -j 1` GREEN (기존 + 2 ignored skip)
> - [ ] `cargo test -p kebab-rag -j 1` GREEN (기존 + 3 mock + 4 boundary)
> - [ ] `cargo test --workspace --no-fail-fast -j 1` GREEN (baseline 1306+ 유지)
> - [ ] `cargo clippy --workspace --all-targets -j 1 -- -D warnings` clean
> - [ ] `cargo test -p kebab-nli --test inference -- --ignored --test-threads=1` GREEN (manual smoke)
> - [ ] Dogfood S3 EN retest: `refusal_reason` 가 `nli_model_unavailable` 아님
> - [ ] Dogfood S3 KR retest: 동일 + retry trace in kb.log
>
> Spec: `docs/superpowers/specs/2026-05-26-s3-nli-model-unavailable-diagnose-spec.md`
> Plan: `docs/superpowers/plans/2026-05-26-s3-nli-model-unavailable-diagnose-plan.md`
> ```

### RR-tier cleanup notes (executor 자기-review 단계 또는 별 cleanup commit)

executor 가 step 1-12 완료 후 *자기-review 단계* 에서 다음 RR1-5 cleanup 확인:

- **RR1 (style)**: `truncate_chars` / `truncate_hypothesis_for_nli_with_budget` doc comment 의 spec §X cross-link 형식이 기존 pipeline.rs 의 doc comment 형식과 일치 (e.g. `truncate_for_nli` 의 doc 와 sibling style).
- **RR2 (naming)**: const `MAX_NLI_HYPOTHESIS_CHARS_INITIAL` / `_MIN` 의 prefix 가 기존 `MAX_NLI_PREMISE_CHARS` 와 sibling 명명 유지.
- **RR3 (test 분류)**: §5.2 boundary 4 test 는 *pipeline.rs `#[cfg(test)] mod tests`* 안 (pure-fn unit) vs *`multi_hop_nli_truncate.rs`* (integration mock) 의 layer 구분 명확. `truncate_chars` 가 `pub(crate)` 라 같은 crate 의 lib test 에서 직접 접근 가능 — integration test 로 빼면 visibility issue. *pipeline.rs 안 #[cfg(test)] mod 로 둠* 권장.
- **RR4 (tracing target)**: `tracing::debug!(target: "kebab-rag", ...)` 의 target 이 기존 `truncate_for_nli` 의 tracing target 과 일치.
- **RR5 (HOTFIXES 위치)**: line 17 직전 (현 HOTFIX #15 위) 정확 위치. HOTFIX 번호 *미부여* (sibling fb-41 layer follow-up — round-1 critic M6 closure).

RR1-5 는 별 cleanup commit 으로 분리하지 말고 implementation commit 의 자기-review 단계에서 inline 처리 권장 (scope 작은 hotfix 라).

---

## §8. plan v1 status

- spec 4 round adversarial review (critic + verifier 모두 ACCEPT/APPROVE) 완료 후 작성.
- round-1 OMC reviewer dispatch 준비 — 본 plan v1 의 critique 가 amend 입력.
- subagent dispatch 는 plan v1 의 reviewer ACCEPT 후 진행.
