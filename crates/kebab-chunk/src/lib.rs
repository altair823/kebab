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

mod code_c_ast_v1;
mod code_cpp_ast_v1;
mod code_go_ast_v1;
mod code_java_ast_v1;
mod code_js_ast_v1;
mod code_kotlin_ast_v1;
mod code_python_ast_v1;
mod code_rust_ast_v1;
pub mod code_text_paragraph_v1;
mod code_ts_ast_v1;
pub mod dockerfile_file_v1;
pub mod k8s_manifest_resource_v1;
pub mod manifest_file_v1;
mod md_heading_v1;
mod pdf_page_v1;
mod tier2_shared;

pub use code_c_ast_v1::CodeCAstV1Chunker;
pub use code_cpp_ast_v1::CodeCppAstV1Chunker;
pub use code_go_ast_v1::CodeGoAstV1Chunker;
pub use code_java_ast_v1::CodeJavaAstV1Chunker;
pub use code_js_ast_v1::CodeJsAstV1Chunker;
pub use code_kotlin_ast_v1::CodeKotlinAstV1Chunker;
pub use code_python_ast_v1::CodePythonAstV1Chunker;
pub use code_rust_ast_v1::CodeRustAstV1Chunker;
pub use code_text_paragraph_v1::CodeTextParagraphV1Chunker;
pub use code_ts_ast_v1::CodeTsAstV1Chunker;
pub use dockerfile_file_v1::DockerfileFileV1Chunker;
pub use k8s_manifest_resource_v1::K8sManifestResourceV1Chunker;
pub use manifest_file_v1::ManifestFileV1Chunker;
pub use md_heading_v1::MdHeadingV1Chunker;
pub use pdf_page_v1::PdfPageV1Chunker;

// ── Korean morphological tokenizer ───────────────────────────────────────────

use lindera::dictionary::{DictionaryKind, load_embedded_dictionary};
use lindera::mode::Mode;
use lindera::segmenter::Segmenter;
use lindera::tokenizer::Tokenizer;

static KOREAN_TOKENIZER: std::sync::OnceLock<Option<Tokenizer>> = std::sync::OnceLock::new();

/// 한국어 chunk text 를 lindera ko-dic 으로 형태소 분해해 공백 join 한 결과를 반환.
/// chunker 들이 `Chunk.tokenized_korean_text` pre-fill 에 사용.
/// 분석 실패 시 None — 호출자는 NULL fallback 처리.
/// Tokenizer 는 OnceLock 으로 1회 초기화; dict load 실패 시 영구 None.
pub fn tokenize_korean_morphological(text: &str) -> Option<String> {
    if text.trim().is_empty() {
        return None;
    }
    let tokenizer = KOREAN_TOKENIZER.get_or_init(|| {
        let dict = match load_embedded_dictionary(DictionaryKind::KoDic) {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!(target: "kebab-chunk", "tokenize_korean_morphological: dict load failed: {e}");
                return None;
            }
        };
        let segmenter = Segmenter::new(Mode::Normal, dict, None);
        Some(Tokenizer::new(segmenter))
    });
    let tokenizer = tokenizer.as_ref()?;
    let tokens = tokenizer.tokenize(text).ok()?;
    let joined = tokens
        .iter()
        .map(|t| t.surface.as_ref())
        .collect::<Vec<_>>()
        .join(" ");
    if joined.is_empty() {
        None
    } else {
        Some(joined)
    }
}
