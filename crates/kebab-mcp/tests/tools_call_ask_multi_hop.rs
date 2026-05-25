//! p9-fb-41 PR-5: MCP `ask` tool with `multi_hop: true` argument.
//!
//! Two Ollama-free pins:
//!
//! 1. `ask_tool_routes_multi_hop_true_to_decompose_first` — multi-hop
//!    dispatch differs from single-pass on dispatch shape. Single-pass
//!    retrieves *first* (empty KB → `NoChunks` refusal, no LLM call,
//!    `grounded=false`). Multi-hop calls *decompose first* (no
//!    retrieval yet), so an empty KB + no Ollama yields `error.v1`
//!    with `code=model_unreachable` — different wire shape than the
//!    refusal envelope. The two surfaces' divergence is the signal
//!    that the `multi_hop` arg actually routed the dispatch.
//! 2. `ask_input_schema_advertises_multi_hop_field` — `AskInput`'s
//!    `JsonSchema` exposes the new field so MCP host capability
//!    discovery (tools/list) renders it for agents.
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
    // on 127.0.0.1:11434 (the developer's box; CI; etc.). Combined
    // with a tight `request_timeout_secs`, the multi-hop dispatch
    // surfaces `model_unreachable` quickly and deterministically.
    cfg.models.llm.endpoint = "http://127.0.0.1:1".to_string();
    cfg.models.llm.request_timeout_secs = 2;
    cfg
}

/// The dispatch contract: with an empty KB, single-pass `ask` short-
/// circuits at retrieval (no LLM call) and returns a refusal Answer
/// (`grounded=false`, `isError=false`). Multi-hop calls *decompose
/// first*, so the same empty KB + unreachable LLM yields `error.v1`
/// with `code=model_unreachable` (`isError=true`). The divergence
/// confirms the `multi_hop` arg actually rerouted the dispatch.
#[tokio::test]
async fn ask_tool_routes_multi_hop_true_to_decompose_first() {
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
    // The dispatch contract is "multi-hop reached the LLM". The exact
    // error code depends on how the host TCP stack reports an
    // unreachable port — fast-path `ECONNREFUSED` classifies as
    // `model_unreachable`, but environments that take the connect
    // timeout path (some CI / Docker network stacks) surface
    // `timeout`. Accept either.
    let mh_code = mh_v["code"].as_str().unwrap_or("");
    assert!(
        matches!(mh_code, "model_unreachable" | "timeout"),
        "multi-hop dispatch must reach the LLM and surface model_unreachable/timeout; \
         got code={mh_code:?} from {mh_v}"
    );

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
