//! Unit + snapshot tests for the Ask pane (P9-3).
//!
//! Worker thread / streaming path is NOT exercised here — that would
//! require a real Ollama + SQLite KB. Tests drive the pane via
//! hand-populated `AskState`.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use kebab_config::Config;
use kebab_core::{
    Answer, AnswerCitation, AnswerRetrievalSummary, Citation, ModelRef,
    PromptTemplateVersion, RefusalReason, SearchMode, TokenUsage, TraceId, WorkspacePath,
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
    assert_eq!(app.ask.as_ref().unwrap().input, "hello");
}

#[test]
fn backspace_pops_input() {
    let mut app = fresh_app();
    {
        app.ask.as_mut().unwrap().input = "abcd".into();
    }
    handle_key_ask(
        &mut app,
        KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
    );
    assert_eq!(app.ask.as_ref().unwrap().input, "abc");
}

#[test]
fn e_toggles_explain_when_input_empty() {
    let mut app = fresh_app();
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
        app.ask.as_mut().unwrap().input = "qu".into();
    }
    handle_key_ask(
        &mut app,
        KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE),
    );
    let s = app.ask.as_ref().unwrap();
    assert_eq!(s.input, "que");
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
        s.input = "anything".into();
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
        s.input = "what is RRF fusion?".into();
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
        s.input = "test".into();
        s.answer = Some(make_answer(true, None, "test answer body [1]."));
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
        s.answer = Some(ans);
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
        s.answer = Some(make_answer(true, None, "answer body."));
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
        s.input = "another question".into();
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
