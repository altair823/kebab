//! p9-fb-32: `App::search` end-to-end staleness wiring.
//!
//! `compute_stale` itself is unit-tested in `kebab_app::staleness`; this
//! file proves the post-process actually fires through the full
//! retriever stack and that the cache-hit re-stamp respects the
//! configured threshold.
//!
//! All three tests run lexical-only (no AVX, no fastembed download).

mod common;

use common::TestEnv;

fn lexical_query_owner() -> kebab_core::SearchQuery {
    common::lexical_query("ownership")
}

/// Fresh ingest at default 30-day threshold → no hit can be stale.
/// `documents.updated_at` is stamped at ingest time (now), so the
/// distance to `now_utc()` is sub-second.
#[test]
fn fresh_doc_is_not_stale_with_default_threshold() {
    let env = TestEnv::lexical_only();
    kebab_app::ingest_with_config(env.config.clone(), env.scope(), true).unwrap();

    let app = kebab_app::App::open_with_config(env.config.clone()).unwrap();
    let hits = app.search(lexical_query_owner()).unwrap();
    assert!(!hits.is_empty(), "expected ≥1 hit for 'ownership'");
    assert!(
        hits.iter().all(|h| !h.stale),
        "freshly-ingested doc must not be stale at default 30d threshold: {:?}",
        hits.iter()
            .map(|h| (h.doc_path.0.clone(), h.stale))
            .collect::<Vec<_>>()
    );
}

/// `stale_threshold_days = 0` disables the feature even for very old
/// `documents.updated_at`. Backdate the row to a year ago, expect
/// `stale: false` on every hit.
#[test]
fn threshold_zero_disables_staleness() {
    let mut env = TestEnv::lexical_only();
    env.config.search.stale_threshold_days = 0;

    kebab_app::ingest_with_config(env.config.clone(), env.scope(), true).unwrap();
    common::backdate_document_updated_at(&env, "intro.md", 365);

    let app = kebab_app::App::open_with_config(env.config.clone()).unwrap();
    let hits = app.search(lexical_query_owner()).unwrap();
    assert!(!hits.is_empty(), "expected ≥1 hit");
    assert!(
        hits.iter().all(|h| !h.stale),
        "threshold=0 disables staleness even for year-old docs: {:?}",
        hits.iter()
            .map(|h| (h.doc_path.0.clone(), h.stale))
            .collect::<Vec<_>>()
    );
}

/// At a 30-day threshold, a 60-day-old `documents.updated_at` must
/// surface as stale on the matching hit. (Other hits — fresh fixtures
/// not backdated — stay fresh, so we use `any` not `all`.)
#[test]
fn old_doc_marked_stale() {
    let mut env = TestEnv::lexical_only();
    env.config.search.stale_threshold_days = 30;

    kebab_app::ingest_with_config(env.config.clone(), env.scope(), true).unwrap();
    common::backdate_document_updated_at(&env, "intro.md", 60);

    let app = kebab_app::App::open_with_config(env.config.clone()).unwrap();
    let hits = app.search(lexical_query_owner()).unwrap();
    assert!(!hits.is_empty(), "expected ≥1 hit");
    let intro_hits: Vec<&kebab_core::SearchHit> = hits
        .iter()
        .filter(|h| h.doc_path.0.ends_with("intro.md"))
        .collect();
    assert!(
        !intro_hits.is_empty(),
        "expected ≥1 hit on intro.md (the backdated doc)"
    );
    assert!(
        intro_hits.iter().all(|h| h.stale),
        "60-day-old intro.md must be stale at 30d threshold: {:?}",
        intro_hits
            .iter()
            .map(|h| (h.doc_path.0.clone(), h.stale))
            .collect::<Vec<_>>()
    );
}
