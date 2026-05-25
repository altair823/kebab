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
    assert!(
        s.entailment < 0.3,
        "expected entailment < 0.3, got {:.4} (full scores: {:?})",
        s.entailment,
        s
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
            "channel {name} non-finite: {x} (full scores: {:?})",
            s
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
