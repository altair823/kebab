//! Run loop — owns the event poll + render cycle. Pane-specific
//! key handlers are dispatched on focus.

use anyhow::Result;
use crossterm::event::{self, Event, KeyEventKind};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use std::time::Duration;

use crate::app::{App, AskState, KeyOutcome, Pane, SearchState};
use crate::ask::{drain_stream, handle_key_ask, poll_worker, render_ask};
use crate::error_popup::{ErrorOverlay, render_error_overlay};
use crate::library::{handle_key_library, refresh_docs, render_library};
use crate::search::{
    debounce_due, fire_search, handle_key_search, refresh_preview, render_search,
};
use crate::terminal::TuiTerminal;

/// Poll interval for crossterm's `event::poll`. Short enough that a
/// pending data refresh shows up promptly, long enough that an idle
/// app doesn't spin the CPU.
const POLL_INTERVAL: Duration = Duration::from_millis(150);

pub(crate) fn run_loop(app: &mut App) -> Result<()> {
    let mut terminal = TuiTerminal::enter()?;

    while !app.should_quit {
        // Per-pane idle work BEFORE rendering so the frame reflects
        // freshly-loaded state.
        if app.error_overlay.is_none() {
            match app.focus {
                Pane::Library => {
                    if app.library.inner.needs_refresh {
                        if let Err(e) = refresh_docs(app) {
                            app.error_overlay = Some(ErrorOverlay::from_anyhow(&e));
                        }
                    }
                }
                Pane::Search => {
                    let due = app
                        .search
                        .as_ref()
                        .map(debounce_due)
                        .unwrap_or(false);
                    if due {
                        if let Err(e) = fire_search(app) {
                            app.error_overlay = Some(ErrorOverlay::from_anyhow(&e));
                        }
                    }
                    // Lazy preview fetch when selection lacks one.
                    let needs_preview = app
                        .search
                        .as_ref()
                        .map(|s| s.preview.is_none() && !s.hits.is_empty())
                        .unwrap_or(false);
                    if needs_preview {
                        if let Err(e) = refresh_preview(app) {
                            app.error_overlay = Some(ErrorOverlay::from_anyhow(&e));
                        }
                    }
                }
                Pane::Ask => {
                    // Token stream + worker completion polled every
                    // tick so the answer area updates without
                    // blocking the event loop.
                    drain_stream(app);
                    poll_worker(app);
                }
                _ => {}
            }
        }

        terminal.inner.draw(|f| render_root(f, app))?;

        if event::poll(POLL_INTERVAL)? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    let outcome = match app.focus {
                        Pane::Library => handle_key_library(app, key),
                        Pane::Search => handle_key_search(app, key),
                        Pane::Ask => handle_key_ask(app, key),
                        // p9-4/5 plug their handlers here as their
                        // crates land. Until then, those panes accept
                        // only `q` / `Esc` to return.
                        Pane::Inspect | Pane::Jobs => {
                            handle_key_unimplemented_pane(app, key)
                        }
                    };
                    match outcome {
                        KeyOutcome::Quit => app.should_quit = true,
                        KeyOutcome::SwitchPane(p) => {
                            app.focus = p;
                            // Lazy-init pane state on first switch.
                            if p == Pane::Search && app.search.is_none() {
                                app.search = Some(SearchState::default());
                            }
                            if p == Pane::Ask && app.ask.is_none() {
                                app.ask = Some(AskState::default());
                            }
                        }
                        KeyOutcome::Refresh => {
                            // Library uses needs_refresh; Search uses
                            // input_dirty_at — pane-specific. The next
                            // loop iteration's idle pass services it.
                        }
                        KeyOutcome::Continue => {}
                    }
                }
                _ => {}
            }
        }
    }

    Ok(())
}

/// Stub key handler for panes whose authoring task has not landed
/// yet. `q` / `Esc` returns to Library; everything else is a no-op.
fn handle_key_unimplemented_pane(
    app: &mut App,
    key: crossterm::event::KeyEvent,
) -> KeyOutcome {
    use crossterm::event::KeyCode;
    if app.error_overlay.is_some() {
        app.error_overlay = None;
        return KeyOutcome::Continue;
    }
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => KeyOutcome::SwitchPane(Pane::Library),
        _ => KeyOutcome::Continue,
    }
}

fn render_root(f: &mut Frame, app: &App) {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(f.area());
    render_header(f, outer[0], app);
    match app.focus {
        Pane::Library => render_library(f, outer[1], app),
        Pane::Search => render_search(f, outer[1], app),
        Pane::Ask => render_ask(f, outer[1], app),
        // p9-4/5 panes (Inspect / Jobs) not yet rendered; placeholder
        // is the Library frame — focus state header still reads
        // "Inspect" / "Jobs" so the user is not misled.
        _ => render_library(f, outer[1], app),
    }
    render_footer(f, outer[2], app);
    if let Some(err) = &app.error_overlay {
        render_error_overlay(f, f.area(), err);
    }
}

fn render_header(f: &mut Frame, area: Rect, app: &App) {
    let pane_label = match app.focus {
        Pane::Library => "Library",
        Pane::Search => "Search",
        Pane::Ask => "Ask",
        Pane::Inspect => "Inspect",
        Pane::Jobs => "Jobs",
    };
    let line = Line::from(vec![
        Span::styled(
            "kebab",
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::raw(" / "),
        Span::raw(pane_label),
    ]);
    f.render_widget(Paragraph::new(line), area);
}

fn render_footer(f: &mut Frame, area: Rect, app: &App) {
    let hints = match app.focus {
        Pane::Library => {
            if app.library.inner.filter_edit.is_some() {
                "Tab=field  Enter=apply  Esc=cancel"
            } else {
                "j/k=move  gg=top  G=bottom  f=filter  /=search  ?=ask  Enter=inspect  q=quit"
            }
        }
        Pane::Search => "type=query  Tab=mode  Enter=search  j/k=move  g=open in $EDITOR  Esc=back",
        Pane::Ask => "type=question  Enter=submit  e=explain (when input empty)  j/k=scroll (when input empty)  Esc=back",
        Pane::Inspect => "Inspect pane not yet implemented (lands with p9-4) — q to return",
        Pane::Jobs => "Jobs pane not yet implemented — q to return",
    };
    let line = Line::from(Span::styled(
        hints,
        Style::default().add_modifier(Modifier::DIM),
    ));
    f.render_widget(
        Paragraph::new(line).block(Block::default().borders(Borders::TOP)),
        area,
    );
}
