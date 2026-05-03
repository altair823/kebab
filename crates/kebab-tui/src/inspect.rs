//! Inspect pane (P9-4).
//!
//! Read-only view of a `CanonicalDocument` (entered from Library
//! `Enter`) or a `Chunk` (entered from Search `i`). Sections
//! (metadata / provenance / blocks / embeddings) are collapsible
//! via `c`. `Esc` returns to the originating pane.
//!
//! Spec deviation (HOTFIXES `2026-05-02 P9-4`):
//! - `render_inspect<B: Backend>` generic dropped (ratatui 0.28 Frame
//!   is backend-agnostic — same as P9-1 / P9-2 / P9-3).
//! - Search pane now exposes `i` to enter chunk inspect (spec says
//!   "from Search pressing `i`"); previously Search had no `i` —
//!   added in p9-2's handler module since this PR can edit it.
//!
//! Per design §1 inspect output, §3.5 Chunk, §2.5 DocSummary,
//! §2.6 ChunkInspection.

use crossterm::event::{KeyCode, KeyEvent};
use kebab_core::{Block, CanonicalDocument, Chunk};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block as RBlock, Borders, Paragraph, Wrap};

use crate::app::{App, InspectState, InspectTarget, KeyOutcome, Pane};

const SECTION_METADATA: &str = "metadata";
const SECTION_PROVENANCE: &str = "provenance";
const SECTION_BLOCKS: &str = "blocks";
const SECTION_EMBEDDINGS: &str = "embeddings";
const SECTION_TEXT: &str = "text";
const SECTION_SPANS: &str = "spans";

/// Render the Inspect pane. Doc target → `render_doc`, chunk target →
/// `render_chunk`. No target → empty hint.
pub fn render_inspect(f: &mut Frame, area: Rect, state: &App) {
    let Some(s) = state.inspect.as_ref() else {
        f.render_widget(
            RBlock::default().title("Inspect").borders(Borders::ALL),
            area,
        );
        return;
    };
    if s.loading {
        let block = RBlock::default().title("Inspect — loading…").borders(Borders::ALL);
        f.render_widget(block, area);
        return;
    }
    match (&s.target, &s.doc, &s.chunk) {
        (Some(InspectTarget::Doc(_)), Some(doc), _) => render_doc(f, area, s, doc, &state.theme),
        (Some(InspectTarget::Chunk(_)), _, Some(chunk)) => {
            render_chunk(f, area, s, chunk, &state.theme)
        }
        _ => {
            let block = RBlock::default()
                .title("Inspect")
                .borders(Borders::ALL);
            let hint = Paragraph::new(Span::styled(
                "(no target — return to Library and press Enter on a doc, \
                 or to Search and press `i` on a hit)",
                state.theme.style(crate::theme::Role::Hint),
            ))
            .wrap(Wrap { trim: false });
            f.render_widget(hint.block(block), area);
        }
    }
}

fn render_doc(f: &mut Frame, area: Rect, s: &InspectState, doc: &CanonicalDocument, theme: &crate::theme::Theme) {
    let lines = build_doc_lines(s, doc, theme);
    let block = RBlock::default()
        .title(format!(
            "Inspect Doc — {}",
            short_id(&doc.doc_id.0)
        ))
        .borders(Borders::ALL);
    let para = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .scroll((s.scroll, 0));
    f.render_widget(para.block(block), area);
}

fn render_chunk(f: &mut Frame, area: Rect, s: &InspectState, chunk: &Chunk, theme: &crate::theme::Theme) {
    let lines = build_chunk_lines(s, chunk, theme);
    let block = RBlock::default()
        .title(format!(
            "Inspect Chunk — {}",
            short_id(&chunk.chunk_id.0)
        ))
        .borders(Borders::ALL);
    let para = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .scroll((s.scroll, 0));
    f.render_widget(para.block(block), area);
}

/// Build the wrapped Lines for a doc inspect view. Pure function so
/// snapshot tests can compare a stable prefix of lines.
pub(crate) fn build_doc_lines<'a>(
    s: &InspectState,
    doc: &'a CanonicalDocument,
    theme: &crate::theme::Theme,
) -> Vec<Line<'a>> {
    let mut lines: Vec<Line> = Vec::new();
    // Header
    lines.push(header_kv("title", &doc.title, theme));
    lines.push(header_kv("doc_path", &doc.workspace_path.0, theme));
    lines.push(header_kv("doc_id", &doc.doc_id.0, theme));
    lines.push(header_kv("lang", &doc.lang.0, theme));
    lines.push(header_kv(
        "source_type",
        &format!("{:?}", doc.metadata.source_type).to_lowercase(),
        theme,
    ));
    lines.push(header_kv(
        "trust_level",
        &format!("{:?}", doc.metadata.trust_level).to_lowercase(),
        theme,
    ));
    lines.push(header_kv("parser_version", &doc.parser_version.0, theme));
    lines.push(blank());

    // metadata
    push_section_header(&mut lines, SECTION_METADATA, s, theme);
    if !s.collapsed.contains(SECTION_METADATA) {
        lines.push(kv("aliases", &format!("{:?}", doc.metadata.aliases), theme));
        lines.push(kv("tags", &format!("{:?}", doc.metadata.tags), theme));
        lines.push(kv("created_at", &fmt_dt(&doc.metadata.created_at), theme));
        lines.push(kv("updated_at", &fmt_dt(&doc.metadata.updated_at), theme));
        // user metadata pretty-printed JSON
        if let Ok(pretty) =
            serde_json::to_string_pretty(&serde_json::Value::Object(
                doc.metadata.user.clone(),
            ))
        {
            for line in pretty.lines() {
                lines.push(Line::from(format!("  {line}")));
            }
        }
        lines.push(blank());
    }

    // provenance
    push_section_header(&mut lines, SECTION_PROVENANCE, s, theme);
    if !s.collapsed.contains(SECTION_PROVENANCE) {
        if doc.provenance.events.is_empty() {
            lines.push(Line::from(Span::styled(
                "  (no events)",
                theme.style(crate::theme::Role::Hint),
            )));
        } else {
            for ev in &doc.provenance.events {
                let kind = format!("{:?}", ev.kind).to_lowercase();
                let note = ev.note.as_deref().unwrap_or("");
                lines.push(Line::from(format!(
                    "  [{}] {} — {}{}{}",
                    fmt_dt(&ev.at),
                    ev.agent,
                    kind,
                    if note.is_empty() { "" } else { ": " },
                    note,
                )));
            }
        }
        lines.push(blank());
    }

    // blocks — section header carries the count inline so a
    // collapsed view still reports "how many" without leaking
    // body lines (R1 review: count must collapse with the rest).
    push_section_header_with_count(
        &mut lines,
        SECTION_BLOCKS,
        s,
        Some(doc.blocks.len()),
        theme,
    );
    if !s.collapsed.contains(SECTION_BLOCKS) {
        let preview_n = 16.min(doc.blocks.len());
        for (i, b) in doc.blocks.iter().take(preview_n).enumerate() {
            lines.push(Line::from(format!(
                "  [{i}] {}",
                describe_block(b)
            )));
        }
        if doc.blocks.len() > preview_n {
            lines.push(Line::from(Span::styled(
                format!("  … +{} more", doc.blocks.len() - preview_n),
                theme.style(crate::theme::Role::Hint),
            )));
        }
    }
    lines
}

pub(crate) fn build_chunk_lines<'a>(
    s: &InspectState,
    chunk: &'a Chunk,
    theme: &crate::theme::Theme,
) -> Vec<Line<'a>> {
    let mut lines: Vec<Line> = Vec::new();
    // Header
    lines.push(header_kv("chunk_id", &chunk.chunk_id.0, theme));
    lines.push(header_kv("doc_id", &chunk.doc_id.0, theme));
    lines.push(header_kv(
        "heading_path",
        &if chunk.heading_path.is_empty() {
            "-".to_string()
        } else {
            chunk.heading_path.join(" / ")
        },
        theme,
    ));
    lines.push(header_kv("chunker_version", &chunk.chunker_version.0, theme));
    lines.push(header_kv("policy_hash", &chunk.policy_hash, theme));
    lines.push(header_kv(
        "token_estimate",
        &chunk.token_estimate.to_string(),
        theme,
    ));
    lines.push(blank());

    // source spans
    push_section_header(&mut lines, SECTION_SPANS, s, theme);
    if !s.collapsed.contains(SECTION_SPANS) {
        if chunk.source_spans.is_empty() {
            lines.push(Line::from(Span::styled(
                "  (no spans)",
                theme.style(crate::theme::Role::Hint),
            )));
        } else {
            for span in &chunk.source_spans {
                lines.push(Line::from(format!("  {}", describe_span(span))));
            }
        }
        lines.push(blank());
    }

    // text
    push_section_header(&mut lines, SECTION_TEXT, s, theme);
    if !s.collapsed.contains(SECTION_TEXT) {
        for line in chunk.text.lines() {
            lines.push(Line::from(format!("  {line}")));
        }
        if chunk.text.is_empty() {
            lines.push(Line::from(Span::styled(
                "  (empty)",
                theme.style(crate::theme::Role::Hint),
            )));
        }
        lines.push(blank());
    }

    // embeddings — section header carries the block_id count inline
    // (spec § Out of scope: full embedding records lookup is P+).
    push_section_header_with_count(
        &mut lines,
        SECTION_EMBEDDINGS,
        s,
        Some(chunk.block_ids.len()),
        theme,
    );
    if !s.collapsed.contains(SECTION_EMBEDDINGS) {
        lines.push(Line::from(Span::styled(
            "  (embedding records not loaded — out of v1 scope)",
            theme.style(crate::theme::Role::Hint),
        )));
        for bid in &chunk.block_ids {
            lines.push(Line::from(format!("    {}", bid.0)));
        }
    }
    lines
}

fn header_kv(k: &str, v: &str, theme: &crate::theme::Theme) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{k:>16}: "),
            theme.style(crate::theme::Role::Heading),
        ),
        Span::raw(v.to_string()),
    ])
}

fn kv(k: &str, v: &str, theme: &crate::theme::Theme) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("  {k}: "),
            theme.style(crate::theme::Role::Hint),
        ),
        Span::raw(v.to_string()),
    ])
}

fn blank() -> Line<'static> {
    Line::from("")
}

fn push_section_header(
    lines: &mut Vec<Line<'static>>,
    name: &'static str,
    s: &InspectState,
    theme: &crate::theme::Theme,
) {
    push_section_header_with_count(lines, name, s, None, theme);
}

/// Section header + optional inline count. Inline-count form is used
/// where a collapsed section should still report \"how many\" — see
/// blocks / embeddings.
fn push_section_header_with_count(
    lines: &mut Vec<Line<'static>>,
    name: &'static str,
    s: &InspectState,
    count: Option<usize>,
    theme: &crate::theme::Theme,
) {
    let collapsed = s.collapsed.contains(name);
    let marker = if collapsed { "▸" } else { "▾" };
    let title = match count {
        Some(n) => format!("{marker} {name} ({n})"),
        None => format!("{marker} {name}"),
    };
    lines.push(Line::from(Span::styled(
        title,
        theme
            .style(crate::theme::Role::Warning)
            .add_modifier(Modifier::BOLD),
    )));
}

fn fmt_dt(dt: &time::OffsetDateTime) -> String {
    dt.format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "?".into())
}

fn short_id(id: &str) -> String {
    if id.len() > 12 {
        format!("{}…", &id[..12])
    } else {
        id.to_string()
    }
}

fn describe_block(b: &Block) -> String {
    match b {
        Block::Heading(h) => format!("Heading L{}: {:?}", h.level, h.text),
        Block::Paragraph(p) => {
            let snippet = p.text.lines().next().unwrap_or("");
            let trimmed = if snippet.chars().count() > 60 {
                format!("{}…", snippet.chars().take(60).collect::<String>())
            } else {
                snippet.to_string()
            };
            format!("Paragraph: {trimmed}")
        }
        Block::Quote(q) => format!("Quote: {} chars", q.text.len()),
        Block::List(l) => format!(
            "List {} ({} items)",
            if l.ordered { "ordered" } else { "unordered" },
            l.items.len()
        ),
        Block::Code(c) => format!(
            "Code [{}]: {} bytes",
            c.lang.as_deref().unwrap_or("?"),
            c.code.len()
        ),
        Block::Table(t) => format!(
            "Table: {} cols × {} rows",
            t.headers.len(),
            t.rows.len()
        ),
        Block::ImageRef(i) => format!(
            "ImageRef: src={} alt={:?} ocr={}",
            i.src,
            i.alt,
            if i.ocr.is_some() { "Y" } else { "N" }
        ),
        Block::AudioRef(a) => format!(
            "AudioRef: asset_id={} duration_ms={}",
            a.asset_id.0, a.duration_ms
        ),
    }
}

fn describe_span(span: &kebab_core::SourceSpan) -> String {
    use kebab_core::SourceSpan;
    match span {
        SourceSpan::Line { start, end } => format!("Line {start}-{end}"),
        SourceSpan::Byte { start, end } => format!("Byte {start}-{end}"),
        SourceSpan::Page {
            page,
            char_start,
            char_end,
        } => match (char_start, char_end) {
            (Some(s), Some(e)) => format!("Page {page} (chars {s}-{e})"),
            _ => format!("Page {page}"),
        },
        SourceSpan::Region { x, y, w, h } => {
            format!("Region xywh={x},{y},{w},{h}")
        }
        SourceSpan::Time { start_ms, end_ms } => {
            format!("Time {start_ms}-{end_ms} ms")
        }
    }
}

/// Inspect pane key dispatch.
pub fn handle_key_inspect(state: &mut App, key: KeyEvent) -> KeyOutcome {
    if state.error_overlay.is_some() {
        state.error_overlay = None;
        return KeyOutcome::Continue;
    }
    let Some(s) = state.inspect.as_mut() else {
        return KeyOutcome::SwitchPane(Pane::Library);
    };
    match (key.code, key.modifiers) {
        (KeyCode::Esc, _) | (KeyCode::Char('q'), _) => KeyOutcome::SwitchPane(s.return_to),
        (KeyCode::Char('j'), _) | (KeyCode::Down, _) => {
            s.scroll = s.scroll.saturating_add(1);
            KeyOutcome::Continue
        }
        (KeyCode::Char('k'), _) | (KeyCode::Up, _) => {
            s.scroll = s.scroll.saturating_sub(1);
            KeyOutcome::Continue
        }
        (KeyCode::PageDown, _) => {
            s.scroll = s.scroll.saturating_add(10);
            KeyOutcome::Continue
        }
        (KeyCode::PageUp, _) => {
            s.scroll = s.scroll.saturating_sub(10);
            KeyOutcome::Continue
        }
        (KeyCode::Char('c'), _) => {
            // Toggle all sections at once. v1 simplification per spec
            // ("focus is implicit by current scroll position; v1 may
            // simplify by toggling all sections").
            toggle_all_sections(s);
            KeyOutcome::Continue
        }
        _ => KeyOutcome::Continue,
    }
}

fn toggle_all_sections(s: &mut InspectState) {
    let candidates: &[&'static str] = &[
        SECTION_METADATA,
        SECTION_PROVENANCE,
        SECTION_BLOCKS,
        SECTION_EMBEDDINGS,
        SECTION_TEXT,
        SECTION_SPANS,
    ];
    let any_collapsed = candidates.iter().any(|n| s.collapsed.contains(*n));
    if any_collapsed {
        // Some collapsed → expand all.
        s.collapsed.clear();
    } else {
        // None collapsed → collapse all.
        for &name in candidates {
            s.collapsed.insert(name);
        }
    }
}

/// Run-loop hook: fetch doc / chunk for the current target if
/// `needs_fetch`. Synchronous (v1).
pub(crate) fn refresh_inspect(state: &mut App) -> anyhow::Result<()> {
    let cfg = state.config.clone();
    let target = {
        let s = state.inspect.as_ref().expect("inspect slot must exist");
        if !s.needs_fetch {
            return Ok(());
        }
        s.target.clone()
    };
    let Some(target) = target else {
        let s = state.inspect.as_mut().unwrap();
        s.needs_fetch = false;
        return Ok(());
    };

    {
        let s = state.inspect.as_mut().unwrap();
        s.loading = true;
    }

    match target {
        InspectTarget::Doc(doc_id) => {
            let result = kebab_app::inspect_doc_with_config(cfg, &doc_id);
            let s = state.inspect.as_mut().unwrap();
            s.loading = false;
            s.needs_fetch = false;
            match result {
                Ok(doc) => {
                    s.doc = Some(doc);
                    s.chunk = None;
                    s.scroll = 0;
                }
                Err(e) => return Err(e),
            }
        }
        InspectTarget::Chunk(chunk_id) => {
            let result = kebab_app::inspect_chunk_with_config(cfg, &chunk_id);
            let s = state.inspect.as_mut().unwrap();
            s.loading = false;
            s.needs_fetch = false;
            match result {
                Ok(chunk) => {
                    s.chunk = Some(chunk);
                    s.doc = None;
                    s.scroll = 0;
                }
                Err(e) => return Err(e),
            }
        }
    }
    Ok(())
}

/// Helper used by Library / Search panes to enter Inspect with a
/// specific target. Sets `needs_fetch` so the run-loop tick
/// services the `kebab-app::inspect_*` call.
pub fn enter_inspect(state: &mut App, target: InspectTarget, return_to: Pane) {
    if state.inspect.is_none() {
        state.inspect = Some(InspectState::default());
    }
    let s = state.inspect.as_mut().unwrap();
    s.target = Some(target);
    s.return_to = return_to;
    s.needs_fetch = true;
    s.doc = None;
    s.chunk = None;
    s.scroll = 0;
    s.collapsed.clear();
}
