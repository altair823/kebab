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

pub mod go;
pub mod javascript;
pub mod lang;
pub mod python;
pub mod repo;
pub mod rust;
pub(crate) mod scaffold;
pub mod skip;
pub mod typescript;

pub use go::{PARSER_VERSION as GO_PARSER_VERSION, GoAstExtractor};
pub use javascript::{PARSER_VERSION as JS_PARSER_VERSION, JavascriptAstExtractor};
pub use lang::{code_lang_for_path, module_path_for_python, module_path_for_tsjs};
pub use python::{PARSER_VERSION as PYTHON_PARSER_VERSION, PythonAstExtractor};
pub use repo::{RepoMeta, detect_repo};
pub use rust::{PARSER_VERSION as RUST_PARSER_VERSION, RustAstExtractor};
pub use skip::{BUILTIN_BLACKLIST, is_generated_file, is_oversized};
pub use typescript::{PARSER_VERSION as TS_PARSER_VERSION, TypescriptAstExtractor};
