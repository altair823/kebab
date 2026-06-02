//! Lexical search integration tests. The vector / hybrid lanes are
//! AVX-gated and live in `search_vector.rs` (`#[ignore]`).

mod common;

use common::TestEnv;

#[test]
fn lexical_search_returns_hits_after_ingest() {
    let env = TestEnv::lexical_only();
    kebab_app::ingest_with_config(env.config.clone(), env.scope(), true).unwrap();

    // "Ownership" appears as a heading + paragraph in intro.md and
    // matches FTS5 default tokenizer easily.
    let hits =
        kebab_app::search_with_config(env.config.clone(), common::lexical_query("ownership"))
            .unwrap();
    assert!(!hits.is_empty(), "expected ≥1 hit for 'ownership'");

    for h in &hits {
        // Lexical retriever sets embedding_model=None per spec.
        assert!(
            h.embedding_model.is_none(),
            "lexical-mode hit must have None embedding_model: {h:?}"
        );
        assert_eq!(
            h.retrieval.method,
            kebab_core::SearchMode::Lexical,
            "method label should be Lexical"
        );
    }
}

#[test]
fn lexical_search_empty_query_returns_empty() {
    let env = TestEnv::lexical_only();
    kebab_app::ingest_with_config(env.config.clone(), env.scope(), true).unwrap();
    let hits =
        kebab_app::search_with_config(env.config.clone(), common::lexical_query("   ")).unwrap();
    assert!(hits.is_empty(), "blank query must short-circuit empty");
}

/// p9-fb-19 — `App::search` returns the same hit list for a repeated
/// query (cache hit doesn't corrupt the result). Both calls share an
/// `App` instance so the cache is in scope.
#[test]
fn cached_search_returns_same_hits_on_repeat() {
    let env = TestEnv::lexical_only();
    kebab_app::ingest_with_config(env.config.clone(), env.scope(), true).unwrap();
    let app = kebab_app::App::open_with_config(env.config.clone()).unwrap();
    let first = app.search(common::lexical_query("ownership")).unwrap();
    assert!(!first.is_empty(), "first call must return ≥1 hit");
    let second = app.search(common::lexical_query("ownership")).unwrap();
    assert_eq!(
        first.len(),
        second.len(),
        "cached call must yield identical hit count"
    );
    for (a, b) in first.iter().zip(second.iter()) {
        assert_eq!(a.chunk_id, b.chunk_id, "chunk_ids must align");
        assert_eq!(a.rank, b.rank, "ranks must align");
    }
}

/// p9-fb-19 — query normalization (NFKC + trim + lowercase) collapses
/// `"Ownership"` / `"OWNERSHIP"` / `" ownership "` into one cache
/// entry. Verified by ensuring all three forms return the same hits.
#[test]
fn cache_key_normalization_treats_case_and_whitespace_as_equivalent() {
    let env = TestEnv::lexical_only();
    kebab_app::ingest_with_config(env.config.clone(), env.scope(), true).unwrap();
    let app = kebab_app::App::open_with_config(env.config.clone()).unwrap();
    let plain = app.search(common::lexical_query("ownership")).unwrap();
    let upper = app.search(common::lexical_query("OWNERSHIP")).unwrap();
    let padded = app.search(common::lexical_query("  Ownership  ")).unwrap();
    assert_eq!(plain.len(), upper.len());
    assert_eq!(plain.len(), padded.len());
    // chunk_ids are deterministic — same query class, same set.
    let plain_ids: Vec<_> = plain.iter().map(|h| h.chunk_id.0.clone()).collect();
    let upper_ids: Vec<_> = upper.iter().map(|h| h.chunk_id.0.clone()).collect();
    assert_eq!(plain_ids, upper_ids);
}

/// p9-fb-19 — `--no-cache` (`search_uncached_with_config`) bypasses
/// the cache. Result correctness is identical to `search_with_config`.
#[test]
fn search_uncached_returns_same_hits_as_cached() {
    let env = TestEnv::lexical_only();
    kebab_app::ingest_with_config(env.config.clone(), env.scope(), true).unwrap();
    let cached =
        kebab_app::search_with_config(env.config.clone(), common::lexical_query("ownership"))
            .unwrap();
    let uncached = kebab_app::search_uncached_with_config(
        env.config.clone(),
        common::lexical_query("ownership"),
    )
    .unwrap();
    assert_eq!(cached.len(), uncached.len());
    for (a, b) in cached.iter().zip(uncached.iter()) {
        assert_eq!(a.chunk_id, b.chunk_id);
    }
}

/// p9-fb-19 — first ingest with commits bumps `corpus_revision` from
/// 0 to ≥1. Verified by reading the persisted kv via a fresh
/// SqliteStore handle (the field on `App` is `pub(crate)`).
#[test]
fn first_ingest_bumps_corpus_revision() {
    let env = TestEnv::lexical_only();
    let store_before = kebab_store_sqlite::SqliteStore::open(&env.config).unwrap();
    store_before.run_migrations().unwrap();
    // V004 seeds 0; V009 + V010 + V011 migrations each bump by 1 to
    // invalidate stale LRU caches (spec §5.2). Baseline before ingest = 3.
    // (V012 derivation_cache + V013 drop-chunk-aliases are structural/additive
    // — neither bumps corpus_revision.)
    let baseline = store_before.corpus_revision();
    assert_eq!(baseline, 3, "fresh store post-V011 baseline = 3");

    let report = kebab_app::ingest_with_config(env.config.clone(), env.scope(), true).unwrap();
    assert!(
        report.new + report.updated > 0,
        "first ingest must commit ≥1 doc"
    );

    let store_after = kebab_store_sqlite::SqliteStore::open(&env.config).unwrap();
    assert!(
        store_after.corpus_revision() > baseline,
        "ingest commit must bump corpus_revision past baseline {baseline} (got {})",
        store_after.corpus_revision(),
    );
}

#[test]
fn vector_mode_with_provider_none_errors_clearly() {
    let env = TestEnv::lexical_only();
    kebab_app::ingest_with_config(env.config.clone(), env.scope(), true).unwrap();

    let q = kebab_core::SearchQuery {
        text: "ownership".to_string(),
        mode: kebab_core::SearchMode::Vector,
        k: 10,
        filters: kebab_core::SearchFilters::default(),
    };
    let err = kebab_app::search_with_config(env.config.clone(), q).unwrap_err();
    let msg = format!("{err:#}");
    assert!(
        msg.contains("embeddings disabled") || msg.contains("disabled"),
        "error must mention embeddings disabled: {msg}"
    );
}
