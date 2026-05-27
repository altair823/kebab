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
    pub bulk_search: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Models {
    pub parser_version: String,
    pub chunker_version: String,
    /// v0.20.1+ (Bug #13). Corpus 안 활성 parser version 전체.
    /// 빈 corpus → empty Vec. backward compat: `parser_version` field 보존.
    #[serde(default)]
    pub active_parsers: Vec<String>,
    /// v0.20.1+ (Bug #13). Corpus 안 활성 chunker version 전체.
    /// 빈 corpus → empty Vec.
    #[serde(default)]
    pub active_chunkers: Vec<String>,
    pub embedding_version: String,
    pub prompt_template_version: String,
    pub index_version: String,
    pub corpus_revision: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Stats {
    pub doc_count: u64,
    pub chunk_count: u64,
    pub asset_count: u64,
    pub last_ingest_at: Option<String>,
    /// p9-fb-37: per-media-kind doc count (5 keys, zero-padded).
    #[serde(default)]
    pub media_breakdown: std::collections::BTreeMap<String, u64>,
    /// p9-fb-37: per-language doc count, NULL keyed as `"null"`.
    #[serde(default)]
    pub lang_breakdown: std::collections::BTreeMap<String, u64>,
    /// p9-fb-37: on-disk byte sums.
    #[serde(default)]
    pub index_bytes: kebab_core::IndexBytes,
    /// p9-fb-37: docs whose `updated_at` exceeds the staleness threshold.
    #[serde(default)]
    pub stale_doc_count: u64,
    /// p10-1A-1: code language breakdown (**doc** counts by canonical
    /// lowercase language identifier). Empty until 1A-2 produces code
    /// docs. v0.17.0 PR-C: doc-count semantics corrected here (the
    /// previous "chunk counts" wording was a longstanding mis-label —
    /// implementation has always been `COUNT(*) FROM documents
    /// GROUP BY code_lang`). Use `code_lang_chunk_breakdown` for the
    /// chunk-level companion.
    #[serde(default)]
    pub code_lang_breakdown: std::collections::BTreeMap<String, u32>,
    /// p10-1A-1: repo breakdown (**doc** counts by `metadata.repo`
    /// value). Empty until 1A-2 produces code docs. v0.17.0 PR-C:
    /// doc-count wording corrected (mirror of code_lang_breakdown).
    #[serde(default)]
    pub repo_breakdown: std::collections::BTreeMap<String, u32>,
    /// v0.17.0 PR-C: sister of [`Self::code_lang_breakdown`] returning
    /// chunk counts instead of doc counts. Indexing-pressure metric —
    /// one PDF spec → 200 chunks vs one Rust file → 5 chunks shows up
    /// here in a way `code_lang_breakdown` (doc count) hides.
    #[serde(default)]
    pub code_lang_chunk_breakdown: std::collections::BTreeMap<String, u32>,
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
    "search_response.v1",
    "doc_summary.v1",
    "chunk_inspection.v1",
    "doctor.v1",
    "ingest_report.v1",
    "ingest_progress.v1",
    "reset_report.v1",
    "citation.v1",
    "schema.v1",
    "error.v1",
    "bulk_search_item.v1",
    "bulk_search_response.v1",
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
    let stats = collect_stats(cfg, &store)?;
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
        streaming_ask: true,
        http_daemon: false,
        mcp_server: true,
        single_file_ingest: true,
        bulk_search: true,
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

fn collect_stats(
    cfg: &Config,
    store: &kebab_store_sqlite::SqliteStore,
) -> anyhow::Result<Stats> {
    let counts = store
        .count_summary_with_threshold(u64::from(cfg.search.stale_threshold_days))?;
    let data_dir = kebab_config::expand_path(&cfg.storage.data_dir, "");
    let index_bytes = kebab_store_sqlite::stats_ext::index_bytes(&data_dir)
        .map_err(|e| anyhow::anyhow!("index_bytes: {e}"))?;
    Ok(Stats {
        doc_count: counts.doc_count,
        chunk_count: counts.chunk_count,
        asset_count: counts.asset_count,
        last_ingest_at: counts.last_ingest_at,
        media_breakdown: counts.media_breakdown,
        lang_breakdown: counts.lang_breakdown,
        index_bytes,
        stale_doc_count: counts.stale_doc_count,
        // p10-1A-2: populated by the store query added in this task.
        code_lang_breakdown: store.code_lang_breakdown()?,
        // p10-1A-2 follow-up: dogfooding (2026-05-20) revealed this was a
        // placeholder — mirror of code_lang_breakdown for the repo field.
        repo_breakdown: store.repo_breakdown()?,
        // v0.17.0 PR-C: chunk-level companion (closes HOTFIXES
        // 2026-05-22 "code_lang_breakdown chunk granularity" LOW).
        code_lang_chunk_breakdown: store.code_lang_chunk_breakdown()?,
    })
}

fn collect_models(cfg: &Config, store: &kebab_store_sqlite::SqliteStore) -> Models {
    let active_parsers = store.fetch_distinct_parser_versions().unwrap_or_default();
    let active_chunkers = store.fetch_distinct_chunker_versions().unwrap_or_default();
    Models {
        // markdown parser only — pdf-page-v1 (P7) / image extractors (P6)
        // maintain their own versions; surface those when SchemaV1.models
        // becomes a multi-medium map (P+).
        parser_version: kebab_parse_md::PARSER_VERSION.to_string(),
        chunker_version: cfg.chunking.chunker_version.clone(),
        active_parsers,
        active_chunkers,
        // EmbeddingModelCfg uses `.model` (not `.id`) — adapt from plan.
        embedding_version: cfg.models.embedding.model.clone(),
        prompt_template_version: cfg.rag.prompt_template_version.clone(),
        index_version: kebab_store_vector::INDEX_VERSION_STR.to_string(),
        // corpus_revision returns u64 directly (no Result) — matches
        // existing impl; treat 0 as the default for a fresh/unrevised store.
        corpus_revision: store.corpus_revision(),
    }
}

#[cfg(test)]
mod tests_stats_ext {
    use super::*;

    /// p10-1A-1: Stats must serialize `code_lang_breakdown` and
    /// `repo_breakdown` so downstream consumers (MCP skill, Claude Code)
    /// can branch on their presence.
    #[test]
    fn stats_includes_code_lang_and_repo_breakdown_fields() {
        let stats = Stats::default();
        let v = serde_json::to_value(&stats).unwrap();
        assert!(
            v.get("code_lang_breakdown").is_some(),
            "Stats JSON must include code_lang_breakdown: {v}"
        );
        assert!(
            v.get("repo_breakdown").is_some(),
            "Stats JSON must include repo_breakdown: {v}"
        );
        // v0.17.0 PR-C: chunk-level companion field.
        assert!(
            v.get("code_lang_chunk_breakdown").is_some(),
            "Stats JSON must include code_lang_chunk_breakdown (v0.17.0 PR-C): {v}"
        );
        // Empty BTreeMap serializes as `{}` — confirm it's an object, not null.
        assert!(
            v["code_lang_breakdown"].is_object(),
            "code_lang_breakdown must be an object: {v}"
        );
        assert!(
            v["repo_breakdown"].is_object(),
            "repo_breakdown must be an object: {v}"
        );
        assert!(
            v["code_lang_chunk_breakdown"].is_object(),
            "code_lang_chunk_breakdown must be an object: {v}"
        );
    }

    #[test]
    fn stats_includes_breakdowns_and_bytes_on_fresh_corpus() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = kebab_config::Config::defaults();
        cfg.storage.data_dir = dir.path().to_string_lossy().into_owned();
        // Bring up migrations so the sqlite file is created.
        let store = kebab_store_sqlite::SqliteStore::open(&cfg).unwrap();
        store.run_migrations().unwrap();
        drop(store);

        let s = schema_with_config(&cfg).unwrap();
        // 5 keys padded.
        assert_eq!(s.stats.media_breakdown.len(), 5);
        assert_eq!(s.stats.media_breakdown.get("markdown"), Some(&0));
        assert_eq!(s.stats.media_breakdown.get("pdf"), Some(&0));
        // lang map empty on empty corpus.
        assert!(s.stats.lang_breakdown.is_empty());
        // sqlite bytes positive after migrations, lancedb 0.
        assert!(s.stats.index_bytes.sqlite > 0);
        assert_eq!(s.stats.index_bytes.lancedb, 0);
        assert_eq!(s.stats.stale_doc_count, 0);
    }
}

#[cfg(test)]
mod tests_capabilities {
    use super::*;

    #[test]
    fn capabilities_streaming_ask_matches_cli_surface() {
        // Bug #9: kebab ask --stream 가 answer_event.v1 ndjson 191 event 정상 emit →
        // capabilities.streaming_ask 가 true 여야 함.
        let caps = capabilities_snapshot();
        assert!(caps.streaming_ask, "streaming_ask must be true (Bug #9)");
    }

    #[test]
    fn capabilities_single_file_ingest_matches_cli_surface() {
        // Bug #9: kebab ingest-file <path> + kebab ingest-stdin --title <T> 양쪽 모두
        // ingest_report.v1 정상 emit → capabilities.single_file_ingest 가 true 여야 함.
        let caps = capabilities_snapshot();
        assert!(caps.single_file_ingest, "single_file_ingest must be true (Bug #9)");
    }
}
