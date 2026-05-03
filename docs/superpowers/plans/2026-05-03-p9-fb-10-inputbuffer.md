# p9-fb-10 InputBuffer Follow-up Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Finish the deferred portion of p9-fb-10 — introduce `kebab-tui::input::InputBuffer` (string + display-column cursor), migrate every text-input pane to use it, render a column-aligned cursor, and add a Korean FTS5 smoke pin.

**Architecture:** Build on the existing `kebab-tui::input::{display_width, truncate_to_display_width}` helpers from PR #87. Add `InputBuffer { content: String, cursor_col: usize }` next to them. Every pane that owns a `String` text input gets swapped to `InputBuffer`. The renderer sets `f.set_cursor(...)` so the terminal caret lives at the actual display column, not the char count. NFC normalization of search queries is already in place upstream, so the FTS5 smoke pin is just an integration assertion.

**Tech Stack:** Rust 2024, `unicode-width = "0.2"` (already a dep), ratatui 0.28, crossterm 0.28, kebab-tui internal modules.

---

## File Structure

- **`crates/kebab-tui/src/input.rs`** — extend with `InputBuffer` struct + methods + tests. Single owner of CJK width logic.
- **`crates/kebab-tui/src/app.rs`** — `SearchState.input: String` → `InputBuffer`, `AskState.input: String` → `InputBuffer`, `FilterEdit.{tags_buf, lang_buf}: String` → `InputBuffer` (Library is the third text input — even though the HOTFIXES checklist used "Editor" loosely, FilterEdit is the correct match).
- **`crates/kebab-tui/src/search.rs`** — key handlers + `render_input_bar` cursor placement.
- **`crates/kebab-tui/src/ask.rs`** — key handlers + render cursor placement.
- **`crates/kebab-tui/src/library.rs`** — FilterEdit key handler + render cursor.
- **`crates/kebab-tui/tests/{search,ask,library}.rs`** — fixture updates + Hangul typing pin.
- **`crates/kebab-app/tests/search_korean.rs`** (new) — Korean FTS5 smoke pin via the facade.
- **`tasks/HOTFIXES.md`** — flip 5 checkboxes done.
- **`tasks/p9/p9-fb-10-tui-cjk-input.md`** — `status: in_progress` → `completed`, DoD boxes ticked.
- **`README.md`** + **`HANDOFF.md`** — mention the cursor + Korean FTS5 pin landing.

---

## Task 1: `InputBuffer` struct + unit tests

**Files:**
- Modify: `crates/kebab-tui/src/input.rs`

- [ ] **Step 1: Append `InputBuffer` struct and methods**

```rust
/// Text input buffer that tracks **display column** position, not
/// char count. Every wide char (Hangul / Kanji / fullwidth) advances
/// `cursor_col` by 2; every ASCII char by 1. Backspace pops one
/// char (`String::pop()` is char-aware) and rewinds the cursor by
/// that char's width.
///
/// Cursor invariant: `cursor_col == display_width(&content)` —
/// the cursor sits at the right edge of the typed content. v1
/// is append-only; mid-string editing (insert at cursor / arrow
/// key navigation) is out of scope and would relax this invariant.
#[derive(Debug, Default, Clone)]
pub struct InputBuffer {
    content: String,
    cursor_col: usize,
}

impl InputBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a single char and advance cursor by its display width.
    /// Zero-width chars (combining marks) leave the cursor in place
    /// but still extend `content`.
    pub fn push_char(&mut self, ch: char) {
        let w = UnicodeWidthChar::width(ch).unwrap_or(0);
        self.content.push(ch);
        self.cursor_col += w;
    }

    /// Append a `&str` char-by-char. Same width semantics as
    /// `push_char` per element.
    pub fn push_str(&mut self, s: &str) {
        for ch in s.chars() {
            self.push_char(ch);
        }
    }

    /// Remove the trailing char (Backspace) and rewind the cursor
    /// by that char's display width. No-op on empty input.
    pub fn pop_char(&mut self) -> Option<char> {
        let ch = self.content.pop()?;
        let w = UnicodeWidthChar::width(ch).unwrap_or(0);
        self.cursor_col = self.cursor_col.saturating_sub(w);
        Some(ch)
    }

    /// Reset to empty.
    pub fn clear(&mut self) {
        self.content.clear();
        self.cursor_col = 0;
    }

    /// Borrow the typed text.
    pub fn as_str(&self) -> &str {
        &self.content
    }

    /// Cursor column (display-width units). Matches
    /// `display_width(self.as_str())` by construction.
    pub fn cursor_col(&self) -> usize {
        self.cursor_col
    }

    /// True when no chars have been typed.
    pub fn is_empty(&self) -> bool {
        self.content.is_empty()
    }

    /// Length of `content` in chars (NOT display columns). Use
    /// `cursor_col()` for column-aware layout.
    pub fn char_len(&self) -> usize {
        self.content.chars().count()
    }
}
```

- [ ] **Step 2: Add unit tests at the bottom of `mod tests`**

```rust
    /// p9-fb-10: ASCII typing advances cursor by 1 per char.
    #[test]
    fn input_buffer_ascii_cursor_advances_by_one() {
        let mut b = InputBuffer::new();
        for ch in "hello".chars() {
            b.push_char(ch);
        }
        assert_eq!(b.cursor_col(), 5);
        assert_eq!(b.as_str(), "hello");
    }

    /// p9-fb-10: Hangul typing advances cursor by 2 per char.
    #[test]
    fn input_buffer_hangul_cursor_advances_by_two() {
        let mut b = InputBuffer::new();
        for ch in "한글".chars() {
            b.push_char(ch);
        }
        assert_eq!(b.cursor_col(), 4);
        assert_eq!(b.as_str(), "한글");
    }

    /// p9-fb-10: Backspace rewinds cursor by the popped char's
    /// width — Hangul rewinds by 2, ASCII by 1.
    #[test]
    fn input_buffer_pop_char_rewinds_cursor_by_width() {
        let mut b = InputBuffer::new();
        b.push_str("러스트");
        assert_eq!(b.cursor_col(), 6);
        let popped = b.pop_char();
        assert_eq!(popped, Some('트'));
        assert_eq!(b.cursor_col(), 4);
        assert_eq!(b.as_str(), "러스");
        b.push_char('a');
        assert_eq!(b.cursor_col(), 5);
        assert_eq!(b.as_str(), "러스a");
    }

    /// p9-fb-10: cursor invariant — cursor_col always equals
    /// display_width(content).
    #[test]
    fn input_buffer_cursor_matches_display_width() {
        let mut b = InputBuffer::new();
        for ch in "Hello, 세계 mixed".chars() {
            b.push_char(ch);
        }
        assert_eq!(b.cursor_col(), display_width(b.as_str()));
    }

    /// p9-fb-10: clear resets both content and cursor.
    #[test]
    fn input_buffer_clear_resets_state() {
        let mut b = InputBuffer::new();
        b.push_str("한글");
        b.clear();
        assert_eq!(b.cursor_col(), 0);
        assert!(b.is_empty());
    }

    /// p9-fb-10: pop_char on empty input returns None and leaves
    /// cursor at 0 (no underflow).
    #[test]
    fn input_buffer_pop_on_empty_is_noop() {
        let mut b = InputBuffer::new();
        assert!(b.pop_char().is_none());
        assert_eq!(b.cursor_col(), 0);
    }
```

- [ ] **Step 3: Re-export `InputBuffer` from `lib.rs`**

```rust
// crates/kebab-tui/src/lib.rs
pub use input::{InputBuffer, display_width, truncate_to_display_width};
```

- [ ] **Step 4: Run unit tests**

```bash
cargo test -p kebab-tui --lib input::
```

Expected: PASS — 6 new tests + 9 existing.

- [ ] **Step 5: Clippy clean**

```bash
cargo clippy -p kebab-tui --all-targets -- -D warnings
```

Expected: no warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/kebab-tui/src/input.rs crates/kebab-tui/src/lib.rs
git commit -m "feat(kebab-tui): InputBuffer struct (display-column cursor)"
```

---

## Task 2: Migrate `SearchState.input` to `InputBuffer`

**Files:**
- Modify: `crates/kebab-tui/src/app.rs` (SearchState declaration + Default)
- Modify: `crates/kebab-tui/src/search.rs` (key handlers + render_input_bar)
- Modify: `crates/kebab-tui/tests/search.rs` (fixtures + assertions)
- Modify: `crates/kebab-tui/src/run.rs` (any external readers — likely just `s.input.as_str()`)

- [ ] **Step 1: Read all sites currently using `SearchState.input`**

```bash
grep -n "\.input\b" crates/kebab-tui/src/{app,search,run}.rs crates/kebab-tui/tests/search.rs
```

Map each call site:
- `s.input.push(c)` → `s.input.push_char(c)`
- `s.input.pop()` → `s.input.pop_char()` (return type matches: `Option<char>`)
- `s.input.clear()` → `s.input.clear()` (same name)
- `s.input.as_str()` / `&s.input` → `s.input.as_str()`
- `s.input.is_empty()` → `s.input.is_empty()`
- Comparisons like `last_query.as_ref().map(|(q, _)| q == &s.input)` → `q.as_str() == s.input.as_str()` (since `last_query` stores `String`)

- [ ] **Step 2: Change the field type in `app.rs`**

```rust
// crates/kebab-tui/src/app.rs (SearchState)
pub input: crate::input::InputBuffer,
```

Update `Default` impl:
```rust
input: crate::input::InputBuffer::new(),
```

- [ ] **Step 3: Update `crates/kebab-tui/src/search.rs` key handlers**

Replace each call site per the map. The mode-authoritative dispatch (Insert vs Normal) for `j`/`k`/Char(c) stays — only the underlying push/pop call shape changes.

- [ ] **Step 4: Update `render_input_bar` to set the cursor at the column**

```rust
fn render_input_bar(f: &mut Frame, area: Rect, s: &SearchState, theme: &crate::theme::Theme) {
    let mode_label = mode_label(s.mode);
    let mode_role = match s.mode { /* unchanged */ };
    let searching_hint = if s.searching { "  searching…" } else { "" };
    let prompt = format!("[{mode_label}] ");
    let prompt_w = crate::input::display_width(&prompt);
    let line = Line::from(vec![
        Span::styled(prompt.clone(), theme.style(mode_role)),
        Span::raw(s.input.as_str()),
        Span::styled(searching_hint, theme.style(crate::theme::Role::Hint)),
    ]);
    let block = Block::default()
        .title("query (Tab=mode  Enter=search  Esc=back)")
        .borders(Borders::ALL);
    let inner = block.inner(area);
    f.render_widget(Paragraph::new(line).block(block), area);
    // Cursor sits inside the block (after the borders), at
    // `prompt_w + cursor_col` columns from the inner left edge.
    // Only place the cursor when the pane is the active text input
    // — the run loop is responsible for choosing which pane gets
    // the caret.
    let cursor_x = inner.x + (prompt_w + s.input.cursor_col()) as u16;
    let cursor_y = inner.y;
    f.set_cursor_position((cursor_x, cursor_y));
}
```

Note: ratatui 0.28 uses `Frame::set_cursor_position((x, y))` — verify the exact API; older drafts used `set_cursor(x, y)`. If the older form is what's compiled in, fall back to `f.set_cursor(cursor_x, cursor_y)`.

The cursor will only show if the terminal isn't currently in `Hide` mode. The run loop currently calls `Hide` on startup. Coordinate with Task 7 (out of scope here — see "Out of plan" below) on whether to flip back to `Show`. For this task: just place the cursor; visibility is a follow-up if needed.

- [ ] **Step 5: Update `crates/kebab-tui/tests/search.rs`**

Replace direct `s.input = "...".to_string()` test setup with `s.input.push_str("...")`. Replace `assert_eq!(app.search.input, "abc")` with `assert_eq!(app.search.input.as_str(), "abc")`.

- [ ] **Step 6: Run tests**

```bash
cargo test -p kebab-tui --test search
cargo test -p kebab-tui --lib search::
```

Expected: PASS.

- [ ] **Step 7: Clippy + full tui suite**

```bash
cargo clippy -p kebab-tui --all-targets -- -D warnings
cargo test -p kebab-tui
```

- [ ] **Step 8: Commit**

```bash
git add crates/kebab-tui/src/{app,search,run}.rs crates/kebab-tui/tests/search.rs
git commit -m "feat(kebab-tui): SearchState.input → InputBuffer"
```

---

## Task 3: Migrate `AskState.input` to `InputBuffer`

**Files:**
- Modify: `crates/kebab-tui/src/app.rs` (AskState declaration)
- Modify: `crates/kebab-tui/src/ask.rs` (key handlers + render cursor)
- Modify: `crates/kebab-tui/tests/ask.rs`

Same pattern as Task 2. Specific call sites today (line numbers approximate):
- `ask.rs` ~line 366: `s.input.pop()` → `s.input.pop_char()`
- `ask.rs` ~line 377: `s.input.push(c)` → `s.input.push_char(c)`
- `ask.rs` ~line 400: `s.input.clear()` (unchanged name)

When the worker thread is spawned, the question text snapshot uses `s.input.as_str()` (or moves out via `mem::take` — adapt: `let question = std::mem::take(&mut s.input).into_content()` if a moving-out helper is added, or just `let question = s.input.as_str().to_owned(); s.input.clear();`).

If a moving-out helper is desired, add to `InputBuffer`:

```rust
/// Move the typed string out, leaving the buffer empty (cursor 0).
/// Convenience for "submit" flows.
pub fn take(&mut self) -> String {
    self.cursor_col = 0;
    std::mem::take(&mut self.content)
}
```

Plus a unit test:

```rust
    /// p9-fb-10: take() returns the content and resets state.
    #[test]
    fn input_buffer_take_returns_content_and_resets() {
        let mut b = InputBuffer::new();
        b.push_str("러스트");
        let s = b.take();
        assert_eq!(s, "러스트");
        assert!(b.is_empty());
        assert_eq!(b.cursor_col(), 0);
    }
```

- [ ] **Step 1: Add `take()` method + unit test to `input.rs`**
- [ ] **Step 2: Change AskState.input field type**
- [ ] **Step 3: Update ask.rs key handlers**
- [ ] **Step 4: Update render to place cursor at the input column** (use the same pattern as Task 2 — find the input rendering site in `ask.rs` and call `f.set_cursor_position`)
- [ ] **Step 5: Update `tests/ask.rs` fixtures + assertions**
- [ ] **Step 6: `cargo test -p kebab-tui` + `cargo clippy`**
- [ ] **Step 7: Commit**

```bash
git add crates/kebab-tui/src/{app,ask,input}.rs crates/kebab-tui/tests/ask.rs
git commit -m "feat(kebab-tui): AskState.input → InputBuffer + take() helper"
```

---

## Task 4: Migrate `FilterEdit.tags_buf` / `lang_buf` to `InputBuffer`

**Files:**
- Modify: `crates/kebab-tui/src/library.rs` (FilterEdit struct + key handler + render_filter_overlay)
- Modify: `crates/kebab-tui/tests/library.rs` (filter overlay test fixtures)

- [ ] **Step 1: Change `FilterEdit` field types**

```rust
pub(crate) struct FilterEdit {
    pub field: FilterField,
    pub tags_buf: crate::input::InputBuffer,
    pub lang_buf: crate::input::InputBuffer,
}
```

Update `from_filter` and `commit_into` to use `as_str()` / `push_str(...)`. The `tags_buf.split(',')` etc. operations work on the `&str` view.

- [ ] **Step 2: Update the key handler in `handle_filter_edit_key`**

Find the function (somewhere in `library.rs`) — replace `tags_buf.push(c)` / `tags_buf.pop()` with `push_char` / `pop_char`. Same for `lang_buf`.

- [ ] **Step 3: Update `render_filter_overlay` to place the cursor on the focused field**

Add `f.set_cursor_position` based on which field is focused (`FilterField::Tags` vs `Lang`) and the buffer's `cursor_col()`. Coordinates: `inner.x + label_width + cursor_col` for the row matching the focused field.

- [ ] **Step 4: Update `crates/kebab-tui/tests/library.rs` overlay test**

The existing `handle_key_library_f_opens_filter_overlay_then_enter_refreshes` test should still pass. Add a Hangul typing pin:

```rust
/// p9-fb-10: filter overlay accepts Hangul tags without panic.
#[test]
fn filter_overlay_accepts_hangul_tags() {
    let mut app = app_with_docs(vec![make_doc("a.md", "A", vec![])]);
    kebab_tui::handle_key_library(
        &mut app,
        KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE),
    );
    for ch in "한글".chars() {
        kebab_tui::handle_key_library(
            &mut app,
            KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE),
        );
    }
    // Press Enter to commit. Filter should commit "한글" as a tag.
    let outcome = kebab_tui::handle_key_library(
        &mut app,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    );
    assert_eq!(outcome, kebab_tui::KeyOutcome::Refresh);
    // Verify the filter actually has the Hangul tag.
    // (Use a getter — `App.library_filter_for_testing()` if it exists,
    // otherwise add a `pub fn` test seam.)
}
```

- [ ] **Step 5: `cargo test -p kebab-tui` + `cargo clippy`**

- [ ] **Step 6: Commit**

```bash
git add crates/kebab-tui/src/library.rs crates/kebab-tui/tests/library.rs
git commit -m "feat(kebab-tui): FilterEdit buffers → InputBuffer"
```

---

## Task 5: Korean query → SQLite FTS5 smoke pin

**Files:**
- Create: `crates/kebab-tui/tests/search_korean.rs` (or extend an existing kebab-app integration test if one already exercises the search facade with a temp config)

- [ ] **Step 1: Locate existing search facade test setup**

```bash
grep -rn "search_with_config\|SearchOptions" crates/kebab-app/tests/ crates/kebab-search/tests/ 2>/dev/null | head
```

If a TempDir + Config + ingest + search fixture exists, reuse it. Otherwise this task gets a tiny self-contained binary using `tempfile::TempDir` + `Config::defaults()` adjusted to the temp dir, an ingest of a Hangul markdown file, and a `kebab_app::search_with_config` call asserting at least one hit comes back.

- [ ] **Step 2: Write the test**

The contract: a Korean query token should hit a Korean document in lexical / hybrid mode. NFC normalization is already applied upstream (see `kebab-normalize`), so this test just verifies that the pipeline is wired end-to-end.

```rust
// crates/kebab-tui/tests/search_korean.rs
//
// p9-fb-10: smoke pin that a Korean query reaches FTS5 and returns
// the matching Hangul document. NFC normalization is upstream; this
// test just exercises the end-to-end facade.

use kebab_app::{ingest_with_config, search_with_config};
use kebab_config::Config;
use kebab_core::SearchMode;
use tempfile::TempDir;

#[test]
fn korean_lexical_query_returns_korean_document() {
    let temp = TempDir::new().unwrap();
    let workspace = temp.path().join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
    let doc_path = workspace.join("러스트-비동기.md");
    std::fs::write(
        &doc_path,
        "# 러스트 비동기 프로그래밍\n\n토큰: 러스트, 비동기, async, await\n",
    )
    .unwrap();

    let mut config = Config::defaults();
    config.workspace.root = workspace.to_string_lossy().into_owned();
    config.storage.data_dir = temp.path().join("data").to_string_lossy().into_owned();
    // Disable any model that the test environment may not have.
    // (If embedding is required for ingest, gate the assertion to
    // `SearchMode::Lexical` only — vector mode requires fastembed.)

    ingest_with_config(&config).expect("ingest must succeed");

    let hits = search_with_config(
        &config,
        "러스트",
        SearchMode::Lexical,
        Default::default(),
    )
    .expect("search must succeed");

    assert!(
        !hits.is_empty(),
        "expected at least one hit for Korean lexical query"
    );
}
```

If the facade signatures differ, adapt the call shapes — the goal is a single integration test that goes ingest → lexical search → assert-non-empty.

- [ ] **Step 3: Run**

```bash
cargo test -p kebab-tui --test search_korean
```

Expected: PASS. If embedding setup is required for ingest, gate appropriately or move the test under `#[ignore]` with a note explaining the environment dependency, then mark the checklist item Done with a "requires KEBAB_EMBED_TEST=1" caveat in HOTFIXES.

- [ ] **Step 4: Commit**

```bash
git add crates/kebab-tui/tests/search_korean.rs
git commit -m "test(kebab-tui): Korean query → FTS5 smoke pin"
```

---

## Task 6: Doc updates + spec status flip

**Files:**
- Modify: `tasks/HOTFIXES.md` (mark 5 checkboxes done, add a "follow-up shipped" note)
- Modify: `tasks/p9/p9-fb-10-tui-cjk-input.md` (`status: in_progress` → `completed`, tick DoD boxes)
- Modify: `README.md` (add "cursor column-aligned in CJK input" line, plus mention the Korean FTS5 pin if visible)
- Modify: `HANDOFF.md` (add a 2026-05-03 entry summarising the InputBuffer follow-up)
- Modify: `tasks/INDEX.md` (if it tracks per-component status — verify whether p9-fb-10 row needs a flip)

- [ ] **Step 1: HOTFIXES.md — mark all 5 checkboxes `[x]`**

Open `tasks/HOTFIXES.md`, find the "p9-fb-10 partial" entry, flip:
```markdown
- [x] `kebab-tui::input::InputBuffer { content: String, cursor_col: usize }` struct
- [x] Ask / Search / Editor pane 의 String + cursor 를 InputBuffer 로 교체
- [x] cursor render 가 wide-char 위에서 column 단위로 정렬 (현재 char-count 기반)
- [x] 한글 query → SQLite FTS5 검색 fixture 추가 (이미 NFC 정규화 됨, 단순 smoke pin)
- [x] DoD 체크박스 3 개 모두 채우고 spec status `in_progress` → `completed`
```

Add a sentence above or below: `**Follow-up shipped 2026-05-03 in PR #?? — cursor column-aligned across all text inputs, Korean FTS5 smoke pin lands.**`

- [ ] **Step 2: spec status flip**

```diff
-status: in_progress
+status: completed
```

In the same spec, tick each DoD box:
```markdown
- [x] `cargo test -p kebab-tui` 통과
- [x] 한글 fixture 추가
- [x] README — CJK 입력 동작 정상 명시
```

Add a `## Notes` line for the follow-up: `- 2026-05-03 follow-up: InputBuffer struct landed; Search/Ask/FilterEdit migrated; cursor column-aligned; Korean FTS5 smoke pin in tests/search_korean.rs.`

- [ ] **Step 3: README.md — add cursor + FTS5 line to the `kebab tui` row**

Append: `Search/Ask/Filter 입력의 cursor 가 wide char 위에서 column 단위로 정렬 — 한글 입력 시 caret 이 글자 옆에 정확히 놓임. Korean lexical 검색은 FTS5 끝까지 통합 테스트로 회귀 핀.`

- [ ] **Step 4: HANDOFF.md — add a 2026-05-03 entry**

Below the existing p9-fb-10 partial entry:
```markdown
- **2026-05-03 P9 도그푸딩 후속 (p9-fb-10 follow-up)** — InputBuffer struct + 모든 text-input pane 마이그레이션. `kebab-tui::InputBuffer { content, cursor_col }` 신규 — push_char / pop_char / clear / take 가 wide-char 단위로 cursor_col 을 진행. SearchState.input / AskState.input / FilterEdit.{tags_buf,lang_buf} 가 InputBuffer 로 교체. render 단계에서 `f.set_cursor_position(...)` 가 prompt 폭 + cursor_col 기반으로 caret 을 정확한 column 에 배치. Korean lexical 검색은 `tests/search_korean.rs` 에서 ingest → search → 결과 한 건 이상 assert 로 회귀 핀. spec status `in_progress` → `completed`.
```

- [ ] **Step 5: Verify all tests still green**

```bash
cargo test -p kebab-tui
cargo clippy --workspace --all-targets -- -D warnings
```

- [ ] **Step 6: Commit**

```bash
git add tasks/HOTFIXES.md tasks/p9/p9-fb-10-tui-cjk-input.md README.md HANDOFF.md tasks/INDEX.md
git commit -m "docs(p9-fb-10): InputBuffer follow-up — spec completed + HOTFIXES checklist done"
```

---

## Out of plan

- **Showing the terminal cursor** — the run loop calls `Hide` on startup. Whether to flip to `Show` while in INSERT mode is a separate UX call. For this PR, `f.set_cursor_position(...)` is invoked unconditionally; if the terminal cursor is hidden, ratatui still tracks the position internally and the cursor reappears whenever a prompt makes it visible. If the user needs a visible caret, that lands as a follow-up entry in HOTFIXES.
- **Mid-string editing** (arrow keys, insert at cursor) — append-only is sufficient for v1 per the spec. Adding cursor movement would relax the `cursor_col == display_width(content)` invariant and is non-trivial; deferred.
- **Surrogate-pair emoji** — covered by `unicode-width`'s best-effort sum; not pinned with extra tests.
