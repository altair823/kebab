//! Ask pane (P9-3).
//!
//! Streaming RAG answers in the TUI. Worker thread calls
//! `kebab-app::ask_with_config` with `AskOpts.stream_sink: Some(tx)`;
//! the pane keeps the matching `rx` and drains it once per render
//! frame so the answer area updates token-by-token without
//! blocking the event loop.
//!
//! Spec deviation (HOTFIXES `2026-05-02 P9-3`):
//! - `render_ask<B: Backend>` generic dropped (ratatui 0.28 Frame is
//!   backend-agnostic — same as P9-1 / P9-2).
//!
//! Per design §1.1–§1.4 (ask scenes), §2.3 (Answer wire), §3.8
//! (`Answer`).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use kebab_core::{RefusalReason, SearchMode};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use std::sync::mpsc;
use std::thread;

use crate::app::{App, AskState, KeyOutcome, Pane};

/// Render the Ask pane. Layout:
/// - top input bar
/// - middle answer area (scrollable when content overflows)
/// - bottom split: status (left) + citations / explain panel (right)
pub fn render_ask(f: &mut Frame, area: Rect, state: &App) {
    let Some(s) = state.ask.as_ref() else {
        f.render_widget(Block::default().title("Ask").borders(Borders::ALL), area);
        return;
    };

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(7),
        ])
        .split(area);

    render_input(f, layout[0], s, &state.theme);
    render_answer(f, layout[1], s, &state.theme);
    render_bottom(f, layout[2], s, &state.theme);
}

fn render_input(f: &mut Frame, area: Rect, s: &AskState, theme: &crate::theme::Theme) {
    const PROMPT: &str = "? ";

    let mode_badge = if s.explain { " explain" } else { "" };
    // Distinguish three async states for the operator:
    // - currently streaming (worker still emitting tokens)
    // - prior worker detached (Esc-cancelled, no rx attached but
    //   thread has not finished yet — Enter is blocked until it ends)
    // - idle
    let busy = if s.streaming {
        "  streaming…"
    } else if s.thread.is_some() {
        "  awaiting prior answer (Enter blocked)"
    } else {
        ""
    };
    let line = Line::from(vec![
        Span::styled(PROMPT, theme.style(crate::theme::Role::Heading)),
        Span::raw(s.input.as_str()),
        Span::styled(mode_badge, theme.style(crate::theme::Role::Warning)),
        Span::styled(busy, theme.style(crate::theme::Role::Hint)),
    ]);
    let block = Block::default()
        .title("ask (Enter=submit  e=explain  Ctrl-L=new conversation  Esc=back)")
        .borders(Borders::ALL);
    let inner = block.inner(area);
    let paragraph = Paragraph::new(line).block(block);
    f.render_widget(paragraph, area);

    // p9-fb-10: ratatui calls show_cursor + MoveTo whenever
    // cursor_position is Some (our case here). When a render fn
    // omits set_cursor_position (Library/Inspect), ratatui calls
    // hide_cursor instead. So this single call both positions and
    // unhides the caret for the Ask input column.
    let prompt_w = crate::input::display_width(PROMPT) as u16;
    let raw_x = inner.x + prompt_w + s.input.cursor_col() as u16;
    let cursor_x = raw_x.min(inner.x + inner.width.saturating_sub(1));
    f.set_cursor_position((cursor_x, inner.y));
}

fn render_answer(f: &mut Frame, area: Rect, s: &AskState, theme: &crate::theme::Theme) {
    let title = if s.turns.is_empty() && !s.streaming {
        "transcript".to_string()
    } else {
        let count = s.turns.len() + if s.streaming { 1 } else { 0 };
        format!("transcript ({} turn{})", count, if count == 1 { "" } else { "s" })
    };
    let block = Block::default().title(title).borders(Borders::ALL);

    // p9-fb-16: render the full conversation as Q/A pairs.
    // Completed turns first (chronological), then the in-flight
    // turn (if any) at the bottom. The most-recent completed
    // turn's grounded flag (from `last_answer`) styles its A line
    // via the theme's Warning role on refusal so the user keeps
    // the P9-3 visual distinction even inside the transcript.
    let last_turn_grounded = s.last_answer.as_ref().map(|a| a.grounded);
    let last_turn_idx = s.turns.len().saturating_sub(1);
    let mut lines: Vec<Line> = Vec::new();
    for (idx, turn) in s.turns.iter().enumerate() {
        let role_override = if idx == last_turn_idx {
            last_turn_grounded.and_then(|g| {
                if g {
                    None
                } else {
                    Some(crate::theme::Role::Warning)
                }
            })
        } else {
            None
        };
        push_turn_lines(
            &mut lines,
            idx,
            &turn.question,
            &turn.answer,
            false,
            role_override,
            theme,
        );
        lines.push(Line::raw(""));
    }

    if s.streaming {
        let q = s.current_question.as_deref().unwrap_or("");
        let mut a = s.partial.clone();
        a.push('▍');
        let idx = s.turns.len();
        push_turn_lines(&mut lines, idx, q, &a, true, None, theme);
    }

    if lines.is_empty() {
        let hint = Paragraph::new(Span::styled(
            "(type a question and press Enter. follow-ups inherit history. Ctrl-L clears the conversation.)",
            theme.style(crate::theme::Role::Hint),
        ))
        .wrap(Wrap { trim: false });
        f.render_widget(hint.block(block), area);
        return;
    }

    let para = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .scroll((s.scroll, 0));
    f.render_widget(para.block(block), area);
}

fn push_turn_lines(
    out: &mut Vec<Line<'static>>,
    idx: usize,
    question: &str,
    answer: &str,
    streaming: bool,
    answer_role_override: Option<crate::theme::Role>,
    theme: &crate::theme::Theme,
) {
    let q_label = format!("Q{}", idx + 1);
    let a_label = format!("A{}", idx + 1);
    out.push(Line::from(vec![
        // `Role::Heading` already includes BOLD in both palettes, so
        // no need to `add_modifier(BOLD)` here — the redundancy would
        // imply Heading lacks BOLD elsewhere.
        Span::styled(q_label, theme.style(crate::theme::Role::Heading)),
        Span::raw(": "),
        Span::raw(question.to_string()),
    ]));
    // p9-fb-11: render markdown (bold/italic/code/list/heading) when
    // the answer is in a normal/grounded state. For refusal (Warning
    // override) and streaming (Hint), force plain styled rendering so
    // the role color stays visible — markdown styling on top would
    // mask the "this is a refusal" / "this is in flight" signal.
    let a_label_span = Span::styled(
        a_label,
        theme
            .style(crate::theme::Role::Success)
            .add_modifier(Modifier::BOLD),
    );
    if let Some(role) = answer_role_override {
        out.push(Line::from(vec![
            a_label_span,
            Span::raw(": "),
            Span::styled(answer.to_string(), theme.style(role)),
        ]));
    } else if streaming {
        out.push(Line::from(vec![
            a_label_span,
            Span::raw(": "),
            Span::styled(answer.to_string(), theme.style(crate::theme::Role::Hint)),
        ]));
    } else {
        // Grounded answer: split A label onto its own marker line, then
        // append markdown-rendered body lines indented two spaces (so
        // the transcript stays readable when the answer wraps).
        out.push(Line::from(vec![a_label_span, Span::raw(":")]));
        for body_line in crate::markdown::render(answer, theme) {
            let mut spans: Vec<Span<'static>> = Vec::with_capacity(body_line.spans.len() + 1);
            spans.push(Span::raw("  "));
            spans.extend(body_line.spans);
            out.push(Line::from(spans));
        }
    }
}

fn render_bottom(f: &mut Frame, area: Rect, s: &AskState, theme: &crate::theme::Theme) {
    let split = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(area);
    render_status(f, split[0], s, theme);
    render_citations_or_explain(f, split[1], s, theme);
}

fn render_status(f: &mut Frame, area: Rect, s: &AskState, theme: &crate::theme::Theme) {
    let block = Block::default().title("status").borders(Borders::ALL);
    let lines: Vec<Line> = match &s.last_answer {
        None => vec![Line::from(Span::styled(
            "(no answer yet)",
            theme.style(crate::theme::Role::Hint),
        ))],
        Some(a) => {
            let grounded = if a.grounded { "✓" } else { "✗" };
            let mode = match a.retrieval.mode {
                SearchMode::Lexical => "lexical",
                SearchMode::Vector => "vector",
                SearchMode::Hybrid => "hybrid",
            };
            let refusal = match a.refusal_reason {
                Some(RefusalReason::ScoreGate) => "  refusal=score_gate",
                Some(RefusalReason::LlmSelfJudge) => "  refusal=llm_self_judge",
                Some(RefusalReason::NoIndex) => "  refusal=no_index",
                Some(RefusalReason::NoChunks) => "  refusal=no_chunks",
                Some(RefusalReason::LlmStreamAborted) => "  refusal=llm_stream_aborted",
                None => "",
            };
            vec![
                Line::from(format!("grounded {grounded}  model {}", a.model.id)),
                Line::from(format!("prompt {}  mode {mode}", a.prompt_template_version.0)),
                Line::from(format!(
                    "k={}  used={}/{}{refusal}",
                    a.retrieval.k, a.retrieval.chunks_used, a.retrieval.chunks_returned
                )),
            ]
        }
    };
    f.render_widget(Paragraph::new(lines).block(block), area);
}

fn render_citations_or_explain(f: &mut Frame, area: Rect, s: &AskState, theme: &crate::theme::Theme) {
    let title = if s.explain { "explain (per-claim)" } else { "citations" };
    let block = Block::default().title(title).borders(Borders::ALL);
    let lines: Vec<Line> = match &s.last_answer {
        None => vec![Line::from(Span::styled(
            "(submit a question to see citations)",
            theme.style(crate::theme::Role::Hint),
        ))],
        Some(a) if a.citations.is_empty() => vec![Line::from(Span::styled(
            if a.grounded { "(no citations)" } else { "(가까운 후보 없음)" },
            theme.style(crate::theme::Role::Hint),
        ))],
        Some(a) => a
            .citations
            .iter()
            .map(|c| {
                let marker = c.marker.as_deref().unwrap_or("?");
                Line::from(vec![
                    Span::styled(
                        format!("[{marker}] "),
                        theme.style(crate::theme::Role::CitationMarker),
                    ),
                    Span::raw(c.citation.to_uri()),
                ])
            })
            .collect(),
    };
    let para = Paragraph::new(lines).wrap(Wrap { trim: false });
    f.render_widget(para.block(block), area);
}

/// Ask pane key dispatch. Submission spawns a worker thread that
/// drives `kebab-app::ask_with_config` with `stream_sink: Some(tx)`.
pub fn handle_key_ask(state: &mut App, key: KeyEvent) -> KeyOutcome {
    if state.error_overlay.is_some() {
        state.error_overlay = None;
        return KeyOutcome::Continue;
    }
    if state.ask.is_none() {
        return KeyOutcome::SwitchPane(Pane::Library);
    }

    match (key.code, key.modifiers) {
        // p9-fb-16: Ctrl-L clears the in-pane conversation (turns +
        // conversation_id). Doesn't kill the in-flight worker — that
        // turn still finishes and its result is silently discarded
        // (joined into a new conversation that didn't exist when the
        // worker was spawned). Behaviour mirrors `:new` slash command.
        (KeyCode::Char('l'), m) if m.contains(KeyModifiers::CONTROL) => {
            let s = state.ask.as_mut().unwrap();
            s.turns.clear();
            s.conversation_id = None;
            s.last_answer = None;
            s.partial.clear();
            s.current_question = None;
            s.scroll = 0;
            // p9-fb-16: detach the in-flight worker so its eventual
            // result does NOT graduate into the new conversation as
            // a stale Turn. JoinHandle Drop on `None` assignment is
            // the same detach pattern P9-3 uses for Esc cancel —
            // worker keeps running in the background, finishes its
            // SQLite `answers` write (the failed-conv attempt is
            // preserved on disk), TUI ignores the result.
            s.thread = None;
            s.rx = None;
            s.streaming = false;
            KeyOutcome::Continue
        }
        (KeyCode::Esc, _) => {
            // Best-effort cancellation per spec — worker keeps running
            // but its result is dropped. Detach by clearing rx /
            // thread; the JoinHandle Drop on later replacement will
            // not block (we never `join` from this path).
            let s = state.ask.as_mut().unwrap();
            s.rx = None;
            s.thread = None;
            s.streaming = false;
            s.current_question = None;
            KeyOutcome::SwitchPane(Pane::Library)
        }
        (KeyCode::Enter, _) => {
            // Submission gates:
            // - empty input → no-op
            // - already streaming → no-op (same worker is in flight)
            // - prior worker still attached (e.g. user pressed Esc
            //   then re-entered Ask before that thread finished) →
            //   no-op. Otherwise the new worker would race the
            //   detached one against the same Ollama endpoint and
            //   the stream output would interleave.
            if state
                .ask
                .as_ref()
                .map(|s| s.streaming || s.thread.is_some() || s.input.as_str().trim().is_empty())
                .unwrap_or(true)
            {
                return KeyOutcome::Continue;
            }
            spawn_ask_worker(state);
            KeyOutcome::Continue
        }
        // p9-fb-12 follow-up: `e` / `j` / `k` are mode-gated. Normal
        // mode → toggle explain / scroll up/down. Insert mode → typed
        // into input buffer. The pre-fb-12 input-empty heuristic
        // ("if input.is_empty() then command else type") is gone —
        // Mode is authoritative.
        (KeyCode::Char('e'), KeyModifiers::NONE) if state.mode == crate::app::Mode::Normal => {
            let s = state.ask.as_mut().unwrap();
            s.explain = !s.explain;
            KeyOutcome::Continue
        }
        (KeyCode::Char('j'), KeyModifiers::NONE) if state.mode == crate::app::Mode::Normal => {
            let s = state.ask.as_mut().unwrap();
            s.scroll = s.scroll.saturating_add(1);
            KeyOutcome::Continue
        }
        (KeyCode::Char('k'), KeyModifiers::NONE) if state.mode == crate::app::Mode::Normal => {
            let s = state.ask.as_mut().unwrap();
            s.scroll = s.scroll.saturating_sub(1);
            KeyOutcome::Continue
        }
        (KeyCode::Backspace, _) => {
            let s = state.ask.as_mut().unwrap();
            s.input.pop_char();
            KeyOutcome::Continue
        }
        // Insert mode: every non-chord Char (incl. e/j/k) types into
        // input. CTRL/ALT chords stay reserved.
        (KeyCode::Char(c), m)
            if state.mode == crate::app::Mode::Insert
                && !m.contains(KeyModifiers::CONTROL)
                && !m.contains(KeyModifiers::ALT) =>
        {
            let s = state.ask.as_mut().unwrap();
            s.input.push_char(c);
            KeyOutcome::Continue
        }
        // Normal mode + un-handled Char → no-op (no typing in Normal).
        _ => KeyOutcome::Continue,
    }
}

fn spawn_ask_worker(state: &mut App) {
    let (tx, rx) = mpsc::channel::<String>();
    let cfg = state.config.clone();
    let s = state.ask.as_mut().unwrap();
    // p9-fb-10: take() consumes the input in one step (no clone +
    // clear). The buffer is left empty with cursor at 0.
    let query = s.input.take();
    let explain = s.explain;
    s.partial.clear();
    s.last_answer = None;
    s.streaming = true;
    s.scroll = 0;
    s.rx = Some(rx);
    // p9-fb-16: graduate the typed input into the in-flight turn,
    // clear the input box, ensure conversation_id exists, snapshot
    // history for the worker.
    s.current_question = Some(query.clone());
    if s.conversation_id.is_none() {
        s.conversation_id = Some(make_conversation_id());
    }
    let conversation_id = s.conversation_id.clone().unwrap();
    let turn_index = u32::try_from(s.turns.len()).unwrap_or(u32::MAX);
    let history = s.turns.clone();

    let opts = kebab_app::AskOpts {
        k: 0, // facade clamps to config.search.default_k floor
        explain,
        mode: kebab_core::SearchMode::Hybrid,
        temperature: None,
        seed: None,
        stream_sink: Some(tx),
        history,
        conversation_id: Some(conversation_id),
        turn_index: Some(turn_index),
    };
    let handle =
        thread::spawn(move || kebab_app::ask_with_config(cfg, &query, opts));
    s.thread = Some(handle);
}

/// Generate a fresh conversation_id. Timestamp-based — unique per
/// session, not cryptographic. spec p9-fb-16 calls for blake3 of
/// (first_question + ts) but the only guarantee we need is
/// per-session uniqueness; nanosecond ts hex is enough.
fn make_conversation_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("conv_{:032x}", nanos)
}

/// Run-loop hook: drain the streaming channel into `partial`. Called
/// on every render frame so the answer area updates as tokens arrive.
pub(crate) fn drain_stream(state: &mut App) {
    let Some(s) = state.ask.as_mut() else { return };
    if let Some(rx) = &s.rx {
        for tok in rx.try_iter() {
            s.partial.push_str(&tok);
        }
    }
}

/// Run-loop hook: poll the worker thread for completion. When the
/// thread finishes, populate `answer` and clear `streaming`.
pub(crate) fn poll_worker(state: &mut App) {
    let Some(s) = state.ask.as_mut() else { return };
    let finished = s
        .thread
        .as_ref()
        .map(|h| h.is_finished())
        .unwrap_or(false);
    if !finished {
        return;
    }
    let handle = s.thread.take().expect("just confirmed Some");
    let result = handle.join();
    s.streaming = false;
    s.rx = None;
    match result {
        Ok(Ok(answer)) => {
            // p9-fb-16: graduate the in-flight (current_question +
            // partial / answer) into a completed Turn appended to
            // `turns`. Next submission's spawn_ask_worker reads
            // `turns` as history and stamps turn_index.
            let question = s.current_question.take().unwrap_or_default();
            s.partial.clear();
            let turn = kebab_core::Turn {
                question,
                answer: answer.answer.clone(),
                citations: answer.citations.clone(),
                created_at: answer.created_at,
            };
            s.turns.push(turn);
            s.last_answer = Some(answer);
        }
        Ok(Err(e)) => {
            s.last_error = Some(format!("{e:#}"));
            state.error_overlay =
                Some(crate::error_popup::ErrorOverlay::from_anyhow(&e));
        }
        Err(panic_payload) => {
            let msg = panic_payload
                .downcast_ref::<&str>()
                .map(|s| (*s).to_string())
                .or_else(|| panic_payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "ask worker panicked".to_string());
            s.last_error = Some(msg.clone());
            state.error_overlay =
                Some(crate::error_popup::ErrorOverlay::from_message(
                    "ask worker panic",
                    msg,
                ));
        }
    }
}

/// Test-only helper. The pane's worker spawns a real `ask_with_config`
/// thread which would touch SQLite + LanceDB + Ollama. Tests bypass it
/// by hand-populating `AskState` and asserting render / key handler
/// behavior directly.
#[cfg(any(test, doc))]
#[allow(dead_code)]
pub(crate) fn debug_partial(state: &App) -> Option<&str> {
    state.ask.as_ref().map(|s| s.partial.as_str())
}
