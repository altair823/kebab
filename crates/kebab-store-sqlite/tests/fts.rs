//! P2-1 FTS5 schema + trigger + rebuild tests.
//!
//! Strategy: `chunks_fts` triggers fire off raw SQL on `chunks`, so we
//! seed and mutate via direct INSERT/UPDATE/DELETE rather than the full
//! `kb-parse-md → kb-normalize → kb-chunk → put_chunks` pipeline. That
//! keeps the assertions about trigger behavior independent of any
//! upstream crate. The `chunks` rows we produce satisfy NOT NULL on the
//! columns required by V001 §5.5; we elide FK pressure on `documents`
//! by disabling foreign keys for the test connection (the trigger logic
//! we exercise has no `documents` dependency).
//!
//! Test connections open a fresh side-channel `rusqlite::Connection`
//! that bypasses the `SqliteStore` mutex; that's fine because each test
//! gets its own tempdir and no concurrent mutator is in flight.

use kebab_store_sqlite::{SqliteStore, rebuild_chunks_fts};
use rusqlite::Connection;

mod common;

/// Insert a chunks row directly. The triggers will mirror it into
/// `chunks_fts` as part of the same statement.
fn insert_chunk(
    conn: &Connection,
    chunk_id: &str,
    doc_id: &str,
    heading_path_json: &str,
    text: &str,
) {
    conn.execute(
        "INSERT INTO chunks (
            chunk_id, doc_id, text, heading_path_json, section_label,
            source_spans_json, token_estimate, chunker_version,
            policy_hash, block_ids_json, created_at
        ) VALUES (?, ?, ?, ?, NULL, '[]', 0, 'v1', 'h', '[]', '2024-01-01T00:00:00Z')",
        rusqlite::params![chunk_id, doc_id, text, heading_path_json],
    )
    .expect("insert chunk row");
}

fn count(conn: &Connection, table: &str) -> i64 {
    conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
        .expect("count")
}

/// Open a fresh side-channel connection with FK enforcement OFF. The
/// FTS triggers we test do not touch `documents`, but `chunks` has a
/// FK to `documents(doc_id)`; turning FK enforcement off lets us seed
/// chunks without first synthesizing a full documents/assets row graph.
fn raw_conn_no_fk(env: &common::TestEnv) -> Connection {
    let conn = Connection::open(env.db_path()).expect("open side conn");
    conn.pragma_update(None, "foreign_keys", "OFF").unwrap();
    conn
}

// ── 1. Migration apply: backfill ──────────────────────────────────────

/// Apply V001 only, seed N rows into `chunks` (which has no FTS shadow
/// at this point — V001 doesn't create `chunks_fts`), then apply V002's
/// SQL verbatim. The V002 backfill INSERT must produce one chunks_fts
/// row per pre-existing chunks row, and each row's columns must match.
///
/// This is the literal cold-upgrade path: V001-shipped database, V002
/// applied on top, existing chunks become searchable without re-ingest.
/// The trigger-based mirror (chunks_ai) is covered by the §2 tests.
#[test]
fn fts_v002_backfills_existing_chunks() {
    let env = common::TestEnv::new();
    let conn = Connection::open(env.db_path()).expect("open db");
    conn.pragma_update(None, "foreign_keys", "OFF").unwrap();

    // 1) Apply V001 only — chunks table exists, chunks_fts does not.
    let v001_sql = include_str!("../../../migrations/V001__init.sql");
    conn.execute_batch(v001_sql).expect("apply V001");
    assert!(
        conn.query_row(
            "SELECT name FROM sqlite_master WHERE type='table' AND name='chunks_fts'",
            [],
            |r| r.get::<_, String>(0),
        )
        .is_err(),
        "chunks_fts must not exist under V001 only"
    );

    // 2) Seed pre-existing chunks rows (the V001-shipped state we expect
    //    on a customer DB upgrading from P1 to P2-1).
    const N: usize = 4;
    for i in 0..N {
        let cid = format!("{i:0>32}");
        insert_chunk(
            &conn,
            &cid,
            &"d".repeat(32),
            "[\"Section\"]",
            &format!("seedrow{i} payload"),
        );
    }
    assert_eq!(count(&conn, "chunks"), N as i64);

    // 3) Apply V002 verbatim — its CREATE VIRTUAL TABLE + triggers + the
    //    final backfill INSERT. The triggers don't fire on this path
    //    (they only fire on chunks INSERT/UPDATE/DELETE); the backfill
    //    INSERT does the work.
    let v002_sql = include_str!("../../../migrations/V002__fts.sql");
    conn.execute_batch(v002_sql).expect("apply V002");

    // 4) Assert: count parity, and the backfilled rows mirror the chunks
    //    rows column-for-column on the indexed/UNINDEXED columns.
    assert_eq!(
        count(&conn, "chunks_fts"),
        N as i64,
        "V002 backfill INSERT must seed one chunks_fts row per chunks row"
    );
    for i in 0..N {
        let cid = format!("{i:0>32}");
        let term = format!("seedrow{i}");
        let hit: String = conn
            .query_row(
                "SELECT chunk_id FROM chunks_fts WHERE chunks_fts MATCH ?",
                [&term],
                |r| r.get(0),
            )
            .unwrap_or_else(|_| panic!("MATCH {term} must hit backfilled row"));
        assert_eq!(hit, cid, "backfill must preserve chunk_id mapping");
    }
}

/// Direct test of the V002 backfill INSERT on a DB seeded under V001.
/// We achieve V001-only state by running all migrations, dropping the
/// FTS rows, then re-running the exact backfill INSERT V002 ships and
/// asserting count parity.
#[test]
fn fts_v002_backfill_select_matches_chunks_count() {
    let env = common::TestEnv::new();
    let store = SqliteStore::open(&env.config()).unwrap();
    store.run_migrations().unwrap();

    let conn = raw_conn_no_fk(&env);
    for i in 0..5 {
        let cid = format!("{i:0>32}");
        insert_chunk(&conn, &cid, &"d".repeat(32), "[]", &format!("row {i}"));
    }
    // Wipe + run the literal V002 backfill INSERT.
    conn.execute("DELETE FROM chunks_fts", []).unwrap();
    assert_eq!(count(&conn, "chunks_fts"), 0);
    conn.execute(
        "INSERT INTO chunks_fts(chunk_id, doc_id, heading_path, text)
         SELECT chunk_id, doc_id, heading_path_json, text FROM chunks",
        [],
    )
    .unwrap();
    assert_eq!(count(&conn, "chunks_fts"), count(&conn, "chunks"));
}

// ── 2. Trigger sync: INSERT / DELETE / UPDATE ────────────────────────

#[test]
fn fts_chunks_ai_trigger_propagates_insert() {
    let env = common::TestEnv::new();
    let store = SqliteStore::open(&env.config()).unwrap();
    store.run_migrations().unwrap();

    let conn = raw_conn_no_fk(&env);
    insert_chunk(
        &conn,
        &"a".repeat(32),
        &"d".repeat(32),
        "[\"Heading\"]",
        "needle in haystack",
    );

    // chunks_fts row count == 1 and MATCH finds it.
    assert_eq!(count(&conn, "chunks_fts"), 1);
    let hit: String = conn
        .query_row(
            "SELECT chunk_id FROM chunks_fts WHERE chunks_fts MATCH 'needle'",
            [],
            |r| r.get(0),
        )
        .expect("MATCH 'needle' must hit");
    assert_eq!(hit, "a".repeat(32));
}

#[test]
fn fts_chunks_ad_trigger_propagates_delete() {
    let env = common::TestEnv::new();
    let store = SqliteStore::open(&env.config()).unwrap();
    store.run_migrations().unwrap();

    let conn = raw_conn_no_fk(&env);
    let cid = "a".repeat(32);
    insert_chunk(&conn, &cid, &"d".repeat(32), "[]", "ephemeral");
    assert_eq!(count(&conn, "chunks_fts"), 1);

    conn.execute("DELETE FROM chunks WHERE chunk_id = ?", [&cid])
        .expect("delete chunk");
    assert_eq!(
        count(&conn, "chunks_fts"),
        0,
        "chunks_ad must remove the FTS row"
    );
}

#[test]
fn fts_chunks_au_trigger_propagates_update() {
    let env = common::TestEnv::new();
    let store = SqliteStore::open(&env.config()).unwrap();
    store.run_migrations().unwrap();

    let conn = raw_conn_no_fk(&env);
    let cid = "a".repeat(32);
    insert_chunk(&conn, &cid, &"d".repeat(32), "[]", "before");

    // Old text is searchable.
    assert_eq!(count_match(&conn, "before"), 1);
    assert_eq!(count_match(&conn, "after"), 0);

    conn.execute(
        "UPDATE chunks SET text = ? WHERE chunk_id = ?",
        rusqlite::params!["after rewrite", cid],
    )
    .expect("update chunk text");

    // New text is searchable; old token is gone. Row count unchanged.
    assert_eq!(count(&conn, "chunks_fts"), 1);
    assert_eq!(
        count_match(&conn, "before"),
        0,
        "old text must not survive UPDATE"
    );
    assert_eq!(count_match(&conn, "after"), 1, "new text must be indexed");
}

fn count_match(conn: &Connection, term: &str) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM chunks_fts WHERE chunks_fts MATCH ?",
        [term],
        |r| r.get(0),
    )
    .expect("count_match")
}

// ── 3. rebuild_chunks_fts ────────────────────────────────────────────

#[test]
fn fts_rebuild_chunks_fts_is_idempotent() {
    let env = common::TestEnv::new();
    let store = SqliteStore::open(&env.config()).unwrap();
    store.run_migrations().unwrap();

    let conn = raw_conn_no_fk(&env);
    for i in 0..3 {
        let cid = format!("{i:0>32}");
        insert_chunk(&conn, &cid, &"d".repeat(32), "[]", &format!("token{i}"));
    }
    let before = count(&conn, "chunks_fts");
    assert_eq!(before, 3);

    // First rebuild: trivial round-trip — same row count.
    rebuild_chunks_fts(&conn).expect("rebuild 1");
    assert_eq!(count(&conn, "chunks_fts"), before);

    // Second rebuild: idempotent (same row count again).
    rebuild_chunks_fts(&conn).expect("rebuild 2");
    assert_eq!(count(&conn, "chunks_fts"), before);

    // After rebuild, MATCH still finds expected tokens.
    for i in 0..3 {
        assert_eq!(count_match(&conn, &format!("token{i}")), 1);
    }
}

#[test]
fn fts_rebuild_chunks_fts_recovers_from_drift() {
    let env = common::TestEnv::new();
    let store = SqliteStore::open(&env.config()).unwrap();
    store.run_migrations().unwrap();

    let conn = raw_conn_no_fk(&env);
    let cid = "a".repeat(32);
    insert_chunk(&conn, &cid, &"d".repeat(32), "[]", "recovered");

    // Manually wipe chunks_fts to simulate drift; this is the failure
    // mode `kb index --rebuild-fts` exists to recover from.
    conn.execute("DELETE FROM chunks_fts", []).unwrap();
    assert_eq!(count(&conn, "chunks_fts"), 0);
    assert_eq!(count(&conn, "chunks"), 1);

    rebuild_chunks_fts(&conn).expect("rebuild");
    assert_eq!(count(&conn, "chunks_fts"), 1);
    assert_eq!(count_match(&conn, "recovered"), 1);
}

// ── 4. Migration double-apply no-op ──────────────────────────────────

#[test]
fn fts_double_run_migrations_is_noop() {
    let env = common::TestEnv::new();
    let store = SqliteStore::open(&env.config()).unwrap();
    store.run_migrations().expect("run 1");
    // Second invocation must be a no-op (refinery's bookkeeping table
    // tracks applied versions). The chunks_fts virtual table is still
    // present and queryable.
    store.run_migrations().expect("run 2");

    let conn = raw_conn_no_fk(&env);
    // The virtual table is queryable.
    let n: i64 = conn
        .query_row("SELECT COUNT(*) FROM chunks_fts", [], |r| r.get(0))
        .expect("chunks_fts queryable after double-run");
    assert_eq!(n, 0);
}

// ── 5. CI diff guard: V002 SQL matches design §5.5 verbatim ──────────

/// Whitespace-normalize a SQL block: trim, then collapse every run of
/// whitespace (newlines included) into a single space. Lets the
/// design-doc ↔ migration-file comparison ignore cosmetic drift like
/// blank-line counts while still catching token-level changes.
fn normalize_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Extract the §5.5 FTS slice from the design doc: locate the
/// `### 5.5 Chunks + FTS5` heading, walk to the next ```sql fenced
/// block, then within that block slice from `CREATE VIRTUAL TABLE
/// chunks_fts` through the last `END;`. The §5.5 fenced block also
/// contains the `chunks` CREATE TABLE — we only want the FTS portion.
///
/// Failure modes (any of these means the design doc layout drifted —
/// the test should fail loud, which is the point):
/// - heading missing
/// - no ```sql block follows
/// - no `CREATE VIRTUAL TABLE chunks_fts` inside that block
/// - no `END;` after the virtual-table line
fn extract_design_5_5_fts_block() -> String {
    let doc = include_str!("../../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md");
    let heading_idx = doc
        .find("### 5.5 Chunks + FTS5")
        .expect("design doc must contain `### 5.5 Chunks + FTS5` heading");
    let after_heading = &doc[heading_idx..];

    // Find the opening fence ```sql after the heading.
    let fence_open_rel = after_heading
        .find("```sql")
        .expect("§5.5 must be followed by a ```sql fenced block");
    // Move past the fence line.
    let body_start_rel = fence_open_rel
        + after_heading[fence_open_rel..]
            .find('\n')
            .expect("```sql fence must end with a newline")
        + 1;
    let body = &after_heading[body_start_rel..];
    let fence_close_rel = body
        .find("\n```")
        .expect("§5.5 ```sql block must close with ``` on its own line");
    let fenced = &body[..fence_close_rel];

    // Within the fenced block, slice from CREATE VIRTUAL TABLE chunks_fts
    // through the last `END;`.
    let virt_idx = fenced
        .find("CREATE VIRTUAL TABLE chunks_fts")
        .expect("§5.5 fenced block must contain `CREATE VIRTUAL TABLE chunks_fts`");
    let fts_slice = &fenced[virt_idx..];
    let last_end = fts_slice
        .rfind("END;")
        .expect("§5.5 FTS slice must terminate with `END;`");
    fts_slice[..last_end + "END;".len()].to_string()
}

/// Extract the §5.5 verbatim block from the V007 migration (replaced V002
/// 's unicode61 tokenizer with trigram — V002 stays in place for
/// historical cold-upgrade replay but V007 is now the source of truth),
/// between the `── §5.5 verbatim block ──` anchor markers V007 carries.
fn extract_migration_5_5_verbatim_block() -> String {
    let migration = include_str!("../../../migrations/V007__fts_trigram.sql");
    // The opening anchor line ends with `── §5.5 verbatim block ─...`.
    let open_marker = "§5.5 verbatim block";
    let close_marker = "End §5.5 verbatim block";

    let open_idx = migration
        .find(open_marker)
        .expect("V007 must carry the `§5.5 verbatim block` opening anchor");
    let after_open_line = open_idx
        + migration[open_idx..]
            .find('\n')
            .expect("opening anchor line must end with a newline")
        + 1;

    let close_idx = migration[after_open_line..]
        .find(close_marker)
        .expect("V007 must carry the `End §5.5 verbatim block` closing anchor")
        + after_open_line;
    // Walk back from the close marker to the start of its comment line.
    let close_line_start = migration[..close_idx].rfind('\n').map_or(0, |n| n + 1);

    migration[after_open_line..close_line_start].to_string()
}

/// CI diff guard: the §5.5 block in `migrations/V007__fts_trigram.sql`
/// must match the design doc verbatim (whitespace-normalized). V007
/// replaced V002 's unicode61 tokenizer with trigram (2026-05-23).
/// V002 stays in place for historical replay of cold-upgrade paths
/// but is no longer compared against the design doc — V007 is now
/// the source of truth.
#[test]
fn fts_v007_matches_design_section_5_5_verbatim() {
    let design = extract_design_5_5_fts_block();
    let migration_block = extract_migration_5_5_verbatim_block();

    // Sanity: the slices we extracted look like the §5.5 FTS block (not
    // some unrelated snippet that happened to match a marker).
    assert!(
        design.contains("CREATE VIRTUAL TABLE chunks_fts"),
        "design slice must include CREATE VIRTUAL TABLE chunks_fts"
    );
    assert!(
        migration_block.contains("CREATE VIRTUAL TABLE chunks_fts"),
        "migration slice must include CREATE VIRTUAL TABLE chunks_fts"
    );
    assert!(
        design.trim_end().ends_with("END;"),
        "design slice must terminate with END;"
    );

    let design_n = normalize_ws(&design);
    let migration_n = normalize_ws(&migration_block);
    assert_eq!(
        design_n, migration_n,
        "V007__fts_trigram.sql §5.5 block must match design doc §5.5 verbatim \
         (whitespace-normalized). If you intentionally changed one, \
         update the other in the same commit."
    );
}

// ── 6. WAL cleanup: drop store before tempdir reaps WAL/SHM ──────────

/// Mirror the P1-6 pattern: opening + migrating + dropping the store
/// must not strand `kebab.sqlite-wal`/`-shm` files such that the tempdir
/// can't be cleaned up. After dropping the store + side-channel conn,
/// the WAL/SHM siblings must either not exist or be removable — if a
/// stray handle were holding them open, on Windows the remove would
/// fail (on Linux unlink succeeds even with open handles, so this is
/// mostly a portability canary, but we still assert).
#[test]
fn fts_store_drop_releases_wal_files() {
    let env = common::TestEnv::new();
    let db_path = env.db_path();
    {
        let store = SqliteStore::open(&env.config()).unwrap();
        store.run_migrations().unwrap();
        // Force at least one trigger fire so WAL has content to flush.
        let conn = raw_conn_no_fk(&env);
        insert_chunk(&conn, &"a".repeat(32), &"d".repeat(32), "[]", "x");
        drop(conn);
        drop(store);
    }

    // After the store drops, any remaining WAL/SHM siblings must be
    // removable. If a connection is still open this would fail on
    // platforms with mandatory file locking.
    for suffix in ["-wal", "-shm"] {
        let p = db_path.with_extension(format!("sqlite{suffix}"));
        if p.exists() {
            std::fs::remove_file(&p).unwrap_or_else(|e| {
                panic!(
                    "WAL/SHM sibling {} should be removable after store drop: {e}",
                    p.display()
                )
            });
        }
    }
    // The main DB file should likewise be removable.
    if db_path.exists() {
        std::fs::remove_file(&db_path).expect("main DB file should be removable after store drop");
    }
}

// ── 7. Trigram tokenizer behavior (V007) — Korean + English ──────────

/// V007 의 trigram tokenizer 가 한국어 3자 이상 연속 substring 을
/// 매칭하는지. Codex round 1/2 가 sqlite 3.45.1 로 검증한 동작을 pin:
/// - raw query 가 3자 이상 공백 없는 substring 인 경우 hit.
/// - raw query 가 공백을 포함하면 FTS5 가 토큰 경계로 분리 →
///   양 토큰이 3자 미만이면 0-hit.
/// - quoted phrase ("..." 안에 공백 포함) 는 통째로 substring 매칭.
#[test]
fn fts_trigram_korean_3char_substring_hits() {
    let env = common::TestEnv::new();
    let store = SqliteStore::open(&env.config()).unwrap();
    store.run_migrations().unwrap();

    let conn = raw_conn_no_fk(&env);
    insert_chunk(
        &conn,
        &"k".repeat(32),
        &"d".repeat(32),
        "[]",
        "해시 충돌은 키와 값을 매핑할 때 발생한다",
    );

    // raw 3+ chars 공백 없는 연속 substring → hit.
    assert_eq!(
        count_match(&conn, "충돌은"),
        1,
        "raw 3-char 공백 없는 substring '충돌은' must hit"
    );
    assert_eq!(
        count_match(&conn, "발생한"),
        1,
        "raw 3-char 공백 없는 substring '발생한' must hit"
    );

    // quoted phrase (공백 포함) → substring 매칭으로 hit.
    assert_eq!(
        count_match(&conn, "\"해시 충돌\""),
        1,
        "quoted whole phrase '해시 충돌' (5 chars including space)"
    );
    assert_eq!(
        count_match(&conn, "\"시 충\""),
        1,
        "quoted phrase '시 충' across the space boundary"
    );

    // raw with no whitespace but substring not present in source → 0-hit.
    assert_eq!(
        count_match(&conn, "해시충"),
        0,
        "원문에 공백 없는 '해시충' trigram 이 없으므로 0-hit"
    );
}

/// V007 trigram 의 핵심 제약: 3 Unicode chars 미만 query 는 색인 단위가
/// 없어 항상 0-hit. design §3.4 + 사용자 결정 (lexical core 정상 0-hit,
/// CLI/TUI wrapper 가 안내 메시지 출력). 회귀 감지 — trigram 구조 변경
/// 또는 다른 tokenizer 도입 시 이 test 가 먼저 fail 한다.
#[test]
fn fts_trigram_korean_short_query_zero_hit_pinned() {
    let env = common::TestEnv::new();
    let store = SqliteStore::open(&env.config()).unwrap();
    store.run_migrations().unwrap();

    let conn = raw_conn_no_fk(&env);
    insert_chunk(
        &conn,
        &"k".repeat(32),
        &"d".repeat(32),
        "[]",
        "해시 충돌은 키와 값을 매핑할 때 발생한다",
    );

    // 2자 한국어 query — 도그푸딩에서 보고된 핵심 케이스 ('충돌'/'값').
    assert_eq!(count_match(&conn, "충돌"), 0, "2-char Korean query");
    // 1자 한국어 query.
    assert_eq!(count_match(&conn, "키"), 0, "1-char Korean query");
}

/// V007 trigram 은 영어에도 substring 매칭으로 동작 — recall ↑, 단어
/// 경계 정밀도 ↓. design §3.4 의 동작 변경을 명시적으로 핀.
#[test]
fn fts_trigram_english_substring_hits() {
    let env = common::TestEnv::new();
    let store = SqliteStore::open(&env.config()).unwrap();
    store.run_migrations().unwrap();

    let conn = raw_conn_no_fk(&env);
    insert_chunk(
        &conn,
        &"e".repeat(32),
        &"d".repeat(32),
        "[]",
        "the tokenizer normalizes whitespace before matching",
    );

    // trigram substring — 'token' hits inside 'tokenizer'.
    assert_eq!(
        count_match(&conn, "token"),
        1,
        "substring of 'tokenizer' — trigram recall"
    );
    assert_eq!(count_match(&conn, "izer"), 1, "substring of 'tokenizer'");
    // 3-char-minimum applies to English too.
    assert_eq!(count_match(&conn, "to"), 0, "2-char English query");
}
