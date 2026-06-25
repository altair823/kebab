//! b2: image-caption derivation cache — deterministic, model-free correctness
//! gate. Drives `kebab_app::cache_image_caption` (the b2 seam extracted from
//! `ingest_one_image_asset`) with a counting mock `LanguageModel`, the only
//! publicly mock-injectable caption seam. The mock COUNTS its `generate_stream`
//! calls, so a re-run on the same image bytes + same caption version key proves
//! a cache HIT (LM NOT re-invoked) that reconstructs a byte-identical
//! `ModelCaption`, and a version-key change (different `prompt_template_version`)
//! proves a MISS (§3.6 invalidation safety).
//!
//! This test exercises the REAL cache code path
//! (`derivation_cache_get/put/touch` + `encode/decode_model_caption`) end-to-end
//! against a real `SqliteStore`; the invocation-count assertion is the
//! non-negotiable validity signal. Mirrors `tests/ocr_caption_cache.rs` (b3).

use std::io::Cursor;
use std::path::PathBuf;
use std::sync::Mutex;

use image::{ImageBuffer, Rgb};
use kebab_config::Config;
use kebab_core::{
    AssetStorage, BlockId, Checksum, CommonBlock, FinishReason, GenerateRequest, ImageRefBlock,
    Lang, LanguageModel, ModelRef, ProvenanceEvent, RawAsset, SourceSpan, SourceUri, TokenChunk,
    TokenUsage, WorkspacePath, id_for_asset,
};
use kebab_store_sqlite::SqliteStore;
use time::OffsetDateTime;

// ── Counting mock LM ────────────────────────────────────────────────────────

/// A `LanguageModel` that returns a fixed caption text (streamed as tokens) and
/// counts how many times `generate_stream` is invoked. The cache-hit assertion
/// reads `call_count()`: a HIT must leave it flat across a re-run.
struct CountingCaptionMock {
    caption: String,
    provider: String,
    calls: Mutex<usize>,
}

impl CountingCaptionMock {
    fn new(caption: &str, provider: &str) -> Self {
        Self {
            caption: caption.to_string(),
            provider: provider.to_string(),
            calls: Mutex::new(0),
        }
    }

    fn call_count(&self) -> usize {
        *self.calls.lock().unwrap()
    }
}

impl LanguageModel for CountingCaptionMock {
    fn model_ref(&self) -> ModelRef {
        ModelRef {
            id: "caption-mock:1b".to_string(),
            provider: self.provider.clone(),
            dimensions: None,
        }
    }

    fn context_tokens(&self) -> usize {
        4096
    }

    fn generate_stream(
        &self,
        _req: GenerateRequest,
    ) -> anyhow::Result<Box<dyn Iterator<Item = anyhow::Result<TokenChunk>> + Send>> {
        *self.calls.lock().unwrap() += 1;
        // One token carrying the whole caption, then a clean Done frame — the
        // caption adapter trims and joins tokens into `ModelCaption.text`.
        let chunks: Vec<TokenChunk> = vec![
            TokenChunk::Token(self.caption.clone()),
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

// ── Fixture helpers ─────────────────────────────────────────────────────────

/// 100×50 solid-red PNG, no EXIF (mirrors `kebab-parse-image`'s
/// `red_100x50_png`). Generated in-memory so the test binary stays
/// self-contained.
fn red_100x50_png() -> Vec<u8> {
    let img: ImageBuffer<Rgb<u8>, _> = ImageBuffer::from_fn(100, 50, |_, _| Rgb([255, 0, 0]));
    let mut buf = Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::Png)
        .expect("encoding tiny PNG must not fail");
    buf.into_inner()
}

/// A fresh `ImageRefBlock` with `caption: None` (mirrors the block the
/// image-ingest pipeline hands to `cache_image_caption`).
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
        asset_id: None,
        src: "img/x.png".to_string(),
        alt: "x.png".to_string(),
        ocr: None,
        caption: None,
    }
}

/// A caption-enabled config with a tweakable `prompt_template_version`. The
/// version key folds the prompt-template version, so bumping it MISSES.
fn caption_config(prompt_version: &str) -> Config {
    let mut cfg = Config::defaults();
    cfg.ingest.image.caption.enabled = true;
    cfg.ingest.image.caption.max_pixels = 512;
    cfg.ingest.image.caption.prompt_template_version = prompt_version.to_string();
    cfg
}

/// Minimal `RawAsset` — only consumed by `record_image_analysis_failure` on the
/// caption-error path, which this happy-path test never hits.
fn dummy_asset() -> RawAsset {
    let fake_hash = "0".repeat(64);
    RawAsset {
        asset_id: id_for_asset(&fake_hash),
        source_uri: SourceUri::File(PathBuf::from("img/x.png")),
        workspace_path: WorkspacePath::new("img/x.png".to_string()).unwrap(),
        media_type: kebab_core::MediaType::Image(kebab_core::ImageType::Png),
        byte_len: 0,
        checksum: Checksum(fake_hash.clone()),
        discovered_at: OffsetDateTime::UNIX_EPOCH,
        stored: AssetStorage::Copied {
            path: PathBuf::from("img/x.png"),
        },
    }
}

/// A real `SqliteStore` over a fresh temp dir, migrations applied. Owns the
/// `TempDir` (returned so the caller keeps it alive for the test's lifetime).
fn temp_store() -> (SqliteStore, tempfile::TempDir) {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = Config::defaults().storage;
    storage.data_dir = temp.path().to_string_lossy().into_owned();
    let store = SqliteStore::open(&storage).expect("open SqliteStore");
    store.run_migrations().expect("run_migrations");
    (store, temp)
}

/// Run `cache_image_caption` once against `cfg` + `store`, returning the
/// resulting `block.caption`. Asserts the helper itself succeeds (caption
/// failures are swallowed into warnings, so a `None` caption here would be a
/// silent miss we want surfaced by the caller's assertions).
fn run_once(
    llm: &CountingCaptionMock,
    image_bytes: &[u8],
    cfg: &Config,
    store: &SqliteStore,
) -> Option<kebab_core::ModelCaption> {
    let mut block = empty_image_block();
    let asset = dummy_asset();
    let mut events: Vec<ProvenanceEvent> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut touch_keys: Vec<String> = Vec::new();
    kebab_app::cache_image_caption(
        llm,
        image_bytes,
        &mut block,
        Some(&Lang("und".to_string())),
        cfg,
        store,
        &asset,
        &mut events,
        &mut warnings,
        OffsetDateTime::UNIX_EPOCH,
        &mut touch_keys,
    )
    .expect("cache_image_caption must not error on the happy path");
    assert!(
        warnings.is_empty(),
        "happy-path caption must not record a warning: {warnings:?}"
    );
    block.caption
}

// ── Test ────────────────────────────────────────────────────────────────────

/// Primary deterministic correctness gate for the b2 caption cache: a re-run on
/// the same image bytes + provider + prompt-template version is a cache HIT (LM
/// NOT re-invoked) that reconstructs a byte-identical `ModelCaption`, and a
/// version-key bump MISSES.
#[test]
fn caption_reingest_is_cache_hit_llm_not_reinvoked() {
    let (store, _temp) = temp_store();
    let bytes = red_100x50_png();
    let llm = CountingCaptionMock::new("a red rectangle", "mock");

    let cfg_v1 = caption_config("caption-v1");

    // Run 1 (cold): LM is invoked, caption produced + cached.
    let cap_first = run_once(&llm, &bytes, &cfg_v1, &store);
    assert_eq!(llm.call_count(), 1, "cold run must invoke the LM exactly once");
    let cap_first = cap_first.expect("cold run produces a caption");
    assert_eq!(cap_first.text, "a red rectangle", "cold run captions via the LM");

    // Run 2 (warm, same bytes + same provider + same prompt version): cache HIT,
    // LM NOT re-invoked.
    let cap_second = run_once(&llm, &bytes, &cfg_v1, &store);
    assert_eq!(
        llm.call_count(),
        1,
        "re-run must be a cache HIT — generate_stream must NOT be called again"
    );
    let cap_second = cap_second.expect("warm run reconstructs the cached caption");
    assert_eq!(
        cap_first, cap_second,
        "cached ModelCaption must be byte-identical (text + model + model_version)"
    );

    // Version bump → MISS (LM re-invoked). Proves §3.6 invalidation safety.
    let cfg_v2 = caption_config("caption-v2");
    let cap_third = run_once(&llm, &bytes, &cfg_v2, &store);
    assert_eq!(
        llm.call_count(),
        2,
        "a prompt_template_version change must MISS and re-invoke the LM"
    );
    let cap_third = cap_third.expect("version-bump miss re-captions");
    assert_eq!(
        cap_third.text, "a red rectangle",
        "miss re-runs the same mock → same caption text"
    );
}
