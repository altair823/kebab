//! p9-fb-10: smoke pin that a Korean query reaches FTS5 and returns
//! the matching Hangul document.  NFC normalization happens upstream
//! in `kebab-normalize`; this test only exercises the end-to-end
//! facade — ingest a Korean .md → lexical search → at least one hit.

mod common;

use common::TestEnv;

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
    let hits = kebab_app::search_with_config(env.config.clone(), common::lexical_query("러스트"))
        .expect("search must succeed");

    assert!(
        !hits.is_empty(),
        "expected at least one hit for Korean lexical query '러스트'"
    );

    // At least one hit must reference our Korean document.
    // "러스트-비동기" is the exact filename stem — a single combined
    // check is unambiguous and avoids false positives from other docs.
    let any_korean = hits.iter().any(|h| h.doc_path.0.contains("러스트-비동기"));
    assert!(
        any_korean,
        "expected at least one hit on the Korean fixture doc, got: {:?}",
        hits.iter().map(|h| &h.doc_path.0).collect::<Vec<_>>()
    );
}

/// A4 Step 1c — multi-token Korean query (`해시 충돌`) must hit when
/// the lexical builder routes it through a whole-phrase MATCH candidate.
///
/// Expected: FAIL until A5 (`build_match_string` redesign) lands — the
/// current builder emits `"해시" "충돌"` AND, but FTS5 trigram tokenizer
/// has no 2-char terms so each side is 0-hit. A5 introduces a whole-
/// phrase candidate (`"해시 충돌"`) OR'd with the token AND, restoring
/// hits for the dominant Korean usage pattern.
#[test]
fn lexical_multi_token_korean_query_hits() {
    let env = TestEnv::lexical_only();

    // Copy the synthetic Korean fixture (introduced in A4 Step 0) into
    // the test workspace. The fixture contains the exact phrase
    // "해시 충돌" multiple times.
    let dest = env.workspace_root.join("hash-table.md");
    let src = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
        .join("search")
        .join("korean")
        .join("hash-table.md");
    std::fs::copy(&src, &dest).expect("copy korean fixture");

    kebab_app::ingest_with_config(env.config.clone(), env.scope(), true)
        .expect("ingest must succeed");

    let hits =
        kebab_app::search_with_config(env.config.clone(), common::lexical_query("해시 충돌"))
            .expect("search must succeed");

    assert!(
        !hits.is_empty(),
        "multi-token Korean query '해시 충돌' must hit the hash-table fixture; got {:?}",
        hits.iter().map(|h| &h.doc_path.0).collect::<Vec<_>>()
    );
    let any_hash_table = hits.iter().any(|h| h.doc_path.0.contains("hash-table"));
    assert!(
        any_hash_table,
        "expected at least one hit on the hash-table fixture, got: {:?}",
        hits.iter().map(|h| &h.doc_path.0).collect::<Vec<_>>()
    );
}

/// A4 Step 1c — mixed Korean+English multi-token query (`Rust 충돌은`).
/// Both tokens are ≥3 chars, so the redesigned builder (A5) emits
/// `("Rust 충돌은") OR ("Rust" AND "충돌은")`. With trigram tokenizer
/// each side has substring coverage in the document, so the AND branch
/// alone is enough. Expected: FAIL pre-A5, PASS post-A5.
#[test]
fn lexical_mixed_korean_english_multi_token_query_hits() {
    let env = TestEnv::lexical_only();
    let doc_path = env.workspace_root.join("rust-hash.md");
    std::fs::write(
        &doc_path,
        "# Rust 해시 테이블\n\nRust 의 std::collections::HashMap 에서 \
         해시 충돌은 SipHash 로 완화한다.\n",
    )
    .expect("write rust-hash fixture");

    kebab_app::ingest_with_config(env.config.clone(), env.scope(), true)
        .expect("ingest must succeed");

    let hits =
        kebab_app::search_with_config(env.config.clone(), common::lexical_query("Rust 충돌은"))
            .expect("search must succeed");

    assert!(
        !hits.is_empty(),
        "mixed Korean+English multi-token query 'Rust 충돌은' must hit the rust-hash fixture; got {:?}",
        hits.iter().map(|h| &h.doc_path.0).collect::<Vec<_>>()
    );
    let any_rust_hash = hits.iter().any(|h| h.doc_path.0.contains("rust-hash"));
    assert!(
        any_rust_hash,
        "expected at least one hit on the rust-hash fixture, got: {:?}",
        hits.iter().map(|h| &h.doc_path.0).collect::<Vec<_>>()
    );
}

// ── S7 V009 morphological tokenizer end-to-end tests ─────────────────

/// S7 — V009 morphological tokenizer: 한국어 2자 query 가 end-to-end
/// lexical 경로에서 hit. lindera ko-dic 이 '한국어를' → '한국어' 형태소로
/// 분해, '서울은' → '서울' 로 분해하여 tokenized_korean_text column 에
/// 기록 → FTS5 매칭.
#[test]
fn korean_morphological_2char_query_lexical_mode() {
    let env = TestEnv::lexical_only();
    let doc_path = env.workspace_root.join("korean-wiki.md");
    std::fs::write(
        &doc_path,
        "# 한국어 위키\n\n한국어를 공부합니다.\n서울은 한국의 수도입니다.\n",
    )
    .expect("write korean-wiki fixture");

    kebab_app::ingest_with_config(env.config.clone(), env.scope(), true)
        .expect("ingest must succeed");

    let hits = kebab_app::search_with_config(env.config.clone(), common::lexical_query("한국"))
        .expect("search 한국");
    assert!(
        !hits.is_empty(),
        "'한국' 2-char Korean query must return at least one hit (V009 morphological); got {:?}",
        hits.iter().map(|h| &h.doc_path.0).collect::<Vec<_>>()
    );

    let hits = kebab_app::search_with_config(env.config.clone(), common::lexical_query("서울"))
        .expect("search 서울");
    assert!(
        !hits.is_empty(),
        "'서울' 2-char Korean query must return at least one hit; got {:?}",
        hits.iter().map(|h| &h.doc_path.0).collect::<Vec<_>>()
    );
}

/// S7 — V009 morphological tokenizer: 한-영 혼합 query lexical hit.
/// 'Rust' (English whole-token) + '최적화' (Korean morpheme) 각각 hit.
#[test]
fn korean_morphological_mixed_english_korean_query() {
    let env = TestEnv::lexical_only();
    let doc_path = env.workspace_root.join("rust-optimization.md");
    std::fs::write(
        &doc_path,
        "# Rust 최적화 노트\n\nRust 최적화는 zero-cost abstraction 을 강조한다.\n",
    )
    .expect("write rust-optimization fixture");

    kebab_app::ingest_with_config(env.config.clone(), env.scope(), true)
        .expect("ingest must succeed");

    let hits = kebab_app::search_with_config(env.config.clone(), common::lexical_query("Rust"))
        .expect("search Rust");
    assert!(
        !hits.is_empty(),
        "'Rust' English whole-token must hit; got {:?}",
        hits.iter().map(|h| &h.doc_path.0).collect::<Vec<_>>()
    );

    let hits = kebab_app::search_with_config(env.config.clone(), common::lexical_query("최적화"))
        .expect("search 최적화");
    assert!(
        !hits.is_empty(),
        "'최적화' Korean morpheme must hit; got {:?}",
        hits.iter().map(|h| &h.doc_path.0).collect::<Vec<_>>()
    );
}
