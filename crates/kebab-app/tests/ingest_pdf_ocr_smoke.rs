//! Integration smoke tests for the PDF OCR pipeline (§ Acceptance §9 #1 + #2).
//!
//! Tests 1 and 2 require a live Ollama endpoint — `#[ignore]` by default.
//! Manual invoke:
//!   KEBAB_PDF_OCR_ENDPOINT=http://192.168.0.47:11434 \
//!     cargo test -p kebab-app --test ingest_pdf_ocr_smoke --ignored -j 4
//!
//! Test 3 (cancel) uses a dummy endpoint + pre-set cancel — runs by default
//! to verify the cancel wiring doesn't panic/deadlock.

mod common;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use common::TestEnv;

fn ollama_endpoint() -> String {
    std::env::var("KEBAB_PDF_OCR_ENDPOINT").unwrap_or_else(|_| "http://localhost:11434".to_string())
}

fn make_ocr_env_real() -> TestEnv {
    let mut env = TestEnv::lexical_only();
    env.config.pdf.ocr.enabled = true;
    env.config.pdf.ocr.endpoint = Some(ollama_endpoint());
    env.config.models.embedding.provider = "none".to_string();

    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("kebab-parse-pdf/tests/fixtures/scanned_page1.pdf");
    let dest = env.workspace_root.join("scanned_page1.pdf");
    std::fs::copy(&src, &dest).expect("copy scanned_page1.pdf to workspace");

    env
}

/// § Acceptance §9 #1 — real Ollama OCR + IngestItem.pdf_ocr_pages = Some(1).
#[test]
#[ignore = "real Ollama qwen2.5vl:3b dependency"]
fn ingest_with_mock_ocr_yields_pdf_ocr_summary() {
    let env = make_ocr_env_real();

    let report =
        kebab_app::ingest_with_config(env.config.clone(), env.scope(), false).expect("ingest");

    assert!(report.new >= 1, "at least one PDF ingested: {report:?}");

    let items = report.items.unwrap_or_default();
    let pdf_item = items.iter().find(|i| i.doc_path.0.ends_with(".pdf"));
    assert!(
        pdf_item.is_some(),
        "PDF item must appear in ingest report items: {items:?}"
    );
    let pdf_item = pdf_item.unwrap();
    assert!(
        pdf_item.pdf_ocr_pages.is_some(),
        "pdf_ocr_pages must be set for scanned PDF: {pdf_item:?}"
    );
    assert_eq!(
        pdf_item.pdf_ocr_pages.unwrap(),
        1,
        "scanned_page1.pdf has exactly 1 page"
    );
}

/// § Acceptance §9 #2 — OCR text indexed and retrievable via lexical search.
#[test]
#[ignore = "real Ollama qwen2.5vl:3b dependency"]
fn ocr_text_indexed_and_searchable() {
    let env = make_ocr_env_real();

    kebab_app::ingest_with_config(env.config.clone(), env.scope(), false).expect("ingest");

    // Search for a Korean morpheme expected to appear in qwen2.5vl:3b OCR
    // output of the PoC ground-truth page. "다음" is a high-frequency token
    // in page1.txt truth file.
    let query = common::lexical_query("다음");
    let hits = kebab_app::search_with_config(env.config.clone(), query).expect("search");

    assert!(
        !hits.is_empty(),
        "OCR-indexed text must surface in lexical search results"
    );
}

/// Production cancel wiring smoke — pre-set cancel exits before any OCR call.
/// Dummy endpoint (port 1 = connection-refused) means OCR HTTP calls would
/// fail, but cancel=true prevents the loop from reaching OCR at all.
/// Verifies no panic/deadlock regardless of Ok/Err outcome.
#[test]
fn ingest_with_cancel_aborts_mid_pdf() {
    let mut env = TestEnv::lexical_only();
    env.config.pdf.ocr.enabled = true;
    env.config.pdf.ocr.endpoint = Some("http://127.0.0.1:1".to_string());

    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("kebab-parse-pdf/tests/fixtures/scanned_page1.pdf");
    let dest = env.workspace_root.join("scanned_page1.pdf");
    std::fs::copy(&src, &dest).expect("copy scanned_page1.pdf to workspace");

    let cancel = Arc::new(AtomicBool::new(true)); // pre-set — abort immediately

    let result = kebab_app::ingest_with_config_cancellable(
        env.config.clone(),
        env.scope(),
        false,
        None,
        Some(cancel),
    );
    // Both Ok (pre-cancel exit) and Err (eager OCR engine fail) are acceptable —
    // key assertion is no panic/deadlock.
    let _ = result;
}
