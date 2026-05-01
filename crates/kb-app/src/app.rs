//! `App` — internal lifecycle struct (§7).
//!
//! A single `App` represents one CLI invocation's worth of state: a
//! resolved `Config`, an open `SqliteStore`, and (when embeddings are
//! enabled) an `Embedder` + `LanceVectorStore`. Each public free
//! function on `kb-app` wraps `App::open(config)` once, runs the
//! requested op, and drops everything on return.
//!
//! The struct is `pub(crate)` because it is an internal seam: `kb-cli`
//! calls only the free functions on the crate root. `kb-tui` (P9) is
//! expected to hold one `App` for the session, at which point the
//! struct may need to be promoted to `pub`. Until then, keep it
//! private to insulate the wiring shape from downstream callers.
//!
//! ## Embedder + Vector store lifetime
//!
//! `App::open` builds the SQLite store unconditionally. The embedder
//! and vector store are *lazy + memoized* — built on first call to
//! [`App::embedder`] / [`App::vector`] and cached in `OnceLock`s — so
//! a long-lived `App` (e.g., the P9 TUI session) pays the ~470 MB
//! ONNX init plus Lance reopen cost exactly once.
//!
//! - `kb list` / `kb inspect` never need them.
//! - `kb search --mode lexical` never needs them.
//! - `kb ingest` and `kb search --mode {vector,hybrid}` always do.
//!
//! Building eagerly would force every CLI invocation to load ~470 MB of
//! ONNX weights, which is the dominant cold-start cost. The lazy
//! pattern keeps the lexical-only paths instant; the memoization makes
//! the TUI's repeated searches cheap after the first.
//!
//! Embeddings can also be **disabled** workspace-wide via
//! `config.models.embedding.provider = "none"` (or `dimensions = 0`);
//! in that mode [`App::embedder`] returns `None` and callers must fall
//! back to lexical-only search.

use std::sync::{Arc, OnceLock};

use anyhow::{Context, Result};

use kb_core::Embedder;
use kb_embed_local::FastembedEmbedder;
use kb_store_sqlite::SqliteStore;
use kb_store_vector::LanceVectorStore;

/// Internal facade state. See module docs for lifetime rules.
pub(crate) struct App {
    pub(crate) config: kb_config::Config,
    pub(crate) sqlite: Arc<SqliteStore>,
    /// Memoized embedder — built lazily on first `embedder()` call when
    /// embeddings are enabled. `OnceLock` keeps the struct `Sync` and
    /// the build path cold-only-once.
    embedder: OnceLock<Arc<dyn Embedder + Send + Sync>>,
    /// Memoized vector store — built lazily on first `vector()` call
    /// when embeddings are enabled. Same rationale as `embedder`.
    vector: OnceLock<Arc<LanceVectorStore>>,
}

impl App {
    /// Open the SQLite store and run migrations. Does NOT load the
    /// embedder or vector store — those are lazy via
    /// [`Self::embedder`] / [`Self::vector`].
    ///
    /// **Caveat:** must be called from a synchronous context.
    /// Downstream `LanceVectorStore::new` (called by [`Self::vector`])
    /// internally drives a `tokio::Runtime::block_on`, which panics if
    /// invoked from inside another tokio runtime.
    pub(crate) fn open(config: kb_config::Config) -> Result<Self> {
        let sqlite = SqliteStore::open(&config).context("kb-app: open SqliteStore")?;
        sqlite
            .run_migrations()
            .context("kb-app: run SqliteStore migrations")?;
        Ok(Self {
            config,
            sqlite: Arc::new(sqlite),
            embedder: OnceLock::new(),
            vector: OnceLock::new(),
        })
    }

    /// Returns `true` when the workspace has embeddings turned off
    /// (`provider = "none"` or `dimensions = 0`). Lexical-only mode.
    pub(crate) fn embeddings_disabled(&self) -> bool {
        let cfg = &self.config.models.embedding;
        cfg.provider == "none" || cfg.dimensions == 0
    }

    /// Build (or reuse) the fastembed embedder. Returns `None` when the
    /// workspace is in lexical-only mode (see
    /// [`Self::embeddings_disabled`]). The first call pays the ~470 MB
    /// ONNX load; subsequent calls are a single `OnceLock` read.
    pub(crate) fn embedder(&self) -> Result<Option<Arc<dyn Embedder + Send + Sync>>> {
        if self.embeddings_disabled() {
            return Ok(None);
        }
        if let Some(e) = self.embedder.get() {
            return Ok(Some(e.clone()));
        }
        let emb: Arc<dyn Embedder + Send + Sync> = Arc::new(
            FastembedEmbedder::new(&self.config)
                .context("kb-app: load FastembedEmbedder")?,
        );
        // `set` returns Err if another thread won the race; in that case
        // the loser still returns the (now-cached) winner via `get()`.
        let _ = self.embedder.set(emb.clone());
        Ok(Some(self.embedder.get().cloned().unwrap_or(emb)))
    }

    /// Build (or reuse) the LanceDB-backed vector store. Returns `None`
    /// when embeddings are disabled. Memoized via `OnceLock` for the
    /// same reasons as [`Self::embedder`].
    pub(crate) fn vector(&self) -> Result<Option<Arc<LanceVectorStore>>> {
        if self.embeddings_disabled() {
            return Ok(None);
        }
        if let Some(v) = self.vector.get() {
            return Ok(Some(v.clone()));
        }
        let store = Arc::new(
            LanceVectorStore::new(&self.config, self.sqlite.clone())
                .context("kb-app: open LanceVectorStore")?,
        );
        let _ = self.vector.set(store.clone());
        Ok(Some(self.vector.get().cloned().unwrap_or(store)))
    }
}
