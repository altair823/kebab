//! Snapshot test pinning the full `CanonicalDocument` JSON for the
//! `code-and-table.md` fixture.
//!
//! This is an integration test (it lives under `tests/`) and depends on
//! `kb-parse-md` only as a dev-dep so the production crate's regular
//! deps still satisfy the §8 boundary (`cargo tree -p kb-normalize
//! --depth 1` without `-e dev` does not list any parser implementation).
//!
//! Non-deterministic fields are stripped before comparison:
//!
//! * `provenance.events[*].at` — each invocation calls `now_utc()` for
//!   the Parsed/Normalized/Warning events. The Discovered event uses
//!   the asset's pinned `discovered_at`, so we keep that one and replace
//!   only indices ≥ 1.

use std::path::PathBuf;

use kebab_core::{
    AssetId, AssetStorage, Checksum, MediaType, ParserVersion, RawAsset, SourceUri,
    WorkspacePath,
};
use kebab_parse_md::{BodyHints, build_canonical_document, parse_blocks, parse_frontmatter};
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
        source_uri: SourceUri::File(PathBuf::from("/tmp/code-and-table.md")),
        workspace_path: wp,
        media_type: MediaType::Markdown,
        byte_len: 0,
        checksum: Checksum("0".repeat(64)),
        // Pin discovered_at so the Discovered provenance event is
        // deterministic across runs.
        discovered_at: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
        stored: AssetStorage::Reference {
            path: PathBuf::from("/tmp/code-and-table.md"),
            sha: Checksum("0".repeat(64)),
        },
    }
}

fn strip_dynamic(mut v: Value) -> Value {
    if let Some(events) = v
        .get_mut("provenance")
        .and_then(|p| p.get_mut("events"))
        .and_then(|e| e.as_array_mut())
    {
        for (i, ev) in events.iter_mut().enumerate() {
            if i > 0
                && let Some(obj) = ev.as_object_mut()
            {
                obj.insert("at".into(), Value::String("<stripped>".into()));
            }
        }
    }
    v
}

#[test]
fn code_and_table_canonical_snapshot() {
    let dir = fixtures_dir();
    let bytes = std::fs::read(dir.join("code-and-table.md")).expect("fixture readable");

    // Frontmatter parse — code-and-table.md has none, so we provide
    // BodyHints with deterministic timestamps so the lifted Metadata
    // is reproducible. The body offset is 1 (no frontmatter prefix).
    //
    // We pin `first_h1` so the BodyHints → user.title → CanonicalDocument.title
    // lift chain is exercised end-to-end (see `assert_eq!` on
    // `doc.title` below). Without this, `code-and-table.md`'s lack of
    // frontmatter title would leave `title == ""` and the chain would
    // be uncovered by the snapshot.
    let asset = fixed_asset("notes/code-and-table.md");
    let hints = BodyHints {
        first_h1: Some("Code And Table".into()),
        fs_ctime: asset.discovered_at,
        fs_mtime: asset.discovered_at,
        fallback_lang: Some("en".into()),
    };
    let (metadata, fm_span, _fm_warns) =
        parse_frontmatter(&bytes, &hints).expect("frontmatter parses");

    // No frontmatter → body starts at line 1. With frontmatter, line
    // count of the prelude is computed from the byte span; this fixture
    // has none, so the constant 1 is fine.
    let body_offset_lines: u32 = match fm_span {
        // Defensive: count the newlines in the prelude. The fixture
        // hits the `None` branch so this code path is not exercised
        // by the test, but kept for completeness.
        Some(span) => bytes[..span.end].iter().filter(|b| **b == b'\n').count() as u32 + 1,
        None => 1,
    };
    let (blocks, parse_warns) =
        parse_blocks(&bytes, body_offset_lines).expect("blocks parse");

    let parser_version = ParserVersion("kb-normalize-snapshot-test-0".into());
    let mut metadata = metadata;
    // The `created_at` / `updated_at` lifted from BodyHints are pinned
    // to `discovered_at` above, so they are already deterministic.
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

    // Assert the BodyHints → first_h1 → user.title → CanonicalDocument.title
    // lift chain end-to-end. Pinned in the snapshot too, but the explicit
    // assertion makes a future drift fail with a clearer message.
    assert_eq!(doc.title, "Code And Table");

    let actual = strip_dynamic(serde_json::to_value(&doc).unwrap());

    let baseline_path = dir.join("code-and-table.canonical.snapshot.json");
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
            "canonical snapshot drift\n--- expected ({}) ---\n{baseline_text}\n--- actual ---\n{pretty}\nIf intentional, re-run with UPDATE_SNAPSHOTS=1.",
            baseline_path.display()
        );
    }
}
