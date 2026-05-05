//! `kb-normalize` — lift parser output (`kb-parse-types`) into a
//! [`kebab_core::CanonicalDocument`] with deterministic IDs.
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
use std::path::Path;

use anyhow::Result;
use kebab_core::{
    Block, BlockId, CanonicalDocument, CodeBlock, CommonBlock, DocumentId, HeadingBlock,
    ImageRefBlock, Inline, Lang, ListBlock, Metadata, ParserVersion, Provenance, ProvenanceEvent,
    ProvenanceKind, RawAsset, TableBlock, TextBlock,
};
use kebab_parse_types::{ParsedBlock, ParsedPayload, Warning, WarningKind};
use time::OffsetDateTime;
use unicode_normalization::UnicodeNormalization;

pub use kebab_core::{id_for_block, id_for_doc};

/// Build a [`CanonicalDocument`] from the raw asset, frontmatter
/// metadata, parser blocks, parser version, and any warnings.
///
/// Behavior contract (per design §3.4 / §4.2 / §4.3 / §3.6):
///
/// * `doc_id = id_for_doc(workspace_path, asset_id, parser_version)` —
///   `workspace_path` is consumed verbatim from `asset` (already NFC +
///   POSIX per `kebab_core::normalize::to_posix`).
/// * `block_id = id_for_block(doc_id, kind, heading_path, ordinal,
///   source_span)` — `ordinal` is **0-based, scoped to (heading_path,
///   block_kind), in document order** per §4.3.
/// * `title` and `lang` are lifted from `metadata.user["title"]` /
///   `metadata.user["lang"]` (where P1-2 stashes them) into the dedicated
///   `CanonicalDocument` fields, and removed from the user map to avoid
///   duplication. Both keys are lifted only if present and stringy;
///   non-stringy values (e.g. `Number`, `Array`) and missing keys
///   silently default to empty title / empty `Lang`. P1-2's frontmatter
///   parser only writes these keys when the source value parses as a
///   string, so the non-stringy branches are defense-in-depth.
/// * `provenance` is seeded with `Discovered` (from `asset.discovered_at`),
///   `Parsed`, `Normalized` events, and one `Warning` event per upstream
///   warning. The two normalize-side events share one `now_utc()` reading
///   so the timestamp jitter inside a single call is bounded — event
///   ordering is preserved by `Vec` position.
/// * `schema_version` and `doc_version` are pinned to `1` (initial).
pub fn build_canonical_document(
    asset: &RawAsset,
    metadata: Metadata,
    blocks: Vec<ParsedBlock>,
    parser_version: &ParserVersion,
    warnings: Vec<Warning>,
) -> Result<CanonicalDocument> {
    let doc_id = id_for_doc(&asset.workspace_path, &asset.asset_id, parser_version);

    // Lift title / lang from `metadata.user` (P1-2 stashed them there
    // because `Metadata` does not carry them directly). Strip after
    // lifting so the wire form does not duplicate the data.
    let mut metadata = metadata;
    let title = metadata
        .user
        .remove("title")
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_default();
    let lang = metadata
        .user
        .remove("lang")
        .and_then(|v| v.as_str().map(|s| Lang(s.to_string())))
        .unwrap_or_else(|| Lang(String::new()));

    // §4.3 ordinal rule — per (heading_path, block_kind), 0-based,
    // document order. A separate counter is kept for each grouping key.
    let mut counters: HashMap<(Vec<String>, &'static str), u32> = HashMap::new();
    // Some lift paths (e.g. AudioRef pre-P8) drop the block entirely and
    // synthesize a Warning so the wire form never carries an invalid
    // `AssetId`. These warnings originate at the lift stage and are
    // attributed to `kb-normalize` (not to whatever upstream emitter the
    // bare `WarningKind` would resolve to via `warning_agent`). They are
    // tracked separately so the agent string is correct in Provenance.
    let mut lift_warnings: Vec<Warning> = Vec::new();
    let lifted_blocks: Vec<Block> = blocks
        .into_iter()
        .filter_map(|pb| lift_block(&doc_id, pb, &mut counters, &mut lift_warnings))
        .collect();

    // p9-fb-07: title fallback chain. `title` so far holds the
    // frontmatter `title` (step 1). If empty / whitespace, walk the
    // lifted blocks for an H1 → H2 → first paragraph excerpt → file
    // stem. NFC-normalize the chosen string so the on-wire title is
    // canonically equivalent to whatever the user stored, regardless
    // of source NFD/NFC form.
    let file_stem = workspace_path_stem(&asset.workspace_path.0);
    let title = derive_title(&title, &lifted_blocks, &file_stem);

    tracing::debug!(
        target: "kebab-normalize",
        "built canonical document doc_id={} blocks={}",
        doc_id.0,
        lifted_blocks.len()
    );

    // Provenance — share `now` between the parse + normalize stages so
    // the per-call timestamp jitter is bounded.
    let now = OffsetDateTime::now_utc();
    let mut events: Vec<ProvenanceEvent> =
        Vec::with_capacity(3 + warnings.len() + lift_warnings.len());
    events.push(ProvenanceEvent {
        at: asset.discovered_at,
        agent: "kb-source-fs".to_string(),
        kind: ProvenanceKind::Discovered,
        note: None,
    });
    events.push(ProvenanceEvent {
        at: now,
        agent: "kb-parse-md".to_string(),
        kind: ProvenanceKind::Parsed,
        note: Some(format!("parser_version={}", parser_version.0)),
    });
    events.push(ProvenanceEvent {
        at: now,
        agent: "kb-normalize".to_string(),
        kind: ProvenanceKind::Normalized,
        note: None,
    });
    // {:?} on WarningKind renders camel-case variant name; intentional
    // for human-readable Provenance trace.
    for w in warnings {
        events.push(ProvenanceEvent {
            at: now,
            agent: warning_agent(&w.kind).to_string(),
            kind: ProvenanceKind::Warning,
            note: Some(format!("{:?}: {}", w.kind, w.note)),
        });
    }
    // Lift-stage warnings (currently only AudioRef-deferred drops) are
    // unconditionally attributed to `kb-normalize`.
    for w in lift_warnings {
        events.push(ProvenanceEvent {
            at: now,
            agent: "kb-normalize".to_string(),
            kind: ProvenanceKind::Warning,
            note: Some(format!("{:?}: {}", w.kind, w.note)),
        });
    }
    let provenance = Provenance { events };

    Ok(CanonicalDocument {
        doc_id,
        source_asset_id: asset.asset_id.clone(),
        workspace_path: asset.workspace_path.clone(),
        title,
        lang,
        blocks: lifted_blocks,
        metadata,
        provenance,
        parser_version: parser_version.clone(),
        schema_version: 1,
        doc_version: 1,
        last_chunker_version: None,
        last_embedding_version: None,
    })
}

/// Resolve a `WarningKind` to the upstream agent that emitted it. Used
/// to fill `ProvenanceEvent::agent` for the warning's event entry.
///
/// `ExtractFailed` is emitted today by `kb-parse-md`'s panic-recovery
/// guard around `parse_blocks` — see `crates/kb-parse-md/src/blocks.rs`.
/// If a future stage (e.g. `kb-normalize` itself, an extractor, …) starts
/// emitting `ExtractFailed`, this mapping needs to grow context (perhaps
/// a separate `WarningSource` field on `Warning`) so attribution stays
/// honest. For now, all `ExtractFailed` warnings observed by
/// `build_canonical_document` originated in the parser.
fn warning_agent(kind: &WarningKind) -> &'static str {
    match kind {
        WarningKind::MalformedFrontmatter | WarningKind::EncodingFallback => "kb-parse-md",
        WarningKind::MalformedTable => "kb-parse-md",
        WarningKind::ExtractFailed => "kb-parse-md",
    }
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
    warnings: &mut Vec<Warning>,
) -> Option<Block> {
    let kind = payload_kind(&pb.payload);
    // Task spec line 73: "All input strings normalized to NFC before
    // hashing." `pulldown-cmark` does not NFC heading text, and
    // `serde_json_canonicalizer` v0.3 does not normalize strings either,
    // so we must NFC-normalize `heading_path` here before it feeds both
    // the §4.2 ID recipe AND the on-disk `CommonBlock.heading_path` (so
    // wire form matches ID input). Without this, NFD `\u{1100}\u{1161}`
    // and NFC `\u{AC00}` (both render as 가) would produce different
    // `block_id`s for what is logically the same heading.
    let heading_path_nfc: Vec<String> =
        pb.heading_path.iter().map(|s| s.nfc().collect()).collect();
    let ordinal = next_ordinal(counters, &heading_path_nfc, kind);
    let block_id: BlockId =
        id_for_block(doc_id, kind, &heading_path_nfc, ordinal, &pb.source_span);
    let common = CommonBlock {
        block_id,
        heading_path: heading_path_nfc,
        source_span: pb.source_span,
    };
    let block = match pb.payload {
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
                    // All list items currently inherit the parent's
                    // CommonBlock (incl. block_id). Per-item IDs would
                    // require a §4.2 recipe extension. Spec (§3.4)
                    // defines `ListBlock.items: Vec<TextBlock>` and
                    // does not allocate per-item BlockIds. Re-using the
                    // parent's common keeps the wire form deterministic
                    // while letting the inline tree carry the item
                    // content.
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
        // TODO(P8): audio extractor will resolve workspace assets and
        // produce real AssetIds. This skip-and-warn shim is a
        // placeholder. `AssetId::from_str` requires a 32-hex string, so
        // synthesizing `AssetId(String::new())` would break the
        // invariant — instead we drop the block and surface a Warning
        // (attributed to `kb-normalize` per §3.6 since this is the
        // lift-stage decision).
        ParsedPayload::AudioRef { src } => {
            warnings.push(Warning {
                kind: WarningKind::ExtractFailed,
                note: format!(
                    "audio-ref AssetId resolution deferred to P8 — block dropped (src={src})"
                ),
            });
            return None;
        }
    };
    Some(block)
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

/// p9-fb-07: derive a usable title from the frontmatter, lifted blocks,
/// and the source filename, using a documented fallback chain.
///
/// Priority (first non-blank wins):
///
/// 1. `frontmatter_title` — verbatim, after trimming whitespace.
/// 2. First `Heading` block at level 1 with non-blank text.
/// 3. First `Heading` block at level 2 with non-blank text.
/// 4. First `Paragraph` block (NOT `Quote`, `List`, `Code`, `Table`,
///    `ImageRef`, `AudioRef`) with non-blank text — first 80 chars.
/// 5. `file_stem` (filename minus extension — returned verbatim, no
///    case transformation; whatever the on-disk filename is becomes
///    the title text).
///
/// The chosen string is NFC-normalized so the on-wire title is
/// canonically equivalent to the source content. Never returns an
/// empty string — if every step is blank (e.g. an empty file), the
/// `file_stem` fallback ensures a non-empty result. If `file_stem` is
/// also blank (pathological), returns `"untitled"` as a last resort.
pub fn derive_title(frontmatter_title: &str, blocks: &[Block], file_stem: &str) -> String {
    let trimmed = frontmatter_title.trim();
    if !trimmed.is_empty() {
        return trimmed.nfc().collect();
    }
    if let Some(text) = first_heading_text(blocks, 1) {
        return text;
    }
    if let Some(text) = first_heading_text(blocks, 2) {
        return text;
    }
    if let Some(excerpt) = first_paragraph_excerpt(blocks, 80) {
        return excerpt;
    }
    // `file_stem` originates from `WorkspacePath`, which `to_posix`
    // already NFC-normalizes (§6.6). No second NFC pass needed — pass
    // through verbatim after a defensive `trim`.
    let stem = file_stem.trim();
    if !stem.is_empty() {
        return stem.to_string();
    }
    "untitled".to_string()
}

fn first_heading_text(blocks: &[Block], level: u8) -> Option<String> {
    blocks.iter().find_map(|b| match b {
        Block::Heading(h) if h.level == level => {
            let trimmed = h.text.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.nfc().collect())
            }
        }
        _ => None,
    })
}

fn first_paragraph_excerpt(blocks: &[Block], max_chars: usize) -> Option<String> {
    blocks.iter().find_map(|b| match b {
        Block::Paragraph(t) => {
            let trimmed = t.text.trim();
            if trimmed.is_empty() {
                None
            } else {
                let nfc: String = trimmed.nfc().collect();
                Some(nfc.chars().take(max_chars).collect())
            }
        }
        _ => None,
    })
}

/// Extract the filename stem (no extension) from a workspace path
/// string. Returns the empty string if no filename can be derived
/// (e.g. trailing slash). Multi-extension cases (`foo.tar.gz`) follow
/// `Path::file_stem` semantics — only the last extension is stripped.
fn workspace_path_stem(workspace_path: &str) -> String {
    Path::new(workspace_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use kebab_core::{
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

    /// Fixed 5-block input used by both the ordinal-scoping pinning test
    /// and the determinism stress test (so the latter exercises the
    /// `lift_block` path, not just the empty-blocks path).
    fn fixture_blocks_five() -> Vec<ParsedBlock> {
        let h1_a = vec!["A".to_string()];
        let h1_b = vec!["B".to_string()];
        vec![
            ParsedBlock {
                kind: kebab_parse_types::ParsedBlockKind::Paragraph,
                heading_path: h1_a.clone(),
                source_span: SourceSpan::Line { start: 1, end: 1 },
                payload: ParsedPayload::Paragraph {
                    text: "p1".into(),
                    inlines: vec![],
                },
            },
            ParsedBlock {
                kind: kebab_parse_types::ParsedBlockKind::Paragraph,
                heading_path: h1_a.clone(),
                source_span: SourceSpan::Line { start: 2, end: 2 },
                payload: ParsedPayload::Paragraph {
                    text: "p2".into(),
                    inlines: vec![],
                },
            },
            ParsedBlock {
                kind: kebab_parse_types::ParsedBlockKind::Paragraph,
                heading_path: h1_a.clone(),
                source_span: SourceSpan::Line { start: 3, end: 3 },
                payload: ParsedPayload::Paragraph {
                    text: "p3".into(),
                    inlines: vec![],
                },
            },
            ParsedBlock {
                kind: kebab_parse_types::ParsedBlockKind::Code,
                heading_path: h1_a,
                source_span: SourceSpan::Line { start: 4, end: 5 },
                payload: ParsedPayload::Code {
                    lang: None,
                    code: "x".into(),
                },
            },
            ParsedBlock {
                kind: kebab_parse_types::ParsedBlockKind::Paragraph,
                heading_path: h1_b,
                source_span: SourceSpan::Line { start: 6, end: 6 },
                payload: ParsedPayload::Paragraph {
                    text: "q1".into(),
                    inlines: vec![],
                },
            },
        ]
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
        let h1_a = vec!["A".to_string()];
        let h1_b = vec!["B".to_string()];
        let blocks = fixture_blocks_five();
        let asset = fixture_asset();
        let metadata = fixture_metadata();
        let pv = parser_version();
        let doc =
            build_canonical_document(&asset, metadata, blocks, &pv, vec![]).unwrap();

        // Compute the expected IDs out-of-band so the test pins both
        // the (heading_path, kind) ordinal grouping AND the value of
        // each block_id under the recipe.
        let p1 = id_for_block(
            &doc.doc_id,
            "paragraph",
            &h1_a,
            0,
            &SourceSpan::Line { start: 1, end: 1 },
        );
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

    /// Provenance events appear in the documented order: `Discovered`
    /// (from the asset), `Parsed`, then `Normalized`. Warnings (none in
    /// this test) would follow.
    #[test]
    fn provenance_contains_stage_events_in_order() {
        let asset = fixture_asset();
        let metadata = fixture_metadata();
        let pv = parser_version();
        let doc =
            build_canonical_document(&asset, metadata, vec![], &pv, vec![]).unwrap();
        let kinds: Vec<_> = doc.provenance.events.iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            vec![
                ProvenanceKind::Discovered,
                ProvenanceKind::Parsed,
                ProvenanceKind::Normalized,
            ]
        );
        let events = &doc.provenance.events;
        assert_eq!(events[0].at, asset.discovered_at);
        assert_eq!(events[0].agent, "kb-source-fs");
        assert_eq!(events[1].agent, "kb-parse-md");
        assert_eq!(events[2].agent, "kb-normalize");
        // Pin the implementation invariant that Parsed and Normalized
        // share the single `now_utc()` reading inside one call.
        assert_eq!(events[1].at, events[2].at, "Parsed and Normalized share now_utc");
    }

    /// Warnings carried into `build_canonical_document` are emitted as
    /// `ProvenanceKind::Warning` events with the upstream agent.
    #[test]
    fn provenance_includes_warnings() {
        let asset = fixture_asset();
        let metadata = fixture_metadata();
        let pv = parser_version();
        let warnings = vec![Warning {
            kind: WarningKind::MalformedFrontmatter,
            note: "missing closing fence".into(),
        }];
        let doc =
            build_canonical_document(&asset, metadata, vec![], &pv, warnings).unwrap();
        assert_eq!(doc.provenance.events.len(), 4);
        let last = doc.provenance.events.last().unwrap();
        assert_eq!(last.kind, ProvenanceKind::Warning);
        assert_eq!(last.agent, "kb-parse-md");
        assert!(last.note.as_deref().unwrap().contains("missing closing fence"));
    }

    /// `metadata.user["title"]` and `metadata.user["lang"]` are lifted
    /// to the dedicated `CanonicalDocument` fields and stripped from
    /// the user map (so the wire form does not duplicate the data).
    /// Other user keys survive intact.
    #[test]
    fn lifts_title_and_lang_from_user_map() {
        let asset = fixture_asset();
        let metadata = fixture_metadata();
        let pv = parser_version();
        let doc =
            build_canonical_document(&asset, metadata, vec![], &pv, vec![]).unwrap();
        assert_eq!(doc.title, "Example");
        assert_eq!(doc.lang, Lang("en".into()));
        assert!(!doc.metadata.user.contains_key("title"));
        assert!(!doc.metadata.user.contains_key("lang"));
        assert!(doc.metadata.user.contains_key("custom"));
    }

    /// Determinism property: 1000 iterations of `build_canonical_document`
    /// over identical inputs produce byte-identical JSON, modulo the two
    /// non-deterministic `now_utc()` timestamps for the Parsed/Normalized
    /// events. We strip those timestamps before comparing. Must finish
    /// within 1 second.
    #[test]
    fn determinism_1000_iterations_under_1s() {
        let asset = fixture_asset();
        let metadata = fixture_metadata();
        let pv = parser_version();

        // Helper: serialize and replace the two now_utc-derived timestamps
        // (Parsed + Normalized + any Warning events) with a sentinel so
        // the comparison only checks the deterministic fields.
        fn strip_dynamic_at(doc: &CanonicalDocument) -> Value {
            let mut v = serde_json::to_value(doc).unwrap();
            if let Some(events) = v
                .get_mut("provenance")
                .and_then(|p| p.get_mut("events"))
                .and_then(|e| e.as_array_mut())
            {
                for (i, ev) in events.iter_mut().enumerate() {
                    // index 0 is Discovered (deterministic — pinned in
                    // the fixture). Strip everything after.
                    if i > 0
                        && let Some(obj) = ev.as_object_mut()
                    {
                        obj.insert("at".into(), Value::String("<stripped>".into()));
                    }
                }
            }
            v
        }

        // Use the same 5-block fixture as the ordinal-scoping test so
        // determinism is exercised on a non-empty `lift_block` path
        // (block_id hashing, NFC normalization, ordinal counters), not
        // just an empty Vec.
        let baseline = build_canonical_document(
            &asset,
            metadata.clone(),
            fixture_blocks_five(),
            &pv,
            vec![],
        )
        .unwrap();
        let baseline_json = serde_json::to_string(&strip_dynamic_at(&baseline)).unwrap();

        let start = std::time::Instant::now();
        for _ in 0..1000 {
            let next = build_canonical_document(
                &asset,
                metadata.clone(),
                fixture_blocks_five(),
                &pv,
                vec![],
            )
            .unwrap();
            let next_json = serde_json::to_string(&strip_dynamic_at(&next)).unwrap();
            assert_eq!(baseline_json, next_json);
        }
        assert!(
            start.elapsed() < std::time::Duration::from_secs(1),
            "1000 iterations took {:?}",
            start.elapsed()
        );
    }

    /// I1 regression — `WarningKind::ExtractFailed` is emitted by
    /// `kb-parse-md` (panic-recovery in `blocks.rs`), so the resulting
    /// `ProvenanceEvent::agent` must read `"kb-parse-md"`. A regression
    /// to `"kb-normalize"` would mis-attribute parse panics and break
    /// stage-filtered debugging.
    #[test]
    fn provenance_with_extract_failed_warning_attributes_to_kb_parse_md() {
        let asset = fixture_asset();
        let metadata = fixture_metadata();
        let pv = parser_version();
        let warnings = vec![Warning {
            kind: WarningKind::ExtractFailed,
            note: "pulldown-cmark panicked; body discarded".into(),
        }];
        let doc =
            build_canonical_document(&asset, metadata, vec![], &pv, warnings).unwrap();
        let warning_event = doc
            .provenance
            .events
            .iter()
            .find(|e| e.kind == ProvenanceKind::Warning)
            .expect("warning event present");
        assert_eq!(warning_event.agent, "kb-parse-md");
        assert!(
            warning_event
                .note
                .as_deref()
                .unwrap()
                .contains("ExtractFailed")
        );
    }

    /// I2 regression — `ParsedPayload::AudioRef` is dropped (not lifted
    /// into a `Block::AudioRef` with a synthesized empty `AssetId`,
    /// which would violate `AssetId::from_str`'s 32-hex invariant). A
    /// `Warning` is surfaced in Provenance, attributed to
    /// `"kb-normalize"` because the decision is made at the lift stage.
    #[test]
    fn audio_ref_block_skipped_with_warning() {
        let span = SourceSpan::Line { start: 1, end: 1 };
        let blocks = vec![ParsedBlock {
            kind: kebab_parse_types::ParsedBlockKind::AudioRef,
            heading_path: vec![],
            source_span: span,
            payload: ParsedPayload::AudioRef {
                src: "voice.m4a".into(),
            },
        }];
        let asset = fixture_asset();
        let metadata = fixture_metadata();
        let pv = parser_version();
        let doc =
            build_canonical_document(&asset, metadata, blocks, &pv, vec![]).unwrap();

        // No AudioRef block in the canonical output.
        assert!(
            !doc.blocks
                .iter()
                .any(|b| matches!(b, Block::AudioRef(_))),
            "AudioRef block should be skipped pre-P8"
        );

        // Exactly one Warning event mentioning the AudioRef src.
        let warning_events: Vec<_> = doc
            .provenance
            .events
            .iter()
            .filter(|e| e.kind == ProvenanceKind::Warning)
            .collect();
        assert_eq!(warning_events.len(), 1);
        let w = warning_events[0];
        assert_eq!(w.agent, "kb-normalize");
        assert!(w.note.as_deref().unwrap().contains("voice.m4a"));
    }

    /// I3 regression — heading-path strings are NFC-normalized before
    /// feeding into `id_for_block`, so canonically-equivalent NFD and
    /// NFC inputs produce the same `block_id`. Mirrors
    /// `nfc_nfd_korean_path_same_id` for `doc_id`.
    #[test]
    fn nfc_nfd_korean_heading_path_same_block_id() {
        let span = SourceSpan::Line { start: 1, end: 1 };
        let nfd_heading = "\u{1100}\u{1161}".to_string(); // 가 (NFD)
        let nfc_heading = "\u{AC00}".to_string(); // 가 (NFC)
        let mk_block = |heading: String| ParsedBlock {
            kind: kebab_parse_types::ParsedBlockKind::Paragraph,
            heading_path: vec![heading],
            source_span: span.clone(),
            payload: ParsedPayload::Paragraph {
                text: "p".into(),
                inlines: vec![],
            },
        };
        let asset = fixture_asset();
        let pv = parser_version();
        let doc_nfd = build_canonical_document(
            &asset,
            fixture_metadata(),
            vec![mk_block(nfd_heading)],
            &pv,
            vec![],
        )
        .unwrap();
        let doc_nfc = build_canonical_document(
            &asset,
            fixture_metadata(),
            vec![mk_block(nfc_heading)],
            &pv,
            vec![],
        )
        .unwrap();
        let id_nfd = match &doc_nfd.blocks[0] {
            Block::Paragraph(t) => &t.common.block_id,
            _ => panic!("expected Paragraph"),
        };
        let id_nfc = match &doc_nfc.blocks[0] {
            Block::Paragraph(t) => &t.common.block_id,
            _ => panic!("expected Paragraph"),
        };
        assert_eq!(id_nfd, id_nfc, "NFD and NFC heading paths must hash equal");
    }

    /// M7 (revised by p9-fb-07) — `metadata.user["title"] = ""` lifts
    /// as an empty string but the new derive_title fallback chain
    /// promotes the file stem so the resulting title is non-empty.
    /// spec p9-fb-07: "빈 문자열 반환 금지".
    #[test]
    fn title_empty_string_in_user_map_falls_back_to_file_stem() {
        let asset = fixture_asset();
        let mut metadata = fixture_metadata();
        metadata
            .user
            .insert("title".into(), Value::String(String::new()));
        let pv = parser_version();
        let doc =
            build_canonical_document(&asset, metadata, vec![], &pv, vec![]).unwrap();
        // workspace_path = "notes/example.md" → stem "example".
        assert_eq!(doc.title, "example");
    }

    /// M7 (revised by p9-fb-07) — `metadata.user["title"] = 42` is
    /// non-stringy and silently drops at the lift stage; derive_title
    /// then falls back through the chain to the file stem.
    /// spec p9-fb-07: "빈 문자열 반환 금지".
    #[test]
    fn title_non_string_in_user_map_falls_back_to_file_stem() {
        let asset = fixture_asset();
        let mut metadata = fixture_metadata();
        metadata
            .user
            .insert("title".into(), Value::Number(42.into()));
        let pv = parser_version();
        let doc =
            build_canonical_document(&asset, metadata, vec![], &pv, vec![]).unwrap();
        assert_eq!(doc.title, "example");
    }

    /// M7 — non-stringy `lang` (e.g. an array) silently drops. This is
    /// defensive: P1-2 frontmatter validates the shape upstream, but we
    /// don't trust it.
    #[test]
    fn lang_invalid_shape_silently_drops() {
        let asset = fixture_asset();
        let mut metadata = fixture_metadata();
        metadata.user.insert("lang".into(), Value::Array(vec![]));
        let pv = parser_version();
        let doc =
            build_canonical_document(&asset, metadata, vec![], &pv, vec![]).unwrap();
        assert_eq!(doc.lang, Lang(String::new()));
    }

    // ── p9-fb-07: derive_title fallback chain ───────────────────────────

    fn span() -> SourceSpan {
        SourceSpan::Line { start: 1, end: 1 }
    }

    fn common_for_test() -> CommonBlock {
        CommonBlock {
            block_id: BlockId("0".repeat(32)),
            heading_path: vec![],
            source_span: span(),
        }
    }

    fn heading(level: u8, text: &str) -> Block {
        Block::Heading(HeadingBlock {
            common: common_for_test(),
            level,
            text: text.to_string(),
        })
    }

    fn paragraph(text: &str) -> Block {
        Block::Paragraph(TextBlock {
            common: common_for_test(),
            text: text.to_string(),
            inlines: vec![],
        })
    }

    /// Step 1 — frontmatter title wins, NFC-normalized.
    #[test]
    fn derive_title_uses_frontmatter_first() {
        let blocks = vec![heading(1, "H1 Title"), paragraph("body")];
        assert_eq!(
            derive_title("Frontmatter Title", &blocks, "fallback-stem"),
            "Frontmatter Title"
        );
    }

    /// Whitespace-only frontmatter title falls through to the next step.
    #[test]
    fn derive_title_blank_frontmatter_falls_through_to_h1() {
        let blocks = vec![heading(1, "First H1")];
        assert_eq!(derive_title("   ", &blocks, "stem"), "First H1");
    }

    /// Step 2 — first H1 wins when frontmatter empty.
    #[test]
    fn derive_title_uses_h1_when_no_frontmatter() {
        let blocks = vec![paragraph("intro"), heading(1, "Real Title"), heading(2, "Sub")];
        assert_eq!(derive_title("", &blocks, "stem"), "Real Title");
    }

    /// Step 3 — first H2 wins when no H1.
    #[test]
    fn derive_title_uses_h2_when_no_h1() {
        let blocks = vec![heading(2, "First H2"), heading(2, "Second H2"), heading(1, "")];
        assert_eq!(derive_title("", &blocks, "stem"), "First H2");
    }

    /// Step 4 — first non-blank Paragraph wins; truncated to 80 chars.
    /// Quotes / Lists / Code / Tables / ImageRefs do not qualify.
    #[test]
    fn derive_title_uses_first_paragraph_excerpt() {
        let blocks = vec![
            Block::Quote(TextBlock {
                common: common_for_test(),
                text: "blockquote should be skipped".into(),
                inlines: vec![],
            }),
            Block::Code(CodeBlock {
                common: common_for_test(),
                lang: None,
                code: "code should be skipped".into(),
            }),
            paragraph("This paragraph wins. Long text that would exceed eighty characters once concatenated end-to-end here."),
        ];
        let title = derive_title("", &blocks, "stem");
        assert_eq!(title.chars().count(), 80);
        assert!(title.starts_with("This paragraph wins."));
    }

    /// Step 5 — file stem is the final fallback when there are no
    /// usable blocks (e.g. table-only doc with no paragraphs).
    #[test]
    fn derive_title_falls_back_to_file_stem() {
        let blocks = vec![Block::Table(TableBlock {
            common: common_for_test(),
            headers: vec!["a".into()],
            rows: vec![vec!["1".into()]],
        })];
        assert_eq!(derive_title("", &blocks, "table-only-doc"), "table-only-doc");
    }

    /// Step 5 sentinel — empty file_stem AND no usable blocks falls back
    /// to the literal `"untitled"`. Pathological case (workspace_path
    /// with no filename component).
    #[test]
    fn derive_title_returns_untitled_when_everything_blank() {
        assert_eq!(derive_title("", &[], ""), "untitled");
        assert_eq!(derive_title("   ", &[], "   "), "untitled");
    }

    /// Korean H1 in NFD form is normalized to NFC before being chosen
    /// as the title. Mirrors the heading_path NFC pin elsewhere.
    #[test]
    fn derive_title_nfc_normalizes_korean_h1() {
        let nfd = "\u{1100}\u{1161}".to_string(); // 가 (NFD)
        let nfc = "\u{AC00}".to_string(); // 가 (NFC)
        let blocks = vec![heading(1, &nfd)];
        assert_eq!(derive_title("", &blocks, "stem"), nfc);
    }

    /// `build_canonical_document` integrates the derive_title chain —
    /// when frontmatter title is empty, the first H1 is used.
    #[test]
    fn build_canonical_document_falls_back_to_first_h1() {
        let asset = fixture_asset();
        let mut metadata = fixture_metadata();
        metadata.user.remove("title");
        let blocks = vec![ParsedBlock {
            kind: kebab_parse_types::ParsedBlockKind::Heading,
            heading_path: vec![],
            source_span: span(),
            payload: ParsedPayload::Heading {
                level: 1,
                text: "Lifted From H1".into(),
            },
        }];
        let pv = parser_version();
        let doc =
            build_canonical_document(&asset, metadata, blocks, &pv, vec![]).unwrap();
        assert_eq!(doc.title, "Lifted From H1");
    }

    /// `build_canonical_document` integrates the file_stem fallback —
    /// no frontmatter title, no headings, no paragraphs → filename
    /// (stripped of extension).
    #[test]
    fn build_canonical_document_falls_back_to_file_stem() {
        let asset = fixture_asset();
        // workspace_path = "notes/example.md" → stem "example"
        let mut metadata = fixture_metadata();
        metadata.user.remove("title");
        let pv = parser_version();
        let doc = build_canonical_document(&asset, metadata, vec![], &pv, vec![]).unwrap();
        assert_eq!(doc.title, "example");
    }
}
