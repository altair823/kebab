//! Snapshot test pinning the JSON wire form of `kebab_core::IngestReport`
//! for an inline fixture run. The store crate doesn't (yet) write
//! IngestReports — that's `kb-app`'s job — but the wire schema lives in
//! `kb-core`, and we want a determinism pin that fails loudly if the
//! shape drifts.
//!
//! Set `UPDATE_SNAPSHOTS=1` to re-bake the baseline.

use std::path::PathBuf;

use kebab_core::{
    AssetId, ChunkerVersion, DocumentId, IngestItem, IngestItemKind, IngestReport,
    ParserVersion, SourceScope, WorkspacePath,
};
use serde_json::Value;

fn baseline_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("snapshots")
        .join("ingest_report.snapshot.json")
}

fn fixture_report() -> IngestReport {
    IngestReport {
        scope: SourceScope {
            root: PathBuf::from("/home/u/KB"),
            include: vec!["**/*.md".into()],
            exclude: vec![".git/**".into()],
        },
        scanned: 3,
        new: 2,
        updated: 1,
        skipped: 0,
        unchanged: 0,
        errors: 0,
        duration_ms: 187,
        skipped_by_extension: std::collections::BTreeMap::new(),
        skipped_gitignore: 0,
        skipped_kebabignore: 0,
        skipped_builtin_blacklist: 0,
        skipped_generated: 0,
        skipped_size_exceeded: 0,
        skip_examples: kebab_core::SkipExamples::default(),
        items: Some(vec![
            IngestItem {
                kind: IngestItemKind::New,
                doc_id: Some(DocumentId("a".repeat(32))),
                doc_path: WorkspacePath::new("notes/alpha.md".into()).unwrap(),
                asset_id: Some(AssetId("a".repeat(32))),
                byte_len: Some(1234),
                block_count: Some(7),
                chunk_count: Some(3),
                parser_version: Some(ParserVersion("md-frontmatter-v2".into())),
                chunker_version: Some(ChunkerVersion("md-heading-v1".into())),
                warnings: vec![],
                error: None,
            },
            IngestItem {
                kind: IngestItemKind::Updated,
                doc_id: Some(DocumentId("b".repeat(32))),
                doc_path: WorkspacePath::new("notes/beta.md".into()).unwrap(),
                asset_id: Some(AssetId("b".repeat(32))),
                byte_len: Some(2048),
                block_count: Some(12),
                chunk_count: Some(5),
                parser_version: Some(ParserVersion("md-frontmatter-v2".into())),
                chunker_version: Some(ChunkerVersion("md-heading-v1".into())),
                warnings: vec!["malformed frontmatter".into()],
                error: None,
            },
        ]),
    }
}

#[test]
fn ingest_report_wire_form_is_stable() {
    let report = fixture_report();
    let actual = serde_json::to_value(&report).unwrap();
    let baseline = match std::fs::read_to_string(baseline_path()) {
        Ok(s) => s,
        Err(_) if std::env::var("UPDATE_SNAPSHOTS").is_ok() => {
            std::fs::create_dir_all(baseline_path().parent().unwrap()).unwrap();
            let pretty = serde_json::to_string_pretty(&actual).unwrap();
            std::fs::write(baseline_path(), format!("{pretty}\n")).unwrap();
            return;
        }
        Err(e) => panic!(
            "missing baseline {}; run with UPDATE_SNAPSHOTS=1: {e}",
            baseline_path().display()
        ),
    };
    let expected: Value = serde_json::from_str(&baseline).unwrap();
    if actual != expected {
        if std::env::var("UPDATE_SNAPSHOTS").is_ok() {
            let pretty = serde_json::to_string_pretty(&actual).unwrap();
            std::fs::write(baseline_path(), format!("{pretty}\n")).unwrap();
            return;
        }
        let pretty = serde_json::to_string_pretty(&actual).unwrap();
        panic!(
            "ingest_report snapshot drift\n\
             --- expected ({}) ---\n{baseline}\n\
             --- actual ---\n{pretty}",
            baseline_path().display()
        );
    }
}
