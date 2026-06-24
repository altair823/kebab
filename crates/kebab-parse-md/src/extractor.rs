//! `kb-parse-md::extractor` — the [`Extractor`] trait impl that wraps the
//! crate's free functions (`parse_frontmatter` + `parse_blocks` +
//! `build_canonical_document`) so markdown ingest flows through the same
//! `App.extractors` registry + `App::extract_for` polymorphic dispatch
//! that pdf / image / code already use.
//!
//! This is a pure structural unification: the byte sequence it runs is
//! identical to the inline arm `kebab-app::ingest_one_asset` used before
//! (frontmatter parse → body-offset count → block parse → canonical lift,
//! same args, same order), so the produced `CanonicalDocument` — and thus
//! `doc_id` / `chunk_id` — is byte-for-byte the same.
//!
//! The one piece of context the inline arm read that the other extractors
//! do not is the per-source `source_id` + `trust_level`: markdown
//! frontmatter can *override* the per-source trust default, and that
//! precedence is resolved *inside* `parse_frontmatter` via [`BodyHints`].
//! [`ExtractContext`] carries both so the resolution stays identical.

use kebab_core::{CanonicalDocument, ExtractContext, Extractor, MediaType, ParserVersion, RawAsset};

use crate::PARSER_VERSION;
use crate::frontmatter::{BodyHints, FrontmatterSpan, parse_frontmatter};
use crate::{build_canonical_document, parse_blocks};

/// Markdown extractor — wraps the crate's free functions behind the
/// [`Extractor`] trait.
pub struct MarkdownExtractor;

impl MarkdownExtractor {
    pub fn new() -> Self {
        Self
    }
}

impl Default for MarkdownExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl Extractor for MarkdownExtractor {
    fn supports(&self, m: &MediaType) -> bool {
        matches!(m, MediaType::Markdown)
    }

    fn parser_version(&self) -> ParserVersion {
        ParserVersion(PARSER_VERSION.to_string())
    }

    fn extract(
        &self,
        ctx: &ExtractContext<'_>,
        bytes: &[u8],
    ) -> anyhow::Result<CanonicalDocument> {
        let asset = ctx.asset;
        let parser_version = self.parser_version();

        // `[[workspace.sources]]`: stamp the owning source id + inject the
        // per-source default trust level (frontmatter still overrides it).
        // Mirrors the old inline `build_body_hints` exactly.
        let body_hints = build_body_hints(asset, ctx.source_id, ctx.source_trust);

        // Frontmatter — `parse_frontmatter` returns Ok even on malformed
        // frontmatter (warnings are surfaced through the `Vec<Warning>`).
        use anyhow::Context as _;
        let (metadata, fm_span, fm_warns) =
            parse_frontmatter(bytes, &body_hints).context("kb-parse-md::parse_frontmatter")?;

        let body_offset_lines = match fm_span {
            Some(span) => count_lines_in(&bytes[..span.end]),
            None => 0,
        };

        let (parsed_blocks, blk_warns) =
            parse_blocks(&bytes[fm_span_end(fm_span)..], body_offset_lines)
                .context("kb-parse-md::parse_blocks")?;

        let mut all_warnings = Vec::with_capacity(fm_warns.len() + blk_warns.len());
        all_warnings.extend(fm_warns);
        all_warnings.extend(blk_warns);

        let canonical =
            build_canonical_document(asset, metadata, parsed_blocks, &parser_version, all_warnings)
                .context("kb-parse-md::build_canonical_document")?;
        Ok(canonical)
    }
}

/// Build `BodyHints` from the asset alone. We use the asset's
/// `discovered_at` for both `fs_ctime` and `fs_mtime` because going
/// through the FS metadata API for every file would be a noticeable
/// overhead for large workspaces and the source-of-truth timestamps
/// are written into the document's frontmatter when the user wants
/// authoritative values.
fn build_body_hints(
    asset: &RawAsset,
    source_id: Option<&str>,
    source_trust: Option<kebab_core::TrustLevel>,
) -> BodyHints {
    BodyHints {
        first_h1: None,
        fs_ctime: asset.discovered_at,
        fs_mtime: asset.discovered_at,
        fallback_lang: None,
        // `[[workspace.sources]]`: stamp the owning source id + inject the
        // per-source default trust level (frontmatter still overrides it).
        source_id: source_id.map(str::to_string),
        fallback_trust_level: source_trust,
    }
}

/// Convenience: end byte of the frontmatter region (or 0 when absent).
fn fm_span_end(span: Option<FrontmatterSpan>) -> usize {
    span.map_or(0, |s| s.end)
}

/// Count `\n` in a byte prefix to convert frontmatter byte span to
/// the line-offset `parse_blocks` expects.
fn count_lines_in(bytes: &[u8]) -> u32 {
    let n = bytes.iter().filter(|&&b| b == b'\n').count();
    u32::try_from(n).unwrap_or(u32::MAX)
}
