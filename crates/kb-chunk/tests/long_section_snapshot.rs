//! Snapshot test pinning the `Vec<Chunk>` JSON for the
//! `fixtures/markdown/long-section.md` fixture.
//!
//! This is an integration test. `kb-parse-md` and `kb-normalize` are
//! dev-dep only — `cargo tree -p kb-chunk --depth 1` (default scope,
//! excludes dev-deps) confirms they are not regular deps. The §8
//! module-boundary rule is preserved.
//!
//! The chunker output is fully deterministic given fixed inputs, so we
//! pin the entire `Vec<Chunk>` JSON.
//!
//! Set `UPDATE_SNAPSHOTS=1` to re-bake the baseline.

use std::path::PathBuf;

use kb_chunk::MdHeadingV1Chunker;
use kb_core::{
    AssetId, AssetStorage, Checksum, ChunkPolicy, ChunkerVersion, Chunker, MediaType,
    ParserVersion, RawAsset, SourceUri, WorkspacePath,
};
use kb_normalize::build_canonical_document;
use kb_parse_md::{BodyHints, parse_blocks, parse_frontmatter};
use serde_json::Value;
use time::OffsetDateTime;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
        .join("markdown")
}

fn fixed_asset(workspace_path: &str) -> RawAsset {
    let wp = WorkspacePath::new(workspace_path.into()).unwrap();
    RawAsset {
        asset_id: AssetId("a".repeat(32)),
        source_uri: SourceUri::File(PathBuf::from("/tmp/long-section.md")),
        workspace_path: wp,
        media_type: MediaType::Markdown,
        byte_len: 0,
        checksum: Checksum("0".repeat(64)),
        discovered_at: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
        stored: AssetStorage::Reference {
            path: PathBuf::from("/tmp/long-section.md"),
            sha: Checksum("0".repeat(64)),
        },
    }
}

#[test]
fn long_section_chunks_snapshot() {
    let dir = fixtures_dir();
    let bytes = std::fs::read(dir.join("long-section.md")).expect("fixture readable");

    let asset = fixed_asset("notes/long-section.md");
    let hints = BodyHints {
        first_h1: Some("Alpha".into()),
        fs_ctime: asset.discovered_at,
        fs_mtime: asset.discovered_at,
        fallback_lang: Some("en".into()),
    };
    let (metadata, fm_span, _fm_warns) =
        parse_frontmatter(&bytes, &hints).expect("frontmatter parses");
    let body_offset_lines: u32 = match fm_span {
        Some(span) => bytes[..span.end].iter().filter(|b| **b == b'\n').count() as u32 + 1,
        None => 1,
    };
    let (blocks, parse_warns) =
        parse_blocks(&bytes, body_offset_lines).expect("blocks parse");

    // Pin parser_version so doc_id / block_ids are reproducible.
    let parser_version = ParserVersion("kb-chunk-snapshot-test-0".into());
    let mut metadata = metadata;
    metadata.aliases.sort();
    metadata.tags.sort();

    let doc =
        build_canonical_document(&asset, metadata, blocks, &parser_version, parse_warns)
            .expect("build_canonical_document");

    // Pin policy so policy_hash and chunk_ids are reproducible.
    let policy = ChunkPolicy {
        target_tokens: 200,
        overlap_tokens: 40,
        respect_markdown_headings: true,
        chunker_version: ChunkerVersion("md-heading-v1".into()),
    };

    let chunks = MdHeadingV1Chunker.chunk(&doc, &policy).expect("chunk");
    let actual = serde_json::to_value(&chunks).unwrap();

    let baseline_path = dir.join("long-section.chunks.snapshot.json");
    let baseline_text = match std::fs::read_to_string(&baseline_path) {
        Ok(s) => s,
        Err(_) if std::env::var("UPDATE_SNAPSHOTS").is_ok() => {
            let pretty = serde_json::to_string_pretty(&actual).unwrap();
            std::fs::write(&baseline_path, format!("{pretty}\n")).unwrap();
            return;
        }
        Err(e) => panic!(
            "missing baseline {}; run with UPDATE_SNAPSHOTS=1 to create: {e}",
            baseline_path.display()
        ),
    };
    let expected: Value =
        serde_json::from_str(&baseline_text).expect("baseline parses as json");

    if actual != expected {
        if std::env::var("UPDATE_SNAPSHOTS").is_ok() {
            let pretty = serde_json::to_string_pretty(&actual).unwrap();
            std::fs::write(&baseline_path, format!("{pretty}\n")).unwrap();
            eprintln!("updated baseline {}", baseline_path.display());
            return;
        }
        let pretty = serde_json::to_string_pretty(&actual).unwrap();
        panic!(
            "long-section chunks snapshot drift\n\
             --- expected ({}) ---\n{baseline_text}\n\
             --- actual ---\n{pretty}\n\
             If intentional, re-run with UPDATE_SNAPSHOTS=1.",
            baseline_path.display()
        );
    }
}

/// Determinism cross-check: re-running the same pipeline yields the same
/// chunk_ids byte-for-byte.
#[test]
fn long_section_chunks_are_deterministic() {
    let dir = fixtures_dir();
    let bytes = std::fs::read(dir.join("long-section.md")).expect("fixture readable");

    let asset = fixed_asset("notes/long-section.md");
    let hints = BodyHints {
        first_h1: Some("Alpha".into()),
        fs_ctime: asset.discovered_at,
        fs_mtime: asset.discovered_at,
        fallback_lang: Some("en".into()),
    };

    let policy = ChunkPolicy {
        target_tokens: 200,
        overlap_tokens: 40,
        respect_markdown_headings: true,
        chunker_version: ChunkerVersion("md-heading-v1".into()),
    };
    let parser_version = ParserVersion("kb-chunk-snapshot-test-0".into());

    let mut baseline: Option<Vec<String>> = None;
    for _ in 0..5 {
        let (metadata, _fm_span, _fm_warns) =
            parse_frontmatter(&bytes, &hints).expect("frontmatter parses");
        let (blocks, parse_warns) = parse_blocks(&bytes, 1).expect("blocks parse");
        let mut metadata = metadata;
        metadata.aliases.sort();
        metadata.tags.sort();
        let doc = build_canonical_document(
            &asset,
            metadata,
            blocks,
            &parser_version,
            parse_warns,
        )
        .expect("build_canonical_document");
        let ids: Vec<String> = MdHeadingV1Chunker
            .chunk(&doc, &policy)
            .unwrap()
            .into_iter()
            .map(|c| c.chunk_id.0)
            .collect();
        match &baseline {
            None => baseline = Some(ids),
            Some(prev) => assert_eq!(prev, &ids),
        }
    }
}
