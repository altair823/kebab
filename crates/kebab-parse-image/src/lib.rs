//! `kebab-parse-image` — image extractor (P6-1) + OCR adapter (P6-2).
//!
//! P6-1 implements [`kebab_core::Extractor`] for `MediaType::Image(_)`,
//! producing a single-block [`CanonicalDocument`] (`ImageRefBlock` with
//! EXIF + dimensions in `metadata.user`). OCR / caption fields stay
//! `None` until populated by the OCR / caption adapters.
//!
//! P6-2 adds the [`ocr`] module: an [`OcrEngine`] trait and an
//! [`OllamaVisionOcr`] default adapter that talks to a vision-capable
//! Ollama model. [`apply_ocr`] is the helper that mutates an
//! [`ImageRefBlock`] in place. Trust note — the LLM-driven default
//! can hallucinate; `OcrText.engine` carries the source identity so
//! consumers can branch trust by engine (Tesseract / Apple Vision
//! adapters, when added, will write a different `engine` string).
//!
//! P6-3 adds the [`caption`] module: [`caption_image`] /
//! [`apply_caption`] route an image through any vision-capable
//! [`kebab_core::LanguageModel`] (text-only LMs are not vision-aware
//! and will surface a model-side error). Captions are explicitly
//! marked **model-generated** — the trust gap between OCR (observed,
//! engine-tagged) and caption (generated, prompt-tagged) is the
//! workspace's central trust contract.
//!
//! Per design §3.4 (Block::ImageRef + ImageRefBlock), §3.7a (OcrText /
//! ModelCaption stubs), §9.1 (image extraction policy / OCR vs caption
//! provenance), §9 (versioning).

pub mod caption;
mod dims;
mod exif_extract;
mod image_prep;
pub mod ocr;
pub mod paddle_onnx;

pub use caption::{apply_caption, caption_image};
pub use ocr::{OLLAMA_VISION_ENGINE, OcrEngine, OllamaVisionOcr, apply_ocr};
pub use paddle_onnx::{
    ModelPaths, OnnxPaddleOcr, PADDLE_ONNX_ENGINE, engine_version_for_config,
    engine_version_for_paths,
};

use anyhow::{Context, Result};
use kebab_core::{
    Block, CanonicalDocument, CommonBlock, Extractor, ImageRefBlock, Lang, MediaType, Metadata,
    ParserVersion, Provenance, ProvenanceEvent, ProvenanceKind, SourceSpan, SourceType, TrustLevel,
    id_for_block, id_for_doc,
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
            dims::DimOutcome::Ok {
                width,
                height,
                format,
            } => {
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
            repo: None,
            git_branch: None,
            git_commit: None,
            code_lang: None,
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
