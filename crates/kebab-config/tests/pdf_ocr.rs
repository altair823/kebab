// crates/kebab-config/tests/pdf_ocr.rs
//
// Integration tests for [pdf.ocr] config section (v0.20.0 sub-item 1).

use kebab_config::{Config, PdfCfg};
use std::collections::HashMap;

// Test 1: toml roundtrip — spec §4.5 line 1034-1047 example block.
// Config requires many required fields; test the [pdf] section via PdfCfg wrapper.
#[derive(serde::Deserialize)]
struct PdfWrapper {
    pdf: PdfCfg,
}

#[test]
fn pdf_ocr_toml_roundtrip() {
    let toml = r#"
[pdf.ocr]
enabled = true
always_on = false
engine = "ollama-vision"
model = "qwen2.5vl:7b"
endpoint = "http://192.168.0.47:11434"
languages = ["eng", "kor"]
max_pixels = 3072
request_timeout_secs = 900
valid_ratio_threshold = 0.6
min_char_count = 30
lang_hint = "kor"
"#;
    let w: PdfWrapper = toml::from_str(toml).expect("parse toml");
    let ocr = &w.pdf.ocr;
    assert!(ocr.enabled);
    assert!(!ocr.always_on);
    assert_eq!(ocr.engine, "ollama-vision");
    assert_eq!(ocr.model, "qwen2.5vl:7b");
    assert_eq!(ocr.endpoint.as_deref(), Some("http://192.168.0.47:11434"));
    assert_eq!(ocr.languages, vec!["eng".to_string(), "kor".to_string()]);
    assert_eq!(ocr.max_pixels, 3072);
    assert_eq!(ocr.request_timeout_secs, 900);
    assert!((ocr.valid_ratio_threshold - 0.6).abs() < 1e-6);
    assert_eq!(ocr.min_char_count, 30);
    assert_eq!(ocr.lang_hint.as_deref(), Some("kor"));
}

// Test 2: defaults — opt-in, qwen2.5vl:3b model, 0.5 threshold, 20 min_char.
#[test]
fn pdf_ocr_defaults_off_with_qwen_3b() {
    let cfg = Config::defaults();
    assert!(!cfg.ingest.pdf.ocr.enabled);
    assert!(!cfg.ingest.pdf.ocr.always_on);
    assert_eq!(cfg.ingest.pdf.ocr.engine, "ollama-vision");
    assert_eq!(cfg.ingest.pdf.ocr.model, "qwen2.5vl:3b");
    assert!(cfg.ingest.pdf.ocr.endpoint.is_none());
    assert_eq!(
        cfg.ingest.pdf.ocr.languages,
        vec!["eng".to_string(), "kor".to_string()]
    );
    assert_eq!(cfg.ingest.pdf.ocr.max_pixels, 2048);
    assert_eq!(cfg.ingest.pdf.ocr.request_timeout_secs, 180); // Bug #11: 600 → 60 → 180 (HOTFIXES 2026-05-28)
    assert!((cfg.ingest.pdf.ocr.valid_ratio_threshold - 0.5).abs() < 1e-6);
    assert_eq!(cfg.ingest.pdf.ocr.min_char_count, 20);
    assert_eq!(cfg.ingest.pdf.ocr.lang_hint.as_deref(), Some("kor"));
}

// Test 3: env var override — pdf-only keys + shared engine knob.
// v5: `model` moved to the shared `KEBAB_OCR_MODEL` (sets both mediums);
// `enabled`/`always_on`/`valid_ratio_threshold` stay pdf-specific.
#[test]
fn pdf_ocr_env_overrides() {
    let mut env: HashMap<String, String> = HashMap::new();
    env.insert("KEBAB_PDF_OCR_ENABLED".to_string(), "true".to_string());
    env.insert("KEBAB_OCR_MODEL".to_string(), "qwen2.5vl:7b".to_string());
    env.insert("KEBAB_PDF_OCR_ALWAYS_ON".to_string(), "true".to_string());
    env.insert(
        "KEBAB_PDF_OCR_VALID_RATIO_THRESHOLD".to_string(),
        "0.75".to_string(),
    );

    let cfg = Config::defaults().apply_env(&env);

    assert!(cfg.ingest.pdf.ocr.enabled);
    assert_eq!(cfg.ingest.pdf.ocr.model, "qwen2.5vl:7b");
    assert!(cfg.ingest.pdf.ocr.always_on);
    assert!((cfg.ingest.pdf.ocr.valid_ratio_threshold - 0.75).abs() < 1e-6);

    // 다른 env var 가 default 보존
    assert_eq!(cfg.ingest.pdf.ocr.engine, "ollama-vision");
    assert_eq!(cfg.ingest.pdf.ocr.min_char_count, 20);
}
