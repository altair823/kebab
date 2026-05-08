//! Unit + snapshot tests for the Ask pane (P9-3).
//!
//! Worker thread / streaming path is NOT exercised here — that would
//! require a real Ollama + SQLite KB. Tests drive the pane via
//! hand-populated `AskState`.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use kebab_config::Config;
use kebab_core::{
    Answer, AnswerCitation, AnswerRetrievalSummary, Citation, ModelRef,
    PromptTemplateVersion, RefusalReason, SearchMode, TokenUsage, TraceId, Turn,
    WorkspacePath,
};
use kebab_tui::{App, AskState, KeyOutcome, Pane, handle_key_ask, render_ask};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use time::OffsetDateTime;

fn fresh_app() -> App {
    let mut config = Config::defaults();
    config.storage.data_dir = "/tmp/kebab-tui-ask-tests-noop".to_string();
    config.workspace.root = "/tmp/kebab-tui-ask-tests-noop/workspace".to_string();
    let mut app = App::new(config).expect("App::new");
    app.focus = Pane::Ask;
    // p9-fb-12 follow-up: mirror the run loop's auto-flip on pane
    // switch — Search/Ask auto-Insert. Tests that want Normal-mode
    // navigation behaviour set `app.mode = Mode::Normal` explicitly.
    app.mode = kebab_tui::Mode::auto_for(Pane::Ask);
    app.ask = Some(AskState::default());
    app
}

fn make_answer(grounded: bool, refusal: Option<RefusalReason>, body: &str) -> Answer {
    Answer {
        answer: body.to_string(),
        citations: vec![AnswerCitation {
            marker: Some("1".to_string()),
            citation: Citation::Line {
                path: WorkspacePath::new("notes/foo.md".into()).unwrap(),
                start: 12,
                end: 14,
                section: Some("Section A".into()),
            },
            // fb-32: TUI ask test fixture pinned to UNIX_EPOCH + stale=false;
            // staleness rendering covered in dedicated tests (Task 11).
            indexed_at: OffsetDateTime::UNIX_EPOCH,
            stale: false,
        }],
        grounded,
        refusal_reason: refusal,
        model: ModelRef {
            id: "qwen2.5:7b-instruct".into(),
            provider: "ollama".into(),
            dimensions: None,
        },
        embedding: Some(ModelRef {
            id: "multilingual-e5-small".into(),
            provider: "fastembed".into(),
            dimensions: Some(384),
        }),
        prompt_template_version: PromptTemplateVersion("rag-v1".into()),
        retrieval: AnswerRetrievalSummary {
            trace_id: TraceId("test-trace".into()),
            mode: SearchMode::Hybrid,
            k: 10,
            score_gate: 0.05,
            top_score: 0.8,
            chunks_returned: 7,
            chunks_used: 3,
        },
        usage: TokenUsage {
            prompt_tokens: 100,
            completion_tokens: 50,
            latency_ms: 1200,
        },
        created_at: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
        conversation_id: None,
        turn_index: None,
    }
}

#[test]
fn esc_returns_to_library_and_clears_streaming() {
    let mut app = fresh_app();
    {
        let s = app.ask.as_mut().unwrap();
        s.streaming = true;
        s.partial = "partial answer…".into();
    }
    let outcome = handle_key_ask(
        &mut app,
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
    );
    assert_eq!(outcome, KeyOutcome::SwitchPane(Pane::Library));
    let s = app.ask.as_ref().unwrap();
    assert!(!s.streaming);
    assert!(s.rx.is_none());
    assert!(s.thread.is_none());
}

#[test]
fn typing_appends_to_input() {
    let mut app = fresh_app();
    for ch in "hello".chars() {
        handle_key_ask(
            &mut app,
            KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE),
        );
    }
    assert_eq!(app.ask.as_ref().unwrap().input.as_str(), "hello");
}

#[test]
fn backspace_pops_input() {
    let mut app = fresh_app();
    {
        app.ask.as_mut().unwrap().input.push_str("abcd");
    }
    handle_key_ask(
        &mut app,
        KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
    );
    assert_eq!(app.ask.as_ref().unwrap().input.as_str(), "abc");
}

/// p9-fb-12 follow-up: `e` types into input in Insert mode (does
/// NOT toggle explain). Replaces the pre-fb-12 heuristic
/// "input.is_empty() then toggle else type" with mode-authoritative
/// dispatch.
#[test]
fn e_types_in_insert_mode_does_not_toggle_explain() {
    let mut app = fresh_app();
    // Insert auto for Ask, but explicit for clarity.
    app.mode = kebab_tui::Mode::Insert;
    assert!(!app.ask.as_ref().unwrap().explain);
    handle_key_ask(
        &mut app,
        KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE),
    );
    let s = app.ask.as_ref().unwrap();
    assert_eq!(s.input.as_str(), "e", "e must type in Insert mode");
    assert!(!s.explain, "explain must NOT toggle in Insert mode");
}

/// p9-fb-12 follow-up: `j` / `k` are scroll commands in Normal mode.
/// In Insert they type. Replaces input-empty heuristic.
#[test]
fn jk_scroll_in_normal_mode_type_in_insert() {
    let mut app = fresh_app();
    app.mode = kebab_tui::Mode::Normal;
    handle_key_ask(
        &mut app,
        KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
    );
    assert_eq!(app.ask.as_ref().unwrap().scroll, 1, "j scrolls down in Normal");
    handle_key_ask(
        &mut app,
        KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE),
    );
    assert_eq!(app.ask.as_ref().unwrap().scroll, 0, "k scrolls up in Normal");
    // Now Insert — j/k type.
    app.mode = kebab_tui::Mode::Insert;
    handle_key_ask(
        &mut app,
        KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
    );
    handle_key_ask(
        &mut app,
        KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE),
    );
    assert_eq!(app.ask.as_ref().unwrap().input.as_str(), "jk");
    assert_eq!(app.ask.as_ref().unwrap().scroll, 0, "no scroll in Insert");
}

/// p9-fb-12 follow-up: `e` toggles explain in Normal mode (was
/// previously gated on `input.is_empty()` heuristic). Test forces
/// Normal explicitly to mirror the run-loop flow (user pressed Esc).
#[test]
fn e_toggles_explain_in_normal_mode() {
    let mut app = fresh_app();
    app.mode = kebab_tui::Mode::Normal;
    assert!(!app.ask.as_ref().unwrap().explain);
    handle_key_ask(
        &mut app,
        KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE),
    );
    assert!(app.ask.as_ref().unwrap().explain);
    handle_key_ask(
        &mut app,
        KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE),
    );
    assert!(!app.ask.as_ref().unwrap().explain);
}

#[test]
fn e_typed_into_input_when_input_nonempty() {
    let mut app = fresh_app();
    {
        app.ask.as_mut().unwrap().input.push_str("qu");
    }
    handle_key_ask(
        &mut app,
        KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE),
    );
    let s = app.ask.as_ref().unwrap();
    assert_eq!(s.input.as_str(), "que");
    assert!(!s.explain, "explain must NOT toggle while typing a word");
}

#[test]
fn enter_with_empty_input_is_continue() {
    let mut app = fresh_app();
    let outcome = handle_key_ask(
        &mut app,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    );
    assert_eq!(outcome, KeyOutcome::Continue);
    assert!(!app.ask.as_ref().unwrap().streaming);
}

#[test]
fn enter_while_streaming_is_noop() {
    let mut app = fresh_app();
    {
        let s = app.ask.as_mut().unwrap();
        s.input.push_str("anything");
        s.streaming = true;
    }
    handle_key_ask(
        &mut app,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    );
    // streaming flag remains true (no new worker spawned)
    assert!(app.ask.as_ref().unwrap().streaming);
    // No thread spawned because enter was a no-op.
    assert!(app.ask.as_ref().unwrap().thread.is_none());
}

#[test]
fn render_pre_submission_shows_hint() {
    let app = fresh_app();
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            let area = Rect::new(0, 0, 80, 24);
            render_ask(f, area, &app);
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
    assert!(rendered.contains("ask"), "input bar visible");
    assert!(
        rendered.contains("type a question") || rendered.contains("Enter"),
        "pre-submission hint visible"
    );
}

#[test]
fn render_streaming_shows_partial_with_cursor() {
    let mut app = fresh_app();
    {
        let s = app.ask.as_mut().unwrap();
        s.input.push_str("what is RRF fusion?");
        s.streaming = true;
        s.partial = "RRF는 reciprocal rank fusion".into();
    }
    let backend = TestBackend::new(80, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            let area = Rect::new(0, 0, 80, 20);
            render_ask(f, area, &app);
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
    assert!(rendered.contains("streaming"), "streaming hint visible");
    assert!(
        rendered.contains("reciprocal rank fusion"),
        "partial body rendered"
    );
    assert!(rendered.contains("▍"), "cursor block rendered mid-stream");
}

#[test]
fn render_grounded_answer_with_citation() {
    let mut app = fresh_app();
    {
        let s = app.ask.as_mut().unwrap();
        s.input.push_str("test");
        let ans = make_answer(true, None, "test answer body [1].");
        // p9-fb-16: transcript renders completed turns; populate one
        // alongside last_answer so the right-panel status + body
        // assertions both have something to find.
        s.turns.push(Turn {
            question: "test question".into(),
            answer: ans.answer.clone(),
            citations: ans.citations.clone(),
            created_at: ans.created_at,
        });
        s.last_answer = Some(ans);
    }
    let backend = TestBackend::new(100, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            let area = Rect::new(0, 0, 100, 24);
            render_ask(f, area, &app);
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
    assert!(rendered.contains("test answer body"), "answer body rendered");
    assert!(rendered.contains("grounded ✓"), "grounded status visible");
    assert!(rendered.contains("notes/foo.md"), "citation path rendered");
    assert!(rendered.contains("[1]"), "citation marker rendered");
}

#[test]
fn render_refusal_score_gate_shows_status_without_citation_index_panic() {
    let mut app = fresh_app();
    {
        let s = app.ask.as_mut().unwrap();
        let mut ans = make_answer(false, Some(RefusalReason::ScoreGate), "insufficient grounding to answer.");
        ans.citations.clear(); // refusal often has no citations
        s.turns.push(Turn {
            question: "test refusal question".into(),
            answer: ans.answer.clone(),
            citations: ans.citations.clone(),
            created_at: ans.created_at,
        });
        s.last_answer = Some(ans);
    }
    let backend = TestBackend::new(120, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    // Test passes if render does not panic on empty citations.
    terminal
        .draw(|f| {
            let area = Rect::new(0, 0, 120, 20);
            render_ask(f, area, &app);
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
    assert!(rendered.contains("insufficient grounding"), "refusal body rendered");
    assert!(rendered.contains("grounded ✗"), "ungrounded status visible");
    assert!(rendered.contains("score_gate"), "refusal reason surfaced");
}

#[test]
fn explain_toggle_changes_panel_title() {
    let mut app = fresh_app();
    {
        let s = app.ask.as_mut().unwrap();
        s.last_answer = Some(make_answer(true, None, "answer body."));
        s.explain = true;
    }
    let backend = TestBackend::new(100, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            let area = Rect::new(0, 0, 100, 24);
            render_ask(f, area, &app);
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
        rendered.contains("explain (per-claim)"),
        "explain mode panel title"
    );
}

#[test]
fn enter_with_detached_prior_thread_is_blocked() {
    // R1 fix: after Esc, the prior worker is detached (thread still
    // running, rx cleared, streaming=false). A new Enter must NOT
    // spawn a second worker against the same Ollama endpoint until
    // the prior thread finishes.
    let mut app = fresh_app();
    {
        let s = app.ask.as_mut().unwrap();
        s.input.push_str("another question");
        s.streaming = false;
        // Simulate a detached prior worker by hand-installing a
        // never-ending JoinHandle. (We can't easily make a sleeping
        // thread without timing flakiness; an empty-loop shim works.)
        s.thread = Some(std::thread::spawn(|| {
            // Loop until the test drops the JoinHandle's owner via
            // App going out of scope. is_finished() will report
            // false until then.
            loop {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }));
    }
    let outcome = handle_key_ask(
        &mut app,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    );
    // Enter is a no-op while a prior thread is attached.
    assert_eq!(outcome, KeyOutcome::Continue);
    let s = app.ask.as_ref().unwrap();
    assert!(!s.streaming, "no second worker spawned");
    // Detach so the never-ending thread can be reaped on test exit.
    let _leaked = app.ask.as_mut().unwrap().thread.take();
}

#[test]
fn no_ask_state_returns_to_library() {
    let mut config = Config::defaults();
    config.storage.data_dir = "/tmp/kebab-tui-ask-tests-noop".into();
    let mut app = App::new(config).unwrap();
    app.focus = Pane::Ask;
    // ask slot intentionally None
    let outcome = handle_key_ask(
        &mut app,
        KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE),
    );
    assert_eq!(outcome, KeyOutcome::SwitchPane(Pane::Library));
}

// ── p9-fb-16: multi-turn conversation transcript ──────────────────────────

#[test]
fn ctrl_l_clears_conversation_state() {
    let mut app = fresh_app();
    {
        let s = app.ask.as_mut().unwrap();
        s.conversation_id = Some("conv_test".into());
        s.turns.push(Turn {
            question: "Q".into(),
            answer: "A".into(),
            citations: Vec::new(),
            created_at: OffsetDateTime::from_unix_timestamp(0).unwrap(),
        });
        s.last_answer = Some(make_answer(true, None, "A"));
        s.partial = "leftover".into();
        s.current_question = Some("in flight".into());
        s.scroll = 5;
        s.streaming = true;
        // Note: thread / rx 는 JoinHandle 인 만큼 직접 mock 어려움 —
        // streaming flag 만으로 detach side-effect 검증.
    }
    let outcome = handle_key_ask(
        &mut app,
        KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL),
    );
    assert_eq!(outcome, KeyOutcome::Continue);
    let s = app.ask.as_ref().unwrap();
    assert!(s.turns.is_empty(), "turns cleared");
    assert!(s.conversation_id.is_none(), "conversation_id cleared");
    assert!(s.last_answer.is_none(), "last_answer cleared");
    assert!(s.partial.is_empty(), "partial cleared");
    assert!(s.current_question.is_none(), "current_question cleared");
    assert_eq!(s.scroll, 0, "scroll reset");
    // 회차 1 fix: streaming flag + thread/rx 도 detach.
    assert!(!s.streaming, "streaming flag cleared");
    assert!(s.thread.is_none(), "thread detached");
    assert!(s.rx.is_none(), "rx detached");
}

#[test]
fn render_refusal_turn_in_transcript_uses_yellow_when_last_answer_ungrounded() {
    let mut app = fresh_app();
    {
        let s = app.ask.as_mut().unwrap();
        let mut ans = make_answer(false, Some(RefusalReason::ScoreGate), "REFUSED BODY");
        ans.citations.clear();
        s.turns.push(Turn {
            question: "Q".into(),
            answer: ans.answer.clone(),
            citations: Vec::new(),
            created_at: ans.created_at,
        });
        s.last_answer = Some(ans);
    }
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            let area = Rect::new(0, 0, 80, 24);
            render_ask(f, area, &app);
        })
        .unwrap();
    // Find the cell containing the first character of REFUSED BODY
    // and assert its fg is Yellow (the refusal-style override).
    let buffer = terminal.backend().buffer().clone();
    let mut found = None;
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            let cell = &buffer[(x, y)];
            if cell.symbol() == "R" {
                // First R after Q: line — likely the answer body.
                // Check fg color.
                if let ratatui::style::Color::Yellow = cell.fg {
                    found = Some((x, y));
                    break;
                }
            }
        }
        if found.is_some() {
            break;
        }
    }
    assert!(
        found.is_some(),
        "expected at least one yellow R cell from REFUSED BODY in the transcript"
    );
}

#[test]
fn render_transcript_shows_completed_turns_in_order() {
    let mut app = fresh_app();
    {
        let s = app.ask.as_mut().unwrap();
        let ts = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
        s.turns.push(Turn {
            question: "first question".into(),
            answer: "first answer".into(),
            citations: Vec::new(),
            created_at: ts,
        });
        s.turns.push(Turn {
            question: "second question".into(),
            answer: "second answer".into(),
            citations: Vec::new(),
            created_at: ts,
        });
    }
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            let area = Rect::new(0, 0, 80, 24);
            render_ask(f, area, &app);
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
    assert!(rendered.contains("Q1"), "Q1 marker rendered");
    assert!(rendered.contains("Q2"), "Q2 marker rendered");
    let q1_pos = rendered.find("Q1").unwrap();
    let q2_pos = rendered.find("Q2").unwrap();
    assert!(q1_pos < q2_pos, "chronological order: Q1 before Q2");
    assert!(rendered.contains("first question"), "first question text");
    assert!(rendered.contains("second answer"), "second answer text");
    assert!(rendered.contains("transcript (2 turns)"), "title shows count");
}

#[test]
fn render_streaming_inflight_turn_appears_below_completed_turns() {
    let mut app = fresh_app();
    {
        let s = app.ask.as_mut().unwrap();
        s.turns.push(Turn {
            question: "first".into(),
            answer: "ANSWERED".into(),
            citations: Vec::new(),
            created_at: OffsetDateTime::from_unix_timestamp(0).unwrap(),
        });
        s.streaming = true;
        s.current_question = Some("follow-up".into());
        s.partial = "PARTIAL".into();
    }
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            let area = Rect::new(0, 0, 80, 24);
            render_ask(f, area, &app);
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
    assert!(rendered.contains("ANSWERED"), "completed turn body");
    assert!(rendered.contains("PARTIAL"), "in-flight partial body");
    assert!(rendered.contains("▍"), "cursor block on in-flight turn");
    let answered_pos = rendered.find("ANSWERED").unwrap();
    let partial_pos = rendered.find("PARTIAL").unwrap();
    assert!(
        answered_pos < partial_pos,
        "completed turn before in-flight; got: {answered_pos} vs {partial_pos}"
    );
}

/// p9-fb-10: typing Hangul into Ask input advances cursor by 2
/// per char and round-trips through the buffer correctly.
#[test]
fn hangul_typing_in_ask_input_advances_cursor_by_two_per_char() {
    let mut app = fresh_app();
    // Switch to ask + INSERT mode so chars type as input.
    app.focus = Pane::Ask;
    app.mode = kebab_tui::Mode::auto_for(Pane::Ask);
    for ch in "한글".chars() {
        kebab_tui::handle_key_ask(
            &mut app,
            KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE),
        );
    }
    assert_eq!(app.ask.as_ref().unwrap().input.as_str(), "한글");
    assert_eq!(app.ask.as_ref().unwrap().input.cursor_col(), 4);
    kebab_tui::handle_key_ask(
        &mut app,
        KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
    );
    assert_eq!(app.ask.as_ref().unwrap().input.as_str(), "한");
    assert_eq!(app.ask.as_ref().unwrap().input.cursor_col(), 2);
}

// ── p9-fb-22: cursor mid-string editing in Ask input ──────────────────────

/// p9-fb-22 (issue #94): Left arrow rewinds the cursor; subsequent
/// Char insertion lands at that mid-string position (not at the end).
#[test]
fn left_arrow_then_typing_inserts_at_cursor_in_ask() {
    let mut app = fresh_app();
    app.mode = kebab_tui::Mode::Insert;
    for ch in "abc".chars() {
        handle_key_ask(&mut app, KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
    }
    handle_key_ask(&mut app, KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
    handle_key_ask(&mut app, KeyEvent::new(KeyCode::Char('X'), KeyModifiers::NONE));
    let s = app.ask.as_ref().unwrap();
    assert_eq!(s.input.as_str(), "abXc", "X inserts before c, not at end");
    assert_eq!(s.input.cursor_col(), 3, "cursor sits between X and c");
}

/// p9-fb-22 (issue #94): Right arrow at end of input is a no-op
/// (no overflow, no panic).
#[test]
fn right_arrow_at_end_is_noop_in_ask() {
    let mut app = fresh_app();
    app.mode = kebab_tui::Mode::Insert;
    handle_key_ask(&mut app, KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
    handle_key_ask(&mut app, KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
    let s = app.ask.as_ref().unwrap();
    assert_eq!(s.input.cursor_col(), 1);
}

/// p9-fb-22 (issue #94): Home jumps cursor to the start; End to
/// the end. Available regardless of mode.
#[test]
fn home_end_jump_cursor_in_ask() {
    let mut app = fresh_app();
    app.mode = kebab_tui::Mode::Insert;
    for ch in "hello".chars() {
        handle_key_ask(&mut app, KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
    }
    handle_key_ask(&mut app, KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
    assert_eq!(app.ask.as_ref().unwrap().input.cursor_col(), 0);
    handle_key_ask(&mut app, KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
    assert_eq!(app.ask.as_ref().unwrap().input.cursor_col(), 5);
}

/// p9-fb-22 (issue #94): Delete key at the cursor removes the next
/// char without rewinding the cursor.
#[test]
fn delete_key_removes_char_at_cursor_in_ask() {
    let mut app = fresh_app();
    app.mode = kebab_tui::Mode::Insert;
    for ch in "abc".chars() {
        handle_key_ask(&mut app, KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
    }
    handle_key_ask(&mut app, KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
    handle_key_ask(&mut app, KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
    let s = app.ask.as_ref().unwrap();
    assert_eq!(s.input.as_str(), "bc", "Delete removed the leading 'a'");
    assert_eq!(s.input.cursor_col(), 0, "cursor stayed at column 0");
}

/// p9-fb-22 (issue #94): Hangul + Left arrow rewinds by 2 display
/// columns (one wide char), keeping the byte boundary intact.
#[test]
fn hangul_left_arrow_rewinds_by_two_cols_in_ask() {
    let mut app = fresh_app();
    app.mode = kebab_tui::Mode::Insert;
    for ch in "한글".chars() {
        handle_key_ask(&mut app, KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
    }
    assert_eq!(app.ask.as_ref().unwrap().input.cursor_col(), 4);
    handle_key_ask(&mut app, KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
    assert_eq!(app.ask.as_ref().unwrap().input.cursor_col(), 2);
    // Inserting at the new cursor position lands between the two
    // syllables, proving cursor_col is not just a display annotation.
    handle_key_ask(&mut app, KeyEvent::new(KeyCode::Char('X'), KeyModifiers::NONE));
    assert_eq!(app.ask.as_ref().unwrap().input.as_str(), "한X글");
}

// ── p9-fb-22: follow-tail auto-scroll on new transcript content ───────────

/// p9-fb-22 (issue #95): a freshly constructed AskState defaults to
/// `follow_tail = true` so the first answer streams into view.
#[test]
fn ask_state_default_follow_tail_is_true() {
    let s = AskState::default();
    assert!(s.follow_tail, "follow_tail is on by default");
}

/// p9-fb-22 (issue #95): pressing `k` in Normal disengages follow-
/// tail so the user can review prior turns without the renderer
/// snapping back to the bottom on the next streamed token.
#[test]
fn k_disengages_follow_tail_in_ask() {
    let mut app = fresh_app();
    app.mode = kebab_tui::Mode::Normal;
    handle_key_ask(&mut app, KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE));
    assert!(!app.ask.as_ref().unwrap().follow_tail);
}

/// p9-fb-22 (issue #95): Shift-G jumps the transcript to the bottom
/// and re-engages follow-tail so subsequent streaming auto-scrolls
/// again.
#[test]
fn shift_g_re_engages_follow_tail_in_ask() {
    let mut app = fresh_app();
    app.mode = kebab_tui::Mode::Normal;
    {
        let s = app.ask.as_mut().unwrap();
        s.follow_tail = false;
        s.scroll = 7;
    }
    handle_key_ask(&mut app, KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT));
    let s = app.ask.as_ref().unwrap();
    assert!(s.follow_tail, "Shift-G re-engages follow-tail");
    assert_eq!(s.scroll, 0, "scroll cleared (renderer recomputes)");
}

/// p9-fb-22 (issue #95): Ctrl-L clears the conversation AND resets
/// follow_tail to true so the next submission auto-scrolls.
#[test]
fn ctrl_l_resets_follow_tail_in_ask() {
    let mut app = fresh_app();
    app.mode = kebab_tui::Mode::Normal;
    app.ask.as_mut().unwrap().follow_tail = false;
    handle_key_ask(&mut app, KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL));
    assert!(app.ask.as_ref().unwrap().follow_tail);
}

/// p9-fb-24: PgDn advances Ask scroll by `PAGE_STEP` (= 10) and
/// disengages follow-tail (matches `j` semantics — manual scroll =
/// freeze).
#[test]
fn page_down_advances_scroll_and_freezes_follow_tail_in_ask() {
    let mut app = fresh_app();
    app.mode = kebab_tui::Mode::Normal;
    let outcome = handle_key_ask(
        &mut app,
        KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE),
    );
    assert_eq!(outcome, KeyOutcome::Continue);
    let s = app.ask.as_ref().unwrap();
    assert_eq!(s.scroll, 10, "PgDn shifts scroll by PAGE_STEP");
    assert!(!s.follow_tail, "PgDn freezes follow_tail like j/k");
}

/// p9-fb-24: PgUp rewinds Ask scroll by `PAGE_STEP` (saturating at 0)
/// and disengages follow-tail.
#[test]
fn page_up_rewinds_scroll_saturating_and_freezes_follow_tail_in_ask() {
    let mut app = fresh_app();
    app.mode = kebab_tui::Mode::Normal;
    app.ask.as_mut().unwrap().scroll = 25;
    app.ask.as_mut().unwrap().follow_tail = true;
    handle_key_ask(
        &mut app,
        KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE),
    );
    let s = app.ask.as_ref().unwrap();
    assert_eq!(s.scroll, 15);
    assert!(!s.follow_tail);
    app.ask.as_mut().unwrap().scroll = 3;
    handle_key_ask(
        &mut app,
        KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE),
    );
    assert_eq!(app.ask.as_ref().unwrap().scroll, 0);
}

/// p9-fb-24: PgUp / PgDn fire from BOTH Insert and Normal modes
/// (physical keys, no typing ambiguity — same as Left/Right/Home/End
/// from p9-fb-22).
#[test]
fn page_keys_fire_from_insert_mode_in_ask() {
    let mut app = fresh_app();
    app.mode = kebab_tui::Mode::Insert;
    handle_key_ask(
        &mut app,
        KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE),
    );
    assert_eq!(app.ask.as_ref().unwrap().scroll, 10);
}

/// p9-fb-22 (issue #95): when follow_tail is on and the transcript
/// has many lines, the rendered buffer's last visible line includes
/// content from the tail of the answer (not the head).
#[test]
fn follow_tail_renders_tail_when_transcript_overflows() {
    let mut app = fresh_app();
    {
        let s = app.ask.as_mut().unwrap();
        // Stuff the transcript with 30 turns so the rendered viewport
        // (height 12 → ~9 inner rows after borders + bottom split)
        // can't show them all.
        for i in 0..30 {
            s.turns.push(Turn {
                question: format!("Q{i}"),
                answer: format!("A{i}-body-text"),
                citations: Vec::new(),
                created_at: OffsetDateTime::from_unix_timestamp(0).unwrap(),
            });
        }
        s.follow_tail = true;
    }
    let backend = TestBackend::new(60, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| render_ask(f, Rect::new(0, 0, 60, 20), &app))
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
    // The very last turn (Q29 / A29) must be visible somewhere in
    // the buffer — without follow-tail, the renderer would pin to
    // the top and only the first few turns would show.
    assert!(
        rendered.contains("A29-body-text"),
        "tail of transcript must be visible when follow_tail is on; got:\n{rendered}"
    );
}
