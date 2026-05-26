---
title: "HOTFIX #15 — kebab-mcp `ask_tool_routes_multi_hop_true_to_decompose_first` 가 빈-KB 에서 no_chunks 로 short-circuit (multi-hop dispatch contract 변경 후 stale)"
date: 2026-05-26
task_id: hotfix-15
status: open
target_version: 0.18.1
sibling_of: tasks/HOTFIXES.md "2026-05-25 — fb-41 pre-v0.18 dogfood … PR-9 NLI refusal: terminal Synthesize hop omitted from hops trace"
test_file: crates/kebab-mcp/tests/tools_call_ask_multi_hop.rs
test_name: ask_tool_routes_multi_hop_true_to_decompose_first
---

# HOTFIX #15 — MCP ask multi_hop test 의 dispatch-divergence assertion 이 새 multi-hop probe-first contract 와 불일치

## §1. 진단 (Root cause)

`crates/kebab-rag/src/pipeline.rs::RagPipeline::ask_multi_hop` (lines 646–700) 의 **"Step 0. Pre-decompose score-gate probe"** 가 multi-hop 의 dispatch shape 를 바꿨다. fb-41 v0.18 pre-cut dogfood (S7 hallucination 회귀) 의 fix 로, *decompose 전에* original query 를 retrieve probe 하여 빈 hits / sub-gate 스코어면 single-pass 와 같은 refuse 경로로 short-circuit 한다. 관련 코드 (line 673–700):

```rust
// ── 0. Pre-decompose score-gate probe (v0.18 dogfood fix) ──────────
let probe_query = SearchQuery { text: query.to_string(), mode: opts.mode, k: k_effective, .. };
let mut probe_hits = self.retriever.search(&probe_query).context(..)?;
// (stale 스탬프 생략)
if probe_hits.is_empty() {
    return self.refuse_no_chunks(query, &opts, k_effective, started, None);  // ← test 가 fail 하는 지점
}
if probe_hits[0].retrieval.fusion_score < self.config.rag.score_gate {
    return self.refuse_score_gate(query, &opts, &probe_hits, k_effective, started, None);
}
// ── 1. Decompose (iter 0) ─ (이후 LLM 호출 시작)
```

### Test 의 stale contract

`tools_call_ask_multi_hop.rs::ask_tool_routes_multi_hop_true_to_decompose_first` (PR-5 작성, `8a2f7af`) 는 두 가지 가정에 의존:

1. **모듈 doc (line 6–12)**: *"Single-pass retrieves first (empty KB → NoChunks refusal, no LLM call). Multi-hop calls decompose first (no retrieval yet), so an empty KB + no Ollama yields error.v1 with code=model_unreachable — different wire shape than the refusal envelope. The two surfaces' divergence is the signal that the multi_hop arg actually routed the dispatch."*
2. **Setup (line 53–65)**: 빈 workspace_root + `ingest_with_config(empty)` → 빈 KB.

새 contract (PR-7 dogfood fix) 에서는 빈 KB 의 multi-hop probe 도 retrieve 를 호출하여 `probe_hits.is_empty() → refuse_no_chunks` 로 short-circuit. 결과적으로:
- `is_error = false` (refusal envelope)
- `schema_version = answer.v1` (`Answer` envelope, not `error.v1`)
- `refusal_reason = NoChunks`, `chunks_returned = 0`

실측 actual:
```
Answer { refusal_reason: "no_chunks", chunks_returned: 0, is_error: Some(false) }
```

→ Line 86–89 의 `assert!(mh.is_error.unwrap_or(false), "multi_hop=true must reach the LLM (decompose first) — got {mh:?}")` 가 panic.

### Wire 상 dispatch divergence 가 사라진 추가 사실

`refuse_no_chunks` (line 1449–1500) 는 `prompt_template_version` 을 `self.config.rag.prompt_template_version` (single-pass 의 default, e.g. `rag-default-v1`) 로 stamp — *not* `PROMPT_TEMPLATE_VERSION_MULTI_HOP` (`"rag-multi-hop-v1"`). 즉 multi-hop probe 가 가져온 refuse Answer 는 wire 상으로 single-pass refuse 와 **byte-identical** (`hops: None` 으로 동일). 따라서 *empty KB 에서는 dispatch divergence 를 wire shape 로 pin 할 방법이 없다* — production 의 새 contract 의 의도된 결과. (다음 hops 추적이 들어가는 path 는 line 720+ 의 decompose 가 호출된 후, 즉 probe 통과 후만 가능.)

### 마지막 PASS 시점 + 변경 추적

- `8a2f7af feat(mcp): fb-41 PR-5 — MCP ask multi_hop arg + SKILL.md 안내` — test 첫 작성 (probe step 도입 전, PASS).
- `2422182 chore(mcp): PR #172 회차 1 리뷰 반영` — 단순 리뷰 반영 (probe step 무관, PASS).
- `7c27633 chore(rag): post-PR9 refactor` — `request_timeout_secs 2→5` 와 `mh_code` discriminator 제거 만, line 86 의 `is_error` assertion 은 그대로. **이 시점에 commit message 가 직접 인정**: *"1 pass + 1 pre-existing flaky (HOTFIX #15, no_chunks short-circuit, executor D fix 와 무관 — line 86 의 base assertion 이 fixture 없어서 fail)"*.

실제 production 의 dispatch shape 가 바뀐 commit 은 PR-7 (probe step 도입; `tasks/HOTFIXES.md` line 33 의 `### Fix (PR-7)` entry). Test 는 PR-5 의 contract 를 그대로 보존한 채 PR-7 의 production 변경을 흡수 못함 → flake.

**결론 — root cause**: PR-7 (v0.18 pre-cut dogfood fix) 가 `ask_multi_hop` 에 pre-decompose probe-first short-circuit 을 도입한 후, PR-5 시점의 test 가 "empty KB → multi-hop 은 decompose first → LLM 도달 → error.v1" 라는 *stale* 한 dispatch contract 에 assertion 을 박아두어 빈-KB fixture 에서 deterministic fail. Fixture 추가 없이는 새 contract 하에서 통과 불가능.

### Alternative root causes considered and ruled out (HOTFIX rigor)

- **Ingest 실패**: `ingest_with_config(empty)` 는 빈 workspace 에서 정상 동작 (0 chunks ingest, no error). production code 와 test fixture setup 정합.
- **Ollama dispatch**: 의도된 unreachable (`127.0.0.1:1`). `model_unreachable` / `timeout` error.v1 emit 은 *decompose 가 LLM 도달 시* 만 — current 의 probe short-circuit 가 그 전에 차단.
- **FTS5 tokenizer 변경**: V007 trigram migration (2026-05-24 v0.17.0 PR-A) 은 빈 KB 에 chunks 0이라 *영향 없음* — probe_hits 가 empty 인 이유는 KB 가 비어 있어서, tokenizer 와 무관.
- **Score gate default 변경**: `config.rag.score_gate` 의 default (0.30) 가 PR-7 시점에 변경된 적 없음. test config `minimal_config()` 가 default 그대로 사용.

---

## §2. 영향

| Surface | 상태 |
|---|---|
| Production behavior (`kebab ask --multi-hop`) | ✅ **올바름.** PR-7 의 probe-first 가 out-of-corpus hallucination 회귀 (S7) 를 막는다. 의도된 동작. |
| Production wire (`answer.v1`, `error.v1`) | ✅ **변경 없음.** refuse_no_chunks 의 envelope 가 single-pass 와 동일. |
| Test `ask_tool_routes_multi_hop_true_to_decompose_first` | ❌ **deterministic fail** on PR-9a/b/c/d 의 모든 workspace test (`cargo test --workspace --no-fail-fast -j 1`). |
| Sibling test `ask_input_schema_advertises_multi_hop_field` | ✅ **PASS.** JsonSchema serialization 만 검증, retrieval 호출 없음. |
| CI / dogfood | ❌ workspace 회귀 시그널이 1 known flaky 로 잡음 누적. 신규 회귀가 같은 binary 에서 발생 시 식별 지연 위험. |
| 사용자 binary (v0.18.0) | ✅ **영향 없음.** 출시된 binary 의 동작은 PR-7 dogfood fix 에 따라 *원래 의도대로*. |

요약: production 은 깨지지 않았다. 테스트의 contract 가 stale.

---

## §3. Fix design

세 가지 option 비교. 권장은 **Option A**.

### Option A — Fixture corpus 1개 + lexical-friendly query + score_gate 우회 (권장)

Test 가 빈 KB 가정을 버리고, **probe 통과 → decompose → LLM unreachable → error.v1** 의 정상 multi-hop path 를 회복한다.

변경 (single test 파일 only):

```rust
fn minimal_config(data_dir: &Path, workspace_root: &Path) -> Config {
    // ... 기존과 동일 ...
    cfg.models.llm.endpoint = "http://127.0.0.1:1".to_string();
    cfg.models.llm.request_timeout_secs = 5;
    // probe 의 두 번째 gate (top_score < score_gate) 우회 — fixture 의 lexical
    // FTS5 score 가 0.0 이상이면 통과. refuse_score_gate path 는 본 test 의
    // dispatch divergence assertion 과 무관 (probe 가 통과해야 decompose 가
    // LLM 호출 시도 → error.v1 surface). production default 0.30 은 production
    // binary 그대로 유지 (test config isolation).
    cfg.rag.score_gate = 0.0;
    cfg
}

#[tokio::test]
async fn ask_tool_routes_multi_hop_true_to_decompose_first() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let workspace_root = dir.path().join("notes");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(&workspace_root).unwrap();

    // probe 가 non-empty 를 반환하도록 lexical-friendly minimal corpus 1개 ingest.
    // (v1 stale 주석 삭제 — round-2 critic-plan N1 closure. v2 정정 reasoning 은 아래 4 줄.)
    let fixture = workspace_root.join("note.md");
    // round-1 critic-plan CRITICAL C1: build_match_string 이
    // `text : (("compound about X and Y") OR ("compound" "about" "and"))`
    // 으로 query 를 변환 — FTS5 implicit-AND 라 token_and branch 가 fixture
    // 의 `"compound", "about", "and"` 셋 다 매칭 필요. fixture body 에 `"about"`
    // 포함 — empirical SQLite REPL (V007 trigram DDL) 로 1 hit 확정.
    std::fs::write(&fixture, "# Compound topic\n\nThis note is about a compound containing X and Y in detail.\n").unwrap();

    let cfg = minimal_config(&data_dir, &workspace_root);
    let scope = SourceScope { root: workspace_root.clone(), include: vec![], exclude: vec![] };
    let _ = kebab_app::ingest_with_config(cfg.clone(), scope, false).unwrap();

    // ... 이후 multi-hop / single-pass 분기 동일 ...
}
```

추가로 단일-pass 분기의 query 도 fixture 와 lexical-match 안 되어야 (`anything`) `chunks_returned=0` 의 단일-pass NoChunks refusal 를 유지. 현재 query 그대로 두면 됨 ("anything" 은 fixture 토큰과 무관).

대안 — fixture 만들지 않고 `cfg.rag.score_gate = -1.0` + empty `Vec` 의 corner 만 우회하는 방법은 *probe_hits.is_empty()* 의 첫 gate 에 막힘. fixture 1개 ingest 가 가장 짧은 fix path.

**모듈 doc 갱신** (line 1–18) 도 같은 PR 에서. 신규 wording draft (round-1 critic NIT 격상 — completed draft):

```rust
//! Pin the MCP `ask` tool's `multi_hop` argument dispatch contract.
//!
//! v0.18 dogfood fix (PR-7) introduced a pre-decompose score-gate probe
//! in `RagPipeline::ask_multi_hop`: empty KB / sub-gate probe -> the
//! single-pass NoChunks refusal envelope (`answer.v1`), not `error.v1`.
//! The two surfaces' divergence is therefore observed *only when the probe
//! passes* — at that point, single-pass returns retrieval + LLM call, and
//! multi-hop calls decompose first (LLM unreachable -> `error.v1`).
//!
//! These two tests pin:
//! 1. `ask_tool_routes_multi_hop_true_to_decompose_first` — with a probe-
//!    passing fixture, multi_hop=true dispatches to decompose (LLM error),
//!    single_pass dispatches to retrieval-first (NoChunks refusal). Wire
//!    shapes diverge: `error.v1` vs `answer.v1`.
//! 2. `ask_tool_multi_hop_short_circuits_when_probe_empty` — with an empty
//!    KB, multi_hop=true takes the probe-empty short-circuit, producing
//!    a NoChunks refusal envelope byte-identical to single-pass. PR-7's
//!    intended behavior is wire-pinned at the MCP layer.
```

### Option B — Test 가 single-pass / multi-hop 의 wire stamp 차이만 검증

빈 KB 그대로 두고, **probe-refuse Answer 의 prompt_template_version** 으로 dispatch 를 pin. 단, §1 마지막 단락에서 본 바 `refuse_no_chunks` 가 multi-hop 에서도 single-pass version 을 stamp 함 → 이 option 은 *production code 수정* 필요 (`refuse_no_chunks` 가 multi-hop 호출자 일 때 `PROMPT_TEMPLATE_VERSION_MULTI_HOP` 을 stamp). **이 task 의 범위 (test-only fix) 를 벗어남.** §6 비범위로 분류.

### Option C — Test 를 `#[ignore]` + follow-up TODO

가장 minimal 이지만 dispatch contract 의 wire pinning 을 영구히 잃음. PR-5 의 의도 (MCP host 가 multi-hop arg 를 실제로 routing 하는지 확인) 가 사라진다. 비추.

### 권장: Option A

- 변경 scope: 단일 test 파일 + 1줄 config 노브 (`score_gate = 0.0`).
- production code touch 0.
- test 의 원래 의도 (dispatch divergence pin) 보존.
- effort: ~30분 (write + verify).

---

## §4. Wire / behavior 영향

| 영역 | 영향 |
|---|---|
| Wire schema (`answer.v1`, `error.v1`, …) | **None.** 변경 없음. |
| Behavior (`kebab ask --multi-hop` 실사용) | **None.** PR-7 의 probe-first 가 그대로 유지. |
| 사용자 visible surface (CLI, TUI, MCP, README) | **None.** test 만 갱신. |
| Cargo workspace `version` | **No bump.** test-only fix 는 version cascade trigger 아님 — wire / behavior / 사용자 binary 변경 0. |
| Release | **No new release.** 다음 v0.18.1 / v0.19.0 release 때 piggyback. |
| HOTFIXES.md | **신규 dated entry 추가** (round-2 critic-plan MAJOR M1/A2 closure). line 17 의 `## 2026-05-25 — fb-41 pre-v0.18 dogfood ...` 직전 (최신 date 가 top — `## YYYY-MM-DD` convention 정합) 에 `## 2026-05-26 — HOTFIX #15 — MCP ask multi_hop dispatch-divergence assertion stale (fixture 보강)` level-1 dated entry 추가. Symptom / Root cause / Action / Amends 4-block. fix PR 가 같은 PR 에서 갱신. |

---

## §5. 검증 plan

### 5.1 단위 / 통합 (fix PR 에서)

```bash
# 1. 단일 test 실행 — fix 후 GREEN.
CARGO_TARGET_DIR=/build/out/cargo-target/target \
  cargo test -p kebab-mcp --test tools_call_ask_multi_hop -j 1 -- --nocapture

#    기대: 2 tests, 2 passed. ask_tool_routes_multi_hop_true_to_decompose_first
#    의 multi-hop 분기가 error.v1 + isError=true, single-pass 분기가
#    answer.v1 + grounded=false 로 분기.

# 2. 같은 crate 전체 — kebab-mcp 의 다른 13 test binary 가 fixture
#    변경의 영향 없음을 확인.
CARGO_TARGET_DIR=/build/out/cargo-target/target \
  cargo test -p kebab-mcp -j 1
#    기대: 모든 test PASS (이전 baseline 유지).

# 3. Workspace-wide — 회귀 0 확인.
cargo test --workspace --no-fail-fast -j 1
#    기대: 워크스페이스 전체 PASS + 1 known flaky → 0 flaky. 정확한 PASS
#    카운트는 PR 시점 실측 후 재산정 (cleanup PR / cut PR 머지 후 baseline
#    변동 가능 — 예: 1304 + post-PR9 9 신규 + post-cleanup 의 일부 ignored).
#    핵심 acceptance = HOTFIX #15 의 deterministic fail 0건 + 신규 회귀 0건.

# 4. clippy — workspace gate.
cargo clippy --workspace --all-targets -j 1 -- -D warnings
#    기대: clean (test fixture 1개 file 작성은 clippy hit 없음).
```

### 5.2 Dogfood (별도 PR 무관, 이미 v0.18.0 cut 후 OK)

- v0.18.0 binary 의 `kebab ask --multi-hop` 실사용 동작 검증은 이미 v0.18.0 cut 시 (`docs/dogfood/v0.18.0/SUMMARY.md`) 완료. test fixture 만 갱신하므로 추가 dogfood 불필요.

### 5.3 회귀 안전망 추가 (포함 — round-1 critic HIGH + verifier note 격상)

Fix PR 에서 *추가* test 1개로 *probe-fail 경로* 도 명시적으로 pin (현재는 implicit). kebab-rag/tests/multi_hop.rs::`multi_hop_empty_probe_pool_refuses_before_any_llm_call` 가 *RAG-layer* 만 pin — *MCP-layer wire shape* 는 본 test 만이 안전망:

```rust
#[tokio::test]
async fn ask_tool_multi_hop_short_circuits_when_probe_empty() {
    // 빈 KB + multi_hop=true → probe-first 의 NoChunks short-circuit 으로
    // single-pass 와 byte-identical refuse Answer. PR-7 의 dogfood fix
    // 가 의도한 동작이 regression 되지 않도록 wire 로 pin.
    // 기대: schema_version=answer.v1, refusal_reason=no_chunks, isError=false.
}
```

이는 §1 의 새 contract 를 명시적으로 문서화하는 효과. **포함 (round-1 critic HIGH + verifier note 격상)**. scope 확장 아님 (같은 test 파일에 1 #[tokio::test] 추가). 같은 사항이 §7 exit 조건에 명시 추가.

---

## §6. 비범위

이 hotfix 는 **test 갱신만** 다룬다. 아래는 같은 PR 에서 *건드리지 않으며*, 필요 시 별 task 후보:

| 비범위 항목 | 이유 | 후보 task |
|---|---|---|
| `refuse_no_chunks` 가 multi-hop caller 일 때 `PROMPT_TEMPLATE_VERSION_MULTI_HOP` 을 stamp 하도록 변경 | wire major implication (실사용 surface 의 prompt_template_version 가 multi-hop probe-refuse 시 바뀜). 별 brainstorm 필요 — Option B 참고. | "multi-hop refuse stamp 일관성" hotfix 또는 v0.19.0 feat. |
| `ask_multi_hop` probe 의 cost / latency 최적화 (probe_hits 를 첫 sub-query pool 의 seed 로 재사용) | line 702–712 에 의도된 invariant (HopRecord.context_chunks_added 의 의미 보존) 가 있음. 후속 분석 필요. | v0.19.0 multi-hop perf. |
| Test 이름 변경 (`_routes_multi_hop_true_to_decompose_first` → `_routes_multi_hop_true_through_probe_to_decompose`) | rename 은 cross-cutting (테스트 명을 reference 하는 PR 메시지 / HOTFIXES / commit history). 기능 무관 cosmetic; defer. | "test name precision" cleanup. |
| `ask_multi_hop` 의 probe step 을 별 함수로 추출 (`fn pre_decompose_probe(...) -> ProbeOutcome`) | refactor scope. v0.18.1 hotfix 의 minimum-diff 원칙 위배. | v0.19.0 refactor. |
| Score-gate probe 의 fixture 가 hybrid / vector mode 에서도 통과하도록 보강 | 현재 test 는 lexical mode 만. embedder="none" 이라 hybrid 불가. | 추가 multi-hop mode coverage task. |

---

## §7. 합의된 출구 조건

이 spec 이 closed 되려면:

1. Fix PR 머지 — `crates/kebab-mcp/tests/tools_call_ask_multi_hop.rs` 갱신 + (round-1 critic HIGH + verifier 격상) **새 `_multi_hop_short_circuits_when_probe_empty` test 1개 포함 (MCP-layer wire pin 안전망)**.
2. `cargo test --workspace --no-fail-fast -j 1` GREEN.
3. `cargo clippy --workspace --all-targets -j 1 -- -D warnings` clean.
4. `tasks/HOTFIXES.md` 에 신규 `## 2026-05-26 — HOTFIX #15 ...` dated entry 추가 (line 17 의 `## 2026-05-25` 직전 — 최신 date 가 top convention. round-2 critic-plan MAJOR M1/A2 closure).
5. `tasks/INDEX.md` phase status 변경 없음 (HOTFIX 만, phase epic 영향 없음).

---

## §8. Risk

- **Risk level: 매우 낮음.** test-only fix. production code 0 touch. wire 0 변경. behavior 0 변경.
- 유일한 작은 위험: fixture file 의 token 이 `query: "compound about X and Y"` 와 lexical match 하지 않으면 다시 probe-empty short-circuit.

  round-1 critic-plan **CRITICAL C1 closure**: empirical SQLite REPL (V007 trigram DDL + `crates/kebab-search/src/lexical.rs::build_match_string` 의 query 변환) 확인 — query `"compound about X and Y"` 는 `text : (("compound about X and Y") OR ("compound" "about" "and"))` 로 변환됨. FTS5 의 token_and branch (`"compound" "about" "and"`) 는 *implicit-AND* — fixture 가 세 token 모두 포함해야 매칭. 본 spec 의 신규 fixture body **`"This note is about a compound containing X and Y in detail."`** 는 세 token (`compound`, `about`, `and`) 모두 포함하여 token_and branch 가 1 hit. (이전 draft `"This note discusses compound X and Y in detail."` 는 `"about"` 미포함 → 0 hits 의 self-reproducing 결함이 round-1 critic-plan 에 발견됨.)
- `cfg.rag.score_gate = 0.0` 의 명시화: 다른 multi-hop refuse path (`refuse_score_gate`, line 691) 가 우회되었음을 test 의 doc comment 에서 명시.
