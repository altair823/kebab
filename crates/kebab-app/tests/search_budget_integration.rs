//! p9-fb-34: App::search_with_opts integration tests.

mod common;

use kebab_app::SearchResponse;
use kebab_core::{SearchFilters, SearchMode, SearchOpts, SearchQuery};

fn lex(text: &str, k: usize) -> SearchQuery {
    SearchQuery {
        text: text.to_string(),
        mode: SearchMode::Lexical,
        k,
        filters: SearchFilters::default(),
    }
}

#[test]
fn search_with_opts_no_budget_matches_search() {
    let env = common::TestEnv::new();
    common::ingest_md(&env, "a.md", "# T\n\napples are red\n");
    let app = env.app();

    let baseline = app.search(lex("apples", 5)).unwrap();
    let resp: SearchResponse = app
        .search_with_opts(lex("apples", 5), SearchOpts::default())
        .unwrap();

    assert_eq!(resp.hits.len(), baseline.len());
    assert!(!resp.truncated);
    assert!(resp.next_cursor.is_none(), "k=5 against 1 doc → no next page");
}

#[test]
fn budget_truncates_snippets_when_below_threshold() {
    let env = common::TestEnv::new();
    let body: String = "rust ownership is a memory model. ".repeat(10);
    common::ingest_md(&env, "a.md", &format!("# T\n\n{body}\n"));
    let app = env.app();

    let unrestricted = app.search(lex("rust", 5)).unwrap();
    let unrestricted_chars: usize = unrestricted.iter().map(|h| h.snippet.chars().count()).sum();

    let resp = app
        .search_with_opts(
            lex("rust", 5),
            SearchOpts {
                max_tokens: Some(50),
                snippet_chars: None,
                cursor: None,
            },
        )
        .unwrap();
    let limited_chars: usize = resp.hits.iter().map(|h| h.snippet.chars().count()).sum();

    assert!(resp.truncated, "small budget must trip truncation");
    assert!(limited_chars < unrestricted_chars, "snippet should shrink");
    assert!(!resp.hits.is_empty(), "always retain ≥1 hit");
}

#[test]
fn cursor_paginates_to_next_page() {
    let env = common::TestEnv::new();
    for i in 0..6 {
        common::ingest_md(&env, &format!("d{i}.md"), &format!("# T{i}\n\nrust topic {i}\n"));
    }
    let app = env.app();

    let page1 = app
        .search_with_opts(lex("rust", 2), SearchOpts::default())
        .unwrap();
    assert_eq!(page1.hits.len(), 2);
    let cursor = page1.next_cursor.expect("more hits available");

    let page2 = app
        .search_with_opts(
            lex("rust", 2),
            SearchOpts {
                max_tokens: None,
                snippet_chars: None,
                cursor: Some(cursor),
            },
        )
        .unwrap();
    assert_eq!(page2.hits.len(), 2);
    let p1_ids: std::collections::HashSet<_> =
        page1.hits.iter().map(|h| h.chunk_id.0.clone()).collect();
    let p2_ids: std::collections::HashSet<_> =
        page2.hits.iter().map(|h| h.chunk_id.0.clone()).collect();
    assert!(p1_ids.is_disjoint(&p2_ids), "page 2 must not repeat page 1 hits");
}

#[test]
fn cursor_rejected_after_corpus_revision_bump() {
    let env = common::TestEnv::new();
    common::ingest_md(&env, "a.md", "# T\n\napples\n");
    let app = env.app();

    let page1 = app
        .search_with_opts(lex("apples", 1), SearchOpts::default())
        .unwrap();
    // p9-fb-34 round-1 review: replaced silent `if let Some(c) = ...`
    // with `.expect(...)` so a fixture regression that breaks the
    // cursor-emission contract fails loudly instead of passing vacuously.
    let c = page1
        .next_cursor
        .expect("k=1 page must emit next_cursor — fixture too small if this fails");

    common::ingest_md(&env, "b.md", "# B\n\nbananas\n");
    let app2 = env.app();

    let result = app2.search_with_opts(
        lex("apples", 1),
        SearchOpts {
            max_tokens: None,
            snippet_chars: None,
            cursor: Some(c),
        },
    );
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("stale_cursor"),
        "must surface stale_cursor: {err}"
    );
}

#[test]
fn max_tokens_zero_returns_one_hit_truncated() {
    // p9-fb-34 round-1 review: pin the documented "≥1 hit floor"
    // contract — even with `max_tokens=0` (an absurdly tight budget)
    // the budget loop must keep one hit and flip `truncated: true`.
    // Fixture intentionally seeds multiple matches so step 2 of the
    // budget loop (pop hits to 1) actually fires.
    let env = common::TestEnv::new();
    for i in 0..3 {
        common::ingest_md(
            &env,
            &format!("d{i}.md"),
            &format!("# T{i}\n\napples are red {i}\n"),
        );
    }
    let app = env.app();

    let resp = app
        .search_with_opts(
            lex("apples", 5),
            SearchOpts {
                max_tokens: Some(0),
                snippet_chars: None,
                cursor: None,
            },
        )
        .unwrap();
    assert_eq!(resp.hits.len(), 1, "max_tokens=0 collapses to 1-hit floor");
    assert!(resp.truncated);
}
