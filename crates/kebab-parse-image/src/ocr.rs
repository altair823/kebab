//! OCR adapter (P6-2).
//!
//! [`OcrEngine`] is a small trait for "image bytes → [`OcrText`]". v1 ships
//! a single implementation, [`OllamaVisionOcr`], which delegates to a
//! vision-capable Ollama model (`gemma4:e4b` by default).
//!
//! ## Spec deviation (Tesseract → Ollama-vision)
//!
//! The original P6-2 spec named Tesseract as the default engine. The dev
//! / CI environment intentionally avoids system-package installs, so the
//! Tesseract Rust crate (which links `libtesseract`) is impractical
//! today. We keep the [`OcrEngine`] trait as the abstraction the spec
//! demanded — Tesseract / Apple Vision / PaddleOCR plug in as future
//! feature-gated alternatives without touching the extractor or
//! chunker. See `tasks/HOTFIXES.md` (2026-05-02) for the full
//! rationale.
//!
//! ## Trust note
//!
//! The original spec marked `OcrText` as "observed text (high trust)"
//! to distinguish it from `ModelCaption`. With an LLM-driven OCR engine
//! the line blurs — the model can hallucinate. Downstream consumers
//! that surface OCR text should still treat it as a hint, not ground
//! truth, and prefer the asset bytes when verifying. The `engine`
//! field on [`OcrText`] makes the source explicit, so a caller can
//! decide whether to trust based on which engine produced the text.

use std::time::Duration;

use anyhow::{Context, Result};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use kebab_core::{ImageRefBlock, Lang, OcrRegion, OcrText, ProvenanceEvent, ProvenanceKind};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::image_prep;

/// Engine name written into `OcrText.engine` for the Ollama-vision adapter.
pub const OLLAMA_VISION_ENGINE: &str = "ollama-vision";

/// Lower bound on `config.image.ocr.max_pixels`. Anything below this is
/// silently bumped to keep the model from receiving an unreadable thumbnail.
const MIN_LONG_EDGE: u32 = 256;

/// Hard cap on `max_pixels` — the spec mentions "downscale aggressively"
/// for vision LMs because input dimension translates directly into
/// prompt cost. 4096 is generous for legibility and still bounded.
const MAX_LONG_EDGE: u32 = 4096;

/// Image-bytes → [`OcrText`] interface. Implementations may shell out
/// (Apple Vision sidecar), call a local library (Tesseract), or — in v1
/// — talk HTTP to a vision LM (Ollama).
pub trait OcrEngine: Send + Sync {
    /// Stable identifier written into `OcrText.engine`. Used by callers
    /// to decide trust level (observed vs. generated).
    fn engine_name(&self) -> &'static str;

    /// Engine version string written into `OcrText.engine_version`.
    /// Adapters that depend on a remote service may include the model
    /// id / version here.
    fn engine_version(&self) -> String;

    /// Run OCR on `image_bytes`. `lang_hint` (BCP-47) can be passed
    /// through to engines that benefit from it (Tesseract languages,
    /// LLM prompt steering); ignore otherwise.
    fn recognize(&self, image_bytes: &[u8], lang_hint: Option<&Lang>) -> Result<OcrText>;
}

/// Mutate `block.ocr` in place by running `engine` over `image_bytes`,
/// then append a [`ProvenanceKind::OcrApplied`] event to `events` so the
/// caller (which owns the `CanonicalDocument`) can splice it into
/// `provenance.events`.
///
/// Returns the engine error verbatim on failure so the caller can decide
/// whether to skip the asset or surface it. `block.ocr` is left
/// untouched on error — partial state is never written.
pub fn apply_ocr(
    engine: &dyn OcrEngine,
    image_bytes: &[u8],
    block: &mut ImageRefBlock,
    lang_hint: Option<&Lang>,
    events: &mut Vec<ProvenanceEvent>,
) -> Result<()> {
    let text = engine.recognize(image_bytes, lang_hint).with_context(|| {
        format!(
            "OCR failed (engine={}, version={})",
            engine.engine_name(),
            engine.engine_version()
        )
    })?;
    let region_count = text.regions.len();
    block.ocr = Some(text);
    events.push(ProvenanceEvent {
        at: OffsetDateTime::now_utc(),
        agent: "kb-parse-image".to_string(),
        kind: ProvenanceKind::OcrApplied,
        note: Some(format!(
            "engine={} version={} regions={}",
            engine.engine_name(),
            engine.engine_version(),
            region_count
        )),
    });
    Ok(())
}

/// Ollama-vision OCR adapter — POSTs the image (base64) to
/// `<endpoint>/api/generate` with a transcription prompt and reads the
/// non-streaming response.
#[derive(Debug)]
pub struct OllamaVisionOcr {
    client: reqwest::blocking::Client,
    endpoint: String,
    model: String,
    languages: Vec<String>,
    max_pixels: u32,
}

impl OllamaVisionOcr {
    /// Build an adapter from a workspace [`kebab_config::Config`].
    /// Reads `config.image.ocr.{model, endpoint, languages, max_pixels}`;
    /// when `endpoint` is empty falls back to `config.models.llm.endpoint`
    /// so the same Ollama host serves both LLM and OCR by default.
    ///
    /// Construction does NOT touch the network — the first HTTP call
    /// happens inside [`OcrEngine::recognize`].
    pub fn new(config: &kebab_config::Config) -> Result<Self> {
        let ocr = &config.image.ocr;
        let endpoint = match ocr.endpoint.as_deref() {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => config.models.llm.endpoint.clone(),
        };
        Self::build(
            endpoint,
            ocr.model.clone(),
            ocr.languages.clone(),
            ocr.max_pixels,
            ocr.request_timeout_secs,
        )
    }

    /// Build directly from explicit fields. Useful for tests that need
    /// to point at a wiremock host without going through `Config`.
    /// Shares the same input validation as [`Self::new`] so the two
    /// constructors agree on what counts as a legal `OllamaVisionOcr` —
    /// callers cannot smuggle an empty endpoint or empty model id past
    /// `from_parts`.
    pub fn from_parts(
        endpoint: impl Into<String>,
        model: impl Into<String>,
        languages: Vec<String>,
        max_pixels: u32,
        request_timeout_secs: u64,
    ) -> Result<Self> {
        Self::build(
            endpoint.into(),
            model.into(),
            languages,
            max_pixels,
            request_timeout_secs,
        )
    }

    /// Shared validation + construction. Centralised so `new` and
    /// `from_parts` cannot drift on what they accept.
    fn build(
        endpoint: String,
        model: String,
        languages: Vec<String>,
        requested_max_pixels: u32,
        request_timeout_secs: u64,
    ) -> Result<Self> {
        if endpoint.is_empty() {
            anyhow::bail!(
                "OllamaVisionOcr: endpoint is empty (set image.ocr.endpoint or models.llm.endpoint)"
            );
        }
        let model = model.trim().to_string();
        if model.is_empty() {
            anyhow::bail!("OllamaVisionOcr: model is empty");
        }
        let max_pixels = requested_max_pixels.clamp(MIN_LONG_EDGE, MAX_LONG_EDGE);
        if max_pixels != requested_max_pixels {
            tracing::warn!(
                target: "kebab-parse-image",
                "image.ocr.max_pixels = {requested_max_pixels} clamped to {max_pixels} \
                 (legal range [{MIN_LONG_EDGE}, {MAX_LONG_EDGE}])"
            );
        }
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(request_timeout_secs))
            .build()
            .context("building OCR HTTP client")?;
        Ok(Self {
            client,
            endpoint,
            model,
            languages,
            max_pixels,
        })
    }

    /// Effective `max_pixels` after the `[MIN_LONG_EDGE, MAX_LONG_EDGE]`
    /// clamp. Exposed so tests can verify the clamp result without
    /// reaching into the private field; production callers don't need
    /// it.
    pub fn max_pixels(&self) -> u32 {
        self.max_pixels
    }

    /// The Ollama model id this engine drives (e.g. `gemma4:e4b`).
    /// Surfaced so the ingest progress display can name the model
    /// running a slow OCR phase (`AssetPhase{phase:"ocr", model}`).
    pub fn model(&self) -> &str {
        &self.model
    }

    fn build_prompt(&self, lang_hint: Option<&Lang>) -> String {
        let langs = if self.languages.is_empty() {
            "any".to_string()
        } else {
            self.languages.join(", ")
        };
        let hint = match lang_hint.map(|l| l.0.as_str()) {
            Some(h) if !h.is_empty() && h != "und" => format!(" (hint: dominant language is {h})"),
            _ => String::new(),
        };
        format!(
            "You are an OCR engine. Transcribe ALL text visible in the image, \
             preserving line breaks. Output only the transcription, no commentary, \
             no markdown fences, no quotes. Expected languages: {langs}{hint}. \
             If the image contains no text, output an empty line."
        )
    }
}

impl OcrEngine for OllamaVisionOcr {
    fn engine_name(&self) -> &'static str {
        OLLAMA_VISION_ENGINE
    }

    fn engine_version(&self) -> String {
        // Compose engine + model id so the wire form is self-describing
        // ("ollama-vision/gemma4:e4b") — the Ollama daemon does not
        // expose a stable per-model revision string we could pin.
        format!("ollama/{}", self.model)
    }

    fn recognize(&self, image_bytes: &[u8], lang_hint: Option<&Lang>) -> Result<OcrText> {
        let (prepared, w, h) = image_prep::downscale_to_png(image_bytes, self.max_pixels)
            .context("preparing image for OCR")?;
        let b64 = BASE64_STANDARD.encode(&prepared);

        let prompt = self.build_prompt(lang_hint);
        let body = OllamaGenerateRequest {
            model: &self.model,
            prompt: &prompt,
            images: [b64.as_str()],
            stream: false,
            options: OllamaOptions {
                temperature: 0.0,
                seed: 0,
            },
        };

        let url = format!("{}/api/generate", self.endpoint.trim_end_matches('/'));
        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .with_context(|| format!("POST {url}"))?;
        let status = resp.status();
        if !status.is_success() {
            let body_text = resp.text().unwrap_or_default();
            anyhow::bail!(
                "OllamaVisionOcr: {status} from {url} — body={}",
                truncate(&body_text, 512)
            );
        }
        let parsed: OllamaGenerateResponse =
            resp.json().context("parsing Ollama OCR response as JSON")?;
        if let Some(err) = parsed.error {
            anyhow::bail!("OllamaVisionOcr: server error — {}", truncate(&err, 512));
        }
        let raw = parsed.response.unwrap_or_default();
        let joined = raw.trim().to_string();

        let regions = if joined.is_empty() {
            Vec::new()
        } else {
            // Ollama-vision returns prose, not bbox-annotated regions.
            // We synthesize a single region covering the whole prepared
            // image (post-downscale dimensions) so the `OcrText` shape
            // remains compatible with consumers that expect at least
            // one region. Confidence is left at 1.0 — there's no
            // per-token score available from the LM.
            vec![OcrRegion {
                bbox: (0, 0, w, h),
                text: joined.clone(),
                confidence: 1.0,
            }]
        };

        tracing::debug!(
            target: "kebab-parse-image",
            "ollama-vision OCR ok (model={}, dims={w}x{h}, chars={})",
            self.model,
            joined.chars().count()
        );

        Ok(OcrText {
            joined,
            regions,
            engine: self.engine_name().to_string(),
            engine_version: self.engine_version(),
        })
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    let mut out: String = s.chars().take(n).collect();
    out.push_str(&format!(
        "... (truncated, original {} chars)",
        s.chars().count()
    ));
    out
}

// ── Wire types ────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct OllamaGenerateRequest<'a> {
    model: &'a str,
    prompt: &'a str,
    /// Always exactly one image — the `OcrEngine` trait takes a single
    /// `&[u8]`, so multi-image batching is out of scope until a future
    /// trait extension. Fixed-size array avoids the `vec![]`
    /// allocation per call.
    images: [&'a str; 1],
    stream: bool,
    options: OllamaOptions,
}

#[derive(Serialize)]
struct OllamaOptions {
    temperature: f32,
    seed: u64,
}

#[derive(Deserialize)]
struct OllamaGenerateResponse {
    #[serde(default)]
    response: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_under_cap_unchanged() {
        assert_eq!(truncate("abc", 5), "abc");
    }

    #[test]
    fn truncate_over_cap_appends_marker() {
        let big = "x".repeat(20);
        let out = truncate(&big, 5);
        assert!(out.starts_with("xxxxx"));
        assert!(out.contains("(truncated, original 20 chars)"));
    }

    /// Build prompt mentions the configured languages and the hint when
    /// supplied.
    #[test]
    fn build_prompt_lists_languages_and_hint() {
        let engine = OllamaVisionOcr::from_parts(
            "http://x",
            "m",
            vec!["eng".into(), "kor".into()],
            1024,
            300,
        )
        .unwrap();
        let p = engine.build_prompt(Some(&Lang("ko".into())));
        assert!(p.contains("eng, kor"));
        assert!(p.contains("hint: dominant language is ko"));
    }

    #[test]
    fn build_prompt_omits_hint_when_lang_und() {
        let engine =
            OllamaVisionOcr::from_parts("http://x", "m", vec!["eng".into()], 1024, 300).unwrap();
        let p = engine.build_prompt(Some(&Lang("und".into())));
        assert!(!p.contains("hint:"));
    }

    /// `from_parts` (and by extension `new`) must reject an empty
    /// endpoint string. Pinned so the bail message stays grep-able and
    /// the constructor cannot drift to "silently accept a bad config".
    #[test]
    fn build_rejects_empty_endpoint() {
        let r = OllamaVisionOcr::from_parts("", "m", vec![], 1024, 300);
        let err = r.expect_err("empty endpoint must bail").to_string();
        assert!(
            err.contains("endpoint is empty"),
            "bail message missing 'endpoint is empty': {err}"
        );
    }

    /// Whitespace-only model id trims to empty and must be rejected —
    /// both `new` and `from_parts` route through the shared `build`,
    /// so testing `from_parts` covers both.
    #[test]
    fn build_rejects_empty_model_after_trim() {
        let r = OllamaVisionOcr::from_parts("http://x", "   ", vec![], 1024, 300);
        let err = r.expect_err("empty model must bail").to_string();
        assert!(
            err.contains("model is empty"),
            "bail message missing 'model is empty': {err}"
        );
    }

    /// Out-of-range `max_pixels` is silently clamped (not rejected) so
    /// a bad config can't kill ingest. The accessor exposes the clamped
    /// value so tests can verify the bound; the warning side-effect is
    /// tested implicitly (no panic, no error).
    #[test]
    fn build_clamps_max_pixels_outside_legal_range() {
        let too_small = OllamaVisionOcr::from_parts("http://x", "m", vec![], 1, 300).unwrap();
        assert_eq!(too_small.max_pixels(), MIN_LONG_EDGE);
        let too_big = OllamaVisionOcr::from_parts("http://x", "m", vec![], u32::MAX, 300).unwrap();
        assert_eq!(too_big.max_pixels(), MAX_LONG_EDGE);
    }
}
