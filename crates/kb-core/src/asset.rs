//! Raw asset, source URI, workspace path (§3.3).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::ids::AssetId;
use crate::media::{Checksum, MediaType};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", tag = "kind", content = "value")]
pub enum SourceUri {
    File(PathBuf),
    /// `kb://` virtual reference.
    Kb(String),
}

/// POSIX-relative path inside the workspace root (§6.6, §4.1). Always
/// produced via `crate::normalize::to_posix`.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct WorkspacePath(pub String);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", tag = "kind")]
pub enum AssetStorage {
    Copied { path: PathBuf },
    Reference { path: PathBuf, sha: Checksum },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RawAsset {
    pub asset_id: AssetId,
    pub source_uri: SourceUri,
    pub workspace_path: WorkspacePath,
    pub media_type: MediaType,
    pub byte_len: u64,
    pub checksum: Checksum,
    #[serde(with = "time::serde::rfc3339")]
    pub discovered_at: OffsetDateTime,
    pub stored: AssetStorage,
}
