//! Crate-local error type per design §10.
//!
//! Boundary code (`kb-app`, `kb-cli`) flattens these into `anyhow::Error`,
//! so the trait impls return `anyhow::Result` directly. Internally we
//! still distinguish `Conflict` (e.g. checksum mismatch) from `Sqlite` /
//! `Migration` so callers that downcast can route refusal-style flows.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("migration error: {0}")]
    Migration(String),

    #[error("conflict: {0}")]
    Conflict(String),
}
