//! Terminal raw-mode / alternate-screen lifecycle. Critical: the
//! `Drop` impl must restore the terminal even if the run loop panics
//! — otherwise the user is left with a corrupted shell.

use anyhow::{Context, Result};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use std::io::{Stdout, stdout};

pub(crate) struct TuiTerminal {
    pub inner: Terminal<CrosstermBackend<Stdout>>,
}

impl TuiTerminal {
    pub fn enter() -> Result<Self> {
        enable_raw_mode().context("crossterm: enable_raw_mode")?;
        let mut out = stdout();
        execute!(out, EnterAlternateScreen).context("crossterm: EnterAlternateScreen")?;
        let backend = CrosstermBackend::new(stdout());
        let inner = Terminal::new(backend).context("ratatui Terminal::new")?;
        Ok(Self { inner })
    }
}

impl Drop for TuiTerminal {
    fn drop(&mut self) {
        // Best-effort. Errors here would clobber a real panic if we
        // propagated them; just log and let the OS recover any
        // remaining noise.
        let _ = disable_raw_mode();
        let _ = execute!(stdout(), LeaveAlternateScreen);
    }
}
