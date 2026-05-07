//! `kebab ingest` progress display — consumes
//! `kebab_app::IngestEvent` and renders to one of three surfaces:
//!
//! - **TTY 사람 모드**: indicatif `ProgressBar` on stderr (spinner
//!   while scanning, bar after `ScanCompleted`, message updates per
//!   asset). stdout is reserved for the final `ingest_report.v1`.
//! - **non-TTY 사람 모드** (CI / pipe): indicatif uses `hidden`
//!   draw target (no terminal control codes), and we emit one
//!   `ingest: scanning…` / `ingest: N/M …` line per event to stderr
//!   instead. CLI consumers redirecting stderr can still parse it.
//! - **`--json` 모드**: stderr stays silent; every event is dumped to
//!   stdout as `ingest_progress.v1` line-delimited JSON. The final
//!   `ingest_report.v1` line follows after the run completes (per
//!   §2.4a backwards-compat).
//!
//! Each subprocess of the binary creates one `ProgressDisplay` and
//! drives it from a background thread that drains an
//! `mpsc::Receiver<IngestEvent>`. The thread terminates when the
//! `Sender` end is dropped (i.e. when `ingest_with_config_progress`
//! returns).

use std::io::{IsTerminal, Write};
use std::sync::mpsc::Receiver;

use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use kebab_app::IngestEvent;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::wire;

/// Rendering mode for `ProgressDisplay`. The mode is fixed at
/// construction — each `kebab ingest` invocation is a single mode
/// (chosen from `--json` plus `IsTerminal` detection).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProgressMode {
    /// stdout = line-delimited `ingest_progress.v1`. stderr stays
    /// silent for events (errors / log frames still go to stderr).
    Json,
    /// stdout reserved for the final report; stderr gets an indicatif
    /// `ProgressBar` (TTY) or one short line per event (non-TTY).
    Human { tty: bool, quiet: bool },
}

impl ProgressMode {
    /// Pick the right mode from caller flags.
    ///
    /// - `json`: `--json` flag — takes priority, returns `Json`.
    /// - `quiet`: `--quiet` flag — suppresses human-readable stderr when `Human`.
    /// - `plain_env`: `KEBAB_PROGRESS=plain` — forces `tty=false` even in a TTY,
    ///   for CI environments that emulate a TTY with a pty wrapper.
    pub fn from_flags(json: bool, quiet: bool, plain_env: bool) -> Self {
        if json {
            Self::Json
        } else {
            let tty = !plain_env && std::io::stderr().is_terminal();
            Self::Human { tty, quiet }
        }
    }
}

/// Drains an `mpsc::Receiver<IngestEvent>` until the sender is dropped
/// and renders each event according to `mode`. Construction only —
/// kick off via [`ProgressDisplay::run`].
pub struct ProgressDisplay {
    mode: ProgressMode,
    bar: Option<ProgressBar>,
}

impl ProgressDisplay {
    pub fn new(mode: ProgressMode) -> Self {
        Self { mode, bar: None }
    }

    /// Block until `rx` returns `Err` (sender dropped). Renders one
    /// frame per received event.
    pub fn run(mut self, rx: Receiver<IngestEvent>) -> anyhow::Result<()> {
        while let Ok(event) = rx.recv() {
            self.handle(&event)?;
        }
        if let Some(bar) = self.bar.take() {
            bar.finish_and_clear();
        }
        Ok(())
    }

    fn handle(&mut self, event: &IngestEvent) -> anyhow::Result<()> {
        match self.mode {
            ProgressMode::Json => emit_json(event),
            ProgressMode::Human { tty, quiet } => self.handle_human(event, tty, quiet),
        }
    }

    /// Render an event in human mode. **Best-effort**: every
    /// `writeln!` into stderr swallows IO errors (`let _ = ...`)
    /// because the progress display must not fail the ingest run if
    /// the terminal is closed mid-stream. Likewise the
    /// `self.bar.as_ref()` / `as_mut()` branches treat a missing
    /// bar as silent skip — the bar is initialized lazily in the
    /// `ScanStarted` arm and §2.4a's ordering invariant
    /// (`ScanStarted` < everything else) guarantees it is `Some` by
    /// the time later events arrive.
    fn handle_human(&mut self, event: &IngestEvent, tty: bool, quiet: bool) -> anyhow::Result<()> {
        match event {
            IngestEvent::ScanStarted { root } => {
                let bar = ProgressBar::new_spinner().with_message(format!("scanning {root}"));
                bar.set_draw_target(if tty && !quiet {
                    ProgressDrawTarget::stderr()
                } else {
                    ProgressDrawTarget::hidden()
                });
                if tty && !quiet {
                    bar.enable_steady_tick(std::time::Duration::from_millis(100));
                }
                self.bar = Some(bar);
                if !tty && !quiet {
                    let mut err = std::io::stderr().lock();
                    let _ = writeln!(err, "ingest: scanning {root}…");
                }
            }
            IngestEvent::ScanCompleted { total } => {
                if let Some(bar) = self.bar.as_mut() {
                    bar.disable_steady_tick();
                    bar.set_length(u64::from(*total));
                    bar.set_position(0);
                    bar.set_style(
                        ProgressStyle::with_template(
                            "ingest [{bar:30}] {pos}/{len} {wide_msg}",
                        )
                        .unwrap()
                        .progress_chars("=> "),
                    );
                    bar.set_message("");
                }
                if !tty && !quiet {
                    let mut err = std::io::stderr().lock();
                    let _ = writeln!(err, "ingest: scan complete ({total} assets)");
                }
            }
            IngestEvent::AssetStarted {
                idx,
                total,
                path,
                media,
            } => {
                if let Some(bar) = self.bar.as_ref() {
                    bar.set_message(format!("{media} {path}"));
                }
                if !tty && !quiet {
                    let mut err = std::io::stderr().lock();
                    let _ = writeln!(err, "ingest: {idx}/{total} {media} {path}");
                }
            }
            IngestEvent::AssetFinished { idx, .. } => {
                if let Some(bar) = self.bar.as_ref() {
                    bar.set_position(u64::from(*idx));
                }
            }
            IngestEvent::Completed { counts } => {
                if let Some(bar) = self.bar.take() {
                    bar.finish_and_clear();
                }
                // Always emit summary in both TTY and non-TTY (unless quiet).
                // Bug fix: previously TTY had no summary line after bar.finish_and_clear().
                if !quiet {
                    let mut err = std::io::stderr().lock();
                    let _ = writeln!(
                        err,
                        "ingest: complete (scanned={} new={} updated={} skipped={} errors={})",
                        counts.scanned,
                        counts.new,
                        counts.updated,
                        counts.skipped,
                        counts.errors,
                    );
                }
            }
            IngestEvent::Aborted { counts } => {
                if let Some(bar) = self.bar.take() {
                    bar.abandon_with_message(format!(
                        "aborted at {}/{}",
                        counts.scanned.saturating_sub(counts.errors),
                        counts.scanned
                    ));
                }
                // Bug fix: was unconditional (fired in TTY too).
                // In TTY, bar.abandon_with_message already prints the final state.
                if !tty && !quiet {
                    let mut err = std::io::stderr().lock();
                    let _ = writeln!(
                        err,
                        "ingest: aborted (scanned={} new={} updated={} skipped={} errors={})",
                        counts.scanned,
                        counts.new,
                        counts.updated,
                        counts.skipped,
                        counts.errors,
                    );
                }
            }
        }
        Ok(())
    }
}

/// Serialize an `IngestEvent` as the `ingest_progress.v1` wire shape
/// (kind discriminator + RFC 3339 `ts`) and println to stdout. One
/// event per line.
fn emit_json(event: &IngestEvent) -> anyhow::Result<()> {
    let value = wire::wire_ingest_progress(event)?;
    let line = serde_json::to_string(&value)?;
    let mut out = std::io::stdout().lock();
    writeln!(out, "{line}")?;
    Ok(())
}

/// Format the current wall-clock as RFC 3339 — used by `wire_ingest_progress`
/// so every emitted event carries an `ts` field per §2.4a / the wire schema.
pub(crate) fn now_rfc3339() -> anyhow::Result<String> {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_flags_json_takes_priority_over_tty() {
        assert_eq!(ProgressMode::from_flags(true, false, false), ProgressMode::Json);
    }

    #[test]
    fn from_flags_human_reflects_stderr_tty() {
        // We can't synthesize a TTY in tests, but we can assert the
        // shape — mode is Human { tty: <something> } when --json=false.
        match ProgressMode::from_flags(false, false, false) {
            ProgressMode::Human { .. } => {}
            other => panic!("expected Human mode, got {other:?}"),
        }
    }

    #[test]
    fn from_flags_quiet_sets_quiet_field() {
        match ProgressMode::from_flags(false, true, false) {
            ProgressMode::Human { quiet: true, .. } => {}
            other => panic!("expected Human{{quiet:true}}, got {other:?}"),
        }
    }

    #[test]
    fn from_flags_plain_env_forces_tty_false() {
        match ProgressMode::from_flags(false, false, true) {
            ProgressMode::Human { tty: false, .. } => {}
            other => panic!("expected Human{{tty:false}}, got {other:?}"),
        }
    }

    #[test]
    fn now_rfc3339_parses_back() {
        let s = now_rfc3339().unwrap();
        // Round-trip via the parser to confirm the formatter emits a
        // well-formed RFC 3339 string.
        OffsetDateTime::parse(&s, &Rfc3339).expect("RFC 3339 round-trip");
    }
}
