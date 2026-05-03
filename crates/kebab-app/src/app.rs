//! `App` — facade lifecycle struct (§7).
//!
//! A single `App` represents one CLI invocation's (or one TUI
//! session's / one eval-runner suite's) worth of state: a resolved
//! `Config`, an open `SqliteStore`, and (when embeddings are enabled)
//! an `Embedder` + `LanceVectorStore`. Each public free function on
//! `kb-app` builds an `App` once, runs the requested op, and drops
//! everything on return; long-lived callers (kb-eval, the future P9
//! TUI session) hold onto an `App` across many calls so the per-query
//! cost is just a method dispatch.
//!
//! ## Embedder + Vector store lifetime
//!
//! `App::open_with_config` builds the SQLite store unconditionally.
//! The embedder and vector store are *lazy + memoized* — built on
//! first call to [`App::embedder`] / [`App::vector`] and cached in
//! `OnceLock`s — so a long-lived `App` (kb-eval driving 50 queries,
//! the P9 TUI session) pays the ~470 MB ONNX init plus Lance reopen
//! cost exactly once.
//!
//! - `kb list` / `kb inspect` never need them.
//! - `kb search --mode lexical` never needs them.
//! - `kb ingest` and `kb search --mode {vector,hybrid}` always do.
//!
//! Building eagerly would force every CLI invocation to load ~470 MB of
//! ONNX weights, which is the dominant cold-start cost. The lazy
//! pattern keeps the lexical-only paths instant; the memoization makes
//! the TUI's repeated searches and the eval runner's per-query loop
//! cheap after the first invocation.
//!
//! Embeddings can also be **disabled** workspace-wide via
//! `config.models.embedding.provider = "none"` (or `dimensions = 0`);
//! in that mode [`App::embedder`] returns `None` and callers must fall
//! back to lexical-only search.

use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{Context, Result, anyhow};
use lru::LruCache;

use kebab_core::{
    Answer, Embedder, IndexVersion, LanguageModel, Retriever, SearchHit, SearchMode,
    SearchQuery, VectorStore,
};
use kebab_embed_local::FastembedEmbedder;
use kebab_llm_local::OllamaLanguageModel;
use kebab_rag::{AskOpts, RagPipeline};
use kebab_search::{HybridRetriever, LexicalRetriever, VectorRetriever};
use kebab_store_sqlite::SqliteStore;
use kebab_store_vector::LanceVectorStore;

/// Facade state — see module docs for lifetime rules.
///
/// The struct is public so long-lived callers (kb-eval, the future P9
/// TUI session) can construct one and reuse it across many search /
/// ask calls. The OnceLock-backed `embedder` / `vector` fields ensure
/// the cold-start cost is paid exactly once per instance.
pub struct App {
    pub(crate) config: kebab_config::Config,
    pub(crate) sqlite: Arc<SqliteStore>,
    /// Memoized embedder — built lazily on first `embedder()` call when
    /// embeddings are enabled. `OnceLock` keeps the struct `Sync` and
    /// the build path cold-only-once.
    embedder: OnceLock<Arc<dyn Embedder + Send + Sync>>,
    /// Memoized vector store — built lazily on first `vector()` call
    /// when embeddings are enabled. Same rationale as `embedder`.
    vector: OnceLock<Arc<LanceVectorStore>>,
    /// Memoized LLM — built lazily on first `ask()` call. Sharing one
    /// across the eval runner avoids re-handshaking the Ollama HTTP
    /// client per query (cheap, but still measurable on a 50-query
    /// suite).
    llm: OnceLock<Arc<dyn LanguageModel>>,
    /// p9-fb-19: in-process LRU search-result cache. Capacity comes
    /// from `config.search.cache_capacity` (default 256, ~1.3 MB
    /// cap). `None` when capacity is 0 (cache disabled). The
    /// `corpus_revision` snapshot embedded in `SearchCacheKey`
    /// invalidates every entry the moment a new ingest commit lands.
    search_cache: Option<Mutex<LruCache<SearchCacheKey, Vec<SearchHit>>>>,
}

/// p9-fb-19: cache key for `App::search`. Includes every field that
/// could change the result set:
/// - normalized query (NFKC + trim + lowercase)
/// - mode + k + snippet_chars (caller knobs)
/// - embedding_version + chunker_version (model identity)
/// - corpus_revision (monotonic counter that ingest bumps)
///
/// Lexical mode has no embedding identity → empty string in that
/// slot, harmless because the rest of the key still distinguishes
/// queries.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct SearchCacheKey {
    pub query_norm: String,
    pub mode: SearchMode,
    pub k: u32,
    pub snippet_chars: u32,
    pub embedding_version: String,
    pub chunker_version: String,
    pub corpus_revision: u64,
}

impl SearchCacheKey {
    /// Normalize `query.text` per spec p9-fb-19: NFKC + trim +
    /// lowercase. Means `"Foo"` / `"FOO"` / `" foo "` collapse to a
    /// single cache entry — redundant work avoided when the user's
    /// input differs only in shape.
    pub fn normalize_query(text: &str) -> String {
        use unicode_normalization::UnicodeNormalization;
        text.trim().nfkc().collect::<String>().to_lowercase()
    }
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
    pub fn open_with_config(config: kebab_config::Config) -> Result<Self> {
        let sqlite = SqliteStore::open(&config).context("kb-app: open SqliteStore")?;
        sqlite
            .run_migrations()
            .context("kb-app: run SqliteStore migrations")?;
        // p9-fb-19: build the LRU cache from config. Capacity 0 →
        // `None` (cache disabled — every search hits the retrievers).
        let search_cache = NonZeroUsize::new(config.search.cache_capacity)
            .map(|cap| Mutex::new(LruCache::new(cap)));
        Ok(Self {
            config,
            sqlite: Arc::new(sqlite),
            embedder: OnceLock::new(),
            vector: OnceLock::new(),
            llm: OnceLock::new(),
            search_cache,
        })
    }

    /// Run a [`SearchQuery`] through the configured retriever stack and
    /// return the top-k hits. p9-fb-19: result is served from the
    /// in-process LRU cache when the same `(query_norm, mode, k,
    /// snippet_chars, embedding_version, chunker_version,
    /// corpus_revision)` tuple was seen before; cache miss falls
    /// through to [`Self::search_uncached`].
    ///
    /// Reuses any previously-built embedder / vector store on this `App`
    /// — long-lived callers (kb-eval, future TUI) get amortized cost
    /// across calls.
    pub fn search(&self, query: SearchQuery) -> Result<Vec<SearchHit>> {
        let Some(cache) = self.search_cache.as_ref() else {
            // Cache disabled (capacity = 0) — straight-line.
            return self.search_uncached(query);
        };
        // Build the cache key. embedding_version is empty for lexical
        // mode (no embedder identity); for vector/hybrid we need the
        // embedder built (which forces the cold-start cost), but
        // that's the cost the cache exists to amortize across
        // *subsequent* identical queries.
        let key = self.build_cache_key(&query)?;
        // Lock the cache long enough to lookup; clone the hit out so
        // we can drop the lock before returning.
        if let Ok(mut guard) = cache.lock() {
            if let Some(hits) = guard.get(&key) {
                tracing::debug!(
                    target: "kebab-app",
                    cache = "hit",
                    corpus_revision = key.corpus_revision,
                    "search served from LRU cache"
                );
                return Ok(hits.clone());
            }
        }
        let hits = self.search_uncached(query)?;
        if let Ok(mut guard) = cache.lock() {
            guard.put(key, hits.clone());
        }
        Ok(hits)
    }

    /// p9-fb-19: bypass the LRU cache and run the search directly.
    /// Used by `--no-cache` CLI invocations and by `search` itself
    /// on cache miss. Identical behavior to the pre-fb-19 `search`.
    pub fn search_uncached(&self, query: SearchQuery) -> Result<Vec<SearchHit>> {
        match query.mode {
            SearchMode::Lexical => {
                let lex = LexicalRetriever::with_settings(
                    self.sqlite.clone(),
                    lexical_index_version(&self.config),
                    self.config.search.snippet_chars,
                );
                lex.search(&query)
            }
            SearchMode::Vector => {
                let (emb, vec_store) = self.require_embeddings()?;
                let vec_iv = vector_index_version(emb.as_ref());
                let vec_dyn: Arc<dyn VectorStore + Send + Sync> = vec_store;
                let emb_dyn: Arc<dyn Embedder> = emb;
                let retr = VectorRetriever::with_settings(
                    vec_dyn,
                    emb_dyn,
                    self.sqlite.clone(),
                    vec_iv,
                    self.config.search.snippet_chars,
                );
                retr.search(&query)
            }
            SearchMode::Hybrid => {
                let lex = Arc::new(LexicalRetriever::with_settings(
                    self.sqlite.clone(),
                    lexical_index_version(&self.config),
                    self.config.search.snippet_chars,
                )) as Arc<dyn Retriever>;
                let (emb, vec_store) = self.require_embeddings()?;
                let vec_iv = vector_index_version(emb.as_ref());
                let vec_dyn: Arc<dyn VectorStore + Send + Sync> = vec_store;
                let emb_dyn: Arc<dyn Embedder> = emb;
                let vec_retr = Arc::new(VectorRetriever::with_settings(
                    vec_dyn,
                    emb_dyn,
                    self.sqlite.clone(),
                    vec_iv,
                    self.config.search.snippet_chars,
                )) as Arc<dyn Retriever>;
                let hybrid = HybridRetriever::new(&self.config, lex, vec_retr);
                hybrid.search(&query)
            }
        }
    }

    /// Run a RAG `ask` against the configured retriever + LLM. Reuses
    /// the memoized embedder / vector / LLM where applicable.
    pub fn ask(&self, query: &str, opts: AskOpts) -> Result<Answer> {
        let retriever: Arc<dyn Retriever> = match opts.mode {
            SearchMode::Lexical => Arc::new(LexicalRetriever::with_settings(
                self.sqlite.clone(),
                lexical_index_version(&self.config),
                self.config.search.snippet_chars,
            )),
            SearchMode::Vector => {
                let (emb, vec_store) = self.require_embeddings()?;
                let vec_iv = vector_index_version(emb.as_ref());
                let vec_dyn: Arc<dyn VectorStore + Send + Sync> = vec_store;
                let emb_dyn: Arc<dyn Embedder> = emb;
                Arc::new(VectorRetriever::with_settings(
                    vec_dyn,
                    emb_dyn,
                    self.sqlite.clone(),
                    vec_iv,
                    self.config.search.snippet_chars,
                ))
            }
            SearchMode::Hybrid => {
                let lex = Arc::new(LexicalRetriever::with_settings(
                    self.sqlite.clone(),
                    lexical_index_version(&self.config),
                    self.config.search.snippet_chars,
                )) as Arc<dyn Retriever>;
                let (emb, vec_store) = self.require_embeddings()?;
                let vec_iv = vector_index_version(emb.as_ref());
                let vec_dyn: Arc<dyn VectorStore + Send + Sync> = vec_store;
                let emb_dyn: Arc<dyn Embedder> = emb;
                let vec_retr = Arc::new(VectorRetriever::with_settings(
                    vec_dyn,
                    emb_dyn,
                    self.sqlite.clone(),
                    vec_iv,
                    self.config.search.snippet_chars,
                )) as Arc<dyn Retriever>;
                Arc::new(HybridRetriever::new(&self.config, lex, vec_retr))
            }
        };

        let llm = self.llm()?;
        let pipeline =
            RagPipeline::new(self.config.clone(), retriever, llm, self.sqlite.clone());
        pipeline.ask(query, opts)
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

    /// Build (or reuse) the configured LLM. Currently always Ollama;
    /// when a second provider lands this is the place to switch on
    /// `config.models.llm.provider`.
    fn llm(&self) -> Result<Arc<dyn LanguageModel>> {
        if let Some(l) = self.llm.get() {
            return Ok(l.clone());
        }
        let llm: Arc<dyn LanguageModel> = Arc::new(
            OllamaLanguageModel::new(&self.config)
                .context("kb-app::ask: build OllamaLanguageModel")?,
        );
        let _ = self.llm.set(llm.clone());
        Ok(self.llm.get().cloned().unwrap_or(llm))
    }

    /// p9-fb-19: build a `SearchCacheKey` for `query`. For lexical
    /// mode the embedding_version slot is left empty (no embedder
    /// identity contributes to the result). For vector / hybrid
    /// modes the embedder is built (cold-start) so the version
    /// label can be read; that's the cost the cache exists to
    /// amortize over the next few identical queries.
    fn build_cache_key(&self, query: &SearchQuery) -> Result<SearchCacheKey> {
        let embedding_version = match query.mode {
            SearchMode::Lexical => String::new(),
            SearchMode::Vector | SearchMode::Hybrid => {
                let emb = self.embedder()?.ok_or_else(|| {
                    anyhow!(
                        "embeddings disabled; vector / hybrid search require an \
                         embedder — switch to --mode lexical or enable a provider"
                    )
                })?;
                vector_index_version(emb.as_ref()).0
            }
        };
        Ok(SearchCacheKey {
            query_norm: SearchCacheKey::normalize_query(&query.text),
            mode: query.mode,
            k: u32::try_from(query.k).unwrap_or(u32::MAX),
            snippet_chars: u32::try_from(self.config.search.snippet_chars).unwrap_or(u32::MAX),
            embedding_version,
            chunker_version: self.config.chunking.chunker_version.clone(),
            corpus_revision: self.sqlite.corpus_revision(),
        })
    }

    /// p9-fb-19: clear the in-process search cache. Useful for tests
    /// and for explicit user actions (e.g. a future `kebab cache
    /// clear` admin command). No-op when the cache is disabled.
    pub fn clear_search_cache(&self) {
        if let Some(cache) = self.search_cache.as_ref() {
            if let Ok(mut guard) = cache.lock() {
                guard.clear();
            }
        }
    }

    /// Resolve the embedder + vector store, surfacing the user-friendly
    /// "switch to --mode lexical" error when embeddings are disabled.
    fn require_embeddings(
        &self,
    ) -> Result<(
        Arc<dyn Embedder + Send + Sync>,
        Arc<LanceVectorStore>,
    )> {
        let emb = self.embedder()?.ok_or_else(|| {
            anyhow!(
                "embeddings disabled (config.models.embedding.provider == \"none\" \
                 or dimensions == 0); vector / hybrid search require embeddings — \
                 switch to --mode lexical or enable an embedding provider in config.toml"
            )
        })?;
        let vec_store = self.vector()?.ok_or_else(|| {
            anyhow!(
                "vector store unavailable while embedder is configured — this should \
                 not happen; check `kb doctor` and the data_dir permissions"
            )
        })?;
        Ok((emb, vec_store))
    }
}

/// Compose a stable `IndexVersion` for the lexical retriever from
/// the active config. This token surfaces in `SearchHit.index_version`
/// and on snapshot tests; including the chunker version pins it to
/// the chunking policy in effect.
fn lexical_index_version(config: &kebab_config::Config) -> IndexVersion {
    IndexVersion(format!("lex:{}", config.chunking.chunker_version))
}

/// Compose a stable `IndexVersion` for the vector retriever. Tracks
/// `(embedding_model, embedding_version, dimensions)` so a model swap
/// flags drift via the existing index_version mismatch warning in
/// `HybridRetriever::new`.
fn vector_index_version(embedder: &dyn Embedder) -> IndexVersion {
    IndexVersion(format!(
        "vec:{}@{}:{}",
        embedder.model_id().0,
        embedder.model_version().0,
        embedder.dimensions(),
    ))
}
