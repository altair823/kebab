//! p9-fb-13: cheatsheet popup (`F1` toggle).
//!
//! Modal overlay listing every key binding the active pane responds
//! to, plus the global mode toggles (`i`/`Esc`). Triggered with
//! `F1` (universal help key — no collision with the existing Library
//! `?` binding, which already opens the Ask pane). `F1` or `Esc`
//! while the popup is visible closes it.
//!
//! Spec p9-fb-13 lists `?` as the trigger and a verb-form hint line
//! above the status bar. Both are deferred:
//!
//! * `?` would clobber Library's quick-Ask binding (`Char('?') →
//!   SwitchPane(Ask)`). We swap to `F1` per HOTFIXES — common help
//!   key, no rebinding needed.
//! * The verb hint line redesign sits in the existing `render_footer`
//!   path; the per-pane string already serves the same role. A
//!   future PR can split it into mode-aware verb fragments.
//!
//! **Maintenance**: the `push_section(...)` calls below hold every
//! key binding as a literal string — there is NO automated link
//! from `handle_key_*` to the cheatsheet entries. A future PR that
//! changes a binding (e.g. swap `r` → `R` for ingest) MUST update
//! the matching entry here. Drift would be silently invisible
//! (the cheatsheet still renders, but lies about the live key).

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::app::{App, Pane};
use crate::theme::{Role, Theme};

/// Render the cheatsheet popup, centered on `area` with a 70% / 60%
/// box (matches the error overlay's footprint so the visual rhythm
/// is consistent). The body is one section per pane plus the global
/// toggles.
pub fn render_cheatsheet(f: &mut Frame, area: Rect, app: &App) {
    // p9-fb-21: bumped from 60% → 75% height so the Inspect section
    // (last in the list) still fits after Search + Ask each gained
    // one row (`o` inspect + `i` Insert toggle).
    let popup_area = centered_rect(area, 70, 75);
    f.render_widget(Clear, popup_area);

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        "kebab TUI — keymap (F1 / Esc to close)",
        app.theme.style(Role::Heading).add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));

    push_section(
        &mut lines,
        &app.theme,
        "Global",
        &[
            ("i", "Normal → Insert (every pane — p9-fb-21)"),
            ("Esc", "Insert → Normal (any pane)"),
            ("F1", "toggle this cheatsheet"),
            ("Tab / Shift-Tab", "(future) cycle pane"),
        ],
    );

    push_section(
        &mut lines,
        &app.theme,
        "Library",
        &[
            ("j / k", "move selection (Normal)"),
            ("gg / G", "top / bottom"),
            ("f", "filter overlay"),
            ("/", "switch to Search"),
            ("?", "switch to Ask"),
            ("Enter", "inspect selected doc"),
            ("r", "background ingest"),
            ("q", "quit"),
        ],
    );

    push_section(
        &mut lines,
        &app.theme,
        "Search",
        &[
            ("type", "query (Insert)"),
            ("Tab", "cycle search mode (lexical / vector / hybrid)"),
            ("Enter", "force search now (skip debounce)"),
            ("j / k", "move selection (Normal)"),
            ("← / →", "move cursor in query (p9-fb-22)"),
            ("Home / End", "cursor to start / end of query"),
            ("Delete", "remove char at cursor"),
            ("g", "open hit's citation in $EDITOR (Normal)"),
            (
                "o",
                "inspect selected hit's chunk (Normal — was `i` pre-fb-21)",
            ),
            ("t", "open retrieval trace popup (Normal — p9-fb-37)"),
            ("i", "Normal → Insert (toggle back to typing)"),
            ("Esc", "back to Library"),
        ],
    );

    push_section(
        &mut lines,
        &app.theme,
        "Ask",
        &[
            ("type", "question (Insert)"),
            ("Enter", "submit"),
            ("e", "toggle explain mode (Normal)"),
            (
                "F2",
                "toggle multi-hop pipeline (p9-fb-41 — affects next submission)",
            ),
            ("j / k", "scroll transcript (Normal — disengages auto-tail)"),
            ("Shift-G", "jump to bottom + re-engage auto-tail (p9-fb-22)"),
            (
                "PgUp / PgDn",
                "page-scroll the transcript (p9-fb-24, disengages auto-tail)",
            ),
            ("← / →", "move cursor in input (p9-fb-22)"),
            ("Home / End", "cursor to start / end of input"),
            ("Delete", "remove char at cursor"),
            ("i", "Normal → Insert (toggle back to typing)"),
            ("Ctrl-L", "new conversation (clears turns)"),
            ("Esc", "back to Library (cancels in-flight worker)"),
        ],
    );

    push_section(
        &mut lines,
        &app.theme,
        "Inspect",
        &[
            ("j / k", "scroll lines"),
            ("PgUp / PgDn", "scroll pages"),
            ("c", "collapse / expand all sections"),
            ("Esc / q", "back to originating pane"),
        ],
    );

    // Pane footer: which pane is currently focused (helps the
    // reader correlate \"the keys above\" with their current
    // context).
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("(currently focused: {})", pane_label(app.focus)),
        app.theme.style(Role::Hint),
    )));

    let block = Block::default()
        .title("? cheatsheet")
        .borders(Borders::ALL)
        .border_style(app.theme.style(Role::Heading));
    let para = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    f.render_widget(para, popup_area);
}

fn push_section(
    lines: &mut Vec<Line<'static>>,
    theme: &Theme,
    name: &'static str,
    keys: &[(&'static str, &'static str)],
) {
    lines.push(Line::from(Span::styled(
        name,
        theme.style(Role::Heading).add_modifier(Modifier::BOLD),
    )));
    for (key, desc) in keys {
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(format!("{key:<18}"), theme.style(Role::CitationMarker)),
            Span::raw("  "),
            Span::raw(desc.to_string()),
        ]));
    }
    lines.push(Line::from(""));
}

fn pane_label(p: Pane) -> &'static str {
    match p {
        Pane::Library => "Library",
        Pane::Search => "Search",
        Pane::Ask => "Ask",
        Pane::Inspect => "Inspect",
        Pane::Jobs => "Jobs",
    }
}

fn centered_rect(area: Rect, percent_x: u16, percent_y: u16) -> Rect {
    let w = (area.width * percent_x / 100).max(40).min(area.width);
    let h = (area.height * percent_y / 100).max(10).min(area.height);
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    Rect::new(x, y, w, h)
}
