//! `kebab-parse-code` — language-aware parsing for code corpora.
//!
//! Repo metadata (`detect_repo`) + per-language AST extractors (Rust = P10-1A-2, Python/TS/JS = P10-1B, Go = P10-1C-Go, Java+Kotlin = P10-1C-JK, C+C++ = P10-1D).
//!
//! lang detect (`code_lang_for_path`) + pre-ingest skip helpers (`is_generated_file`, `is_oversized`, `BUILTIN_BLACKLIST`) 는 v0.18.0+ 부터 `kebab-source-fs::code_meta` 로 이동 — refactor 2026-05-26.
//!
//! 본 crate 의 boundary 는 design §8 — store / embed / llm / rag / UI 의존 금지.

pub mod c;
pub mod cpp;
pub mod go;
pub mod java;
pub mod javascript;
pub mod kotlin;
pub mod lang;
pub mod python;
pub mod repo;
pub mod rust;
pub(crate) mod scaffold;
pub mod typescript;

pub use c::{CAstExtractor, PARSER_VERSION as C_PARSER_VERSION};
pub use cpp::{CppAstExtractor, PARSER_VERSION as CPP_PARSER_VERSION};
pub use go::{GoAstExtractor, PARSER_VERSION as GO_PARSER_VERSION};
pub use java::{JavaAstExtractor, PARSER_VERSION as JAVA_PARSER_VERSION};
pub use javascript::{JavascriptAstExtractor, PARSER_VERSION as JS_PARSER_VERSION};
pub use kotlin::{KotlinAstExtractor, PARSER_VERSION as KOTLIN_PARSER_VERSION};
pub use lang::{module_path_for_python, module_path_for_tsjs};
pub use python::{PARSER_VERSION as PYTHON_PARSER_VERSION, PythonAstExtractor};
pub use repo::{RepoMeta, detect_repo};
pub use rust::{PARSER_VERSION as RUST_PARSER_VERSION, RustAstExtractor};
pub use typescript::{PARSER_VERSION as TS_PARSER_VERSION, TypescriptAstExtractor};
