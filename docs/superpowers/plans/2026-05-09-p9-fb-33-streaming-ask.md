# p9-fb-33 — Streaming Ask Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `kebab ask --stream` that emits ndjson `answer_event.v1` events on stderr (RetrievalDone → Token* → Final) while keeping the final stdout line as the existing `answer.v1`. Cancel via stdout close → LLM stream break + `RefusalReason::LlmStreamAborted`.

**Architecture:** Pipeline-internal `enum StreamEvent` carries discriminated events. `AskOpts.stream_sink` switches type from `Sender<String>` to `Sender<StreamEvent>` (internal API breaking — TUI worker is the only consumer). CLI `--stream` flag spawns a background thread that runs `ask_with_config`; main thread drains the receiver, writes ndjson to stderr, and triggers cancel via `BrokenPipe` → channel drop → pipeline `SendError` break.

**Tech Stack:** Rust 2024, std::sync::mpsc, std::thread, time crate (RFC3339), serde, JSON Schema (answer_event.v1).

**Spec:** `docs/superpowers/specs/2026-05-09-p9-fb-33-streaming-ask-design.md`

---

## File Structure

| File | Responsibility | Action |
|------|----------------|--------|
| `crates/kebab-rag/src/pipeline.rs` | Add `enum StreamEvent`, switch `AskOpts.stream_sink` type, emit RetrievalDone/Token/Final, cancel branch on SendError | modify |
| `crates/kebab-app/src/lib.rs` | Re-export `StreamEvent` alongside existing `AskOpts` | modify |
| `crates/kebab-cli/src/main.rs` | New `--stream` flag on `Cmd::Ask`, background-thread driver, ndjson stderr writer, BrokenPipe handling | modify |
| `crates/kebab-cli/src/wire.rs` | New `wire_answer_event(&StreamEvent) -> Value` helper tagging `schema_version: "answer_event.v1"` | modify |
| `crates/kebab-tui/src/ask.rs` | Switch worker `Sender<String>` → `Sender<StreamEvent>`; `drain_stream` matches on `Token { delta }` | modify |
| `crates/kebab-tui/src/app.rs:217` | `pub rx: Option<Receiver<String>>` → `Option<Receiver<StreamEvent>>` | modify |
| `docs/wire-schema/v1/answer_event.schema.json` | NEW — discriminated ndjson schema | create |
| `crates/kebab-rag/tests/streaming_events.rs` | Unit/integration: order invariants + cancel + serde round-trip | create |
| `crates/kebab-cli/tests/wire_ask_stream.rs` | Integration: stderr ndjson + stdout final answer.v1 + BrokenPipe cancel | create |
| `crates/kebab-cli/tests/common/mod.rs` | Reuse existing helpers (`write_config_with_llm_model`, `ingest`, `backdate_updated_at`); add `run_ask_stream` if needed | modify |
| `README.md` | Quick start mention `--stream` | modify |
| `docs/SMOKE.md` | Walkthrough paragraph for streaming + cancel | modify |
| `tasks/p9/p9-fb-33-streaming-ask.md` | Status flip + design/plan links | modify |
| `tasks/INDEX.md` | fb-33 row → ✅ | modify |
| `integrations/claude-code/kebab/SKILL.md` | One-line CLI fallback note about `--stream` | modify |

---

## Pre-flight

- [ ] **Step 0.1: Branch off main**

```bash
git checkout main
git pull
git checkout -b feat/fb-33-streaming-ask
```

- [ ] **Step 0.2: Confirm spec branch is reachable (or already on main)**

```bash
git log --oneline spec/fb-33-streaming-ask -1
```

Expected: shows `4949775 spec(fb-33): streaming ask (ndjson delta) — design`. If the spec PR has not yet merged into main, `git merge spec/fb-33-streaming-ask` so the spec doc lives on this branch too.

---

## Task 1: Define `StreamEvent` enum + switch sink type

**Files:**
- Modify: `crates/kebab-rag/src/pipeline.rs`

- [ ] **Step 1.1: Write the failing serde test**

Append to `crates/kebab-rag/src/pipeline.rs` `#[cfg(test)] mod compute_stale_mirror_tests` block (or create a new sibling `mod stream_event_serde_tests`):

```rust
#[cfg(test)]
mod stream_event_serde_tests {
    use super::*;
    use kebab_core::{
        AnswerCitation, AnswerRetrievalSummary, ChunkId, ChunkerVersion, Citation,
        DocumentId, IndexVersion, ModelRef, RetrievalDetail, SearchHit, SearchMode,
        TokenUsage, TraceId,
    };
    use kebab_core::asset::WorkspacePath;
    use kebab_core::versions::PromptTemplateVersion;
    use time::macros::datetime;

    fn mk_hit() -> SearchHit {
        SearchHit {
            rank: 1,
            chunk_id: ChunkId("c1".into()),
            doc_id: DocumentId("d1".into()),
            doc_path: WorkspacePath::new("a.md".into()).unwrap(),
            heading_path: vec!["H".into()],
            section_label: None,
            snippet: "s".into(),
            citation: Citation::Line {
                path: WorkspacePath::new("a.md".into()).unwrap(),
                start: 1,
                end: 1,
                section: None,
            },
            retrieval: RetrievalDetail {
                method: SearchMode::Lexical,
                fusion_score: 0.5,
                lexical_score: Some(0.5),
                vector_score: None,
                lexical_rank: Some(1),
                vector_rank: None,
            },
            index_version: IndexVersion("v1".into()),
            embedding_model: None,
            chunker_version: ChunkerVersion("c@1".into()),
            indexed_at: datetime!(2026-05-09 12:00:00 UTC),
            stale: false,
        }
    }

    #[test]
    fn stream_event_token_serializes_with_kind_discriminator() {
        let ev = StreamEvent::Token { delta: "안녕".into(), turn_index: Some(0) };
        let v = serde_json::to_value(&ev).unwrap();
        assert_eq!(v["kind"], "token");
        assert_eq!(v["delta"], "안녕");
        assert_eq!(v["turn_index"], 0);
    }

    #[test]
    fn stream_event_retrieval_done_serializes_hits() {
        let ev = StreamEvent::RetrievalDone { hits: vec![mk_hit()] };
        let v = serde_json::to_value(&ev).unwrap();
        assert_eq!(v["kind"], "retrieval_done");
        assert_eq!(v["hits"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn stream_event_final_serializes_answer() {
        let answer = Answer {
            answer: "x".into(),
            citations: vec![],
            grounded: true,
            refusal_reason: None,
            model: ModelRef { id: "m".into(), provider: "p".into(), dimensions: None },
            embedding: None,
            prompt_template_version: PromptTemplateVersion("rag-v1".into()),
            retrieval: AnswerRetrievalSummary {
                trace_id: TraceId("t".into()),
                mode: SearchMode::Hybrid,
                k: 10, score_gate: 0.3, top_score: 0.5,
                chunks_returned: 1, chunks_used: 1,
            },
            usage: TokenUsage { prompt_tokens: 0, completion_tokens: 0, latency_ms: 0 },
            created_at: datetime!(2026-05-09 12:00:00 UTC),
            conversation_id: None,
            turn_index: None,
        };
        let ev = StreamEvent::Final { answer };
        let v = serde_json::to_value(&ev).unwrap();
        assert_eq!(v["kind"], "final");
        assert!(v["answer"].is_object());
    }
}
```

- [ ] **Step 1.2: Run test — verify failure**

```bash
cargo test -p kebab-rag --lib stream_event_serde_tests
```

Expected: FAIL — `cannot find type StreamEvent in scope`.

- [ ] **Step 1.3: Define the enum + switch AskOpts.stream_sink type**

In `crates/kebab-rag/src/pipeline.rs`, near the existing `PackedCitation` definition (around line 47-62), add:

```rust
/// p9-fb-33: streaming events the pipeline forwards into
/// [`AskOpts::stream_sink`] when present. Discriminated on `kind`
/// to match the wire `answer_event.v1` schema. Three variants:
///
/// - `RetrievalDone` — emitted once after retrieval + stale-stamp.
/// - `Token` — emitted per `TokenChunk::Token` from the LM.
/// - `Final` — emitted once after the full Answer is built (before
///   persistence). Always the terminal event on the success path.
///
/// On caller-side cancel (receiver dropped), the pipeline observes
/// the `SendError` from the next `Token` send and breaks the LM
/// loop — see `RagPipeline::ask` cancel branch. In that case
/// `Final` is NOT emitted (the answer still gets persisted with
/// `RefusalReason::LlmStreamAborted`).
#[derive(Clone, Debug, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StreamEvent {
    RetrievalDone {
        hits: Vec<SearchHit>,
    },
    Token {
        delta: String,
        turn_index: Option<u32>,
    },
    Final {
        answer: Answer,
    },
}
```

`Answer` and `SearchHit` are already imported at the top of the file. Add `serde::Serialize` import via `use serde` if not already in scope (check existing `use` statements; `serde_json` is already a dep).

Switch `AskOpts.stream_sink` (around line 99):

```rust
    /// Optional sink: every staged event (`RetrievalDone`, `Token`,
    /// `Final`) is forwarded synchronously. A dropped receiver
    /// triggers cancel — see `RagPipeline::ask` for the break path.
    pub stream_sink: Option<std::sync::mpsc::Sender<StreamEvent>>,
```

- [ ] **Step 1.4: Run tests — verify pass**

```bash
cargo test -p kebab-rag --lib stream_event_serde_tests
```

Expected: 3 PASS.

The rest of the workspace will fail to compile because:
- `crates/kebab-rag/src/pipeline.rs::ask` uses `sink.send(t)` where `t: String`.
- `crates/kebab-tui/src/ask.rs` declares `mpsc::channel::<String>()` and `Receiver<String>`.
- `crates/kebab-app/...` exposes `AskOpts` with the old type.

That is **expected**. Tasks 2-5 fix the call sites.

- [ ] **Step 1.5: Commit**

```bash
git add crates/kebab-rag/src/pipeline.rs
git commit -m "$(cat <<'EOF'
feat(rag): StreamEvent enum + switch AskOpts.stream_sink (fb-33)

3-variant discriminated enum (RetrievalDone / Token / Final).
AskOpts.stream_sink now carries StreamEvent. Other crates fail
to compile until subsequent tasks adapt their call sites.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: Pipeline emits RetrievalDone + Token + Final + cancel branch

**Files:**
- Modify: `crates/kebab-rag/src/pipeline.rs`

- [ ] **Step 2.1: Write the failing test for ordering invariant**

Create `crates/kebab-rag/tests/streaming_events.rs`:

```rust
//! p9-fb-33: pipeline-level streaming behavior — order invariants,
//! cancel propagation, refusal flagging.

mod common;

use kebab_core::{Answer, FinishReason, RefusalReason, SearchMode, TokenChunk, TokenUsage};
use kebab_rag::{AskOpts, RagPipeline, StreamEvent};
use std::sync::mpsc;

#[test]
fn ask_emits_retrieval_then_tokens_then_final() {
    let env = common::RagEnv::new();
    env.seed_one_doc("a.md", "apples are red.");
    let (tx, rx) = mpsc::channel::<StreamEvent>();
    let opts = AskOpts {
        k: 3,
        explain: false,
        mode: SearchMode::Lexical,
        temperature: None,
        seed: None,
        stream_sink: Some(tx),
        history: vec![],
        conversation_id: None,
        turn_index: None,
    };
    let _ans = env.pipeline().ask("apples", opts).unwrap();
    let events: Vec<StreamEvent> = rx.iter().collect();

    // First event must be RetrievalDone.
    assert!(matches!(events.first(), Some(StreamEvent::RetrievalDone { .. })),
        "first event must be RetrievalDone, got {:?}", events.first());

    // Last event must be Final.
    assert!(matches!(events.last(), Some(StreamEvent::Final { .. })),
        "last event must be Final, got {:?}", events.last());

    // Everything in between is Token.
    for ev in &events[1..events.len() - 1] {
        assert!(matches!(ev, StreamEvent::Token { .. }),
            "middle events must be Token, got {:?}", ev);
    }
}
```

`common::RagEnv` and `seed_one_doc` already exist in `crates/kebab-rag/tests/common/mod.rs` (Task 7's `mk_hit_with_indexed_at` plus the existing `RagEnv` scaffold from earlier tests). Reuse them.

If the test scaffold's existing `MockRetriever` / `CountingLm` doesn't trigger the LLM-citation path naturally for the seeded text, adapt — the goal is just to drive a non-empty token stream past the score-gate. Look at existing `kebab-rag/tests/pipeline.rs` (`grounded_citations_inherit_indexed_at_and_stale_from_hit`) for a working setup.

- [ ] **Step 2.2: Run test — verify it fails**

```bash
cargo test -p kebab-rag --test streaming_events ask_emits_retrieval_then_tokens_then_final
```

Expected: FAIL — pipeline currently sends `String` and the test gets a `mpsc::SendError` on type mismatch (compile error) or `events` only contains tokens (no RetrievalDone, no Final).

- [ ] **Step 2.3: Add RetrievalDone emit after retrieval + stale-stamp**

In `crates/kebab-rag/src/pipeline.rs::ask`, immediately AFTER the staleness stamping loop (around line 205, after `for h in &mut hits { h.stale = ... }`):

```rust
        // p9-fb-33: emit retrieval_done as soon as the hit list is
        // ready (post stale-stamp so consumers see the same `stale`
        // values the App-level wire path emits). Cancel is best-effort
        // here — if the caller already dropped the receiver we just
        // skip and let the LLM-loop SendError handle it consistently.
        if let Some(sink) = &opts.stream_sink {
            let _ = sink.send(StreamEvent::RetrievalDone {
                hits: hits.clone(),
            });
        }
```

- [ ] **Step 2.4: Switch token send to StreamEvent::Token + add cancel branch**

Replace the existing token loop body (around lines 304-325). The current code is:

```rust
        for item in stream {
            let chunk = item.context("kb-rag: stream item")?;
            match chunk {
                TokenChunk::Token(t) => {
                    acc.push_str(&t);
                    if let Some(sink) = &opts.stream_sink {
                        let _ = sink.send(t);
                    }
                }
                TokenChunk::Done {
                    finish_reason: fr,
                    usage: u,
                } => {
                    finish_reason = fr;
                    usage = u;
                    break;
                }
            }
        }
```

Replace with:

```rust
        let mut cancelled = false;
        for item in stream {
            let chunk = item.context("kb-rag: stream item")?;
            match chunk {
                TokenChunk::Token(t) => {
                    acc.push_str(&t);
                    if let Some(sink) = &opts.stream_sink {
                        // p9-fb-33: SendError → caller dropped the
                        // receiver (probably a closed stdout downstream).
                        // Stop generation, mark the answer cancelled so
                        // the persistence path records refusal_reason =
                        // LlmStreamAborted.
                        if sink
                            .send(StreamEvent::Token {
                                delta: t,
                                turn_index: opts.turn_index,
                            })
                            .is_err()
                        {
                            cancelled = true;
                            break;
                        }
                    }
                }
                TokenChunk::Done {
                    finish_reason: fr,
                    usage: u,
                } => {
                    finish_reason = fr;
                    usage = u;
                    break;
                }
            }
        }
        if cancelled {
            finish_reason = FinishReason::Cancelled;
        }
```

`FinishReason::Cancelled` should already exist (it's used for `LlmStreamAborted` per the spec). If it doesn't:

```bash
grep -n "Cancelled\|FinishReason" crates/kebab-core/src/answer.rs crates/kebab-core/src/llm.rs 2>/dev/null
```

If absent in the existing enum, add it to the `FinishReason` enum in `kebab-core` (likely `crates/kebab-core/src/llm.rs`):

```rust
pub enum FinishReason {
    Stop,
    Length,
    Cancelled, // p9-fb-33
}
```

- [ ] **Step 2.5: Honor cancel in refusal logic + emit Final on success**

After the existing grounded/refusal computation block (around lines 348-359), prepend a cancel check:

```rust
        // p9-fb-33: cancel takes priority over LlmSelfJudge — the
        // caller bailed mid-stream, so the recorded reason should
        // reflect that, not "model didn't cite".
        let (grounded, refusal_reason) = if matches!(finish_reason, FinishReason::Cancelled) {
            (false, Some(RefusalReason::LlmStreamAborted))
        } else if grounded {
            (grounded, None)
        } else {
            (grounded, Some(RefusalReason::LlmSelfJudge))
        };
```

(The existing `let grounded = ...; let refusal_reason = ...` block becomes dead code — delete those two `let` bindings and replace with the tuple destructure above. Keep the existing `let cited_set` and downstream logic.)

After the `Answer { ... }` literal is built (around line 422), and BEFORE the persistence step (line 437), emit Final ONLY when the run wasn't cancelled:

```rust
        // p9-fb-33: emit final on the success path. On cancel we
        // skip Final — the receiver is gone and persistence still
        // records the partial answer below.
        if !cancelled
            && let Some(sink) = &opts.stream_sink
        {
            let _ = sink.send(StreamEvent::Final {
                answer: answer.clone(),
            });
        }
```

(`answer.clone()` is the price of streaming. Non-streaming callers pay nothing — `opts.stream_sink` is `None` and the `if let` short-circuits.)

- [ ] **Step 2.6: Run test — verify pass**

```bash
cargo test -p kebab-rag --test streaming_events ask_emits_retrieval_then_tokens_then_final
```

Expected: PASS.

- [ ] **Step 2.7: Add cancel-propagation test**

Append to `crates/kebab-rag/tests/streaming_events.rs`:

```rust
#[test]
fn ask_records_llm_stream_aborted_when_receiver_drops() {
    let env = common::RagEnv::new();
    env.seed_one_doc("a.md", "apples are red.");
    let (tx, rx) = mpsc::channel::<StreamEvent>();
    let opts = AskOpts {
        k: 3,
        explain: false,
        mode: SearchMode::Lexical,
        temperature: None,
        seed: None,
        stream_sink: Some(tx),
        history: vec![],
        conversation_id: None,
        turn_index: None,
    };
    // Drop the receiver immediately so the first Token send fails.
    drop(rx);
    let ans = env.pipeline().ask("apples", opts).unwrap();
    assert!(!ans.grounded);
    assert_eq!(ans.refusal_reason, Some(RefusalReason::LlmStreamAborted));
}
```

- [ ] **Step 2.8: Run cancel test — verify pass**

```bash
cargo test -p kebab-rag --test streaming_events ask_records_llm_stream_aborted
```

Expected: PASS.

- [ ] **Step 2.9: Run full kebab-rag suite**

```bash
cargo test -p kebab-rag
```

Expected: all PASS. Existing pipeline tests should still pass — they don't pass a `stream_sink`, so the new emit code is a no-op for them.

- [ ] **Step 2.10: Commit**

```bash
git add crates/kebab-rag/src/pipeline.rs crates/kebab-rag/tests/streaming_events.rs crates/kebab-core/src/llm.rs
git commit -m "$(cat <<'EOF'
feat(rag): pipeline emits StreamEvent + cancel on SendError (fb-33)

RetrievalDone after retrieve+stale-stamp, Token per LM chunk
(SendError → break, FinishReason::Cancelled, RefusalReason::
LlmStreamAborted), Final on success. answers row still persists
on cancel for audit. Adds FinishReason::Cancelled if absent.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: kebab-app re-exports + adapt TUI worker

**Files:**
- Modify: `crates/kebab-app/src/lib.rs`
- Modify: `crates/kebab-tui/src/app.rs`
- Modify: `crates/kebab-tui/src/ask.rs`

- [ ] **Step 3.1: Add `pub use` for StreamEvent in kebab-app**

In `crates/kebab-app/src/lib.rs`, find the existing `pub use kebab_rag::AskOpts` (or equivalent re-export) and append:

```rust
pub use kebab_rag::{AskOpts, StreamEvent};
```

If the existing line already covers `AskOpts`, just add `StreamEvent` to that brace list.

- [ ] **Step 3.2: Switch TUI Receiver type**

Edit `crates/kebab-tui/src/app.rs:217`:

```diff
-    pub rx: Option<std::sync::mpsc::Receiver<String>>,
+    pub rx: Option<std::sync::mpsc::Receiver<kebab_app::StreamEvent>>,
```

- [ ] **Step 3.3: Switch worker channel type**

Edit `crates/kebab-tui/src/ask.rs::spawn_ask_worker` (around line 486):

```diff
-    let (tx, rx) = mpsc::channel::<String>();
+    let (tx, rx) = mpsc::channel::<kebab_app::StreamEvent>();
```

- [ ] **Step 3.4: Update drain_stream to match Token only**

Edit `crates/kebab-tui/src/ask.rs::drain_stream` (around line 542):

```rust
pub(crate) fn drain_stream(state: &mut App) {
    let Some(s) = state.ask.as_mut() else { return };
    if let Some(rx) = &s.rx {
        for ev in rx.try_iter() {
            match ev {
                kebab_app::StreamEvent::Token { delta, .. } => {
                    s.partial.push_str(&delta);
                }
                // p9-fb-33: TUI ignores RetrievalDone (citation
                // panel renders after completion via `last_answer`)
                // and Final (the worker thread's join already
                // delivers the canonical Answer in poll_worker).
                kebab_app::StreamEvent::RetrievalDone { .. }
                | kebab_app::StreamEvent::Final { .. } => {}
            }
        }
    }
}
```

- [ ] **Step 3.5: Build the TUI crate**

```bash
cargo build -p kebab-tui
```

Expected: clean build. If there are leftover `Receiver<String>` references in test code, fix them — same `StreamEvent` swap.

- [ ] **Step 3.6: Run TUI test suite**

```bash
cargo test -p kebab-tui
```

Expected: all PASS. Existing snapshot/string assertion tests check rendered output (Q/A blocks, citations) — token concat behavior is unchanged, so output is byte-identical.

If a test directly constructs `mpsc::channel::<String>()` for `pub rx` (e.g. a unit test that injects fake tokens), it needs the same swap. Adjust each call site to send `StreamEvent::Token { delta: "...".into(), turn_index: None }` instead of bare strings.

- [ ] **Step 3.7: Commit**

```bash
git add crates/kebab-app/src/lib.rs crates/kebab-tui/
git commit -m "$(cat <<'EOF'
feat(tui): adapt ask worker to StreamEvent sink (fb-33)

Worker channel now carries kebab_app::StreamEvent. drain_stream
matches on Token { delta }; RetrievalDone and Final are ignored
(citations render from last_answer, Final is redundant with
worker join). app::AskState.rx type widened to match.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: Wire schema — `answer_event.v1`

**Files:**
- Create: `docs/wire-schema/v1/answer_event.schema.json`

- [ ] **Step 4.1: Write the schema file**

Create `docs/wire-schema/v1/answer_event.schema.json`:

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://kb.local/wire/v1/answer_event.schema.json",
  "title": "AnswerEvent v1",
  "description": "Streaming event emitted by `kebab ask --stream`. One event per line on stderr (ndjson). Discriminated by `kind`. Terminal: `final`. Final stdout line is `answer.v1` for backwards compat (see ingest_progress.v1 precedent).",
  "type": "object",
  "required": ["schema_version", "kind", "ts"],
  "properties": {
    "schema_version": { "const": "answer_event.v1" },
    "kind":           { "enum": ["retrieval_done", "token", "final"] },
    "ts":             { "type": "string", "format": "date-time" },
    "hits":           { "type": "array",  "description": "retrieval_done: search_hit.v1[]" },
    "delta":          { "type": "string", "description": "token: incremental string chunk" },
    "turn_index":     { "type": ["integer", "null"], "minimum": 0, "description": "token: matches Answer.turn_index" },
    "answer":         { "type": "object", "description": "final: complete answer.v1 payload" }
  }
}
```

- [ ] **Step 4.2: Verify the schema file is valid JSON**

```bash
python3 -c "import json; json.load(open('docs/wire-schema/v1/answer_event.schema.json'))"
```

Expected: silent success.

- [ ] **Step 4.3: Commit**

```bash
git add docs/wire-schema/v1/answer_event.schema.json
git commit -m "$(cat <<'EOF'
feat(wire): answer_event.v1 schema (fb-33)

Discriminated ndjson event for `kebab ask --stream`. Mirrors
the ingest_progress.v1 pattern (stderr stream + stdout final
answer.v1 for backwards compat).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: CLI `--stream` flag + wire helper + background-thread driver

**Files:**
- Modify: `crates/kebab-cli/src/wire.rs`
- Modify: `crates/kebab-cli/src/main.rs`

- [ ] **Step 5.1: Find the existing wire DTO pattern**

```bash
grep -n "tag_object\|wire_answer\|wire_search_hit\|schema_version" crates/kebab-cli/src/wire.rs | head -20
```

The existing pattern uses a `tag_object(value, schema_version)` helper. Our new helper follows the same shape.

- [ ] **Step 5.2: Add `wire_answer_event` helper**

Edit `crates/kebab-cli/src/wire.rs`. Append after the existing `wire_answer` function:

```rust
/// p9-fb-33: tag a `StreamEvent` as `answer_event.v1` ndjson. The
/// timestamp is added at emit time (caller fills `ts`), since the
/// pipeline doesn't carry one in the in-process enum.
pub fn wire_answer_event(ev: &kebab_app::StreamEvent, ts: time::OffsetDateTime) -> Value {
    let mut v = serde_json::to_value(ev).expect("StreamEvent serializes");
    let ts_str = ts
        .format(&time::format_description::well_known::Rfc3339)
        .expect("OffsetDateTime formats as RFC3339");
    if let Value::Object(ref mut map) = v {
        map.insert("ts".to_string(), Value::String(ts_str));
    }
    tag_object(v, "answer_event.v1")
}
```

`time` is already a dep on `kebab-cli` from fb-32. If not, add it to `[dependencies]`:

```bash
grep -n "^time" crates/kebab-cli/Cargo.toml
```

If absent: `time = { workspace = true, features = ["formatting", "macros"] }`.

- [ ] **Step 5.3: Add the `--stream` clap flag**

Edit `crates/kebab-cli/src/main.rs` `Cmd::Ask` variant struct definition:

```bash
grep -n "Ask {" crates/kebab-cli/src/main.rs | head -5
```

Find the `Ask { .. }` enum variant (the clap subcommand definition, with fields like `query`, `k`, `mode`, `explain`, etc.). Add:

```rust
        /// p9-fb-33: emit ndjson answer_event.v1 events on stderr
        /// while streaming. Final stdout line is the existing
        /// answer.v1. Off by default to preserve final-only behavior.
        #[arg(long)]
        stream: bool,
```

Update the `Cmd::Ask { ... }` destructure binding inside the match arm (around line 571 — `Cmd::Ask { query, k, mode, explain, ..., session }`) to include `stream`.

- [ ] **Step 5.4: Implement the stream branch**

Replace the existing `Cmd::Ask` match arm body. The current body (lines 571-630) has a single non-streaming path. Add a `--stream` branch:

```rust
        Cmd::Ask {
            query,
            k,
            mode,
            explain,
            temperature,
            seed,
            show_citations,
            hide_citations,
            session,
            stream,
        } => {
            let cfg = kebab_config::Config::load(cli.config.as_deref())?;
            if *stream {
                use std::sync::mpsc;
                let (tx, rx) = mpsc::channel::<kebab_app::StreamEvent>();
                let opts = kebab_app::AskOpts {
                    k: *k,
                    explain: *explain,
                    mode: (*mode).into(),
                    temperature: *temperature,
                    seed: *seed,
                    stream_sink: Some(tx),
                    history: Vec::new(),
                    conversation_id: None,
                    turn_index: None,
                };
                let cfg2 = cfg.clone();
                let q = query.clone();
                let session2 = session.clone();
                let handle = std::thread::spawn(move || -> anyhow::Result<kebab_core::Answer> {
                    match session2.as_deref() {
                        Some(sid) => kebab_app::ask_with_session_with_config(cfg2, sid, &q, opts),
                        None => kebab_app::ask_with_config(cfg2, &q, opts),
                    }
                });

                // Drain receiver, write ndjson to stderr until completion
                // or BrokenPipe. Drop rx on BrokenPipe so the worker's
                // send returns SendError and the pipeline cancels.
                let mut cancelled_pipe = false;
                {
                    let mut stderr = std::io::stderr().lock();
                    for ev in &rx {
                        let now = time::OffsetDateTime::now_utc();
                        let v = wire::wire_answer_event(&ev, now);
                        let line = serde_json::to_string(&v)?;
                        if let Err(e) = writeln!(stderr, "{line}") {
                            if e.kind() == std::io::ErrorKind::BrokenPipe {
                                cancelled_pipe = true;
                                break;
                            }
                            return Err(e.into());
                        }
                    }
                }
                if cancelled_pipe {
                    drop(rx); // signal to worker — next send returns SendError
                }

                let result = handle
                    .join()
                    .map_err(|_| anyhow::anyhow!("ask worker panicked"))?;
                let ans = result?;

                // Final stdout line — answer.v1 for backwards compat.
                // BrokenPipe on stdout is silent (caller already gone).
                let final_json = serde_json::to_string(&wire::wire_answer(&ans))?;
                let _ = writeln!(std::io::stdout().lock(), "{final_json}");

                if !ans.grounded {
                    return Err(RefusalSignal.into());
                }
                Ok(())
            } else {
                // Existing non-streaming path — unchanged from
                // lines 583-629 in the prior version.
                let opts = kebab_app::AskOpts {
                    k: *k,
                    explain: *explain,
                    mode: (*mode).into(),
                    temperature: *temperature,
                    seed: *seed,
                    stream_sink: None,
                    history: Vec::new(),
                    conversation_id: None,
                    turn_index: None,
                };
                let ans = match session.as_deref() {
                    Some(sid) => kebab_app::ask_with_session_with_config(cfg, sid, query, opts)?,
                    None => kebab_app::ask_with_config(cfg, query, opts)?,
                };
                if cli.json {
                    println!("{}", serde_json::to_string(&wire::wire_answer(&ans))?);
                } else {
                    println!("{}", ans.answer);
                    let print_citations = *show_citations && !*hide_citations;
                    if print_citations && !ans.citations.is_empty() {
                        use std::io::IsTerminal;
                        let color = std::io::stdout().is_terminal();
                        let mut out = std::io::stdout().lock();
                        render_ask_plain_citations(&mut out, &ans, color)?;
                    }
                }
                if !ans.grounded {
                    return Err(RefusalSignal.into());
                }
                Ok(())
            }
        }
```

`writeln!` on stderr's `MutexGuard` requires `std::io::Write` in scope — verify the existing imports include it (most CLI files do).

- [ ] **Step 5.5: Build the CLI**

```bash
cargo build -p kebab-cli
```

Expected: clean build. If `kebab_core::Answer` isn't in scope of the spawn closure return type, the inferred return is fine — the explicit `-> anyhow::Result<kebab_core::Answer>` annotation covers it. If `kebab_core` isn't a dep of `kebab-cli`, swap the annotation to whatever path resolves (`kebab_app::Answer` if it re-exports, or just elide with `-> anyhow::Result<_>`).

```bash
grep -n "^kebab-core\|^kebab_core" crates/kebab-cli/Cargo.toml
```

If `kebab-core` is missing, use `kebab_app::Answer`:

```bash
grep -n "pub use.*Answer" crates/kebab-app/src/lib.rs
```

If not re-exported, add `pub use kebab_core::Answer;` to `crates/kebab-app/src/lib.rs` near the existing `pub use kebab_rag::{AskOpts, StreamEvent};`.

- [ ] **Step 5.6: Smoke-test the CLI flag (skipped on no-Ollama)**

```bash
kebab --help 2>&1 | grep -A2 "ask"
kebab ask --help 2>&1 | grep -A1 stream
```

Expected: `--stream` appears in `ask` subcommand help.

- [ ] **Step 5.7: Commit**

```bash
git add crates/kebab-cli/src/wire.rs crates/kebab-cli/src/main.rs crates/kebab-cli/Cargo.toml crates/kebab-app/src/lib.rs Cargo.lock
git commit -m "$(cat <<'EOF'
feat(cli): kebab ask --stream emits ndjson on stderr (fb-33)

Background-thread driver runs ask_with_config; main thread
drains the receiver, serializes each StreamEvent to ndjson on
stderr. BrokenPipe → drop receiver → pipeline SendError →
cancel + LlmStreamAborted refusal. Final stdout line is the
existing answer.v1 (ingest_progress.v1 backwards-compat
pattern).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: CLI integration tests

**Files:**
- Create: `crates/kebab-cli/tests/wire_ask_stream.rs`
- Modify: `crates/kebab-cli/tests/common/mod.rs`

- [ ] **Step 6.1: Inspect existing common helpers**

```bash
sed -n '1,40p' crates/kebab-cli/tests/common/mod.rs
```

The existing `common::run_ask_lexical(env, query, json: bool)` (or equivalent) is the pattern. We need a `--stream` variant.

- [ ] **Step 6.2: Add `run_ask_stream` helper**

Append to `crates/kebab-cli/tests/common/mod.rs`:

```rust
/// p9-fb-33: invoke `kebab ask --stream`, capturing stdout + stderr.
/// Returns (stdout, stderr).
pub fn run_ask_stream(env: &TestEnv, query: &str) -> (String, String) {
    let exe = env!("CARGO_BIN_EXE_kebab");
    let out = std::process::Command::new(exe)
        .args(["--config", env.config_path(), "ask", "--stream", "--mode", "lexical", query])
        .output()
        .expect("kebab ask --stream");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}
```

Adapt to whatever helper signature `run_ask_lexical` uses — match the same idiom (e.g., if existing helpers take `&CliEnv` and return a struct with stdout/stderr, mirror that).

- [ ] **Step 6.3: Write the integration tests**

Create `crates/kebab-cli/tests/wire_ask_stream.rs`:

```rust
//! p9-fb-33: CLI streaming surface — stderr ndjson + stdout final answer.v1.

mod common;

use serde_json::Value;

#[test]
#[ignore = "requires real Ollama (matches sibling ask integration tests)"]
fn stream_emits_ndjson_events_on_stderr() {
    let env = common::CliEnv::new_with_llm_model("gemma4:e4b");
    common::ingest(&env, "a.md", "# T\n\nrust ownership is a memory model.\n");
    let (stdout, stderr) = common::run_ask_stream(&env, "what is rust ownership");

    // stderr: every line should parse as JSON with schema_version
    // == "answer_event.v1" and a recognized kind.
    let mut kinds: Vec<String> = vec![];
    for line in stderr.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let v: Value = serde_json::from_str(line)
            .unwrap_or_else(|e| panic!("non-JSON stderr line: {line:?}: {e}"));
        assert_eq!(v["schema_version"], "answer_event.v1");
        let kind = v["kind"].as_str().expect("kind").to_string();
        assert!(
            matches!(kind.as_str(), "retrieval_done" | "token" | "final"),
            "unexpected kind: {kind}"
        );
        assert!(v["ts"].is_string(), "ts must be RFC3339 string");
        kinds.push(kind);
    }

    // First event must be retrieval_done. Last must be final.
    assert_eq!(kinds.first().map(String::as_str), Some("retrieval_done"));
    assert_eq!(kinds.last().map(String::as_str), Some("final"));

    // stdout: last line is answer.v1.
    let final_line = stdout.lines().last().expect("stdout has at least one line");
    let answer: Value = serde_json::from_str(final_line).expect("stdout final = answer.v1");
    assert_eq!(answer["schema_version"], "answer.v1");
}

#[test]
#[ignore = "requires real Ollama"]
fn non_stream_path_unchanged() {
    let env = common::CliEnv::new_with_llm_model("gemma4:e4b");
    common::ingest(&env, "a.md", "# T\n\nrust ownership is a memory model.\n");
    let stdout = common::run_ask_json(&env, "what is rust ownership"); // existing helper
    let v: Value = serde_json::from_str(&stdout).expect("answer.v1");
    assert_eq!(v["schema_version"], "answer.v1");
}
```

`common::run_ask_json` already exists from fb-32's wire test scaffold. If the parameter / return shape differs from what's shown, adjust.

- [ ] **Step 6.4: Run new tests (with Ollama available)**

```bash
cargo test -p kebab-cli --test wire_ask_stream -- --ignored
```

Expected: 2 PASS (when Ollama is running locally and `gemma4:e4b` is pulled). Without Ollama, the tests stay ignored — sibling fb-32 integration tests follow the same gate.

- [ ] **Step 6.5: Verify the non-ignored CLI suite still passes**

```bash
cargo test -p kebab-cli
```

Expected: all PASS, ignored count includes the two new tests.

- [ ] **Step 6.6: Commit**

```bash
git add crates/kebab-cli/tests/
git commit -m "$(cat <<'EOF'
test(cli): wire_ask_stream — stderr ndjson + stdout final answer.v1 (fb-33)

Two Ollama-gated integration tests verifying:
- stderr lines parse as answer_event.v1, first=retrieval_done,
  last=final, all carry RFC3339 ts.
- stdout final line is answer.v1 (backwards compat).
- non-stream path (--json without --stream) unchanged.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: BrokenPipe cancel integration test

**Files:**
- Modify: `crates/kebab-cli/tests/wire_ask_stream.rs`

The shell-level `head -c 1` simulation is brittle in cargo test. Use a more direct test: pipe stderr through a writer that fails after N bytes.

- [ ] **Step 7.1: Add the cancel test (Ollama-gated)**

Append to `crates/kebab-cli/tests/wire_ask_stream.rs`:

```rust
#[test]
#[ignore = "requires real Ollama + writes to a closed pipe"]
fn stream_cancels_when_stderr_closes() {
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};

    let env = common::CliEnv::new_with_llm_model("gemma4:e4b");
    common::ingest(&env, "a.md", "# T\n\nrust ownership is a memory model. it tracks lifetimes.\n");

    // Spawn `kebab ask --stream`. Read stderr line-by-line, then
    // immediately drop the stderr reader after the first line (which
    // is retrieval_done). Pipeline should detect SendError and break.
    let exe = env!("CARGO_BIN_EXE_kebab");
    let mut child = Command::new(exe)
        .args([
            "--config", env.config_path(),
            "ask", "--stream", "--mode", "lexical",
            "tell me about rust ownership",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn kebab");

    {
        let stderr = child.stderr.take().expect("stderr piped");
        let mut reader = BufReader::new(stderr);
        let mut first = String::new();
        reader.read_line(&mut first).expect("read first stderr line");
        assert!(
            first.contains("\"kind\":\"retrieval_done\""),
            "first event must be retrieval_done, got {first:?}"
        );
        // Drop the reader → child's stderr write end will see SIGPIPE
        // on the next write → main thread gets BrokenPipe → drops rx →
        // worker's pipeline.send returns SendError → cancel.
    }

    // Process should still terminate cleanly within reasonable time.
    let status = child
        .wait()
        .expect("child completes after cancel");
    // Refusal exits with code 1 (RefusalSignal). Don't assert exact
    // code — different OSes report SIGPIPE differently. Assert just
    // that the process didn't hang.
    let _ = status;
}
```

This is the closest portable approximation of the BrokenPipe scenario without spawning a subprocess that pipes through `head`. The test verifies the process terminates instead of hanging — that's the key invariant.

- [ ] **Step 7.2: Run the cancel test (with Ollama)**

```bash
cargo test -p kebab-cli --test wire_ask_stream stream_cancels_when_stderr_closes -- --ignored
```

Expected: PASS — process exits within `child.wait()` instead of blocking.

- [ ] **Step 7.3: Commit**

```bash
git add crates/kebab-cli/tests/wire_ask_stream.rs
git commit -m "$(cat <<'EOF'
test(cli): BrokenPipe stderr → ask --stream terminates cleanly (fb-33)

Spawn the binary, read first stderr line (retrieval_done), drop
the reader. Pipeline's next Token send returns SendError, cancel
branch fires, child.wait() returns instead of blocking forever.
Ollama-gated.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 8: Workspace test + clippy gate

- [ ] **Step 8.1: Run full workspace test**

```bash
cargo test --workspace --no-fail-fast -j 1 2>&1 | tail -50
```

Expected: all PASS. Snapshot tests in other crates should be unaffected — `StreamEvent` is internal API and the wire emit happens only on `--stream`.

- [ ] **Step 8.2: Clippy gate**

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

Expected: clean. Common new warnings to watch for:
- `clippy::large_enum_variant` on `StreamEvent` (Final::answer is large). If reported, wrap in `Box`: `Final { answer: Box<Answer> }`. Update emit + match sites.
- `clippy::needless_borrow` on `&rx` iteration — adapt as flagged.

- [ ] **Step 8.3: Commit if clippy required fixes**

```bash
git add -A
git commit -m "chore: clippy fixes for fb-33"
```

(Skip this commit if no fixes were needed.)

---

## Task 9: Documentation updates

**Files:**
- Modify: `README.md`
- Modify: `docs/SMOKE.md`
- Modify: `tasks/p9/p9-fb-33-streaming-ask.md`
- Modify: `tasks/INDEX.md`
- Modify: `integrations/claude-code/kebab/SKILL.md`

- [ ] **Step 9.1: README — Quick start mention**

Find the existing `## 명령` table or quick-start block:

```bash
grep -n "kebab ask\|## 명령\|Quick start" README.md | head -10
```

Add a row or paragraph noting `--stream`:

```markdown
| `kebab ask "..." --stream` | RAG 답변을 ndjson event 로 stderr 에 stream — agent token 즉시 소비 (fb-33) |
```

Or, if README format prefers prose, append one short line under the existing `kebab ask` description.

- [ ] **Step 9.2: SMOKE.md — walkthrough**

After the existing ask section, append:

```markdown
### Streaming ask (fb-33)

```bash
kebab ask "what is rust ownership" --stream 2> events.ndjson > final.json
```

stderr 의 events.ndjson 은 한 줄 = 한 event 의 ndjson — `retrieval_done` 한 번, `token` 여러 번, `final` 한 번. final.json 은 기존 `answer.v1` 그대로.

agent 가 `head -c 1` 로 stderr 를 닫으면 pipeline 이 LLM stream 을 즉시 중단하고 `RefusalReason::LlmStreamAborted` 로 partial answer 를 `answers` 테이블에 기록한다.
```

- [ ] **Step 9.3: Task spec status flip**

Edit `tasks/p9/p9-fb-33-streaming-ask.md`:

```diff
 ---
-status: open
+status: completed
 target_version: 0.5.0
```

Replace the `> ⏳ **백로그 only — 미구현.**` block (around line 14):

```markdown
상세 설계: `docs/superpowers/specs/2026-05-09-p9-fb-33-streaming-ask-design.md`.
구현 계획: `docs/superpowers/plans/2026-05-09-p9-fb-33-streaming-ask.md`.
```

- [ ] **Step 9.4: tasks/INDEX.md — fb-33 row**

```diff
-    - [p9-fb-33 streaming ask (ndjson delta)](p9/p9-fb-33-streaming-ask.md) — ⏳ 미구현, brainstorm 필요
+    - [p9-fb-33 streaming ask (ndjson delta)](p9/p9-fb-33-streaming-ask.md) — ✅ 머지 + v0.5.0 cut 후보 (2026-05-09)
```

- [ ] **Step 9.5: Skill — CLI fallback note**

Edit `integrations/claude-code/kebab/SKILL.md`. Find the "CLI fallback" or equivalent section. Append:

```markdown
- `kebab ask --stream`: ndjson `answer_event.v1` events on stderr (`retrieval_done` → `token`* → `final`), plus the existing `answer.v1` as the final stdout line. Use when you need progressive token consumption; otherwise the default non-streaming path is simpler.
```

- [ ] **Step 9.6: Commit docs**

```bash
git add README.md docs/SMOKE.md tasks/p9/p9-fb-33-streaming-ask.md tasks/INDEX.md integrations/claude-code/kebab/SKILL.md
git commit -m "$(cat <<'EOF'
docs(fb-33): README + SMOKE + INDEX + skill notes

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 10: Smoke + push + PR

- [ ] **Step 10.1: Manual smoke**

```bash
cd /tmp/kebab-smoke   # the existing SMOKE.md scratch dir
~/Workspace/projects/kebab/target/release/kebab --config /tmp/kebab-smoke/config.toml ingest
~/Workspace/projects/kebab/target/release/kebab --config /tmp/kebab-smoke/config.toml ask "test" --stream 2>events.ndjson >final.json
head -1 events.ndjson
tail -1 events.ndjson
cat final.json | jq .schema_version
```

Expected:
- first event line includes `"kind":"retrieval_done"`
- last event line includes `"kind":"final"`
- `final.json` contains `"schema_version":"answer.v1"`

- [ ] **Step 10.2: Final workspace test**

```bash
cd ~/Workspace/projects/kebab
cargo test --workspace --no-fail-fast -j 1
```

Expected: all green.

- [ ] **Step 10.3: Push branch**

```bash
git push -u origin feat/fb-33-streaming-ask
```

- [ ] **Step 10.4: Open PR via gitea-pr**

Build the PR body file at `/tmp/fb33-pr-body.md`:

```markdown
## Summary

- adds `kebab ask --stream` emitting `answer_event.v1` ndjson events on stderr (`retrieval_done` → `token`* → `final`), final stdout line stays `answer.v1` for backwards compat
- internal API: `AskOpts.stream_sink` now carries discriminated `StreamEvent` instead of bare `String`; TUI worker adapted
- cancel: stdout/stderr close → BrokenPipe → drop receiver → pipeline `SendError` → LLM loop break + `RefusalReason::LlmStreamAborted`
- MCP `kebab__ask` streaming deferred to v0.5+ (rmcp progress notifications need verification first)

## Test plan

- [x] `cargo test --workspace --no-fail-fast -j 1` — green
- [x] `cargo clippy --workspace --all-targets -- -D warnings` — clean
- [x] new tests: pipeline order invariant + cancel propagation (kebab-rag), `wire_ask_stream` ndjson shape + stdout final + BrokenPipe cancel (kebab-cli, Ollama-gated)
- [x] manual smoke per `docs/SMOKE.md` "Streaming ask" walkthrough

## Architectural notes

- `RetrievalDone` includes the retrieval-stale-stamp result so consumers see the same `stale` values the App-level wire path emits.
- `Final` event mirrors the canonical Answer; TUI worker ignores it (worker join already delivers Answer).
- `StreamEvent` lives in `kebab-rag` to keep the type adjacent to the pipeline that emits it; `kebab-app` re-exports for downstream consumers.

## Files of interest

- spec: `docs/superpowers/specs/2026-05-09-p9-fb-33-streaming-ask-design.md`
- plan: `docs/superpowers/plans/2026-05-09-p9-fb-33-streaming-ask.md`
- pipeline: `crates/kebab-rag/src/pipeline.rs` (StreamEvent + emit + cancel)
- CLI: `crates/kebab-cli/src/main.rs` (Cmd::Ask --stream branch), `crates/kebab-cli/src/wire.rs` (wire_answer_event)
- wire: `docs/wire-schema/v1/answer_event.schema.json`
- TUI: `crates/kebab-tui/src/ask.rs` (drain_stream match)
```

Then open:

```bash
/Users/user/.claude/skills/gitea-ops/bin/gitea-pr \
  --title "feat(fb-33): streaming ask (ndjson delta)" \
  --body "$(cat /tmp/fb33-pr-body.md)" \
  --head feat/fb-33-streaming-ask \
  --base main
```

Capture the returned PR URL.

---

## Self-review checklist

- **Spec coverage:**
  - §Behavior contract / event taxonomy → Tasks 1, 2 (StreamEvent + emit positions)
  - §CLI flag → Task 5 (`--stream`)
  - §Output stream (stderr ndjson + stdout final) → Task 5 + Task 6 tests
  - §Cancel semantics → Task 2 (SendError branch) + Task 7 (BrokenPipe integration test)
  - §Wire schema → Task 4 (`answer_event.schema.json`)
  - §Domain API change → Tasks 1, 3 (AskOpts + TUI adapt)
  - §Components (kebab-rag/app/cli/tui) → Tasks 1-5
  - §Test plan → Tasks 2, 6, 7 cover unit (serde + ordering + cancel) + integration (CLI ndjson, BrokenPipe)
  - §Documentation → Task 9
  - §Risks (BrokenPipe vs IoError, ndjson line-unit, partial markdown) → addressed in Task 5 (only `BrokenPipe` triggers cancel; other IoError fatal)

- **Placeholder scan:**
  - "adapt to existing scaffold" appears in Tasks 2, 6 — these instruct mirroring of existing test infrastructure (RagEnv, CliEnv) rather than inventing new helpers.
  - "if absent, add it" in Task 5 (Cargo.toml `time` dep, kebab-core re-export) — concrete fallback paths spelled out, not deferred.
  - No TODO / "fill in" / "later" remaining.

- **Type consistency:**
  - `StreamEvent` enum variants identical across Tasks 1, 2, 3, 5 (RetrievalDone {hits}, Token {delta, turn_index}, Final {answer}).
  - `AskOpts.stream_sink: Option<mpsc::Sender<StreamEvent>>` consistent.
  - `wire_answer_event(&StreamEvent, OffsetDateTime) -> Value` signature stable.
  - `FinishReason::Cancelled` used consistently (Task 2 step 2.4 + 2.5).
  - `RefusalReason::LlmStreamAborted` matches existing variant (already in `kebab-core`).

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-05-09-p9-fb-33-streaming-ask.md`. Two execution options:

**1. Subagent-Driven (recommended)** — fresh subagent per task, review between tasks, fast iteration.

**2. Inline Execution** — execute tasks in this session using executing-plans, batch execution with checkpoints.

Which approach?
