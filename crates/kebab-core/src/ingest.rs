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
    /// Media-type / source filter (`kb://`, unsupported types).
    pub skipped: u32,
    /// p9-fb-23: assets whose checksum + all version inputs matched —
    /// parse / chunk / embed / vector upsert all skipped.
    pub unchanged: u32,
    pub errors: u32,
    pub duration_ms: u32,
    /// p9-fb-25: per-extension skip count. Key = lowercase extension
    /// without leading dot (e.g. "docx", "txt"); files without an
    /// extension key under "<no-ext>". `BTreeMap` so the wire JSON
    /// has stable key order across runs.
    pub skipped_by_extension: std::collections::BTreeMap<String, u32>,
    /// p10-1A-1: files skipped because they matched a repo-local `.gitignore`.
    #[serde(default)]
    pub skipped_gitignore: u32,
    /// p10-1A-1: files skipped because they matched a `.kebabignore` entry.
    #[serde(default)]
    pub skipped_kebabignore: u32,
    /// p10-1A-1: files skipped because they matched the built-in safety-net
    /// blacklist (`node_modules/`, `target/`, `__pycache__/`, `.venv/`,
    /// `venv/`, `env/`).
    #[serde(default)]
    pub skipped_builtin_blacklist: u32,
    /// p10-1A-1: files skipped because their first ~512 bytes contained a
    /// generated-file marker (`@generated`, `do not edit`, …).
    #[serde(default)]
    pub skipped_generated: u32,
    /// p10-1A-1: files skipped because they exceeded `max_file_bytes` or
    /// `max_file_lines` in `[ingest.code]`.
    #[serde(default)]
    pub skipped_size_exceeded: u32,
    /// p10-1A-1: sample file paths per skip category (≤ 5 each).
    #[serde(default)]
    pub skip_examples: SkipExamples,
    /// Dogfood: docs whose on-disk file was deleted since the last ingest
    /// and were therefore removed from the store. Additive field — older
    /// wire consumers that pre-date this field read it as 0 via
    /// `#[serde(default)]`.
    #[serde(default)]
    pub purged_deleted_files: u32,
    /// `None` ↔ wire `items: null` (`--summary-only`).
    pub items: Option<Vec<IngestItem>>,
}

/// p10-1A-1: per-category sample of skipped file paths. Each category caps at
/// 5 entries (oldest-first). Used for debugging "why was X not indexed?"
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SkipExamples {
    #[serde(default)]
    pub generated: Vec<String>,
    #[serde(default)]
    pub size_exceeded: Vec<String>,
    #[serde(default)]
    pub builtin_blacklist: Vec<String>,
    #[serde(default)]
    pub gitignore: Vec<String>,
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
    /// Media-type filter / kb:// URI / non-supported source — never made
    /// it into the parse step.
    Skipped,
    /// p9-fb-23: blake3 checksum + parser_version + chunker_version +
    /// embedding_version all matched the existing record. Parse / chunk
    /// / embed / vector upsert all skipped.
    Unchanged,
    Error,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::SourceScope;

    #[test]
    fn skip_examples_default_is_empty() {
        let s = SkipExamples::default();
        assert!(s.generated.is_empty());
        assert!(s.size_exceeded.is_empty());
        assert!(s.builtin_blacklist.is_empty());
        assert!(s.gitignore.is_empty());
    }

    #[test]
    fn ingest_report_skip_counters_serialize() {
        let r = IngestReport {
            scope: SourceScope {
                root: std::path::PathBuf::from("/tmp"),
                include: vec![],
                exclude: vec![],
            },
            scanned: 100,
            new: 50,
            updated: 0,
            skipped: 0,
            unchanged: 0,
            errors: 0,
            duration_ms: 1234,
            skipped_by_extension: Default::default(),
            skipped_gitignore: 30,
            skipped_kebabignore: 5,
            skipped_builtin_blacklist: 10,
            skipped_generated: 3,
            skipped_size_exceeded: 2,
            skip_examples: SkipExamples {
                generated: vec!["a/b.pb.rs".into()],
                size_exceeded: vec![],
                builtin_blacklist: vec!["node_modules/x.js".into()],
                gitignore: vec![],
            },
            purged_deleted_files: 0,
            items: None,
        };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["skipped_gitignore"], 30);
        assert_eq!(v["skipped_builtin_blacklist"], 10);
        assert_eq!(v["skipped_generated"], 3);
        assert_eq!(v["skipped_size_exceeded"], 2);
        assert_eq!(v["skip_examples"]["generated"][0], "a/b.pb.rs");
    }
}
