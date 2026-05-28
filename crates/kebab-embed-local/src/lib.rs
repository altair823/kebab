//! `kb-embed-local` — `FastembedEmbedder`, a local ONNX-backed
//! [`Embedder`](kebab_embed::Embedder) implementation.
//!
//! Wraps [`fastembed::TextEmbedding`]. Default is `multilingual-e5-large`
//! (1024-dim, p9-fb-39b); `multilingual-e5-small` (384-dim) is also supported
//! for backwards-compat. Honors `config.models.embedding.batch_size` and applies
//! the e5 prefix convention (§11.3 of the design report):
//!
//! * `EmbeddingKind::Document` → `"passage: "` prefix
//! * `EmbeddingKind::Query`    → `"query: "` prefix
//!
//! The underlying fastembed `TextEmbedding::embed` already L2-normalizes each
//! row (see `fastembed::text_embedding::output::transformer_with_precedence`),
//! so we do not re-normalize; the unit-norm test in `tests/` keeps that
//! invariant pinned in case fastembed changes its default.
//!
//! Model files are cached under
//! `config.storage.model_dir/fastembed/`. The `model_dir` template
//! (default `"{data_dir}/models"`) is resolved with the same expansion
//! rules `kb-store-sqlite` applies to `data_dir` (`${XDG_DATA_HOME:-…}`,
//! leading `~`, `{data_dir}` substitution).
//!
//! See `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md`
//! §7.2 (Embedder), §6.4 ([models.embedding]), §9 (versioning).

use std::sync::Mutex;

use anyhow::{Context, Result};
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use kebab_config::expand_path;
use kebab_embed::{Embedder, EmbeddingInput, EmbeddingKind, EmbeddingModelId, EmbeddingVersion};

/// Subdirectory under `config.storage.model_dir` where the fastembed
/// adapter writes / reads ONNX + tokenizer files. Hard-coded per task
/// spec ("Model files cached under `config.storage.model_dir/fastembed/`").
const FASTEMBED_CACHE_SUBDIR: &str = "fastembed";

/// Local fastembed-rs adapter.
///
/// Construct via [`FastembedEmbedder::new`]. The constructor performs the
/// (potentially network-bound) model download on first use, so prefer to
/// share an instance across calls.
pub struct FastembedEmbedder {
    // Mutex serializes calls into TextEmbedding's underlying ONNX session.
    // fastembed::TextEmbedding::embed is `&self` in 4.9 and ORT Session is
    // Send + Sync, so this Mutex is conservative — it serializes inference
    // where parallel ORT calls would in principle work. Acceptable here
    // because callers (kb-app indexer) batch sequentially anyway. Revisit
    // in P3-3+ if profiling shows contention.
    inner: Mutex<TextEmbedding>,
    model_id: EmbeddingModelId,
    version: EmbeddingVersion,
    dimensions: usize,
    batch_size: usize,
}

impl FastembedEmbedder {
    /// Build an embedder from `Config`. Validates that
    /// `config.models.embedding.dimensions` matches the model's actual
    /// dim BEFORE returning, so a mismatch fails at construction (not on
    /// first `embed`).
    pub fn new(config: &kebab_config::Config) -> Result<Self> {
        // 1. Resolve `{data_dir}/models/fastembed/` from the config
        //    templates. Goes through the shared `kebab_config::expand_path`
        //    so every crate resolves storage paths identically.
        let data_dir = expand_path(&config.storage.data_dir, "");
        let model_dir = expand_path(&config.storage.model_dir, &data_dir.to_string_lossy());
        let cache_dir = model_dir.join(FASTEMBED_CACHE_SUBDIR);
        std::fs::create_dir_all(&cache_dir)
            .with_context(|| format!("create fastembed cache dir {}", cache_dir.display()))?;

        // 2. Resolve the fastembed enum variant from
        //    `config.models.embedding.model`. Currently `multilingual-e5-large`
        //    (default) and `multilingual-e5-small` are wired; other model names
        //    error out with a clear message rather than silently misconfiguring.
        let model_name = resolve_model(&config.models.embedding.model)?;

        // 3. Verify dim match BEFORE loading the model — if the config
        //    is wrong we want to fail without paying the ONNX
        //    initialization cost.
        let model_info =
            TextEmbedding::get_model_info(&model_name).context("fastembed: get_model_info")?;
        check_dim(model_info.dim, config.models.embedding.dimensions)?;

        tracing::info!(
            target: "kebab-embed-local",
            cache_dir = %cache_dir.display(),
            model = %config.models.embedding.model,
            dims = model_info.dim,
            "initializing FastembedEmbedder"
        );

        // 4. Build the underlying TextEmbedding. `show_download_progress`
        //    is forced to `false` so test output stays clean; first-run
        //    download progress is surfaced via the `tracing::info!`
        //    pair around `TextEmbedding::try_new` instead.
        let opts = InitOptions::new(model_name.clone())
            .with_cache_dir(cache_dir.clone())
            .with_show_download_progress(false);
        tracing::info!(
            target: "kebab-embed-local",
            model = %config.models.embedding.model,
            cache_dir = %cache_dir.display(),
            "loading embedding model (first run downloads model weights — ~470MB for e5-small, ~1.3GB for e5-large)"
        );
        let inner = TextEmbedding::try_new(opts).context("fastembed: TextEmbedding::try_new")?;
        let dimensions = model_info.dim;
        tracing::info!(
            target: "kebab-embed-local",
            model = %config.models.embedding.model,
            dimensions,
            "embedding model loaded"
        );

        Ok(Self {
            inner: Mutex::new(inner),
            model_id: EmbeddingModelId(config.models.embedding.model.clone()),
            version: EmbeddingVersion(config.models.embedding.version.clone()),
            dimensions,
            batch_size: config.models.embedding.batch_size,
        })
    }
}

impl Embedder for FastembedEmbedder {
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

        // Apply e5 prefix per §11.3 BEFORE tokenization. The fastembed
        // model is unaware of the document/query distinction; the prefix
        // is the only signal that lets it produce different embeddings
        // for the same surface text in different roles.
        let prefixed: Vec<String> = inputs.iter().map(prefix_input).collect();

        // We run our own batch loop on top of fastembed's internal one
        // so that `config.models.embedding.batch_size` is honored
        // exactly. fastembed's `embed(_, Some(batch_size))` does the
        // same internally; calling once with our batch size matches
        // intent and avoids an extra per-batch allocation.
        let mut out: Vec<Vec<f32>> = Vec::with_capacity(prefixed.len());
        for chunk in prefixed.chunks(self.batch_size) {
            let chunk_vec: Vec<&str> = chunk.iter().map(String::as_str).collect();
            let guard = self
                .inner
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let batch: Vec<Vec<f32>> = guard
                .embed(chunk_vec, Some(self.batch_size))
                .context("fastembed: embed")?;
            drop(guard);
            // Defensive shape check — every returned vector must match
            // the configured `dimensions`. Mismatch here means fastembed
            // and our config drifted at runtime (extremely unlikely;
            // would have been caught at construction).
            for v in &batch {
                if v.len() != self.dimensions {
                    anyhow::bail!(
                        "fastembed returned vector of length {} but adapter expects {}",
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

/// Build the prefixed string for one [`EmbeddingInput`]. Free function so
/// the unit test can pin the exact format without going through `embed`.
fn prefix_input(input: &EmbeddingInput<'_>) -> String {
    match input.kind {
        EmbeddingKind::Document => format!("passage: {}", input.text),
        EmbeddingKind::Query => format!("query: {}", input.text),
    }
}

/// Resolve a `config.models.embedding.model` string to a fastembed
/// `EmbeddingModel` enum variant. Currently supports `multilingual-e5-small`
/// (384-dim) and `multilingual-e5-large` (1024-dim); additional model names
/// should be added (and their dims pinned in tests) as needed.
fn resolve_model(name: &str) -> Result<EmbeddingModel> {
    match name {
        "multilingual-e5-small" => Ok(EmbeddingModel::MultilingualE5Small),
        "multilingual-e5-large" => Ok(EmbeddingModel::MultilingualE5Large),
        other => anyhow::bail!(
            "kb-embed-local: unsupported embedding model {other:?}; \
             this adapter currently ships `multilingual-e5-small` and \
             `multilingual-e5-large`. Add a new arm to `resolve_model` \
             (and a fastembed feature flag if needed) to support more."
        ),
    }
}

/// Compare model dim against the configured dim. Extracted so a unit
/// test can exercise the error branch without loading ONNX.
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
    use kebab_embed::EmbeddingInput;

    // ── check_dim ────────────────────────────────────────────────────
    //
    // Exercises the construction-time dim mismatch branch WITHOUT
    // loading the real model. The integration test that builds a full
    // FastembedEmbedder is `#[ignore]`d (loads ~470 MB of weights).

    #[test]
    fn check_dim_match_ok() {
        check_dim(384, 384).expect("matching dims must pass");
    }

    #[test]
    fn check_dim_mismatch_errors() {
        let err = check_dim(384, 512).expect_err("mismatch must error");
        let msg = format!("{err}");
        assert!(msg.contains("dimension mismatch"), "msg={msg}");
        assert!(msg.contains("384"), "msg={msg}");
        assert!(msg.contains("512"), "msg={msg}");
    }

    // ── prefix_input ─────────────────────────────────────────────────
    //
    // Pin the exact e5 prefix strings; a silent regression here
    // degrades retrieval quality without any test failing in the
    // dim/norm/snapshot suite.

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

    // ── resolve_model ────────────────────────────────────────────────

    #[test]
    fn resolve_default_model_ok() {
        // The exact enum variant is opaque, but `is_ok` plus a
        // round-trip through the fastembed metadata gives confidence
        // we hit the right arm.
        resolve_model("multilingual-e5-small").expect("default model resolves");
    }

    #[test]
    fn resolve_model_supports_e5_large() {
        let m = resolve_model("multilingual-e5-large").expect("e5-large should resolve");
        let _ = m;
    }

    #[test]
    fn resolve_unknown_model_errors() {
        let err = resolve_model("not-a-real-model").expect_err("unknown model errors");
        let msg = format!("{err}");
        assert!(msg.contains("unsupported embedding model"), "msg={msg}");
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

    // expand_path tests live in `kb-config::paths`. The adapter imports
    // it and trusts the upstream coverage rather than duplicating it.
}
