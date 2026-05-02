//! Snapshot test: a fixed corpus + fixed query produces a stable
//! `Vec<VectorHit>` JSON. Pinning the snapshot here catches accidental
//! drift in score scaling, payload shape, or top-k ordering.
//!
//! This test is `#[ignore]` and requires AVX-capable hardware. Run
//! with `cargo test -p kb-store-vector -- --ignored snapshot`.
//!
//! The committed fixture at `tests/fixtures/vector/run-1.json` is a
//! placeholder until first regenerated on AVX hardware. The test
//! detects the placeholder via its `_comment` field and panics with
//! a clear "regenerate me" message — see `assert_no_placeholder`
//! below.

use std::path::PathBuf;

use kebab_core::{SearchFilters, VectorStore};
use serde_json::json;

mod common;
use common::{TestEnv, make_record, require_avx_or_panic};

const MODEL: &str = "snapshot-model";

#[test]
#[ignore = "requires AVX-capable hardware (LanceDB)"]
fn vector_hits_snapshot_run_1() {
    require_avx_or_panic();
    let env = TestEnv::new();
    // Fixed deterministic corpus: 4 unit-norm vectors, each with a
    // known doc / chunk / heading. The query points squarely at
    // chunk 0 so the expected ordering is 0, then the others by
    // distance from dir(0).
    let corpus = vec![
        (0u8, vec![1.0_f32, 0.0, 0.0, 0.0], "alpha", &["A"][..]),
        (1u8, vec![0.95_f32, 0.31, 0.0, 0.0], "beta", &["A", "B"][..]),
        (2u8, vec![0.0_f32, 1.0, 0.0, 0.0], "gamma", &["B"][..]),
        (3u8, vec![0.0_f32, 0.0, 1.0, 0.0], "delta", &[][..]),
    ];

    let mut recs = Vec::new();
    for (i, vec, text, headings) in &corpus {
        let rec = make_record(*i, *i, vec.clone(), text, headings, MODEL);
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

    let q = vec![1.0_f32, 0.0, 0.0, 0.0];
    let hits = env.vector.search(&q, 3, &SearchFilters::default()).unwrap();

    // The snapshot pins:
    //   - top-3 chunk_id ordering (by score desc)
    //   - payload shape: { doc_id, text, heading_path }
    //   - that scores live in [0, 1] and are sorted descending
    let actual = json!(
        hits.iter().map(|h| json!({
            "chunk_id": h.chunk_id.0,
            "score_in_unit_interval": (0.0..=1.0).contains(&h.score),
            "payload": h.payload,
        })).collect::<Vec<_>>()
    );

    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("vector")
        .join("run-1.json");

    if std::env::var_os("KB_UPDATE_SNAPSHOTS").is_some() {
        std::fs::create_dir_all(fixture.parent().unwrap()).unwrap();
        std::fs::write(&fixture, serde_json::to_string_pretty(&actual).unwrap())
            .unwrap();
        return;
    }

    let expected: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&fixture).unwrap_or_else(
            |_| panic!(
                "missing snapshot fixture at {}; run with KB_UPDATE_SNAPSHOTS=1 to create",
                fixture.display()
            ),
        ))
        .unwrap();

    // Refuse to silently "pass" when the fixture is the committed
    // placeholder. The placeholder JSON carries a `_comment` field
    // with regeneration instructions; production fixtures (a captured
    // hits array) do not.
    if expected.get("_comment").is_some() {
        panic!(
            "snapshot fixture is a placeholder — regenerate on AVX hardware then commit. \
             Path: {}. To regenerate: \
             `KB_UPDATE_SNAPSHOTS=1 cargo test -p kb-store-vector -- --ignored snapshot`.",
            fixture.display()
        );
    }

    assert_eq!(
        actual, expected,
        "snapshot drift; rerun with KB_UPDATE_SNAPSHOTS=1 to regenerate"
    );

    // Independent guard: scores must be non-increasing.
    for w in hits.windows(2) {
        assert!(
            w[0].score >= w[1].score,
            "scores not in descending order: {} then {}",
            w[0].score,
            w[1].score
        );
    }
}
