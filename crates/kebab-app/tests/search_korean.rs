//! p9-fb-10: smoke pin that a Korean query reaches FTS5 and returns
//! the matching Hangul document.  NFC normalization happens upstream
//! in `kebab-normalize`; this test only exercises the end-to-end
//! facade — ingest a Korean .md → lexical search → at least one hit.

mod common;

use common::TestEnv;

fn lexical_query(text: &str) -> kebab_core::SearchQuery {
    kebab_core::SearchQuery {
        text: text.to_string(),
        mode: kebab_core::SearchMode::Lexical,
        k: 10,
        filters: kebab_core::SearchFilters::default(),
    }
}

/// p9-fb-10 — A Korean token present in a Hangul document must survive
/// the ingest → FTS5 → search round-trip.  NFC normalization is wired
/// upstream in `kebab-normalize`; this test just verifies the facade
/// doesn't drop or corrupt CJK text along the way.
#[test]
fn korean_lexical_query_returns_korean_document() {
    let env = TestEnv::lexical_only();

    // Write a Korean Markdown document into the temp workspace.
    let doc_path = env.workspace_root.join("러스트-비동기.md");
    std::fs::write(
        &doc_path,
        "# 러스트 비동기 프로그래밍\n\n토큰: 러스트, 비동기, async, await\n",
    )
    .expect("write Korean fixture doc");

    // Ingest — lexical_only() disables fastembed so no AVX required.
    kebab_app::ingest_with_config(env.config.clone(), env.scope(), true)
        .expect("ingest must succeed");

    // Lexical search for "러스트" — must return the Korean document.
    let hits = kebab_app::search_with_config(env.config.clone(), lexical_query("러스트"))
        .expect("search must succeed");

    assert!(
        !hits.is_empty(),
        "expected at least one hit for Korean lexical query '러스트'"
    );

    // At least one hit must reference our Korean document.
    let any_korean = hits.iter().any(|h| {
        let p = &h.doc_path.0;
        p.contains("러스트") || p.contains("비동기")
    });
    assert!(
        any_korean,
        "expected a hit referencing the Korean document; got: {hits:#?}"
    );
}
