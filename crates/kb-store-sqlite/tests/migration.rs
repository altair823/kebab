//! Migration test: a fresh DB, after `run_migrations`, exposes every
//! table and index P1 needs (per §5.1–§5.7).

use kb_store_sqlite::SqliteStore;

mod common;

#[test]
fn fresh_db_has_all_p1_tables_and_indexes() {
    let env = common::TestEnv::new();
    let store = SqliteStore::open(&env.config()).expect("open");
    store.run_migrations().expect("run migrations");

    // Pull the list of user tables from sqlite_master.
    let tables: Vec<String> = env.with_conn(|c| {
        let mut stmt = c.prepare(
            "SELECT name FROM sqlite_master
             WHERE type = 'table' AND name NOT LIKE 'sqlite_%'
             ORDER BY name",
        )?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let tables: rusqlite::Result<Vec<String>> = rows.collect();
        tables
    });

    let required = [
        "answers",
        "assets",
        "blocks",
        "chunks",
        "document_tags",
        "documents",
        "embedding_records",
        "eval_query_results",
        "eval_runs",
        "ingest_runs",
        "jobs",
        // refinery's own bookkeeping table (`refinery_schema_history`)
        // also lands here; we don't pin it but it's expected.
        "migrations",
        "schema_meta",
    ];
    for t in required {
        assert!(
            tables.iter().any(|n| n == t),
            "table `{t}` missing; got {tables:?}"
        );
    }

    // Pin the documented indexes (subset that matters for hot paths).
    let indexes: Vec<String> = env.with_conn(|c| {
        let mut stmt = c.prepare(
            "SELECT name FROM sqlite_master
             WHERE type = 'index' AND name NOT LIKE 'sqlite_%'
             ORDER BY name",
        )?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let idx: rusqlite::Result<Vec<String>> = rows.collect();
        idx
    });
    for i in [
        "idx_assets_workspace_path",
        "idx_assets_media_type",
        "idx_docs_workspace_path",
        "idx_docs_lang",
        "idx_docs_source_type",
        "idx_document_tags_tag",
        "idx_blocks_doc_id",
        "idx_chunks_doc_id",
        "idx_chunks_chunker_version",
        "idx_embed_chunk",
        "idx_embed_model",
        "idx_jobs_status",
        "idx_jobs_kind",
        "idx_answers_created_at",
        "idx_answers_grounded",
    ] {
        assert!(
            indexes.iter().any(|n| n == i),
            "index `{i}` missing; got {indexes:?}"
        );
    }
}
