//! p9-fb-35 verbatim fetch implementation.
//!
//! [`App::fetch`] is the facade entry point. It dispatches on
//! [`FetchQuery`] variants:
//!
//! - `Chunk(id)` — return the chunk row from `chunks.text`, optionally
//!   with ±N surrounding chunks (`FetchOpts::context`).
//! - `Doc(id)` — return the entire document re-serialized to markdown.
//!   (Implemented in Task 4.)
//! - `Span { doc_id, line_start, line_end }` — return a contiguous line
//!   slice. (Implemented in Task 5.)
//!
//! Errors are surfaced as [`StructuredError`] (anyhow-friendly wrapper
//! around `ErrorV1`) so the CLI / MCP wire layer's `classify` keeps the
//! typed `code` (`chunk_not_found` / `doc_not_found` /
//! `span_not_supported`) instead of falling through to `code =
//! "generic"`.

use anyhow::Result;
use time::OffsetDateTime;

use kebab_core::{
    Block, CanonicalDocument, Chunk, ChunkId, DocumentId, DocumentStore, FetchKind, FetchOpts,
    FetchQuery, FetchResult,
};

use crate::App;
use crate::error_wire::{ERROR_V1_ID, ErrorV1, StructuredError};
use crate::staleness::compute_stale;

impl App {
    /// p9-fb-35: verbatim fetch facade. Returns text from
    /// `chunks.text` / `CanonicalDocument` based on the requested
    /// mode. Errors surface as `StructuredError(ErrorV1)` with one
    /// of `chunk_not_found` / `doc_not_found` / `span_not_supported`
    /// so the wire-layer classifier preserves the typed code.
    pub fn fetch(&self, query: FetchQuery, opts: FetchOpts) -> Result<FetchResult> {
        match query {
            FetchQuery::Chunk(id) => fetch_chunk(self, id, opts),
            FetchQuery::Doc(id) => fetch_doc(self, id, opts),
            FetchQuery::Span {
                doc_id,
                line_start,
                line_end,
            } => fetch_span(self, doc_id, line_start, line_end, opts),
        }
    }
}

fn fetch_chunk(app: &App, id: ChunkId, opts: FetchOpts) -> Result<FetchResult> {
    let target = <kebab_store_sqlite::SqliteStore as DocumentStore>::get_chunk(&app.sqlite, &id)?
        .ok_or_else(|| {
            anyhow::Error::new(StructuredError(ErrorV1 {
                schema_version: ERROR_V1_ID.to_string(),
                code: "chunk_not_found".to_string(),
                message: format!("chunk_id '{}' not found", id.0),
                details: serde_json::Value::Null,
                hint: None,
            }))
        })?;

    let doc_id = target.doc_id.clone();
    let doc =
        <kebab_store_sqlite::SqliteStore as DocumentStore>::get_document(&app.sqlite, &doc_id)?
            .ok_or_else(|| {
                anyhow::Error::new(StructuredError(ErrorV1 {
                    schema_version: ERROR_V1_ID.to_string(),
                    code: "doc_not_found".to_string(),
                    message: format!(
                        "doc_id '{}' (parent of chunk '{}') not found",
                        doc_id.0, id.0
                    ),
                    details: serde_json::Value::Null,
                    hint: None,
                }))
            })?;

    let (context_before, context_after) = match opts.context {
        Some(n) if n > 0 => surrounding_chunks(app, &doc_id, &id, n)?,
        _ => (Vec::new(), Vec::new()),
    };

    let now = OffsetDateTime::now_utc();
    let stale = compute_stale(
        doc_metadata_updated_at(&doc),
        now,
        app.config.search.stale_threshold_days,
    );

    Ok(FetchResult {
        kind: FetchKind::Chunk,
        doc_id: doc.doc_id.clone(),
        doc_path: doc.workspace_path.clone(),
        indexed_at: doc_metadata_updated_at(&doc),
        stale,
        chunk: Some(target),
        context_before,
        context_after,
        text: None,
        line_start: None,
        line_end: None,
        effective_end: None,
        truncated: false,
    })
}

fn fetch_doc(_app: &App, _id: DocumentId, _opts: FetchOpts) -> Result<FetchResult> {
    // Implemented in Task 4.
    anyhow::bail!("fetch_doc not yet implemented")
}

fn fetch_span(
    _app: &App,
    _id: DocumentId,
    _line_start: u32,
    _line_end: u32,
    _opts: FetchOpts,
) -> Result<FetchResult> {
    // Implemented in Task 5.
    anyhow::bail!("fetch_span not yet implemented")
}

/// p9-fb-35: list chunks for a document in ordinal order, return
/// `(before, after)` slices around the target chunk_id. `n` caps each
/// side independently — the worst case is `2n` total neighbors when
/// the target sits in the middle of the doc.
fn surrounding_chunks(
    app: &App,
    doc_id: &DocumentId,
    target: &ChunkId,
    n: u32,
) -> Result<(Vec<Chunk>, Vec<Chunk>)> {
    let chunks = list_chunks_in_order(app, doc_id)?;
    let target_idx = chunks
        .iter()
        .position(|c| c.chunk_id == *target)
        .ok_or_else(|| anyhow::anyhow!("chunk not found in doc chunk list"))?;
    let n = n as usize;
    let lo = target_idx.saturating_sub(n);
    let hi = (target_idx + n + 1).min(chunks.len());
    let before: Vec<Chunk> = chunks[lo..target_idx].to_vec();
    let after: Vec<Chunk> = chunks[target_idx + 1..hi].to_vec();
    Ok((before, after))
}

/// p9-fb-35: chunks have no explicit ordinal column, so the underlying
/// helper sorts by `(created_at, chunk_id)` which matches insertion
/// order produced by the chunker (deterministic). The actual SQL lives
/// inside `kebab-store-sqlite` (`SqliteStore::list_chunk_ids_for_doc`)
/// to keep the facade crate free of direct rusqlite usage.
fn list_chunks_in_order(app: &App, doc_id: &DocumentId) -> Result<Vec<Chunk>> {
    let chunk_ids = app.sqlite.list_chunk_ids_for_doc(doc_id)?;
    let mut out: Vec<Chunk> = Vec::with_capacity(chunk_ids.len());
    for cid in chunk_ids {
        if let Some(chunk) =
            <kebab_store_sqlite::SqliteStore as DocumentStore>::get_chunk(&app.sqlite, &cid)?
        {
            out.push(chunk);
        }
    }
    Ok(out)
}

fn doc_metadata_updated_at(doc: &CanonicalDocument) -> OffsetDateTime {
    doc.metadata.updated_at
}

/// p9-fb-35: serialize a `CanonicalDocument` back to markdown. Best-
/// effort round-trip — inline-styled spans (Strong/Emph children)
/// flatten to plain text via the already-flattened `TextBlock.text`
/// field. Good enough for an agent reading verbatim context. Used by
/// Task 4 (doc mode) and Task 5 (span mode).
//
// The first caller lands in Task 4 (`fetch_doc`); silence the
// stop-gap dead-code warning until then so this Task 3 commit lands
// with a clean clippy run.
#[allow(dead_code)]
pub(crate) fn fmt_canonical_to_markdown(doc: &CanonicalDocument) -> String {
    let mut out = String::with_capacity(1024);
    for (i, block) in doc.blocks.iter().enumerate() {
        if i > 0 {
            out.push_str("\n\n");
        }
        match block {
            Block::Heading(h) => {
                let level = h.level.clamp(1, 6) as usize;
                for _ in 0..level {
                    out.push('#');
                }
                out.push(' ');
                out.push_str(&h.text);
            }
            Block::Paragraph(t) => out.push_str(&t.text),
            Block::Quote(t) => {
                // Prefix every line with `> ` so block-quote round-trips.
                for (li, line) in t.text.split('\n').enumerate() {
                    if li > 0 {
                        out.push('\n');
                    }
                    out.push_str("> ");
                    out.push_str(line);
                }
            }
            Block::List(l) => {
                for (idx, item) in l.items.iter().enumerate() {
                    if idx > 0 {
                        out.push('\n');
                    }
                    if l.ordered {
                        out.push_str(&format!("{}. {}", idx + 1, item.text));
                    } else {
                        out.push_str(&format!("- {}", item.text));
                    }
                }
            }
            Block::Code(c) => {
                out.push_str("```");
                if let Some(lang) = &c.lang {
                    out.push_str(lang);
                }
                out.push('\n');
                out.push_str(&c.code);
                if !c.code.ends_with('\n') {
                    out.push('\n');
                }
                out.push_str("```");
            }
            Block::Table(t) => {
                out.push_str(&t.headers.join(" | "));
                out.push('\n');
                // Markdown table separator — N copies of `---|` is
                // acceptable for a verbatim re-serialization (renderer
                // tolerates trailing pipe).
                out.push_str(&"---|".repeat(t.headers.len()));
                for row in &t.rows {
                    out.push('\n');
                    out.push_str(&row.join(" | "));
                }
            }
            Block::ImageRef(img) => {
                out.push_str(&format!("![{}]({})", img.alt, img.src));
            }
            Block::AudioRef(_a) => {
                // Canonical doc carries the transcript on AudioRefBlock,
                // but markdown has no native audio embed. Emit a stub
                // marker so the agent sees something ran here.
                out.push_str("(audio reference)");
            }
        }
    }
    out
}

/// p9-fb-35: free-function entry for CLI / MCP. Mirrors the
/// `*_with_config` pattern documented in the kebab-app crate root —
/// `kebab-cli` calls this so a `--config <path>` flag is honored.
#[doc(hidden)]
pub fn fetch_with_config(
    config: kebab_config::Config,
    query: FetchQuery,
    opts: FetchOpts,
) -> Result<FetchResult> {
    App::open_with_config(config)?.fetch(query, opts)
}
