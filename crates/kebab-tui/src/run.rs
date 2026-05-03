//! Run loop — owns the event poll + render cycle. Pane-specific
//! key handlers are dispatched on focus.

use anyhow::Result;
use crossterm::event::{self, Event, KeyEventKind};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use std::time::Duration;

use crate::app::{App, AskState, InspectState, KeyOutcome, Pane, SearchState};
use crate::ask::{drain_stream, handle_key_ask, poll_worker, render_ask};
use crate::error_popup::{ErrorOverlay, render_error_overlay};
use crate::inspect::{handle_key_inspect, refresh_inspect, render_inspect};
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
        // p9-fb-03: ingest progress is pane-independent. Drain
        // freshly-arrived events every tick + clear the slot a few
        // seconds after the run terminated so the user has time to
        // read the final line.
        crate::ingest_progress::drain_progress(app);
        let clear_now = app
            .ingest_state
            .as_ref()
            .map(crate::ingest_progress::ready_to_clear)
            .unwrap_or(false);
        if clear_now {
            if let Some(mut state) = app.ingest_state.take() {
                // Reap the worker thread now that the user has seen
                // the final status line; ignore the join result —
                // `IngestReport` was already mirrored into the status
                // bar via `Completed { counts }`.
                if let Some(handle) = state.thread.take() {
                    let _ = handle.join();
                }
            }
            // Library may show stale doc list; queue a refresh so the
            // next idle tick picks up the just-ingested rows.
            app.library.inner.needs_refresh = true;
        }

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
                    // p9-fb-08: drain the async search worker first.
                    // Stale generations are silently dropped; the
                    // current generation's result populates `hits`
                    // / clears `searching` here.
                    crate::search::poll_worker(app);
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
                Pane::Inspect => {
                    let due = app
                        .inspect
                        .as_ref()
                        .map(|s| s.needs_fetch)
                        .unwrap_or(false);
                    if due {
                        if let Err(e) = refresh_inspect(app) {
                            app.error_overlay = Some(ErrorOverlay::from_anyhow(&e));
                        }
                    }
                }
                _ => {}
            }
        }

        // p9-fb-09: any code path (editor return, future reset
        // helper, …) that toggled `force_redraw` gets a fresh
        // framebuffer for this draw — without it, residual content
        // from before the suspension would layer through Ratatui's
        // diff and produce a corrupted-looking screen.
        if app.force_redraw {
            terminal.inner.clear()?;
            app.force_redraw = false;
        }

        terminal.inner.draw(|f| render_root(f, app))?;

        if event::poll(POLL_INTERVAL)? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    let outcome = match app.focus {
                        Pane::Library => handle_key_library(app, key),
                        Pane::Search => handle_key_search(app, key),
                        Pane::Ask => handle_key_ask(app, key),
                        Pane::Inspect => handle_key_inspect(app, key),
                        // p9-5 (Jobs) plugs its handler here when it
                        // lands. Until then, accepts only `q` / `Esc`.
                        Pane::Jobs => handle_key_unimplemented_pane(app, key),
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
                            if p == Pane::Inspect && app.inspect.is_none() {
                                app.inspect = Some(InspectState::default());
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

        // p9-fb-09: drain any pending external-program request that
        // a key handler enqueued. The actual suspend / spawn /
        // restore needs the `TuiTerminal` handle, which is only in
        // scope here. After return, `force_redraw` is set so the
        // next iteration's draw paints from a clean canvas.
        if let Some(req) = app.pending_editor.take() {
            let result = crate::search::jump_to_citation(
                &mut terminal,
                &req.citation,
                &req.editor_env,
                &req.workspace_root,
            );
            app.force_redraw = true;
            if let Err(e) = result {
                app.error_overlay = Some(ErrorOverlay::from_anyhow(&e));
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
    // p9-fb-03: insert a 1-line status bar above the footer when an
    // ingest is in flight (or its terminal line is still on hold).
    let has_ingest = app.ingest_state.is_some();
    let constraints: Vec<Constraint> = if has_ingest {
        vec![
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1), // ingest status bar
            Constraint::Length(1), // existing footer hints
        ]
    } else {
        vec![
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ]
    };
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(f.area());
    render_header(f, outer[0], app);
    match app.focus {
        Pane::Library => render_library(f, outer[1], app),
        Pane::Search => render_search(f, outer[1], app),
        Pane::Ask => render_ask(f, outer[1], app),
        Pane::Inspect => render_inspect(f, outer[1], app),
        // p9-5 Jobs not yet rendered; Library placeholder.
        Pane::Jobs => render_library(f, outer[1], app),
    }
    if has_ingest {
        render_ingest_status(f, outer[2], app);
        render_footer(f, outer[3], app);
    } else {
        render_footer(f, outer[2], app);
    }
    if let Some(err) = &app.error_overlay {
        render_error_overlay(f, f.area(), err, &app.theme);
    }
}

fn render_ingest_status(f: &mut Frame, area: Rect, app: &App) {
    let Some(state) = app.ingest_state.as_ref() else {
        return;
    };
    let line = crate::ingest_progress::status_line(state);
    // p9-fb-14: `aborted` is a non-fatal-but-noteworthy state (Ctrl-C
    // partial commit) — `Role::Warning` (yellow) is the right semantic
    // signal, plus an explicit BOLD so the abort line still stands
    // out from the live progress lines around it.
    let style = if state.aborted {
        app.theme
            .style(crate::theme::Role::Warning)
            .add_modifier(ratatui::style::Modifier::BOLD)
    } else {
        app.theme.style(crate::theme::Role::Body)
    };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(line, style))),
        area,
    );
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
        Span::styled("kebab", app.theme.style(crate::theme::Role::Title)),
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
                "j/k=move  gg=top  G=bottom  f=filter  /=search  ?=ask  Enter=inspect  r=ingest  q=quit"
            }
        }
        Pane::Search => "type=query  Tab=mode  Enter=search  j/k=move  g=open in $EDITOR  Esc=back",
        Pane::Ask => "type=question  Enter=submit  e=explain (when input empty)  j/k=scroll (when input empty)  Esc=back",
        Pane::Inspect => "j/k=scroll  PgUp/PgDn=page scroll  c=collapse/expand sections  Esc/q=back",
        Pane::Jobs => "Jobs pane not yet implemented — q to return",
    };
    let line = Line::from(Span::styled(
        hints,
        app.theme.style(crate::theme::Role::Hint),
    ));
    f.render_widget(
        Paragraph::new(line).block(Block::default().borders(Borders::TOP)),
        area,
    );
}
