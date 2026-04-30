//! Job repo support types (§3.7a forward-decl, §7.2 JobRepo).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobKind {
    Ingest,
    Chunk,
    Embed,
    Ocr,
    Transcribe,
    Reindex,
    Doctor,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
    Canceled,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct JobId(pub String);

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct JobFilter {
    pub status: Option<JobStatus>,
    pub kind: Option<JobKind>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JobRow {
    pub job_id: JobId,
    pub kind: JobKind,
    pub status: JobStatus,
    pub payload: Value,
    pub progress: Option<Value>,
    pub error: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub finished_at: Option<OffsetDateTime>,
}
