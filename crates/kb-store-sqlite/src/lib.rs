//! `kb-store-sqlite` — SQLite-backed implementations of
//! [`kb_core::DocumentStore`] and [`kb_core::JobRepo`] (§7.2), plus the
//! asset writer that copies (or references) raw bytes per design §5.2.
//!
//! Schema is owned by `migrations/V001__init.sql` (workspace root), which
//! ships the full §5 DDL minus the FTS5 virtual table + triggers (those
//! land in P2-1's `V002`).
//!
//! Allowed deps per task spec: `kb-core`, `kb-config`, `rusqlite`,
//! `refinery`, `serde_json`, `time`, `blake3`, `tracing`, `anyhow`,
//! `thiserror`. NOT allowed: `kb-parse-*`, `kb-normalize`, `kb-chunk`,
//! `kb-store-vector`, `kb-source-fs`, etc. (`kb-parse-md`, `kb-normalize`,
//! `kb-chunk` may appear as **dev-deps** — see `Cargo.toml` — to drive
//! the contract round-trip test off a real Markdown fixture.)

mod error;
mod schema;
mod store;
mod documents;
mod jobs;

pub use error::StoreError;
pub use store::SqliteStore;
