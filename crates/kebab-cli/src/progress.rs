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
//! `Sender` end is dropped (i.e. when `ingest_with_config`
//! returns).

use std::collections::HashMap;
use std::io::{IsTerminal, Write};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use indicatif::{ProgressBar, ProgressDrawTarget, ProgressState, ProgressStyle};
use kebab_app::IngestEvent;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::wire;

/// v0.26.1: number of slowest assets surfaced in the end-of-run summary.
/// Constant for now (spec defers the config knob).
const SLOWEST_TOP_N: usize = 5;

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
    /// v0.26.1 heartbeat: start `Instant` of the asset currently in
    /// flight, shared with the bar's steady-tick custom template key so
    /// the `(Ns)` elapsed counter advances *between* events (the drain
    /// loop blocks on `recv()`, so without the ticker the counter would
    /// freeze). `None` while scanning / between assets / after completion.
    asset_start: Arc<Mutex<Option<Instant>>>,
    /// v0.26.1: workspace path of the asset currently in flight — set on
    /// `AssetStarted`, reused by `AssetPhase` to render `{path} · {phase}…`.
    current_path: Option<String>,
    /// v0.26.1 slowest summary: idx → path, captured from `AssetStarted`
    /// so `AssetTimings` (which only carries `idx`) can name the asset.
    asset_paths: HashMap<u32, String>,
    /// v0.26.1 slowest summary: (path, total_ms) per asset that reported
    /// `AssetTimings`. Sorted + truncated to top-N on `Completed`.
    timings: Vec<(String, u64)>,
}

impl ProgressDisplay {
    pub fn new(mode: ProgressMode) -> Self {
        Self {
            mode,
            bar: None,
            asset_start: Arc::new(Mutex::new(None)),
            current_path: None,
            asset_paths: HashMap::new(),
            timings: Vec::new(),
        }
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
                    bar.set_length(u64::from(*total));
                    bar.set_position(0);
                    // v0.26.1: a custom `{asset_elapsed}` key reads the shared
                    // per-asset start `Instant` and appends ` (Ns)`. Combined
                    // with the steady tick below, the elapsed counter advances
                    // even while the drain loop is blocked on `recv()` waiting
                    // for the next (possibly very slow) phase event.
                    let asset_start = Arc::clone(&self.asset_start);
                    bar.set_style(
                        ProgressStyle::with_template(
                            "ingest [{bar:30}] {pos}/{len} {wide_msg}{asset_elapsed}",
                        )
                        .unwrap()
                        .with_key(
                            "asset_elapsed",
                            move |_: &ProgressState, w: &mut dyn std::fmt::Write| {
                                if let Ok(guard) = asset_start.lock()
                                    && let Some(started) = *guard
                                {
                                    let secs = started.elapsed().as_secs();
                                    // Only show once the asset has been running
                                    // a moment — avoids `(0s)` flicker on fast
                                    // assets.
                                    if secs >= 1 {
                                        let _ = write!(w, " ({secs}s)");
                                    }
                                }
                            },
                        )
                        .progress_chars("=> "),
                    );
                    bar.set_message("");
                    if tty && !quiet {
                        bar.enable_steady_tick(std::time::Duration::from_secs(1));
                    } else {
                        bar.disable_steady_tick();
                    }
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
                // v0.26.1: remember the path so AssetPhase can render it and
                // the slowest summary (keyed by idx in AssetTimings) can name
                // the asset.
                self.current_path = Some(path.clone());
                self.asset_paths.insert(*idx, path.clone());
                // v0.26.1: (re)start the per-asset heartbeat clock.
                if let Ok(mut guard) = self.asset_start.lock() {
                    *guard = Some(Instant::now());
                }
                if let Some(bar) = self.bar.as_ref() {
                    bar.set_position(u64::from(idx.saturating_sub(1)));
                    // v0.26.1: show the current filename on the bar (TTY).
                    // Previously position-only — the interactive user couldn't
                    // tell which file was in flight. The steady tick redraws
                    // in place, so this no longer pollutes scrollback.
                    bar.set_message(abbreviate_path(path));
                }
                if !tty && !quiet {
                    let mut err = std::io::stderr().lock();
                    let _ = writeln!(err, "ingest: {idx}/{total} {media} {path}");
                }
            }
            IngestEvent::AssetFinished { .. } => {
                // Position is advanced in AssetStarted; bar.finish_and_clear()
                // in Completed handles the final state. v0.26.1: stop the
                // heartbeat clock so the bar doesn't show a stale `(Ns)` in the
                // gap before the next AssetStarted.
                if let Ok(mut guard) = self.asset_start.lock() {
                    *guard = None;
                }
                self.current_path = None;
            }
            // v0.26.1: an asset entered a slow internal phase (ocr / caption /
            // embed). Surface which phase + model is running so a multi-second
            // vision-model call no longer looks frozen.
            IngestEvent::AssetPhase {
                idx,
                total,
                phase,
                model,
            } => {
                let label = match model {
                    Some(m) => format!("{phase}({m})"),
                    None => phase.clone(),
                };
                if let Some(bar) = self.bar.as_ref() {
                    let path = self.current_path.as_deref().unwrap_or("");
                    bar.set_message(format!("{} · {label}…", abbreviate_path(path)));
                }
                if !tty && !quiet {
                    let mut err = std::io::stderr().lock();
                    let _ = writeln!(err, "ingest: {idx}/{total} · {label}…");
                }
            }
            // v0.24.0: asset-internal phase visibility. AssetChunked uses the
            // bar *message* (live sub-progress for the current asset) —
            // distinct from the per-file position draw, so a single large
            // document no longer looks frozen. AssetTimings prints a one-line
            // breakdown when the asset finishes.
            IngestEvent::AssetChunked { idx, total, chunks } => {
                if let Some(bar) = self.bar.as_ref() {
                    bar.set_message(format!("→ {chunks} chunks"));
                }
                if !tty && !quiet {
                    let mut err = std::io::stderr().lock();
                    let _ = writeln!(err, "ingest: {idx}/{total} → {chunks} chunks");
                }
            }
            IngestEvent::AssetTimings {
                idx,
                parse_ms,
                chunk_ms,
                embed_ms,
                store_ms,
                ocr_ms,
                caption_ms,
                ..
            } => {
                // v0.26.1: accumulate (path, total_ms) for the slowest summary.
                // total = every measured phase (expansion_ms is always 0).
                let total_ms = parse_ms + chunk_ms + embed_ms + store_ms + ocr_ms + caption_ms;
                if let Some(path) = self.asset_paths.get(idx) {
                    self.timings.push((path.clone(), total_ms));
                }
                if let Some(bar) = self.bar.as_ref() {
                    bar.set_message("");
                }
                if !quiet {
                    let mut err = std::io::stderr().lock();
                    // v0.26.1: only print ocr / caption when they actually ran
                    // (markdown leaves them 0) so the text path stays uncluttered.
                    let mut parts = vec![
                        format!("parse {}", fmt_ms(*parse_ms)),
                        format!("chunk {}", fmt_ms(*chunk_ms)),
                    ];
                    if *ocr_ms > 0 {
                        parts.push(format!("ocr {}", fmt_ms(*ocr_ms)));
                    }
                    if *caption_ms > 0 {
                        parts.push(format!("caption {}", fmt_ms(*caption_ms)));
                    }
                    parts.push(format!("embed {}", fmt_ms(*embed_ms)));
                    parts.push(format!("store {}", fmt_ms(*store_ms)));
                    let _ = writeln!(err, "  ⏱ {}", parts.join(" · "));
                }
            }
            IngestEvent::Completed { counts } => {
                if let Some(bar) = self.bar.take() {
                    bar.finish_and_clear();
                }
                if let Ok(mut guard) = self.asset_start.lock() {
                    *guard = None;
                }
                // Always emit summary in both TTY and non-TTY (unless quiet).
                // Bug fix: previously TTY had no summary line after bar.finish_and_clear().
                if !quiet {
                    let mut err = std::io::stderr().lock();
                    let _ = writeln!(
                        err,
                        "ingest: complete (scanned={} new={} updated={} skipped={} errors={})",
                        counts.scanned, counts.new, counts.updated, counts.skipped, counts.errors,
                    );
                    // v0.26.1: slowest-asset summary. Useful in both TTY and
                    // non-TTY (it pinpoints the bottleneck file), so it prints
                    // unless --quiet. --json mode never reaches here (emit_json).
                    let _ = write_slowest_summary(&mut err, &self.timings, SLOWEST_TOP_N);
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
                        counts.scanned, counts.new, counts.updated, counts.skipped, counts.errors,
                    );
                }
            }
            // v0.20.0 sub-item 1: per-page PDF OCR events — sub-progress lines
            // under AssetStarted for scanned PDF. spec §4.6.1 line 1085-1086.
            // skipped=true 시 (DCTDecode 부재 또는 engine fail) skip line.
            IngestEvent::PdfOcrStarted { page } => {
                if !quiet {
                    let mut err = std::io::stderr().lock();
                    let _ = writeln!(err, "  📷 OCR page {page}...");
                }
            }
            IngestEvent::PdfOcrFinished {
                page,
                ms,
                chars,
                ocr_engine,
                skipped,
                ..
            } => {
                if !quiet {
                    let mut err = std::io::stderr().lock();
                    if *skipped {
                        let _ = writeln!(
                            err,
                            "  ⊘ OCR page {page} skipped (no DCTDecode or engine fail, {ms}ms)"
                        );
                    } else {
                        let _ = writeln!(
                            err,
                            "  ✓ OCR page {page} ({chars} chars, {ms}ms via {ocr_engine})"
                        );
                    }
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

/// Render a phase duration (milliseconds) compactly for the human-mode
/// `AssetTimings` line: `< 1000ms` stays in `ms`, larger spans collapse to
/// one-decimal seconds so a 45-second embed reads `45.0s`, not `45000ms`.
fn fmt_ms(ms: u64) -> String {
    if ms >= 1000 {
        format!("{:.1}s", ms as f64 / 1000.0)
    } else {
        format!("{ms}ms")
    }
}

/// v0.26.1: shorten an over-long workspace path for the progress-bar
/// message so the live `(Ns)` heartbeat suffix stays visible on a narrow
/// terminal. Keeps the tail (filename + a couple of parents) — that's the
/// distinguishing part — and prefixes `…` when truncated. Paths up to the
/// budget pass through verbatim.
fn abbreviate_path(path: &str) -> String {
    const MAX: usize = 48;
    let char_count = path.chars().count();
    if char_count <= MAX {
        return path.to_string();
    }
    // Keep the last MAX-1 chars (1 reserved for the leading ellipsis).
    let tail: String = path
        .chars()
        .skip(char_count - (MAX - 1))
        .collect::<String>();
    format!("…{tail}")
}

/// v0.26.1: render the end-of-run "slowest assets" summary. Sorts
/// `(path, total_ms)` descending by time, takes the top `n`, and writes a
/// compact table to `w`. No-op (writes nothing) when `timings` is empty so
/// a run with no per-asset timing (e.g. all-skipped) prints no stray header.
fn write_slowest_summary(
    w: &mut impl Write,
    timings: &[(String, u64)],
    n: usize,
) -> std::io::Result<()> {
    if timings.is_empty() {
        return Ok(());
    }
    let mut sorted: Vec<&(String, u64)> = timings.iter().collect();
    // desc by ms; ties broken by path for deterministic output.
    sorted.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let top = &sorted[..sorted.len().min(n)];
    writeln!(w, "⏱ 최장 소요 top-{}:", top.len())?;
    for (rank, (path, ms)) in top.iter().enumerate() {
        writeln!(w, "  {}. {} — {}", rank + 1, path, fmt_ms(*ms))?;
    }
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
        assert_eq!(
            ProgressMode::from_flags(true, false, false),
            ProgressMode::Json
        );
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
    fn fmt_ms_switches_unit_at_one_second() {
        assert_eq!(fmt_ms(0), "0ms");
        assert_eq!(fmt_ms(999), "999ms");
        assert_eq!(fmt_ms(1000), "1.0s");
        assert_eq!(fmt_ms(45_000), "45.0s");
        assert_eq!(fmt_ms(1500), "1.5s");
    }

    #[test]
    fn now_rfc3339_parses_back() {
        let s = now_rfc3339().unwrap();
        // Round-trip via the parser to confirm the formatter emits a
        // well-formed RFC 3339 string.
        OffsetDateTime::parse(&s, &Rfc3339).expect("RFC 3339 round-trip");
    }

    #[test]
    fn abbreviate_path_passes_short_paths_through() {
        assert_eq!(abbreviate_path("notes/foo.md"), "notes/foo.md");
    }

    #[test]
    fn abbreviate_path_keeps_tail_with_ellipsis() {
        let long = "a/very/deeply/nested/directory/structure/that/exceeds/the/budget/file.md";
        let out = abbreviate_path(long);
        assert!(out.starts_with('…'), "should be prefixed with ellipsis: {out}");
        assert!(out.ends_with("file.md"), "should keep the filename tail: {out}");
        // 48-char budget: 1 ellipsis + 47 tail chars.
        assert_eq!(out.chars().count(), 48);
    }

    #[test]
    fn write_slowest_summary_empty_writes_nothing() {
        let mut buf = Vec::new();
        write_slowest_summary(&mut buf, &[], 5).unwrap();
        assert!(buf.is_empty());
    }

    #[test]
    fn write_slowest_summary_sorts_desc_and_truncates() {
        let timings = vec![
            ("a.md".to_string(), 100),
            ("b.png".to_string(), 5_000),
            ("c.pdf".to_string(), 2_000),
            ("d.md".to_string(), 50),
        ];
        let mut buf = Vec::new();
        write_slowest_summary(&mut buf, &timings, 2).unwrap();
        let out = String::from_utf8(buf).unwrap();
        assert!(out.contains("top-2:"), "{out}");
        // b (5s) ranks first, c (2s) second; a/d excluded.
        let b_pos = out.find("b.png").expect("b.png present");
        let c_pos = out.find("c.pdf").expect("c.pdf present");
        assert!(b_pos < c_pos, "b before c: {out}");
        assert!(!out.contains("a.md"), "a.md excluded by top-2: {out}");
        assert!(out.contains("5.0s"), "b renders as 5.0s: {out}");
    }

    #[test]
    fn write_slowest_summary_tie_breaks_by_path() {
        let timings = vec![
            ("z.md".to_string(), 1_000),
            ("a.md".to_string(), 1_000),
        ];
        let mut buf = Vec::new();
        write_slowest_summary(&mut buf, &timings, 5).unwrap();
        let out = String::from_utf8(buf).unwrap();
        assert!(
            out.find("a.md").unwrap() < out.find("z.md").unwrap(),
            "equal ms ties break alphabetically: {out}"
        );
    }
}
