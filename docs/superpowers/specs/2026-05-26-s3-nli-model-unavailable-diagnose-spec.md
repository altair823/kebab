---
title: "S3 dogfood query 의 nli_model_unavailable consistent fail — hypothesis-side 토큰 overflow 가 NLI tokenizer 의 OnlyFirst-too-short 에러를 trip"
date: 2026-05-26
task_id: s3-nli-unavailable
status: open
target_version: 0.18.1
sibling_of: tasks/HOTFIXES.md "2026-05-25 — fb-41 pre-v0.18 dogfood: multi-hop score-gate 우회"
fix_files:
  - crates/kebab-nli/src/lib.rs  # NliVerifier trait 의 hypothesis_token_count default impl
  - crates/kebab-nli/src/onnx.rs # OnnxNliVerifier::hypothesis_token_count override + HYPOTHESIS_TOKEN_BUDGET
  - crates/kebab-rag/src/pipeline.rs # truncate_hypothesis_for_nli_with_budget + MAX_NLI_HYPOTHESIS_CHARS_INITIAL/MIN + step 8.5 callsite
  - crates/kebab-rag/tests/common/mod.rs # SpyNliVerifier helper
fix_symbols:
  - "kebab_nli::NliVerifier::hypothesis_token_count (trait method, default impl)"
  - "kebab_nli::OnnxNliVerifier::HYPOTHESIS_TOKEN_BUDGET (const 256)"
  - "kebab_rag::pipeline::MAX_NLI_HYPOTHESIS_CHARS_INITIAL (const 1200)"
  - "kebab_rag::pipeline::MAX_NLI_HYPOTHESIS_CHARS_MIN (const 150)"
  - "kebab_rag::pipeline::truncate_hypothesis_for_nli_with_budget"
---

# S3 NLI unavailable — Hypothesis-side overflow trips OnlyFirst-only tokenizer

## §1. 진단 (Root cause)

`crates/kebab-nli/src/onnx.rs::OnnxNliVerifier::load_tokenizer` (line 216-223) 는 mDeBERTa-v3 tokenizer 를 다음과 같이 설정:

```rust
tokenizer.with_truncation(Some(TruncationParams {
    max_length: MAX_TOKENS,           // 512
    strategy: TruncationStrategy::OnlyFirst,
    stride: 0,
    direction: TruncationDirection::Right,
}))
```

`OnlyFirst` 는 *2-sequence input* 일 때 첫 번째 sequence (= premise) 만 자른다. 두 번째 sequence (= hypothesis) 가 *단독으로* `max_length - special_tokens (≈ 509)` 를 초과하면, premise 를 0 까지 잘라도 fit 시킬 방법이 없어 `tokenizers` crate 가 `TokenizerError::SequenceTooShortToTruncate` 를 raise:

```
Truncation error: Sequence to truncate too short to respect the provided max_length
```

`pipeline.rs::ask_multi_hop` (line 1041) 는 premise 측만 char-budget 으로 자르고 (`truncate_for_nli` / `MAX_NLI_PREMISE_CHARS = 1600`), hypothesis (`acc` = LLM-synth answer) 는 raw 로 `v.score(&truncated_premise, &acc)` 호출. `Err` 가 나오면 line 1057-1064 의 `Err(e) => return self.refuse_nli_model_unavailable(...)` 가 *모든* tokenizer/inference err 를 `RefusalReason::NliModelUnavailable` 로 단일화 → wire 상 "모델 unavailable" 로 보고.

### S3 의 특이성

- Query: `"Why does kebab combine multilingual-e5, LanceDB, and RRF together?"`
- KB: kebab self-knowledge (README / spec / SMOKE / ARCHITECTURE / HANDOFF). retrieval 이 다수 chunks 반환 → synthesize prompt 가 풍부 → gemma3:4b 가 장황한 답 생성.

**Evidence chain (round-1 critic H1 closure — 두 invocation 분리 명시)**:

1. **LLM answer 길이 evidence**: `/build/cache/dogfood-v018/results/s3-multihop.json` (2026-05-25 dogfood, `nli_threshold = 0` 즉 NLI off) — `usage.completion_tokens = 949`, `answer chars = 4564`. *이 invocation 의 refusal 은 `llm_self_judge` (NLI off 상태)* — NLI tokenizer 는 호출조차 안 됨. *오직* "S3 같은 query 가 KB self-knowledge 라 LLM 이 verbose 답을 생성한다" 는 *답변 길이* 만 evidence.

2. **NLI tokenizer error evidence**: `~/.local/state/kebab/logs/kb.log.2026-05-26` 의 5 회 WARN 라인 (2026-05-26 retest, NLI threshold > 0 활성) — `tokenizer.encode failed: Truncation error: Sequence to truncate too short to respect the provided max_length`. 이 retest 의 JSON 결과는 미보존 (별 follow-up `--json` redirect 누락) — wire evidence 는 log 만.

3. **연결 추론**: long answer (#1) + NLI 활성 (#2) → OnlyFirst-only tokenizer 가 hypothesis 단독 cap 초과 → tokenizer error → `nli_model_unavailable` wire. 두 invocation 의 mechanic 이 같은 root cause 로 수렴.

다른 PASS 케이스 (S1=한국어 컴파일러, S7=caffeine, S10=dinosaurs) 는 KB 에 본문 fact 가 *없거나* 부분만 있어 답이 짧다 (S7 retest: `/build/cache/dogfood-v018/results/post-pr9/s7-multihop.json` 의 `usage.completion_tokens = 114`). 그래서 NLI tokenizer 가 정상 작동하고 `nli_score` 가 측정됨 (낮아서 verification_failed 로 거부됨).

### Tracing surface

진단 evidence (`kb.log.2026-05-26` 의 WARN 5 회) 발견 시 *우회 자체* 가 시간 비용 — 별 task `§6 #2 (KEBAB_LOG_STDERR opt-in)` + `§6 #7 (README/SMOKE 안내 한 줄)`. **본 spec scope 외** (round-1 critic M4 closure — root cause 와 무관, defer 로 정렬).

### Hypothesis 별 검증 (analyst empirical)

- **A (ort Session::run shape err)** — REJECTED. tokenizer 가 먼저 실패해 Session::run 까지 도달하지 않음. 에러 메시지가 ort 가 아니라 `tokenizer.encode failed`.
- **B (tokenizer encode fail)** — **CONFIRMED**. error 메시지 텍스트 일치.
- **C (hf-hub Cache::get probe race)** — REJECTED. 모델 cache 존재 + 매 invocation 결정적 같은 실패.
- **D (OnceLock partial init race)** — REJECTED. sequential dogfood, single-thread `App` boot. 같은 input 으로 매번 같은 실패 → race 아님.

### 313 s latency 의 정체

NLI 가 hang 한 게 아님. dogfood S3 의 `latency_ms` 는 gemma3:4b 의 **synthesize hop LLM streaming 시간**. NLI tokenizer 는 ms 단위 fast-fail.

---

## §2. 영향

| Surface | 영향 |
|---|---|
| Production behavior | NLI gate 활성 (`rag.nli_threshold > 0`) 인 KB 에서 LLM 이 **500 tokens 초과** 답변을 생성하면 항상 `nli_model_unavailable` refusal. KB-rich self-knowledge query / 다단계 multi-hop 결과에서 빈번. |
| Wire (`answer.v1`) | `refusal_reason = "nli_model_unavailable"`, `verification = null`. 사용자/agent 가 "NLI 모델 다운로드 / 로드 문제" 로 오인. |
| Wire (`error.v1`) | 영향 없음 — refusal envelope 만 사용, error envelope 으로는 안 빠짐. |
| 사용자 binary | `[rag] nli_threshold = 0` (현 default) 사용자: 미영향. README + frozen design 의 *권장 production 0.5* 활성 사용자: KB-rich self-knowledge query 또는 verbose LLM 답변에서 빈번 노출 — `nli_model_unavailable` refusal 받음 (round-1 critic M5 정정: "일반 사용자 미영향" 은 *현 default 기준* 만 정확, *권장 사용 기준* 으로는 misleading). |
| NLI model 자체 | **정상**. Cache 정상, `score` 도 짧은 hypothesis 에는 작동. |

---

## §3. Fix design

### Option A (권장) — Char-budget + token-count fallback retry (round-1 critic H3 + verifier Blocker 2 closure)

`crates/kebab-rag/src/pipeline.rs` 의 premise-side `truncate_for_nli` + `MAX_NLI_PREMISE_CHARS = 1600` 패턴을 hypothesis 에도 대칭 적용하되, **char-budget 만으로는 KR-heavy hypothesis (1-2 chars/token) 가 512 cap 을 단독 초과 가능** (1200 KR chars × 1 char/token = 1200 tokens >> 512). 따라서 char-truncate 후 *실 mDeBERTa tokenizer 로 token-count 재검증* → 초과 시 char budget 절반으로 retry. 최악 fallback 시 `nli_model_unavailable` (regression 0).

#### `kebab-nli` crate 의 API 작은 확장

**Trait 확장 + impl placement** (round-3 critic RC1-residual closure — `hypothesis_token_count` 가 *inherent* 가 아닌 *trait impl block* 안에 위치해야 trait dispatch 가 vtable 통해 호출 — production silent NO-OP 회피):

```rust
// crates/kebab-nli/src/lib.rs — trait 확장
pub trait NliVerifier: Send + Sync {
    fn score(&self, premise: &str, hypothesis: &str) -> anyhow::Result<NliScores>;

    /// Probe-only tokenize for caller-side budget verification. round-1
    /// critic H3 closure — pipeline 의 char-budget retry loop 가 이 API 로
    /// `OnlyFirst` dead-end 회피.
    ///
    /// **Default impl 반환 0** — 기존 mock implementations
    /// (`MockNliVerifier`) 가 trait 확장 후에도 backward-compat (compile
    /// fail 회피, retry loop immediate 통과). `OnnxNliVerifier` 는 real
    /// tokenizer 로 *trait impl 블록 안에서* override (round-3 critic
    /// RC1-residual — inherent method 는 vtable 미등록 → trait dispatch
    /// 시 default 호출 → silent NO-OP).
    fn hypothesis_token_count(&self, _hypothesis: &str) -> anyhow::Result<usize> {
        Ok(0)
    }
}
```

```rust
// crates/kebab-nli/src/onnx.rs — inherent 와 trait impl 분리
impl OnnxNliVerifier {
    /// Hypothesis-side budget. Pipeline uses this to size its
    /// char-truncation retry loop. = MAX_TOKENS (512) - 3 special tokens
    /// reserved (CLS, SEP, SEP) - some premise room (caller decides).
    pub const HYPOTHESIS_TOKEN_BUDGET: usize = 256; // 안전 마진: 512 - 3 - 253 premise room
}

impl NliVerifier for OnnxNliVerifier {
    fn score(&self, premise: &str, hypothesis: &str) -> anyhow::Result<NliScores> {
        // ... 기존 score body 그대로 (현재 onnx.rs:229-280)
    }

    /// **Override** the trait default `Ok(0)` with a real mDeBERTa tokenize.
    /// Pipeline 의 `truncate_hypothesis_for_nli_with_budget` retry loop 가
    /// 이 method 를 vtable 통해 호출 — production code path 에서 실 token
    /// count 측정 (round-3 critic RC1-residual closure: inherent impl 이
    /// 아닌 *trait impl block 안* 에 위치해야 vtable 등록).
    fn hypothesis_token_count(&self, hypothesis: &str) -> anyhow::Result<usize> {
        let (_session, tokenizer) = self.ensure_loaded()?;
        let enc = tokenizer
            .encode(hypothesis, /*add_special_tokens=*/false)
            .map_err(|e| anyhow!("kebab-nli: tokenizer.encode (probe) failed: {e}"))?;
        Ok(enc.get_ids().len())
    }
}
```

#### `crates/kebab-rag/src/pipeline.rs` 의 hypothesis-side budget + retry

```rust
/// p9-fb-41 S3 follow-up: NLI hypothesis (= synthesized answer) 가
/// 512-token cap 을 단독으로 초과하면 `OnlyFirst` truncation 이 premise 를
/// 0 까지 잘라도 fit 시킬 수 없어 tokenizer 가 err. char-budget 으로
/// 자른 후 *실 mDeBERTa tokenizer* 로 token count 재검증 — 초과 시 char
/// budget 절반으로 retry. KR safe (round-1 critic H3 closure).
pub const MAX_NLI_HYPOTHESIS_CHARS_INITIAL: usize = 1200;
pub const MAX_NLI_HYPOTHESIS_CHARS_MIN: usize = 150;

/// Truncate hypothesis with `Right` direction (앞부분 보존 — LLM 답변의
/// 도입부에 핵심 claim 이 있음. 후반 손실은 결론 재요약 영향 적음).
/// Empirically validates token-count via `verifier.hypothesis_token_count`
/// 와 절반화 retry. 최종 cap = `HYPOTHESIS_TOKEN_BUDGET`. fallback 도달
/// 시 anyhow::Err — caller (step 8.5 hook) 가 기존 unavailable path.
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

        // verifier 의 internal tokenizer 로 token count 재검증.
        // downcast 는 trait method 이라 직접 호출 가능.
        let token_count = verifier
            .hypothesis_token_count(&candidate)
            .with_context(|| "kebab-rag: hypothesis token-count probe failed")?;
        if token_count <= kebab_nli::OnnxNliVerifier::HYPOTHESIS_TOKEN_BUDGET {
            return Ok((candidate, was_truncated));
        }

        // 초과 — char budget 절반화 retry.
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

#### Callsite (step 8.5 hook) — round-1 plan critic CRITICAL #1 closure

`?` propagation 사용 시 `ask_multi_hop` 이 `Err(anyhow::Error)` 반환 → wire `error.v1` 로 빠짐 (graceful fallback 약속 위반). 기존 `v.score()` Err 분기 (`return self.refuse_nli_model_unavailable(...)`) 와 *대칭* 으로 explicit match + return refuse 사용:

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
// ... 기존 v.score() 호출 path — Err 분기 (line 1057-1064) 와 대칭 패턴
match v.score(&truncated_premise, &truncated_hypothesis) {
    Ok(scores) => { /* 기존 path */ }
    Err(e) => { /* 기존 refuse_nli_model_unavailable */ }
}
```

**핵심**: `?` 대신 explicit `match` + `return self.refuse_nli_model_unavailable(...)` — wire `answer.v1 + refusal_reason=nli_model_unavailable` 유지 (graceful fallback 약속 보존, regression 0).

**Pros**:
- KR + EN *대부분 케이스* 안전 (token-count retry path + char budget 절반화 retry). 최악 case (KR-extreme density e.g. 압축 한자 텍스트) 시 graceful fallback 으로 기존 unavailable 유지 — regression 0 (round-2 critic RM3 closure).
- premise-side `truncate_for_nli` 와 *대칭* (둘 다 pipeline-layer helper).
- `kebab-nli::NliVerifier` trait 에 1 method 추가 — wire 영향 0, API 확장 acceptable (v0.18 의 NLI surface 가 아직 unstable internal).

**Cons**:
- `NliVerifier` trait 에 `hypothesis_token_count` method 추가 — implementations (현재 1 개: `OnnxNliVerifier`) 갱신. mock implementations (`MockNliVerifier`) 도 갱신 필요.
- Retry loop 마다 tokenizer 호출 — KR worst case 4-5 회 (1200→600→300→150). 각 호출 ~1 ms (tokenizer-only, no inference). 부담 적음.

### Option B — Tokenizer 의 `OnlyFirst` → `LongestFirst` 로 변경

`crates/kebab-nli/src/onnx.rs:219` 의 `TruncationStrategy::OnlyFirst` → `LongestFirst`.

**Pros**: char-budget / retry 도입 불필요, 두 sequence 모두 자연스럽게 truncate.
**Cons**: 짧은 premise + 긴 hypothesis 케이스에서 premise 는 살아남고 hypothesis 가 잘림 — premise 가 충분히 사실 정보를 담고 있어야 entailment 가 의미 있음 (보통 그렇지만 보장은 아님). 또한 task spec `2026-05-25-p9-fb-41-finalize-spec.md::§2.2.3` 에 `OnlyFirst` 가 명시되어 있어 task spec amend + HOTFIXES.md entry 필요 (round-1 critic H2 closure — frozen design contract `2026-04-27-kebab-final-form-design.md` 에는 §2.2.3 자체 부재; OnlyFirst 명시는 task spec 수준).

### Option C — NLI crate 가 자기 cap 을 관리 (hypothesis-aware truncate in `score()`)

`crates/kebab-nli/src/onnx.rs::score` 안에서 hypothesis tokenize → token-count 체크 → 초과면 char-budget 으로 truncate 후 retry.

**Pros**: caller 가 cap 을 모르고도 안전. encapsulation 잘 됨.
**Cons**: pipeline-side 의 기존 `truncate_for_nli` 와 layering 불일치 (premise 는 pipeline 이 자르는데 hypothesis 는 nli crate 가 자르는 비대칭).

### 권장: **Option A** (KR best-effort + graceful fallback + 대칭 + frozen design contract 무관)

추후 개선 (별 task): token-count 기반 *premise* budget — 현재 `MAX_NLI_PREMISE_CHARS = 1600` 도 KR-heavy chunks 에서 동일 risk. fix_file 의 기존 주석 *"v0.18.1 candidate: token-count-based budget"* — 별 task 후보 (§6 #1 참조).

### 부수 fix (별 task 권장)

**tracing surface — env var + stderr layer 안내**

진단 시간 단축을 위해 다음 중 하나 (별 PR / 별 task):

- (B-1) `README.md` / `docs/SMOKE.md` 에 *"진단 시 `RUST_LOG=debug` + `tail -f ~/.local/state/kebab/logs/kb.log.$(date -I)`"* 한 줄 추가.
- (B-2) `crates/kebab-app/src/logging.rs` 에 `KEBAB_LOG_STDERR=1` env var 가 set 되어 있으면 stderr fmt::layer 도 추가. opt-in 이라 wire / 일반 사용자 영향 0.

**본 S3 fix PR 의 scope 밖** — analyst 권장 따라 별 task 분리 (PR 의 review lens 분리).

---

## §4. Wire / behavior 영향

- **Wire schema**: 변경 없음. `answer.v1` / `error.v1` 모두 영향 없음.
- **`schema_version` bump**: 불필요.
- **`Cargo.toml` version bump**: 불필요 (test/diagnostic fix, breaking schema 아님).
- **Behavior 변화** (round-2 critic RM3 정합): 긴-답변 multi-hop 케이스 *대다수* 가 `nli_model_unavailable` 대신 *실제 NLI score* 를 받음. score 가 낮으면 `nli_verification_failed` (정상 정확한 refusal), 높으면 `nli_passed=true` (이전엔 도달 못함). KR-extreme density 최악 case 만 기존 `nli_model_unavailable` 유지 (graceful fallback, regression 0). 즉 *false-negative 대다수 줄어듦 + 최악 case 의 wire 결과 보존*.
- **Default behavior**: 사용자 default config 는 `nli_threshold = 0` (NLI off) — 일반 사용자에는 무영향. NLI 활성 사용자에게는 *옳은 방향의 변경* (이전엔 false unavailable, 이후엔 truthful score).

---

## §5. 검증 plan

### 5.1. Unit test — `crates/kebab-nli/tests/inference.rs` (round-1 critic M1 보강)

3 신규 test (network 요구 → `#[ignore]` 패턴 기존과 동일):

```rust
/// Test 6: EN-long hypothesis alone exceeds max_length. Without pipeline-side
/// truncation, OnlyFirst strategy dead-ends. Pin raw nli crate behavior so
/// any future regression in the pipeline-side budget surfaces as a clear
/// nli-level err.
#[test]
#[ignore]
fn score_long_en_hypothesis_returns_err_without_pipeline_truncation() {
    let cfg = Config::defaults();
    let v = OnnxNliVerifier::new(&cfg).expect("verifier construction");
    let premise = "short premise";
    let hypothesis = "lorem ipsum ".repeat(500); // ~6 000 chars / >>512 tokens
    let result = v.score(premise, &hypothesis);
    assert!(result.is_err(), "long hypothesis should err under OnlyFirst");
    let msg = result.err().unwrap().to_string();
    assert!(
        msg.contains("Truncation error") || msg.contains("too short to respect"),
        "expected tokenizer truncation err, got: {msg}"
    );
}

// (round-2 critic RM4 closure: 이전 draft 의 "Test 7 KR-heavy hypothesis" 가
// conditional assertion (`if result.is_err()`) 로 regression pin 가 아닌
// measurement test 패턴이었음 — drop. Test 8 (hypothesis_token_count 의
// bounded range assertion) 이 KR/EN behavior 의 deterministic pin 으로
// 더 안전. KR safety 의 wire-level pin 은 §5.3 의
// `long_kr_synth_answer_retries_with_smaller_budget` 의 retry 시뮬레이션
// 이 담당.)

/// Test 7 (renumbered): hypothesis_token_count helper — pure tokenizer probe.
/// **vtable dispatch 검증** (round-3 critic RC1-residual + round-1 plan verifier
/// Gap 1 closure — concrete type 호출은 inherent method 우선이라 RC1-residual
/// 버그 잡지 못함; `&dyn NliVerifier` 통해 dispatch 해야 vtable 등록 검증).
/// inherent-only 배치 시 default `Ok(0)` 반환 → `assert!(count > 0)` 실패.
/// trait impl block 배치 시 real tokenizer → PASS.
/// Pipeline 이 retry budget 결정에 사용하는 API 의 정확성 pin.
#[test]
#[ignore]
fn hypothesis_token_count_dispatches_correctly_via_dyn_trait() {
    let cfg = Config::defaults();
    let v = OnnxNliVerifier::new(&cfg).expect("verifier construction");
    // ★ vtable dispatch — &dyn NliVerifier 통해 호출. inherent-only 배치 시 default
    // `Ok(0)` 반환 → assert!(count > 0) 실패. trait impl block 배치 시 real
    // tokenizer → PASS. round-3 critic RC1-residual + round-1 plan verifier
    // Gap 1 closure 의 코드-수준 regression pin.
    let v_dyn: &dyn NliVerifier = &v;
    // 짧은 EN — 4 chars/token 추정 (24 chars / 4 = ~6 tokens)
    let en_count = v_dyn.hypothesis_token_count("short english test sentence")
        .expect("EN dyn dispatch must reach real tokenizer (vtable check)");
    assert!(
        en_count > 0 && en_count < 20,
        "EN ~6 tokens expected via vtable dispatch, got {en_count} \
         (Ok(0) signals inherent-only placement bug — RC1-residual)"
    );
    // 짧은 KR — 1-2 chars/token (15 chars / 1.5 = ~10 tokens)
    let kr_count = v_dyn.hypothesis_token_count("짧은 한국어 테스트 문장입니다")
        .expect("KR dyn dispatch must reach real tokenizer");
    assert!(kr_count > 0 && kr_count < 30, "KR ~10 tokens expected, got {kr_count}");
}
```

### 5.2. Pure-fn test — chars-only sub-helper (round-2 critic RM2 closure)

`truncate_hypothesis_for_nli_with_budget(verifier, hypothesis)` 는 *verifier 의존* 이라 pure-fn 아님. round-2 critic RM2 권장 (a) 채택 — chars-only sub-helper 분리:

```rust
// crates/kebab-rag/src/pipeline.rs
/// Chars-only truncation arithmetic. Pure: input → output, no side effect.
/// Used by `truncate_hypothesis_for_nli_with_budget` 의 retry loop 의 각 step.
/// Pure-fn test (§5.2) 가 이 helper 의 산술 회귀 핀.
pub(crate) fn truncate_chars(s: &str, budget: usize) -> (String, bool) {
    if s.chars().count() <= budget {
        (s.to_string(), false)
    } else {
        let truncated: String = s.chars().take(budget).collect();
        (truncated, true)
    }
}
```

Test (`crates/kebab-rag/tests/multi_hop.rs` 또는 pipeline.rs `#[cfg(test)]` mod) — 4 boundary cases:

- 입력 <= budget: identity, `was_truncated=false`.
- 입력 > budget: budget chars, `was_truncated=true`.
- 입력 = empty: identity.
- 입력 = 한글 chars (codepoint, NOT byte): `chars().count()` 정확.

`truncate_hypothesis_for_nli_with_budget` 의 retry-loop integration 은 §5.3 mock test 가 검증 (verifier 의 token_count_fn 시뮬레이션). 두 layer 의 *분리된 검증* — pure-fn 산술 + integration retry path.

### 5.3. Mock multi-hop test — long-answer happy path (verifier Blocker 1 closure)

`crates/kebab-rag/tests/multi_hop_nli_stream.rs` (또는 신규 `multi_hop_nli_truncate.rs`).

**Helper 신규 작성 필요** (verifier Blocker 1): 기존 `crates/kebab-rag/tests/common/mod.rs::MockNliVerifier` 는 *고정 mode* (`MockMode::Scores` / `MockMode::Err`) 만 지원 — closure spy 패턴 불가. 신규 `SpyNliVerifier` (또는 `FakeNliVerifier`) helper 추가:

```rust
// crates/kebab-rag/tests/common/mod.rs 에 추가
use std::sync::{Arc, Mutex};
use kebab_nli::{NliScores, NliVerifier};

/// Closure-based NLI verifier — caller 가 `(premise, hypothesis) -> Result<NliScores>`
/// + `(hypothesis) -> Result<usize>` 두 closures 정의 가능 + spy 로 입력 capture.
/// 기존 `MockNliVerifier` (고정 mode) 와 sibling. round-1 verifier Blocker 1
/// + round-2 verifier Caveat 1 closure (struct 필드 위치 + 2-arg constructor).
pub struct SpyNliVerifier {
    pub score_fn: Arc<dyn Fn(&str, &str) -> anyhow::Result<NliScores> + Send + Sync>,
    pub hypothesis_token_count_fn:
        Arc<dyn Fn(&str) -> anyhow::Result<usize> + Send + Sync>,
    pub received_premises: Mutex<Vec<String>>,
    pub received_hypotheses: Mutex<Vec<String>>,
}

impl SpyNliVerifier {
    /// 2-arg constructor — score + hypothesis_token_count 둘 다 closure 로
    /// 주입. Caveat 1 의 "Arc<T> field mutation 불가" 회피.
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

(NliVerifier trait 확장 + OnnxNliVerifier 의 trait impl override 는 §3 에서 이미 명시 — 중복 제거 (round-3 critic RC1-residual closure 의 일부).)

**Pipeline construction pattern** (round-2 plan critic MAJOR #3 closure: Option B inline 채택, helper 신규 작성 안 함): 각 test 안에서 직접 inline. 아래 3 test 의 `build_test_pipeline_with_long_answer(verifier, char_len)` 호출은 *placeholder semantic only* — 실 구현은 `plan §2 step 8` 의 Option B 5-component recipe (`RagEnv::new()` + `ScriptedRetriever::new(...)` + `ScriptedLm::new(vec!["[\"q1\"]", "[]", &"lorem ".repeat(char_len/12)])` + `RagPipeline::new(cfg, retriever, lm, sqlite).with_verifier(verifier)`) 따를 것. 기존 `crates/kebab-rag/tests/multi_hop.rs` 의 happy-path test pattern 그대로.

**3 신규 test**:

```rust
/// EN long answer → char-budget 만으로 충분 → retry 0회 → grounded.
#[test]
fn long_en_synth_answer_truncated_before_nli_call() {
    let verifier = SpyNliVerifier::new(
        |_premise, _hypothesis| {
            Ok(NliScores { entailment: 0.9, neutral: 0.05, contradiction: 0.05 })
        },
        |_h| Ok(100), // budget 안 — retry 0
    );

    let pipeline = build_test_pipeline_with_long_answer(verifier.clone(), 5_000); // EN-long
    let answer = pipeline.ask_multi_hop("q", AskOpts::default()).unwrap();

    let received = verifier.received_hypotheses.lock().unwrap();
    assert_eq!(received.len(), 1, "verifier called exactly once");

    let hyp = &received[0];
    assert_eq!(
        hyp.chars().count(),
        1200,
        "hypothesis truncated to MAX_NLI_HYPOTHESIS_CHARS_INITIAL"
    );

    // round-1 critic M2: direction Right pin — hypothesis 의 *첫 1200 chars* 가
    // input 의 *첫 1200 chars* 와 일치 (= Right direction = 앞부분 보존). Left/Middle
    // direction 으로 regress 시 본 test 가 즉시 fail.
    let input_first_1200: String = "lorem ipsum ".repeat(2000).chars().take(1200).collect();
    assert_eq!(hyp.as_str(), input_first_1200, "Right direction = front preserved");

    assert_eq!(answer.refusal_reason, None, "long answer must reach happy path");
}

/// KR long answer → token count > budget → char budget 절반화 retry → eventual fit.
/// round-1 critic H3 + verifier Blocker 2 의 KR safety pin.
#[test]
fn long_kr_synth_answer_retries_with_smaller_budget() {
    let token_count_call_count = Arc::new(Mutex::new(0));
    let tcc = token_count_call_count.clone();
    // 시뮬레이션: 1200 chars → 900 tokens (cap 초과), 600 chars → 450 tokens (cap 초과),
    // 300 chars → 220 tokens (cap 안). retry 3 회.
    let verifier = SpyNliVerifier::new(
        |_premise, _hypothesis| {
            Ok(NliScores { entailment: 0.85, neutral: 0.10, contradiction: 0.05 })
        },
        move |h| {
            *tcc.lock().unwrap() += 1;
            let count = h.chars().count();
            if count > 1000 { Ok(900) }
            else if count > 500 { Ok(450) }
            else { Ok(220) }
        },
    );

    let pipeline = build_test_pipeline_with_long_answer(verifier.clone(), 2_500); // 2500 chars KR-sim
    let answer = pipeline.ask_multi_hop("q", AskOpts::default()).unwrap();

    assert!(
        *token_count_call_count.lock().unwrap() >= 3,
        "retry loop must call token_count >= 3 (1200, 600, 300 candidates)"
    );
    let received = verifier.received_hypotheses.lock().unwrap();
    assert!(received[0].chars().count() <= 300, "final hypothesis <= 300 chars after retry");
    assert_eq!(answer.refusal_reason, None, "KR long answer reaches happy path after retry");
}

/// Retry budget 소진 시 graceful unavailable — fix 의 fallback path 가
/// 기존 unavailable wire shape 유지 (regression 0).
#[test]
fn unrelenting_token_overflow_falls_through_to_unavailable() {
    let verifier = SpyNliVerifier::new(
        |_premise, _hypothesis| {
            unreachable!("score not reached when token-count check fails");
        },
        |_h| Ok(9_999), // 모든 budget 에서 token count 초과 — retry 소진
    );

    let pipeline = build_test_pipeline_with_long_answer(verifier.clone(), 3_000);
    let answer = pipeline.ask_multi_hop("q", AskOpts::default()).unwrap();

    assert_eq!(
        answer.refusal_reason,
        Some(RefusalReason::NliModelUnavailable),
        "graceful fallback to unavailable when retry exhausted"
    );
}
```

### 5.4. Dogfood S3 + KR 케이스 retest (round-1 critic H4 + verifier non-blocker closure)

Fix 적용 후 EN (S3) + KR (long-answer 시뮬레이션) 두 케이스 retest:

```bash
# round-1 plan verifier Gap 2 closure — config 의 nli_threshold > 0 사전 확인.
# nli_threshold = 0 (default) 이면 NLI off 상태 → retest 가 vacuous (fix 의 효과 미발현).
grep -E "^\s*nli_threshold\s*=" /build/cache/dogfood-v018/config/config.toml \
  || echo "WARNING: nli_threshold 미설정 — dogfood retest 전에 config 에 [rag] nli_threshold = 0.3 (또는 원하는 값) 추가 필수"
# nli_threshold > 0 확인 후 진행.

CARGO_TARGET_DIR=/build/out/cargo-target/target \
  cargo build --release -p kebab-cli -j 1

# Non-blocker fix: stdout / stderr 분리 (2>&1 제거 — jq parsing 충돌 회피).
# stderr 는 file appender (kb.log) 로만 가므로 stderr redirect 불요.

# EN — S3 본래 케이스
RUST_LOG=info,kebab_rag=debug,kebab_nli=debug \
  /build/out/cargo-target/target/release/kebab ask \
    "Why does kebab combine multilingual-e5, LanceDB, and RRF together?" \
    --multi-hop --config /build/cache/dogfood-v018/config/config.toml \
    --json > /build/cache/dogfood-v018/results/post-s3-fix/s3-en-retest.json

# KR — long-answer 시뮬레이션 (KR self-knowledge query)
# KR-heavy hypothesis 가 본 fix 의 token-count fallback retry path 를 trigger 검증
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

# Tracing log 의 truncate path 활성 확인 (file appender 분리 grep)
grep "NLI hypothesis truncated\|hypothesis_token_count" \
  ~/.local/state/kebab/logs/kb.log.$(date -I)
```

**성공 기준**:
- **EN (S3)**: `refusal_reason` 이 `null` (NLI passed) 또는 `nli_verification_failed` (정상 거부). **`nli_model_unavailable` 아님**.
- **KR (long-answer)**: 동일 — `nli_model_unavailable` 아님. token-count retry path 가 작동했음을 log 의 *retry budget* line 으로 확인.
- `verification.nli_score` 가 finite float (이전엔 `null`).
- `~/.local/state/kebab/logs/kb.log.$(date -I)` 에 `truncate_hypothesis_for_nli_with_budget` 의 retry trace 또는 success debug line emit.

### 5.5. 회귀 — 기존 NLI test 전부

```bash
CARGO_TARGET_DIR=/build/out/cargo-target/target \
  cargo test -p kebab-nli -j 1
CARGO_TARGET_DIR=/build/out/cargo-target/target \
  cargo test -p kebab-rag -j 1
CARGO_TARGET_DIR=/build/out/cargo-target/target \
  cargo test --workspace --no-fail-fast -j 1
CARGO_TARGET_DIR=/build/out/cargo-target/target \
  cargo clippy --workspace --all-targets -j 1 -- -D warnings
```

기존 NLI test 전부 PASS 유지 (e.g. `score_empty_hypothesis_returns_err`, `multi_hop_nli_model_unavailable_refuses`, `nli_model_unavailable_emits_final_stream_event_with_refusal`).

---

## §6. 비범위 (별 task 후보)

1. **Token-count 기반 NLI budget** — `MAX_NLI_PREMISE_CHARS` / `MAX_NLI_HYPOTHESIS_CHARS` 둘 다 *char* 기반. KR SentencePiece tokenization 측정 후 token-count 기반 dynamic budget 으로 전환. `MAX_NLI_PREMISE_CHARS` 의 기존 주석에 이미 *"v0.18.1 candidate"* 명시.

2. **`KEBAB_LOG_STDERR=1` opt-in stderr layer** — diagnostic 시 stderr 로 log 흘려보기. logging.rs 의 fmt::layer 한 줄 추가.

3. **`RefusalReason::NliInputTooLong` 신규 variant** — 현재는 OnlyFirst-err 가 unavailable 로 단일화. 길이 초과만 별 wire variant 로 분리하면 사용자/agent 가 retry 전략 결정 가능 (e.g. answer length cap 후 재요청). **wire schema breaking change** — `answer.v1` enum 확장 = additive minor, OK. 그러나 별 cycle 권장.

4. **NLI multi-model adapter** — sentence-pair NLI 외 token-level entailment scorer / chunk-level entailment scorer 같은 알터너티브.

5. **NLI threshold tuning** — S1 (0.058) / S7 (0.0035) / S10 (0.0028) 모두 *매우 낮은* entailment. default `nli_threshold` 결정 (현재 0 = off). 본 S3 fix 후 truthful score 분포 측정 필요.

6. **gemma3:4b 의 4500-char 답변 자체 억제** — synthesize prompt 의 max-output 가이드 강화. NLI 와 무관한 별 문제.

7. **README / SMOKE 의 tracing 진단 안내** — Option B-1 (한 줄 추가). 별 PR / 별 task.

---

## §7. 합의된 출구 조건

이 spec 이 closed 되려면:

1. Fix PR 머지 — round-2 critic RM1 closure: 정확한 신규 심볼 list (frontmatter `fix_symbols` verbatim):
   - `kebab-nli/src/lib.rs::NliVerifier::hypothesis_token_count` (trait method, default impl `Ok(0)` for backward-compat).
   - `kebab-nli/src/onnx.rs::OnnxNliVerifier::hypothesis_token_count` (override + `HYPOTHESIS_TOKEN_BUDGET = 256` const).
   - `kebab-rag/src/pipeline.rs::MAX_NLI_HYPOTHESIS_CHARS_INITIAL = 1200` + `MAX_NLI_HYPOTHESIS_CHARS_MIN = 150` consts.
   - `kebab-rag/src/pipeline.rs::truncate_hypothesis_for_nli_with_budget(verifier, hypothesis)` helper + step 8.5 hook 의 callsite 수정.
   - `kebab-rag/tests/common/mod.rs::SpyNliVerifier` helper (closure spy + token_count_fn).
2. Unit test 3개 (5.1 ignored / 5.2 pure-fn / 5.3 mock multi-hop) 추가 + 모두 GREEN.
3. Dogfood S3 retest GREEN — `nli_model_unavailable` → `nli_verification_failed` 또는 happy path 로 전환 확인.
4. `cargo test --workspace --no-fail-fast -j 1` GREEN — 기존 NLI test 전부 PASS, 신규 회귀 0.
5. `cargo clippy --workspace --all-targets -j 1 -- -D warnings` clean.
6. `tasks/HOTFIXES.md` 에 신규 dated entry **`## 2026-05-26 — S3 NLI unavailable — hypothesis truncate + token-count fallback`** 추가 (date-top convention). round-1 critic M6 closure: **HOTFIX 번호 부여 안 함** (HOTFIX #15 처럼 fixture-issue 가 아닌 *production behavior fix* — sibling fb-41 PR-9 closure entry 와 같은 layer 의 follow-up). line 17 (현재 HOTFIX #15) 직전, 그 직전이 2026-05-25 fb-41 entry. 형식: Symptom / Root cause / Action / Amends 4-block — HOTFIX #15 entry 와 동일 sibling pattern.

---

## §8. Risk

- **Risk level: 낮음.** fix 가 NLI 입력 정규화 layer (pipeline-side helper + nli crate 의 token-count probe) 만 건드림. Wire 변경 없음, behavior 는 false-negative (정상 답변이 unavailable 로 거부) 줄어듦 + graceful fallback (worst case 시 기존 unavailable 유지 = regression 0).
- **Char-budget 단독 부족 (round-1 critic H3 + verifier Blocker 2 closure)**: Option A 의 token-count fallback retry 가 KR safe 보장. budget 1200 → 600 → 300 → 150 chars 까지 retry → 최악 fallback 시 `nli_model_unavailable` (현재 동작 유지).
- **Hypothesis truncate direction `Right`** (앞부분 보존, 뒤 손실): LLM 답변의 *도입부* 가 핵심 claim. *결론부* 는 보통 도입부 재요약 — Right direction 이 안전한 default. round-1 critic M2 의 test pin (`long_en_synth_answer_truncated_before_nli_call` 의 `assert_eq!(hyp.as_str(), input_first_1200)`) 으로 회귀 detection.

### Pre-mortem (round-1 critic M3 closure)

| 시나리오 | 영향 | Mitigation |
|---|---|---|
| **(a) KR-extreme hypothesis (e.g. 한자/CJK character density)** — char-truncate + retry 모두 token count 초과 | budget 소진 → graceful `nli_model_unavailable` fallback | spec §3 의 `MAX_NLI_HYPOTHESIS_CHARS_MIN = 150` floor + §6 #1 의 token-count budget 별 task. 현재는 graceful refuse (regression 0). |
| **(b) Conclusion-bearing hypothesis** — LLM 의 핵심 claim 이 *후반부* (예: "Therefore, X" 단락 답변 끝) | Right truncation 으로 conclusion 손실 → entailment score 낮아짐 → false `nli_verification_failed` 가능 | NLI entailment 는 *front-loaded fact 위주* — 보통 도입부 claim 만으로 충분. Worst case 도 *false positive (잘못된 entailment passed)* 아닌 *false negative (정상이지만 reject)* — fail-closed semantic 보존. 사용자가 `nli_threshold = 0` 으로 임시 disable + retry 가능. |
| **(c) Grapheme cluster split** (emoji ZWJ, KR 조합형 NFD 분해 등) — `chars().take(N)` 가 grapheme 중간 split | tokenizer 가 UNK 폭주 → score 왜곡 | mDeBERTa SentencePiece 가 codepoint-level subword — grapheme split 영향 미미 (worst case UNK 1-2 개, NLI signal 의미 보존). |
| **(d) `hypothesis_token_count` probe 자체 fail** (e.g. tokenizer crate panic on edge input) | `with_context` 가 anyhow err 로 wrap → graceful fallback | empty hypothesis 는 `acc.trim().is_empty()` guard 가 step 8.5 진입 전 차단. 비-empty 의 edge input 은 mDeBERTa tokenizer 가 robust 처리 (기존 PR-9b unit test 의 long-premise / empty-hypothesis 검증).
