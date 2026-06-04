//! v0.26.2: ingest-config invalidation — changing a setting that affects
//! ingest output auto-re-indexes the affected assets on the next ingest
//! (no `--force-reingest`), while changing an unrelated setting does not.
//!
//! These end-to-end tests exercise the model-free signals (chunking +
//! `[ingest.code]` options vs `search` settings). The exhaustive per-setting
//! mapping (image OCR / caption, pdf.ocr, code options, search/rag/ui
//! invariance) is unit-tested in
//! `kebab-app/src/lib.rs::ingest_config_signature_tests` — those toggles
//! (OCR/caption) require a live vision endpoint to ingest, so the wiring is
//! verified here via the signature-driven chunking path that shares the same
//! `effective_parser_version` plumbing.

mod common;

use common::TestEnv;

use kebab_app::{IngestOpts, ingest_with_config, ingest_with_config_opts};
use kebab_core::IngestItemKind;

/// Seed a workspace with a markdown + a rust file so both the markdown and
/// the code ingest paths are exercised. Returns the first-ingest report.
fn seed_and_first_ingest(env: &TestEnv) -> kebab_core::IngestReport {
    std::fs::write(
        env.workspace_root.join("demo.rs"),
        "/// adds two integers\npub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n",
    )
    .unwrap();
    let first = ingest_with_config(env.config.clone(), env.scope(), false).expect("first ingest");
    assert_eq!(first.errors, 0, "first ingest must not error: {first:?}");
    assert!(first.new >= 1, "first ingest creates docs: {first:?}");
    assert_eq!(first.unchanged, 0, "first ingest has no unchanged: {first:?}");
    first
}

fn reingest(env: &TestEnv) -> kebab_core::IngestReport {
    ingest_with_config_opts(env.config.clone(), env.scope(), false, IngestOpts::default())
        .expect("re-ingest")
}

/// Re-running with the identical config skips every asset (no spurious
/// re-index). Regression guard for over-invalidation.
#[test]
fn identical_config_skips_all_assets() {
    let env = TestEnv::lexical_only();
    let first = seed_and_first_ingest(&env);
    let scanned = first.scanned;

    let second = reingest(&env);
    assert_eq!(second.scanned, scanned);
    assert_eq!(second.new, 0, "no new docs: {second:?}");
    assert_eq!(second.updated, 0, "nothing re-indexed: {second:?}");
    assert_eq!(second.unchanged, scanned, "every doc Unchanged: {second:?}");
    assert_eq!(second.errors, 0);
}

/// Changing a common chunking parameter re-indexes EVERY media type
/// (markdown + code here) without `--force-reingest`.
#[test]
fn chunking_change_reindexes_all_types() {
    let mut env = TestEnv::lexical_only();
    let first = seed_and_first_ingest(&env);
    let scanned = first.scanned;

    // Bump target_tokens — folds into every type's signature.
    env.config.ingest.chunking.target_tokens += 100;

    let second = reingest(&env);
    assert_eq!(second.scanned, scanned);
    assert_eq!(second.new, 0, "no new docs: {second:?}");
    assert_eq!(
        second.unchanged, 0,
        "chunking change must re-index all: {second:?}"
    );
    assert_eq!(
        second.updated, scanned,
        "every doc re-indexed as Updated: {second:?}"
    );
    assert_eq!(second.errors, 0);
}

/// Changing an `[ingest.code]` option re-indexes only the code asset; the
/// markdown assets stay Unchanged.
#[test]
fn code_option_change_reindexes_code_only() {
    let mut env = TestEnv::lexical_only();
    let first = seed_and_first_ingest(&env);
    let scanned = first.scanned;

    // Raise max_file_lines (keeps the tiny demo.rs in-scope; only the code
    // signature changes).
    env.config.ingest.code.max_file_lines += 1000;

    let second = reingest(&env);
    assert_eq!(second.scanned, scanned);
    assert_eq!(second.new, 0, "no new docs: {second:?}");
    assert_eq!(second.errors, 0);
    assert_eq!(
        second.updated, 1,
        "exactly the code asset re-indexed: {second:?}"
    );
    assert_eq!(
        second.unchanged,
        scanned - 1,
        "all markdown assets stay Unchanged: {second:?}"
    );

    let items = second.items.as_ref().expect("items present");
    let code = items
        .iter()
        .find(|i| i.doc_path.0.ends_with("demo.rs"))
        .expect("demo.rs item");
    assert_eq!(
        code.kind,
        IngestItemKind::Updated,
        "demo.rs must be re-indexed: {code:?}"
    );
    for i in items.iter().filter(|i| i.doc_path.0.ends_with(".md")) {
        assert_eq!(
            i.kind,
            IngestItemKind::Unchanged,
            "markdown must be Unchanged: {i:?}"
        );
    }
}

/// Regression guard: changing a non-ingest setting (`search.default_k`) does
/// NOT re-index anything.
#[test]
fn search_setting_change_reindexes_nothing() {
    let mut env = TestEnv::lexical_only();
    let first = seed_and_first_ingest(&env);
    let scanned = first.scanned;

    env.config.search.default_k += 5;
    env.config.search.snippet_chars += 50;
    env.config.rag.score_gate = 0.5;

    let second = reingest(&env);
    assert_eq!(second.scanned, scanned);
    assert_eq!(
        second.unchanged, scanned,
        "search/rag changes must not re-index: {second:?}"
    );
    assert_eq!(second.updated, 0, "nothing re-indexed: {second:?}");
    assert_eq!(second.new, 0);
    assert_eq!(second.errors, 0);
}

/// v3 불변식 #1: `ingest_config_signature` 출력 문자열은 값 기반이라 struct
/// 경로 재편(미디어 ingest 통합) 후에도 v2 와 **바이트 동일**해야 한다. 깨지면
/// 업그레이드 시 전체 재색인 발생. paddle-onnx image 분기 형식 골든.
#[test]
fn ingest_signature_image_paddle_byte_stable() {
    let mut cfg = kebab_config::Config::defaults();
    cfg.ingest.image.ocr.enabled = true;
    cfg.ingest.image.ocr.engine = "paddle-onnx".into();
    let sig = kebab_app::test_ingest_config_signature(
        &cfg,
        &kebab_core::MediaType::Image(kebab_core::ImageType::Png),
    );
    // 골든: chunk:... |ocr:1:paddle-onnx:<engine_version> |cap:0
    assert!(
        sig.starts_with("chunk:500:80:true:md-heading-v1"),
        "chunk prefix drift: {sig}"
    );
    assert!(sig.contains("|ocr:1:paddle-onnx:"), "ocr token drift: {sig}");
    assert!(sig.ends_with("|cap:0"), "cap token drift: {sig}");
}
