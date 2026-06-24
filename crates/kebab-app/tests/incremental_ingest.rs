//! p9-fb-23: incremental ingest — skip parse/chunk/embed when nothing
//! has changed.
//!
//! Task 7 contract: when `IngestOpts::force_reingest == false` and the
//! per-asset (checksum, parser_version, chunker_version, embedding_version)
//! tuple matches the existing DB record, ingest emits
//! `IngestEvent::AssetFinished { result: Unchanged }` and skips
//! parse / chunk / embed / vector upsert. `force_reingest = true`
//! bypasses the skip path and re-processes every asset as `Updated`.

mod common;

use common::TestEnv;

use kebab_app::{IngestOpts, ingest_with_config};

#[test]
fn second_ingest_of_unchanged_corpus_marks_all_unchanged() {
    let env = TestEnv::lexical_only();

    // First ingest — populates the DB. Use the legacy entry so the
    // assertions cover the "previously ingested" set without needing
    // IngestOpts::default() to behave identically.
    let first = ingest_with_config(env.config.clone(), env.scope(), kebab_app::IngestOpts::default()).unwrap();
    assert_eq!(first.errors, 0, "first ingest must not error: {first:?}");
    assert!(
        first.new >= 1,
        "first ingest must create new docs: {first:?}"
    );
    assert_eq!(
        first.unchanged, 0,
        "first ingest cannot have unchanged: {first:?}"
    );

    let scanned = first.scanned;

    // Second ingest — same files, same versions → all assets must be
    // labelled Unchanged (no parse / chunk / embed re-work).
    let second = ingest_with_config(env.config.clone(), env.scope(), IngestOpts::default())
        .unwrap();
    assert_eq!(
        second.scanned, scanned,
        "second scanned matches first: {second:?}"
    );
    assert_eq!(second.new, 0, "no new docs on re-ingest: {second:?}");
    assert_eq!(
        second.updated, 0,
        "nothing should be marked updated: {second:?}"
    );
    assert_eq!(
        second.unchanged, scanned,
        "every doc must be Unchanged: {second:?}"
    );
    assert_eq!(second.errors, 0, "no errors expected: {second:?}");
}

#[test]
fn force_reingest_bypasses_skip() {
    let env = TestEnv::lexical_only();

    let first = ingest_with_config(env.config.clone(), env.scope(), kebab_app::IngestOpts::default()).unwrap();
    assert_eq!(first.errors, 0, "first ingest must not error: {first:?}");
    assert!(
        first.new >= 1,
        "first ingest must create new docs: {first:?}"
    );
    let scanned = first.scanned;

    let second = ingest_with_config(
        env.config.clone(),
        env.scope(),
        IngestOpts {
            force_reingest: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(second.scanned, scanned);
    assert_eq!(
        second.unchanged, 0,
        "force_reingest must bypass skip: {second:?}"
    );
    assert_eq!(
        second.updated, scanned,
        "every doc must be re-processed as Updated: {second:?}"
    );
    assert_eq!(second.new, 0, "no new docs on force reingest: {second:?}");
    assert_eq!(second.errors, 0, "no errors expected: {second:?}");
}
