//! p9-fb-11: render a markdown string to ratatui `Line`s with the
//! current `Theme`.
//!
//! Scope (per spec p9-fb-11):
//! - inline `**bold**`, `*italic*`, `` `code` `` → `Modifier::*`
//! - inline `[text](url)` → underline + `Role::CitationMarker` color
//! - block heading `#`-`######` → bold + role-graded color
//! - block list (`-` / `*` / `1.`) → indent + bullet glyph
//! - block code fence ```` ``` ```` → indented monospace lines
//! - block table `| col |` → row text, `|` separators preserved
//! - block blockquote `>` → left bar `▎` + dim
//!
//! Streaming: the caller re-renders the full answer text every
//! frame. The Ratatui-side cost is a few µs per kilobyte (pulldown
//! is tokenizer-fast), so re-parse is fine. Incomplete inline spans
//! (e.g. unterminated `**`) emit their literal characters as raw
//! text — `pulldown-cmark` treats them as Text events when no
//! closing marker shows up.
//!
//! Out of scope (per spec): images (terminal can't render them),
//! link click/follow (P+).

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::theme::{Role, Theme};

/// Render a markdown answer into styled `Line`s. Always returns at
/// least one line — a fully-empty input emits a single empty line so
/// callers can scroll / measure without an `is_empty()` guard.
pub fn render(text: &str, theme: &Theme) -> Vec<Line<'static>> {
    if text.is_empty() {
        return vec![Line::from("")];
    }

    let mut out: Vec<Line<'static>> = Vec::new();
    // Pending Spans for the line currently under construction.
    // Flushed into `out` on every Hard/SoftBreak or when a block
    // boundary closes the current line.
    let mut current: Vec<Span<'static>> = Vec::new();
    // Stack of inline modifiers active right now (Strong / Emph
    // nest, plus inline Code as a one-off bg/style flag).
    let mut style_stack: Vec<Modifier> = Vec::new();
    // Heading level overrides the per-Text style with a Role-based
    // color until End(Heading) pops it.
    let mut heading_role: Option<Role> = None;
    // Link target: when set, every Text event inside the link gets
    // an underline + CitationMarker color.
    let mut in_link: bool = false;
    // List depth: 0 = no list, 1+ = nested. Indent grows with depth.
    let mut list_depth: usize = 0;
    // Ordered-list counter stack (one entry per ordered list level
    // we've entered). Always pushed/popped in lockstep with list_depth
    // — bullet-list entries push None (rendered as `-`).
    let mut list_counters: Vec<Option<u64>> = Vec::new();
    // Code-fence body — collected so it can be flushed as a block
    // with each line indented + dim-styled.
    let mut in_code_block: bool = false;
    let mut code_block_buf: String = String::new();
    // Blockquote depth: render lines inside with a `▎` prefix.
    let mut quote_depth: usize = 0;

    let parser = Parser::new_ext(text, Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH);

    for event in parser {
        match event {
            Event::Start(tag) => match tag {
                Tag::Heading { level, .. } => {
                    flush_current(&mut current, &mut out);
                    heading_role = Some(heading_role_for(level));
                }
                Tag::Strong => style_stack.push(Modifier::BOLD),
                Tag::Emphasis => style_stack.push(Modifier::ITALIC),
                Tag::Strikethrough => style_stack.push(Modifier::CROSSED_OUT),
                Tag::Link { .. } => in_link = true,
                Tag::List(start) => {
                    flush_current(&mut current, &mut out);
                    list_depth += 1;
                    list_counters.push(start);
                }
                Tag::Item => {
                    flush_current(&mut current, &mut out);
                    let indent = "  ".repeat(list_depth.saturating_sub(1));
                    let bullet = match list_counters.last_mut() {
                        Some(Some(n)) => {
                            let s = format!("{n}. ");
                            *n += 1;
                            s
                        }
                        _ => "- ".to_string(),
                    };
                    current.push(Span::raw(indent));
                    current.push(Span::styled(bullet, theme.style(Role::Bullet)));
                }
                Tag::CodeBlock(kind) => {
                    flush_current(&mut current, &mut out);
                    in_code_block = true;
                    code_block_buf.clear();
                    // pulldown emits `Tag::CodeBlock` once per fence; the
                    // language tag (rust, python, …) is informational —
                    // we don't syntax-highlight in v1, but log it for
                    // future reference.
                    if let CodeBlockKind::Fenced(lang) = kind {
                        if !lang.is_empty() {
                            tracing::trace!(target: "kebab-tui", %lang, "markdown code fence lang");
                        }
                    }
                }
                Tag::BlockQuote(_) => {
                    flush_current(&mut current, &mut out);
                    quote_depth += 1;
                }
                Tag::Paragraph => {
                    // No-op on Start: the next Text event will start
                    // populating `current`. End(Paragraph) flushes.
                }
                Tag::Table(_) | Tag::TableHead | Tag::TableRow => {
                    flush_current(&mut current, &mut out);
                }
                Tag::TableCell => {
                    // Cell separator. Push `| ` prefix so a row
                    // renders as `| col1 | col2 |` (markdown-style).
                    if current.is_empty() {
                        current.push(Span::styled("| ", theme.style(Role::Bullet)));
                    } else {
                        current.push(Span::styled(" | ", theme.style(Role::Bullet)));
                    }
                }
                _ => {}
            },
            Event::End(tag_end) => match tag_end {
                TagEnd::Heading(_) => {
                    heading_role = None;
                    flush_current(&mut current, &mut out);
                }
                TagEnd::Strong | TagEnd::Emphasis | TagEnd::Strikethrough => {
                    style_stack.pop();
                }
                TagEnd::Link => in_link = false,
                TagEnd::List(_) => {
                    flush_current(&mut current, &mut out);
                    list_depth = list_depth.saturating_sub(1);
                    list_counters.pop();
                }
                TagEnd::Item => {
                    flush_current(&mut current, &mut out);
                }
                TagEnd::CodeBlock => {
                    flush_code_block(&mut code_block_buf, &mut out, theme);
                    in_code_block = false;
                }
                TagEnd::BlockQuote(_) => {
                    flush_current(&mut current, &mut out);
                    quote_depth = quote_depth.saturating_sub(1);
                }
                TagEnd::Paragraph => {
                    flush_current(&mut current, &mut out);
                    // Blank line between paragraphs for readability.
                    out.push(Line::from(""));
                }
                TagEnd::TableRow | TagEnd::TableHead => {
                    // Close the row with a trailing `|`.
                    current.push(Span::styled(" |", theme.style(Role::Bullet)));
                    flush_current(&mut current, &mut out);
                }
                TagEnd::Table => {
                    flush_current(&mut current, &mut out);
                    out.push(Line::from(""));
                }
                _ => {}
            },
            Event::Text(t) => {
                if in_code_block {
                    code_block_buf.push_str(&t);
                } else {
                    let style = compose_style(theme, heading_role, &style_stack, in_link, false);
                    push_text_with_quote_prefix(
                        &mut current,
                        &mut out,
                        &t,
                        style,
                        quote_depth,
                        theme,
                    );
                }
            }
            Event::Code(c) => {
                let style = compose_style(theme, heading_role, &style_stack, in_link, true);
                current.push(Span::styled(c.into_string(), style));
            }
            Event::SoftBreak | Event::HardBreak => {
                flush_current(&mut current, &mut out);
            }
            Event::Rule => {
                flush_current(&mut current, &mut out);
                out.push(Line::from(Span::styled(
                    "─".repeat(40),
                    theme.style(Role::Bullet),
                )));
            }
            Event::Html(h) | Event::InlineHtml(h) => {
                // Render raw HTML as text — terminal can't display
                // tags. Use Hint role so it visually distinguishes
                // from user-written prose.
                current.push(Span::styled(
                    h.into_string(),
                    theme.style(Role::Hint),
                ));
            }
            Event::InlineMath(s) | Event::DisplayMath(s) => {
                // No LaTeX rendering in a terminal v1, but preserve
                // the source so the answer's math still reaches the
                // user as readable text instead of vanishing.
                current.push(Span::styled(
                    s.into_string(),
                    theme.style(Role::Hint),
                ));
            }
            Event::FootnoteReference(label) => {
                // Render as `[^label]` so the footnote anchor is
                // visible in the answer body.
                current.push(Span::styled(
                    format!("[^{label}]"),
                    theme.style(Role::CitationMarker),
                ));
            }
            Event::TaskListMarker(checked) => {
                // GFM task lists — surface as `[x] ` / `[ ] ` so
                // checklists stay legible in the answer.
                let marker = if checked { "[x] " } else { "[ ] " };
                current.push(Span::styled(marker, theme.style(Role::Bullet)));
            }
        }
    }

    // Flush any trailing line (e.g. a paragraph not yet closed —
    // happens when input ends mid-line during streaming).
    flush_current(&mut current, &mut out);

    if out.is_empty() {
        out.push(Line::from(""));
    }
    out
}

/// Map an MD heading level to a Role. H1 / H2 use `Heading` (Cyan +
/// BOLD in dark); H3+ degrade to `Title` (White + BOLD) so the
/// hierarchy stays visible without inventing new roles.
fn heading_role_for(level: HeadingLevel) -> Role {
    match level {
        HeadingLevel::H1 | HeadingLevel::H2 => Role::Heading,
        _ => Role::Title,
    }
}

/// Compose the active inline style from the heading override (if any),
/// the modifier stack (Strong/Emph/Strikethrough), and the link /
/// inline-code flags.
///
/// Layering rule: the **base color** comes from the most-specific
/// container — heading first, then link, then inline code, then body.
/// **Modifiers** from `style_stack` AND from link/inline-code overlay
/// on top regardless. So `# Section [docs](url) `code``:
/// - `docs` keeps the heading color (Cyan + BOLD) but also gains
///   `UNDERLINED` from the link, signalling "clickable text" without
///   losing the heading's hierarchy color.
/// - `code` keeps the heading color and adds `DIM` from inline-code.
fn compose_style(
    theme: &Theme,
    heading_role: Option<Role>,
    style_stack: &[Modifier],
    in_link: bool,
    inline_code: bool,
) -> Style {
    let base = if let Some(role) = heading_role {
        theme.style(role)
    } else if in_link {
        theme.style(Role::CitationMarker)
    } else if inline_code {
        // Inline code — represent with Hint (DIM) since Terminal
        // doesn't reliably do bg colors without 256-color, and italic
        // is taken by Emphasis. Conservative-but-visible cue.
        theme.style(Role::Hint)
    } else {
        theme.style(Role::Body)
    };
    let mut acc = Modifier::empty();
    for m in style_stack {
        acc.insert(*m);
    }
    if in_link {
        acc.insert(Modifier::UNDERLINED);
    }
    if inline_code && heading_role.is_some() {
        // Inside a heading, inline code keeps heading color but takes
        // the DIM marker so it still reads as code.
        acc.insert(Modifier::DIM);
    }
    base.add_modifier(acc)
}

/// Push a text run into the current line, splitting on any embedded
/// `\n` (pulldown emits these inside paragraphs occasionally). Each
/// new line inherits the blockquote prefix.
fn push_text_with_quote_prefix(
    current: &mut Vec<Span<'static>>,
    out: &mut Vec<Line<'static>>,
    text: &str,
    style: Style,
    quote_depth: usize,
    theme: &Theme,
) {
    if quote_depth > 0 && current.is_empty() {
        current.push(quote_prefix(quote_depth, theme));
    }
    let mut first = true;
    for chunk in text.split('\n') {
        if !first {
            flush_current(current, out);
            if quote_depth > 0 {
                current.push(quote_prefix(quote_depth, theme));
            }
        }
        if !chunk.is_empty() {
            current.push(Span::styled(chunk.to_string(), style));
        }
        first = false;
    }
}

/// `▎` glyph repeated for nested quotes, dim-styled.
fn quote_prefix(depth: usize, theme: &Theme) -> Span<'static> {
    Span::styled("▎".repeat(depth) + " ", theme.style(Role::Hint))
}

/// Move `current` into a new `Line` and clear it. No-op when empty.
fn flush_current(current: &mut Vec<Span<'static>>, out: &mut Vec<Line<'static>>) {
    if current.is_empty() {
        return;
    }
    let line: Vec<Span<'static>> = std::mem::take(current);
    out.push(Line::from(line));
}

/// Flush a captured code-fence body. Each source line becomes one
/// output `Line`, indented `  ` and `Hint`-styled (DIM) so it visually
/// stands apart from prose. A blank line follows the block.
fn flush_code_block(buf: &mut String, out: &mut Vec<Line<'static>>, theme: &Theme) {
    if buf.is_empty() {
        return;
    }
    for line in buf.lines() {
        out.push(Line::from(Span::styled(
            format!("  {line}"),
            theme.style(Role::Hint),
        )));
    }
    out.push(Line::from(""));
    buf.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn theme() -> Theme {
        Theme::dark()
    }

    /// Empty input still produces a single empty line so callers can
    /// scroll / measure without an `is_empty()` guard.
    #[test]
    fn empty_input_returns_one_empty_line() {
        let lines = render("", &theme());
        assert_eq!(lines.len(), 1);
        assert_eq!(line_text(&lines[0]), "");
    }

    /// Plain text emits one Line with one Span (no styling beyond
    /// `Role::Body`'s default).
    #[test]
    fn plain_text_one_paragraph_one_line() {
        let lines = render("hello world", &theme());
        // Paragraph end emits a blank line, so 2 lines total: text + blank.
        assert_eq!(lines.len(), 2);
        assert_eq!(line_text(&lines[0]), "hello world");
        assert_eq!(line_text(&lines[1]), "");
    }

    /// `**bold**` produces a Span with BOLD modifier.
    #[test]
    fn bold_emits_bold_modifier() {
        let lines = render("**hi**", &theme());
        let bold_spans: Vec<&Span> = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .filter(|s| s.style.add_modifier.contains(Modifier::BOLD))
            .collect();
        assert!(!bold_spans.is_empty(), "expected at least one BOLD span");
        let combined: String = bold_spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(combined, "hi");
    }

    /// `*italic*` produces a Span with ITALIC modifier.
    #[test]
    fn italic_emits_italic_modifier() {
        let lines = render("*hi*", &theme());
        let italic_spans: Vec<&Span> = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .filter(|s| s.style.add_modifier.contains(Modifier::ITALIC))
            .collect();
        assert!(!italic_spans.is_empty(), "expected at least one ITALIC span");
        let combined: String = italic_spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(combined, "hi");
    }

    /// Inline `` `code` `` emits a span with the inline-code style
    /// (DIM in our v1 mapping). Content matches the literal.
    #[test]
    fn inline_code_emits_styled_span() {
        let lines = render("call `frob()` here", &theme());
        let code_spans: Vec<&Span> = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .filter(|s| s.content.as_ref() == "frob()")
            .collect();
        assert_eq!(code_spans.len(), 1);
        assert!(
            code_spans[0].style.add_modifier.contains(Modifier::DIM)
                || code_spans[0].style.fg.is_some()
                || code_spans[0].style.bg.is_some(),
            "inline code span carries no style: {:?}",
            code_spans[0].style
        );
    }

    /// p9-fb-11 R1: link inside a heading layers — heading color
    /// stays (Cyan + BOLD) AND link's UNDERLINE marker is added.
    #[test]
    fn link_inside_heading_layers_underline_on_heading_color() {
        let lines = render("# Section [docs](https://x)", &theme());
        let docs = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .find(|s| s.content.as_ref() == "docs")
            .expect("link text span");
        assert!(
            docs.style.add_modifier.contains(Modifier::UNDERLINED),
            "link inside heading should still get UNDERLINED: {:?}",
            docs.style
        );
        assert!(
            docs.style.add_modifier.contains(Modifier::BOLD),
            "link inside heading should keep heading BOLD: {:?}",
            docs.style
        );
    }

    /// p9-fb-11 R1: math expressions render as text (they used to be
    /// silently dropped, losing answer content).
    #[test]
    fn inline_and_display_math_render_as_text() {
        let inline = render("see $E = mc^2$ here", &theme());
        let combined: String = inline.iter().map(line_text).collect::<String>();
        assert!(
            combined.contains("E = mc^2"),
            "inline math content dropped: {combined:?}"
        );
        let display = render("$$\\sum_i x_i$$", &theme());
        let combined: String = display.iter().map(line_text).collect::<String>();
        assert!(
            combined.contains("\\sum_i x_i") || combined.contains("sum_i x_i"),
            "display math content dropped: {combined:?}"
        );
    }

    /// p9-fb-11 R1: GFM task lists render as `[ ] ` / `[x] `.
    #[test]
    fn task_list_renders_checkbox_glyphs() {
        let md = "- [ ] todo\n- [x] done";
        let lines = render(md, &theme());
        let texts: Vec<String> = lines.iter().map(line_text).collect();
        assert!(
            texts.iter().any(|t| t.contains("[ ] todo")),
            "unchecked task missing: {texts:?}"
        );
        assert!(
            texts.iter().any(|t| t.contains("[x] done")),
            "checked task missing: {texts:?}"
        );
    }

    /// `[text](https://x)` underlines `text`.
    #[test]
    fn link_underlines_text() {
        let lines = render("see [docs](https://example.com)", &theme());
        let link_span = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .find(|s| s.content.as_ref() == "docs")
            .expect("link text span present");
        assert!(
            link_span.style.add_modifier.contains(Modifier::UNDERLINED),
            "link span missing UNDERLINE: {:?}",
            link_span.style
        );
    }

    /// Heading `# Title` styles the title with the H1 Role::Heading
    /// (Cyan + BOLD in dark).
    #[test]
    fn heading_h1_styles_title() {
        let lines = render("# Title here", &theme());
        let title_span = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .find(|s| s.content.as_ref().contains("Title here"))
            .expect("heading text span");
        assert!(title_span.style.add_modifier.contains(Modifier::BOLD));
    }

    /// `- item` emits a bullet glyph + indented item text.
    #[test]
    fn bullet_list_renders_dash_prefix() {
        let lines = render("- first\n- second", &theme());
        let texts: Vec<String> = lines.iter().map(line_text).collect();
        assert!(texts.iter().any(|t| t.starts_with("- first")));
        assert!(texts.iter().any(|t| t.starts_with("- second")));
    }

    /// `1.` / `2.` numbered list prefixes with the actual numbers.
    #[test]
    fn ordered_list_renders_numbered_prefix() {
        let lines = render("1. alpha\n2. beta", &theme());
        let texts: Vec<String> = lines.iter().map(line_text).collect();
        assert!(texts.iter().any(|t| t.starts_with("1. alpha")));
        assert!(texts.iter().any(|t| t.starts_with("2. beta")));
    }

    /// Code fence body is preserved verbatim per line, indented two
    /// spaces.
    #[test]
    fn code_fence_preserves_body_lines() {
        let md = "```rust\nlet x = 1;\nlet y = 2;\n```";
        let lines = render(md, &theme());
        let texts: Vec<String> = lines.iter().map(line_text).collect();
        assert!(texts.iter().any(|t| t == "  let x = 1;"));
        assert!(texts.iter().any(|t| t == "  let y = 2;"));
    }

    /// Blockquote `> hi` prefixes the line with `▎`.
    #[test]
    fn blockquote_renders_left_bar() {
        let lines = render("> quoted text", &theme());
        let texts: Vec<String> = lines.iter().map(line_text).collect();
        assert!(
            texts.iter().any(|t| t.starts_with("▎")),
            "no `▎` prefix in: {texts:?}"
        );
    }

    /// 2x2 table renders as `| col | col |` rows. We don't promote to
    /// `Table` widget since the answer area uses Paragraph-flow.
    #[test]
    fn table_renders_pipe_separated_rows() {
        let md = "| a | b |\n| - | - |\n| 1 | 2 |";
        let lines = render(md, &theme());
        let texts: Vec<String> = lines.iter().map(line_text).collect();
        // header row + body row, both with `|` separators
        assert!(
            texts.iter().any(|t| t.contains("| a") && t.contains("b |")),
            "header row missing pipes: {texts:?}"
        );
        assert!(
            texts.iter().any(|t| t.contains("| 1") && t.contains("2 |")),
            "body row missing pipes: {texts:?}"
        );
    }

    /// Streaming partial: an unterminated `**` MUST NOT drop the
    /// content text. pulldown-cmark 0.13 emits the suffix as a Text
    /// event (with or without preserving the `**` literal — both are
    /// acceptable as long as `still typing` reaches the output).
    /// Splitting the assertion: content presence is a hard constraint
    /// (regression catches `pulldown` upgrades that lose characters);
    /// the literal `**` is cosmetic and not pinned.
    #[test]
    fn unterminated_bold_does_not_drop_content() {
        let lines = render("**still typing", &theme());
        let combined: String = lines.iter().map(line_text).collect::<String>();
        assert!(
            combined.contains("still typing"),
            "stream-mid output dropped content text: {combined:?}"
        );
    }

    /// Composite snapshot — heading + paragraph + list + code render
    /// in document order without swallowing content.
    #[test]
    fn composite_snapshot_preserves_document_order() {
        let md = "# Goal\n\nDescription **here**.\n\n- alpha\n- beta\n\n```\nlet x = 1;\n```";
        let lines = render(md, &theme());
        let texts: Vec<String> = lines.iter().map(line_text).collect();
        let heading_idx = texts.iter().position(|t| t.contains("Goal")).unwrap();
        let para_idx = texts.iter().position(|t| t.contains("Description")).unwrap();
        let alpha_idx = texts.iter().position(|t| t.contains("alpha")).unwrap();
        let code_idx = texts.iter().position(|t| t.contains("let x = 1;")).unwrap();
        assert!(heading_idx < para_idx);
        assert!(para_idx < alpha_idx);
        assert!(alpha_idx < code_idx);
    }

    fn line_text(line: &Line<'_>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }
}
