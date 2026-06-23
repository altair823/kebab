//! p9-fb-25 task 5: skipped per-asset items must carry a human-readable
//! reason in `warnings`, and the report's `skipped_by_extension` must
//! aggregate by lowercase extension.

mod common;

use common::TestEnv;

#[test]
fn unsupported_extension_skip_carries_warning_and_is_aggregated() {
    let env = TestEnv::lexical_only();
    let workspace_root = env.config.resolve_workspace_root();
    std::fs::write(workspace_root.join("legacy.docx"), b"unsupported").unwrap();
    std::fs::write(workspace_root.join("Makefile"), b"unsupported").unwrap();

    let report = kebab_app::ingest_with_config(env.config.clone(), env.scope(), false).unwrap();

    let items = report.items.as_ref().expect("items array populated");
    let docx_item = items
        .iter()
        .find(|i| i.doc_path.0.ends_with("legacy.docx"))
        .expect("docx in items");
    assert_eq!(docx_item.kind, kebab_core::IngestItemKind::Skipped);
    assert_eq!(
        docx_item.warnings,
        vec!["unsupported media type: .docx".to_string()],
    );
    let makefile_item = items
        .iter()
        .find(|i| i.doc_path.0.ends_with("Makefile"))
        .expect("Makefile in items");
    assert_eq!(makefile_item.kind, kebab_core::IngestItemKind::Skipped);
    assert_eq!(
        makefile_item.warnings,
        vec!["unsupported media type: <no-ext>".to_string()],
    );
    assert_eq!(report.skipped_by_extension.get("docx").copied(), Some(1));
    assert_eq!(
        report.skipped_by_extension.get("<no-ext>").copied(),
        Some(1)
    );
}
