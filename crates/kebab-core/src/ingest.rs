//! IngestReport + IngestItem (mirrored from wire §2.4).

use serde::{Deserialize, Serialize};

use crate::asset::WorkspacePath;
use crate::ids::{AssetId, DocumentId};
use crate::traits::SourceScope;
use crate::versions::{ChunkerVersion, ParserVersion};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IngestReport {
    pub scope: SourceScope,
    pub scanned: u32,
    pub new: u32,
    pub updated: u32,
    pub skipped: u32,
    pub errors: u32,
    pub duration_ms: u32,
    /// `None` ↔ wire `items: null` (`--summary-only`).
    pub items: Option<Vec<IngestItem>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IngestItem {
    pub kind: IngestItemKind,
    pub doc_id: Option<DocumentId>,
    pub doc_path: WorkspacePath,
    pub asset_id: Option<AssetId>,
    pub byte_len: Option<u64>,
    pub block_count: Option<u32>,
    pub chunk_count: Option<u32>,
    pub parser_version: Option<ParserVersion>,
    pub chunker_version: Option<ChunkerVersion>,
    pub warnings: Vec<String>,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IngestItemKind {
    New,
    Updated,
    Skipped,
    Error,
}
