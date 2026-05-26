//! p9-fb-09: external-program suspend/restore helper.
//!
//! Spawning `$EDITOR` (or any other foreground child) from the TUI
//! requires a careful dance: leave the alternate screen, drop raw
//! mode, hand the terminal to the child, then on return re-enter the
//! alternate screen, re-enable raw mode, AND clear the framebuffer so
//! Ratatui's next draw doesn't paint on top of stale text from before
//! the suspension.
//!
//! Earlier `kebab-tui::search::jump_to_citation` did the suspend half
//! correctly via a RAII guard but skipped the post-resume `clear()` —
//! the frame from before the editor stayed visible underneath the new
//! draw, producing the "TUI 화면이 깨짐" report (도그푸딩 item 7).
//!
//! `with_external_program` centralizes the dance so any future call
//! site (citation jump, `$VISUAL` invocation, etc.) inherits the fix
//! automatically. Callers pass the `Command` (already configured) and
//! get back the child's `ExitStatus` if the spawn succeeded.

use std::process::{Command, ExitStatus};

use anyhow::{Context, Result};
use crossterm::cursor::{Hide, Show};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};

use crate::terminal::TuiTerminal;

/// Suspend the TUI (leave alt screen, drop raw mode, show cursor),
/// run `cmd` to completion in the host terminal, then restore the
/// TUI (re-enter alt screen, re-enable raw mode, hide cursor) and
/// `clear()` the framebuffer so the next `draw` repaints from a
/// blank canvas instead of layering on top of stale glyphs.
///
/// The restore happens via a RAII guard so a panic inside the child
/// spawn (or in this function before the explicit restore) still
/// puts the terminal back into raw + alternate-screen mode — the
/// shell would otherwise be left in a corrupt state.
///
/// On success, returns the child's `ExitStatus`. The caller decides
/// whether a non-zero exit is an error (editor was cancelled vs.
/// crashed) — this helper only fails if the spawn itself fails.
pub(crate) fn with_external_program(
    terminal: &mut TuiTerminal,
    mut cmd: Command,
) -> Result<ExitStatus> {
    suspend_tui()?;

    // RAII guard: regardless of how we leave (panic, error, normal
    // return) the terminal goes back into raw + alt-screen mode and
    // the framebuffer is cleared.
    struct Restore<'a> {
        terminal: &'a mut TuiTerminal,
    }
    impl Drop for Restore<'_> {
        fn drop(&mut self) {
            // Best-effort: errors here would clobber an in-flight
            // panic if propagated. Match the conservative posture in
            // `TuiTerminal::Drop` — log via `tracing` and continue.
            if let Err(e) = resume_tui(self.terminal) {
                tracing::error!(target: "kebab-tui", error = ?e, "TUI restore failed");
            }
        }
    }
    let restore = Restore { terminal };

    let status = cmd
        .status()
        .with_context(|| format!("spawn child program: {:?}", cmd.get_program()))?;

    drop(restore);
    Ok(status)
}

/// Leave the alternate screen, disable raw mode, and show the cursor
/// so a child process inherits a "normal" terminal.
fn suspend_tui() -> Result<()> {
    let mut out = std::io::stdout();
    execute!(out, LeaveAlternateScreen, Show).context("crossterm: LeaveAlternateScreen + Show")?;
    disable_raw_mode().context("crossterm: disable_raw_mode")?;
    Ok(())
}

/// Re-enter the alternate screen, re-enable raw mode, hide the
/// cursor, and `terminal.clear()` so Ratatui draws a fresh frame
/// without inheriting whatever was on screen before the suspension.
fn resume_tui(terminal: &mut TuiTerminal) -> Result<()> {
    enable_raw_mode().context("crossterm: enable_raw_mode")?;
    let mut out = std::io::stdout();
    execute!(out, EnterAlternateScreen, Hide).context("crossterm: EnterAlternateScreen + Hide")?;
    terminal
        .inner
        .clear()
        .context("ratatui: terminal.clear after editor return")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    /// Sanity check on the OS layer that `with_external_program`
    /// builds on top of: a missing program path makes `Command::
    /// status()` fail with `ENOENT`, which the helper wraps with
    /// `with_context(|| format!("spawn child program: {:?}", ...))`
    /// so the error chain points at the program name.
    ///
    /// We can't construct a `TuiTerminal` in a unit test (no real
    /// terminal), so the helper end-to-end is verified by the
    /// dogfooding loop in the spec rather than here. This test
    /// only pins the OS behavior the helper assumes — if a future
    /// libc / Rust update changes which `ErrorKind` is returned for
    /// `ENOENT`, the helper's error message stays meaningful but
    /// this test catches the platform regression first.
    #[test]
    fn command_status_returns_not_found_for_missing_program() {
        let mut cmd = Command::new("/nonexistent/kebab-test-binary-xxx");
        cmd.arg("dummy-arg");
        let result = cmd.status();
        assert!(result.is_err(), "expected ENOENT-like failure");
        let err = result.unwrap_err();
        assert!(
            matches!(
                err.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied
            ),
            "unexpected error kind: {err:?}",
        );
    }
}
