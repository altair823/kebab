//! `kebab-nli` — NLI-based post-synthesis verification for multi-hop RAG.
//!
//! fb-41 introduces a mDeBERTa-v3 XNLI verifier that runs on
//! `(packed_chunks, generated_answer)` after synthesize. If
//! `NliScores::faithfulness()` < threshold the rag crate refuses the answer
//! with `NliVerificationFailed`. PR-9a (this file) is the trait surface +
//! scaffolding only — `OnnxNliVerifier::score` returns a stub error until
//! PR-9b adds the real ONNX inference path.

use serde::{Deserialize, Serialize};

pub mod onnx;

pub use onnx::OnnxNliVerifier;

/// Three-channel XNLI output. Channel order matches the standard XNLI
/// `id2label` mapping `[entailment, neutral, contradiction]` shipped with
/// the Xenova mDeBERTa-v3 model.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct NliScores {
    pub entailment: f32,
    pub neutral: f32,
    pub contradiction: f32,
}

impl NliScores {
    /// Faithfulness score = entailment channel. The rag crate compares this
    /// against `rag.nli_threshold` to decide whether to refuse.
    pub fn faithfulness(&self) -> f32 {
        self.entailment
    }

    /// Wrap raw XNLI logits (`[entailment, neutral, contradiction]`) into
    /// a normalised `NliScores`. Applies a numerically-stable softmax3.
    pub fn from_xnli_logits(logits: [f32; 3]) -> Self {
        let probs = softmax3(logits);
        Self {
            entailment: probs[0],
            neutral: probs[1],
            contradiction: probs[2],
        }
    }
}

/// Abstract NLI verifier. `score` is called with `(premise = packed chunks,
/// hypothesis = generated answer)` — the standard NLI direction (premise
/// entails hypothesis ⇒ answer is grounded in retrieved evidence).
pub trait NliVerifier: Send + Sync {
    fn score(&self, premise: &str, hypothesis: &str) -> anyhow::Result<NliScores>;

    /// Probe-only tokenize for caller-side budget verification. S3
    /// follow-up (2026-05-26) — pipeline 의 char-budget retry loop 가
    /// 이 API 로 mDeBERTa-v3 의 `OnlyFirst` dead-end (hypothesis 단독이
    /// 512-token cap 초과 시 truncate 불가) 를 회피.
    ///
    /// Required: `OnnxNliVerifier` 는 real tokenizer 로 *trait impl 블록
    /// 안에서* 구현해야 함 — inherent method 는 vtable 미등록 → trait
    /// dispatch 시 호출 안 됨 → production silent NO-OP.
    fn hypothesis_token_count(&self, hypothesis: &str) -> anyhow::Result<usize>;
}

/// Numerically stable 3-way softmax (subtract max for log-sum-exp safety).
/// Private — call sites should go through `NliScores::from_xnli_logits`.
fn softmax3(logits: [f32; 3]) -> [f32; 3] {
    let max = logits[0].max(logits[1]).max(logits[2]);
    let e0 = (logits[0] - max).exp();
    let e1 = (logits[1] - max).exp();
    let e2 = (logits[2] - max).exp();
    let sum = e0 + e1 + e2;
    [e0 / sum, e1 / sum, e2 / sum]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx_eq(a: f32, b: f32, eps: f32) -> bool {
        (a - b).abs() <= eps
    }

    #[test]
    fn softmax3_normalises_to_unit() {
        let p = softmax3([1.0, 2.0, 3.0]);
        assert!(p.iter().all(|x| *x > 0.0));
        assert!(approx_eq(p[0] + p[1] + p[2], 1.0, 1e-6));
        // Monotonic: larger logit ⇒ larger probability.
        assert!(p[0] < p[1] && p[1] < p[2]);
    }

    #[test]
    fn softmax3_is_invariant_to_constant_shift() {
        let a = softmax3([1.0, 2.0, 3.0]);
        let b = softmax3([101.0, 102.0, 103.0]);
        for i in 0..3 {
            assert!(
                approx_eq(a[i], b[i], 1e-6),
                "channel {i} drifted: a={a:?} b={b:?}"
            );
        }
    }

    #[test]
    fn nli_scores_from_xnli_logits_orders_correctly() {
        // entailment dominates ⇒ entailment is the max probability channel.
        let s = NliScores::from_xnli_logits([5.0, 1.0, 0.5]);
        assert!(s.entailment > s.neutral);
        assert!(s.entailment > s.contradiction);
        assert!(approx_eq(
            s.entailment + s.neutral + s.contradiction,
            1.0,
            1e-6
        ));
    }

    #[test]
    fn faithfulness_returns_entailment_channel() {
        let s = NliScores {
            entailment: 0.7,
            neutral: 0.2,
            contradiction: 0.1,
        };
        assert!(approx_eq(s.faithfulness(), 0.7, f32::EPSILON));
    }
}
