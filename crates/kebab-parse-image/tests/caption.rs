//! Integration tests for the caption adapter (P6-3).
//!
//! All hermetic tests use `MockLanguageModel` from `kebab-llm/mock`
//! which captures `req.images` indirectly via the canned response. A
//! single opt-in test (`#[ignore]`) wires the real
//! `kebab-llm-local::OllamaLanguageModel` against the workspace's
//! Ollama daemon to verify the `images: [base64]` round-trip.

mod common;

use std::sync::{Arc, Mutex};

use kebab_config::Config;
use kebab_core::{
    AssetId, BlockId, CommonBlock, FinishReason, GenerateRequest, ImageRefBlock, Lang,
    LanguageModel, ModelRef, ProvenanceEvent, ProvenanceKind, SourceSpan, TokenChunk, TokenUsage,
};
use kebab_llm::MockLanguageModel;
use kebab_parse_image::{apply_caption, caption_image};

use crate::common::red_100x50_png;

fn cfg_with_caption_enabled() -> Config {
    let mut cfg = Config::defaults();
    cfg.ingest.image.caption.enabled = true;
    cfg.ingest.image.caption.max_pixels = 512;
    cfg
}

fn empty_image_block() -> ImageRefBlock {
    ImageRefBlock {
        common: CommonBlock {
            block_id: BlockId("0".repeat(32)),
            heading_path: Vec::new(),
            source_span: SourceSpan::Region {
                x: 0,
                y: 0,
                w: 100,
                h: 50,
            },
        },
        asset_id: Some(AssetId("a".repeat(32))),
        src: "img/x.png".to_string(),
        alt: "x.png".to_string(),
        ocr: None,
        caption: None,
    }
}

fn mk_mock(canned: &str) -> MockLanguageModel {
    MockLanguageModel {
        model_id: "vision-mock:1b".to_string(),
        provider: "mock".to_string(),
        context_tokens: 4096,
        canned_response: canned.to_string(),
        canned_finish: FinishReason::Stop,
        canned_usage: TokenUsage {
            prompt_tokens: 0,
            completion_tokens: 0,
            latency_ms: 0,
        },
    }
}

// ── Disabled feature gate ─────────────────────────────────────────────────

#[test]
fn apply_caption_no_op_when_feature_disabled() {
    let mut cfg = Config::defaults();
    cfg.ingest.image.caption.enabled = false;
    let mock = mk_mock("ignored");
    let mut block = empty_image_block();
    let mut events: Vec<ProvenanceEvent> = Vec::new();
    let bytes = red_100x50_png();
    apply_caption(&mock, &bytes, &mut block, None, &cfg, &mut events)
        .expect("disabled apply_caption must return Ok(())");
    assert!(
        block.caption.is_none(),
        "disabled apply_caption must not write caption"
    );
    assert!(
        events.is_empty(),
        "disabled apply_caption must not append a Provenance event"
    );
}

#[test]
fn caption_image_runs_regardless_of_enabled_flag() {
    // Feature gate lives in `apply_caption`; `caption_image` is the
    // raw operation. Calling it directly with enabled = false must
    // still produce a `ModelCaption` so tests can pin the produced
    // shape independent of pipeline gating.
    let cfg = Config::defaults(); // enabled = false (default)
    let mock = mk_mock("hi");
    let bytes = red_100x50_png();
    let cap = caption_image(&mock, &bytes, None, &cfg)
        .expect("caption_image runs even when enabled = false");
    assert_eq!(cap.text, "hi");
}

// ── Happy path ────────────────────────────────────────────────────────────

#[test]
fn apply_caption_sets_block_caption_and_appends_provenance() {
    let cfg = cfg_with_caption_enabled();
    let mock = mk_mock("사진 한 장");
    let mut block = empty_image_block();
    let mut events: Vec<ProvenanceEvent> = Vec::new();
    let bytes = red_100x50_png();
    apply_caption(
        &mock,
        &bytes,
        &mut block,
        Some(&Lang("ko".to_string())),
        &cfg,
        &mut events,
    )
    .expect("apply_caption must succeed");

    let cap = block.caption.as_ref().expect("caption Some");
    assert_eq!(cap.text, "사진 한 장");
    assert_eq!(cap.model, "vision-mock:1b");
    assert_eq!(cap.model_version, "mock/caption-v1");

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, ProvenanceKind::CaptionApplied);
    assert_eq!(events[0].agent, "kb-parse-image");
    let note = events[0].note.as_deref().unwrap_or("");
    assert!(
        note.contains("vision-mock:1b") && note.contains("caption-v1"),
        "{note}"
    );
}

// ── Empty token stream → empty caption text ──────────────────────────────

#[test]
fn caption_image_empty_stream_yields_empty_text() {
    let cfg = cfg_with_caption_enabled();
    let mock = mk_mock("");
    let bytes = red_100x50_png();
    let cap = caption_image(&mock, &bytes, None, &cfg).expect("empty stream must succeed");
    assert_eq!(cap.text, "");
    // Spec contract: caller can distinguish "captioning attempted, no
    // result" from "captioning never attempted" by `caption.is_some()`.
    // The text being empty does not erase the attempt.
    assert!(!cap.model.is_empty());
}

// ── Korean vs English prompt selection ───────────────────────────────────

/// `LanguageModel` impl that captures the `system` prompt sent to it
/// so tests can verify the language branch picked by `build_prompt`
/// (the function is private; this is the cleanest observable signal).
struct CapturingMock {
    captured_system: Arc<Mutex<Option<String>>>,
    captured_images: Arc<Mutex<Vec<String>>>,
}

impl LanguageModel for CapturingMock {
    fn model_ref(&self) -> ModelRef {
        ModelRef {
            id: "capture:1".to_string(),
            provider: "mock".to_string(),
            dimensions: None,
        }
    }
    fn context_tokens(&self) -> usize {
        4096
    }
    fn generate_stream(
        &self,
        req: GenerateRequest,
    ) -> anyhow::Result<Box<dyn Iterator<Item = anyhow::Result<TokenChunk>> + Send>> {
        *self.captured_system.lock().unwrap() = Some(req.system);
        *self.captured_images.lock().unwrap() = req.images;
        let chunks: Vec<TokenChunk> = vec![
            TokenChunk::Token("ok".to_string()),
            TokenChunk::Done {
                finish_reason: FinishReason::Stop,
                usage: TokenUsage {
                    prompt_tokens: 0,
                    completion_tokens: 0,
                    latency_ms: 0,
                },
            },
        ];
        Ok(Box::new(chunks.into_iter().map(Ok)))
    }
}

#[test]
fn caption_image_routes_image_into_request_images_field() {
    let cfg = cfg_with_caption_enabled();
    let captured_system: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let captured_images: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let mock = CapturingMock {
        captured_system: captured_system.clone(),
        captured_images: captured_images.clone(),
    };
    let bytes = red_100x50_png();
    let _ = caption_image(&mock, &bytes, Some(&Lang("ko".to_string())), &cfg)
        .expect("caption succeeds");

    let imgs = captured_images.lock().unwrap();
    assert_eq!(imgs.len(), 1, "exactly one base64 image routed");
    use base64::Engine as _;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(&imgs[0])
        .expect("base64 decodes");
    assert!(!decoded.is_empty(), "decoded image bytes must be non-empty");

    let sys = captured_system.lock().unwrap().clone().unwrap();
    assert!(
        sys.contains("이미지를 한 문장으로"),
        "Korean hint must produce Korean system prompt: {sys}"
    );
}

#[test]
fn caption_image_uses_english_prompt_for_undetermined_lang() {
    let cfg = cfg_with_caption_enabled();
    let captured_system: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let mock = CapturingMock {
        captured_system: captured_system.clone(),
        captured_images: Arc::new(Mutex::new(Vec::new())),
    };
    let bytes = red_100x50_png();
    let _ = caption_image(&mock, &bytes, Some(&Lang("und".to_string())), &cfg)
        .expect("caption succeeds");
    let sys = captured_system.lock().unwrap().clone().unwrap();
    assert!(sys.contains("Describe the image"), "{sys}");
}

// ── LM error propagates ──────────────────────────────────────────────────

/// LM that returns Err immediately from `generate_stream` (before any
/// token).
struct FailingLm;
impl LanguageModel for FailingLm {
    fn model_ref(&self) -> ModelRef {
        ModelRef {
            id: "fail".into(),
            provider: "mock".into(),
            dimensions: None,
        }
    }
    fn context_tokens(&self) -> usize {
        0
    }
    fn generate_stream(
        &self,
        _req: GenerateRequest,
    ) -> anyhow::Result<Box<dyn Iterator<Item = anyhow::Result<TokenChunk>> + Send>> {
        Err(anyhow::anyhow!("simulated LM connection refused"))
    }
}

#[test]
fn apply_caption_lm_error_leaves_block_untouched() {
    let cfg = cfg_with_caption_enabled();
    let mut block = empty_image_block();
    let mut events: Vec<ProvenanceEvent> = Vec::new();
    let bytes = red_100x50_png();
    let r = apply_caption(&FailingLm, &bytes, &mut block, None, &cfg, &mut events);
    assert!(r.is_err());
    assert!(
        block.caption.is_none(),
        "caption stays None when LM fails — partial state must not leak"
    );
    assert!(events.is_empty(), "no provenance event when LM fails");
}

// ── Determinism — identical mock input → identical caption ───────────────

#[test]
fn caption_image_deterministic_with_identical_inputs() {
    let cfg = cfg_with_caption_enabled();
    let bytes = red_100x50_png();
    let mock1 = mk_mock("a deterministic caption");
    let mock2 = mk_mock("a deterministic caption");
    let cap1 = caption_image(&mock1, &bytes, None, &cfg).unwrap();
    let cap2 = caption_image(&mock2, &bytes, None, &cfg).unwrap();
    assert_eq!(cap1, cap2);
}

// ── max_pixels clamp ─────────────────────────────────────────────────────

/// Out-of-range `max_pixels` is silently clamped at construction so a
/// bad config can't kill ingest. The captured `images` field's
/// decoded long edge confirms the clamp engaged.
#[test]
fn caption_image_clamps_oversized_max_pixels() {
    let mut cfg = Config::defaults();
    cfg.ingest.image.caption.enabled = true;
    cfg.ingest.image.caption.max_pixels = 99_999; // way over MAX_CAPTION_LONG_EDGE
    let captured_images: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let mock = CapturingMock {
        captured_system: Arc::new(Mutex::new(None)),
        captured_images: captured_images.clone(),
    };
    // 4000×3000 PNG well above the 1536 cap.
    let bytes = common::large_blue_4000x3000_png();
    let _ = caption_image(&mock, &bytes, None, &cfg).expect("caption succeeds");
    let imgs = captured_images.lock().unwrap();
    use base64::Engine as _;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(&imgs[0])
        .unwrap();
    let reader = image::ImageReader::new(std::io::Cursor::new(decoded))
        .with_guessed_format()
        .unwrap();
    let (w, h) = reader.into_dimensions().unwrap();
    let long = w.max(h);
    assert!(
        long <= kebab_parse_image::caption::MAX_CAPTION_LONG_EDGE,
        "max_pixels must clamp to MAX_CAPTION_LONG_EDGE={}, got {long}",
        kebab_parse_image::caption::MAX_CAPTION_LONG_EDGE
    );
}

// ── Real Ollama integration (opt-in) ─────────────────────────────────────

/// End-to-end captioning against the workspace's real Ollama daemon
/// via `kebab-llm-local::OllamaLanguageModel` (dev-dep). Skipped by
/// default via `#[ignore]`; opt in with `--ignored`.
///
/// Run with:
///
/// ```sh
/// KEBAB_MODELS_LLM_ENDPOINT=http://192.168.0.47:11434 \
/// KEBAB_MODELS_LLM_MODEL=gemma4:e4b \
/// cargo test -p kebab-parse-image --test caption \
///   caption_integration -- --ignored --nocapture
/// ```
#[test]
#[ignore = "hits a real Ollama daemon; opt in via `cargo test -- --ignored`"]
fn caption_integration_real_ollama_describes_image() {
    use kebab_llm_local::OllamaLanguageModel;

    let mut cfg = Config::defaults();
    cfg.ingest.image.caption.enabled = true;
    cfg.ingest.image.caption.max_pixels = 768;
    if let Ok(ep) = std::env::var("KEBAB_MODELS_LLM_ENDPOINT") {
        cfg.models.llm.endpoint = ep;
    } else {
        cfg.models.llm.endpoint = "http://192.168.0.47:11434".to_string();
    }
    if let Ok(m) = std::env::var("KEBAB_MODELS_LLM_MODEL") {
        cfg.models.llm.model = m;
    } else {
        cfg.models.llm.model = "gemma4:e4b".to_string();
    }
    cfg.models.llm.provider = "ollama".to_string();

    let llm = OllamaLanguageModel::new(&cfg).expect("OllamaLanguageModel::new");
    let bytes = red_100x50_png();
    let cap = caption_image(&llm, &bytes, Some(&Lang("en".to_string())), &cfg)
        .expect("real-Ollama caption_image must succeed");
    eprintln!("integration caption: {}", cap.text);
    assert!(!cap.text.is_empty(), "caption must be non-empty");
    assert_eq!(cap.model, "gemma4:e4b");
    assert!(cap.model_version.contains("ollama"));
    assert!(cap.model_version.contains("caption-v1"));
}
