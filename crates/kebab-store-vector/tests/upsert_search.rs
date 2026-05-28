//! Integration tests for `LanceVectorStore` covering ensure_table,
//! upsert, search, dimension mismatch, filters, model isolation, and
//! determinism.
//!
//! Every test in this file is `#[ignore]` and requires an AVX-capable
//! x86_64 host. Run with:
//!
//! ```text
//! cargo test -p kb-store-vector -- --ignored
//! ```
//!
//! See `tests/common/mod.rs` for the full rationale.

use kebab_core::{EmbeddingModelId, SearchFilters, VectorStore};
use kebab_store_sqlite::EmbeddingRecordRow;
use rusqlite::params;
use time::OffsetDateTime;

mod common;
use common::{TestEnv, make_record, require_avx_or_panic};

const MODEL: &str = "test-model";

/// Helper: produce a unit-norm 4-D vector pointing in one of four
/// directions. The sign pattern keeps cosine similarities cleanly
/// distinct so search ordering tests don't depend on float jitter.
fn dir(idx: u8) -> Vec<f32> {
    match idx {
        0 => vec![1.0, 0.0, 0.0, 0.0],
        1 => vec![0.0, 1.0, 0.0, 0.0],
        2 => vec![0.0, 0.0, 1.0, 0.0],
        _ => vec![0.0, 0.0, 0.0, 1.0],
    }
}

#[test]
#[ignore = "requires AVX-capable hardware (LanceDB)"]
fn ensure_table_idempotent_returns_same_index_id() {
    require_avx_or_panic();
    let env = TestEnv::new();
    let model = EmbeddingModelId(MODEL.to_string());
    let id1 = env.vector.ensure_table(&model, 4).unwrap();
    let id2 = env.vector.ensure_table(&model, 4).unwrap();
    assert_eq!(id1, id2);
}

#[test]
#[ignore = "requires AVX-capable hardware (LanceDB)"]
fn search_before_upsert_returns_empty() {
    require_avx_or_panic();
    let env = TestEnv::new();
    let hits = env
        .vector
        .search(&dir(0), 5, &SearchFilters::default())
        .unwrap();
    assert!(hits.is_empty());
}

#[test]
#[ignore = "requires AVX-capable hardware (LanceDB)"]
fn upsert_ten_then_search_returns_five() {
    require_avx_or_panic();
    let env = TestEnv::new();
    let mut recs = Vec::new();
    for i in 0..10u8 {
        // 4-D vectors clustered near dir(0) for the first half, dir(1)
        // for the rest, with small per-row jitter so they stay
        // distinct in the index.
        let mut v = if i < 5 { dir(0) } else { dir(1) };
        v[3] = f32::from(i) * 0.001;
        let rec = make_record(i, i, v, &format!("text-{i}"), &["A"], MODEL);
        env.seed_chunk(
            &rec.chunk_id.0,
            &rec.doc_id.0,
            &format!("notes/{i}.md"),
            "en",
            &[],
            "primary",
        );
        recs.push(rec);
    }
    env.vector.upsert(&recs).unwrap();

    // 1:1 alignment check: every record has a committed embedding row.
    {
        let conn = env.sqlite.read_conn();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM embedding_records WHERE status = 'committed'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 10);
    }

    let hits = env
        .vector
        .search(&dir(0), 5, &SearchFilters::default())
        .unwrap();
    assert_eq!(hits.len(), 5, "expected 5 hits, got {}", hits.len());

    // Top hits should be from the first half (clustered around dir(0)).
    // make_record lays chunk_idx into the low bits of `0x1100 + i`, so
    // `chunk_idx = u32::from_str_radix(last4, 16) - 0x1100`. The first
    // half (chunk_idx < 5) lives in 0x1100..=0x1104.
    for h in &hits {
        let suffix_hex = &h.chunk_id.0[h.chunk_id.0.len() - 4..];
        let idx = u32::from_str_radix(suffix_hex, 16).unwrap();
        let chunk_idx = idx - 0x1100;
        assert!(
            chunk_idx < 5,
            "top-5 hit unexpectedly came from second cluster: idx={chunk_idx}"
        );
    }
}

#[test]
#[ignore = "requires AVX-capable hardware (LanceDB)"]
fn dimension_mismatch_errors_and_writes_nothing() {
    require_avx_or_panic();
    let env = TestEnv::new();
    let model = EmbeddingModelId(MODEL.to_string());

    // First populate a 4-D table with one row so it exists on disk.
    let r0 = make_record(0, 0, dir(0), "first", &[], MODEL);
    env.seed_chunk(
        &r0.chunk_id.0,
        &r0.doc_id.0,
        "notes/0.md",
        "en",
        &[],
        "primary",
    );
    env.vector.upsert(&[r0]).unwrap();
    assert_eq!(
        env.vector.ensure_table(&model, 4).unwrap(),
        env.vector.ensure_table(&model, 4).unwrap()
    );

    // Now manually open the same table_name path and try to upsert
    // an 8-D vector through `upsert` — the table name function bakes
    // dim into the name, so the only way to drive the real
    // record-vs-table mismatch is to corrupt `dimensions` so the
    // table_name is the existing 4-D table, but the embedded vector
    // is 8-D. Spec line 94: must error, write nothing extra.
    let mut bad = make_record(1, 1, vec![0.1_f32; 8], "second", &[], MODEL);
    // Pretend this is a 4-D vector for table-name purposes; the
    // build_batch then enforces that vector.len() == dim and bails.
    bad.dimensions = 4;
    env.seed_chunk(
        &bad.chunk_id.0,
        &bad.doc_id.0,
        "notes/1.md",
        "en",
        &[],
        "primary",
    );

    let bad_chunk = bad.chunk_id.0.clone();
    let err = env.vector.upsert(&[bad]).unwrap_err();
    let msg = format!("{err:#}");
    assert!(
        msg.to_lowercase().contains("dim") || msg.contains("does not match table dim"),
        "unexpected error message: {msg}"
    );

    // The phase-1 row may have landed before phase 2 detected the
    // mismatch — but the on-disk Lance table must NOT contain the
    // bad record. So we assert that no `committed` row corresponds
    // to chunk_id of the bad record.
    let conn = env.sqlite.read_conn();
    let committed: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM embedding_records WHERE chunk_id = ? AND status = 'committed'",
            rusqlite::params![bad_chunk],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        committed, 0,
        "bad record reached committed state despite dim mismatch"
    );
}

#[test]
#[ignore = "requires AVX-capable hardware (LanceDB)"]
fn filter_tags_any_drops_non_matching_docs() {
    require_avx_or_panic();
    let env = TestEnv::new();

    // Two docs: one with tag "ko-style", one without.
    let r_a = make_record(0xaa, 0xaa, dir(0), "alpha", &[], MODEL);
    let r_b = make_record(0xbb, 0xbb, dir(0), "beta", &[], MODEL);
    env.seed_chunk(
        &r_a.chunk_id.0,
        &r_a.doc_id.0,
        "notes/a.md",
        "en",
        &["ko-style"],
        "primary",
    );
    env.seed_chunk(
        &r_b.chunk_id.0,
        &r_b.doc_id.0,
        "notes/b.md",
        "en",
        &["other"],
        "primary",
    );
    let expected_doc_id = r_a.doc_id.0.clone();
    env.vector.upsert(&[r_a, r_b]).unwrap();

    let filters = SearchFilters {
        tags_any: vec!["ko-style".to_string()],
        ..Default::default()
    };
    let hits = env.vector.search(&dir(0), 10, &filters).unwrap();
    assert_eq!(hits.len(), 1, "expected only the tagged doc to match");
    let payload = &hits[0].payload;
    assert_eq!(payload["doc_id"], expected_doc_id);
}

#[test]
#[ignore = "requires AVX-capable hardware (LanceDB)"]
fn model_isolation_two_models_two_directories() {
    require_avx_or_panic();
    let env = TestEnv::new();
    let r1 = make_record(0xaa, 0xaa, dir(0), "alpha", &[], "model-A");
    env.seed_chunk(
        &r1.chunk_id.0,
        &r1.doc_id.0,
        "notes/a.md",
        "en",
        &[],
        "primary",
    );
    let chunk_id = r1.chunk_id.0.clone();
    env.vector.upsert(&[r1]).unwrap();

    // Same chunk_id, different model — should land in a separate table.
    let mut r2 = make_record(0xaa, 0xaa, dir(0), "alpha", &[], "model-B");
    r2.embedding_id = kebab_core::EmbeddingId("ee01ee01ee01ee01ee01ee01ee01ee01".to_string());
    env.vector.upsert(&[r2]).unwrap();

    // Two on-disk Lance directories, distinguished by table name.
    let lancedb_root = env.data_dir().join("lancedb");
    let entries: Vec<_> = std::fs::read_dir(&lancedb_root)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    let a_count = entries.iter().filter(|e| e.contains("model-A")).count();
    let b_count = entries.iter().filter(|e| e.contains("model-B")).count();
    assert!(a_count >= 1, "model-A table missing: {entries:?}");
    assert!(b_count >= 1, "model-B table missing: {entries:?}");

    // Two embedding_records rows for the same chunk_id, one per model.
    let conn = env.sqlite.read_conn();
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM embedding_records WHERE chunk_id = ?",
            params![chunk_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 2);
}

#[test]
#[ignore = "requires AVX-capable hardware (LanceDB)"]
fn determinism_same_query_same_top_k() {
    require_avx_or_panic();
    let env = TestEnv::new();
    let recs: Vec<_> = (0..6u8)
        .map(|i| {
            let mut v = dir(i % 4);
            v[3] = f32::from(i) * 0.001;
            let rec = make_record(i, i, v, &format!("t-{i}"), &[], MODEL);
            env.seed_chunk(
                &rec.chunk_id.0,
                &rec.doc_id.0,
                &format!("notes/{i}.md"),
                "en",
                &[],
                "primary",
            );
            rec
        })
        .collect();
    env.vector.upsert(&recs).unwrap();

    let q = dir(0);
    let h1 = env.vector.search(&q, 4, &SearchFilters::default()).unwrap();
    let h2 = env.vector.search(&q, 4, &SearchFilters::default()).unwrap();
    let ids1: Vec<_> = h1.iter().map(|h| h.chunk_id.0.clone()).collect();
    let ids2: Vec<_> = h2.iter().map(|h| h.chunk_id.0.clone()).collect();
    assert_eq!(ids1, ids2);
}

#[test]
#[ignore = "requires AVX-capable hardware (LanceDB)"]
fn upsert_retry_promotes_pending_to_committed() {
    // Crash-recovery contract: a phase-1 row that was already
    // committed by a prior batch is left alone by phase-3, but a
    // pending row gets retried and reaches committed once Lance
    // accepts it.
    //
    // Construction of the "crash" state:
    //
    //   1. Stage a row directly via the SQLite phase-1 helper
    //      (`put_embedding_records_pending`). NO Lance write happens
    //      here — this is exactly the on-disk state after a crash
    //      between phase 1 and phase 2. Confirm the row is at
    //      `status='pending'` before doing anything else.
    //
    //   2. Run `LanceVectorStore::upsert` with a `VectorRecord` whose
    //      `embedding_id` matches the pending row. Phase 1's
    //      `INSERT OR REPLACE` is idempotent here (same row payload),
    //      phase 2 actually writes to Lance for the first time, and
    //      phase 3 flips the row to 'committed'.
    //
    //   3. Verify status='committed' and vector_committed=1.
    //
    // This actually exercises the "rows stuck at pending get promoted
    // on next upsert" semantics — the previous version pre-seeded via
    // raw SQL but then the same upsert call overwrote the seed via
    // INSERT OR REPLACE before phase 2 ran, so the recovery path
    // never executed.
    require_avx_or_panic();
    let env = TestEnv::new();
    let rec = make_record(0xaa, 0xaa, dir(0), "alpha", &[], MODEL);
    let chunk_id = rec.chunk_id.0.clone();
    let doc_id = rec.doc_id.0.clone();
    let embedding_id = rec.embedding_id.0.clone();
    env.seed_chunk(&chunk_id, &doc_id, "notes/a.md", "en", &[], "primary");

    // Phase 1 only — go through the same kb-store-sqlite helper that
    // `LanceVectorStore::upsert` uses internally. No Lance write
    // happens, so this models "crashed between phase 1 and phase 2".
    let pending_row = EmbeddingRecordRow {
        embedding_id: embedding_id.clone(),
        chunk_id: chunk_id.clone(),
        model_id: MODEL.to_string(),
        model_version: "v1".to_string(),
        dimensions: 4,
        lance_table: format!("chunk_embeddings_{MODEL}_4"),
        created_at: OffsetDateTime::UNIX_EPOCH,
    };
    env.sqlite
        .put_embedding_records_pending(std::slice::from_ref(&pending_row))
        .unwrap();

    // Sanity: the row is staged but NOT yet committed and Lance has
    // no record of it.
    {
        let conn = env.sqlite.read_conn();
        let (status, committed): (String, i64) = conn
            .query_row(
                "SELECT status, vector_committed FROM embedding_records WHERE embedding_id = ?",
                params![embedding_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            status, "pending",
            "row should be at status=pending after phase-1-only"
        );
        assert_eq!(committed, 0);
    }

    // Now run upsert with the matching record. Phase 1's INSERT OR
    // REPLACE is a no-op equivalent (same row payload), phase 2 lands
    // the Lance row for the first time, phase 3 promotes
    // status='committed'.
    env.vector.upsert(&[rec]).unwrap();

    let conn = env.sqlite.read_conn();
    let (status, committed): (String, i64) = conn
        .query_row(
            "SELECT status, vector_committed FROM embedding_records WHERE embedding_id = ?",
            params![embedding_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(status, "committed");
    assert_eq!(committed, 1);
}
