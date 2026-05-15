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

    /// p10-1A-1: name of the source repo if the file lives inside a git
    /// working tree (`.git/` walk-up). null otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,

    /// p10-1A-1: HEAD branch at ingest time. null when no repo or detached HEAD.
    /// Informational only — current-state observability, not a partition key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_branch: Option<String>,

    /// p10-1A-1: HEAD commit (40-hex) at ingest time. null when no repo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_commit: Option<String>,

    /// p10-1A-1: programming language identifier (lowercase canonical). null
    /// for markdown / pdf / image. Set by `kebab_parse_code::lang::code_lang_for_path`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_lang: Option<String>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_repo_fields_default_to_none_and_omit_when_serialized() {
        let m = Metadata {
            aliases: vec![],
            tags: vec![],
            created_at: time::OffsetDateTime::UNIX_EPOCH,
            updated_at: time::OffsetDateTime::UNIX_EPOCH,
            source_type: SourceType::Markdown,
            trust_level: TrustLevel::Primary,
            user_id_alias: None,
            user: Default::default(),
            repo: None,
            git_branch: None,
            git_commit: None,
            code_lang: None,
        };
        let v = serde_json::to_value(&m).unwrap();
        assert!(v.get("repo").is_none());
        assert!(v.get("git_branch").is_none());
        assert!(v.get("git_commit").is_none());
        assert!(v.get("code_lang").is_none());
    }

    #[test]
    fn metadata_repo_fields_present_when_some() {
        let m = Metadata {
            aliases: vec![],
            tags: vec![],
            created_at: time::OffsetDateTime::UNIX_EPOCH,
            updated_at: time::OffsetDateTime::UNIX_EPOCH,
            source_type: SourceType::Markdown,
            trust_level: TrustLevel::Primary,
            user_id_alias: None,
            user: Default::default(),
            repo: Some("kebab".into()),
            git_branch: Some("main".into()),
            git_commit: Some("a".repeat(40)),
            code_lang: Some("rust".into()),
        };
        let v = serde_json::to_value(&m).unwrap();
        assert_eq!(v["repo"], "kebab");
        assert_eq!(v["git_branch"], "main");
        assert_eq!(v["git_commit"].as_str().unwrap().len(), 40);
        assert_eq!(v["code_lang"], "rust");
    }
}
