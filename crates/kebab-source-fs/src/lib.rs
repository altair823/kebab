//! `kb-source-fs` — local filesystem `SourceConnector`.
//!
//! Walks `config.workspace.root`, applies gitignore-style filters from
//! `config.workspace.exclude` ∪ `.kbignore`, computes BLAKE3 of every file,
//! and emits `Vec<RawAsset>` sorted by `workspace_path` for determinism.
//!
//! Per design §3.3 (RawAsset), §6.2 (workspace + .kbignore), §6.6 (POSIX
//! normalization), §7.1 (SourceScope), §7.2 (SourceConnector), §8 (module
//! boundaries).

mod connector;
mod hash;
mod media;
mod walker;

pub use connector::FsSourceConnector;
