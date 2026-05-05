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
use std::sync::Arc;

use anyhow::{Context, anyhow};
use serde::{Deserialize, Serialize};

use kebab_chunk::{MdHeadingV1Chunker, PdfPageV1Chunker};
use kebab_core::{
    Answer, Block, CanonicalDocument, Chunk, ChunkId, ChunkPolicy, ChunkerVersion, Chunker,
    DocFilter, DocSummary, DocumentId, DocumentStore, Embedder, EmbeddingInput,
    EmbeddingKind, ExtractContext, Extractor, IngestReport, Lang, LanguageModel, MediaType,
    ParserVersion, RawAsset, SearchHit, SearchQuery, SourceConnector, SourceScope,
    SourceUri, VectorRecord, VectorStore,
};
use kebab_llm_local::OllamaLanguageModel;
use kebab_normalize::build_canonical_document;
use kebab_parse_image::{ImageExtractor, OllamaVisionOcr, apply_caption, apply_ocr};
use kebab_parse_pdf::PdfTextExtractor;
use kebab_parse_md::{BodyHints, parse_blocks, parse_frontmatter};
use kebab_source_fs::FsSourceConnector;

mod app;
pub mod doctor_signal;
pub mod ingest_progress;
pub mod logging;
pub mod reset;

pub use app::App;
pub use ingest_progress::{AggregateCounts, IngestEvent, render_skipped_breakdown};
pub use reset::{ResetReport, ResetScope};

/// p9-fb-25: sentinel for files without an extension in
/// `IngestReport.skipped_by_extension` keys + `IngestItem.warnings`
/// `unsupported media type: ...` line. Wire schema description
/// references this literal — changing the sentinel is a wire-
/// compatibility break.
pub const NO_EXT_SENTINEL: &str = "<no-ext>";

/// Parser-version label persisted in `documents.parser_version` for
/// every Markdown file ingested through the `kb-parse-md` pipeline.
/// Kept in lock-step with the literal used in the `kb-store-sqlite`
/// idempotency / round-trip tests so the version label written by the
/// app and the one used in cross-crate fixtures match.
///
/// p9-fb-07 bumped this from `pulldown-cmark-0.x` to `md-frontmatter-v2`
/// because `kebab-normalize::derive_title` now applies a fallback chain
/// (frontmatter → H1 → H2 → first paragraph → file stem) when the
/// frontmatter title is blank. The bump invalidates `doc_id` for every
/// pre-existing Markdown document, so a re-ingest is required for the
/// new titles to land — this is the documented cascade behavior per
/// design §9.
const KEBAB_PARSE_MD_VERSION: &str = "md-frontmatter-v2";

/// Caller-supplied knobs for one [`ask`] invocation.
///
/// Re-exported from [`kebab_rag::AskOpts`] (P4-3 owns the type) so kb-cli's
/// `use kebab_app::AskOpts` keeps working without churn. The struct gained
/// a `stream_sink` field in P4-3; non-streaming callers (kb-cli today)
/// pass `stream_sink: None`.
pub use kebab_rag::AskOpts;

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
        let cfg = kebab_config::Config::defaults();
        let toml_text = toml::to_string_pretty(&cfg)?;
        // p9-fb-05: prepend a header comment documenting the path
        // policy so a user editing this file knows what's allowed
        // for `workspace.root` (and how relative paths resolve).
        // The actual key lives inside `[workspace]` further down;
        // we keep the explanation up top because users skim header
        // comments first.
        let header = "\
# kebab config — `~/.config/kebab/config.toml`.
#
# `workspace.root` accepts:
#   • absolute paths       (`/home/me/KnowledgeBase`)
#   • tilde                (`~/KnowledgeBase`)         ← default
#   • env vars             (`${XDG_DATA_HOME}/kebab`)
#   • relative paths       (`./notes`, `notes`, `../shared/x`)
#     — relative paths resolve against the directory of THIS
#       config file, NOT the user's `cwd` at invocation time.
#
# 처리 가능한 형식 (extractor 가 자동 결정 — config 에 명시할 수 없음):
#   • Markdown: .md
#   • 이미지:   .png .jpg .jpeg  (OCR + caption)
#   • PDF:      .pdf
# 다른 확장자는 ingest 시 자동 skip + warning. 처리 대상 폴더의
# 일부만 ingest 하고 싶으면 `kebab ingest <path>` 로 root 명시
# 또는 `.kebabignore` 파일 / 본 `workspace.exclude` 로 denylist.
#
# Override individual keys at runtime with `KEBAB_*` env vars
# (e.g. `KEBAB_WORKSPACE_ROOT=/tmp/test kebab ingest`).
\n";
        let mut combined = String::with_capacity(header.len() + toml_text.len());
        combined.push_str(header);
        combined.push_str(&toml_text);
        std::fs::write(&cfg_path, combined)?;
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

/// p9-fb-23: optional per-call ingest controls. Kept as a struct (vs.
/// a growing positional arg list) so future flags (e.g. `dry_run`,
/// per-asset `concurrency`) land additively without churning every
/// caller. Mirrors the `AskOpts` pattern from p9-fb-15.
#[derive(Default)]
pub struct IngestOpts {
    /// Streaming progress sink. `None` suppresses emission entirely.
    pub progress: Option<std::sync::mpsc::Sender<crate::ingest_progress::IngestEvent>>,
    /// Cooperative cancel token. `None` = uncancellable.
    pub cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    /// p9-fb-23: when `true`, the per-asset early-skip block is bypassed
    /// — every asset is re-parsed / re-chunked / re-embedded as if the
    /// DB were empty. Default `false` preserves the auto-skip path.
    pub force_reingest: bool,
}

pub fn ingest(scope: SourceScope, summary_only: bool) -> anyhow::Result<IngestReport> {
    let config = load_config()?;
    ingest_with_config(config, scope, summary_only)
}

/// Config-explicit variant — bypasses [`load_config`] when the
/// caller (kb-cli with `--config`, integration tests, TUI session)
/// already has a [`kebab_config::Config`] in hand. The public free
/// function [`ingest`] wraps this with the XDG-default load.
///
/// This is the no-progress entry point retained for callers that
/// don't care about streaming progress (older tests, future code that
/// runs ingest as a one-shot). It forwards into
/// [`ingest_with_config_progress`] with `progress = None`.
#[doc(hidden)]
pub fn ingest_with_config(
    config: kebab_config::Config,
    scope: SourceScope,
    summary_only: bool,
) -> anyhow::Result<IngestReport> {
    ingest_with_config_progress(config, scope, summary_only, None)
}

/// Config + progress variant — same as [`ingest_with_config`] but the
/// caller may inject an `mpsc::Sender<IngestEvent>` to receive
/// streaming progress. CLI (`p9-fb-02`) feeds this into the
/// `ingest_progress.v1` line-delimited dump; TUI (`p9-fb-03`) feeds it
/// into the status-bar reducer; either may pass `None` to suppress
/// emission entirely. Send is best-effort — see [`ingest_progress`]
/// for the contract.
#[doc(hidden)]
pub fn ingest_with_config_progress(
    config: kebab_config::Config,
    scope: SourceScope,
    summary_only: bool,
    progress: Option<std::sync::mpsc::Sender<crate::ingest_progress::IngestEvent>>,
) -> anyhow::Result<IngestReport> {
    ingest_with_config_cancellable(config, scope, summary_only, progress, None)
}

/// Config + opts variant (p9-fb-23). Supersedes the positional
/// `ingest_with_config_cancellable` fn; callers now pass an
/// [`IngestOpts`] struct so future knobs (e.g. `force_reingest`,
/// `dry_run`) land additively without churning every call site.
///
/// Existing callers that still pass positional `progress` + `cancel`
/// should use [`ingest_with_config_cancellable`], which remains as a
/// thin wrapper that builds `IngestOpts` and forwards here.
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
pub fn ingest_with_config_opts(
    config: kebab_config::Config,
    scope: SourceScope,
    summary_only: bool,
    opts: IngestOpts,
) -> anyhow::Result<IngestReport> {
    let progress = opts.progress.as_ref();
    let cancelled = || {
        opts.cancel
            .as_ref()
            .map(|c| c.load(std::sync::atomic::Ordering::Relaxed))
            .unwrap_or(false)
    };
    let force_reingest = opts.force_reingest;
    let started_instant = std::time::Instant::now();

    let app = App::open_with_config(config)?;

    // Walk the workspace.
    crate::ingest_progress::emit(
        progress,
        crate::ingest_progress::IngestEvent::ScanStarted {
            root: scope.root.to_string_lossy().into_owned(),
        },
    );
    let connector = FsSourceConnector::new(&app.config)
        .context("kb-app::ingest: build FsSourceConnector")?;
    let assets = connector
        .scan(&scope)
        .context("kb-app::ingest: scan workspace")?;
    crate::ingest_progress::emit(
        progress,
        crate::ingest_progress::IngestEvent::ScanCompleted {
            total: u32::try_from(assets.len()).unwrap_or(u32::MAX),
        },
    );

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

    let parser_version = ParserVersion(KEBAB_PARSE_MD_VERSION.to_string());
    let chunk_policy = chunk_policy_from_config(&app.config);

    // P6-4: build OCR / caption adapters once per ingest invocation,
    // gated on their respective `enabled` flags. `reqwest::blocking::Client`
    // is internally Arc-shared so reusing one instance across the asset
    // loop is correct and cheap. Construction failure (e.g. invalid
    // endpoint) aborts ingest fail-fast — better than silently disabling
    // OCR/caption mid-run.
    let ocr_engine: Option<OllamaVisionOcr> = if app.config.image.ocr.enabled {
        Some(
            OllamaVisionOcr::new(&app.config)
                .context("kb-app::ingest: build OllamaVisionOcr")?,
        )
    } else {
        None
    };
    let caption_llm: Option<Box<dyn LanguageModel>> = if app.config.image.caption.enabled {
        Some(Box::new(
            OllamaLanguageModel::new(&app.config)
                .context("kb-app::ingest: build OllamaLanguageModel for caption")?,
        ))
    } else {
        None
    };
    let image_extractor = ImageExtractor::new();
    let image_pipeline = ImagePipeline {
        extractor: &image_extractor,
        ocr_engine: ocr_engine.as_ref(),
        caption_llm: caption_llm.as_deref(),
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
        let item = ingest_one_asset(
            &app,
            &asset,
            &parser_version,
            &chunk_policy,
            embedder.as_ref(),
            vector_store.as_ref(),
            &existing_doc_ids,
            &image_pipeline,
            force_reingest,
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
                unchanged_count = unchanged_count.saturating_add(1)
            }
            kebab_core::IngestItemKind::Error => {
                error_count = error_count.saturating_add(1)
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
        "summary_only": summary_only,
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

    let duration_ms = u32::try_from(started_instant.elapsed().as_millis())
        .unwrap_or(u32::MAX);
    let finished_at = time::OffsetDateTime::now_utc();

    // Record the ingest_runs row with aggregate counts.
    // `summary_only=true` writes `items_json=NULL` (per design §5.7);
    // the count columns are populated either way.
    let scope_json = serde_json::to_string(&scope)
        .context("kb-app::ingest: serialize scope for ingest_runs.scope_json")?;
    let items_json: Option<String> = if summary_only {
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
    // commit landed (any new / updated). This invalidates every
    // entry in any in-process LRU search cache (in this process or
    // a sibling) on the next lookup. No-op when nothing changed
    // (skipped-only run) — the cache stays valid.
    if new_count > 0 || updated_count > 0 {
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
        items: if summary_only { None } else { Some(items) },
    })
}

/// Config + progress + cancel variant (p9-fb-04). Retained as a thin
/// wrapper around [`ingest_with_config_opts`] for external callers
/// (test fixtures, CLI) that pass positional `progress` + `cancel`
/// arguments. New callers should prefer [`ingest_with_config_opts`]
/// with an explicit [`IngestOpts`].
///
/// CLI's `Ctrl-C` SIGINT handler and TUI's `Esc` / `Ctrl-C` both
/// flip the `cancel` `AtomicBool`. Pass `None` to retain
/// pre-p9-fb-04 behaviour (uncancellable).
#[doc(hidden)]
pub fn ingest_with_config_cancellable(
    config: kebab_config::Config,
    scope: SourceScope,
    summary_only: bool,
    progress: Option<std::sync::mpsc::Sender<crate::ingest_progress::IngestEvent>>,
    cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
) -> anyhow::Result<IngestReport> {
    ingest_with_config_opts(
        config,
        scope,
        summary_only,
        IngestOpts {
            progress,
            cancel,
            force_reingest: false,
        },
    )
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

/// P6-4: borrowed bundle of the three image-pipeline components built
/// once per ingest invocation. Threaded through `ingest_one_asset` so
/// the dispatch does not need ten separate parameters.
struct ImagePipeline<'a> {
    extractor: &'a ImageExtractor,
    ocr_engine: Option<&'a OllamaVisionOcr>,
    caption_llm: Option<&'a dyn LanguageModel>,
}

/// p9-fb-23 task 7: incremental-ingest early-skip predicate. Shared
/// across the markdown / image / PDF per-asset flows. Returns
/// `Some(IngestItem { kind: Unchanged, .. })` when ALL FOUR conditions
/// hold (per design §9 cascade rule):
///
/// 1. `force_reingest == false` — caller hasn't asked to bypass skip.
/// 2. The freshly-scanned asset's blake3 checksum equals what the
///    existing `assets` row stores at the same `workspace_path`.
/// 3. The doc keyed on `(workspace_path, asset_id, current_parser_version)`
///    exists. If the parser_version changed, `id_for_doc` produces a
///    different `doc_id` so the lookup misses → no skip → re-process.
/// 4. The existing doc's stamped `last_chunker_version` AND
///    `last_embedding_version` match the values the caller is about
///    to use (`Some(v) == Some(v)` and `None == None` — see design
///    doc for the `None == None` rule when no embedder is configured).
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
) -> anyhow::Result<Option<kebab_core::IngestItem>> {
    if force_reingest {
        return Ok(None);
    }
    let existing_asset = match app
        .sqlite
        .get_asset_by_workspace_path(&asset.workspace_path)
    {
        Ok(Some(a)) => a,
        Ok(None) => return Ok(None),
        Err(e) => {
            tracing::debug!(
                target: "kebab-app",
                path = %asset.workspace_path.0,
                error = %e,
                "skip-check: get_asset_by_workspace_path failed; falling through to re-process"
            );
            return Ok(None);
        }
    };
    if existing_asset.checksum != asset.checksum {
        return Ok(None);
    }
    let candidate_doc_id = kebab_core::id_for_doc(
        &asset.workspace_path,
        &asset.asset_id,
        current_parser_version,
    );
    let existing_doc = match app.sqlite.get_document(&candidate_doc_id) {
        Ok(Some(d)) => d,
        Ok(None) => return Ok(None),
        Err(e) => {
            tracing::debug!(
                target: "kebab-app",
                path = %asset.workspace_path.0,
                error = %e,
                "skip-check: get_document failed; falling through to re-process"
            );
            return Ok(None);
        }
    };
    let chunker_match = existing_doc.last_chunker_version.as_ref()
        == Some(current_chunker_version);
    if !chunker_match {
        return Ok(None);
    }
    let embedder_match = existing_doc.last_embedding_version.as_ref()
        == current_embedding_version;
    if !embedder_match {
        return Ok(None);
    }
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
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_else(|| NO_EXT_SENTINEL.to_string())
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

/// Process a single asset: read bytes, parse, normalize, chunk,
/// persist, embed. Per-asset failures bubble up to the caller for
/// labelling as `IngestItemKind::Error` — they do NOT abort the
/// whole run.
#[allow(clippy::too_many_arguments)]
fn ingest_one_asset(
    app: &App,
    asset: &RawAsset,
    parser_version: &ParserVersion,
    chunk_policy: &ChunkPolicy,
    embedder: Option<&Arc<dyn Embedder + Send + Sync>>,
    vector_store: Option<&Arc<kebab_store_vector::LanceVectorStore>>,
    existing_doc_ids: &std::collections::HashSet<String>,
    image_pipeline: &ImagePipeline<'_>,
    force_reingest: bool,
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
                chunk_policy,
                embedder,
                vector_store,
                existing_doc_ids,
                image_pipeline,
                force_reingest,
            );
        }
        MediaType::Pdf => {
            return ingest_one_pdf_asset(
                app,
                asset,
                chunk_policy,
                embedder,
                vector_store,
                existing_doc_ids,
                force_reingest,
            );
        }
        _ => {
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
                error: None,
            });
        }
    };

    // p9-fb-23 task 7: incremental-ingest early-skip. When force_reingest
    // is false AND the on-disk asset's checksum + parser_version +
    // last_chunker_version + last_embedding_version all match the existing
    // DB record, this asset doesn't need to be re-parsed / re-chunked /
    // re-embedded. Return Unchanged so the caller bumps `aggregate.unchanged`
    // and the AssetFinished progress event reflects the skip.
    if let Some(item) = try_skip_unchanged(
        app,
        asset,
        parser_version,
        &MdHeadingV1Chunker.chunker_version(),
        embedder.map(|e| e.model_version()).as_ref(),
        force_reingest,
    )? {
        return Ok(item);
    }

    let bytes = std::fs::read(&path)
        .with_context(|| format!("read asset bytes from {}", path.display()))?;

    let body_hints = build_body_hints(asset);

    // Frontmatter — `parse_frontmatter` returns Ok even on malformed
    // frontmatter (warnings are surfaced through the `Vec<Warning>`).
    let (metadata, fm_span, fm_warns) = parse_frontmatter(&bytes, &body_hints)
        .context("kb-parse-md::parse_frontmatter")?;

    let body_offset_lines = match fm_span {
        Some(span) => count_lines_in(&bytes[..span.end]),
        None => 0,
    };

    let (parsed_blocks, blk_warns) = parse_blocks(&bytes[fm_span_end(fm_span)..], body_offset_lines)
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

    let mut canonical = build_canonical_document(
        asset,
        metadata,
        parsed_blocks,
        parser_version,
        all_warnings,
    )
    .context("kb-normalize::build_canonical_document")?;

    let chunks = MdHeadingV1Chunker
        .chunk(&canonical, chunk_policy)
        .context("kb-chunk::MdHeadingV1Chunker::chunk")?;

    // Stamp chunker + embedding versions so Task 7's skip detection has
    // data on the second run.
    canonical.last_chunker_version = Some(MdHeadingV1Chunker.chunker_version());
    if let Some(emb) = embedder {
        canonical.last_embedding_version = Some(emb.model_version());
    }

    // Persist. Each `put_*` call wraps its own short transaction
    // (per-document tx semantics per design §5.8); composing them is
    // the kb-app job. A failure mid-way leaves the DB in a state the
    // next ingest run can re-converge (UPSERT + DELETE-then-INSERT).
    purge_vector_orphans_for_workspace_path(app, asset, vector_store)?;
    app.sqlite
        .put_asset_with_bytes(asset, &bytes)
        .context("DocumentStore::put_asset_with_bytes")?;
    app.sqlite
        .put_document(&canonical)
        .context("DocumentStore::put_document")?;
    app.sqlite
        .put_blocks(&canonical.doc_id, &canonical.blocks)
        .context("DocumentStore::put_blocks")?;
    app.sqlite
        .put_chunks(&canonical.doc_id, &chunks)
        .context("DocumentStore::put_chunks")?;

    // Embed + vector upsert (only when both sides are configured).
    if let (Some(emb), Some(vec_store)) = (embedder, vector_store) {
        if !chunks.is_empty() {
            let inputs: Vec<EmbeddingInput<'_>> = chunks
                .iter()
                .map(|c| EmbeddingInput {
                    text: c.text.as_str(),
                    kind: EmbeddingKind::Document,
                })
                .collect();
            let vectors = emb
                .embed(&inputs)
                .context("Embedder::embed (document chunks)")?;
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
                .context("VectorStore::upsert")?;
        }
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
        chunker_version: Some(MdHeadingV1Chunker.chunker_version()),
        warnings: warning_notes,
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
    chunk_policy: &ChunkPolicy,
    embedder: Option<&Arc<dyn Embedder + Send + Sync>>,
    vector_store: Option<&Arc<kebab_store_vector::LanceVectorStore>>,
    existing_doc_ids: &std::collections::HashSet<String>,
    image_pipeline: &ImagePipeline<'_>,
    force_reingest: bool,
) -> anyhow::Result<kebab_core::IngestItem> {
    let image_extractor = image_pipeline.extractor;
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
                warnings: vec![
                    "kb:// URI not yet supported".to_string(),
                ],
                error: None,
            });
        }
    };
    // p9-fb-23 task 7: incremental-ingest early-skip for the image flow.
    // Image docs use the `image-meta-v1` parser_version + the same
    // MdHeadingV1Chunker as the markdown flow (single-block doc). The
    // embedding-version check matches the markdown path: when the
    // active embedder's model_version equals what was stamped on the
    // existing doc, the asset is Unchanged.
    let image_parser_version = ParserVersion(kebab_parse_image::PARSER_VERSION.to_string());
    if let Some(item) = try_skip_unchanged(
        app,
        asset,
        &image_parser_version,
        &MdHeadingV1Chunker.chunker_version(),
        embedder.map(|e| e.model_version()).as_ref(),
        force_reingest,
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
    let mut canonical = image_extractor
        .extract(&ctx, &bytes)
        .context("kb-parse-image::ImageExtractor::extract")?;

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
    match canonical.blocks.first_mut() {
        Some(Block::ImageRef(block)) => {
            if let Some(engine) = ocr_engine
                && let Err(e) = apply_ocr(
                    engine,
                    &bytes,
                    block,
                    lang_hint.as_ref(),
                    &mut canonical.provenance.events,
                )
            {
                record_image_analysis_failure(
                    asset,
                    &mut canonical.provenance.events,
                    &mut warning_notes,
                    "OcrFailed",
                    e,
                    now,
                );
            }
            if let Some(llm) = caption_llm
                && let Err(e) = apply_caption(
                    llm,
                    &bytes,
                    block,
                    lang_hint.as_ref(),
                    &app.config,
                    &mut canonical.provenance.events,
                )
            {
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
            canonical.provenance.events.push(kebab_core::ProvenanceEvent {
                at: now,
                agent: "kb-app".to_string(),
                kind: kebab_core::ProvenanceKind::Warning,
                note: Some(
                    "image document missing leading ImageRef block — OCR/caption skipped"
                        .to_string(),
                ),
            });
            warning_notes
                .push("ImageDispatchAnomaly: missing ImageRef block".to_string());
        }
    }

    // 4. Chunk via the same `MdHeadingV1Chunker` markdown uses — its
    //    `Block::ImageRef` arm already produces a single chunk per
    //    image (P1-5). The chunk text now follows the (β) plain-concat
    //    contract per the kebab-chunk render_block_text update.
    let chunks = MdHeadingV1Chunker
        .chunk(&canonical, chunk_policy)
        .context("kb-chunk::MdHeadingV1Chunker::chunk (image)")?;

    // 5. Persist + embed — identical sequence to markdown.
    // Stamp chunker + embedding versions (image uses MdHeadingV1Chunker
    // for its single-block doc, so we record that version).
    canonical.last_chunker_version = Some(MdHeadingV1Chunker.chunker_version());
    if let Some(emb) = embedder {
        canonical.last_embedding_version = Some(emb.model_version());
    }
    purge_vector_orphans_for_workspace_path(app, asset, vector_store)?;
    app.sqlite
        .put_asset_with_bytes(asset, &bytes)
        .context("DocumentStore::put_asset_with_bytes (image)")?;
    app.sqlite
        .put_document(&canonical)
        .context("DocumentStore::put_document (image)")?;
    app.sqlite
        .put_blocks(&canonical.doc_id, &canonical.blocks)
        .context("DocumentStore::put_blocks (image)")?;
    app.sqlite
        .put_chunks(&canonical.doc_id, &chunks)
        .context("DocumentStore::put_chunks (image)")?;

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
        chunker_version: Some(MdHeadingV1Chunker.chunker_version()),
        warnings: warning_notes,
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
/// `config.chunking.chunker_version` is single-valued today and serves
/// the markdown path; per-medium config split is a P+ chunker registry
/// task).
#[allow(clippy::too_many_arguments)]
fn ingest_one_pdf_asset(
    app: &App,
    asset: &RawAsset,
    chunk_policy: &ChunkPolicy,
    embedder: Option<&Arc<dyn Embedder + Send + Sync>>,
    vector_store: Option<&Arc<kebab_store_vector::LanceVectorStore>>,
    existing_doc_ids: &std::collections::HashSet<String>,
    force_reingest: bool,
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
                warnings: vec![
                    "kb:// URI not yet supported".to_string(),
                ],
                error: None,
            });
        }
    };
    // p9-fb-23 task 7: incremental-ingest early-skip for the PDF flow.
    // PDF docs use `pdf-text-v1` as the parser_version and `PdfPageV1Chunker`
    // as the chunker — both pinned per-medium today (no config knob).
    let pdf_parser_version = ParserVersion(kebab_parse_pdf::PARSER_VERSION.to_string());
    if let Some(item) = try_skip_unchanged(
        app,
        asset,
        &pdf_parser_version,
        &PdfPageV1Chunker.chunker_version(),
        embedder.map(|e| e.model_version()).as_ref(),
        force_reingest,
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
    let mut canonical = PdfTextExtractor::new()
        .extract(&ctx, &bytes)
        .context("kb-parse-pdf::PdfTextExtractor::extract")?;

    // Per-medium chunker selection: PDF docs always use pdf-page-v1
    // regardless of `config.chunking.chunker_version`. The chunker
    // validates every block carries `SourceSpan::Page`; failure here
    // means the parser drifted from its contract.
    let chunker = PdfPageV1Chunker;
    let chunks = chunker
        .chunk(&canonical, chunk_policy)
        .context("kb-chunk::PdfPageV1Chunker::chunk")?;

    // Stamp chunker + embedding versions so Task 7's skip detection has
    // data on the second run.
    canonical.last_chunker_version = Some(chunker.chunker_version());
    if let Some(emb) = embedder {
        canonical.last_embedding_version = Some(emb.model_version());
    }

    purge_vector_orphans_for_workspace_path(app, asset, vector_store)?;
    app.sqlite
        .put_asset_with_bytes(asset, &bytes)
        .context("DocumentStore::put_asset_with_bytes (pdf)")?;
    app.sqlite
        .put_document(&canonical)
        .context("DocumentStore::put_document (pdf)")?;
    app.sqlite
        .put_blocks(&canonical.doc_id, &canonical.blocks)
        .context("DocumentStore::put_blocks (pdf)")?;
    app.sqlite
        .put_chunks(&canonical.doc_id, &chunks)
        .context("DocumentStore::put_chunks (pdf)")?;

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
            .context("Embedder::embed (pdf chunks)")?;
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
        error: None,
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
    span.map(|s| s.end).unwrap_or(0)
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
fn build_body_hints(asset: &RawAsset) -> BodyHints {
    BodyHints {
        first_h1: None,
        fs_ctime: asset.discovered_at,
        fs_mtime: asset.discovered_at,
        fallback_lang: None,
    }
}

/// Build a `ChunkPolicy` from the active config.
fn chunk_policy_from_config(config: &kebab_config::Config) -> ChunkPolicy {
    ChunkPolicy {
        target_tokens: config.chunking.target_tokens,
        overlap_tokens: config.chunking.overlap_tokens,
        respect_markdown_headings: config.chunking.respect_markdown_headings,
        chunker_version: ChunkerVersion(config.chunking.chunker_version.clone()),
    }
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

/// p9-fb-18: ask under a persistent chat session. Loads prior turns
/// from `chat_sessions[session_id]`, runs the query as a follow-up
/// (via `RagPipeline::ask_with_history`), and appends the new turn
/// — auto-creating the session header on first use. Returns an
/// `Answer` with `conversation_id = Some(session_id)` and
/// `turn_index` set to the new (post-append) index. CLI `kebab
/// ask --session <id>` entry point (p9-fb-18).
#[doc(hidden)]
pub fn ask_with_session_with_config(
    config: kebab_config::Config,
    session_id: &str,
    query: &str,
    opts: AskOpts,
) -> anyhow::Result<Answer> {
    App::open_with_config(config)?.ask_with_session(session_id, query, opts)
}

/// Run the doctor checks against the explicit config path the user
/// requested via `--config` (or the XDG default if `None`). The
/// `config_loaded` check reports the actual path probed and the
/// `data_dir_writable` check probes the resolved `storage.data_dir`
/// from that config (so `--config` users see their custom paths
/// reflected in the report rather than the XDG defaults).
pub fn doctor_with_config_path(config_path: Option<&std::path::Path>) -> anyhow::Result<DoctorReport> {
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
        (
            false,
            format!("{} (not found)", cfg_path.display()),
            None,
        )
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
