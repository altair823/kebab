---
title: "p9-fb-41 multi-hop RAG (query decomposition + dynamic N-hop)"
date: 2026-05-25
task_id: p9-fb-41
phase: P9
status: open
target_version: 0.18.0
contract_source: ./2026-04-27-kebab-final-form-design.md
contract_sections: [§3.8 RAG, §7 RAG pipeline]
---

# p9-fb-41 — Multi-hop RAG design

## 문제 / 동기

도그푸딩 2026-05-06 — Claude Code 가 kebab CLI 사용 후 "추론 약함" 지적. 다단계 질문 ("X 와 Y 의 공통 prerequisite 인 Z 는?", "A 가 사용하는 library 중 deprecation 된 게 있나?") 에서 single-pass retrieval 이 한 번에 모든 근거 모으지 못함 → LLM 이 context 없는 부분 추측 / hallucinate.

근본 한계: chunk-level retrieval 이 chunk 간 관계 (cross-doc reference, prerequisite chain, entity coreference) 직접 따라가지 못함. semantic embedding 이 query↔chunk 1:1 비교라 "A 를 알아내야 B 를 검색" 같은 sequential dependency 미지원.

## 사용자 결정 (2026-05-25 AskUserQuestion)

| Axis | 결정 | 근거 |
|------|------|------|
| Approach | **Query decomposition** (LLM 서브-질문) | 가장 명확한 multi-hop RAG 패턴. graph-based 는 schema migration 큰 부담, query expansion 만으로는 진짜 multi-hop 아님 |
| Trigger | **Explicit `--multi-hop` flag** | LLM 호출 N 회 비용 명시. 기본 single-pass 유지 (예측 가능한 latency / cost). heuristic auto-detect 는 judge LLM 의 false positive 위험 |
| MVP scope | **Dynamic N-hop** (LLM 이 depth 결정) | ReAct/CoT 형태 — 첫 decompose seed 후 LLM 이 "충분?" 결정. 단순 depth=2 보다 더 자연스러운 reasoning, max_depth cap 으로 안전 |
| Eval | **Multi-hop golden set 먼저** | 구현 전 baseline 측정 → 머지 후 Δ 수치화. fb-39 의 P@k metric 인프라 그대로 활용, multi-hop fixture 만 신규 |

(3 의 dynamic + 1 의 decomposition 은 hybrid 로 결합: 첫 iter 에 decompose seed, 이후 iter 마다 LLM 이 "추가 sub-question?" 결정 + max_depth cap.)

## 동결된 설계 결정

### 1. Pipeline 구조

```
ask_multi_hop(query, opts) →
    iter 0: decompose(query) → [q1, q2, q3, ...]  (LLM call 1)
    iter 1: retrieve(q1), retrieve(q2), retrieve(q3) → context_pool_1
            decide(query, context_pool_1) → continue? + new sub-queries  (LLM call 2)
    iter 2: retrieve(new_q1), retrieve(new_q2) → context_pool_2 (이전 pool 누적)
            decide(query, all_pools) → continue? + new sub-queries  (LLM call 3)
    ...
    iter N: stop signal OR max_depth reached
    synthesize(query, all_pools) → final answer + citations  (LLM call N+1)
```

각 iter 의 sub-queries 개수 cap: `max_sub_queries_per_iter = 5`. 누적 LLM 호출 = (max_depth + 1) — decompose / decide / decide / ... / synthesize.

### 2. Stop condition (dynamic depth)

LLM 의 `decide` call 이 두 신호 중 하나 반환:
- **`continue`**: 새 sub-query JSON array (최대 `max_sub_queries_per_iter`), pipeline 이 retrieve loop 다음 iter.
- **`stop`**: empty array (`[]`), pipeline 이 synthesize 단계로.

추가 안전 cap (LLM 이 영원히 `continue` 반환하는 케이스):
- `max_depth = 3` (default, config 노브). depth 도달 시 강제 synthesize.
- `max_total_sub_queries = 12` 누적. 도달 시 강제 synthesize.

각 cap 도달 시 `Answer.refusal_reason` 가 아닌 정상 답변 — 단 `Answer.hops[].forced_stop = true` 로 trace 명시.

### 3. AskOpts 확장 (additive)

```rust
pub struct AskOpts {
    // ... 기존 필드 ...

    /// p9-fb-41: multi-hop mode 활성화. 기본 false (single-pass).
    /// `kebab ask --multi-hop` flag, MCP `ask` tool 의 `multi_hop: true`,
    /// TUI Ctrl-M toggle 이 모두 이 한 필드로 routing.
    pub multi_hop: bool,
}
```

`AskOpts::default()` (또는 builder) 가 `multi_hop: false` — 기존 caller 자동 backwards-compat.

**메모**: HOTFIXES 2026-05-07 fb-30 entry 가 명시했듯 현재 `AskOpts` 는 `Default` 미구현이라 모든 호출 site (kebab-cli + kebab-tui + kebab-mcp + kebab-app integration test) 가 9 field 를 명시 초기화. fb-41 의 신규 `multi_hop` field 추가 시 모든 site 도 명시적 `multi_hop: false` 추가 필요. PR-2 의 부수 작업으로 `impl Default for AskOpts` 동시 도입 권장 — 향후 field 추가의 maintenance 비용 ↓.

### 4. RagPipeline 신규 method

```rust
impl RagPipeline {
    /// p9-fb-41: multi-hop ask. `opts.multi_hop == true` 일 때만 호출.
    /// 내부적으로 `ask` 와 별도 path (decompose → iterate → synthesize).
    /// 일반 `ask` 와 동일 wire (`Answer`) 반환, `hops` 필드만 추가로 채움.
    pub fn ask_multi_hop(&self, query: &str, opts: AskOpts) -> Result<Answer>;
}
```

`ask` 의 entrypoint 에 dispatcher 한 줄:
```rust
pub fn ask(&self, query: &str, opts: AskOpts) -> Result<Answer> {
    if opts.multi_hop {
        return self.ask_multi_hop(query, opts);
    }
    // ... 기존 single-pass path ...
}
```

CLI / MCP / TUI 모든 caller 가 `opts.multi_hop` 만 set 하고 `ask` 호출 — entry 한 곳에서 분기. multi-turn (`ask_with_history`) 와 multi-hop 은 orthogonal — combined 가능 (history + multi-hop).

### 5. Wire schema additive (answer.v1)

`answer.v1` 에 optional `hops` field 추가:

```json
{
  "schema_version": "answer.v1",
  "answer": "...",
  "grounded": true,
  "citations": [...],
  "conversation_id": null,
  "turn_index": null,
  "hops": [
    {
      "iter": 0,
      "kind": "decompose",
      "sub_queries": ["q1", "q2", "q3"],
      "llm_call_ms": 1234
    },
    {
      "iter": 1,
      "kind": "decide",
      "decision": "continue",
      "new_sub_queries": ["q4"],
      "context_chunks_added": 3,
      "forced_stop": false,
      "llm_call_ms": 890
    },
    {
      "iter": 2,
      "kind": "decide",
      "decision": "stop",
      "new_sub_queries": [],
      "context_chunks_added": 0,
      "forced_stop": false,
      "llm_call_ms": 654
    },
    {
      "iter": 3,
      "kind": "synthesize",
      "total_context_chunks": 8,
      "llm_call_ms": 2103
    }
  ]
}
```

`hops` 가 `None` 이면 single-pass (기존 동작). additive minor — schema_version 그대로 `answer.v1`, JSON Schema description 의 `hops` 필드를 `optional` 명시.

### 6. Prompt templates (kebab-rag 내부)

세 신규 prompt template:

**decompose**:
```
사용자 질문을 다단계 추론에 필요한 sub-question 들로 분해하라.

원본 질문: {query}

규칙:
- 최대 {max_sub_queries_per_iter} 개
- 각 sub-question 은 독립적으로 검색 가능해야 함
- 원본이 이미 단순하면 1 개만 반환
- JSON array 만 출력 (no prose)

출력 예: ["sub-question 1", "sub-question 2"]
```

**decide** (매 iter):
```
원본 질문: {query}

지금까지 모은 근거 (chunk N 개):
{packed_context_snippet}

추가 retrieval 이 필요한가? 필요하면 새 sub-question 들 (최대 {max_sub_queries_per_iter} 개) 을 JSON array 로,
충분하면 빈 array `[]` 를 반환하라.

남은 깊이: {max_depth - current_depth}
```

**synthesize**:
```
원본 질문: {query}

다음 chunk 들을 근거로 답하라:
{packed_context_with_citations}

규칙:
- 모든 주장에 [N] citation marker
- 근거 없는 부분은 명시적으로 말하라 ("문서에서 확인되지 않음")
- 한국어로 답변

답변:
```

`prompt_template_version` cascade: 신규 `rag-multi-hop-v1` 상수. 기본 single-pass 의 `rag-v2` 와 별개로 추적 — `Answer.prompt_template_version` 가 단일 값이라 multi-hop 답변은 `rag-multi-hop-v1`.

### 7. Retrieval 합성 (context pool dedup)

각 iter 의 retrieve 결과를 누적 pool 에 합성. dedup 정책:
- `chunk_id` 동일하면 첫 occurrence 유지 (이후 추가된 sub-query 도 같은 chunk 가져오면 skip).
- 누적 pool 의 max size: `max_pool_chunks = 30` (cfg 노브). 초과 시 가장 마지막 추가된 sub-query 의 lowest-rank chunk 부터 drop.
- pool 의 token 한도: `cfg.rag.max_context_tokens` (single-pass 와 동일) — synthesize 단계에서 char budget 으로 cap.

### 8. Cost / Latency 예측

LLM call 수 (default `max_depth=3`):
- 최소: 2 (decompose + stop + synthesize) — depth=1
- 중간: 3-4 (depth 2-3)
- 최대: 5 (decompose + decide×3 + synthesize) — max_depth 도달

대비 single-pass 는 1 LLM call. 즉 multi-hop = **2-5× LLM 호출 + 2-12× retrieval** (sub-query 별). 사용자가 `--multi-hop` 명시 시만 발동 — cost surprise 회피.

latency: CPU only 환경의 cold-start 8B+ 모델은 multi-hop 가 무의미 (총 5-10 분). 권장 모델 = ≤4B Q4 (v0.17.1 README 의 권장 그대로). `[models.llm] request_timeout_secs` (v0.17.1) cap 각 call 적용.

### 9. Refusal / error handling

- decompose 가 JSON parse 실패 / 빈 array → `RefusalReason::MultiHopDecomposeFailed` (신규). 새 enum variant.
- decide 가 JSON parse 실패 → 강제 synthesize (LLM 가 stop 결정한 것처럼 처리, `forced_stop=true`).
- 어떤 sub-query 가 retrieval 0 hit → skip (해당 hop 의 `context_chunks_added=0`), iter 계속.
- 모든 iter 누적 pool 이 비어 있으면 synthesize 가 `grounded=false` + `RefusalReason::NoChunksFound`.
- LLM stream 도중 cancel → 부분 `hops` array 까지 채워서 `Answer.refusal_reason = LlmStreamAborted` (fb-15 와 동일 패턴).

### 10. Streaming (fb-33 와 통합)

`stream_sink` 가 set 되어 있으면:
- 각 hop 의 LLM call 이 시작될 때 `StreamEvent::Token { delta: "[hop iter=N kind=decompose ...]\n" }` 같은 trace event (debug only) 또는 새 `StreamEvent::HopStarted` variant 신설.
- 최종 synthesize 의 token 만 user-visible delta (single-pass 와 동일).
- 결정: trace event 는 `StreamEvent::HopStarted { iter, kind }` 신규 variant — additive enum.

`AskOpts.stream_sink` 가 `None` 이면 모든 hop blocking, `Final` event 한 번만 emit.

## 호출 면 (Surface) — PR 분할

| PR | 범위 | 영향 |
|----|------|------|
| **PR-1: eval golden set + baseline** | `tasks/eval/multi-hop-golden.toml` 신규 (10-15 question), `kebab-eval` runner 확장 (multi-hop fixture 지원), baseline run | metric 인프라만, RAG pipeline 미변경 |
| **PR-2: kebab-rag MultiHopPipeline (fixed depth=2)** | `RagPipeline::ask_multi_hop` 신규, `AskOpts.multi_hop` 필드 추가, decompose + synthesize prompts, depth=2 fixed (decide skip) | wire `Answer.hops` 미노출 (internal only) |
| **PR-3: dynamic iteration** | `decide` prompt + LLM call loop, `max_depth` / `max_sub_queries_per_iter` / `max_pool_chunks` config 노브, refusal variant 추가 | wire `Answer.hops` 채우기 시작 |
| **PR-4: CLI `--multi-hop` flag + wire** | `kebab-cli` 에 flag, `Answer.hops` JSON Schema additive, `error.v1` code `multi_hop_decompose_failed` | wire breaking 아님 (additive) |
| **PR-5: MCP + SKILL.md** | `mcp__kebab__ask` tool 의 `multi_hop: bool` argument, SKILL.md 의 ask 절에 multi-hop 안내 + cost trade-off | agent 통합 표면 |
| **PR-6: TUI Multi-hop toggle + trace render** | Ask 패널의 multi-hop toggle (`AskState.multi_hop`), 답변 본문 위에 hop trace summary (sub-queries / depth 표시), Inspect 패널에 hop detail | UI 표면 |

> **PR-6 binding note**: `Ctrl-M` 은 terminal protocol 상 `Enter` 와 동일 keycode (`\r`) — crossterm 일부 terminal 에서 두 binding ambiguous. 후보:
> - `F2` (cheatsheet 의 `F1` sibling, 새 functional area)
> - `:m` vim-style command (mode machine 위에 ex command 추가 — 부담 큼)
> - `Ctrl-T` (toggle, 다른 binding 과 충돌 없음 — Library `t` 가 tag filter 와 별개)
>
> PR-6 implementation 단계에서 cheatsheet 갱신 + crossterm test 한 후 최종 선택. spec 은 binding 미확정.

각 PR 가 머지 후 누적, 마지막 PR 후 v0.18.0 cut (minor bump — 사용자-visible 새 surface 추가 + prompt_template_version cascade).

## Eval golden set scope (PR-1)

`tasks/eval/multi-hop-golden.toml` 형식 (fb-39 의 single-pass golden 와 sister):

```toml
[[question]]
id = "mh-001"
query = "rust 의 async runtime 중 kebab 이 사용하는 것은? 그 runtime 의 default executor 는?"
expected_answer_contains = ["tokio", "thread pool"]
expected_sources = [
    "Cargo.toml",
    "crates/kebab-app/src/lib.rs",
]
multi_hop_required = true   # single-pass 로는 잘 안 됨, 검증용 flag

[[question]]
id = "mh-002"
# ... 추가 ...
```

15 question 목표. 출처 분포:
- 5 question: 두 doc 가로질러 (cross-doc reasoning)
- 5 question: 같은 doc 안 두 section 간 (intra-doc multi-hop, single-pass 도 가능할 수 있음 — baseline 비교)
- 5 question: 단순 single-fact (negative — multi-hop 이 single-pass 대비 regression 안 일으키는지 검증)

metric:
- **P@5, P@10**: 기존 fb-39 metric, multi-hop 결과의 citations 도 동일 평가.
- **answer correctness**: `expected_answer_contains` 의 substring 모두 등장 시 1, 아니면 0. 단순 metric — semantic match 아님 (eval LLM 도입은 별 작업).
- **citation coverage**: `expected_sources` 중 actual citations 에 등장하는 비율.

baseline (현 single-pass) → 각 metric 측정 → PR-2/3 머지 후 재측정 → Δ 보고.

## Out of scope (future PR)

- LLM 기반 semantic eval (answer 의 의미 일치도 측정 — gpt-4 같은 strong eval-LLM 필요)
- Graph-based retrieval (chunk 간 link 추출 + 그래프 traversal) — fb-41 spec 의 alternative axis A3 였음, 사용자가 query decomposition 선택
- ReAct-style tool calling (LLM 이 직접 `retrieve(query)` tool invocation) — 현재 decide loop 가 비슷한 동작이지만 tool calling protocol 자체는 도입 안 함
- Heuristic auto-detect (`--multi-hop-auto`) — judge LLM 도입 비용 + 잘못된 분기 위험. 향후 사용자 도그푸딩 결과 기반 재검토
- multi-turn (`history`) + multi-hop combined 의 prompt budget 최적화 — orthogonal 결합 자체는 PR-2 부터 지원, prompt token 한도 정밀 조절은 별 PR

## 검증 (각 PR 별)

| PR | Test |
|----|------|
| PR-1 | 15 question golden fixture parse OK + single-pass baseline metric 출력 |
| PR-2 | `ask_multi_hop` integration test (decompose mock + 2 retrieve mock + synthesize mock) + `AskOpts.multi_hop=false` 시 기존 path 호출 회귀 |
| PR-3 | dynamic iter (depth 2-3) 통합 test, `max_depth` cap 동작, `decide` JSON parse failure → forced synthesize |
| PR-4 | `kebab ask --multi-hop --json` 의 stdout 에 `Answer.hops` 등장 |
| PR-5 | `mcp__kebab__ask` 의 `multi_hop: true` argument tools/call 통과 |
| PR-6 | TUI test — Ctrl-M toggle, hop trace render |

## Cross-link

- Spec stub: `tasks/p9/p9-fb-41-multi-hop-reasoning.md`
- 의존: fb-39 eval foundation (P@k metric 인프라) — 이미 머지됨
- Sister: fb-15 (multi-turn, history) — orthogonal, combined 가능
- 관련 wire: `answer.v1` (additive `hops`), `error.v1` (신규 code `multi_hop_decompose_failed`)
- 관련 design 절: §3.8 RAG — 본 spec 가 sub-section "Multi-hop" 으로 갱신 예정 (PR-3 또는 PR-4 시점에 frozen design doc update)
