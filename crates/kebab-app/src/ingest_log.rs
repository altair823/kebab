//! Per-ingest-run structured ndjson log writer (v0.20.x ingest log feature).
//!
//! Each `kebab ingest` run produces one `ingest-{run_id}.ndjson` file in
//! `config.logging.ingest_log_dir`. Records are appended line by line; the
//! last record is always `kind="summary"`. `IngestLogWriter::open` returns
//! `Ok(None)` when `ingest_log_enabled = false` so callers need not branch.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use time::format_description::well_known::Rfc3339;

pub struct IngestLogWriter {
    file: BufWriter<File>,
    path: PathBuf,
    run_id: String,
    started_at: SystemTime,
}

impl IngestLogWriter {
    /// Open a new log file. Returns `Ok(None)` when `cfg.ingest_log_enabled == false` (AC-6).
    pub fn open(cfg: &kebab_config::LoggingCfg) -> anyhow::Result<Option<Self>> {
        if !cfg.ingest_log_enabled {
            return Ok(None);
        }
        let run_id = generate_run_id();
        let log_dir = expand_log_dir(&cfg.ingest_log_dir);
        std::fs::create_dir_all(&log_dir)?;
        let path = log_dir.join(format!("ingest-{run_id}.ndjson"));
        let file = BufWriter::new(File::create(&path)?);
        Ok(Some(Self {
            file,
            path,
            run_id,
            started_at: SystemTime::now(),
        }))
    }

    pub fn write_event(&mut self, event: &LogEvent<'_>) -> anyhow::Result<()> {
        serde_json::to_writer(&mut self.file, event)?;
        writeln!(self.file)?;
        Ok(())
    }

    pub fn write_summary(&mut self, summary: &IngestSummary) -> anyhow::Result<()> {
        serde_json::to_writer(&mut self.file, summary)?;
        writeln!(self.file)?;
        Ok(())
    }

    pub fn flush(&mut self) -> anyhow::Result<()> {
        self.file.flush()?;
        Ok(())
    }

    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn started_at(&self) -> SystemTime {
        self.started_at
    }
}

impl Drop for IngestLogWriter {
    fn drop(&mut self) {
        let _ = self.file.flush();
    }
}

/// ISO 8601 compact timestamp + uuid v7 suffix: `20260528T013000Z-abc123de`.
/// uuid v7 is the workspace dep (Cargo.toml); `rand` is not added (spec §6 R-5).
fn generate_run_id() -> String {
    use time::macros::format_description;
    let now = time::OffsetDateTime::now_utc();
    let ts = now
        .format(format_description!(
            "[year][month][day]T[hour][minute][second]Z"
        ))
        .unwrap_or_else(|_| "19700101T000000Z".to_string());
    let uid = uuid::Uuid::now_v7().simple().to_string();
    let suffix = &uid[uid.len() - 8..];
    format!("{ts}-{suffix}")
}

/// Expand `{state_dir}` placeholder → XDG state dir (spec §6 R-3).
/// Other tilde/env expansion is delegated to `kebab_config::expand_path`.
fn expand_log_dir(path: &Path) -> PathBuf {
    let path_str = path.to_string_lossy();
    if path_str.contains("{state_dir}") {
        let state_dir = kebab_config::Config::xdg_state_dir();
        PathBuf::from(path_str.replace("{state_dir}", &state_dir.to_string_lossy()))
    } else {
        path.to_path_buf()
    }
}

/// RFC 3339 UTC timestamp for log records.
#[allow(dead_code)]
pub(crate) fn now_ts() -> String {
    time::OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

/// Ingest event record (ndjson line). `kind` is the discriminator.
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LogEvent<'a> {
    Ocr {
        ts: String,
        doc_path: &'a str,
        page: u32,
        image_byte_size: Option<u64>,
        image_width: Option<u32>,
        image_height: Option<u32>,
        ms: u64,
        chars: u32,
        success: bool,
        reason: Option<&'a str>,
        ocr_engine: &'a str,
    },
    ParseError {
        ts: String,
        doc_path: &'a str,
        reason: &'a str,
        message: &'a str,
    },
    Skip {
        ts: String,
        doc_path: &'a str,
        reason: &'a str,
        detail: Option<&'a str>,
    },
    Error {
        ts: String,
        code: &'a str,
        message: &'a str,
    },
}

/// Final summary record — always the last line of the log file.
/// Explicit `kind` field serializes to `"kind": "summary"`.
#[derive(Serialize, Deserialize)]
pub struct IngestSummary {
    pub kind: String,
    pub ts: String,
    pub run_id: String,
    pub scanned: u32,
    pub new: u32,
    pub errors: u32,
    pub ocr_pages: u32,
    pub ocr_failures: u32,
    pub ocr_p50_ms: Option<u64>,
    pub ocr_p90_ms: Option<u64>,
    pub ocr_max_ms: Option<u64>,
    pub duration_ms: u64,
}

impl IngestSummary {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        ts: String,
        run_id: String,
        scanned: u32,
        new: u32,
        errors: u32,
        ocr_pages: u32,
        ocr_failures: u32,
        ocr_ms_samples: &[u64],
        duration_ms: u64,
    ) -> Self {
        let (p50, p90, max) = percentiles(ocr_ms_samples);
        Self {
            kind: "summary".to_string(),
            ts,
            run_id,
            scanned,
            new,
            errors,
            ocr_pages,
            ocr_failures,
            ocr_p50_ms: p50,
            ocr_p90_ms: p90,
            ocr_max_ms: max,
            duration_ms,
        }
    }
}

/// Simple percentile extraction on a sorted copy of `samples`.
/// Returns `(p50, p90, max)`. All `None` when samples is empty.
pub(crate) fn percentiles(samples: &[u64]) -> (Option<u64>, Option<u64>, Option<u64>) {
    if samples.is_empty() {
        return (None, None, None);
    }
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let n = sorted.len();
    let p50 = sorted[n * 50 / 100];
    let p90 = sorted[n * 90 / 100];
    let max = *sorted.last().unwrap();
    (Some(p50), Some(p90), Some(max))
}

#[cfg(test)]
mod tests {
    use super::*;
    use kebab_config::LoggingCfg;
    use tempfile::TempDir;

    #[test]
    fn generate_run_id_has_iso_prefix_and_8_hex_suffix() {
        let id = generate_run_id();
        // Format: YYYYMMDDTHHmmssZ-xxxxxxxx (total len = 16+1+8 = 25)
        assert_eq!(id.len(), 25, "run_id len should be 25: {id}");
        let (prefix, suffix) = id.split_once('-').expect("run_id should contain '-'");
        assert_eq!(prefix.len(), 16, "prefix should be 16 chars: {prefix}");
        assert!(prefix.contains('T'), "prefix should contain T: {prefix}");
        assert!(prefix.ends_with('Z'), "prefix should end with Z: {prefix}");
        assert_eq!(suffix.len(), 8, "suffix should be 8 chars: {suffix}");
        assert!(
            suffix.chars().all(|c| c.is_ascii_hexdigit()),
            "suffix should be hex: {suffix}"
        );
    }

    #[test]
    fn expand_log_dir_substitutes_state_dir_placeholder() {
        let input = PathBuf::from("{state_dir}/logs");
        let expanded = expand_log_dir(&input);
        let expected = kebab_config::Config::xdg_state_dir().join("logs");
        assert_eq!(expanded, expected);
        assert!(!expanded.to_string_lossy().contains("{state_dir}"));
    }

    #[test]
    fn writer_disabled_returns_none() {
        let cfg = LoggingCfg {
            ingest_log_enabled: false,
            ingest_log_dir: PathBuf::from("/tmp/should-not-exist"),
        };
        let result = IngestLogWriter::open(&cfg).expect("open should not error");
        assert!(result.is_none(), "disabled writer should return None");
    }

    #[test]
    fn writer_writes_one_event_per_line_with_kind_discriminator() {
        let tmp = TempDir::new().unwrap();
        let cfg = LoggingCfg {
            ingest_log_enabled: true,
            ingest_log_dir: tmp.path().to_path_buf(),
        };
        let mut writer = IngestLogWriter::open(&cfg).unwrap().unwrap();
        let path = writer.path().to_path_buf();

        writer
            .write_event(&LogEvent::Skip {
                ts: now_ts(),
                doc_path: "a.zip",
                reason: "builtin_blacklist",
                detail: Some(".zip extension"),
            })
            .unwrap();
        writer
            .write_event(&LogEvent::Error {
                ts: now_ts(),
                code: "ingest_fatal",
                message: "something bad",
            })
            .unwrap();
        writer
            .write_event(&LogEvent::ParseError {
                ts: now_ts(),
                doc_path: "weird.pdf",
                reason: "lopdf_error",
                message: "unexpected EOF",
            })
            .unwrap();
        writer.flush().unwrap();

        let contents = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(lines.len(), 3, "expected 3 lines, got: {}", lines.len());
        for line in &lines {
            assert!(
                line.starts_with('{'),
                "each line should be JSON object: {line}"
            );
            assert!(
                line.contains("\"kind\""),
                "each line should have 'kind': {line}"
            );
        }
    }

    #[test]
    fn drop_flushes_pending_buffer() {
        let tmp = TempDir::new().unwrap();
        let cfg = LoggingCfg {
            ingest_log_enabled: true,
            ingest_log_dir: tmp.path().to_path_buf(),
        };
        let mut writer = IngestLogWriter::open(&cfg).unwrap().unwrap();
        let path = writer.path().to_path_buf();
        writer
            .write_event(&LogEvent::Error {
                ts: now_ts(),
                code: "test",
                message: "drop flush test",
            })
            .unwrap();
        // Drop without explicit flush — Drop impl should flush BufWriter.
        drop(writer);
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(
            contents.lines().count() >= 1,
            "file should have at least 1 line after drop"
        );
    }
}
