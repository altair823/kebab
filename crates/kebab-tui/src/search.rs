//! Search pane (P9-2).
//!
//! `App.search` slot is filled lazily by the run loop on first
//! `Pane::Search` switch. `handle_key_search` mutates only
//! `app.search` (parallel-safety contract from p9-1) — never touches
//! Library / Ask / Inspect state.
//!
//! Spec deviation (HOTFIXES `2026-05-02 P9-2`):
//! - `render_search<B: Backend>` generic dropped (ratatui 0.28 Frame
//!   is backend-agnostic, same as P9-1).
//! - `jump_to_citation` gained a `workspace_root: &Path` argument
//!   missing from spec literal — citations carry workspace-relative
//!   paths and the editor needs an absolute path to open.
//!
//! Per design §1.5 / §1.6 (search output dense format), §3.7
//! (`SearchHit`), §0 Q3 (citation URI fragments).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use kebab_core::{Citation, SearchHit, SearchMode, SearchQuery};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use crate::app::{App, KeyOutcome, Pane, SearchState};

/// Debounce window after the last keystroke before re-searching.
/// Matches the spec's 200 ms.
pub const SEARCH_DEBOUNCE: Duration = Duration::from_millis(200);

/// Maximum hits to fetch per query — matches `config.search.default_k`
/// in production but the trait does not expose `Config`, so we cap
/// here. Users running deep recall should `kebab search --json` for
/// large `k`.
const SEARCH_K: usize = 10;

/// Render the Search pane: input bar (top), result list (middle),
/// preview (bottom). Each result row uses §1.5's 4-line dense format.
pub fn render_search(f: &mut Frame, area: Rect, state: &App) {
    let Some(s) = state.search.as_ref() else {
        // Pane has no state yet — should not happen because the run
        // loop lazy-inits before render. Defensive empty block.
        f.render_widget(
            Block::default().title("Search").borders(Borders::ALL),
            area,
        );
        return;
    };

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(3),
            Constraint::Length(7),
        ])
        .split(area);

    render_input_bar(f, layout[0], s, &state.theme);
    render_result_list(f, layout[1], s, &state.theme);
    render_preview(f, layout[2], s, &state.theme);
}

fn render_input_bar(f: &mut Frame, area: Rect, s: &SearchState, theme: &crate::theme::Theme) {
    let mode_label = mode_label(s.mode);
    let mode_role = match s.mode {
        SearchMode::Lexical => crate::theme::Role::ModeLexical,
        SearchMode::Vector => crate::theme::Role::ModeVector,
        SearchMode::Hybrid => crate::theme::Role::ModeHybrid,
    };
    let searching_hint = if s.searching { "  searching…" } else { "" };
    let line = Line::from(vec![
        Span::styled(format!("[{mode_label}] "), theme.style(mode_role)),
        Span::raw(s.input.as_str()),
        Span::styled(searching_hint, theme.style(crate::theme::Role::Hint)),
    ]);
    let block = Block::default()
        .title("query (Tab=mode  Enter=search  Esc=back)")
        .borders(Borders::ALL);
    f.render_widget(Paragraph::new(line).block(block), area);
}

fn mode_label(m: SearchMode) -> &'static str {
    match m {
        SearchMode::Lexical => "lexical",
        SearchMode::Vector => "vector",
        SearchMode::Hybrid => "hybrid",
    }
}

fn render_result_list(f: &mut Frame, area: Rect, s: &SearchState, theme: &crate::theme::Theme) {
    let block = Block::default()
        .title(format!("results ({})", s.hits.len()))
        .borders(Borders::ALL);

    if s.hits.is_empty() {
        f.render_widget(block, area);
        return;
    }

    let items: Vec<ListItem> = s
        .hits
        .iter()
        .map(|h| ListItem::new(format_hit_lines(h, theme)))
        .collect();
    let list = List::new(items)
        .block(block)
        .highlight_style(theme.style(crate::theme::Role::Selected))
        .highlight_symbol("> ");
    let mut list_state = ListState::default();
    list_state.select(Some(s.selected_hit.min(s.hits.len().saturating_sub(1))));
    f.render_stateful_widget(list, area, &mut list_state);
}

/// §1.5 dense format — 4 lines per hit:
/// 1. `<rank>. <fusion_score>  <path#frag>`
/// 2. `<heading_path joined by " / "> | section_label?`
/// 3. snippet line 1
/// 4. snippet line 2 (or trailing blank for layout symmetry)
fn format_hit_lines(h: &SearchHit, theme: &crate::theme::Theme) -> Vec<Line<'static>> {
    let header = format!(
        "{}. {:.4}  {}",
        h.rank,
        h.retrieval.fusion_score,
        h.citation.to_uri(),
    );
    let path_line = {
        let hp = if h.heading_path.is_empty() {
            String::from("-")
        } else {
            h.heading_path.join(" / ")
        };
        match h.section_label.as_deref() {
            Some(s) if !s.is_empty() => format!("  {hp} | {s}"),
            _ => format!("  {hp}"),
        }
    };
    let mut snippet_lines = h.snippet.lines();
    let s1 = snippet_lines.next().unwrap_or("").to_string();
    let s2 = snippet_lines.next().unwrap_or("").to_string();
    vec![
        Line::from(Span::styled(header, theme.style(crate::theme::Role::Title))),
        Line::from(Span::styled(path_line, theme.style(crate::theme::Role::Path))),
        Line::from(format!("  {s1}")),
        Line::from(format!("  {s2}")),
    ]
}

fn render_preview(f: &mut Frame, area: Rect, s: &SearchState, theme: &crate::theme::Theme) {
    let block = Block::default()
        .title("preview (g=open in $EDITOR)")
        .borders(Borders::ALL);
    let body = match (&s.preview, s.hits.is_empty()) {
        (_, true) => Paragraph::new(""),
        (Some(text), _) => Paragraph::new(text.as_str()).wrap(Wrap { trim: false }),
        (None, _) => Paragraph::new(Span::styled(
            "(loading preview… select a hit to fetch its chunk text)",
            theme.style(crate::theme::Role::Hint),
        )),
    };
    f.render_widget(body.block(block), area);
}

/// Search pane key dispatch. Returns `KeyOutcome::Refresh` when the
/// run loop should re-fire `kebab-app::search`. Pure mutation on
/// `app.search` — never touches another pane's state.
pub fn handle_key_search(state: &mut App, key: KeyEvent) -> KeyOutcome {
    if state.error_overlay.is_some() {
        state.error_overlay = None;
        return KeyOutcome::Continue;
    }
    if state.search.is_none() {
        // No search state — bail back to Library.
        return KeyOutcome::SwitchPane(Pane::Library);
    }

    // `g` (editor jump) requires re-borrowing `state` for
    // workspace_root after dropping the `&mut state.search` borrow.
    // Handle it as a pre-pass so the rest of the function can use
    // `state.search.as_mut()` without scope juggling.
    // `i` (chunk inspect) — pre-pass like `g`. Only fires on plain
    // press, so typing 'i' in queries like "instance" still reaches
    // the input buffer (P9-2 SHIFT/none convention).
    if matches!(
        (key.code, key.modifiers),
        (KeyCode::Char('i'), KeyModifiers::NONE)
    ) {
        let chunk_id = {
            let s = state.search.as_ref().unwrap();
            if s.hits.is_empty() {
                None
            } else {
                Some(s.hits[s.selected_hit].chunk_id.clone())
            }
        };
        if let Some(chunk_id) = chunk_id {
            crate::inspect::enter_inspect(
                state,
                crate::app::InspectTarget::Chunk(chunk_id),
                Pane::Search,
            );
            return KeyOutcome::SwitchPane(Pane::Inspect);
        }
        return KeyOutcome::Continue;
    }

    // `g` only fires the editor jump on plain (no-modifier) press —
    // SHIFT-G in vim land is "go to bottom" (not implemented here),
    // and CTRL/ALT chords stay reserved.
    if matches!(
        (key.code, key.modifiers),
        (KeyCode::Char('g'), KeyModifiers::NONE)
    ) {
        let (citation, has_hits) = {
            let s = state.search.as_ref().unwrap();
            if s.hits.is_empty() {
                (None, false)
            } else {
                (Some(s.hits[s.selected_hit].citation.clone()), true)
            }
        };
        if has_hits {
            // p9-fb-09: enqueue the spawn for the run loop. Calling
            // `jump_to_citation` directly here would not have access
            // to the TuiTerminal handle, so the post-resume
            // `terminal.clear()` couldn't happen — leaving the
            // previous frame leaking through the new draw.
            let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".into());
            // `~/...` / `${XDG_…}` expansion via `kebab-config::expand_path`
            // — same helper used by the markdown / image / PDF ingest
            // paths (HOTFIXES 2026-05-02 P9-4 follow-up).
            let workspace_root =
                kebab_config::expand_path(&state.config.workspace.root, "");
            state.pending_editor = Some(crate::app::EditorRequest {
                citation: citation.unwrap(),
                editor_env: editor,
                workspace_root,
            });
        }
        return KeyOutcome::Continue;
    }

    let s = state.search.as_mut().unwrap();

    match (key.code, key.modifiers) {
        (KeyCode::Esc, _) => KeyOutcome::SwitchPane(Pane::Library),
        (KeyCode::Tab, _) => {
            s.mode = cycle_mode(s.mode);
            // Force re-search at the new mode if there's a query.
            if !s.input.trim().is_empty() {
                s.input_dirty_at = Some(time::OffsetDateTime::now_utc());
            }
            KeyOutcome::Continue
        }
        (KeyCode::Enter, _) => {
            // Skip debounce; refresh now if there's anything to query.
            if s.input.trim().is_empty() {
                KeyOutcome::Continue
            } else {
                s.input_dirty_at = None;
                s.last_query = None;
                KeyOutcome::Refresh
            }
        }
        // `j` / `k` only fire as selection movers when *no* modifier is
        // held. SHIFT-bearing keypresses (`J`, `K`) are typed input —
        // letting them through here would corrupt every \"JSON\" /
        // \"PostgreSQL\" search query. Down / Up arrows still accept
        // any modifier (no typing collision).
        (KeyCode::Char('j'), KeyModifiers::NONE) => {
            move_selection(s, 1);
            s.preview = None;
            KeyOutcome::Continue
        }
        (KeyCode::Down, m) if !is_typing_mod(m) => {
            move_selection(s, 1);
            s.preview = None;
            KeyOutcome::Continue
        }
        (KeyCode::Char('k'), KeyModifiers::NONE) => {
            move_selection(s, -1);
            s.preview = None;
            KeyOutcome::Continue
        }
        (KeyCode::Up, m) if !is_typing_mod(m) => {
            move_selection(s, -1);
            s.preview = None;
            KeyOutcome::Continue
        }
        (KeyCode::Backspace, _) => {
            if !s.input.is_empty() {
                s.input.pop();
                s.input_dirty_at = Some(time::OffsetDateTime::now_utc());
            }
            KeyOutcome::Continue
        }
        (KeyCode::Char(c), _) => {
            // Treat 'g' separately above; here 'g' would reach this
            // branch only when `is_typing_mod` triggered — i.e. SHIFT
            // 'G'. Fold into typing.
            s.input.push(c);
            s.input_dirty_at = Some(time::OffsetDateTime::now_utc());
            KeyOutcome::Continue
        }
        _ => KeyOutcome::Continue,
    }
}

fn cycle_mode(m: SearchMode) -> SearchMode {
    match m {
        SearchMode::Lexical => SearchMode::Vector,
        SearchMode::Vector => SearchMode::Hybrid,
        SearchMode::Hybrid => SearchMode::Lexical,
    }
}

fn is_typing_mod(m: KeyModifiers) -> bool {
    // SHIFT alone is fine for typing capital letters, but CTRL/ALT
    // means a chord — don't swallow as input.
    m.contains(KeyModifiers::CONTROL) || m.contains(KeyModifiers::ALT)
}

fn move_selection(s: &mut SearchState, delta: i32) {
    if s.hits.is_empty() {
        return;
    }
    let current = s.selected_hit as i32;
    let last = (s.hits.len() as i32) - 1;
    let next = (current + delta).clamp(0, last);
    s.selected_hit = next as usize;
}

/// Build the editor command for a citation. Splits out from
/// `jump_to_citation` so unit tests can assert command shape without
/// spawning a process.
///
/// Returns `(program, args)` where `program` is the `$EDITOR` value
/// (or `vi` fallback) and `args` opens the file at the cited line /
/// page / region (best-effort for non-text citations).
pub fn build_jump_command(
    citation: &Citation,
    editor_env: &str,
    workspace_root: &Path,
) -> (String, Vec<String>) {
    let (program, leading_args) = parse_editor_env(editor_env);
    let path = workspace_root.join(&citation.path().0);
    let path_str = path.to_string_lossy().into_owned();
    let mut args = leading_args;

    let editor_basename = std::path::Path::new(&program)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| program.clone());

    match citation {
        Citation::Line { start, .. } => {
            if editor_basename.contains("code") || editor_basename.contains("cursor") {
                // VS Code / Cursor: `code -g <path>:<line>`
                args.push("-g".into());
                args.push(format!("{path_str}:{start}"));
            } else {
                // vim / nvim / vi / emacs / hx all accept `+<N>`.
                args.push(format!("+{start}"));
                args.push(path_str);
            }
        }
        Citation::Page { page, .. } => {
            // No standard editor jump for PDFs across vim / VS Code /
            // emacs. Earlier versions of this branch tried to push a
            // `# page N` string as a final arg, but every common
            // editor treats it as a *second file to open* — opening
            // a stray buffer or splitting the window. Path-only is
            // the honest best-effort: the user's PDF reader (or the
            // editor's PDF plugin) handles in-document navigation.
            // A `KEBAB_EDITOR_JUMP_FORMAT="pdf=evince -p {page} {path}"`
            // env hook stays a P+ enhancement (per spec § Risks).
            tracing::debug!(
                target: "kebab-tui",
                page,
                "PDF citation — opening file only; editor page-jump unsupported"
            );
            args.push(path_str);
        }
        _ => {
            args.push(path_str);
        }
    }
    (program, args)
}

/// Suspend the TUI, spawn `$EDITOR`, restore the TUI on return.
///
/// p9-fb-09: delegates the suspend/restore dance to
/// [`crate::editor::with_external_program`] so the post-resume
/// `terminal.clear()` lands consistently — without it, the previous
/// frame leaked through the new draw and the user saw a corrupted
/// screen on return (도그푸딩 item 7).
///
/// Errors propagate; the helper's RAII guard restores the terminal
/// even on panic.
pub(crate) fn jump_to_citation(
    terminal: &mut crate::terminal::TuiTerminal,
    citation: &Citation,
    editor_env: &str,
    workspace_root: &Path,
) -> anyhow::Result<()> {
    let (program, args) = build_jump_command(citation, editor_env, workspace_root);
    let mut cmd = Command::new(&program);
    cmd.args(&args);
    let status = crate::editor::with_external_program(terminal, cmd)?;
    if !status.success() {
        anyhow::bail!("{program} exited with {status:?}");
    }
    Ok(())
}

fn parse_editor_env(env: &str) -> (String, Vec<String>) {
    // `$EDITOR` may carry args, e.g. `vim -p`. Split on whitespace.
    let mut parts = env.split_whitespace();
    let program = parts.next().unwrap_or("vi").to_string();
    let leading: Vec<String> = parts.map(str::to_string).collect();
    (program, leading)
}

/// Run-loop hook: tick called every poll cycle. Returns `true` if a
/// search should fire this tick (debounce expired and query
/// changed).
pub(crate) fn debounce_due(s: &SearchState) -> bool {
    let Some(at) = s.input_dirty_at else { return false };
    let elapsed = (time::OffsetDateTime::now_utc() - at)
        .try_into()
        .unwrap_or(Duration::ZERO);
    if elapsed < SEARCH_DEBOUNCE {
        return false;
    }
    let q = s.input.trim();
    if q.is_empty() {
        return false;
    }
    !matches!(
        &s.last_query,
        Some((prev_input, prev_mode))
            if prev_input == &s.input && *prev_mode == s.mode
    )
}

/// Run-loop hook: actually perform the search, populate `hits`. The
/// state's `input_dirty_at` is cleared, `last_query` snapshots, and
/// `searching` flag toggles around the call.
pub(crate) fn fire_search(state: &mut App) -> anyhow::Result<()> {
    let cfg = state.config.clone();
    let (q_text, mode) = {
        let s = state.search.as_mut().expect("Search slot must exist");
        s.searching = true;
        s.input_dirty_at = None;
        s.last_query = Some((s.input.clone(), s.mode));
        (s.input.clone(), s.mode)
    };
    let query = SearchQuery {
        text: q_text,
        mode,
        k: SEARCH_K,
        filters: kebab_core::SearchFilters::default(),
    };
    let result = kebab_app::search_with_config(cfg, query);
    let s = state.search.as_mut().expect("Search slot must exist");
    s.searching = false;
    match result {
        Ok(hits) => {
            s.hits = hits;
            s.selected_hit = 0;
            s.preview = None;
            Ok(())
        }
        Err(e) => {
            s.hits.clear();
            s.selected_hit = 0;
            Err(e)
        }
    }
}

/// Run-loop hook: lazy-fetch preview text for the selected hit.
pub(crate) fn refresh_preview(state: &mut App) -> anyhow::Result<()> {
    let cfg = state.config.clone();
    let chunk_id = {
        let s = state.search.as_ref().expect("Search slot must exist");
        if s.preview.is_some() || s.hits.is_empty() {
            return Ok(());
        }
        let Some(hit) = s.hits.get(s.selected_hit) else {
            return Ok(());
        };
        hit.chunk_id.clone()
    };
    let chunk = kebab_app::inspect_chunk_with_config(cfg, &chunk_id)?;
    let s = state.search.as_mut().expect("Search slot must exist");
    s.preview = Some(chunk.text);
    Ok(())
}

