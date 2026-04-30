//! Metadata + Provenance (§3.6).

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Metadata {
    pub aliases: Vec<String>,
    pub tags: Vec<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    pub source_type: SourceType,
    pub trust_level: TrustLevel,
    pub user_id_alias: Option<String>,
    /// Frontmatter keys we don't recognise are preserved here per §0 Q9.
    pub user: Map<String, Value>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceType {
    Markdown,
    Note,
    Paper,
    Reference,
    Inbox,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrustLevel {
    Primary,
    Secondary,
    Generated,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Provenance {
    pub events: Vec<ProvenanceEvent>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProvenanceEvent {
    #[serde(with = "time::serde::rfc3339")]
    pub at: OffsetDateTime,
    pub agent: String,
    pub kind: ProvenanceKind,
    pub note: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProvenanceKind {
    Discovered,
    Parsed,
    Normalized,
    Chunked,
    OcrApplied,
    CaptionApplied,
    Transcribed,
    Embedded,
    Indexed,
    Warning,
    Error,
}
