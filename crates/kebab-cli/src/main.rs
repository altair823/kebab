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

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Initialise XDG dirs + workspace + `config.toml`.
    Init {
        /// Overwrite an existing `config.toml`.
        #[arg(long)]
        force: bool,
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

    /// Lexical / vector / hybrid search over chunks.
    Search {
        query: String,

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

        /// p9-fb-18: persistent multi-turn chat session id. First call
        /// auto-creates the session in SQLite (`chat_sessions`), each
        /// subsequent call with the same id loads prior turns as
        /// history and appends the new Q/A. Without this flag, ask
        /// is single-shot (no persistence). The session id is
        /// caller-supplied — pick anything stable per conversation
        /// (e.g. `kebab-rust-async-2026-05`).
        #[arg(long, value_name = "ID")]
        session: Option<String>,
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

        /// Skip the interactive confirm. Required in non-interactive
        /// contexts (CI, pipes).
        #[arg(long)]
        yes: bool,
    },

    /// Health check.
    Doctor,

    /// Print introspection report (wire schemas, capabilities, model versions, stats).
    Schema,

    /// Launch the Ratatui shell (P9-1 — Library pane only; search /
    /// ask / inspect panes land with p9-2 / p9-3 / p9-4).
    Tui,

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

/// Parse boolean env var accepting "1", "true", "yes", "on" (case-insensitive)
/// as truthy; "0", "false", "no", "off" as falsy. Used for `KEBAB_READONLY`.
fn parse_bool_env(s: &str) -> Result<bool, String> {
    match s.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        other => Err(format!("expected 1/0/true/false/yes/no/on/off, got {other:?}")),
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
                println!("created  {}", kebab_config::Config::xdg_data_dir().display());
                println!("created  {}", kebab_config::Config::xdg_state_dir().display());
                println!("hint     edit the config above, then `kebab ingest`");
            }
            Ok(())
        }

        Cmd::Ingest {
            root,
            summary_only,
            force_reingest,
        } => {
            let cfg = kebab_config::Config::load(cli.config.as_deref())?;
            let scope = kebab_core::SourceScope {
                root: root.clone().unwrap_or_else(|| PathBuf::from(&cfg.workspace.root)),
                exclude: cfg.workspace.exclude.clone(),
                ..Default::default()
            };

            // p9-fb-02: spawn the progress display on a background
            // thread; the ingest call below holds the `Sender` end of
            // the channel and emits per-step events into it. When the
            // call returns, the `Sender` drops and the display thread
            // sees `recv()` return Err — exits cleanly.
            let plain_env = std::env::var("KEBAB_PROGRESS")
                .map(|v| v.eq_ignore_ascii_case("plain"))
                .unwrap_or(false);
            let mode = progress::ProgressMode::from_flags(cli.json, cli.quiet, plain_env);
            let (tx, rx) = std::sync::mpsc::channel::<kebab_app::IngestEvent>();
            let display_handle = std::thread::spawn(move || {
                progress::ProgressDisplay::new(mode).run(rx)
            });

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
                let skipped_breakdown = kebab_app::render_skipped_breakdown(&report.skipped_by_extension);
                println!(
                    "scanned {}  new {}  updated {}  skipped {}{}  errors {}  ({} ms)",
                    report.scanned,
                    report.new,
                    report.updated,
                    report.skipped,
                    skipped_breakdown,
                    report.errors,
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
                    println!("{}", serde_json::to_string(&wire::wire_doc_summaries(&docs))?);
                } else {
                    for d in &docs {
                        println!("{}\t{}", d.doc_id, d.doc_path.0);
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
                println!("{}", serde_json::to_string(&wire::wire_chunk_inspection(&chunk))?);
                Ok(())
            }
        },

        Cmd::Search {
            query,
            k,
            mode,
            explain: _,
            no_cache,
        } => {
            let cfg = kebab_config::Config::load(cli.config.as_deref())?;
            let q = kebab_core::SearchQuery {
                text: query.clone(),
                mode: (*mode).into(),
                k: *k,
                filters: kebab_core::SearchFilters::default(),
            };
            // p9-fb-19: --no-cache routes to the uncached facade.
            // Both calls go through the same App; only the cache
            // lookup/insert is skipped.
            let hits = if *no_cache {
                kebab_app::search_uncached_with_config(cfg, q)?
            } else {
                kebab_app::search_with_config(cfg, q)?
            };
            if cli.json {
                println!("{}", serde_json::to_string(&wire::wire_search_hits(&hits))?);
            } else {
                // p9-fb-32: prefix `[stale]` on the doc_path for hits
                // whose `stale: true`. Yellow on TTY, plain otherwise —
                // mirrors the warning convention used by the progress
                // renderer (`progress.rs`). Detection uses stdlib
                // `IsTerminal` against stdout (the surface this print
                // lands on); no new dep.
                use std::io::IsTerminal;
                let color = std::io::stdout().is_terminal();
                for h in &hits {
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
                        h.rank,
                        h.retrieval.fusion_score,
                        stale_tag,
                        h.doc_path.0,
                        heading,
                    );
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
            session,
        } => {
            let cfg = kebab_config::Config::load(cli.config.as_deref())?;
            let opts = kebab_app::AskOpts {
                k: *k,
                explain: *explain,
                mode: (*mode).into(),
                temperature: *temperature,
                seed: *seed,
                // CLI ask is non-streaming today (the answer prints all at
                // once on completion). The TUI ask pane (P9-3) is what
                // wires up a real `mpsc::Sender` here.
                stream_sink: None,
                // p9-fb-18: when `--session` is set, the facade
                // (`ask_with_session_with_config`) loads prior turns
                // from SQLite and stuffs them into AskOpts.history
                // before calling `ask_with_history`. Single-shot path
                // (no `--session`) keeps the empty defaults.
                history: Vec::new(),
                conversation_id: None,
                turn_index: None,
            };
            let ans = match session.as_deref() {
                Some(sid) => kebab_app::ask_with_session_with_config(cfg, sid, query, opts)?,
                None => kebab_app::ask_with_config(cfg, query, opts)?,
            };
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
                    println!();
                    println!("근거:");
                    for (idx, c) in ans.citations.iter().enumerate() {
                        let marker = c
                            .marker
                            .clone()
                            .unwrap_or_else(|| format!("{}", idx + 1));
                        println!("  [{}] {}", marker, c.citation.to_uri());
                    }
                    // p9-fb-20: retrieval 메타는 citation 별 점수가
                    // AnswerCitation 에 없는 (`top_score` 만 retrieval-
                    // 전체 max) 한계상 한 줄로 분리. per-citation score
                    // 노출은 facade + AnswerCitation 의 미래 확장 후.
                    println!(
                        "(retrieval: top_score={:.2}, k={}, used={}/{})",
                        ans.retrieval.top_score,
                        ans.retrieval.k,
                        ans.retrieval.chunks_used,
                        ans.retrieval.chunks_returned,
                    );
                }
            }
            // Refusal → exit 1.
            if !ans.grounded {
                return Err(RefusalSignal.into());
            }
            Ok(())
        }

        Cmd::Reset {
            all,
            data_only: _,
            vector_only,
            config_only,
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
            } else {
                ResetScope::DataOnly
            };

            let cfg = kebab_config::Config::load(cli.config.as_deref())?;
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

        Cmd::Tui => {
            // P9-1: Ratatui shell with Library pane. Search / Ask /
            // Inspect panes land in p9-2 / p9-3 / p9-4.
            let config = match cli.config.as_deref() {
                Some(path) => kebab_config::Config::load(Some(path))?,
                None => kebab_config::Config::load(None)?,
            };
            let mut app = kebab_tui::App::new(config)?;
            app.run()
        }

        Cmd::Eval { what } => match what {
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
                let run = kebab_eval::run_eval(&opts)?;
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
                let agg = kebab_eval::compute_aggregate(run_id)?;
                kebab_eval::store_aggregate(run_id, &agg)?;
                if cli.json {
                    println!("{}", serde_json::to_string_pretty(&agg)?);
                } else {
                    println!("run_id: {run_id}");
                    println!("queries: {} ({} failed)", agg.total_queries, agg.failed_queries);
                    println!("hit@1:   {:.4}", agg.hit_at_k.get(&1).copied().unwrap_or(0.0));
                    println!("hit@5:   {:.4}", agg.hit_at_k.get(&5).copied().unwrap_or(0.0));
                    println!("MRR:     {:.4}", agg.mrr);
                }
                Ok(())
            }

            EvalWhat::Compare {
                run_a,
                run_b,
                strict_chunker_version,
                write_report,
            } => {
                let cfg = kebab_config::Config::load(None)?;
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
                    let resolved_data_dir = kebab_config::expand_path(&cfg.storage.data_dir, "");
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
        },

        Cmd::IngestFile { path } => {
            let cfg = kebab_config::Config::load(cli.config.as_deref())?;
            let report = kebab_app::ingest_file_with_config(cfg, path)?;
            if cli.json {
                let v = wire::wire_ingest(&report);
                println!("{}", serde_json::to_string(&v)?);
            } else {
                println!(
                    "ingest-file: scanned={} new={} updated={} unchanged={} skipped={} errors={}",
                    report.scanned, report.new, report.updated,
                    report.unchanged, report.skipped, report.errors
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
            let report = kebab_app::ingest_stdin_with_config(
                cfg,
                &body,
                title,
                source_uri.as_deref(),
            )?;
            if cli.json {
                let v = wire::wire_ingest(&report);
                println!("{}", serde_json::to_string(&v)?);
            } else {
                println!(
                    "ingest-stdin: scanned={} new={} updated={} unchanged={} skipped={} errors={}",
                    report.scanned, report.new, report.updated,
                    report.unchanged, report.skipped, report.errors
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
    println!("  prompt_template_version {}", s.models.prompt_template_version);
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
    writeln!(out, "kebab reset ({:?}): about to remove", scope)?;
    for p in paths {
        writeln!(out, "  - {}", p.display())?;
    }
    writeln!(out, "estimated total: {} bytes", bytes)?;
    write!(out, "Proceed? [y/N] ")?;
    out.flush()?;

    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    let s = line.trim().to_ascii_lowercase();
    Ok(matches!(s.as_str(), "y" | "yes"))
}

