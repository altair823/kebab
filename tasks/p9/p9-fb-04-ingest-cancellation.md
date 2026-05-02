---
phase: P9
component: kebab-app + kebab-cli + kebab-tui
task_id: p9-fb-04
title: "Cooperative ingest cancellation (Ctrl-C / Esc)"
status: in_progress
depends_on: [p9-fb-01]
unblocks: []
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§7 ingest]
source_feedback: p9-dogfooding-feedback.md item 2
---

# p9-fb-04 — Ingest cancellation

## Goal

ingest 가 사용자 cancel 신호 (Ctrl-C / Esc) 받으면 step boundary 에서 즉시 중단. 부분 진행은 SQLite 에 commit 된 상태 유지 — resume 은 idempotent.

## Allowed dependencies

- `std::sync::atomic::AtomicBool` (Arc 공유). 외부 crate X.

## Public surface

```rust
#[doc(hidden)]
pub fn ingest_with_config_cancellable(
    config: kebab_config::Config,
    scope: SourceScope,
    summary_only: bool,
    progress: Option<Sender<IngestEvent>>,
    cancel: Option<Arc<AtomicBool>>,
) -> anyhow::Result<IngestReport>;
```

기존 `ingest_with_config_progress` (p9-fb-01) 가 `cancel=None` forwarding.

## Behavior contract

- check 위치 (step boundary): asset loop iteration 시작, embed batch 시작, vector upsert 직후. 가장 긴 wait 인 LLM 호출 (OCR / caption) 도 가능하면 token boundary 에서 check (Ollama HTTP 응답 stream 이라 partial cancel 가능).
- cancel triggered 시: 현재 in-flight asset 마무리 (rollback 하면 idempotent 깨질 수 있음 — commit 후 종료가 안전), 이후 asset 미실행, `IngestEvent::Aborted { partial_counts }` 발신, `Ok(IngestReport)` 반환 (Err 가 아님 — 정상 종료의 한 형태).
- CLI `kebab ingest`: Ctrl-C SIGINT handler 가 `cancel.store(true, Ordering::Relaxed)`. 두 번째 Ctrl-C 는 hard exit.
- TUI: `Esc` (ingest 진행 중에만) 또는 `Ctrl-C` 가 cancel signal. p9-fb-03 의 `IngestState.cancel_tx` 와 wiring.

## Test plan

| kind | description |
|------|-------------|
| unit | cancel=true set 후 다음 step 에서 Aborted event |
| integration | 100 md fixture, idx=10 cancel → DB 에 10 docs 만 commit, idempotent re-ingest 가능 |

## DoD

- [ ] `cargo test -p kebab-app` 통과
- [ ] CLI Ctrl-C handler test
- [ ] TUI Esc cancel test
- [ ] HOTFIXES X (신규)
- [ ] README — Ctrl-C 동작 명시

## Out of scope

- resume from checkpoint (현재는 idempotent re-run 으로 충분)
- embed / RAG streaming cancel (별도 task)
