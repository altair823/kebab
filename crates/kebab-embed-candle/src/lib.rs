//! `kebab-embed-candle` — [`CandleEmbedder`], a pure-Rust (candle)
//! implementation of [`Embedder`](kebab_core::Embedder).
//!
//! Runs an XLM-RoBERTa-large embedding model through `candle`
//! (`candle-transformers`' XLM-RoBERTa) instead of onnxruntime. Two models
//! are wired through a small **registry** ([`MODEL_REGISTRY`]):
//!
//! * `multilingual-e5-large` — the same weights the default
//!   [`FastembedEmbedder`](kebab_embed_local) uses (mean pooling,
//!   `query: `/`passage: ` prefixes). candle is the NUMA-safe drop-in:
//!   fastembed 4.9's onnxruntime hard-codes 48 intra-op threads, which
//!   corrupts the heap (double-free) on dual-socket NUMA hosts. candle's
//!   CPU backend sizes its threads off the global rayon pool, so a one-shot
//!   [`rayon::ThreadPoolBuilder`] cap (config `num_threads` / env
//!   `KEBAB_EMBED_THREADS`) keeps the worker count NUMA-safe.
//! * `snowflake-arctic-embed-l-v2.0` — Snowflake's arctic-embed v2.0
//!   (CLS pooling, `query: ` on queries / no prefix on documents). Same
//!   XLM-RoBERTa-large architecture, dim 1024, so it rides the exact same
//!   tokenize → forward → L2 pipeline; only the pooling step and prefixes
//!   differ (both keyed off the per-model [`EmbedModelSpec`]).
//!
//! Output parity with the onnxruntime path (for e5) was proven by the
//! Phase 0 spike (cosine 1.000000); the arctic path's pooling/prefix
//! correctness is pinned by an `#[ignore]`d cosine>0.99 cross-check against
//! Ollama's `snowflake-arctic-embed2` (see `tests/arctic_ollama_parity.rs`).
//! The shared pipeline:
//!
//! 1. instruction prefix per [`EmbedModelSpec`] (query/doc);
//! 2. tokenize (max_len 512, batch-longest padding, special tokens);
//! 3. XLM-RoBERTa forward on the selected [`Device`];
//! 4. pooling — mean (attention-mask-weighted) or CLS (first token);
//! 5. L2 normalization.
//!
//! Model files (`config.json`, `tokenizer.json`, `model.safetensors`) are
//! fetched via `hf-hub` into `{config.storage.model_dir}/candle/` (hf-hub's
//! cache layout namespaces by repo, so e5 and arctic never collide).
//!
//! This crate is **opt-in** (`config.models.embedding.provider = "candle"`);
//! the default provider stays `fastembed`. See
//! `docs/superpowers/specs/2026-06-01-embed-candle-track-spec.md` and
//! `docs/superpowers/specs/2026-06-03-arctic-embedder-spec.md`.

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

/// Token truncation length (both e5 and arctic-embed-l-v2.0 train at 512).
const MAX_LEN: usize = 512;

/// Env var that overrides `config.models.embedding.num_threads`. Read once in
/// [`CandleEmbedder::new`]; `0`/unset/unparseable means "leave rayon default".
const ENV_EMBED_THREADS: &str = "KEBAB_EMBED_THREADS";

/// Pooling strategy over the model's last hidden state. Keyed per-model by
/// [`EmbedModelSpec::pooling`] — e5 is mean, arctic is CLS.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pooling {
    /// Attention-mask-weighted mean over all tokens (e5 / sentence-transformers
    /// `pooling_mode_mean_tokens`).
    Mean,
    /// First token (`<s>`/`[CLS]`) hidden state (arctic-embed v2.0 —
    /// `1_Pooling/config.json` has `pooling_mode_cls_token: true`).
    Cls,
}

/// One supported embedding model: the HF repo candle downloads, the pooling
/// strategy, and the e5-style instruction prefixes. [`MODEL_REGISTRY`] maps a
/// `config.models.embedding.model` value to one of these.
#[derive(Clone, Copy, Debug)]
pub struct EmbedModelSpec {
    /// The short `config.models.embedding.model` value that selects this spec.
    pub name: &'static str,
    /// HuggingFace repo id candle fetches `config.json` / `tokenizer.json` /
    /// `model.safetensors` from.
    pub hf_repo: &'static str,
    /// Pooling over the last hidden state.
    pub pooling: Pooling,
    /// Prefix prepended to **query** inputs before tokenization.
    pub query_prefix: &'static str,
    /// Prefix prepended to **document** inputs before tokenization (arctic
    /// uses `""` — documents are embedded raw).
    pub doc_prefix: &'static str,
    /// Expected embedding dimension (model hidden size).
    pub dim: usize,
    /// Suffix folded into `model_version` so switching **to** this model
    /// triggers the `embedding_version` cascade even if the operator forgets
    /// to bump `config.version`. `None` keeps the bare `config.version` — used
    /// by e5 so candle-e5 and fastembed-e5 report the *same* version and stay
    /// interchangeable (the NUMA drop-in invariant — Phase 0 cosine 1.0).
    pub version_tag: Option<&'static str>,
}

/// The models the candle adapter can load. Adding a model = one entry here
/// (plus, for a non-XLM-R architecture, a new forward path — both current
/// entries are XLM-RoBERTa-large so they share everything but pooling/prefix).
static MODEL_REGISTRY: &[EmbedModelSpec] = &[
    EmbedModelSpec {
        name: "multilingual-e5-large",
        hf_repo: "intfloat/multilingual-e5-large",
        pooling: Pooling::Mean,
        query_prefix: "query: ",
        doc_prefix: "passage: ",
        dim: 1024,
        version_tag: None,
    },
    EmbedModelSpec {
        name: "snowflake-arctic-embed-l-v2.0",
        hf_repo: "Snowflake/snowflake-arctic-embed-l-v2.0",
        pooling: Pooling::Cls,
        query_prefix: "query: ",
        doc_prefix: "",
        dim: 1024,
        version_tag: Some("arctic-cls"),
    },
];

/// Look up a model spec by `config.models.embedding.model`. Accepts either the
/// short `name` or the full `hf_repo` id (mirrors the old e5 guard, which
/// accepted both `multilingual-e5-large` and `intfloat/multilingual-e5-large`).
pub(crate) fn lookup_spec(model: &str) -> Option<&'static EmbedModelSpec> {
    MODEL_REGISTRY
        .iter()
        .find(|s| s.name == model || s.hf_repo == model)
}

/// Comma-separated list of supported model names, for the
/// unsupported-model error message.
fn supported_models() -> String {
    MODEL_REGISTRY
        .iter()
        .map(|s| s.name)
        .collect::<Vec<_>>()
        .join("`, `")
}

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
    /// The resolved model spec (pooling + prefixes) — drives `embed` and
    /// `embed_batch`.
    spec: &'static EmbedModelSpec,
    model_id: EmbeddingModelId,
    version: EmbeddingVersion,
    dimensions: usize,
    batch_size: usize,
}

impl CandleEmbedder {
    /// Build an embedder from `Config`. Resolves the model spec from
    /// `config.models.embedding.model`, applies the NUMA thread cap, fetches
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

        // 1b. Model registry lookup. If the operator configured a model the
        //     candle adapter doesn't know, fail fast (BEFORE the ~2GB
        //     download) — never silently download one model and then label its
        //     vectors with another name via `model_id()`, which would mislabel
        //     `embedding_version` and corrupt a mixed index.
        let want = config.models.embedding.model.as_str();
        let spec = lookup_spec(want).ok_or_else(|| {
            anyhow::anyhow!(
                "candle provider supports the models `{}`, but \
                 config.models.embedding.model = '{want}'. Use provider=fastembed \
                 for other models, or pick a supported one.",
                supported_models()
            )
        })?;

        // 2. Resolve `{data_dir}/models/candle/` exactly like the fastembed
        //    adapter resolves its own subdir.
        let data_dir = expand_path(&config.storage.data_dir, "");
        let model_dir = expand_path(&config.storage.model_dir, &data_dir.to_string_lossy());
        let cache_dir = model_dir.join(CANDLE_CACHE_SUBDIR);
        std::fs::create_dir_all(&cache_dir)
            .with_context(|| format!("create candle cache dir {}", cache_dir.display()))?;

        let device = select_device();

        // 3. Fetch model files via hf-hub into the candle cache.
        tracing::info!(
            target: "kebab-embed-candle",
            cache_dir = %cache_dir.display(),
            model = spec.hf_repo,
            pooling = ?spec.pooling,
            "loading candle embedding model (first run downloads ~2GB safetensors)"
        );
        let api = hf_hub::api::sync::ApiBuilder::new()
            .with_cache_dir(cache_dir.clone())
            .build()
            .context("kb-embed-candle: build hf-hub api")?;
        let repo = api.model(spec.hf_repo.to_string());
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

        // model_version: fold the model tag in for non-e5 models so a switch
        // triggers the embedding_version cascade; e5 keeps the bare
        // config.version to stay interchangeable with fastembed-e5.
        let version = match spec.version_tag {
            Some(tag) => {
                EmbeddingVersion(format!("{}+{}", config.models.embedding.version, tag))
            }
            None => EmbeddingVersion(config.models.embedding.version.clone()),
        };

        tracing::info!(
            target: "kebab-embed-candle",
            dimensions = cfg.hidden_size,
            layers = cfg.num_hidden_layers,
            model = spec.name,
            "candle embedding model loaded"
        );

        Ok(Self {
            model: Mutex::new(model),
            tokenizer,
            device,
            spec,
            model_id: EmbeddingModelId(config.models.embedding.model.clone()),
            version,
            dimensions: cfg.hidden_size,
            batch_size: config.models.embedding.batch_size.max(1),
        })
    }

    /// Embed one batch of **already-prefixed** strings (the per-model prefix
    /// is applied by the caller [`CandleEmbedder::embed`]) through the candle
    /// pipeline: tokenize → forward → pool (mean|CLS) → L2.
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

        // Pooling — per the model spec.
        let pooled = match self.spec.pooling {
            Pooling::Mean => {
                // attention-mask-weighted mean pooling
                let mask3 = attn_f32.unsqueeze(2)?; // (b, seq, 1)
                let summed = hidden.broadcast_mul(&mask3)?.sum(1)?; // (b, hidden)
                // counts ≥ 1 always: every input is prefixed AND special
                // tokens are added (encode_batch(_, true)), so no row has an
                // all-zero mask. If that invariant ever breaks, broadcast_div
                // would emit NaN vectors.
                let counts = mask3.sum(1)?; // (b, 1)
                summed.broadcast_div(&counts)?
            }
            Pooling::Cls => {
                // CLS pooling: the first token's hidden state. arctic-embed
                // v2.0 prepends `<s>` (the XLM-R BOS/CLS) at index 0, so
                // `hidden[:, 0, :]` is the sentence embedding.
                hidden.narrow(1, 0, 1)?.squeeze(1)? // (b, hidden)
            }
        };

        // L2 normalize
        let norm = pooled.sqr()?.sum_keepdim(1)?.sqrt()?;
        let normalized = pooled.broadcast_div(&norm)?;

        // `.contiguous()` before host copy: broadcast ops can leave a strided
        // view, which `to_vec2` rejects on the Metal backend (CPU tolerates it).
        Ok(normalized.contiguous()?.to_vec2::<f32>()?)
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

        // Per-model instruction prefix BEFORE tokenization (same convention as
        // FastembedEmbedder for e5; arctic uses `query: `/no-prefix).
        let prefixed: Vec<String> = inputs.iter().map(|i| prefix_input(self.spec, i)).collect();

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

/// Build the prefixed string for one [`EmbeddingInput`] using the model spec.
/// Free function so a unit test can pin the format without loading the model.
/// For e5 this is byte-identical to `kebab-embed-local`'s `prefix_input` — the
/// two backends MUST agree there or their vectors diverge.
fn prefix_input(spec: &EmbedModelSpec, input: &EmbeddingInput<'_>) -> String {
    match input.kind {
        EmbeddingKind::Document => format!("{}{}", spec.doc_prefix, input.text),
        EmbeddingKind::Query => format!("{}{}", spec.query_prefix, input.text),
    }
}

/// Select the compute device. Built with the `metal` feature (Apple Silicon
/// GPU), try Metal and fall back to CPU on failure; otherwise CPU. Metal only
/// compiles/runs on macOS — the Linux server builds the CPU path. Embedding
/// vectors are model-defined, so Metal-produced and CPU-produced embeddings
/// are cross-compatible (a Mac can ingest on GPU, the server query on CPU).
fn select_device() -> Device {
    #[cfg(feature = "metal")]
    {
        match Device::new_metal(0) {
            Ok(d) => {
                tracing::info!(target: "kebab-embed-candle", "candle device = Metal (GPU)");
                return d;
            }
            Err(e) => {
                tracing::warn!(
                    target: "kebab-embed-candle",
                    error = %e,
                    "Metal device unavailable; falling back to CPU"
                );
            }
        }
    }
    tracing::info!(target: "kebab-embed-candle", "candle device = CPU");
    Device::Cpu
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

    fn e5_spec() -> &'static EmbedModelSpec {
        lookup_spec("multilingual-e5-large").expect("e5 in registry")
    }

    fn arctic_spec() -> &'static EmbedModelSpec {
        lookup_spec("snowflake-arctic-embed-l-v2.0").expect("arctic in registry")
    }

    // ── registry ─────────────────────────────────────────────────────

    #[test]
    fn registry_resolves_e5_by_name_and_hf_repo() {
        assert_eq!(
            lookup_spec("multilingual-e5-large").map(|s| s.name),
            Some("multilingual-e5-large")
        );
        assert_eq!(
            lookup_spec("intfloat/multilingual-e5-large").map(|s| s.name),
            Some("multilingual-e5-large")
        );
    }

    #[test]
    fn registry_resolves_arctic_and_its_pooling_is_cls() {
        let s = arctic_spec();
        assert_eq!(s.name, "snowflake-arctic-embed-l-v2.0");
        assert_eq!(s.hf_repo, "Snowflake/snowflake-arctic-embed-l-v2.0");
        assert_eq!(s.pooling, Pooling::Cls);
        assert_eq!(s.dim, 1024);
        assert_eq!(s.version_tag, Some("arctic-cls"));
    }

    #[test]
    fn registry_e5_is_mean_pooling_no_version_tag() {
        let s = e5_spec();
        assert_eq!(s.pooling, Pooling::Mean);
        assert_eq!(s.version_tag, None);
    }

    #[test]
    fn registry_rejects_unknown_model() {
        assert!(lookup_spec("multilingual-e5-small").is_none());
    }

    // ── prefix_input ─────────────────────────────────────────────────
    // e5 prefixes MUST match kebab-embed-local::prefix_input or candle vs
    // fastembed parity breaks; arctic uses query-only prefixing.

    #[test]
    fn e5_prefix_document_uses_passage() {
        let input = EmbeddingInput {
            text: "hello world",
            kind: EmbeddingKind::Document,
        };
        assert_eq!(prefix_input(e5_spec(), &input), "passage: hello world");
    }

    #[test]
    fn e5_prefix_query_uses_query() {
        let input = EmbeddingInput {
            text: "hello world",
            kind: EmbeddingKind::Query,
        };
        assert_eq!(prefix_input(e5_spec(), &input), "query: hello world");
    }

    #[test]
    fn arctic_prefix_query_uses_query_doc_is_bare() {
        let doc = EmbeddingInput {
            text: "후입선출 자료구조",
            kind: EmbeddingKind::Document,
        };
        let qry = EmbeddingInput {
            text: "스택 자료구조",
            kind: EmbeddingKind::Query,
        };
        // arctic: documents are embedded raw, queries get `query: `.
        assert_eq!(prefix_input(arctic_spec(), &doc), "후입선출 자료구조");
        assert_eq!(prefix_input(arctic_spec(), &qry), "query: 스택 자료구조");
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
        assert_eq!(prefix_input(e5_spec(), &doc), "passage: ");
        assert_eq!(prefix_input(e5_spec(), &qry), "query: ");
        assert_eq!(prefix_input(arctic_spec(), &doc), "");
        assert_eq!(prefix_input(arctic_spec(), &qry), "query: ");
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
    // A model name not in the registry must fail fast (BEFORE the ~2GB
    // download), so we never download one model yet label its vectors with
    // another name via model_id() — which would mislabel embedding_version.

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
            msg.contains("candle provider supports the models"),
            "expected model-registry error, got: {msg}"
        );
    }
}
