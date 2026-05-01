//! Per-query eval runner. See [`run_eval`] / [`run_eval_with_config`].

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;

use anyhow::{Context, Result};
use kb_core::{SearchFilters, SearchQuery};
use kb_store_sqlite::{EvalRunRow, SqliteStore};
use time::OffsetDateTime;

use crate::loader::{load_golden_set, validate_against_db};
use crate::types::{EvalRun, EvalRunOpts, GoldenQuery, QueryResult};

/// Env var that overrides the default `fixtures/golden_queries.yaml`
/// path. Resolved relative to the current working directory.
const KB_EVAL_GOLDEN: &str = "KB_EVAL_GOLDEN";

/// Default golden YAML path (relative to CWD when set).
const DEFAULT_GOLDEN_PATH: &str = "fixtures/golden_queries.yaml";

/// Run the golden suite end-to-end against the active XDG-loaded
/// [`kb_config::Config`]. Wraps [`run_eval_with_config`] with
/// `Config::load(None)`.
pub fn run_eval(opts: &EvalRunOpts) -> Result<EvalRun> {
    let cfg = kb_config::Config::load(None).context("load Config for run_eval")?;
    run_eval_with_config(&cfg, opts)
}

/// Run the golden suite end-to-end against an explicit
/// [`kb_config::Config`]. Used by integration tests (TempDir-backed
/// data_dir) and any future caller that wants to drive the runner
/// against a non-default config.
pub fn run_eval_with_config(cfg: &kb_config::Config, opts: &EvalRunOpts) -> Result<EvalRun> {
    let started = std::time::Instant::now();

    // ── 1. Load golden set ────────────────────────────────────────────────
    let golden_path = resolve_golden_path();
    tracing::debug!(
        target: "kb-eval",
        path = %golden_path.display(),
        "kb-eval: loading golden set"
    );
    let queries = load_golden_set(&golden_path).with_context(|| {
        format!(
            "load golden set from {} (override via KB_EVAL_GOLDEN)",
            golden_path.display()
        )
    })?;
    validate_against_db(&queries, cfg)?;

    // ── 2. Mint identifiers + open store ──────────────────────────────────
    let run_id = mint_run_id();
    let created_at = OffsetDateTime::now_utc();
    let commit_hash = std::env::var("KB_COMMIT_HASH")
        .ok()
        .filter(|s| !s.is_empty());

    // Open the store once so every per-query write reuses the same
    // connection-mutex lifetime.
    let store = SqliteStore::open(cfg).context("open SqliteStore for run_eval")?;
    store
        .run_migrations()
        .context("run migrations for run_eval")?;

    // ── 3. Build config_snapshot_json ─────────────────────────────────────
    let config_snapshot_json = build_config_snapshot(cfg)?;
    let config_snapshot_text =
        serde_json::to_string(&config_snapshot_json).context("serialize config_snapshot_json")?;

    // ── 4. Per-query execution ────────────────────────────────────────────
    let mut per_query: Vec<QueryResult> = Vec::with_capacity(queries.len());
    for gq in &queries {
        let qr = execute_query(cfg, gq, opts);
        per_query.push(qr);
    }

    // ── 5. Persist eval_runs + eval_query_results ────────────────────────
    // Serialize per-query JSON up front so the SQLite transaction below
    // never holds the connection mutex through serde failures.
    let mut results: Vec<(String, String)> = Vec::with_capacity(per_query.len());
    for qr in &per_query {
        let json = serde_json::to_string(qr)
            .with_context(|| format!("serialize QueryResult for {}", qr.query_id))?;
        results.push((qr.query_id.clone(), json));
    }
    let row = EvalRunRow {
        run_id: &run_id,
        suite: opts.suite.as_str(),
        config_snapshot_json: &config_snapshot_text,
        aggregate_json: "{}",
        commit_hash: commit_hash.as_deref(),
        created_at,
    };
    store
        .record_eval_run_with_results(&row, &results)
        .context("record eval_runs + eval_query_results (transactional)")?;

    // ── 6. Mirror to runs_dir/<run_id>/per_query.jsonl ────────────────────
    write_per_query_jsonl(cfg, &run_id, &per_query)?;

    let duration_ms = started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;
    tracing::info!(
        target: "kb-eval",
        run_id = %run_id,
        suite = %opts.suite,
        queries = per_query.len(),
        duration_ms,
        "kb-eval: run complete"
    );

    Ok(EvalRun {
        run_id,
        created_at,
        commit_hash,
        config_snapshot_json,
        per_query,
    })
}

/// Mint a `run_<lower>` identifier. UUIDv7 stands in for ULID — same
/// timestamp-ordered monotonicity, already in workspace deps. Lower-
/// case simple form to match the `ulid_lower()` shape the spec asks
/// for.
fn mint_run_id() -> String {
    let id = uuid::Uuid::now_v7().simple().to_string();
    format!("run_{id}")
}

/// Resolve the golden YAML path. Honors the `KB_EVAL_GOLDEN` env
/// override; otherwise relative to CWD. The path is NOT expanded for
/// `~` / `${...}` placeholders — direct file paths only.
fn resolve_golden_path() -> PathBuf {
    match std::env::var(KB_EVAL_GOLDEN) {
        Ok(s) if !s.is_empty() => PathBuf::from(s),
        _ => PathBuf::from(DEFAULT_GOLDEN_PATH),
    }
}

/// Run one [`GoldenQuery`] through the kb-app facade. Errors are
/// captured into `QueryResult.error` so the run continues.
fn execute_query(cfg: &kb_config::Config, gq: &GoldenQuery, opts: &EvalRunOpts) -> QueryResult {
    let started = std::time::Instant::now();

    let search_query = SearchQuery {
        text: gq.query.clone(),
        mode: opts.mode,
        k: opts.k,
        filters: SearchFilters::default(),
    };

    let (hits_top_k, mut error) = match kb_app::search_with_config(cfg.clone(), search_query) {
        Ok(hits) => (hits, None),
        Err(e) => (Vec::new(), Some(format!("{e:#}"))),
    };

    // Optional RAG path: only attempted when `with_rag` and the search
    // call did not already error out (we want one error per query, not
    // a duplicated one).
    let answer = if opts.with_rag && error.is_none() {
        let ask_opts = kb_app::AskOpts {
            k: opts.k,
            explain: true,
            mode: opts.mode,
            temperature: opts.temperature,
            seed: opts.seed,
            stream_sink: None,
        };
        match kb_app::ask_with_config(cfg.clone(), &gq.query, ask_opts) {
            Ok(ans) => Some(ans),
            Err(e) => {
                error = Some(format!("{e:#}"));
                None
            }
        }
    } else {
        None
    };

    let elapsed_ms = started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;

    QueryResult {
        query_id: gq.id.clone(),
        query: gq.query.clone(),
        mode: opts.mode,
        hits_top_k,
        answer,
        elapsed_ms,
        error,
    }
}

/// Build the `config_snapshot_json` value: full Config as `config` plus
/// the auxiliary version fields the spec calls out.
///
/// `index_version` is intentionally `None` here — it is composed
/// dynamically by `kb-app` on a per-call basis from the configured
/// embedder (e.g., `vec:<model>@<version>:<dim>`), so it is not a
/// stable run-time property of the config alone. P5-2 may compose it
/// from `embedding.{model,version,dimensions}` if it needs the field
/// for compare reports.
fn build_config_snapshot(cfg: &kb_config::Config) -> Result<serde_json::Value> {
    let cfg_value = serde_json::to_value(cfg).context("serialize Config")?;
    Ok(serde_json::json!({
        "config": cfg_value,
        "chunker_version": cfg.chunking.chunker_version,
        "embedding": {
            "model": cfg.models.embedding.model,
            "version": cfg.models.embedding.version,
            "dimensions": cfg.models.embedding.dimensions,
            "provider": cfg.models.embedding.provider,
        },
        "llm": {
            "model_id": cfg.models.llm.model,
            "provider": cfg.models.llm.provider,
        },
        "prompt_template_version": cfg.rag.prompt_template_version,
        "score_gate": cfg.rag.score_gate,
        "rrf_k": cfg.search.rrf_k,
        "index_version": serde_json::Value::Null,
    }))
}

/// Write the `runs_dir/<run_id>/per_query.jsonl` mirror (design §6.3).
/// Each `QueryResult` is one line, separator `\n`. The directory is
/// created if it doesn't exist; an existing file is overwritten (a
/// `run_id` collision would already have failed the `eval_runs`
/// PRIMARY KEY upstream).
fn write_per_query_jsonl(
    cfg: &kb_config::Config,
    run_id: &str,
    per_query: &[QueryResult],
) -> Result<()> {
    let runs_dir = expand_path(&cfg.storage.runs_dir, &cfg.storage.data_dir);
    let run_dir = runs_dir.join(run_id);
    std::fs::create_dir_all(&run_dir)
        .with_context(|| format!("create run dir {}", run_dir.display()))?;
    let path = run_dir.join("per_query.jsonl");
    let file = File::create(&path)
        .with_context(|| format!("create per_query.jsonl at {}", path.display()))?;
    let mut w = BufWriter::new(file);
    for qr in per_query {
        serde_json::to_writer(&mut w, qr)
            .with_context(|| format!("serialize QueryResult for {}", qr.query_id))?;
        w.write_all(b"\n")
            .context("write newline separator in per_query.jsonl")?;
    }
    w.flush().context("flush per_query.jsonl")?;
    Ok(())
}

/// Expand `{data_dir}` / `${XDG_DATA_HOME:-…}` / leading `~`. Mirror
/// of `kb-store-vector::paths::expand_path` and
/// `kb-store-sqlite::expand_data_dir` — kept private here because
/// `kb-config` does not (yet) expose a shared resolver helper.
fn expand_path(raw: &str, data_dir: &str) -> PathBuf {
    let mut s = raw.to_string();

    // First, resolve `data_dir` itself so any `{data_dir}` substitution
    // points at an already-expanded base path. `data_dir` may contain
    // `${XDG_DATA_HOME:-…}` and `~`; resolve them once and re-use the
    // result.
    let resolved_data_dir = expand_xdg_and_tilde(data_dir);
    s = s.replace("{data_dir}", &resolved_data_dir);

    expand_xdg_and_tilde_path(&s)
}

fn expand_xdg_and_tilde(raw: &str) -> String {
    let s = expand_xdg(raw);
    expand_tilde_str(&s)
}

fn expand_xdg_and_tilde_path(raw: &str) -> PathBuf {
    let s = expand_xdg_and_tilde(raw);
    PathBuf::from(s)
}

fn expand_xdg(raw: &str) -> String {
    let mut s = raw.to_string();
    if let Some(start) = s.find("${XDG_DATA_HOME") {
        if let Some(rel_end) = s[start..].find('}') {
            let end = start + rel_end + 1;
            let inner = &s[start + 2..end - 1];
            let replacement = match std::env::var("XDG_DATA_HOME") {
                Ok(v) if !v.is_empty() => v,
                _ => match inner.split_once(":-") {
                    Some((_, default)) => default.to_string(),
                    None => String::new(),
                },
            };
            s.replace_range(start..end, &replacement);
        }
    }
    s
}

fn expand_tilde_str(raw: &str) -> String {
    if let Some(rest) = raw.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            let mut p = PathBuf::from(home);
            p.push(rest);
            return p.to_string_lossy().into_owned();
        }
    }
    if raw == "~" {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).to_string_lossy().into_owned();
        }
    }
    raw.to_string()
}

#[cfg(test)]
mod expand_tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn expand_substitutes_data_dir() {
        let p = expand_path("{data_dir}/runs", "/tmp/kbtest");
        assert_eq!(p, Path::new("/tmp/kbtest/runs"));
    }

    #[test]
    fn expand_passthrough_absolute() {
        let p = expand_path("/abs/runs", "/ignored");
        assert_eq!(p, Path::new("/abs/runs"));
    }
}
