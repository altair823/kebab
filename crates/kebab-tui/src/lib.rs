//! `kebab-tui` — Ratatui shell + Library pane (P9-1).
//!
//! Per design §8 module boundary: UI crates may only touch the
//! `kebab-app` facade. The store / search / embed / llm / rag layers
//! stay invisible behind it. P9-1 establishes the shell (App loop,
//! key dispatch, error popup, raw-mode panic guard) plus the Library
//! pane. P9-2/3/4 plug into the same `App` struct via the
//! `Option<*State>` slot pattern (parallel-safety: their sub-state
//! types start as `pub struct *State;` opaque forward declarations
//! and only their authoring crate fills the body).
//!
//! Per report §16.2 (TUI epic), design §1 (UX scenes), design §3.7
//! (`SearchHit` / `DocSummary`).

mod app;
mod ask;
mod cheatsheet;
mod editor;
mod error_popup;
mod ingest_progress;
mod input;
mod inspect;
mod library;
mod markdown;
mod run;
mod search;
mod terminal;
mod theme;

pub use input::{InputBuffer, display_width, place_cursor_x, truncate_to_display_width};
pub use theme::{Palette, Role, Theme};
pub use app::{
    App, AskState, IngestState, InspectState, InspectTarget, KeyOutcome, LibraryState, Mode,
    Pane, SearchState, SearchWorkerMessage, TERMINAL_LINE_HOLD_SECS,
};
pub use ask::{handle_key_ask, render_ask};
pub use error_popup::{ErrorOverlay, render_error_overlay};
pub use ingest_progress::{
    cancel_running_ingest, drain_progress, ready_to_clear, start_ingest, status_line,
};
pub use inspect::{enter_inspect, handle_key_inspect, render_inspect};
pub use library::{handle_key_library, render_library};
// `editor::with_external_program` and `search::jump_to_citation`
// stay `pub(crate)` — they take the internal `TuiTerminal` handle,
// which is intentionally module-private (its `Drop` lifecycle is the
// only safe constructor path for raw mode + alt-screen). External
// callers stage editor spawns via `App.pending_editor` instead.
pub use search::{build_jump_command, handle_key_search, render_search};
// p9-fb-08: expose `poll_worker` + `debounce_due` so integration
// tests can drive the stale-result drop / fresh-result apply paths
// without spawning the real thread (they inject a
// `SearchWorkerMessage` directly via a channel they construct in
// the test) and can pin the in-flight-skip invariant of debounce.
pub use search::poll_worker as poll_search_worker;
pub use search::debounce_due as search_debounce_due;
// p9-fb-12: expose the global mode-toggle intercept so integration
// tests can pin the i/Esc behavior without standing up the full
// run loop.
pub use run::mode_intercept;
// p9-fb-13: expose the cheatsheet-toggle intercept + render fn
// for integration tests + future TUI consumers.
pub use cheatsheet::render_cheatsheet;
pub use run::cheatsheet_intercept;
// p9-fb-13 follow-up: expose footer_hints so integration tests can
// pin the verb-form per (pane, mode) without standing up the run loop.
pub use run::footer_hints;
