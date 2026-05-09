---
title: "p9-fb-33 — Streaming ask (ndjson delta) design"
phase: P9
component: kebab-rag + kebab-cli + kebab-tui + wire-schema
task_id: p9-fb-33
status: design
target_version: 0.5.0
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§7 RAG, §10 UX, wire-schema answer.v1]
date: 2026-05-09
---

# p9-fb-33 — Streaming ask (ndjson delta)

## Goal

`kebab ask --stream` — agent 가 LLM token 을 도착 즉시 소비할 수 있도록 retrieval / token / final 세 단계 ndjson event 를 stderr 에 흘리고, 마지막 stdout 한 줄은 기존 `answer.v1` 그대로 유지. CLI surface 우선, MCP `kebab__ask` streaming 은 v0.5+ 별도 검토 (이 spec 의 scope 아님).

## Behavior contract

### Stream event taxonomy

3 variant 로 confined. `kind` discriminator + `ts` 타임스탬프 + variant 별 페이로드.

1. **`retrieval_done`** — pipeline 의 retrieve + stale-stamp 직후 1회. 페이로드는 `hits: search_hit.v1[]` (fb-32 의 `indexed_at` / `stale` 포함).
2. **`token`** — LLM 의 `TokenChunk::Token` 매 도착 시. 페이로드는 `delta: string` + `turn_index: integer | null` (multi-turn ask 의 `Answer.turn_index` 와 일치).
3. **`final`** — 모든 token 수신 + citation extract / validate 완료 후 1회. 페이로드는 `answer: answer.v1` (스키마 v1 통째).

terminal event = `final`. 모든 ask 는 `final` 또는 (cancel 경로) 0개 event 로 끝남 — 후자는 ndjson 흐름이 중간에 끊긴 형태.

### CLI flag

`kebab ask --stream` (boolean flag, default off). `--json` 와 독립:

| flag 조합 | stderr | stdout |
|----------|--------|--------|
| (없음) | (없음) | plain text answer + 근거 블록 |
| `--json` | (없음) | `answer.v1` 1회 |
| `--stream` | ndjson `answer_event.v1` events | `answer.v1` 1회 (final stdout line) |
| `--stream --json` | 동일 (stream 이 dominant) | 동일 |

backwards-compat: `--stream` 미사용 시 모든 동작 보존.

### Output stream

- ndjson event → **stderr**. 매 줄 한 event, `serde_json::to_string` + `writeln!`.
- final `answer.v1` → **stdout**. 기존 final-only consumer 가 stdout 만 파싱해도 호환.
- 선례: `ingest_progress.v1` 가 stderr ndjson + stdout `ingest_report.v1` final 패턴 사용.

### Cancel semantics

`kebab ask --stream` 의 stdout/stderr 가 외부에서 닫힘 (예: agent 가 SIGPIPE / `head -c 1` / connection close):

1. CLI main thread 의 `writeln!(stderr, ...)` 가 `io::ErrorKind::BrokenPipe` 반환.
2. CLI 가 receiver 폐기 (rx drop).
3. background thread 의 `pipeline.ask` 가 `stream_sink.send(StreamEvent::Token { .. })` 시 `SendError` 반환.
4. pipeline 의 token loop — 현재 `let _ = sink.send(t)` 로 swallow 하지만 본 task 에서 cancel 분기 추가: `SendError` 감지 시 LLM stream `break`, `finish_reason = FinishReason::Cancelled`, `RefusalReason::LlmStreamAborted` 로 Answer 채움, `answers` 테이블에 partial answer + cancel 사유 기록.
5. CLI background thread join → cancel 사유 명시한 Answer return → CLI 종료. stdout 은 이미 닫혀 final answer.v1 출력 시도해도 BrokenPipe 무시.

`io::ErrorKind::BrokenPipe` 만 cancel 처리. 그 외 IoError 는 fatal — `error.v1` stderr emit + exit 2.

### Wire schema delta

신규 `docs/wire-schema/v1/answer_event.schema.json`:

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://kb.local/wire/v1/answer_event.schema.json",
  "title": "AnswerEvent v1",
  "description": "Streaming event emitted by `kebab ask --stream`. One event per line on stderr. Discriminated by `kind`. Terminal: `final`. Final stdout line is `answer.v1` for backwards compat.",
  "type": "object",
  "required": ["schema_version", "kind", "ts"],
  "properties": {
    "schema_version": { "const": "answer_event.v1" },
    "kind": { "enum": ["retrieval_done", "token", "final"] },
    "ts":   { "type": "string", "format": "date-time" },
    "hits":       { "type": "array",   "description": "retrieval_done: search_hit.v1[]" },
    "delta":      { "type": "string",  "description": "token: incremental string chunk" },
    "turn_index": { "type": ["integer", "null"], "minimum": 0,
                    "description": "token: matches Answer.turn_index" },
    "answer":     { "type": "object",  "description": "final: complete answer.v1 payload" }
  }
}
```

기존 `answer.v1` / `search_hit.v1` / `citation.v1` 변경 없음.

### Domain API change

`kebab-rag::pipeline`:

```rust
#[derive(Clone, Debug)]
pub enum StreamEvent {
    RetrievalDone { hits: Vec<SearchHit> },
    Token { delta: String, turn_index: Option<u32> },
    Final { answer: Answer },
}

pub struct AskOpts {
    // ... 기존 필드
    /// p9-fb-33: was `Option<Sender<String>>`. Now carries discriminated
    /// events so callers can distinguish retrieval / per-token / final.
    pub stream_sink: Option<std::sync::mpsc::Sender<StreamEvent>>,
}
```

- internal API breaking. consumer = TUI worker + (없을 시) MCP. TUI 만 갱신.
- non-streaming consumer (`stream_sink: None`) 는 무영향.

## Allowed / forbidden dependencies

각 crate 기존 deps 유지. `mpsc::Sender` 는 std. 신규 dep 없음.

- `kebab-core` 는 `StreamEvent` 정의 안 함 (도메인 type 가 wire 변환과 분리되어 있고, StreamEvent 는 pipeline 의 communication channel — kebab-rag 안 위치 적절).
- `kebab-cli` 는 wire 변환 코드 (`wire::wire_answer_event(&StreamEvent) -> Value`) 추가 — `kebab-cli/src/wire.rs` 의 기존 패턴 따라.
- UI crate (kebab-tui) 가 직접 retriever / store 호출 X — `kebab-app` facade 통과만.

## Components

### kebab-rag::pipeline

- `enum StreamEvent` 신규 정의 (`pub`).
- `AskOpts.stream_sink` 타입 변경.
- `RagPipeline::ask`:
  - retrieve + stale-stamp 직후 `if let Some(sink) = &opts.stream_sink { let _ = sink.send(StreamEvent::RetrievalDone { hits: hits.clone() }); }` 발사. cancel 시 즉시 break out (이때는 LLM 도 안 부름).
  - token loop: `sink.send(StreamEvent::Token { delta: t, turn_index: opts.turn_index })`. SendError → cancel 분기.
  - 끝에서 `Final { answer: built_answer.clone() }` 발사.
- cancel 분기:
  ```rust
  // p9-fb-33: SendError → caller (CLI) closed the receiver,
  // probably due to BrokenPipe on stdout. Stop generation, mark
  // refusal, persist partial answer.
  if matches!(send_result, Err(_)) {
      finish_reason = FinishReason::Cancelled;
      break;
  }
  ```
  finish_reason = Cancelled 일 때 grounded=false + RefusalReason::LlmStreamAborted.

### kebab-app

- `AskOpts` re-export 만 (이미 public). `StreamEvent` 도 `pub use`.
- `App::ask` / `ask_with_session` 변경 없음 (opts 통과).

### kebab-cli

- `Cmd::Ask` 에 `#[arg(long)] stream: bool` 추가.
- `--stream` 분기:
  ```rust
  if cli.json && !stream || !cli.json && !stream {
      // 기존 final-only path
  } else if stream {
      let (tx, rx) = std::sync::mpsc::channel::<StreamEvent>();
      let cfg2 = cfg.clone();
      let q = query.clone();
      let opts2 = AskOpts { stream_sink: Some(tx), ..opts };
      let handle = std::thread::spawn(move || {
          kebab_app::ask_with_config(cfg2, &q, opts2)
      });
      let mut stderr = std::io::stderr().lock();
      let mut cancelled = false;
      for ev in rx {
          let v = wire::wire_answer_event(&ev);
          let line = serde_json::to_string(&v)?;
          if let Err(e) = writeln!(stderr, "{line}") {
              if e.kind() == std::io::ErrorKind::BrokenPipe {
                  cancelled = true;
                  break;
              }
              return Err(e.into());
          }
      }
      drop(stderr);
      let result = handle.join().expect("ask thread panic");
      let ans = result?;
      // final stdout line
      let mut stdout = std::io::stdout().lock();
      let _ = writeln!(stdout, "{}", serde_json::to_string(&wire::wire_answer(&ans))?);
      // cancel 또는 refusal 시 exit 1
      if !ans.grounded { return Err(RefusalSignal.into()); }
      Ok(())
  }
  ```
- `wire::wire_answer_event(&StreamEvent) -> Value` 추가 — discriminated by variant, schema_version 태그.

### kebab-tui

- ask worker 가 받던 `Sender<String>` → `Sender<StreamEvent>`.
- worker thread 의 receive loop:
  - `StreamEvent::Token { delta, .. }` → 기존 token 누적 path 그대로.
  - `StreamEvent::RetrievalDone { hits }` → minimal 안에선 ignore (citation 은 final 도착 후 표시 — fb-22 에서 살펴봄).
  - `StreamEvent::Final { answer }` → 이미 `App::ask` return 으로 받으므로 무시 가능 (또는 sanity check).
- snapshot 영향 없음 (token concat 결과 동일).

### kebab-mcp

변경 없음. `stream_sink: None` 유지. 향후 v0.5+ 에서 rmcp progress notification 채택 검토.

## Test plan

| kind | description |
|------|-------------|
| unit (kebab-rag) | `StreamEvent` serde round-trip — RetrievalDone / Token / Final 각각 한 줄 ndjson |
| unit (kebab-rag) | pipeline.ask + MockLm + sink: 발사 순서 = `RetrievalDone` 1회 → `Token`* → `Final` 1회 |
| unit (kebab-rag) | sink SendError (rx drop) → LLM loop 즉시 break + Answer.refusal_reason = `LlmStreamAborted` + answers row 기록 |
| unit (kebab-rag) | RetrievalDone 의 hits 가 Final.answer.citations 의 부분집합 (LLM 이 마커 안 쓴 hit 도 RetrievalDone 에 포함) |
| 통합 (kebab-cli) | `kebab ask --stream` stderr 가 valid ndjson — schema_version/kind/ts 모두 정상 |
| 통합 (kebab-cli) | `kebab ask --stream --json` stdout 마지막 줄이 `answer.v1` 통째 |
| 통합 (kebab-cli) | `kebab ask --json` (no --stream) 동작 무변경 — stdout final-only |
| 통합 (kebab-cli) | stdout 닫힘 시뮬 (`kebab ask --stream | head -c 1`) → process 정상 종료 + answers row 의 refusal_reason = LlmStreamAborted |
| 통합 (wire-schema) | answer_event.schema.json validate — RetrievalDone/Token/Final 샘플 |
| 통합 (kebab-tui) | 기존 ask snapshot 모두 통과 (token concat 결과 동일) |

LLM 의존: pipeline unit test 는 MockLm 활용 (이미 `crates/kebab-rag/tests/common/mod.rs` 의 `CountingLm` 패턴). CLI 통합 test 는 Ollama 필요 → `#[ignore]` gate.

## Implementation steps (high-level)

1. wire schema 신규 `answer_event.schema.json`.
2. `kebab-rag::pipeline::StreamEvent` enum 정의 + `AskOpts.stream_sink` 타입 변경.
3. `RagPipeline::ask`:
   - RetrievalDone 발사 추가.
   - token loop sink.send 의 SendError → cancel 분기.
   - Final 발사 추가.
4. `kebab-app` re-exports 갱신.
5. `kebab-tui` worker 의 `Sender<String>` → `Sender<StreamEvent>` 변환.
6. `kebab-cli`:
   - `--stream` flag.
   - `wire::wire_answer_event` 헬퍼.
   - background thread + main thread receive loop.
7. 단위 + 통합 테스트.
8. README + SMOKE — `--stream` 사용 예시.
9. tasks/INDEX.md / spec status flip.
10. `integrations/claude-code/kebab/SKILL.md` — agent 가 ndjson stream 을 어떻게 소비하는지 한 단락.

## Risks / notes

- **TUI sink 타입 breaking**: 1 곳만 수정. 기존 token 누적 path 는 `StreamEvent::Token { delta, .. }` 만 매치하면 동일 동작. snapshot 영향 없음.
- **`Final` event 의 Answer clone**: streaming path 만 부담. non-streaming caller 무영향.
- **BrokenPipe vs 일반 IoError**: `io::ErrorKind::BrokenPipe` 만 cancel. 그 외는 `error.v1` stderr emit + exit 2.
- **ndjson 줄 단위**: serde_json::to_string + writeln! 충분. embedded newline 은 serde 가 escape.
- **partial markdown safety**: out of scope. agent 책임.
- **multi-turn token_index**: streaming 과 fb-15 multi-turn 의 상호작용. 새 turn 마다 streaming 재시작이 자연스러움 (`Token.turn_index` 가 각 ask 호출 단위로 일관).

## Documentation updates (implementation PR 동시)

- `README.md` — Quick start 또는 명령 표에 `--stream` 한 줄.
- `docs/SMOKE.md` — `kebab ask --stream` walkthrough (실행 예시 + agent 가 stderr 파싱하는 패턴 한 단락).
- `tasks/p9/p9-fb-33-streaming-ask.md` — `status: open → completed`, design/plan 링크 추가.
- `tasks/INDEX.md` — fb-33 행 ✅ 표시.
- `integrations/claude-code/kebab/SKILL.md` — `--stream` 멘션 (CLI fallback 섹션).
- `tasks/HOTFIXES.md` — internal API breaking (AskOpts.stream_sink 타입 변경) 결정 로그 (선택, 머지 후 의문 발생 시).
