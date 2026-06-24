//! `kb-chunk` — chunkers that emit [`kebab_core::Chunk`] batches.
//!
//! Per design §3.5 (Chunk), §4.2 (chunk_id recipe), §7.2 (`Chunker`
//! trait), §0 Q3/§14 (chunking priority).
//!
//! Public surface:
//!
//! * [`MdHeadingV1Chunker`] — heading-aware chunker for Markdown
//!   `CanonicalDocument`s, emitting `chunker_version = "md-heading-v1"`.
//! * [`MdHeadingV2Chunker`] — byte-identical to v1 in its chunking pass,
//!   then applies a generic post-pass: any chunk whose byte/3 estimate
//!   exceeds `max_chunk_tokens` is split at line (then UTF-8 char)
//!   boundaries. Covers all block kinds (list, code, paragraph, table).
//!   Emits `chunker_version = "md-heading-v2"`; the hardcoded markdown
//!   default (design §9 label bump).
//!
//! Behavior contract is enumerated on [`MdHeadingV1Chunker`] (v2 inherits
//! it; the divergence is the generic post-pass documented on [`MdHeadingV2Chunker`]).
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
mod md_heading_v2;
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
pub use md_heading_v2::MdHeadingV2Chunker;
pub use pdf_page_v1::PdfPageV1Chunker;

// ── Korean morphological tokenizer ───────────────────────────────────────────

use lindera::dictionary::{DictionaryKind, load_embedded_dictionary};
use lindera::mode::Mode;
use lindera::segmenter::Segmenter;
use lindera::tokenizer::Tokenizer;

static KOREAN_TOKENIZER: std::sync::OnceLock<Option<Tokenizer>> = std::sync::OnceLock::new();

/// 한 codepoint 가 한글 음절 또는 자모인지 판정 — N-gram supplement 의 emit 대상 필터링.
fn is_hangul(c: char) -> bool {
    matches!(
        c,
        '\u{AC00}'..='\u{D7A3}'  // 한글 음절 (precomposed)
        | '\u{1100}'..='\u{11FF}' // 한글 자모
        | '\u{3130}'..='\u{318F}' // 한글 호환 자모
    )
}

/// 한국어 chunk text 를 lindera ko-dic 으로 형태소 분해해 공백 join 한 결과를 반환.
/// chunker 들이 `Chunk.tokenized_korean_text` pre-fill 에 사용.
/// 분석 실패 시 None — 호출자는 NULL fallback 처리.
/// Tokenizer 는 OnceLock 으로 1회 초기화; dict load 실패 시 영구 None.
///
/// v0.21.0 — N-gram supplement (Option β, post-v0.20.1 enhancement).
/// ko-dic 가 compound noun (`한국정부`, `서울특별시` 등) 을 단일 token 으로
/// 저장하는 정책 의 한계 해소 — morpheme 길이 ≥ 3 인 한글 token 에 대해
/// 2-char sliding window n-gram 도 추가 emit. `'한국정부'` morpheme →
/// `[한국정부, 한국, 국정, 정부]` 의 4 token 으로 expand. 사용자 의 2-char
/// query (`'한국'`) 가 compound chunk 에서도 hit. 영어/숫자 token 은 영향
/// 없음 (is_hangul filter). DB size + ingest latency 의 trade-off 는
/// HOTFIXES 2026-05-28 의 "N-gram supplement (Option β)" 보강 entry.
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

    let mut out_tokens: Vec<String> = Vec::with_capacity(tokens.len() * 2);
    for tok in tokens.iter() {
        let surface = tok.surface.as_ref();
        out_tokens.push(surface.to_string());

        // N-gram supplement: 한글 morpheme 의 2-char sliding window.
        let chars: Vec<char> = surface.chars().collect();
        if chars.len() >= 3 && chars.iter().all(|c| is_hangul(*c)) {
            for window in chars.windows(2) {
                out_tokens.push(window.iter().collect());
            }
        }
    }

    let joined = out_tokens.join(" ");
    if joined.is_empty() {
        None
    } else {
        Some(joined)
    }
}
