//! Markdown body → flat `Vec<kb_parse_types::ParsedBlock>` (§3.4 / §3.7b).
//!
//! Uses `pulldown-cmark` (with GFM tables enabled at runtime via
//! `Options::ENABLE_TABLES`) to walk the body once and emit a flat list of
//! parsed blocks. Heading paths are computed by tracking the most-recent
//! heading text at each level. Source spans are reported as
//! [`kb_core::SourceSpan::Line`] in 1-indexed file-line coordinates by
//! converting `pulldown-cmark`'s byte offsets to line numbers and adding the
//! caller-supplied `body_offset_lines`.
//!
//! ## Coordinate conventions
//!
//! * Body lines are *0-indexed* internally — line 0 is the first line of the
//!   body slice. We map to file coordinates with
//!   `file_line = body_line_zero_indexed + body_offset_lines`. So a caller
//!   that passes `body_offset_lines = 6` is saying "the first line of `body`
//!   is line 6 of the original file".
//! * `SourceSpan::Line { start, end }` is inclusive on both ends.
//!
//! ## Inline filter
//!
//! [`kb_core::Inline`] only models `Text | Code | Link | Strong | Emph`.
//! Inline images, footnotes, hard breaks, etc. are dropped silently per
//! design §3.4. Block-level `![alt](src)` (an image as the sole content of a
//! paragraph) is lifted to [`kb_parse_types::ParsedPayload::ImageRef`].
//!
//! ## CRLF
//!
//! Line numbers are computed by counting `\n` bytes in the prefix; CRLF
//! input still has `\n` at end-of-line so the math is identical to LF input.
//! `pulldown-cmark` may include `\r` characters inside its emitted text
//! events; we leave them as-is for now (they round-trip through serde).

use std::ops::Range;

use kb_core::{Inline, SourceSpan};
use kb_parse_types::{ParsedBlock, ParsedBlockKind, ParsedPayload, Warning, WarningKind};
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

/// Parse a Markdown body into a flat `Vec<ParsedBlock>` plus any warnings.
///
/// `body_offset_lines` is added to every body-relative line number so that
/// reported [`SourceSpan::Line`] values address the **original** file. See
/// the module-level docs for the coordinate convention.
///
/// # Errors
///
/// `Err` is reserved for genuinely fatal conditions; the current
/// implementation never produces one — even adversarial inputs degrade to
/// `Ok((vec![], vec![Warning::ExtractFailed]))` via a top-level
/// `catch_unwind` guard. The `Result` is kept on the signature so a future
/// I/O-backed input can be added without breaking callers.
pub fn parse_blocks(
    body: &[u8],
    body_offset_lines: u32,
) -> anyhow::Result<(Vec<ParsedBlock>, Vec<Warning>)> {
    // Adversarial-input safety: pulldown-cmark is documented as
    // panic-free on valid UTF-8, but a defensive catch_unwind keeps the
    // contract ("never panics") even if the dependency regresses.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        parse_blocks_inner(body, body_offset_lines)
    }));
    match result {
        Ok(out) => Ok(out),
        Err(_) => {
            tracing::warn!("parse_blocks panicked on adversarial input; returning empty");
            Ok((
                Vec::new(),
                vec![Warning {
                    kind: WarningKind::ExtractFailed,
                    note: "pulldown-cmark panicked; body discarded".to_string(),
                }],
            ))
        }
    }
}

fn parse_blocks_inner(body: &[u8], body_offset_lines: u32) -> (Vec<ParsedBlock>, Vec<Warning>) {
    // Lossy UTF-8: pulldown-cmark wants `&str`. Non-UTF-8 inputs get
    // U+FFFD substitution; the byte→line mapping is built from the original
    // bytes so spans remain accurate.
    let body_str = match std::str::from_utf8(body) {
        Ok(s) => std::borrow::Cow::Borrowed(s),
        Err(_) => String::from_utf8_lossy(body),
    };

    let line_index = LineIndex::new(body);
    let mut warnings: Vec<Warning> = Vec::new();
    let mut state = WalkState::new(body_offset_lines, &line_index, body);

    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    let parser = Parser::new_ext(body_str.as_ref(), options);

    for (event, range) in parser.into_offset_iter() {
        state.handle_event(event, range, &mut warnings);
    }

    // If line-number arithmetic saturated at any point (e.g. caller passed
    // `body_offset_lines = u32::MAX`), discard the (potentially clamped)
    // blocks and surface a single `ExtractFailed` warning. Returning the
    // possibly-inverted spans would be more harmful than dropping output.
    if state.overflow_detected {
        let at = state
            .overflow_at_body_line
            .map(|n| n.to_string())
            .unwrap_or_else(|| "?".to_string());
        return (
            Vec::new(),
            vec![Warning {
                kind: WarningKind::ExtractFailed,
                note: format!("body_offset_lines overflow at body line {at}"),
            }],
        );
    }

    (state.finish(), warnings)
}

// ---------------------------------------------------------------------------
// Line index — byte offset → 1-indexed body line
// ---------------------------------------------------------------------------

/// Maps byte offsets in the body to body-line numbers (0-indexed).
///
/// Built once per parse. Stores the byte offset of every newline so
/// `byte → line` is a binary search.
struct LineIndex {
    /// Byte offsets of `\n` characters in the body, in ascending order.
    /// `newlines[i]` is the position of the `i`-th newline (0-indexed).
    newlines: Vec<usize>,
    body_len: usize,
}

impl LineIndex {
    fn new(body: &[u8]) -> Self {
        let newlines = body
            .iter()
            .enumerate()
            .filter_map(|(i, &b)| if b == b'\n' { Some(i) } else { None })
            .collect();
        Self {
            newlines,
            body_len: body.len(),
        }
    }

    /// Body line (0-indexed) containing byte offset `pos`.
    ///
    /// Bytes before the first `\n` are line 0. Bytes after the last `\n` are
    /// line `newlines.len()`. A position equal to a newline byte itself is
    /// considered to be on the line *ending at* that newline (so `pos` that
    /// points to `\n` reports the line that newline terminates).
    fn line_zero_indexed(&self, pos: usize) -> u32 {
        // Clamp to body length — pulldown-cmark may emit ranges with `end`
        // equal to body.len() for the final block.
        let pos = pos.min(self.body_len);
        // partition_point returns the count of newlines whose offset is `< pos`.
        // That's exactly the line-index for the line containing `pos`, with
        // the convention "trailing newline belongs to the previous line".
        let n_before = self.newlines.partition_point(|&nl| nl < pos);
        n_before as u32
    }

    /// Body line (0-indexed) containing byte offset `pos - 1` — i.e., the
    /// line on which the byte just **before** `pos` lives. Used for `end`
    /// positions of `pulldown-cmark` ranges, which are exclusive (point one
    /// past the last byte of the construct).
    fn end_line_zero_indexed(&self, end: usize) -> u32 {
        if end == 0 {
            return 0;
        }
        self.line_zero_indexed(end - 1)
    }
}

// ---------------------------------------------------------------------------
// Walker state machine
// ---------------------------------------------------------------------------

struct WalkState<'a> {
    body_offset_lines: u32,
    line_index: &'a LineIndex,
    body: &'a [u8],

    blocks: Vec<ParsedBlock>,
    /// Most-recent heading text at each level (1..=6), or `None` if no
    /// heading at that level has been seen since a higher-level heading was
    /// encountered. `heading_path()` snapshots this into a `Vec<String>`.
    heading_stack: [Option<String>; 6],

    /// Accumulator for the currently-open block. We push events into this
    /// frame and convert to a `ParsedBlock` when the matching End tag fires.
    frames: Vec<Frame>,

    /// Set if any line-number arithmetic in [`WalkState::span_for`] would
    /// overflow `u32::MAX`. When set, [`Self::finish`] discards accumulated
    /// blocks and the caller emits a single `Warning::ExtractFailed` instead.
    overflow_detected: bool,
    /// Body-line (0-indexed) where overflow was first observed, for the
    /// warning note.
    overflow_at_body_line: Option<u32>,
}

/// One in-flight container we are accumulating events into. Stack-shaped so
/// nested constructs (list inside list, paragraph inside blockquote) work.
enum Frame {
    Heading {
        level: u8,
        range: Range<usize>,
        inlines: InlineBuf,
    },
    Paragraph {
        range: Range<usize>,
        inlines: InlineBuf,
        /// Block-level image detection: tracks the single-image-only
        /// signature `![alt](src)` as a paragraph's *entire* content.
        ///
        /// When `Tag::Image` opens, `image_depth` is bumped (>0 ⇒ alt-text
        /// accumulates into `image_alt` and is suppressed from `inlines`).
        /// `image_count` records how many distinct images we've seen and
        /// `non_image_text_seen` flags any other inline content. At
        /// `End(Paragraph)` the paragraph is lifted to `ImageRef` iff
        /// `image_count == 1 && !non_image_text_seen`.
        image_depth: u32,
        image_count: u32,
        non_image_text_seen: bool,
        image_src: Option<String>,
        image_alt: String,
    },
    Quote {
        range: Range<usize>,
        children: Vec<ParsedBlock>,
    },
    List {
        ordered: bool,
        range: Range<usize>,
        items: Vec<Vec<Inline>>,
        /// True iff the immediate parent of this list is another list
        /// item — i.e. this is a nested sub-list. Nested sub-lists get
        /// flattened into the parent item's inline buffer instead of
        /// being emitted as their own `ParsedPayload::List`.
        nested_in_item: bool,
    },
    /// One `<li>`. Inlines flow into `inlines`; nested sub-lists append
    /// flattened text into `inlines` as well.
    ListItem { inlines: InlineBuf },
    Code {
        lang: Option<String>,
        range: Range<usize>,
        code: String,
    },
    Table {
        range: Range<usize>,
        headers: Vec<String>,
        rows: Vec<Vec<String>>,
        in_head: bool,
        current_row: Vec<String>,
        current_cell: String,
        cols: usize,
        malformed: Option<String>,
    },
}

/// Inline accumulator with a stack of formatting wrappers (Strong / Emph /
/// Link). The base of the stack is the top-level inline list.
struct InlineBuf {
    /// Stack: bottom is the top-level vec; each push starts a new nested
    /// `Vec<Inline>` for `Strong`/`Emph`/etc. that gets popped & wrapped
    /// when its End tag fires.
    stack: Vec<InlineFrame>,
    /// Plain-text accumulator for `paragraph.text` — the literal text shown
    /// to the user without formatting. Matches the inline events 1:1.
    text: String,
}

enum InlineFrame {
    Top(Vec<Inline>),
    Strong(Vec<Inline>),
    Emph(Vec<Inline>),
    Link { href: String, text: String, kids: Vec<Inline> },
}

impl InlineBuf {
    fn new() -> Self {
        Self {
            stack: vec![InlineFrame::Top(Vec::new())],
            text: String::new(),
        }
    }

    fn push_inline(&mut self, inline: Inline) {
        match self.stack.last_mut().expect("inline stack non-empty") {
            InlineFrame::Top(v) => v.push(inline),
            InlineFrame::Strong(v) => v.push(inline),
            InlineFrame::Emph(v) => v.push(inline),
            InlineFrame::Link { kids, .. } => kids.push(inline),
        }
    }

    fn push_text(&mut self, s: &str) {
        self.text.push_str(s);
        self.push_inline(Inline::Text(s.to_string()));
    }

    fn push_code(&mut self, s: &str) {
        self.text.push_str(s);
        self.push_inline(Inline::Code(s.to_string()));
    }

    fn open_strong(&mut self) {
        self.stack.push(InlineFrame::Strong(Vec::new()));
    }
    fn close_strong(&mut self) {
        if let Some(InlineFrame::Strong(kids)) = self.stack.pop() {
            self.push_inline(Inline::Strong(kids));
        }
    }

    fn open_emph(&mut self) {
        self.stack.push(InlineFrame::Emph(Vec::new()));
    }
    fn close_emph(&mut self) {
        if let Some(InlineFrame::Emph(kids)) = self.stack.pop() {
            self.push_inline(Inline::Emph(kids));
        }
    }

    fn open_link(&mut self, href: String) {
        self.stack.push(InlineFrame::Link {
            href,
            text: String::new(),
            kids: Vec::new(),
        });
    }
    fn close_link(&mut self) {
        if let Some(InlineFrame::Link { href, text, kids }) = self.stack.pop() {
            // Flatten the link's text contents to a single string for the
            // `Inline::Link.text` field. Code/strong/emph inside a link are
            // collapsed to their plain text — `Inline::Link` doesn't model
            // formatting inside the link.
            let flat = if !text.is_empty() {
                text
            } else {
                flatten_inlines_to_text(&kids)
            };
            self.push_inline(Inline::Link { text: flat, href });
        }
    }

    /// Append plain text to the current link's flattened text accumulator
    /// (called from inside a link frame).
    fn push_link_text(&mut self, s: &str) {
        if let Some(InlineFrame::Link { text, .. }) = self.stack.last_mut() {
            text.push_str(s);
        }
    }

    fn finish(mut self) -> (Vec<Inline>, String) {
        // Normal flow: the only remaining frame is the Top, which we unwrap.
        // If formatting tags were unbalanced we close them defensively.
        while self.stack.len() > 1 {
            match self.stack.pop().unwrap() {
                InlineFrame::Strong(kids) => self.push_inline(Inline::Strong(kids)),
                InlineFrame::Emph(kids) => self.push_inline(Inline::Emph(kids)),
                InlineFrame::Link { href, text, kids } => {
                    let flat = if !text.is_empty() {
                        text
                    } else {
                        flatten_inlines_to_text(&kids)
                    };
                    self.push_inline(Inline::Link { text: flat, href });
                }
                InlineFrame::Top(_) => break,
            }
        }
        let top = match self.stack.pop().unwrap() {
            InlineFrame::Top(v) => v,
            _ => Vec::new(),
        };
        (top, self.text)
    }

}

/// Flatten an emitted block into the inline buffer of an enclosing list
/// item, preserving content but losing structure. This keeps document
/// order and avoids "block escapes the list" — the alternative would be
/// pushing the block to the top-level `blocks` vec, where it appears
/// after the entire list and out of source order.
///
/// Code blocks render as a fenced-text approximation so the language
/// hint and body survive. Images render as `![alt](src)`. Headings,
/// tables, quotes, etc. render as their text payload.
fn flatten_block_into_item(block: &ParsedBlock, inlines: &mut InlineBuf) {
    match &block.payload {
        ParsedPayload::Code { lang, code } => {
            let mut rendered = String::from("\n\n```");
            if let Some(l) = lang {
                rendered.push_str(l);
            }
            rendered.push('\n');
            rendered.push_str(code);
            if !code.ends_with('\n') {
                rendered.push('\n');
            }
            rendered.push_str("```\n");
            inlines.push_text(&rendered);
        }
        ParsedPayload::ImageRef { src, alt } => {
            inlines.push_text(&format!("![{alt}]({src})"));
        }
        ParsedPayload::AudioRef { src } => {
            inlines.push_text(&format!("[audio]({src})"));
        }
        ParsedPayload::Heading { level, text } => {
            // Render with leading hashes so structure is recognizable.
            let hashes = "#".repeat((*level as usize).clamp(1, 6));
            inlines.push_text(&format!("\n{hashes} {text}\n"));
        }
        ParsedPayload::Paragraph { text, inlines: child } => {
            // Paragraphs inside list items normally don't reach this path
            // (the Start(Tag::Paragraph) handler suppresses creating a
            // Paragraph frame when the parent is a ListItem). This branch
            // is defensive — e.g. a paragraph emitted via some future
            // synthetic-event path.
            inlines.push_text("\n");
            for c in child {
                inlines.push_inline(c.clone());
            }
            inlines.text.push_str(text);
        }
        ParsedPayload::Quote { text, .. } => {
            inlines.push_text(&format!("\n> {text}\n"));
        }
        ParsedPayload::List { ordered, items } => {
            // A non-nested-flag List that still ended up inside a ListItem
            // (shouldn't normally happen — `nested_in_item` flattens at
            // `End(List)`). Fall back to the same rendering as the nested
            // path for safety.
            let mut rendered = String::new();
            for (i, item) in items.iter().enumerate() {
                rendered.push('\n');
                rendered.push_str("  ");
                if *ordered {
                    rendered.push_str(&format!("{}. ", i + 1));
                } else {
                    rendered.push_str("- ");
                }
                rendered.push_str(&flatten_inlines_to_text(item));
            }
            inlines.push_text(&rendered);
        }
        ParsedPayload::Table { headers, rows } => {
            // Pipe-table approximation; structure lost, content preserved.
            let mut rendered = String::from("\n");
            rendered.push_str(&headers.join(" | "));
            rendered.push('\n');
            for row in rows {
                rendered.push_str(&row.join(" | "));
                rendered.push('\n');
            }
            inlines.push_text(&rendered);
        }
    }
}

fn flatten_inlines_to_text(inlines: &[Inline]) -> String {
    let mut out = String::new();
    for i in inlines {
        flatten_one(i, &mut out);
    }
    out
}

fn flatten_one(i: &Inline, out: &mut String) {
    match i {
        Inline::Text(s) | Inline::Code(s) => out.push_str(s),
        Inline::Link { text, .. } => out.push_str(text),
        Inline::Strong(v) | Inline::Emph(v) => {
            for c in v {
                flatten_one(c, out);
            }
        }
    }
}

impl<'a> WalkState<'a> {
    fn new(body_offset_lines: u32, line_index: &'a LineIndex, body: &'a [u8]) -> Self {
        Self {
            body_offset_lines,
            line_index,
            body,
            blocks: Vec::new(),
            heading_stack: Default::default(),
            frames: Vec::new(),
            overflow_detected: false,
            overflow_at_body_line: None,
        }
    }

    fn finish(self) -> Vec<ParsedBlock> {
        self.blocks
    }

    fn heading_path(&self) -> Vec<String> {
        // Skip slots whose stored heading text is empty (e.g. a `#` heading
        // with no following text). We deliberately keep `Some("")` in the
        // stack so deeper headings still nest under their implicit slot,
        // but the path itself filters empties out so child blocks don't
        // get a `""` segment polluting their ancestry.
        self.heading_stack
            .iter()
            .filter_map(|s| s.clone().filter(|t| !t.is_empty()))
            .collect()
    }

    fn span_for(&mut self, range: &Range<usize>) -> SourceSpan {
        let start_body = self.line_index.line_zero_indexed(range.start);
        let end_body = if range.end <= range.start {
            start_body
        } else {
            self.line_index.end_line_zero_indexed(range.end)
        };
        // Saturating add — but also remember whether overflow happened so the
        // caller can degrade gracefully rather than silently emitting an
        // inverted span. Without this guard, debug builds panic with
        // "attempt to add with overflow" (caught by `catch_unwind`, masking
        // the real cause) and release builds wrap to `start > end`.
        match (
            start_body.checked_add(self.body_offset_lines),
            end_body.checked_add(self.body_offset_lines),
        ) {
            (Some(start), Some(end)) => SourceSpan::Line { start, end },
            _ => {
                if !self.overflow_detected {
                    self.overflow_detected = true;
                    self.overflow_at_body_line = Some(start_body);
                }
                SourceSpan::Line {
                    start: start_body.saturating_add(self.body_offset_lines),
                    end: end_body.saturating_add(self.body_offset_lines),
                }
            }
        }
    }

    /// Where to emit a finished block: into the current container if any
    /// (Quote / ListItem), otherwise into the top-level `blocks` vec.
    ///
    /// Block-level content (code, image, heading, table, ...) inside a list
    /// item cannot be represented structurally by `ParsedPayload::List`
    /// (items hold `Vec<Inline>`, not child blocks). To preserve content
    /// and document order we **flatten** such blocks into a textual
    /// rendering and append it to the enclosing list item's inline buffer.
    /// Without this, a code block inside a list item escapes to the
    /// top-level `blocks` vec — out of order, and detached from its parent.
    fn emit_block(&mut self, block: ParsedBlock) {
        // Find the nearest enclosing container that accepts child blocks.
        // Walk in reverse: ListItem and Quote both qualify; we honor
        // whichever is closer to the top of the frame stack.
        for idx in (0..self.frames.len()).rev() {
            match &mut self.frames[idx] {
                Frame::Quote { children, .. } => {
                    children.push(block);
                    return;
                }
                Frame::ListItem { inlines } => {
                    // Render the block as text + inlines and append to the
                    // item's inline buffer. Document order is preserved
                    // because we run inside the item's frame.
                    flatten_block_into_item(&block, inlines);
                    return;
                }
                _ => continue,
            }
        }
        self.blocks.push(block);
    }

    fn handle_event(&mut self, event: Event<'_>, range: Range<usize>, warnings: &mut Vec<Warning>) {
        match event {
            // ---- Container starts -----------------------------------------------
            Event::Start(Tag::Heading { level, .. }) => {
                self.frames.push(Frame::Heading {
                    level: heading_level_to_u8(level),
                    range,
                    inlines: InlineBuf::new(),
                });
            }
            Event::Start(Tag::Paragraph) => {
                // If we're directly inside a list item, the inlines flow into
                // the item, not a new paragraph block.
                if matches!(self.frames.last(), Some(Frame::ListItem { .. })) {
                    return;
                }
                self.frames.push(Frame::Paragraph {
                    range,
                    inlines: InlineBuf::new(),
                    image_depth: 0,
                    image_count: 0,
                    non_image_text_seen: false,
                    image_src: None,
                    image_alt: String::new(),
                });
            }
            Event::Start(Tag::BlockQuote(_)) => {
                self.frames.push(Frame::Quote {
                    range,
                    children: Vec::new(),
                });
            }
            Event::Start(Tag::List(start)) => {
                let nested_in_item = matches!(
                    self.frames.last(),
                    Some(Frame::ListItem { .. })
                );
                self.frames.push(Frame::List {
                    ordered: start.is_some(),
                    range,
                    items: Vec::new(),
                    nested_in_item,
                });
            }
            Event::Start(Tag::Item) => {
                self.frames.push(Frame::ListItem {
                    inlines: InlineBuf::new(),
                });
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                let lang = match kind {
                    CodeBlockKind::Indented => None,
                    CodeBlockKind::Fenced(info) => {
                        let trimmed = info.trim();
                        if trimmed.is_empty() {
                            None
                        } else {
                            // Take only the first whitespace-delimited token,
                            // matching how editors render the info string.
                            Some(trimmed.split_whitespace().next().unwrap_or(trimmed).to_string())
                        }
                    }
                };
                self.frames.push(Frame::Code {
                    lang,
                    range,
                    code: String::new(),
                });
            }
            Event::Start(Tag::Table(aligns)) => {
                self.frames.push(Frame::Table {
                    range,
                    headers: Vec::new(),
                    rows: Vec::new(),
                    in_head: false,
                    current_row: Vec::new(),
                    current_cell: String::new(),
                    cols: aligns.len(),
                    malformed: None,
                });
            }
            Event::Start(Tag::TableHead) => {
                if let Some(Frame::Table { in_head, .. }) = self.frames.last_mut() {
                    *in_head = true;
                }
            }
            Event::Start(Tag::TableRow) => {
                if let Some(Frame::Table { current_row, .. }) = self.frames.last_mut() {
                    current_row.clear();
                }
            }
            Event::Start(Tag::TableCell) => {
                if let Some(Frame::Table { current_cell, .. }) = self.frames.last_mut() {
                    current_cell.clear();
                }
            }
            Event::Start(Tag::Strong) => {
                self.flag_non_image_in_paragraph();
                self.with_current_inlines(|buf| buf.open_strong());
            }
            Event::Start(Tag::Emphasis) => {
                self.flag_non_image_in_paragraph();
                self.with_current_inlines(|buf| buf.open_emph());
            }
            Event::Start(Tag::Link { dest_url, .. }) => {
                self.flag_non_image_in_paragraph();
                let href = dest_url.into_string();
                self.with_current_inlines(|buf| buf.open_link(href));
            }
            Event::Start(Tag::Image { dest_url, .. }) => {
                // If we're inside a paragraph, this image becomes a
                // candidate for block-level lifting. Record its src and
                // start accumulating the alt text from the upcoming Text
                // events.
                if let Some(Frame::Paragraph {
                    image_depth,
                    image_count,
                    image_src,
                    image_alt,
                    ..
                }) = self.frames.last_mut()
                {
                    *image_depth += 1;
                    if *image_count == 0 {
                        *image_src = Some(dest_url.into_string());
                        image_alt.clear();
                    }
                    *image_count += 1;
                }
                // Outside a paragraph (e.g. inside a list item, heading,
                // table cell): inline images are dropped silently per §3.4.
            }

            // ---- Container ends -------------------------------------------------
            Event::End(TagEnd::Heading(_level)) => {
                // The Tag::Heading frame is the source of truth for the
                // level — `_level` from TagEnd is identical for well-formed
                // input. We trust the frame.
                if let Some(Frame::Heading { level: level_to_use, range, inlines }) = self.frames.pop() {
                    let (_inline_vec, text) = inlines.finish();
                    let text = text.trim().to_string();

                    // Update heading stack: clear deeper levels, set this level.
                    if (1..=6).contains(&level_to_use) {
                        let idx = (level_to_use - 1) as usize;
                        for slot in &mut self.heading_stack[idx + 1..] {
                            *slot = None;
                        }
                        self.heading_stack[idx] = Some(text.clone());
                    }

                    // The heading_path on the heading block ITSELF excludes
                    // the heading's own text (it's the path of ancestors).
                    // Empty heading texts are skipped so they don't create
                    // a `""` segment in the path (matches `heading_path()`).
                    let path = self
                        .heading_stack
                        .iter()
                        .take((level_to_use - 1) as usize)
                        .filter_map(|s| s.clone().filter(|t| !t.is_empty()))
                        .collect();

                    let block = ParsedBlock {
                        kind: ParsedBlockKind::Heading,
                        heading_path: path,
                        source_span: self.span_for(&range),
                        payload: ParsedPayload::Heading { level: level_to_use, text },
                    };
                    self.emit_block(block);
                }
            }
            Event::End(TagEnd::Paragraph) => {
                if matches!(self.frames.last(), Some(Frame::Paragraph { .. })) {
                    if let Some(Frame::Paragraph {
                        range,
                        inlines,
                        image_count,
                        non_image_text_seen,
                        image_src,
                        image_alt,
                        ..
                    }) = self.frames.pop()
                    {
                        // Block-level image lift: paragraph whose only
                        // content is exactly one `![alt](src)`. Source
                        // (with optional title), alt, and angle-bracket
                        // wrapping are all captured by pulldown-cmark from
                        // the `Tag::Image` event itself, so the title is
                        // dropped and angle brackets are stripped without
                        // any byte-level scanning.
                        if image_count == 1 && !non_image_text_seen {
                            let span = self.span_for(&range);
                            let block = ParsedBlock {
                                kind: ParsedBlockKind::ImageRef,
                                heading_path: self.heading_path(),
                                source_span: span,
                                payload: ParsedPayload::ImageRef {
                                    src: image_src.unwrap_or_default(),
                                    alt: image_alt,
                                },
                            };
                            self.emit_block(block);
                            return;
                        }
                        let (inline_vec, text) = inlines.finish();
                        let span = self.span_for(&range);
                        let block = ParsedBlock {
                            kind: ParsedBlockKind::Paragraph,
                            heading_path: self.heading_path(),
                            source_span: span,
                            payload: ParsedPayload::Paragraph { text, inlines: inline_vec },
                        };
                        self.emit_block(block);
                    }
                }
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                if let Some(Frame::Quote { range, children }) = self.frames.pop() {
                    // Concatenate child text for the Quote payload.
                    let mut text = String::new();
                    let mut inlines: Vec<Inline> = Vec::new();
                    for c in &children {
                        match &c.payload {
                            ParsedPayload::Paragraph { text: t, inlines: il }
                            | ParsedPayload::Quote { text: t, inlines: il } => {
                                if !text.is_empty() {
                                    text.push('\n');
                                }
                                text.push_str(t);
                                inlines.extend(il.iter().cloned());
                            }
                            ParsedPayload::Heading { text: t, .. } => {
                                if !text.is_empty() {
                                    text.push('\n');
                                }
                                text.push_str(t);
                                inlines.push(Inline::Text(t.clone()));
                            }
                            _ => {}
                        }
                    }
                    let block = ParsedBlock {
                        kind: ParsedBlockKind::Quote,
                        heading_path: self.heading_path(),
                        source_span: self.span_for(&range),
                        payload: ParsedPayload::Quote { text, inlines },
                    };
                    self.emit_block(block);
                }
            }
            Event::End(TagEnd::List(_)) => {
                if let Some(Frame::List { ordered, range, items, nested_in_item }) = self.frames.pop() {
                    if nested_in_item {
                        // Flatten this sub-list into the enclosing list
                        // item's inline text. Each item becomes a line
                        // prefixed with "  - " (or "  1. " for ordered) so
                        // structure is recognizable downstream.
                        let mut rendered = String::new();
                        for (i, item) in items.iter().enumerate() {
                            rendered.push('\n');
                            rendered.push_str("  ");
                            if ordered {
                                rendered.push_str(&format!("{}. ", i + 1));
                            } else {
                                rendered.push_str("- ");
                            }
                            rendered.push_str(&flatten_inlines_to_text(item));
                        }
                        if let Some(Frame::ListItem { inlines }) = self.frames.last_mut() {
                            inlines.push_text(&rendered);
                        }
                    } else {
                        let block = ParsedBlock {
                            kind: ParsedBlockKind::List,
                            heading_path: self.heading_path(),
                            source_span: self.span_for(&range),
                            payload: ParsedPayload::List { ordered, items },
                        };
                        self.emit_block(block);
                    }
                }
            }
            Event::End(TagEnd::Item) => {
                if let Some(Frame::ListItem { inlines }) = self.frames.pop() {
                    let (inline_vec, _text) = inlines.finish();
                    if let Some(Frame::List { items, .. }) = self.frames.last_mut() {
                        items.push(inline_vec);
                    }
                }
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some(Frame::Code { lang, range, mut code }) = self.frames.pop() {
                    // Fenced code blocks include a trailing newline from the
                    // last source line; pulldown-cmark already strips the
                    // closing fence. We trim a single trailing `\n` for
                    // clean round-trips with hand-authored fixtures.
                    if code.ends_with('\n') {
                        code.pop();
                    }
                    let block = ParsedBlock {
                        kind: ParsedBlockKind::Code,
                        heading_path: self.heading_path(),
                        source_span: self.span_for(&range),
                        payload: ParsedPayload::Code { lang, code },
                    };
                    self.emit_block(block);
                }
            }
            Event::End(TagEnd::Table) => {
                if let Some(Frame::Table {
                    range,
                    headers,
                    rows,
                    malformed,
                    ..
                }) = self.frames.pop()
                {
                    let block = if let Some(note) = malformed {
                        // Fall back to a paragraph carrying the raw markdown.
                        warnings.push(Warning {
                            kind: WarningKind::MalformedTable,
                            note,
                        });
                        let raw = std::str::from_utf8(
                            self.body.get(range.clone()).unwrap_or(&[]),
                        )
                        .unwrap_or_default()
                        .to_string();
                        ParsedBlock {
                            kind: ParsedBlockKind::Paragraph,
                            heading_path: self.heading_path(),
                            source_span: self.span_for(&range),
                            payload: ParsedPayload::Paragraph {
                                text: raw.clone(),
                                inlines: vec![Inline::Text(raw)],
                            },
                        }
                    } else {
                        ParsedBlock {
                            kind: ParsedBlockKind::Table,
                            heading_path: self.heading_path(),
                            source_span: self.span_for(&range),
                            payload: ParsedPayload::Table { headers, rows },
                        }
                    };
                    self.emit_block(block);
                }
            }
            Event::End(TagEnd::TableHead) => {
                if let Some(Frame::Table {
                    in_head,
                    headers,
                    current_row,
                    cols,
                    malformed,
                    ..
                }) = self.frames.last_mut()
                {
                    *in_head = false;
                    *headers = std::mem::take(current_row);
                    if *cols == 0 {
                        *cols = headers.len();
                    } else if headers.len() != *cols && malformed.is_none() {
                        *malformed = Some(format!(
                            "header has {} cells, table reported {} columns",
                            headers.len(),
                            cols
                        ));
                    }
                }
            }
            Event::End(TagEnd::TableRow) => {
                if let Some(Frame::Table {
                    rows,
                    current_row,
                    cols,
                    malformed,
                    ..
                }) = self.frames.last_mut()
                {
                    let row = std::mem::take(current_row);
                    if row.len() != *cols && malformed.is_none() {
                        *malformed = Some(format!(
                            "row {} has {} cells, headers have {}",
                            rows.len() + 1,
                            row.len(),
                            cols
                        ));
                    }
                    rows.push(row);
                }
            }
            Event::End(TagEnd::TableCell) => {
                if let Some(Frame::Table {
                    current_row,
                    current_cell,
                    ..
                }) = self.frames.last_mut()
                {
                    current_row.push(std::mem::take(current_cell));
                }
            }
            Event::End(TagEnd::Strong) => {
                self.with_current_inlines(|buf| buf.close_strong());
            }
            Event::End(TagEnd::Emphasis) => {
                self.with_current_inlines(|buf| buf.close_emph());
            }
            Event::End(TagEnd::Link) => {
                self.with_current_inlines(|buf| buf.close_link());
            }
            Event::End(TagEnd::Image) => {
                if let Some(Frame::Paragraph { image_depth, .. }) = self.frames.last_mut() {
                    if *image_depth > 0 {
                        *image_depth -= 1;
                    }
                }
            }

            // ---- Leaf events -----------------------------------------------------
            Event::Text(s) => {
                // Code blocks accumulate into the code string instead of
                // the inline buffer.
                if let Some(Frame::Code { code, .. }) = self.frames.last_mut() {
                    code.push_str(&s);
                    return;
                }
                if let Some(Frame::Table {
                    in_head,
                    current_cell,
                    ..
                }) = self.frames.last_mut()
                {
                    let _ = in_head;
                    current_cell.push_str(&s);
                    return;
                }
                // If this text is inside a `Tag::Image` opened inside a
                // paragraph, route it to the image's alt accumulator and
                // suppress it from the inline buffer (so a paragraph that
                // is *only* an image doesn't carry the alt as visible
                // inline text in the fallback case either).
                if let Some(Frame::Paragraph {
                    image_depth,
                    image_alt,
                    ..
                }) = self.frames.last_mut()
                {
                    if *image_depth > 0 {
                        image_alt.push_str(&s);
                        return;
                    }
                }
                // Otherwise: visible non-image content.
                if let Some(Frame::Paragraph {
                    non_image_text_seen,
                    ..
                }) = self.frames.last_mut()
                {
                    if !s.is_empty() {
                        *non_image_text_seen = true;
                    }
                }
                let owned = s.into_string();
                self.with_current_inlines(|buf| {
                    buf.push_text(&owned);
                    buf.push_link_text(&owned);
                });
            }
            Event::Code(s) => {
                if let Some(Frame::Table { current_cell, .. }) = self.frames.last_mut() {
                    current_cell.push_str(&s);
                    return;
                }
                if let Some(Frame::Paragraph {
                    non_image_text_seen,
                    image_depth,
                    ..
                }) = self.frames.last_mut()
                {
                    // Code inside an image's alt — extremely rare but pin
                    // behavior: count as visible non-image content so the
                    // paragraph isn't lifted to ImageRef.
                    if *image_depth == 0 {
                        *non_image_text_seen = true;
                    }
                }
                let owned = s.into_string();
                self.with_current_inlines(|buf| {
                    buf.push_code(&owned);
                    buf.push_link_text(&owned);
                });
            }
            Event::SoftBreak | Event::HardBreak => {
                if let Some(Frame::Code { code, .. }) = self.frames.last_mut() {
                    code.push('\n');
                    return;
                }
                if let Some(Frame::Table { current_cell, .. }) = self.frames.last_mut() {
                    current_cell.push(' ');
                    return;
                }
                // Update both `paragraph.text` (via push_text) and the
                // open link's flattened text accumulator (via
                // push_link_text). Without push_link_text here, a
                // multi-line `[text\nmore](href)` collapses to "textmore"
                // — losing the visible space between words.
                self.with_current_inlines(|buf| {
                    buf.push_text(" ");
                    buf.push_link_text(" ");
                });
            }
            // Everything else (HTML, footnote refs, task list markers, math,
            // rules, etc.) is dropped silently per design §3.4.
            _ => {}
        }
    }

    /// If the top frame is an open paragraph that hasn't yet escaped the
    /// "single image only" signature, mark it as containing visible
    /// non-image content so it won't be lifted to ImageRef at End.
    fn flag_non_image_in_paragraph(&mut self) {
        if let Some(Frame::Paragraph {
            non_image_text_seen,
            image_depth,
            ..
        }) = self.frames.last_mut()
        {
            if *image_depth == 0 {
                *non_image_text_seen = true;
            }
        }
    }

    /// Run `f` on whichever inline accumulator is open at the top of the
    /// frame stack. No-op if no inline-accepting frame is open.
    fn with_current_inlines<F: FnOnce(&mut InlineBuf)>(&mut self, f: F) {
        for frame in self.frames.iter_mut().rev() {
            match frame {
                Frame::Paragraph { inlines, .. }
                | Frame::Heading { inlines, .. }
                | Frame::ListItem { inlines } => {
                    f(inlines);
                    return;
                }
                Frame::Quote { .. } | Frame::List { .. } => continue,
                Frame::Code { .. } | Frame::Table { .. } => return,
            }
        }
    }
}

fn heading_level_to_u8(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(body: &str, offset: u32) -> (Vec<ParsedBlock>, Vec<Warning>) {
        parse_blocks(body.as_bytes(), offset).unwrap()
    }

    // ---- heading tree depth + heading_path correctness ----------------------

    #[test]
    fn heading_path_is_ancestors_only() {
        let body = "# H1 text\n\n## H2 text\n\nbody under h2\n";
        let (blocks, warns) = parse(body, 1);
        assert!(warns.is_empty(), "warns: {warns:?}");
        // Expect: H1 heading, H2 heading, paragraph.
        assert_eq!(blocks.len(), 3);
        // H1 has no ancestor.
        assert_eq!(blocks[0].heading_path, Vec::<String>::new());
        match &blocks[0].payload {
            ParsedPayload::Heading { level, text } => {
                assert_eq!(*level, 1);
                assert_eq!(text, "H1 text");
            }
            _ => panic!("expected heading"),
        }
        // H2 nests under H1.
        assert_eq!(blocks[1].heading_path, vec!["H1 text".to_string()]);
        // Paragraph nests under H1 → H2.
        assert_eq!(
            blocks[2].heading_path,
            vec!["H1 text".to_string(), "H2 text".to_string()]
        );
    }

    #[test]
    fn h1_resets_deeper_path() {
        let body = "# A\n\n## A.1\n\np1\n\n# B\n\np2\n";
        let (blocks, _) = parse(body, 1);
        // p1 under [A, A.1]; p2 under [B].
        let p1 = blocks.iter().find(|b| matches!(b.payload, ParsedPayload::Paragraph { ref text, .. } if text == "p1")).unwrap();
        let p2 = blocks.iter().find(|b| matches!(b.payload, ParsedPayload::Paragraph { ref text, .. } if text == "p2")).unwrap();
        assert_eq!(p1.heading_path, vec!["A".to_string(), "A.1".to_string()]);
        assert_eq!(p2.heading_path, vec!["B".to_string()]);
    }

    // ---- empty heading edge cases -------------------------------------------

    #[test]
    fn empty_heading_does_not_pollute_path() {
        // `# ` with no text used to seed the heading_stack with `Some("")`,
        // which then leaked into every child block's `heading_path` as a
        // `""` segment. Now empty entries are filtered from the path.
        let body = "# \n# Real H1\n## Sub\nbody\n";
        let (blocks, _) = parse(body, 1);
        // body is the last block; verify its heading_path.
        let para = blocks
            .iter()
            .find(|b| matches!(b.payload, ParsedPayload::Paragraph { .. }))
            .expect("paragraph present");
        assert_eq!(
            para.heading_path,
            vec!["Real H1".to_string(), "Sub".to_string()],
            "empty heading should not appear in path; got {:?}",
            para.heading_path
        );
    }

    #[test]
    fn empty_h1_then_h2_does_not_break_stack() {
        // An empty H1 overwrites the H1 slot with `Some("")` (so a later
        // H2 is still treated as positioned at level 2), but the path
        // filter drops the empty entry. So Inner's path is `[]` not `[""]`,
        // and the body's path is `["Inner"]` — neither carries a `""` and
        // the parser doesn't panic or skip blocks.
        let body = "# Outer\n\n# \n\n## Inner\nbody\n";
        let (blocks, _) = parse(body, 1);
        let inner = blocks
            .iter()
            .find(|b| {
                matches!(b.payload, ParsedPayload::Heading { ref text, .. } if text == "Inner")
            })
            .expect("Inner heading present");
        assert_eq!(
            inner.heading_path,
            Vec::<String>::new(),
            "empty H1 leaves an empty slot, filtered from the path"
        );
        let para = blocks
            .iter()
            .find(|b| matches!(b.payload, ParsedPayload::Paragraph { .. }))
            .expect("paragraph present");
        assert_eq!(
            para.heading_path,
            vec!["Inner".to_string()],
            "body's path drops the empty H1 between root and Inner"
        );
    }

    // ---- code blocks ---------------------------------------------------------

    #[test]
    fn code_block_lang_preserved() {
        let body = "```rust\nfn main(){}\n```\n";
        let (blocks, _) = parse(body, 1);
        assert_eq!(blocks.len(), 1);
        match &blocks[0].payload {
            ParsedPayload::Code { lang, code } => {
                assert_eq!(lang.as_deref(), Some("rust"));
                assert_eq!(code, "fn main(){}");
            }
            _ => panic!("expected code"),
        }
    }

    #[test]
    fn code_block_no_lang_is_none() {
        let body = "```\nfoo\n```\n";
        let (blocks, _) = parse(body, 1);
        match &blocks[0].payload {
            ParsedPayload::Code { lang, .. } => assert_eq!(lang, &None),
            _ => panic!("expected code"),
        }
    }

    // ---- tables --------------------------------------------------------------

    #[test]
    fn gfm_table_parses_headers_and_rows() {
        let body = "| a | b | c |\n|---|---|---|\n| 1 | 2 | 3 |\n| x | y | z |\n";
        let (blocks, warns) = parse(body, 1);
        assert!(warns.is_empty(), "warns: {warns:?}");
        assert_eq!(blocks.len(), 1);
        match &blocks[0].payload {
            ParsedPayload::Table { headers, rows } => {
                assert_eq!(headers, &vec!["a".to_string(), "b".to_string(), "c".to_string()]);
                assert_eq!(rows.len(), 2);
                assert_eq!(rows[0], vec!["1".to_string(), "2".to_string(), "3".to_string()]);
                assert_eq!(rows[1], vec!["x".to_string(), "y".to_string(), "z".to_string()]);
            }
            _ => panic!("expected table, got {:?}", blocks[0].payload),
        }
    }

    /// Drive the malformed-table fallback path directly by feeding a
    /// hand-built event stream through `WalkState::handle_event`. This
    /// pins the contract: when the table frame's `malformed` field is set
    /// at end-of-table, the block degrades to a paragraph and a
    /// `MalformedTable` warning is emitted.
    ///
    /// Doing it via events (instead of a real markdown source) is
    /// necessary because `pulldown-cmark` auto-normalizes GFM tables —
    /// short rows are padded, long rows are truncated, so a real markdown
    /// input never reaches the malformed branch.
    #[test]
    fn malformed_table_falls_back_to_paragraph_with_warning() {
        use pulldown_cmark::{Alignment, CowStr};

        let body = b"| a | b |\n|---|---|\n| 1 |\n";
        let line_index = LineIndex::new(body);
        let mut state = WalkState::new(1, &line_index, body);
        let mut warnings = Vec::new();

        // Synthetic events — fake a 3-column table with a 2-cell header so
        // the `malformed` branch fires.
        let aligns = vec![Alignment::None, Alignment::None, Alignment::None];
        state.handle_event(Event::Start(Tag::Table(aligns)), 0..body.len(), &mut warnings);
        state.handle_event(Event::Start(Tag::TableHead), 0..0, &mut warnings);
        state.handle_event(Event::Start(Tag::TableCell), 0..0, &mut warnings);
        state.handle_event(Event::Text(CowStr::Borrowed("a")), 0..0, &mut warnings);
        state.handle_event(Event::End(TagEnd::TableCell), 0..0, &mut warnings);
        state.handle_event(Event::Start(Tag::TableCell), 0..0, &mut warnings);
        state.handle_event(Event::Text(CowStr::Borrowed("b")), 0..0, &mut warnings);
        state.handle_event(Event::End(TagEnd::TableCell), 0..0, &mut warnings);
        // Two-cell header with cols=3 → triggers malformed.
        state.handle_event(Event::End(TagEnd::TableHead), 0..0, &mut warnings);
        state.handle_event(Event::End(TagEnd::Table), 0..body.len(), &mut warnings);

        let blocks = state.finish();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].kind, ParsedBlockKind::Paragraph);
        match &blocks[0].payload {
            ParsedPayload::Paragraph { text, .. } => {
                assert!(text.contains("| a | b |"), "raw markdown preserved: {text:?}");
            }
            _ => panic!("expected paragraph fallback"),
        }
        assert_eq!(warnings.len(), 1);
        assert!(matches!(warnings[0].kind, WarningKind::MalformedTable));
        assert!(warnings[0].note.contains("header"));
    }

    // ---- line ranges (LF) ----------------------------------------------------

    #[test]
    fn line_range_lf_paragraph_at_body_line_5() {
        // body lines: 1=blank, 2=blank, 3=blank, 4=blank, 5=paragraph
        let body = "\n\n\n\nhello world\n";
        // body_offset_lines=6 → body line 0 (zero-indexed) → file line 6.
        // The paragraph is at body 0-indexed line 4 → file line 10.
        let (blocks, _) = parse(body, 6);
        assert_eq!(blocks.len(), 1);
        match blocks[0].source_span {
            SourceSpan::Line { start, end } => {
                assert_eq!(start, 10, "paragraph should start at file line 10");
                assert_eq!(end, 10);
            }
            _ => panic!("expected line span"),
        }
    }

    #[test]
    fn line_range_lf_multi_line_paragraph() {
        // body line 0 = "a", 1 = "b", 2 = "c"; one paragraph, lines 0..=2.
        let body = "a\nb\nc\n";
        let (blocks, _) = parse(body, 1);
        assert_eq!(blocks.len(), 1);
        match blocks[0].source_span {
            SourceSpan::Line { start, end } => {
                assert_eq!(start, 1);
                assert_eq!(end, 3);
            }
            _ => panic!(),
        }
    }

    // ---- line ranges (CRLF) --------------------------------------------------

    #[test]
    fn line_range_crlf_matches_lf() {
        let body_lf = "\n\n\n\nhello\n";
        let body_crlf = "\r\n\r\n\r\n\r\nhello\r\n";
        let (lf_blocks, _) = parse(body_lf, 6);
        let (crlf_blocks, _) = parse(body_crlf, 6);
        assert_eq!(lf_blocks.len(), 1);
        assert_eq!(crlf_blocks.len(), 1);
        assert_eq!(lf_blocks[0].source_span, crlf_blocks[0].source_span);
    }

    // ---- image ref -----------------------------------------------------------

    #[test]
    fn image_ref_block_captures_src_and_alt() {
        let body = "![hello](pic.png)\n";
        let (blocks, _) = parse(body, 1);
        assert_eq!(blocks.len(), 1);
        match &blocks[0].payload {
            ParsedPayload::ImageRef { src, alt } => {
                assert_eq!(alt, "hello");
                assert_eq!(src, "pic.png");
            }
            _ => panic!("expected image ref, got {:?}", blocks[0].payload),
        }
        assert_eq!(blocks[0].kind, ParsedBlockKind::ImageRef);
    }

    #[test]
    fn image_with_title_attribute() {
        // Source includes a title, but pulldown-cmark exposes it
        // separately on `Tag::Image`; we ignore the title — only `src`
        // and `alt` survive. Previously the byte-scanner pulled
        // `src "title"` into `src`.
        let body = "![alt](src.png \"title\")\n";
        let (blocks, _) = parse(body, 1);
        assert_eq!(blocks.len(), 1);
        match &blocks[0].payload {
            ParsedPayload::ImageRef { src, alt } => {
                assert_eq!(src, "src.png");
                assert_eq!(alt, "alt");
            }
            _ => panic!("expected image ref, got {:?}", blocks[0].payload),
        }
    }

    #[test]
    fn image_with_angle_bracketed_url() {
        // `<…>` wrapping is a CommonMark feature for URLs containing
        // spaces. pulldown-cmark strips the angle brackets and decodes
        // the URL; we should reflect that.
        let body = "![alt](<https://x.com/a b>)\n";
        let (blocks, _) = parse(body, 1);
        assert_eq!(blocks.len(), 1);
        match &blocks[0].payload {
            ParsedPayload::ImageRef { src, alt } => {
                assert_eq!(
                    src, "https://x.com/a b",
                    "angle brackets should be stripped"
                );
                assert_eq!(alt, "alt");
            }
            _ => panic!("expected image ref, got {:?}", blocks[0].payload),
        }
    }

    #[test]
    fn empty_image_alt_and_src() {
        // Pin behavior on the degenerate `![]()` shape. Both fields are
        // empty strings; the block is still classified as ImageRef.
        let body = "![]()\n";
        let (blocks, _) = parse(body, 1);
        assert_eq!(blocks.len(), 1);
        match &blocks[0].payload {
            ParsedPayload::ImageRef { src, alt } => {
                assert_eq!(src, "");
                assert_eq!(alt, "");
            }
            _ => panic!("expected image ref, got {:?}", blocks[0].payload),
        }
    }

    #[test]
    fn inline_image_inside_paragraph_is_dropped() {
        // The image is part of a longer paragraph → not a block-level image.
        let body = "see ![alt](pic.png) here\n";
        let (blocks, _) = parse(body, 1);
        assert_eq!(blocks.len(), 1);
        match &blocks[0].payload {
            ParsedPayload::Paragraph { inlines, .. } => {
                // Only Text/Code/Link/Strong/Emph allowed.
                for inl in inlines {
                    assert!(
                        matches!(
                            inl,
                            Inline::Text(_) | Inline::Code(_) | Inline::Link { .. } | Inline::Strong(_) | Inline::Emph(_)
                        ),
                        "unexpected inline kind: {:?}",
                        inl
                    );
                }
            }
            _ => panic!("expected paragraph, got {:?}", blocks[0].payload),
        }
    }

    // ---- nested lists --------------------------------------------------------

    #[test]
    fn nested_list_flattens_into_parent_item() {
        let body = "- a\n  - x\n  - y\n- b\n";
        let (blocks, _) = parse(body, 1);
        assert_eq!(blocks.len(), 1);
        match &blocks[0].payload {
            ParsedPayload::List { ordered, items } => {
                assert!(!ordered);
                assert_eq!(items.len(), 2, "two top-level items");
                // First item should contain "a" plus a flattened rendering
                // of the nested sub-list.
                let flat = flatten_inlines_to_text(&items[0]);
                assert!(flat.contains("a"), "first item missing 'a': {flat:?}");
                assert!(flat.contains("- x"), "first item missing '- x': {flat:?}");
                assert!(flat.contains("- y"), "first item missing '- y': {flat:?}");
                let flat2 = flatten_inlines_to_text(&items[1]);
                assert_eq!(flat2.trim(), "b");
            }
            _ => panic!("expected list, got {:?}", blocks[0].payload),
        }
    }

    // ---- block content inside list items ------------------------------------

    #[test]
    fn code_block_inside_list_item_flattens_into_parent() {
        // Without the routing fix, the code block would escape the list and
        // appear at top level (in wrong source order).
        let body = "- item\n\n  ```rust\n  fn f(){}\n  ```\n\n- next\n";
        let (blocks, _) = parse(body, 1);
        // Expect a single List block, no escaped Code at top level.
        assert_eq!(
            blocks.len(),
            1,
            "code should not escape to top level: got {} blocks",
            blocks.len()
        );
        match &blocks[0].payload {
            ParsedPayload::List { ordered, items } => {
                assert!(!ordered);
                assert_eq!(items.len(), 2);
                let flat = flatten_inlines_to_text(&items[0]);
                assert!(flat.contains("item"), "first item missing 'item': {flat:?}");
                assert!(flat.contains("fn f(){}"), "first item missing code body: {flat:?}");
                assert!(flat.contains("```"), "first item missing fence: {flat:?}");
                assert_eq!(flatten_inlines_to_text(&items[1]).trim(), "next");
            }
            _ => panic!("expected list, got {:?}", blocks[0].payload),
        }
    }

    #[test]
    fn image_inside_list_item_flattens_into_parent() {
        let body = "- ![alt](pic.png)\n";
        let (blocks, _) = parse(body, 1);
        // The image should NOT escape to top level as a standalone ImageRef.
        assert_eq!(blocks.len(), 1);
        match &blocks[0].payload {
            ParsedPayload::List { items, .. } => {
                assert_eq!(items.len(), 1);
                let flat = flatten_inlines_to_text(&items[0]);
                assert!(
                    flat.contains("![alt](pic.png)") || flat.contains("alt"),
                    "image should be rendered into item: {flat:?}"
                );
            }
            _ => panic!("expected list, got {:?}", blocks[0].payload),
        }
    }

    #[test]
    fn block_content_in_list_preserves_document_order() {
        // Two items with code between; the resulting blocks should be in
        // strictly source order: just one List, no top-level code block.
        let body = "- a\n\n  ```\n  c\n  ```\n\n- b\n";
        let (blocks, _) = parse(body, 1);
        assert_eq!(blocks.len(), 1, "expected single list block");
        match &blocks[0].kind {
            kb_parse_types::ParsedBlockKind::List => {}
            other => panic!("expected list, got {other:?}"),
        }
    }

    // ---- malformed input no-panic -------------------------------------------

    #[test]
    fn random_bytes_do_not_panic() {
        // Tiny xorshift PRNG seeded with a constant; deterministic inputs
        // covering both valid-ish and invalid markdown / utf-8 mixes.
        // We also vary `body_offset_lines` across the full u32 range —
        // including u32::MAX — to exercise the saturating-add path in
        // `span_for` and confirm the overflow guard prevents panics.
        let mut state: u32 = 0x1234_5678;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state
        };
        for i in 0..100 {
            let len = (next() as usize) % 256;
            let bytes: Vec<u8> = (0..len).map(|_| next() as u8).collect();
            // Mix a u32::MAX-style offset every few iterations so the fuzz
            // exercises the overflow path explicitly.
            let offset = match i % 4 {
                0 => 0,
                1 => 1,
                2 => next(),
                _ => u32::MAX - (next() % 16),
            };
            let _ = parse_blocks(&bytes, offset).expect("must be Ok");
        }
    }

    #[test]
    fn body_offset_lines_max_returns_extract_failed() {
        // u32::MAX + any positive line number overflows; the parser must
        // surface `ExtractFailed` and discard blocks rather than panic
        // (debug) or emit an inverted span (release).
        let body = b"# h\nbody\n";
        let (blocks, warns) = parse_blocks(body, u32::MAX).expect("must be Ok");
        assert!(blocks.is_empty(), "blocks should be discarded on overflow");
        assert_eq!(warns.len(), 1, "expected one warning, got: {warns:?}");
        assert_eq!(warns[0].kind, WarningKind::ExtractFailed);
        assert!(
            warns[0].note.contains("overflow"),
            "note should mention overflow: {:?}",
            warns[0].note
        );
    }

    #[test]
    fn body_offset_lines_zero_at_max_minus_one_no_overflow() {
        // A body with exactly one logical line + offset = u32::MAX would
        // still saturate. Pick offset such that no line ever reaches
        // overflow — this should succeed normally.
        let body = b"hi\n";
        let (blocks, warns) = parse_blocks(body, u32::MAX - 100).expect("must be Ok");
        assert_eq!(blocks.len(), 1);
        assert!(warns.is_empty(), "no overflow expected, got: {warns:?}");
    }

    #[test]
    fn adversarial_inputs_no_panic() {
        // Hand-crafted oddities.
        let cases: &[&[u8]] = &[
            b"",
            b"\0\0\0",
            b"```\nunclosed",
            b"# heading\n```\nfn main() {",
            b"| a | b |\n|---|---|\n| 1 |\n",   // short row
            b"| a | b |\n|---|\n| 1 | 2 |\n",   // header/sep mismatch
            b"![",
            b"](",
            b"---\nfm: yes\n",
            b"#######",                                                  // 7 hashes (invalid heading)
            b"\xff\xfe\x00\x00garbage",                                   // non-utf8
            "# 한글\n\n본문\n".as_bytes(),
        ];
        for c in cases {
            let _ = parse_blocks(c, 1).expect("must be Ok");
        }
    }

    // ---- inline filter -------------------------------------------------------

    #[test]
    fn link_with_soft_break_preserves_space_in_text() {
        // Without the push_link_text fix, this collapses to "multiline".
        let body = "[multi\nline](http://x)\n";
        let (blocks, _) = parse(body, 1);
        assert_eq!(blocks.len(), 1);
        match &blocks[0].payload {
            ParsedPayload::Paragraph { inlines, .. } => {
                let link = inlines
                    .iter()
                    .find(|i| matches!(i, Inline::Link { .. }))
                    .expect("link present");
                match link {
                    Inline::Link { text, href } => {
                        assert_eq!(text, "multi line");
                        assert_eq!(href, "http://x");
                    }
                    _ => unreachable!(),
                }
            }
            _ => panic!("expected paragraph, got {:?}", blocks[0].payload),
        }
    }

    #[test]
    fn link_with_hard_break_preserves_space_in_text() {
        // Two trailing spaces + newline = HardBreak in CommonMark.
        let body = "[multi  \nline](http://x)\n";
        let (blocks, _) = parse(body, 1);
        assert_eq!(blocks.len(), 1);
        match &blocks[0].payload {
            ParsedPayload::Paragraph { inlines, .. } => {
                let link = inlines
                    .iter()
                    .find(|i| matches!(i, Inline::Link { .. }))
                    .expect("link present");
                match link {
                    Inline::Link { text, .. } => {
                        assert_eq!(text, "multi line");
                    }
                    _ => unreachable!(),
                }
            }
            _ => panic!("expected paragraph"),
        }
    }

    #[test]
    fn only_allowed_inlines_emitted() {
        let body = "**bold** *em* `code` [link](u)\n";
        let (blocks, _) = parse(body, 1);
        match &blocks[0].payload {
            ParsedPayload::Paragraph { inlines, .. } => {
                let kinds: Vec<&'static str> = inlines.iter().map(|i| match i {
                    Inline::Text(_) => "Text",
                    Inline::Code(_) => "Code",
                    Inline::Link { .. } => "Link",
                    Inline::Strong(_) => "Strong",
                    Inline::Emph(_) => "Emph",
                }).collect();
                assert!(kinds.contains(&"Strong"));
                assert!(kinds.contains(&"Emph"));
                assert!(kinds.contains(&"Code"));
                assert!(kinds.contains(&"Link"));
            }
            _ => panic!(),
        }
    }
}
