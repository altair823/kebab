//! T11 e2e accuracy gate for the paddle-onnx OCR engine.
//!
//! Runs the full `OnnxPaddleOcr` pipeline (det → rectify → rec → CTC) over the
//! synthetic OCR benchmark fixtures and asserts the mean character error rate
//! (CER) over the clean text set is `<= 0.05`, matching the spec gate.
//!
//! Model assets come from `KEBAB_TEST_OCR_MODEL_DIR` (default: the crate's
//! bundled `assets/paddleocr-onnx/`). Fixtures come from
//! `KEBAB_TEST_OCR_FIXTURE_DIR` (default: the dogfood corpus). If either is
//! absent the test skips with a warning rather than failing — CI without the
//! large models / fixtures stays green (plan T0/M4).

use std::collections::HashMap;
use std::path::PathBuf;

use kebab_parse_image::{ModelPaths, OcrEngine, OnnxPaddleOcr};

/// Collapse all whitespace runs to a single space + trim — matches the Python
/// `score_lib.norm` so the Rust gate and the bench harness agree.
fn norm(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Character error rate = Levenshtein(gt, pred) / len(gt), both normalized.
fn cer(gt: &str, pred: &str) -> f64 {
    let g: Vec<char> = norm(gt).chars().collect();
    let p: Vec<char> = norm(pred).chars().collect();
    if g.is_empty() {
        return if p.is_empty() { 0.0 } else { 1.0 };
    }
    let (m, n) = (g.len(), p.len());
    let mut prev: Vec<usize> = (0..=n).collect();
    for i in 1..=m {
        let mut cur = vec![i; n + 1];
        for j in 1..=n {
            let cost = if g[i - 1] == p[j - 1] { 0 } else { 1 };
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        prev = cur;
    }
    prev[n] as f64 / m as f64
}

fn fixture_dir() -> PathBuf {
    std::env::var("KEBAB_TEST_OCR_FIXTURE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from("/build/dogfood/corpus/images/synthetic-ocr-bench")
        })
}

/// T10: undecodable image bytes must surface as an error (the kebab-app caller
/// then skips the asset + records provenance), not panic or return garbage.
#[test]
fn paddle_onnx_decode_failure_is_error() {
    let paths = ModelPaths::from_default_dir();
    if !paths.det.exists() || !paths.rec.exists() || !paths.dict.exists() {
        eprintln!("SKIP paddle_onnx_decode_failure_is_error: model assets not found");
        return;
    }
    let engine = OnnxPaddleOcr::from_paths(&paths, 0.3, 1.5, 1000, 1600).unwrap();
    let err = engine
        .recognize(b"not a real image", None)
        .expect_err("garbage bytes must fail to decode");
    let msg = format!("{err:#}");
    assert!(msg.contains("decoding image"), "unexpected error: {msg}");
}

#[test]
fn paddle_onnx_cer_gate() {
    let paths = ModelPaths::from_default_dir();
    if !paths.det.exists() || !paths.rec.exists() || !paths.dict.exists() {
        eprintln!(
            "SKIP paddle_onnx_cer_gate: model assets not found (det={}). \
             Set KEBAB_TEST_OCR_MODEL_DIR or place assets/paddleocr-onnx/.",
            paths.det.display()
        );
        return;
    }
    let fdir = fixture_dir();
    let gt_path = fdir.join("gt.json");
    if !gt_path.exists() {
        eprintln!(
            "SKIP paddle_onnx_cer_gate: fixtures not found at {}",
            fdir.display()
        );
        return;
    }

    let gt: HashMap<String, String> =
        serde_json::from_str(&std::fs::read_to_string(&gt_path).unwrap()).unwrap();

    let engine = OnnxPaddleOcr::from_paths(&paths, 0.3, 1.5, 1000, 1600)
        .expect("build OnnxPaddleOcr from bundled assets");

    // "clean" set used for the gate — the standard, well-formed text fixtures.
    // low_contrast / small_dense are intentionally hard and tracked but not
    // part of the hard gate.
    let gate_set = [
        "clean_paragraph.png",
        "title_body.png",
        "tech_terms.png",
        "korean_heavy.png",
        "numbers_table.png",
    ];

    let mut gate_cers = Vec::new();
    let mut names: Vec<&String> = gt.keys().collect();
    names.sort();
    println!("\n=== paddle-onnx CER per fixture ===");
    for name in names {
        let img_path = fdir.join(name);
        if !img_path.exists() {
            continue;
        }
        let bytes = std::fs::read(&img_path).unwrap();
        let t0 = std::time::Instant::now();
        let out = engine.recognize(&bytes, None).expect("recognize");
        let dt = t0.elapsed();
        let c = cer(&gt[name], &out.joined);
        if std::env::var("KEBAB_OCR_DUMP").is_ok() {
            println!("  GT  [{name}]: {:?}", norm(&gt[name]));
            println!("  OUT [{name}]: {:?}", norm(&out.joined));
        }
        let gated = gate_set.contains(&name.as_str());
        println!(
            "{:<22} CER={:.4} {} ({} regions, {} ms)",
            name,
            c,
            if gated { "[gate]" } else { "      " },
            out.regions.len(),
            dt.as_millis()
        );
        if gated {
            gate_cers.push(c);
        }
    }

    assert!(!gate_cers.is_empty(), "no gate fixtures were scored");
    let mean = gate_cers.iter().sum::<f64>() / gate_cers.len() as f64;
    println!("=== mean gate CER = {mean:.4} (threshold 0.05) ===\n");
    assert!(
        mean <= 0.05,
        "paddle-onnx mean CER {mean:.4} exceeds 0.05 gate"
    );
}
