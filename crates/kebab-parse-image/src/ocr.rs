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

use std::io::Cursor;
use std::time::Duration;

use anyhow::{Context, Result};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use image::{ImageFormat, ImageReader};
use kebab_core::{ImageRefBlock, Lang, OcrRegion, OcrText, ProvenanceEvent, ProvenanceKind};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

/// Engine name written into `OcrText.engine` for the Ollama-vision adapter.
pub const OLLAMA_VISION_ENGINE: &str = "ollama-vision";

/// Hard ceiling on the OCR HTTP exchange. Cold-loading a vision model on
/// first call can take ~30s; 5 minutes is generous without being open-ended.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(300);

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
    fn recognize(
        &self,
        image_bytes: &[u8],
        lang_hint: Option<&Lang>,
    ) -> Result<OcrText>;
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
        if endpoint.is_empty() {
            anyhow::bail!(
                "OllamaVisionOcr: endpoint is empty (set image.ocr.endpoint or models.llm.endpoint)"
            );
        }
        let model = ocr.model.trim().to_string();
        if model.is_empty() {
            anyhow::bail!("OllamaVisionOcr: image.ocr.model is empty");
        }
        let max_pixels = ocr.max_pixels.clamp(MIN_LONG_EDGE, MAX_LONG_EDGE);
        let client = reqwest::blocking::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .context("building OCR HTTP client")?;
        Ok(Self {
            client,
            endpoint,
            model,
            languages: ocr.languages.clone(),
            max_pixels,
        })
    }

    /// Build directly from explicit fields. Useful for tests that need
    /// to point at a wiremock host without going through `Config`.
    pub fn from_parts(
        endpoint: impl Into<String>,
        model: impl Into<String>,
        languages: Vec<String>,
        max_pixels: u32,
    ) -> Result<Self> {
        let client = reqwest::blocking::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .context("building OCR HTTP client")?;
        Ok(Self {
            client,
            endpoint: endpoint.into(),
            model: model.into(),
            languages,
            max_pixels: max_pixels.clamp(MIN_LONG_EDGE, MAX_LONG_EDGE),
        })
    }

    /// Effective `max_pixels` after the `[MIN_LONG_EDGE, MAX_LONG_EDGE]`
    /// clamp. Exposed so tests can verify the clamp result without
    /// reaching into the private field; production callers don't need
    /// it.
    pub fn max_pixels(&self) -> u32 {
        self.max_pixels
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

    fn recognize(
        &self,
        image_bytes: &[u8],
        lang_hint: Option<&Lang>,
    ) -> Result<OcrText> {
        let (prepared, w, h) = downscale_to_long_edge(image_bytes, self.max_pixels)
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
        let parsed: OllamaGenerateResponse = resp
            .json()
            .context("parsing Ollama OCR response as JSON")?;
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

// ── Image preparation ─────────────────────────────────────────────────────

/// Decode `bytes`, downscale so the long edge is at most `max_long_edge`,
/// and re-encode as PNG. Returns `(png_bytes, final_w, final_h)`.
///
/// PNG sources that already fit the cap are passthrough (zero decodes,
/// just a `Vec` clone). Every other path decodes the image exactly
/// once: the cheap header sniff peeks at the format / dimensions before
/// committing to a decode, so non-PNG passthrough and downscale share
/// the same `decode → optionally resize → re-encode` tail.
fn downscale_to_long_edge(bytes: &[u8], max_long_edge: u32) -> Result<(Vec<u8>, u32, u32)> {
    let reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .context("reading image header for OCR")?;
    let format = reader.format();
    let (w, h) = reader
        .into_dimensions()
        .context("reading image dimensions for OCR")?;

    let long = w.max(h);

    // Hot path — PNG within budget already matches the wire format we
    // send Ollama, so we ship the bytes verbatim without paying for a
    // decode + re-encode round-trip.
    if long <= max_long_edge && format == Some(ImageFormat::Png) {
        return Ok((bytes.to_vec(), w, h));
    }

    // Every remaining branch needs the pixels — either to re-encode as
    // PNG (non-PNG within budget) or to resize first (over budget).
    // One decode covers both.
    let img = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .context("re-reading image for OCR decode")?
        .decode()
        .context("decoding image for OCR")?;

    let (final_w, final_h, final_img) = if long <= max_long_edge {
        (w, h, img)
    } else {
        let scale = max_long_edge as f32 / long as f32;
        let new_w = ((w as f32) * scale).round().max(1.0) as u32;
        let new_h = ((h as f32) * scale).round().max(1.0) as u32;
        let resized =
            img.resize_exact(new_w, new_h, image::imageops::FilterType::Triangle);
        (new_w, new_h, resized)
    };

    let mut out = Cursor::new(Vec::new());
    final_img
        .write_to(&mut out, ImageFormat::Png)
        .context("encoding image as PNG for OCR")?;
    Ok((out.into_inner(), final_w, final_h))
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    let mut out: String = s.chars().take(n).collect();
    out.push_str(&format!("... (truncated, original {} chars)", s.chars().count()));
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
        )
        .unwrap();
        let p = engine.build_prompt(Some(&Lang("ko".into())));
        assert!(p.contains("eng, kor"));
        assert!(p.contains("hint: dominant language is ko"));
    }

    #[test]
    fn build_prompt_omits_hint_when_lang_und() {
        let engine = OllamaVisionOcr::from_parts(
            "http://x",
            "m",
            vec!["eng".into()],
            1024,
        )
        .unwrap();
        let p = engine.build_prompt(Some(&Lang("und".into())));
        assert!(!p.contains("hint:"));
    }
}
