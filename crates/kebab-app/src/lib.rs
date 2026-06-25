//! `kb-app` — facade that downstream `kb-cli` / `kb-tui` / `kb-desktop`
//! depend on (§7, §8).
//!
//! P3-5 swapped the `bail!("not yet wired")` stubs for real bodies that
//! compose the libraries shipped through P3-4. After this task, `kb
//! ingest` actually walks a workspace and persists chunks, and `kb
//! search --mode {lexical,vector,hybrid}` returns real `SearchHit`s.
//! `kb-app::ask` stays stubbed (P4-3 owns it).
//!
//! ## Wire-schema convention
//!
//! `kb-app` returns pure domain types (`IngestReport`, `DocSummary`,
//! `Chunk`, `SearchHit`, `Answer`, …) re-exported from `kb-core`. These do
//! NOT carry a `schema_version` field. The CLI (`kb-cli/src/wire.rs`) is
//! responsible for wrapping each Ok-path return value with the matching
//! `*.v1` envelope before emitting JSON on stdout in `--json` mode. The
//! sole exception is [`DoctorReport`], whose `schema_version` is part of
//! the struct because the doctor wire object IS its own structured
//! surface (no domain-side equivalent in `kb-core`). When adding a new
//! facade function in a later phase, remember: keep the return type pure,
//! and add a matching `wire_*` helper in `kb-cli/src/wire.rs`.
//!
//! ## Config seam (`*_with_config`)
//!
//! Each public free function has a `#[doc(hidden)] pub fn *_with_config`
//! companion that takes a fully-resolved [`kebab_config::Config`] directly.
//! Three callers go through it: (1) the top-level free functions
//! themselves, after `load_config()`; (2) `kb-cli` when the user passes
//! `--config <path>` (CLI builds the Config via
//! `Config::load(cli.config.as_deref())` and threads it in directly so
//! the flag is honored); (3) integration tests, which mutate a Config
//! to point at a `TempDir` to avoid polluting the user's real
//! `data_dir` / `model_dir`. `#[doc(hidden)]` keeps rustdoc clean while
//! still allowing the cross-crate calls.

use std::path::PathBuf;

use anyhow::anyhow;
use serde::{Deserialize, Serialize};

use kebab_core::{
    Answer, CanonicalDocument, Chunk, ChunkId, DocFilter, DocSummary, DocumentId, DocumentStore,
    SearchHit, SearchQuery,
};

mod app;
mod bulk;
pub mod cursor;
pub mod derivation_payload;
pub mod doctor_signal;
pub mod error_signal;
pub mod error_wire;
pub mod external;
pub mod fetch;
mod ingest;
pub mod ingest_log;
pub mod ingest_progress;
pub mod logging;
pub mod pdf_ocr_apply;
pub mod reset;
pub mod schema;
mod staleness;

pub use app::{App, SearchResponse};
#[doc(hidden)]
pub use bulk::{BULK_QUERIES_MAX, bulk_search_with_config};
pub use error_wire::{ERROR_V1_ID, ErrorV1, StructuredError, classify};
pub use fetch::fetch_with_config;
pub use ingest::{
    IngestOpts, ingest, ingest_file_with_config, ingest_stdin_with_config, ingest_with_config,
};
#[doc(hidden)]
pub use ingest::{cache_image_caption, test_ingest_config_signature};
pub use ingest_log::{IngestLogWriter, IngestSummary, LogEvent};
pub use ingest_progress::{AggregateCounts, IngestEvent, render_skipped_breakdown};
pub use kebab_config::{ConfigInvalid, ConfigNotFound};
pub use reset::{ResetReport, ResetScope, enumerate_orphans};
pub use schema::{
    Capabilities, Models, SCHEMA_V1_ID, SchemaV1, Stats, WireBlock, schema_with_config,
};
pub use staleness::{compute_stale, mark_stale_in_place};

/// p9-fb-25: sentinel for files without an extension in
/// `IngestReport.skipped_by_extension` keys + `IngestItem.warnings`
/// `unsupported media type: ...` line. Wire schema description
/// references this literal — changing the sentinel is a wire-
/// compatibility break.
pub const NO_EXT_SENTINEL: &str = "<no-ext>";

/// Caller-supplied knobs for one [`ask`] invocation.
///
/// Re-exported from [`kebab_rag::AskOpts`] (P4-3 owns the type) so kb-cli's
/// `use kebab_app::AskOpts` keeps working without churn. The struct gained
/// a `stream_sink` field in P4-3; non-streaming callers (kb-cli today)
/// pass `stream_sink: None`.
pub use kebab_rag::{AskOpts, StreamEvent};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DoctorReport {
    /// Wire schema version label (`"doctor.v1"`).
    pub schema_version: String,
    pub ok: bool,
    pub checks: Vec<DoctorCheck>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DoctorCheck {
    pub name: String,
    pub ok: bool,
    pub detail: String,
    pub hint: Option<String>,
}

/// Create XDG dirs and write a starter `config.toml`. Idempotent unless
/// `force=true` (which overwrites an existing config).
pub fn init_workspace(force: bool) -> anyhow::Result<()> {
    let cfg_path = kebab_config::Config::xdg_config_path();
    let data_dir = kebab_config::Config::xdg_data_dir();
    let cache_dir = kebab_config::Config::xdg_cache_dir();
    let state_dir = kebab_config::Config::xdg_state_dir();

    for d in [
        cfg_path.parent().map(PathBuf::from).unwrap_or_default(),
        data_dir.clone(),
        cache_dir,
        state_dir.clone(),
        state_dir.join("logs"),
    ] {
        if !d.as_os_str().is_empty() {
            std::fs::create_dir_all(&d)?;
        }
    }

    let workspace_root = kebab_config::Config::defaults().resolve_workspace_root();
    std::fs::create_dir_all(&workspace_root)?;

    if !cfg_path.exists() || force {
        // init 과 migrate 가 동일한 "주석 달린 default" 문서를 공유한다
        // (주석 카탈로그·헤더의 단일 원천 = kebab_config::migrate).
        let doc = kebab_config::migrate::annotated_default_document();
        std::fs::write(&cfg_path, doc.to_string())?;
    }

    Ok(())
}

fn expand_tilde(s: &str) -> PathBuf {
    if let Some(rest) = s.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    if s == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    }
    PathBuf::from(s)
}

/// Load the active Config from XDG (or fall back to defaults). Mirrors
/// what `kb-cli` does at the top of every subcommand path; we re-do
/// the load inside each facade entry so callers don't have to thread
/// a Config through.
///
/// Callers that already have a Config in hand (CLI honoring `--config`,
/// integration tests, TUI session) should bypass this and call the
/// matching `*_with_config` helper directly.
pub(crate) fn load_config() -> anyhow::Result<kebab_config::Config> {
    kebab_config::Config::load(None)
}


// ── list_docs / inspect_doc / inspect_chunk ───────────────────────────────

pub fn list_docs(filter: DocFilter) -> anyhow::Result<Vec<DocSummary>> {
    let config = load_config()?;
    list_docs_with_config(config, filter)
}

/// Test-only seam — kb-cli must call the public free function
/// ([`list_docs`]), not this.
#[doc(hidden)]
pub fn list_docs_with_config(
    config: kebab_config::Config,
    filter: DocFilter,
) -> anyhow::Result<Vec<DocSummary>> {
    let app = App::open_with_config(config)?;
    app.sqlite.list_documents(&filter)
}

pub fn inspect_doc(id: &DocumentId) -> anyhow::Result<CanonicalDocument> {
    let config = load_config()?;
    inspect_doc_with_config(config, id)
}

/// Test-only seam — kb-cli must call the public free function
/// ([`inspect_doc`]), not this.
#[doc(hidden)]
pub fn inspect_doc_with_config(
    config: kebab_config::Config,
    id: &DocumentId,
) -> anyhow::Result<CanonicalDocument> {
    let app = App::open_with_config(config)?;
    app.sqlite
        .get_document(id)?
        .ok_or_else(|| anyhow!("document not found: {} (try `kb list docs`)", id.0))
}

pub fn inspect_chunk(id: &ChunkId) -> anyhow::Result<Chunk> {
    let config = load_config()?;
    inspect_chunk_with_config(config, id)
}

/// Test-only seam — kb-cli must call the public free function
/// ([`inspect_chunk`]), not this.
#[doc(hidden)]
pub fn inspect_chunk_with_config(
    config: kebab_config::Config,
    id: &ChunkId,
) -> anyhow::Result<Chunk> {
    let app = App::open_with_config(config)?;
    app.sqlite
        .get_chunk(id)?
        .ok_or_else(|| anyhow!("chunk not found: {} (try `kb inspect doc <id>`)", id.0))
}

// ── search ────────────────────────────────────────────────────────────────

pub fn search(query: SearchQuery) -> anyhow::Result<Vec<SearchHit>> {
    let config = load_config()?;
    search_with_config(config, query)
}

/// Test-only seam — kb-cli must call the public free function
/// ([`search`]), not this. Builds a one-shot `App` and delegates to
/// [`App::search`]; long-lived callers should hold an `App` instance
/// directly to amortize the embedder / vector-store cold start.
#[doc(hidden)]
pub fn search_with_config(
    config: kebab_config::Config,
    query: SearchQuery,
) -> anyhow::Result<Vec<SearchHit>> {
    App::open_with_config(config)?.search(query)
}

/// p9-fb-19: bypass the LRU search cache for one call. Same shape as
/// [`search_with_config`] but routes through [`App::search_uncached`]
/// — used by `kebab search --no-cache`.
#[doc(hidden)]
pub fn search_uncached_with_config(
    config: kebab_config::Config,
    query: SearchQuery,
) -> anyhow::Result<Vec<SearchHit>> {
    App::open_with_config(config)?.search_uncached(query)
}

/// p9-fb-34: budget-aware search free function. Mirrors
/// [`search_with_config`] but threads `SearchOpts` (max_tokens,
/// snippet_chars, cursor) and returns the [`SearchResponse`]
/// pagination wrapper. Tasks 6+8 surface this via CLI / MCP.
#[doc(hidden)]
pub fn search_with_opts_with_config(
    config: kebab_config::Config,
    query: kebab_core::SearchQuery,
    opts: kebab_core::SearchOpts,
) -> anyhow::Result<SearchResponse> {
    App::open_with_config(config)?.search_with_opts(query, opts)
}

// ── ask ──────────────────────────────────────────────────────────────────
//
// P4-3 wires `ask` end-to-end. The retriever is built per `opts.mode`;
// vector / hybrid require an enabled embedding provider (else we surface
// the same "switch to --mode lexical" error as `search`). The LLM is
// always Ollama for now — when we grow a second provider (llama.cpp,
// candle, etc.) this is the place to switch on `config.models.llm.provider`.

pub fn ask(query: &str, opts: AskOpts) -> anyhow::Result<Answer> {
    let config = load_config()?;
    ask_with_config(config, query, opts)
}

/// Test-only seam — kb-cli must call the public free function
/// ([`ask`]), not this. Builds a one-shot `App` and delegates to
/// [`App::ask`].
#[doc(hidden)]
pub fn ask_with_config(
    config: kebab_config::Config,
    query: &str,
    opts: AskOpts,
) -> anyhow::Result<Answer> {
    App::open_with_config(config)?.ask(query, opts)
}

/// Run the doctor checks against the explicit config path the user
/// requested via `--config` (or the XDG default if `None`). The
/// `config_loaded` check reports the actual path probed and the
/// `data_dir_writable` check probes the resolved `storage.data_dir`
/// from that config (so `--config` users see their custom paths
/// reflected in the report rather than the XDG defaults).
pub fn doctor_with_config_path(
    config_path: Option<&std::path::Path>,
) -> anyhow::Result<DoctorReport> {
    tracing::debug!("doctor() invoked");
    let mut checks = Vec::new();

    // Resolve the config path the same way `Config::load` does: explicit
    // override first, else XDG default. Report whichever was probed.
    let cfg_path: PathBuf = match config_path {
        Some(p) => p.to_path_buf(),
        None => kebab_config::Config::xdg_config_path(),
    };
    let (config_ok, config_detail, loaded_cfg) = if cfg_path.exists() {
        match kebab_config::Config::from_file(&cfg_path) {
            Ok(c) => (true, cfg_path.display().to_string(), Some(c)),
            Err(e) => (false, format!("{} ({e})", cfg_path.display()), None),
        }
    } else if config_path.is_some() {
        // Explicit `--config <path>` that doesn't exist is a hard error
        // — defaults would silently mask the user's intent.
        (false, format!("{} (not found)", cfg_path.display()), None)
    } else {
        // No `--config` and no XDG file: defaults are always loadable.
        (true, format!("{} (defaults)", cfg_path.display()), None)
    };
    checks.push(DoctorCheck {
        name: "config_loaded".to_string(),
        ok: config_ok,
        detail: config_detail,
        hint: if config_ok {
            None
        } else if config_path.is_some() {
            Some("--config path does not exist or is malformed".to_string())
        } else {
            Some("run `kb init` to seed config".to_string())
        },
    });

    // data_dir_writable — probe the resolved storage.data_dir from the
    // loaded config when present, else the XDG default. Apply env
    // overrides so KEBAB_STORAGE_DATA_DIR is respected too.
    let data_dir = match loaded_cfg.as_ref() {
        Some(c) => {
            // Re-apply env overrides on top so the same precedence as
            // Config::load is preserved here.
            let env: std::collections::HashMap<String, String> = std::env::vars().collect();
            let merged = c.clone().apply_env(&env);
            expand_tilde(&merged.storage.data_dir)
        }
        None => kebab_config::Config::xdg_data_dir(),
    };
    let writable = (|| -> anyhow::Result<()> {
        std::fs::create_dir_all(&data_dir)?;
        let probe = data_dir.join(".kb-doctor-probe");
        std::fs::write(&probe, b"ok")?;
        std::fs::remove_file(&probe).ok();
        Ok(())
    })();
    let (data_ok, data_detail, data_hint) = match writable {
        Ok(()) => (true, data_dir.display().to_string(), None),
        Err(e) => (
            false,
            format!("{} ({e})", data_dir.display()),
            Some("ensure the configured data_dir is writable".to_string()),
        ),
    };
    checks.push(DoctorCheck {
        name: "data_dir_writable".to_string(),
        ok: data_ok,
        detail: data_detail,
        hint: data_hint,
    });

    // config_migration — 사용자 파일이 새 스키마와 동기인지(dry-run 마이그레이션).
    // 파일이 존재할 때만 점검(없으면 defaults 사용 중이라 마이그레이션 무의미).
    if cfg_path.exists() {
        if let Ok(text) = std::fs::read_to_string(&cfg_path) {
            let outcome = kebab_config::migrate::migrate_document(&text);
            let (mok, detail, hint) = if outcome.changed() {
                let added = outcome
                    .changes
                    .iter()
                    .filter(|c| {
                        matches!(
                            c.kind,
                            kebab_config::migrate::ChangeKind::AddedSection
                                | kebab_config::migrate::ChangeKind::AddedKey
                        )
                    })
                    .count();
                let removed = outcome.changes.len() - added;
                (
                    false,
                    format!(
                        "{} pending changes (added {added}, removed {removed} deprecated)",
                        outcome.changes.len()
                    ),
                    Some("run `kebab config migrate` to update your config.toml".to_string()),
                )
            } else {
                (
                    true,
                    format!("config up to date (schema v{})", outcome.to_schema_version),
                    None,
                )
            };
            checks.push(DoctorCheck {
                name: "config_migration".to_string(),
                ok: mok,
                detail,
                hint,
            });
        }
    }

    let ok = checks.iter().all(|c| c.ok);
    Ok(DoctorReport {
        schema_version: "doctor.v1".to_string(),
        ok,
        checks,
    })
}

/// Run the doctor checks against the XDG-default config. Convenience
/// wrapper that mirrors the historical `kb-app::doctor()` signature
/// for callers that don't honor `--config` (e.g., legacy code paths
/// or smoke harnesses).
pub fn doctor() -> anyhow::Result<DoctorReport> {
    doctor_with_config_path(None)
}

/// `kebab config migrate` 의 결과(wire `config_migration.v1` 소스).
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct ConfigMigrationReport {
    /// 항상 `"config_migration.v1"`.
    pub schema_version: String,
    pub config_path: String,
    pub dry_run: bool,
    pub from_schema_version: u32,
    pub to_schema_version: u32,
    pub changed: bool,
    pub backup_path: Option<String>,
    pub changes: Vec<kebab_config::migrate::MigrationChange>,
}

/// 사용자 config.toml 을 새 스키마로 마이그레이션한다(facade).
/// `config_path` 미지정 시 XDG 기본. `dry_run=true` 면 파일·백업 미변경.
/// 안전: 변경 시 `.bak` 백업 후 tmp 에 쓰고 round-trip 검증 → atomic rename.
pub fn config_migrate_with_config_path(
    config_path: Option<&std::path::Path>,
    dry_run: bool,
) -> anyhow::Result<ConfigMigrationReport> {
    let path: PathBuf = match config_path {
        Some(p) => p.to_path_buf(),
        None => kebab_config::Config::xdg_config_path(),
    };
    if !path.exists() {
        anyhow::bail!(
            "config 파일이 없습니다: {} — 먼저 `kebab init` 을 실행하세요.",
            path.display()
        );
    }
    let text = std::fs::read_to_string(&path)?;
    let outcome = kebab_config::migrate::migrate_document(&text);

    let mut backup_path = None;
    if !dry_run && outcome.changed() {
        let bak = path.with_extension("toml.bak");
        std::fs::copy(&path, &bak)?;
        backup_path = Some(bak.display().to_string());
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, &outcome.new_text)?;
        if kebab_config::Config::from_file(&tmp).is_err() {
            std::fs::remove_file(&tmp).ok();
            anyhow::bail!("마이그레이션 결과가 유효하지 않아 원본을 보존합니다.");
        }
        std::fs::rename(&tmp, &path)?;
    }

    Ok(ConfigMigrationReport {
        schema_version: "config_migration.v1".to_string(),
        config_path: path.display().to_string(),
        dry_run,
        from_schema_version: outcome.from_schema_version,
        to_schema_version: outcome.to_schema_version,
        changed: outcome.changed(),
        backup_path,
        changes: outcome.changes,
    })
}


