//! `kebab schema` — introspection report. See spec
//! `docs/superpowers/specs/2026-05-07-p9-fb-27-introspection-and-error-wire-design.md`.

use serde::{Deserialize, Serialize};

use kebab_config::Config;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaV1 {
    pub schema_version: String,
    pub kebab_version: String,
    pub wire: WireBlock,
    pub capabilities: Capabilities,
    pub models: Models,
    pub stats: Stats,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireBlock {
    pub schemas: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Capabilities {
    pub json_mode: bool,
    pub ingest_progress: bool,
    pub ingest_cancellation: bool,
    pub rag_multi_turn: bool,
    pub search_cache: bool,
    pub incremental_ingest: bool,
    pub streaming_ask: bool,
    pub http_daemon: bool,
    pub mcp_server: bool,
    pub single_file_ingest: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Models {
    pub parser_version: String,
    pub chunker_version: String,
    pub embedding_version: String,
    pub prompt_template_version: String,
    pub index_version: String,
    pub corpus_revision: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stats {
    pub doc_count: u64,
    pub chunk_count: u64,
    pub asset_count: u64,
    pub last_ingest_at: Option<String>,
}

const KEBAB_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Wire schema id for [`SchemaV1`]. Single source of truth — `kebab-cli`
/// re-uses this via `kebab_app::schema::SCHEMA_V1_ID` when wrapping.
pub const SCHEMA_V1_ID: &str = "schema.v1";

// Authoritative list of wire schemas this binary emits. Keep in sync with
// `docs/wire-schema/v1/*.schema.json` and `kebab-cli::wire::wire_*` helpers.
const WIRE_SCHEMAS: &[&str] = &[
    "answer.v1",
    "search_hit.v1",
    "doc_summary.v1",
    "chunk_inspection.v1",
    "doctor.v1",
    "ingest_report.v1",
    "ingest_progress.v1",
    "reset_report.v1",
    "citation.v1",
    "schema.v1",
    "error.v1",
];

/// Build a [`SchemaV1`] introspection report for the given config.
///
/// Opens the SQLite store read-only via [`kebab_store_sqlite::SqliteStore::open_existing`]
/// so the caller (kebab-cli) does not need write access to the data dir.
/// Returns a [`kebab_store_sqlite::NotIndexed`] error (wrapped in `anyhow`)
/// if the database file does not exist — the CLI translates that to an
/// `error.v1` / `"not_indexed"` wire record.
#[doc(hidden)]
pub fn schema_with_config(cfg: &Config) -> anyhow::Result<SchemaV1> {
    let store = open_store_for_stats(cfg)?;
    let stats = collect_stats(&store)?;
    let models = collect_models(cfg, &store);
    Ok(SchemaV1 {
        schema_version: SCHEMA_V1_ID.to_string(),
        kebab_version: KEBAB_VERSION.to_string(),
        wire: WireBlock {
            schemas: WIRE_SCHEMAS.iter().map(|s| (*s).to_string()).collect(),
        },
        capabilities: capabilities_snapshot(),
        models,
        stats,
    })
}

fn capabilities_snapshot() -> Capabilities {
    Capabilities {
        json_mode: true,
        ingest_progress: true,
        ingest_cancellation: true,
        rag_multi_turn: true,
        search_cache: true,
        incremental_ingest: true,
        streaming_ask: false,
        http_daemon: false,
        mcp_server: true,
        single_file_ingest: false,
    }
}

fn open_store_for_stats(cfg: &Config) -> anyhow::Result<kebab_store_sqlite::SqliteStore> {
    // Mirror the data_dir resolution used in SqliteStore::open:
    // kebab_config::expand_path(&cfg.storage.data_dir, "") resolves tilde
    // and env vars. The SQLITE_FILE name ("kebab.sqlite") is the canonical
    // file name defined in kebab-store-sqlite.
    let data_dir = kebab_config::expand_path(&cfg.storage.data_dir, "");
    let db_path = data_dir.join("kebab.sqlite");
    kebab_store_sqlite::SqliteStore::open_existing(&db_path)
}

fn collect_stats(store: &kebab_store_sqlite::SqliteStore) -> anyhow::Result<Stats> {
    let counts = store.count_summary()?;
    Ok(Stats {
        doc_count: counts.doc_count,
        chunk_count: counts.chunk_count,
        asset_count: counts.asset_count,
        last_ingest_at: counts.last_ingest_at,
    })
}

fn collect_models(cfg: &Config, store: &kebab_store_sqlite::SqliteStore) -> Models {
    Models {
        // markdown parser only — pdf-page-v1 (P7) / image extractors (P6)
        // maintain their own versions; surface those when SchemaV1.models
        // becomes a multi-medium map (P+).
        parser_version: kebab_parse_md::PARSER_VERSION.to_string(),
        chunker_version: cfg.chunking.chunker_version.clone(),
        // EmbeddingModelCfg uses `.model` (not `.id`) — adapt from plan.
        embedding_version: cfg.models.embedding.model.clone(),
        prompt_template_version: cfg.rag.prompt_template_version.clone(),
        index_version: kebab_store_vector::INDEX_VERSION_STR.to_string(),
        // corpus_revision returns u64 directly (no Result) — matches
        // existing impl; treat 0 as the default for a fresh/unrevised store.
        corpus_revision: store.corpus_revision(),
    }
}
