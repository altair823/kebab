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
use std::sync::{Arc, Mutex};

use anyhow::{Context, anyhow};
use serde::{Deserialize, Serialize};

use kebab_chunk::{
    CodeCAstV1Chunker, CodeCppAstV1Chunker, CodeGoAstV1Chunker, CodeJavaAstV1Chunker,
    CodeJsAstV1Chunker, CodeKotlinAstV1Chunker, CodePythonAstV1Chunker, CodeRustAstV1Chunker,
    CodeTextParagraphV1Chunker, CodeTsAstV1Chunker, DockerfileFileV1Chunker,
    K8sManifestResourceV1Chunker, ManifestFileV1Chunker, MdHeadingV2Chunker, PdfPageV1Chunker,
};
use kebab_core::{
    Answer, Block, CanonicalDocument, Chunk, ChunkId, ChunkPolicy, Chunker, ChunkerVersion,
    DocFilter, DocSummary, DocumentId, DocumentStore, Embedder, EmbeddingInput, EmbeddingKind,
    ExtractContext, IngestReport, Lang, LanguageModel, MediaType, ParserVersion, RawAsset,
    SearchHit, SearchQuery, SourceScope, SourceType, SourceUri, TrustLevel, VectorRecord,
    VectorStore,
};
use kebab_llm_local::OllamaLanguageModel;
use kebab_parse_image::{
    OLLAMA_VISION_ENGINE, OcrEngine, OllamaVisionOcr, OnnxPaddleOcr, PADDLE_ONNX_ENGINE,
    apply_caption, apply_ocr, engine_version_for_paths,
};
use kebab_parse_md::{BodyHints, build_canonical_document, parse_blocks, parse_frontmatter};
use kebab_source_fs::FsSourceConnector;

mod app;
mod bulk;
pub mod cursor;
pub mod derivation_payload;
pub mod doctor_signal;
pub mod error_signal;
pub mod error_wire;
pub mod external;
pub mod fetch;
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
fn load_config() -> anyhow::Result<kebab_config::Config> {
    kebab_config::Config::load(None)
}

// ── ingest ────────────────────────────────────────────────────────────────

/// Per-call ingest controls. Kept as a struct (vs. a growing positional
/// arg list) so future flags (e.g. `dry_run`, per-asset `concurrency`)
/// land additively without churning every caller. Mirrors the `AskOpts`
/// pattern from p9-fb-15.
///
/// `summary_only` was formerly a positional arg on every ingest entry
/// point; it lives here now (Phase 3 Unit 3.1 collapse).
#[derive(Default)]
pub struct IngestOpts {
    /// Streaming progress sink. `None` suppresses emission entirely.
    pub progress: Option<std::sync::mpsc::Sender<crate::ingest_progress::IngestEvent>>,
    /// Cooperative cancel token. `None` = uncancellable.
    pub cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    /// When `true`, the per-asset early-skip block is bypassed — every
    /// asset is re-parsed / re-chunked / re-embedded as if the DB were
    /// empty. Default `false` preserves the auto-skip path.
    pub force_reingest: bool,
    /// When `true`, only chunk/index metadata is written; embeddings are
    /// skipped. Equivalent to the former positional `summary_only` arg.
    pub summary_only: bool,
}

/// Facade entry point — loads [`kebab_config::Config`] from the XDG
/// default path, then forwards to [`ingest_with_config`].
///
/// Per the facade rule: the bare `ingest` form always re-loads the XDG
/// config. Callers with an explicit config (CLI `--config`, tests, TUI)
/// should call [`ingest_with_config`] directly.
pub fn ingest(scope: SourceScope, opts: IngestOpts) -> anyhow::Result<IngestReport> {
    let config = load_config()?;
    ingest_with_config(config, scope, opts)
}

/// Config-explicit ingest entry point — bypasses [`load_config`] when
/// the caller (kebab-cli with `--config`, integration tests, TUI
/// session) already has a [`kebab_config::Config`] in hand.
///
/// This is the orchestrator: all former intermediate variants
/// (`ingest_with_config_progress`, `ingest_with_config_cancellable`,
/// `ingest_with_config_opts`) are collapsed here. Pass progress /
/// cancel / force_reingest / summary_only through [`IngestOpts`].
///
/// Per design §10 (cancellation contract — unchanged from p9-fb-04):
///
/// - The current in-flight asset finishes (rollback would break
///   idempotent re-run). Subsequent assets are skipped.
/// - Cancellation is a normal exit, not an error — `Result::Err` is
///   reserved for actual failures.
/// - Partial commits in SQLite are kept; the next `kebab ingest` run
///   picks up where this one left off (deterministic asset_id +
///   doc_id recipes).
///
/// CLI's `Ctrl-C` SIGINT handler and TUI's `Esc` / `Ctrl-C` both
/// flip the same `AtomicBool` (via `opts.cancel`).
#[doc(hidden)]
pub fn ingest_with_config(
    config: kebab_config::Config,
    scope: SourceScope,
    opts: IngestOpts,
) -> anyhow::Result<IngestReport> {
    let progress = opts.progress.as_ref();
    let cancelled = || {
        opts.cancel
            .as_ref()
            .is_some_and(|c| c.load(std::sync::atomic::Ordering::Relaxed))
    };
    let force_reingest = opts.force_reingest;
    let started_instant = std::time::Instant::now();

    let app = App::open_with_config(config)?;

    // v0.20.x Hook 1: init per-run log writer (None when disabled or on open failure).
    let log_writer: Option<Arc<Mutex<crate::ingest_log::IngestLogWriter>>> =
        match crate::ingest_log::IngestLogWriter::open(&app.config.logging) {
            Ok(Some(w)) => Some(Arc::new(Mutex::new(w))),
            Ok(None) => None,
            Err(e) => {
                tracing::warn!(
                    target: "kebab-app",
                    error = %e,
                    "ingest_log: failed to open log file; logging disabled for this run"
                );
                None
            }
        };
    let ocr_ms_samples: Arc<Mutex<Vec<u64>>> = Arc::new(Mutex::new(Vec::new()));
    let ocr_pages_cnt: Arc<Mutex<u32>> = Arc::new(Mutex::new(0u32));
    let ocr_failures_cnt: Arc<Mutex<u32>> = Arc::new(Mutex::new(0u32));

    // v0.20.x r2: prune stale pdf_ocr_events rows once per ingest run.
    let _pruned = app
        .sqlite
        .prune_pdf_ocr_events(app.config.logging.retention_days)
        .unwrap_or_else(|e| {
            tracing::warn!(target: "kebab-app", "pdf_ocr_events prune failed: {e}");
            0
        });

    // Walk the workspace. `[[workspace.sources]]`: when the caller did not
    // pin an explicit `scope.root` (the normal `kebab ingest` path), iterate
    // over every configured source — each scanned with its own root + exclude
    // and tagged with its `id` + default trust. When `scope.root` IS pinned
    // (single-file ingest, `--root` override), scan that one root as the
    // implicit `default` source — preserving pre-multi-source behavior.
    crate::ingest_progress::emit(
        progress,
        crate::ingest_progress::IngestEvent::ScanStarted {
            root: scope.root.to_string_lossy().into_owned(),
        },
    );
    let connector =
        FsSourceConnector::new(&app.config).context("kb-app::ingest: build FsSourceConnector")?;

    // Per-source scan plan: (source_id, source_trust, scan_scope).
    let scan_plan: Vec<(String, Option<TrustLevel>, SourceScope)> =
        if scope.root.as_os_str().is_empty() && scope.include.is_empty() {
            app.config
                .resolved_sources()
                .into_iter()
                .map(|s| {
                    let scan_scope = SourceScope {
                        root: s.root,
                        include: scope.include.clone(),
                        exclude: s.exclude,
                    };
                    (s.id, s.trust_level, scan_scope)
                })
                .collect()
        } else {
            // Explicit-root / single-file / include-restricted ingest: one
            // ad-hoc `default` source rooted at the pinned scope.
            vec![(
                kebab_config::DEFAULT_SOURCE_ID.to_string(),
                None,
                scope.clone(),
            )]
        };

    // Accumulate assets across sources + a per-path lookup of which source
    // (id + trust) each asset came from. workspace_path is unique per asset
    // within a scan; on the rare overlap across sources, last-write-wins
    // (sources should not share roots — a config smell, not enforced).
    let mut assets: Vec<RawAsset> = Vec::new();
    let mut source_by_path: std::collections::HashMap<String, (String, Option<TrustLevel>)> =
        std::collections::HashMap::new();
    let mut fs_skips = kebab_source_fs::FsScanSkips::default();
    for (sid, strust, scan_scope) in &scan_plan {
        let (src_assets, src_skips) = connector
            .scan_with_skips(scan_scope)
            .with_context(|| format!("kb-app::ingest: scan source `{sid}`"))?;
        for a in &src_assets {
            source_by_path.insert(a.workspace_path.0.clone(), (sid.clone(), *strust));
        }
        assets.extend(src_assets);
        fs_skips.merge(src_skips);
    }
    crate::ingest_progress::emit(
        progress,
        crate::ingest_progress::IngestEvent::ScanCompleted {
            total: u32::try_from(assets.len()).unwrap_or(u32::MAX),
        },
    );

    // v0.20.x Hook 4: emit skip events from scan into log writer.
    if let Some(ref lw) = log_writer {
        for ev in &fs_skips.events {
            if let Ok(mut w) = lw.lock() {
                let _ = w.write_event(&crate::ingest_log::LogEvent::Skip {
                    ts: crate::ingest_log::now_ts(),
                    doc_path: &ev.doc_path,
                    reason: ev.reason,
                    detail: ev.detail.as_deref(),
                });
            }
        }
    }

    // Embedder + vector store: build once at the top so the cold-start
    // cost is paid once even when the workspace has 1000 markdown files.
    let embedder = app.embedder()?;
    let vector_store = app.vector()?;

    // If both are present, ensure the table exists for the (model, dim)
    // pair so the first per-doc upsert doesn't pay the create-table
    // round-trip.
    if let (Some(emb), Some(vec)) = (embedder.as_ref(), vector_store.as_ref()) {
        let mid = emb.model_id();
        vec.ensure_table(&mid, emb.dimensions())
            .context("kb-app::ingest: ensure Lance table")?;
    }

    let parser_version = ParserVersion(kebab_parse_md::PARSER_VERSION.to_string());
    let chunk_policy = chunk_policy_from_config(&app.config);

    // P6-4: build OCR / caption adapters once per ingest invocation,
    // gated on their respective `enabled` flags. `reqwest::blocking::Client`
    // is internally Arc-shared so reusing one instance across the asset
    // loop is correct and cheap. Construction failure (e.g. invalid
    // endpoint) aborts ingest fail-fast — better than silently disabling
    // OCR/caption mid-run.
    let ocr_engine: Option<Box<dyn OcrEngine>> = if app.config.image_ocr().enabled {
        Some(build_image_ocr_engine(&app.config).context("kb-app::ingest: build image OCR engine")?)
    } else {
        None
    };
    let caption_llm: Option<Box<dyn LanguageModel>> = if app.config.ingest.image.caption.enabled {
        Some(Box::new(OllamaLanguageModel::new(&app.config).context(
            "kb-app::ingest: build OllamaLanguageModel for caption",
        )?))
    } else {
        None
    };
    let image_pipeline = ImagePipeline {
        ocr_engine: ocr_engine.as_deref(),
        caption_llm: caption_llm.as_deref(),
    };

    // p10 / v0.20 sub-item 1: PDF OCR engine eager init (H-5 resolution).
    // image OCR pattern mirror — per-ingest 1회 build, fallible → fail-fast.
    let pdf_ocr_engine: Option<Box<dyn OcrEngine>> =
        if app.config.pdf_ocr().enabled || app.config.pdf_ocr().always_on {
            Some(
                build_pdf_ocr_engine(&app.config)
                    .context("kb-app::ingest: build pdf OCR engine")?,
            )
        } else {
            None
        };

    // Pre-load every existing doc_id so we can label `IngestItem.kind`
    // as `New` vs `Updated` correctly. `list_documents` returns one
    // row per `(workspace_path, asset_id)` — index by the deterministic
    // `doc_id` recipe input so the first ingest of an unseen file is
    // labelled `New`.
    let existing_doc_ids: std::collections::HashSet<String> = app
        .sqlite
        .list_documents(&DocFilter::default())
        .context("kb-app::ingest: list existing documents")?
        .into_iter()
        .map(|d| d.doc_id.0)
        .collect();

    // Dogfood: post-walker sweep to remove stored docs whose source
    // file has been deleted from the filesystem. Must run BEFORE the
    // per-asset loop so the loop's New/Updated labelling is based on
    // the post-purge store state (the purged doc_ids won't be in
    // `existing_doc_ids` above — they were already removed, OR the
    // sweep here removes them before we start counting).
    //
    // Critical design invariant: only purge when the file is TRULY
    // absent from disk. A file that is still on disk but outside the
    // current walker scope (config narrowing / include-glob change) is
    // NOT purged — we leave it in place to protect against accidental
    // data loss via config edits.
    let scanned_paths: std::collections::HashSet<kebab_core::WorkspacePath> =
        assets.iter().map(|a| a.workspace_path.clone()).collect();
    let purged_deleted_files = sweep_deleted_files(
        &app,
        &scanned_paths,
        vector_store.as_ref().map(std::convert::AsRef::as_ref),
    )?;

    let started_at = time::OffsetDateTime::now_utc();

    let mut items: Vec<kebab_core::IngestItem> = Vec::new();
    let mut new_count: u32 = 0;
    let mut updated_count: u32 = 0;
    let mut skipped_count: u32 = 0;
    let mut unchanged_count: u32 = 0;
    let mut error_count: u32 = 0;
    // Aggregate counts surfaced into `ingest_runs` (and tracing). Not
    // exposed on `IngestReport` today — `kebab_core::IngestReport` is a
    // wire-stable struct without these fields — but persisting them
    // means audit tooling and `kb jobs` (P+) can recover the totals
    // without re-walking the DB.
    let mut chunks_indexed: u32 = 0;
    let mut embeddings_indexed: u32 = 0;
    // p9-fb-25: per-extension skip count, populated in the Skipped arm below.
    let mut skipped_by_extension: std::collections::BTreeMap<String, u32> =
        std::collections::BTreeMap::new();
    let scanned_count: u32 = u32::try_from(assets.len()).unwrap_or(u32::MAX);

    let embed_active = embedder.is_some() && vector_store.is_some();

    // p9-fb-04: track whether the loop exited via cancellation (vs
    // running to completion) so we can emit `Aborted` rather than
    // `Completed` and surface the right summary.
    let mut was_cancelled = false;

    for (zero_idx, asset) in assets.into_iter().enumerate() {
        // Step boundary check (p9-fb-04). Designed §10 invariant: the
        // current in-flight asset finishes (idempotent re-run guard);
        // subsequent assets are skipped. Check here is the cheapest
        // possible — atomic load each iteration, no lock.
        if cancelled() {
            was_cancelled = true;
            break;
        }
        let idx = u32::try_from(zero_idx + 1).unwrap_or(u32::MAX);
        crate::ingest_progress::emit(
            progress,
            crate::ingest_progress::IngestEvent::AssetStarted {
                idx,
                total: scanned_count,
                path: asset.workspace_path.0.clone(),
                media: crate::ingest_progress::media_label(&asset.media_type).to_string(),
            },
        );
        // `[[workspace.sources]]`: resolve which source this asset came from.
        // Missing only if an asset slipped in outside the scan plan (defensive
        // — fall back to the implicit `default` source).
        let (source_id, source_trust) = source_by_path
            .get(&asset.workspace_path.0)
            .map_or((kebab_config::DEFAULT_SOURCE_ID, None), |(id, trust)| {
                (id.as_str(), *trust)
            });
        let item = ingest_one_asset(
            &app,
            &asset,
            idx,
            scanned_count,
            &parser_version,
            &chunk_policy,
            embedder.as_ref(),
            vector_store.as_ref(),
            &existing_doc_ids,
            source_id,
            source_trust,
            &image_pipeline,
            force_reingest,
            pdf_ocr_engine.as_deref(),
            progress,
            opts.cancel.as_ref(),
            log_writer.clone(),
            ocr_ms_samples.clone(),
            ocr_pages_cnt.clone(),
            ocr_failures_cnt.clone(),
        );

        let item = match item {
            Ok(i) => i,
            Err(e) => {
                tracing::error!(
                    target: "kebab-app",
                    path = %asset.workspace_path.0,
                    error = %e,
                    "kb-app::ingest: per-file fatal"
                );
                // v0.20.x Hook 3: write per-asset error to log writer.
                if let Some(ref lw) = log_writer {
                    if let Ok(mut w) = lw.lock() {
                        let _ = w.write_event(&crate::ingest_log::LogEvent::Error {
                            ts: crate::ingest_log::now_ts(),
                            code: "ingest_asset_error",
                            message: &format!("{e:#}"),
                        });
                    }
                }
                // Note: `error_count += 1` happens below in the
                // `match item.kind { Error => ... }` arm — incrementing
                // here too would double-count (a regression first
                // surfaced by P6-4 image dispatch where Err returns
                // are common; markdown rarely propagated Err so the
                // bug went unnoticed).
                kebab_core::IngestItem {
                    kind: kebab_core::IngestItemKind::Error,
                    doc_id: None,
                    doc_path: asset.workspace_path.clone(),
                    asset_id: Some(asset.asset_id.clone()),
                    byte_len: Some(asset.byte_len),
                    block_count: None,
                    chunk_count: None,
                    parser_version: None,
                    chunker_version: None,
                    warnings: Vec::new(),
                    pdf_ocr_pages: None,
                    pdf_ocr_ms_total: None,
                    error: Some(format!("{e:#}")),
                }
            }
        };

        match item.kind {
            kebab_core::IngestItemKind::New => {
                new_count = new_count.saturating_add(1);
                let n = item.chunk_count.unwrap_or(0);
                chunks_indexed = chunks_indexed.saturating_add(n);
                if embed_active {
                    embeddings_indexed = embeddings_indexed.saturating_add(n);
                }
            }
            kebab_core::IngestItemKind::Updated => {
                updated_count = updated_count.saturating_add(1);
                let n = item.chunk_count.unwrap_or(0);
                chunks_indexed = chunks_indexed.saturating_add(n);
                if embed_active {
                    embeddings_indexed = embeddings_indexed.saturating_add(n);
                }
            }
            kebab_core::IngestItemKind::Skipped => {
                skipped_count = skipped_count.saturating_add(1);
                let ext = ext_for_skip_warning(&item.doc_path.0);
                *skipped_by_extension.entry(ext).or_insert(0) += 1;
            }
            kebab_core::IngestItemKind::Unchanged => {
                unchanged_count = unchanged_count.saturating_add(1);
            }
            kebab_core::IngestItemKind::Error => {
                error_count = error_count.saturating_add(1);
            }
        }
        crate::ingest_progress::emit(
            progress,
            crate::ingest_progress::IngestEvent::AssetFinished {
                idx,
                total: scanned_count,
                result: item.kind,
                chunks: item.chunk_count.unwrap_or(0),
            },
        );
        items.push(item);
    }

    // Record a row in `jobs` so `kb jobs` (P+) can list the run. Distinct
    // from the `ingest_runs` row written below — the `jobs` table is the
    // generic job-lifecycle surface (`kind=ingest`), `ingest_runs` is the
    // ingest-specific aggregate counts row.
    let payload = serde_json::json!({
        "scope": scope,
        "summary_only": opts.summary_only,
    });
    let job_id_res = <SqliteStoreAlias as kebab_core::JobRepo>::create(
        &app.sqlite,
        kebab_core::JobKind::Ingest,
        payload,
    );
    match job_id_res {
        Ok(jid) => {
            // Stash the aggregate counts as the job's `progress_json`
            // so a future `kb jobs show` can surface them without
            // joining `ingest_runs`.
            let progress = serde_json::json!({
                "scanned": scanned_count,
                "new": new_count,
                "updated": updated_count,
                "skipped": skipped_count,
                "errors": error_count,
                "chunks_indexed": chunks_indexed,
                "embeddings_indexed": embeddings_indexed,
            });
            if let Err(e) = <SqliteStoreAlias as kebab_core::JobRepo>::update_progress(
                &app.sqlite,
                &jid,
                progress,
            ) {
                tracing::warn!(
                    target: "kebab-app",
                    error = %e,
                    "kb-app::ingest: JobRepo::update_progress failed"
                );
            }
            if let Err(e) = <SqliteStoreAlias as kebab_core::JobRepo>::finish(
                &app.sqlite,
                &jid,
                kebab_core::JobStatus::Succeeded,
                None,
            ) {
                tracing::warn!(
                    target: "kebab-app",
                    error = %e,
                    "kb-app::ingest: JobRepo::finish failed"
                );
            }
        }
        Err(e) => {
            tracing::warn!(
                target: "kebab-app",
                error = %e,
                "kb-app::ingest: JobRepo::create failed; run not recorded in `jobs`"
            );
        }
    }

    let duration_ms = u32::try_from(started_instant.elapsed().as_millis()).unwrap_or(u32::MAX);
    let finished_at = time::OffsetDateTime::now_utc();

    // Record the ingest_runs row with aggregate counts.
    // `summary_only=true` writes `items_json=NULL` (per design §5.7);
    // the count columns are populated either way.
    let scope_json = serde_json::to_string(&scope)
        .context("kb-app::ingest: serialize scope for ingest_runs.scope_json")?;
    let items_json: Option<String> = if opts.summary_only {
        None
    } else {
        match serde_json::to_string(&items) {
            Ok(s) => Some(s),
            Err(e) => {
                tracing::warn!(
                    target: "kebab-app",
                    error = %e,
                    "kb-app::ingest: failed to serialize items_json; storing NULL"
                );
                None
            }
        }
    };
    let run_id = mint_ingest_run_id(&scope_json, started_at);
    let row = kebab_store_sqlite::IngestRunRow {
        run_id: &run_id,
        scope_json: &scope_json,
        scanned: scanned_count,
        new_count,
        updated_count,
        skipped_count,
        error_count,
        duration_ms,
        started_at,
        finished_at,
        items_json: items_json.as_deref(),
    };
    if let Err(e) = app.sqlite.record_ingest_run(&row) {
        tracing::warn!(
            target: "kebab-app",
            error = %e,
            "kb-app::ingest: record_ingest_run failed"
        );
    }

    tracing::info!(
        target: "kebab-app",
        scanned = scanned_count,
        new = new_count,
        updated = updated_count,
        skipped = skipped_count,
        errors = error_count,
        chunks_indexed,
        embeddings_indexed,
        duration_ms,
        "kb-app::ingest: run complete"
    );

    let final_counts = crate::ingest_progress::AggregateCounts {
        scanned: scanned_count,
        new: new_count,
        updated: updated_count,
        skipped: skipped_count,
        unchanged: unchanged_count,
        errors: error_count,
        chunks_indexed,
        embeddings_indexed,
        skipped_by_extension: skipped_by_extension.clone(),
    };
    let terminal_event = if was_cancelled {
        crate::ingest_progress::IngestEvent::Aborted {
            counts: final_counts,
        }
    } else {
        crate::ingest_progress::IngestEvent::Completed {
            counts: final_counts,
        }
    };
    crate::ingest_progress::emit(progress, terminal_event);

    // p9-fb-19: bump the persistent corpus_revision counter when a
    // commit landed (any new / updated / purged). This invalidates every
    // entry in any in-process LRU search cache (in this process or
    // a sibling) on the next lookup. No-op when nothing changed
    // (skipped-only run) — the cache stays valid.
    if new_count > 0 || updated_count > 0 || purged_deleted_files > 0 {
        match app.sqlite.bump_corpus_revision() {
            Ok(rev) => tracing::debug!(
                target: "kebab-app",
                corpus_revision = rev,
                "bumped corpus_revision after ingest commit"
            ),
            Err(e) => tracing::warn!(
                target: "kebab-app",
                error = %e,
                "bump_corpus_revision failed; cache may serve stale results until process restart"
            ),
        }
    }

    // v0.20.x Hook 1 exit: write summary record + flush log writer.
    if let Some(ref lw) = log_writer {
        if let Ok(mut w) = lw.lock() {
            let run_id = w.run_id().to_string();
            let ms_samples = ocr_ms_samples.lock().map(|v| v.clone()).unwrap_or_default();
            let pages = ocr_pages_cnt.lock().map_or(0, |v| *v);
            let failures = ocr_failures_cnt.lock().map_or(0, |v| *v);
            let summary = crate::ingest_log::IngestSummary::new(
                crate::ingest_log::now_ts(),
                run_id,
                scanned_count,
                new_count,
                error_count,
                pages,
                failures,
                &ms_samples,
                started_instant.elapsed().as_millis() as u64,
            );
            let _ = w.write_summary(&summary);
            let _ = w.flush();
        }
    }

    Ok(IngestReport {
        scope,
        scanned: scanned_count,
        new: new_count,
        updated: updated_count,
        skipped: skipped_count,
        unchanged: unchanged_count,
        errors: error_count,
        duration_ms,
        skipped_by_extension,
        skipped_gitignore: fs_skips.skipped_gitignore,
        skipped_kebabignore: fs_skips.skipped_kebabignore,
        skipped_builtin_blacklist: fs_skips.skipped_builtin_blacklist,
        skipped_generated: fs_skips.skipped_generated,
        skipped_size_exceeded: fs_skips.skipped_size_exceeded,
        skip_examples: fs_skips.skip_examples,
        purged_deleted_files,
        items: if opts.summary_only { None } else { Some(items) },
    })
}

/// Mint a stable 32-hex-char `run_id` for an `ingest_runs` row.
/// `(scope, started_at_nanos)` is enough to make two runs with the
/// same scope started a nanosecond apart distinguish — same shape as
/// the JobId recipe in `kb-store-sqlite::jobs`.
fn mint_ingest_run_id(scope_json: &str, at: time::OffsetDateTime) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(scope_json.as_bytes());
    hasher.update(&at.unix_timestamp_nanos().to_be_bytes());
    let hex = hasher.finalize().to_hex().to_string();
    hex[..32].to_string()
}

/// Trait alias type used to disambiguate the two impls (`DocumentStore`
/// vs `JobRepo`) on the same store. Plain `app.sqlite.create(...)`
/// would pick one based on inherent vs trait methods; we go through
/// `<… as JobRepo>` to be explicit.
type SqliteStoreAlias = kebab_store_sqlite::SqliteStore;

/// v0.27.0 (T8): build the image OCR engine selected by
/// `config.ingest.image.ocr.engine`. Returns a boxed trait object so the ingest
/// pipeline is engine-agnostic. Construction is fail-fast (model load /
/// hash / endpoint validation) — mirrors the prior concrete-type behaviour.
///
/// `--config` facade: the caller threads the explicit [`kebab_config::Config`]
/// in, so `OnnxPaddleOcr::new` honours `image.ocr.{det_model,rec_model,dict,…}`
/// overrides resolved from that config (not a re-loaded XDG default).
fn build_image_ocr_engine(
    config: &kebab_config::Config,
) -> anyhow::Result<Box<dyn OcrEngine>> {
    match config.image_ocr().engine.as_str() {
        OLLAMA_VISION_ENGINE => Ok(Box::new(
            OllamaVisionOcr::new(config).context("build OllamaVisionOcr")?,
        )),
        PADDLE_ONNX_ENGINE => Ok(Box::new(
            OnnxPaddleOcr::new(config).context("build OnnxPaddleOcr")?,
        )),
        other => anyhow::bail!(
            "unknown image.ocr.engine {other:?}; expected \
             {OLLAMA_VISION_ENGINE:?} or {PADDLE_ONNX_ENGINE:?}"
        ),
    }
}

/// v0.27.0 (T8): build the PDF OCR engine selected by `pdf.ocr.engine`. The
/// ollama-vision arm uses the resolved PDF OCR knobs (`model` / `languages` /
/// `max_pixels` / `request_timeout_secs`, endpoint fallback to
/// `models.llm.endpoint`) from [`Config::pdf_ocr`].
///
/// # Paddle-ONNX assets (v5)
///
/// The paddle-onnx arm still builds via `OnnxPaddleOcr::new(config)`, which
/// resolves its ONNX asset paths from the image OCR block
/// ([`Config::image_ocr`]). After the v5 `[ingest.ocr]` consolidation both
/// mediums inherit the same shared engine defaults, so image and PDF paddle
/// resolve to one identical set of tuned ONNX knobs — the historical
/// "PDF borrows image's paddle assets" behaviour, now expressed as a single
/// shared block rather than a cross-medium read.
fn build_pdf_ocr_engine(
    config: &kebab_config::Config,
) -> anyhow::Result<Box<dyn OcrEngine>> {
    match config.pdf_ocr().engine.as_str() {
        OLLAMA_VISION_ENGINE => {
            let cfg = config.pdf_ocr();
            let endpoint = match cfg.endpoint.as_deref() {
                Some(s) if !s.is_empty() => s.to_string(),
                _ => config.models.llm.endpoint.clone(),
            };
            Ok(Box::new(
                OllamaVisionOcr::from_parts(
                    endpoint,
                    cfg.model.clone(),
                    cfg.languages.clone(),
                    cfg.max_pixels,
                    cfg.request_timeout_secs,
                )
                .context("build OllamaVisionOcr (pdf)")?,
            ))
        }
        PADDLE_ONNX_ENGINE => Ok(Box::new(
            OnnxPaddleOcr::new(config).context("build OnnxPaddleOcr (pdf)")?,
        )),
        other => anyhow::bail!(
            "unknown pdf.ocr.engine {other:?}; expected \
             {OLLAMA_VISION_ENGINE:?} or {PADDLE_ONNX_ENGINE:?}"
        ),
    }
}

/// P6-4: borrowed bundle of the three image-pipeline components built
/// once per ingest invocation. Threaded through `ingest_one_asset` so
/// the dispatch does not need ten separate parameters.
struct ImagePipeline<'a> {
    ocr_engine: Option<&'a dyn OcrEngine>,
    caption_llm: Option<&'a dyn LanguageModel>,
}

/// p9-fb-23 task 7: incremental-ingest early-skip predicate. Shared
/// across the markdown / image / PDF per-asset flows. Returns
/// `Some(IngestItem { kind: Unchanged, .. })` when ALL FOUR conditions
/// hold (per design §9 cascade rule):
///
/// 1. `force_reingest == false` — caller hasn't asked to bypass skip.
/// 2. A document already exists at this `workspace_path`
///    (`get_document_by_workspace_path`). The lookup is document-side, not
///    asset-side, so twin files (identical content at different paths) each
///    hit their own stable doc row — `documents.workspace_path` is UNIQUE
///    while `assets` may dedupe content into a single row with a flip-flop
///    `workspace_path` column (dogfood bug #4, see `tasks/HOTFIXES.md`).
/// 3. The existing doc's `source_asset_id` equals the freshly-scanned
///    asset's blake3 checksum (content unchanged).
/// 4. The existing doc's `parser_version` matches the current extractor's
///    `parser_version` (extractor not upgraded). Combined with `chunker_version`
///    and `last_embedding_version` checks immediately below — full cascade
///    per design §9.
///
/// Returns `Ok(None)` (proceed with full re-process) when any check
/// fails or any DB read errors out — the skip path is opportunistic;
/// a missed skip is correct (just slower), a wrong skip would corrupt
/// the index.
fn try_skip_unchanged(
    app: &App,
    asset: &RawAsset,
    current_parser_version: &ParserVersion,
    current_chunker_version: &ChunkerVersion,
    current_embedding_version: Option<&kebab_core::EmbeddingVersion>,
    force_reingest: bool,
    fallback_chunker_version: Option<&ChunkerVersion>, // p10-3 fix
) -> anyhow::Result<Option<kebab_core::IngestItem>> {
    if force_reingest {
        return Ok(None);
    }
    // Document-centric skip: look up the existing document row by
    // workspace_path directly. This avoids the twin-file flip-flop
    // that the old asset-side lookup suffers from — multiple files
    // with identical content share one `assets` row whose
    // `workspace_path` is overwritten on every UPSERT, so
    // `get_asset_by_workspace_path(path1)` could return the OTHER
    // twin's path (or None) after any ingest of the twin. The
    // `documents` table has a UNIQUE index on `workspace_path` (V001),
    // so each twin has its own stable row regardless of asset de-dup.
    let existing_doc = match app
        .sqlite
        .get_document_by_workspace_path(&asset.workspace_path)
    {
        Ok(Some(d)) => d,
        Ok(None) => return Ok(None),
        Err(e) => {
            tracing::debug!(
                target: "kebab-app",
                path = %asset.workspace_path.0,
                error = %e,
                "skip-check: get_document_by_workspace_path failed; falling through to re-process"
            );
            return Ok(None);
        }
    };
    // 1. Content unchanged: the freshly-computed asset_id (blake3
    //    content hash) must match what this document was ingested from.
    if existing_doc.source_asset_id != asset.asset_id {
        return Ok(None);
    }
    // p10-3 fix: detect "stored doc was previously Tier 3 fallback".
    // When a Tier 1/2 extractor emits empty chunks, the fallback wrapper
    // retries with CodeTextParagraphV1Chunker and stores
    // last_chunker_version = "code-text-paragraph-v1" + parser_version = "none-v1".
    // On the next ingest the caller computes current_parser_version /
    // current_chunker_version from the Tier 1/2 dispatch (e.g.
    // "k8s-manifest-resource-v1"), which can never match the stored
    // fallback values, causing spurious re-ingests. Detect this case
    // and bypass the parser/chunker equality checks — only the embedder
    // version still must match.
    let stored_is_tier3_fallback = fallback_chunker_version.is_some_and(|fbv| {
        existing_doc.last_chunker_version.as_ref() == Some(fbv)
            && existing_doc.parser_version.0 == "none-v1"
    });

    if stored_is_tier3_fallback {
        // Embedder version still must match.
        let embedder_match =
            existing_doc.last_embedding_version.as_ref() == current_embedding_version;
        if !embedder_match {
            return Ok(None);
        }
        let candidate_doc_id = existing_doc.doc_id.clone();
        tracing::debug!(
            target: "kebab-app::ingest",
            path = %asset.workspace_path.0,
            doc_id = %candidate_doc_id.0,
            "skip-unchanged: tier 3 fallback state detected; bypassing parser/chunker equality"
        );
        return Ok(Some(kebab_core::IngestItem {
            kind: kebab_core::IngestItemKind::Unchanged,
            doc_id: Some(candidate_doc_id),
            doc_path: asset.workspace_path.clone(),
            asset_id: Some(asset.asset_id.clone()),
            byte_len: Some(asset.byte_len),
            block_count: u32::try_from(existing_doc.blocks.len()).ok(),
            chunk_count: None,
            parser_version: Some(existing_doc.parser_version.clone()),
            chunker_version: existing_doc.last_chunker_version.clone(),
            warnings: Vec::new(),
            pdf_ocr_pages: None,
            pdf_ocr_ms_total: None,
            error: None,
        }));
    }

    // 2. Parser unchanged: parser_version is baked into id_for_doc so
    //    a version bump yields a different doc_id and the row above
    //    would have been missing. Checking here explicitly keeps the
    //    logic self-documenting and guards against future id_for_doc
    //    changes.
    if existing_doc.parser_version != *current_parser_version {
        // v0.17.0 PR-B: parser_version bump cascade. Same bytes (same
        // asset_id) → asset-keyed `stale_chunk_ids_at` is a no-op, but
        // the stale `documents` row at this workspace_path still
        // collides with `idx_docs_workspace_path` on the next INSERT
        // and the LanceDB rows under the old chunk_ids orphan. Sweep
        // both stores here, before returning Ok(None), so the caller's
        // full-ingest path lands a clean slate. The `keep_doc_id = ""`
        // sentinel removes every doc at this path (the new doc_id is
        // not yet known here — it's computed downstream from the new
        // PARSER_VERSION).
        purge_workspace_path_for_parser_bump(app, asset)
            .with_context(|| format!("parser-bump orphan purge at {}", asset.workspace_path.0))?;
        return Ok(None);
    }
    // 3. Chunker unchanged.
    let chunker_match = existing_doc.last_chunker_version.as_ref() == Some(current_chunker_version);
    if !chunker_match {
        return Ok(None);
    }
    // 4. Embedder unchanged.
    let embedder_match = existing_doc.last_embedding_version.as_ref() == current_embedding_version;
    if !embedder_match {
        return Ok(None);
    }
    let candidate_doc_id = existing_doc.doc_id.clone();
    tracing::debug!(
        target: "kebab-app::ingest",
        path = %asset.workspace_path.0,
        doc_id = %candidate_doc_id.0,
        "skip-unchanged: checksum + parser/chunker/embedding versions match"
    );
    Ok(Some(kebab_core::IngestItem {
        kind: kebab_core::IngestItemKind::Unchanged,
        doc_id: Some(candidate_doc_id),
        doc_path: asset.workspace_path.clone(),
        asset_id: Some(asset.asset_id.clone()),
        byte_len: Some(asset.byte_len),
        block_count: u32::try_from(existing_doc.blocks.len()).ok(),
        chunk_count: None,
        parser_version: Some(existing_doc.parser_version.clone()),
        chunker_version: existing_doc.last_chunker_version.clone(),
        warnings: Vec::new(),
        pdf_ocr_pages: None,
        pdf_ocr_ms_total: None,
        error: None,
    }))
}

/// p9-fb-25: extract the lowercase extension (no leading dot) from a
/// workspace path for use in the `unsupported media type: .X` warning
/// and `IngestReport.skipped_by_extension` key. Returns [`NO_EXT_SENTINEL`]
/// for paths with no extension. Always lowercase so `Foo.DOCX` and
/// `bar.docx` aggregate under the same key.
fn ext_for_skip_warning(path: &str) -> String {
    std::path::Path::new(path)
        .extension()
        .and_then(|s| s.to_str())
        .map_or_else(|| NO_EXT_SENTINEL.to_string(), str::to_ascii_lowercase)
}

/// p9-fb-25: render the `IngestItem.warnings` line for a Skipped
/// asset. [`NO_EXT_SENTINEL`] renders without a leading dot;
/// everything else gets `.ext` form.
fn unsupported_media_warning(path: &str) -> String {
    let ext = ext_for_skip_warning(path);
    if ext == NO_EXT_SENTINEL {
        format!("unsupported media type: {NO_EXT_SENTINEL}")
    } else {
        format!("unsupported media type: .{ext}")
    }
}

/// Embed `texts` with the derivation cache (design 2026-05-31 §3.4).
///
/// 1) 각 text 의 embedding cache_key 계산 → 히트/미스 분리.
/// 2) 미스 text 만 `emb.embed`(축소 배치) 호출.
/// 3) 미스 결과를 `Vec<f32>` little-endian 으로 캐시 put.
/// 4) 히트(bytes→Vec<f32>) + 미스 벡터를 **원래 순서대로** 합쳐 반환.
///
/// 손상된 payload(길이 misalign)는 미스로 강등 → 재계산(정확성 우선, §3.5).
/// 히트 키는 `touch_keys` 에 누적(호출측이 배치로 last_used_at 갱신).
fn embed_with_cache(
    emb: &dyn Embedder,
    sqlite: &kebab_store_sqlite::SqliteStore,
    texts: &[&str],
    version_key: &str,
    hit: &mut usize,
    miss: &mut usize,
    touch_keys: &mut Vec<String>,
) -> anyhow::Result<Vec<Vec<f32>>> {
    let mut out: Vec<Option<Vec<f32>>> = Vec::with_capacity(texts.len());
    let mut miss_indices: Vec<usize> = Vec::new();
    let mut miss_inputs: Vec<EmbeddingInput<'_>> = Vec::new();
    let mut keys: Vec<String> = Vec::with_capacity(texts.len());

    for (i, text) in texts.iter().enumerate() {
        let key = kebab_core::derivation_cache_key("embedding", text, version_key);
        // 히트 = 캐시에 있고 payload 가 정상 디코드되는 경우. 손상 payload 는
        // 미스로 강등(재계산, 정확성 우선 §3.5).
        let cached = sqlite
            .derivation_cache_get(&key)?
            .and_then(|p| crate::derivation_payload::decode_embedding(&p));
        if let Some(v) = cached {
            *hit += 1;
            touch_keys.push(key.clone());
            out.push(Some(v));
        } else {
            *miss += 1;
            miss_indices.push(i);
            miss_inputs.push(EmbeddingInput {
                text,
                kind: EmbeddingKind::Document,
            });
            out.push(None);
        }
        keys.push(key);
    }

    if !miss_inputs.is_empty() {
        let miss_vectors = emb.embed(&miss_inputs)?;
        for (slot, v) in miss_indices.iter().zip(miss_vectors) {
            sqlite.derivation_cache_put(
                &keys[*slot],
                "embedding",
                &crate::derivation_payload::encode_embedding(&v),
            )?;
            out[*slot] = Some(v);
        }
    }

    Ok(out
        .into_iter()
        .map(|v| v.expect("every slot filled by hit or miss"))
        .collect())
}

/// Process a single asset: read bytes, parse, normalize, chunk,
/// persist, embed. Per-asset failures bubble up to the caller for
/// labelling as `IngestItemKind::Error` — they do NOT abort the
/// whole run.
#[allow(clippy::too_many_arguments)]
fn ingest_one_asset(
    app: &App,
    asset: &RawAsset,
    idx: u32,
    total: u32,
    parser_version: &ParserVersion,
    chunk_policy: &ChunkPolicy,
    embedder: Option<&Arc<dyn Embedder + Send + Sync>>,
    vector_store: Option<&Arc<kebab_store_vector::LanceVectorStore>>,
    existing_doc_ids: &std::collections::HashSet<String>,
    // `[[workspace.sources]]`: id of the source this asset belongs to (stamped
    // onto `documents.source_id`) + that source's default trust level
    // (markdown frontmatter overrides it).
    source_id: &str,
    source_trust: Option<TrustLevel>,
    image_pipeline: &ImagePipeline<'_>,
    force_reingest: bool,
    pdf_ocr_engine: Option<&dyn OcrEngine>,
    progress: Option<&std::sync::mpsc::Sender<crate::ingest_progress::IngestEvent>>,
    cancel: Option<&std::sync::Arc<std::sync::atomic::AtomicBool>>,
    log_writer: Option<Arc<Mutex<crate::ingest_log::IngestLogWriter>>>,
    ocr_ms_samples: Arc<Mutex<Vec<u64>>>,
    ocr_pages_cnt: Arc<Mutex<u32>>,
    ocr_failures_cnt: Arc<Mutex<u32>>,
) -> anyhow::Result<kebab_core::IngestItem> {
    tracing::debug!(
        target: "kebab-app::ingest",
        path = %asset.workspace_path.0,
        media_type = ?asset.media_type,
        "processing asset"
    );
    // P6-4: dispatch on media_type. Markdown takes the existing
    // parse-md / normalize path; image takes the new
    // ImageExtractor + (optional) OCR + (optional) caption path.
    // Anything else (PDF, audio, unknown) is skipped — the
    // respective phases (P7 / P8) wire them in later.
    match &asset.media_type {
        MediaType::Markdown => { /* fall through to markdown path */ }
        MediaType::Image(_) => {
            return ingest_one_image_asset(
                app,
                asset,
                idx,
                total,
                chunk_policy,
                embedder,
                vector_store,
                existing_doc_ids,
                source_id,
                image_pipeline,
                force_reingest,
                progress,
            );
        }
        MediaType::Pdf => {
            return ingest_one_pdf_asset(
                app,
                asset,
                idx,
                total,
                chunk_policy,
                embedder,
                vector_store,
                existing_doc_ids,
                source_id,
                force_reingest,
                pdf_ocr_engine,
                progress,
                cancel,
                log_writer,
                ocr_ms_samples,
                ocr_pages_cnt,
                ocr_failures_cnt,
            );
        }
        // p10-1A-2 / 1B: code ingest dispatch. p10-2: Tier 2 langs added. p10-3: shell added. p10-1D: c/cpp added.
        MediaType::Code(lang)
            if matches!(
                lang.as_str(),
                "rust"
                    | "python"
                    | "typescript"
                    | "javascript"
                    | "go"
                    | "java"
                    | "kotlin"
                    | "yaml"
                    | "dockerfile"
                    | "toml"
                    | "json"
                    | "xml"
                    | "groovy"
                    | "go-mod"
                    | "shell"
                    | "c"
                    | "cpp"
            ) =>
        {
            return ingest_one_code_asset(
                app,
                asset,
                chunk_policy,
                embedder,
                vector_store,
                existing_doc_ids,
                force_reingest,
                lang.as_str(),
                source_id,
            );
        }
        // p10-1A-2: non-Rust Code, Audio, and Other are not yet wired;
        // skip until their respective phases.
        MediaType::Code(_) | MediaType::Audio(_) | MediaType::Other(_) => {
            return Ok(kebab_core::IngestItem {
                kind: kebab_core::IngestItemKind::Skipped,
                doc_id: None,
                doc_path: asset.workspace_path.clone(),
                asset_id: Some(asset.asset_id.clone()),
                byte_len: Some(asset.byte_len),
                block_count: None,
                chunk_count: None,
                parser_version: None,
                chunker_version: None,
                warnings: vec![unsupported_media_warning(&asset.workspace_path.0)],
                pdf_ocr_pages: None,
                pdf_ocr_ms_total: None,
                error: None,
            });
        }
    }

    let path = match &asset.source_uri {
        SourceUri::File(p) => p.clone(),
        SourceUri::Kb(_) => {
            return Ok(kebab_core::IngestItem {
                kind: kebab_core::IngestItemKind::Skipped,
                doc_id: None,
                doc_path: asset.workspace_path.clone(),
                asset_id: Some(asset.asset_id.clone()),
                byte_len: Some(asset.byte_len),
                block_count: None,
                chunk_count: None,
                parser_version: None,
                chunker_version: None,
                warnings: vec!["kb:// URI not yet supported".to_string()],
                pdf_ocr_pages: None,
                pdf_ocr_ms_total: None,
                error: None,
            });
        }
    };

    // v0.26.2: fold the ingest-config signature into the effective
    // parser_version for the skip compare + the stored doc field, so a
    // change to any markdown-affecting setting (chunking params) re-indexes.
    // `doc_id` keeps deriving from the base version below (stability).
    let eff_parser_version = effective_parser_version(&app.config, asset, parser_version);

    // p9-fb-23 task 7: incremental-ingest early-skip. When force_reingest
    // is false AND the on-disk asset's checksum + parser_version +
    // last_chunker_version + last_embedding_version all match the existing
    // DB record, this asset doesn't need to be re-parsed / re-chunked /
    // re-embedded. Return Unchanged so the caller bumps `aggregate.unchanged`
    // and the AssetFinished progress event reflects the skip.
    if let Some(item) = try_skip_unchanged(
        app,
        asset,
        &eff_parser_version,
        &md_chunker_from_config(&app.config).chunker_version(),
        embedder.map(|e| e.model_version()).as_ref(),
        force_reingest,
        None,
    )? {
        return Ok(item);
    }

    // v0.24.0 phase timing: parse spans from here (byte read) through
    // `build_canonical_document`, i.e. everything before the chunker runs.
    let t_parse = std::time::Instant::now();

    let bytes = std::fs::read(&path)
        .with_context(|| format!("read asset bytes from {}", path.display()))?;

    let body_hints = build_body_hints(asset, Some(source_id), source_trust);

    // Frontmatter — `parse_frontmatter` returns Ok even on malformed
    // frontmatter (warnings are surfaced through the `Vec<Warning>`).
    let (metadata, fm_span, fm_warns) =
        parse_frontmatter(&bytes, &body_hints).context("kb-parse-md::parse_frontmatter")?;

    let body_offset_lines = match fm_span {
        Some(span) => count_lines_in(&bytes[..span.end]),
        None => 0,
    };

    let (parsed_blocks, blk_warns) =
        parse_blocks(&bytes[fm_span_end(fm_span)..], body_offset_lines)
            .context("kb-parse-md::parse_blocks")?;

    let mut all_warnings = Vec::with_capacity(fm_warns.len() + blk_warns.len());
    all_warnings.extend(fm_warns);
    all_warnings.extend(blk_warns);

    // Snapshot warning notes for the IngestItem before the vec is
    // consumed by `build_canonical_document`.
    let warning_notes: Vec<String> = all_warnings
        .iter()
        .map(|w| format!("{:?}: {}", w.kind, w.note))
        .collect();

    let mut canonical =
        build_canonical_document(asset, metadata, parsed_blocks, parser_version, all_warnings)
            .context("kb-parse-md::build_canonical_document")?;
    // v0.26.2: persist the composite parser_version (base|signature) so the
    // next run's skip compare matches what was computed above. doc_id was
    // already derived from the base version inside build_canonical_document.
    canonical.parser_version = eff_parser_version.clone();

    let parse_ms = u64::try_from(t_parse.elapsed().as_millis()).unwrap_or(u64::MAX);

    let t_chunk = std::time::Instant::now();
    let chunks = md_chunker_from_config(&app.config)
        .chunk(&canonical, chunk_policy)
        .context("kb-chunk::MdHeadingV2Chunker::chunk")?;
    let chunk_ms = u64::try_from(t_chunk.elapsed().as_millis()).unwrap_or(u64::MAX);

    // v0.24.0: surface the chunk count immediately, before the (potentially
    // very slow) expansion / embed phases — so a single large document no
    // longer looks frozen at `idx/total` while its chunks churn.
    let total_chunks = u32::try_from(chunks.len()).unwrap_or(u32::MAX);
    crate::ingest_progress::emit(
        progress,
        crate::ingest_progress::IngestEvent::AssetChunked {
            idx,
            total,
            chunks: total_chunks,
        },
    );

    // doc-side expansion(별칭) 제거됨 (HOTFIXES 2026-06-03). `expansion_ms`
    // 는 wire 호환을 위해 AssetTimings 에 남기되 항상 0.
    let expansion_ms = 0_u64;

    // Stamp chunker + embedding versions so Task 7's skip detection has
    // data on the second run.
    canonical.last_chunker_version = Some(md_chunker_from_config(&app.config).chunker_version());
    if let Some(emb) = embedder {
        canonical.last_embedding_version = Some(emb.model_version());
    }

    // Persist. Each `put_*` call wraps its own short transaction
    // (per-document tx semantics per design §5.8); composing them is
    // the kb-app job. A failure mid-way leaves the DB in a state the
    // next ingest run can re-converge (UPSERT + DELETE-then-INSERT).
    let t_store = std::time::Instant::now();
    store_document_records(app, asset, &bytes, &canonical, &chunks, "")?;
    let store_ms = u64::try_from(t_store.elapsed().as_millis()).unwrap_or(u64::MAX);

    // Embed + vector upsert (only when both sides are configured).
    // v0.26.1: surface the embed phase + model so a long embed run reads as
    // "embedding(<model>)…" rather than a frozen bar (markdown path too).
    crate::ingest_progress::emit(
        progress,
        crate::ingest_progress::IngestEvent::AssetPhase {
            idx,
            total,
            phase: "embed".to_string(),
            model: embedder.map(|e| e.model_id().0),
        },
    );
    let t_embed = std::time::Instant::now();
    // Stale-vector purge is LanceDB I/O, so it belongs to the embed/vector
    // phase — not the SQLite `store` phase. Keeping it here makes `store_ms`
    // mean "SQLite persist only" and `embed_ms` cover all vector-store work
    // (purge + upsert), so per-phase timings attribute the bottleneck
    // correctly (review fix). Runs before any new upsert, as before.
    purge_vector_orphans_for_workspace_path(app, asset, vector_store)?;
    let mut emb_cache_hit = 0_usize;
    let mut emb_cache_miss = 0_usize;
    if let (Some(emb), Some(vec_store)) = (embedder, vector_store) {
        if !chunks.is_empty() {
            let model_id = emb.model_id();
            let model_version = emb.model_version();
            let dimensions = emb.dimensions();
            // derivation cache(§3.4): embedding version_key =
            // {kind}|{model_id}|{model_version}|{dimensions}.
            // 본문 청크 + 별칭 문자열 양쪽이 같은 메커니즘(같은 text → 같은 캐시).
            // kind 토큰("doc") 을 맨 앞에 둔다: 임베더가 kind 별 프리픽스
            // (Document=`passage:`, Query=`query:`)를 붙여 같은 text 라도 벡터가
            // 달라지므로, 미래에 query 임베딩이 같은 캐시를 타도 충돌하지 않도록
            // 방어적으로 분리(현재 ingest 는 Document 고정이라 live 버그 없음).
            let emb_version_key =
                format!("doc|{}|{}|{}", model_id.0, model_version.0, dimensions);
            let mut emb_touch_keys: Vec<String> = Vec::new();
            // 본문 청크 text 로 캐시 조회 → 미스만 embed → 원래 순서로 합침.
            let body_texts: Vec<&str> = chunks.iter().map(|c| c.text.as_str()).collect();
            let vectors = embed_with_cache(
                &**emb,
                &app.sqlite,
                &body_texts,
                &emb_version_key,
                &mut emb_cache_hit,
                &mut emb_cache_miss,
                &mut emb_touch_keys,
            )
            .context("Embedder::embed (document chunks)")?;
            let records: Vec<VectorRecord> = chunks
                .iter()
                .zip(vectors)
                .map(|(c, v)| VectorRecord {
                    embedding_id: kebab_core::id_for_embedding(
                        &c.chunk_id,
                        &model_id,
                        &model_version,
                        dimensions,
                    ),
                    chunk_id: c.chunk_id.clone(),
                    vector: v,
                    doc_id: canonical.doc_id.clone(),
                    text: c.text.clone(),
                    heading_path: c.heading_path.clone(),
                    model_id: model_id.clone(),
                    model_version: model_version.clone(),
                    dimensions,
                })
                .collect();
            vec_store.upsert(&records).context("VectorStore::upsert")?;
            // 히트한 embedding 키들의 last_used_at 갱신(LRU 보존, §3.5).
            app.sqlite.derivation_cache_touch(&emb_touch_keys)?;
        }
    }

    let embed_ms = u64::try_from(t_embed.elapsed().as_millis()).unwrap_or(u64::MAX);

    // v0.24.0: phase-timing breakdown for this asset (markdown path).
    // ocr_ms / caption_ms are 0 — markdown has no image-analysis phases.
    crate::ingest_progress::emit(
        progress,
        crate::ingest_progress::IngestEvent::AssetTimings {
            idx,
            total,
            parse_ms,
            chunk_ms,
            expansion_ms,
            embed_ms,
            store_ms,
            ocr_ms: 0,
            caption_ms: 0,
        },
    );

    // 검증용 hit/miss 카운트 노출(§3.4 / §6): warm 재색인이 embed 0회임을
    // 로그로 확인. tracing target 은 stderr 로 흐른다.
    if emb_cache_hit + emb_cache_miss > 0 {
        tracing::info!(
            target: "kebab-app",
            doc = %canonical.doc_id.0,
            "derivation cache: embedding hit={emb_cache_hit} miss={emb_cache_miss}"
        );
    }

    let kind = if existing_doc_ids.contains(&canonical.doc_id.0) {
        kebab_core::IngestItemKind::Updated
    } else {
        kebab_core::IngestItemKind::New
    };

    Ok(kebab_core::IngestItem {
        kind,
        doc_id: Some(canonical.doc_id.clone()),
        doc_path: asset.workspace_path.clone(),
        asset_id: Some(asset.asset_id.clone()),
        byte_len: Some(asset.byte_len),
        block_count: u32::try_from(canonical.blocks.len()).ok(),
        chunk_count: u32::try_from(chunks.len()).ok(),
        parser_version: Some(parser_version.clone()),
        chunker_version: Some(md_chunker_from_config(&app.config).chunker_version()),
        warnings: warning_notes,
        pdf_ocr_pages: None,
        pdf_ocr_ms_total: None,
        error: None,
    })
}

/// P6-4: process one `MediaType::Image(_)` asset end-to-end.
///
/// Pipeline: read bytes → `ImageExtractor::extract` → optional
/// `apply_ocr` → optional `apply_caption` → existing chunker / embedder
/// / store path (the same one markdown uses, which already handles
/// `Block::ImageRef` per P1-5).
///
/// Failure semantics (per P6-4 spec):
/// - `ImageExtractor::extract` Err → propagate (caller increments
///   `errors`).
/// - OCR / caption Err → log + `Provenance::Warning` event, continue.
///   `block.ocr` / `block.caption` stay `None`. `errors` NOT incremented.
#[allow(clippy::too_many_arguments)]
fn ingest_one_image_asset(
    app: &App,
    asset: &RawAsset,
    idx: u32,
    total: u32,
    chunk_policy: &ChunkPolicy,
    embedder: Option<&Arc<dyn Embedder + Send + Sync>>,
    vector_store: Option<&Arc<kebab_store_vector::LanceVectorStore>>,
    existing_doc_ids: &std::collections::HashSet<String>,
    source_id: &str,
    image_pipeline: &ImagePipeline<'_>,
    force_reingest: bool,
    progress: Option<&std::sync::mpsc::Sender<crate::ingest_progress::IngestEvent>>,
) -> anyhow::Result<kebab_core::IngestItem> {
    let ocr_engine = image_pipeline.ocr_engine;
    let caption_llm = image_pipeline.caption_llm;
    let path = match &asset.source_uri {
        SourceUri::File(p) => p.clone(),
        SourceUri::Kb(_) => {
            return Ok(kebab_core::IngestItem {
                kind: kebab_core::IngestItemKind::Skipped,
                doc_id: None,
                doc_path: asset.workspace_path.clone(),
                asset_id: Some(asset.asset_id.clone()),
                byte_len: Some(asset.byte_len),
                block_count: None,
                chunk_count: None,
                parser_version: None,
                chunker_version: None,
                warnings: vec!["kb:// URI not yet supported".to_string()],
                pdf_ocr_pages: None,
                pdf_ocr_ms_total: None,
                error: None,
            });
        }
    };
    // p9-fb-23 task 7: incremental-ingest early-skip for the image flow.
    // Image docs use the `image-meta-v1` parser_version + the same
    // MdHeadingV2Chunker as the markdown flow (single-block doc). The
    // embedding-version check matches the markdown path: when the
    // active embedder's model_version equals what was stamped on the
    // existing doc, the asset is Unchanged.
    // v0.26.2: composite parser_version folds image OCR / caption + chunking
    // settings, so toggling `[image.ocr]` / `[image.caption]` (or changing
    // their model / prompt version) auto-re-indexes the affected images.
    let image_parser_version = ParserVersion(kebab_parse_image::PARSER_VERSION.to_string());
    let eff_parser_version = effective_parser_version(&app.config, asset, &image_parser_version);
    if let Some(item) = try_skip_unchanged(
        app,
        asset,
        &eff_parser_version,
        &md_chunker_from_config(&app.config).chunker_version(),
        embedder.map(|e| e.model_version()).as_ref(),
        force_reingest,
        None,
    )? {
        return Ok(item);
    }
    let bytes = std::fs::read(&path)
        .with_context(|| format!("read image asset bytes from {}", path.display()))?;

    // 1. Decode + EXIF + dimensions. ExtractContext.config carries
    //    nothing the image extractor reads today; we pass a default
    //    instance per the trait shape.
    let extract_config = kebab_core::ExtractConfig::default();
    // `~` / `${XDG_…}` expansion via the same helper the markdown
    // path uses, so a `~/KnowledgeBase` workspace.root resolves
    // identically across all media (HOTFIXES 2026-05-02 P9-4 follow-up).
    // p9-fb-05: relative `workspace.root` resolves against the config
    // file's directory (Config.source_dir), not the user's cwd.
    let workspace_root = app.config.resolve_workspace_root();
    let ctx = ExtractContext {
        asset,
        workspace_root: &workspace_root,
        config: &extract_config,
    };
    let t_parse = std::time::Instant::now();
    let mut canonical = app
        .extract_for(&asset.media_type, &ctx, &bytes)
        .context("kb-app::extract_for (image)")?;
    // v0.26.2: store the composite parser_version (extractor baked the base
    // `image-meta-v1`, which already fixed doc_id). Skip compare + stored
    // field must agree for next-run detection.
    canonical.parser_version = eff_parser_version.clone();
    // `[[workspace.sources]]`: stamp the owning source id (image extractor
    // leaves it None).
    canonical.metadata.source_id = Some(source_id.to_string());
    let parse_ms = u64::try_from(t_parse.elapsed().as_millis()).unwrap_or(u64::MAX);

    // 2 + 3. Apply OCR / caption when their adapters exist. Both are
    //        Lenient — failure is captured into Provenance Warning,
    //        `block.ocr` / `block.caption` stay `None`. P6-4 spec
    //        explicitly: such partial failures do NOT increment the
    //        `errors` counter.
    //
    //        Determinism stress (per spec Risks): the per-document
    //        Provenance timestamps for any analysis-stage Warning
    //        events share a single `now_utc()` reading taken once
    //        here, mirroring `kb-normalize::build_canonical_document`.
    let lang_hint = lang_hint_from_doc(&canonical);
    let now = time::OffsetDateTime::now_utc();
    let mut warning_notes: Vec<String> = Vec::new();
    // v0.26.1: vision phases (OCR / caption) are the usual bottleneck on an
    // image-heavy vault and emitted no progress before — so the bar looked
    // frozen. Surface each as an `AssetPhase` and measure its wall-clock for
    // the slowest-asset summary.
    let mut ocr_ms = 0_u64;
    let mut caption_ms = 0_u64;
    match canonical.blocks.first_mut() {
        Some(Block::ImageRef(block)) => {
            if let Some(engine) = ocr_engine {
                crate::ingest_progress::emit(
                    progress,
                    crate::ingest_progress::IngestEvent::AssetPhase {
                        idx,
                        total,
                        phase: "ocr".to_string(),
                        model: Some(engine.model().to_string()),
                    },
                );
                let t_ocr = std::time::Instant::now();
                let res = apply_ocr(
                    engine,
                    &bytes,
                    block,
                    lang_hint.as_ref(),
                    &mut canonical.provenance.events,
                );
                ocr_ms = u64::try_from(t_ocr.elapsed().as_millis()).unwrap_or(u64::MAX);
                if let Err(e) = res {
                    record_image_analysis_failure(
                        asset,
                        &mut canonical.provenance.events,
                        &mut warning_notes,
                        "OcrFailed",
                        e,
                        now,
                    );
                }
            }
            if let Some(llm) = caption_llm {
                crate::ingest_progress::emit(
                    progress,
                    crate::ingest_progress::IngestEvent::AssetPhase {
                        idx,
                        total,
                        phase: "caption".to_string(),
                        model: Some(llm.model_ref().id),
                    },
                );
                let t_caption = std::time::Instant::now();
                let res = apply_caption(
                    llm,
                    &bytes,
                    block,
                    lang_hint.as_ref(),
                    &app.config,
                    &mut canonical.provenance.events,
                );
                caption_ms = u64::try_from(t_caption.elapsed().as_millis()).unwrap_or(u64::MAX);
                if let Err(e) = res {
                    record_image_analysis_failure(
                        asset,
                        &mut canonical.provenance.events,
                        &mut warning_notes,
                        "CaptionFailed",
                        e,
                        now,
                    );
                }
            }
        }
        // P6-1 contract: image documents always have exactly one
        // `Block::ImageRef`. If a future task introduces multi-block
        // image documents the silent-skip would mask a real bug, so
        // this arm surfaces the divergence loudly.
        other => {
            tracing::warn!(
                target: "kebab-app",
                path = %asset.workspace_path.0,
                blocks = canonical.blocks.len(),
                "image document missing leading ImageRef block — OCR/caption skipped (first block: {:?})",
                other.map(|b| std::mem::discriminant(b))
            );
            canonical
                .provenance
                .events
                .push(kebab_core::ProvenanceEvent {
                    at: now,
                    agent: "kb-app".to_string(),
                    kind: kebab_core::ProvenanceKind::Warning,
                    note: Some(
                        "image document missing leading ImageRef block — OCR/caption skipped"
                            .to_string(),
                    ),
                });
            warning_notes.push("ImageDispatchAnomaly: missing ImageRef block".to_string());
        }
    }

    // 4. Chunk via the same `MdHeadingV2Chunker` markdown uses — its
    //    `Block::ImageRef` arm already produces a single chunk per
    //    image (P1-5). The chunk text follows the (β) plain-concat
    //    contract per the kebab-chunk render_block_text update. Using v2
    //    here keeps the markdown family consistent: a pathologically
    //    large OCR text dump splits at line boundaries just like a giant
    //    fenced code block would, instead of overflowing the embedder.
    let t_chunk = std::time::Instant::now();
    let chunks = md_chunker_from_config(&app.config)
        .chunk(&canonical, chunk_policy)
        .context("kb-chunk::MdHeadingV2Chunker::chunk (image)")?;
    let chunk_ms = u64::try_from(t_chunk.elapsed().as_millis()).unwrap_or(u64::MAX);

    // v0.24.0: surface chunk count for the image path too.
    crate::ingest_progress::emit(
        progress,
        crate::ingest_progress::IngestEvent::AssetChunked {
            idx,
            total,
            chunks: u32::try_from(chunks.len()).unwrap_or(u32::MAX),
        },
    );

    // 5. Persist + embed — identical sequence to markdown.
    // Stamp chunker + embedding versions (image uses MdHeadingV2Chunker
    // for its single-block doc, so we record that version).
    canonical.last_chunker_version = Some(md_chunker_from_config(&app.config).chunker_version());
    if let Some(emb) = embedder {
        canonical.last_embedding_version = Some(emb.model_version());
    }
    let t_store = std::time::Instant::now();
    purge_vector_orphans_for_workspace_path(app, asset, vector_store)?;
    store_document_records(app, asset, &bytes, &canonical, &chunks, " (image)")?;
    let store_ms = u64::try_from(t_store.elapsed().as_millis()).unwrap_or(u64::MAX);

    crate::ingest_progress::emit(
        progress,
        crate::ingest_progress::IngestEvent::AssetPhase {
            idx,
            total,
            phase: "embed".to_string(),
            model: embedder.map(|e| e.model_id().0),
        },
    );
    let t_embed = std::time::Instant::now();
    if let (Some(emb), Some(vec_store)) = (embedder, vector_store)
        && !chunks.is_empty()
    {
        let inputs: Vec<EmbeddingInput<'_>> = chunks
            .iter()
            .map(|c| EmbeddingInput {
                text: c.text.as_str(),
                kind: EmbeddingKind::Document,
            })
            .collect();
        let vectors = emb
            .embed(&inputs)
            .context("Embedder::embed (image chunks)")?;
        let model_id = emb.model_id();
        let model_version = emb.model_version();
        let dimensions = emb.dimensions();
        let records: Vec<VectorRecord> = chunks
            .iter()
            .zip(vectors)
            .map(|(c, v)| VectorRecord {
                embedding_id: kebab_core::id_for_embedding(
                    &c.chunk_id,
                    &model_id,
                    &model_version,
                    dimensions,
                ),
                chunk_id: c.chunk_id.clone(),
                vector: v,
                doc_id: canonical.doc_id.clone(),
                text: c.text.clone(),
                heading_path: c.heading_path.clone(),
                model_id: model_id.clone(),
                model_version: model_version.clone(),
                dimensions,
            })
            .collect();
        vec_store
            .upsert(&records)
            .context("VectorStore::upsert (image)")?;
    }
    let embed_ms = u64::try_from(t_embed.elapsed().as_millis()).unwrap_or(u64::MAX);

    // v0.26.1: per-phase timing for the image path — ocr_ms / caption_ms
    // carry the vision-model cost so the slowest-asset summary attributes
    // an image-heavy run's bottleneck correctly.
    crate::ingest_progress::emit(
        progress,
        crate::ingest_progress::IngestEvent::AssetTimings {
            idx,
            total,
            parse_ms,
            chunk_ms,
            expansion_ms: 0,
            embed_ms,
            store_ms,
            ocr_ms,
            caption_ms,
        },
    );

    let kind = if existing_doc_ids.contains(&canonical.doc_id.0) {
        kebab_core::IngestItemKind::Updated
    } else {
        kebab_core::IngestItemKind::New
    };

    Ok(kebab_core::IngestItem {
        kind,
        doc_id: Some(canonical.doc_id.clone()),
        doc_path: asset.workspace_path.clone(),
        asset_id: Some(asset.asset_id.clone()),
        byte_len: Some(asset.byte_len),
        block_count: u32::try_from(canonical.blocks.len()).ok(),
        chunk_count: u32::try_from(chunks.len()).ok(),
        parser_version: Some(canonical.parser_version.clone()),
        chunker_version: Some(md_chunker_from_config(&app.config).chunker_version()),
        warnings: warning_notes,
        pdf_ocr_pages: None,
        pdf_ocr_ms_total: None,
        error: None,
    })
}

/// Centralised handling for image-analysis (OCR / caption) failures.
/// Emits a `tracing::warn!`, appends a `ProvenanceKind::Warning`
/// event sharing the caller's per-document `now`, and pushes a
/// `<WarningKind>: <err>` note onto the `IngestItem.warnings` slot
/// using the same shape the markdown path uses (so downstream wire
/// readers don't have to learn two formats — see kb-normalize's
/// `warning_agent`).
fn record_image_analysis_failure(
    asset: &RawAsset,
    events: &mut Vec<kebab_core::ProvenanceEvent>,
    warning_notes: &mut Vec<String>,
    kind_label: &str,
    err: anyhow::Error,
    now: time::OffsetDateTime,
) {
    let detail = format!("{err:#}");
    let note = format!("{kind_label}: {detail}");
    tracing::warn!(
        target: "kebab-app",
        path = %asset.workspace_path.0,
        "image analysis stage {} failed: {}",
        kind_label,
        detail
    );
    events.push(kebab_core::ProvenanceEvent {
        at: now,
        agent: "kb-app".to_string(),
        kind: kebab_core::ProvenanceKind::Warning,
        note: Some(note.clone()),
    });
    warning_notes.push(note);
}

/// v0.17.0 PR-B: parser-bump cascade. When a code extractor ships a
/// new `PARSER_VERSION` (e.g. `code-c-v1` → `code-c-v2`), the same
/// (workspace_path, asset_id) pair re-emerges with a fresh `doc_id`.
/// The existing asset-keyed [`purge_vector_orphans_for_workspace_path`]
/// only fires on asset_id changes (file bytes edited) and is a no-op
/// here. Without an explicit doc-keyed sweep the next INSERT raises
/// `idx_docs_workspace_path` UNIQUE and the LanceDB rows under the
/// stale chunk_ids orphan. This helper:
///
/// 1. Fetches every stale chunk_id at `workspace_path` from SQLite
///    (`keep_doc_id = ""` means "all existing docs are stale" —
///    `try_skip_unchanged` calls this before the new doc_id is
///    computed).
/// 2. Deletes the matching vectors from every Lance table (no-op if
///    embeddings are disabled).
/// 3. Sweeps the SQLite `documents` row (CASCADE drops `blocks` /
///    `chunks` / `embedding_records`). The `assets` row stays — same
///    bytes, same asset_id, only the derived `doc_id` changed.
fn purge_workspace_path_for_parser_bump(app: &App, asset: &RawAsset) -> anyhow::Result<()> {
    let path = &asset.workspace_path.0;
    let stale = app
        .sqlite
        .stale_chunk_ids_for_workspace_path_except_doc_id(path, "")
        .context("SqliteStore::stale_chunk_ids_for_workspace_path_except_doc_id")?;
    if !stale.is_empty() {
        if let Some(vec_store) = app.vector().context("App::vector")? {
            use kebab_core::VectorStore as _;
            vec_store
                .delete_by_chunk_ids(&stale)
                .context("VectorStore::delete_by_chunk_ids (parser-bump orphans)")?;
        }
    }
    app.sqlite
        .purge_document_at_workspace_path_except_doc_id(path, "")
        .context("SqliteStore::purge_document_at_workspace_path_except_doc_id")?;
    tracing::debug!(
        target: "kebab-app",
        path = %path,
        count = stale.len(),
        "purged orphan vectors + document for parser_version bump"
    );
    Ok(())
}

/// HOTFIXES 2026-05-02 P7-3 follow-up: when a tracked file's bytes
/// change, `purge_orphan_at_workspace_path` (in `kebab-store-sqlite`)
/// sweeps the SQLite chain (documents → blocks / chunks / embedding_records)
/// but the LanceDB rows keyed on the now-deleted `chunk_id`s live in a
/// separate store. This helper fetches the stale `chunk_id`s from
/// SQLite **before** they get cascade-deleted, then deletes the
/// matching vectors from every Lance table.
///
/// Called by every per-medium ingest helper at the same point —
/// immediately before `put_asset_with_bytes` runs, so the SELECT
/// still sees the old chunk_ids and the DELETE happens before the
/// new rows land. Empty workspace_path / no embedder → no-op.
fn purge_vector_orphans_for_workspace_path(
    app: &App,
    asset: &RawAsset,
    vector_store: Option<&Arc<kebab_store_vector::LanceVectorStore>>,
) -> anyhow::Result<()> {
    let Some(vec_store) = vector_store else {
        return Ok(());
    };
    let stale = app
        .sqlite
        .stale_chunk_ids_at(&asset.workspace_path.0, &asset.asset_id.0)
        .context("SqliteStore::stale_chunk_ids_at")?;
    if stale.is_empty() {
        return Ok(());
    }
    use kebab_core::VectorStore as _;
    vec_store
        .delete_by_chunk_ids(&stale)
        .context("VectorStore::delete_by_chunk_ids (orphan vector cleanup)")?;
    tracing::debug!(
        target: "kebab-app",
        path = %asset.workspace_path.0,
        count = stale.len(),
        "purged orphan vectors for edited asset"
    );
    Ok(())
}

/// Persist one asset's SQLite records: asset bytes → document → blocks →
/// chunks. The four `put_*` calls were duplicated verbatim across every
/// per-medium ingest helper (markdown / image / pdf / code); this is the
/// genuinely-shared subsequence. Each `put_*` wraps its own short
/// transaction (per-document tx semantics per design §5.8); composing
/// them is the kb-app job. A failure mid-way leaves the DB in a state the
/// next ingest run can re-converge (UPSERT + DELETE-then-INSERT).
///
/// `label` suffixes the error context (e.g. `" (image)"`) so the per-medium
/// annotations stay byte-identical to the inlined form. The embed + vector
/// upsert step is intentionally NOT folded in here: it diverges per medium
/// (markdown uses the derivation cache, others embed directly) and its
/// timing boundary differs, so the callers keep it.
fn store_document_records(
    app: &App,
    asset: &RawAsset,
    bytes: &[u8],
    canonical: &CanonicalDocument,
    chunks: &[Chunk],
    label: &str,
) -> anyhow::Result<()> {
    app.sqlite
        .put_asset_with_bytes(asset, bytes)
        .with_context(|| format!("DocumentStore::put_asset_with_bytes{label}"))?;
    app.sqlite
        .put_document(canonical)
        .with_context(|| format!("DocumentStore::put_document{label}"))?;
    app.sqlite
        .put_blocks(&canonical.doc_id, &canonical.blocks)
        .with_context(|| format!("DocumentStore::put_blocks{label}"))?;
    app.sqlite
        .put_chunks(&canonical.doc_id, chunks)
        .with_context(|| format!("DocumentStore::put_chunks{label}"))?;
    Ok(())
}

/// Dogfood: post-walker sweep that purges stored documents whose source
/// file has been physically deleted from the filesystem.
///
/// Algorithm:
/// 1. Query `documents` for every `workspace_path` currently stored.
/// 2. Compute `orphan_candidates = stored_paths - scanned_paths`.
/// 3. For each candidate: resolve to an absolute path and call
///    `Path::try_exists().unwrap_or(true)` — transient FS errors
///    (EACCES, NFS hiccup, ownership change) conservatively count as
///    "still present" so we never purge on uncertain signal. If the
///    file still exists on disk it was merely out-of-scope this run
///    (config narrowing / include-glob change) — leave it alone. Only
///    files that are truly absent trigger a purge.
/// 4. For absent files: call `purge_deleted_workspace_path` (SQLite
///    cascade delete + optional copied-asset file removal) and, if a
///    vector store is present, delete the associated vectors.
///
/// Returns the number of documents purged.
///
/// Non-fatal design: individual purge failures are logged and counted
/// as errors on the per-file level but do NOT abort the sweep — a
/// partial failure is preferable to blocking the rest of ingest. The
/// return value only counts successful purges.
fn sweep_deleted_files(
    app: &App,
    scanned_paths: &std::collections::HashSet<kebab_core::WorkspacePath>,
    vector_store: Option<&kebab_store_vector::LanceVectorStore>,
) -> anyhow::Result<u32> {
    use kebab_core::DocumentStore as _;

    let stored_paths = app
        .sqlite
        .all_workspace_paths()
        .context("sweep_deleted_files: all_workspace_paths")?;

    if stored_paths.is_empty() {
        return Ok(0);
    }

    let workspace_root = app.config.resolve_workspace_root();
    let mut purged: u32 = 0;

    for stored_path in stored_paths {
        if scanned_paths.contains(&stored_path) {
            continue; // still in scope — skip
        }

        // Resolve to an absolute path and check existence on disk.
        // Use `try_exists` + `unwrap_or(true)` so transient FS errors
        // (EACCES on a path we lack read on, NFS hiccups, ownership
        // change) are CONSERVATIVELY treated as "file still present" —
        // never purge on uncertain signal (data-safety: PR #148 review).
        // `exists()` would return false on Err and trigger a wrongful
        // purge. Files whose path cannot be joined (theoretically
        // impossible for non-empty workspace_path strings, but
        // defense-in-depth) are likewise treated as still present.
        let abs = workspace_root.join(&stored_path.0);
        if abs.try_exists().unwrap_or(true) {
            // File is on disk but not in this scan's scope (config
            // narrowing). DO NOT purge — critical design constraint.
            tracing::debug!(
                target: "kebab-app",
                path = %stored_path.0,
                "sweep_deleted_files: file on disk but out of scope — leaving in store"
            );
            continue;
        }

        // File is truly absent → purge.
        let chunk_ids =
            match kebab_store_sqlite::purge_deleted_workspace_path(&app.sqlite, &stored_path) {
                Ok(ids) => ids,
                Err(e) => {
                    tracing::warn!(
                        target: "kebab-app",
                        path = %stored_path.0,
                        error = %e,
                        "sweep_deleted_files: purge failed; skipping this path"
                    );
                    continue;
                }
            };

        // Purge associated vectors (best-effort; partial failure
        // acceptable — orphan vectors get cleaned by `kebab reset
        // --vector-only` if they accumulate).
        if let Some(vec) = vector_store {
            if !chunk_ids.is_empty() {
                use kebab_core::VectorStore as _;
                if let Err(e) = vec.delete_by_chunk_ids(&chunk_ids) {
                    tracing::warn!(
                        target: "kebab-app",
                        path = %stored_path.0,
                        count = chunk_ids.len(),
                        error = %e,
                        "sweep_deleted_files: vector delete failed; SQLite side already cleaned"
                    );
                }
            }
        }

        tracing::info!(
            target: "kebab-app",
            path = %stored_path.0,
            "sweep_deleted_files: purged document for deleted file"
        );
        purged = purged.saturating_add(1);
    }

    Ok(purged)
}

/// P7-3: process one `MediaType::Pdf` asset end-to-end.
///
/// - Reads bytes from disk.
/// - Calls [`PdfTextExtractor::extract`]. Failure (corrupt header,
///   encrypted PDF, etc.) → `IngestItemKind::Error` with the formatted
///   message (so the `qpdf --decrypt` hint surfaces verbatim for the
///   encrypted-PDF case). Continue to next asset; do not abort.
/// - Hands the `CanonicalDocument` to [`PdfPageV1Chunker`] (per-medium
///   chunker selection — keyed on `MediaType::Pdf` at compile time).
///   Chunker validation failure (would only fire on P7-1 contract
///   drift OR a future routing bug) is treated as `Error` too.
/// - Persists doc + blocks + chunks via the same `DocumentStore`
///   calls the markdown / image branches use.
/// - Embeds chunks if both an embedder and a vector store are
///   configured. Embed failure marks the item as `Error` AFTER
///   doc/block/chunk rows are already written — re-running ingest
///   re-attempts the embed (consistent with the markdown path; whole-
///   asset rollback on embed-fail is a P+ task).
///
/// `chunker_version` is hard-coded to `pdf-page-v1` (HOTFIXES entry —
/// `config.ingest.chunking.chunker_version` is single-valued today and serves
/// the markdown path; per-medium config split is a P+ chunker registry
/// task).
#[allow(clippy::too_many_arguments)]
fn ingest_one_pdf_asset(
    app: &App,
    asset: &RawAsset,
    idx: u32,
    total: u32,
    chunk_policy: &ChunkPolicy,
    embedder: Option<&Arc<dyn Embedder + Send + Sync>>,
    vector_store: Option<&Arc<kebab_store_vector::LanceVectorStore>>,
    existing_doc_ids: &std::collections::HashSet<String>,
    source_id: &str,
    force_reingest: bool,
    pdf_ocr_engine: Option<&dyn OcrEngine>,
    progress: Option<&std::sync::mpsc::Sender<crate::ingest_progress::IngestEvent>>,
    cancel: Option<&std::sync::Arc<std::sync::atomic::AtomicBool>>,
    log_writer: Option<Arc<Mutex<crate::ingest_log::IngestLogWriter>>>,
    ocr_ms_samples: Arc<Mutex<Vec<u64>>>,
    ocr_pages_cnt: Arc<Mutex<u32>>,
    ocr_failures_cnt: Arc<Mutex<u32>>,
) -> anyhow::Result<kebab_core::IngestItem> {
    let path = match &asset.source_uri {
        SourceUri::File(p) => p.clone(),
        SourceUri::Kb(_) => {
            return Ok(kebab_core::IngestItem {
                kind: kebab_core::IngestItemKind::Skipped,
                doc_id: None,
                doc_path: asset.workspace_path.clone(),
                asset_id: Some(asset.asset_id.clone()),
                byte_len: Some(asset.byte_len),
                block_count: None,
                chunk_count: None,
                parser_version: None,
                chunker_version: None,
                warnings: vec!["kb:// URI not yet supported".to_string()],
                pdf_ocr_pages: None,
                pdf_ocr_ms_total: None,
                error: None,
            });
        }
    };
    // p9-fb-23 task 7: incremental-ingest early-skip for the PDF flow.
    // PDF docs use `pdf-text-v1` as the parser_version and `PdfPageV1Chunker`
    // as the chunker — both pinned per-medium today (no config knob).
    // v0.26.2: composite parser_version folds pdf.ocr (enabled/always_on/
    // model) + chunking, so enabling scanned-PDF OCR auto-re-indexes PDFs.
    let pdf_parser_version = ParserVersion(kebab_parse_pdf::PARSER_VERSION.to_string());
    let eff_parser_version = effective_parser_version(&app.config, asset, &pdf_parser_version);
    if let Some(item) = try_skip_unchanged(
        app,
        asset,
        &eff_parser_version,
        &pdf_chunker_from_config(&app.config).chunker_version(),
        embedder.map(|e| e.model_version()).as_ref(),
        force_reingest,
        None,
    )? {
        return Ok(item);
    }
    let bytes = std::fs::read(&path)
        .with_context(|| format!("read PDF asset bytes from {}", path.display()))?;

    let extract_config = kebab_core::ExtractConfig::default();
    // `~` / `${XDG_…}` expansion (HOTFIXES 2026-05-02 P9-4 follow-up).
    // p9-fb-05: relative `workspace.root` resolves against the config
    // file's directory (Config.source_dir), not the user's cwd.
    let workspace_root = app.config.resolve_workspace_root();
    let ctx = ExtractContext {
        asset,
        workspace_root: &workspace_root,
        config: &extract_config,
    };
    let t_parse = std::time::Instant::now();
    let mut canonical = app
        .extract_for(&asset.media_type, &ctx, &bytes)
        .context("kb-app::extract_for (pdf)")?;
    // v0.26.2: store the composite parser_version (base `pdf-text-v1` already
    // fixed doc_id) so the next run's skip compare matches.
    canonical.parser_version = eff_parser_version.clone();
    // `[[workspace.sources]]`: stamp the owning source id (pdf extractor
    // leaves it None).
    canonical.metadata.source_id = Some(source_id.to_string());
    let parse_ms = u64::try_from(t_parse.elapsed().as_millis()).unwrap_or(u64::MAX);

    // v0.20 sub-item 1: post-extract OCR enrichment (PR #187 registry
    // dispatch invariant 보존 — extract_for 가 normal entry).
    let (pdf_ocr_pages, pdf_ocr_ms_total): (Option<u32>, Option<u64>) = {
        let pdf_ocr = app.config.pdf_ocr();
        if pdf_ocr.enabled || pdf_ocr.always_on {
            match pdf_ocr_engine {
                Some(engine) => {
                    let ocr_opts = crate::pdf_ocr_apply::PdfOcrOpts {
                        enabled: pdf_ocr.enabled || pdf_ocr.always_on,
                        always_on: pdf_ocr.always_on,
                        valid_ratio_threshold: pdf_ocr.valid_ratio_threshold,
                        min_char_count: pdf_ocr.min_char_count,
                        lang_hint: pdf_ocr.lang_hint.clone().map(kebab_core::Lang),
                        cancel: cancel.cloned(),
                    };
                    // v0.20.x Hook 2: pre-clone Arcs for capture by OCR closure.
                    let lw_for_ocr = log_writer.clone();
                    let samples_for_ocr = ocr_ms_samples.clone();
                    let pages_for_ocr = ocr_pages_cnt.clone();
                    let failures_for_ocr = ocr_failures_cnt.clone();
                    let doc_path_for_log = asset.workspace_path.0.clone();
                    // v0.20.x r2 Step 3: pre-capture for dual-write (F1 + G1 resolution).
                    let doc_id_for_log: String = canonical.doc_id.0.clone();
                    let store_for_ocr = Arc::clone(&app.sqlite);
                    let run_id_for_log: String = lw_for_ocr
                        .as_ref()
                        .and_then(|lw| lw.lock().ok().map(|w| w.run_id().to_string()))
                        .unwrap_or_default();

                    let summary = crate::pdf_ocr_apply::apply_ocr_to_pdf_pages(
                        &mut canonical,
                        engine,
                        &bytes,
                        &ocr_opts,
                        |p| match p {
                            crate::pdf_ocr_apply::PdfOcrProgress::Started { page } => {
                                if let Some(sender) = progress {
                                    let _ = sender.send(
                                        crate::ingest_progress::IngestEvent::PdfOcrStarted { page },
                                    );
                                }
                            }
                            crate::pdf_ocr_apply::PdfOcrProgress::Finished {
                                page,
                                ms,
                                chars,
                                skipped,
                                image_byte_size,
                                image_width,
                                image_height,
                                ref failure_reason,
                            } => {
                                if let Some(sender) = progress {
                                    let _ = sender.send(
                                        crate::ingest_progress::IngestEvent::PdfOcrFinished {
                                            page,
                                            ms,
                                            chars,
                                            ocr_engine: engine.engine_name().to_string(),
                                            skipped,
                                            image_byte_size,
                                            image_width,
                                            image_height,
                                            failure_reason: failure_reason.clone(),
                                        },
                                    );
                                }
                                // v0.20.x Hook 2: write OCR event to log writer.
                                let success = !skipped && failure_reason.is_none();
                                let ts_for_event = crate::ingest_log::now_ts();
                                if let Some(ref lw) = lw_for_ocr {
                                    if let Ok(mut w) = lw.lock() {
                                        let _ = w.write_event(&crate::ingest_log::LogEvent::Ocr {
                                            ts: ts_for_event.clone(),
                                            doc_id: Some(&doc_id_for_log),
                                            doc_path: &doc_path_for_log,
                                            page,
                                            image_byte_size,
                                            image_width,
                                            image_height,
                                            ms,
                                            chars,
                                            success,
                                            reason: failure_reason.as_deref(),
                                            ocr_engine: engine.engine_name(),
                                        });
                                    }
                                }
                                // v0.20.x r2: SQLite dual-write (non-critical — R-1).
                                if let Err(e) = store_for_ocr.record_pdf_ocr_event(
                                    &run_id_for_log,
                                    &ts_for_event,
                                    Some(&doc_id_for_log),
                                    &doc_path_for_log,
                                    page,
                                    image_byte_size,
                                    image_width,
                                    image_height,
                                    ms,
                                    chars,
                                    success,
                                    failure_reason.as_deref(),
                                    engine.engine_name(),
                                ) {
                                    tracing::warn!(
                                        target: "kebab-app",
                                        "sqlite ocr event insert failed: {e}"
                                    );
                                }
                                if let Ok(mut p) = pages_for_ocr.lock() {
                                    *p += 1;
                                }
                                if success {
                                    if let Ok(mut s) = samples_for_ocr.lock() {
                                        s.push(ms);
                                    }
                                } else if let Ok(mut f) = failures_for_ocr.lock() {
                                    *f += 1;
                                }
                            }
                        },
                    )?;
                    (Some(summary.pages_ocrd), Some(summary.ms_total))
                }
                None => (Some(0), Some(0)),
            }
        } else {
            (None, None)
        }
    };

    // Per-medium chunker selection: PDF docs always use pdf-page-v1
    // regardless of `config.ingest.chunking.chunker_version`. The chunker
    // validates every block carries `SourceSpan::Page`; failure here
    // means the parser drifted from its contract. v1.2: the tier-2 oversize
    // split budget is threaded from `config.ingest.chunking.max_chunk_tokens`
    // (no new config key — same one md uses).
    let chunker = pdf_chunker_from_config(&app.config);
    let t_chunk = std::time::Instant::now();
    let chunks = chunker
        .chunk(&canonical, chunk_policy)
        .context("kb-chunk::PdfPageV1Chunker::chunk")?;
    let chunk_ms = u64::try_from(t_chunk.elapsed().as_millis()).unwrap_or(u64::MAX);

    // v0.24.0: surface chunk count for the PDF path too.
    crate::ingest_progress::emit(
        progress,
        crate::ingest_progress::IngestEvent::AssetChunked {
            idx,
            total,
            chunks: u32::try_from(chunks.len()).unwrap_or(u32::MAX),
        },
    );

    // Stamp chunker + embedding versions so Task 7's skip detection has
    // data on the second run.
    canonical.last_chunker_version = Some(chunker.chunker_version());
    if let Some(emb) = embedder {
        canonical.last_embedding_version = Some(emb.model_version());
    }

    let t_store = std::time::Instant::now();
    purge_vector_orphans_for_workspace_path(app, asset, vector_store)?;
    store_document_records(app, asset, &bytes, &canonical, &chunks, " (pdf)")?;
    let store_ms = u64::try_from(t_store.elapsed().as_millis()).unwrap_or(u64::MAX);

    crate::ingest_progress::emit(
        progress,
        crate::ingest_progress::IngestEvent::AssetPhase {
            idx,
            total,
            phase: "embed".to_string(),
            model: embedder.map(|e| e.model_id().0),
        },
    );
    let t_embed = std::time::Instant::now();
    if let (Some(emb), Some(vec_store)) = (embedder, vector_store)
        && !chunks.is_empty()
    {
        let inputs: Vec<EmbeddingInput<'_>> = chunks
            .iter()
            .map(|c| EmbeddingInput {
                text: c.text.as_str(),
                kind: EmbeddingKind::Document,
            })
            .collect();
        let vectors = emb.embed(&inputs).context("Embedder::embed (pdf chunks)")?;
        let model_id = emb.model_id();
        let model_version = emb.model_version();
        let dimensions = emb.dimensions();
        let records: Vec<VectorRecord> = chunks
            .iter()
            .zip(vectors)
            .map(|(c, v)| VectorRecord {
                embedding_id: kebab_core::id_for_embedding(
                    &c.chunk_id,
                    &model_id,
                    &model_version,
                    dimensions,
                ),
                chunk_id: c.chunk_id.clone(),
                vector: v,
                doc_id: canonical.doc_id.clone(),
                text: c.text.clone(),
                heading_path: c.heading_path.clone(),
                model_id: model_id.clone(),
                model_version: model_version.clone(),
                dimensions,
            })
            .collect();
        vec_store
            .upsert(&records)
            .context("VectorStore::upsert (pdf)")?;
    }
    let embed_ms = u64::try_from(t_embed.elapsed().as_millis()).unwrap_or(u64::MAX);

    // v0.26.1: per-phase timing for the PDF path. `ocr_ms` reuses the
    // page-OCR total already computed above so a scanned-PDF run's OCR cost
    // shows up in the slowest-asset summary; caption is markdown/image-only.
    crate::ingest_progress::emit(
        progress,
        crate::ingest_progress::IngestEvent::AssetTimings {
            idx,
            total,
            parse_ms,
            chunk_ms,
            expansion_ms: 0,
            embed_ms,
            store_ms,
            ocr_ms: pdf_ocr_ms_total.unwrap_or(0),
            caption_ms: 0,
        },
    );

    let kind = if existing_doc_ids.contains(&canonical.doc_id.0) {
        kebab_core::IngestItemKind::Updated
    } else {
        kebab_core::IngestItemKind::New
    };

    // Surface every `Provenance::Warning` note onto `IngestItem.warnings`
    // so the ingest summary shows partial-success signals (e.g. "page 2
    // empty (scanned candidate)") without forcing the operator into
    // `kebab inspect doc <id>`. Mirrors how the markdown path threads
    // frontmatter / block warnings up to the same field.
    let warnings: Vec<String> = canonical
        .provenance
        .events
        .iter()
        .filter(|e| e.kind == kebab_core::ProvenanceKind::Warning)
        .filter_map(|e| e.note.clone())
        .collect();

    Ok(kebab_core::IngestItem {
        kind,
        doc_id: Some(canonical.doc_id.clone()),
        doc_path: asset.workspace_path.clone(),
        asset_id: Some(asset.asset_id.clone()),
        byte_len: Some(asset.byte_len),
        block_count: u32::try_from(canonical.blocks.len()).ok(),
        chunk_count: u32::try_from(chunks.len()).ok(),
        parser_version: Some(canonical.parser_version.clone()),
        chunker_version: Some(chunker.chunker_version()),
        warnings,
        pdf_ocr_pages,
        pdf_ocr_ms_total,
        error: None,
    })
}

/// p10-1A-2 Task 8: process one `MediaType::Code("rust")` asset end-to-end.
///
/// Mirrors `ingest_one_pdf_asset` line-for-line with the substitutions
/// documented in the task spec:
///   - parser_version → `code-rust-v1` (via `RUST_PARSER_VERSION`)
///   - extractor     → `RustAstExtractor`
///   - chunker       → `CodeRustAstV1Chunker`
///
/// All other steps (incremental skip, byte read, ExtractContext, put_*,
/// embed, purge_vector_orphans) are identical to the PDF function.
#[allow(clippy::too_many_arguments)]
fn ingest_one_code_asset(
    app: &App,
    asset: &RawAsset,
    chunk_policy: &ChunkPolicy,
    embedder: Option<&Arc<dyn Embedder + Send + Sync>>,
    vector_store: Option<&Arc<kebab_store_vector::LanceVectorStore>>,
    existing_doc_ids: &std::collections::HashSet<String>,
    force_reingest: bool,
    code_lang: &str, // <-- NEW (p10-1b Task D)
    source_id: &str,
) -> anyhow::Result<kebab_core::IngestItem> {
    let path = match &asset.source_uri {
        SourceUri::File(p) => p.clone(),
        SourceUri::Kb(_) => {
            return Ok(kebab_core::IngestItem {
                kind: kebab_core::IngestItemKind::Skipped,
                doc_id: None,
                doc_path: asset.workspace_path.clone(),
                asset_id: Some(asset.asset_id.clone()),
                byte_len: Some(asset.byte_len),
                block_count: None,
                chunk_count: None,
                parser_version: None,
                chunker_version: None,
                warnings: vec!["kb:// URI not yet supported".to_string()],
                pdf_ocr_pages: None,
                pdf_ocr_ms_total: None,
                error: None,
            });
        }
    };

    // p10-1b Task D/G/J: parser_version per-lang.
    let parser_version = match code_lang {
        "rust" => ParserVersion(kebab_parse_code::RUST_PARSER_VERSION.to_string()),
        "python" => ParserVersion(kebab_parse_code::PYTHON_PARSER_VERSION.to_string()),
        "typescript" => ParserVersion(kebab_parse_code::TS_PARSER_VERSION.to_string()),
        "javascript" => ParserVersion(kebab_parse_code::JS_PARSER_VERSION.to_string()),
        "go" => ParserVersion(kebab_parse_code::GO_PARSER_VERSION.to_string()),
        "java" => ParserVersion(kebab_parse_code::JAVA_PARSER_VERSION.to_string()),
        "kotlin" => ParserVersion(kebab_parse_code::KOTLIN_PARSER_VERSION.to_string()),
        // p10-2: Tier 2 has no parse step — sentinel "none-v1".
        "yaml" | "dockerfile" | "toml" | "json" | "xml" | "groovy" | "go-mod" => {
            ParserVersion("none-v1".to_string())
        }
        // p10-3: shell direct routes to Tier 3 (no parse step).
        "shell" => ParserVersion("none-v1".to_string()),
        // p10-1D: C + C++ AST extractors.
        "c" => ParserVersion(kebab_parse_code::C_PARSER_VERSION.to_string()),
        "cpp" => ParserVersion(kebab_parse_code::CPP_PARSER_VERSION.to_string()),
        other => anyhow::bail!("unsupported code_lang: {other}"),
    };

    // p10-1b Task D/G/J/L: chunker_version per-lang.
    let mut chunker_version = match code_lang {
        "rust" => CodeRustAstV1Chunker.chunker_version(),
        "python" => CodePythonAstV1Chunker.chunker_version(),
        "typescript" => CodeTsAstV1Chunker.chunker_version(),
        "javascript" => CodeJsAstV1Chunker.chunker_version(),
        "go" => CodeGoAstV1Chunker.chunker_version(),
        "java" => CodeJavaAstV1Chunker.chunker_version(),
        "kotlin" => CodeKotlinAstV1Chunker.chunker_version(),
        // p10-2 Tier 2:
        "yaml" => K8sManifestResourceV1Chunker.chunker_version(),
        "dockerfile" => DockerfileFileV1Chunker.chunker_version(),
        "toml" | "json" | "xml" | "groovy" | "go-mod" => ManifestFileV1Chunker.chunker_version(),
        // p10-3:
        "shell" => CodeTextParagraphV1Chunker.chunker_version(),
        // p10-1D: C + C++ AST chunkers.
        "c" => CodeCAstV1Chunker.chunker_version(),
        "cpp" => CodeCppAstV1Chunker.chunker_version(),
        other => anyhow::bail!("unreachable chunker_version: {other}"),
    };

    // p10-3 fix: if this lang can fall back to Tier 3, compute the fallback
    // chunker_version so try_skip_unchanged can detect the stored-as-Tier-3
    // state and skip parser/chunker equality checks.
    let tier3_fallback_cv: Option<ChunkerVersion> = match code_lang {
        "rust" | "python" | "typescript" | "javascript"
        | "go" | "java" | "kotlin"
        | "yaml" | "dockerfile" | "toml" | "json" | "xml" | "groovy" | "go-mod"
        | "c" | "cpp" // p10-1D
            => Some(CodeTextParagraphV1Chunker.chunker_version()),
        _ => None,
    };

    // v0.26.2: composite parser_version folds [ingest.code] options + common
    // chunking so editing any code-ingest setting auto-re-indexes code assets.
    // The base per-lang version still derives doc_id (synthesize_tier2_document
    // / extract_for keep using `parser_version`). A Tier-3 fallback document
    // intentionally keeps the bare "none-v1" parser_version (the
    // `stored_is_tier3_fallback` bypass in try_skip_unchanged depends on the
    // exact "none-v1" sentinel), so the composite is only stamped on the
    // normal (non-fallback) outcome below.
    let eff_parser_version = effective_parser_version(&app.config, asset, &parser_version);
    if let Some(item) = try_skip_unchanged(
        app,
        asset,
        &eff_parser_version,
        &chunker_version,
        embedder.map(|e| e.model_version()).as_ref(),
        force_reingest,
        tier3_fallback_cv.as_ref(),
    )? {
        return Ok(item);
    }
    let bytes = std::fs::read(&path)
        .with_context(|| format!("read code asset bytes from {}", path.display()))?;

    let extract_config = kebab_core::ExtractConfig::default();
    let workspace_root = app.config.resolve_workspace_root();
    let ctx = ExtractContext {
        asset,
        workspace_root: &workspace_root,
        config: &extract_config,
    };

    // post-v0.18.0 extractor-dispatch-unification:
    // 9 AST lang 의 dispatch 가 polymorphic — App.extractors registry 의
    // `*AstExtractor` entry 가 lang string 으로 disjoint `supports()` 비교
    // 후 단일 hit. Tier 2 (manifest) + Tier 3 (shell) 은 free-function
    // `synthesize_tier2_document` 유지 (Extractor impl 아님 — 별 PR).
    // p10-3: capture Result so Tier 1 extractor errors can fall back to Tier 3.
    let canonical_result: anyhow::Result<kebab_core::CanonicalDocument> = match code_lang {
        // 9 AST lang: rust / python / typescript / javascript / go / java / kotlin / c / cpp
        "rust" | "python" | "typescript" | "javascript" | "go" | "java" | "kotlin" | "c"
        | "cpp" => app
            .extract_for(&asset.media_type, &ctx, &bytes)
            .with_context(|| format!("kb-app::extract_for (code:{code_lang})")),
        // p10-2 Tier 2: no extractor — synthesize Document directly from raw bytes.
        "yaml" | "dockerfile" | "toml" | "json" | "xml" | "groovy" | "go-mod" => {
            synthesize_tier2_document(asset, &bytes, code_lang, &parser_version)
        }
        // p10-3: shell reuses the same synthesizer.
        "shell" => synthesize_tier2_document(asset, &bytes, "shell", &parser_version),
        other => anyhow::bail!("unreachable (extract): {other}"),
    };

    // p10-3: Tier 1 extractor failure → fall back to Tier 3 synthesized doc.
    // Tier 2 (yaml/dockerfile/…) and shell errors are real (e.g. non-UTF-8) — propagate.
    let mut canonical = match canonical_result {
        Ok(d) => d,
        Err(e)
            if code_lang == "shell"
                || matches!(
                    code_lang,
                    "yaml" | "dockerfile" | "toml" | "json" | "xml" | "groovy" | "go-mod"
                ) =>
        {
            return Err(e).context("synthesize_tier2_document failed for tier 2/3 lang");
        }
        Err(e) => {
            // Tier 1 extractor errored — fall back to Tier 3 synthesized doc.
            tracing::warn!(
                workspace_path = %asset.workspace_path.0,
                code_lang = code_lang,
                error = %e,
                "tier1 extract errored; falling back to tier 3 synthesized doc"
            );
            chunker_version = CodeTextParagraphV1Chunker.chunker_version();
            let tier3_parser_version = ParserVersion("none-v1".to_string());
            synthesize_tier2_document(asset, &bytes, code_lang, &tier3_parser_version)
                .context("synthesize_tier2_document for tier 3 fallback after extract error")?
        }
    };

    // `[[workspace.sources]]`: stamp the owning source id on the synthesized /
    // extracted code doc (covers both Tier 1 extract_for and Tier 2/3
    // synthesize paths — neither knows the source id).
    canonical.metadata.source_id = Some(source_id.to_string());

    // p10-1b Task D/G/J/L: chunker per-lang.
    // p10-3: track whether the extract stage already fell back to Tier 3.
    // Tier 2 langs already have "none-v1" parser_version normally, so exclude them
    // from the extract_fell_back guard with the !matches! exclusion.
    let extract_fell_back = canonical.parser_version.0 == "none-v1"
        && !matches!(
            code_lang,
            "yaml" | "dockerfile" | "toml" | "json" | "xml" | "groovy" | "go-mod" | "shell"
        );

    let chunks_result: anyhow::Result<Vec<Chunk>> = if extract_fell_back {
        // Tier 1 lang whose extractor errored — go straight to Tier 3 chunker.
        CodeTextParagraphV1Chunker
            .chunk(&canonical, chunk_policy)
            .context("kb-chunk::CodeTextParagraphV1Chunker::chunk (tier 3 after extract fallback)")
    } else {
        match code_lang {
            "rust" => CodeRustAstV1Chunker
                .chunk(&canonical, chunk_policy)
                .context("kb-chunk::CodeRustAstV1Chunker::chunk (code:rust)"),
            "python" => CodePythonAstV1Chunker
                .chunk(&canonical, chunk_policy)
                .context("kb-chunk::CodePythonAstV1Chunker::chunk (code:python)"),
            "typescript" => CodeTsAstV1Chunker
                .chunk(&canonical, chunk_policy)
                .context("kb-chunk::CodeTsAstV1Chunker::chunk (code:typescript)"),
            "javascript" => CodeJsAstV1Chunker
                .chunk(&canonical, chunk_policy)
                .context("kb-chunk::CodeJsAstV1Chunker::chunk (code:javascript)"),
            "go" => CodeGoAstV1Chunker
                .chunk(&canonical, chunk_policy)
                .context("kb-chunk::CodeGoAstV1Chunker::chunk (code:go)"),
            "java" => CodeJavaAstV1Chunker
                .chunk(&canonical, chunk_policy)
                .context("kb-chunk::CodeJavaAstV1Chunker::chunk (code:java)"),
            "kotlin" => CodeKotlinAstV1Chunker
                .chunk(&canonical, chunk_policy)
                .context("kb-chunk::CodeKotlinAstV1Chunker::chunk (code:kotlin)"),
            // p10-2 Tier 2:
            "yaml" => K8sManifestResourceV1Chunker
                .chunk(&canonical, chunk_policy)
                .context("kb-chunk::K8sManifestResourceV1Chunker::chunk"),
            "dockerfile" => DockerfileFileV1Chunker
                .chunk(&canonical, chunk_policy)
                .context("kb-chunk::DockerfileFileV1Chunker::chunk"),
            "toml" | "json" | "xml" | "groovy" | "go-mod" => ManifestFileV1Chunker
                .chunk(&canonical, chunk_policy)
                .context("kb-chunk::ManifestFileV1Chunker::chunk"),
            // p10-3:
            "shell" => CodeTextParagraphV1Chunker
                .chunk(&canonical, chunk_policy)
                .context("kb-chunk::CodeTextParagraphV1Chunker::chunk (code:shell)"),
            // p10-1D: C + C++ AST chunkers.
            "c" => CodeCAstV1Chunker
                .chunk(&canonical, chunk_policy)
                .context("kebab-chunk::CodeCAstV1Chunker::chunk (code:c)"),
            "cpp" => CodeCppAstV1Chunker
                .chunk(&canonical, chunk_policy)
                .context("kebab-chunk::CodeCppAstV1Chunker::chunk (code:cpp)"),
            other => anyhow::bail!("unreachable (chunk): {other}"),
        }
    };

    // p10-3: Tier 1/2 0-chunk OR error → Tier 3 fallback retry.
    // "shell" direct path is already Tier 3 — don't retry-double-up.
    let chunks: Vec<Chunk> = match chunks_result {
        Ok(v) if !v.is_empty() => v,
        other if code_lang == "shell" => other?, // shell propagates directly
        Ok(_empty) => {
            tracing::warn!(
                workspace_path = %asset.workspace_path.0,
                code_lang = code_lang,
                "tier1/2 emitted 0 chunks; falling back to tier 3 (code-text-paragraph-v1)"
            );
            chunker_version = CodeTextParagraphV1Chunker.chunker_version();
            canonical.parser_version = ParserVersion("none-v1".to_string());
            CodeTextParagraphV1Chunker
                .chunk(&canonical, chunk_policy)
                .context("kb-chunk::CodeTextParagraphV1Chunker::chunk (tier 3 fallback)")?
        }
        Err(e) => {
            tracing::warn!(
                workspace_path = %asset.workspace_path.0,
                code_lang = code_lang,
                error = %e,
                "tier1/2 chunker errored; falling back to tier 3 (code-text-paragraph-v1)"
            );
            chunker_version = CodeTextParagraphV1Chunker.chunker_version();
            canonical.parser_version = ParserVersion("none-v1".to_string());
            CodeTextParagraphV1Chunker
                .chunk(&canonical, chunk_policy)
                .context(
                    "kb-chunk::CodeTextParagraphV1Chunker::chunk (tier 3 fallback after error)",
                )?
        }
    };

    // v0.26.2: stamp the composite parser_version for the normal outcome so
    // editing any [ingest.code] / chunking setting re-indexes this asset next
    // run. A Tier-3 fallback (an AST / manifest lang whose extractor or
    // chunker degraded to CodeTextParagraphV1Chunker) must keep the bare
    // "none-v1" sentinel, because `try_skip_unchanged`'s
    // `stored_is_tier3_fallback` bypass keys off that exact string. `shell`
    // is native Tier 3 (no bypass — `tier3_fallback_cv` is None for it), so it
    // still gets the composite.
    let is_tier3_fallback_outcome =
        code_lang != "shell" && chunker_version == CodeTextParagraphV1Chunker.chunker_version();
    if !is_tier3_fallback_outcome {
        canonical.parser_version = eff_parser_version.clone();
    }

    // Stamp chunker + embedding versions so incremental skip detection has
    // data on the second run.
    canonical.last_chunker_version = Some(chunker_version.clone());
    if let Some(emb) = embedder {
        canonical.last_embedding_version = Some(emb.model_version());
    }

    purge_vector_orphans_for_workspace_path(app, asset, vector_store)?;
    store_document_records(app, asset, &bytes, &canonical, &chunks, " (code)")?;

    if let (Some(emb), Some(vec_store)) = (embedder, vector_store)
        && !chunks.is_empty()
    {
        let inputs: Vec<EmbeddingInput<'_>> = chunks
            .iter()
            .map(|c| EmbeddingInput {
                text: c.text.as_str(),
                kind: EmbeddingKind::Document,
            })
            .collect();
        let vectors = emb
            .embed(&inputs)
            .context("Embedder::embed (code chunks)")?;
        let model_id = emb.model_id();
        let model_version = emb.model_version();
        let dimensions = emb.dimensions();
        let records: Vec<VectorRecord> = chunks
            .iter()
            .zip(vectors)
            .map(|(c, v)| VectorRecord {
                embedding_id: kebab_core::id_for_embedding(
                    &c.chunk_id,
                    &model_id,
                    &model_version,
                    dimensions,
                ),
                chunk_id: c.chunk_id.clone(),
                vector: v,
                doc_id: canonical.doc_id.clone(),
                text: c.text.clone(),
                heading_path: c.heading_path.clone(),
                model_id: model_id.clone(),
                model_version: model_version.clone(),
                dimensions,
            })
            .collect();
        vec_store
            .upsert(&records)
            .context("VectorStore::upsert (code)")?;
    }

    let kind = if existing_doc_ids.contains(&canonical.doc_id.0) {
        kebab_core::IngestItemKind::Updated
    } else {
        kebab_core::IngestItemKind::New
    };

    // Surface every `Provenance::Warning` note onto `IngestItem.warnings`.
    let warnings: Vec<String> = canonical
        .provenance
        .events
        .iter()
        .filter(|e| e.kind == kebab_core::ProvenanceKind::Warning)
        .filter_map(|e| e.note.clone())
        .collect();

    Ok(kebab_core::IngestItem {
        kind,
        doc_id: Some(canonical.doc_id.clone()),
        doc_path: asset.workspace_path.clone(),
        asset_id: Some(asset.asset_id.clone()),
        byte_len: Some(asset.byte_len),
        block_count: u32::try_from(canonical.blocks.len()).ok(),
        chunk_count: u32::try_from(chunks.len()).ok(),
        parser_version: Some(canonical.parser_version.clone()),
        chunker_version: Some(chunker_version),
        warnings,
        pdf_ocr_pages: None,
        pdf_ocr_ms_total: None,
        error: None,
    })
}

/// p10-2: Build a minimal [`CanonicalDocument`] for Tier 2 code assets
/// (yaml / dockerfile / toml / json / xml / groovy / go-mod) that have
/// no AST extractor. Produces a single `Block::Code` whose source span
/// covers the entire file, mirroring the shape the Tier 1 extractors
/// produce for glue / top-level regions.
fn synthesize_tier2_document(
    asset: &RawAsset,
    bytes: &[u8],
    code_lang: &str,
    parser_version: &ParserVersion,
) -> anyhow::Result<kebab_core::CanonicalDocument> {
    use anyhow::Context as _;
    use kebab_core::{
        BlockId, CodeBlock, CommonBlock, Lang, Metadata, Provenance, ProvenanceEvent,
        ProvenanceKind, SourceSpan, id_for_block, id_for_doc,
    };

    let text = std::str::from_utf8(bytes)
        .with_context(|| format!("tier2 doc not utf-8: {}", asset.workspace_path.0))?
        .to_string();

    let doc_id = id_for_doc(&asset.workspace_path, &asset.asset_id, parser_version);

    let n_lines = text.lines().count().max(1) as u32;
    let span = SourceSpan::Code {
        line_start: 1,
        line_end: n_lines,
        symbol: Some("<file>".to_string()),
        lang: Some(code_lang.to_string()),
    };
    let block_id: BlockId = id_for_block(&doc_id, "code", &[], 0, &span);
    let block = kebab_core::Block::Code(CodeBlock {
        common: CommonBlock {
            block_id,
            heading_path: vec![],
            source_span: span,
        },
        lang: Some(code_lang.to_string()),
        code: text,
    });

    let now = time::OffsetDateTime::now_utc();
    let events = vec![
        ProvenanceEvent {
            at: asset.discovered_at,
            agent: "kb-source-fs".to_string(),
            kind: ProvenanceKind::Discovered,
            note: None,
        },
        ProvenanceEvent {
            at: now,
            agent: "kb-app".to_string(),
            kind: ProvenanceKind::Parsed,
            note: Some(format!(
                "parser_version={}; tier2_synthesized; lang={}",
                parser_version.0, code_lang
            )),
        },
    ];

    // Resolve absolute path for repo detection. FsSourceConnector always
    // emits absolute paths in SourceUri::File (verified in connector.rs); Kb
    // URIs were rejected earlier in ingest_one_code_asset (returns Skipped),
    // so the fallback below is purely defensive. This does NOT mirror
    // RustAstExtractor — that extractor joins ctx.workspace_root for relative
    // paths, but Tier 2 trusts the connector invariant.
    let abs_path = match &asset.source_uri {
        kebab_core::SourceUri::File(p) => p.clone(),
        kebab_core::SourceUri::Kb(_) => std::path::PathBuf::new(),
    };
    let (repo, git_branch, git_commit) = match kebab_parse_code::detect_repo(&abs_path) {
        Some(r) => (Some(r.name), r.branch, r.commit),
        None => (None, None, None),
    };

    let title = {
        let fname = asset
            .workspace_path
            .0
            .rsplit('/')
            .next()
            .unwrap_or(&asset.workspace_path.0);
        // strip extension
        match fname.rfind('.') {
            Some(i) => fname[..i].to_string(),
            None => fname.to_string(),
        }
    };

    let metadata = Metadata {
        aliases: vec![],
        tags: vec![],
        created_at: asset.discovered_at,
        updated_at: asset.discovered_at,
        source_type: SourceType::Note,
        trust_level: TrustLevel::Primary,
        user_id_alias: None,
        user: serde_json::Map::new(),
        repo,
        git_branch,
        git_commit,
        code_lang: Some(code_lang.to_string()),
        // `[[workspace.sources]]`: stamped by the caller
        // (`ingest_one_code_asset`) post-build so Tier 1 (extract_for) and
        // Tier 2/3 (this synthesizer) share one code path.
        source_id: None,
    };

    tracing::debug!(
        target: "kebab-app",
        "synthesized tier2 doc_id={} workspace_path={} lang={}",
        doc_id.0,
        asset.workspace_path.0,
        code_lang,
    );

    Ok(kebab_core::CanonicalDocument {
        doc_id,
        source_asset_id: asset.asset_id.clone(),
        workspace_path: asset.workspace_path.clone(),
        title,
        lang: Lang("und".to_string()),
        blocks: vec![block],
        metadata,
        provenance: Provenance { events },
        parser_version: parser_version.clone(),
        schema_version: 1,
        doc_version: 1,
        last_chunker_version: None,
        last_embedding_version: None,
    })
}

/// Pull the BCP-47 language hint from the canonical document. P6-1
/// stamps `Lang("und")` by default; image-pipeline OCR / caption
/// adapters special-case "und" so the hint is intentionally dropped
/// from prompts.
fn lang_hint_from_doc(doc: &CanonicalDocument) -> Option<Lang> {
    let s = doc.lang.0.as_str();
    if s.is_empty() || s == "und" {
        None
    } else {
        Some(doc.lang.clone())
    }
}

/// Convenience: end byte of the frontmatter region (or 0 when absent).
fn fm_span_end(span: Option<kebab_parse_md::FrontmatterSpan>) -> usize {
    span.map_or(0, |s| s.end)
}

/// Count `\n` in a byte prefix to convert frontmatter byte span to
/// the line-offset `parse_blocks` expects.
fn count_lines_in(bytes: &[u8]) -> u32 {
    let n = bytes.iter().filter(|&&b| b == b'\n').count();
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// Build `BodyHints` from the asset alone. We use the asset's
/// `discovered_at` for both `fs_ctime` and `fs_mtime` because going
/// through the FS metadata API for every file would be a noticeable
/// overhead for large workspaces and the source-of-truth timestamps
/// are written into the document's frontmatter when the user wants
/// authoritative values.
fn build_body_hints(
    asset: &RawAsset,
    source_id: Option<&str>,
    source_trust: Option<TrustLevel>,
) -> BodyHints {
    BodyHints {
        first_h1: None,
        fs_ctime: asset.discovered_at,
        fs_mtime: asset.discovered_at,
        fallback_lang: None,
        // `[[workspace.sources]]`: stamp the owning source id + inject the
        // per-source default trust level (frontmatter still overrides it).
        source_id: source_id.map(str::to_string),
        fallback_trust_level: source_trust,
    }
}

/// Build a `ChunkPolicy` from the active config.
fn chunk_policy_from_config(config: &kebab_config::Config) -> ChunkPolicy {
    ChunkPolicy {
        target_tokens: config.ingest.chunking.target_tokens,
        overlap_tokens: config.ingest.chunking.overlap_tokens,
        respect_markdown_headings: config.ingest.chunking.respect_markdown_headings,
        chunker_version: ChunkerVersion(config.ingest.chunking.chunker_version.clone()),
    }
}

/// Construct the markdown chunker (the hardcoded `md-heading-v2`) with the
/// split budget threaded from config. Used by the markdown ingest path
/// AND the image-OCR / caption path (which flows its synthetic
/// `Block::ImageRef` text through the same chunker), so a giant OCR dump
/// is split like any other oversize chunk. The PDF path stays pinned to
/// `pdf-page-v1` and code paths keep their own AST chunkers — only the
/// markdown-family default moved v1 → v2.
fn md_chunker_from_config(config: &kebab_config::Config) -> MdHeadingV2Chunker {
    MdHeadingV2Chunker {
        max_chunk_tokens: config.ingest.chunking.max_chunk_tokens,
    }
}

/// Construct the PDF chunker (`pdf-page-v1.2`) with the tier-2 oversize
/// split budget threaded from config — mirrors [`md_chunker_from_config`].
/// The PDF path stays pinned to `pdf-page-v1` regardless of
/// `config.ingest.chunking.chunker_version`; only the tier-2 budget is
/// config-driven (no new config key — it reuses `max_chunk_tokens`, already
/// folded into `ingest_config_signature` so a budget change re-indexes PDFs
/// without `--force-reingest`). The budget also folds into the v1.2
/// `policy_hash`, aligning the PDF chunk_id cascade with markdown.
fn pdf_chunker_from_config(config: &kebab_config::Config) -> PdfPageV1Chunker {
    PdfPageV1Chunker {
        max_chunk_tokens: config.ingest.chunking.max_chunk_tokens,
    }
}

/// v0.26.2: deterministic signature of the **ingest-output-affecting**
/// config for an asset's media type, folded into the effective
/// `parser_version` (both the `try_skip_unchanged` compare field AND the
/// persisted `documents.parser_version`). When any setting that changes the
/// produced chunks / embeddings is edited, the next ingest's signature no
/// longer matches the stored one → the affected assets (only) are
/// automatically re-indexed without `--force-reingest`.
///
/// Inclusion rule: "does changing this value alter the chunk / embedding
/// content that gets indexed?" Settings that do NOT (search / rag / nli /
/// ui / logging / storage / workspace, plus runtime-only knobs like
/// `max_pixels` / `languages` / `*_timeout_secs`) are deliberately excluded
/// to avoid over-invalidation. Embedding model/dim is already covered by the
/// separate `embedding_version` cascade in [`try_skip_unchanged`], so it is
/// not duplicated here.
///
/// The output is purely a comparison token — it is never parsed back, so the
/// exact format is internal. Field order is fixed and `Vec`s are joined so
/// the same `Config` always yields the same string.
/// Process-wide memo of the paddle-onnx `engine_version`, keyed by the
/// resolved (det,rec,dict) override triple. Hashing the ~17 MB of model bytes
/// happens once per triple per process (m3 — never re-hash per asset); the
/// per-asset [`ingest_config_signature`] calls hit this cache.
static PADDLE_OCR_VERSION_MEMO: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<String, String>>,
> = std::sync::OnceLock::new();

/// T9/v3: resolve the OCR `engine_version` string used inside the ingest config
/// signature. ollama-vision is self-describing from `engine/model` (cheap, no
/// I/O). paddle-onnx hashes the bundled/override model assets (memoized).
///
/// v3: paddle 경로(det/rec/dict)는 **호출자가 미디어별로** 넘긴다 — image 는
/// `[ingest.image.ocr]`, pdf 는 `[ingest.pdf.ocr]`. v2 의 "pdf 가 image paddle
/// 을 빌려쓰던" 비대칭을 제거한다. 마이그레이션(T5)이 pdf 대칭 키를 image 값
/// 으로 채우므로 미변환 v2 → v3 의 signature 는 바이트 동일하게 유지된다.
fn ocr_engine_version_for_sig(
    engine: &str,
    model: &str,
    det: Option<&str>,
    rec: Option<&str>,
    dict: Option<&str>,
) -> String {
    if engine != PADDLE_ONNX_ENGINE {
        // ollama-vision (and any non-paddle engine): the daemon exposes no
        // stable per-model revision, so engine/model is the identity.
        return format!("ollama/{model}");
    }
    let key = format!(
        "{}|{}|{}",
        det.unwrap_or("<bundled>"),
        rec.unwrap_or("<bundled>"),
        dict.unwrap_or("<bundled>"),
    );
    let memo = PADDLE_OCR_VERSION_MEMO.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    if let Some(v) = memo.lock().unwrap().get(&key) {
        return v.clone();
    }
    // First call for this triple in this process: hash once. In any real
    // ingest the engine was already built (fail-fast) so the assets are
    // present and this succeeds; the path-derived identity below is an
    // unreachable-in-practice guard that keeps the signature total.
    let version = engine_version_for_paths(det, rec, dict).unwrap_or_else(|e| {
        tracing::warn!(
            target: "kebab-app::ingest",
            error = %e,
            "paddle-onnx engine_version hash failed; using path-derived identity for signature"
        );
        format!("ppocrv5-mobile-kor-paths:{key}")
    });
    memo.lock().unwrap().insert(key, version.clone());
    version
}

/// v3: signature 바이트 불변 골든을 위한 테스트 seam. `ingest_config_signature`
/// 는 private 이라 통합 테스트에서 직접 못 부른다. 값 기반이라 struct 경로가
/// 바뀌어도(미디어 ingest 통합) 출력 문자열은 v2 와 바이트 동일해야 한다.
#[doc(hidden)]
pub fn test_ingest_config_signature(c: &kebab_config::Config, m: &MediaType) -> String {
    ingest_config_signature(c, m)
}

fn ingest_config_signature(config: &kebab_config::Config, media: &MediaType) -> String {
    // Common (every media type): chunking parameters that move chunk
    // boundaries. `target_tokens` / `overlap_tokens` change re-chunking for
    // markdown / image / pdf / code alike, so a change re-indexes all types.
    let c = &config.ingest.chunking;
    // `max_chunk_tokens` is appended as a 5th field: md-heading-v2
    // splits any oversize chunk (list, code, paragraph, table) at this
    // budget, so changing it moves markdown chunk boundaries and must
    // re-index. It also folds into the v2 policy_hash, but the signature
    // is what the no-`--force` skip-check compares, so it must be here
    // too. Appended (not inserted) so the existing 4-field prefix
    // `chunk:T:O:H:V` stays a stable substring for any existing golden.
    let mut sig = format!(
        "chunk:{}:{}:{}:{}:{}",
        c.target_tokens,
        c.overlap_tokens,
        c.respect_markdown_headings,
        c.chunker_version,
        c.max_chunk_tokens
    );
    match media {
        MediaType::Image(_) => {
            // OCR / caption only affect output when their `enabled` flag is
            // on; the model / prompt version matters only then. Off ↔ off is
            // a stable empty token so re-running the same config skips.
            let ocr = config.image_ocr();
            if ocr.enabled {
                // v0.27.0 (T9): engine + engine_version so switching engine
                // (ollama-vision ↔ paddle-onnx) OR changing the model/assets
                // invalidates downstream chunks (design §9 cascade).
                sig.push_str(&format!(
                    "|ocr:1:{}:{}",
                    ocr.engine,
                    ocr_engine_version_for_sig(
                        &ocr.engine,
                        &ocr.model,
                        ocr.det_model.as_deref(),
                        ocr.rec_model.as_deref(),
                        ocr.dict.as_deref(),
                    )
                ));
            } else {
                sig.push_str("|ocr:0");
            }
            let cap = &config.ingest.image.caption;
            if cap.enabled {
                sig.push_str(&format!("|cap:1:{}", cap.prompt_template_version));
            } else {
                sig.push_str("|cap:0");
            }
        }
        MediaType::Pdf => {
            // PDF OCR is active when EITHER `enabled` or `always_on` is set
            // (mirrors the ingest gate). `model` only matters when active.
            let ocr = config.pdf_ocr();
            if ocr.enabled || ocr.always_on {
                // v0.27.0 (T9): engine + engine_version (same cascade rule as
                // image OCR above) alongside the enabled/always_on gate.
                sig.push_str(&format!(
                    "|pdfocr:{}:{}:{}:{}",
                    ocr.enabled,
                    ocr.always_on,
                    ocr.engine,
                    ocr_engine_version_for_sig(
                        &ocr.engine,
                        &ocr.model,
                        ocr.det_model.as_deref(),
                        ocr.rec_model.as_deref(),
                        ocr.dict.as_deref(),
                    )
                ));
            } else {
                sig.push_str("|pdfocr:0");
            }
        }
        MediaType::Code(_) => {
            let cc = &config.ingest.code;
            sig.push_str(&format!(
                "|code:{}:{}:{}:{}:{}:{}:{}",
                cc.skip_generated_header,
                cc.max_file_bytes,
                cc.max_file_lines,
                cc.extra_skip_globs.join(","),
                cc.ast_chunk_max_lines,
                cc.fallback_lines_per_chunk,
                cc.fallback_lines_overlap
            ));
        }
        // Markdown carries common-only; Audio / Other are not ingested yet.
        MediaType::Markdown | MediaType::Audio(_) | MediaType::Other(_) => {}
    }
    sig
}

/// Compose an extractor's base `parser_version` with the ingest-config
/// signature for `asset`'s media type. The result is used as the
/// `try_skip_unchanged` compare value and stored on the persisted document,
/// while the **base** version is what derives `doc_id` (kept stable to avoid
/// orphan churn — see the spec at
/// `docs/superpowers/specs/2026-06-03-ocr-toggle-invalidation-spec.md`).
fn effective_parser_version(
    config: &kebab_config::Config,
    asset: &RawAsset,
    base: &ParserVersion,
) -> ParserVersion {
    ParserVersion(format!(
        "{}|{}",
        base.0,
        ingest_config_signature(config, &asset.media_type)
    ))
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

/// Single-file ingest (p9-fb-31). Copies the file to
/// `<workspace.root>/_external/<blake3-12>.<ext>` and runs the
/// per-medium ingest pipeline on that single asset. Returns an
/// `IngestReport` with `scanned: 1` (and either `new: 1` or
/// `unchanged: 1` depending on whether the content hash + version
/// cascade match an existing doc — incremental ingest from p9-fb-23).
///
/// `path` may point inside or outside the workspace.
///
/// `.kebabignore` patterns matching `path` are bypassed with a stderr
/// `warn:` line — explicit ingest is intent.
#[doc(hidden)]
pub fn ingest_file_with_config(
    config: kebab_config::Config,
    path: &std::path::Path,
) -> anyhow::Result<IngestReport> {
    if !path.exists() {
        anyhow::bail!(
            "ingest-file: source path does not exist: {}",
            path.display()
        );
    }
    if !path.is_file() {
        anyhow::bail!("ingest-file: not a regular file: {}", path.display());
    }

    let ext_raw = path.extension().and_then(|e| e.to_str()).ok_or_else(|| {
        anyhow::anyhow!("ingest-file: source has no extension: {}", path.display())
    })?;
    let ext = ext_raw.to_lowercase();

    const SUPPORTED_EXTS: &[&str] = &["md", "pdf", "png", "jpg", "jpeg"];
    if !SUPPORTED_EXTS.contains(&ext.as_str()) {
        anyhow::bail!(
            "ingest-file: unsupported extension `.{ext}` (supported: {SUPPORTED_EXTS:?})"
        );
    }

    let bytes = std::fs::read(path)
        .with_context(|| format!("ingest-file: read source {}", path.display()))?;

    let workspace_root = config.resolve_workspace_root();

    // .kebabignore check — warn but continue.
    let ignore_match = check_kebabignore_match(&workspace_root, path);
    if ignore_match {
        eprintln!(
            "warn: {} matches .kebabignore patterns; proceeding (explicit ingest bypasses ignore)",
            path.display()
        );
    }

    // Set up _external/ dir + auto-ignore line.
    let external_dir = crate::external::ensure_external_dir(&workspace_root)
        .context("ingest-file: ensure _external/ dir")?;
    crate::external::ensure_kebabignore_entry(&workspace_root)
        .context("ingest-file: append _external/ to .kebabignore")?;

    // Copy bytes to _external/<hash>.<ext>.
    let dest = crate::external::copy_to_external(&external_dir, &bytes, &ext)
        .context("ingest-file: copy to _external")?;

    // Build a SourceScope that targets _external/ with include filter
    // restricting walk to the single dest filename.
    let filename = dest
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("ingest-file: dest has no filename"))?
        .to_string_lossy()
        .into_owned();
    let scope = kebab_core::SourceScope {
        root: external_dir.clone(),
        include: vec![filename],
        exclude: config.workspace.exclude.clone(),
    };

    ingest_with_config(config, scope, IngestOpts::default())
}

/// Stdin ingest (p9-fb-31, v1 markdown only). Prepends a YAML
/// frontmatter block (`title` + optional `source_uri`) to `body`,
/// writes the wrapped markdown to `_external/<hash12>.md`, and runs
/// `ingest_file_with_config` on the resulting file.
///
/// Errors if `body` already starts with `---` (the user should call
/// `ingest_file_with_config` directly for files that already carry
/// frontmatter).
#[doc(hidden)]
pub fn ingest_stdin_with_config(
    config: kebab_config::Config,
    body: &str,
    title: &str,
    source_uri: Option<&str>,
) -> anyhow::Result<IngestReport> {
    let wrapped = crate::external::inject_frontmatter(body, title, source_uri)?;

    let workspace_root = config.resolve_workspace_root();
    // Note: ensure_external_dir + ensure_kebabignore_entry + copy_to_external
    // are called here AND inside ingest_file_with_config. All three are
    // idempotent; the redundancy is intentional — keeping stdin's wrapped
    // bytes accessible by `ingest_file_with_config` requires the dest path
    // to exist. The ~ms double-stat overhead is negligible at v1 scale.
    let external_dir = crate::external::ensure_external_dir(&workspace_root)?;
    crate::external::ensure_kebabignore_entry(&workspace_root)?;

    let dest = crate::external::copy_to_external(&external_dir, wrapped.as_bytes(), "md")?;

    ingest_file_with_config(config, &dest)
}

/// Returns true if `source_path` matches any `.kebabignore` pattern
/// rooted at `workspace_root`. Used by `ingest_file_with_config` to
/// emit a stderr warn before bypassing the ignore.
fn check_kebabignore_match(
    workspace_root: &std::path::Path,
    source_path: &std::path::Path,
) -> bool {
    let kebabignore = workspace_root.join(".kebabignore");
    if !kebabignore.exists() {
        return false;
    }
    let text = match std::fs::read_to_string(&kebabignore) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let mut builder = ignore::gitignore::GitignoreBuilder::new(workspace_root);
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let _ = builder.add_line(None, line);
    }
    let matcher = match builder.build() {
        Ok(m) => m,
        Err(_) => return false,
    };
    matcher
        .matched(source_path, source_path.is_dir())
        .is_ignore()
}


#[cfg(test)]
mod ingest_config_signature_tests {
    //! v0.26.2: unit tests for [`ingest_config_signature`] — the
    //! ingest-output-affecting config fingerprint that is folded into the
    //! effective `parser_version` so that changing any setting that alters
    //! the produced chunks/embeddings auto-re-indexes the affected assets,
    //! while changes to unrelated settings (search/rag/ui/…) do not.

    use kebab_config::Config;
    use kebab_core::{ImageType, MediaType};

    use super::ingest_config_signature;

    fn img() -> MediaType {
        MediaType::Image(ImageType::Png)
    }
    fn pdf() -> MediaType {
        MediaType::Pdf
    }
    fn code() -> MediaType {
        MediaType::Code("rust".to_string())
    }
    fn md() -> MediaType {
        MediaType::Markdown
    }

    /// The signature is deterministic: same config + same media → same string.
    #[test]
    fn deterministic_for_unchanged_config() {
        let c = Config::defaults();
        for m in [md(), img(), pdf(), code()] {
            assert_eq!(
                ingest_config_signature(&c, &m),
                ingest_config_signature(&c, &m),
                "signature must be stable for {m:?}"
            );
        }
    }

    /// Changing a common chunking parameter changes the signature for EVERY
    /// media type (re-chunk cascade).
    #[test]
    fn chunking_change_invalidates_all_types() {
        let base = Config::defaults();
        let mut bumped = base.clone();
        bumped.ingest.chunking.target_tokens += 100;
        for m in [md(), img(), pdf(), code()] {
            assert_ne!(
                ingest_config_signature(&base, &m),
                ingest_config_signature(&bumped, &m),
                "target_tokens change must invalidate {m:?}"
            );
        }

        let mut overlap = base.clone();
        overlap.ingest.chunking.overlap_tokens += 10;
        assert_ne!(
            ingest_config_signature(&base, &md()),
            ingest_config_signature(&overlap, &md())
        );

        let mut headings = base.clone();
        headings.ingest.chunking.respect_markdown_headings = !base.ingest.chunking.respect_markdown_headings;
        assert_ne!(
            ingest_config_signature(&base, &md()),
            ingest_config_signature(&headings, &md())
        );
    }

    /// Image OCR toggle (off→on) changes only the image signature; pdf / code
    /// / markdown are unaffected.
    #[test]
    fn image_ocr_toggle_invalidates_image_only() {
        let base = Config::defaults();
        assert!(!base.ingest.image.ocr.enabled, "default OCR is off");
        let mut on = base.clone();
        on.ingest.image.ocr.enabled = true;

        assert_ne!(
            ingest_config_signature(&base, &img()),
            ingest_config_signature(&on, &img()),
            "image OCR toggle must invalidate images"
        );
        for m in [md(), pdf(), code()] {
            assert_eq!(
                ingest_config_signature(&base, &m),
                ingest_config_signature(&on, &m),
                "image OCR toggle must NOT touch {m:?}"
            );
        }
    }

    /// When OCR is enabled, changing the OCR model changes the image
    /// signature; when OCR is off, the model field is irrelevant.
    #[test]
    fn image_ocr_model_matters_only_when_enabled() {
        let mut off_a = Config::defaults();
        let mut off_b = off_a.clone();
        off_b.ingest.image.ocr.model = "some-other-model".to_string();
        assert_eq!(
            ingest_config_signature(&off_a, &img()),
            ingest_config_signature(&off_b, &img()),
            "OCR model is irrelevant while OCR is off"
        );

        off_a.ingest.image.ocr.enabled = true;
        let mut on_b = off_a.clone();
        on_b.ingest.image.ocr.model = "some-other-model".to_string();
        assert_ne!(
            ingest_config_signature(&off_a, &img()),
            ingest_config_signature(&on_b, &img()),
            "OCR model change matters while OCR is on"
        );
    }

    /// Image caption toggle + prompt-template-version change invalidate images.
    #[test]
    fn image_caption_toggle_and_prompt_invalidate_image() {
        let base = Config::defaults();
        let mut on = base.clone();
        on.ingest.image.caption.enabled = true;
        assert_ne!(
            ingest_config_signature(&base, &img()),
            ingest_config_signature(&on, &img())
        );

        let mut prompt = on.clone();
        prompt.ingest.image.caption.prompt_template_version = "caption-v9".to_string();
        assert_ne!(
            ingest_config_signature(&on, &img()),
            ingest_config_signature(&prompt, &img()),
            "caption prompt version change matters while caption is on"
        );
    }

    /// PDF OCR `enabled` and `always_on` both invalidate PDFs (either turns
    /// OCR on); they do not touch other media types.
    #[test]
    fn pdf_ocr_toggle_invalidates_pdf_only() {
        let base = Config::defaults();
        let mut enabled = base.clone();
        enabled.ingest.pdf.ocr.enabled = true;
        assert_ne!(
            ingest_config_signature(&base, &pdf()),
            ingest_config_signature(&enabled, &pdf()),
            "pdf.ocr.enabled toggle must invalidate PDFs"
        );

        let mut always = base.clone();
        always.ingest.pdf.ocr.always_on = true;
        assert_ne!(
            ingest_config_signature(&base, &pdf()),
            ingest_config_signature(&always, &pdf()),
            "pdf.ocr.always_on toggle must invalidate PDFs"
        );

        for m in [md(), img(), code()] {
            assert_eq!(
                ingest_config_signature(&base, &m),
                ingest_config_signature(&enabled, &m),
                "pdf OCR toggle must NOT touch {m:?}"
            );
        }
    }

    /// Each `[ingest.code]` option change invalidates code assets only.
    #[test]
    fn code_options_invalidate_code_only() {
        let base = Config::defaults();

        let mut variants = Vec::new();
        let mut v = base.clone();
        v.ingest.code.skip_generated_header = !base.ingest.code.skip_generated_header;
        variants.push(v);
        let mut v = base.clone();
        v.ingest.code.max_file_bytes += 1;
        variants.push(v);
        let mut v = base.clone();
        v.ingest.code.max_file_lines += 1;
        variants.push(v);
        let mut v = base.clone();
        v.ingest.code.extra_skip_globs.push("**/vendor/**".to_string());
        variants.push(v);
        let mut v = base.clone();
        v.ingest.code.ast_chunk_max_lines += 1;
        variants.push(v);
        let mut v = base.clone();
        v.ingest.code.fallback_lines_per_chunk += 1;
        variants.push(v);
        let mut v = base.clone();
        v.ingest.code.fallback_lines_overlap += 1;
        variants.push(v);

        for v in &variants {
            assert_ne!(
                ingest_config_signature(&base, &code()),
                ingest_config_signature(v, &code()),
                "code option change must invalidate code assets"
            );
            // ...but must NOT touch md / image / pdf.
            for m in [md(), img(), pdf()] {
                assert_eq!(
                    ingest_config_signature(&base, &m),
                    ingest_config_signature(v, &m),
                    "code option change must NOT touch {m:?}"
                );
            }
        }
    }

    /// Regression guard: search / rag / nli / ui / logging / storage /
    /// workspace settings — and ingest runtime-only knobs that do NOT change
    /// indexed output — never change the signature for ANY media type.
    #[test]
    fn unrelated_settings_never_invalidate() {
        let base = Config::defaults();
        let mut other = base.clone();
        // search
        other.search.default_k += 5;
        other.search.rrf_k += 1;
        other.search.snippet_chars += 10;
        // rag
        other.rag.score_gate += 0.1;
        other.rag.prompt_template_version = "rag-v99".to_string();
        // ui
        other.ui.theme = "light".to_string();
        // image runtime-only (non-output) knobs
        other.ingest.image.ocr.max_pixels += 100;
        other.ingest.image.ocr.languages.push("jpn".to_string());
        other.ingest.image.ocr.request_timeout_secs += 10;
        // pdf runtime-only knobs
        other.ingest.pdf.ocr.max_pixels += 100;
        other.ingest.pdf.ocr.request_timeout_secs += 10;
        other.ingest.pdf.ocr.languages.push("jpn".to_string());

        for m in [md(), img(), pdf(), code()] {
            assert_eq!(
                ingest_config_signature(&base, &m),
                ingest_config_signature(&other, &m),
                "unrelated/runtime-only settings must NOT invalidate {m:?}"
            );
        }
    }

    // ── v0.27.0 (T9): engine + engine_version cascade ─────────────────────

    /// (a) Switching the engine (ollama-vision → paddle-onnx) with the SAME
    /// model id changes the image signature — different engines produce
    /// different output even from an identically-named model.
    #[test]
    fn image_ocr_engine_switch_invalidates_image() {
        let mut ollama = Config::defaults();
        ollama.ingest.image.ocr.enabled = true;
        // same `model` string on both — only the engine differs
        let mut paddle = ollama.clone();
        paddle.ingest.image.ocr.engine = "paddle-onnx".to_string();
        assert_ne!(
            ingest_config_signature(&ollama, &img()),
            ingest_config_signature(&paddle, &img()),
            "engine switch with identical model must invalidate images"
        );
    }

    /// (b) A different engine_version (here: a different ollama model id, which
    /// the signature folds into `ollama/{model}`) changes the image signature.
    #[test]
    fn image_ocr_engine_version_change_invalidates_image() {
        let mut a = Config::defaults();
        a.ingest.image.ocr.enabled = true;
        a.ingest.image.ocr.model = "gemma4:e4b".to_string();
        let mut b = a.clone();
        b.ingest.image.ocr.model = "qwen2.5vl:3b".to_string();
        assert_ne!(
            ingest_config_signature(&a, &img()),
            ingest_config_signature(&b, &img()),
            "engine_version change must invalidate images"
        );
    }

    /// (b') For the paddle-onnx engine, pointing at a different model asset
    /// (override path) yields a different engine_version → different signature.
    #[test]
    fn image_ocr_paddle_model_path_change_invalidates_image() {
        let mut base = Config::defaults();
        base.ingest.image.ocr.enabled = true;
        base.ingest.image.ocr.engine = "paddle-onnx".to_string();
        let mut overridden = base.clone();
        overridden.ingest.image.ocr.det_model = Some("/some/other/det.onnx".to_string());
        assert_ne!(
            ingest_config_signature(&base, &img()),
            ingest_config_signature(&overridden, &img()),
            "paddle-onnx model path change must invalidate images"
        );
    }

    /// (c) Unrelated settings leave the paddle-onnx image signature stable
    /// (engine_version is memoized + deterministic for a fixed asset triple).
    #[test]
    fn paddle_image_signature_stable_for_unrelated_change() {
        let mut base = Config::defaults();
        base.ingest.image.ocr.enabled = true;
        base.ingest.image.ocr.engine = "paddle-onnx".to_string();
        let mut other = base.clone();
        other.search.default_k += 3;
        other.ingest.image.ocr.max_pixels += 100; // runtime-only knob
        assert_eq!(
            ingest_config_signature(&base, &img()),
            ingest_config_signature(&other, &img()),
            "unrelated/runtime-only changes must not invalidate paddle images"
        );
    }

    /// PDF OCR: engine switch with the same model invalidates pdf only.
    #[test]
    fn pdf_ocr_engine_switch_invalidates_pdf() {
        let mut ollama = Config::defaults();
        ollama.ingest.pdf.ocr.enabled = true;
        let mut paddle = ollama.clone();
        paddle.ingest.pdf.ocr.engine = "paddle-onnx".to_string();
        assert_ne!(
            ingest_config_signature(&ollama, &pdf()),
            ingest_config_signature(&paddle, &pdf()),
            "pdf engine switch must invalidate pdf"
        );
        for m in [md(), img(), code()] {
            assert_eq!(
                ingest_config_signature(&ollama, &m),
                ingest_config_signature(&paddle, &m),
                "pdf engine switch must NOT touch {m:?}"
            );
        }
    }
}
