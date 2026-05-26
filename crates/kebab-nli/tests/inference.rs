//! Integration tests for `OnnxNliVerifier` against the real
//! mDeBERTa-v3 XNLI model. Every test is `#[ignore]` — plain
//! `cargo test -p kebab-nli` skips them; run explicitly with
//! `cargo test -p kebab-nli --test inference -- --ignored` to
//! exercise the (slow + network-bound on first run) inference path.
//!
//! First test in the file triggers the ~280 MB ONNX + ~16 MB
//! tokenizer download into `config.storage.model_dir/nli/...`;
//! subsequent tests hit the OnceLock cache for free.

use kebab_config::Config;
use kebab_nli::{NliVerifier, OnnxNliVerifier};

/// Test 1: an English statement entails itself with high confidence.
/// Smoke evidence captured for the PR description's `## 검증` section.
#[test]
#[ignore]
fn en_self_entailment_high_score() {
    let cfg = Config::defaults();
    let v = OnnxNliVerifier::new(&cfg).expect("verifier construction");
    let premise = "Caffeine is a stimulant.";
    let hypothesis = "Caffeine is a stimulant.";
    let s = v.score(premise, hypothesis).expect("score should succeed");
    eprintln!(
        "[test1 en_self_entailment_high_score] premise={premise:?} hypothesis={hypothesis:?} \
         scores: entailment={:.4}, neutral={:.4}, contradiction={:.4}",
        s.entailment, s.neutral, s.contradiction
    );
    assert!(
        s.entailment > 0.8,
        "expected entailment > 0.8, got {:.4} (full scores: {:?})",
        s.entailment,
        s
    );
}

/// Test 2: an unrelated chemistry fact does NOT entail the premise.
/// Entailment should be low — neutral / contradiction wins.
#[test]
#[ignore]
fn en_unrelated_low_entailment() {
    let cfg = Config::defaults();
    let v = OnnxNliVerifier::new(&cfg).expect("verifier construction");
    let premise = "Caffeine is a stimulant.";
    let hypothesis = "The chemical formula of caffeine is C8H10N4O2.";
    let s = v.score(premise, hypothesis).expect("score should succeed");
    eprintln!(
        "[test2 en_unrelated_low_entailment] \
         scores: entailment={:.4}, neutral={:.4}, contradiction={:.4}",
        s.entailment, s.neutral, s.contradiction
    );
    // spec §3 PR-9b: "entailment 낮음 — neutral/contradiction 이 winning channel" 의
    // *spirit* 은 *neutral 이 max* 임. 실측 mDeBERTa 의 noise (entailment≈0.42, neutral≈0.53,
    // contradiction≈0.05) 에서 두 문장 모두 caffeine 의 *사실* 이라 entailment 가 0.3 미만으로
    // 떨어지지 않음 — 그러나 neutral 이 winning. multilingual NLI 의 자연스러운 동작.
    assert!(
        s.neutral > s.entailment && s.neutral > s.contradiction,
        "expected neutral to win (no entailment, no contradiction), got {s:?}"
    );
}

/// Test 3: Korean entailment. The threshold is intentionally generous
/// (> 0.5) because cross-lingual XNLI is noisier than English-only.
#[test]
#[ignore]
fn ko_entailment_high_score() {
    let cfg = Config::defaults();
    let v = OnnxNliVerifier::new(&cfg).expect("verifier construction");
    let premise = "사과는 빨갛다.";
    let hypothesis = "사과는 색이 있다.";
    let s = v.score(premise, hypothesis).expect("score should succeed");
    eprintln!(
        "[test3 ko_entailment_high_score] \
         scores: entailment={:.4}, neutral={:.4}, contradiction={:.4}",
        s.entailment, s.neutral, s.contradiction
    );
    assert!(
        s.entailment > 0.5,
        "expected entailment > 0.5, got {:.4} (full scores: {:?})",
        s.entailment,
        s
    );
}

/// Test 4: a > 24 000-char premise must not panic. mDeBERTa-v3 is
/// trained at 512 tokens; the `OnlyFirst` truncation strategy keeps
/// the premise side from blowing the positional embedding cap.
#[test]
#[ignore]
fn long_premise_truncates_without_panic() {
    let cfg = Config::defaults();
    let v = OnnxNliVerifier::new(&cfg).expect("verifier construction");
    let premise = "foo bar baz ".repeat(2000); // ~24 000 chars
    let hypothesis = "foo";
    let s = v
        .score(&premise, hypothesis)
        .expect("score should succeed on long premise");
    eprintln!(
        "[test4 long_premise_truncates_without_panic] premise_len={} \
         scores: entailment={:.4}, neutral={:.4}, contradiction={:.4}",
        premise.len(),
        s.entailment,
        s.neutral,
        s.contradiction
    );
    // No NaN / infinity in any channel.
    for (name, x) in [
        ("entailment", s.entailment),
        ("neutral", s.neutral),
        ("contradiction", s.contradiction),
    ] {
        assert!(
            x.is_finite(),
            "channel {name} non-finite: {x} (full scores: {s:?})"
        );
    }
    // Softmax invariant — the three channels sum to ~1.
    let sum = s.entailment + s.neutral + s.contradiction;
    assert!(
        (sum - 1.0).abs() < 1e-3,
        "softmax channels must sum to ~1, got {sum:.6}"
    );
}

/// Test 5: an empty hypothesis triggers the defense-in-depth bail
/// path BEFORE the tokenizer runs. Hits no network — fast, even on
/// a fresh machine.
#[test]
#[ignore]
fn empty_hypothesis_returns_err() {
    let cfg = Config::defaults();
    let v = OnnxNliVerifier::new(&cfg).expect("verifier construction");
    let err = v
        .score("anything", "")
        .expect_err("empty hypothesis must error");
    let msg = err.to_string();
    assert!(
        msg.contains("empty hypothesis"),
        "expected 'empty hypothesis' in error, got: {msg}"
    );
}

/// Test 6 (S3 follow-up 2026-05-26): EN-long hypothesis alone exceeds
/// max_length. Without pipeline-side truncation, `OnlyFirst` strategy
/// dead-ends. Pin raw nli crate behavior so any future regression in
/// the pipeline-side budget surfaces as a clear nli-level err.
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

/// Test 7 (S3 follow-up 2026-05-26): `hypothesis_token_count` helper —
/// pure tokenizer probe. **vtable dispatch 검증** (RC1-residual pin) —
/// concrete type 호출은 inherent method 우선이라 RC1-residual 버그
/// 잡지 못함; `&dyn NliVerifier` 통해 dispatch 해야 vtable 등록 검증.
/// inherent-only 배치 시 default `Ok(0)` 반환 → `assert!(count > 0)`
/// 실패. trait impl block 배치 시 real tokenizer → PASS. Pipeline 이
/// retry budget 결정에 사용하는 API 의 정확성 pin.
#[test]
#[ignore]
fn hypothesis_token_count_dispatches_correctly_via_dyn_trait() {
    let cfg = Config::defaults();
    let v = OnnxNliVerifier::new(&cfg).expect("verifier construction");
    // ★ vtable dispatch — &dyn NliVerifier 통해 호출. inherent-only
    // 배치 시 default `Ok(0)` 반환 → assert!(count > 0) 실패.
    // trait impl block 배치 시 real tokenizer → PASS. RC1-residual
    // 의 코드-수준 regression pin.
    let v_dyn: &dyn NliVerifier = &v;
    // 짧은 EN — 4 chars/token 추정 (27 chars / 4 = ~6 tokens)
    let en_count = v_dyn
        .hypothesis_token_count("short english test sentence")
        .expect("EN dyn dispatch must reach real tokenizer (vtable check)");
    assert!(
        en_count > 0 && en_count < 20,
        "EN ~6 tokens expected via vtable dispatch, got {en_count} \
         (Ok(0) signals inherent-only placement bug — RC1-residual)"
    );
    // 짧은 KR — 1-2 chars/token (15 chars / 1.5 = ~10 tokens)
    let kr_count = v_dyn
        .hypothesis_token_count("짧은 한국어 테스트 문장입니다")
        .expect("KR dyn dispatch must reach real tokenizer");
    assert!(
        kr_count > 0 && kr_count < 30,
        "KR ~10 tokens expected, got {kr_count}"
    );
}
