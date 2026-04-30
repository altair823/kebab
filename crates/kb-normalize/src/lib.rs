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

use std::collections::HashMap;

use anyhow::Result;
use kb_core::{
    AudioRefBlock, Block, BlockId, CanonicalDocument, CodeBlock, CommonBlock, DocumentId,
    HeadingBlock, ImageRefBlock, Inline, Lang, ListBlock, Metadata, ParserVersion, Provenance,
    RawAsset, TableBlock, TextBlock,
};
use kb_parse_types::{ParsedBlock, ParsedPayload, Warning};

pub use kb_core::{id_for_block, id_for_doc};

/// Build a [`CanonicalDocument`] from the raw asset, frontmatter
/// metadata, parser blocks, parser version, and any warnings.
///
/// This commit fills in the §4.3 ordinal rule and the §3.4 block lift.
/// `Provenance` and the title/lang lift are added in the next commit.
pub fn build_canonical_document(
    asset: &RawAsset,
    metadata: Metadata,
    blocks: Vec<ParsedBlock>,
    parser_version: &ParserVersion,
    _warnings: Vec<Warning>,
) -> Result<CanonicalDocument> {
    let doc_id = id_for_doc(&asset.workspace_path, &asset.asset_id, parser_version);

    // §4.3 ordinal rule — per (heading_path, block_kind), 0-based,
    // document order. A separate counter is kept for each grouping key.
    let mut counters: HashMap<(Vec<String>, &'static str), u32> = HashMap::new();
    let lifted_blocks: Vec<Block> = blocks
        .into_iter()
        .map(|pb| lift_block(&doc_id, pb, &mut counters))
        .collect();

    Ok(CanonicalDocument {
        doc_id,
        source_asset_id: asset.asset_id.clone(),
        workspace_path: asset.workspace_path.clone(),
        title: String::new(),
        lang: Lang(String::new()),
        blocks: lifted_blocks,
        metadata,
        provenance: Provenance { events: Vec::new() },
        parser_version: parser_version.clone(),
        schema_version: 1,
        doc_version: 1,
    })
}

/// Map a `ParsedPayload` variant to the lowercase, no-spaces string used
/// as `block_kind` in the §4.2 ID tuple.
fn payload_kind(payload: &ParsedPayload) -> &'static str {
    match payload {
        ParsedPayload::Heading { .. } => "heading",
        ParsedPayload::Paragraph { .. } => "paragraph",
        ParsedPayload::List { .. } => "list",
        ParsedPayload::Code { .. } => "code",
        ParsedPayload::Table { .. } => "table",
        ParsedPayload::Quote { .. } => "quote",
        ParsedPayload::ImageRef { .. } => "imageref",
        ParsedPayload::AudioRef { .. } => "audioref",
    }
}

fn next_ordinal(
    counters: &mut HashMap<(Vec<String>, &'static str), u32>,
    heading_path: &[String],
    kind: &'static str,
) -> u32 {
    let key = (heading_path.to_vec(), kind);
    let entry = counters.entry(key).or_insert(0);
    let ordinal = *entry;
    *entry += 1;
    ordinal
}

fn lift_block(
    doc_id: &DocumentId,
    pb: ParsedBlock,
    counters: &mut HashMap<(Vec<String>, &'static str), u32>,
) -> Block {
    let kind = payload_kind(&pb.payload);
    let ordinal = next_ordinal(counters, &pb.heading_path, kind);
    let block_id: BlockId = id_for_block(doc_id, kind, &pb.heading_path, ordinal, &pb.source_span);
    let common = CommonBlock {
        block_id,
        heading_path: pb.heading_path,
        source_span: pb.source_span,
    };
    match pb.payload {
        ParsedPayload::Heading { level, text } => Block::Heading(HeadingBlock {
            common,
            level,
            text,
        }),
        ParsedPayload::Paragraph { text, inlines } => Block::Paragraph(TextBlock {
            common,
            text,
            inlines,
        }),
        ParsedPayload::List { ordered, items } => Block::List(ListBlock {
            common: common.clone(),
            ordered,
            items: items
                .into_iter()
                .map(|item_inlines| TextBlock {
                    // List items inherit the parent list's CommonBlock; spec
                    // (§3.4) defines `ListBlock.items: Vec<TextBlock>` and
                    // does not allocate per-item BlockIds. Re-using the
                    // parent's common keeps the wire form deterministic
                    // while letting the inline tree carry the item content.
                    common: common.clone(),
                    text: flatten_inlines(&item_inlines),
                    inlines: item_inlines,
                })
                .collect(),
        }),
        ParsedPayload::Code { lang, code } => Block::Code(CodeBlock { common, lang, code }),
        ParsedPayload::Table { headers, rows } => Block::Table(TableBlock {
            common,
            headers,
            rows,
        }),
        ParsedPayload::Quote { text, inlines } => Block::Quote(TextBlock {
            common,
            text,
            inlines,
        }),
        ParsedPayload::ImageRef { src, alt } => Block::ImageRef(ImageRefBlock {
            common,
            asset_id: None,
            src,
            alt,
            ocr: None,
            caption: None,
        }),
        // P1-4 does not extract audio metadata from disk — `asset_id`
        // and `duration_ms` placeholders are filled in by the audio
        // extractor (P8). For now we synthesize a minimal record so
        // the document is well-typed.
        ParsedPayload::AudioRef { src: _ } => Block::AudioRef(AudioRefBlock {
            common,
            asset_id: kb_core::AssetId(String::new()),
            duration_ms: 0,
            transcript: None,
        }),
    }
}

/// Flatten a `Vec<Inline>` into a plain text string. Used by list-item
/// `TextBlock.text` since `ParsedPayload::List` only carries inline trees
/// per item.
fn flatten_inlines(inlines: &[Inline]) -> String {
    let mut out = String::new();
    for i in inlines {
        flatten_inline(i, &mut out);
    }
    out
}

fn flatten_inline(i: &Inline, out: &mut String) {
    match i {
        Inline::Text { text } => out.push_str(text),
        Inline::Code { code } => out.push_str(code),
        Inline::Link { text, .. } => out.push_str(text),
        Inline::Strong { children } | Inline::Emph { children } => {
            for c in children {
                flatten_inline(c, out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kb_core::{
        AssetId, AssetStorage, Checksum, MediaType, SourceSpan, SourceType, SourceUri,
        TrustLevel, WorkspacePath, normalize::to_posix,
    };
    use serde_json::Value;
    use std::path::{Path, PathBuf};
    use time::OffsetDateTime;

    fn fixture_asset() -> RawAsset {
        let workspace_path = WorkspacePath::new("notes/example.md".into()).unwrap();
        RawAsset {
            asset_id: AssetId("a".repeat(32)),
            source_uri: SourceUri::File(PathBuf::from("/tmp/example.md")),
            workspace_path,
            media_type: MediaType::Markdown,
            byte_len: 0,
            checksum: Checksum("0".repeat(64)),
            // Pin a fixed timestamp so determinism tests can compare
            // outputs across runs without timestamp jitter outside the
            // fields we explicitly strip.
            discovered_at: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
            stored: AssetStorage::Reference {
                path: PathBuf::from("/tmp/example.md"),
                sha: Checksum("0".repeat(64)),
            },
        }
    }

    fn fixture_metadata() -> Metadata {
        let mut user = serde_json::Map::new();
        user.insert("title".into(), Value::String("Example".into()));
        user.insert("lang".into(), Value::String("en".into()));
        user.insert("custom".into(), Value::Bool(true));
        Metadata {
            aliases: vec![],
            tags: vec![],
            created_at: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
            updated_at: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
            source_type: SourceType::Markdown,
            trust_level: TrustLevel::Primary,
            user_id_alias: None,
            user,
        }
    }

    fn parser_version() -> ParserVersion {
        ParserVersion("kb-normalize-test-0".into())
    }

    /// `id_for_doc` is deterministic across 1000 invocations on the same
    /// input — a regression in canonical JSON or BLAKE3 would surface
    /// here immediately.
    #[test]
    fn id_for_doc_deterministic_1000() {
        let path = WorkspacePath::new("a/b.md".into()).unwrap();
        let asset = AssetId("0123456789abcdef0123456789abcdef".into());
        let pv = ParserVersion("v1".into());
        let first = id_for_doc(&path, &asset, &pv);
        for _ in 0..1000 {
            assert_eq!(id_for_doc(&path, &asset, &pv), first);
        }
    }

    /// NFC vs NFD inputs for the same Korean glyph must produce the
    /// same `doc_id` because `to_posix` runs NFC normalization.
    #[test]
    fn nfc_nfd_korean_path_same_id() {
        let nfd = to_posix(Path::new("\u{1100}\u{1161}.md")).unwrap();
        let nfc = to_posix(Path::new("\u{AC00}.md")).unwrap();
        let asset = AssetId("0123456789abcdef0123456789abcdef".into());
        let pv = parser_version();
        assert_eq!(id_for_doc(&nfd, &asset, &pv), id_for_doc(&nfc, &asset, &pv));
    }

    /// `./a/b.md` and `a/b.md` must collapse to the same POSIX form
    /// before `id_for_doc`.
    #[test]
    fn posix_curdir_collapses_to_same_id() {
        let a = to_posix(Path::new("./a/b.md")).unwrap();
        let b = to_posix(Path::new("a/b.md")).unwrap();
        let asset = AssetId("0123456789abcdef0123456789abcdef".into());
        let pv = parser_version();
        assert_eq!(id_for_doc(&a, &asset, &pv), id_for_doc(&b, &asset, &pv));
    }

    /// Ordinals are scoped to (heading_path, block_kind) per §4.3:
    /// three paragraphs under H1 → 0/1/2; a code block under the same
    /// H1 starts a fresh counter at 0; a paragraph under a different
    /// H1 also starts a fresh counter at 0.
    #[test]
    fn block_ordinals_scoped_per_heading_and_kind() {
        let span = SourceSpan::Line { start: 1, end: 1 };
        let h1_a = vec!["A".to_string()];
        let h1_b = vec!["B".to_string()];
        let blocks = vec![
            ParsedBlock {
                kind: kb_parse_types::ParsedBlockKind::Paragraph,
                heading_path: h1_a.clone(),
                source_span: span.clone(),
                payload: ParsedPayload::Paragraph {
                    text: "p1".into(),
                    inlines: vec![],
                },
            },
            ParsedBlock {
                kind: kb_parse_types::ParsedBlockKind::Paragraph,
                heading_path: h1_a.clone(),
                source_span: SourceSpan::Line { start: 2, end: 2 },
                payload: ParsedPayload::Paragraph {
                    text: "p2".into(),
                    inlines: vec![],
                },
            },
            ParsedBlock {
                kind: kb_parse_types::ParsedBlockKind::Paragraph,
                heading_path: h1_a.clone(),
                source_span: SourceSpan::Line { start: 3, end: 3 },
                payload: ParsedPayload::Paragraph {
                    text: "p3".into(),
                    inlines: vec![],
                },
            },
            ParsedBlock {
                kind: kb_parse_types::ParsedBlockKind::Code,
                heading_path: h1_a.clone(),
                source_span: SourceSpan::Line { start: 4, end: 5 },
                payload: ParsedPayload::Code {
                    lang: None,
                    code: "x".into(),
                },
            },
            ParsedBlock {
                kind: kb_parse_types::ParsedBlockKind::Paragraph,
                heading_path: h1_b.clone(),
                source_span: SourceSpan::Line { start: 6, end: 6 },
                payload: ParsedPayload::Paragraph {
                    text: "q1".into(),
                    inlines: vec![],
                },
            },
        ];
        let asset = fixture_asset();
        let metadata = fixture_metadata();
        let pv = parser_version();
        let doc =
            build_canonical_document(&asset, metadata, blocks, &pv, vec![]).unwrap();

        // Compute the expected IDs out-of-band so the test pins both
        // the (heading_path, kind) ordinal grouping AND the value of
        // each block_id under the recipe.
        let p1 = id_for_block(&doc.doc_id, "paragraph", &h1_a, 0, &span);
        let p2 = id_for_block(
            &doc.doc_id,
            "paragraph",
            &h1_a,
            1,
            &SourceSpan::Line { start: 2, end: 2 },
        );
        let p3 = id_for_block(
            &doc.doc_id,
            "paragraph",
            &h1_a,
            2,
            &SourceSpan::Line { start: 3, end: 3 },
        );
        let c0 = id_for_block(
            &doc.doc_id,
            "code",
            &h1_a,
            0,
            &SourceSpan::Line { start: 4, end: 5 },
        );
        let q0 = id_for_block(
            &doc.doc_id,
            "paragraph",
            &h1_b,
            0,
            &SourceSpan::Line { start: 6, end: 6 },
        );

        let ids: Vec<&BlockId> = doc
            .blocks
            .iter()
            .map(|b| match b {
                Block::Paragraph(t) | Block::Quote(t) => &t.common.block_id,
                Block::Heading(h) => &h.common.block_id,
                Block::List(l) => &l.common.block_id,
                Block::Code(c) => &c.common.block_id,
                Block::Table(t) => &t.common.block_id,
                Block::ImageRef(i) => &i.common.block_id,
                Block::AudioRef(a) => &a.common.block_id,
            })
            .collect();
        assert_eq!(ids, vec![&p1, &p2, &p3, &c0, &q0]);
    }
}
