//! Library pane — list + filter + key dispatch.
//!
//! State / render / key handler are kept in one module so the slot
//! pattern (p9-2/3/4 in their own modules) has a clear template to
//! follow. The renderer is `Frame`-typed — ratatui 0.28 dropped the
//! `B: Backend` generic from `Frame` (it's bound at `Terminal` init),
//! so the spec's `render_library<B: Backend>` literal is collapsed
//! here. Logged in HOTFIXES.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use kebab_core::{DocFilter, DocSummary, Lang};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};

use crate::app::{App, KeyOutcome, Pane};
use crate::input::{display_width, truncate_to_display_width};

/// Width (in display columns) of the `tags` column in the doc-list
/// row. Used twice — truncate input + pad calculation — so a const
/// keeps them in sync.
const TAGS_COL_W: usize = 12;

/// Internal state owned by `LibraryState`. Public-by-crate so
/// `handle_key_library` can mutate it without crossing the
/// `pub`-visibility boundary `LibraryState` exposes.
pub(crate) struct LibraryStateInner {
    pub docs: Vec<DocSummary>,
    pub list_state: ListState,
    pub filter: DocFilter,
    /// Edit overlay for the filter (toggled by `f`). `Some` while
    /// the user is editing tags / lang fields.
    pub filter_edit: Option<FilterEdit>,
    /// True after `App::new` and again after every filter refresh,
    /// flipped to false once the run loop services the refresh.
    pub needs_refresh: bool,
    /// True while the run loop is awaiting `kebab-app::list_docs_with_config`
    /// — drives the "loading…" header span. Synchronous in v1
    /// (acceptable hang per spec).
    pub loading: bool,
    /// `g` waiting for the second `g` (vim-style `gg` → top).
    pub pending_g: bool,
}

impl Default for LibraryStateInner {
    fn default() -> Self {
        let mut list_state = ListState::default();
        list_state.select(None);
        Self {
            docs: Vec::new(),
            list_state,
            filter: DocFilter::default(),
            filter_edit: None,
            needs_refresh: true,
            loading: false,
            pending_g: false,
        }
    }
}

/// Filter edit overlay state. `f` toggles in/out of edit mode;
/// while editing, `tab` cycles between fields and `Enter` commits.
pub(crate) struct FilterEdit {
    pub field: FilterField,
    pub tags_buf: String,
    pub lang_buf: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FilterField {
    Tags,
    Lang,
}

impl FilterEdit {
    pub fn from_filter(filter: &DocFilter) -> Self {
        Self {
            field: FilterField::Tags,
            tags_buf: filter.tags_any.join(","),
            lang_buf: filter
                .lang
                .as_ref()
                .map(|l| l.0.clone())
                .unwrap_or_default(),
        }
    }

    pub fn commit_into(&self, filter: &mut DocFilter) {
        filter.tags_any = self
            .tags_buf
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        let trimmed = self.lang_buf.trim();
        filter.lang = if trimmed.is_empty() {
            None
        } else {
            Some(Lang(trimmed.to_string()))
        };
    }
}

/// Render the Library pane. `area` is the full body region;
/// header / footer are owned by the run loop.
pub fn render_library(f: &mut Frame, area: Rect, state: &App) {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(filter_overlay_height(state)),
            Constraint::Min(1),
        ])
        .split(area);

    if let Some(edit) = &state.library.inner.filter_edit {
        render_filter_overlay(f, layout[0], edit, &state.theme);
    }
    render_doc_list(f, layout[1], state);
}

fn filter_overlay_height(state: &App) -> u16 {
    if state.library.inner.filter_edit.is_some() {
        4
    } else {
        0
    }
}

fn render_filter_overlay(f: &mut Frame, area: Rect, edit: &FilterEdit, theme: &crate::theme::Theme) {
    let block = Block::default()
        .title("Filter (Tab=cycle field, Enter=apply, Esc=cancel)")
        .borders(Borders::ALL);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let lines = vec![
        line_with_focus("tags_any (csv): ", &edit.tags_buf, edit.field == FilterField::Tags, theme),
        line_with_focus("lang:           ", &edit.lang_buf, edit.field == FilterField::Lang, theme),
    ];
    let para = Paragraph::new(lines);
    f.render_widget(para, inner);
}

fn line_with_focus<'a>(
    label: &'a str,
    value: &'a str,
    focused: bool,
    theme: &crate::theme::Theme,
) -> Line<'a> {
    let style = if focused {
        theme.style(crate::theme::Role::Selected)
    } else {
        theme.style(crate::theme::Role::Body)
    };
    Line::from(vec![Span::raw(label), Span::styled(value, style)])
}

fn render_doc_list(f: &mut Frame, area: Rect, state: &App) {
    let inner = &state.library.inner;
    let header_text = if inner.loading {
        "Library — loading…"
    } else if inner.docs.is_empty() {
        "Library — no docs (run `kebab ingest` first, then press F5 or re-open)"
    } else {
        "Library"
    };
    let block = Block::default().title(header_text).borders(Borders::ALL);

    if inner.docs.is_empty() {
        f.render_widget(block, area);
        return;
    }

    let title_w = (area.width as usize).saturating_sub(40).max(20);
    let items: Vec<ListItem> = inner
        .docs
        .iter()
        .map(|d| ListItem::new(format_doc_row(d, title_w)))
        .collect();

    let list = List::new(items)
        .block(block)
        .highlight_style(state.theme.style(crate::theme::Role::Selected))
        .highlight_symbol("> ");

    let mut list_state = inner.list_state.clone();
    f.render_stateful_widget(list, area, &mut list_state);
}

/// Format a `DocSummary` row using display-width-aware truncation
/// and padding. Korean / wide chars contribute 2 columns each.
pub(crate) fn format_doc_row(d: &DocSummary, title_w: usize) -> String {
    let title = truncate_to_display_width(&d.title, title_w);
    let tags = if d.tags.is_empty() {
        "-".to_string()
    } else {
        d.tags.join(",")
    };
    let tags = truncate_to_display_width(&tags, TAGS_COL_W);
    let updated = d
        .updated_at
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "?".to_string());
    let updated_short = updated.split('T').next().unwrap_or("?");
    // std::fmt's `<width$>` form pads by **char count**, which
    // overshoots when the value contains wide chars (each Hangul
    // adds 2 cols but counts as 1 char → padding is half-short and
    // downstream columns drift). Compute the pad spaces ourselves
    // from `display_width`, then concatenate — the truncate above
    // already guarantees `display_width(title) <= title_w`.
    let title_pad = title_w.saturating_sub(display_width(&title));
    let tags_pad = TAGS_COL_W.saturating_sub(display_width(&tags));
    format!(
        "{title}{:title_pad$}  {tags}{:tags_pad$}  {updated_short:<10}  {chunk_count}",
        "",
        "",
        title = title,
        tags = tags,
        updated_short = updated_short,
        chunk_count = d.chunk_count,
        title_pad = title_pad,
        tags_pad = tags_pad,
    )
}

/// Library pane key dispatch. Mutates `App.library.inner`; never
/// touches another pane's state (parallel-safety contract).
pub fn handle_key_library(state: &mut App, key: KeyEvent) -> KeyOutcome {
    if state.error_overlay.is_some() {
        // Any key dismisses the popup.
        state.error_overlay = None;
        return KeyOutcome::Continue;
    }

    if state.library.inner.filter_edit.is_some() {
        return handle_filter_edit_key(state, key);
    }

    // p9-fb-04: Esc / Ctrl-C while ingest is in flight flips the
    // worker's cancel token (instead of triggering the quit path).
    // Done BEFORE the `inner` borrow so we can re-borrow `state`.
    let is_cancel_chord = match (key.code, key.modifiers) {
        (KeyCode::Esc, _) => true,
        (KeyCode::Char('c'), m) => m.contains(KeyModifiers::CONTROL),
        _ => false,
    };
    if is_cancel_chord && crate::ingest_progress::cancel_running_ingest(state) {
        return KeyOutcome::Continue;
    }

    let inner = &mut state.library.inner;
    let pending_g = std::mem::take(&mut inner.pending_g);

    match (key.code, key.modifiers) {
        (KeyCode::Char('q'), _) | (KeyCode::Esc, _) => {
            state.should_quit = true;
            KeyOutcome::Quit
        }
        (KeyCode::Char('j'), _) | (KeyCode::Down, _) => {
            move_selection(inner, 1);
            KeyOutcome::Continue
        }
        (KeyCode::Char('k'), _) | (KeyCode::Up, _) => {
            move_selection(inner, -1);
            KeyOutcome::Continue
        }
        (KeyCode::Char('g'), m) if !m.contains(KeyModifiers::SHIFT) => {
            if pending_g {
                set_selection(inner, 0);
                KeyOutcome::Continue
            } else {
                inner.pending_g = true;
                KeyOutcome::Continue
            }
        }
        (KeyCode::Char('G'), _) => {
            let last = inner.docs.len().saturating_sub(1);
            set_selection(inner, last);
            KeyOutcome::Continue
        }
        (KeyCode::Char('f'), _) => {
            inner.filter_edit = Some(FilterEdit::from_filter(&inner.filter));
            KeyOutcome::Continue
        }
        (KeyCode::Char('r'), _) => {
            // p9-fb-03: trigger background ingest. The `inner` mutable
            // borrow above is not used in this arm, so NLL releases it
            // before we re-borrow `state` for `start_ingest`. Errors
            // (e.g. "ingest already running") surface via the error
            // overlay.
            if let Err(e) = crate::ingest_progress::start_ingest(state) {
                state.error_overlay =
                    Some(crate::ErrorOverlay::from_anyhow(&e));
            }
            KeyOutcome::Continue
        }
        (KeyCode::Char('/'), _) => KeyOutcome::SwitchPane(Pane::Search),
        (KeyCode::Char('?'), _) => KeyOutcome::SwitchPane(Pane::Ask),
        (KeyCode::Enter, _) => {
            if inner.docs.is_empty() {
                KeyOutcome::Continue
            } else {
                let idx = inner.list_state.selected().unwrap_or(0);
                // Capture doc_id and exit the `inner` borrow scope
                // before re-borrowing `state` for `enter_inspect`.
                let doc_id = inner.docs[idx].doc_id.clone();
                // NLL releases the `inner` borrow at last use above;
                // we can re-borrow `state` mutably for the inspect-side
                // mutation below.
                let target = crate::app::InspectTarget::Doc(doc_id);
                crate::inspect::enter_inspect(state, target, Pane::Library);
                KeyOutcome::SwitchPane(Pane::Inspect)
            }
        }
        _ => KeyOutcome::Continue,
    }
}

fn handle_filter_edit_key(state: &mut App, key: KeyEvent) -> KeyOutcome {
    let Some(edit) = state.library.inner.filter_edit.as_mut() else {
        return KeyOutcome::Continue;
    };
    match key.code {
        KeyCode::Esc => {
            state.library.inner.filter_edit = None;
            KeyOutcome::Continue
        }
        KeyCode::Tab => {
            edit.field = match edit.field {
                FilterField::Tags => FilterField::Lang,
                FilterField::Lang => FilterField::Tags,
            };
            KeyOutcome::Continue
        }
        KeyCode::Enter => {
            let edit = state.library.inner.filter_edit.take().unwrap();
            edit.commit_into(&mut state.library.inner.filter);
            state.library.inner.needs_refresh = true;
            KeyOutcome::Refresh
        }
        KeyCode::Backspace => {
            let buf = match edit.field {
                FilterField::Tags => &mut edit.tags_buf,
                FilterField::Lang => &mut edit.lang_buf,
            };
            buf.pop();
            KeyOutcome::Continue
        }
        KeyCode::Char(c) => {
            let buf = match edit.field {
                FilterField::Tags => &mut edit.tags_buf,
                FilterField::Lang => &mut edit.lang_buf,
            };
            buf.push(c);
            KeyOutcome::Continue
        }
        _ => KeyOutcome::Continue,
    }
}

fn move_selection(inner: &mut LibraryStateInner, delta: i32) {
    if inner.docs.is_empty() {
        return;
    }
    let current = inner.list_state.selected().unwrap_or(0) as i32;
    let last = (inner.docs.len() as i32) - 1;
    let next = (current + delta).clamp(0, last);
    inner.list_state.select(Some(next as usize));
}

fn set_selection(inner: &mut LibraryStateInner, idx: usize) {
    if inner.docs.is_empty() {
        inner.list_state.select(None);
    } else {
        let clamped = idx.min(inner.docs.len() - 1);
        inner.list_state.select(Some(clamped));
    }
}

/// Run-loop hook: refresh `docs` from the facade. Public-by-crate
/// because the run loop owns the call site.
pub(crate) fn refresh_docs(state: &mut App) -> anyhow::Result<()> {
    state.library.inner.loading = true;
    let result = kebab_app::list_docs_with_config(
        state.config.clone(),
        state.library.inner.filter.clone(),
    );
    state.library.inner.loading = false;
    match result {
        Ok(docs) => {
            let prior = state.library.inner.list_state.selected();
            state.library.inner.docs = docs;
            // Clamp selection.
            let len = state.library.inner.docs.len();
            if len == 0 {
                state.library.inner.list_state.select(None);
            } else {
                let next = prior.map(|p| p.min(len - 1)).unwrap_or(0);
                state.library.inner.list_state.select(Some(next));
            }
            state.library.inner.needs_refresh = false;
            Ok(())
        }
        Err(e) => {
            state.library.inner.needs_refresh = false;
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kebab_core::{
        ChunkerVersion, DocSummary, DocumentId, Lang, ParserVersion, SourceType, TrustLevel,
        WorkspacePath,
    };
    use time::OffsetDateTime;

    fn doc(title: &str, tags: &[&str]) -> DocSummary {
        DocSummary {
            doc_id: DocumentId("a".repeat(32)),
            doc_path: WorkspacePath::new("x.md".into()).unwrap(),
            title: title.into(),
            lang: Lang("en".into()),
            tags: tags.iter().map(|s| (*s).into()).collect(),
            trust_level: TrustLevel::Primary,
            source_type: SourceType::Note,
            byte_len: 1,
            chunk_count: 1,
            created_at: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
            updated_at: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
            parser_version: ParserVersion("p".into()),
            chunker_version: ChunkerVersion("c".into()),
        }
    }

    /// p9-fb-10: format_doc_row pads by display width (not char
    /// count) so wide-char titles don't shift downstream columns.
    /// Regression pin — `<title_w$>` (std::fmt char-count form)
    /// would fail this for any Hangul title.
    #[test]
    fn format_doc_row_pads_by_display_width_for_hangul_title() {
        let row = format_doc_row(&doc("러스트로 만드는 KB", &["rust"]), 30);
        // Expected layout (display cols):
        //   title 30  +  "  "(2)  +  tags 12  +  "  "(2)  +  date 10  +  "  "(2)  +  chunk
        // chunk = "1" → 1 col. Total = 30+2+12+2+10+2+1 = 59.
        assert_eq!(
            display_width(&row),
            59,
            "row must align to display columns, not char count: {row:?}"
        );
    }

    /// p9-fb-10: Hangul tag also pads by display width.
    #[test]
    fn format_doc_row_pads_by_display_width_for_hangul_tag() {
        let row = format_doc_row(&doc("ascii", &["한글"]), 20);
        // title 20 + "  " + tags 12 + "  " + date 10 + "  " + "1" = 49
        assert_eq!(display_width(&row), 49, "row: {row:?}");
    }
}
