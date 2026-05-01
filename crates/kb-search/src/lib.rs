//! `kb-search` — `kb_core::Retriever` implementations.
//!
//! P2-2 ships [`LexicalRetriever`], a SQLite-FTS5-backed retriever for
//! `SearchMode::Lexical`. Vector + Hybrid retrievers land in P3-3 / P3-4.
//!
//! Allowed deps per task spec: `kb-core`, `kb-config`, `kb-store-sqlite`,
//! `rusqlite`, `globset`, `tracing`, `thiserror`, `anyhow`. Forbidden:
//! `kb-source-fs`, `kb-parse-md`, `kb-normalize`, `kb-chunk`,
//! `kb-store-vector`, `kb-embed*`, `kb-llm*`, `kb-rag`, `kb-tui`,
//! `kb-desktop`. Only `serde_json` is a transitive helper used to decode
//! JSON-typed columns from `chunks` / `documents`.

mod lexical;

pub use lexical::LexicalRetriever;
