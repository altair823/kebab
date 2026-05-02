//! `CoreError` (§10).

use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("invalid id: {0}")]
    InvalidId(String),
    #[error("invalid citation: {0}")]
    InvalidCitation(String),
    #[error("invalid source span: {0}")]
    InvalidSpan(String),
    #[error("malformed input: {0}")]
    Malformed(String),
}
