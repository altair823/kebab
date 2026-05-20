//! `kebab-parse-code` — language-aware parsing for code corpora.
//!
//! Phase 1A-1 ships infrastructure only:
//!
//! - [`lang::code_lang_for_path`] — extension → language identifier.
//! - [`repo::detect_repo`] — `.git/` walk-up → repo / branch / commit metadata.
//! - [`skip::is_generated_file`] / [`skip::is_oversized`] — pre-ingest skip
//!   helpers consulted by `kebab-source-fs`.
//! - [`skip::BUILTIN_BLACKLIST`] — 6-entry safety-net pattern list.
//!
//! Per-language parser modules (`rust`, `python`, `typescript`, …) land in
//! later phases (1A-2 onwards). The crate boundary follows other
//! `kebab-parse-*` crates per design §8: must NOT depend on store / embed
//! / llm / rag.

pub mod lang;
pub mod repo;
pub mod rust;
pub mod skip;

pub use lang::{code_lang_for_path, module_path_for_python, module_path_for_tsjs};
pub use repo::{RepoMeta, detect_repo};
pub use rust::{PARSER_VERSION as RUST_PARSER_VERSION, RustAstExtractor};
pub use skip::{BUILTIN_BLACKLIST, is_generated_file, is_oversized};
