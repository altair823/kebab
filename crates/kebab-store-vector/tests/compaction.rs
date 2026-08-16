//! Regression test for Lance version/fragment accumulation.
//!
//! Every `upsert` is one Lance `merge_insert`, and every `merge_insert`
//! creates a new table version whose manifest lists **all** fragments in
//! the table. kebab upserts once per ingested document, so without
//! periodic compaction an N-document corpus leaves N fragments and the
//! N-th write rewrites a manifest with N entries — write cost grows
//! linearly with corpus size and manifest bytes grow quadratically.
//!
//! Measured on a 16.8k-document dogfood KB before the fix: ingest decayed
//! from 30.7 to 4.3 documents/min, and `_versions/` held 12.2 GB of
//! manifests against 1.7 GB of actual vectors. One full compaction took
//! 153 s and brought the table back to 1.70 GB / 1 version.
//!
//! This test drives many more upserts than the compaction interval and
//! asserts the on-disk version count stays bounded.
//!
//! `#[ignore]` + AVX gate per `tests/common/mod.rs` policy.

use kebab_core::VectorStore;

mod common;
use common::{TestEnv, make_record, require_avx_or_panic};

const MODEL: &str = "compaction-model";

/// Count `*.manifest` files under the single Lance table directory.
/// Lance keeps one manifest per surviving version, so this is a direct
/// proxy for "how much history is still on disk".
fn manifest_count(data_dir: &std::path::Path) -> usize {
    let lance_root = data_dir.join("lancedb");
    let mut n = 0;
    let mut stack = vec![lance_root];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "manifest") {
                n += 1;
            }
        }
    }
    n
}

#[test]
#[ignore = "requires AVX-capable hardware (LanceDB)"]
fn repeated_upserts_do_not_accumulate_lance_versions() {
    require_avx_or_panic();

    // Compact every 8 upserts so the test stays quick; production uses
    // `COMPACT_EVERY_N_UPSERTS`.
    let env = TestEnv::with_compact_interval(8);
    env.seed_chunk(
        &format!("{:032x}", 0x1100u32),
        &format!("{:032x}", 0xd0c0u32),
        "note.md",
        "en",
        &[],
        "primary",
    );

    let rec = make_record(0, 0, vec![1.0, 0.0, 0.0, 0.0], "hi", &[], MODEL);
    const UPSERTS: usize = 40;
    for _ in 0..UPSERTS {
        env.vector.upsert(std::slice::from_ref(&rec)).unwrap();
    }

    let manifests = manifest_count(&env.data_dir());
    assert!(
        manifests <= 12,
        "Lance versions accumulated: {manifests} manifests after {UPSERTS} upserts \
         (compaction interval 8). Without compaction this grows one-per-upsert."
    );

    // Compaction must not lose rows: the record is still searchable.
    let hits = env
        .vector
        .search(
            &[1.0, 0.0, 0.0, 0.0],
            5,
            &kebab_core::SearchFilters::default(),
        )
        .unwrap();
    assert_eq!(hits.len(), 1, "compaction dropped the upserted row");
}

/// The delete path commits just like the upsert path, and `sweep_deleted_files`
/// used to call it once per purged document — issue #230 measured 5,834 purges
/// becoming 5,834 Lance commits. Batching moved that to one call per sweep, but
/// a large sweep still commits per 200-id batch, so the delete path needs the
/// same compaction the upsert path got.
#[test]
#[ignore = "requires AVX-capable hardware (LanceDB)"]
fn repeated_deletes_do_not_accumulate_lance_versions() {
    require_avx_or_panic();

    let env = TestEnv::with_compact_interval(8);
    let doc = format!("{:032x}", 0xd0c0u32);
    for i in 0..40u8 {
        env.seed_chunk(
            &format!("{:032x}", 0x2000u32 + u32::from(i)),
            &doc,
            "note.md",
            "en",
            &[],
            "primary",
        );
    }

    let recs: Vec<_> = (0..40u8)
        .map(|i| {
            let mut r = make_record(i, 0, vec![1.0, 0.0, 0.0, 0.0], "hi", &[], MODEL);
            r.chunk_id = kebab_core::ChunkId(format!("{:032x}", 0x2000u32 + u32::from(i)));
            r
        })
        .collect();
    env.vector.upsert(&recs).unwrap();

    // Delete one at a time — the shape the un-batched sweep produced.
    for r in &recs {
        env.vector
            .delete_by_chunk_ids(std::slice::from_ref(&r.chunk_id))
            .unwrap();
    }

    let manifests = manifest_count(&env.data_dir());
    assert!(
        manifests <= 12,
        "delete path accumulated Lance versions: {manifests} manifests after 40 deletes \
         (compaction interval 8)"
    );
}
