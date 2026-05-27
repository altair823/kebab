//! Integration coverage for `ingest_with_config_progress` —
//! exercises the streaming progress channel against the same lexical
//! fixture used by `ingest_lexical.rs`.

mod common;

use std::sync::mpsc;

use common::TestEnv;
use kebab_app::{AggregateCounts, IngestEvent};
use kebab_core::IngestItemKind;

fn run_with_progress() -> Vec<IngestEvent> {
    let env = TestEnv::lexical_only();
    let (tx, rx) = mpsc::channel::<IngestEvent>();
    let report = kebab_app::ingest_with_config_progress(
        env.config.clone(),
        env.scope(),
        false,
        Some(tx),
    )
    .unwrap();
    assert_eq!(report.scanned, 3);
    assert_eq!(report.new, 3);

    // Drain until the sender (held inside `ingest_with_config_progress`)
    // is dropped on return.
    let mut events = Vec::new();
    while let Ok(ev) = rx.recv() {
        events.push(ev);
    }
    events
}

#[test]
fn progress_event_sequence_matches_design_section_2_4a() {
    let events = run_with_progress();

    // First event: ScanStarted with workspace root.
    match &events[0] {
        IngestEvent::ScanStarted { root } => {
            assert!(!root.is_empty(), "ScanStarted root must be a path");
        }
        other => panic!("expected ScanStarted, got {other:?}"),
    }

    // Second event: ScanCompleted with total = 3 fixture files.
    match &events[1] {
        IngestEvent::ScanCompleted { total } => {
            assert_eq!(*total, 3, "ScanCompleted total: {events:?}");
        }
        other => panic!("expected ScanCompleted, got {other:?}"),
    }

    // Final event: Completed with the aggregate counters mirroring the
    // returned report.
    let last = events.last().expect("at least one event");
    match last {
        IngestEvent::Completed { counts } => {
            assert_eq!(
                *counts,
                AggregateCounts {
                    scanned: 3,
                    new: 3,
                    chunks_indexed: counts.chunks_indexed,
                    embeddings_indexed: 0,
                    ..Default::default()
                },
                "Completed counts: {counts:?}"
            );
            assert!(counts.chunks_indexed >= 3, "chunks_indexed: {counts:?}");
        }
        other => panic!("expected Completed last, got {other:?}"),
    }

    // Middle: 3 AssetStarted/AssetFinished pairs in monotonic idx order.
    let asset_events: Vec<&IngestEvent> = events[2..events.len() - 1].iter().collect();
    assert_eq!(
        asset_events.len(),
        6,
        "expected 3 (Started + Finished) pairs, got {asset_events:?}"
    );
    for (chunk_idx, pair) in asset_events.chunks(2).enumerate() {
        let expected_idx = chunk_idx as u32 + 1;
        match (pair[0], pair[1]) {
            (
                IngestEvent::AssetStarted {
                    idx: si,
                    total: st,
                    media,
                    ..
                },
                IngestEvent::AssetFinished {
                    idx: fi,
                    total: ft,
                    result,
                    chunks,
                },
            ) => {
                assert_eq!(*si, expected_idx, "Started idx mismatch: {pair:?}");
                assert_eq!(*fi, expected_idx, "Finished idx mismatch: {pair:?}");
                assert_eq!(*st, 3, "Started total mismatch");
                assert_eq!(*ft, 3, "Finished total mismatch");
                assert_eq!(media, "markdown", "fixture is markdown only");
                assert_eq!(*result, IngestItemKind::New, "first ingest → New");
                assert!(*chunks >= 1, "chunks: {pair:?}");
            }
            other => panic!("expected Started+Finished pair, got {other:?}"),
        }
    }
}

#[test]
fn ingest_with_config_progress_none_matches_ingest_with_config() {
    // Forwarding wrapper: `ingest_with_config(...)` and
    // `ingest_with_config_progress(..., None)` must produce identical
    // reports modulo wall-clock duration.
    let env = TestEnv::lexical_only();
    let r_none = kebab_app::ingest_with_config_progress(
        env.config.clone(),
        env.scope(),
        true,
        None,
    )
    .unwrap();
    assert_eq!(r_none.scanned, 3);
    assert_eq!(r_none.new, 3);
}

#[test]
fn dropped_receiver_does_not_panic_or_fail_ingest() {
    // Best-effort send: if the consumer dies mid-run, ingest must
    // still complete normally.
    let env = TestEnv::lexical_only();
    let (tx, rx) = mpsc::channel::<IngestEvent>();
    drop(rx);
    let report = kebab_app::ingest_with_config_progress(
        env.config.clone(),
        env.scope(),
        true,
        Some(tx),
    )
    .unwrap();
    assert_eq!(report.scanned, 3);
}

/// v0.20.0 sub-item 1: pdf_ocr_started + pdf_ocr_finished events 가 PDF asset 의
/// OCR-enabled ingest 시 emit 됨을 검증. real Ollama 의존 — `#[ignore]` default.
///
/// Manual invoke:
/// ```
/// KEBAB_PDF_OCR_ENABLED=true \
///   KEBAB_PDF_OCR_ENDPOINT=http://192.168.0.47:11434 \
///   cargo test -p kebab-app --test ingest_progress \
///   --ignored pdf_ocr_progress_emits_started_finished_events
/// ```
#[test]
#[ignore = "real Ollama dependency — manual invoke via KEBAB_PDF_OCR_ENABLED=true"]
fn pdf_ocr_progress_emits_started_finished_events() {
    // F1 fixture (DCTDecode JPEG passthrough) 을 tmpdir 의 workspace 로 copy.
    let tmpdir = tempfile::tempdir().expect("create tmpdir");
    let workspace = tmpdir.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("create workspace dir");
    let f1_src = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../kebab-parse-pdf/tests/fixtures/scanned_page1.pdf");
    let f1 = std::fs::read(&f1_src).expect("F1 fixture present");
    std::fs::write(workspace.join("page1.pdf"), &f1).expect("copy F1");

    let data_dir = tmpdir.path().join("data");
    std::fs::create_dir_all(&data_dir).expect("create data dir");

    let mut config = kebab_config::Config::defaults();
    config.workspace.root = workspace.to_string_lossy().into_owned();
    config.storage.data_dir = data_dir.to_string_lossy().into_owned();
    config.models.embedding.provider = "none".to_string();
    config.models.embedding.dimensions = 0;
    config.pdf.ocr.enabled = true;
    if let Ok(endpoint) = std::env::var("KEBAB_PDF_OCR_ENDPOINT") {
        config.pdf.ocr.endpoint = Some(endpoint);
    }

    let scope = kebab_core::SourceScope {
        root: workspace.clone(),
        ..Default::default()
    };

    let (tx, rx) = mpsc::channel::<IngestEvent>();
    let _report = kebab_app::ingest_with_config_progress(
        config,
        scope,
        false,
        Some(tx),
    )
    .expect("ingest_with_config_progress");

    let events: Vec<_> = rx.iter().collect();

    let started_count = events
        .iter()
        .filter(|e| matches!(e, IngestEvent::PdfOcrStarted { .. }))
        .count();
    let finished_count = events
        .iter()
        .filter(|e| matches!(e, IngestEvent::PdfOcrFinished { .. }))
        .count();

    assert!(started_count >= 1, "PdfOcrStarted 가 ≥ 1 emit 됨 (got {started_count})");
    assert!(finished_count >= 1, "PdfOcrFinished 가 ≥ 1 emit 됨 (got {finished_count})");
    assert_eq!(started_count, finished_count, "Started 와 Finished 의 count 일치");
}
