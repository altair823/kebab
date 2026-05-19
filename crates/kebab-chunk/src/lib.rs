//! `kb-chunk` — chunkers that emit [`kebab_core::Chunk`] batches.
//!
//! Per design §3.5 (Chunk), §4.2 (chunk_id recipe), §7.2 (`Chunker`
//! trait), §0 Q3/§14 (chunking priority).
//!
//! Public surface:
//!
//! * [`MdHeadingV1Chunker`] — heading-aware chunker for Markdown
//!   `CanonicalDocument`s, emitting `chunker_version = "md-heading-v1"`.
//!
//! Behavior contract is enumerated on [`MdHeadingV1Chunker`].
//!
//! This crate must NOT depend on any parser implementation
//! (`kb-parse-md`, `kb-parse-pdf`, …), the document/vector store, the
//! embedder, the retriever, the LLM, the RAG layer, or the UI layers.
//! It consumes `CanonicalDocument` purely through `kb-core` types.

mod code_rust_ast_v1;
mod md_heading_v1;
mod pdf_page_v1;

pub use code_rust_ast_v1::CodeRustAstV1Chunker;
pub use md_heading_v1::MdHeadingV1Chunker;
pub use pdf_page_v1::PdfPageV1Chunker;
