//! P2-2 integration tests for `LexicalRetriever`.
//!
//! Strategy: seed the SQLite store via raw inserts with `foreign_keys =
//! OFF` (mirroring the P2-1 FTS tests). This avoids dragging
//! `kb-parse-md` / `kb-normalize` / `kb-chunk` into kb-search's dev-deps,
//! which would violate the task's "Allowed deps" list.

use std::sync::Arc;

use kebab_config::Config;
use kebab_core::{
    DocumentId, IndexVersion, Lang, MediaType, Retriever, ScoreKind, SearchFilters, SearchHit,
    SearchMode, SearchQuery, TrustLevel,
};
use kebab_search::LexicalRetriever;
use kebab_store_sqlite::SqliteStore;
use rusqlite::Connection;
use tempfile::TempDir;
use time::OffsetDateTime;

// ── Test scaffolding ─────────────────────────────────────────────────────

struct Env {
    _temp: TempDir,
    store: Arc<SqliteStore>,
    db_path: std::path::PathBuf,
}

impl Env {
    fn new() -> Self {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut config = Config::defaults();
        config.storage.data_dir = temp.path().to_string_lossy().into_owned();
        let store = SqliteStore::open(&config).expect("open store");
        store.run_migrations().expect("run migrations");
        let db_path = temp.path().join("kebab.sqlite");
        Self {
            _temp: temp,
            store: Arc::new(store),
            db_path,
        }
    }

    /// Side-channel raw connection with FK enforcement off — same
    /// trick used by P2-1's FTS tests so we can seed `chunks` /
    /// `documents` directly without the full ingest graph.
    fn raw_conn(&self) -> Connection {
        let conn = Connection::open(&self.db_path).expect("open side conn");
        conn.pragma_update(None, "foreign_keys", "OFF").unwrap();
        conn
    }

    fn retriever(&self) -> LexicalRetriever {
        LexicalRetriever::new(
            Arc::clone(&self.store),
            IndexVersion("v1.0".to_string()),
        )
    }

    fn retriever_with_snippet_chars(&self, snippet_chars: usize) -> LexicalRetriever {
        LexicalRetriever::with_settings(
            Arc::clone(&self.store),
            IndexVersion("v1.0".to_string()),
            snippet_chars,
        )
    }
}

/// Minimal documents row. Many columns are NOT NULL and we don't care
/// about their exact values for retrieval tests, so we wedge in
/// reasonable defaults.
#[allow(clippy::too_many_arguments)]
fn insert_document(
    conn: &Connection,
    doc_id: &str,
    workspace_path: &str,
    title: &str,
    lang: &str,
    trust_level: &str,
    tags: &[&str],
) {
    // assets row first — documents.asset_id has a FK with ON DELETE
    // RESTRICT but FKs are OFF on this connection. Still we insert a
    // matching row so JOINs pick it up.
    let asset_id = format!("{:0>32}", &doc_id[..1.min(doc_id.len())]); // 32-hex-ish
    let asset_id = format!("{:0>32}", asset_id.chars().take(32).collect::<String>());
    conn.execute(
        "INSERT OR IGNORE INTO assets (
            asset_id, source_uri, workspace_path, media_type, byte_len,
            checksum, storage_kind, storage_path, discovered_at
        ) VALUES (?, 'file:///x', ?, '\"markdown\"', 0,
                  'd0', 'reference', '/x', '2024-01-01T00:00:00Z')",
        rusqlite::params![asset_id, workspace_path],
    )
    .expect("insert asset");

    conn.execute(
        "INSERT INTO documents (
            doc_id, asset_id, workspace_path, title, lang,
            source_type, trust_level, parser_version,
            doc_version, schema_version, metadata_json,
            provenance_json, created_at, updated_at
        ) VALUES (?, ?, ?, ?, ?, 'markdown', ?, 'pv1', 1, 1,
                  '{}', '{\"events\":[]}',
                  '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
        rusqlite::params![doc_id, asset_id, workspace_path, title, lang, trust_level],
    )
    .expect("insert document");

    for tag in tags {
        conn.execute(
            "INSERT INTO document_tags (doc_id, tag) VALUES (?, ?)",
            rusqlite::params![doc_id, tag],
        )
        .expect("insert tag");
    }
}

#[allow(clippy::too_many_arguments)]
fn insert_chunk(
    conn: &Connection,
    chunk_id: &str,
    doc_id: &str,
    text: &str,
    heading_path: &[&str],
    section_label: Option<&str>,
    source_spans_json: &str,
    chunker_version: &str,
) {
    let heading_json = serde_json::to_string(heading_path).unwrap();
    conn.execute(
        "INSERT INTO chunks (
            chunk_id, doc_id, text, heading_path_json, section_label,
            source_spans_json, token_estimate, chunker_version,
            policy_hash, block_ids_json, created_at
        ) VALUES (?, ?, ?, ?, ?, ?, 0, ?, 'h', '[]', '2024-01-01T00:00:00Z')",
        rusqlite::params![
            chunk_id,
            doc_id,
            text,
            heading_json,
            section_label,
            source_spans_json,
            chunker_version,
        ],
    )
    .expect("insert chunk");
}

/// Pad a short ID to the 32-hex shape kebab_core newtypes expect.
fn id32(prefix: &str) -> String {
    let mut s = prefix.to_string();
    while s.len() < 32 {
        s.push('0');
    }
    s.truncate(32);
    s
}

// ── Tests ────────────────────────────────────────────────────────────────

#[test]
fn lexical_empty_corpus_returns_empty_vec() {
    let env = Env::new();
    let r = env.retriever();
    let q = SearchQuery {
        text: "rust".to_string(),
        mode: SearchMode::Lexical,
        k: 10,
        filters: SearchFilters::default(),
    };
    let hits = r.search(&q).expect("search");
    assert!(hits.is_empty(), "empty corpus must yield empty Vec");
}

#[test]
fn lexical_empty_query_returns_empty_vec_without_db_hit() {
    // Even with rows in the DB, a blank query must short-circuit to [].
    let env = Env::new();
    let conn = env.raw_conn();
    insert_document(&conn, &id32("d"), "notes/a.md", "A", "en", "primary", &[]);
    insert_chunk(
        &conn,
        &id32("c1"),
        &id32("d"),
        "rust cargo macros",
        &["A"],
        None,
        r#"[{"kind":"line","start":1,"end":3}]"#,
        "v1",
    );
    drop(conn);

    let r = env.retriever();
    for empty in ["", "   ", "''"] {
        let q = SearchQuery {
            text: empty.to_string(),
            mode: SearchMode::Lexical,
            k: 5,
            filters: SearchFilters::default(),
        };
        let hits = r.search(&q).unwrap();
        assert!(hits.is_empty(), "query {empty:?} must yield empty Vec");
    }
}

#[test]
fn lexical_single_doc_match_returns_one_hit_with_citation_round_trip() {
    let env = Env::new();
    let conn = env.raw_conn();
    insert_document(&conn, &id32("d"), "notes/rust.md", "Rust Notes", "en", "primary", &[]);
    insert_chunk(
        &conn,
        &id32("c1"),
        &id32("d"),
        "Rust borrow checker enforces ownership.",
        &["Notes"],
        Some("Notes"),
        r#"[{"kind":"line","start":4,"end":4}]"#,
        "v1",
    );
    drop(conn);

    let r = env.retriever();
    let q = SearchQuery {
        text: "borrow".to_string(),
        mode: SearchMode::Lexical,
        k: 10,
        filters: SearchFilters::default(),
    };
    let hits = r.search(&q).expect("search");
    assert_eq!(hits.len(), 1);
    let h = &hits[0];
    assert_eq!(h.rank, 1);
    assert_eq!(h.doc_path.0, "notes/rust.md");
    assert_eq!(h.heading_path, vec!["Notes".to_string()]);
    assert_eq!(h.section_label.as_deref(), Some("Notes"));
    assert_eq!(h.retrieval.method, SearchMode::Lexical);
    assert_eq!(h.retrieval.lexical_rank, Some(1));
    assert!(h.retrieval.vector_score.is_none());

    // Citation round-trips through `to_uri`/`parse` (line variant).
    let uri = h.citation.to_uri();
    let parsed = kebab_core::Citation::parse(&uri).expect("parse uri");
    // Reparsed citation has section=None (URI fragment doesn't carry it),
    // so compare by `to_uri` equivalence rather than struct equality.
    assert_eq!(parsed.to_uri(), uri);
    // Sanity: this is a Line citation matching the seeded source span.
    assert_eq!(uri, "notes/rust.md#L4");
}

#[test]
fn lexical_snippet_length_capped_at_snippet_chars() {
    let env = Env::new();
    let conn = env.raw_conn();
    insert_document(
        &conn,
        &id32("d"),
        "notes/long.md",
        "Long",
        "en",
        "primary",
        &[],
    );
    // A text long enough that FTS5 might return a snippet > 80 chars
    // when given a high word budget. We instead set a tight cap below
    // and rely on `trim_snippet` as the backstop.
    let mut text = String::new();
    for _ in 0..50 {
        text.push_str("alpha beta gamma delta epsilon ");
    }
    insert_chunk(
        &conn,
        &id32("c1"),
        &id32("d"),
        &text,
        &["Long"],
        None,
        r#"[{"kind":"line","start":1,"end":1}]"#,
        "v1",
    );
    drop(conn);

    // Set snippet_chars to a known bound; the retriever clamps + trims
    // any snippet to fit.
    let r = env.retriever_with_snippet_chars(80);
    let hits = r
        .search(&SearchQuery {
            text: "alpha".to_string(),
            mode: SearchMode::Lexical,
            k: 1,
            filters: SearchFilters::default(),
        })
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert!(
        hits[0].snippet.chars().count() <= 80,
        "snippet must be ≤ snippet_chars; got {} chars: {:?}",
        hits[0].snippet.chars().count(),
        hits[0].snippet
    );
}

#[test]
fn lexical_filter_tags_any_excludes_untagged_docs() {
    let env = Env::new();
    let conn = env.raw_conn();
    insert_document(&conn, &id32("d1"), "notes/a.md", "A", "en", "primary", &["rust"]);
    insert_document(&conn, &id32("d2"), "notes/b.md", "B", "en", "primary", &["python"]);
    insert_chunk(
        &conn,
        &id32("c1"),
        &id32("d1"),
        "ownership and borrow checker",
        &["A"],
        None,
        r#"[{"kind":"line","start":1,"end":1}]"#,
        "v1",
    );
    insert_chunk(
        &conn,
        &id32("c2"),
        &id32("d2"),
        "borrow semantics in python",
        &["B"],
        None,
        r#"[{"kind":"line","start":1,"end":1}]"#,
        "v1",
    );
    drop(conn);

    let r = env.retriever();
    let q = SearchQuery {
        text: "borrow".to_string(),
        mode: SearchMode::Lexical,
        k: 10,
        filters: SearchFilters {
            tags_any: vec!["rust".to_string()],
            ..Default::default()
        },
    };
    let hits = r.search(&q).unwrap();
    assert_eq!(hits.len(), 1, "tags_any=[rust] must exclude python doc");
    assert_eq!(hits[0].doc_path.0, "notes/a.md");
}

#[test]
fn lexical_filter_lang_and_trust_min_compose() {
    let env = Env::new();
    let conn = env.raw_conn();
    insert_document(&conn, &id32("d1"), "ko/a.md", "A", "ko", "primary", &[]);
    insert_document(&conn, &id32("d2"), "en/b.md", "B", "en", "primary", &[]);
    insert_document(&conn, &id32("d3"), "en/c.md", "C", "en", "generated", &[]);
    for (cid, did, body) in [
        ("c1", "d1", "검색 키워드 alpha"),
        ("c2", "d2", "alpha bravo"),
        ("c3", "d3", "alpha gamma"),
    ] {
        insert_chunk(
            &conn,
            &id32(cid),
            &id32(did),
            body,
            &[],
            None,
            r#"[{"kind":"line","start":1,"end":1}]"#,
            "v1",
        );
    }
    drop(conn);

    let r = env.retriever();
    // lang=en + trust_min=secondary → only d2 (primary ≥ secondary).
    let hits = r
        .search(&SearchQuery {
            text: "alpha".to_string(),
            mode: SearchMode::Lexical,
            k: 10,
            filters: SearchFilters {
                lang: Some(Lang("en".to_string())),
                trust_min: Some(TrustLevel::Secondary),
                ..Default::default()
            },
        })
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].doc_path.0, "en/b.md");
}

#[test]
fn lexical_filter_path_glob_does_not_cross_slash() {
    let env = Env::new();
    let conn = env.raw_conn();
    insert_document(&conn, &id32("d1"), "notes/a.md", "A", "en", "primary", &[]);
    insert_document(&conn, &id32("d2"), "notes/sub/b.md", "B", "en", "primary", &[]);
    insert_chunk(
        &conn,
        &id32("c1"),
        &id32("d1"),
        "shared keyword",
        &[],
        None,
        r#"[{"kind":"line","start":1,"end":1}]"#,
        "v1",
    );
    insert_chunk(
        &conn,
        &id32("c2"),
        &id32("d2"),
        "shared keyword",
        &[],
        None,
        r#"[{"kind":"line","start":1,"end":1}]"#,
        "v1",
    );
    drop(conn);

    let r = env.retriever();
    let hits = r
        .search(&SearchQuery {
            text: "keyword".to_string(),
            mode: SearchMode::Lexical,
            k: 10,
            filters: SearchFilters {
                path_glob: Some("notes/*.md".to_string()),
                ..Default::default()
            },
        })
        .unwrap();
    let paths: Vec<&str> = hits.iter().map(|h| h.doc_path.0.as_str()).collect();
    assert_eq!(paths, vec!["notes/a.md"], "* must not match across `/`");
}

#[test]
fn lexical_citation_round_trip_against_first_source_span() {
    let env = Env::new();
    let conn = env.raw_conn();
    insert_document(&conn, &id32("d"), "notes/m.md", "M", "en", "primary", &[]);
    insert_chunk(
        &conn,
        &id32("c1"),
        &id32("d"),
        "echo bravo",
        &[],
        None,
        // Two spans; the citation uses the first.
        r#"[{"kind":"line","start":12,"end":34},{"kind":"line","start":60,"end":61}]"#,
        "v1",
    );
    drop(conn);

    let r = env.retriever();
    let hits = r
        .search(&SearchQuery {
            text: "bravo".to_string(),
            mode: SearchMode::Lexical,
            k: 1,
            filters: SearchFilters::default(),
        })
        .unwrap();
    assert_eq!(hits.len(), 1);
    let uri = hits[0].citation.to_uri();
    assert_eq!(uri, "notes/m.md#L12-L34");
    let parsed = kebab_core::Citation::parse(&uri).unwrap();
    assert_eq!(parsed.to_uri(), uri);
}

#[test]
fn lexical_top_score_within_unit_interval_three_chunks() {
    let env = Env::new();
    let conn = env.raw_conn();
    insert_document(&conn, &id32("d"), "notes/r.md", "R", "en", "primary", &[]);
    // Three chunks of varying relevance to the query 'alpha':
    //   c1: alpha alpha alpha (best)
    //   c2: alpha bravo
    //   c3: bravo charlie alpha (one occurrence)
    for (cid, body) in [
        ("c1", "alpha alpha alpha keyword"),
        ("c2", "alpha bravo charlie"),
        ("c3", "bravo charlie alpha"),
    ] {
        insert_chunk(
            &conn,
            &id32(cid),
            &id32("d"),
            body,
            &[],
            None,
            r#"[{"kind":"line","start":1,"end":1}]"#,
            "v1",
        );
    }
    drop(conn);

    let r = env.retriever();
    let hits = r
        .search(&SearchQuery {
            text: "alpha".to_string(),
            mode: SearchMode::Lexical,
            k: 10,
            filters: SearchFilters::default(),
        })
        .unwrap();
    assert!(!hits.is_empty(), "must surface at least one hit");
    let top = hits[0].retrieval.fusion_score;
    assert!(
        top > 0.0 && top <= 1.0,
        "top normalized score must be in (0, 1]; got {top}"
    );
    // All scores in [0, 1].
    for h in &hits {
        let s = h.retrieval.fusion_score;
        assert!((0.0..=1.0).contains(&s), "hit score {s} out of [0, 1]");
        // lexical_score and fusion_score equal in lexical-only mode.
        assert_eq!(h.retrieval.lexical_score, Some(s));
    }
    // bm25 should rank c1 (3 occurrences) above c2 / c3.
    assert!(hits[0].chunk_id.0.starts_with("c1"));
}

#[test]
fn lexical_determinism_same_query_twice() {
    let env = Env::new();
    let conn = env.raw_conn();
    insert_document(&conn, &id32("d"), "notes/r.md", "R", "en", "primary", &[]);
    for (cid, body) in [
        ("c1", "alpha alpha"),
        ("c2", "alpha bravo"),
        ("c3", "alpha charlie"),
        ("c4", "alpha delta"),
    ] {
        insert_chunk(
            &conn,
            &id32(cid),
            &id32("d"),
            body,
            &[],
            None,
            r#"[{"kind":"line","start":1,"end":1}]"#,
            "v1",
        );
    }
    drop(conn);

    let r = env.retriever();
    let q = SearchQuery {
        text: "alpha".to_string(),
        mode: SearchMode::Lexical,
        k: 10,
        filters: SearchFilters::default(),
    };
    let a = r.search(&q).unwrap();
    let b = r.search(&q).unwrap();
    assert_eq!(a, b, "same DB + same query must yield identical Vec<SearchHit>");
}

#[test]
fn lexical_determinism_chunk_id_tiebreaker_on_equal_bm25() {
    // Two chunks with byte-identical text + length → identical bm25 scores
    // for any `MATCH` against them. The retriever must fall back to
    // `chunk_id` ordering so the result is stable across runs.
    let env = Env::new();
    let conn = env.raw_conn();
    insert_document(&conn, &id32("d"), "notes/tie.md", "Tie", "en", "primary", &[]);
    let cid_a = id32("aaaa");
    let cid_b = id32("bbbb");
    assert!(cid_a < cid_b, "test premise: aaaa-id sorts before bbbb-id");
    for cid in [&cid_a, &cid_b] {
        insert_chunk(
            &conn,
            cid,
            &id32("d"),
            "alpha bravo charlie",
            &[],
            None,
            r#"[{"kind":"line","start":1,"end":1}]"#,
            "v1",
        );
    }
    drop(conn);

    let r = env.retriever();
    let q = SearchQuery {
        text: "alpha".to_string(),
        mode: SearchMode::Lexical,
        k: 10,
        filters: SearchFilters::default(),
    };
    let a = r.search(&q).unwrap();
    let b = r.search(&q).unwrap();
    assert_eq!(a.len(), 2, "both chunks should match");
    // bm25 must be equal for byte-identical chunks; the secondary sort
    // by chunk_id pins the order.
    assert!(
        (a[0].retrieval.fusion_score - a[1].retrieval.fusion_score).abs() < 1e-9,
        "byte-identical chunks must score equally; got {} vs {}",
        a[0].retrieval.fusion_score,
        a[1].retrieval.fusion_score
    );
    assert!(
        a[0].chunk_id.0 < a[1].chunk_id.0,
        "tiebreaker must order by chunk_id ascending; got {} then {}",
        a[0].chunk_id.0,
        a[1].chunk_id.0
    );
    assert_eq!(a, b, "tiebreaker order must be stable across runs");
}

#[test]
fn lexical_index_version_is_returned_unchanged() {
    let env = Env::new();
    let r = LexicalRetriever::new(
        Arc::clone(&env.store),
        IndexVersion("custom-label-1".to_string()),
    );
    assert_eq!(r.index_version().0, "custom-label-1");
}

#[test]
fn search_hit_carries_indexed_at_from_documents_updated_at() {
    // p9-fb-32: SearchHit.indexed_at must be populated from
    // documents.updated_at via the JOIN. We seed documents with
    // updated_at=now (RFC3339) and assert the parsed OffsetDateTime
    // round-trips within ±60s of wall-clock now.
    use time::OffsetDateTime;
    use time::format_description::well_known::Rfc3339;

    let env = Env::new();
    let conn = env.raw_conn();
    // The `insert_document` helper hard-codes updated_at='2024-01-01...';
    // override that here so the assertion against `now` is meaningful.
    let now = OffsetDateTime::now_utc();
    let now_rfc = now.format(&Rfc3339).expect("format now as rfc3339");
    let doc_id = id32("d");
    let asset_id = format!("{:0>32}", "d");
    conn.execute(
        "INSERT OR IGNORE INTO assets (
            asset_id, source_uri, workspace_path, media_type, byte_len,
            checksum, storage_kind, storage_path, discovered_at
        ) VALUES (?, 'file:///x', 'a.md', '\"markdown\"', 0,
                  'd0', 'reference', '/x', '2024-01-01T00:00:00Z')",
        rusqlite::params![asset_id],
    )
    .expect("insert asset");
    conn.execute(
        "INSERT INTO documents (
            doc_id, asset_id, workspace_path, title, lang,
            source_type, trust_level, parser_version,
            doc_version, schema_version, metadata_json,
            provenance_json, created_at, updated_at
        ) VALUES (?, ?, 'a.md', 'T', 'en', 'markdown', 'primary', 'pv1', 1, 1,
                  '{}', '{\"events\":[]}',
                  ?, ?)",
        rusqlite::params![doc_id, asset_id, now_rfc, now_rfc],
    )
    .expect("insert document");
    insert_chunk(
        &conn,
        &id32("c1"),
        &doc_id,
        "body about apples",
        &["T"],
        None,
        r#"[{"kind":"line","start":1,"end":1}]"#,
        "v1",
    );
    drop(conn);

    let r = env.retriever();
    let hits = r
        .search(&SearchQuery {
            text: "apples".to_string(),
            mode: SearchMode::Lexical,
            k: 5,
            filters: SearchFilters::default(),
        })
        .expect("search");
    let hit = hits.first().expect("at least one hit");
    let now2 = OffsetDateTime::now_utc();
    let delta = (now2 - hit.indexed_at).whole_seconds().abs();
    assert!(delta < 60, "indexed_at within ±60s of now, got {delta}s");
    // stale is a placeholder set by the retriever; the App layer overwrites.
    assert!(!hit.stale, "lexical retriever must default stale=false");
}

#[test]
fn lexical_retriever_hits_carry_bm25_score_kind() {
    // p9-fb-38: verify that every hit returned by LexicalRetriever
    // has score_kind == ScoreKind::Bm25. This establishes the
    // relationship: Lexical-only search → Bm25 score semantics.
    let env = Env::new();
    let conn = env.raw_conn();
    insert_document(&conn, &id32("d"), "notes/bm25.md", "Bm25", "en", "primary", &[]);
    for (cid, body) in [
        ("c1", "alpha bravo charlie"),
        ("c2", "alpha delta"),
        ("c3", "bravo echo"),
    ] {
        insert_chunk(
            &conn,
            &id32(cid),
            &id32("d"),
            body,
            &["Bm25"],
            None,
            r#"[{"kind":"line","start":1,"end":1}]"#,
            "v1",
        );
    }
    drop(conn);

    let r = env.retriever();
    let hits = r
        .search(&SearchQuery {
            text: "alpha".to_string(),
            mode: SearchMode::Lexical,
            k: 10,
            filters: SearchFilters::default(),
        })
        .expect("search");
    assert!(
        !hits.is_empty(),
        "fixture should produce at least one hit for 'alpha'"
    );
    for h in &hits {
        assert_eq!(
            h.score_kind, ScoreKind::Bm25,
            "lexical retriever must label all hits with ScoreKind::Bm25"
        );
    }
}

// ── TestEnv helper for fb-36 filter tests ───────────────────────────────

/// Convenience wrapper over `Env` that exposes higher-level fixture helpers
/// for the fb-36 filter tests.  Intentionally kept separate from `Env` so
/// the original tests are untouched.
struct TestEnv {
    inner: Env,
    counter: std::cell::Cell<u32>,
}

impl TestEnv {
    fn new() -> Self {
        Self {
            inner: Env::new(),
            counter: std::cell::Cell::new(0),
        }
    }

    /// Allocate a fresh monotone counter suffix so every inserted doc / chunk
    /// gets a unique 32-hex ID without the caller worrying about collisions.
    fn next_id(&self, prefix: &str) -> String {
        let n = self.counter.get();
        self.counter.set(n + 1);
        let suffix = format!("{prefix}{n:04}");
        id32(&suffix)
    }

    /// Insert a markdown doc with the given `body` and return its `DocumentId`.
    fn insert_doc(&self, path: &str, body: &str) -> DocumentId {
        self.insert_doc_with_media(path, body, MediaType::Markdown)
    }

    /// Insert a doc whose `assets.media_type` JSON is set to the serialized
    /// form of `media`.  The `documents.updated_at` defaults to now.
    fn insert_doc_with_media(&self, path: &str, body: &str, media: MediaType) -> DocumentId {
        self.insert_doc_full(path, body, media, OffsetDateTime::now_utc())
    }

    /// Insert a doc with an explicit `updated_at` timestamp (for
    /// `ingested_after` filter tests).
    fn insert_doc_with_updated_at(
        &self,
        path: &str,
        body: &str,
        updated_at: OffsetDateTime,
    ) -> DocumentId {
        self.insert_doc_full(path, body, MediaType::Markdown, updated_at)
    }

    fn insert_doc_full(
        &self,
        path: &str,
        body: &str,
        media: MediaType,
        updated_at: OffsetDateTime,
    ) -> DocumentId {
        use time::format_description::well_known::Rfc3339;
        let doc_id = self.next_id("doc");
        let chunk_id = self.next_id("chk");
        let asset_id = self.next_id("ast");
        let media_json = serde_json::to_string(&media).expect("serialize MediaType");
        let updated_at_str = updated_at.format(&Rfc3339).expect("format updated_at");

        let conn = self.inner.raw_conn();
        conn.execute(
            "INSERT OR IGNORE INTO assets (
                asset_id, source_uri, workspace_path, media_type, byte_len,
                checksum, storage_kind, storage_path, discovered_at
            ) VALUES (?, ?, ?, ?, 0,
                      'd0', 'reference', ?, '2024-01-01T00:00:00Z')",
            rusqlite::params![asset_id, format!("file:///{path}"), path, media_json, path],
        )
        .expect("insert asset");

        conn.execute(
            "INSERT INTO documents (
                doc_id, asset_id, workspace_path, title, lang,
                source_type, trust_level, parser_version,
                doc_version, schema_version, metadata_json,
                provenance_json, created_at, updated_at
            ) VALUES (?, ?, ?, NULL, 'en', 'markdown', 'primary', 'pv1', 1, 1,
                      '{}', '{\"events\":[]}',
                      '2024-01-01T00:00:00Z', ?)",
            rusqlite::params![doc_id, asset_id, path, updated_at_str],
        )
        .expect("insert document");

        let empty_headings: Vec<&str> = vec![];
        let heading_json = serde_json::to_string(&empty_headings).unwrap();
        conn.execute(
            "INSERT INTO chunks (
                chunk_id, doc_id, text, heading_path_json, section_label,
                source_spans_json, token_estimate, chunker_version,
                policy_hash, block_ids_json, created_at
            ) VALUES (?, ?, ?, ?, NULL,
                      '[{\"kind\":\"line\",\"start\":1,\"end\":1}]',
                      1, 'v1', 'h', '[]', '2024-01-01T00:00:00Z')",
            rusqlite::params![chunk_id, doc_id, body, heading_json],
        )
        .expect("insert chunk");

        DocumentId(doc_id)
    }

    fn run_search(&self, query: &str, filters: &SearchFilters) -> Vec<SearchHit> {
        let r = self.inner.retriever();
        let q = SearchQuery {
            text: query.to_string(),
            mode: SearchMode::Lexical,
            k: 10,
            filters: filters.clone(),
        };
        r.search(&q).expect("search")
    }
}

// ── fb-36 filter tests ───────────────────────────────────────────────────

#[test]
fn lexical_filter_by_media() {
    let env = TestEnv::new();
    env.insert_doc_with_media("md1.md", "rust ownership", MediaType::Markdown);
    env.insert_doc_with_media("doc.pdf", "rust pdf body", MediaType::Pdf);
    let filters = SearchFilters {
        media: vec!["pdf".to_string()],
        ..Default::default()
    };
    let hits = env.run_search("rust", &filters);
    assert_eq!(hits.len(), 1, "only pdf doc should match");
    assert!(hits[0].doc_path.0.ends_with(".pdf"), "got: {}", hits[0].doc_path.0);
}

#[test]
fn lexical_filter_by_ingested_after() {
    let env = TestEnv::new();
    env.insert_doc_with_updated_at(
        "old.md",
        "ingest test",
        time::macros::datetime!(2020-01-01 00:00:00 UTC),
    );
    env.insert_doc_with_updated_at(
        "new.md",
        "ingest test",
        time::macros::datetime!(2026-01-01 00:00:00 UTC),
    );
    let filters = SearchFilters {
        ingested_after: Some(time::macros::datetime!(2025-01-01 00:00:00 UTC)),
        ..Default::default()
    };
    let hits = env.run_search("ingest", &filters);
    assert_eq!(hits.len(), 1, "only post-2025 doc matches");
}

#[test]
fn lexical_filter_by_doc_id() {
    let env = TestEnv::new();
    let target = env.insert_doc("a.md", "shared term");
    env.insert_doc("b.md", "shared term");
    let filters = SearchFilters {
        doc_id: Some(target.clone()),
        ..Default::default()
    };
    let hits = env.run_search("shared", &filters);
    assert!(!hits.is_empty(), "should get at least one hit for target doc");
    for h in &hits {
        assert_eq!(h.doc_id, target, "all hits must be from target doc");
    }
}

#[test]
fn lexical_filter_combinator_is_and() {
    let env = TestEnv::new();
    let target = env.insert_doc_with_media("a.md", "rust", MediaType::Markdown);
    env.insert_doc_with_media("b.pdf", "rust", MediaType::Pdf);
    let filters = SearchFilters {
        media: vec!["markdown".to_string()],
        doc_id: Some(target.clone()),
        ..Default::default()
    };
    let hits = env.run_search("rust", &filters);
    assert!(!hits.is_empty(), "target doc should match combined filter");
    assert!(hits.iter().all(|h| h.doc_id == target));
}

#[test]
fn lexical_filter_unknown_media_returns_empty() {
    let env = TestEnv::new();
    env.insert_doc("a.md", "rust");
    let filters = SearchFilters {
        media: vec!["nonexistent_kind".to_string()],
        ..Default::default()
    };
    let hits = env.run_search("rust", &filters);
    assert!(hits.is_empty(), "unknown media → no hits, no error");
}

#[test]
fn lexical_empty_filters_match_default_behavior() {
    let env = TestEnv::new();
    env.insert_doc("a.md", "rust");
    let with_default = env.run_search("rust", &SearchFilters::default());
    assert!(!with_default.is_empty());
}

#[test]
fn lexical_snapshot_run_1() {
    // Pinned snapshot. A small, deterministic corpus; the JSON shape of
    // `Vec<SearchHit>` for a fixed query is checked verbatim against
    // `tests/fixtures/search/lexical/run-1.json`. Update both sides in
    // the same commit when intentional changes ship.
    // Stable because rusqlite ships bundled SQLite — a tokenizer/bm25 algorithm change in a future SQLite bump will require regenerating run-1.json via `KEBAB_UPDATE_SNAPSHOTS=1`.
    let env = Env::new();
    let conn = env.raw_conn();
    insert_document(&conn, &id32("d"), "notes/snap.md", "Snap", "en", "primary", &[]);
    for (cid, body, span) in [
        (
            "c1",
            "alpha bravo charlie",
            r#"[{"kind":"line","start":1,"end":2}]"#,
        ),
        (
            "c2",
            "bravo only here",
            r#"[{"kind":"line","start":4,"end":5}]"#,
        ),
        (
            "c3",
            "alpha alpha",
            r#"[{"kind":"line","start":7,"end":8}]"#,
        ),
    ] {
        insert_chunk(&conn, &id32(cid), &id32("d"), body, &["Snap"], Some("Snap"), span, "v1");
    }
    drop(conn);

    let r = env.retriever();
    let hits = r
        .search(&SearchQuery {
            text: "alpha".to_string(),
            mode: SearchMode::Lexical,
            k: 10,
            filters: SearchFilters::default(),
        })
        .unwrap();
    let actual = serde_json::to_value(&hits).unwrap();

    let baseline_path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/search/lexical/run-1.json");
    if std::env::var_os("KEBAB_UPDATE_SNAPSHOTS").is_some() {
        std::fs::write(&baseline_path, serde_json::to_string_pretty(&actual).unwrap()).unwrap();
    }
    let baseline_text = std::fs::read_to_string(&baseline_path)
        .expect("baseline snapshot must exist; run with KEBAB_UPDATE_SNAPSHOTS=1 to seed");
    let expected: serde_json::Value = serde_json::from_str(&baseline_text).unwrap();
    assert_eq!(actual, expected, "lexical run-1 snapshot drift");
}
