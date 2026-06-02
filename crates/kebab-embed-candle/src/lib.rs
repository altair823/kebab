//! `kebab-embed-candle` — [`CandleEmbedder`], a pure-Rust (candle)
//! implementation of [`Embedder`](kebab_core::Embedder).
//!
//! Runs the same `intfloat/multilingual-e5-large` model as the default
//! [`FastembedEmbedder`](kebab_embed_local) but through `candle`
//! (`candle-transformers`' XLM-RoBERTa) instead of onnxruntime. Motivation:
//! fastembed 4.9's onnxruntime hard-codes 48 intra-op threads, which corrupts
//! the heap (double-free) on dual-socket NUMA hosts. candle's CPU backend
//! sizes its threads off the global rayon pool, so a one-shot
//! [`rayon::ThreadPoolBuilder`] cap (config `num_threads` / env
//! `KEBAB_EMBED_THREADS`) keeps the worker count NUMA-safe.
//!
//! Output parity with the onnxruntime path was proven by the Phase 0 spike
//! (cosine 1.000000); this crate absorbs that pipeline verbatim:
//!
//! 1. e5 prefix (`passage: ` for documents, `query: ` for queries — the same
//!    convention as `kebab-embed-local`'s `prefix_input`);
//! 2. tokenize (max_len 512, batch-longest padding, special tokens);
//! 3. XLM-RoBERTa forward on `Device::Cpu`;
//! 4. attention-mask-weighted mean pooling;
//! 5. L2 normalization.
//!
//! Model files (`config.json`, `tokenizer.json`, `model.safetensors`) are
//! fetched via `hf-hub` into `{config.storage.model_dir}/candle/`.
//!
//! This crate is **opt-in** (`config.models.embedding.provider = "candle"`);
//! the default provider stays `fastembed`. See
//! `docs/superpowers/specs/2026-06-01-embed-candle-track-spec.md`.

use std::sync::Mutex;

use anyhow::{Context, Result};
use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::xlm_roberta::{Config as XlmConfig, XLMRobertaModel};
use kebab_config::{Config, expand_path};
use kebab_core::{Embedder, EmbeddingInput, EmbeddingKind, EmbeddingModelId, EmbeddingVersion};
use tokenizers::{PaddingParams, PaddingStrategy, Tokenizer, TruncationParams};

/// Subdirectory under `config.storage.model_dir` where the candle adapter
/// caches safetensors + tokenizer. Mirrors `kebab-embed-local`'s
/// `fastembed/` subdir so the two backends never collide.
const CANDLE_CACHE_SUBDIR: &str = "candle";

/// HuggingFace repo id for the multilingual e5 large model. Same weights the
/// onnxruntime path uses, just the safetensors variant candle can read.
const HF_MODEL: &str = "intfloat/multilingual-e5-large";

/// The only `config.models.embedding.model` value the candle adapter accepts
/// (the e5-large weights `HF_MODEL` resolves to). Guards against silently
/// downloading e5-large while `model_id()` reports a different name.
const SUPPORTED_MODEL: &str = "multilingual-e5-large";

/// Token truncation length (e5 was trained at 512).
const MAX_LEN: usize = 512;

/// Env var that overrides `config.models.embedding.num_threads`. Read once in
/// [`CandleEmbedder::new`]; `0`/unset/unparseable means "leave rayon default".
const ENV_EMBED_THREADS: &str = "KEBAB_EMBED_THREADS";

/// Pure-Rust candle adapter. Construct via [`CandleEmbedder::new`]; the
/// constructor downloads the model on first use, so share one instance.
pub struct CandleEmbedder {
    // candle's `forward` is `&self`, but `XLMRobertaModel` is not guaranteed
    // `Sync`; the `Mutex` both supplies that bound and serializes inference
    // (callers batch sequentially anyway — same rationale as
    // `FastembedEmbedder`).
    model: Mutex<XLMRobertaModel>,
    tokenizer: Tokenizer,
    device: Device,
    model_id: EmbeddingModelId,
    version: EmbeddingVersion,
    dimensions: usize,
    batch_size: usize,
}

impl CandleEmbedder {
    /// Build an embedder from `Config`. Applies the NUMA thread cap, fetches
    /// the model into `{model_dir}/candle/`, and validates that the model's
    /// hidden size matches `config.models.embedding.dimensions` before
    /// returning.
    pub fn new(config: &Config) -> Result<Self> {
        // 1. NUMA thread cap. env `KEBAB_EMBED_THREADS` wins over the config
        //    field; `0`/unset leaves rayon's default. `build_global` errors if
        //    the pool was already initialized — intentionally ignored so a
        //    second embedder (or a prior rayon user) is a no-op, not a failure.
        let n_threads = std::env::var(ENV_EMBED_THREADS)
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(config.models.embedding.num_threads as usize);
        if n_threads > 0 {
            if apply_thread_cap(n_threads) {
                tracing::info!(
                    target: "kebab-embed-candle",
                    num_threads = n_threads,
                    "capped global rayon pool for candle CPU backend"
                );
            } else {
                tracing::debug!(
                    target: "kebab-embed-candle",
                    requested = n_threads,
                    "global rayon pool already initialized; thread cap not applied"
                );
            }
        }

        // 1b. Model guard. `HF_MODEL` is hard-coded (candle currently only wires
        //     e5-large), so if the operator configured a *different* model name
        //     we must NOT silently download e5-large and then label its vectors
        //     with the configured name via `model_id()` — that would mislabel
        //     `embedding_version` and corrupt a mixed index. Fail fast, before
        //     the ~2GB download.
        let want = config.models.embedding.model.as_str();
        if want != SUPPORTED_MODEL && want != HF_MODEL {
            anyhow::bail!(
                "candle provider currently supports only '{SUPPORTED_MODEL}' (or \
                 the HF id '{HF_MODEL}'), but config.models.embedding.model = \
                 '{want}'. Use provider=fastembed for other models, or set \
                 model = \"{SUPPORTED_MODEL}\"."
            );
        }

        // 2. Resolve `{data_dir}/models/candle/` exactly like the fastembed
        //    adapter resolves its own subdir.
        let data_dir = expand_path(&config.storage.data_dir, "");
        let model_dir = expand_path(&config.storage.model_dir, &data_dir.to_string_lossy());
        let cache_dir = model_dir.join(CANDLE_CACHE_SUBDIR);
        std::fs::create_dir_all(&cache_dir)
            .with_context(|| format!("create candle cache dir {}", cache_dir.display()))?;

        let device = Device::Cpu;

        // 3. Fetch model files via hf-hub into the candle cache.
        tracing::info!(
            target: "kebab-embed-candle",
            cache_dir = %cache_dir.display(),
            model = HF_MODEL,
            "loading candle embedding model (first run downloads ~2GB safetensors)"
        );
        let api = hf_hub::api::sync::ApiBuilder::new()
            .with_cache_dir(cache_dir.clone())
            .build()
            .context("kb-embed-candle: build hf-hub api")?;
        let repo = api.model(HF_MODEL.to_string());
        let config_path = repo.get("config.json").context("download config.json")?;
        let tokenizer_path = repo
            .get("tokenizer.json")
            .context("download tokenizer.json")?;
        let weights_path = repo
            .get("model.safetensors")
            .context("download model.safetensors")?;

        // 4. Build the candle XLM-RoBERTa model.
        let cfg_json = std::fs::read_to_string(&config_path)
            .with_context(|| format!("read {}", config_path.display()))?;
        let cfg: XlmConfig =
            serde_json::from_str(&cfg_json).context("kb-embed-candle: parse XLM-R config")?;

        // Validate dim BEFORE building the model so a misconfigured
        // `dimensions` fails cheaply (matches FastembedEmbedder's contract).
        check_dim(cfg.hidden_size, config.models.embedding.dimensions)?;

        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&[weights_path], DType::F32, &device)
                .context("kb-embed-candle: mmap safetensors")?
        };
        let model =
            XLMRobertaModel::new(&cfg, vb).context("kb-embed-candle: build XLMRobertaModel")?;

        let mut tokenizer = Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| anyhow::anyhow!("kb-embed-candle: load tokenizer: {e}"))?;
        tokenizer
            .with_padding(Some(PaddingParams {
                strategy: PaddingStrategy::BatchLongest,
                ..Default::default()
            }))
            .with_truncation(Some(TruncationParams {
                max_length: MAX_LEN,
                ..Default::default()
            }))
            .map_err(|e| anyhow::anyhow!("kb-embed-candle: set truncation: {e}"))?;

        tracing::info!(
            target: "kebab-embed-candle",
            dimensions = cfg.hidden_size,
            layers = cfg.num_hidden_layers,
            "candle embedding model loaded"
        );

        Ok(Self {
            model: Mutex::new(model),
            tokenizer,
            device,
            model_id: EmbeddingModelId(config.models.embedding.model.clone()),
            version: EmbeddingVersion(config.models.embedding.version.clone()),
            dimensions: cfg.hidden_size,
            batch_size: config.models.embedding.batch_size.max(1),
        })
    }

    /// Embed one batch of **already-prefixed** strings (the e5 `query:`/
    /// `passage:` prefix is applied by the caller [`CandleEmbedder::embed`])
    /// through the candle pipeline: tokenize → forward → masked mean pool → L2.
    fn embed_batch(&self, prefixed: &[String]) -> Result<Vec<Vec<f32>>> {
        let encodings = self
            .tokenizer
            .encode_batch(prefixed.to_vec(), true)
            .map_err(|e| anyhow::anyhow!("kb-embed-candle: encode_batch: {e}"))?;

        let bsz = encodings.len();
        // `embed` already returns early on empty input and `.chunks()` never
        // yields an empty slice, so this is currently unreachable — but guard
        // the index so a future refactor can't turn it into a panic.
        let Some(first) = encodings.first() else {
            return Ok(Vec::new());
        };
        let seq = first.get_ids().len();

        let mut ids = Vec::with_capacity(bsz * seq);
        let mut mask = Vec::with_capacity(bsz * seq);
        for enc in &encodings {
            ids.extend(enc.get_ids().iter().copied());
            mask.extend(enc.get_attention_mask().iter().map(|&m| m as f32));
        }

        let input_ids = Tensor::from_vec(ids, (bsz, seq), &self.device)?;
        let attn_f32 = Tensor::from_vec(mask, (bsz, seq), &self.device)?;
        let token_type_ids = input_ids.zeros_like()?;

        let hidden = {
            let guard = self
                .model
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            // forward: (input_ids, attention_mask, token_type_ids, past,
            // encoder_hidden, encoder_mask)
            guard.forward(&input_ids, &attn_f32, &token_type_ids, None, None, None)?
        };

        // attention-mask-weighted mean pooling
        let mask3 = attn_f32.unsqueeze(2)?; // (b, seq, 1)
        let summed = hidden.broadcast_mul(&mask3)?.sum(1)?; // (b, hidden)
        // counts ≥ 1 always: every input is e5-prefixed AND special tokens are
        // added (encode_batch(_, true)), so no row has an all-zero mask. If that
        // invariant ever breaks, broadcast_div would emit NaN vectors.
        let counts = mask3.sum(1)?; // (b, 1)
        let mean = summed.broadcast_div(&counts)?;

        // L2 normalize
        let norm = mean.sqr()?.sum_keepdim(1)?.sqrt()?;
        let normalized = mean.broadcast_div(&norm)?;

        Ok(normalized.to_vec2::<f32>()?)
    }
}

impl Embedder for CandleEmbedder {
    fn model_id(&self) -> EmbeddingModelId {
        self.model_id.clone()
    }

    fn model_version(&self) -> EmbeddingVersion {
        self.version.clone()
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }

    fn embed(&self, inputs: &[EmbeddingInput<'_>]) -> Result<Vec<Vec<f32>>> {
        if inputs.is_empty() {
            return Ok(Vec::new());
        }

        // e5 prefix per §11.3 BEFORE tokenization (same convention as
        // FastembedEmbedder so the two backends produce comparable vectors).
        let prefixed: Vec<String> = inputs.iter().map(prefix_input).collect();

        let mut out: Vec<Vec<f32>> = Vec::with_capacity(prefixed.len());
        for chunk in prefixed.chunks(self.batch_size) {
            let batch = self.embed_batch(chunk)?;
            for v in &batch {
                if v.len() != self.dimensions {
                    anyhow::bail!(
                        "candle returned vector of length {} but adapter expects {}",
                        v.len(),
                        self.dimensions
                    );
                }
            }
            out.extend(batch);
        }

        debug_assert_eq!(out.len(), inputs.len());
        Ok(out)
    }
}

/// Build the e5-prefixed string for one [`EmbeddingInput`]. Free function so
/// a unit test can pin the format without loading the model. Byte-identical to
/// `kebab-embed-local`'s `prefix_input` — the two backends MUST agree here or
/// their vectors diverge.
fn prefix_input(input: &EmbeddingInput<'_>) -> String {
    match input.kind {
        EmbeddingKind::Document => format!("passage: {}", input.text),
        EmbeddingKind::Query => format!("query: {}", input.text),
    }
}

/// Apply a one-shot global rayon thread cap (the NUMA-safety lever). Returns
/// `true` if this call set the pool, `false` if it was already initialized
/// (cap not applied) or `n_threads == 0`. `#[doc(hidden)] pub` so the
/// thread-cap test can drive it without loading the 2GB model.
#[doc(hidden)]
pub fn apply_thread_cap(n_threads: usize) -> bool {
    if n_threads == 0 {
        return false;
    }
    rayon::ThreadPoolBuilder::new()
        .num_threads(n_threads)
        .build_global()
        .is_ok()
}

/// Compare model hidden size against the configured dim. Extracted so a unit
/// test can exercise the error branch without loading the model.
pub(crate) fn check_dim(model_dim: usize, cfg_dim: usize) -> Result<()> {
    if model_dim != cfg_dim {
        anyhow::bail!(
            "dimension mismatch: model={model_dim}, config={cfg_dim}; \
             update `config.models.embedding.dimensions` to match the model \
             (or pick a different model)."
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── prefix_input ─────────────────────────────────────────────────
    // Pin the exact e5 prefix strings; these MUST match
    // kebab-embed-local::prefix_input or candle vs fastembed parity breaks.

    #[test]
    fn prefix_document_uses_passage() {
        let input = EmbeddingInput {
            text: "hello world",
            kind: EmbeddingKind::Document,
        };
        assert_eq!(prefix_input(&input), "passage: hello world");
    }

    #[test]
    fn prefix_query_uses_query() {
        let input = EmbeddingInput {
            text: "hello world",
            kind: EmbeddingKind::Query,
        };
        assert_eq!(prefix_input(&input), "query: hello world");
    }

    #[test]
    fn prefix_handles_empty_text() {
        let doc = EmbeddingInput {
            text: "",
            kind: EmbeddingKind::Document,
        };
        let qry = EmbeddingInput {
            text: "",
            kind: EmbeddingKind::Query,
        };
        assert_eq!(prefix_input(&doc), "passage: ");
        assert_eq!(prefix_input(&qry), "query: ");
    }

    // ── check_dim ────────────────────────────────────────────────────

    #[test]
    fn check_dim_passes_for_1024() {
        check_dim(1024, 1024).expect("matching dims must pass");
    }

    #[test]
    fn check_dim_rejects_384_vs_1024() {
        let err = check_dim(384, 1024).expect_err("dim mismatch must error");
        let msg = format!("{err}");
        assert!(
            msg.contains("384") && msg.contains("1024"),
            "error must mention both dims, got: {msg}"
        );
    }

    // ── model guard ──────────────────────────────────────────────────
    // A non-e5-large model name must fail fast (BEFORE the ~2GB download),
    // so we never download e5-large yet label its vectors with another name
    // via model_id() — which would mislabel embedding_version.

    #[test]
    fn new_rejects_unsupported_model() {
        let mut config = kebab_config::Config::defaults();
        config.models.embedding.model = "multilingual-e5-small".to_string();
        // num_threads defaults to 0, so no global rayon side effect here.
        // `.err()` (not `expect_err`) avoids requiring `CandleEmbedder: Debug`
        // — it holds a Mutex/Tokenizer and intentionally derives no Debug.
        let err = CandleEmbedder::new(&config)
            .err()
            .expect("unsupported model must error");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("candle provider currently supports only"),
            "expected model-guard error, got: {msg}"
        );
    }
}
