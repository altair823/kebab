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
//! 1. `ask_tool_routes_multi_hop_true_to_decompose_first` — probe-passing
//!    fixture, multi_hop=true → decompose (LLM error), single_pass → retrieval
//!    NoChunks. Wire shapes diverge: `error.v1` vs `answer.v1`.
//! 2. `ask_tool_multi_hop_short_circuits_when_probe_empty` — empty KB,
//!    multi_hop=true → probe-empty short-circuit, NoChunks refusal byte-
//!    identical to single-pass. PR-7 의 intent 가 MCP layer 에 pin.
//!
//! A live-Ollama end-to-end multi-hop pin lands in a follow-up
//! `#[ignore]` test (same pattern as `wire_ask_stale.rs`).

use kebab_config::Config;
use kebab_core::SourceScope;
use kebab_mcp::{KebabAppState, KebabHandler};
use rmcp::model::RawContent;

fn minimal_config(data_dir: &std::path::Path, workspace_root: &std::path::Path) -> Config {
    let mut cfg = Config::defaults();
    cfg.storage.data_dir = data_dir.to_string_lossy().into_owned();
    cfg.storage.model_dir = data_dir.join("models").to_string_lossy().into_owned();
    cfg.workspace.root = workspace_root.to_string_lossy().into_owned();
    cfg.workspace.exclude.clear();
    cfg.models.embedding.provider = "none".to_string();
    cfg.models.embedding.dimensions = 0;
    // Force the LLM endpoint to a known-unreachable port so this test
    // is robust against whether a real Ollama happens to be running
    // on 127.0.0.1:11434 (the developer's box; CI; etc.). The
    // `request_timeout_secs = 5` gives slow CI / Docker network stacks
    // enough headroom that *some* error fires deterministically — the
    // dispatch contract below only cares that `is_error` flipped, not
    // which specific error code surfaced.
    cfg.models.llm.endpoint = "http://127.0.0.1:1".to_string();
    cfg.models.llm.request_timeout_secs = 5;
    // Bypass the second probe gate (`top_score < score_gate`) so that the
    // probe-pass path in `RagPipeline::ask_multi_hop` (PR-7 v0.18 dogfood
    // fix) is reachable from a tiny lexical fixture whose FTS5 fusion
    // score may sit below the production default (0.30). The probe's
    // first gate (`probe_hits.is_empty()`) is unaffected — the empty-KB
    // short-circuit test below still exercises it. Production default
    // 0.30 remains untouched (test config isolation only).
    cfg.rag.score_gate = 0.0;
    cfg
}

/// The dispatch contract (post-PR-7 probe-first): with a probe-passing
/// fixture, single-pass `ask` retrieves first and returns a NoChunks
/// refusal Answer for an unrelated query (`grounded=false`,
/// `isError=false`). Multi-hop's probe passes on the same fixture →
/// decompose runs → unreachable LLM yields `error.v1` with
/// `code=model_unreachable` (`isError=true`). The divergence confirms
/// the `multi_hop` arg actually rerouted the dispatch *after* the
/// probe gate.
#[tokio::test]
async fn ask_tool_routes_multi_hop_true_to_decompose_first() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let workspace_root = dir.path().join("notes");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(&workspace_root).unwrap();

    // Lexical-friendly fixture so the multi-hop probe (PR-7 v0.18 dogfood
    // fix) returns at least one hit and we exercise the post-probe
    // decompose path. `build_match_string` rewrites the query
    // `"compound about X and Y"` into
    // `text : (("compound about X and Y") OR ("compound" "about" "and"))`
    // — the token_and branch is FTS5 implicit-AND, so the fixture body
    // MUST keep all three tokens (`compound`, `about`, `and`). Do not
    // collapse to a single-token body or the probe short-circuits to
    // NoChunks and the dispatch divergence below disappears.
    let fixture = workspace_root.join("note.md");
    std::fs::write(
        &fixture,
        "# Compound topic\n\nThis note is about a compound containing X and Y in detail.\n",
    )
    .unwrap();

    let cfg = minimal_config(&data_dir, &workspace_root);

    let scope = SourceScope {
        root: workspace_root.clone(),
        include: vec![],
        exclude: vec![],
    };
    let _ = kebab_app::ingest_with_config(cfg.clone(), scope, false).unwrap();

    let state = KebabAppState::new(cfg, None);
    let handler = KebabHandler::new(state);

    // Multi-hop branch — decompose runs first, hits the unreachable
    // endpoint, MCP wraps as error.v1.
    let state_mh = handler.state().clone();
    let mh = tokio::task::spawn_blocking(move || {
        kebab_mcp::tools::ask::handle(
            &state_mh,
            kebab_mcp::tools::ask::AskInput {
                query: "compound about X and Y".to_string(),
                session_id: None,
                mode: Some("lexical".to_string()),
                multi_hop: Some(true),
            },
        )
    })
    .await
    .unwrap();
    assert!(
        mh.is_error.unwrap_or(false),
        "multi_hop=true must reach the LLM (decompose first) — got {mh:?}"
    );
    let mh_text = match &mh.content.first().unwrap().raw {
        RawContent::Text(t) => t.text.clone(),
        other => panic!("expected text, got {other:?}"),
    };
    let mh_v: serde_json::Value = serde_json::from_str(&mh_text).unwrap();
    assert_eq!(mh_v["schema_version"], "error.v1");
    // The dispatch contract is "multi-hop's probe passed, then decompose
    // tried to talk to the LLM and failed" — i.e. `is_error` fires
    // because, *after* the PR-7 probe gate, decompose attempted an LLM
    // call against the unreachable endpoint. Which *specific* error code
    // lands (`model_unreachable` on fast ECONNREFUSED hosts, `timeout`
    // on slow connect-timeout stacks, etc.) is implementation detail of
    // the host TCP/HTTP path; pinning it here would just produce flakes
    // on slow CI.

    // Single-pass branch — empty KB short-circuits at retrieve, no LLM
    // call happens, refusal Answer comes back as isError=false.
    let state_sp = handler.state().clone();
    let sp = tokio::task::spawn_blocking(move || {
        kebab_mcp::tools::ask::handle(
            &state_sp,
            kebab_mcp::tools::ask::AskInput {
                query: "anything".to_string(),
                session_id: None,
                mode: Some("lexical".to_string()),
                multi_hop: Some(false),
            },
        )
    })
    .await
    .unwrap();
    assert!(
        !sp.is_error.unwrap_or(false),
        "single-pass empty-KB refusal must NOT be isError — got {sp:?}"
    );
    let sp_text = match &sp.content.first().unwrap().raw {
        RawContent::Text(t) => t.text.clone(),
        other => panic!("expected text, got {other:?}"),
    };
    let sp_v: serde_json::Value = serde_json::from_str(&sp_text).unwrap();
    assert_eq!(sp_v["schema_version"], "answer.v1");
    assert_eq!(sp_v["grounded"], false);
}

/// PR-7 의 probe-empty short-circuit 이 MCP-layer 의 wire shape 로 pin.
/// 빈 KB + multi_hop=true → `RagPipeline::ask_multi_hop` 의 첫 probe
/// gate (`probe_hits.is_empty()`) 에 막혀 `refuse_no_chunks` 가 single-pass
/// 와 byte-identical 한 `answer.v1` refusal envelope 을 반환한다.
/// kebab-rag::multi_hop_empty_probe_pool_refuses_before_any_llm_call 가
/// RAG-layer 만 pin — MCP-layer 의 wire shape 는 본 test 만이 안전망.
#[tokio::test]
async fn ask_tool_multi_hop_short_circuits_when_probe_empty() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let workspace_root = dir.path().join("notes");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(&workspace_root).unwrap();

    let cfg = minimal_config(&data_dir, &workspace_root);
    let scope = SourceScope {
        root: workspace_root.clone(),
        include: vec![],
        exclude: vec![],
    };
    let _ = kebab_app::ingest_with_config(cfg.clone(), scope, false).unwrap();

    let state = KebabAppState::new(cfg.clone(), None);
    let handler = KebabHandler::new(state);
    let state_mh = handler.state().clone();
    let mh = tokio::task::spawn_blocking(move || {
        kebab_mcp::tools::ask::handle(
            &state_mh,
            kebab_mcp::tools::ask::AskInput {
                query: "compound about X and Y".to_string(),
                session_id: None,
                mode: Some("lexical".to_string()),
                multi_hop: Some(true),
            },
        )
    })
    .await
    .unwrap();

    assert_eq!(
        mh.is_error,
        Some(false),
        "probe-empty short-circuit must yield refusal envelope, not error.v1 — got {mh:?}"
    );
    let mh_text = match &mh.content.first().unwrap().raw {
        RawContent::Text(t) => t.text.clone(),
        other => panic!("expected text content, got {other:?}"),
    };
    let body: serde_json::Value = serde_json::from_str(&mh_text).unwrap();
    assert_eq!(body["schema_version"], "answer.v1");
    assert_eq!(body["refusal_reason"], "no_chunks");
}

/// AskInput's JSON-schema (rendered for tools/list) advertises the
/// new `multi_hop` field. Pins agent / MCP host capability discovery
/// against accidental schema-rename or omission.
#[test]
fn ask_input_schema_advertises_multi_hop_field() {
    let schema = schemars::schema_for!(kebab_mcp::tools::ask::AskInput);
    let v = serde_json::to_value(&schema).unwrap();
    let props = v
        .get("properties")
        .and_then(|p| p.as_object())
        .expect("AskInput schema must declare properties");
    assert!(
        props.contains_key("multi_hop"),
        "AskInput.multi_hop must surface in the JsonSchema — got keys: {:?}",
        props.keys().collect::<Vec<_>>()
    );
}
