//! ONNX-backed `NliVerifier` adapter (mDeBERTa-v3 XNLI).
//!
//! PR-9b: real implementation. `new` resolves the cache directory from
//! `config.storage.model_dir/nli/<sanitized-model-id>/` (matching the
//! fastembed adapter's pattern of `model_dir/fastembed/`) and stamps it
//! on `self`. The (potentially network-bound) model + tokenizer download
//! is deferred to the first `score` call via `OnceLock<Session>` /
//! `OnceLock<Tokenizer>` — keeping `new` cheap so the rag crate can
//! construct the verifier eagerly during `App` boot without paying for
//! a model load on every CLI invocation.
//!
//! Per design §2.2.2 (Lazy init), §2.2.3 (truncation = `OnlyFirst`,
//! premise truncates, hypothesis preserved). PR-9c-1 will wire the
//! `[models.nli]` config section; until then the model id is hard-coded
//! to the Xenova mDeBERTa-v3 XNLI multilingual checkpoint.

use std::path::PathBuf;
use std::sync::OnceLock;

use anyhow::{Context, Result, anyhow};
use kebab_config::expand_path;
use ort::session::Session;
use tokenizers::{
    Tokenizer, TruncationDirection, TruncationParams, TruncationStrategy,
};

use crate::{NliScores, NliVerifier};

/// Default HuggingFace model id for the XNLI verifier. PR-9c-1 will
/// replace this constant with a `config.models.nli.model` lookup once
/// the `NliCfg` section lands. The Xenova repo packages the
/// mDeBERTa-v3-base XNLI multilingual checkpoint as ONNX under the
/// `onnx/model.onnx` path; the tokenizer ships at `tokenizer.json`.
const DEFAULT_MODEL_ID: &str = "Xenova/mDeBERTa-v3-base-xnli-multilingual-nli-2mil7";

/// Filename inside the HF repo (NOT a path on disk).
const HF_MODEL_FILE: &str = "onnx/model.onnx";
/// Filename inside the HF repo (NOT a path on disk).
const HF_TOKENIZER_FILE: &str = "tokenizer.json";

/// Subdirectory under `config.storage.model_dir` where the NLI adapter
/// writes / reads ONNX + tokenizer files. Mirrors the fastembed
/// adapter's `model_dir/fastembed/` layout.
const NLI_CACHE_SUBDIR: &str = "nli";

/// XNLI label order in the Xenova mDeBERTa-v3 checkpoint: the model's
/// output logits are `[entailment, neutral, contradiction]`. Pinned as
/// a constant so a future model swap (different label order) is a
/// single-site change.
const LOGITS_LEN: usize = 3;

/// Max input length passed to the tokenizer. mDeBERTa-v3 is trained
/// at 512-token context, matches the Xenova ONNX export's positional
/// embedding shape. `OnlyFirst` strategy makes the premise (which is
/// allowed to be the packed-chunks context) absorb the truncation;
/// the hypothesis (the generated answer) is preserved.
const MAX_TOKENS: usize = 512;

/// ONNX-runtime mDeBERTa-v3 XNLI verifier.
///
/// `session` + `tokenizer` are lazily populated by the first call to
/// `ensure_loaded`. `new` is eager only for cache_dir create_dir_all
/// (cheap) so that the rag crate can construct an instance during
/// `App` boot without paying for the ~280 MB model download.
pub struct OnnxNliVerifier {
    model_id: String,
    cache_dir: PathBuf,
    session: OnceLock<Session>,
    tokenizer: OnceLock<Tokenizer>,
}

impl OnnxNliVerifier {
    /// Construct a verifier from the user's `Config`. Eagerly resolves
    /// `cache_dir = config.storage.model_dir/nli/<sanitized-model-id>/`
    /// and runs `create_dir_all` so the first `score` call can drop
    /// straight into download + load without re-deriving paths.
    ///
    /// PR-9c-1 will swap `DEFAULT_MODEL_ID` for `config.models.nli.model`.
    pub fn new(config: &kebab_config::Config) -> Result<Self> {
        let model_id = DEFAULT_MODEL_ID.to_string();

        // Match kebab-embed-local's two-step expansion: data_dir first,
        // then model_dir with `{data_dir}` substituted in.
        let data_dir = expand_path(&config.storage.data_dir, "");
        let model_dir = expand_path(&config.storage.model_dir, &data_dir.to_string_lossy());
        let cache_dir = model_dir
            .join(NLI_CACHE_SUBDIR)
            .join(sanitize_model_id(&model_id));
        std::fs::create_dir_all(&cache_dir)
            .with_context(|| format!("create kebab-nli cache dir {}", cache_dir.display()))?;

        Ok(Self {
            model_id,
            cache_dir,
            session: OnceLock::new(),
            tokenizer: OnceLock::new(),
        })
    }

    /// Download (if needed) + load the ONNX session and tokenizer on
    /// first call; return cached refs on subsequent calls. Uses two
    /// `OnceLock`s rather than one because a single `OnceLock<(_, _)>`
    /// would need to construct both atomically — keeping them split
    /// lets us short-circuit on the (rare) hit path where only one
    /// side is missing.
    ///
    /// `OnceLock::get_or_try_init` is still unstable (rust-lang/rust#109737)
    /// so we implement the fallible init by hand: probe `get`, on miss
    /// compute the value, then `set` it. The race between two threads is
    /// resolved by `OnceLock::set` — the loser gets `Err`, falls through
    /// to a second `get`, and reads the winner's value. Each thread that
    /// races + loses does pay the cost of one redundant download (rare in
    /// practice: rag boot is single-threaded today), but the cache stays
    /// consistent.
    fn ensure_loaded(&self) -> Result<(&Session, &Tokenizer)> {
        if self.session.get().is_none() {
            let s = self.load_session()?;
            let _ = self.session.set(s); // loser of a race: discard local value
        }
        if self.tokenizer.get().is_none() {
            let t = self.load_tokenizer()?;
            let _ = self.tokenizer.set(t);
        }
        // Both OnceLocks are populated at this point; `expect` is a
        // tighter post-condition than `unwrap_or_else` would be.
        let session = self.session.get().expect("session populated above");
        let tokenizer = self.tokenizer.get().expect("tokenizer populated above");
        Ok((session, tokenizer))
    }

    /// Build an `hf_hub::api::sync::Api` rooted at `self.cache_dir` and
    /// fetch `filename` from `self.model_id`. Logs cache hits at INFO
    /// so a user reading kebab logs can see which artifact source the
    /// pipeline picked.
    fn fetch(&self, filename: &str) -> Result<PathBuf> {
        let api = hf_hub::api::sync::ApiBuilder::new()
            .with_cache_dir(self.cache_dir.clone())
            .build()
            .with_context(|| {
                format!(
                    "kebab-nli: hf-hub ApiBuilder::build failed (cache_dir={})",
                    self.cache_dir.display()
                )
            })?;
        let repo = api.model(self.model_id.clone());

        // `ApiRepo::get` returns the local path if cached, otherwise
        // downloads. We can't tell after the fact whether the file
        // was already cached without an extra `Cache::repo::get`
        // probe, so do that probe first to emit the right log line.
        let cache_path = api
            .repo(hf_hub::Repo::new(
                self.model_id.clone(),
                hf_hub::RepoType::Model,
            ))
            .get(filename)
            .ok();
        if cache_path.is_some() {
            tracing::info!(
                target: "kebab-nli",
                model_id = %self.model_id,
                file = %filename,
                "NLI artifact cache hit"
            );
        } else {
            tracing::info!(
                target: "kebab-nli",
                model_id = %self.model_id,
                file = %filename,
                cache_dir = %self.cache_dir.display(),
                "downloading NLI artifact"
            );
        }

        repo.get(filename).with_context(|| {
            format!(
                "kebab-nli: hf-hub fetch failed for {filename} (model_id={}, cache_dir={})",
                self.model_id,
                self.cache_dir.display()
            )
        })
    }

    fn load_session(&self) -> Result<Session> {
        tracing::info!(
            target: "kebab-nli",
            model_id = %self.model_id,
            "downloading NLI model + tokenizer (first run only)"
        );
        let model_path = self.fetch(HF_MODEL_FILE)?;
        let session = Session::builder()
            .with_context(|| "kebab-nli: ort Session::builder failed")?
            .commit_from_file(&model_path)
            .with_context(|| {
                format!(
                    "kebab-nli: ort Session::commit_from_file({}) failed",
                    model_path.display()
                )
            })?;
        tracing::info!(
            target: "kebab-nli",
            model_id = %self.model_id,
            model_path = %model_path.display(),
            "NLI model ready"
        );
        Ok(session)
    }

    fn load_tokenizer(&self) -> Result<Tokenizer> {
        let tokenizer_path = self.fetch(HF_TOKENIZER_FILE)?;
        let mut tokenizer = Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| anyhow!("kebab-nli: Tokenizer::from_file({}) failed: {e}", tokenizer_path.display()))?;
        tokenizer
            .with_truncation(Some(TruncationParams {
                max_length: MAX_TOKENS,
                strategy: TruncationStrategy::OnlyFirst,
                stride: 0,
                direction: TruncationDirection::Right,
            }))
            .map_err(|e| anyhow!("kebab-nli: Tokenizer::with_truncation failed: {e}"))?;
        Ok(tokenizer)
    }
}

impl NliVerifier for OnnxNliVerifier {
    fn score(&self, premise: &str, hypothesis: &str) -> Result<NliScores> {
        // Defense-in-depth: spec §2.3 has the caller skip empty answers,
        // but a degenerate empty hypothesis here would tokenize to a
        // [CLS][SEP][SEP] triple that yields a near-uniform softmax —
        // misleading both faithfulness gate and any future logging.
        if hypothesis.trim().is_empty() {
            anyhow::bail!("kebab-nli: empty hypothesis");
        }

        let (session, tokenizer) = self.ensure_loaded()?;

        let enc = tokenizer
            .encode((premise, hypothesis), true)
            .map_err(|e| anyhow!("kebab-nli: tokenizer.encode failed: {e}"))?;

        let ids: Vec<i64> = enc.get_ids().iter().map(|&u| u as i64).collect();
        let mask: Vec<i64> = enc
            .get_attention_mask()
            .iter()
            .map(|&u| u as i64)
            .collect();
        let seq_len = ids.len();

        // mDeBERTa-v3 ONNX export expects [batch, seq_len] for both
        // input_ids and attention_mask. We always feed batch=1.
        let ids_arr = ndarray::Array2::from_shape_vec((1, seq_len), ids)
            .with_context(|| "kebab-nli: input_ids ndarray shape build failed")?;
        let mask_arr = ndarray::Array2::from_shape_vec((1, seq_len), mask)
            .with_context(|| "kebab-nli: attention_mask ndarray shape build failed")?;

        let outputs = session
            .run(ort::inputs! {
                "input_ids" => ids_arr,
                "attention_mask" => mask_arr,
            }?)
            .with_context(|| "kebab-nli: ort Session::run failed")?;

        let logits = outputs["logits"]
            .try_extract_tensor::<f32>()
            .with_context(|| "kebab-nli: logits try_extract_tensor::<f32> failed")?;

        // Expected shape [1, 3]. Defensive check — a model swap with a
        // different head would silently produce wrong scores otherwise.
        let shape = logits.shape();
        if shape != [1, LOGITS_LEN] {
            anyhow::bail!(
                "kebab-nli: unexpected logits shape {:?}, expected [1, {LOGITS_LEN}]",
                shape
            );
        }
        let l = [logits[[0, 0]], logits[[0, 1]], logits[[0, 2]]];
        Ok(NliScores::from_xnli_logits(l))
    }
}

/// Make a HuggingFace model id (`"owner/repo"`) into a single
/// path component safe to use as a directory name. `/` → `_` is
/// enough for current ids; if more exotic chars appear we'll
/// widen this then.
fn sanitize_model_id(s: &str) -> String {
    s.replace('/', "_")
}

#[cfg(test)]
mod tests {
    use super::*;
    use kebab_config::Config;

    #[test]
    fn new_succeeds_on_default_config() {
        let cfg = Config::defaults();
        let v = OnnxNliVerifier::new(&cfg).expect("new should succeed on default config");
        // cache_dir must include the sanitized model id (no '/').
        let s = v.cache_dir.to_string_lossy();
        assert!(s.contains(NLI_CACHE_SUBDIR), "cache_dir lacks nli/: {s}");
        assert!(
            !s.contains("Xenova/mDeBERTa"),
            "cache_dir must sanitize '/' in model id: {s}"
        );
        assert!(
            s.contains("Xenova_mDeBERTa"),
            "cache_dir should contain sanitized id: {s}"
        );
    }

    /// Empty hypothesis takes the defense-in-depth early bail path —
    /// reaches no model load, so this is a pure unit test (no network).
    /// Replaces PR-9a's `score_returns_err_in_skeleton` (stub-only).
    #[test]
    fn score_empty_hypothesis_returns_err() {
        let cfg = Config::defaults();
        let v = OnnxNliVerifier::new(&cfg).unwrap();
        let err = v.score("anything", "").expect_err("empty hypothesis must error");
        assert!(
            err.to_string().contains("empty hypothesis"),
            "unexpected error message: {err}"
        );
    }
}
