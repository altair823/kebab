//! CanonicalDocument, Block, SourceSpan, Inline, plus the forward-declared
//! OCR / caption / transcript stubs (§3.4 + §3.7a).

use serde::{Deserialize, Serialize};

use crate::asset::WorkspacePath;
use crate::ids::{AssetId, BlockId, DocumentId};
use crate::media::Lang;
use crate::metadata::{Metadata, Provenance};
use crate::versions::{ChunkerVersion, EmbeddingVersion, ParserVersion};

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
    /// p9-fb-23: chunker version active when this document was last
    /// chunked. `None` for rows ingested before V006 migration; the
    /// next ingest stamps the current version. Compared against the
    /// active chunker version for the incremental-ingest skip path.
    pub last_chunker_version: Option<ChunkerVersion>,
    /// p9-fb-23: embedding model version active when this document
    /// was last embedded. `None` if no embedder is configured (skip
    /// path treats `None == None` as a match — see design doc).
    pub last_embedding_version: Option<EmbeddingVersion>,
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
    Text { text: String },
    Code { code: String },
    Link { text: String, href: String },
    Strong { children: Vec<Inline> },
    Emph { children: Vec<Inline> },
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
    /// p10-1A-2: AST-unit span for code ingest. Internal storage shape
    /// (chunks.source_spans_json) — `citation_helper` maps this to the
    /// wire `Citation::Code` (added 1A-1). `symbol` is the per-language
    /// self-reference path (design §3.4); `<top-level>` / `<module>` for
    /// glue regions, never null for an identified unit. `lang` is the
    /// canonical code_lang.
    Code {
        line_start: u32,
        line_end: u32,
        symbol: Option<String>,
        lang: Option<String>,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Each `Inline` variant must serialize and deserialize cleanly under
    /// the internally-tagged representation. Newtype-with-primitive variants
    /// (`Text(String)`, `Code(String)`, `Strong(Vec<…>)`, `Emph(Vec<…>)`)
    /// previously failed at serde runtime because `tag = "kind"` cannot
    /// describe a newtype carrying a non-struct value. The struct-variant
    /// shape used here is the §9 schema migration.
    #[test]
    fn source_span_code_round_trips_and_tags_lowercase() {
        let s = SourceSpan::Code {
            line_start: 10,
            line_end: 42,
            symbol: Some("foo::Bar::baz".to_string()),
            lang: Some("rust".to_string()),
        };
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["kind"], "code");
        assert_eq!(v["line_start"], 10);
        assert_eq!(v["line_end"], 42);
        assert_eq!(v["symbol"], "foo::Bar::baz");
        assert_eq!(v["lang"], "rust");
        let back: SourceSpan = serde_json::from_value(v).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn inline_serde_round_trip() {
        let cases = vec![
            Inline::Text { text: "hi".into() },
            Inline::Code { code: "x".into() },
            Inline::Link {
                text: "t".into(),
                href: "h".into(),
            },
            Inline::Strong {
                children: vec![Inline::Text { text: "bold".into() }],
            },
            Inline::Emph {
                children: vec![Inline::Text { text: "em".into() }],
            },
        ];
        for c in cases {
            let s = serde_json::to_string(&c).expect("serialize");
            let back: Inline = serde_json::from_str(&s).expect("deserialize");
            assert_eq!(c, back);
        }
    }
}
