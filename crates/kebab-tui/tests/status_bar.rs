//! p9-fb-24: integration tests for the always-visible status bar.

use kebab_config::Config;
use kebab_tui::{App, Pane};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;

fn fresh_app(focus: Pane) -> App {
    let mut config = Config::defaults();
    config.storage.data_dir = "/tmp/kebab-tui-status-bar-tests-noop".to_string();
    config.workspace.root = Some("/tmp/kebab-tui-status-bar-tests-noop/workspace".to_string());
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

#[test]
fn status_bar_shows_streaming_when_ask_streaming() {
    let mut app = fresh_app(Pane::Ask);
    app.ask = Some(kebab_tui::AskState {
        streaming: true,
        ..Default::default()
    });
    let rendered = render_to_string(&app, 100);
    assert!(
        rendered.contains("streaming…"),
        "streaming marker missing: rendered=\n{rendered}"
    );
    assert!(
        !rendered.contains("idle"),
        "idle should not appear when streaming: rendered=\n{rendered}"
    );
}

#[test]
fn status_bar_shows_searching_when_search_worker_active() {
    let mut app = fresh_app(Pane::Search);
    app.search = Some(kebab_tui::SearchState {
        searching: true,
        ..Default::default()
    });
    let rendered = render_to_string(&app, 100);
    assert!(
        rendered.contains("searching…"),
        "searching marker missing: rendered=\n{rendered}"
    );
}

#[test]
fn status_bar_shows_ask_conv_id_when_in_ask_with_context() {
    let mut app = fresh_app(Pane::Ask);
    app.ask = Some(kebab_tui::AskState {
        conversation_id: Some("conv_a3f9b2c1d4e5f6a7b8c9d0e1f2a3b4c5".to_string()),
        current_question: Some("test?".to_string()),
        ..Default::default()
    });
    let rendered = render_to_string(&app, 100);
    assert!(
        rendered.contains("conv_a3f9b2c1…"),
        "8-hex prefix conv id missing: rendered=\n{rendered}"
    );
}

#[test]
fn status_bar_omits_conv_id_when_ask_has_no_context() {
    let mut app = fresh_app(Pane::Ask);
    app.ask = Some(kebab_tui::AskState::default());
    let rendered = render_to_string(&app, 100);
    assert!(
        !rendered.contains("conv_"),
        "conv id should not appear without context: rendered=\n{rendered}"
    );
}

#[test]
fn status_bar_omits_conv_id_outside_ask() {
    let mut app = fresh_app(Pane::Library);
    app.ask = Some(kebab_tui::AskState {
        conversation_id: Some("conv_a3f9b2c1d4e5f6a7b8c9d0e1f2a3b4c5".to_string()),
        current_question: Some("test?".to_string()),
        ..Default::default()
    });
    let rendered = render_to_string(&app, 100);
    assert!(
        !rendered.contains("conv_"),
        "conv id leaked outside Ask pane: rendered=\n{rendered}"
    );
}

#[test]
fn status_bar_shows_ingest_progress_in_dynamic_slot() {
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    let mut app = fresh_app(Pane::Library);
    let (_tx, rx) = std::sync::mpsc::channel();
    app.ingest_state = Some(kebab_tui::IngestState {
        rx,
        counts: kebab_app::AggregateCounts {
            scanned: 40,
            ..Default::default()
        },
        current_path: Some("notes/foo.md".to_string()),
        current_idx: 12,
        started_at: std::time::Instant::now(),
        terminal_at: None,
        aborted: false,
        thread: None,
        cancel: Arc::new(AtomicBool::new(false)),
    });
    let rendered = render_to_string(&app, 200);
    assert!(
        rendered.contains("12/40"),
        "ingest progress fragment missing: rendered=\n{rendered}"
    );
    assert!(
        rendered.contains("30%"),
        "ingest percentage missing: rendered=\n{rendered}"
    );
    assert!(
        !rendered.contains("idle"),
        "idle should not appear during ingest: rendered=\n{rendered}"
    );
}
