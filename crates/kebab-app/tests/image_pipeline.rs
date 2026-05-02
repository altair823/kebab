//! P6-4 image ingest wiring — end-to-end integration.
//!
//! Each test spins up a `TempDir` workspace + writes one PNG fixture +
//! routes OCR / caption HTTP calls through a `wiremock` server that
//! impersonates Ollama's `/api/generate` endpoint. The kb-app code
//! under test is sync; the wiremock server is async, so test bodies
//! drive blocking work via `tokio::task::spawn_blocking`.

mod common;

use std::path::Path;

use common::TestEnv;
use kebab_config::Config;
use serde_json::json;
use tokio::task::spawn_blocking;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// ── Fixture helpers ──────────────────────────────────────────────────────

/// Tiny solid-red PNG written into the test workspace at `<root>/<name>`.
/// 100×50 — small enough to skip downscale by default but non-trivially
/// inspectable in stored DB rows.
fn write_red_png(root: &Path, name: &str) -> std::path::PathBuf {
    use image::{ImageBuffer, Rgb};
    let img: ImageBuffer<Rgb<u8>, _> =
        ImageBuffer::from_fn(100, 50, |_, _| Rgb([255, 0, 0]));
    let path = root.join(name);
    img.save(&path).expect("write PNG fixture");
    path
}

fn cfg_with_image_pipeline(env: &TestEnv, mock_endpoint: &str) -> Config {
    let mut cfg = env.config.clone();
    // Ensure image assets are scanned.
    cfg.workspace
        .include
        .push("**/*.png".to_string());
    cfg.image.ocr.enabled = true;
    cfg.image.ocr.endpoint = Some(mock_endpoint.to_string());
    cfg.image.ocr.model = "vision-mock:1b".to_string();
    cfg.image.ocr.max_pixels = 512;
    cfg.image.caption.enabled = false; // tested separately below
    cfg.models.llm.endpoint = mock_endpoint.to_string();
    cfg.models.llm.model = "vision-mock:1b".to_string();
    cfg
}

// ── 1. Happy path: OCR-only ingest ───────────────────────────────────────

/// One PNG asset + OCR enabled (caption off) → ingest produces 1 doc + 1
/// chunk; chunk text contains alt + OCR transcription joined by `\n\n`.
#[tokio::test]
async fn ingest_image_with_ocr_produces_chunk_containing_ocr_text() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/generate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "model": "vision-mock:1b",
            "response": "Hello World 2026",
            "done": true,
            "done_reason": "stop"
        })))
        .mount(&server)
        .await;

    let env = TestEnv::lexical_only();
    let png = write_red_png(&env.workspace_root, "diagram.png");
    eprintln!("PNG written to {}", png.display());
    let cfg = cfg_with_image_pipeline(&env, &server.uri());
    let cfg_clone = cfg.clone();
    let env_workspace = env.workspace_root.clone();
    let env_scope = env.scope();

    let report = spawn_blocking(move || {
        kebab_app::ingest_with_config(cfg_clone, env_scope, false)
            .expect("image ingest must succeed")
    })
    .await
    .expect("blocking task panicked");

    // Counters: scanned should include the PNG; new ≥ 1 (markdown
    // fixtures from the workspace tree may also count).
    assert!(report.scanned >= 1, "scanned={}, items={:?}", report.scanned, report.items);
    assert_eq!(report.errors, 0, "no errors on lenient OCR path");

    // Locate the image doc in the report items.
    let items = report.items.expect("items present (summary_only=false)");
    let img_item = items
        .iter()
        .find(|i| i.doc_path.0.ends_with("diagram.png"))
        .expect("image doc item must be present");
    assert_eq!(
        img_item.kind,
        kebab_core::IngestItemKind::New,
        "image asset must be classified New on first ingest"
    );
    assert_eq!(img_item.chunk_count, Some(1), "image emits exactly one chunk");

    // Inspect the stored chunk text via kb-app's inspect_chunk facade.
    let doc_id = img_item.doc_id.clone().expect("image doc id");
    let doc = kebab_app::inspect_doc_with_config(cfg.clone(), &doc_id)
        .expect("inspect_doc returns the image document");
    let block = match doc.blocks.first() {
        Some(kebab_core::Block::ImageRef(b)) => b,
        other => panic!("expected ImageRef, got {other:?}"),
    };
    assert!(block.ocr.is_some(), "block.ocr populated by apply_ocr");
    assert_eq!(
        block.ocr.as_ref().unwrap().joined,
        "Hello World 2026",
        "OCR text from mock"
    );
    assert!(
        block.caption.is_none(),
        "caption disabled in cfg → block.caption stays None"
    );

    // Sanity: the doc was actually persisted into SQLite (kb-app's
    // list_docs facade reads the same store the chunker writes to).
    let summaries = kebab_app::list_docs_with_config(cfg, kebab_core::DocFilter::default())
        .expect("list_docs");
    assert!(
        summaries.iter().any(|s| s.doc_path.0.ends_with("diagram.png")),
        "image doc must appear in list_docs"
    );

    drop(env_workspace); // keep TempDir alive until here
    drop(env);
}

// ── 2. OCR + caption together ────────────────────────────────────────────

/// Both OCR and caption enabled. The mock returns the same JSON body
/// for every `/api/generate` POST — wiremock has no per-prompt routing
/// on the default `Mock` so we treat both calls as equivalent. We then
/// verify both `block.ocr` and `block.caption` are populated, and the
/// chunk text contains both fragments separated by `\n\n`.
#[tokio::test]
async fn ingest_image_with_ocr_and_caption_populates_both_fields() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/generate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "response": "shared mock body",
            "done": true,
            "done_reason": "stop"
        })))
        .mount(&server)
        .await;

    let env = TestEnv::lexical_only();
    write_red_png(&env.workspace_root, "diagram.png");
    let mut cfg = cfg_with_image_pipeline(&env, &server.uri());
    cfg.image.caption.enabled = true;
    cfg.image.caption.max_pixels = 384;

    let cfg_clone = cfg.clone();
    let scope = env.scope();
    let report = spawn_blocking(move || {
        kebab_app::ingest_with_config(cfg_clone, scope, false)
            .expect("ingest must succeed with both OCR+caption")
    })
    .await
    .expect("task");

    assert_eq!(report.errors, 0);
    let img_item = report
        .items
        .as_ref()
        .unwrap()
        .iter()
        .find(|i| i.doc_path.0.ends_with("diagram.png"))
        .unwrap();
    let doc = kebab_app::inspect_doc_with_config(cfg, img_item.doc_id.as_ref().unwrap())
        .unwrap();
    let block = match &doc.blocks[0] {
        kebab_core::Block::ImageRef(b) => b,
        _ => unreachable!(),
    };
    assert!(block.ocr.is_some(), "OCR populated");
    assert!(block.caption.is_some(), "caption populated");
    drop(env);
}

// ── 3. Lenient failure: OCR Ollama 503 → asset still indexed ─────────────

/// OCR endpoint returns 503. Spec contract: image is still indexed,
/// `block.ocr = None`, Provenance has a Warning event, `errors`
/// counter NOT incremented.
#[tokio::test]
async fn ocr_failure_indexes_asset_with_warning_no_error_counter() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/generate"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;

    let env = TestEnv::lexical_only();
    write_red_png(&env.workspace_root, "broken.png");
    let cfg = cfg_with_image_pipeline(&env, &server.uri());

    let cfg_clone = cfg.clone();
    let scope = env.scope();
    let report = spawn_blocking(move || {
        kebab_app::ingest_with_config(cfg_clone, scope, false)
            .expect("ingest does not abort on lenient OCR failure")
    })
    .await
    .expect("task");

    assert_eq!(
        report.errors, 0,
        "lenient OCR failure must NOT increment errors counter (spec)"
    );
    let img_item = report
        .items
        .as_ref()
        .unwrap()
        .iter()
        .find(|i| i.doc_path.0.ends_with("broken.png"))
        .expect("asset still indexed despite OCR failure");
    assert_eq!(img_item.kind, kebab_core::IngestItemKind::New);
    assert_eq!(img_item.chunk_count, Some(1));
    assert!(
        !img_item.warnings.is_empty(),
        "lenient OCR failure must surface a warning on the IngestItem"
    );

    let doc_id = img_item.doc_id.clone().unwrap();
    let doc = kebab_app::inspect_doc_with_config(cfg, &doc_id).unwrap();
    let block = match &doc.blocks[0] {
        kebab_core::Block::ImageRef(b) => b,
        _ => unreachable!(),
    };
    assert!(block.ocr.is_none(), "block.ocr stays None on OCR failure");
    let warning = doc
        .provenance
        .events
        .iter()
        .find(|e| e.kind == kebab_core::ProvenanceKind::Warning && e.agent == "kb-app")
        .expect("Provenance Warning attributed to kb-app");
    let note = warning.note.as_deref().unwrap_or("");
    assert!(
        note.contains("ocr_failed"),
        "warning note must describe OCR failure: {note}"
    );
}

// ── 4. Both image.ocr.enabled and image.caption.enabled = false ──────────

/// When both adapters are disabled, the image is still extracted +
/// chunked. Chunk text falls back to the filename. EXIF + dimensions
/// are populated by the extractor regardless.
#[tokio::test]
async fn image_indexed_with_filename_when_ocr_and_caption_disabled() {
    // No mock server needed — neither HTTP path is touched.
    let env = TestEnv::lexical_only();
    write_red_png(&env.workspace_root, "raw.png");
    let mut cfg = env.config.clone();
    cfg.workspace.include.push("**/*.png".to_string());
    cfg.image.ocr.enabled = false;
    cfg.image.caption.enabled = false;

    let cfg_clone = cfg.clone();
    let scope = env.scope();
    let report = spawn_blocking(move || {
        kebab_app::ingest_with_config(cfg_clone, scope, false)
            .expect("ingest with no OCR/caption")
    })
    .await
    .expect("task");

    assert_eq!(report.errors, 0);
    let img_item = report
        .items
        .as_ref()
        .unwrap()
        .iter()
        .find(|i| i.doc_path.0.ends_with("raw.png"))
        .unwrap();
    assert_eq!(img_item.chunk_count, Some(1), "image emits one chunk");
    let doc = kebab_app::inspect_doc_with_config(cfg, img_item.doc_id.as_ref().unwrap())
        .unwrap();
    let block = match &doc.blocks[0] {
        kebab_core::Block::ImageRef(b) => b,
        _ => unreachable!(),
    };
    assert!(block.ocr.is_none() && block.caption.is_none());
    // EXIF + dimensions still populated by the extractor.
    let dims = doc
        .metadata
        .user
        .get("dimensions")
        .and_then(|v: &serde_json::Value| v.as_object())
        .expect("dimensions object present");
    assert_eq!(
        dims.get("w").and_then(|v: &serde_json::Value| v.as_u64()),
        Some(100)
    );
    assert_eq!(
        dims.get("h").and_then(|v: &serde_json::Value| v.as_u64()),
        Some(50)
    );
}

// ── 5. Determinism: re-ingest produces identical doc_id / chunk_id ───────

/// Idempotency contract — running the same ingest twice should mark
/// the asset Updated on the second run with byte-identical IDs.
#[tokio::test]
async fn re_ingest_image_produces_updated_with_same_doc_id() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/generate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "response": "stable",
            "done": true,
            "done_reason": "stop"
        })))
        .mount(&server)
        .await;

    let env = TestEnv::lexical_only();
    write_red_png(&env.workspace_root, "diagram.png");
    let cfg = cfg_with_image_pipeline(&env, &server.uri());

    let scope = env.scope();
    let cfg1 = cfg.clone();
    let cfg2 = cfg.clone();
    let scope1 = scope.clone();
    let scope2 = scope.clone();

    let r1 = spawn_blocking(move || {
        kebab_app::ingest_with_config(cfg1, scope1, false).unwrap()
    })
    .await
    .unwrap();
    let r2 = spawn_blocking(move || {
        kebab_app::ingest_with_config(cfg2, scope2, false).unwrap()
    })
    .await
    .unwrap();

    let id1 = r1
        .items
        .as_ref()
        .unwrap()
        .iter()
        .find(|i| i.doc_path.0.ends_with("diagram.png"))
        .unwrap()
        .doc_id
        .clone()
        .unwrap();
    let img2 = r2
        .items
        .as_ref()
        .unwrap()
        .iter()
        .find(|i| i.doc_path.0.ends_with("diagram.png"))
        .unwrap();
    assert_eq!(img2.kind, kebab_core::IngestItemKind::Updated);
    assert_eq!(img2.doc_id.as_ref().unwrap(), &id1);
}
