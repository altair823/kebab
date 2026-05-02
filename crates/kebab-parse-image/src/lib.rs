//! `kebab-parse-image` — image extractor (P6-1).
//!
//! Implements [`kebab_core::Extractor`] for `MediaType::Image(_)`. One asset
//! produces one [`CanonicalDocument`] with a single
//! [`Block::ImageRef`](kebab_core::Block::ImageRef). EXIF is captured into
//! `metadata.user["exif"]`, dimensions into `metadata.user["dimensions"]`.
//! OCR / caption fields stay `None`; later tasks (P6-2 / P6-3) populate
//! them.
//!
//! Per design §3.4 (Block::ImageRef + ImageRefBlock), §3.7a (OcrText /
//! ModelCaption stubs), §9.1 (image extraction policy), §9 (versioning).

mod dims;
mod exif_extract;

use anyhow::{Context, Result};
use kebab_core::{
    Block, CanonicalDocument, CommonBlock, Extractor, ImageRefBlock, Lang, MediaType, Metadata,
    ParserVersion, Provenance, ProvenanceEvent, ProvenanceKind, SourceSpan, SourceType,
    TrustLevel, id_for_block, id_for_doc,
};
use serde_json::{Map, Value};
use time::OffsetDateTime;

/// Parser version label for the image extractor (§9 versioning).
pub const PARSER_VERSION: &str = "image-meta-v1";

/// Maximum decode dimension (per axis) before we refuse to read the image.
/// Matches the §9.1 "cap decode at ~16k" policy in the design doc.
pub const MAX_DECODE_DIM: u32 = 16_384;

/// Image extractor — produces a single-block `CanonicalDocument` whose body
/// is exactly one [`ImageRefBlock`].
pub struct ImageExtractor;

impl ImageExtractor {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ImageExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl Extractor for ImageExtractor {
    fn supports(&self, m: &MediaType) -> bool {
        matches!(m, MediaType::Image(_))
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
                "kebab-parse-image: unsupported media_type for ImageExtractor: {:?}",
                asset.media_type
            );
        }

        let parser_version = self.parser_version();
        let doc_id = id_for_doc(&asset.workspace_path, &asset.asset_id, &parser_version);

        // Dimensions / format. `Err` here means the bytes don't even resolve
        // to a known image format — we propagate so the caller can skip the
        // asset (per spec failure modes: "Unsupported format → anyhow::Error").
        let dim_outcome = dims::probe(bytes).context("guessing image format")?;

        // EXIF is best-effort regardless of dimension outcome. A corrupt
        // pixel stream may still carry a readable EXIF block (and vice
        // versa), so the two probes are independent.
        let exif_map = exif_extract::extract_whitelisted(bytes);

        let (span, dims_value, dim_warning) = match &dim_outcome {
            dims::DimOutcome::Ok { width, height, format } => {
                let mut dims = Map::new();
                dims.insert("w".into(), Value::Number((*width).into()));
                dims.insert("h".into(), Value::Number((*height).into()));
                dims.insert("format".into(), Value::String(format.to_string()));
                (
                    SourceSpan::Region {
                        x: 0,
                        y: 0,
                        w: *width,
                        h: *height,
                    },
                    Value::Object(dims),
                    None,
                )
            }
            dims::DimOutcome::Failed { reason } => (
                SourceSpan::Region {
                    x: 0,
                    y: 0,
                    w: 0,
                    h: 0,
                },
                Value::Null,
                Some(reason.clone()),
            ),
        };

        let block_id = id_for_block(&doc_id, "imageref", &[], 0, &span);

        let workspace_path_str = asset.workspace_path.0.clone();
        let filename = filename_from_workspace_path(&workspace_path_str);
        let title = strip_extension(&filename);

        let block = Block::ImageRef(ImageRefBlock {
            common: CommonBlock {
                block_id,
                heading_path: Vec::new(),
                source_span: span,
            },
            asset_id: Some(asset.asset_id.clone()),
            src: workspace_path_str,
            alt: filename,
            ocr: None,
            caption: None,
        });

        let now = OffsetDateTime::now_utc();
        // Discovered + Parsed (always) + optional Warning when the
        // dim probe failed.
        let mut events: Vec<ProvenanceEvent> =
            Vec::with_capacity(if dim_warning.is_some() { 3 } else { 2 });
        events.push(ProvenanceEvent {
            at: asset.discovered_at,
            agent: "kb-source-fs".to_string(),
            kind: ProvenanceKind::Discovered,
            note: None,
        });
        events.push(ProvenanceEvent {
            at: now,
            agent: "kb-parse-image".to_string(),
            kind: ProvenanceKind::Parsed,
            note: Some(format!("parser_version={}", parser_version.0)),
        });
        if let Some(reason) = dim_warning {
            events.push(ProvenanceEvent {
                at: now,
                agent: "kb-parse-image".to_string(),
                kind: ProvenanceKind::Warning,
                note: Some(reason),
            });
        }

        // Metadata. `created_at` / `updated_at` are sourced from the asset's
        // `discovered_at` so the wire form does not embed a fresh timestamp
        // for every extract call (which would break determinism).
        let mut user = Map::new();
        user.insert("exif".into(), Value::Object(exif_map));
        user.insert("dimensions".into(), dims_value);
        let metadata = Metadata {
            aliases: Vec::new(),
            tags: Vec::new(),
            created_at: asset.discovered_at,
            updated_at: asset.discovered_at,
            source_type: SourceType::Reference,
            trust_level: TrustLevel::Primary,
            user_id_alias: None,
            user,
        };

        tracing::debug!(
            target: "kebab-parse-image",
            "extracted image doc_id={} workspace_path={} dim_ok={}",
            doc_id.0,
            asset.workspace_path.0,
            matches!(dim_outcome, dims::DimOutcome::Ok { .. })
        );

        Ok(CanonicalDocument {
            doc_id,
            source_asset_id: asset.asset_id.clone(),
            workspace_path: asset.workspace_path.clone(),
            title,
            lang: Lang("und".to_string()),
            blocks: vec![block],
            metadata,
            provenance: Provenance { events },
            parser_version,
            schema_version: 1,
            doc_version: 1,
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
