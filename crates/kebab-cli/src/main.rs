//! `kebab` — command-line interface. Each subcommand maps 1:1 to a `kebab-app`
//! function. Exit codes per design §10.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Context;
use clap::{Parser, Subcommand};

use kebab_app::doctor_signal::{DoctorUnhealthy, NoHitSignal, RefusalSignal};

mod cancel;
mod progress;
mod wire;

#[derive(Parser, Debug)]
#[command(name = "kebab", version, about = "personal local knowledge base")]
struct Cli {
    /// Path to a non-default `config.toml`.
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    /// Show anyhow chain on errors.
    #[arg(long, global = true)]
    verbose: bool,

    /// Show tracing target/level on errors.
    #[arg(long, global = true)]
    debug: bool,

    /// Emit machine-readable wire JSON (`*.v1`).
    #[arg(long, global = true)]
    json: bool,

    /// Disable all write-path subcommands (also: KEBAB_READONLY=1 env var).
    #[arg(long, global = true, env = "KEBAB_READONLY",
          value_parser = parse_bool_env)]
    readonly: bool,

    /// Suppress all human-readable stderr output: progress lines, hints.
    /// Implied by `--json`.
    #[arg(long, global = true)]
    quiet: bool,

    #[command(subcommand)]
    command: Cmd,
}

// p10-1A-1: adding `repo` and `code_lang` Vec<String> fields pushed `Cmd`
// over clippy's large_enum_variant threshold. The enum is short-lived
// (parsed once at startup, never cloned in a hot path) — boxing would add
// noise with no real benefit.
#[allow(clippy::large_enum_variant)]
#[derive(Subcommand, Debug)]
enum Cmd {
    /// Initialise XDG dirs + workspace + `config.toml`.
    Init {
        /// Overwrite an existing `config.toml`.
        #[arg(long)]
        force: bool,
    },

    /// config.toml 관리 (스키마 마이그레이션 등).
    Config {
        #[command(subcommand)]
        what: ConfigWhat,
    },

    /// Scan the workspace and ingest new/updated documents.
    Ingest {
        /// Workspace root override.
        #[arg(long)]
        root: Option<PathBuf>,

        /// Suppress the per-file `items` list.
        #[arg(long)]
        summary_only: bool,

        /// p9-fb-23: bypass the per-asset early-skip path. Every asset is
        /// re-parsed, re-chunked, re-embedded, and re-upserted regardless
        /// of whether the DB already has a record with matching checksum
        /// and version stamps. Useful after manual schema bumps or when
        /// the user suspects the corpus is in a stale state.
        #[arg(long)]
        force_reingest: bool,
    },

    /// Listing subcommands.
    List {
        #[command(subcommand)]
        what: ListWhat,
    },

    /// Inspect documents or chunks by ID.
    Inspect {
        #[command(subcommand)]
        what: InspectWhat,
    },

    /// p9-fb-35: verbatim chunk / doc / span fetch.
    Fetch {
        #[command(subcommand)]
        what: FetchWhat,
    },

    /// Lexical / vector / hybrid search over chunks.
    Search {
        /// Query text. Not required when `--bulk` is set (queries from stdin).
        query: Option<String>,

        #[arg(long, default_value_t = 10)]
        k: usize,

        #[arg(long, value_enum, default_value_t = ModeFlag::Hybrid)]
        mode: ModeFlag,

        #[arg(long)]
        explain: bool,

        /// p9-fb-19: bypass the in-process LRU search cache for
        /// this invocation. Forces a fresh retriever run even when
        /// the same query was just served from cache. Useful when
        /// debugging retriever behavior — and a no-op for the CLI
        /// (each invocation is a new process anyway, so the cache
        /// starts empty), but the flag stays for parity with the
        /// future TUI cache-aware search and for explicit intent.
        #[arg(long)]
        no_cache: bool,

        /// p9-fb-34: cap result wire JSON size at approximately N tokens
        /// (chars/4 estimate). When set, smaller snippets and fewer hits
        /// may be returned; check `truncated` in the JSON wire.
        #[arg(long)]
        max_tokens: Option<usize>,

        /// p9-fb-34: per-hit snippet character cap, overrides
        /// `config.search.snippet_chars` for this call only.
        #[arg(long)]
        snippet_chars: Option<usize>,

        /// p9-fb-34: opaque cursor from a previous response's
        /// `next_cursor` to fetch the next page. Mismatched
        /// `corpus_revision` returns `error.v1.code = stale_cursor`.
        #[arg(long)]
        cursor: Option<String>,

        /// p9-fb-36: filter by `metadata.tags`. Repeatable; OR-within (any tag).
        #[arg(long)]
        tag: Vec<String>,

        /// p9-fb-36: filter by `documents.lang` (ISO code).
        #[arg(long)]
        lang: Option<String>,

        /// p9-fb-36: filter by `documents.workspace_path` glob.
        #[arg(long)]
        path_glob: Option<String>,

        /// p9-fb-36: filter by minimum `documents.trust_level`.
        #[arg(long, value_enum)]
        trust_min: Option<TrustLevelFlag>,

        /// p9-fb-36: filter by `assets.media_type` kind. Comma-separated.
        /// Aliases: `md` → `markdown`. Other accepted: `markdown`, `pdf`,
        /// `image`, `audio`, `code`, `other`. Unknown values match nothing.
        #[arg(long, value_delimiter = ',')]
        media: Vec<String>,

        /// p9-fb-36: filter to docs whose `updated_at` is >= this RFC3339
        /// timestamp (UTC). Invalid format → exit 2 with error.v1
        /// code = config_invalid.
        #[arg(long)]
        ingested_after: Option<String>,

        /// p9-fb-36: filter to a single doc by id.
        #[arg(long)]
        doc_id: Option<String>,

        /// p10-1A-1: filter by repo name (`metadata.repo`). Repeatable;
        /// multi-value = OR.  Empty = no filter (all repos returned).
        #[arg(long = "repo", value_name = "NAME", num_args = 1)]
        repo: Vec<String>,

        /// p10-1A-1: filter by code language identifier (lowercase
        /// canonical).  Repeatable or comma-separated.
        /// Examples: `rust`, `python`, `typescript`.
        /// Unknown values produce empty hits.
        #[arg(
            long = "code-lang",
            value_name = "LANG",
            num_args = 1,
            value_delimiter = ','
        )]
        code_lang: Vec<String>,

        /// Phase-2: filter by document source_type
        /// (`markdown`, `note`, `paper`, `reference`, `inbox`).
        /// Repeatable or comma-separated. Empty = no filter.
        /// The clean source/provenance lever for mixed-source KBs.
        #[arg(
            long = "source-type",
            value_name = "TYPE",
            num_args = 1,
            value_delimiter = ','
        )]
        source_type: Vec<String>,

        /// [[workspace.sources]]: filter by source id — the `id` of the
        /// `[[workspace.sources]]` entry a document was ingested from
        /// (e.g. `default`, `notes`, `code`). Repeatable or
        /// comma-separated. Empty = no filter. The named-source
        /// provenance lever for multi-source KBs.
        #[arg(
            long = "source",
            value_name = "ID",
            num_args = 1,
            value_delimiter = ','
        )]
        source: Vec<String>,

        /// p9-fb-37: emit pre-fusion lexical / vector / RRF candidate
        /// lists + per-stage timing in the response. Bypasses cache
        /// (debug intent — fresh run guaranteed). Requires embeddings
        /// when `--mode hybrid` or `--mode vector`; lexical mode runs
        /// without embeddings via a no-op vector stub.
        #[arg(long)]
        trace: bool,

        /// p9-fb-42: bulk multi-query mode. Reads ndjson from stdin —
        /// one JSON object per line, each item shape mirrors the
        /// single-query input. Output is per-query ndjson on stdout
        /// (one `bulk_search_item.v1` per line) plus a summary line on
        /// stderr. Single-query flags (`--mode`, `--k`, `--tag`, etc.)
        /// are ignored when `--bulk` is set; pass them per-item in the
        /// stdin JSON instead. Caps at 100 queries per call.
        #[arg(long)]
        bulk: bool,
    },

    /// Retrieval-augmented question answering.
    Ask {
        query: String,

        #[arg(long, default_value_t = 8)]
        k: usize,

        #[arg(long, value_enum, default_value_t = ModeFlag::Hybrid)]
        mode: ModeFlag,

        #[arg(long)]
        explain: bool,

        #[arg(long)]
        temperature: Option<f32>,

        #[arg(long)]
        seed: Option<u64>,

        /// p9-fb-20: print the `근거:` block (full path / line range
        /// / score, one per line) after the answer. Default on.
        /// `--json` mode is unaffected — citations are always
        /// included in the wire payload regardless of this flag.
        #[arg(long, action = clap::ArgAction::SetTrue,
              conflicts_with = "hide_citations",
              default_value_t = true)]
        show_citations: bool,

        /// p9-fb-20: opt out of the `근거:` block (sticky-overrides
        /// `--show-citations`). Useful when piping the answer body
        /// to another tool that doesn't want trailing metadata.
        #[arg(long)]
        hide_citations: bool,

        /// p9-fb-33: emit ndjson `answer_event.v1` events on stderr
        /// while streaming. Final stdout line is the existing
        /// `answer.v1`. Off by default to preserve final-only behavior.
        #[arg(long)]
        stream: bool,

        /// p9-fb-41: route this ask through the multi-hop pipeline
        /// — the query is decomposed into sub-questions, each
        /// retrieved independently, then synthesized over the
        /// merged chunk pool. Cost trade-off: 2–5× LLM calls
        /// (decompose + 0..N decide + synthesize) vs. single-pass.
        /// Worth it for compound questions (X 와 Y 의 관계, prereq
        /// chain, cross-doc reasoning); single-pass is faster for
        /// simple fact lookups. The full per-hop trace is exposed
        /// on `Answer.hops` in `--json` mode.
        #[arg(long)]
        multi_hop: bool,
    },

    /// Wipe XDG data dirs (and optionally the Lance vector store) so the
    /// workspace can be re-initialised. **Irreversible.** Without
    /// `--yes`, prompts on TTY; aborts in non-interactive contexts.
    Reset {
        /// Wipe config + data + cache + state. Implies losing
        /// `config.toml` — re-run `kebab init` afterwards.
        #[arg(long, group = "reset_scope")]
        all: bool,

        /// Default. Wipe data + cache + state. Config is preserved.
        #[arg(long, group = "reset_scope")]
        data_only: bool,

        /// Wipe only the Lance vector store + truncate
        /// `embedding_records`. SQLite documents / chunks survive so the
        /// next `kebab ingest` re-embeds without re-parsing.
        #[arg(long, group = "reset_scope")]
        vector_only: bool,

        /// Wipe only the config dir.
        #[arg(long, group = "reset_scope")]
        config_only: bool,

        /// Purge stored docs that are outside the current walker scope
        /// (config narrowing / removed sub-directory). No filesystem paths
        /// are removed — this is purely a store-level reconciliation.
        /// Filesystem existence is NOT checked; anything the current walker
        /// would not visit is considered an orphan and removed from the store.
        #[arg(long, group = "reset_scope")]
        orphans_only: bool,

        /// Skip the interactive confirm. Required in non-interactive
        /// contexts (CI, pipes).
        #[arg(long)]
        yes: bool,
    },

    /// Health check.
    Doctor,

    /// Print introspection report (wire schemas, capabilities, model versions, stats).
    Schema,

    /// Eval suite (placeholder; lands in P9).
    Eval {
        #[command(subcommand)]
        what: EvalWhat,
    },

    /// Run the MCP (Model Context Protocol) stdio server. Used by
    /// agent hosts (Claude Code / Cursor / OpenAI Agents) to call kebab
    /// tools (search / ask / schema / doctor).
    Mcp,

    /// Ingest a single file (workspace external paths allowed).
    /// Bytes are copied into `<workspace.root>/_external/<hash>.<ext>`.
    IngestFile {
        /// File path to ingest.
        path: std::path::PathBuf,
    },

    /// Ingest markdown content from stdin. v1 markdown only.
    /// Frontmatter (title + source_uri) is auto-injected.
    IngestStdin {
        /// Title — required, written to frontmatter.
        #[arg(long)]
        title: String,
        /// Source URI — optional, written to frontmatter when present.
        #[arg(long)]
        source_uri: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
enum ConfigWhat {
    /// 기존 config.toml 을 새 스키마로 마이그레이션(빠진 섹션 추가 + 멱등 + .bak 백업).
    Migrate {
        /// 변경만 출력하고 파일은 수정하지 않는다.
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand, Debug)]
enum ListWhat {
    /// List documents currently indexed.
    Docs,
}

#[derive(Subcommand, Debug)]
enum InspectWhat {
    /// Inspect a single document by ID.
    Doc { id: String },
    /// Inspect a single chunk by ID.
    Chunk { id: String },
    /// Corpus-wide OCR statistics (total events, latency percentiles, engine breakdown).
    OcrStats,
    /// Recent OCR failures, optionally filtered by document ID.
    OcrFailures {
        /// Filter failures to a single document UUID.
        #[arg(long)]
        doc_id: Option<String>,
        /// Maximum number of failure rows to return.
        #[arg(long, default_value_t = 10)]
        limit: usize,
    },
}

#[derive(Subcommand, Debug)]
enum FetchWhat {
    /// Fetch a single chunk verbatim, optionally with surrounding context.
    Chunk {
        id: String,
        /// p9-fb-35: include ±N chunks before and after the target.
        #[arg(long)]
        context: Option<u32>,
    },
    /// Fetch the entire normalized markdown text of a document.
    Doc {
        id: String,
        /// p9-fb-35: chars/4 budget cap.
        #[arg(long)]
        max_tokens: Option<usize>,
    },
    /// Fetch a 1-based line range of a document. PDF / audio rejected.
    Span {
        doc_id: String,
        line_start: u32,
        line_end: u32,
        /// p9-fb-35: chars/4 budget cap.
        #[arg(long)]
        max_tokens: Option<usize>,
    },
}

#[derive(Subcommand, Debug)]
enum EvalWhat {
    /// Run the golden suite end-to-end and persist `eval_runs` +
    /// `eval_query_results` + `runs_dir/<run_id>/per_query.jsonl`
    /// (P5-1).
    Run {
        #[arg(long, default_value = "golden")]
        suite: String,
        #[arg(long, value_enum, default_value_t = ModeFlag::Lexical)]
        mode: ModeFlag,
        #[arg(long, default_value_t = 10)]
        k: usize,
        #[arg(long)]
        with_rag: bool,
        #[arg(long)]
        temperature: Option<f32>,
        #[arg(long)]
        seed: Option<u64>,
    },

    /// Compute aggregate metrics for a stored run and write them back
    /// into `eval_runs.aggregate_json` (P5-2).
    Aggregate { run_id: String },

    /// Compute variant-consistency metrics for a stored run and print
    /// a Markdown report (or JSON with `--json`).
    Variants {
        run_id: String,
        #[arg(long)]
        json: bool,
    },

    /// Diff two stored runs (P5-2). Default output is a Markdown
    /// summary; use `--json` (top-level flag) for the raw report.
    Compare {
        run_a: String,
        run_b: String,
        /// Refuse to compare when the two runs' `chunker_version`
        /// differ (default is graceful doc-id fallback).
        #[arg(long)]
        strict_chunker_version: bool,
        /// Also write the Markdown report to
        /// `runs_dir/<run_b>/report.md`.
        #[arg(long)]
        write_report: bool,
    },
}

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
enum ModeFlag {
    Lexical,
    Vector,
    Hybrid,
}

impl From<ModeFlag> for kebab_core::SearchMode {
    fn from(m: ModeFlag) -> Self {
        match m {
            ModeFlag::Lexical => kebab_core::SearchMode::Lexical,
            ModeFlag::Vector => kebab_core::SearchMode::Vector,
            ModeFlag::Hybrid => kebab_core::SearchMode::Hybrid,
        }
    }
}

/// p9-fb-36: clap value enum for `--trust-min`. Maps to
/// `kebab_core::TrustLevel` via `From`.
#[derive(clap::ValueEnum, Clone, Debug)]
enum TrustLevelFlag {
    Primary,
    Secondary,
    Generated,
}

impl From<TrustLevelFlag> for kebab_core::TrustLevel {
    fn from(f: TrustLevelFlag) -> Self {
        match f {
            TrustLevelFlag::Primary => kebab_core::TrustLevel::Primary,
            TrustLevelFlag::Secondary => kebab_core::TrustLevel::Secondary,
            TrustLevelFlag::Generated => kebab_core::TrustLevel::Generated,
        }
    }
}

/// Parse boolean env var accepting "1", "true", "yes", "on" (case-insensitive)
/// as truthy; "0", "false", "no", "off" as falsy. Used for `KEBAB_READONLY`.
fn parse_bool_env(s: &str) -> Result<bool, String> {
    match s.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        other => Err(format!(
            "expected 1/0/true/false/yes/no/on/off, got {other:?}"
        )),
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let level = if cli.debug {
        kebab_app::logging::LogLevel::Debug
    } else if cli.verbose {
        kebab_app::logging::LogLevel::Verbose
    } else {
        kebab_app::logging::LogLevel::Default
    };
    // Fail-soft: if logging init errors (e.g. XDG state dir is read-only),
    // proceed without a guard rather than crashing — `kebab` is still usable.
    let _log_guard = kebab_app::logging::init(level).ok();
    if cli.readonly && is_mutating(&cli.command) {
        let msg = "kebab: readonly mode — mutating commands are disabled";
        if cli.json {
            let v1 = kebab_app::ErrorV1 {
                schema_version: kebab_app::ERROR_V1_ID.to_string(),
                code: "readonly_mode".to_string(),
                message: msg.to_string(),
                details: serde_json::json!({}),
                hint: Some(
                    "remove --readonly (or unset KEBAB_READONLY) to allow writes".to_string(),
                ),
            };
            let v = wire::wire_error_v1(&v1);
            eprintln!(
                "{}",
                serde_json::to_string(&v).unwrap_or_else(|_| msg.to_string())
            );
        } else {
            eprintln!("{msg}");
        }
        return ExitCode::from(1);
    }
    match run(&cli) {
        Ok(()) => ExitCode::from(0),
        Err(e) => {
            let code = exit_code(&e);
            // Refusals at exit code 1 print to stdout (already done by the
            // caller); errors go to stderr.
            if code != 1 {
                if cli.json {
                    let v1 = kebab_app::classify(&e, cli.verbose);
                    let v = wire::wire_error_v1(&v1);
                    eprintln!("{}", serde_json::to_string(&v).unwrap_or_else(|_| {
                        "{\"schema_version\":\"error.v1\",\"code\":\"generic\",\"message\":\"serialize failed\"}".to_string()
                    }));
                } else {
                    eprintln!("error: {e}");
                    if cli.verbose {
                        for cause in e.chain().skip(1) {
                            eprintln!("  caused by: {cause}");
                        }
                    }
                }
            }
            ExitCode::from(code)
        }
    }
}

fn exit_code(err: &anyhow::Error) -> u8 {
    if err.downcast_ref::<RefusalSignal>().is_some() {
        return 1;
    }
    if err.downcast_ref::<NoHitSignal>().is_some() {
        return 1;
    }
    if err.downcast_ref::<DoctorUnhealthy>().is_some() {
        return 3;
    }
    2
}

fn run(cli: &Cli) -> anyhow::Result<()> {
    match &cli.command {
        Cmd::Init { force } => {
            kebab_app::init_workspace(*force)?;
            if !cli.json {
                println!(
                    "created  {}",
                    kebab_config::Config::xdg_config_path().display()
                );
                println!(
                    "created  {}",
                    kebab_config::Config::xdg_data_dir().display()
                );
                println!(
                    "created  {}",
                    kebab_config::Config::xdg_state_dir().display()
                );
                println!("hint     edit the config above, then `kebab ingest`");
                println!(
                    "hint     remote Ollama 사용 시 config 의 `[models.llm] endpoint` 를 갱신 (기본 http://127.0.0.1:11434)"
                );
            }
            Ok(())
        }

        Cmd::Ingest {
            root,
            summary_only,
            force_reingest,
        } => {
            let cfg = kebab_config::Config::load(cli.config.as_deref())?;
            // [[workspace.sources]]: when the user passes `--root <dir>` we pin
            // that single root (one ad-hoc `default` source). Otherwise we
            // leave `scope.root` EMPTY so the app iterates every configured
            // source (`config.resolved_sources()`); a bare empty scope.exclude
            // is fine because each source carries its own merged exclude.
            let scope = match root.clone() {
                Some(r) => kebab_core::SourceScope {
                    root: r,
                    exclude: cfg.workspace.exclude.clone(),
                    ..Default::default()
                },
                None => kebab_core::SourceScope::default(),
            };

            // p9-fb-02: spawn the progress display on a background
            // thread; the ingest call below holds the `Sender` end of
            // the channel and emits per-step events into it. When the
            // call returns, the `Sender` drops and the display thread
            // sees `recv()` return Err — exits cleanly.
            let plain_env = std::env::var("KEBAB_PROGRESS")
                .is_ok_and(|v| v.eq_ignore_ascii_case("plain"));
            let mode = progress::ProgressMode::from_flags(cli.json, cli.quiet, plain_env);

            // Surface the active embedding backend/device on the terminal so the
            // user sees it without grepping kb.log. Suppressed under --json/--quiet.
            if !cli.json && !cli.quiet {
                let backend = match cfg.models.embedding.provider.as_str() {
                    "fastembed" | "onnx" | "" => "fastembed (onnxruntime)",
                    "none" => "비활성 (lexical-only)",
                    other => other,
                };
                eprintln!("임베딩 백엔드: {backend} · 모델 {} ({}-dim)",
                    cfg.models.embedding.model, cfg.models.embedding.dimensions);
            }

            let (tx, rx) = std::sync::mpsc::channel::<kebab_app::IngestEvent>();
            let display_handle =
                std::thread::spawn(move || progress::ProgressDisplay::new(mode).run(rx));

            // p9-fb-04: register a Ctrl-C handler that flips the same
            // AtomicBool the facade polls at each step boundary. The
            // *second* Ctrl-C is a hard exit (handled inside `cancel`).
            let cancel_token = cancel::install_sigint_cancel()?;

            // p9-fb-23: use IngestOpts so force_reingest threads through
            // without churning the positional-arg list.
            let ingest_result = kebab_app::ingest_with_config_opts(
                cfg,
                scope,
                *summary_only,
                kebab_app::IngestOpts {
                    progress: Some(tx),
                    cancel: Some(cancel_token),
                    force_reingest: *force_reingest,
                },
            );

            // Join the display thread *before* surfacing the ingest
            // outcome so the spinner / final newline is flushed
            // regardless of whether ingest returned Ok or Err.
            // join() returns Result<Result<(), anyhow::Error>, Box<dyn Any>>;
            // we discard both — display thread errors / panics are
            // best-effort and must not change ingest's exit code.
            let _ = display_handle.join();

            let report = ingest_result?;
            if cli.json {
                println!("{}", serde_json::to_string(&wire::wire_ingest(&report))?);
            } else {
                let skipped_breakdown =
                    kebab_app::render_skipped_breakdown(&report.skipped_by_extension);
                let purged_suffix = if report.purged_deleted_files > 0 {
                    format!("  purged {}", report.purged_deleted_files)
                } else {
                    String::new()
                };
                println!(
                    "scanned {}  new {}  updated {}  skipped {}{}  errors {}{}  ({} ms)",
                    report.scanned,
                    report.new,
                    report.updated,
                    report.skipped,
                    skipped_breakdown,
                    report.errors,
                    purged_suffix,
                    report.duration_ms
                );
            }
            Ok(())
        }

        Cmd::List { what } => match what {
            ListWhat::Docs => {
                let cfg = kebab_config::Config::load(cli.config.as_deref())?;
                let docs = kebab_app::list_docs_with_config(cfg, kebab_core::DocFilter::default())?;
                if cli.json {
                    println!(
                        "{}",
                        serde_json::to_string(&wire::wire_doc_summaries(&docs))?
                    );
                } else {
                    for d in &docs {
                        println!("{}", wire::format_doc_row(d));
                    }
                }
                Ok(())
            }
        },

        Cmd::Inspect { what } => match what {
            InspectWhat::Doc { id } => {
                let cfg = kebab_config::Config::load(cli.config.as_deref())?;
                let doc_id: kebab_core::DocumentId = id.parse()?;
                let doc = kebab_app::inspect_doc_with_config(cfg, &doc_id)?;
                // Inspect doc emits a `CanonicalDocument` — there's no §2
                // wire schema for it (P1-5 will decide whether this also
                // becomes a tagged wrapper or stays as the raw domain
                // object). Until then keep raw JSON, matching pre-P0-1
                // behaviour.
                println!("{}", serde_json::to_string(&doc)?);
                Ok(())
            }
            InspectWhat::Chunk { id } => {
                let cfg = kebab_config::Config::load(cli.config.as_deref())?;
                let chunk_id: kebab_core::ChunkId = id.parse()?;
                let chunk = kebab_app::inspect_chunk_with_config(cfg, &chunk_id)?;
                println!(
                    "{}",
                    serde_json::to_string(&wire::wire_chunk_inspection(&chunk))?
                );
                Ok(())
            }
            InspectWhat::OcrStats => {
                let cfg = kebab_config::Config::load(cli.config.as_deref())?;
                let app = kebab_app::App::open_with_config(cfg.clone())?;
                let stats = app.inspect_ocr_stats_with_config(&cfg)?;
                println!("{}", serde_json::to_string(&stats)?);
                Ok(())
            }
            InspectWhat::OcrFailures { doc_id, limit } => {
                let cfg = kebab_config::Config::load(cli.config.as_deref())?;
                let app = kebab_app::App::open_with_config(cfg.clone())?;
                let failures =
                    app.inspect_ocr_failures_with_config(&cfg, doc_id.as_deref(), *limit)?;
                println!("{}", serde_json::to_string(&failures)?);
                Ok(())
            }
        },

        Cmd::Fetch { what } => {
            let cfg = kebab_config::Config::load(cli.config.as_deref())?;
            let (query, opts) = match what {
                FetchWhat::Chunk { id, context } => (
                    kebab_core::FetchQuery::Chunk(kebab_core::ChunkId(id.clone())),
                    kebab_core::FetchOpts {
                        context: *context,
                        max_tokens: None,
                    },
                ),
                FetchWhat::Doc { id, max_tokens } => (
                    kebab_core::FetchQuery::Doc(kebab_core::DocumentId(id.clone())),
                    kebab_core::FetchOpts {
                        context: None,
                        max_tokens: *max_tokens,
                    },
                ),
                FetchWhat::Span {
                    doc_id,
                    line_start,
                    line_end,
                    max_tokens,
                } => (
                    kebab_core::FetchQuery::Span {
                        doc_id: kebab_core::DocumentId(doc_id.clone()),
                        line_start: *line_start,
                        line_end: *line_end,
                    },
                    kebab_core::FetchOpts {
                        context: None,
                        max_tokens: *max_tokens,
                    },
                ),
            };
            let result = kebab_app::fetch_with_config(cfg, query, opts)?;
            if cli.json {
                println!(
                    "{}",
                    serde_json::to_string(&wire::wire_fetch_result(&result))?
                );
            } else {
                render_fetch_plain(&result);
            }
            Ok(())
        }

        Cmd::Search {
            query,
            k,
            mode,
            explain: _,
            no_cache: _,
            max_tokens,
            snippet_chars,
            cursor,
            tag,
            lang,
            path_glob,
            trust_min,
            media,
            ingested_after,
            doc_id,
            repo,
            code_lang,
            source_type,
            source,
            trace,
            bulk,
        } => {
            // p9-fb-42: bulk mode — stdin ndjson → bulk_search_with_config
            // → stdout ndjson per query + stderr summary. Single-query
            // flags are ignored (each item supplies its own).
            if *bulk {
                use std::io::{BufRead, Write};

                let cfg = kebab_config::Config::load(cli.config.as_deref())?;

                let stdin = std::io::stdin();
                let stdin_locked = stdin.lock();
                let mut raw_items: Vec<serde_json::Value> = Vec::new();
                for (lineno, line) in stdin_locked.lines().enumerate() {
                    let line = line?;
                    if line.trim().is_empty() {
                        continue;
                    }
                    let v: serde_json::Value = serde_json::from_str(&line).map_err(|e| {
                        anyhow::Error::new(kebab_app::StructuredError(kebab_app::ErrorV1 {
                            schema_version: kebab_app::ERROR_V1_ID.to_string(),
                            code: "config_invalid".to_string(),
                            message: format!("stdin ndjson line {} parse error: {e}", lineno + 1),
                            details: serde_json::Value::Null,
                            hint: Some(
                                "each line must be a JSON object with at least `query`".to_string(),
                            ),
                        }))
                    })?;
                    raw_items.push(v);
                }

                let (items, summary) = kebab_app::bulk_search_with_config(cfg, raw_items)?;

                if cli.json {
                    let mut stdout = std::io::stdout().lock();
                    for item in &items {
                        let v = wire::wire_bulk_search_item(item);
                        writeln!(stdout, "{}", serde_json::to_string(&v)?)?;
                    }
                    eprintln!(
                        "bulk_summary: total={} succeeded={} failed={}",
                        summary.total, summary.succeeded, summary.failed,
                    );
                } else {
                    let mut stdout = std::io::stdout().lock();
                    for (idx, item) in items.iter().enumerate() {
                        writeln!(
                            stdout,
                            "# Query {}: {}",
                            idx + 1,
                            serde_json::to_string(&item.query)?,
                        )?;
                        if let Some(err) = &item.error {
                            writeln!(stdout, "error: {err}")?;
                        } else if let Some(resp) = &item.response {
                            writeln!(stdout, "{}", serde_json::to_string_pretty(resp)?)?;
                        }
                        writeln!(stdout)?;
                    }
                    eprintln!(
                        "bulk_summary: total={} succeeded={} failed={}",
                        summary.total, summary.succeeded, summary.failed,
                    );
                }
                return Ok(());
            }

            let cfg = kebab_config::Config::load(cli.config.as_deref())?;

            // p9-fb-42: bulk mode requires no query; single-query mode requires query.
            let query_text = match query.as_ref() {
                Some(q) if q.trim().is_empty() => {
                    return Err(anyhow::Error::new(kebab_app::StructuredError(
                        kebab_app::ErrorV1 {
                            schema_version: kebab_app::ERROR_V1_ID.to_string(),
                            code: "invalid_input".to_string(),
                            message: "query is empty; provide a non-empty search term or use --bulk".into(),
                            details: serde_json::Value::Null,
                            hint: Some("e.g. `kebab search 'rust async'` or `kebab search --bulk < queries.ndjson`".into()),
                        },
                    )));
                }
                Some(q) => q.clone(),
                None => {
                    return Err(anyhow::anyhow!("query is required unless --bulk is set"));
                }
            };

            // p9-fb-36: normalize --media aliases (md → markdown).
            fn normalize_media_alias(s: &str) -> String {
                match s.to_ascii_lowercase().as_str() {
                    "md" => "markdown".to_string(),
                    other => other.to_string(),
                }
            }
            let media_norm: Vec<String> = media.iter().map(|s| normalize_media_alias(s)).collect();

            // p9-fb-36: parse --ingested-after as RFC3339; structured error on failure.
            let ingested_after_parsed: Option<time::OffsetDateTime> =
                match ingested_after.as_deref() {
                    Some(s) => {
                        match time::OffsetDateTime::parse(
                            s,
                            &time::format_description::well_known::Rfc3339,
                        ) {
                            Ok(ts) => Some(ts),
                            Err(e) => {
                                return Err(anyhow::Error::new(kebab_app::StructuredError(
                                    kebab_app::ErrorV1 {
                                        schema_version: kebab_app::ERROR_V1_ID.to_string(),
                                        code: "config_invalid".to_string(),
                                        message: format!(
                                            "--ingested-after: invalid RFC3339 timestamp '{s}': {e}"
                                        ),
                                        details: serde_json::Value::Null,
                                        hint: Some(
                                            "expected format like 2026-04-01T00:00:00Z".to_string(),
                                        ),
                                    },
                                )));
                            }
                        }
                    }
                    None => None,
                };

            // p9-fb-36 + p10-1A-1: build SearchFilters from CLI flags.
            let filters = kebab_core::SearchFilters {
                tags_any: tag.clone(),
                lang: lang.as_ref().map(|s| kebab_core::Lang(s.clone())),
                path_glob: path_glob.clone(),
                trust_min: trust_min.clone().map(Into::into),
                media: media_norm,
                ingested_after: ingested_after_parsed,
                doc_id: doc_id.as_ref().map(|s| kebab_core::DocumentId(s.clone())),
                repo: repo.clone(),
                code_lang: code_lang.clone(),
                source_type: source_type.clone(),
                source_id: source.clone(),
            };

            let q = kebab_core::SearchQuery {
                text: query_text,
                mode: (*mode).into(),
                k: *k,
                filters,
            };
            let opts = kebab_core::SearchOpts {
                max_tokens: *max_tokens,
                snippet_chars: *snippet_chars,
                cursor: cursor.clone(),
                trace: *trace,
            };
            // p9-fb-34: budget-aware path.
            let app = kebab_app::App::open_with_config(cfg)?;
            let resp = app.search_with_opts(q, opts)?;

            if cli.json {
                println!(
                    "{}",
                    serde_json::to_string(&wire::wire_search_response(&resp))?
                );
            } else {
                // p9-fb-32: prefix `[stale]` on the doc_path for hits
                // whose `stale: true`. Yellow on TTY, plain otherwise —
                // mirrors the warning convention used by the progress
                // renderer (`progress.rs`). Detection uses stdlib
                // `IsTerminal` against stdout (the surface this print
                // lands on); no new dep.
                use std::io::IsTerminal;
                let color = std::io::stdout().is_terminal();
                for h in &resp.hits {
                    // Show 4-digit score so RRF fused scores (bounded
                    // ~0–0.033 for k_rrf=60) don't all collapse to "0.02".
                    // Append heading_path so multiple chunks from the same
                    // document are distinguishable on a single line.
                    let heading = if h.heading_path.is_empty() {
                        String::new()
                    } else {
                        format!("  >  {}", h.heading_path.join(" / "))
                    };
                    let stale_tag = if h.stale {
                        if color {
                            "\x1b[33m[stale]\x1b[0m "
                        } else {
                            "[stale] "
                        }
                    } else {
                        ""
                    };
                    println!(
                        "{:>2}. {:.4}  {}{}{}",
                        h.rank, h.retrieval.fusion_score, stale_tag, h.doc_path.0, heading,
                    );
                }
                // p9-fb-34: truncation hint goes to stderr so it
                // doesn't pollute the stdout hit list.
                if resp.truncated {
                    let next = resp.next_cursor.as_deref().unwrap_or("(none)");
                    eprintln!("[truncated; use --cursor {next} for the next page]");
                }
                // v0.17.0 A5 Step 4: short-query advisory. `resp.hint`
                // is `Some` only when the result list is empty and the
                // trimmed query is shorter than the trigram tokenizer
                // can resolve (raw FTS5 mode opts out). stderr so it
                // doesn't pollute the stdout hit list. `--json` skips
                // this branch entirely; the field rides the wire.
                if let Some(hint) = &resp.hint {
                    eprintln!("[hint] {hint}");
                }
                if *trace {
                    if let Some(t) = &resp.trace {
                        eprintln!();
                        eprintln!("Trace:");
                        eprintln!(
                            "  lexical ({} hits, {}ms):",
                            t.lexical.len(),
                            t.timing.lexical_ms
                        );
                        for c in t.lexical.iter().take(3) {
                            eprintln!(
                                "    rank={} score={:.4} chunk={}",
                                c.rank, c.score, c.chunk_id.0
                            );
                        }
                        eprintln!(
                            "  vector ({} hits, {}ms):",
                            t.vector.len(),
                            t.timing.vector_ms
                        );
                        for c in t.vector.iter().take(3) {
                            eprintln!(
                                "    rank={} score={:.4} chunk={}",
                                c.rank, c.score, c.chunk_id.0
                            );
                        }
                        eprintln!(
                            "  fusion ({} inputs, {}ms)",
                            t.rrf_inputs.len(),
                            t.timing.fusion_ms
                        );
                        eprintln!("  total: {}ms", t.timing.total_ms);
                    }
                }
            }
            Ok(())
        }

        Cmd::Ask {
            query,
            k,
            mode,
            explain,
            temperature,
            seed,
            show_citations,
            hide_citations,
            stream,
            multi_hop,
        } => {
            let cfg = kebab_config::Config::load(cli.config.as_deref())?;
            if query.trim().is_empty() {
                return Err(anyhow::Error::new(kebab_app::StructuredError(
                    kebab_app::ErrorV1 {
                        schema_version: kebab_app::ERROR_V1_ID.to_string(),
                        code: "invalid_input".to_string(),
                        message: "query is empty; provide a non-empty prompt".into(),
                        details: serde_json::Value::Null,
                        hint: Some("e.g. `kebab ask \"explain this code\"`".into()),
                    },
                )));
            }
            if *stream {
                // p9-fb-33: streaming branch. Background thread runs
                // ask_with_config (which calls into the rag pipeline);
                // main thread drains the receiver and writes
                // `answer_event.v1` ndjson to stderr. On BrokenPipe
                // (downstream consumer closed), drop the receiver so
                // the worker's next `send` returns SendError →
                // pipeline cancels with LlmStreamAborted. Final stdout
                // line is the existing `answer.v1` (mirrors
                // ingest_progress.v1 + ingest_report.v1 split).
                use std::io::Write;
                use std::sync::mpsc;

                let (tx, rx) = mpsc::channel::<kebab_app::StreamEvent>();
                let opts = kebab_app::AskOpts {
                    k: *k,
                    explain: *explain,
                    mode: (*mode).into(),
                    temperature: *temperature,
                    seed: *seed,
                    stream_sink: Some(tx),
                    multi_hop: *multi_hop,
                };
                let cfg2 = cfg.clone();
                let q = query.clone();
                let handle = std::thread::spawn(move || -> anyhow::Result<kebab_core::Answer> {
                    kebab_app::ask_with_config(cfg2, &q, opts)
                });

                // Drain receiver, write ndjson to stderr until
                // completion or BrokenPipe.
                let mut cancelled_pipe = false;
                {
                    let mut stderr = std::io::stderr().lock();
                    for ev in &rx {
                        let now = time::OffsetDateTime::now_utc();
                        let v = wire::wire_answer_event(&ev, now);
                        let line = serde_json::to_string(&v)?;
                        if let Err(e) = writeln!(stderr, "{line}") {
                            if e.kind() == std::io::ErrorKind::BrokenPipe {
                                cancelled_pipe = true;
                                break;
                            }
                            return Err(e.into());
                        }
                    }
                }
                if cancelled_pipe {
                    // Dropping the receiver signals to the worker —
                    // the next `send` returns SendError, which the
                    // pipeline interprets as a cancel.
                    drop(rx);
                }

                let result = handle
                    .join()
                    .map_err(|_| anyhow::anyhow!("ask worker panicked"))?;
                let ans = result?;

                // Final stdout line — answer.v1 for backwards
                // compat. BrokenPipe on stdout is silent (caller
                // already gone).
                let final_json = serde_json::to_string(&wire::wire_answer(&ans))?;
                let _ = writeln!(std::io::stdout().lock(), "{final_json}");

                if !ans.grounded {
                    return Err(RefusalSignal.into());
                }
                Ok(())
            } else {
                let opts = kebab_app::AskOpts {
                    k: *k,
                    explain: *explain,
                    mode: (*mode).into(),
                    temperature: *temperature,
                    seed: *seed,
                    // CLI ask is non-streaming by default (the answer
                    // prints all at once on completion). `--stream`
                    // takes the branch above; the TUI ask pane (P9-3)
                    // wires up its own `mpsc::Sender`.
                    stream_sink: None,
                    multi_hop: *multi_hop,
                };
                let ans = kebab_app::ask_with_config(cfg, query, opts)?;
                if cli.json {
                    println!("{}", serde_json::to_string(&wire::wire_answer(&ans))?);
                } else {
                    println!("{}", ans.answer);
                    // p9-fb-20: print the citation block after the
                    // answer body when --hide-citations is not set
                    // (--show-citations is the default). Skipped on
                    // refusal-with-zero-citations to avoid an empty
                    // `근거:` header.
                    let print_citations = *show_citations && !*hide_citations;
                    if print_citations && !ans.citations.is_empty() {
                        // p9-fb-32: yellow `[stale]` prefix on TTY (mirrors
                        // the search renderer's pattern in `Cmd::Search`).
                        use std::io::IsTerminal;
                        let color = std::io::stdout().is_terminal();
                        let mut out = std::io::stdout().lock();
                        render_ask_plain_citations(&mut out, &ans, color)?;
                    }
                }
                // Refusal → exit 1.
                if !ans.grounded {
                    return Err(RefusalSignal.into());
                }
                Ok(())
            }
        }

        Cmd::Reset {
            all,
            data_only: _,
            vector_only,
            config_only,
            orphans_only,
            yes,
        } => {
            use kebab_app::ResetScope;
            // `--data-only` explicit OR no scope flag at all → DataOnly.
            // The `data_only: _` binding above is intentional — clap's
            // `group = "reset_scope"` already enforces mutual exclusion,
            // so the flag's presence does not change the resolved scope.
            let scope = if *all {
                ResetScope::All
            } else if *vector_only {
                ResetScope::VectorOnly
            } else if *config_only {
                ResetScope::ConfigOnly
            } else if *orphans_only {
                ResetScope::OrphansOnly
            } else {
                ResetScope::DataOnly
            };

            let cfg = kebab_config::Config::load(cli.config.as_deref())?;

            if matches!(scope, ResetScope::OrphansOnly) {
                // OrphansOnly: confirm UI shows orphan count + sample paths
                // rather than on-disk directory sizes.
                let orphan_paths = kebab_app::enumerate_orphans(&cfg)?;

                if !*yes {
                    use std::io::IsTerminal;
                    if !std::io::stdin().is_terminal() {
                        anyhow::bail!(
                            "reset --orphans-only is destructive and stdin is non-interactive — pass --yes to proceed"
                        );
                    }
                    if !confirm_orphans_only(&orphan_paths)? {
                        if !cli.quiet {
                            eprintln!("aborted.");
                        }
                        return Ok(());
                    }
                }

                let report = kebab_app::reset::execute(scope, &cfg)?;
                if cli.json {
                    println!("{}", serde_json::to_string(&wire::wire_reset(&report))?);
                } else if report.orphans_purged > 0 {
                    println!("orphans purged: {}", report.orphans_purged);
                    for p in &report.purged_paths {
                        println!("  - {}", p.0);
                    }
                } else {
                    println!("no orphaned docs found — store is already in sync with walker scope");
                }
                return Ok(());
            }

            let paths = kebab_app::reset::enumerate_paths(scope, &cfg);
            let bytes = kebab_app::reset::estimate_size_bytes(&paths);

            if !*yes {
                use std::io::IsTerminal;
                if !std::io::stdin().is_terminal() {
                    anyhow::bail!(
                        "reset is destructive and stdin is non-interactive — pass --yes to proceed"
                    );
                }
                if !confirm_destructive(scope, &paths, bytes)? {
                    if !cli.quiet {
                        eprintln!("aborted.");
                    }
                    return Ok(());
                }
            }

            let report = kebab_app::reset::execute(scope, &cfg)?;
            if cli.json {
                println!("{}", serde_json::to_string(&wire::wire_reset(&report))?);
            } else {
                println!(
                    "removed {} path(s); embedding_rows_truncated={}",
                    report.removed_paths.len(),
                    report.embedding_rows_truncated
                );
                for p in &report.removed_paths {
                    println!("  - {}", p.display());
                }
                if matches!(scope, ResetScope::All | ResetScope::ConfigOnly) {
                    println!("hint: run `kebab init` to recreate config.toml");
                }
            }
            Ok(())
        }

        Cmd::Schema => {
            let cfg = kebab_config::Config::load(cli.config.as_deref())?;
            let report = kebab_app::schema_with_config(&cfg)?;
            if cli.json {
                let v = wire::wire_schema(&report);
                println!("{}", serde_json::to_string(&v)?);
            } else {
                print_schema_text(&report);
            }
            Ok(())
        }

        Cmd::Config { what } => match what {
            ConfigWhat::Migrate { dry_run } => {
                let report =
                    kebab_app::config_migrate_with_config_path(cli.config.as_deref(), *dry_run)?;
                if cli.json {
                    println!(
                        "{}",
                        serde_json::to_string(&wire::wire_config_migration(&report))?
                    );
                } else if !report.changed {
                    println!(
                        "config 이미 최신입니다 (schema v{}).",
                        report.to_schema_version
                    );
                } else {
                    let verb = if report.dry_run { "변경 예정" } else { "적용됨" };
                    println!(
                        "config 마이그레이션 {verb}: v{} → v{} ({} changes)",
                        report.from_schema_version,
                        report.to_schema_version,
                        report.changes.len()
                    );
                    for c in &report.changes {
                        println!("  - [{:?}] {} — {}", c.kind, c.path, c.detail);
                    }
                    if let Some(bak) = &report.backup_path {
                        println!("백업: {bak}");
                    }
                    if report.dry_run {
                        println!("(--dry-run: 파일 미수정. 적용하려면 --dry-run 없이 재실행)");
                    }
                }
                Ok(())
            }
        },

        Cmd::Doctor => {
            let report = kebab_app::doctor_with_config_path(cli.config.as_deref())?;
            if cli.json {
                println!("{}", serde_json::to_string(&wire::wire_doctor(&report))?);
            } else {
                for c in &report.checks {
                    let mark = if c.ok { "✓" } else { "✗" };
                    println!("{mark} {:<20} {}", c.name, c.detail);
                    if let (false, Some(hint)) = (c.ok, c.hint.as_ref()) {
                        println!("  hint: {hint}");
                    }
                }
                if !report.ok {
                    println!();
                    let failed = report.checks.iter().filter(|c| !c.ok).count();
                    println!("{failed} check(s) failed.");
                }
            }
            if !report.ok {
                return Err(DoctorUnhealthy.into());
            }
            Ok(())
        }

        Cmd::Eval { what } => {
            let cfg = kebab_config::Config::load(cli.config.as_deref())?;
            match what {
                EvalWhat::Run {
                    suite,
                    mode,
                    k,
                    with_rag,
                    temperature,
                    seed,
                } => {
                    let opts = kebab_eval::EvalRunOpts {
                        suite: suite.clone(),
                        mode: (*mode).into(),
                        with_rag: *with_rag,
                        k: *k,
                        temperature: *temperature,
                        seed: *seed,
                    };
                    let run = kebab_eval::run_eval_with_config(&cfg, &opts)?;
                    if cli.json {
                        println!("{}", serde_json::to_string_pretty(&run)?);
                    } else {
                        println!("run_id: {}", run.run_id);
                        println!("queries: {}", run.per_query.len());
                        let failed = run.per_query.iter().filter(|q| q.error.is_some()).count();
                        println!("failed:  {failed}");
                    }
                    Ok(())
                }

                EvalWhat::Aggregate { run_id } => {
                    let agg = kebab_eval::compute_aggregate_with_config(&cfg, run_id)?;
                    kebab_eval::store_aggregate_with_config(&cfg, run_id, &agg)?;
                    if cli.json {
                        println!("{}", serde_json::to_string_pretty(&agg)?);
                    } else {
                        println!("run_id: {run_id}");
                        println!(
                            "queries: {} ({} failed)",
                            agg.total_queries, agg.failed_queries
                        );
                        println!(
                            "hit@1:   {:.4}",
                            agg.hit_at_k.get(&1).copied().unwrap_or(0.0)
                        );
                        println!(
                            "hit@5:   {:.4}",
                            agg.hit_at_k.get(&5).copied().unwrap_or(0.0)
                        );
                        println!("MRR:     {:.4}", agg.mrr);
                    }
                    Ok(())
                }

                EvalWhat::Variants { run_id, json } => {
                    let rep = kebab_eval::compute_variant_consistency_with_config(&cfg, run_id)?;
                    if *json {
                        println!("{}", serde_json::to_string_pretty(&rep)?);
                    } else {
                        print!("{}", kebab_eval::render_variants_md(&rep));
                    }
                    Ok(())
                }

                EvalWhat::Compare {
                    run_a,
                    run_b,
                    strict_chunker_version,
                    write_report,
                } => {
                    let opts = kebab_eval::CompareOpts {
                        strict_chunker_version: *strict_chunker_version,
                    };
                    let report = kebab_eval::compare_runs_with_config(&cfg, run_a, run_b, &opts)?;
                    let md = kebab_eval::render_report_md(&report);
                    if cli.json {
                        println!("{}", serde_json::to_string_pretty(&report)?);
                    } else {
                        print!("{md}");
                    }
                    if *write_report {
                        let resolved_data_dir =
                            kebab_config::expand_path(&cfg.storage.data_dir, "");
                        let runs_dir = kebab_config::expand_path(
                            &cfg.storage.runs_dir,
                            &resolved_data_dir.to_string_lossy(),
                        );
                        let dir = runs_dir.join(run_b);
                        std::fs::create_dir_all(&dir)?;
                        let path = dir.join("report.md");
                        std::fs::write(&path, &md)?;
                        if !cli.json {
                            eprintln!("wrote {}", path.display());
                        }
                    }
                    Ok(())
                }
            }
        }

        Cmd::IngestFile { path } => {
            let cfg = kebab_config::Config::load(cli.config.as_deref())?;
            let report = kebab_app::ingest_file_with_config(cfg, path)?;
            if cli.json {
                let v = wire::wire_ingest(&report);
                println!("{}", serde_json::to_string(&v)?);
            } else {
                println!(
                    "ingest-file: scanned={} new={} updated={} unchanged={} skipped={} errors={}",
                    report.scanned,
                    report.new,
                    report.updated,
                    report.unchanged,
                    report.skipped,
                    report.errors
                );
            }
            Ok(())
        }

        Cmd::IngestStdin { title, source_uri } => {
            use std::io::Read;
            let mut body = String::new();
            std::io::stdin()
                .read_to_string(&mut body)
                .context("kebab ingest-stdin: read stdin")?;
            let cfg = kebab_config::Config::load(cli.config.as_deref())?;
            let report =
                kebab_app::ingest_stdin_with_config(cfg, &body, title, source_uri.as_deref())?;
            if cli.json {
                let v = wire::wire_ingest(&report);
                println!("{}", serde_json::to_string(&v)?);
            } else {
                println!(
                    "ingest-stdin: scanned={} new={} updated={} unchanged={} skipped={} errors={}",
                    report.scanned,
                    report.new,
                    report.updated,
                    report.unchanged,
                    report.skipped,
                    report.errors
                );
            }
            Ok(())
        }

        Cmd::Mcp => {
            let cfg = kebab_config::Config::load(cli.config.as_deref())?;
            kebab_mcp::serve_stdio(cfg, cli.config.clone())
        }
    }
}

/// p9-fb-32: render the plain (non-JSON) citation block for `kebab ask`.
/// Mirrors the `Cmd::Search` plain renderer's `[stale]` convention —
/// yellow ANSI on TTY, plain text otherwise. Detection uses stdlib
/// `IsTerminal` at the call site; this function takes the resolved
/// `color` boolean so tests can pin both branches deterministically.
///
/// Skipping the empty / no-citation path is the caller's responsibility
/// (matches the original inline guard at the call site).
fn render_ask_plain_citations(
    w: &mut impl std::io::Write,
    ans: &kebab_core::Answer,
    color: bool,
) -> std::io::Result<()> {
    writeln!(w)?;
    writeln!(w, "근거:")?;
    for (idx, c) in ans.citations.iter().enumerate() {
        let marker = c.marker.clone().unwrap_or_else(|| format!("{}", idx + 1));
        // p9-fb-32: `[stale]` prefix on the URI for citations whose
        // `stale: true`. Yellow on TTY, plain otherwise — mirrors the
        // search-plain renderer in `Cmd::Search`.
        let stale_tag = if c.stale {
            if color {
                "\x1b[33m[stale]\x1b[0m "
            } else {
                "[stale] "
            }
        } else {
            ""
        };
        writeln!(w, "  [{}] {}{}", marker, stale_tag, c.citation.to_uri())?;
    }
    // p9-fb-20: retrieval 메타는 citation 별 점수가 AnswerCitation 에
    // 없는 (`top_score` 만 retrieval-전체 max) 한계상 한 줄로 분리.
    // per-citation score 노출은 facade + AnswerCitation 의 미래 확장 후.
    writeln!(
        w,
        "(retrieval: top_score={:.2}, k={}, used={}/{})",
        ans.retrieval.top_score,
        ans.retrieval.k,
        ans.retrieval.chunks_used,
        ans.retrieval.chunks_returned,
    )?;
    Ok(())
}

fn print_schema_text(s: &kebab_app::SchemaV1) {
    println!("kebab v{}", s.kebab_version);
    println!();

    println!("wire schemas");
    println!("  {}", s.wire.schemas.join(", "));
    println!();

    println!("capabilities");
    let caps = [
        ("json_mode", s.capabilities.json_mode),
        ("ingest_progress", s.capabilities.ingest_progress),
        ("ingest_cancellation", s.capabilities.ingest_cancellation),
        ("rag_multi_turn", s.capabilities.rag_multi_turn),
        ("search_cache", s.capabilities.search_cache),
        ("incremental_ingest", s.capabilities.incremental_ingest),
        ("streaming_ask", s.capabilities.streaming_ask),
        ("http_daemon", s.capabilities.http_daemon),
        ("mcp_server", s.capabilities.mcp_server),
        ("single_file_ingest", s.capabilities.single_file_ingest),
        ("bulk_search", s.capabilities.bulk_search),
    ];
    for (name, on) in caps {
        let mark = if on { "✓" } else { "✗" };
        println!("  {mark} {name}");
    }
    println!();

    println!("models");
    println!("  parser_version          {}", s.models.parser_version);
    println!("  chunker_version         {}", s.models.chunker_version);
    println!("  embedding_version       {}", s.models.embedding_version);
    println!(
        "  prompt_template_version {}",
        s.models.prompt_template_version
    );
    println!("  index_version           {}", s.models.index_version);
    println!("  corpus_revision         {}", s.models.corpus_revision);
    println!();

    println!("stats");
    println!("  doc_count               {}", s.stats.doc_count);
    println!("  chunk_count             {}", s.stats.chunk_count);
    println!("  asset_count             {}", s.stats.asset_count);
    let last = s.stats.last_ingest_at.as_deref().unwrap_or("(never)");
    println!("  last_ingest_at          {last}");
}

fn is_mutating(cmd: &Cmd) -> bool {
    matches!(
        cmd,
        Cmd::Ingest { .. } | Cmd::IngestFile { .. } | Cmd::IngestStdin { .. } | Cmd::Reset { .. }
    )
}

/// Minimal stdin/stdout confirm prompt for destructive ops. No new dep —
/// uses stdlib `IsTerminal` (the caller is expected to have already
/// short-circuited the non-TTY case). Returns `Ok(true)` only when the
/// user types `y` / `Y` / `yes`. Empty input or anything else → `false`
/// (safe default).
fn confirm_destructive(
    scope: kebab_app::ResetScope,
    paths: &[std::path::PathBuf],
    bytes: u64,
) -> anyhow::Result<bool> {
    use std::io::Write;
    let mut out = std::io::stderr().lock();
    writeln!(out, "kebab reset ({scope:?}): about to remove")?;
    for p in paths {
        writeln!(out, "  - {}", p.display())?;
    }
    writeln!(out, "estimated total: {bytes} bytes")?;
    write!(out, "Proceed? [y/N] ")?;
    out.flush()?;

    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    let s = line.trim().to_ascii_lowercase();
    Ok(matches!(s.as_str(), "y" | "yes"))
}

/// Confirm prompt for `--orphans-only`: shows the orphan count + a
/// sample of up to 5 paths so the user knows what will be purged before
/// committing. No filesystem paths are removed — only store records.
fn confirm_orphans_only(orphan_paths: &[kebab_core::WorkspacePath]) -> anyhow::Result<bool> {
    use std::io::Write;
    let n = orphan_paths.len();
    let mut out = std::io::stderr().lock();

    if n == 0 {
        writeln!(out, "no orphaned docs found — nothing to purge.")?;
        out.flush()?;
        // Nothing to do; treat as confirmed so the caller can emit the
        // "no orphans" report without prompting.
        return Ok(true);
    }

    let sample: Vec<&str> = orphan_paths.iter().take(5).map(|p| p.0.as_str()).collect();
    let sample_str = sample.join(", ");
    let ellipsis = if n > 5 { ", …" } else { "" };

    writeln!(
        out,
        "Purge {n} stored doc(s) outside the current walker scope? (no filesystem paths removed)"
    )?;
    writeln!(out, "  sample: {sample_str}{ellipsis}")?;
    write!(out, "[y/N] ")?;
    out.flush()?;

    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    let s = line.trim().to_ascii_lowercase();
    Ok(matches!(s.as_str(), "y" | "yes"))
}

/// p9-fb-35: human-friendly plain output for `kebab fetch`.
fn render_fetch_plain(r: &kebab_core::FetchResult) {
    println!("# {} ({})", r.doc_path.0, format_kind(r.kind));
    if r.stale {
        println!("[stale; indexed_at = {}]", r.indexed_at);
    }
    match r.kind {
        kebab_core::FetchKind::Chunk => {
            if !r.context_before.is_empty() {
                println!("\n=== before ===");
                for c in &r.context_before {
                    let heading = c
                        .heading_path
                        .last()
                        .map_or("", std::string::String::as_str);
                    println!("[{} § {}]\n{}\n", c.chunk_id.0, heading, c.text);
                }
            }
            if let Some(c) = &r.chunk {
                println!("\n=== target ===");
                let heading = c
                    .heading_path
                    .last()
                    .map_or("", std::string::String::as_str);
                println!("[{} § {}]\n{}\n", c.chunk_id.0, heading, c.text);
            }
            if !r.context_after.is_empty() {
                println!("\n=== after ===");
                for c in &r.context_after {
                    let heading = c
                        .heading_path
                        .last()
                        .map_or("", std::string::String::as_str);
                    println!("[{} § {}]\n{}\n", c.chunk_id.0, heading, c.text);
                }
            }
        }
        kebab_core::FetchKind::Doc | kebab_core::FetchKind::Span => {
            if let Some(text) = &r.text {
                println!("\n{text}");
            }
            if r.truncated {
                eprintln!("[truncated; widen --max-tokens for fuller text]");
            }
        }
    }
}

fn format_kind(k: kebab_core::FetchKind) -> &'static str {
    match k {
        kebab_core::FetchKind::Chunk => "chunk",
        kebab_core::FetchKind::Doc => "doc",
        kebab_core::FetchKind::Span => "span",
    }
}

#[cfg(test)]
mod tests {
    //! p9-fb-32: unit tests for `render_ask_plain_citations`. The
    //! integration end-to-end (`tests/wire_ask_stale.rs`) is gated on
    //! a real Ollama, so we cover the renderer's `[stale]` logic here
    //! against a synthetic `Answer` instead.
    use super::*;
    use kebab_core::{
        Answer, AnswerCitation, AnswerRetrievalSummary, Citation, ModelRef, PromptTemplateVersion,
        SearchMode, TokenUsage, TraceId, WorkspacePath,
    };
    use time::OffsetDateTime;

    fn mk_answer(citations: Vec<AnswerCitation>) -> Answer {
        Answer {
            answer: "ans".into(),
            citations,
            grounded: true,
            refusal_reason: None,
            model: ModelRef {
                id: "test".into(),
                provider: "test".into(),
                dimensions: None,
            },
            embedding: None,
            prompt_template_version: PromptTemplateVersion("rag-v2".into()),
            retrieval: AnswerRetrievalSummary {
                trace_id: TraceId("ret_test".into()),
                mode: SearchMode::Lexical,
                k: 5,
                score_gate: 0.30,
                top_score: 0.80,
                chunks_returned: 1,
                chunks_used: 1,
            },
            usage: TokenUsage {
                prompt_tokens: 0,
                completion_tokens: 0,
                latency_ms: 0,
            },
            created_at: OffsetDateTime::now_utc(),
            hops: None,
            verification: None,
        }
    }

    fn mk_citation(path: &str, stale: bool) -> AnswerCitation {
        AnswerCitation {
            marker: Some("1".into()),
            citation: Citation::Line {
                path: WorkspacePath::new(path.into()).unwrap(),
                start: 1,
                end: 1,
                section: None,
            },
            indexed_at: OffsetDateTime::now_utc(),
            stale,
        }
    }

    #[test]
    fn plain_marks_stale_citation_no_color() {
        let ans = mk_answer(vec![mk_citation("a.md", true)]);
        let mut buf = Vec::new();
        render_ask_plain_citations(&mut buf, &ans, false).unwrap();
        let out = String::from_utf8(buf).unwrap();
        assert!(
            out.contains("[stale]"),
            "expected `[stale]` marker in plain output, got:\n{out}"
        );
        // No ANSI when color = false.
        assert!(
            !out.contains("\x1b["),
            "unexpected ANSI escape in non-color output:\n{out}"
        );
    }

    #[test]
    fn plain_marks_stale_citation_color_uses_yellow_ansi() {
        let ans = mk_answer(vec![mk_citation("a.md", true)]);
        let mut buf = Vec::new();
        render_ask_plain_citations(&mut buf, &ans, true).unwrap();
        let out = String::from_utf8(buf).unwrap();
        // Yellow ANSI + reset around the `[stale]` token, mirroring the
        // search-plain renderer in `Cmd::Search`.
        assert!(
            out.contains("\x1b[33m[stale]\x1b[0m"),
            "expected yellow [stale] ANSI sequence in color output, got:\n{out:?}"
        );
    }

    #[test]
    fn plain_no_stale_tag_for_fresh_citation() {
        let ans = mk_answer(vec![mk_citation("a.md", false)]);
        let mut buf = Vec::new();
        render_ask_plain_citations(&mut buf, &ans, true).unwrap();
        let out = String::from_utf8(buf).unwrap();
        assert!(
            !out.contains("[stale]"),
            "unexpected `[stale]` marker for fresh citation:\n{out}"
        );
    }
}
