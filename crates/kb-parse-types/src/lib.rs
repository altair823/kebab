//! `kb-parse-types` — parser intermediate representations (§3.7b).
//!
//! Depends ONLY on `kb-core`. Must NOT depend on any parser library
//! (`pulldown-cmark`, `pdf-extract`, `image`, `whisper-rs`, …) and must
//! NOT depend on any other `kb-*` crate.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ParsedBlock {
    pub kind: ParsedBlockKind,
    pub heading_path: Vec<String>,
    pub source_span: kb_core::SourceSpan,
    pub payload: ParsedPayload,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParsedBlockKind {
    Heading,
    Paragraph,
    List,
    Code,
    Table,
    Quote,
    ImageRef,
    AudioRef,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", tag = "kind")]
pub enum ParsedPayload {
    Heading {
        level: u8,
        text: String,
    },
    Paragraph {
        text: String,
        inlines: Vec<kb_core::Inline>,
    },
    List {
        ordered: bool,
        items: Vec<Vec<kb_core::Inline>>,
    },
    Code {
        lang: Option<String>,
        code: String,
    },
    Table {
        headers: Vec<String>,
        rows: Vec<Vec<String>>,
    },
    Quote {
        text: String,
        inlines: Vec<kb_core::Inline>,
    },
    ImageRef {
        src: String,
        alt: String,
    },
    /// `duration_ms` is filled in by the extractor before chunking — see
    /// design §3.7b.
    AudioRef {
        src: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Warning {
    pub kind: WarningKind,
    pub note: String,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WarningKind {
    MalformedFrontmatter,
    MalformedTable,
    EncodingFallback,
    ExtractFailed,
}

// Forward-declared (P6/P7/P8). Bodies stay minimal for now.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ParsedImageRegion;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ParsedPdfPage {
    pub page: u32,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ParsedAudioSegment {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}
