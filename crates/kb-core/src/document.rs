//! CanonicalDocument, Block, SourceSpan, Inline, plus the forward-declared
//! OCR / caption / transcript stubs (§3.4 + §3.7a).

use serde::{Deserialize, Serialize};

use crate::asset::WorkspacePath;
use crate::ids::{AssetId, BlockId, DocumentId};
use crate::media::Lang;
use crate::metadata::{Metadata, Provenance};
use crate::versions::ParserVersion;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CanonicalDocument {
    pub doc_id: DocumentId,
    pub source_asset_id: AssetId,
    pub workspace_path: WorkspacePath,
    pub title: String,
    pub lang: Lang,
    pub blocks: Vec<Block>,
    pub metadata: Metadata,
    pub provenance: Provenance,
    pub parser_version: ParserVersion,
    pub schema_version: u32,
    pub doc_version: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", tag = "kind")]
pub enum Block {
    Heading(HeadingBlock),
    Paragraph(TextBlock),
    List(ListBlock),
    Code(CodeBlock),
    Table(TableBlock),
    Quote(TextBlock),
    ImageRef(ImageRefBlock),
    AudioRef(AudioRefBlock),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommonBlock {
    pub block_id: BlockId,
    pub heading_path: Vec<String>,
    pub source_span: SourceSpan,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HeadingBlock {
    pub common: CommonBlock,
    pub level: u8,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextBlock {
    pub common: CommonBlock,
    pub text: String,
    pub inlines: Vec<Inline>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ListBlock {
    pub common: CommonBlock,
    pub ordered: bool,
    pub items: Vec<TextBlock>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CodeBlock {
    pub common: CommonBlock,
    pub lang: Option<String>,
    pub code: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TableBlock {
    pub common: CommonBlock,
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImageRefBlock {
    pub common: CommonBlock,
    pub asset_id: Option<AssetId>,
    pub src: String,
    pub alt: String,
    pub ocr: Option<OcrText>,
    pub caption: Option<ModelCaption>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioRefBlock {
    pub common: CommonBlock,
    pub asset_id: AssetId,
    pub duration_ms: u64,
    pub transcript: Option<Transcript>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", tag = "kind")]
pub enum Inline {
    Text(String),
    Code(String),
    Link { text: String, href: String },
    Strong(Vec<Inline>),
    Emph(Vec<Inline>),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", tag = "kind")]
pub enum SourceSpan {
    Line {
        start: u32,
        end: u32,
    },
    Byte {
        start: u64,
        end: u64,
    },
    Page {
        page: u32,
        char_start: Option<u32>,
        char_end: Option<u32>,
    },
    Region {
        x: u32,
        y: u32,
        w: u32,
        h: u32,
    },
    Time {
        start_ms: u64,
        end_ms: u64,
    },
}

// ── Forward-declared stubs (§3.7a). Bodies are final per design. ────────

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OcrText {
    pub joined: String,
    pub regions: Vec<OcrRegion>,
    pub engine: String,
    pub engine_version: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OcrRegion {
    pub bbox: (u32, u32, u32, u32),
    pub text: String,
    pub confidence: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelCaption {
    pub text: String,
    pub model: String,
    pub model_version: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Transcript {
    pub segments: Vec<TranscriptSegment>,
    pub engine: String,
    pub engine_version: String,
    pub language: Lang,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TranscriptSegment {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    pub speaker: Option<String>,
    pub confidence: Option<f32>,
}
