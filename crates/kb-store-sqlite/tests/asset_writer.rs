//! Asset writer tests: copy mode (file written 0o644), reference mode
//! (no copy, row records source), and checksum mismatch (Conflict).

use std::path::PathBuf;

use kb_core::{AssetId, AssetStorage, Checksum, MediaType, RawAsset, SourceUri, WorkspacePath};
use kb_store_sqlite::SqliteStore;
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
