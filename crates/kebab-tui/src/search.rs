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
    // p9-fb-10: compute prompt display width before moving the String
    // into the Span so we can place the cursor without a second alloc.
    let prompt = format!("[{mode_label}] ");
    let prompt_w = crate::input::display_width(&prompt);
    let line = Line::from(vec![
        Span::styled(prompt, theme.style(mode_role)),
        Span::raw(s.input.as_str()),
        Span::styled(searching_hint, theme.style(crate::theme::Role::Hint)),
    ]);
    let block = Block::default()
        .title("query (Tab=mode  Enter=search  Esc=back)")
        .borders(Borders::ALL);
    let inner = block.inner(area);
    f.render_widget(Paragraph::new(line).block(block), area);
    // p9-fb-10: ratatui calls show_cursor + MoveTo whenever
    // cursor_position is Some (our case here). When a render fn
    // omits set_cursor_position (Library/Inspect), ratatui calls
    // hide_cursor instead. So this single call both positions and
    // unhides the caret for the Search input column.
    let raw_x = inner.x + (prompt_w + s.input.cursor_col()) as u16;
    // Clamp to the right edge of the inner area — a long CJK query
    // in a narrow terminal could otherwise place the caret beyond
    // the box; crossterm passes coords through verbatim.
    let cursor_x = raw_x.min(inner.x + inner.width.saturating_sub(1));
    let cursor_y = inner.y;
    f.set_cursor_position((cursor_x, cursor_y));
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

    // p9-fb-12 follow-up: `i` (chunk inspect) + `g` (editor jump) are
    // Normal-mode commands. In Insert they type as characters into
    // the query buffer (mode-authoritative dispatch — replaces the
    // pre-fb-12 SHIFT/none heuristic).
    let is_normal = state.mode == crate::app::Mode::Normal;

    if is_normal
        && matches!(
            (key.code, key.modifiers),
            (KeyCode::Char('i'), KeyModifiers::NONE)
        )
    {
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

    if is_normal
        && matches!(
            (key.code, key.modifiers),
            (KeyCode::Char('g'), KeyModifiers::NONE)
        )
    {
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

    // p9-fb-12 follow-up: mode-authoritative dispatch. The pre-fb-12
    // `is_typing_mod` heuristic (SHIFT-aware char filter) is gone —
    // mode now decides whether a Char goes to the input buffer or
    // becomes a navigation command. `Tab` (mode cycle), `Enter`
    // (refresh), `Backspace`, arrow keys, Esc work in both modes
    // because they have no typing ambiguity.
    match (key.code, key.modifiers) {
        (KeyCode::Esc, _) => KeyOutcome::SwitchPane(Pane::Library),
        (KeyCode::Tab, _) => {
            s.mode = cycle_mode(s.mode);
            // Force re-search at the new mode if there's a query.
            if !s.input.as_str().trim().is_empty() {
                s.input_dirty_at = Some(time::OffsetDateTime::now_utc());
            }
            KeyOutcome::Continue
        }
        (KeyCode::Enter, _) => {
            // Skip debounce; refresh now if there's anything to query.
            if s.input.as_str().trim().is_empty() {
                KeyOutcome::Continue
            } else {
                s.input_dirty_at = None;
                s.last_query = None;
                KeyOutcome::Refresh
            }
        }
        (KeyCode::Down, _) => {
            move_selection(s, 1);
            s.preview = None;
            KeyOutcome::Continue
        }
        (KeyCode::Up, _) => {
            move_selection(s, -1);
            s.preview = None;
            KeyOutcome::Continue
        }
        (KeyCode::Backspace, _) => {
            if !s.input.is_empty() {
                s.input.pop_char();
                s.input_dirty_at = Some(time::OffsetDateTime::now_utc());
            }
            KeyOutcome::Continue
        }
        // p9-fb-12 follow-up: Char dispatch is mode-gated. Normal
        // mode → j/k navigate; Insert mode → typed into input.
        // Single arm per key, body branches on mode (clearer than
        // duplicate-arm + guard).
        (KeyCode::Char('j'), KeyModifiers::NONE) => {
            if is_normal {
                move_selection(s, 1);
                s.preview = None;
            } else {
                s.input.push_char('j');
                s.input_dirty_at = Some(time::OffsetDateTime::now_utc());
            }
            KeyOutcome::Continue
        }
        (KeyCode::Char('k'), KeyModifiers::NONE) => {
            if is_normal {
                move_selection(s, -1);
                s.preview = None;
            } else {
                s.input.push_char('k');
                s.input_dirty_at = Some(time::OffsetDateTime::now_utc());
            }
            KeyOutcome::Continue
        }
        (KeyCode::Char(c), m)
            if !is_normal
                && !m.contains(KeyModifiers::CONTROL)
                && !m.contains(KeyModifiers::ALT) =>
        {
            // Insert mode: every plain or SHIFT-only Char goes to
            // input. CTRL/ALT chords stay reserved for future
            // bindings (and don't currently match any Search
            // command, so they're a safe fall-through to Continue).
            s.input.push_char(c);
            s.input_dirty_at = Some(time::OffsetDateTime::now_utc());
            KeyOutcome::Continue
        }
        // Normal mode + un-handled Char → no-op (no typing in
        // Normal). Modifier chords always no-op.
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
/// changed). p9-fb-08 adds two skip cases:
/// - if a worker is already in flight for the *same* `(input, mode)`
///   the spawn is redundant — wait for the result.
/// - dedupe against `last_query` (was already there pre-fb-08, kept).
pub fn debounce_due(s: &SearchState) -> bool {
    let Some(at) = s.input_dirty_at else { return false };
    let elapsed = (time::OffsetDateTime::now_utc() - at)
        .try_into()
        .unwrap_or(Duration::ZERO);
    if elapsed < SEARCH_DEBOUNCE {
        return false;
    }
    let q = s.input.as_str().trim();
    if q.is_empty() {
        return false;
    }
    // p9-fb-08: if the most-recent in-flight query is identical to
    // the current input/mode pair, don't spawn another worker — the
    // existing result will land via `poll_worker`.
    if s.searching {
        if let Some((prev_input, prev_mode)) = &s.last_query {
            if prev_input.as_str() == s.input.as_str() && *prev_mode == s.mode {
                return false;
            }
        }
    }
    !matches!(
        &s.last_query,
        Some((prev_input, prev_mode))
            if prev_input.as_str() == s.input.as_str() && *prev_mode == s.mode
    )
}

/// Run-loop hook: spawn an asynchronous search worker. Returns
/// immediately so the event loop keeps polling — the result lands in
/// `state.search.worker_rx` and is applied by `poll_worker` on a
/// later tick. p9-fb-08 deviation from the original synchronous
/// design (the user typed faster than vector search could complete,
/// freezing the UI for 50-200 ms per keystroke under hybrid mode).
///
/// Behavior:
/// 1. Increment `generation` so any in-flight result becomes stale
///    on receive (`poll_worker` drops it).
/// 2. Drop the prior `worker_rx` (the old worker keeps running and
///    its result is silently discarded — search is a pure read with
///    no cleanup obligation).
/// 3. Snapshot `last_query` + clear `input_dirty_at` for the
///    debounce machinery (so a no-op keystroke doesn't re-spawn).
/// 4. Spawn a fresh worker carrying its generation token.
pub(crate) fn fire_search(state: &mut App) -> anyhow::Result<()> {
    let cfg = state.config.clone();
    let (q_text, mode, generation) = {
        let s = state.search.as_mut().expect("Search slot must exist");
        s.generation = s.generation.wrapping_add(1);
        s.searching = true;
        s.input_dirty_at = None;
        let q_text = s.input.as_str().to_string();
        s.last_query = Some((q_text.clone(), s.mode));
        (q_text, s.mode, s.generation)
    };

    let (tx, rx) = std::sync::mpsc::channel();
    // Fire-and-forget — `JoinHandle` is dropped immediately so the
    // OS detaches the thread. Search is a pure read with no
    // cleanup obligation; if the receiver is replaced (next
    // keystroke spawns a fresh worker), the old worker's
    // `tx.send` no-ops and it exits silently.
    std::thread::Builder::new()
        .name(format!("kebab-tui-search-gen{generation}"))
        .spawn(move || {
            let query = SearchQuery {
                text: q_text,
                mode,
                k: SEARCH_K,
                filters: kebab_core::SearchFilters::default(),
            };
            let result = kebab_app::search_with_config(cfg, query);
            let _ = tx.send(crate::app::SearchWorkerMessage::Done {
                generation,
                result,
            });
        })
        .map_err(|e| anyhow::anyhow!("spawn search worker: {e}"))?;

    let s = state.search.as_mut().expect("Search slot must exist");
    s.worker_rx = Some(rx);
    Ok(())
}

/// Run-loop hook: drain any pending message from the search worker.
/// Stale results (newer query already in flight) are silently
/// dropped per the generation-counter contract. `pub` so integration
/// tests can drive the stale-result paths by injecting a channel.
pub fn poll_worker(state: &mut App) {
    let Some(s) = state.search.as_mut() else { return };
    let Some(rx) = s.worker_rx.as_ref() else { return };
    let msg = match rx.try_recv() {
        Ok(m) => m,
        Err(std::sync::mpsc::TryRecvError::Empty) => return,
        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
            // Worker panicked or dropped tx without sending. Clear
            // the rx + searching flag so the next debounce tick can
            // re-fire if needed.
            s.worker_rx = None;
            s.searching = false;
            return;
        }
    };
    s.worker_rx = None;
    match msg {
        crate::app::SearchWorkerMessage::Done { generation, result } => {
            // p9-fb-08: stale guard. The user kept typing after this
            // worker spawned and a newer query is in flight — drop
            // the result. Don't clear `searching` because the newer
            // worker (if any) is still running; if there's no newer
            // worker (rare race), the next debounce_due tick will
            // re-fire `fire_search` and reset everything.
            if generation != s.generation {
                tracing::debug!(
                    target: "kebab-tui",
                    stale_gen = generation,
                    current_gen = s.generation,
                    "dropping stale search result"
                );
                return;
            }
            s.searching = false;
            match result {
                Ok(hits) => {
                    s.hits = hits;
                    s.selected_hit = 0;
                    s.preview = None;
                }
                Err(e) => {
                    s.hits.clear();
                    s.selected_hit = 0;
                    state.error_overlay =
                        Some(crate::error_popup::ErrorOverlay::from_anyhow(&e));
                }
            }
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

