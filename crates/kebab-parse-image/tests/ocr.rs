//! Integration tests for the OCR adapter (P6-2).
//!
//! Pattern mirrors `kebab-llm-local/tests/streaming.rs` — `wiremock` is
//! async, so test fns are `#[tokio::test]` and the sync adapter is
//! invoked from `spawn_blocking`.

mod common;

use kebab_config::Config;
use kebab_core::{
    AssetId, BlockId, CommonBlock, ImageRefBlock, Lang, ProvenanceEvent, ProvenanceKind, SourceSpan,
};
use kebab_parse_image::{OcrEngine, OllamaVisionOcr, apply_ocr};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::common::red_100x50_png;

fn cfg_for_endpoint(endpoint: &str) -> Config {
    let mut cfg = Config::defaults();
    cfg.image.ocr.endpoint = Some(endpoint.to_string());
    cfg.image.ocr.model = "gemma4:e4b".to_string();
    cfg.image.ocr.languages = vec!["eng".to_string(), "kor".to_string()];
    cfg.image.ocr.max_pixels = 1024;
    cfg
}

fn run_recognize(
    cfg: Config,
    bytes: Vec<u8>,
    lang_hint: Option<Lang>,
) -> anyhow::Result<kebab_core::OcrText> {
    let engine = OllamaVisionOcr::new(&cfg)?;
    engine.recognize(&bytes, lang_hint.as_ref())
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

// ── Happy path ────────────────────────────────────────────────────────────

#[tokio::test]
async fn ocr_recognize_decodes_response_into_ocr_text() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/generate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "model": "gemma4:e4b",
            "response": "Hello World 2026",
            "done": true,
            "done_reason": "stop"
        })))
        .mount(&server)
        .await;

    let bytes = red_100x50_png();
    let cfg = cfg_for_endpoint(&server.uri());
    let text = tokio::task::spawn_blocking(move || run_recognize(cfg, bytes, None))
        .await
        .expect("blocking task panicked")
        .expect("recognize must succeed");

    assert_eq!(text.joined, "Hello World 2026");
    assert_eq!(text.engine, "ollama-vision");
    assert!(text.engine_version.starts_with("ollama/gemma4:e4b"));
    assert_eq!(
        text.regions.len(),
        1,
        "non-empty joined → exactly one region"
    );
    assert_eq!(text.regions[0].text, "Hello World 2026");
    assert!((text.regions[0].confidence - 1.0).abs() < 1e-6);
    // Region bbox covers prepared image dimensions (100×50 < max_pixels
    // 1024 so no downscale, dims preserved).
    assert_eq!(text.regions[0].bbox, (0, 0, 100, 50));
}

// ── Empty response ────────────────────────────────────────────────────────

#[tokio::test]
async fn ocr_recognize_empty_response_yields_empty_regions() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/generate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "response": "",
            "done": true
        })))
        .mount(&server)
        .await;

    let bytes = red_100x50_png();
    let cfg = cfg_for_endpoint(&server.uri());
    let text = tokio::task::spawn_blocking(move || run_recognize(cfg, bytes, None))
        .await
        .expect("blocking task panicked")
        .expect("recognize on empty response must succeed");

    assert_eq!(text.joined, "");
    assert!(text.regions.is_empty(), "empty joined → no regions");
    assert_eq!(text.engine, "ollama-vision");
}

// ── Server error mapping ──────────────────────────────────────────────────

#[tokio::test]
async fn ocr_recognize_500_response_returns_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/generate"))
        .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
        .mount(&server)
        .await;

    let bytes = red_100x50_png();
    let cfg = cfg_for_endpoint(&server.uri());
    let r = tokio::task::spawn_blocking(move || run_recognize(cfg, bytes, None))
        .await
        .expect("blocking task panicked");
    assert!(r.is_err(), "5xx must surface as Err");
    let msg = format!("{:#}", r.unwrap_err());
    assert!(
        msg.contains("500") && msg.contains("boom"),
        "error must include status + body: {msg}"
    );
}

// ── error envelope on 200 stream ─────────────────────────────────────────

#[tokio::test]
async fn ocr_recognize_error_envelope_on_200_returns_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/generate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "error": "model 'gemma4:e4b' not found"
        })))
        .mount(&server)
        .await;

    let bytes = red_100x50_png();
    let cfg = cfg_for_endpoint(&server.uri());
    let r = tokio::task::spawn_blocking(move || run_recognize(cfg, bytes, None))
        .await
        .expect("blocking task panicked");
    assert!(r.is_err(), "server error envelope must surface");
    let msg = format!("{:#}", r.unwrap_err());
    assert!(
        msg.contains("not found"),
        "error must include server message: {msg}"
    );
}

// ── apply_ocr mutates block + appends provenance ─────────────────────────

#[tokio::test]
async fn apply_ocr_sets_block_ocr_and_appends_provenance() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/generate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "response": "안녕 2026",
            "done": true
        })))
        .mount(&server)
        .await;

    let bytes = red_100x50_png();
    let cfg = cfg_for_endpoint(&server.uri());

    let (block, events) = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        let engine = OllamaVisionOcr::new(&cfg)?;
        let mut block = empty_image_block();
        let mut events: Vec<ProvenanceEvent> = Vec::new();
        apply_ocr(
            &engine,
            &bytes,
            &mut block,
            Some(&Lang("ko".to_string())),
            &mut events,
        )?;
        Ok((block, events))
    })
    .await
    .expect("blocking task panicked")
    .expect("apply_ocr must succeed");

    let ocr = block.ocr.as_ref().expect("ocr Some after apply_ocr");
    assert_eq!(ocr.joined, "안녕 2026");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, ProvenanceKind::OcrApplied);
    assert_eq!(events[0].agent, "kb-parse-image");
    let note = events[0].note.as_deref().unwrap_or("");
    assert!(
        note.contains("engine=ollama-vision") && note.contains("regions=1"),
        "provenance note must describe engine + region count: {note}"
    );
}

// ── apply_ocr error leaves block untouched ───────────────────────────────

#[tokio::test]
async fn apply_ocr_error_leaves_block_untouched() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/generate"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;

    let bytes = red_100x50_png();
    let cfg = cfg_for_endpoint(&server.uri());

    let (block, events, err) = tokio::task::spawn_blocking(move || {
        let engine = OllamaVisionOcr::new(&cfg).expect("engine");
        let mut block = empty_image_block();
        let mut events: Vec<ProvenanceEvent> = Vec::new();
        let res = apply_ocr(&engine, &bytes, &mut block, None, &mut events);
        (block, events, res.err())
    })
    .await
    .expect("blocking task panicked");

    assert!(err.is_some(), "503 must propagate as Err");
    assert!(
        block.ocr.is_none(),
        "block.ocr stays None when apply_ocr fails — partial state must not leak"
    );
    assert!(
        events.is_empty(),
        "no Provenance event when OCR fails — kb-normalize would otherwise lie about success"
    );
}

// ── Downscale: large input shrinks before sending ─────────────────────────

#[tokio::test]
async fn ocr_downscales_large_image_before_sending() {
    use std::sync::{Arc, Mutex};

    // Capture the request body so we can pull out the base64 image and
    // measure its dimensions.
    let captured: Arc<Mutex<Option<Vec<u8>>>> = Arc::new(Mutex::new(None));

    let server = MockServer::start().await;
    let cap = captured.clone();
    Mock::given(method("POST"))
        .and(path("/api/generate"))
        .respond_with(move |req: &wiremock::Request| {
            let body = req.body.clone();
            *cap.lock().unwrap() = Some(body);
            ResponseTemplate::new(200).set_body_json(json!({
                "response": "ok",
                "done": true
            }))
        })
        .mount(&server)
        .await;

    // 4000×3000 PNG (long edge 4000) — well above the cfg max 1024.
    let big = common::large_blue_4000x3000_png();
    let cfg = cfg_for_endpoint(&server.uri());
    let _ = tokio::task::spawn_blocking({
        let cfg = cfg.clone();
        move || run_recognize(cfg, big, None)
    })
    .await
    .expect("blocking task panicked")
    .expect("recognize succeeds");

    // Pull the request body, parse JSON, base64-decode the image, and
    // verify the long edge is at most max_pixels (1024).
    let raw = captured.lock().unwrap().clone().expect("request captured");
    let value: serde_json::Value = serde_json::from_slice(&raw).expect("request body is JSON");
    let imgs = value
        .get("images")
        .and_then(|v| v.as_array())
        .expect("images field present");
    assert_eq!(imgs.len(), 1, "exactly one image sent");
    let b64 = imgs[0].as_str().expect("image is base64 string");
    use base64::Engine as _;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .expect("base64 decodes");
    let reader = image::ImageReader::new(std::io::Cursor::new(decoded))
        .with_guessed_format()
        .expect("guess format");
    let (w, h) = reader.into_dimensions().expect("dims");
    let long = w.max(h);
    assert!(
        long <= 1024,
        "long edge after downscale must be <= max_pixels (got {long})"
    );
    // Aspect ratio preserved within rounding.
    let ratio_in = 4000.0 / 3000.0;
    let ratio_out = w as f32 / h as f32;
    assert!(
        (ratio_in - ratio_out).abs() < 0.02,
        "aspect ratio drift: in={ratio_in} out={ratio_out}"
    );
}

// ── from_parts construction ──────────────────────────────────────────────

#[test]
fn from_parts_clamps_max_pixels_into_legal_range() {
    // Below MIN_LONG_EDGE — bumped up to the floor.
    let too_small = OllamaVisionOcr::from_parts("http://x", "m", vec![], 10, 300).unwrap();
    assert_eq!(
        too_small.max_pixels(),
        256,
        "max_pixels must be raised to MIN_LONG_EDGE"
    );

    // Above MAX_LONG_EDGE — capped at the ceiling.
    let too_big = OllamaVisionOcr::from_parts("http://x", "m", vec![], 99_999, 300).unwrap();
    assert_eq!(
        too_big.max_pixels(),
        4096,
        "max_pixels must be capped at MAX_LONG_EDGE"
    );

    // Inside the legal range — pass through untouched.
    let in_range = OllamaVisionOcr::from_parts("http://x", "m", vec![], 1024, 300).unwrap();
    assert_eq!(in_range.max_pixels(), 1024);
}

// ── Integration test against real Ollama (opt-in) ────────────────────────

/// End-to-end OCR against the workspace's real Ollama daemon. Skipped
/// by default via `#[ignore]` (matching the `kebab-llm-local`
/// convention); a developer who explicitly opts in via `--ignored` is
/// signalling they want the network call. Endpoint / model can still
/// be overridden via env to point at a non-default Ollama host.
///
/// Run with:
///
/// ```sh
/// KEBAB_IMAGE_OCR_ENDPOINT=http://192.168.0.47:11434 \
/// cargo test -p kebab-parse-image --test ocr ocr_integration -- --ignored
/// ```
#[tokio::test]
#[ignore = "hits a real Ollama daemon; opt in via `cargo test -- --ignored`"]
async fn ocr_integration_real_ollama_transcribes_text() {
    let endpoint = std::env::var("KEBAB_IMAGE_OCR_ENDPOINT")
        .unwrap_or_else(|_| "http://192.168.0.47:11434".to_string());
    let model = std::env::var("KEBAB_IMAGE_OCR_MODEL").unwrap_or_else(|_| "gemma4:e4b".to_string());

    // Generate a fixture with known text. If the DejaVu font is
    // missing from this dev box, skip rather than crash.
    let bytes = match common::hello_world_png() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("skipping ocr_integration: {e:#}");
            return;
        }
    };
    let cfg = {
        let mut c = Config::defaults();
        c.image.ocr.endpoint = Some(endpoint);
        c.image.ocr.model = model;
        c.image.ocr.max_pixels = 1024;
        c
    };
    let text = tokio::task::spawn_blocking(move || run_recognize(cfg, bytes, None))
        .await
        .expect("blocking task panicked")
        .expect("real Ollama OCR must succeed");
    eprintln!("integration OCR result: {:?}", text.joined);
    let normalized = text.joined.to_lowercase().replace(',', "").replace('.', "");
    assert!(
        normalized.contains("hello") && normalized.contains("world"),
        "integration OCR did not capture expected text: {:?}",
        text.joined
    );
}
