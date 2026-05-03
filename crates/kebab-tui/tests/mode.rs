//! p9-fb-12: integration tests for `mode_intercept`. Drives the
//! global i/Esc dispatch by constructing KeyEvents directly without
//! standing up the full run loop (terminal-side).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use kebab_config::Config;
use kebab_tui::{App, Mode, Pane, mode_intercept};

fn fresh_app(focus: Pane) -> App {
    let mut config = Config::defaults();
    config.storage.data_dir = "/tmp/kebab-tui-mode-tests-noop".to_string();
    config.workspace.root = "/tmp/kebab-tui-mode-tests-noop/workspace".to_string();
    let mut app = App::new(config).expect("App::new");
    app.focus = focus;
    app.mode = Mode::auto_for(focus);
    app
}

/// p9-fb-12: `Esc` from Insert mode flips to Normal on any pane.
/// Returns `true` (consumed) so the pane handler doesn't ALSO see
/// the Esc as a "back to Library" signal.
#[test]
fn esc_in_insert_flips_to_normal_and_consumes() {
    for &pane in &[Pane::Library, Pane::Search, Pane::Ask, Pane::Inspect] {
        let mut app = fresh_app(pane);
        app.mode = Mode::Insert;
        let consumed = mode_intercept(
            &mut app,
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
        );
        assert!(consumed, "Esc in Insert must be consumed (pane: {pane:?})");
        assert_eq!(app.mode, Mode::Normal, "mode flipped to Normal (pane: {pane:?})");
    }
}

/// p9-fb-12: `Esc` from Normal mode is a no-op (not consumed) so the
/// pane's existing Esc handler (e.g. Library `Esc` → quit) keeps
/// working.
#[test]
fn esc_in_normal_mode_falls_through() {
    let mut app = fresh_app(Pane::Library);
    assert_eq!(app.mode, Mode::Normal);
    let consumed = mode_intercept(
        &mut app,
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
    );
    assert!(!consumed, "Esc in Normal must fall through to pane");
    assert_eq!(app.mode, Mode::Normal, "mode unchanged");
}

/// p9-fb-12: `i` in Normal mode on Library / Inspect / Jobs flips
/// to Insert. (`i` has no pre-fb-12 meaning on those panes, so the
/// global interception is safe.)
#[test]
fn i_in_normal_on_library_inspect_jobs_flips_to_insert() {
    for &pane in &[Pane::Library, Pane::Inspect, Pane::Jobs] {
        let mut app = fresh_app(pane);
        assert_eq!(app.mode, Mode::Normal, "auto_for({pane:?}) should be Normal");
        let consumed = mode_intercept(
            &mut app,
            KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE),
        );
        assert!(consumed, "i in Normal on {pane:?} must be consumed");
        assert_eq!(app.mode, Mode::Insert, "mode flipped to Insert (pane: {pane:?})");
    }
}

/// p9-fb-12: `i` on Search / Ask falls through (the pane is already
/// in Insert via Mode::auto_for, so the global `i` interception
/// would swallow what should be a typed character).
#[test]
fn i_on_search_or_ask_falls_through_to_pane() {
    for &pane in &[Pane::Search, Pane::Ask] {
        let mut app = fresh_app(pane);
        assert_eq!(app.mode, Mode::Insert, "auto_for({pane:?}) should be Insert");
        let consumed = mode_intercept(
            &mut app,
            KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE),
        );
        assert!(!consumed, "i on {pane:?} must fall through to pane");
        assert_eq!(app.mode, Mode::Insert, "mode unchanged");
    }
}

/// p9-fb-12: modifier-bearing keys (Ctrl+Esc, Alt+i) are NOT the
/// mode toggle. Falls through so chord handlers downstream get a
/// shot.
#[test]
fn modifier_keys_do_not_trigger_intercept() {
    let mut app = fresh_app(Pane::Library);
    app.mode = Mode::Insert;
    let consumed = mode_intercept(
        &mut app,
        KeyEvent::new(KeyCode::Esc, KeyModifiers::CONTROL),
    );
    assert!(!consumed, "Ctrl+Esc must fall through");
    assert_eq!(app.mode, Mode::Insert, "mode unchanged");

    app.mode = Mode::Normal;
    let consumed = mode_intercept(
        &mut app,
        KeyEvent::new(KeyCode::Char('i'), KeyModifiers::ALT),
    );
    assert!(!consumed, "Alt+i must fall through");
    assert_eq!(app.mode, Mode::Normal, "mode unchanged");
}

/// p9-fb-12: SHIFT alone is allowed (the toggle keys are unshifted
/// `i` / `Esc`, but a future `Shift+Esc` chord is unlikely; pre-
/// allow SHIFT so capital-letter typing in Search/Ask doesn't
/// accidentally fall into the modifier-block branch).
#[test]
fn shift_modifier_passes_modifier_filter() {
    // SHIFT+Esc is a strange combo but the filter passes it. (The
    // actual outcome — does mode flip? — depends on the case
    // matching i/Esc. SHIFT+Esc still matches KeyCode::Esc, so it
    // toggles. SHIFT+I would be KeyCode::Char('I') (capital), NOT
    // 'i', so it falls through. Both are intentional.)
    let mut app = fresh_app(Pane::Library);
    app.mode = Mode::Insert;
    let consumed = mode_intercept(
        &mut app,
        KeyEvent::new(KeyCode::Esc, KeyModifiers::SHIFT),
    );
    assert!(consumed, "Shift+Esc still toggles (modifier filter allows SHIFT)");

    let mut app = fresh_app(Pane::Library);
    let consumed = mode_intercept(
        &mut app,
        KeyEvent::new(KeyCode::Char('I'), KeyModifiers::SHIFT),
    );
    assert!(!consumed, "Shift+I (capital) falls through — only lowercase 'i' toggles");
}
