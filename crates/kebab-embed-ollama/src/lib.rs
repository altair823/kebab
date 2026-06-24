//! `kebab-embed-ollama` — [`OllamaEmbedder`], a `reqwest::blocking` adapter
//! implementing [`Embedder`](kebab_core::Embedder) over Ollama's
//! `POST /api/embed` endpoint.
//!
//! ## Why this exists
//!
//! This crate offloads embedding to a local/remote Ollama daemon
//! (`snowflake-arctic-embed2`), which is exactly the route the recall
//! measurements used — so it reproduces the measured numbers (recall@10
//! 130/132) byte-for-route. Opt-in via
//! `config.models.embedding.provider = "ollama"`.
//!
//! ## Wire shape
//!
//! Request (`POST {endpoint}/api/embed`):
//!
//! ```json
//! { "model": "snowflake-arctic-embed2", "input": ["query: 스택", "후입선출 ..."] }
//! ```
//!
//! Response:
//!
//! ```json
//! { "model": "...", "embeddings": [[0.01, ...], [0.02, ...]] }
//! ```
//!
//! ## Pipeline
//!
//! 1. instruction prefix per model ([`prefixes_for`] — arctic: `query: ` on
//!    queries, no prefix on documents; e5: `query: `/`passage: `);
//! 2. batch into `BATCH` (48) inputs per request;
//! 3. `POST /api/embed`, with fail-soft retry (`MAX_RETRIES`);
//! 4. **L2 normalize** each returned vector — Ollama returns raw (un-normalized)
//!    embeddings, so we normalize for cosine consistency with the candle path;
//! 5. dim check against `config.models.embedding.dimensions`.
//!
//! ## Send-safety
//!
//! `reqwest::blocking::Client: Send + Sync`; the adapter holds only the client,
//! an endpoint string, and small config scalars, so it is trivially `Send + Sync`
//! as the [`Embedder`] trait requires.

use std::time::Duration;

use anyhow::{Context, Result};
use kebab_core::{Embedder, EmbeddingInput, EmbeddingKind, EmbeddingModelId, EmbeddingVersion};
use serde::{Deserialize, Serialize};

/// Inputs per `/api/embed` request. Ollama handles arbitrary batch sizes, but
/// a cap keeps a single HTTP body bounded and lets a partial failure retry a
/// smaller unit.
const BATCH: usize = 48;

/// Fail-soft retry attempts per batch before the error propagates. Cold model
/// load on the Ollama side can transiently 500/timeout; a couple of retries
/// smooth that over without masking a hard misconfiguration.
const MAX_RETRIES: u32 = 3;

/// Default per-request HTTP timeout (seconds). Cold-loading an embedding model
/// on first call can take tens of seconds; this matches the generous default
/// used by the LLM adapter.
const REQUEST_TIMEOUT_SECS: u64 = 300;

/// Resolve the (query_prefix, doc_prefix) for an Ollama embedding model tag.
///
/// Resolve the (query_prefix, doc_prefix) for an Ollama embedding model tag,
/// keyed on the **Ollama model tag** (which differs from the HF id — e.g.
/// `snowflake-arctic-embed2` vs `Snowflake/snowflake-arctic-embed-l-v2.0`).
///
/// An unrecognized model gets no prefix (`("", "")`): many embedding models
/// are not instruction-tuned, so embedding the raw text is the correct default
/// — and a misspelled known model surfaces as a recall regression, not a silent
/// wrong-prefix, because the dim check still passes either way.
fn prefixes_for(model: &str) -> (&'static str, &'static str) {
    let m = model.to_ascii_lowercase();
    if m.contains("arctic-embed") {
        // arctic-embed v2.0: `query: ` on queries, documents embedded raw.
        ("query: ", "")
    } else if m.contains("e5") {
        // multilingual-e5: `query: ` / `passage: `.
        ("query: ", "passage: ")
    } else {
        ("", "")
    }
}

/// `reqwest::blocking` adapter implementing [`Embedder`] over Ollama's
/// `/api/embed`. Construction is offline; the first network call happens in
/// [`Embedder::embed`].
pub struct OllamaEmbedder {
    client: reqwest::blocking::Client,
    /// Validated endpoint base (e.g. `"http://127.0.0.1:11434"`).
    endpoint: String,
    /// Ollama model tag (e.g. `"snowflake-arctic-embed2"`).
    model: String,
    query_prefix: &'static str,
    doc_prefix: &'static str,
    model_id: EmbeddingModelId,
    version: EmbeddingVersion,
    dimensions: usize,
}

impl OllamaEmbedder {
    /// Build from a workspace [`kebab_config::Config`]. Reads
    /// `config.models.embedding.{model, dimensions}` and resolves the endpoint
    /// as `models.embedding.endpoint` → fallback `models.llm.endpoint`.
    ///
    /// Does NOT touch the network. The caller (app layer) is expected to have
    /// validated `provider == "ollama"`.
    pub fn new(config: &kebab_config::Config) -> Result<Self> {
        let emb = &config.models.embedding;
        let endpoint = emb
            .endpoint
            .clone()
            .filter(|e| !e.is_empty())
            .unwrap_or_else(|| config.models.llm.endpoint.clone());
        if endpoint.is_empty() {
            anyhow::bail!(
                "ollama embedding provider needs an endpoint: set \
                 `models.embedding.endpoint` (or `models.llm.endpoint`)"
            );
        }
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
            .build()
            .context("kb-embed-ollama: build reqwest client")?;
        let (query_prefix, doc_prefix) = prefixes_for(&emb.model);
        Ok(Self {
            client,
            endpoint,
            model: emb.model.clone(),
            query_prefix,
            doc_prefix,
            model_id: EmbeddingModelId(emb.model.clone()),
            // model_version = `ollama:{model}` so a provider/model switch
            // triggers the embedding_version cascade and never collides with
            // the candle path's version string for the same model.
            version: EmbeddingVersion(format!("ollama:{}", emb.model)),
            dimensions: emb.dimensions,
        })
    }

    /// Embed one already-prefixed batch via `/api/embed`, with fail-soft retry.
    fn embed_batch(&self, prefixed: &[String]) -> Result<Vec<Vec<f32>>> {
        let url = format!("{}/api/embed", self.endpoint.trim_end_matches('/'));
        let body = EmbedRequest {
            model: &self.model,
            input: prefixed,
        };

        let mut last_err: Option<anyhow::Error> = None;
        for attempt in 1..=MAX_RETRIES {
            match self.try_once(&url, &body) {
                Ok(resp) => return self.finalize(resp, prefixed.len()),
                Err(e) => {
                    tracing::warn!(
                        target: "kebab-embed-ollama",
                        attempt,
                        max = MAX_RETRIES,
                        error = %e,
                        "ollama /api/embed attempt failed; retrying"
                    );
                    last_err = Some(e);
                }
            }
        }
        Err(last_err.unwrap_or_else(|| {
            anyhow::anyhow!("kb-embed-ollama: all {MAX_RETRIES} attempts failed")
        }))
    }

    /// One HTTP round-trip. Network / non-2xx / decode errors all map to
    /// `Err` so the retry loop can decide.
    fn try_once(&self, url: &str, body: &EmbedRequest<'_>) -> Result<EmbedResponse> {
        let resp = self
            .client
            .post(url)
            .json(body)
            .send()
            .with_context(|| format!("kb-embed-ollama: POST {url}"))?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().unwrap_or_default();
            anyhow::bail!("kb-embed-ollama: /api/embed returned {status}: {text}");
        }
        resp.json::<EmbedResponse>()
            .context("kb-embed-ollama: decode /api/embed response")
    }

    /// Validate count + dim, then L2-normalize each vector.
    fn finalize(&self, resp: EmbedResponse, expected: usize) -> Result<Vec<Vec<f32>>> {
        if resp.embeddings.len() != expected {
            anyhow::bail!(
                "kb-embed-ollama: expected {expected} embeddings, got {}",
                resp.embeddings.len()
            );
        }
        let mut out = Vec::with_capacity(resp.embeddings.len());
        for v in resp.embeddings {
            if v.len() != self.dimensions {
                anyhow::bail!(
                    "kb-embed-ollama: model returned dim {} but config expects {} \
                     (check models.embedding.dimensions vs the Ollama model)",
                    v.len(),
                    self.dimensions
                );
            }
            out.push(l2_normalize(v));
        }
        Ok(out)
    }
}

impl Embedder for OllamaEmbedder {
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
        let prefixed: Vec<String> = inputs.iter().map(|i| self.prefix(i)).collect();
        let mut out = Vec::with_capacity(prefixed.len());
        for chunk in prefixed.chunks(BATCH) {
            out.extend(self.embed_batch(chunk)?);
        }
        debug_assert_eq!(out.len(), inputs.len());
        Ok(out)
    }
}

impl OllamaEmbedder {
    /// Prefix one input per the resolved model prefixes.
    fn prefix(&self, input: &EmbeddingInput<'_>) -> String {
        match input.kind {
            EmbeddingKind::Document => format!("{}{}", self.doc_prefix, input.text),
            EmbeddingKind::Query => format!("{}{}", self.query_prefix, input.text),
        }
    }
}

/// L2-normalize a vector in place-ish (consumes + returns). A zero vector is
/// returned unchanged (norm 0 → no division) so a degenerate embedding can
/// never produce NaNs.
fn l2_normalize(mut v: Vec<f32>) -> Vec<f32> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in &mut v {
            *x /= norm;
        }
    }
    v
}

// ── Wire types ──────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct EmbedRequest<'a> {
    model: &'a str,
    input: &'a [String],
}

#[derive(Deserialize)]
struct EmbedResponse {
    embeddings: Vec<Vec<f32>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixes_for_arctic_is_query_only() {
        assert_eq!(prefixes_for("snowflake-arctic-embed2"), ("query: ", ""));
        assert_eq!(prefixes_for("snowflake-arctic-embed2:latest"), ("query: ", ""));
    }

    #[test]
    fn prefixes_for_e5_is_query_passage() {
        assert_eq!(prefixes_for("multilingual-e5-large"), ("query: ", "passage: "));
    }

    #[test]
    fn prefixes_for_unknown_is_bare() {
        assert_eq!(prefixes_for("nomic-embed-text"), ("", ""));
    }

    #[test]
    fn l2_normalize_unit_length() {
        let v = l2_normalize(vec![3.0, 4.0]);
        let norm = (v[0] * v[0] + v[1] * v[1]).sqrt();
        assert!((norm - 1.0).abs() < 1e-6, "norm = {norm}");
    }

    #[test]
    fn l2_normalize_zero_vector_is_unchanged() {
        assert_eq!(l2_normalize(vec![0.0, 0.0, 0.0]), vec![0.0, 0.0, 0.0]);
    }
}
