//! `App` — TUI shell state, owned by p9-1.
//!
//! The struct's full set of fields is owned here; the layout reserves
//! one `Option<*State>` slot per pane so p9-2 / p9-3 / p9-4 can plug
//! their state in WITHOUT modifying the struct definition. p9-1 is the
//! only crate that ever changes `App`.

use kebab_config::Config;

use crate::error_popup::ErrorOverlay;
use crate::library::LibraryStateInner;

/// TUI panes (design §1 UX scenes).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Pane {
    Library,
    Search,
    Ask,
    Inspect,
    Jobs,
}

/// Outcome of a key handler — what the run loop should do next.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyOutcome {
    /// Stay on the current pane; re-render only.
    Continue,
    /// Quit the app (`q` / `Esc` from Library, or any pane's quit key).
    Quit,
    /// Switch focus to the named pane.
    SwitchPane(Pane),
    /// Re-run the pane's data fetch (e.g. Library after a filter edit).
    Refresh,
}

/// Library pane state — fully owned by p9-1.
pub struct LibraryState {
    pub(crate) inner: LibraryStateInner,
}

impl LibraryState {
    pub fn new() -> Self {
        Self {
            inner: LibraryStateInner::default(),
        }
    }
}

impl Default for LibraryState {
    fn default() -> Self {
        Self::new()
    }
}

/// Search pane state — owned by p9-2.
///
/// Field-set kept in `app.rs` (not in `search.rs`) so cross-module
/// access from `run.rs` (lazy-init, debounce tick) does not require
/// re-exporting field accessors. The pane behavior + render live in
/// `crate::search`.
pub struct SearchState {
    pub input: String,
    pub mode: kebab_core::SearchMode,
    pub hits: Vec<kebab_core::SearchHit>,
    pub selected_hit: usize,
    /// When the input last changed; the run loop debounces searches
    /// against this (200 ms after the last keystroke).
    pub input_dirty_at: Option<time::OffsetDateTime>,
    /// Snapshot of `(input, mode)` at the moment the last search
    /// fired. The debounce skips re-searches when nothing changed.
    pub last_query: Option<(String, kebab_core::SearchMode)>,
    /// True while a synchronous search call is in flight. The run
    /// loop uses this to overlay a "searching…" hint.
    pub searching: bool,
    /// Cached preview text for the currently-selected hit (lazily
    /// fetched via `kebab-app::inspect_chunk_with_config`).
    pub preview: Option<String>,
}

impl Default for SearchState {
    fn default() -> Self {
        Self {
            input: String::new(),
            mode: kebab_core::SearchMode::Hybrid,
            hits: Vec::new(),
            selected_hit: 0,
            input_dirty_at: None,
            last_query: None,
            searching: false,
            preview: None,
        }
    }
}

/// Ask pane state — owned by p9-3.
///
/// The worker thread (`thread`) owns the `mpsc::Sender<String>` that
/// `kebab-app::ask` writes tokens into. The pane keeps the matching
/// `rx` and drains it once per render frame (no blocking).
#[derive(Default)]
pub struct AskState {
    pub input: String,
    /// Toggled by the `e` key. Re-applied on the next `Enter`.
    pub explain: bool,
    /// True between `Enter` press and worker thread completion.
    pub streaming: bool,
    /// Tokens accumulated from the worker so far. Cleared on each
    /// new submission.
    pub partial: String,
    /// Final `Answer` once the worker thread finishes.
    pub answer: Option<kebab_core::Answer>,
    /// In-flight worker; `take()`n when it finishes.
    pub thread: Option<std::thread::JoinHandle<anyhow::Result<kebab_core::Answer>>>,
    /// Token receiver paired with the worker's `Sender`. Drained
    /// every render frame.
    pub rx: Option<std::sync::mpsc::Receiver<String>>,
    /// Vertical scroll offset for the answer area when content
    /// exceeds the viewport.
    pub scroll: u16,
    /// Last error from the worker thread (rendered in popup if Some).
    pub last_error: Option<String>,
}


/// Forward-declared opaque sub-state. p9-4 fills the body.
pub struct InspectState;

/// TUI application. The shell that p9-1 stands up; later p9-* tasks
/// add panes by populating their `Option<*State>` slot.
pub struct App {
    pub config: Config,
    pub focus: Pane,
    pub library: LibraryState,
    /// Populated by p9-2 (None until that crate links in).
    pub search: Option<SearchState>,
    /// Populated by p9-3.
    pub ask: Option<AskState>,
    /// Populated by p9-4.
    pub inspect: Option<InspectState>,
    /// In-flight error overlay (popup); `Some` when the last facade
    /// call returned `Err` and the user has not dismissed yet.
    pub(crate) error_overlay: Option<ErrorOverlay>,
    /// Set by `handle_key_library` when the user presses `q` / `Esc`
    /// or by a future pane's quit key. The run loop drains this on
    /// each tick.
    pub(crate) should_quit: bool,
}

impl App {
    /// Build an `App` against `config`. Does not load documents — the
    /// run loop calls `library.refresh` on first frame so a slow
    /// `kebab-app::list_docs_with_config` does not block startup.
    pub fn new(config: Config) -> anyhow::Result<Self> {
        Ok(Self {
            config,
            focus: Pane::Library,
            library: LibraryState::new(),
            search: None,
            ask: None,
            inspect: None,
            error_overlay: None,
            should_quit: false,
        })
    }

    /// Blocking event loop. Returns when the user quits or a fatal
    /// error escapes the loop (terminal raw-mode is restored either
    /// way via the `Terminal` Drop guard).
    pub fn run(&mut self) -> anyhow::Result<()> {
        crate::run::run_loop(self)
    }

    /// Test-only: hand-populate the Library pane with docs without
    /// going through `kebab-app::list_docs_with_config`. Snapshot /
    /// key-handler tests use this to drive a deterministic view
    /// instead of standing up a TempDir SQLite KB.
    ///
    /// Marked `#[doc(hidden)]` because it is a test seam, not part
    /// of the official UI API.
    #[doc(hidden)]
    pub fn populate_library_for_testing(
        &mut self,
        docs: Vec<kebab_core::DocSummary>,
    ) {
        self.library.inner.docs = docs;
        self.library.inner.needs_refresh = false;
        let len = self.library.inner.docs.len();
        if len == 0 {
            self.library.inner.list_state.select(None);
        } else {
            self.library.inner.list_state.select(Some(0));
        }
    }
}
