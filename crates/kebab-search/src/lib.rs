//! `kb-search` — `kebab_core::Retriever` implementations.
//!
//! - [`LexicalRetriever`] (P2-2): SQLite-FTS5 + bm25 backed retriever
//!   for `SearchMode::Lexical`.
//! - [`VectorRetriever`] (P3-4): wraps a `dyn VectorStore` (typically
//!   `kb-store-vector::LanceVectorStore`) and a `dyn Embedder`,
//!   hydrating SQLite metadata for full `SearchHit`s.
//! - [`HybridRetriever`] (P3-4): composes lexical + vector retrievers,
//!   dispatches by `SearchMode`, fuses Hybrid via [`FusionPolicy::Rrf`].
//!
//! Allowed deps per the P2-2 + P3-4 task specs: `kb-core`, `kb-config`,
//! `kb-store-sqlite`, `kb-store-vector`, `kb-embed` (trait re-export
//! only — concrete adapters like `kb-embed-local` are runtime-injected
//! via `Arc<dyn Embedder>`), `rusqlite`, `globset`, `serde_json`,
//! `tracing`, `thiserror`, `anyhow`. Forbidden: `kb-source-fs`,
//! `kb-parse-md`, `kb-normalize`, `kb-chunk`, `kb-embed-local` (concrete
//! adapter), `kb-llm*`, `kb-rag`, `kb-tui`, `kb-desktop`.

mod citation_helper;
mod hybrid;
mod lexical;
mod trace;
mod vector;

pub use hybrid::{FusionPolicy, HybridRetriever};
pub use lexical::LexicalRetriever;
pub use vector::VectorRetriever;
