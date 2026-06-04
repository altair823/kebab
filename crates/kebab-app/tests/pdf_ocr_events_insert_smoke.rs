//! Integration smoke test: dual-write (ndjson + SQLite) for PDF OCR events.
//! AC-3: SQLite row count and doc_id matches ndjson LogEvent::Ocr.
//!
//! Uses wiremock to stub the Ollama `/api/generate` endpoint so the test
//! runs without a live Ollama instance.

mod common;

use std::path::PathBuf;

use common::TestEnv;
use kebab_config::LoggingCfg;
use serde_json::Value;
use tokio::task::spawn_blocking;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn scanned_pdf_src() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("kebab-parse-pdf/tests/fixtures/scanned_page1.pdf")
}

/// AC-3: ndjson OCR line count == pdf_ocr_events row count, and doc_id matches.
#[tokio::test]
async fn ingest_dual_write_doc_id_matches_ndjson() {
    let src = scanned_pdf_src();
    if !src.exists() {
        eprintln!("skipping test: scanned_page1.pdf fixture not found");
        return;
    }

    let server = MockServer::start().await;
    // Stub Ollama /api/generate to return a minimal OCR response.
    Mock::given(method("POST"))
        .and(path("/api/generate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "model": "qwen2.5vl:3b",
            "response": "test ocr output",
            "done": true,
            "done_reason": "stop"
        })))
        .mount(&server)
        .await;

    let mock_url = server.uri();

    let result = spawn_blocking(move || {
        let mut env = TestEnv::lexical_only();
        // Enable PDF OCR + set up mock endpoint
        env.config.ingest.pdf.ocr.enabled = true;
        env.config.ingest.pdf.ocr.endpoint = Some(mock_url.clone());
        env.config.ingest.pdf.ocr.model = "qwen2.5vl:3b".to_string();
        // Enable ingest log
        let log_dir = env.temp.path().join("logs");
        std::fs::create_dir_all(&log_dir).unwrap();
        env.config.logging = LoggingCfg {
            ingest_log_enabled: true,
            ingest_log_dir: log_dir.clone(),
            ..Default::default()
        };

        // Copy scanned PDF into workspace
        let dest = env.workspace_root.join("scanned.pdf");
        std::fs::copy(scanned_pdf_src(), &dest).expect("copy scanned PDF");

        // Run ingest
        kebab_app::ingest_with_config(env.config.clone(), env.scope(), false).expect("ingest");

        // Read ndjson log
        let log_files: Vec<_> = std::fs::read_dir(&log_dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                name.starts_with("ingest-") && name.ends_with(".ndjson")
            })
            .collect();
        assert_eq!(log_files.len(), 1, "expected 1 ndjson log file");

        let body = std::fs::read_to_string(log_files[0].path()).unwrap();
        let ocr_lines: Vec<Value> = body
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .filter(|v: &Value| v.get("kind").and_then(Value::as_str) == Some("ocr"))
            .collect();

        // Read pdf_ocr_events from SQLite
        let db_path = PathBuf::from(&env.config.storage.data_dir).join("kebab.sqlite");
        let conn = rusqlite::Connection::open(&db_path).expect("open db");
        let rows: Vec<(Option<String>, String)> = {
            let mut stmt = conn
                .prepare("SELECT doc_id, doc_path FROM pdf_ocr_events ORDER BY id")
                .expect("prepare");
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .expect("query")
                .map(|r| r.expect("row"))
                .collect()
        };

        (ocr_lines, rows)
    })
    .await
    .expect("spawn_blocking");

    let (ocr_lines, rows) = result;

    // At least one OCR event must be produced
    assert!(!ocr_lines.is_empty(), "expected ≥1 ndjson ocr line");
    assert!(!rows.is_empty(), "expected ≥1 pdf_ocr_events row");

    // Row counts must match
    assert_eq!(
        ocr_lines.len(),
        rows.len(),
        "ndjson ocr lines ({}) must equal pdf_ocr_events rows ({})",
        ocr_lines.len(),
        rows.len()
    );

    // doc_id in both sources must be non-null and consistent
    for (line, (sql_doc_id, _sql_doc_path)) in ocr_lines.iter().zip(rows.iter()) {
        let json_doc_id = line.get("doc_id").and_then(Value::as_str);
        assert!(
            json_doc_id.is_some(),
            "ndjson ocr line should have doc_id: {line}"
        );
        assert!(
            sql_doc_id.is_some(),
            "pdf_ocr_events row should have doc_id"
        );
        assert_eq!(
            json_doc_id,
            sql_doc_id.as_deref(),
            "ndjson doc_id must equal SQLite doc_id"
        );
    }
}
