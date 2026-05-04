//! Integration tests for `kb-app::ingest` + `list_docs` + `inspect_*`
//! along the lexical-only path (no embeddings → no AVX requirement).

mod common;

use common::TestEnv;

#[test]
fn ingest_then_list_inspects_round_trip() {
    let env = TestEnv::lexical_only();
    let report =
        kebab_app::ingest_with_config(env.config.clone(), env.scope(), false).unwrap();

    // The fixture has 3 markdown files; first ingest should label them
    // all as New.
    assert_eq!(report.scanned, 3, "scanned: {report:?}");
    assert_eq!(report.new, 3, "new: {report:?}");
    assert_eq!(report.updated, 0, "updated: {report:?}");
    assert_eq!(report.errors, 0, "errors: {report:?}");
    let items = report.items.as_ref().expect("items present");
    assert_eq!(items.len(), 3);
    for it in items {
        assert!(it.error.is_none(), "per-item error: {it:?}");
        assert!(it.doc_id.is_some());
        // Each fixture file emits ≥1 chunk.
        assert!(it.chunk_count.unwrap_or(0) >= 1, "chunks: {it:?}");
    }

    // list_docs returns the 3 docs.
    let docs = kebab_app::list_docs_with_config(
        env.config.clone(),
        kebab_core::DocFilter::default(),
    )
    .unwrap();
    assert_eq!(docs.len(), 3, "docs: {docs:?}");

    // inspect_doc round-trips one of them.
    let any_doc_id = docs[0].doc_id.clone();
    let canonical = kebab_app::inspect_doc_with_config(env.config.clone(), &any_doc_id)
        .unwrap();
    assert_eq!(canonical.doc_id, any_doc_id);
    assert!(!canonical.blocks.is_empty(), "blocks empty");
}

#[test]
fn ingest_idempotent_on_second_run() {
    let env = TestEnv::lexical_only();

    let r1 =
        kebab_app::ingest_with_config(env.config.clone(), env.scope(), false).unwrap();
    assert_eq!(r1.new, 3);

    let r2 =
        kebab_app::ingest_with_config(env.config.clone(), env.scope(), false).unwrap();
    // Same files re-ingested — p9-fb-23 task 7 introduced the early-skip
    // path: when checksum + parser/chunker/embedding versions all match,
    // the second run reports `Unchanged` rather than `Updated`. Pre-p9-fb-23
    // returned `Updated` here. The `force_reingest=true` path still returns
    // `Updated` and is exercised by `incremental_ingest.rs`.
    assert_eq!(r2.scanned, 3, "second scan: {r2:?}");
    assert_eq!(r2.new, 0, "second run new should be 0: {r2:?}");
    assert_eq!(r2.updated, 0, "second run updated: {r2:?}");
    assert_eq!(r2.unchanged, 3, "second run unchanged: {r2:?}");

    // list_docs still has 3 docs (no duplicates).
    let docs = kebab_app::list_docs_with_config(
        env.config.clone(),
        kebab_core::DocFilter::default(),
    )
    .unwrap();
    assert_eq!(docs.len(), 3);
}

#[test]
fn ingest_summary_only_drops_items() {
    let env = TestEnv::lexical_only();
    let report =
        kebab_app::ingest_with_config(env.config.clone(), env.scope(), true).unwrap();
    assert_eq!(report.scanned, 3);
    assert!(report.items.is_none(), "summary-only should null items");
}

#[test]
fn ingest_records_ingest_runs_row_with_aggregate_counts() {
    // The ingest_runs table is the §5.7 sibling of `jobs`: dedicated
    // count columns (`scanned`, `new_count`, …) populated at the end
    // of every run. `summary_only=true` writes `items_json=NULL`; the
    // counts MUST still be present.
    let env = TestEnv::lexical_only();
    let report = kebab_app::ingest_with_config(env.config.clone(), env.scope(), true)
        .unwrap();
    assert_eq!(report.scanned, 3);

    let db_path = std::path::PathBuf::from(&env.config.storage.data_dir)
        .join("kebab.sqlite");
    let conn = rusqlite::Connection::open(&db_path).expect("open kebab.sqlite");
    let (scanned, new_c, updated, skipped, errors, items_json): (
        i64,
        i64,
        i64,
        i64,
        i64,
        Option<String>,
    ) = conn
        .query_row(
            "SELECT scanned, new_count, updated_count, skipped_count,
                    error_count, items_json
             FROM ingest_runs
             ORDER BY started_at DESC
             LIMIT 1",
            [],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            },
        )
        .expect("ingest_runs row present");
    assert_eq!(scanned, 3);
    assert_eq!(new_c, 3);
    assert_eq!(updated, 0);
    assert_eq!(skipped, 0);
    assert_eq!(errors, 0);
    assert!(
        items_json.is_none(),
        "summary_only=true must store items_json=NULL: {items_json:?}"
    );
}

#[test]
fn ingest_provider_none_skips_lance() {
    // `provider="none"` must short-circuit the embedder + vector store
    // build entirely, so the LanceDB directory MUST NOT be created on
    // disk during ingest. `IngestReport` currently has no
    // `embeddings_indexed` field, so we assert via the on-disk lance
    // tree shape (no `<data_dir>/lancedb` directory, or no `*.lance`
    // tables under it).
    let env = TestEnv::lexical_only();
    let report =
        kebab_app::ingest_with_config(env.config.clone(), env.scope(), false).unwrap();
    assert_eq!(report.errors, 0, "lexical-only run must not error");
    assert_eq!(report.new, 3);

    let lance_dir = std::path::PathBuf::from(&env.config.storage.data_dir)
        .join("lancedb");
    if lance_dir.exists() {
        // If the dir was created (e.g., by an earlier consumer touching
        // the path), it MUST contain no `.lance` tables.
        let mut had_lance_table = false;
        for entry in std::fs::read_dir(&lance_dir).expect("read lance_dir") {
            let entry = entry.unwrap();
            if entry
                .path()
                .extension()
                .and_then(|s| s.to_str())
                == Some("lance")
            {
                had_lance_table = true;
                break;
            }
        }
        assert!(
            !had_lance_table,
            "provider=none must not produce any *.lance table under {}",
            lance_dir.display()
        );
    }
}

#[test]
fn list_docs_filters_by_tags_any() {
    let env = TestEnv::lexical_only();
    kebab_app::ingest_with_config(env.config.clone(), env.scope(), true).unwrap();

    let filter = kebab_core::DocFilter {
        tags_any: vec!["python".to_string()],
        ..Default::default()
    };
    let docs = kebab_app::list_docs_with_config(env.config.clone(), filter).unwrap();
    assert_eq!(docs.len(), 1, "expected only the python doc: {docs:?}");
    assert!(docs[0].tags.contains(&"python".to_string()));

    let rust_filter = kebab_core::DocFilter {
        tags_any: vec!["rust".to_string()],
        ..Default::default()
    };
    let rust_docs =
        kebab_app::list_docs_with_config(env.config.clone(), rust_filter).unwrap();
    // intro.md and notes/cargo.md both tag "rust".
    assert_eq!(rust_docs.len(), 2, "expected 2 rust docs: {rust_docs:?}");
}

#[test]
fn inspect_doc_not_found_returns_actionable_error() {
    let env = TestEnv::lexical_only();
    let bogus =
        kebab_core::DocumentId("0000000000000000000000000000000000000000000000000000000000000000".to_string());
    let err = kebab_app::inspect_doc_with_config(env.config.clone(), &bogus).unwrap_err();
    let msg = format!("{err:#}");
    assert!(
        msg.contains("not found"),
        "error must mention not-found: {msg}"
    );
    assert!(
        msg.contains("kb list docs") || msg.contains("list"),
        "error must hint at `kb list docs`: {msg}"
    );
}

#[test]
fn inspect_chunk_not_found_returns_actionable_error() {
    let env = TestEnv::lexical_only();
    let bogus = kebab_core::ChunkId(
        "0000000000000000000000000000000000000000000000000000000000000000".to_string(),
    );
    let err = kebab_app::inspect_chunk_with_config(env.config.clone(), &bogus)
        .unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("not found"), "got: {msg}");
}

/// p9-fb-23 task 6: `ingest_with_config_opts` with `IngestOpts::default()`
/// must behave identically to `ingest_with_config` — first ingest reports
/// all assets as new, no errors, no unchanged.
#[test]
fn ingest_with_config_opts_default_matches_legacy_behaviour() {
    let env = TestEnv::lexical_only();
    let report = kebab_app::ingest_with_config_opts(
        env.config.clone(),
        env.scope(),
        false,
        kebab_app::IngestOpts::default(),
    )
    .unwrap();
    assert!(report.new >= 1, "expected at least one new doc: {report:?}");
    assert_eq!(report.errors, 0, "no errors expected: {report:?}");
    assert_eq!(
        report.unchanged, 0,
        "first ingest cannot have unchanged: {report:?}"
    );
}

/// p9-fb-23 task 5: every freshly-ingested markdown doc must carry
/// `last_chunker_version`. With `provider="none"` (lexical-only),
/// `last_embedding_version` stays `None`.
#[test]
fn ingest_stamps_chunker_version_on_document() {
    let env = TestEnv::lexical_only();
    let report =
        kebab_app::ingest_with_config(env.config.clone(), env.scope(), false).unwrap();
    assert!(report.new >= 1, "expected at least one new doc: {report:?}");
    assert_eq!(report.errors, 0, "no errors expected: {report:?}");

    let docs = kebab_app::list_docs_with_config(
        env.config.clone(),
        kebab_core::DocFilter::default(),
    )
    .unwrap();
    assert!(!docs.is_empty(), "no docs after ingest");

    for doc_entry in &docs {
        let canonical =
            kebab_app::inspect_doc_with_config(env.config.clone(), &doc_entry.doc_id)
                .unwrap();
        assert!(
            canonical.last_chunker_version.is_some(),
            "last_chunker_version must be stamped for doc {}: got {:?}",
            doc_entry.doc_id.0,
            canonical.last_chunker_version,
        );
        // provider="none" → embedder is None → last_embedding_version stays None.
        assert!(
            canonical.last_embedding_version.is_none(),
            "last_embedding_version must be None when provider=none for doc {}: got {:?}",
            doc_entry.doc_id.0,
            canonical.last_embedding_version,
        );
    }
}
