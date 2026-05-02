//! Caption adapter (P6-3).
//!
//! [`caption_image`] runs a vision-capable [`LanguageModel`] over an
//! image and produces a [`ModelCaption`]. [`apply_caption`] is the
//! helper that mutates an [`ImageRefBlock`] in place and emits a
//! [`ProvenanceKind::CaptionApplied`] event.
//!
//! ## Trust note
//!
//! Captions are **model-generated** (`TrustLevel::Generated`), not
//! observed text. Vision LMs hallucinate; the system prompt explicitly
//! forbids guessing but expect false captions. Downstream UI / RAG
//! must label captions as model-generated and surface the model id +
//! prompt template version (carried in `ModelCaption.model_version`)
//! so a regression in either is auditable.
//!
//! ## Spec deviation (cargo `caption` feature dropped)
//!
//! The original P6-3 spec asked for a cargo feature `caption` (default
//! OFF at compile time). We collapse this into a single runtime gate
//! (`config.image.caption.enabled = false`, default OFF). Reasoning:
//! the captioning module's only extra deps are `base64` + `image` +
//! `kebab-llm` trait — all already pulled in by the rest of the
//! crate. A cargo feature would only complicate the build matrix
//! without saving meaningful binary weight. See `tasks/HOTFIXES.md`
//! (2026-05-02) for the deviation log.

use std::io::Cursor;

use anyhow::{Context, Result};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use image::{ImageFormat, ImageReader};
use kebab_core::{
    FinishReason, GenerateRequest, ImageRefBlock, Lang, LanguageModel, ModelCaption,
    ProvenanceEvent, ProvenanceKind, TokenChunk,
};
use time::OffsetDateTime;

/// Long-edge clamp range for caption inputs. Smaller than OCR's
/// `[256, 4096]` because vision LMs charge proportionally to input
/// dimension — captions tolerate aggressive downscale better than
/// OCR.
const MIN_CAPTION_LONG_EDGE: u32 = 128;
const MAX_CAPTION_LONG_EDGE: u32 = 1536;

/// Token budget for captions. Captions are one-sentence by spec — 96
/// tokens covers a 50-word English sentence or a 30-token Korean one
/// with headroom for the LM's preamble before the stop sequence.
const CAPTION_MAX_TOKENS: usize = 96;

/// Run a caption pass and return the resulting `ModelCaption`. Honours
/// `config.image.caption.enabled` — when disabled the function is a
/// no-op and returns an `Err` so the caller can route the asset
/// through `apply_caption` instead, which knows to short-circuit.
///
/// Direct callers should prefer [`apply_caption`] for end-to-end
/// pipeline integration; this lower-level entry exists so tests can
/// pin the produced `ModelCaption` independent of block mutation.
pub fn caption_image(
    llm: &dyn LanguageModel,
    image_bytes: &[u8],
    lang_hint: Option<&Lang>,
    cfg: &kebab_config::Config,
) -> Result<ModelCaption> {
    if !cfg.image.caption.enabled {
        anyhow::bail!(
            "captioning is disabled (set image.caption.enabled = true in config to enable)"
        );
    }

    let max_pixels = cfg
        .image
        .caption
        .max_pixels
        .clamp(MIN_CAPTION_LONG_EDGE, MAX_CAPTION_LONG_EDGE);
    if max_pixels != cfg.image.caption.max_pixels {
        tracing::warn!(
            target: "kebab-parse-image",
            "image.caption.max_pixels = {} clamped to {} (legal range [{}, {}])",
            cfg.image.caption.max_pixels,
            max_pixels,
            MIN_CAPTION_LONG_EDGE,
            MAX_CAPTION_LONG_EDGE
        );
    }

    let prepared = downscale_to_png(image_bytes, max_pixels)
        .context("preparing image for caption")?;
    let b64 = BASE64_STANDARD.encode(&prepared);

    let lang = lang_hint
        .map(|l| l.0.as_str())
        .filter(|s| !s.is_empty() && *s != "und");
    let (system, user) = build_prompt(lang);

    // Determinism — temperature 0.0 + seed 0, same convention as RAG
    // and OCR. The LM adapter routes the base64 image via its
    // provider-specific channel (Ollama: `images: [base64]`).
    let req = GenerateRequest {
        system,
        user,
        stop: vec!["\n\n".to_string()],
        max_tokens: CAPTION_MAX_TOKENS,
        temperature: 0.0,
        seed: Some(0),
        images: vec![b64],
    };

    let stream = llm
        .generate_stream(req)
        .context("captioning LM call failed")?;

    let mut text = String::new();
    let mut saw_done = false;
    for chunk in stream {
        match chunk? {
            TokenChunk::Token(t) => {
                text.push_str(&t);
            }
            TokenChunk::Done { finish_reason, .. } => {
                saw_done = true;
                if let FinishReason::Error(e) = finish_reason {
                    anyhow::bail!("captioning LM ended with error: {e}");
                }
                break;
            }
        }
    }
    if !saw_done {
        anyhow::bail!("captioning LM stream ended without a Done frame");
    }

    let caption_text = text.trim().to_string();

    let model_ref = llm.model_ref();
    let prompt_v = &cfg.image.caption.prompt_template_version;
    let model_version = format!(
        "{provider}/{prompt}",
        provider = model_ref.provider,
        prompt = prompt_v
    );

    tracing::debug!(
        target: "kebab-parse-image",
        "caption ok (model={}, prompt={}, chars={})",
        model_ref.id,
        prompt_v,
        caption_text.chars().count()
    );

    Ok(ModelCaption {
        text: caption_text,
        model: model_ref.id,
        model_version,
    })
}

/// Mutate `block.caption` in place by running `caption_image` over
/// `image_bytes`. When `config.image.caption.enabled = false` the
/// function is a clean no-op (returns `Ok(())` without invoking the
/// LM and without writing a Provenance event).
///
/// On LM failure, `block.caption` stays `None` — partial state is
/// never written. The caller decides whether to skip the asset or
/// surface the error.
pub fn apply_caption(
    llm: &dyn LanguageModel,
    image_bytes: &[u8],
    block: &mut ImageRefBlock,
    lang_hint: Option<&Lang>,
    cfg: &kebab_config::Config,
    events: &mut Vec<ProvenanceEvent>,
) -> Result<()> {
    if !cfg.image.caption.enabled {
        tracing::debug!(
            target: "kebab-parse-image",
            "captioning skipped — image.caption.enabled = false"
        );
        return Ok(());
    }
    let caption = caption_image(llm, image_bytes, lang_hint, cfg)?;
    let model_label = caption.model.clone();
    let model_version_label = caption.model_version.clone();
    block.caption = Some(caption);
    events.push(ProvenanceEvent {
        at: OffsetDateTime::now_utc(),
        agent: "kb-parse-image".to_string(),
        kind: ProvenanceKind::CaptionApplied,
        note: Some(format!(
            "model={model_label} model_version={model_version_label}"
        )),
    });
    Ok(())
}

/// Compose the `(system, user)` prompt pair for the caption call.
/// Korean / English split keeps the model on the requested output
/// language; everything else falls through to English.
fn build_prompt(lang_hint: Option<&str>) -> (String, String) {
    match lang_hint {
        Some("ko") | Some("kor") => (
            "이미지를 한 문장으로 객관적으로 설명한다. 추측은 피하고, \
             보이는 것만 적는다. 마크다운 / 따옴표 / 부가 설명 없이 \
             한 문장만 출력."
                .to_string(),
            "위 이미지를 한국어로 한 문장으로 설명하라.".to_string(),
        ),
        _ => (
            "Describe the image in one objective sentence. Do not \
             speculate; describe only what is visible. No markdown, \
             no quotes, no commentary — output a single sentence."
                .to_string(),
            "Describe the image above in one English sentence.".to_string(),
        ),
    }
}

/// Decode `bytes`, downscale long-edge to `max_long_edge`, re-encode as
/// PNG. Mirrors the OCR pipeline's pattern but with the caption-side
/// long-edge bounds. PNG sources within the cap pass through without
/// re-encode.
fn downscale_to_png(bytes: &[u8], max_long_edge: u32) -> Result<Vec<u8>> {
    let reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .context("reading image header for caption")?;
    let format = reader.format();
    let (w, h) = reader
        .into_dimensions()
        .context("reading image dimensions for caption")?;

    let long = w.max(h);
    if long <= max_long_edge && format == Some(ImageFormat::Png) {
        return Ok(bytes.to_vec());
    }

    let img = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .context("re-reading image for caption decode")?
        .decode()
        .context("decoding image for caption")?;
    let final_img = if long <= max_long_edge {
        img
    } else {
        let scale = max_long_edge as f32 / long as f32;
        let mut new_w = ((w as f32) * scale).round().max(1.0) as u32;
        let mut new_h = ((h as f32) * scale).round().max(1.0) as u32;
        if w >= h {
            new_w = new_w.min(max_long_edge);
        } else {
            new_h = new_h.min(max_long_edge);
        }
        img.resize_exact(new_w, new_h, image::imageops::FilterType::Triangle)
    };

    let mut out = Cursor::new(Vec::new());
    final_img
        .write_to(&mut out, ImageFormat::Png)
        .context("encoding image as PNG for caption")?;
    Ok(out.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_prompt_korean_for_ko_hint() {
        let (sys, user) = build_prompt(Some("ko"));
        assert!(sys.contains("이미지를 한 문장으로"));
        assert!(user.contains("한국어로"));
    }

    #[test]
    fn build_prompt_english_for_no_hint_or_und() {
        let (sys, _) = build_prompt(None);
        assert!(sys.contains("Describe the image"));
        let (sys2, _) = build_prompt(Some("en"));
        assert!(sys2.contains("Describe the image"));
    }
}
