//! Streaming progress events for `ingest_with_config_progress`.
//!
//! The facade emits one [`IngestEvent`] per step boundary into an
//! optional `mpsc::Sender<IngestEvent>` injected by the caller. CLI
//! (`p9-fb-02`), TUI (`p9-fb-03`), and future desktop UI all consume the
//! same stream — CLI dumps it as line-delimited JSON
//! (`ingest_progress.v1`), TUI feeds it into a status-bar reducer, and
//! anyone else can plug in their own receiver.
//!
//! Send is **best-effort**: a receiver that has been dropped is treated
//! as a no-op, never as an error. The ingest hot path must not stall
//! on a slow consumer.
//!
//! Cancellation lands in `p9-fb-04` and adds `IngestEvent::Aborted`
//! emission; this task only ever emits `Completed`.

use serde::{Deserialize, Serialize};

use kebab_core::IngestItemKind;

/// Aggregate counters surfaced on the terminal `Completed` (and, in
/// `p9-fb-04`, `Aborted`) events. Mirrors the fields persisted into
/// `ingest_runs.progress_json` so external tooling can reconstruct the
/// run's outcome from either side.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct AggregateCounts {
    pub scanned: u32,
    pub new: u32,
    pub updated: u32,
    pub skipped: u32,
    /// p9-fb-23: assets whose checksum + all version inputs matched the
    /// existing DB record — parse / chunk / embed / vector upsert all
    /// skipped.
    pub unchanged: u32,
    pub errors: u32,
    pub chunks_indexed: u32,
    pub embeddings_indexed: u32,
    /// p9-fb-25: per-extension skip count. See [`IngestReport::skipped_by_extension`].
    pub skipped_by_extension: std::collections::BTreeMap<String, u32>,
}

/// One streaming progress event. The CLI's `--json` mode serializes this
/// into the wire-stable `ingest_progress.v1` schema; in-memory consumers
/// (TUI / desktop) take the typed value directly.
///
/// Ordering invariant per design §2.4a:
///
/// ```text
/// ScanStarted < ScanCompleted
///   < ( AssetStarted
///         [< (PdfOcrStarted < PdfOcrFinished)*]
///         [< AssetChunked]
///         [< AssetTimings]
///       < AssetFinished )*
///   < (Completed | Aborted)
/// ```
///
/// `[]` = optional. `PdfOcr*` is per-PDF asset only (v0.20.0 sub-item 1).
/// `AssetChunked` / `AssetTimings` are the v0.24.0 asset-internal phase
/// events: `AssetChunked` fires once right after chunking (markdown /
/// image / PDF); `AssetTimings` reports per-phase wall-clock once
/// (markdown only).
///
/// Embed-batch events (`embed_batch_started` / `embed_batch_finished`
/// in §2.4a) are reserved for a future iteration and are not emitted
/// by this task; the spec calls them out as "임의 위치" (optional).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum IngestEvent {
    /// Workspace walk has started. Emitted before the connector scan
    /// returns, so consumers can paint a "scanning…" state immediately
    /// even if the workspace is large enough that the scan takes time.
    ScanStarted { root: String },
    /// Scan finished; `total` assets are about to be processed.
    ScanCompleted { total: u32 },
    /// About to process the `idx`-th asset (1-based). `media` is a
    /// short label (`markdown` / `pdf` / `image` / `audio` / `other`).
    AssetStarted {
        idx: u32,
        total: u32,
        path: String,
        media: String,
    },
    /// Finished processing the `idx`-th asset. `result` mirrors the
    /// asset's `IngestItemKind`; `chunks` is the number of chunks
    /// produced (0 for `Skipped` / `Error`).
    AssetFinished {
        idx: u32,
        total: u32,
        result: IngestItemKind,
        chunks: u32,
    },
    /// v0.24.0 (additive): emitted right after an asset is chunked, before
    /// expansion / embed / store. Surfaces "this document is N chunks"
    /// immediately so a single large document no longer looks frozen at
    /// `idx/total` while its per-chunk phases churn. `chunks` is the chunk
    /// count for asset `idx`.
    AssetChunked { idx: u32, total: u32, chunks: u32 },
    /// v0.24.0 (additive): per-phase wall-clock (milliseconds) for asset
    /// `idx`, emitted once the asset's markdown pipeline finishes. Lets a
    /// user see *where* the time went (parse / chunk / embed / store)
    /// without parsing logs. Only the markdown path emits this; the
    /// image / PDF paths surface `AssetChunked` but skip phase timing (their
    /// phase shapes differ — OCR / caption). `expansion_ms` is retained for
    /// wire compatibility but is always 0 since doc-side expansion was
    /// removed (HOTFIXES 2026-06-03).
    AssetTimings {
        idx: u32,
        total: u32,
        parse_ms: u64,
        chunk_ms: u64,
        expansion_ms: u64,
        embed_ms: u64,
        store_ms: u64,
    },
    /// Run finished normally. `counts` is the final aggregate.
    Completed { counts: AggregateCounts },
    /// Run finished by user cancellation. `counts` is the partial
    /// aggregate at the cancel boundary. Emitted by `p9-fb-04`; this
    /// task never produces `Aborted`.
    Aborted { counts: AggregateCounts },
    /// PDF page 별 OCR 시작 시 emit. v0.20.0 sub-item 1.
    PdfOcrStarted { page: u32 },
    /// PDF page 별 OCR 종료 시 emit. v0.20.0 sub-item 1.
    /// `skipped` = `true` 일 시 OCR 미수행 (DCTDecode 부재 또는 engine 실패).
    /// `chars = 0` 만으로는 "skip" 과 "0-char OCR result" 구분 불가, `skipped` field 가 명시적.
    PdfOcrFinished {
        page: u32,
        ms: u64,
        chars: u32,
        ocr_engine: String,
        skipped: bool,
        /// v0.20.x ingest log: raster image byte size (additive minor, optional).
        #[serde(skip_serializing_if = "Option::is_none")]
        image_byte_size: Option<u64>,
        /// v0.20.x ingest log: raster image width in pixels (additive minor, optional).
        #[serde(skip_serializing_if = "Option::is_none")]
        image_width: Option<u32>,
        /// v0.20.x ingest log: raster image height in pixels (additive minor, optional).
        #[serde(skip_serializing_if = "Option::is_none")]
        image_height: Option<u32>,
        /// v0.20.x ingest log: OCR failure reason (additive minor, optional).
        #[serde(skip_serializing_if = "Option::is_none")]
        failure_reason: Option<String>,
    },
}

/// Map a `MediaType` to the short label used by `IngestEvent::AssetStarted`.
/// Mirrors the §2.4a description text — `markdown` / `pdf` / `image` /
/// `audio` / `other`.
pub fn media_label(media: &kebab_core::MediaType) -> &'static str {
    match media {
        kebab_core::MediaType::Markdown => "markdown",
        kebab_core::MediaType::Pdf => "pdf",
        kebab_core::MediaType::Image(_) => "image",
        kebab_core::MediaType::Audio(_) => "audio",
        kebab_core::MediaType::Code(_) => "code",
        kebab_core::MediaType::Other(_) => "other",
    }
}

/// p9-fb-25: render `": A docx, B txt"` breakdown after the
/// `N skipped` count when the map is non-empty. Empty → empty
/// string (no extra punctuation). desc sort by count, ties broken
/// by key alphabetic.
pub fn render_skipped_breakdown(map: &std::collections::BTreeMap<String, u32>) -> String {
    if map.is_empty() {
        return String::new();
    }
    let mut entries: Vec<_> = map.iter().collect();
    entries.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
    let parts: Vec<String> = entries.iter().map(|(k, v)| format!("{v} {k}")).collect();
    format!(": {}", parts.join(", "))
}

/// Best-effort send into an optional `mpsc::Sender`. A dropped receiver
/// is silently absorbed — the ingest hot path must not stall on a slow
/// consumer. Logged at `trace` for diagnostics.
pub(crate) fn emit(progress: Option<&std::sync::mpsc::Sender<IngestEvent>>, event: IngestEvent) {
    if let Some(tx) = progress {
        if tx.send(event).is_err() {
            tracing::trace!(
                target: "kebab-app::ingest_progress",
                "progress receiver dropped; event discarded (best-effort send per ingest_progress contract)"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kebab_core::MediaType;

    #[test]
    fn media_label_covers_every_variant() {
        assert_eq!(media_label(&MediaType::Markdown), "markdown");
        assert_eq!(media_label(&MediaType::Pdf), "pdf");
        assert_eq!(
            media_label(&MediaType::Image(kebab_core::ImageType::Png)),
            "image"
        );
        assert_eq!(
            media_label(&MediaType::Audio(kebab_core::AudioType::Wav)),
            "audio"
        );
        assert_eq!(media_label(&MediaType::Code("rust".into())), "code");
        assert_eq!(media_label(&MediaType::Other("x".into())), "other");
    }

    #[test]
    fn ingest_event_serializes_with_discriminator() {
        // The `#[serde(tag = "kind", rename_all = "snake_case")]`
        // attribute mirrors §2.4a's wire shape — the CLI's wire layer
        // re-tags with `schema_version` on top.
        let ev = IngestEvent::AssetStarted {
            idx: 1,
            total: 10,
            path: "notes/foo.md".into(),
            media: "markdown".into(),
        };
        let v = serde_json::to_value(&ev).unwrap();
        assert_eq!(
            v.get("kind").and_then(|s| s.as_str()),
            Some("asset_started")
        );
        assert_eq!(v.get("idx").and_then(serde_json::Value::as_u64), Some(1));
        assert_eq!(v.get("total").and_then(serde_json::Value::as_u64), Some(10));
        assert_eq!(v.get("path").and_then(|s| s.as_str()), Some("notes/foo.md"));
        assert_eq!(v.get("media").and_then(|s| s.as_str()), Some("markdown"));
    }

    #[test]
    fn asset_chunked_serializes_with_discriminator() {
        // v0.24.0 additive variant — `kind` must be snake_case
        // `asset_chunked` so wire v1 consumers branch on it cleanly.
        let ev = IngestEvent::AssetChunked {
            idx: 3,
            total: 10,
            chunks: 142,
        };
        let v = serde_json::to_value(&ev).unwrap();
        assert_eq!(
            v.get("kind").and_then(|s| s.as_str()),
            Some("asset_chunked")
        );
        assert_eq!(v.get("idx").and_then(serde_json::Value::as_u64), Some(3));
        assert_eq!(
            v.get("chunks").and_then(serde_json::Value::as_u64),
            Some(142)
        );
    }

    #[test]
    fn asset_timings_serializes_all_phase_fields() {
        let ev = IngestEvent::AssetTimings {
            idx: 2,
            total: 7,
            parse_ms: 12,
            chunk_ms: 3,
            expansion_ms: 45_000,
            embed_ms: 800,
            store_ms: 20,
        };
        let v = serde_json::to_value(&ev).unwrap();
        assert_eq!(
            v.get("kind").and_then(|s| s.as_str()),
            Some("asset_timings")
        );
        // All five phase fields are present (plain u64, always serialized).
        for (field, want) in [
            ("parse_ms", 12u64),
            ("chunk_ms", 3),
            ("expansion_ms", 45_000),
            ("embed_ms", 800),
            ("store_ms", 20),
        ] {
            assert_eq!(
                v.get(field).and_then(serde_json::Value::as_u64),
                Some(want),
                "field {field}"
            );
        }
    }

    #[test]
    fn ingest_event_completed_has_counts() {
        let ev = IngestEvent::Completed {
            counts: AggregateCounts {
                scanned: 5,
                new: 2,
                ..Default::default()
            },
        };
        let v = serde_json::to_value(&ev).unwrap();
        assert_eq!(v.get("kind").and_then(|s| s.as_str()), Some("completed"));
        let counts = v.get("counts").unwrap();
        assert_eq!(
            counts.get("scanned").and_then(serde_json::Value::as_u64),
            Some(5)
        );
        assert_eq!(
            counts.get("new").and_then(serde_json::Value::as_u64),
            Some(2)
        );
    }

    #[test]
    fn emit_with_no_sender_is_noop() {
        // Compiles + does not panic. Doc-test of the contract.
        emit(None, IngestEvent::ScanStarted { root: "/x".into() });
    }

    #[test]
    fn emit_with_dropped_receiver_does_not_panic() {
        let (tx, rx) = std::sync::mpsc::channel::<IngestEvent>();
        drop(rx);
        emit(Some(&tx), IngestEvent::ScanStarted { root: "/x".into() });
    }

    #[test]
    fn emit_delivers_event_to_live_receiver() {
        let (tx, rx) = std::sync::mpsc::channel::<IngestEvent>();
        emit(Some(&tx), IngestEvent::ScanCompleted { total: 42 });
        match rx.try_recv().unwrap() {
            IngestEvent::ScanCompleted { total } => assert_eq!(total, 42),
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn render_skipped_breakdown_desc_sort_with_tiebreak() {
        use std::collections::BTreeMap;
        let mut m = BTreeMap::new();
        assert_eq!(render_skipped_breakdown(&m), "");
        m.insert("txt".to_string(), 1);
        m.insert("docx".to_string(), 2);
        m.insert("epub".to_string(), 1);
        // 2 docx 먼저 (count desc), 그 다음 1 epub / 1 txt 는 alphabetic.
        assert_eq!(
            render_skipped_breakdown(&m),
            ": 2 docx, 1 epub, 1 txt".to_string()
        );
    }
}
