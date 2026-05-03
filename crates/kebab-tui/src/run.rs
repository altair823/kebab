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
                    // p9-fb-13: cheatsheet popup toggle takes
                    // precedence over both mode + pane dispatch.
                    // F1 toggles open/close. While visible, Esc
                    // also closes — and the rest of the dispatch
                    // is skipped so `Esc` doesn't double as
                    // "Insert→Normal" while the user is reading
                    // the cheatsheet.
                    if cheatsheet_intercept(app, key) {
                        continue;
                    }
                    // p9-fb-12: global mode toggle. `Esc` from
                    // Insert → Normal is intercepted here so it
                    // works on every pane uniformly. `i` from
                    // Normal → Insert is also intercepted, but
                    // ONLY on Library/Inspect (where `i` has no
                    // pre-fb-12 meaning); on Search/Ask the user
                    // is already in Insert by Mode::auto_for, so
                    // `i` falls through as a typed character.
                    if mode_intercept(app, key) {
                        continue;
                    }
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
                            // p9-fb-12: auto-flip mode on switch.
                            // Library/Inspect/Jobs → Normal,
                            // Search/Ask → Insert. User can still
                            // press i/Esc to override.
                            app.mode = crate::app::Mode::auto_for(p);
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
    // p9-fb-13: cheatsheet sits on top of the error overlay so the
    // user can summon help even mid-error (the cheatsheet's own
    // Esc/F1 close still works first; the next key reaches the
    // error-dismiss path).
    if app.cheatsheet_visible {
        crate::cheatsheet::render_cheatsheet(f, f.area(), app);
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
    // p9-fb-12: mode label colored — Insert = Success (green), Normal
    // = Heading (cyan + bold). The literal text is the user-visible
    // signal; color is reinforcement (a11y: never color-only).
    let mode_role = match app.mode {
        crate::app::Mode::Insert => crate::theme::Role::Success,
        crate::app::Mode::Normal => crate::theme::Role::Heading,
    };
    let line = Line::from(vec![
        Span::styled("kebab", app.theme.style(crate::theme::Role::Title)),
        Span::raw(" / "),
        Span::raw(pane_label),
        Span::raw("  "),
        Span::styled(app.mode.label(), app.theme.style(mode_role)),
    ]);
    f.render_widget(Paragraph::new(line), area);
}

fn render_footer(f: &mut Frame, area: Rect, app: &App) {
    let hints = footer_hints(app.focus, app.mode, app.library.inner.filter_edit.is_some());
    let line = Line::from(Span::styled(
        hints,
        app.theme.style(crate::theme::Role::Hint),
    ));
    f.render_widget(
        Paragraph::new(line).block(Block::default().borders(Borders::TOP)),
        area,
    );
}

/// p9-fb-13 follow-up: produce the footer hint text for a given
/// `(focus, mode, filter_open)` tuple. Pure function — extracted so
/// integration tests can pin the verb-form fragments per pane×mode
/// without standing up the full render loop.
///
/// Style contract:
/// - **Verb-form Korean fragments** (e.g. `"위로"` not `"=move"`).
///   The original `key=action` form was English-only and read like
///   a dev cheat-sheet, not user help.
/// - **Mode-aware**: NORMAL shows navigation verbs;
///   INSERT shows typing verbs + `Esc 로 NORMAL 모드` reminder.
/// - **Filter overlay** overrides Library hints — short list of the
///   3 keys that work inside the overlay.
/// - **Order**: most-frequent verb first; last fragment is always
///   the way back out (`Esc`/`q`).
pub fn footer_hints(focus: Pane, mode: crate::app::Mode, filter_open: bool) -> &'static str {
    use crate::app::Mode::*;
    match (focus, mode, filter_open) {
        // Library filter overlay — same on both modes (overlay
        // captures every key, mode label irrelevant).
        (Pane::Library, _, true) => "Tab 필드전환  Enter 적용  Esc 취소",
        // Library Normal: full navigation surface.
        (Pane::Library, Normal, false) => "↑/k 위로  ↓/j 아래로  gg 맨위  G 맨아래  f 필터  / 검색  ? 질문  Enter 자세히  r 인덱싱  q 종료",
        // Library Insert: degenerate — nothing types in Library, so
        // tell the user how to get back out.
        (Pane::Library, Insert, false) => "Esc 로 NORMAL 모드",
        // Search Insert: typing the query is the dominant action.
        (Pane::Search, Insert, _) => "타이핑 검색어  Tab 모드전환  Enter 검색  Esc 로 NORMAL 모드 (j/k 이동  i 인스펙트  g 에디터)",
        // Search Normal: navigation + commands.
        (Pane::Search, Normal, _) => "↑/k 위로  ↓/j 아래로  Tab 모드전환  Enter 검색  i 인스펙트  g 에디터  Esc 종료",
        // Ask Insert: typing the question.
        (Pane::Ask, Insert, _) => "타이핑 질문  Enter 전송  Esc 로 NORMAL 모드 (e 상세  j/k 스크롤)",
        // Ask Normal: scroll + toggle.
        (Pane::Ask, Normal, _) => "e 상세설명  ↑/k 위로  ↓/j 아래로  Enter 전송  Ctrl-L 새대화  Esc 종료",
        // Inspect Normal (default): scroll + collapse.
        (Pane::Inspect, Normal, _) => "↑/k 위로  ↓/j 아래로  PgUp/PgDn 페이지  c 섹션접기  Esc/q 뒤로",
        // Inspect Insert: degenerate.
        (Pane::Inspect, Insert, _) => "Esc 로 NORMAL 모드",
        // Jobs pane: placeholder.
        (Pane::Jobs, _, _) => "Jobs pane 미구현 — q 로 복귀",
    }
}

/// p9-fb-12: global mode toggle interception. Returns `true` when
/// the key was consumed (caller should `continue` and skip pane
/// dispatch); `false` when the key should fall through to the
/// active pane's handler.
///
/// Rules:
/// - **`Esc` in Insert mode** → flip to Normal. Consumed (do NOT
///   forward as a back-out signal to the pane). Library/Inspect
///   start in Normal so this is a no-op there.
/// - **`i` in Normal mode on Library / Inspect / Jobs** → flip to
///   Insert. Consumed. (`i` has no pre-fb-12 meaning on these
///   panes; on Search/Ask the pane is already Insert by
///   `Mode::auto_for`, so the global `i` interception would
///   swallow what should be a typed character. We let `i` fall
///   through there.)
/// - Everything else → not consumed.
///
/// `pub` so integration tests + future TUI consumers can drive the
/// intercept paths by constructing KeyEvents directly without
/// standing up the full run loop.
pub fn mode_intercept(app: &mut crate::app::App, key: crossterm::event::KeyEvent) -> bool {
    use crossterm::event::{KeyCode, KeyModifiers};
    use crate::app::{Mode, Pane};

    // Modifier-bearing keys (Ctrl-Esc etc.) are not the toggle.
    if !key.modifiers.is_empty() && key.modifiers != KeyModifiers::SHIFT {
        return false;
    }
    match (key.code, app.mode, app.focus) {
        (KeyCode::Esc, Mode::Insert, _) => {
            app.mode = Mode::Normal;
            true
        }
        (KeyCode::Char('i'), Mode::Normal, Pane::Library | Pane::Inspect | Pane::Jobs) => {
            app.mode = Mode::Insert;
            true
        }
        _ => false,
    }
}

/// p9-fb-13: cheatsheet popup interception. Returns `true` when
/// consumed. Rules:
/// - **`F1`** → toggle visibility (open if closed, close if open).
///   Modifier-bearing variants (Ctrl-F1 etc.) are NOT the trigger.
/// - **`Esc` while visible** → close. Returning `true` here means
///   the global `mode_intercept` does NOT also see the Esc, so the
///   user's "close cheatsheet" action stays a single keystroke
///   instead of also flipping mode. **Trade-off**: a user in
///   Insert mode with the cheatsheet open needs a SECOND `Esc` to
///   flip to Normal. Single-effect-per-keystroke wins over
///   compound actions.
/// - Any other key while visible → fall through (so the key reaches
///   the active pane normally — useful if the user wants to keep
///   the popup open and still navigate). The popup auto-closes
///   only via F1 / Esc.
///
/// `pub` so integration tests can drive without standing up the
/// full run loop.
pub fn cheatsheet_intercept(app: &mut crate::app::App, key: crossterm::event::KeyEvent) -> bool {
    use crossterm::event::{KeyCode, KeyModifiers};
    let plain_or_shift =
        key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT;
    if !plain_or_shift {
        return false;
    }
    match key.code {
        KeyCode::F(1) => {
            app.cheatsheet_visible = !app.cheatsheet_visible;
            true
        }
        KeyCode::Esc if app.cheatsheet_visible => {
            app.cheatsheet_visible = false;
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod footer_hints_tests {
    use super::*;
    use crate::app::Mode;

    /// p9-fb-13 follow-up: Library Normal hint includes nav verbs in
    /// Korean and ends with the quit shortcut.
    #[test]
    fn library_normal_hint_uses_korean_verb_fragments() {
        let h = footer_hints(Pane::Library, Mode::Normal, false);
        assert!(h.contains("위로"), "expected 위로 verb: {h}");
        assert!(h.contains("아래로"), "expected 아래로 verb: {h}");
        assert!(h.contains("필터"), "expected 필터 verb: {h}");
        assert!(h.ends_with("q 종료"), "expected q 종료 last: {h}");
    }

    /// p9-fb-13 follow-up: Library filter overlay overrides the
    /// usual hint with the 3 keys that actually work in the overlay.
    #[test]
    fn library_filter_overlay_hint_lists_overlay_keys_only() {
        let h = footer_hints(Pane::Library, Mode::Normal, true);
        assert_eq!(h, "Tab 필드전환  Enter 적용  Esc 취소");
    }

    /// p9-fb-13 follow-up: Insert mode reminds user how to leave —
    /// this is the most common confusion point per the dogfooding
    /// feedback.
    #[test]
    fn insert_mode_hint_mentions_esc_to_normal() {
        for pane in [Pane::Library, Pane::Search, Pane::Ask, Pane::Inspect] {
            let h = footer_hints(pane, Mode::Insert, false);
            assert!(
                h.contains("Esc") && h.contains("NORMAL"),
                "{pane:?} insert hint must mention Esc + NORMAL: {h}"
            );
        }
    }

    /// p9-fb-13 follow-up: Search Insert hint leads with the typing
    /// verb (the dominant action) and lists the NORMAL-only commands
    /// in parentheses so the user knows they're gated.
    #[test]
    fn search_insert_hint_leads_with_typing_verb() {
        let h = footer_hints(Pane::Search, Mode::Insert, false);
        assert!(h.starts_with("타이핑 검색어"), "should lead with 타이핑: {h}");
        assert!(h.contains("Tab 모드전환"), "expected Tab 모드전환: {h}");
        assert!(h.contains("Enter 검색"), "expected Enter 검색: {h}");
    }

    /// p9-fb-13 follow-up: Ask Insert hint leads with typing.
    #[test]
    fn ask_insert_hint_leads_with_typing_verb() {
        let h = footer_hints(Pane::Ask, Mode::Insert, false);
        assert!(h.starts_with("타이핑 질문"), "should lead with 타이핑: {h}");
        assert!(h.contains("Enter 전송"), "expected Enter 전송: {h}");
    }

    /// p9-fb-13 follow-up: Inspect Normal hint covers scroll +
    /// collapse + back-out.
    #[test]
    fn inspect_normal_hint_covers_scroll_collapse_back() {
        let h = footer_hints(Pane::Inspect, Mode::Normal, false);
        assert!(h.contains("위로"), "expected 위로 verb: {h}");
        assert!(h.contains("페이지"), "expected 페이지 verb: {h}");
        assert!(h.contains("섹션접기"), "expected 섹션접기 verb: {h}");
        assert!(h.contains("뒤로"), "expected 뒤로 verb: {h}");
    }

    /// p9-fb-13 follow-up: Search Normal hint enables j/k/i/g as
    /// commands (no parens — they're first-class in Normal mode).
    #[test]
    fn search_normal_hint_lists_commands_directly() {
        let h = footer_hints(Pane::Search, Mode::Normal, false);
        assert!(h.contains("위로"), "expected 위로 verb: {h}");
        assert!(h.contains("Tab 모드전환"), "expected Tab 모드전환: {h}");
        assert!(h.contains("i 인스펙트"), "expected i 인스펙트: {h}");
        assert!(h.contains("g 에디터"), "expected g 에디터: {h}");
    }

    /// p9-fb-13 follow-up: every (pane, mode, filter_open) tuple
    /// returns a non-empty hint — exhaustive sanity that the match
    /// covers every arm.
    #[test]
    fn every_pane_mode_combo_returns_non_empty_hint() {
        for pane in [Pane::Library, Pane::Search, Pane::Ask, Pane::Inspect, Pane::Jobs] {
            for mode in [Mode::Normal, Mode::Insert] {
                for filter_open in [false, true] {
                    let h = footer_hints(pane, mode, filter_open);
                    assert!(!h.is_empty(), "{pane:?}/{mode:?}/filter={filter_open} empty");
                }
            }
        }
    }
}
