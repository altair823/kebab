//! PR1: image/pdf/code embedding now flows through `embed_with_cache`.
//! Proves a code asset's second (forced) ingest is a derivation-cache HIT
//! with byte-identical vectors. AVX-gated (`#[ignore]`) like all embedding
//! tests — run with `cargo test -p kebab-app --test embed_cache_reingest -- --ignored`.

mod common;

use common::TestEnv;

/// Read the row count of the "embedding" derivation-cache namespace from the
/// test KB's SQLite file.
fn embedding_cache_rows(data_dir: &std::path::Path) -> i64 {
    let db = data_dir.join("kebab.sqlite");
    let conn = rusqlite::Connection::open(db).expect("open kebab.sqlite");
    conn.query_row(
        "SELECT COUNT(*) FROM derivation_cache WHERE kind = 'embedding'",
        [],
        |r| r.get(0),
    )
    .expect("count embedding cache rows")
}

#[test]
#[ignore = "requires AVX + fastembed model download"]
fn code_reingest_is_embedding_cache_hit_and_byte_identical() {
    require_avx_or_panic();

    let env = TestEnv::with_embeddings();
    let data_dir = std::path::PathBuf::from(&env.config.storage.data_dir);

    // Isolate the code handler: the fixture workspace ships markdown that the
    // (already-cached) markdown handler would embed, which would populate the
    // SAME global `derivation_cache` and mask whether the *code* handler is
    // wired through `embed_with_cache`. Clear the workspace so `sample.rs` is
    // the only ingested asset — then every embedding cache row is the code
    // handler's, and the assertions reflect the code path exclusively.
    clear_workspace(&env.workspace_root);

    // Write a small Rust source file into the workspace so the code handler runs.
    let src = env.workspace_root.join("sample.rs");
    std::fs::write(
        &src,
        "pub fn add(a: i32, b: i32) -> i32 { a + b }\n\
         pub fn sub(a: i32, b: i32) -> i32 { a - b }\n",
    )
    .unwrap();

    // First ingest: cold cache → embeddings computed + cached.
    let opts1 = kebab_app::IngestOpts::default();
    kebab_app::ingest_with_config(env.config.clone(), env.scope(), opts1).expect("first ingest");
    let rows_after_first = embedding_cache_rows(&data_dir);
    assert!(
        rows_after_first > 0,
        "first ingest must populate the embedding derivation cache (got {rows_after_first})"
    );

    // Capture vector-search results after the first ingest. Vector mode
    // exercises the embeddings; identical vectors ⇒ identical scores.
    let hits_first = search_hits(&env.config);
    assert!(
        !hits_first.is_empty(),
        "first ingest produced no searchable vectors"
    );

    // Second ingest with force_reingest: same source bytes + same versions →
    // every chunk text is a cache HIT, so no new cache rows, identical vectors.
    let opts2 = kebab_app::IngestOpts {
        force_reingest: true,
        ..Default::default()
    };
    kebab_app::ingest_with_config(env.config.clone(), env.scope(), opts2).expect("re-ingest");
    let rows_after_second = embedding_cache_rows(&data_dir);
    assert_eq!(
        rows_after_first, rows_after_second,
        "re-ingest must be a pure cache hit — no new embedding cache rows"
    );

    let hits_second = search_hits(&env.config);
    assert_eq!(
        hits_first, hits_second,
        "re-ingest must yield byte-identical vector-search results (cache hit ⇒ same vectors ⇒ same scores)"
    );
}

/// Remove every entry under the workspace root so the only ingested asset is
/// the one this test writes afterward. `TestEnv::with_embeddings()` copies the
/// fixture markdown tree in; leaving it would let the (separately cached)
/// markdown handler populate the shared embedding cache and mask the code
/// handler's wiring.
fn clear_workspace(root: &std::path::Path) {
    for entry in std::fs::read_dir(root).expect("read workspace root") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            std::fs::remove_dir_all(&path).expect("remove workspace subdir");
        } else {
            std::fs::remove_file(&path).expect("remove workspace file");
        }
    }
}

/// Panic if the host CPU lacks AVX. Mirrors the helper in
/// `tests/search_vector.rs` so a `--ignored` invocation on a non-AVX host
/// fails loudly with a clear message instead of crashing inside Lance's
/// SIMD kernel.
fn require_avx_or_panic() {
    #[cfg(target_arch = "x86_64")]
    {
        assert!(
            std::is_x86_feature_detected!("avx"),
            "kebab-app vector integration test requires AVX-capable hardware; \
             host CPU lacks AVX. Run on an AVX-capable machine."
        );
    }
}

/// Vector-mode search results as `(chunk_id, score_bits)`, sorted, for a
/// byte-exact cross-ingest comparison. `score.to_bits()` makes the f32
/// comparison exact; identical embedding vectors produce identical scores.
/// SearchQuery construction mirrors `tests/search_vector.rs`.
///
/// `SearchHit` has no top-level `score` field — the wire-level "score" for a
/// hit is `retrieval.fusion_score` (in vector-only mode this *is* the cosine
/// score; `score_kind = Cosine`). We compare its `to_bits()` for an exact f32
/// match across ingests.
fn search_hits(config: &kebab_config::Config) -> Vec<(String, u32)> {
    let q = kebab_core::SearchQuery {
        text: "add".to_string(),
        mode: kebab_core::SearchMode::Vector,
        k: 10,
        filters: kebab_core::SearchFilters::default(),
    };
    let mut hits = kebab_app::search_with_config(config.clone(), q)
        .expect("vector search")
        .into_iter()
        .map(|h| (h.chunk_id.0, h.retrieval.fusion_score.to_bits()))
        .collect::<Vec<_>>();
    hits.sort();
    hits
}
