// § Acceptance §9 #3: real Ollama qwen2.5vl:3b 의 alnum accuracy.
// F1 ≥ 0.85, F2 ≥ 0.70. real Ollama 의존 — `#[ignore]` default.
//
// Manual invoke:
// KEBAB_OCR_ENDPOINT=http://192.168.0.47:11434 \
//   cargo test -p kebab-parse-pdf --test ocr_e2e --ignored -j 4

use kebab_core::Lang;
use kebab_parse_image::{OcrEngine, OllamaVisionOcr};
use kebab_parse_pdf::extract_dctdecode_page_image;
use lopdf::Document;

fn run_real_ollama_ocr(pdf: &[u8], page: u32) -> anyhow::Result<String> {
    // v5: shared KEBAB_OCR_* env (manual harness reads it directly).
    let endpoint = std::env::var("KEBAB_OCR_ENDPOINT")
        .unwrap_or_else(|_| "http://localhost:11434".to_string());
    let doc = Document::load_mem(pdf)?;
    let jpeg = extract_dctdecode_page_image(&doc, page)?
        .ok_or_else(|| anyhow::anyhow!("page {page} 의 DCTDecode image XObject 부재"))?;

    let engine = OllamaVisionOcr::from_parts(
        endpoint,
        "qwen2.5vl:3b".to_string(),
        vec!["eng".to_string(), "kor".to_string()],
        2048,
        600,
    )?;

    let result = engine.recognize(&jpeg, Some(&Lang("kor".into())))?;
    Ok(result.joined)
}

fn alnum_accuracy(actual: &str, expected: &str) -> f32 {
    let a: String = actual.chars().filter(|c| c.is_alphanumeric()).collect();
    let e: String = expected.chars().filter(|c| c.is_alphanumeric()).collect();
    if e.is_empty() {
        return 0.0;
    }
    let dist = strsim::levenshtein(&a, &e) as f32;
    ((e.chars().count() as f32 - dist) / e.chars().count() as f32).max(0.0)
}

#[test]
#[ignore = "real Ollama qwen2.5vl:3b dependency"]
fn f1_alnum_accuracy_ge_85() {
    let pdf = include_bytes!("fixtures/scanned_page1.pdf");
    let ocr = run_real_ollama_ocr(pdf, 1).expect("OCR");
    let expected = include_str!("fixtures/scanned_page1_truth.txt");
    let accuracy = alnum_accuracy(&ocr, expected);
    println!("F1 alnum accuracy = {accuracy:.4}");
    assert!(accuracy >= 0.85, "F1 alnum accuracy {accuracy:.4} < 0.85");
}

#[test]
#[ignore = "real Ollama qwen2.5vl:3b dependency"]
fn f2_alnum_accuracy_ge_70() {
    let pdf = include_bytes!("fixtures/scanned_page2.pdf");
    let ocr = run_real_ollama_ocr(pdf, 1).expect("OCR");
    let expected = include_str!("fixtures/scanned_page2_truth.txt");
    let accuracy = alnum_accuracy(&ocr, expected);
    println!("F2 alnum accuracy = {accuracy:.4}");
    assert!(accuracy >= 0.70, "F2 alnum accuracy {accuracy:.4} < 0.70");
}
