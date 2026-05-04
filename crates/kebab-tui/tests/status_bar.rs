//! p9-fb-24: integration tests for the always-visible status bar.

use kebab_config::Config;
use kebab_tui::{App, Pane};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;

fn fresh_app(focus: Pane) -> App {
    let mut config = Config::defaults();
    config.storage.data_dir = "/tmp/kebab-tui-status-bar-tests-noop".to_string();
    config.workspace.root = "/tmp/kebab-tui-status-bar-tests-noop/workspace".to_string();
    let mut app = App::new(config).expect("App::new");
    app.focus = focus;
    app
}

fn render_to_string(app: &App, width: u16) -> String {
    let backend = TestBackend::new(width, 1);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| kebab_tui::render_status_bar(f, Rect::new(0, 0, width, 1), app))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn status_bar_shows_kebab_version_first() {
    let app = fresh_app(Pane::Library);
    let rendered = render_to_string(&app, 100);
    let expected = format!("kebab v{}", env!("CARGO_PKG_VERSION"));
    assert!(
        rendered.contains(&expected),
        "version not in status bar: rendered=\n{rendered}"
    );
}

#[test]
fn status_bar_shows_pane_label() {
    for (focus, expected) in [
        (Pane::Library, "Library"),
        (Pane::Search, "Search"),
        (Pane::Ask, "Ask"),
        (Pane::Inspect, "Inspect"),
        (Pane::Jobs, "Jobs"),
    ] {
        let app = fresh_app(focus);
        let rendered = render_to_string(&app, 100);
        assert!(
            rendered.contains(expected),
            "pane label '{expected}' not visible for focus={focus:?}: rendered=\n{rendered}"
        );
    }
}

#[test]
fn status_bar_shows_doc_count() {
    let app = fresh_app(Pane::Library);
    let rendered = render_to_string(&app, 100);
    assert!(
        rendered.contains("0 docs"),
        "doc count missing: rendered=\n{rendered}"
    );
}

#[test]
fn status_bar_idle_when_no_dynamic_state() {
    let app = fresh_app(Pane::Library);
    let rendered = render_to_string(&app, 100);
    assert!(
        rendered.contains("idle"),
        "idle marker missing: rendered=\n{rendered}"
    );
}
