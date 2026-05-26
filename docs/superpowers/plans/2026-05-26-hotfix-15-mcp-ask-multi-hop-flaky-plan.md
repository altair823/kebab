---
title: "HOTFIX #15 implementation plan v1 — MCP ask multi-hop test fixture 보강"
date: 2026-05-26
task_id: hotfix-15
status: open
target_version: 0.18.1
design: ../specs/2026-05-26-hotfix-15-mcp-ask-multi-hop-flaky-spec.md
---

# HOTFIX #15 implementation plan v1 — MCP ask multi-hop test fixture 보강

## §0. 개요

v0.18.0 cut 직후 HOTFIX. PR-7 (v0.18 pre-cut dogfood fix) 가 `RagPipeline::ask_multi_hop` 에 *pre-decompose score-gate probe* 를 도입한 뒤, PR-5 시점의 MCP test `ask_tool_routes_multi_hop_true_to_decompose_first` 가 stale dispatch contract (empty KB → multi-hop → LLM → `error.v1`) 위에 assertion 을 박아두어 deterministic fail. **Test-only fix (Option A)** — fixture corpus 1개를 추가해 probe 통과 → decompose → LLM unreachable → `error.v1` 의 정상 multi-hop path 회복. Production code touch 0, wire 변경 0, behavior 변경 0. 단일 PR, target version 변경 없음 (v0.18.1 / v0.19.0 piggyback). 본 plan 은 spec (`../specs/2026-05-26-hotfix-15-mcp-ask-multi-hop-flaky-spec.md`) 의 §3 / §5 / §7 / §8 을 *step-by-step* 으로 풀어낸다 — spec 재정의 아님.

## §1. 단일 PR 내용

변경 파일 (2개):

| 파일 | 변경 요약 |
|---|---|
| `crates/kebab-mcp/tests/tools_call_ask_multi_hop.rs` | `minimal_config` 에 `score_gate = 0.0` 노브 + inline doc 추가, 기존 test 에 fixture `note.md` ingest 추가 (workspace_root 에 corpus 1개), 모듈 doc rewrite (probe-first contract 명시), 신규 `_multi_hop_short_circuits_when_probe_empty` test 1개 추가 (REQUIRED — spec §5.3). |
| `tasks/HOTFIXES.md` | 신규 dated entry **`## 2026-05-26 — HOTFIX #15 — MCP ask multi_hop dispatch-divergence assertion stale (fixture 보강)`** 을 line 17 의 `## 2026-05-25 — fb-41 pre-v0.18 dogfood ...` 직전 (최신 date 가 top — round-1 critic-plan MAJOR M1 closure: 기존 `## YYYY-MM-DD` convention 정합, 2026-05-25 우산 안에 2026-05-26 subsection 삽입 회피). Symptom / Root cause / Action / Amends 4-block. |

Production code (`crates/kebab-rag/**`, `crates/kebab-mcp/src/**`, `crates/kebab-app/**`) 는 **0 touch**. wire schema / CLI flag / config default 변경 **0**. README / HANDOFF / docs/ARCHITECTURE 갱신 **불필요** (사용자 visible surface 변경 0).

## §2. 구현 step list

Subagent 가 다음 순서대로 작업한다:

1. **`minimal_config` 갱신** — `crates/kebab-mcp/tests/tools_call_ask_multi_hop.rs::minimal_config` (line 25–43) 의 끝부분에 `cfg.rag.score_gate = 0.0;` 1줄 추가. 위에 inline doc comment (spec §3 의 권장 wording) 로 *왜 0.0 인지* 명시 — probe 의 두 번째 gate (`top_score < score_gate`) 우회, fixture lexical FTS5 score 가 default 0.30 미만일 가능성을 차단, test config isolation (production default 0.30 유지).

2. **Fixture corpus 1 file 추가** — `ask_tool_routes_multi_hop_true_to_decompose_first` 의 setup 구간 (`std::fs::create_dir_all(&workspace_root).unwrap();` 직후) 에 다음을 삽입:
   ```rust
   let fixture = workspace_root.join("note.md");
   std::fs::write(&fixture, "# Compound topic\n\nThis note is about a compound containing X and Y in detail.\n").unwrap();
   ```
   spec §8 의 token 매칭 근거 (round-1 critic-plan CRITICAL C1 empirical 확인): `build_match_string` 이 query `"compound about X and Y"` 를 `text : (("compound about X and Y") OR ("compound" "about" "and"))` 으로 변환. token_and branch (`"compound" "about" "and"`) 는 FTS5 *implicit-AND* — fixture 가 세 token 모두 포함 필요. 신규 fixture body `"This note is about a compound containing X and Y in detail."` 는 `compound` + `about` + `and` 모두 포함 → empirical SQLite REPL 1 hit 확정. (이전 draft "discusses compound X and Y" 는 `"about"` 미포함 → 0 hits self-reproducing 결함 — round-1 critic-plan 발견.)

3. **workspace_root + data_dir setup 갱신** — 기존 `ingest_with_config(cfg.clone(), scope, false)` 호출은 그대로 유지 (line 65). `SourceScope` 구조 (line 60–64) 도 그대로 — `include: vec![]` / `exclude: vec![]` 가 workspace_root 전체를 ingest 한다. step 2 의 fixture 가 자동 포함.

4. **기존 assertion 보존** — line 86–89 의 `assert!(mh.is_error.unwrap_or(false), …)`, line 119–129 의 single-pass branch (query=`"anything"` 으로 fixture token 과 lexical-match 안 됨 → NoChunks refusal 유지) **모두 그대로**. fixture 추가 후의 변화는 *multi-hop probe 통과* 만, single-pass branch 는 query 가 fixture 토큰과 매칭 안 되어 retrieval empty → `grounded=false` + `is_error=false` 의 기존 assertion 유효. **단 line 94–101 의 inline 주석 (예: "The dispatch contract is 'multi-hop reached the LLM' — i.e. `is_error` fires because decompose tried to talk to the LLM and failed.")** 는 새 contract (probe-first → probe 통과 시 decompose → LLM) 와 partial 정합 — step 6 의 module doc rewrite scope 에 inline 주석도 함께 갱신하여 *"probe 가 통과한 후* decompose 가 LLM 시도"* 로 sharpen (round-1 critic-plan Med2).

5. **신규 `_multi_hop_short_circuits_when_probe_empty` test 추가 (REQUIRED — spec §5.3 + §7 조건 1)** — 같은 파일 끝에 `#[tokio::test]` 1개 추가. **별 `tempfile::tempdir().unwrap()` 호출로 fresh tempdir** (round-1 critic-plan Med1 — `#[tokio::test]` 병렬 실행 시 race 회피, 기존 test 의 dir 와 분리). workspace_root 를 빈 디렉토리로 두고 (no fixture ingest), `multi_hop=true` 로 dispatch. 기대값: `schema_version="answer.v1"`, `refusal_reason="no_chunks"`, `is_error=Some(false)`. PR-7 의 *probe-empty short-circuit* 이 MCP-layer 의 wire shape 로 pin 됨. 같은 `minimal_config` 재사용 (score_gate=0.0 은 첫 gate `probe_hits.is_empty()` 우회와 무관).

6. **Module doc 갱신** — `crates/kebab-mcp/tests/tools_call_ask_multi_hop.rs` line 1–18 의 module-level doc comment 를 spec §3 의 신규 draft (round-1 critic NIT 격상 completed) 로 교체. 두 test 가 각각 pin 하는 contract 를 enumerate (1. probe-passing fixture → divergence, 2. probe-empty → byte-identical refuse).

7. **`tasks/HOTFIXES.md` 신규 dated entry (round-1 critic-plan MAJOR M1 closure)** — line 17 의 `## 2026-05-25 — fb-41 pre-v0.18 dogfood ...` 직전 (가장 최신 date 가 top — HOTFIXES.md 의 `## YYYY-MM-DD` convention) 에 다음 형식으로 신규 dated entry 1개 추가:
   - 제목: `## 2026-05-26 — HOTFIX #15 — MCP ask multi_hop dispatch-divergence assertion stale (fixture 보강)`.
   - **Symptom**: PR-7 (multi-hop probe-first dogfood fix) 머지 후 `kebab-mcp::tools_call_ask_multi_hop::ask_tool_routes_multi_hop_true_to_decompose_first` 가 모든 workspace test 에서 deterministic fail (no_chunks short-circuit 으로 인한 `is_error=Some(false)`).
   - **Root cause**: PR-5 의 test 가 *empty KB → multi-hop 은 decompose first → LLM 도달* 의 stale contract 에 assert. PR-7 의 pre-decompose probe 가 빈 KB → refuse_no_chunks short-circuit. spec §1.
   - **Action**: test fixture 보강 — `minimal_config.score_gate = 0.0` + workspace_root 에 `note.md` (`"This note is about a compound containing X and Y in detail."`) ingest → probe 통과 → decompose → unreachable LLM → `error.v1` 의 원래 dispatch divergence 회복. + 신규 `_multi_hop_short_circuits_when_probe_empty` test 1개 (probe-empty short-circuit 의 MCP-layer wire pin 안전망). + module doc rewrite (probe-first contract 명시).
   - **Amends**: spec `docs/superpowers/specs/2026-05-26-hotfix-15-mcp-ask-multi-hop-flaky-spec.md` cross-link. production code 0 touch (PR-7 의 probe-first 는 의도된 동작 유지).

## §3. 검증

Spec §5 의 cargo command verbatim:

```bash
# 1. 단일 test binary — fix 후 GREEN.
CARGO_TARGET_DIR=/build/out/cargo-target/target \
  cargo test -p kebab-mcp --test tools_call_ask_multi_hop -j 1 -- --nocapture
# 기대: 3 tests, 3 passed (기존 2 + 신규 _multi_hop_short_circuits_when_probe_empty).

# 2. 같은 crate 전체 — 다른 kebab-mcp test binary 가 fixture 영향 받지 않는지 확인.
CARGO_TARGET_DIR=/build/out/cargo-target/target \
  cargo test -p kebab-mcp -j 1
# 기대: 이전 baseline PASS 유지.

# 3. Workspace-wide — 회귀 0 + known flaky 1 → 0.
cargo test --workspace --no-fail-fast -j 1
# 기대: HOTFIX #15 의 deterministic fail 0건 + 신규 회귀 0건.
# PASS count 는 PR 시점 실측 후 baseline 재산정 (post-cleanup 의 ignored 변동 가능).

# 4. clippy — workspace gate.
cargo clippy --workspace --all-targets -j 1 -- -D warnings
# 기대: clean.
```

머신 RAM 16 GiB 제약 (`CLAUDE.md` workspace) — `-j 1` 필수. test binary 들이 lance/datafusion link 단계에서 OOM 가능. 직렬 실행.

Dogfood 추가 불필요 — v0.18.0 binary 의 multi-hop 동작은 이미 v0.18.0 cut 시 검증 완료 (spec §5.2).

## §4. 시간 추정

| 단계 | wall time |
|---|---|
| 구현 (step 1–7 작성) | 30–45 min |
| `cargo test` × 4 verification + clippy | 20–30 min (`-j 1` 직렬, full workspace 1회 포함) |
| Review iteration (round-1 critic / verifier 피드백 반영 여지) | 30–60 min |
| **합계** | **wall time 1–2h** |

Scope 가 작아 single subagent dispatch 로 충분 (parallel split 불필요).

## §5. 위험 / 회피

Spec §8 cross-ref. 핵심 위험과 회피:

- **Risk level**: 매우 낮음. test-only fix, production code 0 touch, wire 0 변경.
- **유일한 실패 모드**: fixture token 이 query 와 lexical-match 안 되어 probe 0 hits → refuse_no_chunks short-circuit. → 검증 (round-2 critic-plan A1 closure — round-1 의 단일-token reasoning debunked): `build_match_string` 이 query `"compound about X and Y"` 를 `text : (("compound about X and Y") OR ("compound" "about" "and"))` 으로 변환. token_and branch 는 FTS5 *implicit-AND* — fixture 가 `compound`, `about`, `and` **셋 다 포함 필요**. v2 fixture `"This note is about a compound containing X and Y in detail."` 가 세 token 모두 포함 → empirical SQLite REPL (V007 trigram DDL) 로 1 hit 확정. retry path 불필요 — 다음 fixture 변경 시 *세 token 모두 보존* 필수 (단일-token 으로 축소 금지).
- **`score_gate = 0.0`** 명시화: `refuse_score_gate` (line 691) 우회 의도 — 모듈 doc 의 inline comment 로 명시. production default 0.30 은 production binary 그대로 유지 (test config isolation).
- **신규 test (`_multi_hop_short_circuits_when_probe_empty`)** 가 같은 `minimal_config` 사용 — `score_gate = 0.0` 이 첫 gate (`probe_hits.is_empty()`) 우회와 무관해 영향 없음. 빈 KB → 첫 gate 에 막히고 refuse_no_chunks short-circuit.
- **Sibling test (`ask_input_schema_advertises_multi_hop_field`)**: JsonSchema-only, retrieval 호출 없음 → fixture 변경 영향 없음 (spec §2 cross-ref).

## §6. 자기-review

Spec items → plan step 매핑 확인 (round-1 critic / verifier 피드백 빠짐 없음):

| Spec 항목 | Plan step |
|---|---|
| §3 Option A — `score_gate = 0.0` 노브 | §2 step 1 |
| §3 Option A — fixture corpus 1개 + lexical-friendly query | §2 step 2 |
| §3 Option A — workspace_root + ingest_with_config 유지 | §2 step 3 |
| §3 Option A — 기존 assertion 보존 | §2 step 4 |
| §3 모듈 doc 신규 draft (critic NIT 격상) | §2 step 6 |
| §5.3 신규 `_multi_hop_short_circuits_when_probe_empty` test (REQUIRED — critic HIGH + verifier note 격상) | §2 step 5 |
| §7 조건 1 — fix PR 머지 + 신규 test 포함 | §2 step 5 + step 7 |
| §7 조건 2 — `cargo test --workspace` GREEN | §3 명령 #3 |
| §7 조건 3 — `cargo clippy` clean | §3 명령 #4 |
| §7 조건 4 — HOTFIXES.md 신규 `## 2026-05-26` dated entry (round-1 MAJOR M1 / round-2 A2 closure) | §2 step 7 |
| §7 조건 5 — tasks/INDEX.md 변경 없음 | (변경 없음 — plan 의 file list 에 INDEX.md 없음) |
| §8 fixture token 매칭 위험 + V007 trigram 근거 | §5 |
| §4 wire / behavior / version cascade 영향 0 | §0 + §1 |
| §6 비범위 (refuse_no_chunks multi-hop stamp / probe 함수 추출 / test name rename / hybrid mode coverage) | 본 plan 의 file list 에 production crate 없음 → 비범위 자동 준수 |

누락 항목 0건.

## §7. Subagent dispatch task

`/oh-my-claudecode:subagent-driven-development` (or executor agent) 의 single task description:

**Task name**: `HOTFIX #15 — MCP ask multi-hop test fixture 보강`

**Description**:

> Spec `docs/superpowers/specs/2026-05-26-hotfix-15-mcp-ask-multi-hop-flaky-spec.md` §3 Option A 적용 — `crates/kebab-mcp/tests/tools_call_ask_multi_hop.rs` 갱신:
>
> 1. `minimal_config` 에 `cfg.rag.score_gate = 0.0;` 추가 (probe 의 두 번째 gate 우회, inline doc).
> 2. `ask_tool_routes_multi_hop_true_to_decompose_first` setup 에 `workspace_root/note.md` (content: `# Compound topic\n\nThis note is about a compound containing X and Y in detail.\n`) ingest.
> 3. 모듈 doc (line 1–18) 을 spec §3 의 신규 draft 로 교체 — 두 test 가 각각 pin 하는 contract enumerate.
> 4. 신규 `#[tokio::test] async fn ask_tool_multi_hop_short_circuits_when_probe_empty()` 추가 (spec §5.3 REQUIRED). 빈 KB + `multi_hop=true` → `schema_version=answer.v1`, `refusal_reason=no_chunks`, `is_error=false` pin.
> 5. `tasks/HOTFIXES.md` 의 line 17 (`## 2026-05-25 — fb-41 pre-v0.18 dogfood ...`) **직전** 에 신규 dated entry `## 2026-05-26 — HOTFIX #15 — MCP ask multi_hop dispatch-divergence assertion stale (fixture 보강)` 추가 (round-1 critic-plan MAJOR M1 — date convention 정합, 가장 최신 date 가 top). Symptom / Root cause / Action / Amends 4-block (plan §2 step 7 verbatim).
>
> 신규 test skeleton (round-1 critic-plan MAJOR M2 — self-contained 화):
> ```rust
> /// PR-7 의 probe-empty short-circuit 이 MCP-layer 의 wire shape 로 pin.
> /// 빈 KB + multi_hop=true → refuse_no_chunks → answer.v1 envelope.
> #[tokio::test]
> async fn ask_tool_multi_hop_short_circuits_when_probe_empty() {
>     let dir = tempfile::tempdir().unwrap();                    // fresh tempdir (Med1)
>     let data_dir = dir.path().join("data");
>     let workspace_root = dir.path().join("notes");
>     std::fs::create_dir_all(&data_dir).unwrap();
>     std::fs::create_dir_all(&workspace_root).unwrap();         // 빈 디렉토리 — fixture 없음
>
>     let cfg = minimal_config(&data_dir, &workspace_root);
>     let scope = SourceScope { root: workspace_root.clone(), include: vec![], exclude: vec![] };
>     let _ = kebab_app::ingest_with_config(cfg.clone(), scope, false).unwrap();
>
>     // 기존 test (line 67-89, 103-130) 의 actual dispatch pattern 일관 사용
>     // (round-2 critic-plan N5 + round-3 N5b closure — fictional helpers 제거.
>     // 실제 type: `KebabAppState::new` + `KebabHandler::new` 모두 sync).
>     let state = kebab_mcp::KebabAppState::new(cfg.clone(), None);
>     let handler = kebab_mcp::KebabHandler::new(state);
>     let state_mh = handler.state().clone();
>     let mh = tokio::task::spawn_blocking(move || {
>         kebab_mcp::tools::ask::handle(
>             &state_mh,
>             kebab_mcp::tools::ask::AskInput {
>                 query: "compound about X and Y".to_string(),
>                 session_id: None,
>                 mode: Some("lexical".to_string()),
>                 multi_hop: Some(true),
>             },
>         )
>     }).await.unwrap();
>
>     // probe-empty short-circuit → refuse_no_chunks → answer.v1 envelope
>     assert_eq!(mh.is_error, Some(false), "probe-empty short-circuit must yield refusal envelope, not error.v1");
>     let mh_text = match &mh.content.first().unwrap().raw {
>         rmcp::model::RawContent::Text(t) => t.text.clone(),
>         other => panic!("expected text content, got {other:?}"),
>     };
>     let body: serde_json::Value = serde_json::from_str(&mh_text).unwrap();
>     assert_eq!(body["schema_version"], "answer.v1");
>     assert_eq!(body["refusal_reason"], "no_chunks");
> }
> ```
>
> 모듈 doc rewrite (spec §3 line 138-159 verbatim 형식 — round-1 critic-plan MAJOR M2):
> ```rust
> //! Pin the MCP `ask` tool's `multi_hop` argument dispatch contract.
> //!
> //! v0.18 dogfood fix (PR-7) introduced a pre-decompose score-gate probe
> //! in `RagPipeline::ask_multi_hop`: empty KB / sub-gate probe -> the
> //! single-pass NoChunks refusal envelope (`answer.v1`), not `error.v1`.
> //! The two surfaces' divergence is therefore observed *only when the probe
> //! passes* — at that point, single-pass returns retrieval + LLM call, and
> //! multi-hop calls decompose first (LLM unreachable -> `error.v1`).
> //!
> //! These two tests pin:
> //! 1. `ask_tool_routes_multi_hop_true_to_decompose_first` — probe-passing
> //!    fixture, multi_hop=true → decompose (LLM error), single_pass → retrieval
> //!    NoChunks. Wire shapes diverge: `error.v1` vs `answer.v1`.
> //! 2. `ask_tool_multi_hop_short_circuits_when_probe_empty` — empty KB,
> //!    multi_hop=true → probe-empty short-circuit, NoChunks refusal byte-
> //!    identical to single-pass. PR-7 의 intent 가 MCP layer 에 pin.
> ```
>
> HOTFIXES.md 신규 entry 형식 (round-1 critic-plan MAJOR M2):
> ```markdown
> ## 2026-05-26 — HOTFIX #15 — MCP ask multi_hop dispatch-divergence assertion stale (fixture 보강)
>
> **Symptom**: PR-7 (multi-hop probe-first dogfood fix) 머지 후 `kebab-mcp::tools_call_ask_multi_hop::ask_tool_routes_multi_hop_true_to_decompose_first` 가 모든 workspace test 에서 deterministic fail (no_chunks short-circuit 으로 `is_error=Some(false)`).
>
> **Root cause**: PR-5 의 test 가 *empty KB → multi-hop 은 decompose first → LLM 도달* 의 stale contract 에 assert. PR-7 의 pre-decompose probe 가 빈 KB → refuse_no_chunks short-circuit. spec §1.
>
> **Action**: test fixture 보강 — `minimal_config.score_gate = 0.0` + workspace_root 에 `note.md` ("This note is about a compound containing X and Y in detail.") ingest → probe 통과 → decompose → unreachable LLM → `error.v1` 의 원래 dispatch divergence 회복. + 신규 `_multi_hop_short_circuits_when_probe_empty` test (probe-empty short-circuit 의 MCP-layer wire pin 안전망). + module doc rewrite.
>
> **Amends**: spec `docs/superpowers/specs/2026-05-26-hotfix-15-mcp-ask-multi-hop-flaky-spec.md` cross-link. production code 0 touch (PR-7 의 probe-first 는 의도된 동작 유지).
> ```

**Production code touch 0** (`crates/kebab-rag/**`, `crates/kebab-mcp/src/**`, `crates/kebab-app/**` 수정 금지). README / HANDOFF / docs/ARCHITECTURE 갱신 불필요.

**검증 cargo commands** (모두 GREEN 필요):

```bash
CARGO_TARGET_DIR=/build/out/cargo-target/target cargo test -p kebab-mcp --test tools_call_ask_multi_hop -j 1 -- --nocapture
CARGO_TARGET_DIR=/build/out/cargo-target/target cargo test -p kebab-mcp -j 1
cargo test --workspace --no-fail-fast -j 1
cargo clippy --workspace --all-targets -j 1 -- -D warnings
```

**단일 commit + commit msg 예시**:

```
fix(mcp,tests): HOTFIX #15 — pin multi-hop probe-first contract via fixture + new short-circuit test

PR-7 (v0.18 pre-cut dogfood) introduced a pre-decompose score-gate
probe in RagPipeline::ask_multi_hop. The PR-5 test
`ask_tool_routes_multi_hop_true_to_decompose_first` assumed empty
KB → multi-hop → LLM error, but the new probe short-circuits to
NoChunks refusal before reaching decompose. Test-only fix:

- minimal_config: score_gate = 0.0 (bypass second probe gate).
- ingest a lexical-friendly fixture (note.md with "compound" token)
  so probe_hits.is_empty() == false → decompose runs → LLM
  unreachable → error.v1 (original dispatch divergence restored).
- new test `ask_tool_multi_hop_short_circuits_when_probe_empty`
  pins the PR-7 probe-empty short-circuit at the MCP wire layer.
- module doc rewritten to describe the two pinned contracts.
- HOTFIXES.md: new `## 2026-05-26 — HOTFIX #15 ...` dated entry above the 2026-05-25 umbrella (round-2 MAJOR M1/A2 closure).

Production code 0 touch. Wire / behavior / version cascade 0.

Refs: docs/superpowers/specs/2026-05-26-hotfix-15-mcp-ask-multi-hop-flaky-spec.md
```

**PR title + body skeleton**:

- Title: `fix(mcp,tests): HOTFIX #15 — pin multi-hop probe-first contract via fixture + new short-circuit test`
- Body:
  ```
  ## Summary
  - PR-7 (v0.18 dogfood) introduced probe-first short-circuit; PR-5 test had stale empty-KB contract → deterministic fail. Test-only fix.
  - Adds fixture corpus + score_gate=0.0 to restore probe-pass path → original dispatch divergence assertion holds.
  - New `_multi_hop_short_circuits_when_probe_empty` pins the probe-empty refuse path at the MCP wire layer (round-1 critic HIGH + verifier note).
  - Module doc rewritten; HOTFIXES.md new `## 2026-05-26` dated entry added (date-top convention).

  ## Test plan
  - [ ] `cargo test -p kebab-mcp --test tools_call_ask_multi_hop -j 1 -- --nocapture` (3 passed).
  - [ ] `cargo test -p kebab-mcp -j 1` (baseline).
  - [ ] `cargo test --workspace --no-fail-fast -j 1` (GREEN, known flaky 1 → 0).
  - [ ] `cargo clippy --workspace --all-targets -j 1 -- -D warnings` clean.

  Refs: `docs/superpowers/specs/2026-05-26-hotfix-15-mcp-ask-multi-hop-flaky-spec.md`
  ```

Plan v1. Round-1 review 후 amend 가능.
