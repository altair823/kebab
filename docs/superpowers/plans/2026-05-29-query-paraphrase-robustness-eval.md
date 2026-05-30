# Query-paraphrase Robustness Eval (Phase 1) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `kebab-eval`에 "같은 의미의 여러 표현(동의어·다른 어휘·풀어쓴 문장·한/영)"을 묶는 변형 그룹과, 그룹 내 답변/검색 품질 일관성을 재고 (A)순위출렁/(B)어휘격차를 판별하는 진단 메트릭을 추가한다.

**Architecture:** `GoldenQuery`에 `group: Option<String>` 추가(additive) → loader가 그룹 정합성 검증 → 신규 `variant.rs`가 저장된 run의 per-query 결과를 그룹으로 묶어 recall@narrow(10) vs recall@pool(50) 대비로 변형 일관성 + A/B 분류 산출 → `kebab eval variants <run_id>` CLI로 표/JSON 리포트. 기존 `AggregateMetrics` 경로는 불변(group=None이면 기존 동작).

**Tech Stack:** Rust 2024, `kebab-eval` 크레이트, serde/serde_yaml, anyhow, rusqlite(간접). 측정은 release `kebab` + dogfood KB.

**빌드/테스트 규약 (이 환경 필수):** 모든 cargo는 `CARGO_TARGET_DIR=/build/out/cargo-target/target` + `-j 4`, 결과를 **파일 redirect + exit code 확인 후에만** 커밋 (`grep|tail` 금지 — pipe exit가 cargo 실패를 마스킹). 출력 노이즈로 빌드 오독 사례 다수.

---

## File Structure

| File | 책임 | 변경 |
|---|---|---|
| `crates/kebab-eval/src/types.rs` | `GoldenQuery`에 `group` 필드 | Modify |
| `crates/kebab-eval/src/loader.rs` | 그룹 정합성 검증(`check_group_integrity`) | Modify |
| `crates/kebab-eval/src/variant.rs` | 변형 일관성 메트릭 + A/B 분류 + 렌더 | **Create** |
| `crates/kebab-eval/src/lib.rs` | `variant` 모듈 등록 + re-export | Modify |
| `crates/kebab-cli/src/main.rs` | `kebab eval variants <run_id>` 서브커맨드 | Modify |
| `/build/dogfood/golden_queries.yaml` | 변형 그룹 큐레이션 (in-repo 아님) | Modify (data) |

---

## Task 1: `group` 필드 + loader 그룹 정합성 검증

**모델:** sonnet (작은 스키마 + 검증 함수)

**Files:**
- Modify: `crates/kebab-eval/src/types.rs:13-29` (GoldenQuery)
- Modify: `crates/kebab-eval/src/loader.rs` (`load_golden_set` + 신규 `check_group_integrity`)
- Test: `crates/kebab-eval/src/loader.rs` (in-module `#[cfg(test)]`)

- [ ] **Step 1: `group` 필드 추가**

`crates/kebab-eval/src/types.rs`의 `GoldenQuery`에 `difficulty` 아래로 추가:

```rust
    #[serde(default)]
    pub difficulty: Option<String>,
    /// 같은 의미의 여러 표현(동의어·다른 어휘·풀어쓴 문장·한/영)을 묶는
    /// 의도 그룹 id. 같은 그룹의 모든 변형은 동일한 `expected_doc_ids`(집합)를
    /// 공유해야 한다(loader가 강제). `None`이면 단독 쿼리(기존 동작 불변).
    #[serde(default)]
    pub group: Option<String>,
```

- [ ] **Step 2: 실패하는 테스트 작성**

`crates/kebab-eval/src/loader.rs`의 `#[cfg(test)] mod tests` 안에 추가:

```rust
    #[test]
    fn rejects_group_with_divergent_expected_docs() {
        let tmp = tempdir().unwrap();
        let yaml_path = tmp.path().join("golden.yaml");
        fs::write(
            &yaml_path,
            "- id: g1\n  query: \"러스트 소유권\"\n  group: ownership\n  expected_doc_ids: [\"docA\"]\n\
             - id: g2\n  query: \"rust ownership\"\n  group: ownership\n  expected_doc_ids: [\"docB\"]\n",
        )
        .unwrap();
        let err = load_golden_set(&yaml_path).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("group"), "msg: {msg}");
        assert!(msg.contains("ownership"), "msg: {msg}");
    }

    #[test]
    fn accepts_group_with_matching_expected_docs() {
        let tmp = tempdir().unwrap();
        let yaml_path = tmp.path().join("golden.yaml");
        fs::write(
            &yaml_path,
            "- id: g1\n  query: \"러스트 소유권\"\n  group: ownership\n  expected_doc_ids: [\"docA\"]\n\
             - id: g2\n  query: \"rust ownership\"\n  group: ownership\n  expected_doc_ids: [\"docA\"]\n",
        )
        .unwrap();
        let qs = load_golden_set(&yaml_path).unwrap();
        assert_eq!(qs.len(), 2);
        assert_eq!(qs[0].group.as_deref(), Some("ownership"));
    }
```

- [ ] **Step 3: 테스트 실패 확인**

Run: `CARGO_TARGET_DIR=/build/out/cargo-target/target cargo test -p kebab-eval -j 4 rejects_group_with_divergent > /build/cache/tmp/t1.txt 2>&1; echo "EXIT=$?"`
Expected: 컴파일은 되나 `rejects_group_with_divergent_expected_docs` FAIL (현재 정합성 검증 없음 → `load_golden_set`이 Ok 반환).

- [ ] **Step 4: `check_group_integrity` 구현 + 배선**

`crates/kebab-eval/src/loader.rs`의 `load_golden_set`에서 `check_unique_ids(&queries)?;` 바로 다음 줄에 `check_group_integrity(&queries)?;` 추가. `check_unique_ids` 함수 아래에 신규 함수:

```rust
/// 같은 `group`에 속한 모든 쿼리가 동일한 `expected_doc_ids`(집합)를
/// 공유하는지 검증. 변형 일관성 메트릭은 "같은 정답을 가진 다른 표현들"을
/// 전제하므로, 그룹 내 정답이 갈리면 측정이 무의미해진다 → bail.
fn check_group_integrity(queries: &[GoldenQuery]) -> Result<()> {
    use std::collections::BTreeMap;
    // group -> (대표 정답 집합, 대표 query id)
    let mut canonical: BTreeMap<&str, (BTreeSet<String>, &str)> = BTreeMap::new();
    let mut offenders: BTreeSet<String> = BTreeSet::new();
    for q in queries {
        let Some(group) = q.group.as_deref() else {
            continue;
        };
        let docs: BTreeSet<String> = q.expected_doc_ids.iter().map(|d| d.0.clone()).collect();
        match canonical.get(group) {
            None => {
                canonical.insert(group, (docs, q.id.as_str()));
            }
            Some((expected, _first)) if *expected != docs => {
                offenders.insert(group.to_string());
            }
            Some(_) => {}
        }
    }
    if offenders.is_empty() {
        Ok(())
    } else {
        let list: Vec<String> = offenders.into_iter().collect();
        Err(anyhow!(
            "group(s) with divergent expected_doc_ids (same group must share one expected doc set): {}",
            list.join(", ")
        ))
    }
}
```

`BTreeSet`는 파일 상단 `use std::collections::{BTreeSet, HashSet};`에 이미 포함됨(확인). 누락 시 추가.

- [ ] **Step 5: 테스트 통과 확인**

Run: `CARGO_TARGET_DIR=/build/out/cargo-target/target cargo test -p kebab-eval -j 4 group > /build/cache/tmp/t1b.txt 2>&1; echo "EXIT=$?"`
Expected: `rejects_group_with_divergent_expected_docs` + `accepts_group_with_matching_expected_docs` PASS. EXIT=0.

- [ ] **Step 6: clippy + 커밋**

Run: `CARGO_TARGET_DIR=/build/out/cargo-target/target cargo clippy -p kebab-eval --all-targets -j 4 -- -D warnings > /build/cache/tmp/c1.txt 2>&1; echo "EXIT=$?"`
Expected: EXIT=0.

```bash
git add crates/kebab-eval/src/types.rs crates/kebab-eval/src/loader.rs
git commit -m "feat(eval): GoldenQuery.group + 그룹 정합성 검증 (변형 일관성 기반)"
```

---

## Task 2: 변형 일관성 메트릭 모듈 (`variant.rs`)

**모델:** opus (핵심 로직 — recall@narrow/pool, A/B 분류, 그룹 롤업)

**Files:**
- Create: `crates/kebab-eval/src/variant.rs`
- Modify: `crates/kebab-eval/src/lib.rs` (모듈 등록 + re-export)
- Test: `crates/kebab-eval/src/variant.rs` (in-module `#[cfg(test)]`)

- [ ] **Step 1: 모듈 골격 + 타입 작성**

`crates/kebab-eval/src/variant.rs` 생성:

```rust
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
```

- [ ] **Step 2: 실패하는 테스트 작성**

같은 파일 하단에:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use kebab_core::{
        ChunkId, ChunkerVersion, Citation, IndexVersion, RetrievalDetail, SearchMode, WorkspacePath,
        ScoreKind,
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
```

- [ ] **Step 3: 테스트 실패 확인**

먼저 `lib.rs`에 모듈 등록(아래 Step 5 일부 선행): `crates/kebab-eval/src/lib.rs`의 모듈 선언부에 `mod variant;` + `pub use variant::{VariantConsistencyReport, VariantGroupReport, VariantResult, VariantClass, compute_variant_consistency, compute_variant_consistency_with_config, render_variants_md};` 추가(아직 함수 미정의 → 다음 스텝에서 채움). 우선 컴파일 통과를 위해 `compute_variant_consistency`만 stub 없이 진행하면 컴파일 에러로 실패함을 확인.

Run: `CARGO_TARGET_DIR=/build/out/cargo-target/target cargo test -p kebab-eval -j 4 variant > /build/cache/tmp/t2.txt 2>&1; echo "EXIT=$?"`
Expected: 컴파일 에러(함수 미정의). 다음 스텝에서 구현.

- [ ] **Step 4: `compute_variant_consistency` + 헬퍼 구현**

`variant.rs`의 타입 정의 아래, `#[cfg(test)]` 위에 추가:

```rust
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
        let max = measurable.iter().cloned().fold(f32::MIN, f32::max);
        let min = measurable.iter().cloned().fold(f32::MAX, f32::min);
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
    let queries = crate::metrics::load_golden_for_metrics_pub()?;
    compute_variant_consistency(&queries, &rows)
}
```

주: `compute_variant_consistency_with_config`는 golden 로드에 `metrics`의 비공개 헬퍼가 필요하다. `crates/kebab-eval/src/metrics.rs`의 `fn load_golden_for_metrics()`를 `pub(crate) fn load_golden_for_metrics_pub()`로 노출하는 얇은 래퍼를 추가하거나, 기존 `load_golden_for_metrics`를 `pub(crate)`로 바꿔 `crate::metrics::load_golden_for_metrics()`로 직접 호출. **후자 채택**: `metrics.rs`의 `fn load_golden_for_metrics`를 `pub(crate) fn load_golden_for_metrics`로 변경하고, 위 호출을 `crate::metrics::load_golden_for_metrics()?`로 수정.

- [ ] **Step 5: 렌더 함수 + lib.rs 등록**

`variant.rs`에 사람이 읽는 표 렌더 추가(`#[cfg(test)]` 위):

```rust
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
```

`crates/kebab-eval/src/lib.rs`: 모듈 선언 영역에 `mod variant;` 추가, re-export에 추가:

```rust
pub use variant::{
    VariantClass, VariantConsistencyReport, VariantGroupReport, VariantResult,
    compute_variant_consistency, compute_variant_consistency_with_config, render_variants_md,
};
```

(기존 `pub use` 패턴은 `lib.rs`에서 `compare`/`metrics` re-export를 보고 맞춤. 정확한 위치/형식은 그 패턴을 따른다.)

- [ ] **Step 6: 테스트 + clippy 통과 확인**

Run: `CARGO_TARGET_DIR=/build/out/cargo-target/target cargo test -p kebab-eval -j 4 > /build/cache/tmp/t2b.txt 2>&1; echo "EXIT=$?"`
Expected: 3개 신규 variant 테스트 + 기존 테스트 모두 PASS. EXIT=0. (기존 `aggregate` 테스트가 그대로 통과 = group=None 경로 불변 회귀 가드)

Run: `CARGO_TARGET_DIR=/build/out/cargo-target/target cargo clippy -p kebab-eval --all-targets -j 4 -- -D warnings > /build/cache/tmp/c2.txt 2>&1; echo "EXIT=$?"`
Expected: EXIT=0.

- [ ] **Step 7: 커밋**

```bash
git add crates/kebab-eval/src/variant.rs crates/kebab-eval/src/lib.rs crates/kebab-eval/src/metrics.rs
git commit -m "feat(eval): 변형 일관성 메트릭 + A/B(순위출렁/어휘격차) 분류"
```

---

## Task 3: CLI `kebab eval variants <run_id>` 서브커맨드

**모델:** sonnet (작은 CLI 배선)

**Files:**
- Modify: `crates/kebab-cli/src/main.rs` (`EvalWhat` enum ~414 + `Cmd::Eval` 매치 ~1361)
- Test: 수동 (Task 5에서 실제 run으로 검증) + 컴파일/clippy

- [ ] **Step 1: `EvalWhat::Variants` 변형 추가**

`crates/kebab-cli/src/main.rs`의 `enum EvalWhat`에 `Aggregate` 변형 옆으로 추가 (clap 파생 스타일은 인접 변형을 그대로 따른다):

```rust
    /// 변형 그룹 일관성 진단 — 같은 의도의 여러 표현에서 recall@10 vs
    /// recall@50 대비로 (A)순위출렁/(B)어휘격차를 판별.
    Variants {
        /// 진단할 저장된 run_id.
        run_id: String,
        /// JSON으로 출력 (기본은 마크다운 표).
        #[arg(long)]
        json: bool,
    },
```

- [ ] **Step 2: `Cmd::Eval` 매치 암(arm) 추가**

`Cmd::Eval { what } => { match what { ... } }` 내부, `EvalWhat::Aggregate { .. } => { .. }` 암 다음에:

```rust
                EvalWhat::Variants { run_id, json } => {
                    let rep = kebab_eval::compute_variant_consistency_with_config(&cfg, run_id)?;
                    if *json {
                        println!("{}", serde_json::to_string_pretty(&rep)?);
                    } else {
                        print!("{}", kebab_eval::render_variants_md(&rep));
                    }
                }
```

(`cfg`는 같은 스코프에서 `EvalWhat::Aggregate` 암이 쓰는 것과 동일하게 로드됨 — 그 암의 `cfg` 획득 방식을 그대로 따른다. `run_id`가 `&String`이면 `compute_..._with_config(&cfg, run_id)`로 deref 강제됨; 필요시 `run_id.as_str()`.)

- [ ] **Step 3: 빌드 + clippy 통과 확인**

Run: `CARGO_TARGET_DIR=/build/out/cargo-target/target cargo build -p kebab-cli -j 4 > /build/cache/tmp/t3.txt 2>&1; echo "EXIT=$?"`
Expected: EXIT=0.

Run: `CARGO_TARGET_DIR=/build/out/cargo-target/target cargo clippy -p kebab-cli --all-targets -j 4 -- -D warnings > /build/cache/tmp/c3.txt 2>&1; echo "EXIT=$?"`
Expected: EXIT=0.

- [ ] **Step 4: 커밋**

```bash
git add crates/kebab-cli/src/main.rs
git commit -m "feat(cli): kebab eval variants <run_id> — 변형 일관성 진단 리포트"
```

---

## Task 4: dogfood golden_queries.yaml 변형 그룹 큐레이션

**모델:** opus (정답 문서를 corpus 의미로 판정 — 판단 필요)

**Files:**
- Modify: `/build/dogfood/golden_queries.yaml` (in-repo 아님 — dogfood 데이터)

**큐레이션 원칙 (순환 회피, [[feedback_search_quality_dogfood]]):** 정답 *문서*는 corpus 의미로
판정한다. **검색 결과 상위를 정답으로 베끼지 말 것.** 의도에 맞는 문서를 corpus 내용으로 고른 뒤,
그 문서의 doc_id/chunk_id를 SQLite에서 조회한다.

- [ ] **Step 1: 의도(그룹) 6–10개 선정**

선행 ablation 토픽 재사용 + 동의어/다른어휘/풀어쓴문장 추가. 후보 의도(각 그룹 3–5 표현):

| group | 표현 예시 (한/영/동의어/풀어쓴문장) |
|---|---|
| `ownership` | "러스트 소유권" / "rust ownership" / "러스트 메모리 소유권 규칙" / "who owns a value in rust" |
| `lifetime` | "러스트 lifetime" / "rust lifetime" / "러스트 수명" / "빌림 검사기 수명" |
| `database_index` | "데이터베이스 인덱스" / "database index" / "DB 색인" / "쿼리 빠르게 하는 인덱스" |
| `gc` | "가비지 컬렉션" / "garbage collection" / "자동 메모리 회수" |
| `async` | "비동기 프로그래밍" / "async programming" / "논블로킹 동시성" |
| `kubernetes_deploy` | "쿠버네티스 배포" / "kubernetes deployment" / "k8s 앱 배포" |

(corpus에 명확한 정답 문서가 없는 의도는 제외. rust류 + 일반 토픽 섞기.)

- [ ] **Step 2: 각 의도의 정답 문서를 corpus 의미로 판정 + ID 조회**

dogfood KB(`/build/dogfood/config.toml`)에서, 의도별로 corpus 내용상 그 주제를 다루는 문서를
식별한다. doc_id/chunk_id 조회 (release 바이너리):

```bash
BIN=/build/out/cargo-target/target/release/kebab
CFG=/build/dogfood/config.toml
# 후보 문서를 폭넓게 본 뒤 내용으로 정답 판정 (상위 1개 자동채택 금지):
$BIN search "rust ownership" --config $CFG --mode hybrid --k 20 --json --quiet \
  | python3 -c 'import sys,json; [print(h["doc_id"], h.get("doc_path"), h["chunk_id"]) for h in json.load(sys.stdin)["hits"]]'
```

각 그룹마다: 내용으로 맞는 문서 1–2개의 `doc_id`(+대표 `chunk_id`)를 확정. 같은 그룹의 모든 변형은
**동일한 `expected_doc_ids`** 를 갖는다(Task 1의 정합성 검증이 강제).

- [ ] **Step 3: must_contain 핵심 사실 큐레이션 (그룹 공유)**

각 그룹에 답변이 반드시 포함해야 할 핵심 substring 1–2개 (정답 문서 내용에서 발췌). 한/영 답변
모두에서 성립하는 표현으로 (예: 고유명사·숫자·식별자). 너무 길거나 표현 특정적이면 피한다.

- [ ] **Step 4: yaml에 그룹 엔트리 추가**

`/build/dogfood/golden_queries.yaml`에 그룹별로 추가 (기존 dg0xx 엔트리는 유지). 형식:

```yaml
# --- variant groups (paraphrase robustness, 2026-05-29) ---
- id: vg_ownership_ko
  query: "러스트 소유권"
  lang: ko
  group: ownership
  difficulty: medium
  expected_doc_ids: ["<조회한 doc_id>"]
  expected_chunk_ids: ["<조회한 chunk_id>"]
  must_contain: ["<핵심 사실>"]
- id: vg_ownership_en
  query: "rust ownership"
  lang: en
  group: ownership
  difficulty: medium
  expected_doc_ids: ["<같은 doc_id>"]
  expected_chunk_ids: ["<같은 chunk_id>"]
  must_contain: ["<같은 핵심 사실>"]
# ... (그룹당 3–5 변형, 그룹 6–10개)
```

- [ ] **Step 5: 로드 검증 (정합성 + ID 실재)**

release 바이너리로 eval run 시작 직전까지 가서 loader가 통과하는지 확인 (Task 5의 run이 시작 시
ID 실재 + 그룹 정합성을 검증 → bail 안 하면 OK). 빠른 단독 검증:

```bash
KEBAB_EVAL_GOLDEN=/build/dogfood/golden_queries.yaml \
$BIN eval run --config $CFG --mode hybrid --k 50 --json --quiet > /build/cache/tmp/t4_loadcheck.txt 2>&1
echo "EXIT=$?"   # 0 또는 run 진행이면 로드 통과; "duplicate"/"divergent"/"missing" 이면 수정
```

(이 run 자체가 Task 5의 측정으로 이어짐 — 여기선 로드 통과만 확인.)

- [ ] **Step 6: 커밋 불요 (dogfood 데이터)**

`/build/dogfood/`는 repo 밖. 큐레이션 결과는 Task 5 측정 후 HOTFIXES에 그룹 목록을 요약 기록.

---

## Task 5: 측정 실행 + (A)/(B) 진단 리포트

**모델:** 오케스트레이터(나) 직접 또는 sonnet

**Files:**
- 산출: `/build/cache/tmp/rr_variant_*.txt`, `tasks/HOTFIXES.md`(dated entry), 핸드오프 갱신

- [ ] **Step 1: release 빌드**

Run (background): `CARGO_TARGET_DIR=/build/out/cargo-target/target cargo build --release -p kebab-cli -j 4 > /build/cache/tmp/rr_variant_build.txt 2>&1; echo "EXIT=$?"`
Expected: EXIT=0. 바이너리 mtime이 갱신됐는지 확인.

- [ ] **Step 2: eval run (k=50, hybrid + vector, with-rag)**

```bash
BIN=/build/out/cargo-target/target/release/kebab
CFG=/build/dogfood/config.toml
export KEBAB_EVAL_GOLDEN=/build/dogfood/golden_queries.yaml
# 검색 전용(빠름) — recall 진단의 핵심:
$BIN eval run --config $CFG --mode hybrid --k 50 > /build/cache/tmp/rr_variant_run_hybrid.txt 2>&1; echo "EXIT=$?"
# run_id를 출력에서 추출 (clean grep)
```

`--with-rag`는 answer_consistency가 필요할 때만 (LLM 비용 큼). 1차는 검색 전용으로 recall 기반
A/B 진단부터. answer_consistency는 별도 `--with-rag` run으로.

- [ ] **Step 3: variants 리포트 산출**

```bash
$BIN eval variants <run_id> --config $CFG > /build/cache/tmp/rr_variant_report_hybrid.txt 2>&1; echo "EXIT=$?"
$BIN eval variants <run_id> --config $CFG --json > /build/cache/tmp/rr_variant_report_hybrid.json 2>&1; echo "EXIT=$?"
```

- [ ] **Step 4: 결과 Read 검증 + A/B 판정**

`/build/cache/tmp/rr_variant_report_hybrid.txt`를 Read로 직접 확인 (측정값 추측 절대 금지,
[[project_rerank_experiment]] 교훈). 판정:
- `a_dominant_groups > b_dominant_groups` → (A) 우세 → Phase 2 처방 = near-tie 흡수.
- `b_dominant_groups > a_dominant_groups` → (B) 우세 → Phase 2 처방 = 쿼리 확장/번역.
- 혼재면 그룹별로 분리 처방 + 토픽 특성 기록.

- [ ] **Step 5: HOTFIXES + 핸드오프 기록**

`tasks/HOTFIXES.md`에 dated entry: 그룹 목록, recall_spread/worst 표, A/B 분류, Phase 2 방향.
핸드오프 문서에 측정 결과 + Phase 2 게이트 결정.

```bash
git add tasks/HOTFIXES.md docs/superpowers/handoffs/2026-05-29-crossscript-rerank-progress-handoff.md
git commit -m "docs: 변형 일관성 측정 결과 + Phase 2 처방 방향 (A/B 진단)"
```

---

## Self-Review (작성자 점검)

**1. Spec coverage:**
- spec §2 Phase 1 "변형 그룹 + 일관성 메트릭 + A/B 판별 + 큐레이션 + 측정" → Task 1(그룹), Task 2(메트릭+A/B), Task 3(surface), Task 4(큐레이션), Task 5(측정). ✓
- spec §3 "kebab-eval 단독, AggregateMetrics 불변" → Task 2 Step 6이 기존 테스트 통과로 회귀 가드. ✓
- spec §5 "clean 측정 + Read 검증 + baseline이 deliverable" → Task 5 Step 4. ✓
- spec §7 미결: group 정합성=bail(Task 1), A/B 임계=classify 정의(Task 2), surface=`eval variants`(Task 3), 큐레이션(Task 4), must_contain(Task 4 Step 3). ✓

**2. Placeholder scan:** Task 4의 `<조회한 doc_id>` 등은 데이터 큐레이션의 실제 조회 산출물(코드 placeholder 아님). 코드 스텝은 전부 완성 코드. ✓

**3. Type consistency:** `compute_variant_consistency(queries, rows)` 시그니처가 Task 2 정의 ↔ Task 2 `_with_config` 호출 ↔ Task 3 CLI 호출에서 일치. `VariantConsistencyReport`/`render_variants_md` 이름이 lib.rs re-export(Task 2 Step 5) ↔ CLI(Task 3 Step 2)에서 일치. `EvalQueryResultRecord{query_id, result_json}` 필드가 Task 2 테스트 ↔ 실제 metrics.rs 사용과 일치. ✓

**의존성 주의:** Task 2가 `metrics::load_golden_for_metrics`를 `pub(crate)`로 승격(Step 4 주석) → 그 변경이 Task 2 커밋에 포함됨(`git add ... metrics.rs`). Task 3는 Task 2의 re-export에 의존 → 순서 준수.
