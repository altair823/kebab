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

mod code_go_ast_v1;
mod code_java_ast_v1;
mod code_js_ast_v1;
mod code_kotlin_ast_v1;
mod code_python_ast_v1;
mod code_rust_ast_v1;
mod code_ts_ast_v1;
mod md_heading_v1;
mod pdf_page_v1;
mod tier2_shared;
pub mod k8s_manifest_resource_v1;
pub mod dockerfile_file_v1;
pub mod manifest_file_v1;
pub mod code_text_paragraph_v1;

pub use code_go_ast_v1::CodeGoAstV1Chunker;
pub use code_java_ast_v1::CodeJavaAstV1Chunker;
pub use code_js_ast_v1::CodeJsAstV1Chunker;
pub use code_kotlin_ast_v1::CodeKotlinAstV1Chunker;
pub use code_python_ast_v1::CodePythonAstV1Chunker;
pub use code_rust_ast_v1::CodeRustAstV1Chunker;
pub use code_ts_ast_v1::CodeTsAstV1Chunker;
pub use md_heading_v1::MdHeadingV1Chunker;
pub use pdf_page_v1::PdfPageV1Chunker;
pub use k8s_manifest_resource_v1::K8sManifestResourceV1Chunker;
pub use dockerfile_file_v1::DockerfileFileV1Chunker;
pub use manifest_file_v1::ManifestFileV1Chunker;
pub use code_text_paragraph_v1::CodeTextParagraphV1Chunker;
