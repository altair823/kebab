---
phase: P9
component: kebab-rag + kebab-app
task_id: p9-fb-15
title: "RAG multi-turn — history-aware prompt + token budget"
status: planned
depends_on: []
unblocks: [p9-fb-16, p9-fb-17, p9-fb-18]
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§7 RAG]
source_feedback: p9-dogfooding-feedback.md item 13
---

# p9-fb-15 — RAG multi-turn core

## Goal

`kebab-rag` 가 conversation history (`Vec<Turn>`) 를 받아 prompt 빌드. token budget 안에서 retrieval k 와 history truncation 정책으로 fit.

## Allowed dependencies

- 기존 kebab-rag deps.
- `tiktoken-rs` 또는 LLM family-specific tokenizer (gemma 토큰화). 우선 char 기반 ÷4 근사 (cheap & 의존 X).

## Public surface

```rust
pub struct Turn {
    pub question: String,
    pub answer: String,
    pub citations: Vec<Citation>,
    pub ts: OffsetDateTime,
}

pub fn ask_with_history(
    cfg: &Config,
    new_question: &str,
    history: &[Turn],
    stream: Sender<RagEvent>,
) -> anyhow::Result<Answer>;
```

`kebab-app` 도 `ask_with_config_and_history(cfg, q, history, stream)` 추가.

## Behavior contract

- prompt 구조: `system_prompt + history_serialized + retrieved_chunks + new_question`. 형식 (roles 또는 plain text) 는 `prompt_template_version` bump (`rag-v1` → `rag-v2`).
- token budget: `cfg.rag.max_context_tokens`.
  - 우선순위: system + new_question 항상 포함.
  - 다음: retrieved chunks (k=cfg.search.default_k 부터, budget 초과시 k 감소).
  - 마지막: history. budget 남은 만큼 newest turn 부터 포함, 부족하면 oldest turn drop. 최소 0 turn 까지 가능.
- retrieval query: `new_question + " " + last_turn.answer.first_N_chars(200)` concat (cheap query expansion). LLM 기반 standalone question rewriting 은 P+.
- streaming: `RagEvent::Token(s)` / `RagEvent::Done(answer)` / `RagEvent::Error(e)`.

## Test plan

| kind | description |
|------|-------------|
| unit | history 5 turn → token budget 초과 시 oldest 부터 drop |
| unit | retrieved_chunks vs history 의 priority |
| integration | 가짜 history (Q1/A1) + new Q2 → prompt 에 Q1/A1 포함 (snapshot) |

## DoD

- [ ] `cargo test -p kebab-rag -p kebab-app` 통과
- [ ] `prompt_template_version` bump (`rag-v2`)
- [ ] HOTFIXES X (신규)
- [ ] frozen design §7 RAG 절 갱신 (multi-turn 정책)

## Out of scope

- LLM 기반 question rewriting (P+)
- conversation 영속화 (p9-fb-17)
- UI (p9-fb-16, p9-fb-18)
