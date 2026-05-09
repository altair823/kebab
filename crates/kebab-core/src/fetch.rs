//! p9-fb-35 verbatim fetch domain types.
//!
//! Three modes (chunk / doc / span) carried by [`FetchQuery`]; one
//! response shape ([`FetchResult`]) discriminated by [`FetchKind`].
//! All types are `Serialize` so the CLI / MCP wire layers can hand
//! them straight through `serde_json::to_value`.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::asset::WorkspacePath;
use crate::chunk::Chunk;
use crate::ids::{ChunkId, DocumentId};

#[derive(Clone, Debug)]
pub enum FetchQuery {
    Chunk(ChunkId),
    Doc(DocumentId),
    Span {
        doc_id: DocumentId,
        line_start: u32,
        line_end: u32,
    },
}

#[derive(Clone, Debug, Default)]
pub struct FetchOpts {
    /// chunk mode only: ±N chunks. None = no surrounding context.
    pub context: Option<u32>,
    /// doc / span mode only: chars/4 budget. None = no cap.
    pub max_tokens: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FetchKind {
    Chunk,
    Doc,
    Span,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FetchResult {
    pub kind: FetchKind,
    pub doc_id: DocumentId,
    pub doc_path: WorkspacePath,
    #[serde(with = "time::serde::rfc3339")]
    pub indexed_at: OffsetDateTime,
    pub stale: bool,
    // chunk mode payloads
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunk: Option<Chunk>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub context_before: Vec<Chunk>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub context_after: Vec<Chunk>,
    // doc / span payloads
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_start: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_end: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_end: Option<u32>,
    pub truncated: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fetch_opts_default_is_all_none() {
        let o = FetchOpts::default();
        assert!(o.context.is_none());
        assert!(o.max_tokens.is_none());
    }

    #[test]
    fn fetch_kind_serializes_snake_case() {
        let v = serde_json::to_value(FetchKind::Chunk).unwrap();
        assert_eq!(v, serde_json::json!("chunk"));
        let v = serde_json::to_value(FetchKind::Span).unwrap();
        assert_eq!(v, serde_json::json!("span"));
    }
}
