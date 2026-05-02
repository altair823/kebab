//! Asset writer tests: copy mode (file written 0o644), reference mode
//! (no copy, row records source), and checksum mismatch (Conflict).

use std::path::PathBuf;

use kebab_core::{AssetId, AssetStorage, Checksum, MediaType, RawAsset, SourceUri, WorkspacePath};
use kebab_store_sqlite::SqliteStore;
use time::OffsetDateTime;

mod common;

fn fixed_asset(_bytes: &[u8], byte_len: u64, declared_checksum: &str) -> RawAsset {
    RawAsset {
        // 32-hex AssetId per kb-core newtype invariant.
        asset_id: AssetId("a".repeat(32)),
        source_uri: SourceUri::File(PathBuf::from("/some/source.md")),
        workspace_path: WorkspacePath::new("notes/foo.md".into()).unwrap(),
        media_type: MediaType::Markdown,
        byte_len,
        checksum: Checksum(declared_checksum.into()),
        discovered_at: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
        stored: AssetStorage::Reference {
            path: PathBuf::from("/some/source.md"),
            sha: Checksum("0".repeat(64)),
        },
    }
}

fn b3_full_hex(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

#[test]
fn copy_mode_writes_file_with_0o644_and_correct_bytes() {
    let env = common::TestEnv::with_threshold(100);
    let store = SqliteStore::open(&env.config()).unwrap();
    store.run_migrations().unwrap();

    let bytes = b"hello, sqlite";
    let cs = b3_full_hex(bytes);
    let asset = fixed_asset(bytes, bytes.len() as u64, &cs);

    store.put_asset_with_bytes(&asset, bytes).expect("write");

    // Path: data_dir/assets/aa/aaaaaa…aa
    let aa = &asset.asset_id.0[..2];
    let dest = env.data_dir().join("assets").join(aa).join(&asset.asset_id.0);
    assert!(dest.exists(), "asset file not written at {}", dest.display());
    let on_disk = std::fs::read(&dest).unwrap();
    assert_eq!(on_disk, bytes);

    // Mode 0o644 on Unix.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&dest).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o644, "expected 0o644, got 0o{mode:o}");
    }

    // Row recorded copied.
    let storage_kind: String = env.with_conn(|c| {
        c.query_row(
            "SELECT storage_kind FROM assets WHERE asset_id = ?",
            [&asset.asset_id.0],
            |r| r.get(0),
        )
    });
    assert_eq!(storage_kind, "copied");
}

#[test]
fn reference_mode_does_not_write_file_but_records_path() {
    // copy_threshold_mb=0 → every byte lands on the reference branch.
    let env = common::TestEnv::with_threshold(0);
    let store = SqliteStore::open(&env.config()).unwrap();
    store.run_migrations().unwrap();

    let bytes = b"big-pretend-bytes";
    let cs = b3_full_hex(bytes);
    // byte_len declared > 0 so the threshold check picks reference. With
    // copy_threshold_bytes=0 even byte_len=1 trips the else branch.
    let mut asset = fixed_asset(bytes, 1, &cs);
    asset.source_uri = SourceUri::File(PathBuf::from("/path/to/original.md"));

    store.put_asset_with_bytes(&asset, bytes).expect("ref write");

    let aa = &asset.asset_id.0[..2];
    let dest = env.data_dir().join("assets").join(aa).join(&asset.asset_id.0);
    assert!(!dest.exists(), "reference mode must not copy bytes");

    let (storage_kind, storage_path): (String, String) = env.with_conn(|c| {
        c.query_row(
            "SELECT storage_kind, storage_path FROM assets WHERE asset_id = ?",
            [&asset.asset_id.0],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
    });
    assert_eq!(storage_kind, "reference");
    assert_eq!(storage_path, "/path/to/original.md");
}

#[test]
fn put_asset_with_bytes_sweeps_workspace_path_orphan() {
    // HOTFIXES 2026-05-02 P7-3: the original behaviour erred on
    // workspace_path UNIQUE conflict (`ON CONFLICT(asset_id)` only) so
    // a re-ingest of an edited file was unrecoverable. The fix is
    // `purge_orphan_at_workspace_path`, which sweeps the stale
    // documents → assets chain before the new INSERT lands.
    //
    // This test exercises the no-documents flavour (raw asset row only)
    // — the put_asset_with_bytes path. The documents-cascade flavour
    // is exercised end-to-end in `kebab-app::tests::pdf_pipeline::
    // re_ingest_edited_pdf_produces_new_doc_id`.
    let env = common::TestEnv::with_threshold(100);
    let store = SqliteStore::open(&env.config()).unwrap();
    store.run_migrations().unwrap();

    // Pre-populate a row that owns `notes/foo.md` under a *different*
    // asset_id, simulating a prior ingest of an earlier byte version.
    env.with_conn(|c| {
        c.execute(
            "INSERT INTO assets (
                asset_id, source_uri, workspace_path, media_type, byte_len,
                checksum, storage_kind, storage_path, discovered_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params![
                "b".repeat(32),
                "file:///elsewhere/foo.md",
                "notes/foo.md",
                "\"markdown\"",
                7_i64,
                "0".repeat(64),
                "reference",
                "/elsewhere/foo.md",
                "2024-01-01T00:00:00Z",
            ],
        )
    });

    let bytes = b"hello, sqlite";
    let cs = b3_full_hex(bytes);
    let asset = fixed_asset(bytes, bytes.len() as u64, &cs);

    store
        .put_asset_with_bytes(&asset, bytes)
        .expect("orphan sweep + INSERT must succeed");

    // Stale row gone, new row owns the workspace_path.
    let stale_count: i64 = env.with_conn(|c| {
        c.query_row(
            "SELECT COUNT(*) FROM assets WHERE asset_id = ?",
            rusqlite::params!["b".repeat(32)],
            |row| row.get(0),
        )
    });
    assert_eq!(stale_count, 0, "stale asset_id must be purged");
    let new_count: i64 = env.with_conn(|c| {
        c.query_row(
            "SELECT COUNT(*) FROM assets WHERE asset_id = ?",
            rusqlite::params![asset.asset_id.0],
            |row| row.get(0),
        )
    });
    assert_eq!(new_count, 1, "new asset_id must own the workspace_path slot");

    // New asset's bytes published at the final destination.
    let aa = &asset.asset_id.0[..2];
    let dest = env.data_dir().join("assets").join(aa).join(&asset.asset_id.0);
    assert!(
        dest.exists(),
        "new asset bytes must be visible at {}",
        dest.display()
    );
}

#[test]
fn put_asset_with_bytes_rejects_invalid_asset_id() {
    // `kebab_core::AssetId(pub String)` lets a hand-construction bypass the
    // 32-hex `FromStr` invariant. The store boundary must reject any ID
    // whose shape would let path construction escape `data_dir/assets/`.
    let env = common::TestEnv::with_threshold(100);
    let store = SqliteStore::open(&env.config()).unwrap();
    store.run_migrations().unwrap();

    // 32 chars but contains a `/` — would let `assets_path_for` stitch
    // together a path outside the shard tree.
    let evil_id = "../etc/passwd_padded_to_xx_xxxxx".to_string();
    assert_eq!(evil_id.len(), 32, "test fixture must be 32 chars to exercise length-only checks");
    let mut asset = fixed_asset(b"x", 1, &b3_full_hex(b"x"));
    asset.asset_id = AssetId(evil_id.clone());

    let err = store
        .put_asset_with_bytes(&asset, b"x")
        .expect_err("must reject non-hex AssetId");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("invalid AssetId shape"),
        "expected AssetId-shape rejection, got: {msg}"
    );

    // And the bytes must NOT have been staged anywhere under the assets
    // tree (no I/O should have happened before validation).
    let assets_dir = env.data_dir().join("assets");
    if assets_dir.exists() {
        for entry in std::fs::read_dir(&assets_dir).unwrap().flatten() {
            // Recurse one level into shard dirs and assert empty.
            if let Some(sub) = std::fs::read_dir(entry.path()).unwrap().flatten().next() {
                panic!(
                    "invalid AssetId still produced filesystem artifact at {}",
                    sub.path().display()
                );
            }
        }
    }
}

#[test]
fn checksum_mismatch_returns_conflict() {
    let env = common::TestEnv::new();
    let store = SqliteStore::open(&env.config()).unwrap();
    store.run_migrations().unwrap();

    let bytes = b"the real bytes";
    // Tampered checksum: hash a different payload.
    let wrong_cs = b3_full_hex(b"different bytes");
    let asset = fixed_asset(bytes, bytes.len() as u64, &wrong_cs);

    let err = store
        .put_asset_with_bytes(&asset, bytes)
        .expect_err("must reject checksum mismatch");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("checksum mismatch") || msg.contains("conflict"),
        "expected Conflict-flavoured error, got: {msg}"
    );
}
