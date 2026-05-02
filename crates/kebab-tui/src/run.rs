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

use crate::app::{App, KeyOutcome, Pane};
use crate::error_popup::{ErrorOverlay, render_error_overlay};
use crate::library::{handle_key_library, refresh_docs, render_library};
use crate::terminal::TuiTerminal;

/// Poll interval for crossterm's `event::poll`. Short enough that a
/// pending data refresh shows up promptly, long enough that an idle
/// app doesn't spin the CPU.
const POLL_INTERVAL: Duration = Duration::from_millis(150);

pub(crate) fn run_loop(app: &mut App) -> Result<()> {
    let mut terminal = TuiTerminal::enter()?;

    while !app.should_quit {
        if app.library.inner.needs_refresh
            && app.focus == Pane::Library
            && app.error_overlay.is_none()
        {
            if let Err(e) = refresh_docs(app) {
                app.error_overlay = Some(ErrorOverlay::from_anyhow(&e));
            }
        }

        terminal.inner.draw(|f| render_root(f, app))?;

        if event::poll(POLL_INTERVAL)? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    let outcome = match app.focus {
                        Pane::Library => handle_key_library(app, key),
                        // p9-2/3/4 plug their handlers here as their
                        // crates land. Until then, the non-Library
                        // panes accept only `q` / `Esc` to return —
                        // anything else is a no-op. The footer hint
                        // tells the user the pane is unimplemented.
                        Pane::Search | Pane::Ask | Pane::Inspect | Pane::Jobs => {
                            handle_key_unimplemented_pane(app, key)
                        }
                    };
                    match outcome {
                        KeyOutcome::Quit => app.should_quit = true,
                        KeyOutcome::SwitchPane(p) => app.focus = p,
                        KeyOutcome::Refresh => {
                            // `needs_refresh` was already set by the
                            // pane handler; the next loop iteration
                            // services it.
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
/// Does NOT delegate to `handle_key_library` because that would let
/// `j` / `k` / `f` mutate Library state while focus says otherwise —
/// confusing UX.
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
        // Until p9-2/3/4 land, the run loop never actually moves
        // focus to those panes; render_library serves as a safe
        // placeholder.
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
    // p9-2/3/4 가 머지되기 전에는 SwitchPane(Search/Ask/Inspect) 가
    // focus 만 바꾸고 본문은 Library 가 그려지는 절뚝거림이 사용자에게
    // 보임. footer 에서 \"미구현\" 을 명시해 거짓말 안 함.
    let hints = match app.focus {
        Pane::Library => {
            if app.library.inner.filter_edit.is_some() {
                "Tab=field  Enter=apply  Esc=cancel"
            } else {
                "j/k=move  gg=top  G=bottom  f=filter  /=search  ?=ask  Enter=inspect  q=quit"
            }
        }
        Pane::Search => "Search pane not yet implemented (lands with p9-2) — q to return",
        Pane::Ask => "Ask pane not yet implemented (lands with p9-3) — q to return",
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
