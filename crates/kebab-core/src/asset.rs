//! Raw asset, source URI, workspace path (§3.3).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::errors::CoreError;
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
/// produced via `crate::normalize::to_posix` (filesystem side) or
/// `WorkspacePath::new` (parse side). The inner string is forbidden from
/// containing the `#` character: a workspace path must never collide with
/// the W3C-Media-Fragments separator that `Citation` URIs rely on, so the
/// invariant is enforced at construction rather than at every call site.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct WorkspacePath(pub String);

impl WorkspacePath {
    /// Construct a `WorkspacePath` from a string, rejecting any input that
    /// contains `#`. Use this on the parser side (e.g. `Citation::parse`)
    /// where the input does not flow through `to_posix`.
    pub fn new(s: String) -> Result<Self, CoreError> {
        if s.contains('#') {
            return Err(CoreError::Malformed(format!(
                "workspace path must not contain '#': {s:?}"
            )));
        }
        Ok(Self(s))
    }
}

/// On-disk storage decision for a `RawAsset`.
///
/// **Important convention** — `path` field semantics differ by variant:
///
/// - `Copied { path }`: at scan time, `path` is the **source** path on the
///   user's filesystem. The asset writer (P1-6) is responsible for actually
///   copying the bytes into the workspace asset store, AND for overwriting
///   `path` with the destination path after the copy completes.
///
/// - `Reference { path, sha }`: `path` is always the **source** path. No
///   bytes are ever copied; downstream readers stream from `path` directly.
///   `sha` is the BLAKE3 full hex (matches `RawAsset::checksum`).
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
