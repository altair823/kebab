//! Hybrid integration tests — touch a real `LanceVectorStore` +
//! `SqliteStore` + `MockEmbedder`. These tests are `#[ignore]`-d and
//! AVX-gated; see `tests/common/mod.rs` for the policy rationale.
//!
//! Mock-retriever unit tests live alongside the implementation in
//! `crates/kb-search/src/hybrid.rs` (no Lance, no AVX needed) — the
//! tests here exercise the full plumbing with the real Lance store.

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use common::{
    HybridEnv, id32, require_avx_or_panic, TEST_LEX_INDEX_VERSION, TEST_VEC_INDEX_VERSION,
};
use kb_core::{
    Retriever, SearchFilters, SearchHit, SearchMode, SearchQuery,
};
use kb_search::{FusionPolicy, HybridRetriever};
use serde_json::json;

fn build_hybrid(env: &HybridEnv) -> HybridRetriever {
    let lex: Arc<dyn Retriever> = Arc::new(env.lexical_retriever());
    let vec: Arc<dyn Retriever> = Arc::new(env.vector_retriever());
    HybridRetriever::with_policy(lex, vec, FusionPolicy::Rrf { k_rrf: 60 }, 5)
}

/// Seed a tiny corpus that lets us prove hybrid recall ≥ each side
/// independently. Two chunks are lexical-only matches ("rust cargo");
/// two chunks are vector-only matches (their text doesn't contain
/// the query token but their embedding still scores nearby because
/// MockEmbedder's hash distributes over all chunks).
fn seed_disjoint_corpus(env: &HybridEnv) -> Vec<String> {
    // The lexical side will only match chunks that contain the query
    // tokens. The vector side will rank ALL chunks by embedding
    // similarity to the query — even ones whose text doesn't share
    // a token with the query.
    let chunks = [
        // (chunk_id, doc_id, path, text, headings)
        (id32("c1"), id32("d1"), "notes/rust1.md", "rust cargo macros", &["A"][..]),
        (id32("c2"), id32("d2"), "notes/rust2.md", "rust traits and lifetimes", &["B"][..]),
        (id32("c3"), id32("d3"), "notes/python.md", "python dataclasses tutorial", &["C"][..]),
        (id32("c4"), id32("d4"), "notes/go.md", "go interfaces and channels", &["D"][..]),
    ];
    let mut ids = Vec::new();
    for (cid, did, path, text, headings) in &chunks {
        env.seed_chunk(cid, did, path, text, headings, &[]);
        env.embed_and_upsert(cid, did, text, headings);
        ids.push(cid.clone());
    }
    ids
}

#[test]
#[ignore = "requires AVX-capable hardware (LanceDB)"]
fn hybrid_recall_disjoint_returns_union() {
    require_avx_or_panic();
    let env = HybridEnv::new();
    let _ids = seed_disjoint_corpus(&env);
    let h = build_hybrid(&env);

    let q = SearchQuery {
        text: "rust".to_string(),
        mode: SearchMode::Hybrid,
        k: 4,
        filters: SearchFilters::default(),
    };
    let hits = h.search(&q).unwrap();

    // The vector side will return up to 4 candidates regardless of
    // text overlap; the lexical side will return only the rust* ones.
    // Together the union must cover at least the lexical hits AND
    // include at least one non-lexical chunk if vector found one.
    assert!(!hits.is_empty(), "hybrid must return at least one hit");
    // Every hit's RetrievalDetail.method must be Hybrid.
    for h in &hits {
        assert_eq!(h.retrieval.method, SearchMode::Hybrid);
        // At least one of lex/vec_score must be Some.
        assert!(
            h.retrieval.lexical_score.is_some() || h.retrieval.vector_score.is_some(),
            "hybrid hit must carry at least one mode's score"
        );
    }
    // index_version composite token.
    let iv = h.index_version();
    assert!(iv.0.starts_with("hybrid:"));
    assert!(iv.0.contains(TEST_LEX_INDEX_VERSION));
    assert!(iv.0.contains(TEST_VEC_INDEX_VERSION));

    // Lexical-only chunks (c1, c2) MUST appear: they're the only ones
    // matching the FTS5 query, and the vector side over-fetches enough
    // to include them too.
    let ids: Vec<&str> = hits.iter().map(|h| h.chunk_id.0.as_str()).collect();
    assert!(ids.contains(&id32("c1").as_str()));
    assert!(ids.contains(&id32("c2").as_str()));
}

#[test]
#[ignore = "requires AVX-capable hardware (LanceDB)"]
fn hybrid_determinism_same_query_twice() {
    require_avx_or_panic();
    let env = HybridEnv::new();
    let _ = seed_disjoint_corpus(&env);
    let h = build_hybrid(&env);

    let q = SearchQuery {
        text: "rust".to_string(),
        mode: SearchMode::Hybrid,
        k: 4,
        filters: SearchFilters::default(),
    };
    let a = h.search(&q).unwrap();
    let b = h.search(&q).unwrap();
    assert_eq!(a, b, "identical query must yield byte-identical Vec<SearchHit>");
}

#[test]
#[ignore = "requires AVX-capable hardware (LanceDB)"]
fn hybrid_snapshot_run_1() {
    require_avx_or_panic();
    let env = HybridEnv::new();
    let _ = seed_disjoint_corpus(&env);
    let h = build_hybrid(&env);

    let q = SearchQuery {
        text: "rust".to_string(),
        mode: SearchMode::Hybrid,
        k: 4,
        filters: SearchFilters::default(),
    };
    let hits = h.search(&q).unwrap();

    // Snapshot pins the structural shape:
    //   - chunk_id ordering
    //   - which side contributed (lexical_rank / vector_rank
    //     populated as Some/None)
    //   - that fusion_score is non-increasing
    //   - method = Hybrid for every hit
    let actual = json!(
        hits.iter().map(|h: &SearchHit| json!({
            "chunk_id": h.chunk_id.0,
            "rank": h.rank,
            "method": h.retrieval.method,
            "lexical_rank": h.retrieval.lexical_rank,
            "vector_rank": h.retrieval.vector_rank,
            "lex_some": h.retrieval.lexical_score.is_some(),
            "vec_some": h.retrieval.vector_score.is_some(),
            "fusion_score_positive": h.retrieval.fusion_score > 0.0,
        })).collect::<Vec<_>>()
    );

    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("search")
        .join("hybrid")
        .join("run-1.json");

    if std::env::var_os("KB_UPDATE_SNAPSHOTS").is_some() {
        std::fs::create_dir_all(fixture.parent().unwrap()).unwrap();
        std::fs::write(&fixture, serde_json::to_string_pretty(&actual).unwrap()).unwrap();
        eprintln!("[snapshot] regenerated {}", fixture.display());
        // Fail loudly so that accidentally setting KB_UPDATE_SNAPSHOTS
        // in CI surfaces as a test failure rather than a silent
        // overwrite + green run. Same fail-loud-instead-of-silent-pass
        // philosophy as P3-2's `SNAPSHOT_HASH_BASELINE = 0` and P3-3's
        // placeholder fixture guards.
        panic!(
            "[snapshot] regenerated {}, re-run without KB_UPDATE_SNAPSHOTS to verify pin",
            fixture.display()
        );
    }

    let expected: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&fixture).unwrap_or_else(|_| {
            panic!(
                "missing snapshot fixture at {}; run with \
                 KB_UPDATE_SNAPSHOTS=1 to create",
                fixture.display()
            )
        }))
        .unwrap();

    // Refuse to silently "pass" against the committed placeholder. The
    // placeholder JSON carries a `_comment` field with regeneration
    // instructions; production fixtures (a captured list) do not.
    if expected.get("_comment").is_some() {
        panic!(
            "snapshot fixture is a placeholder — regenerate on AVX hardware then commit. \
             Path: {}. To regenerate: \
             `KB_UPDATE_SNAPSHOTS=1 cargo test -p kb-search -- --ignored hybrid_snapshot`.",
            fixture.display()
        );
    }

    assert_eq!(
        actual, expected,
        "hybrid snapshot drift; rerun with KB_UPDATE_SNAPSHOTS=1 to regenerate"
    );

    // Independent guard: fusion scores must be non-increasing across
    // the result list (rrf is rank-biased, so this is the
    // semantically-correct ordering invariant).
    for w in hits.windows(2) {
        assert!(
            w[0].retrieval.fusion_score >= w[1].retrieval.fusion_score,
            "fusion scores not in descending order: {} then {}",
            w[0].retrieval.fusion_score,
            w[1].retrieval.fusion_score
        );
    }
}
