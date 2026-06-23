//! `kebab-parse-pdf` — text PDF extractor (P7-1).
//!
//! Implements [`kebab_core::Extractor`] for [`MediaType::Pdf`]. Extracts
//! text page-by-page via `lopdf`'s per-page API and emits one
//! [`Block::Paragraph`] per page with [`SourceSpan::Page`] (1-based page,
//! `char_start = 0`, `char_end = chars().count()`).
//!
//! Pages where text extraction fails or returns empty get an empty
//! `Block::Paragraph` plus a `Provenance::Warning` flagging the page as
//! a "scanned candidate" — out-of-scope OCR fallback can pick those up.
//!
//! Scope is intentionally narrow: page text + page numbers. Layout
//! reconstruction (multi-column reading order, tables, math), form
//! fields, bookmarks, and OCR for scanned PDFs are explicitly **not**
//! in this task. See `tasks/p7/p7-1-pdf-text-extractor.md`.
//!
//! Per design §3.4 (`SourceSpan::Page` / `Block::Paragraph`),
//! §9.2 (PDF text extraction), §9 versioning.

mod info;
mod page_image;
mod page_text;
mod text_quality;

pub use page_image::extract_dctdecode_page_image;
pub use text_quality::compute_valid_char_ratio;

use anyhow::{Context, Result};
use kebab_core::{
    Block, CanonicalDocument, CommonBlock, Extractor, Inline, Lang, MediaType, Metadata,
    ParserVersion, Provenance, ProvenanceEvent, ProvenanceKind, SourceSpan, SourceType, TextBlock,
    TrustLevel, id_for_block, id_for_doc,
};
use serde_json::{Map, Value};
use time::OffsetDateTime;

pub const PARSER_VERSION: &str = "pdf-text-v1";

/// Text-PDF extractor. Per-page text via `lopdf::Document::extract_text`
/// (the only stable per-page API in the lopdf / pdf-extract pair —
/// pdf-extract 0.7 only exposes whole-document calls).
pub struct PdfTextExtractor;

impl PdfTextExtractor {
    pub fn new() -> Self {
        Self
    }
}

impl Default for PdfTextExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl Extractor for PdfTextExtractor {
    fn supports(&self, m: &MediaType) -> bool {
        matches!(m, MediaType::Pdf)
    }

    fn parser_version(&self) -> ParserVersion {
        ParserVersion(PARSER_VERSION.to_string())
    }

    fn extract(
        &self,
        ctx: &kebab_core::ExtractContext<'_>,
        bytes: &[u8],
    ) -> Result<CanonicalDocument> {
        let asset = ctx.asset;
        if !self.supports(&asset.media_type) {
            anyhow::bail!(
                "kebab-parse-pdf: unsupported media_type for PdfTextExtractor: {:?}",
                asset.media_type
            );
        }

        let parser_version = self.parser_version();
        let doc_id = id_for_doc(&asset.workspace_path, &asset.asset_id, &parser_version);

        // Catastrophic-decode guard via lopdf. `pdf-extract` is intentionally
        // not used for parsing here — it only exposes whole-doc text and
        // would re-parse the bytes a second time.
        let pdf_doc = lopdf::Document::load_mem(bytes)
            .context("kebab-parse-pdf: failed to parse PDF (corrupt header or not a PDF)")?;

        if pdf_doc.is_encrypted() {
            anyhow::bail!(
                "kebab-parse-pdf: encrypted PDF; remove encryption (e.g. `qpdf --decrypt`) before ingest"
            );
        }

        let info = info::extract_info(&pdf_doc);
        // `get_pages()` returns BTreeMap<u32, ObjectId> with 1-based page
        // numbers. We iterate keys in BTreeMap natural order so output is
        // deterministic.
        let pages = pdf_doc.get_pages();
        let page_count = pages.len() as u32;

        let now = OffsetDateTime::now_utc();
        let mut events: Vec<ProvenanceEvent> = Vec::with_capacity(2 + pages.len());
        events.push(ProvenanceEvent {
            at: asset.discovered_at,
            agent: "kb-source-fs".to_string(),
            kind: ProvenanceKind::Discovered,
            note: None,
        });
        events.push(ProvenanceEvent {
            at: now,
            agent: "kb-parse-pdf".to_string(),
            kind: ProvenanceKind::Parsed,
            note: Some(format!(
                "parser_version={}; page_count={}",
                parser_version.0, page_count
            )),
        });

        let mut blocks: Vec<Block> = Vec::with_capacity(pages.len());
        for &page_num in pages.keys() {
            let (text, warning) = match page_text::extract_one(&pdf_doc, page_num) {
                Ok(t) if !t.trim().is_empty() => (t, None),
                Ok(_) => (
                    String::new(),
                    Some(format!("page{page_num} empty (scanned candidate)")),
                ),
                Err(e) => (
                    String::new(),
                    Some(format!(
                        "page{page_num} extract failed: {e} (scanned candidate)"
                    )),
                ),
            };
            let char_count = text.chars().count() as u32;
            let span = SourceSpan::Page {
                page: page_num,
                char_start: Some(0),
                char_end: Some(char_count),
            };
            // lopdf's `get_pages()` is 1-based by contract. A 0-key would
            // collapse two pages onto the same ordinal (silently breaking
            // ordinal-based sorting downstream), so we assert the
            // invariant in dev builds. The release fallback still uses
            // saturating_sub so a future lopdf regression degrades to
            // garbled order rather than panic.
            debug_assert!(page_num >= 1, "lopdf get_pages() returned 0-based page key");
            let ordinal = page_num.saturating_sub(1);
            let block_id = id_for_block(&doc_id, "paragraph", &[], ordinal, &span);
            let common = CommonBlock {
                block_id,
                heading_path: Vec::new(),
                source_span: span,
            };
            let inlines = if text.is_empty() {
                Vec::new()
            } else {
                vec![Inline::Text { text: text.clone() }]
            };
            blocks.push(Block::Paragraph(TextBlock {
                common,
                text,
                inlines,
            }));
            if let Some(note) = warning {
                events.push(ProvenanceEvent {
                    at: now,
                    agent: "kb-parse-pdf".to_string(),
                    kind: ProvenanceKind::Warning,
                    note: Some(note),
                });
            }
        }

        let title = info
            .title
            .clone()
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| {
                let fname = filename_from_workspace_path(&asset.workspace_path.0);
                strip_extension(&fname)
            });

        let mut user = Map::new();
        let mut pdf_meta = Map::new();
        pdf_meta.insert("page_count".into(), Value::Number(page_count.into()));
        if let Some(p) = &info.producer {
            pdf_meta.insert("producer".into(), Value::String(p.clone()));
        }
        if let Some(c) = &info.creator {
            pdf_meta.insert("creator".into(), Value::String(c.clone()));
        }
        user.insert("pdf".into(), Value::Object(pdf_meta));

        let metadata = Metadata {
            aliases: Vec::new(),
            tags: Vec::new(),
            created_at: asset.discovered_at,
            updated_at: asset.discovered_at,
            source_type: SourceType::Paper,
            trust_level: TrustLevel::Primary,
            user_id_alias: None,
            user,
            repo: None,
            git_branch: None,
            git_commit: None,
            code_lang: None,
            source_id: None,
        };

        tracing::debug!(
            target: "kebab-parse-pdf",
            "extracted PDF doc_id={} workspace_path={} pages={}",
            doc_id.0,
            asset.workspace_path.0,
            page_count
        );

        Ok(CanonicalDocument {
            doc_id,
            source_asset_id: asset.asset_id.clone(),
            workspace_path: asset.workspace_path.clone(),
            title,
            lang: Lang("und".to_string()),
            blocks,
            metadata,
            provenance: Provenance { events },
            parser_version,
            schema_version: 1,
            doc_version: 1,
            last_chunker_version: None,
            last_embedding_version: None,
        })
    }
}

fn filename_from_workspace_path(p: &str) -> String {
    p.rsplit('/').next().unwrap_or(p).to_string()
}

fn strip_extension(filename: &str) -> String {
    match filename.rfind('.') {
        Some(0) => filename.to_string(),
        Some(idx) => filename[..idx].to_string(),
        None => filename.to_string(),
    }
}
