//! Compare two eval runs (P5-2 — design §5.7, phase epic
//! `tasks/phase-5-evaluation.md`).
//!
//! Reads `eval_runs` + `eval_query_results` for two `run_id`s, calls
//! [`crate::metrics::compute_aggregate_with_config`] for each, then
//! diffs them per-query. Emits a [`CompareReport`] (machine) and an
//! optional Markdown render (human).
//!
//! Pure computation — no `kb-app` / retrieval imports.

use std::collections::HashMap;
use std::fmt::Write as _;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use kb_config::Config;
use kb_core::{ChunkId, DocumentId};
use kb_store_sqlite::SqliteStore;

use crate::loader::load_golden_set;
use crate::metrics::{
    AggregateMetrics, compute_aggregate_with_config, resolve_golden_path,
};
use crate::types::{GoldenQuery, QueryResult};

/// Strict-mode behavior pivot used by [`CompareOpts::strict_chunker_version`].
/// When `false` (default) and the two runs' `chunker_version` differ,
/// per-query matching falls back to doc-id-only comparison and the
/// report's `deltas.chunker_version_match` field is set to
/// `"fallback_doc"`.
///
/// **Spec deviation (intentional, documented):** the spec called for a
/// `"fallback_doc_span"` mode that augments doc-id matching with a 50%
/// `source_spans` overlap criterion. That requires `chunks` table
/// reads from both runs simultaneously — but in practice you re-index
/// (and overwrite the chunks table) before evaluating a chunker
/// change, so the run-A chunks are gone by the time run-B is computed.
/// We log the simpler doc-id-only fallback as `"fallback_doc"` and
/// defer span-overlap matching to a future phase that owns
/// chunker-version archival. The `strict-chunker-version` flag is
/// preserved verbatim from the spec.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CompareOpts {
    pub strict_chunker_version: bool,
}

/// Per-metric + per-query diff between two stored eval runs.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompareReport {
    pub run_a: String,
    pub run_b: String,
    pub aggregate_a: AggregateMetrics,
    pub aggregate_b: AggregateMetrics,
    /// Per-metric delta (`b - a`) plus the `chunker_version_match`
    /// audit field. JSON object so consumers can pluck individual
    /// metrics by name without keeping the struct shape in sync.
    pub deltas: serde_json::Value,
    pub per_query: Vec<QueryComparison>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QueryComparison {
    pub query_id: String,
    pub kind: ComparisonKind,
    pub a_hit_rank: Option<u32>,
    pub b_hit_rank: Option<u32>,
    pub note: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ComparisonKind {
    Win,
    Loss,
    Draw,
    Regression,
}

/// Compare two runs using the active XDG-loaded [`Config`]. Wraps
/// [`compare_runs_with_config`] with `Config::load(None)`.
pub fn compare_runs(run_id_a: &str, run_id_b: &str) -> Result<CompareReport> {
    let cfg = Config::load(None).context("load Config for compare_runs")?;
    compare_runs_with_config(&cfg, run_id_a, run_id_b, &CompareOpts::default())
}

/// Compare two runs against an explicit [`Config`] + [`CompareOpts`].
/// Used by integration tests and the future `kb eval compare --strict`
/// CLI surface.
pub fn compare_runs_with_config(
    cfg: &Config,
    run_id_a: &str,
    run_id_b: &str,
    opts: &CompareOpts,
) -> Result<CompareReport> {
    let store = SqliteStore::open(cfg).context("open SqliteStore for compare_runs")?;
    store.run_migrations().context("run migrations")?;

    // Pull both run rows up-front so we can extract chunker_version and
    // bail early on a missing run before doing any metric work.
    let run_a = store
        .load_eval_run(run_id_a)
        .context("load eval_runs row A")?
        .ok_or_else(|| anyhow::anyhow!("compare_runs: no eval_runs row for run_id {run_id_a}"))?;
    let run_b = store
        .load_eval_run(run_id_b)
        .context("load eval_runs row B")?
        .ok_or_else(|| anyhow::anyhow!("compare_runs: no eval_runs row for run_id {run_id_b}"))?;

    let aggregate_a = compute_aggregate_with_config(cfg, run_id_a)?;
    let aggregate_b = compute_aggregate_with_config(cfg, run_id_b)?;

    let chunker_a = extract_chunker_version(&run_a.config_snapshot_json);
    let chunker_b = extract_chunker_version(&run_b.config_snapshot_json);
    let chunker_match_mode = if chunker_a == chunker_b {
        "exact"
    } else if opts.strict_chunker_version {
        anyhow::bail!(
            "compare_runs: chunker_version mismatch (a={chunker_a:?}, b={chunker_b:?}) and \
             strict_chunker_version=true. Pass strict_chunker_version=false to use the doc-id \
             fallback."
        );
    } else {
        "fallback_doc"
    };

    let rows_a = store.load_eval_query_results(run_id_a)?;
    let rows_b = store.load_eval_query_results(run_id_b)?;
    let qrs_a = parse_results(&rows_a)?;
    let qrs_b = parse_results(&rows_b)?;

    let golden = load_golden_set(&resolve_golden_path()).context("reload golden set")?;
    let golden_by_id: HashMap<&str, &GoldenQuery> =
        golden.iter().map(|q| (q.id.as_str(), q)).collect();

    let per_query = build_per_query(&qrs_a, &qrs_b, &golden_by_id, chunker_match_mode);
    let deltas = build_deltas(&aggregate_a, &aggregate_b, chunker_match_mode);

    Ok(CompareReport {
        run_a: run_id_a.to_owned(),
        run_b: run_id_b.to_owned(),
        aggregate_a,
        aggregate_b,
        deltas,
        per_query,
    })
}

/// Render a Markdown summary of `report`. Output is for human eyes
/// (saved to `runs_dir/<run_b>/report.md` by callers that want it) —
/// not a wire schema. Stable enough for snapshot tests.
pub fn render_report_md(report: &CompareReport) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# Eval compare: `{}` vs `{}`", report.run_a, report.run_b);
    let _ = writeln!(out);
    let _ = writeln!(out, "## Aggregate deltas");
    let _ = writeln!(out);
    let _ = writeln!(out, "| metric | a | b | Δ (b - a) |");
    let _ = writeln!(out, "|---|---|---|---|");
    let a = &report.aggregate_a;
    let b = &report.aggregate_b;
    for k in crate::metrics::TOP_K_VARIANTS {
        let _ = writeln!(
            out,
            "| hit@{k} | {} | {} | {} |",
            fmt(a.hit_at_k.get(k).copied().unwrap_or(f32::NAN)),
            fmt(b.hit_at_k.get(k).copied().unwrap_or(f32::NAN)),
            fmt_delta(
                a.hit_at_k.get(k).copied().unwrap_or(f32::NAN),
                b.hit_at_k.get(k).copied().unwrap_or(f32::NAN),
            ),
        );
    }
    let _ = writeln!(out, "| MRR | {} | {} | {} |", fmt(a.mrr), fmt(b.mrr), fmt_delta(a.mrr, b.mrr));
    for k in crate::metrics::TOP_K_VARIANTS {
        let _ = writeln!(
            out,
            "| recall@{k}_doc | {} | {} | {} |",
            fmt(a.recall_at_k_doc.get(k).copied().unwrap_or(f32::NAN)),
            fmt(b.recall_at_k_doc.get(k).copied().unwrap_or(f32::NAN)),
            fmt_delta(
                a.recall_at_k_doc.get(k).copied().unwrap_or(f32::NAN),
                b.recall_at_k_doc.get(k).copied().unwrap_or(f32::NAN),
            ),
        );
    }
    let _ = writeln!(
        out,
        "| citation_coverage | {} | {} | {} |",
        fmt(a.citation_coverage),
        fmt(b.citation_coverage),
        fmt_delta(a.citation_coverage, b.citation_coverage),
    );
    let _ = writeln!(
        out,
        "| groundedness | {} | {} | {} |",
        fmt(a.groundedness),
        fmt(b.groundedness),
        fmt_delta(a.groundedness, b.groundedness),
    );
    let _ = writeln!(
        out,
        "| empty_result_rate | {} | {} | {} |",
        fmt(a.empty_result_rate),
        fmt(b.empty_result_rate),
        fmt_delta(a.empty_result_rate, b.empty_result_rate),
    );
    let _ = writeln!(
        out,
        "| refusal_correctness | {} | {} | {} |",
        fmt(a.refusal_correctness),
        fmt(b.refusal_correctness),
        fmt_delta(a.refusal_correctness, b.refusal_correctness),
    );
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "chunker_version_match: `{}`",
        report
            .deltas
            .get("chunker_version_match")
            .and_then(|v| v.as_str())
            .unwrap_or("?")
    );
    let _ = writeln!(out);

    let wins: Vec<_> = report.per_query.iter().filter(|c| c.kind == ComparisonKind::Win).collect();
    let losses: Vec<_> = report.per_query.iter().filter(|c| c.kind == ComparisonKind::Loss).collect();
    let regressions: Vec<_> = report
        .per_query
        .iter()
        .filter(|c| c.kind == ComparisonKind::Regression)
        .collect();

    let _ = writeln!(
        out,
        "## Wins ({}) / Losses ({}) / Regressions ({})",
        wins.len(),
        losses.len(),
        regressions.len()
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "| query_id | kind | rank_a | rank_b | note |");
    let _ = writeln!(out, "|---|---|---|---|---|");
    for c in &report.per_query {
        let _ = writeln!(
            out,
            "| {} | {} | {} | {} | {} |",
            c.query_id,
            comparison_kind_label(c.kind),
            c.a_hit_rank.map(|r| r.to_string()).unwrap_or_else(|| "—".into()),
            c.b_hit_rank.map(|r| r.to_string()).unwrap_or_else(|| "—".into()),
            c.note.as_deref().unwrap_or(""),
        );
    }
    out
}

fn comparison_kind_label(k: ComparisonKind) -> &'static str {
    match k {
        ComparisonKind::Win => "win",
        ComparisonKind::Loss => "loss",
        ComparisonKind::Draw => "draw",
        ComparisonKind::Regression => "regression",
    }
}

fn fmt(v: f32) -> String {
    if v.is_nan() {
        "—".into()
    } else {
        format!("{v:.4}")
    }
}

fn fmt_delta(a: f32, b: f32) -> String {
    if a.is_nan() || b.is_nan() {
        return "—".into();
    }
    let d = b - a;
    if d >= 0.0 {
        format!("+{d:.4}")
    } else {
        format!("{d:.4}")
    }
}

/// Pull `chunker_version` out of a `config_snapshot_json` payload. The
/// runner writes `{"chunker_version": "<id>", ...}`; missing or
/// malformed → `None`. Two `None`s compare as equal and route through
/// the "exact" matcher, but only the runner writes these snapshots
/// and it always emits `chunker_version` — so `None == None` can only
/// arise from a hand-edited DB or a pre-P5-1 fixture, both of which
/// are out-of-scope failure modes that the strict-mode flag covers.
fn extract_chunker_version(snapshot_json: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(snapshot_json).ok()?;
    v.get("chunker_version")
        .and_then(|x| x.as_str())
        .map(|s| s.to_owned())
}

fn parse_results(
    rows: &[kb_store_sqlite::EvalQueryResultRecord],
) -> Result<HashMap<String, QueryResult>> {
    let mut out = HashMap::with_capacity(rows.len());
    for row in rows {
        let qr: QueryResult = serde_json::from_str(&row.result_json)
            .with_context(|| format!("parse result_json for {}", row.query_id))?;
        out.insert(row.query_id.clone(), qr);
    }
    Ok(out)
}

/// Find the top-ranked hit in `qr` whose `chunk_id` is in `expected`
/// (exact mode) or whose `doc_id` is in `expected_docs` (fallback).
fn first_hit_rank(
    qr: &QueryResult,
    expected_chunks: &[ChunkId],
    expected_docs: &[DocumentId],
    fallback_doc_only: bool,
) -> Option<u32> {
    if !fallback_doc_only && !expected_chunks.is_empty() {
        let exp: std::collections::HashSet<&ChunkId> = expected_chunks.iter().collect();
        return qr
            .hits_top_k
            .iter()
            .filter(|h| exp.contains(&h.chunk_id))
            .map(|h| h.rank)
            .min();
    }
    if expected_docs.is_empty() {
        return None;
    }
    let exp: std::collections::HashSet<&DocumentId> = expected_docs.iter().collect();
    qr.hits_top_k
        .iter()
        .filter(|h| exp.contains(&h.doc_id))
        .map(|h| h.rank)
        .min()
}

fn build_per_query(
    qrs_a: &HashMap<String, QueryResult>,
    qrs_b: &HashMap<String, QueryResult>,
    golden: &HashMap<&str, &GoldenQuery>,
    chunker_match_mode: &str,
) -> Vec<QueryComparison> {
    let fallback = chunker_match_mode == "fallback_doc";
    let mut ids: Vec<&String> = qrs_a.keys().chain(qrs_b.keys()).collect();
    ids.sort();
    ids.dedup();

    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        let a = qrs_a.get(id);
        let b = qrs_b.get(id);
        let gq = golden.get(id.as_str()).copied();

        let (a_rank, b_rank) = match gq {
            Some(g) => (
                a.and_then(|q| first_hit_rank(q, &g.expected_chunk_ids, &g.expected_doc_ids, fallback)),
                b.and_then(|q| first_hit_rank(q, &g.expected_chunk_ids, &g.expected_doc_ids, fallback)),
            ),
            None => (None, None),
        };

        let (kind, note) = classify(a_rank, b_rank, gq);

        out.push(QueryComparison {
            query_id: id.clone(),
            kind,
            a_hit_rank: a_rank,
            b_hit_rank: b_rank,
            note,
        });
    }
    out
}

fn classify(
    a_rank: Option<u32>,
    b_rank: Option<u32>,
    gq: Option<&GoldenQuery>,
) -> (ComparisonKind, Option<String>) {
    match (a_rank, b_rank) {
        (None, Some(_)) => (ComparisonKind::Win, None),
        (Some(_), None) => {
            // Hit → miss is a regression specifically when the query had
            // an expected chunk to find. Without that, downgrade to Loss
            // so refusal-flow queries (no expected_*) don't appear as
            // regressions.
            let has_expected = gq
                .map(|g| !g.expected_chunk_ids.is_empty() || !g.expected_doc_ids.is_empty())
                .unwrap_or(false);
            if has_expected {
                (ComparisonKind::Regression, Some("hit→miss".into()))
            } else {
                (ComparisonKind::Loss, None)
            }
        }
        (Some(ra), Some(rb)) if ra == rb => (ComparisonKind::Draw, None),
        (Some(ra), Some(rb)) if rb < ra => (ComparisonKind::Win, Some(format!("rank {ra}→{rb}"))),
        (Some(ra), Some(rb)) => (ComparisonKind::Loss, Some(format!("rank {ra}→{rb}"))),
        (None, None) => (ComparisonKind::Draw, None),
    }
}

fn build_deltas(
    a: &AggregateMetrics,
    b: &AggregateMetrics,
    chunker_match_mode: &str,
) -> serde_json::Value {
    fn d(a: f32, b: f32) -> serde_json::Value {
        if a.is_nan() || b.is_nan() {
            serde_json::Value::Null
        } else {
            serde_json::Value::from((b - a) as f64)
        }
    }
    let mut hit = serde_json::Map::new();
    let mut recall = serde_json::Map::new();
    for k in crate::metrics::TOP_K_VARIANTS {
        hit.insert(
            k.to_string(),
            d(
                a.hit_at_k.get(k).copied().unwrap_or(f32::NAN),
                b.hit_at_k.get(k).copied().unwrap_or(f32::NAN),
            ),
        );
        recall.insert(
            k.to_string(),
            d(
                a.recall_at_k_doc.get(k).copied().unwrap_or(f32::NAN),
                b.recall_at_k_doc.get(k).copied().unwrap_or(f32::NAN),
            ),
        );
    }
    serde_json::json!({
        "hit_at_k": hit,
        "mrr": d(a.mrr, b.mrr),
        "recall_at_k_doc": recall,
        "citation_coverage": d(a.citation_coverage, b.citation_coverage),
        "groundedness": d(a.groundedness, b.groundedness),
        "empty_result_rate": d(a.empty_result_rate, b.empty_result_rate),
        "refusal_correctness": d(a.refusal_correctness, b.refusal_correctness),
        "chunker_version_match": chunker_match_mode,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_win_loss_draw_regression() {
        let g = GoldenQuery {
            id: "q1".into(),
            query: "q".into(),
            lang: kb_core::Lang(String::new()),
            expected_doc_ids: vec![],
            expected_chunk_ids: vec![kb_core::ChunkId("c1".into())],
            must_contain: vec![],
            forbidden: vec![],
            difficulty: None,
        };
        let g = Some(&g);
        // a miss, b hit → Win
        assert_eq!(classify(None, Some(2), g).0, ComparisonKind::Win);
        // a hit, b miss, has expected → Regression
        assert_eq!(classify(Some(1), None, g).0, ComparisonKind::Regression);
        // both same rank → Draw
        assert_eq!(classify(Some(3), Some(3), g).0, ComparisonKind::Draw);
        // b improved rank → Win
        assert_eq!(classify(Some(5), Some(2), g).0, ComparisonKind::Win);
        // b worse rank → Loss
        assert_eq!(classify(Some(2), Some(5), g).0, ComparisonKind::Loss);
        // both miss → Draw
        assert_eq!(classify(None, None, g).0, ComparisonKind::Draw);
    }

    #[test]
    fn delta_null_when_either_nan() {
        let a = AggregateMetrics {
            hit_at_k: Default::default(),
            mrr: 0.5,
            recall_at_k_doc: Default::default(),
            citation_coverage: f32::NAN,
            groundedness: 0.0,
            empty_result_rate: 0.0,
            refusal_correctness: f32::NAN,
            total_queries: 0,
            failed_queries: 0,
        };
        let b = AggregateMetrics { mrr: 0.75, ..a.clone() };
        let d = build_deltas(&a, &b, "exact");
        assert!(d["citation_coverage"].is_null());
        assert!(d["refusal_correctness"].is_null());
        assert!((d["mrr"].as_f64().unwrap() - 0.25).abs() < 1e-6);
        assert_eq!(d["chunker_version_match"], "exact");
    }

    #[test]
    fn extract_chunker_version_from_snapshot() {
        let s = r#"{"config":{},"chunker_version":"slot@1"}"#;
        assert_eq!(extract_chunker_version(s), Some("slot@1".into()));
        assert_eq!(extract_chunker_version("not json"), None);
        assert_eq!(extract_chunker_version("{}"), None);
    }
}
