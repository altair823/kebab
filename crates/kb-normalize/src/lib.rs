//! `kb-normalize` — lift parser output (`kb-parse-types`) into a
//! [`kb_core::CanonicalDocument`] with deterministic IDs.
//!
//! Per design §3.4 (CanonicalDocument / Block), §4.2 (ID recipe), §4.3
//! (ordinal rule), §3.6 (Provenance), §8 (module boundaries).
//!
//! Public surface:
//!
//! * [`build_canonical_document`] — assemble a `CanonicalDocument` from
//!   `(RawAsset, Metadata, Vec<ParsedBlock>, ParserVersion, Vec<Warning>)`.
//! * [`id_for_doc`], [`id_for_block`] — re-exports of the canonical
//!   ID-recipe functions in `kb-core::ids` (§4.2). `kb-core` is the only
//!   implementation; `kb-normalize` is the canonical *entry point* per
//!   design §8.
//!
//! This crate must NOT depend on any parser implementation crate
//! (`kb-parse-md`, `kb-parse-pdf`, …). All parser output flows in via
//! the shared `kb-parse-types` crate.

use anyhow::Result;
use kb_core::{
    CanonicalDocument, Lang, Metadata, ParserVersion, Provenance, RawAsset,
};
use kb_parse_types::{ParsedBlock, Warning};

pub use kb_core::{id_for_block, id_for_doc};

/// Build a [`CanonicalDocument`] from the raw asset, frontmatter
/// metadata, parser blocks, parser version, and any warnings. Full
/// behavior (block ID assignment, provenance, title/lang lift) is
/// filled in by subsequent commits in this series; this stub establishes
/// the public signature and the doc_id derivation only.
pub fn build_canonical_document(
    asset: &RawAsset,
    metadata: Metadata,
    blocks: Vec<ParsedBlock>,
    parser_version: &ParserVersion,
    _warnings: Vec<Warning>,
) -> Result<CanonicalDocument> {
    let doc_id = id_for_doc(&asset.workspace_path, &asset.asset_id, parser_version);
    Ok(CanonicalDocument {
        doc_id,
        source_asset_id: asset.asset_id.clone(),
        workspace_path: asset.workspace_path.clone(),
        title: String::new(),
        lang: Lang(String::new()),
        blocks: Vec::new(),
        metadata,
        provenance: Provenance { events: Vec::new() },
        parser_version: parser_version.clone(),
        schema_version: 1,
        doc_version: 1,
    })
    .map(|d| {
        // `blocks` is consumed but not yet lifted — flag it as live to
        // satisfy the unused-binding lint until the next commit fills
        // in the real lifting logic.
        let _ = blocks;
        d
    })
}
