//! ONNX-backed `NliVerifier` adapter (mDeBERTa-v3 XNLI).
//!
//! PR-9a: scaffolding only. `new` succeeds against the default `Config`
//! and `score` returns an explicit `"PR-9a stub"` error so any caller that
//! wires this up before PR-9b lands gets a loud failure instead of silent
//! all-zero scores. PR-9b will add ort `Session` + `Tokenizer` lazy init
//! and real inference.

use crate::{NliScores, NliVerifier};

/// ONNX-runtime mDeBERTa-v3 XNLI verifier.
///
/// PR-9a scaffolding holds no state — fields land in PR-9b
/// (`model_id`, `cache_dir`, `session: OnceLock<ort::Session>`,
/// `tokenizer: OnceLock<tokenizers::Tokenizer>`).
pub struct OnnxNliVerifier {
    _private: (),
}

impl OnnxNliVerifier {
    /// Construct a verifier from the user's `Config`. PR-9a always returns
    /// `Ok` because the real model + tokenizer download is deferred to
    /// PR-9b's first `score` call.
    pub fn new(_config: &kebab_config::Config) -> anyhow::Result<Self> {
        Ok(Self { _private: () })
    }
}

impl NliVerifier for OnnxNliVerifier {
    fn score(&self, _premise: &str, _hypothesis: &str) -> anyhow::Result<NliScores> {
        anyhow::bail!("PR-9a stub — ONNX inference lands in PR-9b")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kebab_config::Config;

    #[test]
    fn new_succeeds_on_default_config() {
        let cfg = Config::defaults();
        let v = OnnxNliVerifier::new(&cfg).expect("new should succeed on default config");
        // Silence unused-binding lint without weakening the assertion.
        let _ = &v;
    }

    #[test]
    fn score_returns_err_in_skeleton() {
        let cfg = Config::defaults();
        let v = OnnxNliVerifier::new(&cfg).unwrap();
        let err = v.score("a", "b").expect_err("PR-9a stub must error");
        assert!(
            err.to_string().contains("PR-9a stub"),
            "unexpected error message: {err}"
        );
    }
}
