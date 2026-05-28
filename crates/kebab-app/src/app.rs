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
    Answer, DocumentStore, Embedder, ExtractContext, Extractor, IndexVersion, LanguageModel,
    MediaType, Retriever, SearchHit, SearchMode, SearchOpts, SearchQuery, VectorStore,
};
use kebab_embed_local::FastembedEmbedder;
use kebab_llm_local::OllamaLanguageModel;
use kebab_parse_code::{
    CAstExtractor, CppAstExtractor, GoAstExtractor, JavaAstExtractor, JavascriptAstExtractor,
    KotlinAstExtractor, PythonAstExtractor, RustAstExtractor, TypescriptAstExtractor,
};
use kebab_parse_image::ImageExtractor;
use kebab_parse_pdf::PdfTextExtractor;
use kebab_rag::{AskOpts, RagPipeline};
use kebab_search::{HybridRetriever, LexicalRetriever, VectorRetriever};
use kebab_store_sqlite::SqliteStore;
use kebab_store_vector::LanceVectorStore;

/// p9-fb-34: top-level wrapper around a paginated, budget-limited
/// search result. Mirrors the wire `search_response.v1` shape.
///
/// `next_cursor` is non-null whenever more hits may be reachable —
/// either the retriever filled the page (more behind it), or the
/// budget loop popped hits (those popped hits remain fetchable
/// from `offset + returned`). It is null only when the retriever
/// returned fewer hits than requested AND nothing was popped — i.e.
/// the corpus has nothing more for this query.
///
/// `truncated` is independent of `next_cursor`: it signals that
/// the budget loop modified the page (snippet shorten or k pop).
/// Caller may either widen `max_tokens` (and re-issue the same
/// query) or follow `next_cursor` (to advance through more hits)
/// or both.
#[derive(Clone, Debug)]
pub struct SearchResponse {
    pub hits: Vec<SearchHit>,
    pub next_cursor: Option<String>,
    pub truncated: bool,
    /// p9-fb-37: present when caller passed `SearchOpts.trace = true`.
    /// Consumers that ignore trace should leave this `None`.
    pub trace: Option<kebab_core::SearchTrace>,
    /// v0.17.0 A5 Step 4b: human / agent-readable advisory string set
    /// when the empty hit list is likely due to a query shorter than the
    /// FTS5 trigram tokenizer's 3-char minimum. `None` otherwise. CLI
    /// surfaces it on stderr (text mode); MCP / `--json` consumers
    /// surface it however they prefer. See
    /// `docs/superpowers/specs/2026-05-22-korean-trigram-tokenizer-design.md`
    /// §3.3.
    pub hint: Option<String>,
}

/// v0.17.0 A5 Step 4b: decide whether to attach a "3자 이상 키워드 권장"
/// hint to a `SearchResponse`. Fires only when the result set is empty
/// *and* the trimmed query is shorter than the trigram tokenizer can
/// resolve. Raw FTS5 mode (`'...'`) opts out — the user explicitly
/// invoked FTS5 syntax. Identical condition powers the CLI stderr line
/// and (separately) the TUI status bar.
pub fn short_query_hint(query_text: &str, hits_empty: bool) -> Option<String> {
    if !hits_empty {
        return None;
    }
    let trimmed = query_text.trim();
    let bytes = trimmed.as_bytes();
    // Raw single-quote mode: user opted into FTS5 syntax, no advisory.
    if bytes.len() >= 2 && bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\'' {
        return None;
    }
    if trimmed.chars().count() < 3 {
        Some("3자 이상 키워드 권장 (trigram tokenizer 제약)".to_string())
    } else {
        None
    }
}

/// Facade state — see module docs for lifetime rules.
///
/// The struct is public so long-lived callers (kb-eval, the future P9
/// TUI session) can construct one and reuse it across many search /
/// ask calls. The OnceLock-backed `embedder` / `vector` fields ensure
/// the cold-start cost is paid exactly once per instance.
pub struct App {
    pub(crate) config: kebab_config::Config,
    pub(crate) sqlite: Arc<SqliteStore>,
    /// post-v0.18.0 extractor-dispatch-unification: polymorphic Extractor
    /// registry. App init 시 1회 등록되어 `extract_for(...)` 가 lookup
    /// 한다. 현재 11 entry (ImageExtractor + PdfTextExtractor + 9 AST).
    /// MarkdownExtractor 는 별 PR 에서 추가 — markdown ingest path 는
    /// 본 PR 에서 free-function 그대로 유지.
    pub(crate) extractors: Vec<Box<dyn Extractor + Send + Sync>>,
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
    /// p9-fb-41 PR-9c-2: NLI verifier built eagerly at
    /// `open_with_config` time when `config.rag.nli_threshold > 0`,
    /// consumed by `RagPipeline::with_verifier` on every `ask` /
    /// `ask_with_session` call. `None` when the gate is disabled
    /// (default, threshold = 0) — multi-hop skips step 8.5 entirely
    /// and single-pass never touches the verifier.
    ///
    /// Built eagerly (not lazy) so the `open_with_config` `?`
    /// propagation surfaces NLI model construction errors at App
    /// boot time, before any user query runs.
    pipeline_verifier: Option<Arc<dyn kebab_nli::NliVerifier>>,
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
///
/// **Naming note**: spec p9-fb-19 calls the invalidation counter
/// `index_version`, but the impl renames it to `corpus_revision` to
/// avoid confusion with the pre-existing `IndexVersion` newtype
/// (design §9 — embedding-index identity label, a completely
/// different concept). The `corpus_revision` row in the §9
/// versioning table documents the new dimension; HOTFIXES entry
/// tracks the rename.
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
        // post-v0.18.0 extractor-dispatch-unification: build the 11-entry
        // Extractor registry. All entries are state-less unit structs with
        // zero-cost `new()`, so init cost is effectively 0 and side effects
        // are 0 — `pipeline_verifier` fallible `?` below may bail but the
        // already-constructed `extractors` Vec drops without cost. Markdown
        // is NOT registered (see field doc).
        let extractors: Vec<Box<dyn Extractor + Send + Sync>> = vec![
            Box::new(ImageExtractor::new()),
            Box::new(PdfTextExtractor::new()),
            Box::new(RustAstExtractor::new()),
            Box::new(PythonAstExtractor::new()),
            Box::new(TypescriptAstExtractor::new()),
            Box::new(JavascriptAstExtractor::new()),
            Box::new(GoAstExtractor::new()),
            Box::new(JavaAstExtractor::new()),
            Box::new(KotlinAstExtractor::new()),
            Box::new(CAstExtractor::new()),
            Box::new(CppAstExtractor::new()),
        ];
        // p9-fb-41 PR-9c-2: build the NLI verifier when the gate is
        // enabled. App carries it on `RagPipeline` via
        // `with_verifier` so the rag crate doesn't have to know about
        // kebab-nli construction. Failure (`?`) surfaces as a user-
        // facing error at App boot — never a panic in the pipeline's
        // `expect("verifier must be Some when nli_threshold > 0.0")`.
        let pipeline_verifier: Option<Arc<dyn kebab_nli::NliVerifier>> = if config.rag.nli_threshold
            > 0.0
        {
            let v = kebab_nli::OnnxNliVerifier::new(&config)
                .context("kebab-app: construct OnnxNliVerifier (config.rag.nli_threshold > 0)")?;
            Some(Arc::new(v))
        } else {
            None
        };
        Ok(Self {
            config,
            sqlite: Arc::new(sqlite),
            extractors,
            embedder: OnceLock::new(),
            vector: OnceLock::new(),
            llm: OnceLock::new(),
            search_cache,
            pipeline_verifier,
        })
    }

    /// Polymorphic dispatcher for the [`Extractor`] trait. Looks up the
    /// first Extractor whose `supports(media)` returns true and invokes
    /// `extract(ctx, bytes)` on it.
    ///
    /// Errors with `anyhow!("no Extractor for media_type {media:?}")`
    /// when no matching Extractor is registered. Callers in
    /// `ingest_one_*_asset` reach this only after the outer 4-arm
    /// dispatch (`MediaType::Markdown` / `Image` / `Pdf` / `Code(lang)`)
    /// has matched, so a miss is a programming error — NOT a user-
    /// facing skip.
    pub(crate) fn extract_for(
        &self,
        media: &MediaType,
        ctx: &ExtractContext<'_>,
        bytes: &[u8],
    ) -> Result<kebab_core::CanonicalDocument> {
        let extractor = self
            .extractors
            .iter()
            .find(|e| e.supports(media))
            .ok_or_else(|| anyhow!("no Extractor for media_type {media:?}"))?;
        extractor.extract(ctx, bytes)
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
        // we can drop the lock before returning. Mutex poison
        // recovery: `into_inner()` of a poison error returns the
        // (still-valid) underlying guard so we can keep using the
        // cache after a panic in another thread. Log once so the
        // poison itself is visible — the cache is still functional
        // but a panic in a previous search is worth knowing about.
        let mut guard = cache.lock().unwrap_or_else(|e| {
            tracing::warn!(
                target: "kebab-app",
                "search_cache mutex was poisoned; recovering and continuing — \
                 a previous search-thread panic preceded this call"
            );
            e.into_inner()
        });
        if let Some(hits) = guard.get(&key) {
            tracing::debug!(
                target: "kebab-app",
                cache = "hit",
                corpus_revision = key.corpus_revision,
                "search served from LRU cache"
            );
            // p9-fb-32: re-stamp staleness on every cache hit. The cache
            // entry was stamped at insert time against an older `now`
            // and an older threshold; if either has shifted (config
            // reload, time passing) the cached `stale: false` may now
            // be wrong. Re-stamping is cheap (per-hit comparison) and
            // avoids invalidating the cache on threshold changes.
            let mut hits = hits.clone();
            drop(guard);
            let now = time::OffsetDateTime::now_utc();
            crate::staleness::mark_stale_in_place(
                &mut hits,
                now,
                self.config.search.stale_threshold_days,
            );
            return Ok(hits);
        }
        // Drop the lock before the (potentially slow) retriever call
        // so other in-flight searches can use the cache concurrently.
        drop(guard);
        let hits = self.search_uncached(query)?;
        let mut guard = cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.put(key, hits.clone());
        Ok(hits)
    }

    /// p9-fb-19: bypass the LRU cache and run the search directly.
    /// Used by `--no-cache` CLI invocations and by `search` itself
    /// on cache miss. Identical behavior to the pre-fb-19 `search`.
    pub fn search_uncached(&self, query: SearchQuery) -> Result<Vec<SearchHit>> {
        let mut hits = match query.mode {
            SearchMode::Lexical => {
                let lex = LexicalRetriever::with_settings(
                    self.sqlite.clone(),
                    lexical_index_version(&self.config),
                    self.config.search.snippet_chars,
                );
                lex.search(&query)?
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
                retr.search(&query)?
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
                hybrid.search(&query)?
            }
        };
        // p9-fb-32: stamp staleness against the freshest possible `now`
        // and the current threshold. Cheap (per-hit comparison).
        let now = time::OffsetDateTime::now_utc();
        crate::staleness::mark_stale_in_place(
            &mut hits,
            now,
            self.config.search.stale_threshold_days,
        );
        // p10-1A-2: backfill `code_lang` from the Citation::Code `lang`
        // field. The search layer (kebab-search) constructs SearchHit with
        // `code_lang: None`; we own the post-processing here in kebab-app
        // and can fill it cheaply from data already present in the hit.
        backfill_code_lang(&mut hits);
        // p10-1A-2 Task 8b: backfill `repo` from the document's
        // `Metadata.repo`. Unlike `code_lang`, this cannot be derived from
        // the Citation alone — it requires a store lookup by `doc_id`.
        self.backfill_repo(&mut hits);
        Ok(hits)
    }

    /// p9-fb-34: budget-aware search facade. Returns hits trimmed to
    /// `opts.max_tokens` (chars/4 approximation) plus pagination
    /// metadata. `App::search` is now a thin wrapper that drops the
    /// metadata for backwards compat.
    ///
    /// `SearchResponse.next_cursor` and `truncated` are independent
    /// signals — see `SearchResponse` doc for details.
    pub fn search_with_opts(&self, query: SearchQuery, opts: SearchOpts) -> Result<SearchResponse> {
        use crate::cursor;

        let corpus_revision = self.sqlite.corpus_revision().to_string();
        let offset = match opts.cursor.as_ref() {
            // p9-fb-34: wrap the typed ErrorV1 in StructuredError so
            // anyhow carries the structured payload all the way to
            // `classify` — string formatting here would degrade
            // `code = "stale_cursor"` to `code = "generic"` on the wire.
            Some(c) => cursor::decode(c, &corpus_revision)
                .map_err(|e| anyhow::Error::new(crate::error_wire::StructuredError(e)))?,
            None => 0,
        };

        let snippet_chars = opts
            .snippet_chars
            .unwrap_or(self.config.search.snippet_chars);

        // Fetch enough to satisfy offset + the requested page. The
        // retriever returns at most `fetch_k` hits — we then drop
        // `offset` and keep the next `k_effective`. `k = 0` is
        // treated as "use config default" so a caller passing through
        // a default-constructed `SearchQuery` still gets useful work
        // out of the budget facade.
        let k_effective = if query.k == 0 {
            self.config.search.default_k
        } else {
            query.k
        };
        let fetch_k = offset.saturating_add(k_effective);
        let fetch_query = SearchQuery {
            k: fetch_k,
            ..query.clone()
        };

        // p9-fb-37: when --trace is requested, bypass the LRU cache and
        // run through `HybridRetriever::search_with_trace`, which
        // dispatches by mode internally. Vector / hybrid modes require
        // embeddings (same as `--mode hybrid`); lexical mode skips
        // embedder construction via `NoopRetriever` so lexical-only
        // workspaces (provider = "none") can use `--trace` without
        // surfacing the "switch to --mode lexical" error.
        if opts.trace {
            let lex = Arc::new(LexicalRetriever::with_settings(
                self.sqlite.clone(),
                lexical_index_version(&self.config),
                self.config.search.snippet_chars,
            )) as Arc<dyn Retriever>;
            let vec_retr: Arc<dyn Retriever> = if matches!(query.mode, SearchMode::Lexical) {
                // `HybridRetriever::search_with_trace` never invokes the
                // vector retriever for `SearchMode::Lexical` (Task 4).
                // A no-op stand-in lets us avoid the ~470 MB embedder
                // load when the user only asked for lexical trace.
                Arc::new(NoopRetriever)
            } else {
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
                )) as Arc<dyn Retriever>
            };
            let hybrid = HybridRetriever::new(&self.config, lex, vec_retr);
            let (mut traced_hits, trace) = hybrid.search_with_trace(&fetch_query)?;

            // Stamp staleness — same as search_uncached.
            let now = time::OffsetDateTime::now_utc();
            crate::staleness::mark_stale_in_place(
                &mut traced_hits,
                now,
                self.config.search.stale_threshold_days,
            );
            // p10-1A-2: backfill code_lang — same as search_uncached.
            backfill_code_lang(&mut traced_hits);
            // p10-1A-2 Task 8b: backfill repo — same as search_uncached.
            self.backfill_repo(&mut traced_hits);

            // Apply offset + k_effective truncation (mirrors non-trace path).
            let drop_n = offset.min(traced_hits.len());
            traced_hits.drain(..drop_n);
            let mut hits: Vec<SearchHit> = traced_hits.into_iter().take(k_effective).collect();

            // Snippet truncation if opts.snippet_chars set (mirror non-trace path).
            if opts.snippet_chars.is_some() {
                for h in &mut hits {
                    if h.snippet.chars().count() > snippet_chars {
                        h.snippet = trim_to_chars(&h.snippet, snippet_chars);
                    }
                }
            }

            // Trace path skips the budget loop. Caller will inspect
            // `hits.len()` and `trace.timing` rather than paginate.
            let hint = short_query_hint(&query.text, hits.is_empty());
            return Ok(SearchResponse {
                hits,
                next_cursor: None,
                truncated: false,
                trace: Some(trace),
                hint,
            });
        }

        // backfill_code_lang + backfill_repo are applied inside `search`
        // via `search_uncached` — no explicit call needed here. Trace
        // branch above calls them directly because it bypasses `search`.
        let mut all_hits = self.search(fetch_query)?;

        // Skip offset.
        let drop_n = offset.min(all_hits.len());
        all_hits.drain(..drop_n);
        let mut hits: Vec<SearchHit> = all_hits.into_iter().take(k_effective).collect();

        // Apply snippet_chars override if shorter than what the
        // retriever returned (retriever already honored
        // `config.search.snippet_chars`; this only kicks in when the
        // caller asked for *less*).
        if opts.snippet_chars.is_some() {
            for h in &mut hits {
                if h.snippet.chars().count() > snippet_chars {
                    h.snippet = trim_to_chars(&h.snippet, snippet_chars);
                }
            }
        }

        // Budget loop.
        let mut truncated = false;
        if let Some(max_tokens) = opts.max_tokens {
            let max_chars = max_tokens.saturating_mul(4);
            // Step 1: shorten snippets progressively to a 60-char floor.
            const SNIPPET_FLOOR: usize = 60;
            let mut current_snippet_cap = snippet_chars;
            while estimate_chars(&hits) > max_chars && current_snippet_cap > SNIPPET_FLOOR {
                current_snippet_cap = (current_snippet_cap / 2).max(SNIPPET_FLOOR);
                for h in &mut hits {
                    if h.snippet.chars().count() > current_snippet_cap {
                        h.snippet = trim_to_chars(&h.snippet, current_snippet_cap);
                        truncated = true;
                    }
                }
            }
            // Step 2: pop hits from the end until we fit, but always
            // keep ≥ 1.
            while estimate_chars(&hits) > max_chars && hits.len() > 1 {
                hits.pop();
                truncated = true;
            }
        }

        // p9-fb-34: emit cursor whenever more hits may be reachable.
        // Three cases produce a non-null cursor:
        //   (a) returned == k_effective: retriever filled the page; there
        //       may be more behind it. Speculative — next call may return
        //       an empty page if nothing remains.
        //   (b) truncated by k-pop: returned < k_effective because we
        //       popped hits to fit the budget. Those popped hits live at
        //       offset+returned..; next call (with same or wider budget)
        //       resumes from there.
        //   (c) truncated by snippet-only shrink: returned == k_effective,
        //       falls under (a). Cursor lets caller paginate; widening
        //       --max-tokens lets caller re-fetch fuller snippets at the
        //       same offset.
        //
        // No cursor when neither (a) nor (b) applies — i.e. the retriever
        // returned fewer than k_effective AND we didn't pop. That means
        // end of available results.
        let returned = hits.len();
        let next_cursor = if returned == k_effective || truncated {
            if offset.saturating_add(returned) > 0 {
                Some(cursor::encode(offset + returned, &corpus_revision))
            } else {
                None
            }
        } else {
            None
        };

        let hint = short_query_hint(&query.text, hits.is_empty());
        Ok(SearchResponse {
            hits,
            next_cursor,
            truncated,
            trace: None,
            hint,
        })
    }

    /// Run a RAG `ask` against the configured retriever + LLM. Reuses
    /// the memoized embedder / vector / LLM where applicable.
    pub fn ask(&self, query: &str, opts: AskOpts) -> Result<Answer> {
        let retriever = self.build_retriever(opts.mode)?;
        let llm = self.llm()?;
        let pipeline = self.build_pipeline(retriever, llm);
        pipeline.ask(query, opts)
    }

    /// p9-fb-41 PR-9c-2: shared pipeline builder used by [`Self::ask`]
    /// and [`Self::ask_with_session`]. Attaches the App-built NLI
    /// verifier (when `cfg.rag.nli_threshold > 0`) via
    /// `RagPipeline::with_verifier`, keeping the construction site in
    /// a single place so the two call paths can't drift.
    fn build_pipeline(
        &self,
        retriever: Arc<dyn Retriever>,
        llm: Arc<dyn LanguageModel>,
    ) -> RagPipeline {
        let pipeline = RagPipeline::new(self.config.clone(), retriever, llm, self.sqlite.clone());
        match &self.pipeline_verifier {
            Some(v) => pipeline.with_verifier(v.clone()),
            None => pipeline,
        }
    }

    /// p9-fb-18: shared retriever-stack builder used by [`Self::ask`]
    /// and [`Self::ask_with_session`]. Lexical mode uses the FTS5
    /// retriever directly; vector / hybrid require embeddings (and
    /// surface the same "switch to --mode lexical" error from
    /// [`Self::require_embeddings`] when disabled).
    fn build_retriever(&self, mode: SearchMode) -> Result<Arc<dyn Retriever>> {
        Ok(match mode {
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
        })
    }

    /// p9-fb-18: ask under a persistent chat session. Loads the
    /// session's prior turns (if any), runs the query through
    /// `RagPipeline::ask_with_history`, then appends the new turn
    /// + (auto-)creates the session row on first use.
    ///
    /// `session_id` is caller-supplied. If the session doesn't
    /// exist yet, a new `chat_sessions` row is created with title
    /// derived from the first question (≤40 chars, trimmed and
    /// NFC-normalized). Subsequent calls with the same
    /// `session_id` extend the conversation.
    ///
    /// The returned `Answer` carries `conversation_id = Some(
    /// session_id)` and `turn_index = Some(n)` per p9-fb-15. The
    /// new `chat_turns` row is committed before this method
    /// returns; on persistence error, the answer is still returned
    /// (don't lose the user's compute) but the error is logged so
    /// the operator notices.
    pub fn ask_with_session(&self, session_id: &str, query: &str, opts: AskOpts) -> Result<Answer> {
        use kebab_core::traits::{ChatSessionRepo, ChatSessionRow, ChatTurnRow};
        use std::time::{SystemTime, UNIX_EPOCH};

        // Load (or create) the session header.
        let now_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let existing = self.sqlite.get_session(session_id)?;
        let prior_turns = match &existing {
            Some(_) => self.sqlite.list_turns(session_id)?,
            None => Vec::new(),
        };
        let next_index = u32::try_from(prior_turns.len()).unwrap_or(u32::MAX);

        // Build history Vec<Turn> from the persisted rows. Citations
        // are decoded best-effort — a corrupted citations_json
        // becomes an empty Vec rather than a panic (history is
        // advisory, not authoritative).
        let history: Vec<kebab_core::Turn> = prior_turns
            .iter()
            .map(|row| kebab_core::Turn {
                question: row.question.clone(),
                answer: row.answer.clone(),
                citations: serde_json::from_str(&row.citations_json).unwrap_or_default(),
                created_at: time::OffsetDateTime::from_unix_timestamp(row.created_at)
                    .unwrap_or(time::OffsetDateTime::UNIX_EPOCH),
            })
            .collect();

        // p9-fb-18 R1: shared retriever builder removes the prior
        // copy of `ask`'s 35-line stack — see [`Self::build_retriever`].
        // p9-fb-41 PR-9c-2: shared `build_pipeline` attaches the NLI
        // verifier when the gate is enabled.
        let retriever = self.build_retriever(opts.mode)?;
        let llm = self.llm()?;
        let pipeline = self.build_pipeline(retriever, llm);
        let answer =
            pipeline.ask_with_history(query, history, session_id.to_string(), next_index, opts)?;

        // Auto-create the session header on first use. Title from
        // the first question (≤40 chars after trim).
        if existing.is_none() {
            let title = first_question_title(query);
            let session_row = ChatSessionRow {
                session_id: session_id.to_string(),
                created_at: now_unix,
                updated_at: now_unix,
                title: Some(title),
                config_snapshot_json: serde_json::json!({
                    "prompt_template_version": self.config.rag.prompt_template_version,
                    "llm.model": self.config.models.llm.model,
                    "max_context_tokens": self.config.rag.max_context_tokens,
                })
                .to_string(),
            };
            if let Err(e) = self.sqlite.create_session(&session_row) {
                tracing::warn!(
                    target: "kebab-app",
                    error = %e,
                    session_id = %session_id,
                    "ask_with_session: create_session failed; continuing — turn append will surface a more useful error"
                );
            }
        }

        // Append the new turn. Failure is logged but does NOT mask
        // the answer — the user still gets their response, the
        // operator sees the persistence error in the warn log.
        let turn_id = format!(
            "{:032x}",
            blake3_truncate(&format!("{session_id}:{next_index}")),
        );
        let turn_row = ChatTurnRow {
            turn_id,
            session_id: session_id.to_string(),
            turn_index: next_index,
            question: query.to_string(),
            answer: answer.answer.clone(),
            citations_json: serde_json::to_string(&answer.citations)
                .unwrap_or_else(|_| "[]".to_string()),
            created_at: now_unix,
        };
        if let Err(e) = self.sqlite.append_turn(&turn_row) {
            tracing::warn!(
                target: "kebab-app",
                error = %e,
                session_id = %session_id,
                turn_index = next_index,
                "ask_with_session: append_turn failed; answer returned regardless"
            );
        }

        Ok(answer)
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
            FastembedEmbedder::new(&self.config).context("kb-app: load FastembedEmbedder")?,
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
            let mut guard = cache
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            guard.clear();
        }
    }

    /// p10-1A-2 Task 8b: back-fill `SearchHit.repo` from the originating
    /// document's `Metadata.repo` for every hit whose `repo` field is
    /// currently `None`. The search layer (kebab-search) constructs hits
    /// with `repo: None` because it has no store access; we fill it here
    /// in kebab-app post-retrieval via a per-distinct-`doc_id` store lookup.
    ///
    /// Deduplication: a small `HashMap` accumulates the
    /// `(doc_id → Option<String>)` mapping so each unique document is
    /// fetched at most once. Search result sets are small (default k ≤ 20),
    /// so the map overhead is negligible. A `None` entry is cached too
    /// (document not found or no repo in metadata) to avoid re-querying.
    ///
    /// Non-repo documents (markdown, PDF, plain text, code files outside a
    /// git tree) correctly keep `repo: None` — `Metadata.repo` is already
    /// `None` for those, so the assignment is a no-op.
    fn backfill_repo(&self, hits: &mut [SearchHit]) {
        use kebab_core::DocumentId;
        use std::collections::HashMap;

        // doc_id → Option<String> where None means "not found / no repo"
        let mut cache: HashMap<DocumentId, Option<String>> = HashMap::new();

        for hit in hits.iter_mut() {
            if hit.repo.is_some() {
                continue;
            }
            let repo_val = cache.entry(hit.doc_id.clone()).or_insert_with(|| {
                // Deliberately non-aborting: a failed store lookup for
                // one hit must not abort the whole search response. Log
                // the error so it's observable rather than silently
                // dropped (review #140 round 1).
                match self.sqlite.get_document(&hit.doc_id) {
                    Ok(opt) => opt.and_then(|doc| doc.metadata.repo),
                    Err(e) => {
                        tracing::warn!(
                            target: "kebab-app",
                            doc_id = %hit.doc_id,
                            error = %e,
                            "backfill_repo: get_document failed; leaving hit.repo = None"
                        );
                        None
                    }
                }
            });
            if let Some(r) = repo_val {
                hit.repo = Some(r.clone());
            }
        }
    }

    /// Resolve the embedder + vector store, surfacing the user-friendly
    /// "switch to --mode lexical" error when embeddings are disabled.
    fn require_embeddings(
        &self,
    ) -> Result<(Arc<dyn Embedder + Send + Sync>, Arc<LanceVectorStore>)> {
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

/// p9-fb-37: stand-in for the vector retriever in the trace path when
/// `query.mode == SearchMode::Lexical`. `HybridRetriever::search_with_trace`'s
/// Lexical branch never calls `vector.search()`, so returning an empty
/// hit list here is safe and lets lexical-only workspaces (embedding
/// `provider = "none"`) use `--trace` without paying the ~470 MB
/// embedder load.
struct NoopRetriever;

impl Retriever for NoopRetriever {
    fn search(&self, _q: &kebab_core::SearchQuery) -> anyhow::Result<Vec<kebab_core::SearchHit>> {
        Ok(Vec::new())
    }

    fn index_version(&self) -> kebab_core::IndexVersion {
        kebab_core::IndexVersion("noop:trace".into())
    }
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

/// p9-fb-18: derive a chat-session title from the first question.
/// Trim, NFC, take first ~40 chars. Always non-empty (falls back
/// to `"untitled"`) — same defensive shape as kebab-normalize's
/// derive_title.
fn first_question_title(question: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    let nfc: String = question.trim().nfc().collect();
    let truncated: String = nfc.chars().take(40).collect();
    if truncated.is_empty() {
        "untitled".to_string()
    } else {
        truncated
    }
}

/// p9-fb-18: 32-hex `turn_id` derived from session_id + turn_index.
/// blake3 hash truncated to first 16 bytes; format as 32-char lowercase
/// hex so it slots into the `chat_turns.turn_id` column without
/// collision concerns under any realistic per-session turn count.
fn blake3_truncate(input: &str) -> u128 {
    let hash = blake3::hash(input.as_bytes());
    let bytes = hash.as_bytes();
    let mut buf = [0u8; 16];
    buf.copy_from_slice(&bytes[..16]);
    u128::from_be_bytes(buf)
}

/// p9-fb-34: trim `s` to at most `n` Unicode scalar chars. Cheap
/// alternative to a `.chars().take(n).collect::<String>()` pattern;
/// reserves capacity proportional to UTF-8 worst case (4 bytes / char)
/// so the inner push never re-allocates.
fn trim_to_chars(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    let mut out = String::with_capacity(n.saturating_mul(4));
    for (i, c) in s.chars().enumerate() {
        if i >= n {
            break;
        }
        out.push(c);
    }
    out
}

/// p9-fb-34: estimate wire JSON char cost of the hit list. Returns 0
/// per-hit when serialization fails — a SearchHit serialization
/// failure is an invariant violation; we degrade gracefully (loop
/// terminates early) rather than panic in the budget loop.
fn estimate_chars(hits: &[SearchHit]) -> usize {
    hits.iter()
        .map(|h| serde_json::to_string(h).map(|s| s.len()).unwrap_or(0))
        .sum()
}

/// p10-1A-2: back-fill `SearchHit.code_lang` from `Citation::Code.lang`
/// for every code hit in the list. The search layer (kebab-search)
/// constructs hits with `code_lang: None`; we fill it here in kebab-app
/// post-retrieval so callers see the correct language identifier without
/// requiring a second SQL query.
fn backfill_code_lang(hits: &mut [SearchHit]) {
    for hit in hits.iter_mut() {
        if let kebab_core::Citation::Code { lang, .. } = &hit.citation {
            if hit.code_lang.is_none() {
                hit.code_lang = lang.clone();
            }
        }
    }
}

// ── v0.20.x r2 Enhancement 3: OCR stats + failures inspect ──────────────

/// Wire type for `kebab inspect ocr-stats --json` (`ocr_stats.v1`).
#[derive(serde::Serialize)]
pub struct OcrStatsV1 {
    pub schema_version: &'static str,
    pub total_events: u64,
    pub total_runs: u64,
    pub success_count: u64,
    pub failure_count: u64,
    pub success_rate: f64,
    pub p50_ms: Option<u64>,
    pub p90_ms: Option<u64>,
    pub p99_ms: Option<u64>,
    pub max_ms: Option<u64>,
    pub by_engine: std::collections::BTreeMap<String, u64>,
    pub by_doc: Vec<OcrStatsByDoc>,
}

/// Per-doc breakdown row inside `OcrStatsV1`.
#[derive(serde::Serialize)]
pub struct OcrStatsByDoc {
    pub doc_id: String,
    pub failure_count: u64,
    pub success_count: u64,
    pub p90_ms: Option<u64>,
}

/// Wire type for `kebab inspect ocr-failures --json` (`ocr_failures.v1`).
#[derive(serde::Serialize)]
pub struct OcrFailuresV1 {
    pub schema_version: &'static str,
    pub doc_id: Option<String>,
    pub failure_count: u64,
    pub failures: Vec<OcrFailureRow>,
}

/// Single failure row inside `OcrFailuresV1`.
#[derive(serde::Serialize)]
pub struct OcrFailureRow {
    pub ts: String,
    pub page: u32,
    pub ms: u64,
    pub reason: String,
    pub image_byte_size: Option<u64>,
}

impl App {
    /// Corpus-wide OCR statistics from the `pdf_ocr_events` SQLite mirror.
    pub fn inspect_ocr_stats(&self) -> Result<OcrStatsV1> {
        self.inspect_ocr_stats_with_config(&self.config)
    }

    #[doc(hidden)]
    pub fn inspect_ocr_stats_with_config(&self, _cfg: &kebab_config::Config) -> Result<OcrStatsV1> {
        use crate::ingest_log::percentiles;
        let conn = self.sqlite.read_conn();

        // 1. Aggregate counters
        let (total_events, success_count, failure_count, total_runs): (u64, u64, u64, u64) = conn
            .query_row(
                "SELECT COUNT(*), \
                        SUM(CASE WHEN success=1 THEN 1 ELSE 0 END), \
                        SUM(CASE WHEN success=0 THEN 1 ELSE 0 END), \
                        COUNT(DISTINCT run_id) \
                   FROM pdf_ocr_events",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap_or((0, 0, 0, 0));

        let success_rate = if total_events == 0 {
            0.0
        } else {
            success_count as f64 / total_events as f64
        };

        // 2. Latency percentiles from successful events
        let samples: Vec<u64> = {
            let mut stmt = conn
                .prepare("SELECT ms FROM pdf_ocr_events WHERE success=1 ORDER BY ms")
                .context("prepare ms query")?;
            stmt.query_map([], |r| r.get::<_, u64>(0))
                .context("query ms")?
                .filter_map(|r| r.ok())
                .collect()
        };
        let (p50_ms, p90_ms, p99_ms, max_ms) = percentiles(&samples);

        // 3. Engine breakdown
        let mut by_engine = std::collections::BTreeMap::new();
        {
            let mut stmt = conn
                .prepare("SELECT ocr_engine, COUNT(*) FROM pdf_ocr_events GROUP BY ocr_engine")
                .context("prepare engine query")?;
            let rows = stmt
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?)))
                .context("query engine")?;
            for row in rows.filter_map(|r| r.ok()) {
                by_engine.insert(row.0, row.1);
            }
        }

        // 4. Top-10 docs by failure count
        let by_doc: Vec<OcrStatsByDoc> = {
            let mut stmt = conn
                .prepare(
                    "SELECT doc_id, \
                            SUM(CASE WHEN success=0 THEN 1 ELSE 0 END), \
                            SUM(CASE WHEN success=1 THEN 1 ELSE 0 END) \
                       FROM pdf_ocr_events \
                      WHERE doc_id IS NOT NULL \
                      GROUP BY doc_id \
                      ORDER BY 2 DESC \
                      LIMIT 10",
                )
                .context("prepare by_doc query")?;
            stmt.query_map([], |r| {
                Ok(OcrStatsByDoc {
                    doc_id: r.get(0)?,
                    failure_count: r.get(1)?,
                    success_count: r.get(2)?,
                    p90_ms: None, // per-doc p90 deferred (open question #3)
                })
            })
            .context("query by_doc")?
            .filter_map(|r| r.ok())
            .collect()
        };

        Ok(OcrStatsV1 {
            schema_version: "ocr_stats.v1",
            total_events,
            total_runs,
            success_count,
            failure_count,
            success_rate,
            p50_ms,
            p90_ms,
            p99_ms,
            max_ms,
            by_engine,
            by_doc,
        })
    }

    /// Recent OCR failure rows, optionally filtered by `doc_id`.
    pub fn inspect_ocr_failures(
        &self,
        doc_id: Option<&str>,
        limit: usize,
    ) -> Result<OcrFailuresV1> {
        self.inspect_ocr_failures_with_config(&self.config, doc_id, limit)
    }

    #[doc(hidden)]
    pub fn inspect_ocr_failures_with_config(
        &self,
        _cfg: &kebab_config::Config,
        doc_id: Option<&str>,
        limit: usize,
    ) -> Result<OcrFailuresV1> {
        let conn = self.sqlite.read_conn();
        let failures: Vec<OcrFailureRow> = if let Some(did) = doc_id {
            let mut stmt = conn
                .prepare(
                    "SELECT ts, page, ms, COALESCE(reason,'unknown'), image_byte_size \
                       FROM pdf_ocr_events \
                      WHERE success=0 AND doc_id=? \
                      ORDER BY ts DESC \
                      LIMIT ?",
                )
                .context("prepare failures by doc_id")?;
            stmt.query_map(rusqlite::params![did, limit as i64], |r| {
                Ok(OcrFailureRow {
                    ts: r.get(0)?,
                    page: r.get(1)?,
                    ms: r.get(2)?,
                    reason: r.get(3)?,
                    image_byte_size: r.get(4)?,
                })
            })
            .context("query failures by doc_id")?
            .filter_map(|r| r.ok())
            .collect()
        } else {
            let mut stmt = conn
                .prepare(
                    "SELECT ts, page, ms, COALESCE(reason,'unknown'), image_byte_size \
                       FROM pdf_ocr_events \
                      WHERE success=0 \
                      ORDER BY ts DESC \
                      LIMIT ?",
                )
                .context("prepare failures corpus-wide")?;
            stmt.query_map(rusqlite::params![limit as i64], |r| {
                Ok(OcrFailureRow {
                    ts: r.get(0)?,
                    page: r.get(1)?,
                    ms: r.get(2)?,
                    reason: r.get(3)?,
                    image_byte_size: r.get(4)?,
                })
            })
            .context("query failures corpus-wide")?
            .filter_map(|r| r.ok())
            .collect()
        };
        Ok(OcrFailuresV1 {
            schema_version: "ocr_failures.v1",
            doc_id: doc_id.map(String::from),
            failure_count: failures.len() as u64,
            failures,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// p9-fb-18: title trims, NFC-normalizes, caps at 40 chars.
    #[test]
    fn first_question_title_trims_and_caps() {
        assert_eq!(first_question_title("  hello  "), "hello");
        let long = "a".repeat(100);
        assert_eq!(first_question_title(&long).chars().count(), 40);
    }

    /// p9-fb-18: empty / whitespace-only question falls back to
    /// `"untitled"` (never returns empty).
    #[test]
    fn first_question_title_falls_back_to_untitled() {
        assert_eq!(first_question_title(""), "untitled");
        assert_eq!(first_question_title("   "), "untitled");
        assert_eq!(first_question_title("\t\n"), "untitled");
    }

    /// p9-fb-18: korean NFD → NFC.
    #[test]
    fn first_question_title_nfc_normalizes_korean() {
        let nfd = "\u{1100}\u{1161}".to_string(); // 가 (NFD)
        let title = first_question_title(&nfd);
        assert_eq!(title, "\u{AC00}", "expected NFC composed form");
    }

    /// p9-fb-18: blake3_truncate is deterministic and differs across
    /// distinct inputs.
    #[test]
    fn blake3_truncate_deterministic_and_distinct() {
        let a = blake3_truncate("session-x:0");
        let b = blake3_truncate("session-x:0");
        let c = blake3_truncate("session-x:1");
        let d = blake3_truncate("session-y:0");
        assert_eq!(a, b, "same input → same hash");
        assert_ne!(a, c, "different turn_index → different hash");
        assert_ne!(a, d, "different session_id → different hash");
    }
}

#[cfg(test)]
mod tests_trace {
    use super::*;
    use kebab_core::{SearchMode, SearchOpts, SearchQuery};

    fn open_app_with_temp_dir() -> (tempfile::TempDir, App) {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = kebab_config::Config::defaults();
        cfg.storage.data_dir = dir.path().to_string_lossy().into_owned();
        // Bring up migrations.
        let store = kebab_store_sqlite::SqliteStore::open(&cfg).unwrap();
        store.run_migrations().unwrap();
        drop(store);
        let app = App::open_with_config(cfg).unwrap();
        (dir, app)
    }

    #[test]
    fn search_response_trace_none_when_opts_trace_false() {
        let (_dir, app) = open_app_with_temp_dir();
        let q = SearchQuery {
            text: "x".into(),
            mode: SearchMode::Lexical,
            k: 1,
            filters: Default::default(),
        };
        let resp = app.search_with_opts(q, SearchOpts::default()).unwrap();
        assert!(resp.trace.is_none());
    }

    #[test]
    fn search_response_trace_some_when_opts_trace_true_lexical_mode() {
        // Lexical mode doesn't require embeddings — the trace path
        // builds HybridRetriever with a `NoopRetriever` stand-in for
        // the vector side, since `HybridRetriever::search_with_trace`'s
        // Lexical branch never invokes `vector.search()`. Default
        // Config has embedding `provider = "none"`, and lexical-mode
        // trace must succeed under that config (no embedder load).
        let (_dir, app) = open_app_with_temp_dir();
        let q = SearchQuery {
            text: "x".into(),
            mode: SearchMode::Lexical,
            k: 1,
            filters: Default::default(),
        };
        let opts = SearchOpts {
            trace: true,
            ..Default::default()
        };
        let resp = app
            .search_with_opts(q, opts)
            .expect("lexical-mode trace must succeed without embeddings");
        assert!(resp.trace.is_some(), "trace populated when opts.trace=true");
    }
}

/// post-v0.18.0 extractor-dispatch-unification: in-crate unit tests for
/// the `App.extractors` registry + `App::extract_for` polymorphic
/// dispatch. In-crate (not `tests/`) because `extractors` + `extract_for`
/// are `pub(crate)` — integration tests cannot reach them.
///
/// Spec §5.1 + plan §2 Step 10 — 3 test class:
/// 1. registry length = 11 (image + pdf + 9 AST).
/// 2. mutually-exclusive `supports()` grid over 16 sample MediaTypes.
/// 3. `extract_for` returns `Err("no Extractor ...")` for registry-NOT-cover
///    MediaType (Audio).
#[cfg(test)]
mod tests_extractor_dispatch {
    use super::*;
    use kebab_core::{AudioType, ExtractConfig, ImageType};

    /// helper: tempdir-isolated App for tests (mirrors `tests_trace`'s
    /// `open_app_with_temp_dir` pattern).
    fn open_app_with_temp_dir() -> (tempfile::TempDir, App) {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = kebab_config::Config::defaults();
        cfg.storage.data_dir = dir.path().to_string_lossy().into_owned();
        // Bring up migrations.
        let store = kebab_store_sqlite::SqliteStore::open(&cfg).unwrap();
        store.run_migrations().unwrap();
        drop(store);
        let app = App::open_with_config(cfg).unwrap();
        (dir, app)
    }

    /// Registry length invariant: 11 Extractor (image + pdf + 9 AST).
    /// Markdown is NOT registered (free-function path — defer to a
    /// separate PR per spec §3.4).
    #[test]
    fn registry_has_eleven_extractors() {
        let (_dir, app) = open_app_with_temp_dir();
        assert_eq!(
            app.extractors.len(),
            11,
            "registry must hold 11 Extractors (image + pdf + 9 AST). \
             markdown 은 별 PR."
        );
    }

    /// 11 Extractor 의 `supports()` 가 16 sample MediaType 에 대해
    /// mutually exclusive — 어떤 두 Extractor 도 동일 MediaType 에
    /// 대해 true 반환 안 됨.
    #[test]
    fn supports_grid_is_mutually_exclusive() {
        let (_dir, app) = open_app_with_temp_dir();
        let samples = vec![
            MediaType::Markdown,
            MediaType::Pdf,
            MediaType::Image(ImageType::Png),
            MediaType::Image(ImageType::Jpeg),
            MediaType::Code("rust".into()),
            MediaType::Code("python".into()),
            MediaType::Code("typescript".into()),
            MediaType::Code("javascript".into()),
            MediaType::Code("go".into()),
            MediaType::Code("java".into()),
            MediaType::Code("kotlin".into()),
            MediaType::Code("c".into()),
            MediaType::Code("cpp".into()),
            MediaType::Code("yaml".into()),   // registry NOT cover
            MediaType::Code("shell".into()),  // registry NOT cover
            MediaType::Audio(AudioType::Wav), // registry NOT cover
        ];
        for sample in &samples {
            let hits: Vec<_> = app
                .extractors
                .iter()
                .filter(|e| e.supports(sample))
                .collect();
            assert!(
                hits.len() <= 1,
                "mutually exclusive violated for {sample:?}: {} hits",
                hits.len()
            );
        }
    }

    /// `extract_for` 가 registry NOT cover MediaType (Audio) 에 대해
    /// `Err("no Extractor for media_type ...")` 반환. Audio MediaType
    /// 사용으로 RawAsset 의 actual content 의존 회피 — registry NOT
    /// cover → 즉시 Err.
    #[test]
    fn extract_for_unsupported_media_errors() {
        let (_dir, app) = open_app_with_temp_dir();

        // Minimal RawAsset. Actual content never read — Audio MediaType
        // 는 registry NOT cover → `extract_for` 가 dispatch loop 안에서
        // 바로 Err 반환. RawAsset field set 은 `crates/kebab-core/src/
        // asset.rs:62-73` 와 정합 (8 field).
        let asset = kebab_core::RawAsset {
            asset_id: kebab_core::AssetId("00".repeat(16)),
            source_uri: kebab_core::SourceUri::File("/tmp/dummy.wav".into()),
            workspace_path: kebab_core::WorkspacePath("dummy.wav".to_string()),
            media_type: MediaType::Audio(AudioType::Wav),
            byte_len: 0,
            checksum: kebab_core::Checksum("00".repeat(32)),
            discovered_at: time::OffsetDateTime::now_utc(),
            // AssetStorage::Inline 미존재 — actual variant `Copied { path }`
            // 사용 (kebab-core/src/asset.rs:55-60).
            stored: kebab_core::AssetStorage::Copied {
                path: std::path::PathBuf::from("/tmp/dummy.wav"),
            },
        };

        let workspace_root: std::path::PathBuf = std::path::PathBuf::from("/tmp");
        let cfg = ExtractConfig::default();
        let ctx = ExtractContext {
            asset: &asset,
            workspace_root: &workspace_root,
            config: &cfg,
        };
        let result = app.extract_for(&MediaType::Audio(AudioType::Wav), &ctx, &[]);
        assert!(result.is_err(), "Audio 는 registry 미포함 → Err 기대");
        let err_msg = format!("{:#}", result.unwrap_err());
        assert!(
            err_msg.contains("no Extractor"),
            "unexpected err: {err_msg}"
        );
    }
}
