# p9-fb-24 — TUI status/key bar + Library header + page scroll Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Split the bottom screen area into a 2-row footer (always-visible status bar + key hint bar), absorb the ingest progress row into the new status bar, add a column header above the Library list, and add `PgUp`/`PgDn` page scrolling to the Ask transcript with a single `PAGE_STEP` constant shared by Inspect.

**Architecture:** Single-crate change inside `kebab-tui`. The status bar is a new pure render function that pulls live state directly from `App` (pane label, doc count, version, dynamic-state cascade across `streaming` / `searching` / `indexing` / `idle`). The Library header reuses the existing `format_doc_row` column-width math via a new `format_doc_header(area_width)` function and is composed via a vertical `Layout` (header row + `List`). Page-scroll constants live in one module so future viewport-aware refinement has a single edit point.

**Tech Stack:** Rust 2024, ratatui 0.28, crossterm 0.28. No new deps.

**Spec:** `docs/superpowers/specs/2026-05-04-p9-fb-24-tui-affordances-design.md`

---

## File Structure

**Created:**
- `crates/kebab-tui/src/pager.rs` — `pub(crate) const PAGE_STEP: u16 = 10;` + future viewport-aware helper hook.
- `crates/kebab-tui/tests/status_bar.rs` — integration tests for `render_status_bar`.

**Modified:**
- `crates/kebab-tui/src/lib.rs` — add `mod pager;`.
- `crates/kebab-tui/src/run.rs` — drop the conditional 4-row layout, render `render_status_bar` + `render_key_hints` (renamed from `render_footer`) instead, delete the `render_ingest_status` private fn (absorbed).
- `crates/kebab-tui/src/library.rs` — new `format_doc_header(area_width: u16) -> Line<'static>` + `render_doc_list` splits its inner area into header row + List.
- `crates/kebab-tui/src/ask.rs` — `KeyCode::PageDown` / `PageUp` arms; both flip `follow_tail = false` and shift `s.scroll` by `pager::PAGE_STEP`.
- `crates/kebab-tui/src/inspect.rs` — replace the literal `10` in `PageDown` / `PageUp` with `pager::PAGE_STEP`.
- `crates/kebab-tui/src/cheatsheet.rs` — Ask section gains `PgUp / PgDn` row.
- `crates/kebab-tui/tests/library.rs` — extend the existing render test to assert the header row text is visible.
- `crates/kebab-tui/tests/ask.rs` — new tests for `PageUp` / `PageDown` (sets `scroll` by 10, flips `follow_tail` to false).

---

### Task 1: `pager` module — single source for `PAGE_STEP`

**Files:**
- Create: `crates/kebab-tui/src/pager.rs`
- Modify: `crates/kebab-tui/src/lib.rs`

- [ ] **Step 1: Create the new module file**

```rust
// crates/kebab-tui/src/pager.rs
//! p9-fb-24: page-step constant shared by Ask + Inspect PgUp/PgDn.
//!
//! Fixed `10` rows per page (independent of viewport height). The
//! design doc considered viewport-aware paging but deliberately
//! deferred it — Inspect already shipped with `+/-10`, so unifying
//! on the same constant is the smallest path that closes the
//! "Ask has no PgUp/PgDn" feedback. A future viewport-aware upgrade
//! lives behind this single edit point.

/// Rows scrolled per `PgUp` / `PgDn` keystroke.
pub(crate) const PAGE_STEP: u16 = 10;
```

- [ ] **Step 2: Wire the module into the crate**

Open `crates/kebab-tui/src/lib.rs`, find the existing `mod input;` line, add `mod pager;` directly after it (alphabetical). Do NOT re-export it — `pager::PAGE_STEP` stays `pub(crate)`.

- [ ] **Step 3: Verify the build**

Run: `cargo build -p kebab-tui`
Expected: `Finished dev profile`. No warnings.

- [ ] **Step 4: Commit**

```bash
git add crates/kebab-tui/src/pager.rs crates/kebab-tui/src/lib.rs
git commit -m "feat(kebab-tui): p9-fb-24 task 1 — pager module + PAGE_STEP constant"
```

---

### Task 2: Refactor Inspect to use `PAGE_STEP`

**Files:**
- Modify: `crates/kebab-tui/src/inspect.rs:428-435`

- [ ] **Step 1: Pin existing Inspect PgUp/PgDn behaviour with a regression test**

Open `crates/kebab-tui/tests/inspect.rs`. Find the existing `#[test]` functions (look for `j_scrolls_down` or similar). Append:

```rust
/// p9-fb-24 task 2: PageDown advances scroll by `PAGE_STEP` (= 10).
/// Pins the constant so a future viewport-aware refactor surfaces
/// here, not silently in user-visible behaviour.
#[test]
fn page_down_scrolls_by_ten_in_inspect() {
    let mut app = fresh_app_with_inspect();
    let outcome = handle_key_inspect(
        &mut app,
        KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE),
    );
    assert_eq!(outcome, KeyOutcome::Continue);
    assert_eq!(app.inspect.as_ref().unwrap().scroll, 10);
}

/// p9-fb-24 task 2: PageUp rewinds scroll by `PAGE_STEP`, saturating
/// at 0 (no underflow).
#[test]
fn page_up_rewinds_by_ten_saturating_in_inspect() {
    let mut app = fresh_app_with_inspect();
    app.inspect.as_mut().unwrap().scroll = 25;
    handle_key_inspect(
        &mut app,
        KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE),
    );
    assert_eq!(app.inspect.as_ref().unwrap().scroll, 15);
    // Walk past zero — saturating, not panicking.
    app.inspect.as_mut().unwrap().scroll = 3;
    handle_key_inspect(
        &mut app,
        KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE),
    );
    assert_eq!(app.inspect.as_ref().unwrap().scroll, 0);
}
```

If `fresh_app_with_inspect` does not exist, scan the top of `tests/inspect.rs` for the existing helper (likely named `fresh_app` or similar) and use that name. The pattern follows the existing tests — copy whatever helper they use.

- [ ] **Step 2: Run the new tests against current code (literal `10`)**

Run: `cargo test -p kebab-tui --test inspect page_`
Expected: PASS. The current implementation already uses `10`; the tests pin the value so the constant swap is a no-op semantically.

- [ ] **Step 3: Replace literals with `PAGE_STEP`**

Open `crates/kebab-tui/src/inspect.rs`. Find the lines with `s.scroll.saturating_add(10)` and `s.scroll.saturating_sub(10)` (PageDown / PageUp arms, around lines 428-435). Replace:

```rust
        (KeyCode::PageDown, _) => {
            let s = state.inspect.as_mut().unwrap();
            s.scroll = s.scroll.saturating_add(crate::pager::PAGE_STEP);
            KeyOutcome::Continue
        }
        (KeyCode::PageUp, _) => {
            let s = state.inspect.as_mut().unwrap();
            s.scroll = s.scroll.saturating_sub(crate::pager::PAGE_STEP);
            KeyOutcome::Continue
        }
```

(If the surrounding code structure differs — e.g. `s` is bound earlier — keep the same overall control flow and only swap the literal `10` for `crate::pager::PAGE_STEP`.)

- [ ] **Step 4: Run the regression tests against the refactored code**

Run: `cargo test -p kebab-tui --test inspect page_`
Expected: PASS. Same behaviour, now via the constant.

- [ ] **Step 5: Commit**

```bash
git add crates/kebab-tui/src/inspect.rs crates/kebab-tui/tests/inspect.rs
git commit -m "refactor(kebab-tui): p9-fb-24 task 2 — Inspect PgUp/PgDn via pager::PAGE_STEP"
```

---

### Task 3: Add Ask PgUp/PgDn

**Files:**
- Modify: `crates/kebab-tui/src/ask.rs` (key dispatch in `handle_key_ask`)
- Modify: `crates/kebab-tui/tests/ask.rs` (new integration tests)

- [ ] **Step 1: Write the failing tests**

Open `crates/kebab-tui/tests/ask.rs`. Append (after the existing `follow_tail_renders_tail_when_transcript_overflows` test):

```rust
/// p9-fb-24: PgDn advances Ask scroll by `PAGE_STEP` (= 10) and
/// disengages follow-tail (matches `j` semantics — manual scroll =
/// freeze).
#[test]
fn page_down_advances_scroll_and_freezes_follow_tail_in_ask() {
    let mut app = fresh_app();
    app.mode = kebab_tui::Mode::Normal;
    let outcome = handle_key_ask(
        &mut app,
        KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE),
    );
    assert_eq!(outcome, KeyOutcome::Continue);
    let s = app.ask.as_ref().unwrap();
    assert_eq!(s.scroll, 10, "PgDn shifts scroll by PAGE_STEP");
    assert!(!s.follow_tail, "PgDn freezes follow_tail like j/k");
}

/// p9-fb-24: PgUp rewinds Ask scroll by `PAGE_STEP` (saturating at 0)
/// and disengages follow-tail.
#[test]
fn page_up_rewinds_scroll_saturating_and_freezes_follow_tail_in_ask() {
    let mut app = fresh_app();
    app.mode = kebab_tui::Mode::Normal;
    app.ask.as_mut().unwrap().scroll = 25;
    app.ask.as_mut().unwrap().follow_tail = true;
    handle_key_ask(
        &mut app,
        KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE),
    );
    let s = app.ask.as_ref().unwrap();
    assert_eq!(s.scroll, 15);
    assert!(!s.follow_tail);
    // Walk past zero — saturating, not panicking.
    app.ask.as_mut().unwrap().scroll = 3;
    handle_key_ask(
        &mut app,
        KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE),
    );
    assert_eq!(app.ask.as_ref().unwrap().scroll, 0);
}

/// p9-fb-24: PgUp / PgDn fire from BOTH Insert and Normal modes
/// (physical keys, no typing ambiguity — same as Left/Right/Home/End
/// from p9-fb-22).
#[test]
fn page_keys_fire_from_insert_mode_in_ask() {
    let mut app = fresh_app();
    app.mode = kebab_tui::Mode::Insert;
    handle_key_ask(
        &mut app,
        KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE),
    );
    assert_eq!(app.ask.as_ref().unwrap().scroll, 10);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p kebab-tui --test ask page_`
Expected: FAIL — `scroll` stays `0` because `KeyCode::PageDown` / `PageUp` are not yet matched (fall through to the `_ => KeyOutcome::Continue` arm).

- [ ] **Step 3: Add the PageDown / PageUp arms to `handle_key_ask`**

Open `crates/kebab-tui/src/ask.rs`. Find the existing `KeyCode::Delete` arm (around line 421, added in p9-fb-22). Insert the new arms directly after it, before the `KeyCode::Char(c)` Insert-typing arm:

```rust
        // p9-fb-24: PgUp / PgDn page-scroll the transcript by
        // `pager::PAGE_STEP` rows. Mode-agnostic (physical keys, no
        // typing ambiguity). Both flip `follow_tail` to false so the
        // user pinning the view via paging doesn't get yanked back to
        // the bottom on the next streamed token (same contract as
        // `j` / `k` from p9-fb-22).
        (KeyCode::PageDown, _) => {
            let s = state.ask.as_mut().unwrap();
            s.follow_tail = false;
            s.scroll = s.scroll.saturating_add(crate::pager::PAGE_STEP);
            KeyOutcome::Continue
        }
        (KeyCode::PageUp, _) => {
            let s = state.ask.as_mut().unwrap();
            s.follow_tail = false;
            s.scroll = s.scroll.saturating_sub(crate::pager::PAGE_STEP);
            KeyOutcome::Continue
        }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p kebab-tui --test ask page_`
Expected: PASS (3/3).

- [ ] **Step 5: Run the full kebab-tui suite to confirm no regressions**

Run: `cargo test -p kebab-tui`
Expected: All tests pass. p9-fb-22's `follow_tail` / `j` / `k` / `Shift-G` tests must still pass — the new arms do not touch them.

- [ ] **Step 6: Commit**

```bash
git add crates/kebab-tui/src/ask.rs crates/kebab-tui/tests/ask.rs
git commit -m "feat(kebab-tui): p9-fb-24 task 3 — Ask PgUp/PgDn page scroll"
```

---

### Task 4: Library column header — `format_doc_header` function

**Files:**
- Modify: `crates/kebab-tui/src/library.rs` (new `format_doc_header` + unit test)

- [ ] **Step 1: Write the failing test**

Open `crates/kebab-tui/src/library.rs`. Find the existing `#[cfg(test)] mod tests` block (around line 470+, contains `format_doc_row_pads_by_display_width_for_hangul_title`). Append a new test inside the same module:

```rust
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
        // Header text contains every column label.
        assert!(header_text.contains("TITLE"), "header has TITLE label");
        assert!(header_text.contains("TAGS"), "header has TAGS label");
        assert!(header_text.contains("UPDATED"), "header has UPDATED label");
        assert!(header_text.contains("CHUNKS"), "header has CHUNKS label");
        // Header column boundaries match a representative row.
        // TAGS label starts at the same column as a row's tags column.
        let row = format_doc_row(&doc("ascii-title", &["rust"]), title_w);
        let tags_start_in_row = row.find("rust").expect("row has tags");
        let tags_start_in_header = header_text.find("TAGS").expect("header has TAGS");
        // Both labels are display-width-aligned via the same math, so
        // the header label starts at *or before* the row's data —
        // never after (which would imply the header drifted right).
        assert!(
            tags_start_in_header <= tags_start_in_row,
            "TAGS header drifted past row tags: header={tags_start_in_header} row={tags_start_in_row}"
        );
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p kebab-tui --lib format_doc_header`
Expected: FAIL — `format_doc_header` does not exist.

- [ ] **Step 3: Implement `format_doc_header`**

Still in `crates/kebab-tui/src/library.rs`, directly above the existing `pub(crate) fn format_doc_row` (around line 226), insert the new function:

```rust
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
```

If `Line` is not yet imported in this file, add `use ratatui::text::Line;` to the existing imports at the top (it is already imported in nearby render fns; copy the same path).

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p kebab-tui --lib format_doc_header`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/kebab-tui/src/library.rs
git commit -m "feat(kebab-tui): p9-fb-24 task 4 — Library format_doc_header"
```

---

### Task 5: Library column header — wire into `render_doc_list`

**Files:**
- Modify: `crates/kebab-tui/src/library.rs` (`render_doc_list`)
- Modify: `crates/kebab-tui/tests/library.rs` (extend or add render test)

- [ ] **Step 1: Write the failing integration test**

Open `crates/kebab-tui/tests/library.rs`. If a render-fixture test already exists (look for `TestBackend` + `render_library`), extend it to assert the header text is visible. Otherwise, append:

```rust
/// p9-fb-24: rendered Library pane shows the column header row above
/// the data rows. Header is in `Role::Heading` style; data rows in
/// the `Role::Body` / `Role::Selected` defaults.
#[test]
fn library_renders_column_header_row() {
    let mut app = fresh_app_with_three_docs();
    let backend = TestBackend::new(80, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| kebab_tui::render_library(f, Rect::new(0, 0, 80, 20), &app))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let rendered: String = (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        rendered.contains("TITLE") && rendered.contains("TAGS")
            && rendered.contains("UPDATED") && rendered.contains("CHUNKS"),
        "header row labels not visible in:\n{rendered}"
    );
    // Header sits ABOVE at least one data row — find a Y where
    // TITLE appears, then ensure a row below has actual doc content.
    let title_line = rendered
        .lines()
        .position(|line| line.contains("TITLE"))
        .expect("TITLE in some row");
    let after_header = rendered.lines().skip(title_line + 1).collect::<Vec<_>>();
    assert!(
        after_header.iter().any(|line| line.contains("doc-")),
        "no data rows after header:\n{rendered}"
    );
    // Suppress unused warning if app is otherwise unused.
    let _ = &app;
}
```

If `fresh_app_with_three_docs` does not exist in this file, look at the pre-existing fixture fn (likely `fresh_app` or `app_with_docs`) and either reuse it or add a small helper near the top of the test file:

```rust
fn fresh_app_with_three_docs() -> kebab_tui::App {
    let mut config = kebab_config::Config::defaults();
    config.storage.data_dir = "/tmp/kebab-tui-library-tests-noop".to_string();
    let mut app = kebab_tui::App::new(config).expect("App::new");
    app.library.inner.docs = vec![
        kebab_core::DocSummary {
            doc_id: kebab_core::DocumentId("d1".into()),
            title: "doc-alpha".to_string(),
            tags: vec!["rust".to_string()],
            updated_at: time::OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
            chunk_count: 5,
            // any remaining fields per current DocSummary shape
            ..Default::default()
        },
        kebab_core::DocSummary {
            doc_id: kebab_core::DocumentId("d2".into()),
            title: "doc-beta".to_string(),
            tags: vec!["docs".to_string()],
            updated_at: time::OffsetDateTime::from_unix_timestamp(1_700_001_000).unwrap(),
            chunk_count: 12,
            ..Default::default()
        },
        kebab_core::DocSummary {
            doc_id: kebab_core::DocumentId("d3".into()),
            title: "doc-gamma".to_string(),
            tags: vec![],
            updated_at: time::OffsetDateTime::from_unix_timestamp(1_700_002_000).unwrap(),
            chunk_count: 0,
            ..Default::default()
        },
    ];
    app
}
```

If `DocSummary` does not implement `Default`, hand-fill every field (look at the current struct definition in `kebab-core`).

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p kebab-tui --test library library_renders_column_header_row`
Expected: FAIL — header row absent because `render_doc_list` does not render it yet.

- [ ] **Step 3: Wire the header row into `render_doc_list`**

Open `crates/kebab-tui/src/library.rs`. Replace the body of `render_doc_list` (around line 192) with:

```rust
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
```

Note: the `block` is rendered first against `area`, then the inner content (header + list) is drawn into the block-inner area. The `List` no longer takes `.block(block)` — the block is rendered separately so the header row can sit inside it. If `Layout` / `Constraint` / `Direction` are not yet imported in this file's import block, add them: `use ratatui::layout::{Constraint, Direction, Layout};`.

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p kebab-tui --test library library_renders_column_header_row`
Expected: PASS.

- [ ] **Step 5: Run the full library test suite for regressions**

Run: `cargo test -p kebab-tui --test library`
Expected: All tests pass. The empty-state and Hangul-truncate tests must still hold.

- [ ] **Step 6: Commit**

```bash
git add crates/kebab-tui/src/library.rs crates/kebab-tui/tests/library.rs
git commit -m "feat(kebab-tui): p9-fb-24 task 5 — Library column header row"
```

---

### Task 6: Status bar — pane label, version, doc count, idle/streaming/searching cascade

**Files:**
- Modify: `crates/kebab-tui/src/run.rs` (new `render_status_bar`, rename `render_footer` → `render_key_hints`, drop `render_ingest_status` + conditional layout)
- Create: `crates/kebab-tui/tests/status_bar.rs`

- [ ] **Step 1: Write failing tests for the new `render_status_bar`**

Create `crates/kebab-tui/tests/status_bar.rs` with the full test suite:

```rust
//! p9-fb-24: integration tests for the always-visible status bar.

use kebab_config::Config;
use kebab_tui::{App, Pane};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;

fn fresh_app(focus: Pane) -> App {
    let mut config = Config::defaults();
    config.storage.data_dir = "/tmp/kebab-tui-status-bar-tests-noop".to_string();
    config.workspace.root = "/tmp/kebab-tui-status-bar-tests-noop/workspace".to_string();
    let mut app = App::new(config).expect("App::new");
    app.focus = focus;
    app
}

fn render_to_string(app: &App, width: u16) -> String {
    let backend = TestBackend::new(width, 1);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| kebab_tui::render_status_bar(f, Rect::new(0, 0, width, 1), app))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn status_bar_shows_kebab_version_first() {
    let app = fresh_app(Pane::Library);
    let rendered = render_to_string(&app, 100);
    let expected = format!("kebab v{}", env!("CARGO_PKG_VERSION"));
    assert!(
        rendered.contains(&expected),
        "version not in status bar: rendered=\n{rendered}"
    );
}

#[test]
fn status_bar_shows_pane_label() {
    for (focus, expected) in [
        (Pane::Library, "Library"),
        (Pane::Search, "Search"),
        (Pane::Ask, "Ask"),
        (Pane::Inspect, "Inspect"),
        (Pane::Jobs, "Jobs"),
    ] {
        let app = fresh_app(focus);
        let rendered = render_to_string(&app, 100);
        assert!(
            rendered.contains(expected),
            "pane label '{expected}' not visible for focus={focus:?}: rendered=\n{rendered}"
        );
    }
}

#[test]
fn status_bar_shows_doc_count() {
    let app = fresh_app(Pane::Library);
    // fresh_app's library starts with 0 docs.
    let rendered = render_to_string(&app, 100);
    assert!(
        rendered.contains("0 docs"),
        "doc count missing: rendered=\n{rendered}"
    );
}

#[test]
fn status_bar_idle_when_no_dynamic_state() {
    let app = fresh_app(Pane::Library);
    let rendered = render_to_string(&app, 100);
    assert!(
        rendered.contains("idle"),
        "idle marker missing: rendered=\n{rendered}"
    );
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p kebab-tui --test status_bar`
Expected: FAIL — `kebab_tui::render_status_bar` does not exist (linker error at the test compile step).

- [ ] **Step 3: Implement `render_status_bar` (idle path)**

Open `crates/kebab-tui/src/run.rs`. Add the new function directly after `render_header`:

```rust
/// p9-fb-24: always-visible status bar. Layout (left → right):
///
/// ```
/// kebab v0.1.0  │  <pane>  │  <docs> docs  │  [conv_<8hex>…  │  ]<state>
/// ```
///
/// `<state>` is one of `streaming…` / `searching…` / `indexing N/M (P%)` / `idle`,
/// chosen via the priority cascade:
///   1. Ask streaming → `streaming…`
///   2. Search worker active → `searching…`
///   3. Ingest worker active (or terminal-line still on hold) → ingest `status_line`
///   4. fallback → `idle`
///
/// `<conv_…>` only appears when `app.focus == Ask` AND the pane has
/// either an in-flight question or at least one completed turn — the
/// signal that "this Ask session has context".
pub fn render_status_bar(f: &mut Frame, area: Rect, app: &App) {
    let pane_label = match app.focus {
        Pane::Library => "Library",
        Pane::Search => "Search",
        Pane::Ask => "Ask",
        Pane::Inspect => "Inspect",
        Pane::Jobs => "Jobs",
    };
    let doc_count = app.library.inner.docs.len();
    let dynamic = dynamic_status(app);

    let sep = "  │  ";
    let mut line_text = format!(
        "kebab v{}{sep}{}{sep}{} docs{sep}",
        env!("CARGO_PKG_VERSION"),
        pane_label,
        doc_count,
    );
    if let Some(conv) = ask_conv_id_short(app) {
        line_text.push_str(&conv);
        line_text.push_str(sep);
    }
    line_text.push_str(&dynamic);

    let line = Line::from(Span::styled(
        line_text,
        app.theme.style(crate::theme::Role::Hint),
    ));
    f.render_widget(Paragraph::new(line), area);
}

/// Priority-cascade dynamic state for the status bar. See
/// `render_status_bar` for the priority order.
fn dynamic_status(app: &App) -> String {
    if app.ask.as_ref().map(|s| s.streaming).unwrap_or(false) {
        return "streaming…".to_string();
    }
    if app.search.as_ref().map(|s| s.searching).unwrap_or(false) {
        return "searching…".to_string();
    }
    if let Some(state) = app.ingest_state.as_ref() {
        return crate::ingest_progress::status_line(state);
    }
    "idle".to_string()
}

/// Short form of the Ask `conversation_id` for the status bar
/// (`conv_<first 8 hex chars>…`). Returns `None` when not in Ask, or
/// when the Ask pane has no context (no in-flight question and no
/// completed turns).
fn ask_conv_id_short(app: &App) -> Option<String> {
    if app.focus != Pane::Ask {
        return None;
    }
    let s = app.ask.as_ref()?;
    let has_context = s.current_question.is_some() || !s.turns.is_empty();
    if !has_context {
        return None;
    }
    let id = s.conversation_id.as_deref()?;
    // ID form is `conv_<32 hex>` per p9-fb-16; show first 8 hex of the
    // hex tail (skip the `conv_` prefix to keep the bar compact).
    let hex = id.strip_prefix("conv_").unwrap_or(id);
    let head: String = hex.chars().take(8).collect();
    Some(format!("conv_{head}…"))
}
```

- [ ] **Step 4: Export `render_status_bar` from the crate**

Open `crates/kebab-tui/src/lib.rs`. Find the `pub use run::cheatsheet_intercept;` (or similar `pub use run::*`) line. Add `pub use run::render_status_bar;` next to it.

- [ ] **Step 5: Run the new tests**

Run: `cargo test -p kebab-tui --test status_bar`
Expected: 4/4 PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/kebab-tui/src/run.rs crates/kebab-tui/src/lib.rs crates/kebab-tui/tests/status_bar.rs
git commit -m "feat(kebab-tui): p9-fb-24 task 6 — render_status_bar (version + pane + docs + idle)"
```

---

### Task 7: Status bar — streaming / searching / Ask conv_id paths

**Files:**
- Modify: `crates/kebab-tui/tests/status_bar.rs` (new tests)

This task adds tests that cover the cascade branches already implemented in Task 6. Pure verification — no production code changes if Task 6's `dynamic_status` + `ask_conv_id_short` are correct. Any test failure here is a Task-6 bug to fix in this task's commit.

- [ ] **Step 1: Add tests for streaming / searching / Ask conv_id**

Append to `crates/kebab-tui/tests/status_bar.rs`:

```rust
#[test]
fn status_bar_shows_streaming_when_ask_streaming() {
    let mut app = fresh_app(Pane::Ask);
    app.ask = Some(kebab_tui::AskState {
        streaming: true,
        ..Default::default()
    });
    let rendered = render_to_string(&app, 100);
    assert!(
        rendered.contains("streaming…"),
        "streaming marker missing: rendered=\n{rendered}"
    );
    assert!(
        !rendered.contains("idle"),
        "idle should not appear when streaming: rendered=\n{rendered}"
    );
}

#[test]
fn status_bar_shows_searching_when_search_worker_active() {
    let mut app = fresh_app(Pane::Search);
    // SearchState::default may not be public; create manually.
    let mut search_state = kebab_tui::SearchState::default();
    search_state.searching = true;
    app.search = Some(search_state);
    let rendered = render_to_string(&app, 100);
    assert!(
        rendered.contains("searching…"),
        "searching marker missing: rendered=\n{rendered}"
    );
}

#[test]
fn status_bar_shows_ask_conv_id_when_in_ask_with_context() {
    let mut app = fresh_app(Pane::Ask);
    let mut ask_state = kebab_tui::AskState::default();
    ask_state.conversation_id = Some("conv_a3f9b2c1d4e5f6a7b8c9d0e1f2a3b4c5".to_string());
    ask_state.current_question = Some("test?".to_string());
    app.ask = Some(ask_state);
    let rendered = render_to_string(&app, 100);
    assert!(
        rendered.contains("conv_a3f9b2c1…"),
        "8-hex prefix conv id missing: rendered=\n{rendered}"
    );
}

#[test]
fn status_bar_omits_conv_id_when_ask_has_no_context() {
    // Ask pane focused, but no question / no turns yet.
    let mut app = fresh_app(Pane::Ask);
    app.ask = Some(kebab_tui::AskState::default());
    let rendered = render_to_string(&app, 100);
    assert!(
        !rendered.contains("conv_"),
        "conv id should not appear without context: rendered=\n{rendered}"
    );
}

#[test]
fn status_bar_omits_conv_id_outside_ask() {
    let mut app = fresh_app(Pane::Library);
    // Even with an Ask state populated, focus = Library hides conv id.
    let mut ask_state = kebab_tui::AskState::default();
    ask_state.conversation_id = Some("conv_a3f9b2c1d4e5f6a7b8c9d0e1f2a3b4c5".to_string());
    ask_state.current_question = Some("test?".to_string());
    app.ask = Some(ask_state);
    let rendered = render_to_string(&app, 100);
    assert!(
        !rendered.contains("conv_"),
        "conv id leaked outside Ask pane: rendered=\n{rendered}"
    );
}
```

If `AskState::default()` is not exposed at the crate root, add `pub use app::AskState;` to `crates/kebab-tui/src/lib.rs`. Same for `SearchState` if needed.

- [ ] **Step 2: Run the new tests**

Run: `cargo test -p kebab-tui --test status_bar`
Expected: 9/9 PASS (4 from Task 6 + 5 new).

If any fail, fix the corresponding branch in `dynamic_status` or `ask_conv_id_short` in `crates/kebab-tui/src/run.rs`.

- [ ] **Step 3: Commit**

```bash
git add crates/kebab-tui/src/lib.rs crates/kebab-tui/tests/status_bar.rs
git commit -m "test(kebab-tui): p9-fb-24 task 7 — status bar streaming / searching / conv_id"
```

---

### Task 8: Status bar — ingest progress absorbed

**Files:**
- Modify: `crates/kebab-tui/tests/status_bar.rs`

This task pins that the existing `kebab_tui::ingest_progress::status_line` output appears verbatim in the status bar's dynamic slot when an ingest is in flight, so the absorption (Task 9) is provably equivalent to the pre-fb-24 dedicated row.

- [ ] **Step 1: Add the ingest absorb test**

Append to `crates/kebab-tui/tests/status_bar.rs`:

```rust
#[test]
fn status_bar_shows_ingest_progress_in_dynamic_slot() {
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    let mut app = fresh_app(Pane::Library);
    // Construct a minimal IngestState mirroring an in-flight worker.
    // counts.scanned = 40 means "40 assets enumerated"; current_idx = 12
    // means "currently working on asset 12 of 40" → 30%.
    let (_tx, rx) = std::sync::mpsc::channel();
    app.ingest_state = Some(kebab_tui::IngestState {
        rx,
        counts: kebab_app::AggregateCounts {
            scanned: 40,
            ..Default::default()
        },
        current_path: Some("notes/foo.md".to_string()),
        current_idx: 12,
        started_at: std::time::Instant::now(),
        terminal_at: None,
        aborted: false,
        thread: None,
        cancel: Arc::new(AtomicBool::new(false)),
    });
    let rendered = render_to_string(&app, 200);
    assert!(
        rendered.contains("12/40"),
        "ingest progress fragment missing: rendered=\n{rendered}"
    );
    assert!(
        rendered.contains("30%"),
        "ingest percentage missing: rendered=\n{rendered}"
    );
    assert!(
        !rendered.contains("idle"),
        "idle should not appear during ingest: rendered=\n{rendered}"
    );
}
```

If `IngestState`, `AggregateCounts`, or `kebab_app` are not at the test-visible re-export path, add the necessary `pub use` lines to `crates/kebab-tui/src/lib.rs` (e.g. `pub use app::IngestState;`). Test imports may need `kebab_app` (path-dep already present in Cargo.toml as a workspace dep — add `kebab-app = { path = "../kebab-app" }` to `[dev-dependencies]` if not already there).

- [ ] **Step 2: Run the test**

Run: `cargo test -p kebab-tui --test status_bar status_bar_shows_ingest_progress`
Expected: PASS — `dynamic_status` already calls `ingest_progress::status_line` per Task 6.

If the test cannot construct an `IngestState` because some field is missing or has a non-`Default` type, hand-fill it from the struct definition in `crates/kebab-tui/src/app.rs`. Do NOT add a new `Default` impl to `IngestState` — the test should reflect a realistic in-flight state.

- [ ] **Step 3: Commit**

```bash
git add crates/kebab-tui/tests/status_bar.rs crates/kebab-tui/Cargo.toml
git commit -m "test(kebab-tui): p9-fb-24 task 8 — status bar absorbs ingest progress"
```

---

### Task 9: Wire status bar + key hint bar into `render_root` (drop ingest row)

**Files:**
- Modify: `crates/kebab-tui/src/run.rs` (`render_root`, rename `render_footer`, drop `render_ingest_status`)

- [ ] **Step 1: Restructure `render_root` for the new 4-row layout**

Open `crates/kebab-tui/src/run.rs`. Replace the body of `render_root` (around line 234) with:

```rust
fn render_root(f: &mut Frame, app: &App) {
    // p9-fb-24: bottom is always 2 rows — status bar + key hints.
    // The pre-fb-24 conditional ingest-status row is gone; the
    // ingest progress text now appears in the status bar's dynamic
    // slot (see `dynamic_status` priority cascade).
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),  // top header
            Constraint::Min(1),     // pane content
            Constraint::Length(1),  // status bar
            Constraint::Length(1),  // key hint bar
        ])
        .split(f.area());
    render_header(f, outer[0], app);
    match app.focus {
        Pane::Library => render_library(f, outer[1], app),
        Pane::Search => render_search(f, outer[1], app),
        Pane::Ask => render_ask(f, outer[1], app),
        Pane::Inspect => render_inspect(f, outer[1], app),
        // p9-5 Jobs not yet rendered; Library placeholder.
        Pane::Jobs => render_library(f, outer[1], app),
    }
    render_status_bar(f, outer[2], app);
    render_key_hints(f, outer[3], app);
    if let Some(err) = &app.error_overlay {
        render_error_overlay(f, f.area(), err, &app.theme);
    }
    if app.cheatsheet_visible {
        crate::cheatsheet::render_cheatsheet(f, f.area(), app);
    }
}
```

- [ ] **Step 2: Rename `render_footer` to `render_key_hints`**

Still in `crates/kebab-tui/src/run.rs`, find `fn render_footer(f: &mut Frame, area: Rect, app: &App)` (around line 330). Rename it to `render_key_hints`. The body is unchanged.

- [ ] **Step 3: Delete the obsolete `render_ingest_status`**

In the same file, delete the entire `fn render_ingest_status` (around lines 283-303). Its content is now covered by `dynamic_status` calling `ingest_progress::status_line`.

- [ ] **Step 4: Verify the build + full test suite**

Run: `cargo build -p kebab-tui`
Expected: Finished. No unused-import / dead-code warnings (if any pop up, the corresponding import in `run.rs` — likely the `ingest_progress` direct import — should be cleaned up).

Run: `cargo test -p kebab-tui`
Expected: All tests pass. The status_bar suite from Tasks 6–8, the library suite, the ask suite, and the existing `footer_hints_tests` (whose function names like `render_footer` may need updating in the test module — the rename is intra-crate, so any caller in `run::tests` must follow). Inspect, Search, Cheatsheet suites unaffected.

- [ ] **Step 5: Run clippy**

Run: `cargo clippy -p kebab-tui --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/kebab-tui/src/run.rs
git commit -m "feat(kebab-tui): p9-fb-24 task 9 — render_root uses status bar + key hints (drop ingest row)"
```

---

### Task 10: Cheatsheet — Ask gains PgUp / PgDn row

**Files:**
- Modify: `crates/kebab-tui/src/cheatsheet.rs`

- [ ] **Step 1: Find the existing Ask section in the cheatsheet**

Open `crates/kebab-tui/src/cheatsheet.rs`. Find the `push_section(&mut lines, &app.theme, "Ask", &[ ... ]);` block (around line 88). It currently lists keys including `j / k` "scroll transcript" and `Shift-G` "jump to bottom".

- [ ] **Step 2: Add the `PgUp / PgDn` row directly after the `Shift-G` entry**

Replace the Ask `push_section` array with:

```rust
    push_section(&mut lines, &app.theme, "Ask", &[
        ("type", "question (Insert)"),
        ("Enter", "submit"),
        ("e", "toggle explain mode (Normal)"),
        ("j / k", "scroll transcript (Normal — disengages auto-tail)"),
        ("Shift-G", "jump to bottom + re-engage auto-tail (p9-fb-22)"),
        ("PgUp / PgDn", "page-scroll the transcript (p9-fb-24, disengages auto-tail)"),
        ("← / →", "move cursor in input (p9-fb-22)"),
        ("Home / End", "cursor to start / end of input"),
        ("Delete", "remove char at cursor"),
        ("i", "Normal → Insert (toggle back to typing)"),
        ("Ctrl-L", "new conversation (clears turns)"),
        ("Esc", "back to Library (cancels in-flight worker)"),
    ]);
```

- [ ] **Step 3: Verify the cheatsheet test still passes**

Run: `cargo test -p kebab-tui --test cheatsheet`
Expected: All tests pass. The existing `cheatsheet_popup_contains_global_and_pane_sections` test asserts presence of section headings, not row counts.

- [ ] **Step 4: Commit**

```bash
git add crates/kebab-tui/src/cheatsheet.rs
git commit -m "docs(kebab-tui): p9-fb-24 task 10 — cheatsheet Ask gains PgUp / PgDn row"
```

---

### Task 11: Final workspace verification + docs sync

**Files:**
- Modify: `README.md`
- Modify: `HANDOFF.md`
- Modify: `tasks/HOTFIXES.md`
- Modify: `tasks/INDEX.md`
- Create: `tasks/p9/p9-fb-24-tui-affordances.md`

- [ ] **Step 1: Run the full workspace test**

Run: `cargo test --workspace --no-fail-fast -j 1`
Expected: 720+ passed, 0 failed (the pre-fb-24 baseline was 699; this PR adds ~20 tests across status_bar / library / ask).

- [ ] **Step 2: Run workspace clippy**

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 3: Update README.md**

Open `README.md`. Find the `kebab tui` row in the command table (around line 79). Append to the cell:

```
모든 모드에서 항상 떠 있는 상태바 — `kebab v<version> │ <pane> │ <docs> docs │ <state>` (state: streaming/searching/indexing/idle, ingest 진행 중에는 progress 가 같은 자리에 흡수됨). Ask 진입 시 conversation id 8 자 prefix 도 함께 표시. Ask 트랜스크립트와 Inspect 양쪽에서 `PgUp / PgDn` 으로 10 줄씩 페이지 스크롤. Library 의 doc list 위에는 `TITLE / TAGS / UPDATED / CHUNKS` 컬럼 헤더 행 표시 (display-width 정렬, Hangul / CJK 안전).
```

(Place this in the same long-line cell as existing TUI description; do not add a new row.)

- [ ] **Step 4: Update HANDOFF.md**

Open `HANDOFF.md`. Find the `## 머지 후 발견된 버그 / 결정 (요약)` section. Add a new entry directly above the `2026-05-04 P9 post-도그푸딩 (p9-fb-22)` row:

```
- **2026-05-04 P9 post-도그푸딩 (p9-fb-24)** — TUI status/key bar + Library 컬럼 헤더 + Ask/Inspect PgUp/PgDn. 사용자 도그푸딩 3 건 (Library 컬럼 의미 부재, 페이지 스크롤 키 부재, 상태바 + 버전 정보 항상 노출 요청) 을 단일 PR 로 통합. bottom 영역을 status bar (1 row, version + pane + docs + dynamic state) + key hint bar (1 row, 기존 `footer_hints` 그대로) 두 줄로 분할; 기존 ingest progress dedicated row 는 status bar 의 dynamic slot 에 흡수 (priority cascade: streaming → searching → indexing → idle). Library `List` 위에 `format_doc_header` 행 + Layout 분할로 헤더 표시 (TITLE / TAGS / UPDATED / CHUNKS, display-width 정렬). `kebab-tui::pager::PAGE_STEP = 10` 신규 — Ask 의 PgUp/PgDn 추가 + Inspect 의 기존 +/-10 hardcode 가 같은 상수 참조로 통일. Ask 의 page-scroll 은 `j`/`k` 와 동일하게 `follow_tail = false` 로 freeze. spec: `tasks/p9/p9-fb-24-tui-affordances.md`. HOTFIXES `2026-05-04 — p9-fb-24` 항목이 footer 단행 row (p9-fb-13) + ingest dedicated row (p9-fb-03) 와의 layout 충돌의 source of truth.
```

- [ ] **Step 5: Update HOTFIXES.md**

Open `tasks/HOTFIXES.md`. Add a new entry at the top of the dated list (above the `2026-05-04 — p9-fb-22` entry):

```markdown
## 2026-05-04 — p9-fb-24 (post-dogfooding): TUI status bar + Library 헤더 + page scroll

**Source feedback**: 사용자 도그푸딩 2026-05-04 — (1) Library 컬럼이 무엇을 뜻하는지 헤더 부재, (2) Ask 트랜스크립트 / Inspect 둘 다 페이지 단위 스크롤 키 필요, (3) 모든 모드에서 항상 떠 있는 상태바 + 키 안내바 (버전 정보 포함) 가 있으면 좋겠다.

**Live binding 변경**:

- bottom 영역을 2 row 로 분할. 윗줄 = status bar (`kebab v<version> │ <pane> │ <docs> docs │ <state>`), 아랫줄 = key hint bar (기존 `footer_hints` 그대로). p9-fb-13 follow-up 의 single-row footer 와 충돌 — frozen spec 텍스트 보존, 본 항목이 live source of truth.
- ingest progress 의 dedicated row (p9-fb-03) 는 status bar 의 dynamic slot 으로 흡수. priority cascade: streaming → searching → indexing → idle. 시각적 위치 변경, 콘텐츠 동등.
- `Paragraph::line_count` 등 unstable feature 추가 없음.
- `crates/kebab-tui/src/pager.rs::PAGE_STEP = 10` 신규. Ask 의 PgUp/PgDn 추가 (mode 무관, `follow_tail = false` flip), Inspect 의 기존 +/-10 hardcode 가 같은 상수 참조로 일원화.
- `format_doc_header(area_width)` 신규 (kebab-tui/src/library.rs). Library 의 doc list 위에 1-row 헤더 (TITLE / TAGS / UPDATED / CHUNKS, display-width 정렬). Block 의 inner area 를 `Layout` 으로 header (Length 1) + list (Min 0) 로 분할.
- cheatsheet popup Ask section 에 `PgUp / PgDn` row 추가 (Inspect 는 이미 명시).

**Spec contract impact**: p9-fb-13 follow-up (footer 단행 row) + p9-fb-03 (ingest dedicated row) frozen spec 들과 layout 충돌. frozen 텍스트 보존, 본 HOTFIXES 항목 + spec `tasks/p9/p9-fb-24-tui-affordances.md` + design `docs/superpowers/specs/2026-05-04-p9-fb-24-tui-affordances-design.md` 가 live source of truth.

**Tests added**: 약 20 신규 (status_bar 통합 9 + library 헤더 1 + Ask PgUp/PgDn 3 + Inspect PgUp/PgDn 회귀 2 + format_doc_header 단위 1, 잔여는 cascade branch 별). 기존 720+ 워크스페이스 테스트 무수정 통과.

**Known limitation (deferred)**: `PAGE_STEP = 10` 은 viewport-aware 가 아님 — 24 row 작은 터미널에서 한 페이지 > viewport, 80 row 큰 터미널에서 한 페이지 < viewport. 후속 task 에서 viewport-aware 로 업그레이드 가능.
```

- [ ] **Step 6: Update INDEX.md**

Open `tasks/INDEX.md`. Find the `p9-fb-22` entry and append below:

```
  - [p9-fb-24 status bar + Library header + page scroll (post-도그푸딩)](p9/p9-fb-24-tui-affordances.md)
```

- [ ] **Step 7: Create the per-task spec file**

Create `tasks/p9/p9-fb-24-tui-affordances.md`:

```markdown
---
phase: P9
component: kebab-tui
task_id: p9-fb-24
title: "TUI status/key bar + Library 컬럼 헤더 + Ask/Inspect PgUp/PgDn (post-merge dogfooding)"
status: completed
depends_on: [p9-fb-03, p9-fb-13, p9-fb-22]
unblocks: []
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§1 UX, §10 UX]
source_feedback: 사용자 도그푸딩 2026-05-04 — Library 컬럼 의미 부재, 페이지 스크롤 키 부재, 항상 떠 있는 상태바 (버전 포함) 요청.
---

# p9-fb-24 — TUI status/key bar + Library 헤더 + page scroll

상세 설계: `docs/superpowers/specs/2026-05-04-p9-fb-24-tui-affordances-design.md`.
구현 계획: `docs/superpowers/plans/2026-05-04-p9-fb-24-tui-affordances.md`.

## Goal

- bottom 영역을 2 row 로 분할 (status bar + key hint bar). 모든 모드 / pane 에서 항상 노출.
- ingest progress 의 dedicated row 를 status bar 의 dynamic slot 으로 흡수.
- Library doc list 위에 컬럼 헤더 row.
- Ask + Inspect 양쪽에 `PgUp` / `PgDn` (fixed `PAGE_STEP = 10`).

## Behavior contract

- Status bar 좌→우: `kebab v<version> │ <pane> │ <docs> docs │ [conv_<8hex>…  │ ]<dynamic_status>`.
- Dynamic state cascade: streaming (Ask) → searching (Search) → indexing (Ingest) → idle.
- conv_id (8-hex prefix + ellipsis) 는 Ask focused + (current_question 또는 turns) 일 때만.
- Library 헤더: `TITLE / TAGS / UPDATED / CHUNKS`, `Role::Heading`. `format_doc_row` 와 boundary 일치.
- Ask `PgUp/PgDn`: `j`/`k` 와 동일 follow_tail freeze. mode 무관.
- Inspect `PgUp/PgDn`: 기존 +/-10 그대로 (단 PAGE_STEP 상수 참조).

## Tests

- status_bar 통합 약 9 (version / pane / docs / idle / streaming / searching / ingest absorb / Ask conv_id present / Ask conv_id absent).
- library 통합 1 (헤더 row visible).
- Ask 통합 3 (PgDn / PgUp / PgUp saturating + freeze follow_tail).
- Inspect 통합 2 (PAGE_STEP regression).
- format_doc_header 단위 1.
- 기존 720+ 테스트 무수정 통과.

## Risks / notes

- `PAGE_STEP = 10` magic — viewport-aware 후속 task 가능.
- 60 컬럼 미만 터미널은 status bar wrap → 1 row 추가 차지.

Live deviations 반영 위치: `tasks/HOTFIXES.md` `2026-05-04 — p9-fb-24` 항목.
```

- [ ] **Step 8: Final commit**

```bash
git add README.md HANDOFF.md tasks/HOTFIXES.md tasks/INDEX.md tasks/p9/p9-fb-24-tui-affordances.md
git commit -m "docs(p9-fb-24): README + HANDOFF + HOTFIXES + INDEX + per-task spec"
```

---

## Self-Review Notes (writer)

**Spec coverage:**
- Status bar (4 fragments + cascade + Ask conv_id) → Tasks 6, 7, 8.
- Layout 2-row split + ingest absorb → Task 9.
- Library column header → Tasks 4, 5.
- Ask PgUp/PgDn → Task 3.
- Inspect PAGE_STEP unification → Task 2.
- pager module → Task 1.
- Cheatsheet update → Task 10.
- Docs sync (README + HANDOFF + HOTFIXES + INDEX + spec) → Task 11.

**Type / API consistency:** `pager::PAGE_STEP` is `pub(crate) const u16 = 10`, used by both Ask and Inspect. `render_status_bar` is `pub` (re-exported from `lib.rs`). `render_key_hints` replaces `render_footer` (rename only — same signature). `format_doc_header(title_w: usize) -> Line<'static>`. `dynamic_status(app: &App) -> String` and `ask_conv_id_short(app: &App) -> Option<String>` are private to `run.rs`.

**Placeholder scan:** No `TBD` / `TODO`. Each step has the full code or exact command. The Library test fixture (`fresh_app_with_three_docs`) is shown in full.

**Risks documented:** `PAGE_STEP = 10` magic constant deferred for viewport-aware refinement. 60-col wrap behaviour acknowledged. Default impls (`AskState`, `SearchState`) may need verification at test time — the plan flags this so the implementer can adjust.
