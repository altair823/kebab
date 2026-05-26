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
    pub tags_buf: crate::input::InputBuffer,
    pub lang_buf: crate::input::InputBuffer,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FilterField {
    Tags,
    Lang,
}

impl FilterEdit {
    /// Borrow the buffer for the currently-focused field. Centralizes
    /// the `match edit.field` pick so the key-handler arms (Backspace
    /// / arrows / Delete / typed Char) don't each re-spell the same
    /// 2-arm dispatch.
    fn active_buf_mut(&mut self) -> &mut crate::input::InputBuffer {
        match self.field {
            FilterField::Tags => &mut self.tags_buf,
            FilterField::Lang => &mut self.lang_buf,
        }
    }

    pub fn from_filter(filter: &DocFilter) -> Self {
        let mut tags_buf = crate::input::InputBuffer::new();
        tags_buf.push_str(&filter.tags_any.join(","));
        let mut lang_buf = crate::input::InputBuffer::new();
        if let Some(lang) = filter.lang.as_ref() {
            lang_buf.push_str(&lang.0);
        }
        Self { field: FilterField::Tags, tags_buf, lang_buf }
    }

    pub fn commit_into(&self, filter: &mut DocFilter) {
        filter.tags_any = self
            .tags_buf
            .as_str()
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        let trimmed = self.lang_buf.as_str().trim();
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

/// Single source of truth for the filter overlay row labels — used
/// both by `line_with_focus` (display) and the cursor-placement
/// `display_width(...)` math below. Editing one without the other
/// would silently miscolumn the caret.
const LABEL_TAGS: &str = "tags_any (csv): ";
const LABEL_LANG: &str = "lang:           ";

fn render_filter_overlay(f: &mut Frame, area: Rect, edit: &FilterEdit, theme: &crate::theme::Theme) {
    let block = Block::default()
        .title("Filter (Tab=cycle field, Enter=apply, Esc=cancel)")
        .borders(Borders::ALL);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let lines = vec![
        line_with_focus(LABEL_TAGS, edit.tags_buf.as_str(), edit.field == FilterField::Tags, theme),
        line_with_focus(LABEL_LANG, edit.lang_buf.as_str(), edit.field == FilterField::Lang, theme),
    ];
    let para = Paragraph::new(lines);
    f.render_widget(para, inner);

    // p9-fb-10: ratatui calls show_cursor + MoveTo whenever
    // cursor_position is Some (our case here). When a render fn
    // omits set_cursor_position (Library/Inspect main view), ratatui
    // calls hide_cursor instead. So this single call positions the
    // caret on the focused field of the filter overlay.
    // place_cursor_x sums in usize (avoiding u16 wrap) and clamps to
    // the right edge of the inner area.
    let (label, focused_buf, row_offset) = match edit.field {
        FilterField::Tags => (LABEL_TAGS, &edit.tags_buf, 0u16),
        FilterField::Lang => (LABEL_LANG, &edit.lang_buf, 1u16),
    };
    let label_w = display_width(label);
    let cursor_x = crate::input::place_cursor_x(inner.x, inner.width, label_w, focused_buf.cursor_col());
    f.set_cursor_position((cursor_x, inner.y + row_offset));
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
    let block_inner = block.inner(area);
    f.render_widget(block, area);

    if inner.docs.is_empty() {
        return;
    }

    // p9-fb-24: split the inner area into a 1-row column header on top
    // and the doc list below. Header reuses the same width math as
    // `format_doc_row` so labels line up with their data columns.
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(block_inner);
    let header_area = layout[0];
    let list_area = layout[1];

    let title_w = (list_area.width as usize).saturating_sub(40).max(20);

    let header_para = Paragraph::new(format_doc_header(title_w))
        .style(state.theme.style(crate::theme::Role::Heading));
    f.render_widget(header_para, header_area);

    let items: Vec<ListItem> = inner
        .docs
        .iter()
        .map(|d| ListItem::new(format_doc_row(d, title_w)))
        .collect();

    let list = List::new(items)
        .highlight_style(state.theme.style(crate::theme::Role::Selected))
        .highlight_symbol("> ");

    let mut list_state = inner.list_state.clone();
    f.render_stateful_widget(list, list_area, &mut list_state);
}

/// p9-fb-24: render the column-label row that sits directly above
/// the doc list. Uses the same width math as `format_doc_row` so
/// the labels line up with their data columns regardless of Hangul
/// / CJK width drift.
///
/// Layout: `TITLE<title_pad>  TAGS<tags_pad>  UPDATED  CHUNKS`.
/// The title column width matches `area.width.saturating_sub(40).max(20)`
/// — the same calculation `render_doc_list` uses for `title_w`.
pub(crate) fn format_doc_header(title_w: usize) -> Line<'static> {
    let title_label = "TITLE";
    let tags_label = "TAGS";
    let title_pad = title_w.saturating_sub(display_width(title_label));
    let tags_pad = TAGS_COL_W.saturating_sub(display_width(tags_label));
    let text = format!(
        "{title_label}{:title_pad$}  {tags_label}{:tags_pad$}  {updated:<10}  {chunks}",
        "",
        "",
        title_label = title_label,
        tags_label = tags_label,
        updated = "UPDATED",
        chunks = "CHUNKS",
        title_pad = title_pad,
        tags_pad = tags_pad,
    );
    Line::from(text)
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
        (KeyCode::Char('q') | KeyCode::Esc, _) => {
            state.should_quit = true;
            KeyOutcome::Quit
        }
        (KeyCode::Char('j') | KeyCode::Down, _) => {
            move_selection(inner, 1);
            KeyOutcome::Continue
        }
        (KeyCode::Char('k') | KeyCode::Up, _) => {
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
            edit.active_buf_mut().pop_char();
            KeyOutcome::Continue
        }
        // p9-fb-22: cursor navigation + Delete inside the active filter
        // field. Tab still cycles between Tags / Lang fields; arrows
        // only move within the focused buffer.
        KeyCode::Left => {
            edit.active_buf_mut().move_left();
            KeyOutcome::Continue
        }
        KeyCode::Right => {
            edit.active_buf_mut().move_right();
            KeyOutcome::Continue
        }
        KeyCode::Home => {
            edit.active_buf_mut().move_home();
            KeyOutcome::Continue
        }
        KeyCode::End => {
            edit.active_buf_mut().move_end();
            KeyOutcome::Continue
        }
        KeyCode::Delete => {
            edit.active_buf_mut().delete_after();
            KeyOutcome::Continue
        }
        KeyCode::Char(c) => {
            edit.active_buf_mut().push_char(c);
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
                let next = prior.map_or(0, |p| p.min(len - 1));
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

    /// p9-fb-24: column header row uses the same width math as
    /// `format_doc_row` so labels line up with their data columns.
    /// The TITLE label sits in the title column, TAGS sits in the
    /// 12-col TAGS column, UPDATED in the 10-col date column, and
    /// CHUNKS at the trailing position.
    #[test]
    fn format_doc_header_aligns_with_format_doc_row() {
        let title_w = 30;
        let header = format_doc_header(title_w);
        let header_text: String = header
            .spans
            .iter()
            .map(|sp| sp.content.as_ref())
            .collect();
        assert!(header_text.contains("TITLE"), "header has TITLE label");
        assert!(header_text.contains("TAGS"), "header has TAGS label");
        assert!(header_text.contains("UPDATED"), "header has UPDATED label");
        assert!(header_text.contains("CHUNKS"), "header has CHUNKS label");
        let row = format_doc_row(&doc("ascii-title", &["rust"]), title_w);
        let tags_start_in_row = row.find("rust").expect("row has tags");
        let tags_start_in_header = header_text.find("TAGS").expect("header has TAGS");
        assert!(
            tags_start_in_header <= tags_start_in_row,
            "TAGS header drifted past row tags: header={tags_start_in_header} row={tags_start_in_row}"
        );
    }
}
