//! 변형(paraphrase) 일관성 진단 메트릭.
//!
//! 같은 의도(`GoldenQuery.group`)의 여러 표현이 같은 정답 문서를 공유한다는
//! 전제 아래, 표현마다 검색/답변 품질이 얼마나 출렁이는지를 잰다. 핵심은
//! `recall@narrow`(사용자가 보는 top-10) vs `recall@pool`(넓은 후보 폭)의 대비:
//!
//! - (A) 순위 출렁(`MisRanked`): 정답이 pool엔 있는데 top-10 밖 → near-tie 흡수로 해결 후보.
//! - (B) 어휘 격차(`Missing`):   정답이 pool에도 없음 → 쿼리 확장/번역 필요.
//!
//! 진단 전용. 기존 [`crate::metrics::AggregateMetrics`] 경로는 건드리지 않는다.

use std::collections::{BTreeMap, HashMap, HashSet};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use kebab_config::Config;
use kebab_core::DocumentId;
use kebab_store_sqlite::SqliteStore;

use crate::types::{GoldenQuery, QueryResult};

/// 사용자가 실제 보는 답변 context 폭.
const NARROW_K: u32 = 10;
/// 넓은 후보 폭. recall@pool vs recall@narrow 대비로 A/B를 가른다.
/// eval run은 `--k`를 이 값 이상으로 줘서 `hits_top_k`가 pool을 담아야 한다.
const POOL_K: u32 = 50;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum VariantClass {
    /// recall@narrow == 1.0 (정답 전부 top-10 안).
    Ok,
    /// recall@pool > recall@narrow (정답이 pool엔 있는데 top-10 밖). (A)
    MisRanked,
    /// recall@pool == recall@narrow < 1.0 (못 찾은 정답이 pool에도 없음). (B)
    Missing,
    /// 정답 문서 미지정(검증 불가).
    NoExpected,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VariantResult {
    pub query_id: String,
    pub query: String,
    pub recall_narrow: f32,
    pub recall_pool: f32,
    /// must_contain 통과 여부. RAG 답변(`--with-rag`)이 없으면 `None`.
    pub answer_ok: Option<bool>,
    pub class: VariantClass,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VariantGroupReport {
    pub group: String,
    pub variants: Vec<VariantResult>,
    /// max-min recall_narrow (정답 지정 변형들만). 0 = 완전 일관.
    pub recall_spread_narrow: f32,
    pub worst_recall_narrow: f32,
    /// 모든 변형이 must_contain 통과면 Some(true), 하나라도 실패 Some(false),
    /// RAG 답변이 전혀 없으면 None.
    pub answer_consistency: Option<bool>,
    pub mis_ranked: u32,
    pub missing: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VariantConsistencyReport {
    pub groups: Vec<VariantGroupReport>,
    pub mean_recall_spread_narrow: f32,
    /// spread==0 && worst_recall_narrow==1.0 인 그룹 수.
    pub fully_consistent_groups: u32,
    pub total_groups: u32,
    /// mis_ranked>0 && mis_ranked>=missing 인 그룹 수 (near-tie 처방 우선).
    pub a_dominant_groups: u32,
    /// missing>0 && missing>mis_ranked 인 그룹 수 (쿼리 확장 처방 우선).
    pub b_dominant_groups: u32,
}

/// 저장된 run을 그룹으로 묶어 변형 일관성 리포트를 만든다.
/// `rows`는 [`crate::metrics::aggregate_from_rows`]와 동일한 입력
/// (저장된 per-query 결과). `group`이 없는 쿼리는 무시한다.
pub fn compute_variant_consistency(
    queries: &[GoldenQuery],
    rows: &[kebab_store_sqlite::EvalQueryResultRecord],
) -> Result<VariantConsistencyReport> {
    let golden_by_id: HashMap<&str, &GoldenQuery> =
        queries.iter().map(|q| (q.id.as_str(), q)).collect();

    let mut grouped: BTreeMap<String, Vec<VariantResult>> = BTreeMap::new();
    for row in rows {
        let qr: QueryResult = serde_json::from_str(&row.result_json)
            .with_context(|| format!("parse result_json for {}", row.query_id))?;
        let Some(gq) = golden_by_id.get(qr.query_id.as_str()) else {
            continue;
        };
        let Some(group) = gq.group.clone() else {
            continue;
        };
        let (recall_narrow, recall_pool) = recall_narrow_pool(&qr, &gq.expected_doc_ids);
        let answer_ok = qr.answer.as_ref().map(|a| {
            gq.must_contain.iter().all(|s| a.answer.contains(s))
                && !gq.forbidden.iter().any(|s| a.answer.contains(s))
        });
        let class = classify(&gq.expected_doc_ids, recall_narrow, recall_pool);
        grouped.entry(group).or_default().push(VariantResult {
            query_id: qr.query_id.clone(),
            query: qr.query.clone(),
            recall_narrow,
            recall_pool,
            answer_ok,
            class,
        });
    }

    let mut groups: Vec<VariantGroupReport> = Vec::with_capacity(grouped.len());
    for (group, variants) in grouped {
        groups.push(rollup_group(group, variants));
    }

    let total_groups = u32::try_from(groups.len()).unwrap_or(u32::MAX);
    let fully_consistent_groups = groups
        .iter()
        .filter(|g| g.recall_spread_narrow == 0.0 && g.worst_recall_narrow == 1.0)
        .count() as u32;
    let a_dominant_groups = groups
        .iter()
        .filter(|g| g.mis_ranked > 0 && g.mis_ranked >= g.missing)
        .count() as u32;
    let b_dominant_groups = groups
        .iter()
        .filter(|g| g.missing > 0 && g.missing > g.mis_ranked)
        .count() as u32;
    let mean_recall_spread_narrow = if groups.is_empty() {
        0.0
    } else {
        groups.iter().map(|g| g.recall_spread_narrow).sum::<f32>() / groups.len() as f32
    };

    Ok(VariantConsistencyReport {
        groups,
        mean_recall_spread_narrow,
        fully_consistent_groups,
        total_groups,
        a_dominant_groups,
        b_dominant_groups,
    })
}

/// 정답 문서 집합에 대한 recall@NARROW_K, recall@POOL_K.
/// 정답 미지정이면 (NaN, NaN).
fn recall_narrow_pool(qr: &QueryResult, expected: &[DocumentId]) -> (f32, f32) {
    if expected.is_empty() {
        return (f32::NAN, f32::NAN);
    }
    let exp: HashSet<&DocumentId> = expected.iter().collect();
    let cover = |k: u32| -> f32 {
        let topk: HashSet<&DocumentId> = qr
            .hits_top_k
            .iter()
            .filter(|h| h.rank <= k)
            .map(|h| &h.doc_id)
            .collect();
        exp.iter().filter(|d| topk.contains(*d)).count() as f32 / exp.len() as f32
    };
    (cover(NARROW_K), cover(POOL_K))
}

fn classify(expected: &[DocumentId], recall_narrow: f32, recall_pool: f32) -> VariantClass {
    if expected.is_empty() {
        VariantClass::NoExpected
    } else if recall_narrow >= 1.0 {
        VariantClass::Ok
    } else if recall_pool > recall_narrow {
        VariantClass::MisRanked
    } else {
        VariantClass::Missing
    }
}

fn rollup_group(group: String, variants: Vec<VariantResult>) -> VariantGroupReport {
    let measurable: Vec<f32> = variants
        .iter()
        .filter(|v| !v.recall_narrow.is_nan())
        .map(|v| v.recall_narrow)
        .collect();
    let (recall_spread_narrow, worst_recall_narrow) = if measurable.is_empty() {
        (0.0, f32::NAN)
    } else {
        let max = measurable.iter().copied().fold(f32::MIN, f32::max);
        let min = measurable.iter().copied().fold(f32::MAX, f32::min);
        (max - min, min)
    };
    let answer_flags: Vec<bool> = variants.iter().filter_map(|v| v.answer_ok).collect();
    let answer_consistency = if answer_flags.is_empty() {
        None
    } else {
        Some(answer_flags.iter().all(|&ok| ok))
    };
    let mis_ranked = variants.iter().filter(|v| v.class == VariantClass::MisRanked).count() as u32;
    let missing = variants.iter().filter(|v| v.class == VariantClass::Missing).count() as u32;
    VariantGroupReport {
        group,
        variants,
        recall_spread_narrow,
        worst_recall_narrow,
        answer_consistency,
        mis_ranked,
        missing,
    }
}

/// 활성 XDG Config로 저장된 run을 읽어 변형 일관성을 계산
/// ([`crate::metrics::compute_aggregate_with_config`]와 동일한 로딩 패턴).
pub fn compute_variant_consistency_with_config(
    cfg: &Config,
    run_id: &str,
) -> Result<VariantConsistencyReport> {
    let store = SqliteStore::open(cfg).context("open SqliteStore for variant consistency")?;
    store.run_migrations().context("run migrations")?;
    if store.load_eval_run(run_id).context("load eval_runs row")?.is_none() {
        anyhow::bail!("compute_variant_consistency: no eval_runs row for run_id {run_id}");
    }
    let rows = store
        .load_eval_query_results(run_id)
        .context("load eval_query_results")?;
    let queries = crate::metrics::load_golden_for_metrics()?;
    compute_variant_consistency(&queries, &rows)
}

/// 변형 일관성 리포트를 사람이 읽는 마크다운 표로 렌더
/// ([`crate::render_report_md`] 스타일).
pub fn render_variants_md(rep: &VariantConsistencyReport) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    let _ = writeln!(s, "# Variant consistency\n");
    let _ = writeln!(
        s,
        "groups={} fully_consistent={} A_dominant={} B_dominant={} mean_spread@{}={:.3}\n",
        rep.total_groups,
        rep.fully_consistent_groups,
        rep.a_dominant_groups,
        rep.b_dominant_groups,
        NARROW_K,
        rep.mean_recall_spread_narrow,
    );
    for g in &rep.groups {
        let ac = match g.answer_consistency {
            Some(true) => "all-ok",
            Some(false) => "MIXED",
            None => "n/a",
        };
        let _ = writeln!(
            s,
            "## {} — spread@{}={:.2} worst={:.2} A={} B={} answers={}",
            g.group, NARROW_K, g.recall_spread_narrow, g.worst_recall_narrow, g.mis_ranked, g.missing, ac
        );
        let _ = writeln!(s, "| variant | recall@{NARROW_K} | recall@{POOL_K} | class | answer |");
        let _ = writeln!(s, "|---|---|---|---|---|");
        for v in &g.variants {
            let ans = match v.answer_ok {
                Some(true) => "ok",
                Some(false) => "BAD",
                None => "-",
            };
            let _ = writeln!(
                s,
                "| {} | {:.2} | {:.2} | {:?} | {} |",
                v.query, v.recall_narrow, v.recall_pool, v.class, ans
            );
        }
        let _ = writeln!(s);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use kebab_core::{
        ChunkId, ChunkerVersion, Citation, IndexVersion, RetrievalDetail, ScoreKind, SearchMode,
        WorkspacePath,
    };
    use kebab_store_sqlite::EvalQueryResultRecord;

    fn hit(doc: &str, rank: u32) -> kebab_core::SearchHit {
        let path = WorkspacePath::new(format!("{doc}.md")).unwrap();
        kebab_core::SearchHit {
            rank,
            chunk_id: ChunkId(format!("c-{doc}-{rank}")),
            doc_id: DocumentId(doc.to_string()),
            doc_path: path.clone(),
            heading_path: vec![],
            section_label: None,
            snippet: String::new(),
            citation: Citation::Line { path, start: 1, end: 1, section: None },
            retrieval: RetrievalDetail {
                method: SearchMode::Vector,
                fusion_score: 1.0 / rank as f32,
                lexical_score: None,
                vector_score: Some(1.0 / rank as f32),
                lexical_rank: None,
                vector_rank: Some(rank),
            },
            index_version: IndexVersion("v1".into()),
            embedding_model: None,
            chunker_version: ChunkerVersion("v1".into()),
            indexed_at: time::OffsetDateTime::UNIX_EPOCH,
            stale: false,
            score_kind: ScoreKind::Cosine,
            repo: None,
            code_lang: None,
        }
    }

    fn gq(id: &str, group: &str, expected_doc: &str) -> GoldenQuery {
        GoldenQuery {
            id: id.into(),
            query: id.into(),
            lang: kebab_core::Lang(String::new()),
            expected_doc_ids: vec![DocumentId(expected_doc.into())],
            expected_chunk_ids: vec![],
            must_contain: vec![],
            forbidden: vec![],
            difficulty: None,
            group: Some(group.into()),
        }
    }

    fn row(query_id: &str, hits: Vec<kebab_core::SearchHit>) -> EvalQueryResultRecord {
        let qr = QueryResult {
            query_id: query_id.into(),
            query: query_id.into(),
            mode: SearchMode::Vector,
            hits_top_k: hits,
            answer: None,
            elapsed_ms: 0,
            error: None,
        };
        EvalQueryResultRecord {
            query_id: query_id.into(),
            result_json: serde_json::to_string(&qr).unwrap(),
        }
    }

    #[test]
    fn classifies_mis_ranked_vs_missing_and_spread() {
        // group "g": 정답 docX.
        //  v1: docX at rank 3  → narrow=1.0  → Ok
        //  v2: docX at rank 25 → narrow=0.0, pool=1.0 → MisRanked (A)
        //  v3: docX 없음        → narrow=0.0, pool=0.0 → Missing   (B)
        let queries = vec![gq("v1", "g", "docX"), gq("v2", "g", "docX"), gq("v3", "g", "docX")];
        let rows = vec![
            row("v1", vec![hit("docX", 3)]),
            row("v2", vec![hit("docX", 25)]),
            row("v3", vec![hit("other", 1)]),
        ];
        let rep = compute_variant_consistency(&queries, &rows).unwrap();
        assert_eq!(rep.total_groups, 1);
        let g = &rep.groups[0];
        assert_eq!(g.group, "g");
        assert_eq!(g.variants.len(), 3);
        // spread = max(1.0) - min(0.0) = 1.0
        assert!((g.recall_spread_narrow - 1.0).abs() < 1e-6);
        assert!((g.worst_recall_narrow - 0.0).abs() < 1e-6);
        assert_eq!(g.mis_ranked, 1);
        assert_eq!(g.missing, 1);
        let classes: Vec<VariantClass> = g.variants.iter().map(|v| v.class).collect();
        assert!(classes.contains(&VariantClass::Ok));
        assert!(classes.contains(&VariantClass::MisRanked));
        assert!(classes.contains(&VariantClass::Missing));
        assert_eq!(rep.a_dominant_groups + rep.b_dominant_groups, 1); // tie→정의대로 하나로 분류
    }

    #[test]
    fn fully_consistent_group_when_all_ok() {
        let queries = vec![gq("v1", "g", "docX"), gq("v2", "g", "docX")];
        let rows = vec![row("v1", vec![hit("docX", 1)]), row("v2", vec![hit("docX", 2)])];
        let rep = compute_variant_consistency(&queries, &rows).unwrap();
        assert_eq!(rep.fully_consistent_groups, 1);
        assert!((rep.groups[0].recall_spread_narrow - 0.0).abs() < 1e-6);
    }

    #[test]
    fn ungrouped_queries_are_ignored() {
        let mut q = gq("solo", "g", "docX");
        q.group = None;
        let rep = compute_variant_consistency(&[q], &[row("solo", vec![hit("docX", 1)])]).unwrap();
        assert_eq!(rep.total_groups, 0);
    }
}
