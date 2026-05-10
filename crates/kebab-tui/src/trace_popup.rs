//! p9-fb-37: TUI trace popup. Opens from Search pane via `t` key
//! when results are visible. Re-runs the current query with
//! `SearchOpts.trace = true` and displays the lex / vec / rrf union
//! + per-stage timing as a single scroll list.

use crossterm::event::{KeyCode, KeyEvent};
use kebab_core::SearchTrace;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

#[derive(Debug, Clone)]
pub struct TracePopupState {
    pub trace: SearchTrace,
    pub scroll: u16,
}

impl TracePopupState {
    pub fn new(trace: SearchTrace) -> Self {
        Self { trace, scroll: 0 }
    }
}

pub fn render_trace_popup(f: &mut Frame, area: Rect, state: &TracePopupState) {
    let mut lines: Vec<Line> = Vec::new();
    let bold = Style::default().add_modifier(Modifier::BOLD);

    lines.push(Line::from(Span::styled(
        format!(
            "Lexical ({} hits, {} ms)",
            state.trace.lexical.len(),
            state.trace.timing.lexical_ms,
        ),
        bold,
    )));
    for c in &state.trace.lexical {
        lines.push(Line::from(format!(
            "  #{:>2} score={:.4} chunk={}",
            c.rank, c.score, c.chunk_id.0
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!(
            "Vector ({} hits, {} ms)",
            state.trace.vector.len(),
            state.trace.timing.vector_ms,
        ),
        bold,
    )));
    for c in &state.trace.vector {
        lines.push(Line::from(format!(
            "  #{:>2} score={:.4} chunk={}",
            c.rank, c.score, c.chunk_id.0
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!(
            "RRF inputs ({} entries, {} ms fusion)",
            state.trace.rrf_inputs.len(),
            state.trace.timing.fusion_ms,
        ),
        bold,
    )));
    for e in &state.trace.rrf_inputs {
        lines.push(Line::from(format!(
            "  chunk={} lex={:?} vec={:?} fusion={:.4}",
            e.chunk_id.0, e.lexical_rank, e.vector_rank, e.fusion_score
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("Total: {} ms", state.trace.timing.total_ms),
        bold,
    )));

    let block = Block::default()
        .title("Trace — Esc to close, j/k or ↑↓ to scroll")
        .borders(Borders::ALL);
    let p = Paragraph::new(lines)
        .block(block)
        .scroll((state.scroll, 0))
        .wrap(Wrap { trim: false });
    f.render_widget(p, area);
}

/// Handle keys while popup is open. Returns true if the popup should close.
pub fn handle_key_trace_popup(state: &mut TracePopupState, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Esc => true,
        KeyCode::Char('j') | KeyCode::Down => {
            state.scroll = state.scroll.saturating_add(1);
            false
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.scroll = state.scroll.saturating_sub(1);
            false
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;
    use kebab_core::TraceTiming;

    fn dummy_state() -> TracePopupState {
        TracePopupState::new(SearchTrace {
            lexical: vec![],
            vector: vec![],
            rrf_inputs: vec![],
            timing: TraceTiming::default(),
        })
    }

    #[test]
    fn esc_closes() {
        let mut s = dummy_state();
        assert!(handle_key_trace_popup(
            &mut s,
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
        ));
    }

    #[test]
    fn j_scrolls_down() {
        let mut s = dummy_state();
        assert!(!handle_key_trace_popup(
            &mut s,
            KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
        ));
        assert_eq!(s.scroll, 1);
    }
}
