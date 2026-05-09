//! p9-fb-34 opaque pagination cursor.
//!
//! Format: base64(JSON({offset: usize, corpus_revision: string})).
//! Opaque to callers — they MUST NOT decode the contents themselves;
//! the schema is internal and may change without notice.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error_wire::ErrorV1;

#[derive(Serialize, Deserialize)]
struct Payload {
    offset: usize,
    corpus_revision: String,
}

/// Encode `(offset, corpus_revision)` as an opaque base64 string.
pub fn encode(offset: usize, corpus_revision: &str) -> String {
    let payload = Payload {
        offset,
        corpus_revision: corpus_revision.to_string(),
    };
    let json = serde_json::to_vec(&payload).expect("Payload serializes");
    URL_SAFE_NO_PAD.encode(&json)
}

/// Decode an opaque cursor against the expected `corpus_revision`.
/// Mismatch or malformed input returns an `ErrorV1` with
/// `code = "stale_cursor"`.
pub fn decode(s: &str, expected_revision: &str) -> Result<usize, ErrorV1> {
    let bytes = URL_SAFE_NO_PAD
        .decode(s.as_bytes())
        .map_err(|_| stale("<malformed>", expected_revision))?;
    let payload: Payload = serde_json::from_slice(&bytes)
        .map_err(|_| stale("<malformed>", expected_revision))?;
    if payload.corpus_revision != expected_revision {
        return Err(stale(&payload.corpus_revision, expected_revision));
    }
    Ok(payload.offset)
}

fn stale(found: &str, expected: &str) -> ErrorV1 {
    ErrorV1 {
        schema_version: "error.v1".to_string(),
        code: "stale_cursor".to_string(),
        message: format!(
            "cursor was issued against corpus_revision '{found}'; current revision is \
             '{expected}'. Re-issue search to obtain a fresh cursor."
        ),
        details: Value::Null,
        hint: None,
    }
}
