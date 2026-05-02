//! Unit + snapshot tests for the Search pane (P9-2).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use kebab_config::Config;
use kebab_core::{
    Citation, ChunkId, ChunkerVersion, DocumentId, EmbeddingModelId, IndexVersion,
    RetrievalDetail, SearchHit, SearchMode, WorkspacePath,
};
use kebab_tui::{
    App, KeyOutcome, Pane, SearchState, build_jump_command, handle_key_search, render_search,
};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use std::path::Path;

fn fresh_app() -> App {
    let mut config = Config::defaults();
    config.storage.data_dir = "/tmp/kebab-tui-search-tests-noop".to_string();
    config.workspace.root = "/tmp/kebab-tui-search-tests-noop/workspace".to_string();
    let mut app = App::new(config).expect("App::new");
    app.focus = Pane::Search;
    app.search = Some(SearchState::default());
    app
}

fn make_hit(rank: u32, path: &str, snippet: &str, citation: Citation) -> SearchHit {
    SearchHit {
        rank,
        chunk_id: ChunkId(format!("{:0<32}", rank)),
        doc_id: DocumentId(format!("{:0<32}", rank * 2)),
        doc_path: WorkspacePath::new(path.into()).unwrap(),
        heading_path: vec!["Section".into(), "Sub".into()],
        section_label: Some("Sub".into()),
        snippet: snippet.into(),
        citation,
        retrieval: RetrievalDetail {
            method: SearchMode::Hybrid,
            fusion_score: 0.9,
            lexical_score: Some(0.8),
            vector_score: Some(0.95),
            lexical_rank: Some(rank),
            vector_rank: Some(rank),
        },
        index_version: IndexVersion("v1".into()),
        embedding_model: Some(EmbeddingModelId("multilingual-e5-small".into())),
        chunker_version: ChunkerVersion("md-heading-v1".into()),
    }
}

fn line_citation(path: &str, line: u32) -> Citation {
    Citation::Line {
        path: WorkspacePath::new(path.into()).unwrap(),
        start: line,
        end: line,
        section: None,
    }
}

#[test]
fn esc_returns_to_library() {
    let mut app = fresh_app();
    let outcome = handle_key_search(
        &mut app,
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
    );
    assert_eq!(outcome, KeyOutcome::SwitchPane(Pane::Library));
}

#[test]
fn typing_appends_to_input_and_marks_dirty() {
    let mut app = fresh_app();
    for ch in "hello".chars() {
        handle_key_search(
            &mut app,
            KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE),
        );
    }
    let s = app.search.as_ref().unwrap();
    assert_eq!(s.input, "hello");
    assert!(s.input_dirty_at.is_some());
}

#[test]
fn backspace_removes_last_char() {
    let mut app = fresh_app();
    {
        let s = app.search.as_mut().unwrap();
        s.input = "abc".into();
    }
    handle_key_search(
        &mut app,
        KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
    );
    assert_eq!(app.search.as_ref().unwrap().input, "ab");
}

#[test]
fn tab_cycles_mode_lex_vec_hybrid() {
    let mut app = fresh_app();
    {
        let s = app.search.as_mut().unwrap();
        s.mode = SearchMode::Lexical;
    }
    let press_tab = |app: &mut App| {
        handle_key_search(app, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    };
    press_tab(&mut app);
    assert_eq!(app.search.as_ref().unwrap().mode, SearchMode::Vector);
    press_tab(&mut app);
    assert_eq!(app.search.as_ref().unwrap().mode, SearchMode::Hybrid);
    press_tab(&mut app);
    assert_eq!(app.search.as_ref().unwrap().mode, SearchMode::Lexical);
}

#[test]
fn enter_with_query_emits_refresh() {
    let mut app = fresh_app();
    app.search.as_mut().unwrap().input = "rust".into();
    let outcome = handle_key_search(
        &mut app,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    );
    assert_eq!(outcome, KeyOutcome::Refresh);
}

#[test]
fn enter_with_empty_query_is_continue() {
    let mut app = fresh_app();
    let outcome = handle_key_search(
        &mut app,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    );
    assert_eq!(outcome, KeyOutcome::Continue);
}

#[test]
fn j_k_move_selection_within_bounds() {
    let mut app = fresh_app();
    {
        let s = app.search.as_mut().unwrap();
        s.hits = vec![
            make_hit(1, "a.md", "snip a\nline2", line_citation("a.md", 1)),
            make_hit(2, "b.md", "snip b\nline2", line_citation("b.md", 5)),
            make_hit(3, "c.md", "snip c\nline2", line_citation("c.md", 7)),
        ];
        s.selected_hit = 0;
    }
    handle_key_search(
        &mut app,
        KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
    );
    assert_eq!(app.search.as_ref().unwrap().selected_hit, 1);
    handle_key_search(
        &mut app,
        KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
    );
    assert_eq!(app.search.as_ref().unwrap().selected_hit, 2);
    // Bounds clamp.
    handle_key_search(
        &mut app,
        KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
    );
    assert_eq!(app.search.as_ref().unwrap().selected_hit, 2);
    handle_key_search(
        &mut app,
        KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE),
    );
    assert_eq!(app.search.as_ref().unwrap().selected_hit, 1);
}

#[test]
fn build_jump_command_line_uses_plus_n_for_vim() {
    let citation = line_citation("notes/foo.md", 42);
    let (program, args) =
        build_jump_command(&citation, "vim", Path::new("/tmp/workspace"));
    assert_eq!(program, "vim");
    assert_eq!(args, vec!["+42".to_string(), "/tmp/workspace/notes/foo.md".into()]);
}

#[test]
fn build_jump_command_line_uses_g_flag_for_code() {
    let citation = line_citation("notes/foo.md", 42);
    let (program, args) =
        build_jump_command(&citation, "code", Path::new("/tmp/workspace"));
    assert_eq!(program, "code");
    assert_eq!(args, vec!["-g".to_string(), "/tmp/workspace/notes/foo.md:42".into()]);
}

#[test]
fn build_jump_command_passes_through_editor_args() {
    let citation = line_citation("a.md", 7);
    let (program, args) = build_jump_command(&citation, "nvim -p", Path::new("/ws"));
    assert_eq!(program, "nvim");
    // Leading `-p` from $EDITOR env preserved before the +N path arg.
    assert!(args[0] == "-p", "leading editor arg preserved: {args:?}");
    assert!(args.contains(&"+7".to_string()));
    assert!(args.contains(&"/ws/a.md".to_string()));
}

#[test]
fn render_search_with_hits_shows_input_and_path() {
    let mut app = fresh_app();
    {
        let s = app.search.as_mut().unwrap();
        s.input = "rust traits".into();
        s.mode = SearchMode::Hybrid;
        s.hits = vec![
            make_hit(1, "notes/rust.md", "trait dispatch\nis dynamic", line_citation("notes/rust.md", 12)),
            make_hit(2, "notes/dyn.md", "dynamic dispatch\nvtable", line_citation("notes/dyn.md", 3)),
        ];
        s.selected_hit = 0;
    }
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            let area = Rect::new(0, 0, 80, 24);
            render_search(f, area, &app);
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
    assert!(rendered.contains("hybrid"), "mode badge rendered: {rendered}");
    assert!(rendered.contains("rust traits"), "input text rendered");
    assert!(rendered.contains("notes/rust.md"), "first hit path rendered");
    assert!(rendered.contains("notes/dyn.md"), "second hit path rendered");
}

#[test]
fn empty_state_renders_without_panic() {
    let app = fresh_app();
    let backend = TestBackend::new(80, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            let area = Rect::new(0, 0, 80, 20);
            render_search(f, area, &app);
        })
        .unwrap();
}

#[test]
fn no_search_state_returns_to_library() {
    let mut config = Config::defaults();
    config.storage.data_dir = "/tmp/kebab-tui-search-tests-noop".into();
    let mut app = App::new(config).unwrap();
    app.focus = Pane::Search;
    // search slot intentionally None.
    let outcome = handle_key_search(
        &mut app,
        KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE),
    );
    assert_eq!(outcome, KeyOutcome::SwitchPane(Pane::Library));
}
