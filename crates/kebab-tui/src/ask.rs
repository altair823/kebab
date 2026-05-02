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
use ratatui::style::{Color, Modifier, Style};
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

    render_input(f, layout[0], s);
    render_answer(f, layout[1], s);
    render_bottom(f, layout[2], s);
}

fn render_input(f: &mut Frame, area: Rect, s: &AskState) {
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
        Span::styled("? ", Style::default().fg(Color::Cyan)),
        Span::raw(s.input.as_str()),
        Span::styled(mode_badge, Style::default().fg(Color::Yellow)),
        Span::styled(busy, Style::default().add_modifier(Modifier::DIM)),
    ]);
    let block = Block::default()
        .title("ask (Enter=submit  e=explain  Esc=back)")
        .borders(Borders::ALL);
    f.render_widget(Paragraph::new(line).block(block), area);
}

fn render_answer(f: &mut Frame, area: Rect, s: &AskState) {
    let block = Block::default().title("answer").borders(Borders::ALL);

    if s.streaming {
        // Mid-stream: show partial + cursor block.
        let mut content = s.partial.clone();
        content.push('▍');
        let para = Paragraph::new(content)
            .wrap(Wrap { trim: false })
            .scroll((s.scroll, 0));
        f.render_widget(para.block(block), area);
        return;
    }

    if let Some(answer) = &s.answer {
        // Style refusal answers (grounded=false) in yellow so the user
        // immediately spots they're not getting a sourced answer.
        let style = if answer.grounded {
            Style::default()
        } else {
            Style::default().fg(Color::Yellow)
        };
        let para = Paragraph::new(Span::styled(answer.answer.as_str(), style))
            .wrap(Wrap { trim: false })
            .scroll((s.scroll, 0));
        f.render_widget(para.block(block), area);
        return;
    }

    // No question yet.
    let hint = Paragraph::new(Span::styled(
        "(type a question and press Enter to ask. RAG answers stream token-by-token.)",
        Style::default().add_modifier(Modifier::DIM),
    ))
    .wrap(Wrap { trim: false });
    f.render_widget(hint.block(block), area);
}

fn render_bottom(f: &mut Frame, area: Rect, s: &AskState) {
    let split = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(area);
    render_status(f, split[0], s);
    render_citations_or_explain(f, split[1], s);
}

fn render_status(f: &mut Frame, area: Rect, s: &AskState) {
    let block = Block::default().title("status").borders(Borders::ALL);
    let lines: Vec<Line> = match &s.answer {
        None => vec![Line::from(Span::styled(
            "(no answer yet)",
            Style::default().add_modifier(Modifier::DIM),
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

fn render_citations_or_explain(f: &mut Frame, area: Rect, s: &AskState) {
    let title = if s.explain { "explain (per-claim)" } else { "citations" };
    let block = Block::default().title(title).borders(Borders::ALL);
    let lines: Vec<Line> = match &s.answer {
        None => vec![Line::from(Span::styled(
            "(submit a question to see citations)",
            Style::default().add_modifier(Modifier::DIM),
        ))],
        Some(a) if a.citations.is_empty() => vec![Line::from(Span::styled(
            if a.grounded { "(no citations)" } else { "(가까운 후보 없음)" },
            Style::default().add_modifier(Modifier::DIM),
        ))],
        Some(a) => a
            .citations
            .iter()
            .map(|c| {
                let marker = c.marker.as_deref().unwrap_or("?");
                Line::from(vec![
                    Span::styled(
                        format!("[{marker}] "),
                        Style::default().fg(Color::Cyan),
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
        (KeyCode::Esc, _) => {
            // Best-effort cancellation per spec — worker keeps running
            // but its result is dropped. Detach by clearing rx /
            // thread; the JoinHandle Drop on later replacement will
            // not block (we never `join` from this path).
            let s = state.ask.as_mut().unwrap();
            s.rx = None;
            s.thread = None;
            s.streaming = false;
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
                .map(|s| s.streaming || s.thread.is_some() || s.input.trim().is_empty())
                .unwrap_or(true)
            {
                return KeyOutcome::Continue;
            }
            spawn_ask_worker(state);
            KeyOutcome::Continue
        }
        // `e` only as a plain (no-modifier) press — typing 'e' in a
        // word like "explain" must still reach the input buffer.
        // The spec lists `e` as the explain-toggle; we apply the same
        // SHIFT-aware convention as P9-2's `g` jump.
        (KeyCode::Char('e'), KeyModifiers::NONE) => {
            // Ambiguity with typing — distinguish via empty input as
            // a heuristic: when input is empty, `e` toggles; while
            // typing, `e` reaches the buffer. Vim users will recognise
            // this "command vs insert" split applied at the keystroke
            // level.
            let s = state.ask.as_mut().unwrap();
            if s.input.is_empty() {
                s.explain = !s.explain;
                KeyOutcome::Continue
            } else {
                s.input.push('e');
                KeyOutcome::Continue
            }
        }
        (KeyCode::Char('j'), KeyModifiers::NONE) => {
            let s = state.ask.as_mut().unwrap();
            if s.input.is_empty() {
                s.scroll = s.scroll.saturating_add(1);
            } else {
                s.input.push('j');
            }
            KeyOutcome::Continue
        }
        (KeyCode::Char('k'), KeyModifiers::NONE) => {
            let s = state.ask.as_mut().unwrap();
            if s.input.is_empty() {
                s.scroll = s.scroll.saturating_sub(1);
            } else {
                s.input.push('k');
            }
            KeyOutcome::Continue
        }
        (KeyCode::Backspace, _) => {
            let s = state.ask.as_mut().unwrap();
            s.input.pop();
            KeyOutcome::Continue
        }
        (KeyCode::Char(c), _) => {
            let s = state.ask.as_mut().unwrap();
            s.input.push(c);
            KeyOutcome::Continue
        }
        _ => KeyOutcome::Continue,
    }
}

fn spawn_ask_worker(state: &mut App) {
    let (tx, rx) = mpsc::channel::<String>();
    let cfg = state.config.clone();
    let s = state.ask.as_mut().unwrap();
    let query = s.input.clone();
    let explain = s.explain;
    s.partial.clear();
    s.answer = None;
    s.streaming = true;
    s.scroll = 0;
    s.rx = Some(rx);

    let opts = kebab_app::AskOpts {
        k: 0, // facade clamps to config.search.default_k floor
        explain,
        mode: kebab_core::SearchMode::Hybrid,
        temperature: None,
        seed: None,
        stream_sink: Some(tx),
        // p9-fb-15: TUI ask is single-shot in this task; multi-turn
        // conversation UI lands in p9-fb-16.
        history: Vec::new(),
        conversation_id: None,
        turn_index: None,
    };
    let handle =
        thread::spawn(move || kebab_app::ask_with_config(cfg, &query, opts));
    s.thread = Some(handle);
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
            // Final partial is the full answer text; replace partial
            // with the canonical answer.answer so post-stream rendering
            // is identical regardless of stream pacing.
            s.partial.clear();
            s.answer = Some(answer);
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
