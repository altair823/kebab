//! TUI background-ingest worker + status-bar reducer (p9-fb-03).
//!
//! The Library pane's `r` key fires `start_ingest`, which spawns a
//! worker thread calling
//! `kebab_app::ingest_with_config_progress(.., Some(tx))`. The run
//! loop drains the matching `rx` once per frame via
//! `drain_progress` and re-renders the status bar from the
//! accumulated counts. When the worker emits a terminal event
//! (`Completed` / `Aborted`) the status line freezes for a few
//! seconds (`TERMINAL_LINE_HOLD_SECS`) and then `tick_clear` returns
//! true so the run loop can drop the slot.
//!
//! Cancel (p9-fb-04) is wired by sharing an `Arc<AtomicBool>`
//! between the worker thread (polled at each asset-loop boundary
//! inside `kebab_app::ingest_with_config_cancellable`) and the TUI
//! key handler (`Esc` / `Ctrl-C` flips it via `cancel_running_ingest`).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;

use kebab_app::IngestEvent;
use kebab_core::SourceScope;

use crate::app::{App, IngestState, TERMINAL_LINE_HOLD_SECS};

/// Already-running guard. Returns `Err` if `app.ingest_state` is
/// already populated — pressing `r` twice in a row should not spawn
/// two parallel workers (SQLite is mutexed but Lance writes can race
/// each other).
pub fn start_ingest(app: &mut App) -> anyhow::Result<()> {
    if app.ingest_state.is_some() {
        anyhow::bail!("ingest already running");
    }
    let cfg = app.config.clone();
    let scope = SourceScope {
        root: std::path::PathBuf::from(&cfg.workspace.root),
        exclude: cfg.workspace.exclude.clone(),
        ..Default::default()
    };
    let (tx, rx) = mpsc::channel::<IngestEvent>();
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_for_worker = cancel.clone();
    let cfg_for_thread = cfg;
    let thread = thread::spawn(move || {
        kebab_app::ingest_with_config_cancellable(
            cfg_for_thread,
            scope,
            true,
            Some(tx),
            Some(cancel_for_worker),
        )
    });
    app.ingest_state = Some(IngestState {
        rx,
        counts: kebab_app::AggregateCounts::default(),
        current_path: None,
        current_idx: 0,
        started_at: std::time::Instant::now(),
        terminal_at: None,
        aborted: false,
        thread: Some(thread),
        cancel,
    });
    Ok(())
}

/// Flip the cancel token of an in-flight ingest. Returns `true` if a
/// run was actually in flight (and thus the signal will reach the
/// worker), `false` if there was nothing to cancel — the caller
/// (key handler) can decide whether to swallow the keypress or let
/// the original pane action run.
pub fn cancel_running_ingest(app: &App) -> bool {
    match app.ingest_state.as_ref() {
        Some(state) if state.terminal_at.is_none() => {
            state.cancel.store(true, Ordering::Relaxed);
            true
        }
        _ => false,
    }
}

/// Drain whatever progress events have arrived since the last tick.
/// Non-blocking. Caller (the run loop) calls this once per frame.
///
/// On a terminal event (`Completed` / `Aborted`) the function records
/// `terminal_at = Instant::now()` so subsequent ticks can decide when
/// to clear the slot.
pub fn drain_progress(app: &mut App) {
    let Some(state) = app.ingest_state.as_mut() else {
        return;
    };
    while let Ok(event) = state.rx.try_recv() {
        apply_event(state, event);
    }
}

fn apply_event(state: &mut IngestState, event: IngestEvent) {
    match event {
        IngestEvent::ScanStarted { .. } => {
            // No counter to update; `started_at` already set by
            // `start_ingest`. The status line shows "scanning…" while
            // counts.scanned is zero.
        }
        IngestEvent::ScanCompleted { total } => {
            state.counts.scanned = total;
        }
        IngestEvent::AssetStarted { idx, path, .. } => {
            state.current_idx = idx;
            state.current_path = Some(path);
        }
        IngestEvent::AssetFinished { result, chunks, .. } => {
            // Per-asset counter increments mirror the way
            // `kebab-app::ingest_with_config_progress` aggregates the
            // final report — kept in sync so the status bar's running
            // totals match the eventual `Completed { counts }`.
            match result {
                kebab_core::IngestItemKind::New => {
                    state.counts.new = state.counts.new.saturating_add(1);
                    state.counts.chunks_indexed =
                        state.counts.chunks_indexed.saturating_add(chunks);
                }
                kebab_core::IngestItemKind::Updated => {
                    state.counts.updated = state.counts.updated.saturating_add(1);
                    state.counts.chunks_indexed =
                        state.counts.chunks_indexed.saturating_add(chunks);
                }
                kebab_core::IngestItemKind::Skipped => {
                    state.counts.skipped = state.counts.skipped.saturating_add(1);
                }
                kebab_core::IngestItemKind::Unchanged => {
                    state.counts.unchanged = state.counts.unchanged.saturating_add(1);
                }
                kebab_core::IngestItemKind::Error => {
                    state.counts.errors = state.counts.errors.saturating_add(1);
                }
            }
        }
        IngestEvent::Completed { counts } => {
            // Trust the facade's authoritative aggregate — replaces
            // any tiny drift between our running totals and the
            // final report.
            state.counts = counts;
            state.current_path = None;
            state.terminal_at = Some(std::time::Instant::now());
            state.aborted = false;
        }
        IngestEvent::Aborted { counts } => {
            state.counts = counts;
            state.current_path = None;
            state.terminal_at = Some(std::time::Instant::now());
            state.aborted = true;
        }
        // v0.20.0 sub-item 1: per-page PDF OCR events — TUI does not
        // surface per-page OCR progress in v1; no counter to update.
        IngestEvent::PdfOcrStarted { .. }
        | IngestEvent::PdfOcrFinished { .. }
        // v0.24.0 asset-internal phase events: the status-bar reducer tracks
        // per-asset counters, not sub-asset phase progress, so these are
        // no-ops here (the CLI / --json surfaces render them).
        | IngestEvent::AssetChunked { .. }
        | IngestEvent::ExpansionProgress { .. }
        | IngestEvent::AssetTimings { .. } => {}
    }
}

/// Should the run loop drop `app.ingest_state` now? True when the
/// terminal event arrived ≥ `TERMINAL_LINE_HOLD_SECS` ago.
pub fn ready_to_clear(state: &IngestState) -> bool {
    match state.terminal_at {
        Some(t) => t.elapsed().as_secs() >= TERMINAL_LINE_HOLD_SECS,
        None => false,
    }
}

/// Render the status-bar text for the current `IngestState`. Pure —
/// the run loop wraps this in a Paragraph widget. Returns the
/// human-friendly line per spec §p9-fb-03 ("`ingest: 142/1024 (14%)
/// parsing notes/foo.md  [0:42]`").
pub fn status_line(state: &IngestState) -> String {
    if state.terminal_at.is_some() {
        let elapsed = state.started_at.elapsed();
        let secs = elapsed.as_secs();
        if state.aborted {
            let skipped_breakdown = kebab_app::ingest_progress::render_skipped_breakdown(
                &state.counts.skipped_by_extension,
            );
            return format!(
                "✗ ingest aborted at {}/{} after {}s (new={} updated={} unchanged={} skipped={}{} errors={})",
                state.counts.scanned.saturating_sub(state.counts.errors),
                state.counts.scanned,
                secs,
                state.counts.new,
                state.counts.updated,
                state.counts.unchanged,
                state.counts.skipped,
                skipped_breakdown,
                state.counts.errors,
            );
        }
        let skipped_breakdown = kebab_app::ingest_progress::render_skipped_breakdown(
            &state.counts.skipped_by_extension,
        );
        return format!(
            "✓ ingest: {} docs ({} new, {} updated, {} unchanged, {} skipped{}), {} chunks indexed in {}s",
            state.counts.scanned,
            state.counts.new,
            state.counts.updated,
            state.counts.unchanged,
            state.counts.skipped,
            skipped_breakdown,
            state.counts.chunks_indexed,
            secs,
        );
    }
    if state.counts.scanned == 0 {
        let secs = state.started_at.elapsed().as_secs();
        return format!("ingest: scanning… [{secs}s]");
    }
    let pct =
        u64::from(state.current_idx).saturating_mul(100) / u64::from(state.counts.scanned.max(1));
    let elapsed = state.started_at.elapsed();
    let mm = elapsed.as_secs() / 60;
    let ss = elapsed.as_secs() % 60;
    let path = state.current_path.as_deref().unwrap_or("…");
    format!(
        "ingest: {}/{} ({}%) {} [{}:{:02}]",
        state.current_idx, state.counts.scanned, pct, path, mm, ss,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use kebab_app::AggregateCounts;
    use kebab_core::IngestItemKind;
    use std::sync::mpsc;

    fn fresh_state() -> IngestState {
        let (_tx, rx) = mpsc::channel::<IngestEvent>();
        IngestState {
            rx,
            counts: AggregateCounts::default(),
            current_path: None,
            current_idx: 0,
            started_at: std::time::Instant::now(),
            terminal_at: None,
            aborted: false,
            thread: None,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    #[test]
    fn apply_scan_completed_sets_total() {
        let mut s = fresh_state();
        apply_event(&mut s, IngestEvent::ScanCompleted { total: 42 });
        assert_eq!(s.counts.scanned, 42);
    }

    #[test]
    fn apply_asset_finished_accumulates_per_kind_counters() {
        let mut s = fresh_state();
        apply_event(
            &mut s,
            IngestEvent::AssetFinished {
                idx: 1,
                total: 3,
                result: IngestItemKind::New,
                chunks: 5,
            },
        );
        apply_event(
            &mut s,
            IngestEvent::AssetFinished {
                idx: 2,
                total: 3,
                result: IngestItemKind::Updated,
                chunks: 2,
            },
        );
        apply_event(
            &mut s,
            IngestEvent::AssetFinished {
                idx: 3,
                total: 3,
                result: IngestItemKind::Skipped,
                chunks: 0,
            },
        );
        assert_eq!(s.counts.new, 1);
        assert_eq!(s.counts.updated, 1);
        assert_eq!(s.counts.skipped, 1);
        assert_eq!(s.counts.chunks_indexed, 7);
    }

    #[test]
    fn apply_completed_replaces_counts_and_marks_terminal() {
        let mut s = fresh_state();
        let final_counts = AggregateCounts {
            scanned: 10,
            new: 5,
            updated: 5,
            chunks_indexed: 50,
            ..Default::default()
        };
        apply_event(
            &mut s,
            IngestEvent::Completed {
                counts: final_counts.clone(),
            },
        );
        assert_eq!(s.counts, final_counts);
        assert!(s.terminal_at.is_some());
        assert!(!s.aborted);
    }

    #[test]
    fn apply_aborted_marks_aborted_flag() {
        let mut s = fresh_state();
        apply_event(
            &mut s,
            IngestEvent::Aborted {
                counts: AggregateCounts::default(),
            },
        );
        assert!(s.terminal_at.is_some());
        assert!(s.aborted);
    }

    #[test]
    fn status_line_scanning_shows_dots() {
        let s = fresh_state();
        let line = status_line(&s);
        assert!(line.starts_with("ingest: scanning…"), "got: {line}");
    }

    #[test]
    fn status_line_in_progress_shows_count_path_pct() {
        let mut s = fresh_state();
        apply_event(&mut s, IngestEvent::ScanCompleted { total: 100 });
        apply_event(
            &mut s,
            IngestEvent::AssetStarted {
                idx: 14,
                total: 100,
                path: "notes/foo.md".into(),
                media: "markdown".into(),
            },
        );
        let line = status_line(&s);
        assert!(line.contains("14/100"), "got: {line}");
        assert!(line.contains("(14%)"), "got: {line}");
        assert!(line.contains("notes/foo.md"), "got: {line}");
    }

    #[test]
    fn status_line_terminal_completed_shows_check_mark_and_totals() {
        let mut s = fresh_state();
        apply_event(
            &mut s,
            IngestEvent::Completed {
                counts: AggregateCounts {
                    scanned: 10,
                    new: 8,
                    updated: 1,
                    skipped: 1,
                    chunks_indexed: 50,
                    ..Default::default()
                },
            },
        );
        let line = status_line(&s);
        assert!(line.starts_with("✓ ingest:"), "got: {line}");
        assert!(line.contains("10 docs"), "got: {line}");
        assert!(line.contains("50 chunks"), "got: {line}");
    }

    #[test]
    fn status_line_terminal_aborted_shows_cross() {
        let mut s = fresh_state();
        s.current_idx = 7;
        apply_event(
            &mut s,
            IngestEvent::Aborted {
                counts: AggregateCounts {
                    scanned: 100,
                    errors: 0,
                    ..Default::default()
                },
            },
        );
        let line = status_line(&s);
        assert!(line.starts_with("✗ ingest aborted"), "got: {line}");
        assert!(line.contains("100/100"), "got: {line}");
    }

    #[test]
    fn ready_to_clear_false_until_hold_elapses() {
        let mut s = fresh_state();
        s.terminal_at = Some(std::time::Instant::now());
        assert!(!ready_to_clear(&s));
    }

    #[test]
    fn ready_to_clear_true_in_absence_of_terminal_is_false() {
        let s = fresh_state();
        assert!(!ready_to_clear(&s));
    }

    #[test]
    fn cancel_running_ingest_returns_false_when_no_state() {
        let cfg = kebab_config::Config::defaults();
        let app = App::new(cfg).unwrap();
        assert!(!cancel_running_ingest(&app));
    }

    #[test]
    fn cancel_running_ingest_flips_token_when_in_flight() {
        let cfg = kebab_config::Config::defaults();
        let mut app = App::new(cfg).unwrap();
        app.ingest_state = Some(fresh_state());
        let token = app.ingest_state.as_ref().unwrap().cancel.clone();
        assert!(!token.load(Ordering::Relaxed));
        assert!(cancel_running_ingest(&app));
        assert!(token.load(Ordering::Relaxed));
    }

    #[test]
    fn cancel_running_ingest_returns_false_when_terminal_already_seen() {
        let cfg = kebab_config::Config::defaults();
        let mut app = App::new(cfg).unwrap();
        let mut s = fresh_state();
        s.terminal_at = Some(std::time::Instant::now());
        app.ingest_state = Some(s);
        // No worker to cancel — already terminated.
        assert!(!cancel_running_ingest(&app));
    }

    #[test]
    fn status_line_terminal_includes_skipped_breakdown() {
        let mut s = fresh_state();
        let skipped_by_extension = std::collections::BTreeMap::from([
            ("docx".to_string(), 2u32),
            ("txt".to_string(), 1u32),
        ]);
        let counts = AggregateCounts {
            scanned: 10,
            skipped: 3,
            skipped_by_extension,
            ..Default::default()
        };
        apply_event(&mut s, IngestEvent::Completed { counts });
        let line = status_line(&s);
        assert!(
            line.contains("3 skipped: 2 docx, 1 txt"),
            "breakdown must appear in: {line}"
        );
    }

    #[test]
    fn status_line_aborted_includes_skipped_breakdown() {
        let mut s = fresh_state();
        let skipped_by_extension = std::collections::BTreeMap::from([("pdf".to_string(), 2u32)]);
        let counts = AggregateCounts {
            scanned: 5,
            skipped: 2,
            skipped_by_extension,
            ..Default::default()
        };
        apply_event(&mut s, IngestEvent::Aborted { counts });
        let line = status_line(&s);
        assert!(
            line.contains("skipped=2: 2 pdf"),
            "breakdown must appear in: {line}"
        );
    }
}
