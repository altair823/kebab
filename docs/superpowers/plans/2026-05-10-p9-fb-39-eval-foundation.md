# fb-39 Eval Foundation (P@k Metric) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add chunk-level `precision_at_k_chunk` metric (P@5, P@10) to kebab-eval `AggregateMetrics`, plus golden-set ground-truth documentation strengthening — so a future fb-39b can measure whether a lever (chunk policy / RRF / cross-encoder / embedding upgrade) actually moves the rank-5+ noise needle.

**Architecture:** Single new field on `AggregateMetrics`, computed inside the existing `compute_aggregate_with_config` loop using the same accumulator pattern as `recall_at_k_doc` (sum-of-per-query-ratios / denominator), serialized via the existing `round_recall_map` helper. Denominator is k (fixed), matching the `hit_at_k` convention. Skip queries with empty `expected_chunk_ids`. Golden set schema unchanged — `expected_chunk_ids` is the ground truth (curator fills per-workspace).

**Tech Stack:** Rust 2024, serde, serde_yaml. No new deps.

**Spec:** `docs/superpowers/specs/2026-05-10-p9-fb-39-eval-foundation-design.md`

---

## File map

**Modify:**
- `crates/kebab-eval/src/metrics.rs` — add `precision_at_k_chunk` field on `AggregateMetrics`, init/accumulate/finalize inside `compute_aggregate_with_config`, plus unit tests.
- `fixtures/golden_queries.yaml` — strengthen header comment about `expected_chunk_ids` being P@k ground truth.
- `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md` — add `precision_at_k_chunk` to §11 eval metric table.
- `tasks/p9/p9-fb-39-retrieval-precision-tuning.md` — flip status, link design + plan, "lever 적용 deferred to fb-39b" banner.
- `tasks/INDEX.md` — flip fb-39 row to ✅ (eval foundation only).

**Create:** none.

---

## Task 1: Add precision_at_k_chunk field + serde backwards-compat

**Files:**
- Modify: `crates/kebab-eval/src/metrics.rs`

- [ ] **Step 1: Append failing test to `mod tests`**

```rust
#[test]
fn precision_at_k_chunk_field_default_empty_on_old_json() {
    // Old eval_runs.metrics_json predates fb-39 — no precision_at_k_chunk field.
    // serde(default) should yield empty BTreeMap.
    let old = serde_json::json!({
        "hit_at_k": {"1": 0.5, "3": 0.5, "5": 0.5, "10": 0.5},
        "mrr": 0.5,
        "recall_at_k_doc": {"1": 0.0, "3": 0.0, "5": 0.0, "10": 0.0},
        "citation_coverage": null,
        "groundedness": 0.0,
        "empty_result_rate": 0.0,
        "refusal_correctness": null,
        "total_queries": 1,
        "failed_queries": 0
    });
    let parsed: AggregateMetrics = serde_json::from_value(old).expect("backwards-compat deserialize");
    assert!(parsed.precision_at_k_chunk.is_empty());
}

#[test]
fn precision_at_k_chunk_field_serializes_when_populated() {
    let mut p = BTreeMap::new();
    p.insert(5, 0.6_f32);
    p.insert(10, 0.3_f32);
    let agg = AggregateMetrics {
        hit_at_k: BTreeMap::new(),
        mrr: 0.0,
        recall_at_k_doc: BTreeMap::new(),
        precision_at_k_chunk: p,
        citation_coverage: 0.0,
        groundedness: 0.0,
        empty_result_rate: 0.0,
        refusal_correctness: 0.0,
        total_queries: 0,
        failed_queries: 0,
    };
    let v = serde_json::to_value(&agg).unwrap();
    assert_eq!(v["precision_at_k_chunk"]["5"], 0.6);
    assert_eq!(v["precision_at_k_chunk"]["10"], 0.3);
}
```

- [ ] **Step 2: Run tests — expect compile errors (field undefined)**

```bash
cargo test -p kebab-eval --lib precision_at_k_chunk
```
Expected: errors — `precision_at_k_chunk` field missing on `AggregateMetrics`.

- [ ] **Step 3: Add field to `AggregateMetrics`**

In `crates/kebab-eval/src/metrics.rs`, find `pub struct AggregateMetrics { ... }` (~line 57). Add field after `recall_at_k_doc`:

```rust
    /// p9-fb-39: chunk-level precision at k. Binary relevance via
    /// `expected_chunk_ids` (a hit is "relevant" if its chunk_id is
    /// in the golden's `expected_chunk_ids`). Denominator is k (fixed)
    /// — `hits.len() < k` still divides by k, treating shortfall as
    /// precision loss (mirrors `hit_at_k`). Queries with empty
    /// `expected_chunk_ids` are skipped (mirrors `hit_at_k_chunk`).
    #[serde(default)]
    pub precision_at_k_chunk: BTreeMap<u32, f32>,
```

The other tests in the file (e.g. `hit_at_k_handles_ranks_1_4_miss`, `recall_at_k_doc_partial`) construct `AggregateMetrics` via the public `compute_aggregate_with_config` path, not via struct literal, so the new `#[serde(default)]` field does NOT break them. Only direct struct-literal constructions need updates — search the file to confirm:

```bash
grep -n "AggregateMetrics {" crates/kebab-eval/src/metrics.rs
```

For each direct struct-literal site, add `precision_at_k_chunk: BTreeMap::new(),` to the literal.

- [ ] **Step 4: Run tests — expect both new tests pass**

```bash
cargo test -p kebab-eval --lib precision_at_k_chunk
```
Expected: both pass.

- [ ] **Step 5: Run clippy**

```bash
cargo clippy -p kebab-eval --all-targets -- -D warnings
```
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/kebab-eval/src/metrics.rs
git commit -m "feat(eval): AggregateMetrics.precision_at_k_chunk field (fb-39)"
```

---

## Task 2: Compute precision_at_k_chunk in aggregate loop

**Files:**
- Modify: `crates/kebab-eval/src/metrics.rs`

- [ ] **Step 1: Append failing tests to `mod tests`**

(Use the existing `make_query_result` / fixture helpers — read the top of the test module for available helpers, e.g. `mk_qr_with_chunks(query_id, chunk_ids_with_ranks)`.)

```rust
#[test]
fn precision_at_k_chunk_exact_match() {
    // 1 query, expected = [c1, c2, c3]. Top-5 hits: [c1@1, c2@2, c3@3, x@4, y@5].
    // P@5 = 3/5 = 0.6. P@10 = 3/10 = 0.3.
    let queries = vec![mk_golden(
        "g1",
        &[],                                  // expected_doc_ids
        &["c1", "c2", "c3"],                  // expected_chunk_ids
        &[],                                  // must_contain
        &[],                                  // forbidden
        None,                                 // expected_refusal
    )];
    let rows = vec![mk_query_row(
        "g1",
        &[("c1", 1), ("c2", 2), ("c3", 3), ("x", 4), ("y", 5)],
    )];
    let agg = compute_from_inputs(&queries, &rows);
    assert_eq!(agg.precision_at_k_chunk[&5], 0.6);
    assert_eq!(agg.precision_at_k_chunk[&10], 0.3);
}

#[test]
fn precision_at_k_chunk_partial_topk_divides_by_k() {
    // 1 query, expected = [c1, c2]. Top hits: only [c1@1, c2@2] (3 results total).
    // P@5 = 2/5 = 0.4 (denominator k, not hits.len).
    let queries = vec![mk_golden("g1", &[], &["c1", "c2"], &[], &[], None)];
    let rows = vec![mk_query_row("g1", &[("c1", 1), ("c2", 2), ("x", 3)])];
    let agg = compute_from_inputs(&queries, &rows);
    assert_eq!(agg.precision_at_k_chunk[&5], 0.4);
    assert_eq!(agg.precision_at_k_chunk[&10], 0.2);
}

#[test]
fn precision_at_k_chunk_zero_relevant_in_topk() {
    // 1 query, expected = [c1]. Top hits all unrelated.
    // P@5 = 0/5 = 0.0.
    let queries = vec![mk_golden("g1", &[], &["c1"], &[], &[], None)];
    let rows = vec![mk_query_row("g1", &[("x", 1), ("y", 2), ("z", 3)])];
    let agg = compute_from_inputs(&queries, &rows);
    assert_eq!(agg.precision_at_k_chunk[&5], 0.0);
    assert_eq!(agg.precision_at_k_chunk[&10], 0.0);
}

#[test]
fn precision_at_k_chunk_empty_expected_skipped() {
    // 1 query, expected_chunk_ids = []. Should be skipped — denom 0 → entry value 0.0
    // (matches `recall_at_k_doc` behavior in `round_recall_map` for zero-denom).
    let queries = vec![mk_golden("g1", &[], &[], &[], &[], None)];
    let rows = vec![mk_query_row("g1", &[("c1", 1)])];
    let agg = compute_from_inputs(&queries, &rows);
    // Mirrors recall_at_k_doc: zero-denom → 0.0 in map (not absent).
    assert_eq!(agg.precision_at_k_chunk[&5], 0.0);
    assert_eq!(agg.precision_at_k_chunk[&10], 0.0);
}

#[test]
fn precision_at_k_chunk_two_queries_averaged() {
    // q1: expected=[c1], hits=[c1@1, x@2, y@3]    → P@5 = 1/5 = 0.2
    // q2: expected=[c1, c2], hits=[c1@1, c2@2]   → P@5 = 2/5 = 0.4
    // Avg P@5 = (0.2 + 0.4) / 2 = 0.3.
    let queries = vec![
        mk_golden("g1", &[], &["c1"], &[], &[], None),
        mk_golden("g2", &[], &["c1", "c2"], &[], &[], None),
    ];
    let rows = vec![
        mk_query_row("g1", &[("c1", 1), ("x", 2), ("y", 3)]),
        mk_query_row("g2", &[("c1", 1), ("c2", 2)]),
    ];
    let agg = compute_from_inputs(&queries, &rows);
    assert_eq!(agg.precision_at_k_chunk[&5], 0.3);
}
```

The `mk_golden` / `mk_query_row` / `compute_from_inputs` helpers are existing test helpers in this file. Read the top of `mod tests` (~line 380-510) to confirm the actual helper names and signatures. If your helpers have different shapes (e.g. `mk_qr_with_chunks(id, &[(chunk, rank)])`), adapt the test calls accordingly.

If those helpers don't exist, look for the pattern in the existing `hit_at_k_handles_ranks_1_4_miss` test (~line 513) and mirror it.

- [ ] **Step 2: Run tests — expect failures**

```bash
cargo test -p kebab-eval --lib precision_at_k_chunk
```
Expected: 5 failures — `precision_at_k_chunk` map empty (only `#[serde(default)]` populates it from JSON; the compute path doesn't yet).

- [ ] **Step 3: Implement aggregation in `compute_aggregate_with_config`**

In `crates/kebab-eval/src/metrics.rs`, find `compute_aggregate_with_config` body. After the `recall_at_k_doc` accumulator init (~line 188-189), add:

```rust
    let mut precision_at_k_chunk: BTreeMap<u32, (f64, u32)> =
        TOP_K_VARIANTS.iter().map(|k| (*k, (0.0_f64, 0_u32))).collect();
```

Inside the loop, after the existing `hit@k + MRR` block (~line 222-247) which already gates on `!gq.expected_chunk_ids.is_empty()`, add a sibling `for k in TOP_K_VARIANTS { ... }` that updates `precision_at_k_chunk`. Place it INSIDE the same `if !gq.expected_chunk_ids.is_empty() { ... }` block so the skip-empty policy is shared:

```rust
        // hit@k + MRR (chunk-level, requires non-empty expected_chunk_ids)
        if !gq.expected_chunk_ids.is_empty() {
            let expected: HashSet<&ChunkId> = gq.expected_chunk_ids.iter().collect();
            // ... existing hit@k + MRR computation ...

            // p9-fb-39: precision@k_chunk — count of top-k hits whose
            // chunk_id is in `expected`, divided by k (fixed denominator).
            for k in TOP_K_VARIANTS {
                let hits_in_topk_relevant = qr
                    .hits_top_k
                    .iter()
                    .filter(|h| h.rank <= *k && expected.contains(&h.chunk_id))
                    .count();
                let entry = precision_at_k_chunk.get_mut(k).expect("init");
                entry.0 += hits_in_topk_relevant as f64 / f64::from(*k);
                entry.1 += 1;
            }
        }
```

Then at the final `Ok(AggregateMetrics { ... })` return (~line 325-345), add:

```rust
        precision_at_k_chunk: round_recall_map(&precision_at_k_chunk),
```

(`round_recall_map` is the existing helper at line ~366; it accepts `BTreeMap<u32, (f64, u32)>` and divides sum by denom, returning `BTreeMap<u32, f32>`. Same shape used by `recall_at_k_doc`.)

- [ ] **Step 4: Run tests — expect all 5 pass**

```bash
cargo test -p kebab-eval --lib precision_at_k_chunk
```
Expected: 5 passes.

- [ ] **Step 5: Run full kebab-eval suite**

```bash
cargo test -p kebab-eval
cargo clippy -p kebab-eval --all-targets -- -D warnings
```
Expected: no regressions; clippy clean.

- [ ] **Step 6: Commit**

```bash
git add crates/kebab-eval/src/metrics.rs
git commit -m "feat(eval): compute precision_at_k_chunk in aggregate loop (fb-39)"
```

---

## Task 3: Strengthen golden YAML header documentation

**Files:**
- Modify: `fixtures/golden_queries.yaml`

- [ ] **Step 1: Read existing header**

```bash
head -20 fixtures/golden_queries.yaml
```

- [ ] **Step 2: Replace header comment**

Find the existing header (the comment block above the first `- id: g001` entry). Replace with:

```yaml
# Golden query suite for `kebab eval run` (P5-1 / P5-2 / fb-39).
#
# Top-level: list of queries. Required fields: `id`, `query`. All
# others are optional and default to empty / null.
#
# Curators: `expected_doc_ids` and `expected_chunk_ids` MUST refer to
# real rows in the active workspace's SQLite store at run time. Stale
# references make the runner bail at start. The shipped template
# leaves them empty so the file is loadable on any fresh workspace —
# fill them in after a `kebab ingest` to enable the metrics that
# require ground truth (P5-2 + fb-39):
#
#   - `expected_chunk_ids` →  hit_at_k, MRR, precision_at_k_chunk (fb-39)
#   - `expected_doc_ids`   →  recall_at_k_doc
#
# `precision_at_k_chunk` (fb-39): of the top-k retrieved hits, what
# fraction's `chunk_id` is in `expected_chunk_ids`. Denominator is k
# (fixed) — `top-k` shortfall is treated as precision loss. Queries
# with empty `expected_chunk_ids` are skipped from this metric.
#
# `must_contain` / `forbidden` drive the rule-based groundedness
# metric (P5-2).
```

- [ ] **Step 3: Verify YAML still parses**

```bash
cargo test -p kebab-eval --test golden_loader 2>/dev/null || cargo test -p kebab-eval load_golden
```

If a loader test exists, it should still pass. If not, run a quick parse check:

```bash
cargo run --bin kebab -- eval --help 2>/dev/null || true
```

(The shipped `golden_queries.yaml` is just a fixture — the workspace test loader will read it during integration tests and fail loudly if YAML is malformed.)

- [ ] **Step 4: Commit**

```bash
git add fixtures/golden_queries.yaml
git commit -m "docs(eval): document expected_chunk_ids as P@k ground truth (fb-39)"
```

---

## Task 4: Update design doc + spec status flip + INDEX

**Files:**
- Modify: `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md`
- Modify: `tasks/p9/p9-fb-39-retrieval-precision-tuning.md`
- Modify: `tasks/INDEX.md`

- [ ] **Step 1: Update design §11 eval metric list**

```bash
grep -n "^## §11\|^## 11\|hit_at_k\|recall_at_k_doc\|precision" docs/superpowers/specs/2026-04-27-kebab-final-form-design.md | head -10
```

Find the §11 eval section (or wherever metrics are listed). Add a `precision_at_k_chunk` line next to `hit_at_k` / `recall_at_k_doc`:

```markdown
- `precision_at_k_chunk` (fb-39): top-k 안 chunk_id 가 `expected_chunk_ids` 에 포함된 비율. 분모 = k (fixed). `expected_chunk_ids` 빈 query 는 skip.
```

If the design doc doesn't currently list metrics inline, add a short subsection or bullet under §11 introducing it.

- [ ] **Step 2: Flip task spec status**

```bash
sed -i.bak 's/^status: open$/status: completed/' tasks/p9/p9-fb-39-retrieval-precision-tuning.md
rm tasks/p9/p9-fb-39-retrieval-precision-tuning.md.bak
```

Replace the existing `> ⏳ **백로그 only — 미구현.**` skeleton banner with:

```markdown
> ✅ **Eval foundation 부분 구현 완료.** P@k metric (P@5, P@10) 추가. 본 spec 의 lever 적용 (chunk policy / RRF / cross-encoder / embedding 업그레이드) 은 별도 task 로 분리 (fb-39b 이후).
>
> - Design: [`docs/superpowers/specs/2026-05-10-p9-fb-39-eval-foundation-design.md`](../../docs/superpowers/specs/2026-05-10-p9-fb-39-eval-foundation-design.md)
> - Plan: [`docs/superpowers/plans/2026-05-10-p9-fb-39-eval-foundation.md`](../../docs/superpowers/plans/2026-05-10-p9-fb-39-eval-foundation.md)
```

- [ ] **Step 3: Flip INDEX row**

In `tasks/INDEX.md`, find the fb-39 row. Replace its status with `✅ 머지 (2026-05-10) — eval foundation only, lever 적용 deferred` (mirror the fb-42 row format from the previous PR for consistency).

- [ ] **Step 4: Workspace test + clippy gate**

```bash
cargo test --workspace --no-fail-fast -j 1 2>&1 | tail -10
cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -5
```

`-j 1` REQUIRED.

Expected: all green.

- [ ] **Step 5: Commit**

```bash
git add docs/superpowers/specs/2026-04-27-kebab-final-form-design.md tasks/p9/p9-fb-39-retrieval-precision-tuning.md tasks/INDEX.md
git commit -m "docs(fb-39): design §11 + spec status + INDEX (eval foundation)"
```

---

## Final verification checklist

- [ ] `cargo test --workspace --no-fail-fast -j 1` green
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` clean
- [ ] `kebab eval run` (against any workspace with non-empty `expected_chunk_ids` in golden) emits `precision_at_k_chunk: {5: ..., 10: ...}` in the run's `metrics_json`
- [ ] design §11 + INDEX + task spec status flipped
