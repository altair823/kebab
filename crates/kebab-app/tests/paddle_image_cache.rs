//! PR2 (b1): IMAGE OCR derivation-cache, execution-proven against the REAL
//! bundled `paddle-onnx` engine through the full ingest facade.
//!
//! `ocr_caption_cache.rs` drives the b3 PDF seam with a `MockOcrEngine`; this
//! file closes the loop on b1 — the only path that runs
//! `ingest_one_image_asset`'s `"ocr"` cache wrap with a *real* engine. Paddle's
//! ONNX inference is deterministic (CPU, no sampling), so a re-ingest of the
//! same image bytes + same engine version key is a cache HIT: the
//! `kind='ocr'` derivation-cache row count stays flat and the stored `OcrText`
//! is reconstructed byte-identically.
//!
//! AVX-gated `#[ignore]` (mirrors `search_vector.rs` / `embed_cache_reingest.rs`)
//! because the ONNX runtime uses AVX SIMD kernels. The bundled paddle assets
//! (`crates/kebab-parse-image/assets/paddleocr-onnx/`) ship in-tree — no
//! download — so the only host requirement is AVX.
//!
//! Run with: `cargo test -p kebab-app --test paddle_image_cache -- --ignored`.

mod common;

use common::TestEnv;

/// Panic if the host CPU lacks AVX. Mirrors `tests/search_vector.rs` so a
/// `--ignored` invocation on a non-AVX host fails loudly with a clear message
/// instead of crashing inside an ONNX SIMD kernel.
fn require_avx_or_panic() {
    #[cfg(target_arch = "x86_64")]
    {
        assert!(
            std::is_x86_feature_detected!("avx"),
            "kebab-app paddle image OCR integration test requires AVX-capable \
             hardware; host CPU lacks AVX. Run on an AVX-capable machine."
        );
    }
}

/// Row count of the `"ocr"` derivation-cache namespace in the test KB's SQLite
/// file. b1 is the only producer of these rows in this test (caption stays off,
/// embeddings are `provider="none"`), so the count is attributable to the image
/// OCR cache wrap exclusively.
fn ocr_cache_rows(data_dir: &std::path::Path) -> i64 {
    let db = data_dir.join("kebab.sqlite");
    let conn = rusqlite::Connection::open(db).expect("open kebab.sqlite");
    conn.query_row(
        "SELECT COUNT(*) FROM derivation_cache WHERE kind = 'ocr'",
        [],
        |r| r.get(0),
    )
    .expect("count ocr cache rows")
}

/// `created_at` of the single `kind='ocr'` row. This is the HIT/MISS
/// discriminator the row-count alone cannot provide: the b1 cache key is
/// deterministic, so a MISS re-`put`s the SAME key (`INSERT OR REPLACE`),
/// keeping the row COUNT flat while REFRESHING `created_at` to the second-run
/// timestamp. A genuine HIT never calls `put` (only `touch`, which bumps
/// `last_used_at` and leaves `created_at` frozen), so `created_at` is invariant
/// across re-ingest iff the second run served from cache. See
/// `derivation_cache_put` (INSERT OR REPLACE, both timestamps = now) vs
/// `derivation_cache_touch` (UPDATE last_used_at only).
fn ocr_cache_created_at(data_dir: &std::path::Path) -> String {
    let db = data_dir.join("kebab.sqlite");
    let conn = rusqlite::Connection::open(db).expect("open kebab.sqlite");
    conn.query_row(
        "SELECT created_at FROM derivation_cache WHERE kind = 'ocr'",
        [],
        |r| r.get(0),
    )
    .expect("read ocr cache created_at")
}

/// Remove every entry under the workspace root so the PNG written afterward is
/// the SOLE ingested asset. `TestEnv` copies the fixture markdown tree in;
/// leaving it would let the markdown handler run (it produces no `kind='ocr'`
/// rows, but clearing keeps the asset set unambiguous and the ingest fast).
/// Mirrors `embed_cache_reingest.rs::clear_workspace`.
fn clear_workspace(root: &std::path::Path) {
    for entry in std::fs::read_dir(root).expect("read workspace root") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            std::fs::remove_dir_all(&path).expect("remove workspace subdir");
        } else {
            std::fs::remove_file(&path).expect("remove workspace file");
        }
    }
}

/// Tiny solid-color PNG written into the test workspace. Mirrors
/// `image_pipeline.rs::write_red_png` / the `gen_smoke_png` example. A
/// solid-color image yields EMPTY OCR text — that's fine: `apply_ocr` still
/// sets `block.ocr = Some(OcrText { joined: "", .. })`, so b1 caches it and the
/// re-ingest still HITS. The test proves the cache MECHANISM on the real image
/// path, not OCR quality.
fn write_png(root: &std::path::Path, name: &str) -> std::path::PathBuf {
    use image::{ImageBuffer, Rgb};
    let img: ImageBuffer<Rgb<u8>, _> = ImageBuffer::from_fn(100, 50, |_, _| Rgb([255, 0, 0]));
    let path = root.join(name);
    img.save(&path).expect("write PNG fixture");
    path
}

/// b1 execution proof: ingesting an image then RE-ingesting it (force) is an
/// OCR cache HIT — the `kind='ocr'` row count stays flat on the second run, so
/// the second ingest served `block.ocr` from the cache (`decode_ocr_text`)
/// instead of re-running the paddle engine.
///
/// First run loads the ~18 MB ONNX models, so expect tens of seconds.
#[test]
#[ignore = "requires AVX + bundled paddle-onnx assets"]
fn image_reingest_is_ocr_cache_hit_paddle() {
    require_avx_or_panic();

    let mut env = TestEnv::lexical_only();
    // Enable image OCR with the bundled (no-download) paddle-onnx engine.
    // `image_ocr()` reads `ingest.image.ocr` directly, so these two fields
    // select the paddle arm in `build_image_ocr_engine`. Caption stays off
    // (default) — we're testing the OCR cache exclusively.
    env.config.ingest.image.ocr.enabled = true;
    env.config.ingest.image.ocr.engine = "paddle-onnx".to_string();

    // Make the PNG the SOLE ingested asset.
    clear_workspace(&env.workspace_root);
    write_png(&env.workspace_root, "diagram.png");

    let data_dir = std::path::PathBuf::from(&env.config.storage.data_dir);

    // Run 1 (cold): paddle OCRs the image → at least one `kind='ocr'` row.
    kebab_app::ingest_with_config(
        env.config.clone(),
        env.scope(),
        kebab_app::IngestOpts::default(),
    )
    .expect("cold ingest");
    let ocr_rows_first = ocr_cache_rows(&data_dir);
    assert_eq!(
        ocr_rows_first, 1,
        "cold ingest must populate exactly one ocr cache row (got {ocr_rows_first})"
    );
    // Freeze the cold-run `created_at` — the HIT/MISS discriminator below.
    let created_at_first = ocr_cache_created_at(&data_dir);

    // Run 2 (force re-ingest): same image bytes + same engine version → cache
    // HIT. Two independent signals must both hold for a genuine hit:
    //   1. row COUNT stays flat (no second row appears), AND
    //   2. `created_at` is UNCHANGED — proving the second run served `block.ocr`
    //      from `decode_ocr_text` (touch-only) rather than re-OCRing and
    //      re-`put`ting (which `INSERT OR REPLACE`s a fresh `created_at`).
    // Signal 2 is the load-bearing one: because the cache key is deterministic,
    // a MISS would keep the COUNT flat too, so a row-count assertion alone
    // cannot distinguish HIT from MISS.
    kebab_app::ingest_with_config(
        env.config.clone(),
        env.scope(),
        kebab_app::IngestOpts {
            force_reingest: true,
            ..Default::default()
        },
    )
    .expect("re-ingest");
    let ocr_rows_second = ocr_cache_rows(&data_dir);
    assert_eq!(
        ocr_rows_first, ocr_rows_second,
        "re-ingest must be an OCR cache HIT — no new ocr rows \
         (first={ocr_rows_first}, second={ocr_rows_second})"
    );
    let created_at_second = ocr_cache_created_at(&data_dir);
    assert_eq!(
        created_at_first, created_at_second,
        "re-ingest must be an OCR cache HIT — the ocr row's `created_at` must be \
         frozen (a MISS re-OCRs and `INSERT OR REPLACE`s a fresh `created_at`)"
    );
}
