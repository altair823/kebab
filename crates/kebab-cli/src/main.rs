//! `kb` — command-line interface. Each subcommand maps 1:1 to a `kb-app`
//! function. Exit codes per design §10.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use kebab_app::doctor_signal::{DoctorUnhealthy, NoHitSignal, RefusalSignal};

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
    },

    /// Health check.
    Doctor,

    /// Eval suite (placeholder; lands in P9).
    Eval {
        #[command(subcommand)]
        what: EvalWhat,
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
    // proceed without a guard rather than crashing — `kb` is still usable.
    let _log_guard = kebab_app::logging::init(level).ok();
    match run(&cli) {
        Ok(()) => ExitCode::from(0),
        Err(e) => {
            let code = exit_code(&e);
            // Refusals at exit code 1 print to stdout (already done by the
            // caller); errors go to stderr.
            if code != 1 {
                eprintln!("error: {e}");
                if cli.verbose {
                    for cause in e.chain().skip(1) {
                        eprintln!("  caused by: {cause}");
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
                println!("hint     edit the config above, then `kb ingest`");
            }
            Ok(())
        }

        Cmd::Ingest {
            root,
            summary_only,
        } => {
            let cfg = kebab_config::Config::load(cli.config.as_deref())?;
            let scope = kebab_core::SourceScope {
                root: root.clone().unwrap_or_else(|| PathBuf::from(&cfg.workspace.root)),
                include: cfg.workspace.include.clone(),
                exclude: cfg.workspace.exclude.clone(),
            };
            let report = kebab_app::ingest_with_config(cfg, scope, *summary_only)?;
            if cli.json {
                println!("{}", serde_json::to_string(&wire::wire_ingest(&report))?);
            } else {
                println!(
                    "scanned {}  new {}  updated {}  skipped {}  errors {}  ({} ms)",
                    report.scanned,
                    report.new,
                    report.updated,
                    report.skipped,
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
        } => {
            let cfg = kebab_config::Config::load(cli.config.as_deref())?;
            let q = kebab_core::SearchQuery {
                text: query.clone(),
                mode: (*mode).into(),
                k: *k,
                filters: kebab_core::SearchFilters::default(),
            };
            let hits = kebab_app::search_with_config(cfg, q)?;
            if cli.json {
                println!("{}", serde_json::to_string(&wire::wire_search_hits(&hits))?);
            } else {
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
                    println!(
                        "{:>2}. {:.4}  {}{}",
                        h.rank,
                        h.retrieval.fusion_score,
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
            };
            let ans = kebab_app::ask_with_config(cfg, query, opts)?;
            if cli.json {
                println!("{}", serde_json::to_string(&wire::wire_answer(&ans))?);
            } else {
                println!("{}", ans.answer);
            }
            // Refusal → exit 1.
            if !ans.grounded {
                return Err(RefusalSignal.into());
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
    }
}

