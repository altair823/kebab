//! Integration tests for [`FastembedEmbedder`] that load the real ONNX
//! model.
//!
//! ## Why every test in this file is `#[ignore]`
//!
//! The first call to `FastembedEmbedder::new` downloads ~470 MB of
//! weights from Hugging Face into `data_dir/models/fastembed/`. Doing
//! that on every `cargo test` invocation is wasteful, so the bare
//! invocation skips this file entirely.
//!
//! Run the full suite with:
//! ```text
//! cargo test -p kb-embed-local -- --ignored
//! ```
//!
//! All tests share a `OnceLock<FastembedEmbedder>` so the model loads
//! exactly once per process invocation (ONNX runtime first-load latency
//! is 1-2 s on M-series Macs per design risks list).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::OnceLock;
use std::time::Instant;

use kebab_embed::{Embedder, EmbeddingInput, EmbeddingKind};
use kebab_embed_local::FastembedEmbedder;

/// Build a `Config` whose `data_dir` lives in a per-process temp dir so
/// the test never writes into the developer's real `~/.local/share/kb`.
/// Returns the `Config` and the `TempDir` guard (caller keeps the guard
/// alive for the test duration).
fn test_config() -> (kebab_config::Config, tempfile::TempDir) {
    let tmp = tempfile::tempdir().expect("create tempdir");
    let mut cfg = kebab_config::Config::defaults();
    cfg.storage.data_dir = tmp.path().to_string_lossy().into_owned();
    // model_dir keeps its default `{data_dir}/models` template; the
    // adapter resolves it itself.
    (cfg, tmp)
}

/// Single shared embedder for the `--ignored` lane. Held behind a
/// `OnceLock` so we pay the ~1-2 s ONNX init + (first run only) the
/// network download just once.
fn shared_embedder() -> &'static FastembedEmbedder {
    static EMBEDDER: OnceLock<FastembedEmbedder> = OnceLock::new();
    EMBEDDER.get_or_init(|| {
        let (cfg, _tmp) = test_config();
        // We deliberately leak `_tmp` here: the OnceLock outlives the
        // function scope so the cache directory must persist for the
        // process. (`tempfile::TempDir`'s `Drop` would erase the cache
        // and wreck subsequent calls.) The OS will reclaim the leaked
        // path when the test process exits.
        let _ = std::mem::ManuallyDrop::new(_tmp);
        FastembedEmbedder::new(&cfg).expect("init FastembedEmbedder")
    })
}

// ─── construction ─────────────────────────────────────────────────────

#[test]
#[ignore = "downloads ~470MB ONNX model on first run; CI-only"]
fn default_config_constructs_with_dims_384() {
    let emb = shared_embedder();
    assert_eq!(emb.dimensions(), 384);
    assert_eq!(emb.model_id().0, "multilingual-e5-small");
    assert_eq!(emb.model_version().0, "v1");
}

#[test]
#[ignore = "downloads ~470MB ONNX model on first run; CI-only"]
fn mismatched_dims_in_config_errors_at_construction() {
    let (mut cfg, _tmp) = test_config();
    cfg.models.embedding.dimensions = 512; // model is 384
    // `FastembedEmbedder` deliberately does not implement `Debug`
    // (its inner ONNX session has no useful debug shape), so we
    // can't use `expect_err`; match the Result manually.
    let err = match FastembedEmbedder::new(&cfg) {
        Ok(_) => panic!("dim mismatch must error"),
        Err(e) => e,
    };
    let msg = format!("{err}");
    assert!(msg.contains("dimension mismatch"), "msg={msg}");
    assert!(msg.contains("384"), "msg={msg}");
    assert!(msg.contains("512"), "msg={msg}");
}

// ─── e5 prefix differentiation ────────────────────────────────────────

#[test]
#[ignore = "loads ONNX model; CI-only"]
fn document_and_query_yield_different_vectors() {
    let emb = shared_embedder();
    let text = "The quick brown fox jumps over the lazy dog.";
    let out = emb
        .embed(&[
            EmbeddingInput {
                text,
                kind: EmbeddingKind::Document,
            },
            EmbeddingInput {
                text,
                kind: EmbeddingKind::Query,
            },
        ])
        .expect("embed two inputs");
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].len(), 384);
    assert_eq!(out[1].len(), 384);

    // Both vectors are L2-normalized → cosine similarity == dot product.
    let cos: f32 = out[0]
        .iter()
        .zip(out[1].iter())
        .map(|(a, b)| a * b)
        .sum();
    // Same text, different prefix → vectors must NOT be identical.
    assert!(
        cos < 0.9999,
        "expected distinct vectors for Document vs Query, got cos={cos}"
    );
}

// ─── L2 normalization ─────────────────────────────────────────────────

#[test]
#[ignore = "loads ONNX model; CI-only"]
fn output_vectors_are_l2_normalized() {
    let emb = shared_embedder();
    let inputs = [
        EmbeddingInput {
            text: "hello world",
            kind: EmbeddingKind::Document,
        },
        EmbeddingInput {
            text: "vector search",
            kind: EmbeddingKind::Document,
        },
        EmbeddingInput {
            text: "embedding model",
            kind: EmbeddingKind::Query,
        },
    ];
    let out = emb.embed(&inputs).expect("embed");
    // Per `kebab_embed::assert_unit_norm` docs: `5e-4` is the safe bound at
    // 384 dims (f32::EPSILON × √384 ≈ 2.3e-6, but ONNX kernels add
    // their own per-component noise; 1e-3 is very generous and matches
    // the spec's `± 1e-3`).
    kebab_embed::assert_unit_norm(&out, 1e-3);
    kebab_embed::assert_vector_shape(&out, 384);
}

// ─── determinism ──────────────────────────────────────────────────────

#[test]
#[ignore = "loads ONNX model; CI-only"]
fn identical_input_yields_identical_output() {
    let emb = shared_embedder();
    let inputs = [
        EmbeddingInput {
            text: "deterministic embedding test",
            kind: EmbeddingKind::Document,
        },
        EmbeddingInput {
            text: "second sentence for variety",
            kind: EmbeddingKind::Document,
        },
    ];
    let a = emb.embed(&inputs).expect("first embed");
    let b = emb.embed(&inputs).expect("second embed");
    assert_eq!(a, b, "two calls with the same inputs must be byte-equal");
}

// ─── performance ──────────────────────────────────────────────────────

#[test]
#[ignore = "performance test; downloads model and runs 64-vec batch"]
fn batch_of_64_short_inputs_under_5s() {
    let emb = shared_embedder();
    // 64 distinct short strings → forces the full default batch_size
    // through one fastembed call.
    let texts: Vec<String> = (0..64)
        .map(|i| format!("perf-test sentence number {i}"))
        .collect();
    let inputs: Vec<EmbeddingInput<'_>> = texts
        .iter()
        .map(|t| EmbeddingInput {
            text: t.as_str(),
            kind: EmbeddingKind::Document,
        })
        .collect();
    let t0 = Instant::now();
    let out = emb.embed(&inputs).expect("embed batch of 64");
    let elapsed = t0.elapsed();
    assert_eq!(out.len(), 64);
    assert!(
        elapsed.as_secs_f32() < 5.0,
        "batch-64 took {elapsed:?}, expected < 5s"
    );
}

// ─── snapshot ─────────────────────────────────────────────────────────

/// Aggregate hash of vectors for the 5 fixture sentences.
///
/// Computed by:
/// 1. embed each sentence as `EmbeddingKind::Document`,
/// 2. round each `f32` component to 4 decimal places (multiply by 1e4,
///    round, store as `i32`),
/// 3. write the rounded i32 components into a `DefaultHasher` in row-
///    major order,
/// 4. read out the `u64` finish value.
///
/// The 4-decimal tolerance is intentional float-tolerance per task spec:
/// exact f32 equality is too strict given ONNX kernel + hardware
/// variation.
///
/// **Pinning workflow** (a snapshot test must FAIL UNTIL PINNED):
/// 1. With `SNAPSHOT_HASH_BASELINE = 0`, run
///    `cargo test -p kb-embed-local -- --ignored snapshot`. The test
///    panics with a message containing the captured hash.
/// 2. Paste the printed hex value into `SNAPSHOT_HASH_BASELINE` below.
/// 3. Re-run the same command — the test now asserts equality and
///    passes, confirming the pin.
///
/// On a genuine model upgrade, reset to `0`, re-pin, and bump
/// `EmbeddingVersion` per design §9 in the same PR.
const SNAPSHOT_HASH_BASELINE: u64 = 0;

#[test]
#[ignore = "loads ONNX model; CI-only"]
fn snapshot_aggregate_hash_is_stable() {
    let emb = shared_embedder();
    let fixture_path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/embed/known-sentences.json");
    let raw = std::fs::read_to_string(&fixture_path).expect("read fixture");
    let json: serde_json::Value = serde_json::from_str(&raw).expect("parse fixture json");
    let sentences: Vec<String> = json["sentences"]
        .as_array()
        .expect("`sentences` array")
        .iter()
        .map(|v| v.as_str().expect("sentence is str").to_string())
        .collect();
    assert_eq!(sentences.len(), 5, "fixture must have exactly 5 sentences");

    let inputs: Vec<EmbeddingInput<'_>> = sentences
        .iter()
        .map(|s| EmbeddingInput {
            text: s.as_str(),
            kind: EmbeddingKind::Document,
        })
        .collect();
    let out = emb.embed(&inputs).expect("embed snapshot fixture");

    // Round every component to 4 decimal places, hash deterministically.
    let mut hasher = DefaultHasher::new();
    for (i, v) in out.iter().enumerate() {
        assert_eq!(v.len(), 384, "row {i} dim mismatch");
        for x in v {
            let rounded: i32 = (*x * 1.0e4).round() as i32;
            rounded.hash(&mut hasher);
        }
    }
    let observed = hasher.finish();
    if SNAPSHOT_HASH_BASELINE == 0 {
        // Unpinned baseline: panic with the captured hash. A snapshot
        // test that silently passes on first run defeats its purpose,
        // so we hard-fail until a maintainer commits the pin. Both
        // hex (paste-friendly) and decimal forms are printed.
        eprintln!(
            "kb-embed-local snapshot baseline (paste into SNAPSHOT_HASH_BASELINE): \
             {observed:#x} ({observed})"
        );
        panic!(
            "snapshot baseline unpinned — paste {observed:#x} into \
             SNAPSHOT_HASH_BASELINE then re-run"
        );
    }
    assert_eq!(
        observed, SNAPSHOT_HASH_BASELINE,
        "snapshot drift: model output for the fixture sentences changed; \
         either fastembed weights changed (bump EmbeddingVersion per §9) \
         or there's an ONNX kernel diff."
    );
}
