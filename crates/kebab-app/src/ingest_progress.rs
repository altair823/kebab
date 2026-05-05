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
/// ScanStarted < ScanCompleted < (AssetStarted < AssetFinished)*
///                             < (Completed | Aborted)
/// ```
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
    /// Run finished normally. `counts` is the final aggregate.
    Completed { counts: AggregateCounts },
    /// Run finished by user cancellation. `counts` is the partial
    /// aggregate at the cancel boundary. Emitted by `p9-fb-04`; this
    /// task never produces `Aborted`.
    Aborted { counts: AggregateCounts },
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
pub(crate) fn emit(
    progress: Option<&std::sync::mpsc::Sender<IngestEvent>>,
    event: IngestEvent,
) {
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
        assert_eq!(v.get("kind").and_then(|s| s.as_str()), Some("asset_started"));
        assert_eq!(v.get("idx").and_then(|n| n.as_u64()), Some(1));
        assert_eq!(v.get("total").and_then(|n| n.as_u64()), Some(10));
        assert_eq!(v.get("path").and_then(|s| s.as_str()), Some("notes/foo.md"));
        assert_eq!(v.get("media").and_then(|s| s.as_str()), Some("markdown"));
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
        assert_eq!(counts.get("scanned").and_then(|n| n.as_u64()), Some(5));
        assert_eq!(counts.get("new").and_then(|n| n.as_u64()), Some(2));
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
