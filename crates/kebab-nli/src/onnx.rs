//! ONNX-backed `NliVerifier` adapter (mDeBERTa-v3 XNLI).
//!
//! `new` resolves the cache directory from
//! `config.storage.model_dir/nli/<sanitized-model-id>/` (matching the
//! fastembed adapter's pattern of `model_dir/fastembed/`) and stamps it
//! on `self`. The (potentially network-bound) model + tokenizer download
//! is deferred to the first `score` call via `OnceLock<Session>` /
//! `OnceLock<Tokenizer>` — keeping `new` cheap so the rag crate can
//! construct the verifier eagerly during `App` boot without paying for
//! a model load on every CLI invocation.
//!
//! Per design §2.2.2 (Lazy init), §2.2.3 (truncation = `OnlyFirst`,
//! premise truncates, hypothesis preserved). The model id flows from
//! `config.models.nli.model`; `config.models.nli.provider` selects the
//! verifier impl (only `"onnx"` is implemented in v0.18).

use std::path::PathBuf;
use std::sync::OnceLock;

use anyhow::{Context, Result, anyhow};
use kebab_config::expand_path;
use ort::session::Session;
use tokenizers::{Tokenizer, TruncationDirection, TruncationParams, TruncationStrategy};

use crate::{NliScores, NliVerifier};

/// Filename inside the HF repo (NOT a path on disk). The Xenova repo
/// packages the mDeBERTa-v3-base XNLI multilingual checkpoint (the
/// default `config.models.nli.model` — see `kebab-config::NliCfg::defaults`)
/// as ONNX under this path; the tokenizer ships at `tokenizer.json`.
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
    /// Hypothesis-side budget. Pipeline 의
    /// `truncate_hypothesis_for_nli_with_budget` retry loop 가 char-truncate
    /// 후 token-count 재검증 시 이 값을 cap 으로 사용. = `MAX_TOKENS`
    /// (512) - 3 special tokens reserved (CLS, SEP, SEP) - 253 premise
    /// room (caller decides). 안전 마진 (S3 follow-up 2026-05-26).
    pub const HYPOTHESIS_TOKEN_BUDGET: usize = 256;

    /// Construct a verifier from the user's `Config`. Eagerly resolves
    /// `cache_dir = config.storage.model_dir/nli/<sanitized-model-id>/`
    /// and runs `create_dir_all` so the first `score` call can drop
    /// straight into download + load without re-deriving paths.
    ///
    /// Reads `config.models.nli.model` for the HuggingFace model id
    /// and `config.models.nli.provider` to select the verifier impl —
    /// only `"onnx"` is implemented in v0.18. The defaults live in
    /// `kebab-config::NliCfg::defaults` so this path always receives
    /// a non-empty model id.
    pub fn new(config: &kebab_config::Config) -> Result<Self> {
        let provider = config.models.nli.provider.as_str();
        if provider != "onnx" {
            anyhow::bail!(
                "kebab-nli: unsupported provider {provider:?} (only 'onnx' is implemented in v0.18)"
            );
        }
        let model_id = config.models.nli.model.clone();

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
        // Round-1 review N1 fix: `Api::get` triggers download on miss,
        // so we can't use it as a hit probe. `Cache::get` is fs-only —
        // returns Some(path) if cached, None otherwise. No network.
        let repo = hf_hub::Repo::new(self.model_id.clone(), hf_hub::RepoType::Model);
        let cached = hf_hub::Cache::new(self.cache_dir.clone())
            .repo(repo.clone())
            .get(filename)
            .is_some();
        if cached {
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

        let api = hf_hub::api::sync::ApiBuilder::new()
            .with_cache_dir(self.cache_dir.clone())
            .build()
            .with_context(|| {
                format!(
                    "kebab-nli: hf-hub ApiBuilder::build failed (cache_dir={})",
                    self.cache_dir.display()
                )
            })?;
        api.model(self.model_id.clone())
            .get(filename)
            .with_context(|| {
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
        let mut tokenizer = Tokenizer::from_file(&tokenizer_path).map_err(|e| {
            anyhow!(
                "kebab-nli: Tokenizer::from_file({}) failed: {e}",
                tokenizer_path.display()
            )
        })?;
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

        let ids: Vec<i64> = enc.get_ids().iter().map(|&u| i64::from(u)).collect();
        let mask: Vec<i64> = enc
            .get_attention_mask()
            .iter()
            .map(|&u| i64::from(u))
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
                "kebab-nli: unexpected logits shape {shape:?}, expected [1, {LOGITS_LEN}]"
            );
        }
        let l = [logits[[0, 0]], logits[[0, 1]], logits[[0, 2]]];
        Ok(NliScores::from_xnli_logits(l))
    }

    /// **Override** the trait default `Ok(0)` with a real mDeBERTa
    /// tokenize. Pipeline 의 `truncate_hypothesis_for_nli_with_budget`
    /// retry loop 가 이 method 를 vtable 통해 호출 — production code
    /// path 에서 실 token count 측정.
    ///
    /// **CRITICAL placement**: 이 method 는 *trait impl block 안* 에
    /// 위치해야 vtable 에 등록 — inherent `impl OnnxNliVerifier {}` 안에
    /// 두면 dispatch 시 trait default (`Ok(0)`) 호출 → retry loop
    /// 즉시 통과 → production silent NO-OP (S3 follow-up 2026-05-26
    /// RC1-residual closure).
    fn hypothesis_token_count(&self, hypothesis: &str) -> Result<usize> {
        let (_session, tokenizer) = self.ensure_loaded()?;
        let enc = tokenizer
            .encode(hypothesis, /*add_special_tokens=*/ false)
            .map_err(|e| anyhow!("kebab-nli: tokenizer.encode (probe) failed: {e}"))?;
        Ok(enc.get_ids().len())
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
    use tempfile::TempDir;

    /// Round-1 review N2 fix: redirect Config.storage.{data,model}_dir
    /// into a tempdir so unit tests don't litter the user's XDG dirs
    /// with empty `nli/` subdirs.
    fn tempdir_config() -> (TempDir, Config) {
        let tmp = TempDir::new().expect("tempdir");
        let mut cfg = Config::defaults();
        cfg.storage.data_dir = tmp.path().to_string_lossy().into_owned();
        cfg.storage.model_dir = "{data_dir}/models".to_string();
        (tmp, cfg)
    }

    #[test]
    fn new_succeeds_on_default_config() {
        let (_tmp, cfg) = tempdir_config();
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
        let (_tmp, cfg) = tempdir_config();
        let v = OnnxNliVerifier::new(&cfg).unwrap();
        let err = v
            .score("anything", "")
            .expect_err("empty hypothesis must error");
        assert!(
            err.to_string().contains("empty hypothesis"),
            "unexpected error message: {err}"
        );
    }

    /// Pins that `config.models.nli.model` flows into `OnnxNliVerifier`
    /// instead of being silently overridden by a hardcoded constant.
    /// `model_id` is a private field, but this test lives in the same
    /// module so it can read it directly — the wiring contract is
    /// "whatever the user puts in TOML / KEBAB_MODELS_NLI_MODEL is the
    /// id the verifier uses".
    #[test]
    fn new_uses_config_model_id() {
        let (_tmp, mut cfg) = tempdir_config();
        cfg.models.nli.model = "custom-org/custom-nli-model".to_string();
        let v = OnnxNliVerifier::new(&cfg).expect("new should succeed with custom model id");
        assert_eq!(v.model_id, "custom-org/custom-nli-model");
        // The custom id also flows into the on-disk cache_dir layout
        // (sanitized so `/` doesn't escape the namespace).
        let s = v.cache_dir.to_string_lossy();
        assert!(
            s.contains("custom-org_custom-nli-model"),
            "cache_dir should embed sanitized custom model id: {s}"
        );
    }

    /// Pins that a non-`"onnx"` provider value errors out at `new` —
    /// the field is no longer silently ignored.
    #[test]
    fn new_rejects_unsupported_provider() {
        let (_tmp, mut cfg) = tempdir_config();
        cfg.models.nli.provider = "candle".to_string();
        let result = OnnxNliVerifier::new(&cfg);
        assert!(result.is_err(), "non-onnx provider must error");
        let msg = result.err().unwrap().to_string();
        assert!(
            msg.contains("unsupported provider") && msg.contains("candle"),
            "error should name the rejected provider: {msg}"
        );
    }

    // ── sanitize_model_id pure-fn coverage ────────────────────────────────
    //
    // Three tests pin the behavior of the private `sanitize_model_id`
    // helper. These are orthogonal to the H1 executor tests above
    // (which cover config-wiring); these cover the transformation
    // contract of the sanitizer itself.

    #[test]
    fn sanitize_model_id_replaces_slash_with_underscore() {
        let input = "Xenova/mDeBERTa-v3-base-xnli-multilingual-nli-2mil7";
        let expected = "Xenova_mDeBERTa-v3-base-xnli-multilingual-nli-2mil7";
        assert_eq!(sanitize_model_id(input), expected);
    }

    #[test]
    fn sanitize_model_id_is_idempotent_on_already_sanitized() {
        // Input with no '/' must come back byte-for-byte unchanged.
        let input = "Xenova_mDeBERTa-v3-base-xnli-multilingual-nli-2mil7";
        assert_eq!(sanitize_model_id(input), input);
    }

    #[test]
    fn sanitize_model_id_leaves_other_chars_untouched() {
        // Hyphens, digits, dots, and underscores must all pass through
        // unchanged — only '/' is replaced with '_'.
        let input = "org_name/model-name_v2.3-alpha";
        let got = sanitize_model_id(input);
        assert_eq!(got, "org_name_model-name_v2.3-alpha");
        assert!(!got.contains('/'), "no slash must remain after sanitize");
        assert!(got.contains('-'), "hyphens must be preserved");
        assert!(got.contains('.'), "dots must be preserved");
        assert!(got.contains('_'), "underscores must be preserved");
    }
}
