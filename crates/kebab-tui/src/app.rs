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

/// Ask pane state — owned by p9-3, extended by p9-fb-16 for
/// multi-turn conversation transcript.
///
/// The worker thread (`thread`) owns the `mpsc::Sender<String>` that
/// `kebab-app::ask` writes tokens into. The pane keeps the matching
/// `rx` and drains it once per render frame (no blocking).
///
/// p9-fb-16: completed `Turn`s accumulate in `turns`; the worker
/// passes a snapshot of `turns` as `history` to
/// `RagPipeline::ask_with_history`, so each follow-up question sees
/// the full prior conversation. `conversation_id` is auto-generated
/// on the first submission (timestamp-based — unique per session,
/// not cryptographic). `Ctrl-L` clears `turns + conversation_id` to
/// start a fresh conversation.
#[derive(Default)]
pub struct AskState {
    pub input: String,
    /// Toggled by the `e` key. Re-applied on the next `Enter`.
    pub explain: bool,
    /// True between `Enter` press and worker thread completion.
    pub streaming: bool,
    /// Tokens accumulated from the worker so far. Cleared on each
    /// new submission. Mid-stream this is what the transcript shows
    /// for the in-flight turn.
    pub partial: String,
    /// In-flight worker; `take()`n when it finishes.
    pub thread: Option<std::thread::JoinHandle<anyhow::Result<kebab_core::Answer>>>,
    /// Token receiver paired with the worker's `Sender`. Drained
    /// every render frame.
    pub rx: Option<std::sync::mpsc::Receiver<String>>,
    /// Vertical scroll offset for the transcript area when content
    /// exceeds the viewport.
    pub scroll: u16,
    /// Last error from the worker thread (rendered in popup if Some).
    pub last_error: Option<String>,
    /// p9-fb-16: completed turns of the current conversation. Each
    /// turn = (question, full answer text, citations, ts). Streaming
    /// turn (the one being generated right now) lives in
    /// `current_question` + `partial` and only graduates into
    /// `turns` on `poll_worker` completion.
    pub turns: Vec<kebab_core::Turn>,
    /// p9-fb-16: question text for the in-flight turn. Cleared at
    /// submission (input → current_question, input → empty),
    /// finalized into the new Turn at completion.
    pub current_question: Option<String>,
    /// p9-fb-16: shared id stamped onto every `Answer` of this
    /// conversation. Auto-generated on first submission, cleared by
    /// `Ctrl-L` (next submission generates a fresh id).
    pub conversation_id: Option<String>,
    /// p9-fb-16: most-recent `Answer` for citation / status display
    /// in the right panel. Same data also lives inside the last
    /// `Turn`; this slot is just the easiest place for the panel
    /// renderer to look.
    pub last_answer: Option<kebab_core::Answer>,
}


/// What the Inspect pane is currently showing — owned by p9-4.
#[derive(Clone, Debug)]
pub enum InspectTarget {
    Doc(kebab_core::DocumentId),
    Chunk(kebab_core::ChunkId),
}

/// Inspect pane state — owned by p9-4.
///
/// Read-only view; data fetched on each target change via the
/// `kebab-app::inspect_*_with_config` facade (run-loop hook).
pub struct InspectState {
    pub target: Option<InspectTarget>,
    pub doc: Option<kebab_core::CanonicalDocument>,
    pub chunk: Option<kebab_core::Chunk>,
    /// Section names currently collapsed (e.g. "metadata", "provenance",
    /// "blocks", "embeddings"). Toggled by `c`.
    pub collapsed: std::collections::HashSet<&'static str>,
    pub scroll: u16,
    /// Pane the user came from — Library or Search. `Esc` returns
    /// here.
    pub return_to: Pane,
    /// True when `target` differs from the last fetched result; the
    /// run loop's idle tick services it.
    pub needs_fetch: bool,
    /// True while the inspect call is in flight (synchronous in v1).
    pub loading: bool,
}

impl Default for InspectState {
    fn default() -> Self {
        Self {
            target: None,
            doc: None,
            chunk: None,
            collapsed: std::collections::HashSet::new(),
            scroll: 0,
            return_to: Pane::Library,
            needs_fetch: false,
            loading: false,
        }
    }
}

/// Background-ingest state — owned by p9-fb-03 + extended by
/// p9-fb-04 (cancel).
///
/// The TUI lets the user fire `kebab ingest` from inside the shell
/// without blocking the event loop. Pressing `r` on the Library pane
/// spawns a worker thread that calls
/// `kebab_app::ingest_with_config_cancellable(.., Some(tx), Some(cancel))`;
/// the run loop drains `rx` once per frame and updates the visible
/// status bar. When the worker thread joins (Sender dropped →
/// `recv()` Err), the final aggregate counts stay on screen for a
/// few seconds and then the slot clears.
///
/// `cancel` is the same `Arc<AtomicBool>` the worker polls at each
/// step boundary. The `Esc` / `Ctrl-C` key (only while ingest is
/// in flight) flips it via `cancel.store(true, Ordering::Relaxed)`
/// — the worker breaks at its next iteration check, emits
/// `IngestEvent::Aborted { counts: <partial> }`, and joins.
pub struct IngestState {
    pub rx: std::sync::mpsc::Receiver<kebab_app::IngestEvent>,
    pub counts: kebab_app::AggregateCounts,
    pub current_path: Option<String>,
    pub current_idx: u32,
    pub started_at: std::time::Instant,
    /// `Some(_)` once a `Completed` or `Aborted` event has arrived;
    /// the run loop holds the final line on screen for
    /// `TERMINAL_LINE_HOLD_SECS` seconds and then clears the slot.
    pub terminal_at: Option<std::time::Instant>,
    /// True when the terminal event was `Aborted` (vs `Completed`).
    /// Used to colour the final line.
    pub aborted: bool,
    /// Worker thread handle. `take()`n at clear time so the join
    /// happens after the user has had time to read the final line.
    pub thread: Option<std::thread::JoinHandle<anyhow::Result<kebab_core::IngestReport>>>,
    /// p9-fb-04: shared cancel token. `Esc` / `Ctrl-C` flip it; the
    /// worker thread polls it at each asset-loop boundary.
    pub cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

/// Seconds the final ingest status line stays on screen after a run
/// completes / aborts. After this elapses the run loop clears
/// `App.ingest_state` so the footer returns to the standard hints.
pub const TERMINAL_LINE_HOLD_SECS: u64 = 3;

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
    /// Populated by p9-fb-03 when the user kicks off an in-shell
    /// ingest (Library `r`). Cleared by the run loop a few seconds
    /// after the run reaches a terminal event.
    pub ingest_state: Option<IngestState>,
    /// In-flight error overlay (popup); `Some` when the last facade
    /// call returned `Err` and the user has not dismissed yet.
    pub(crate) error_overlay: Option<ErrorOverlay>,
    /// Set by `handle_key_library` when the user presses `q` / `Esc`
    /// or by a future pane's quit key. The run loop drains this on
    /// each tick.
    pub(crate) should_quit: bool,
    /// p9-fb-09: deferred external-program request. A pane's key
    /// handler enqueues an `EditorRequest` here when the user wants
    /// to spawn `$EDITOR` (e.g. Search `g` jumps to a citation in
    /// vim) — the actual suspend / spawn / restore happens in the
    /// run loop, where the `TuiTerminal` handle is in scope.
    /// Drained every tick after the key dispatch.
    ///
    /// `pub(crate)` because the enqueue/take invariant ("set by a
    /// key handler, drained by the next run-loop tick") only holds
    /// for in-crate callers; external mutation could leave a stale
    /// request that never gets serviced.
    pub(crate) pending_editor: Option<EditorRequest>,
    /// p9-fb-09: when set, the next run-loop draw runs
    /// `terminal.clear()` first so any leftover screen content from
    /// a suspension (post-editor, future config-reload, …) is wiped
    /// before Ratatui's diff renders the new frame. Reset back to
    /// false after the clear. Independent of `pending_editor` —
    /// any future code path that needs a forced redraw can flip
    /// this flag.
    pub(crate) force_redraw: bool,
}

/// p9-fb-09: external-program spawn request. Posted by a pane's key
/// handler, serviced by the run loop on the next tick.
#[derive(Clone, Debug)]
pub struct EditorRequest {
    pub citation: kebab_core::Citation,
    pub editor_env: String,
    pub workspace_root: std::path::PathBuf,
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
            ingest_state: None,
            error_overlay: None,
            should_quit: false,
            pending_editor: None,
            force_redraw: false,
        })
    }

    /// Read-only accessor for the in-flight external-program request.
    /// Tests and future external observers (e.g. integration smokes)
    /// use this to assert that a key dispatch enqueued a spawn —
    /// mutating the slot stays `pub(crate)` to preserve the
    /// "set-then-drained-on-next-tick" invariant.
    pub fn pending_editor(&self) -> Option<&EditorRequest> {
        self.pending_editor.as_ref()
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
