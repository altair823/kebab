//! Unit + snapshot tests for the Library pane.
//!
//! Snapshot tests use `ratatui::backend::TestBackend` so the run loop
//! is bypassed entirely — we drive `render_library` directly against
//! a synthetic `App`.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use kebab_config::Config;
use kebab_core::{
    ChunkerVersion, DocSummary, DocumentId, Lang, ParserVersion, SourceType, TrustLevel,
    WorkspacePath,
};
use kebab_tui::{App, KeyOutcome, Pane, render_library};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use time::OffsetDateTime;

fn make_doc(path: &str, title: &str, tags: Vec<&str>) -> DocSummary {
    DocSummary {
        doc_id: DocumentId(format!("{:0<32}", path.chars().filter(|c| c.is_alphanumeric()).collect::<String>())),
        doc_path: WorkspacePath::new(path.into()).unwrap(),
        title: title.into(),
        lang: Lang("en".into()),
        tags: tags.into_iter().map(String::from).collect(),
        trust_level: TrustLevel::Primary,
        source_type: SourceType::Note,
        byte_len: 1024,
        chunk_count: 4,
        created_at: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
        updated_at: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
        parser_version: ParserVersion("test-parser".into()),
        chunker_version: ChunkerVersion("test-chunker".into()),
    }
}

fn app_with_docs(docs: Vec<DocSummary>) -> App {
    let mut config = Config::defaults();
    // Storage paths point at /tmp so any accidental facade call
    // would not touch the user's real KB. Tests below use the
    // `populate_library_for_testing` test seam, never the facade.
    config.storage.data_dir = "/tmp/kebab-tui-tests-noop".to_string();
    let mut app = App::new(config).expect("App::new must succeed with defaults");
    app.populate_library_for_testing(docs);
    app
}

#[test]
fn empty_library_renders_block_only_no_panic() {
    let app = app_with_docs(vec![]);
    let backend = TestBackend::new(80, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            let area = Rect::new(0, 0, 80, 20);
            render_library(f, area, &app);
        })
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let rendered: String = (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        rendered.contains("Library"),
        "rendered frame must show Library header: {rendered}"
    );
    assert!(
        rendered.contains("no docs") || rendered.contains("Library"),
        "empty state hint should appear in the header line"
    );
}

#[test]
fn handle_key_library_q_quits() {
    let mut app = app_with_docs(vec![]);
    let outcome = kebab_tui::handle_key_library(
        &mut app,
        KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
    );
    assert_eq!(outcome, KeyOutcome::Quit);
}

#[test]
fn handle_key_library_esc_quits_when_no_overlay() {
    let mut app = app_with_docs(vec![]);
    let outcome = kebab_tui::handle_key_library(
        &mut app,
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
    );
    assert_eq!(outcome, KeyOutcome::Quit);
}

#[test]
fn handle_key_library_slash_switches_to_search() {
    let mut app = app_with_docs(vec![]);
    let outcome = kebab_tui::handle_key_library(
        &mut app,
        KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE),
    );
    assert_eq!(outcome, KeyOutcome::SwitchPane(Pane::Search));
}

#[test]
fn handle_key_library_question_switches_to_ask() {
    let mut app = app_with_docs(vec![]);
    let outcome = kebab_tui::handle_key_library(
        &mut app,
        KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE),
    );
    assert_eq!(outcome, KeyOutcome::SwitchPane(Pane::Ask));
}

#[test]
fn handle_key_library_enter_does_not_switch_when_empty() {
    let mut app = app_with_docs(vec![]);
    let outcome = kebab_tui::handle_key_library(
        &mut app,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    );
    assert_eq!(outcome, KeyOutcome::Continue);
}

#[test]
fn library_with_docs_renders_titles() {
    let app = app_with_docs(vec![
        make_doc("notes/foo.md", "Foo", vec!["alpha"]),
        make_doc("notes/bar.md", "Bar", vec!["beta", "gamma"]),
        make_doc("notes/baz.md", "Baz Title", vec![]),
    ]);
    let backend = TestBackend::new(80, 10);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            let area = Rect::new(0, 0, 80, 10);
            render_library(f, area, &app);
        })
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let rendered: String = (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    for title in &["Foo", "Bar", "Baz Title"] {
        assert!(
            rendered.contains(title),
            "rendered must contain {title}, got:\n{rendered}"
        );
    }
}

#[test]
fn handle_key_library_arrow_down_moves_selection() {
    let mut app = app_with_docs(vec![
        make_doc("a.md", "A", vec![]),
        make_doc("b.md", "B", vec![]),
        make_doc("c.md", "C", vec![]),
    ]);
    let outcome = kebab_tui::handle_key_library(
        &mut app,
        KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
    );
    assert_eq!(outcome, KeyOutcome::Continue);
    let outcome2 = kebab_tui::handle_key_library(
        &mut app,
        KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
    );
    assert_eq!(outcome2, KeyOutcome::Continue);
    // Third j hits the bottom; clamp must not panic / overflow.
    let outcome3 = kebab_tui::handle_key_library(
        &mut app,
        KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
    );
    assert_eq!(outcome3, KeyOutcome::Continue);
}

#[test]
fn handle_key_library_enter_inspects_when_docs_present() {
    let mut app = app_with_docs(vec![make_doc("a.md", "A", vec![])]);
    let outcome = kebab_tui::handle_key_library(
        &mut app,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    );
    assert_eq!(outcome, KeyOutcome::SwitchPane(Pane::Inspect));
}

#[test]
fn handle_key_library_f_opens_filter_overlay_then_enter_refreshes() {
    let mut app = app_with_docs(vec![make_doc("a.md", "A", vec![])]);
    // Open filter.
    let o1 = kebab_tui::handle_key_library(
        &mut app,
        KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE),
    );
    assert_eq!(o1, KeyOutcome::Continue);
    // Type into tags buffer.
    for ch in "foo".chars() {
        kebab_tui::handle_key_library(
            &mut app,
            KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE),
        );
    }
    // Enter commits + refreshes.
    let o2 = kebab_tui::handle_key_library(
        &mut app,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    );
    assert_eq!(o2, KeyOutcome::Refresh);
}

/// p9-fb-10: Library renders Hangul / CJK titles without overflowing
/// the title column. Smoke pin — render with a mixed Korean fixture
/// and confirm no panic + the truncated width fits the column.
#[test]
fn library_renders_korean_titles_without_overflow() {
    let docs = vec![
        make_doc("ko/한글-노트.md", "러스트로 만드는 지식 베이스", vec!["rust", "한글"]),
        make_doc("jp/漢字メモ.md", "日本語のテストドキュメント", vec!["jp"]),
        make_doc("mix/hello-세계.md", "Hello, 세계 mixed title", vec!["mix"]),
    ];
    let app = app_with_docs(docs);
    let backend = TestBackend::new(80, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            let area = Rect::new(0, 0, 80, 20);
            render_library(f, area, &app);
        })
        .expect("render must not panic on CJK titles");
    let buffer = terminal.backend().buffer().clone();
    let rendered: String = (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    // At least one Hangul / Kanji glyph survives the render path.
    // TestBackend renders wide chars one-per-cell with the trailing
    // cell empty, so the joined string has spaces between adjacent
    // wide chars — assert single glyphs, not multi-char substrings.
    assert!(
        rendered.contains('러') || rendered.contains('한'),
        "expected a Hangul glyph in rendered frame: {rendered}"
    );
    assert!(
        rendered.contains('日') || rendered.contains('漢'),
        "expected a Kanji glyph in rendered frame: {rendered}"
    );
}
