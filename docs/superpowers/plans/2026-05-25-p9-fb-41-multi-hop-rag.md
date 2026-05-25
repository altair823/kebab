---
title: "p9-fb-41 multi-hop RAG implementation plan"
date: 2026-05-25
task_id: p9-fb-41
phase: P9
status: open
target_version: 0.18.0
design: ../specs/2026-05-25-p9-fb-41-multi-hop-rag-design.md
---

# p9-fb-41 implementation plan

Design: `docs/superpowers/specs/2026-05-25-p9-fb-41-multi-hop-rag-design.md`.

XL 작업 — 6 PR 분할 (각 머지 후 누적, 마지막 PR 후 v0.18.0 cut).

## PR-1: Multi-hop eval golden set + baseline

**Goal**: 구현 전 baseline 측정 anchor 확보. RAG pipeline 미변경 — metric 인프라 + fixture 만.

**Files**:
- `tasks/eval/multi-hop-golden.toml` 신규 — 15 question (5 cross-doc + 5 intra-doc + 5 single-fact negative).
- `crates/kebab-eval/src/golden.rs` 또는 sister 모듈 — multi-hop fixture parsing 지원. 기존 single-pass fixture 와 같은 `[[question]]` table 형식, `multi_hop_required: bool` 필드 추가.
- `crates/kebab-eval/src/runner.rs` — multi-hop fixture 인식 시 현재는 `--multi-hop` flag 없으니 single-pass 로 그대로 실행 (baseline 측정 의도). PR-4 머지 후 multi-hop path 도 호출하도록 갱신.
- `crates/kebab-eval/tests/multi_hop_golden_smoke.rs` — fixture parse + baseline run round-trip pin (실제 LLM 호출 없음, mock LLM 또는 `#[ignore]`).

**Implementation order**:
1. fixture file 작성 (15 question, kebab repo 자체 corpus 기반). 질문 작성은 작업 시 사용자에게 1-2 sample 제공 + 나머지 자동 생성 (workspace 의 README / HANDOFF / design doc 기반 cross-doc 질문 합성).
2. `MultiHopGoldenQuestion` struct + TOML deserialize.
3. eval runner 가 새 fixture 인식 + 기존 metric (`precision_at_5`, `precision_at_10`, `citation_coverage`) 호출.
4. baseline run command: `kebab-eval --fixture multi-hop-golden.toml --baseline-report /tmp/mh-baseline-v0.17.2.json`. 결과 commit 하지 않음 (artifact 는 별 디렉토리).

**Test**:
- fixture parse 통과 (15 question 모두 valid).
- baseline run 가 P@k 계산 출력 (실제 수치는 baseline anchor 라 PR commit 에 포함 안 함, separate run 으로 캡처).

**Wire 영향**: 없음.

**Risks**:
- fixture 질문 품질 — 너무 쉽거나 너무 어려우면 baseline vs multi-hop 차이가 noise 에 묻힘. 사용자 sample 1-2 question 확인 후 진행.

---

## PR-2: kebab-rag MultiHopPipeline (fixed depth=2) + AskOpts.multi_hop

**Goal**: multi-hop dispatch + decompose + synthesize 두 단계 구현. dynamic iter (decide loop) 는 PR-3.

**Files**:
- `crates/kebab-rag/src/pipeline.rs`:
  - `AskOpts.multi_hop: bool` 필드 추가.
  - `impl Default for AskOpts` 도입 (HOTFIXES 2026-05-07 의 known limitation 해소).
  - `RagPipeline::ask_multi_hop(query, opts) -> Result<Answer>` 신규.
  - `RagPipeline::ask` 의 entry 에 dispatcher 한 줄: `if opts.multi_hop { return self.ask_multi_hop(query, opts); }`.
  - depth=2 hard-coded: decompose 1 회 → 각 sub-query retrieve → synthesize 1 회. decide loop 없음.
  - prompts: `MULTI_HOP_DECOMPOSE_PROMPT` + `MULTI_HOP_SYNTHESIZE_PROMPT` const.
  - `PROMPT_TEMPLATE_VERSION_MULTI_HOP = "rag-multi-hop-v1"` const.
- `crates/kebab-core/src/lib.rs` (또는 traits):
  - `RefusalReason::MultiHopDecomposeFailed` 신규 variant.
- All `AskOpts` 명시 초기화 site (kebab-cli + kebab-tui + kebab-mcp + integration test):
  - `multi_hop: false` 명시 추가 — `Default` 도입으로 점진 정리 가능하지만 PR-2 는 명시.
- `crates/kebab-rag/tests/multi_hop.rs` 신규 — mock LLM (`MockLlm` trait impl) + mock Retriever 로 dispatch / decompose / synthesize 동작 핀.

**Implementation order**:
1. `AskOpts.multi_hop` 필드 + `Default` impl.
2. 모든 caller 갱신 (`multi_hop: false` 또는 `..Default::default()` 사용).
3. `MULTI_HOP_DECOMPOSE_PROMPT` + `MULTI_HOP_SYNTHESIZE_PROMPT` const.
4. `ask_multi_hop` 의 mock-friendly skeleton:
   - decompose LLM call → JSON array parse
   - 각 sub-query 로 `retriever.search()` 호출
   - chunk pool 누적 + dedup
   - synthesize LLM call → Answer 생성
5. `RagPipeline::ask` 에 dispatcher.
6. `RefusalReason::MultiHopDecomposeFailed` variant.
7. Integration test: mock LLM 가 ["q1", "q2"] decompose 후 "Final answer" synthesize 반환 → Answer.answer 검증.

**Test**:
- `ask_multi_hop_dispatches_when_flag_set` — `opts.multi_hop=true` 시 multi-hop path 호출 확인.
- `ask_multi_hop_decompose_parse_failure_returns_refusal` — decompose LLM 가 잘못된 JSON 반환 시 `MultiHopDecomposeFailed` refusal.
- `ask_multi_hop_empty_decompose_falls_back_to_single_query` — decompose 가 `[]` 또는 `[원본]` 반환 시 sub-query 1 개 (원본) 로 진행.
- `ask_with_multi_hop_false_keeps_legacy_path` — 회귀 핀.

**Wire 영향**: `Answer.hops` 미노출 (internal only). PR-3 에서 채우기 시작.

**Risks**:
- prompt JSON parse 견고성 — LLM 이 markdown code fence (`\`\`\`json ... \`\`\``) wrap 가능. parser 가 fence strip + array deserialize.
- 모든 `AskOpts` caller 갱신 누락 시 compile fail (긍정적 측면 — 자동 발견).

---

## PR-3: Dynamic iteration (decide loop + caps)

**Goal**: depth=2 fixed → dynamic N-hop. LLM 의 decide signal + max_depth / max_sub_queries / max_pool_chunks cap.

**Files**:
- `crates/kebab-rag/src/pipeline.rs`:
  - `ask_multi_hop` 의 decompose + synthesize 사이에 decide loop.
  - `MULTI_HOP_DECIDE_PROMPT` const.
  - hop trace 누적 (`Vec<HopRecord>`) — `Answer.hops` field 의 internal staging.
- `crates/kebab-config/src/lib.rs`:
  - `RagCfg` 에 `multi_hop_max_depth: u32` (default 3), `multi_hop_max_sub_queries_per_iter: u32` (default 5), `multi_hop_max_pool_chunks: u32` (default 30) 추가. 모두 additive serde default. env override 동반.
- `crates/kebab-core/src/lib.rs`:
  - `Answer.hops: Option<Vec<HopRecord>>` 필드 additive.
  - `HopRecord { iter, kind, sub_queries, decision, new_sub_queries, context_chunks_added, total_context_chunks, forced_stop, llm_call_ms }` struct.
  - `HopKind::Decompose | Decide | Synthesize` enum.

**Implementation order**:
1. `HopRecord` + `HopKind` 도메인 타입.
2. `Answer.hops` field 추가 (additive).
3. RagCfg 새 3 노브 + config tests (default / env override / legacy parse — 기존 `legacy_config_without_*_uses_default` 패턴).
4. `MULTI_HOP_DECIDE_PROMPT` const.
5. `ask_multi_hop` 내부에 decide loop:
   - iter 0: decompose → sub_queries (HopRecord 1).
   - iter 1+: retrieve → pool 누적 → decide LLM call (HopRecord 추가) → continue 면 다음 iter 의 retrieve, stop 면 break.
   - cap 도달 (max_depth / max_total_sub_queries / max_pool_chunks) 시 forced_stop=true 로 break.
   - synthesize → Answer.hops 에 누적된 HopRecord array 첨부.
6. decide JSON parse failure → forced_stop synthesize (refusal 아님, 안전한 graceful degrade).

**Test**:
- `multi_hop_decide_stop_triggers_synthesize` — decide 가 `[]` 반환 시 즉시 synthesize.
- `multi_hop_decide_continue_adds_more_chunks` — decide 가 ["q4"] 반환 시 추가 retrieve + iter 2 진행.
- `multi_hop_max_depth_force_stops` — depth=max_depth 도달 시 `forced_stop=true` + 정상 answer.
- `multi_hop_pool_chunks_dedup_by_chunk_id` — 같은 chunk 가 두 sub-query 에서 나와도 pool 에 1 회.
- `multi_hop_decide_parse_failure_falls_through_to_synthesize` — decide JSON 파싱 실패 시 forced synthesize.

**Wire 영향**: `Answer.hops` 노출. `docs/wire-schema/v1/answer.schema.json` 갱신 (additive — `required` 변경 없음, `properties.hops.type = "array"` + `optional`).

**Risks**:
- prompt token cost — depth 깊을수록 packed_context 가 매 decide call 마다 LLM 에 보내짐. `cfg.rag.max_context_tokens` 안에서 trim.
- LLM 이 영원히 continue 반환 — max_depth cap 으로 강제 break, dogfood 후 default 3 검증.

---

## PR-4: CLI `--multi-hop` flag + wire JSON Schema

**Goal**: 사용자가 `kebab ask --multi-hop "..."` 로 진입. `Answer.hops` JSON Schema additive.

**Files**:
- `crates/kebab-cli/src/main.rs`:
  - `Ask` subcommand 에 `--multi-hop` flag (default false).
  - `AskOpts.multi_hop` 로 전달.
  - `--show-citations` 와 동일 surface — `--hide-citations` 와 orthogonal.
- `docs/wire-schema/v1/answer.schema.json`:
  - `hops` field 추가 (optional, array of HopRecord). HopRecord 의 JSON Schema 도 inline 또는 `$defs/HopRecord`.
- `docs/wire-schema/v1/error.schema.json`:
  - `multi_hop_decompose_failed` code description 추가 (additive `enum` 확장 — strict validator 영향 있지만 single-producer 환경이라 patch minor 처리, HOTFIXES 2026-05-09 fb-32 패턴).
- `crates/kebab-app/src/error_wire.rs`:
  - `RefusalReason::MultiHopDecomposeFailed` → `error.v1.code = "multi_hop_decompose_failed"` 매핑.
- `crates/kebab-cli/tests/cli_ask_multi_hop.rs` 신규 — spawn-based test (mock environment, real binary), `--multi-hop --json` 출력에 `hops` field 등장 확인.

**Implementation order**:
1. CLI flag 정의 + AskOpts wiring.
2. wire schema JSON 갱신.
3. error_wire 매핑.
4. Integration test (spawn).

**Test**:
- `cli_ask_multi_hop_json_includes_hops` — `--multi-hop --json` 출력 parse 후 `Answer.hops` non-empty.
- `cli_ask_without_multi_hop_omits_hops` — 회귀 핀, 기존 single-pass 가 `hops: null` 또는 absent.

**Wire 영향**: `answer.v1` schema additive (description 갱신 + optional field). `schema_version` 그대로 `answer.v1` (additive minor, fb-32 패턴).

---

## PR-5: MCP `multi_hop` argument + SKILL.md

**Goal**: agent 가 `mcp__kebab__ask` 호출 시 `multi_hop: true` 옵션 사용 가능.

**Files**:
- `crates/kebab-mcp/src/lib.rs`:
  - `ask` tool 의 input schema 에 `multi_hop: bool` (default false) 추가.
  - tools/list 의 ask description 에 multi-hop 한 줄.
  - call_tool 의 ask arm 가 `AskOpts.multi_hop` 전달.
- `integrations/claude-code/kebab/SKILL.md`:
  - ask 절 (line ~95-115) 에 multi-hop bullet 추가:
    - 비용 trade-off (2-5× LLM call)
    - `multi_hop: true` argument 사용 케이스 (X 와 Y 의 관계, prerequisite chain, cross-doc reasoning)
    - `Answer.hops` 의 trace 정보 surface
    - 비-multi-hop 인 경우 (단순 fact-finding 은 single-pass 가 더 빠름)
- `crates/kebab-mcp/tests/tools_call_ask_multi_hop.rs` 신규 — `multi_hop: true` argument 가 multi-hop pipeline 호출 확인.

**Implementation order**:
1. MCP tool schema + dispatch.
2. SKILL.md 갱신.
3. Integration test.

**Test**:
- MCP `tools/call` ask 가 `multi_hop: true` 받고 정상 처리.
- `capabilities.multi_hop_ask` schema.v1 noeve flag 도입 검토 (선택 — agent 가 binary 지원 여부 detect 가능). additive bool.

---

## PR-6: TUI Ask Multi-hop toggle + hop trace render

**Goal**: TUI Ask 패널에서 multi-hop 모드 켜고 답변 본문에 hop trace 시각화.

**Files**:
- `crates/kebab-tui/src/ask.rs`:
  - `AskState.multi_hop: bool` 필드.
  - keybinding: `F2` 또는 `Ctrl-T` (spec 의 binding note 참조, implementation 단계 결정). PR commit 메시지에 결정 근거 명시.
  - 답변 본문 위에 `[multi-hop: depth=3, sub_queries=8]` 같은 trace summary row.
- `crates/kebab-tui/src/inspect.rs` (또는 신규 hop_inspect 모듈):
  - Inspect 패널에 `InspectTarget::Hop(turn_index)` variant — Ask 트랜스크립트의 한 turn 의 hop trace detail (각 sub-query + retrieved chunks + decide signal) 표시.
- `crates/kebab-tui/src/cheatsheet.rs`:
  - cheatsheet popup 에 multi-hop toggle binding + Inspect hop detail navigation 추가.

**Implementation order**:
1. `AskState.multi_hop` field + toggle binding (cheatsheet test 로 binding 확정).
2. trace summary row 렌더.
3. Inspect hop detail target.
4. cheatsheet 갱신.

**Test**:
- TUI integration: toggle 시 `AskState.multi_hop` 가 flip.
- multi-hop 답변 후 trace summary row 표시.
- Inspect 진입 후 hop detail navigate.

**Wire 영향**: 없음 (TUI 표면만).

---

## v0.18.0 cut (PR-6 머지 후)

**Trigger** (CLAUDE.md 의 release rule):
- frozen design contract 변경 (§3.8 RAG sub-section "Multi-hop" 추가) — PR-3 또는 PR-4 시점에 frozen design doc update.
- 사용자 도그푸딩 영향 (새 `--multi-hop` surface).
- `prompt_template_version` cascade (`rag-multi-hop-v1` 신규).

**Cut steps**:
1. workspace Cargo.toml version 0.17.2 → 0.18.0 (minor bump — surface 확장).
2. HANDOFF.md 한 줄 요약 갱신 (v0.18.0 cut + fb-41 multi-hop).
3. HOTFIXES.md 의 PR-2~PR-6 entry 들 anchor 정리 (`post-fb-41` → `v0.18.0`).
4. `gitea-release v0.18.0 --auto-notes` + release notes.
5. INDEX.md 의 fb-41 status `open` → `completed`.

## Self-review notes

- PR-1 (eval) 가 PR-2 와 독립 가능. PR-2 는 PR-1 머지 없이도 ship 가능 (단 baseline 측정 안 됨). 즉 직렬 dependency 는 없으나 PR-1 부터 진행 권장.
- PR-3 의 RagCfg 새 3 노브가 legacy config 파싱과 호환 — `#[serde(default)]` 패턴, kebab-config tests 의 legacy fixture 갱신은 PR-3 의 책임.
- AskOpts.multi_hop 가 PR-2 에서 도입되지만 actual multi-hop path 는 PR-2 의 fixed depth=2 만 동작. PR-3 의 decide loop 가 도입돼야 진짜 dynamic. caller 가 PR-2 단계에서 `multi_hop: true` 설정하면 단순 decompose+synthesize (depth=2) 만 — 의도된 staging.
- 모든 PR 가 회귀 핀 (existing single-pass path 동작 무변경) 포함. fb-15 multi-turn 와 fb-33 streaming 와의 orthogonality 도 회귀 핀 후보 — PR-3 단계에서 추가 검토.
- frozen design 갱신 timing: PR-3 의 wire `Answer.hops` 노출 시점이 적당. PR-3 commit 에 design doc §3.8 의 "Multi-hop" sub-section 추가 (verbatim, 본 spec 의 §1-§5 요약).
