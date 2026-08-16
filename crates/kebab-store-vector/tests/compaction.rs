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
/// becoming 5,834 Lance commits. Batching moved that to one call per flush, but
/// one call still commits per 200-id batch, so the delete path needs the same
/// compaction the upsert path got.
///
/// Deliberately large enough that a single call spans more than
/// `compact_every` batches. That is the case a `version % compact_every == 0`
/// trigger misses: the call steps the version by several at once and clears the
/// multiple without ever landing on it. A handful of ids would pass against
/// either trigger and prove nothing.
#[test]
#[ignore = "requires AVX-capable hardware (LanceDB)"]
fn batched_delete_spanning_many_commits_still_compacts() {
    require_avx_or_panic();

    const INTERVAL: u64 = 8;
    // `delete_by_chunk_ids` commits per 200 ids, so this is ~10 commits in
    // one call — comfortably past INTERVAL.
    const IDS: u32 = 2_000;

    let env = TestEnv::with_compact_interval(INTERVAL);
    let doc = format!("{:032x}", 0xd0c0u32);
    for i in 0..IDS {
        env.seed_chunk(
            &format!("{:032x}", 0x2000u32 + i),
            &doc,
            "note.md",
            "en",
            &[],
            "primary",
        );
    }

    let recs: Vec<_> = (0..IDS)
        .map(|i| {
            let mut r = make_record(0, 0, vec![1.0, 0.0, 0.0, 0.0], "hi", &[], MODEL);
            r.chunk_id = kebab_core::ChunkId(format!("{:032x}", 0x2000u32 + i));
            r.embedding_id = kebab_core::EmbeddingId(format!("{:032x}", 0xee000000u32 + i));
            r
        })
        .collect();
    env.vector.upsert(&recs).unwrap();
    let after_upsert = manifest_count(&env.data_dir());

    let ids: Vec<_> = recs.iter().map(|r| r.chunk_id.clone()).collect();
    env.vector.delete_by_chunk_ids(&ids).unwrap();

    let manifests = manifest_count(&env.data_dir());
    assert!(
        manifests <= after_upsert + 2,
        "batched delete left {manifests} manifests (was {after_upsert} before the delete) — \
         a single {IDS}-id call spans ~{} commits and must still trigger compaction",
        IDS / 200
    );
}
