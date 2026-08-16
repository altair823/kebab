//! Issue #228: the deleted-file sweep runs between the scan and the asset
//! loop and used to emit nothing at all — no progress events, no ndjson
//! log lines. A sweep that took 32 hours in dogfooding was externally
//! indistinguishable from a hang, and users killed the run three times in
//! a row. These tests pin the observability, not the purging (which
//! `file_deletion_auto_purge.rs` already covers).

mod common;

use std::sync::mpsc;

use common::TestEnv;
use kebab_app::{IngestEvent, IngestOpts, ingest_with_config};
use kebab_core::SourceScope;

/// Ingest whatever is in the workspace, collecting progress events.
fn ingest_collecting(env: &TestEnv) -> Vec<IngestEvent> {
    let (tx, rx) = mpsc::channel::<IngestEvent>();
    ingest_with_config(
        env.config.clone(),
        env.scope(),
        IngestOpts {
            progress: Some(tx),
            ..Default::default()
        },
    )
    .expect("ingest must succeed");
    let mut events = Vec::new();
    while let Ok(ev) = rx.recv() {
        events.push(ev);
    }
    events
}

fn sweep_bounds(events: &[IngestEvent]) -> (Option<u32>, Option<(u32, u32)>) {
    let started = events.iter().find_map(|e| match e {
        IngestEvent::SweepStarted { total } => Some(*total),
        _ => None,
    });
    let completed = events.iter().find_map(|e| match e {
        IngestEvent::SweepCompleted {
            checked, purged, ..
        } => Some((*checked, *purged)),
        _ => None,
    });
    (started, completed)
}

#[test]
fn sweep_emits_a_bounded_phase_with_one_event_per_candidate() {
    let env = TestEnv::lexical_only();
    for i in 0..4 {
        std::fs::write(
            env.workspace_root.join(format!("gone{i}.rs")),
            format!("// file {i}\nfn f{i}() {{}}\n"),
        )
        .unwrap();
    }
    let first = ingest_collecting(&env);
    assert!(
        matches!(sweep_bounds(&first), (None, None)),
        "a first ingest has no stored paths outside its own scan, so there \
         is no sweep phase to announce: {first:?}"
    );

    for i in 0..4 {
        std::fs::remove_file(env.workspace_root.join(format!("gone{i}.rs"))).unwrap();
    }
    let second = ingest_collecting(&env);

    let (total, done) = sweep_bounds(&second);
    let total = total.expect("second ingest must announce the sweep phase");
    let (checked, purged) = done.expect("and must announce its end");
    assert_eq!(total, 4, "the four deleted files are the candidates");
    assert_eq!(checked, total, "every announced candidate is examined");
    assert_eq!(purged, 4, "all four are truly gone");

    // A denominator is only useful if the numerator actually walks it.
    let mut indices: Vec<u32> = second
        .iter()
        .filter_map(|e| match e {
            IngestEvent::SweepProgress { idx, total: t, .. } => {
                assert_eq!(*t, total, "every progress event carries the same total");
                Some(*idx)
            }
            _ => None,
        })
        .collect();
    indices.sort_unstable();
    assert_eq!(
        indices,
        (1..=total).collect::<Vec<_>>(),
        "one event per candidate, 1-based and contiguous: {second:?}"
    );

    // Ordering is what makes the phase legible: the bar has to be told the
    // length before it is told a position.
    let start_at = second
        .iter()
        .position(|e| matches!(e, IngestEvent::SweepStarted { .. }))
        .unwrap();
    let end_at = second
        .iter()
        .position(|e| matches!(e, IngestEvent::SweepCompleted { .. }))
        .unwrap();
    let first_progress = second
        .iter()
        .position(|e| matches!(e, IngestEvent::SweepProgress { .. }))
        .unwrap();
    assert!(
        start_at < first_progress && first_progress < end_at,
        "SweepStarted < SweepProgress* < SweepCompleted: {second:?}"
    );
}

/// The reported case was a sweep that purged thousands of documents, but
/// the same silence happens when a narrowed `include` glob leaves stored
/// paths out of scope: the sweep still stats every one of them and purges
/// none. "checked 12115, purged 0" is precisely the answer the user was
/// missing, so a zero-purge sweep must still announce itself.
#[test]
fn a_sweep_that_purges_nothing_still_announces_itself() {
    let env = TestEnv::lexical_only();
    std::fs::write(
        env.workspace_root.join("kept.rs"),
        "// still here\nfn kept() {}\n",
    )
    .unwrap();
    ingest_collecting(&env);

    // Narrow the scan so the fixtures are stored but out of scope. They
    // are still on disk, so the sweep must examine each and leave it alone.
    let narrowed = SourceScope {
        root: env.workspace_root.clone(),
        include: vec!["kept.rs".to_string()],
        exclude: env.config.workspace.exclude.clone(),
    };
    let (tx, rx) = mpsc::channel::<IngestEvent>();
    ingest_with_config(
        env.config.clone(),
        narrowed,
        IngestOpts {
            progress: Some(tx),
            ..Default::default()
        },
    )
    .expect("narrowed ingest must succeed");
    let events: Vec<IngestEvent> = rx.into_iter().collect();

    let (total, done) = sweep_bounds(&events);
    let total = total.expect("the phase is announced even when it purges nothing");
    let (checked, purged) = done.expect("and it reports its end");
    assert!(total > 0, "the out-of-scope fixtures are candidates");
    assert_eq!(checked, total);
    assert_eq!(purged, 0, "nothing was deleted from disk: {events:?}");
    assert!(
        events
            .iter()
            .any(|e| matches!(e, IngestEvent::SweepProgress { removed: false, .. })),
        "and the candidates it left alone are still reported: {events:?}"
    );
}

/// The CLI's first Ctrl-C prints "aborting after current asset" and flips
/// the cancel flag. The sweep did not look at it, so during a long sweep
/// that message was simply untrue — and the user's only recourse was a
/// second Ctrl-C, which is `exit(130)` and strands whatever vector deletes
/// are buffered. Issue #228 is a report of someone pressing Ctrl-C three
/// times in exactly this phase.
#[test]
fn a_cancelled_sweep_stops_and_reports_only_what_it_examined() {
    let env = TestEnv::lexical_only();
    for i in 0..6 {
        std::fs::write(
            env.workspace_root.join(format!("bye{i}.rs")),
            format!("// file {i}\nfn g{i}() {{}}\n"),
        )
        .unwrap();
    }
    ingest_collecting(&env);
    for i in 0..6 {
        std::fs::remove_file(env.workspace_root.join(format!("bye{i}.rs"))).unwrap();
    }

    // Pre-cancelled: the sweep must stop at the top of its first
    // iteration rather than walk every candidate.
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let (tx, rx) = mpsc::channel::<IngestEvent>();
    let report = ingest_with_config(
        env.config.clone(),
        env.scope(),
        IngestOpts {
            progress: Some(tx),
            cancel: Some(cancel),
            ..Default::default()
        },
    )
    .expect("a cancelled ingest still returns a report");
    let events: Vec<IngestEvent> = rx.into_iter().collect();

    assert_eq!(
        report.purged_deleted_files, 0,
        "nothing is purged after the flag is already set: {report:?}"
    );
    let (total, done) = sweep_bounds(&events);
    assert!(
        total.expect("the phase is still announced") > 0,
        "the candidates were counted before the check: {events:?}"
    );
    let (checked, purged) = done.expect("and it still reports its end");
    assert_eq!(
        (checked, purged),
        (0, 0),
        "a cancelled sweep must not claim work it did not do — reporting \
         the announced total here would be a lie: {events:?}"
    );
}
