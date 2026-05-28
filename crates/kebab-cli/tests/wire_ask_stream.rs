//! p9-fb-33: CLI streaming surface — stderr ndjson `answer_event.v1`
//! events while the answer streams; final stdout line is the existing
//! `answer.v1` (backwards compat with the non-`--stream` path).
//!
//! These end-to-end checks exercise `kebab ask --stream`, which
//! requires a real Ollama on `127.0.0.1:11434` (same constraint as
//! `wire_ask_stale.rs` + `kebab-app/tests/ask_smoke.rs`). All three
//! tests are therefore `#[ignore]` by default — run with
//! `cargo test -p kebab-cli --test wire_ask_stream -- --ignored`
//! against a live Ollama with `gemma4:e4b` pulled.
//!
//! The `BrokenPipe → cancel` test (Task 7 of the fb-33 plan) verifies
//! that closing the stderr reader propagates SendError through the
//! pipeline so the child terminates instead of hanging. That's the
//! main thing the integration test layer can prove that unit tests
//! can't — pipeline cancel is a cross-process concern.
//!
//! Shared TempDir / ingest helpers live in `tests/common/mod.rs`.

mod common;

use std::fs;
use std::path::Path;

use serde_json::Value;

/// Drop `[rag].score_gate` to ~0 in the test config so the
/// score-gate refusal path doesn't short-circuit the LLM call.
/// Lexical retrieval against a one-doc corpus produces tiny fusion
/// scores (well below the default 0.30 gate); the pipeline would
/// take the `refuse_score_gate` early-return — which does not emit
/// a `Final` event — making the streaming-event ordering assertion
/// vacuous. Lower the gate so the LLM actually runs.
fn relax_score_gate(cfg: &Path) {
    let body = fs::read_to_string(cfg).expect("read config.toml");
    let body = body.replace("score_gate = 0.30", "score_gate = 0.0");
    fs::write(cfg, body).expect("write relaxed config.toml");
}

#[test]
#[ignore = "requires real Ollama on 127.0.0.1:11434"]
fn stream_emits_ndjson_events_on_stderr() {
    let dir = tempfile::tempdir().unwrap();
    let (cfg, workspace, _data) = common::write_config_with_llm_model(dir.path(), 30, "gemma4:e4b");
    relax_score_gate(&cfg);
    fs::write(
        workspace.join("a.md"),
        "# T\n\nrust ownership is a memory model.\n",
    )
    .unwrap();
    common::ingest(&cfg, &workspace);

    let (stdout, stderr) = common::run_ask_stream(&cfg, "ownership");

    // stderr: every non-empty line should parse as JSON with
    // schema_version == "answer_event.v1" and a recognized kind.
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
    // Note: this test only exercises the LLM-running path which always
    // closes with `final`. score-gate / no-chunks refusal paths emit
    // only `retrieval_done` and skip `final` — that's why the test uses
    // `relax_score_gate()` above to force the LLM path. See
    // `stream_score_gate_refusal_emits_only_retrieval_done` for the
    // refusal-path coverage.
    assert_eq!(
        kinds.first().map(String::as_str),
        Some("retrieval_done"),
        "first event must be retrieval_done, all kinds: {kinds:?}"
    );
    assert_eq!(
        kinds.last().map(String::as_str),
        Some("final"),
        "last event must be final, all kinds: {kinds:?}"
    );

    // stdout: last line is answer.v1 (backwards compat with the
    // non-streaming path — same wire shape, just emitted after the
    // ndjson event stream rather than instead of it).
    let final_line = stdout.lines().last().expect("stdout has at least one line");
    let answer: Value = serde_json::from_str(final_line).expect("stdout final line = answer.v1");
    assert_eq!(answer["schema_version"], "answer.v1");
}

#[test]
#[ignore = "requires real Ollama on 127.0.0.1:11434"]
fn non_stream_path_unchanged() {
    // Verify that the non-streaming JSON path (no `--stream`) still
    // emits a single `answer.v1` line on stdout — fb-33 must not
    // perturb the existing wire surface.
    let dir = tempfile::tempdir().unwrap();
    let (cfg, workspace, _data) = common::write_config_with_llm_model(dir.path(), 30, "gemma4:e4b");
    relax_score_gate(&cfg);
    fs::write(
        workspace.join("a.md"),
        "# T\n\nrust ownership is a memory model.\n",
    )
    .unwrap();
    common::ingest(&cfg, &workspace);

    let stdout = common::run_ask_json(&cfg, "ownership");
    let v: Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("expected answer.v1, got {stdout:?}: {e}"));
    assert_eq!(v["schema_version"], "answer.v1");
}

// p9-fb-33 (Task 7): BrokenPipe → cancel propagation. Spawn the
// binary, read the first stderr line (retrieval_done), drop the
// reader. The pipeline's next `Token` send returns SendError, the
// cancel branch fires, child.wait() returns instead of blocking
// forever. The key invariant is *liveness* — that `wait()` returns
// in bounded time. Don't assert exit code: refusal is exit 1, but
// the child may also exit 0 if the LLM happened to finish before
// cancel propagated.
#[test]
#[ignore = "requires real Ollama on 127.0.0.1:11434 + writes to a closed pipe"]
fn stream_cancels_when_stderr_closes() {
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};

    let dir = tempfile::tempdir().unwrap();
    let (cfg, workspace, _data) = common::write_config_with_llm_model(dir.path(), 30, "gemma4:e4b");
    relax_score_gate(&cfg);
    fs::write(
        workspace.join("a.md"),
        "# T\n\nrust ownership is a memory model. it tracks lifetimes.\n",
    )
    .unwrap();
    common::ingest(&cfg, &workspace);

    let bin = env!("CARGO_BIN_EXE_kebab");
    let mut child = Command::new(bin)
        .args([
            "--config",
            cfg.to_str().unwrap(),
            "ask",
            "--stream",
            "--mode",
            "lexical",
            "ownership",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn kebab");

    {
        let stderr = child.stderr.take().expect("stderr piped");
        let mut reader = BufReader::new(stderr);
        let mut first = String::new();
        reader
            .read_line(&mut first)
            .expect("read first stderr line");
        assert!(
            first.contains("\"kind\":\"retrieval_done\""),
            "first event must be retrieval_done, got {first:?}"
        );
        // Drop the reader → child's stderr write end will see
        // BrokenPipe on the next write → main thread drops rx →
        // worker's pipeline.send returns SendError → cancel.
    }

    let status = child.wait().expect("child completes after cancel");
    // Don't assert specific exit code — refusal is exit 1, but child
    // may also exit 0 if the LLM finished before cancel propagated.
    // The load-bearing assertion is that wait() returned at all.
    let _ = status;
}

// p9-fb-33 (PR #124 round 1, item 4): score-gate refusal path —
// thin doc + unrelated query trips the default 0.30 score gate
// before the LLM runs. The pipeline emits only `retrieval_done`
// on stderr (no `token`, no `final`); stdout still carries the
// canonical `answer.v1` with `grounded=false`.
#[test]
#[ignore = "requires real Ollama on 127.0.0.1:11434"]
fn stream_score_gate_refusal_emits_only_retrieval_done() {
    let dir = tempfile::tempdir().unwrap();
    let (cfg, workspace, _data) = common::write_config_with_llm_model(dir.path(), 30, "gemma4:e4b");
    // Intentionally NO relax_score_gate — keep the default 0.30
    // so the thin-doc + unrelated-query combo trips refusal.
    fs::write(workspace.join("a.md"), "# Title\n\nrust is a language.\n").unwrap();
    common::ingest(&cfg, &workspace);

    let (stdout, stderr) =
        common::run_ask_stream(&cfg, "completely unrelated topic about cooking pasta");

    let kinds: Vec<String> = stderr
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter_map(|v| v["kind"].as_str().map(String::from))
        .collect();

    // Refusal path: only retrieval_done, no token, no final.
    assert!(
        kinds.iter().all(|k| k == "retrieval_done"),
        "refusal path must emit only retrieval_done, got {kinds:?}"
    );
    assert!(
        !kinds.is_empty(),
        "expected at least one retrieval_done event, got empty stderr"
    );

    // Stdout still has answer.v1 with grounded=false.
    let final_line = stdout.lines().last().expect("stdout has at least one line");
    let answer: Value = serde_json::from_str(final_line).expect("answer.v1");
    assert_eq!(answer["schema_version"], "answer.v1");
    assert_eq!(answer["grounded"], false);
}
