//! Unit + snapshot tests for the Search pane (P9-2).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use kebab_config::Config;
use kebab_core::{
    ChunkId, ChunkerVersion, Citation, DocumentId, EmbeddingModelId, IndexVersion, RetrievalDetail,
    SearchHit, SearchMode, WorkspacePath,
};
use kebab_tui::{
    App, KeyOutcome, Mode, Pane, SearchState, SearchWorkerMessage, build_jump_command,
    handle_key_search, poll_search_worker, render_search, search_debounce_due,
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
    // p9-fb-12 follow-up: mirror the run loop's auto-flip — Search
    // pane auto-Insert. Tests that exercise Normal-mode navigation
    // (j/k move selection, i / g pre-pass) set Mode::Normal
    // explicitly.
    app.mode = kebab_tui::Mode::auto_for(Pane::Search);
    app.search = Some(SearchState::default());
    app
}

fn make_hit(rank: u32, path: &str, snippet: &str, citation: Citation) -> SearchHit {
    SearchHit {
        rank,
        chunk_id: ChunkId(format!("{rank:0<32}")),
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
        // fb-32: TUI search test fixtures pinned to UNIX_EPOCH + stale=false;
        // staleness rendering covered in dedicated tests (Task 11).
        indexed_at: time::OffsetDateTime::UNIX_EPOCH,
        stale: false,
        score_kind: kebab_core::ScoreKind::Rrf,
        repo: None,
        code_lang: None,
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
    let outcome = handle_key_search(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
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
    assert_eq!(s.input.as_str(), "hello");
    assert!(s.input_dirty_at.is_some());
}

#[test]
fn backspace_removes_last_char() {
    let mut app = fresh_app();
    {
        let s = app.search.as_mut().unwrap();
        s.input.push_str("abc");
    }
    handle_key_search(
        &mut app,
        KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
    );
    assert_eq!(app.search.as_ref().unwrap().input.as_str(), "ab");
    assert_eq!(app.search.as_ref().unwrap().input.cursor_col(), 2);
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
    {
        let s = app.search.as_mut().unwrap();
        s.input.push_str("rust");
    }
    let outcome = handle_key_search(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(outcome, KeyOutcome::Refresh);
}

#[test]
fn enter_with_empty_query_is_continue() {
    let mut app = fresh_app();
    let outcome = handle_key_search(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(outcome, KeyOutcome::Continue);
}

#[test]
fn j_k_move_selection_within_bounds() {
    let mut app = fresh_app();
    // p9-fb-12 follow-up: j/k navigate only in Normal mode. Search
    // pane auto-Insert via fresh_app, flip to Normal explicitly to
    // exercise the navigation branch.
    app.mode = kebab_tui::Mode::Normal;
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
    let (program, args) = build_jump_command(&citation, "vim", Path::new("/tmp/workspace"));
    assert_eq!(program, "vim");
    assert_eq!(
        args,
        vec!["+42".to_string(), "/tmp/workspace/notes/foo.md".into()]
    );
}

#[test]
fn build_jump_command_line_uses_g_flag_for_code() {
    let citation = line_citation("notes/foo.md", 42);
    let (program, args) = build_jump_command(&citation, "code", Path::new("/tmp/workspace"));
    assert_eq!(program, "code");
    assert_eq!(
        args,
        vec!["-g".to_string(), "/tmp/workspace/notes/foo.md:42".into()]
    );
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
        s.input.push_str("rust traits");
        s.mode = SearchMode::Hybrid;
        s.hits = vec![
            make_hit(
                1,
                "notes/rust.md",
                "trait dispatch\nis dynamic",
                line_citation("notes/rust.md", 12),
            ),
            make_hit(
                2,
                "notes/dyn.md",
                "dynamic dispatch\nvtable",
                line_citation("notes/dyn.md", 3),
            ),
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
    assert!(
        rendered.contains("hybrid"),
        "mode badge rendered: {rendered}"
    );
    assert!(rendered.contains("rust traits"), "input text rendered");
    assert!(
        rendered.contains("notes/rust.md"),
        "first hit path rendered"
    );
    assert!(
        rendered.contains("notes/dyn.md"),
        "second hit path rendered"
    );
}

/// p9-fb-32: Search pane prefixes the rank/score header line with a
/// Warning-styled `[STALE] ` Span when `hit.stale == true`. Pin the
/// text-level signal (color is exercised via the cell scan below).
#[test]
fn search_pane_shows_stale_badge_for_old_doc() {
    let mut app = fresh_app();
    {
        let s = app.search.as_mut().unwrap();
        s.input.push_str("rust");
        s.mode = SearchMode::Hybrid;
        let mut stale_hit = make_hit(
            1,
            "notes/old.md",
            "ancient trait dispatch\nstill relevant",
            line_citation("notes/old.md", 7),
        );
        // Synthesize an indexed_at well past any threshold; combined
        // with `stale: true` this matches the post-process output of
        // `kebab_app::mark_stale_in_place`.
        stale_hit.indexed_at = time::OffsetDateTime::UNIX_EPOCH;
        stale_hit.stale = true;
        let fresh_hit = make_hit(
            2,
            "notes/new.md",
            "modern dispatch\nvtable",
            line_citation("notes/new.md", 3),
        );
        s.hits = vec![stale_hit, fresh_hit];
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
    assert!(
        rendered.contains("[STALE]"),
        "[STALE] badge must render as text on stale hit: {rendered}"
    );
    // The badge appears on the same line that begins with rank `1.`
    // — the stale hit. The fresh `notes/new.md` row must NOT carry
    // the badge.
    let stale_line = rendered
        .lines()
        .find(|l| l.contains("notes/old.md"))
        .expect("stale hit's header line must render");
    assert!(
        stale_line.contains("[STALE]"),
        "stale row must carry [STALE] badge: {stale_line}"
    );
    let fresh_line = rendered
        .lines()
        .find(|l| l.contains("notes/new.md"))
        .expect("fresh hit's header line must render");
    assert!(
        !fresh_line.contains("[STALE]"),
        "fresh row must NOT carry [STALE] badge: {fresh_line}"
    );
    // Color side: the `[` of `[STALE]` must be Yellow (Warning role,
    // dark palette default).
    let mut stale_yellow_found = false;
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            let cell = &buffer[(x, y)];
            if cell.symbol() == "[" {
                // The cell to the right should be 'S' if this is the
                // start of `[STALE]` — narrow check to avoid the
                // rank/score `[` cells (there shouldn't be any there).
                if x + 1 < buffer.area.width && buffer[(x + 1, y)].symbol() == "S" {
                    if let ratatui::style::Color::Yellow = cell.fg {
                        stale_yellow_found = true;
                    }
                }
            }
        }
    }
    assert!(
        stale_yellow_found,
        "[STALE] badge must be rendered with Yellow (Warning role) fg"
    );
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

/// p9-fb-12 follow-up: in Insert mode, plain `j` types into input
/// (does NOT move selection). Replaces the pre-fb-12 heuristic
/// "is_typing_mod" with mode-authoritative dispatch.
#[test]
fn j_in_insert_types_does_not_move_selection() {
    let mut app = fresh_app();
    // Insert is auto for Search, but explicit for clarity.
    app.mode = kebab_tui::Mode::Insert;
    {
        let s = app.search.as_mut().unwrap();
        s.hits = vec![
            make_hit(1, "a.md", "snip", line_citation("a.md", 1)),
            make_hit(2, "b.md", "snip", line_citation("b.md", 1)),
        ];
        s.selected_hit = 0;
    }
    handle_key_search(
        &mut app,
        KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
    );
    let s = app.search.as_ref().unwrap();
    assert_eq!(s.input.as_str(), "j", "j must type in Insert mode");
    assert_eq!(s.selected_hit, 0, "selection must NOT move in Insert");
}

/// p9-fb-12 follow-up: in Normal mode, plain Char other than j/k/i/g
/// is a no-op (no typing in Normal). Pin so a future char binding
/// addition has to think about Normal-mode behavior.
#[test]
fn arbitrary_char_in_normal_mode_is_noop() {
    let mut app = fresh_app();
    app.mode = kebab_tui::Mode::Normal;
    handle_key_search(
        &mut app,
        KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE),
    );
    let s = app.search.as_ref().unwrap();
    assert_eq!(s.input.as_str(), "", "Normal-mode Char must NOT type");
}

#[test]
fn shift_j_stays_in_input_does_not_move_selection() {
    // R1 fix: SHIFT-J / SHIFT-K must reach the typing branch so
    // queries like \"JSON\" / \"PostgreSQL\" don't get \"J\" eaten as
    // a selection move.
    let mut app = fresh_app();
    {
        let s = app.search.as_mut().unwrap();
        s.hits = vec![
            make_hit(1, "a.md", "snip\nl2", line_citation("a.md", 1)),
            make_hit(2, "b.md", "snip\nl2", line_citation("b.md", 1)),
        ];
        s.selected_hit = 0;
    }
    handle_key_search(
        &mut app,
        KeyEvent::new(KeyCode::Char('J'), KeyModifiers::SHIFT),
    );
    let s = app.search.as_ref().unwrap();
    assert_eq!(s.selected_hit, 0, "selection must NOT move on SHIFT-J");
    assert_eq!(s.input.as_str(), "J", "SHIFT-J must reach the input buffer");
}

#[test]
fn shift_g_does_not_trigger_editor_jump() {
    // R1 fix: capital G must not invoke jump_to_citation. Keep it
    // as plain typing so \"Go\" / \"Greetings\" search queries work.
    let mut app = fresh_app();
    {
        let s = app.search.as_mut().unwrap();
        s.hits = vec![make_hit(1, "a.md", "snip\nl2", line_citation("a.md", 1))];
    }
    let outcome = handle_key_search(
        &mut app,
        KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT),
    );
    assert_eq!(outcome, KeyOutcome::Continue);
    assert_eq!(app.search.as_ref().unwrap().input.as_str(), "G");
}

/// p9-fb-09 — `g` on a hit enqueues an `EditorRequest` on `App.pending_editor`
/// rather than spawning the child synchronously. The run loop services the
/// queue with the `TuiTerminal` handle in scope so the post-resume
/// `terminal.clear()` can land (preventing the corrupted-redraw bug).
#[test]
fn g_key_enqueues_pending_editor_request() {
    let mut app = fresh_app();
    // p9-fb-12 follow-up: `g` (editor jump) is a Normal-mode command;
    // in Insert mode it types as 'g'. Flip explicitly.
    app.mode = kebab_tui::Mode::Normal;
    {
        let s = app.search.as_mut().unwrap();
        s.hits = vec![make_hit(
            1,
            "notes/x.md",
            "snippet",
            line_citation("notes/x.md", 42),
        )];
        s.selected_hit = 0;
    }
    assert!(app.pending_editor().is_none(), "queue starts empty");
    let outcome = handle_key_search(
        &mut app,
        KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE),
    );
    assert_eq!(outcome, KeyOutcome::Continue);
    let req = app
        .pending_editor()
        .expect("g on a hit must enqueue an EditorRequest");
    match &req.citation {
        Citation::Line { path, start, .. } => {
            assert_eq!(path.0, "notes/x.md");
            assert_eq!(*start, 42);
        }
        other => panic!("unexpected citation variant: {other:?}"),
    }
    // editor_env reads $EDITOR — fall back to "vi" for tests.
    assert!(!req.editor_env.is_empty(), "editor_env must be populated");
}

/// p9-fb-09 — `g` with no hits is a no-op; the queue stays empty.
#[test]
fn g_key_with_no_hits_does_not_enqueue() {
    let mut app = fresh_app();
    // Search slot present, hits empty.
    let _outcome = handle_key_search(
        &mut app,
        KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE),
    );
    assert!(
        app.pending_editor().is_none(),
        "g with no hits must not enqueue"
    );
}

// ── p9-fb-08: async search worker + generation counter ────────────

/// `poll_search_worker` applies a fresh result (matching generation)
/// to `state.search.hits` and clears `searching`.
#[test]
fn poll_worker_applies_fresh_result_to_hits() {
    let mut app = fresh_app();
    let (tx, rx) = std::sync::mpsc::channel();
    {
        let s = app.search.as_mut().unwrap();
        s.generation = 5;
        s.searching = true;
        s.worker_rx = Some(rx);
    }
    let hit = make_hit(1, "a.md", "snip", line_citation("a.md", 1));
    tx.send(SearchWorkerMessage::Done {
        generation: 5,
        result: Ok(vec![hit]),
    })
    .unwrap();
    poll_search_worker(&mut app);
    let s = app.search.as_ref().unwrap();
    assert_eq!(s.hits.len(), 1, "fresh result populates hits");
    assert!(!s.searching, "searching cleared");
    assert!(s.worker_rx.is_none(), "rx drained");
}

/// p9-fb-08 — a stale result (generation mismatch) is silently
/// dropped. `searching` remains true since a newer worker is
/// (presumed) still in flight.
#[test]
fn poll_worker_drops_stale_result() {
    let mut app = fresh_app();
    let (tx, rx) = std::sync::mpsc::channel();
    {
        let s = app.search.as_mut().unwrap();
        s.generation = 7;
        s.searching = true;
        s.worker_rx = Some(rx);
    }
    let hit = make_hit(1, "stale.md", "snip", line_citation("stale.md", 1));
    // generation 3 < current 7 → stale.
    tx.send(SearchWorkerMessage::Done {
        generation: 3,
        result: Ok(vec![hit]),
    })
    .unwrap();
    poll_search_worker(&mut app);
    let s = app.search.as_ref().unwrap();
    assert!(s.hits.is_empty(), "stale result must not populate hits");
    assert!(
        s.searching,
        "searching stays true so newer worker can resolve it"
    );
    assert!(
        s.worker_rx.is_none(),
        "stale message still drains the rx slot — worker is one-shot"
    );
}

/// p9-fb-08 — `poll_search_worker` is a no-op when no worker is in
/// flight (no rx). Common case on every tick the user isn't typing.
#[test]
fn poll_worker_noop_when_no_rx() {
    let mut app = fresh_app();
    {
        let s = app.search.as_mut().unwrap();
        s.hits = vec![make_hit(1, "x.md", "snip", line_citation("x.md", 1))];
    }
    poll_search_worker(&mut app);
    let s = app.search.as_ref().unwrap();
    assert_eq!(s.hits.len(), 1, "existing hits preserved");
    assert!(s.worker_rx.is_none());
}

/// Helper for the debounce_due tests — build a state with the four
/// fields the test cares about set, others default.
#[allow(clippy::field_reassign_with_default)]
fn search_state_with(
    input: &str,
    mode: SearchMode,
    searching: bool,
    last_query: Option<(String, SearchMode)>,
) -> SearchState {
    let mut s = SearchState::default();
    s.input.push_str(input);
    s.mode = mode;
    s.searching = searching;
    s.last_query = last_query;
    s.input_dirty_at = Some(time::OffsetDateTime::now_utc() - time::Duration::seconds(1));
    s
}

/// p9-fb-08 — `debounce_due` skips when an in-flight worker is
/// already running for the same `(input, mode)` pair. Without this
/// guard, a "phantom keystroke" (re-typing the same chars) would
/// pile up workers and burn CPU.
#[test]
fn debounce_due_skips_when_in_flight_for_same_query() {
    let s = search_state_with(
        "hello",
        SearchMode::Hybrid,
        true,
        Some(("hello".into(), SearchMode::Hybrid)),
    );
    assert!(
        !search_debounce_due(&s),
        "in-flight worker for same query → debounce must skip"
    );
}

/// p9-fb-08 — `debounce_due` still fires when a different query is
/// in flight (user typed past the in-flight one). The new spawn
/// makes the prior result stale (handled by `poll_worker`).
#[test]
fn debounce_due_fires_when_in_flight_for_different_query() {
    let s = search_state_with(
        "hello world",
        SearchMode::Hybrid,
        true,
        Some(("hello".into(), SearchMode::Hybrid)),
    );
    assert!(
        search_debounce_due(&s),
        "in-flight worker for old query → new query still spawns"
    );
}

/// p9-fb-08 — disconnected channel (worker panicked) clears the rx
/// + searching flag so the next debounce tick can re-fire cleanly.
#[test]
fn poll_worker_handles_disconnected_channel() {
    let mut app = fresh_app();
    let (tx, rx) = std::sync::mpsc::channel::<SearchWorkerMessage>();
    {
        let s = app.search.as_mut().unwrap();
        s.searching = true;
        s.worker_rx = Some(rx);
    }
    drop(tx); // simulate worker panic before send
    poll_search_worker(&mut app);
    let s = app.search.as_ref().unwrap();
    assert!(!s.searching, "searching cleared on disconnect");
    assert!(s.worker_rx.is_none());
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

/// p9-fb-10: typing Hangul into Search input advances cursor by 2
/// per char and round-trips through the buffer correctly.
#[test]
fn hangul_typing_in_search_input_advances_cursor_by_two_per_char() {
    let mut app = fresh_app();
    // Switch to search and ensure Insert mode so chars type.
    app.focus = Pane::Search;
    app.mode = kebab_tui::Mode::auto_for(Pane::Search);
    for ch in "한글".chars() {
        handle_key_search(
            &mut app,
            KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE),
        );
    }
    assert_eq!(app.search.as_ref().unwrap().input.as_str(), "한글");
    assert_eq!(app.search.as_ref().unwrap().input.cursor_col(), 4);
    // Backspace pops the trailing Hangul char and rewinds 2 cols.
    handle_key_search(
        &mut app,
        KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
    );
    assert_eq!(app.search.as_ref().unwrap().input.as_str(), "한");
    assert_eq!(app.search.as_ref().unwrap().input.cursor_col(), 2);
}

/// p9-fb-21: chunk-inspect was rebound from `i` to `o` so `i`
/// could become the universal Normal→Insert toggle. Pin the new
/// `o` key — Normal mode + at least one hit + selected → SwitchPane(Inspect).
#[test]
fn o_in_normal_with_hits_enters_inspect() {
    let mut app = fresh_app();
    app.focus = Pane::Search;
    app.mode = Mode::Normal;
    let s = app.search.as_mut().unwrap();
    s.hits = vec![make_hit(1, "a.md", "snippet", line_citation("a.md", 1))];
    s.selected_hit = 0;
    let outcome = kebab_tui::handle_key_search(
        &mut app,
        KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE),
    );
    assert_eq!(outcome, KeyOutcome::SwitchPane(Pane::Inspect));
}

/// p9-fb-21: `o` with empty hits is a no-op (Continue) — do not
/// enter Inspect with no target.
#[test]
fn o_in_normal_with_empty_hits_is_continue() {
    let mut app = fresh_app();
    app.focus = Pane::Search;
    app.mode = Mode::Normal;
    let outcome = kebab_tui::handle_key_search(
        &mut app,
        KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE),
    );
    assert_eq!(outcome, KeyOutcome::Continue);
}

/// p9-fb-21: in Insert mode, `o` types as a regular char (the
/// chunk-inspect intercept only fires in Normal). Pin so a future
/// regression that drops the `is_normal` guard would fail this.
#[test]
fn o_in_insert_types_into_input() {
    let mut app = fresh_app();
    app.focus = Pane::Search;
    app.mode = Mode::Insert;
    let outcome = kebab_tui::handle_key_search(
        &mut app,
        KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE),
    );
    assert_eq!(outcome, KeyOutcome::Continue);
    assert_eq!(app.search.as_ref().unwrap().input.as_str(), "o");
}
