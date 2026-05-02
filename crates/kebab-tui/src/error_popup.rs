//! Error popup overlay — rendered on top of any pane when the last
//! facade call returned `Err`. Any key dismisses (handled by the
//! pane's key handler before its own dispatch).

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

/// Captured snapshot of an `anyhow::Error` for rendering. We do NOT
/// store the `anyhow::Error` itself (it is `!Sync` in pre-1.0.99
/// versions on some toolchains and would force lifetime gymnastics
/// on `App`); we render the formatted chain at capture time.
#[derive(Clone, Debug)]
pub struct ErrorOverlay {
    pub title: String,
    /// Each chain link as a separate line, root-cause last.
    pub chain: Vec<String>,
}

impl ErrorOverlay {
    pub fn from_anyhow(err: &anyhow::Error) -> Self {
        let chain: Vec<String> = err.chain().map(|c| c.to_string()).collect();
        Self {
            title: "error".to_string(),
            chain,
        }
    }

    pub fn from_message(title: impl Into<String>, msg: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            chain: vec![msg.into()],
        }
    }
}

/// Render the popup centred in `area`. Caller is responsible for
/// clearing the underlying region (`Clear` widget); we do that here.
pub fn render_error_overlay(f: &mut Frame, area: Rect, overlay: &ErrorOverlay) {
    let popup_area = centered_rect(area, 60, 50);
    f.render_widget(Clear, popup_area);

    let mut lines: Vec<Line> = Vec::with_capacity(overlay.chain.len() + 2);
    lines.push(Line::from(Span::styled(
        format!("{}: {}", overlay.title, overlay.chain.first().map_or("(unknown)", String::as_str)),
        Style::default()
            .fg(Color::Red)
            .add_modifier(Modifier::BOLD),
    )));
    for cause in overlay.chain.iter().skip(1) {
        lines.push(Line::from(format!("  caused by: {cause}")));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "press any key to dismiss",
        Style::default().add_modifier(Modifier::DIM),
    )));

    let block = Block::default()
        .title("error")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red));
    let para = Paragraph::new(lines).block(block).wrap(Wrap { trim: false });
    f.render_widget(para, popup_area);
}

fn centered_rect(area: Rect, percent_x: u16, percent_y: u16) -> Rect {
    let w = (area.width * percent_x / 100).max(20).min(area.width);
    let h = (area.height * percent_y / 100).max(5).min(area.height);
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    Rect::new(x, y, w, h)
}
