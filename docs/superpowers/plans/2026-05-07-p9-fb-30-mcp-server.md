# p9-fb-30 Implementation Plan — MCP server (stdio)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `kebab mcp` subcommand backed by new `kebab-mcp` crate exposing 4 read-only tools (`search` / `ask` / `schema` / `doctor`) over stdio JSON-RPC. Lets Claude Code / Cursor / OpenAI Agents and any MCP-aware host call kebab without writing custom subprocess wrappers.

**Architecture:** New crate `kebab-mcp` (lib only) wraps `rmcp 1.6` server SDK. It owns a long-lived `KebabApp` state (loaded `Config`, cached store handles) so tool calls hit hot caches. `kebab-cli` gets `Cmd::Mcp` arm that calls `kebab_mcp::serve_stdio(cfg)`. The fb-27 `error_classify` module is promoted from `kebab-cli` to `kebab-app::error_wire` so both UI crates share it (facade-rule compliance). 4 tools surface existing `kebab-app` facade methods (`search_with_config` / `ask_with_session_with_config` / `schema_with_config` / `doctor_with_config_path`) and serialize the result into wire-schema-v1 JSON inside MCP `text` content blocks.

**Tech Stack:** Rust 2024, `rmcp = { version = "1.6", features = ["server", "macros", "transport-io", "schemars"] }`, tokio (workspace, multi-thread), serde / serde_json / anyhow / tracing (all workspace).

**Spec source:** `docs/superpowers/specs/2026-05-07-p9-fb-30-mcp-server-design.md` (commit on branch `spec/p9-fb-30-mcp-server`).

---

## File map

**Create:**
- `crates/kebab-mcp/Cargo.toml`
- `crates/kebab-mcp/src/lib.rs` — `serve_stdio(cfg)` entry + `KebabHandler` rmcp impl
- `crates/kebab-mcp/src/state.rs` — `KebabAppState { config: Arc<Config>, store: OnceLock<...> }`
- `crates/kebab-mcp/src/tools/mod.rs` — `pub mod` index for 4 tool modules
- `crates/kebab-mcp/src/tools/search.rs` — `search` tool input schema + handler
- `crates/kebab-mcp/src/tools/ask.rs`
- `crates/kebab-mcp/src/tools/schema.rs`
- `crates/kebab-mcp/src/tools/doctor.rs`
- `crates/kebab-mcp/src/error.rs` — `to_tool_error_content(&anyhow::Error) -> CallToolResult` helper
- `crates/kebab-mcp/tests/initialize.rs`
- `crates/kebab-mcp/tests/tools_list.rs`
- `crates/kebab-mcp/tests/tools_call_search.rs`
- `crates/kebab-mcp/tests/tools_call_ask.rs`
- `crates/kebab-mcp/tests/tools_call_schema.rs`
- `crates/kebab-mcp/tests/tools_call_doctor.rs`
- `crates/kebab-mcp/tests/error_mapping.rs`
- `crates/kebab-cli/tests/cli_mcp_smoke.rs` — `target/debug/kebab mcp` spawn + JSON-RPC round-trip
- `crates/kebab-app/src/error_wire.rs` — promoted from `kebab-cli/src/error_classify.rs`

**Modify:**
- `Cargo.toml` (workspace root) — add `kebab-mcp` to `members`, add `rmcp = { version = "1.6.0", features = ["server", "macros", "transport-io", "schemars"] }` to `[workspace.dependencies]`
- `crates/kebab-app/Cargo.toml` — add `reqwest` to `[dev-dependencies]` (for `error_wire::tests::llm_unreachable_classifies_to_model_unreachable` migration)
- `crates/kebab-app/src/lib.rs` — `pub mod error_wire;` + `pub use error_wire::{ErrorV1, classify};`
- `crates/kebab-app/src/schema.rs` — `capabilities_snapshot()` flip `mcp_server: false` → `true`
- `crates/kebab-app/tests/schema_report.rs` — assertion update for `mcp_server: true`
- `crates/kebab-cli/Cargo.toml` — add `kebab-mcp` to `[dependencies]`, drop `reqwest` from `[dev-dependencies]` (moves with classify)
- `crates/kebab-cli/src/main.rs` — `mod error_classify;` 줄 제거, `use kebab_app::error_wire` 로 교체, `Cmd::Mcp` variant + arm 추가
- `crates/kebab-cli/src/wire.rs` — `wire_error_v1` 의 `&crate::error_classify::ErrorV1` → `&kebab_app::ErrorV1` 1줄
- `crates/kebab-cli/src/error_classify.rs` — **DELETE**
- `README.md` — `kebab mcp` row to commands table + MCP usage section
- `HANDOFF.md` — post-도그푸딩 entry
- `CLAUDE.md` — facade rule list 에 `kebab-mcp` 추가, crate 카운트 갱신
- `integrations/claude-code/kebab/SKILL.md` — MCP usage 추가
- `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md` — §10.1 MCP 절 추가
- `tasks/HOTFIXES.md` — 신규 entry
- `tasks/p9/p9-fb-30-mcp-server.md` — status `open` → `completed`, banner

---

## Task 1 — Promote `error_classify` → `kebab-app::error_wire`

**Files:**
- Create: `crates/kebab-app/src/error_wire.rs` (= 기존 `kebab-cli/src/error_classify.rs` 그대로)
- Modify: `crates/kebab-app/src/lib.rs`
- Modify: `crates/kebab-app/Cargo.toml` (add reqwest dev-dep)
- Modify: `crates/kebab-cli/src/main.rs`
- Modify: `crates/kebab-cli/src/wire.rs`
- Modify: `crates/kebab-cli/Cargo.toml` (drop reqwest dev-dep — moved)
- Delete: `crates/kebab-cli/src/error_classify.rs`

- [ ] **Step 1: Copy contents to new location**

```bash
cp /Users/user/Workspace/projects/kebab/crates/kebab-cli/src/error_classify.rs \
   /Users/user/Workspace/projects/kebab/crates/kebab-app/src/error_wire.rs
```

- [ ] **Step 2: Update kebab-app Cargo.toml — add reqwest dev-dep**

Open `crates/kebab-app/Cargo.toml`. In `[dev-dependencies]` block add:

```toml
reqwest = { version = "0.12", default-features = false, features = ["blocking", "rustls-tls"] }
```

(Mirror exactly what `crates/kebab-cli/Cargo.toml` had.)

- [ ] **Step 3: Wire module into kebab-app lib.rs**

Open `crates/kebab-app/src/lib.rs`. Find the existing `pub mod error_signal;` line. Add right after:

```rust
pub mod error_wire;
```

And in the re-export block (where `pub use schema::{...}` lives):

```rust
pub use error_wire::{ErrorV1, classify};
```

- [ ] **Step 4: Verify kebab-app builds + tests pass**

```bash
cd /Users/user/Workspace/projects/kebab
cargo test -p kebab-app --lib error_wire 2>&1 | tail -10
```

Expected: 7 tests pass (ConfigInvalid / NotIndexed / 2 LlmError variants / generic / generic+verbose / io_error). Tests are the SAME tests previously in `kebab-cli::error_classify::tests` — they migrate verbatim.

- [ ] **Step 5: Update kebab-cli main.rs imports**

Open `crates/kebab-cli/src/main.rs`. Find the line `mod error_classify;` (around line 12). Delete it.

Find any `error_classify::classify(...)` call inside `fn main()`. Replace with `kebab_app::classify(...)`.

(Likely around line 286 inside the `Err(e)` arm json branch — verify with `grep -n "error_classify" crates/kebab-cli/src/main.rs`.)

- [ ] **Step 6: Update kebab-cli wire.rs**

Open `crates/kebab-cli/src/wire.rs`. Find `wire_error_v1`:

```rust
pub fn wire_error_v1(e: &crate::error_classify::ErrorV1) -> Value {
```

Change to:

```rust
pub fn wire_error_v1(e: &kebab_app::ErrorV1) -> Value {
```

In the `#[cfg(test)] mod tests` block, find the test that imports `crate::error_classify::ErrorV1` and update to `kebab_app::ErrorV1`.

- [ ] **Step 7: Drop reqwest dev-dep from kebab-cli**

Open `crates/kebab-cli/Cargo.toml`. In `[dev-dependencies]` remove the `reqwest = { ... }` line. (It moved to kebab-app.)

- [ ] **Step 8: Delete the old file**

```bash
rm /Users/user/Workspace/projects/kebab/crates/kebab-cli/src/error_classify.rs
```

- [ ] **Step 9: Verify kebab-cli still compiles + tests pass**

```bash
cd /Users/user/Workspace/projects/kebab
cargo test -p kebab-cli --lib wire::tests 2>&1 | tail -10
cargo build -p kebab-cli 2>&1 | tail -3
```

Expected: 8 wire tests pass; build clean.

- [ ] **Step 10: Workspace clippy gate**

```bash
cargo clippy -p kebab-app -p kebab-cli --all-targets -- -D warnings 2>&1 | tail -5
```

Expected: clean, zero warnings.

- [ ] **Step 11: Commit**

```bash
git add crates/kebab-app/src/error_wire.rs crates/kebab-app/src/lib.rs crates/kebab-app/Cargo.toml crates/kebab-cli/src/main.rs crates/kebab-cli/src/wire.rs crates/kebab-cli/Cargo.toml
git rm crates/kebab-cli/src/error_classify.rs
git commit -m "$(cat <<'EOF'
🏗️ refactor(kebab-app): promote error_classify → kebab-app::error_wire (fb-30 prep)

fb-30 의 새 crate `kebab-mcp` 가 동일 classify 모듈 사용 — UI crate 끼리
import 는 facade rule 위반이므로 kebab-app 으로 promotion. fb-27 commit
c91228e 의 코드 그대로 이전 (struct + classify + classify_llm + 7 unit
test). reqwest dev-dep 도 함께 이동.

kebab-cli 는 `kebab_app::ErrorV1` / `kebab_app::classify` 로 import 경로
1줄 변경 + wire.rs 의 `&crate::error_classify::ErrorV1` 1줄 교체. 동작
무영향.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2 — New crate `kebab-mcp` skeleton

**Files:**
- Create: `crates/kebab-mcp/Cargo.toml`
- Create: `crates/kebab-mcp/src/lib.rs`
- Modify: `Cargo.toml` (workspace root)

- [ ] **Step 1: Add to workspace members**

Open root `Cargo.toml`. Find `members = [...]` block (search `[workspace]` section). Add `"crates/kebab-mcp"`. Maintain alphabetical order if existing members are sorted.

- [ ] **Step 2: Add rmcp to workspace dependencies**

In root `Cargo.toml` `[workspace.dependencies]` section, add:

```toml
rmcp = { version = "1.6", default-features = false, features = ["server", "macros", "transport-io", "schemars"] }
```

- [ ] **Step 3: Create crate Cargo.toml**

Write `crates/kebab-mcp/Cargo.toml`:

```toml
[package]
name        = "kebab-mcp"
edition     = { workspace = true }
rust-version = { workspace = true }
license     = { workspace = true }
repository  = { workspace = true }
version     = { workspace = true }

[dependencies]
rmcp        = { workspace = true }
tokio       = { workspace = true, features = ["rt-multi-thread", "macros", "io-util", "io-std"] }
serde       = { workspace = true }
serde_json  = { workspace = true }
anyhow      = { workspace = true }
tracing     = { workspace = true }
schemars    = "0.9"

kebab-app    = { workspace = true }
kebab-config = { workspace = true }
kebab-core   = { workspace = true }

[dev-dependencies]
tempfile = { workspace = true }
```

(`schemars` version may need bumping if rmcp pins a specific one — check with `cargo tree -p rmcp -e normal | grep schemars` after first build.)

- [ ] **Step 4: Create lib skeleton**

Write `crates/kebab-mcp/src/lib.rs`:

```rust
//! MCP (Model Context Protocol) server over stdio. Exposes 4 read-only
//! tools (`search` / `ask` / `schema` / `doctor`) backed by `kebab-app`
//! facade methods. Used by `kebab-cli`'s `Cmd::Mcp` arm.
//!
//! See spec `docs/superpowers/specs/2026-05-07-p9-fb-30-mcp-server-design.md`.

use anyhow::Result;

use kebab_config::Config;

/// Run the MCP server on stdio JSON-RPC. Blocks until the client closes
/// the stream (typically when the agent host exits).
pub fn serve_stdio(_cfg: Config) -> Result<()> {
    // Skeleton — actual rmcp wiring lands in Task 3.
    anyhow::bail!("kebab-mcp: serve_stdio not yet implemented")
}
```

- [ ] **Step 5: Verify workspace builds**

```bash
cd /Users/user/Workspace/projects/kebab
cargo build -p kebab-mcp 2>&1 | tail -5
```

Expected: PASS (rmcp downloads + builds, may take 1-3 min on first run).

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock crates/kebab-mcp
git commit -m "$(cat <<'EOF'
🏗️ chore(kebab-mcp): scaffold new crate (fb-30)

Empty lib + serve_stdio entry that bails until Task 3 wires rmcp. Adds
rmcp 1.6 to workspace dependencies (server + macros + transport-io +
schemars features) + tokio multi-thread.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 3 — `KebabHandler` skeleton + initialize handshake test

**Files:**
- Create: `crates/kebab-mcp/src/state.rs`
- Modify: `crates/kebab-mcp/src/lib.rs`
- Create: `crates/kebab-mcp/tests/initialize.rs`

- [ ] **Step 1: Define server state**

Write `crates/kebab-mcp/src/state.rs`:

```rust
//! Long-lived server state — holds Config so per-request handlers don't
//! reload from disk. Future: cache opened SqliteStore / Lance handles
//! here so first tool call pays the cost, subsequent calls hit warm
//! state.

use std::sync::Arc;

use kebab_config::Config;

#[derive(Clone)]
pub struct KebabAppState {
    pub config: Arc<Config>,
}

impl KebabAppState {
    pub fn new(config: Config) -> Self {
        Self {
            config: Arc::new(config),
        }
    }
}
```

- [ ] **Step 2: Implement KebabHandler with rmcp**

Open `crates/kebab-mcp/src/lib.rs` and replace the skeleton with:

```rust
//! MCP (Model Context Protocol) server over stdio. Exposes 4 read-only
//! tools (`search` / `ask` / `schema` / `doctor`) backed by `kebab-app`
//! facade methods.
//!
//! See spec `docs/superpowers/specs/2026-05-07-p9-fb-30-mcp-server-design.md`.

use anyhow::Result;

use rmcp::ServerHandler;
use rmcp::model::{Implementation, ProtocolVersion, ServerCapabilities, ServerInfo};
use rmcp::service::ServiceExt;
use rmcp::transport::stdio;

use kebab_config::Config;

pub mod state;
use state::KebabAppState;

#[derive(Clone)]
pub struct KebabHandler {
    state: KebabAppState,
}

impl KebabHandler {
    pub fn new(state: KebabAppState) -> Self {
        Self { state }
    }
}

impl ServerHandler for KebabHandler {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::default(),
            capabilities: ServerCapabilities::builder()
                .enable_tools()
                .build(),
            server_info: Implementation {
                name: "kebab".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
            instructions: None,
        }
    }
}

/// Run the MCP server on stdio JSON-RPC. Blocks until the client closes
/// the stream.
#[tokio::main(flavor = "multi_thread")]
pub async fn serve_stdio(cfg: Config) -> Result<()> {
    tracing::info!("kebab-mcp: starting stdio server");
    let state = KebabAppState::new(cfg);
    let handler = KebabHandler::new(state);
    let service = handler.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
```

(rmcp's exact API may differ slightly — `ServerCapabilities::builder().enable_tools().build()` is the rmcp 1.6 pattern. If it errs, check `cargo doc -p rmcp --open` and adapt to whatever the actual builder name is.)

- [ ] **Step 3: Write the failing initialize test**

Create `crates/kebab-mcp/tests/initialize.rs`:

```rust
//! Integration: in-process round-trip — initialize handshake.
//!
//! rmcp 1.6 provides `serve_in_memory` style transport for tests. If
//! not available, fall back to spawning `target/debug/kebab mcp` and
//! sending JSON-RPC over its stdio.

use std::sync::Arc;

use kebab_config::Config;
use kebab_mcp::{KebabAppState, KebabHandler};

#[tokio::test]
async fn initialize_returns_kebab_server_info() {
    // Build a default-config state — initialize doesn't actually open
    // any store, so this can be cheap.
    let cfg = Config::defaults();
    let state = KebabAppState::new(cfg);
    let handler = KebabHandler::new(state);

    // Use rmcp's in-memory client/server pair for a fast round-trip.
    // (Pattern: rmcp::test_helpers::in_memory_pair OR equivalent —
    // consult rmcp docs.rs/rmcp/1.6/rmcp/test_helpers/index.html)
    let info = handler.get_info();
    assert_eq!(info.server_info.name, "kebab");
    assert!(!info.server_info.version.is_empty());
    assert!(info.capabilities.tools.is_some());
}
```

(For the initial commit, asserting `get_info()` directly is sufficient — full client/server round-trip lands when we have at least one tool to call. If rmcp's in-memory transport is documented, prefer it.)

- [ ] **Step 4: Run test**

```bash
cargo test -p kebab-mcp --test initialize 2>&1 | tail -10
```

Expected: PASS — handler builds, get_info returns correct shape.

If test fails because `rmcp::ServerCapabilities::builder()` API differs, consult rmcp docs and adapt. Common alternatives:
- `ServerCapabilitiesBuilder` direct
- `Default::default()` + manual field set

- [ ] **Step 5: Commit**

```bash
git add crates/kebab-mcp
git commit -m "$(cat <<'EOF'
✨ feat(kebab-mcp): handler skeleton + initialize handshake (fb-30)

KebabHandler implements rmcp::ServerHandler::get_info — returns
serverInfo (name="kebab", version from CARGO_PKG_VERSION) and
capabilities.tools. KebabAppState wraps Config in Arc for cheap clone
into per-request task scope. serve_stdio entry runs server until
client closes the stream.

Tools wire-up lands in subsequent tasks (one tool per task).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 4 — `schema` tool (simplest, no input args)

**Files:**
- Create: `crates/kebab-mcp/src/tools/mod.rs`
- Create: `crates/kebab-mcp/src/tools/schema.rs`
- Create: `crates/kebab-mcp/src/error.rs`
- Modify: `crates/kebab-mcp/src/lib.rs` — register `tools` mod, plug into handler
- Create: `crates/kebab-mcp/tests/tools_call_schema.rs`

- [ ] **Step 1: Create error helper**

Write `crates/kebab-mcp/src/error.rs`:

```rust
//! Map `anyhow::Error` returned by kebab-app facades to MCP
//! `CallToolResult` with `isError: true` + error.v1 JSON content.

use rmcp::model::{CallToolResult, Content};

use kebab_app::classify;

pub fn to_tool_error(err: &anyhow::Error) -> CallToolResult {
    let v1 = classify(err, false);
    let body = serde_json::to_string(&v1).unwrap_or_else(|_| {
        r#"{"schema_version":"error.v1","code":"generic","message":"serialize failed"}"#
            .to_string()
    });
    let mut result = CallToolResult::error(vec![Content::text(body)]);
    // Some rmcp versions: result.is_error = Some(true) is auto-set by
    // CallToolResult::error. Verify post-build.
    result
}

/// Wrap a successful wire-schema JSON string as a `CallToolResult`.
pub fn to_tool_success(json: String) -> CallToolResult {
    CallToolResult::success(vec![Content::text(json)])
}
```

(rmcp 1.6 signatures: check `CallToolResult::error` / `success` variants. If they differ, adapt — the goal is `is_error=true|false` + single text content.)

- [ ] **Step 2: Create tools/mod.rs**

Write `crates/kebab-mcp/src/tools/mod.rs`:

```rust
//! Tool implementations — one module per tool.

pub mod schema;
// pub mod doctor;  // wired in Task 5
// pub mod search;  // wired in Task 6
// pub mod ask;     // wired in Task 7
```

- [ ] **Step 3: Implement schema tool**

Write `crates/kebab-mcp/src/tools/schema.rs`:

```rust
//! `schema` tool — wraps `kebab_app::schema_with_config`.
//! Input: {} (no args). Output: schema.v1 JSON.

use rmcp::model::CallToolResult;
use serde::{Deserialize, Serialize};
use schemars::JsonSchema;

use crate::error::{to_tool_error, to_tool_success};
use crate::state::KebabAppState;

#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
pub struct SchemaInput {}

pub fn handle(state: &KebabAppState, _input: SchemaInput) -> CallToolResult {
    match kebab_app::schema_with_config(&state.config) {
        Ok(report) => match serde_json::to_string(&report) {
            Ok(json) => to_tool_success(json),
            Err(e) => to_tool_error(&anyhow::Error::from(e)),
        },
        Err(e) => to_tool_error(&e),
    }
}
```

- [ ] **Step 4: Register tool in handler**

Open `crates/kebab-mcp/src/lib.rs`. Add the macro-driven tool routing block. The exact form for rmcp 1.6 is the `#[tool_router]` macro — adapt this template:

```rust
use rmcp::{tool, tool_handler, tool_router};
use rmcp::handler::server::router::tool::ToolRouter;

#[derive(Clone)]
pub struct KebabHandler {
    state: KebabAppState,
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl KebabHandler {
    pub fn new(state: KebabAppState) -> Self {
        Self {
            state,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(description = "Introspection — wire schemas, capabilities, model versions, index stats.")]
    async fn schema(&self) -> Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
        Ok(crate::tools::schema::handle(&self.state, crate::tools::schema::SchemaInput::default()))
    }
}

#[tool_handler]
impl rmcp::ServerHandler for KebabHandler {
    fn get_info(&self) -> ServerInfo {
        // ... (unchanged from Task 3)
    }
}
```

(Exact macro names may differ in rmcp 1.6 — check `examples/` in the rmcp repo for canonical patterns. If macros aren't ergonomic for our case, write the `tools/list` and `tools/call` dispatch by hand.)

Add `pub mod tools;` to lib.rs declarations.

- [ ] **Step 5: Write the failing test**

Create `crates/kebab-mcp/tests/tools_call_schema.rs`:

```rust
//! Integration: tools/call name=schema — verify response is schema.v1.

use kebab_config::Config;
use kebab_mcp::{KebabAppState, KebabHandler};

#[tokio::test]
async fn schema_tool_returns_schema_v1_json() {
    // Use a TempDir KB so schema_with_config has a valid SqliteStore.
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = Config::defaults();
    cfg.storage.data_dir = dir.path().to_string_lossy().into_owned();
    cfg.workspace.root = dir.path().join("notes").to_string_lossy().into_owned();
    cfg.models.embedding.provider = "none".to_string();
    cfg.models.embedding.dimensions = 0;
    std::fs::create_dir_all(&cfg.workspace.root).unwrap();

    // schema_with_config requires kebab.sqlite to exist — seed via a
    // 0-file ingest. (Mirrors crates/kebab-app/tests/schema_report.rs
    // pattern.)
    let scope = kebab_core::SourceScope {
        root: std::path::PathBuf::from(&cfg.workspace.root),
        include: vec![],
        exclude: vec![],
    };
    let _ = kebab_app::ingest_with_config(&cfg, false, scope).unwrap();

    // Direct handler invocation (no transport — rmcp test harness if
    // available, else direct call).
    let state = KebabAppState::new(cfg);
    let handler = KebabHandler::new(state);

    // The simplest assertion path: call the schema handler directly.
    // Full round-trip via tools/call comes via tools_list.rs in Task 8.
    let result = crate::tools::schema::handle(
        &handler.state(),
        crate::tools::schema::SchemaInput::default(),
    );
    assert!(!result.is_error.unwrap_or(false));
    let content = result.content.first().unwrap();
    let text = match content {
        rmcp::model::Content::Text(t) => &t.text,
        other => panic!("expected text content, got {other:?}"),
    };
    let v: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(v.get("schema_version").and_then(|s| s.as_str()), Some("schema.v1"));
}
```

(If `KebabHandler::state()` accessor doesn't exist, add a `pub fn state(&self) -> &KebabAppState` to KebabHandler. The test calls the tool's `handle` fn directly to keep the test off rmcp's transport surface for now.)

(`ingest_with_config` may take 2 args (cfg, summary_only) not 3 — check current signature with `grep -n "pub fn ingest_with_config" crates/kebab-app/src/lib.rs`. Adapt the call.)

- [ ] **Step 6: Run test**

```bash
cargo test -p kebab-mcp --test tools_call_schema 2>&1 | tail -10
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/kebab-mcp
git commit -m "$(cat <<'EOF'
✨ feat(kebab-mcp): schema tool (fb-30)

First tool wired — `schema` (no input args, returns schema.v1 JSON
mirroring `kebab schema --json`). Sets up the per-tool module pattern
(crates/kebab-mcp/src/tools/<name>.rs) + error helper that maps
anyhow::Error to MCP CallToolResult.error with error.v1 content.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 5 — `doctor` tool (no input args)

**Files:**
- Create: `crates/kebab-mcp/src/tools/doctor.rs`
- Modify: `crates/kebab-mcp/src/tools/mod.rs` — uncomment `pub mod doctor;`
- Modify: `crates/kebab-mcp/src/lib.rs` — `#[tool]` for doctor
- Create: `crates/kebab-mcp/tests/tools_call_doctor.rs`

- [ ] **Step 1: Implement doctor tool**

Write `crates/kebab-mcp/src/tools/doctor.rs`:

```rust
//! `doctor` tool — wraps `kebab_app::doctor_with_config_path`.
//! Input: {}. Output: doctor.v1 JSON.

use rmcp::model::CallToolResult;
use serde::{Deserialize, Serialize};
use schemars::JsonSchema;

use crate::error::{to_tool_error, to_tool_success};
use crate::state::KebabAppState;

#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
pub struct DoctorInput {}

pub fn handle(state: &KebabAppState, _input: DoctorInput) -> CallToolResult {
    // doctor_with_config_path takes Option<&Path>. We have a loaded
    // Config in state; the equivalent path-explicit call is what
    // kebab-cli uses. Surface what kebab-app exposes — likely
    // `doctor_with_config(&Config)` or similar; check the actual API.
    //
    // If only `doctor_with_config_path(Option<&Path>)` exists, we
    // need to know the original config path — KebabAppState should
    // carry it (Task 3 state would also need to remember the path).
    //
    // For minimal change, use whatever public facade variant exists.
    match kebab_app::doctor_with_config_path(None) {
        Ok(report) => match serde_json::to_string(&report) {
            Ok(json) => to_tool_success(json),
            Err(e) => to_tool_error(&anyhow::Error::from(e)),
        },
        Err(e) => to_tool_error(&e),
    }
}
```

If only `doctor_with_config_path(Option<&Path>)` exists and we lose the explicit path through `KebabAppState`, extend `KebabAppState` with `config_path: Option<PathBuf>` (set in Task 3) and pass it here. Mirror what `kebab-cli::main::Cmd::Doctor` does today.

- [ ] **Step 2: Wire up tool in handler**

Open `crates/kebab-mcp/src/lib.rs`. Inside the `#[tool_router] impl KebabHandler` block, add:

```rust
    #[tool(description = "Health check — config / data dir / Ollama reachability.")]
    async fn doctor(&self) -> Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
        Ok(crate::tools::doctor::handle(&self.state, crate::tools::doctor::DoctorInput::default()))
    }
```

Uncomment `pub mod doctor;` in `crates/kebab-mcp/src/tools/mod.rs`.

- [ ] **Step 3: Write the test**

Create `crates/kebab-mcp/tests/tools_call_doctor.rs`:

```rust
//! Integration: tools/call name=doctor — returns doctor.v1.

use kebab_config::Config;
use kebab_mcp::{KebabAppState, KebabHandler};

#[tokio::test]
async fn doctor_tool_returns_doctor_v1_json() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = Config::defaults();
    cfg.storage.data_dir = dir.path().to_string_lossy().into_owned();
    cfg.workspace.root = dir.path().join("notes").to_string_lossy().into_owned();
    cfg.models.embedding.provider = "none".to_string();
    cfg.models.embedding.dimensions = 0;
    std::fs::create_dir_all(&cfg.workspace.root).unwrap();

    let state = KebabAppState::new(cfg);
    let handler = KebabHandler::new(state);

    let result = kebab_mcp::tools::doctor::handle(
        handler.state(),
        kebab_mcp::tools::doctor::DoctorInput::default(),
    );
    let content = result.content.first().unwrap();
    let text = match content {
        rmcp::model::Content::Text(t) => &t.text,
        other => panic!("expected text content, got {other:?}"),
    };
    let v: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(v.get("schema_version").and_then(|s| s.as_str()), Some("doctor.v1"));
    // doctor.v1 has `ok` boolean — assert presence (value can be either
    // depending on whether Ollama is reachable in the test env).
    assert!(v.get("ok").and_then(|b| b.as_bool()).is_some());
}
```

- [ ] **Step 4: Run test + commit**

```bash
cargo test -p kebab-mcp --test tools_call_doctor 2>&1 | tail -10
git add crates/kebab-mcp
git commit -m "✨ feat(kebab-mcp): doctor tool (fb-30)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 6 — `search` tool (input args: query / mode / k)

**Files:**
- Create: `crates/kebab-mcp/src/tools/search.rs`
- Modify: `crates/kebab-mcp/src/tools/mod.rs`
- Modify: `crates/kebab-mcp/src/lib.rs`
- Create: `crates/kebab-mcp/tests/tools_call_search.rs`

- [ ] **Step 1: Implement search tool**

Write `crates/kebab-mcp/src/tools/search.rs`:

```rust
//! `search` tool — wraps `kebab_app::search_with_config`.
//! Input: { query, mode?, k? }. Output: search_hit.v1 array JSON.

use rmcp::model::CallToolResult;
use serde::{Deserialize, Serialize};
use schemars::JsonSchema;

use crate::error::{to_tool_error, to_tool_success};
use crate::state::KebabAppState;

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SearchInput {
    /// User query (free text).
    pub query: String,
    /// Retrieval mode. Defaults to "hybrid".
    #[serde(default = "default_mode")]
    pub mode: String,
    /// Top-K results. Defaults to 10. Range 1-100.
    #[serde(default = "default_k")]
    pub k: usize,
}

fn default_mode() -> String { "hybrid".to_string() }
fn default_k() -> usize { 10 }

pub fn handle(state: &KebabAppState, input: SearchInput) -> CallToolResult {
    let k = input.k.clamp(1, 100);
    let mode = match input.mode.as_str() {
        "lexical" => kebab_core::SearchMode::Lexical,
        "vector" => kebab_core::SearchMode::Vector,
        "hybrid" | _ => kebab_core::SearchMode::Hybrid,
    };
    match kebab_app::search_with_config(&state.config, &input.query, mode, k) {
        Ok(hits) => {
            // serialize as wire-schema array — kebab-cli has the same
            // pattern in main.rs Cmd::Search arm. Replicate the
            // wire_search_hits transformation inline.
            let array: Vec<serde_json::Value> = hits
                .iter()
                .map(|h| serde_json::to_value(h).unwrap_or_default())
                .collect();
            // Each element gets schema_version tag.
            let tagged: Vec<serde_json::Value> = array
                .into_iter()
                .map(|mut v| {
                    if let serde_json::Value::Object(ref mut map) = v {
                        map.insert(
                            "schema_version".to_string(),
                            serde_json::Value::String("search_hit.v1".to_string()),
                        );
                    }
                    v
                })
                .collect();
            let json = serde_json::to_string(&serde_json::Value::Array(tagged)).unwrap();
            to_tool_success(json)
        }
        Err(e) => to_tool_error(&e),
    }
}
```

(`kebab_app::search_with_config` exact signature: check with `grep -n "pub fn search_with_config" crates/kebab-app/src/lib.rs`. Adapt arg order / SearchMode enum location. Re-use `kebab-cli::wire::wire_search_hits` if a public version becomes available — but that lives in kebab-cli which we can't import. Inline the tag pattern as shown.)

- [ ] **Step 2: Wire tool**

In `lib.rs`:

```rust
    #[tool(description = "Lexical / vector / hybrid retrieval over indexed corpus.")]
    async fn search(
        &self,
        rmcp::handler::server::tool::Parameters(input): rmcp::handler::server::tool::Parameters<crate::tools::search::SearchInput>,
    ) -> Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
        Ok(crate::tools::search::handle(&self.state, input))
    }
```

(rmcp's `Parameters` extractor parses + validates against the inputSchema derived from `JsonSchema`. Exact import path verify in rmcp docs — common alternatives: `rmcp::Params`, `rmcp::extractors::Json`.)

Uncomment `pub mod search;` in `tools/mod.rs`.

- [ ] **Step 3: Write the test**

Create `crates/kebab-mcp/tests/tools_call_search.rs`:

```rust
use std::fs;

use kebab_config::Config;
use kebab_mcp::{KebabAppState, KebabHandler};

#[tokio::test]
async fn search_tool_returns_search_hits_array() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = Config::defaults();
    cfg.storage.data_dir = dir.path().to_string_lossy().into_owned();
    cfg.workspace.root = dir.path().join("notes").to_string_lossy().into_owned();
    cfg.models.embedding.provider = "none".to_string();
    cfg.models.embedding.dimensions = 0;
    std::fs::create_dir_all(&cfg.workspace.root).unwrap();

    fs::write(
        std::path::PathBuf::from(&cfg.workspace.root).join("a.md"),
        "# Alpha\n\nThis document mentions kebab and bread.",
    ).unwrap();

    let scope = kebab_core::SourceScope {
        root: std::path::PathBuf::from(&cfg.workspace.root),
        include: vec![],
        exclude: vec![],
    };
    let _ = kebab_app::ingest_with_config(&cfg, false, scope).unwrap();

    let state = KebabAppState::new(cfg);
    let handler = KebabHandler::new(state);

    let result = kebab_mcp::tools::search::handle(
        handler.state(),
        kebab_mcp::tools::search::SearchInput {
            query: "kebab".to_string(),
            mode: "lexical".to_string(),
            k: 5,
        },
    );
    assert!(!result.is_error.unwrap_or(false));

    let text = match result.content.first().unwrap() {
        rmcp::model::Content::Text(t) => &t.text,
        other => panic!("expected text content, got {other:?}"),
    };
    let v: serde_json::Value = serde_json::from_str(text).unwrap();
    let arr = v.as_array().expect("search returns array");
    assert!(!arr.is_empty(), "expected at least one hit for 'kebab' in 'a.md'");
    assert_eq!(
        arr[0].get("schema_version").and_then(|s| s.as_str()),
        Some("search_hit.v1"),
    );
}
```

- [ ] **Step 4: Run test + commit**

```bash
cargo test -p kebab-mcp --test tools_call_search 2>&1 | tail -10
git add crates/kebab-mcp
git commit -m "✨ feat(kebab-mcp): search tool (fb-30)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 7 — `ask` tool (input: query / session_id?)

**Files:**
- Create: `crates/kebab-mcp/src/tools/ask.rs`
- Modify: `crates/kebab-mcp/src/tools/mod.rs`
- Modify: `crates/kebab-mcp/src/lib.rs`
- Create: `crates/kebab-mcp/tests/tools_call_ask.rs`

- [ ] **Step 1: Implement ask tool**

Write `crates/kebab-mcp/src/tools/ask.rs`:

```rust
//! `ask` tool — wraps `kebab_app::ask_with_config` (or
//! `ask_with_session_with_config` when session_id provided).
//! Input: { query, session_id? }. Output: answer.v1 JSON.

use rmcp::model::CallToolResult;
use serde::{Deserialize, Serialize};
use schemars::JsonSchema;

use crate::error::{to_tool_error, to_tool_success};
use crate::state::KebabAppState;

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct AskInput {
    /// The user question.
    pub query: String,
    /// Optional session id for multi-turn RAG context.
    pub session_id: Option<String>,
}

pub fn handle(state: &KebabAppState, input: AskInput) -> CallToolResult {
    let opts = kebab_app::AskOpts::default();
    let result = match input.session_id {
        Some(sid) => kebab_app::ask_with_session_with_config(&state.config, &sid, &input.query, opts),
        None => kebab_app::ask_with_config(&state.config, &input.query, opts),
    };
    match result {
        Ok(answer) => {
            let mut v = serde_json::to_value(&answer).unwrap_or_default();
            if let serde_json::Value::Object(ref mut map) = v {
                map.insert(
                    "schema_version".to_string(),
                    serde_json::Value::String("answer.v1".to_string()),
                );
            }
            to_tool_success(v.to_string())
        }
        Err(e) => to_tool_error(&e),
    }
}
```

(`AskOpts` exact field set: check `grep -n "pub struct AskOpts" crates/kebab-app/src/lib.rs` and use `Default::default()`. `ask_with_session_with_config` arg order: verify with `grep -n "pub fn ask_with_session_with_config" crates/kebab-app/src/lib.rs`.)

- [ ] **Step 2: Wire tool**

In `lib.rs`:

```rust
    #[tool(description = "Grounded RAG answer with citations. Returns answer.v1 with grounded=false when KB lacks context.")]
    async fn ask(
        &self,
        rmcp::handler::server::tool::Parameters(input): rmcp::handler::server::tool::Parameters<crate::tools::ask::AskInput>,
    ) -> Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
        Ok(crate::tools::ask::handle(&self.state, input))
    }
```

Uncomment `pub mod ask;` in `tools/mod.rs`.

- [ ] **Step 3: Write the test**

Create `crates/kebab-mcp/tests/tools_call_ask.rs`:

```rust
//! `ask` tool returns answer.v1 — refusal path covered (no Ollama
//! required for refusal-on-empty-corpus case).

use kebab_config::Config;
use kebab_mcp::{KebabAppState, KebabHandler};

#[tokio::test]
async fn ask_tool_returns_answer_v1_with_refusal_on_empty_kb() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = Config::defaults();
    cfg.storage.data_dir = dir.path().to_string_lossy().into_owned();
    cfg.workspace.root = dir.path().join("notes").to_string_lossy().into_owned();
    cfg.models.embedding.provider = "none".to_string();
    cfg.models.embedding.dimensions = 0;
    std::fs::create_dir_all(&cfg.workspace.root).unwrap();

    let scope = kebab_core::SourceScope {
        root: std::path::PathBuf::from(&cfg.workspace.root),
        include: vec![],
        exclude: vec![],
    };
    let _ = kebab_app::ingest_with_config(&cfg, false, scope).unwrap();

    let state = KebabAppState::new(cfg);
    let handler = KebabHandler::new(state);

    let result = kebab_mcp::tools::ask::handle(
        handler.state(),
        kebab_mcp::tools::ask::AskInput {
            query: "what is the meaning of life".to_string(),
            session_id: None,
        },
    );
    // Empty KB → refusal (grounded:false) is normal — NOT isError.
    assert!(!result.is_error.unwrap_or(false));

    let text = match result.content.first().unwrap() {
        rmcp::model::Content::Text(t) => &t.text,
        other => panic!("expected text content, got {other:?}"),
    };
    let v: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(v.get("schema_version").and_then(|s| s.as_str()), Some("answer.v1"));
    assert_eq!(v.get("grounded").and_then(|b| b.as_bool()), Some(false));
}
```

- [ ] **Step 4: Run test + commit**

```bash
cargo test -p kebab-mcp --test tools_call_ask 2>&1 | tail -10
git add crates/kebab-mcp
git commit -m "✨ feat(kebab-mcp): ask tool (fb-30)

Multi-turn via optional session_id (kebab_app::ask_with_session_with_config).
Refusal (grounded:false) NOT mapped to isError — agent branches on
the wire payload's grounded flag.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 8 — Tool error mapping (bad config → isError + error.v1)

**Files:**
- Create: `crates/kebab-mcp/tests/error_mapping.rs`

- [ ] **Step 1: Write the test**

```rust
//! tools/call with bad config → isError=true + error.v1 content.

use std::path::PathBuf;

use kebab_config::Config;
use kebab_mcp::{KebabAppState, KebabHandler};

#[tokio::test]
async fn schema_tool_emits_error_v1_when_db_missing() {
    // Point at a directory that does NOT have kebab.sqlite.
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = Config::defaults();
    cfg.storage.data_dir = dir.path().to_string_lossy().into_owned();
    cfg.workspace.root = dir.path().join("notes").to_string_lossy().into_owned();
    cfg.models.embedding.provider = "none".to_string();
    cfg.models.embedding.dimensions = 0;
    // Note: NO ingest, so kebab.sqlite is absent → schema_with_config
    // calls open_existing → NotIndexed → tool error.

    let state = KebabAppState::new(cfg);
    let handler = KebabHandler::new(state);

    let result = kebab_mcp::tools::schema::handle(
        handler.state(),
        kebab_mcp::tools::schema::SchemaInput::default(),
    );
    assert_eq!(result.is_error, Some(true), "expected isError=true on missing DB");

    let text = match result.content.first().unwrap() {
        rmcp::model::Content::Text(t) => &t.text,
        other => panic!("expected text content, got {other:?}"),
    };
    let v: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(v.get("schema_version").and_then(|s| s.as_str()), Some("error.v1"));
    assert_eq!(v.get("code").and_then(|s| s.as_str()), Some("not_indexed"));
}
```

- [ ] **Step 2: Run test + commit**

```bash
cargo test -p kebab-mcp --test error_mapping 2>&1 | tail -10
git add crates/kebab-mcp/tests/error_mapping.rs
git commit -m "🧪 test(kebab-mcp): error mapping — bad config → error.v1 (fb-30)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 9 — `tools/list` integration (full round-trip via rmcp client)

**Files:**
- Create: `crates/kebab-mcp/tests/tools_list.rs`

- [ ] **Step 1: Write the test**

This test exercises the FULL round-trip through rmcp's transport. Use rmcp's in-memory client/server pair if documented (look in `https://github.com/modelcontextprotocol/rust-sdk/tree/main/examples`); otherwise spawn the binary in Task 10's `cli_mcp_smoke.rs` style.

In-memory pattern (verify rmcp 1.6 API):

```rust
use kebab_config::Config;
use kebab_mcp::{KebabAppState, KebabHandler};

#[tokio::test]
async fn tools_list_returns_four_tools() {
    let cfg = Config::defaults();
    let state = KebabAppState::new(cfg);
    let handler = KebabHandler::new(state);

    // rmcp::test_helpers::serve_in_memory or equivalent — adjust to
    // actual rmcp 1.6 helper. Goal: get a `Client` connected to our
    // handler over an in-process duplex stream.
    let (client, _server) = rmcp::transport::serve_in_memory(handler).await.unwrap();

    let tools = client.list_tools(Default::default()).await.unwrap();
    let names: Vec<_> = tools.tools.iter().map(|t| t.name.as_str()).collect();
    assert!(names.contains(&"schema"));
    assert!(names.contains(&"doctor"));
    assert!(names.contains(&"search"));
    assert!(names.contains(&"ask"));
    assert_eq!(names.len(), 4);

    // Verify search has its inputSchema with required `query` field.
    let search = tools.tools.iter().find(|t| t.name == "search").unwrap();
    let schema = search.input_schema.as_object().unwrap();
    let required = schema.get("required").unwrap().as_array().unwrap();
    assert!(required.iter().any(|v| v == "query"));
}
```

If rmcp 1.6 doesn't expose `serve_in_memory`, fall back to spawning the CLI binary (Task 10) for this test as well, and remove this `tools_list.rs` integration in favor of a unit-level assertion that tools/list contains the 4 names.

- [ ] **Step 2: Run + commit**

```bash
cargo test -p kebab-mcp --test tools_list 2>&1 | tail -10
git add crates/kebab-mcp/tests/tools_list.rs
git commit -m "🧪 test(kebab-mcp): tools/list returns 4 tools with input schemas (fb-30)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 10 — `kebab-cli Cmd::Mcp` arm + smoke test

**Files:**
- Modify: `crates/kebab-cli/Cargo.toml` — add `kebab-mcp` to `[dependencies]`
- Modify: `crates/kebab-cli/src/main.rs`
- Create: `crates/kebab-cli/tests/cli_mcp_smoke.rs`

- [ ] **Step 1: Add kebab-mcp dep to kebab-cli**

In `crates/kebab-cli/Cargo.toml` `[dependencies]`:

```toml
kebab-mcp = { workspace = true }
```

Then add `kebab-mcp = { path = "../kebab-mcp" }` to root `[workspace.dependencies]` if not present.

- [ ] **Step 2: Add Cmd::Mcp variant**

In `crates/kebab-cli/src/main.rs`, find `enum Cmd` and add:

```rust
    /// Run the MCP (Model Context Protocol) stdio server. Used by
    /// agent hosts (Claude Code / Cursor / OpenAI Agents) to call kebab
    /// tools (search / ask / schema / doctor).
    Mcp,
```

In `fn run`, add the arm:

```rust
        Cmd::Mcp => {
            let cfg = kebab_config::Config::load(cli.config.as_deref())?;
            kebab_mcp::serve_stdio(cfg)
        }
```

- [ ] **Step 3: Build + manual smoke**

```bash
cd /Users/user/Workspace/projects/kebab
cargo build -p kebab-cli 2>&1 | tail -3
target/debug/kebab mcp --help 2>&1 | head -5
```

Expected: build clean; `kebab mcp` shows up in help.

Manual JSON-RPC round-trip:

```bash
printf '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"smoke","version":"0"}}}\n{"jsonrpc":"2.0","method":"notifications/initialized"}\n{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}\n' | target/debug/kebab mcp 2>&1 | head -3
```

Expected: 2 JSON responses (initialize result + tools/list result with 4 tools). If the binary blocks waiting for input, ensure each line ends with `\n` and the parent shell closes stdin.

- [ ] **Step 4: Write spawn-based smoke test**

Create `crates/kebab-cli/tests/cli_mcp_smoke.rs`:

```rust
//! Spawn `target/debug/kebab mcp` and exercise initialize → tools/list.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

#[test]
fn cli_mcp_initialize_then_tools_list() {
    let bin = env!("CARGO_BIN_EXE_kebab");
    let mut child = Command::new(bin)
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout);

    // initialize
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"2025-03-26","capabilities":{{}},"clientInfo":{{"name":"test","version":"0"}}}}}}"#).unwrap();
    // initialized notification
    writeln!(stdin, r#"{{"jsonrpc":"2.0","method":"notifications/initialized"}}"#).unwrap();
    // tools/list
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{{}}}}"#).unwrap();

    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let init: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
    assert_eq!(init.get("id").and_then(|i| i.as_i64()), Some(1));
    assert!(init.get("result").is_some());

    line.clear();
    reader.read_line(&mut line).unwrap();
    let list: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
    assert_eq!(list.get("id").and_then(|i| i.as_i64()), Some(2));
    let tools = list["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 4);

    // Close stdin so the server exits cleanly.
    drop(stdin);
    let _ = child.wait().unwrap();
}
```

- [ ] **Step 5: Run smoke + commit**

```bash
cargo test -p kebab-cli --test cli_mcp_smoke 2>&1 | tail -10
git add crates/kebab-cli/Cargo.toml crates/kebab-cli/src/main.rs crates/kebab-cli/tests/cli_mcp_smoke.rs Cargo.toml
git commit -m "$(cat <<'EOF'
✨ feat(kebab-cli): kebab mcp subcommand (fb-30)

Wires kebab_mcp::serve_stdio into kebab-cli. `--config <path>` honored
via the established Config::load pattern.

Smoke test spawns the binary + sends initialize + initialized +
tools/list over stdin, asserts 4 tools returned.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 11 — Capability flag flip (`mcp_server: true`)

**Files:**
- Modify: `crates/kebab-app/src/schema.rs`
- Modify: `crates/kebab-app/tests/schema_report.rs`

- [ ] **Step 1: Flip the flag**

Open `crates/kebab-app/src/schema.rs`. Find `capabilities_snapshot()`:

```rust
        mcp_server: false,
```

Change to:

```rust
        mcp_server: true,
```

- [ ] **Step 2: Update the schema_report test**

Open `crates/kebab-app/tests/schema_report.rs`. Find `schema_report_reflects_freshly_ingested_kb`. Find the line asserting `streaming_ask: false`. Add nearby:

```rust
    assert!(schema.capabilities.mcp_server, "mcp_server should be true after fb-30");
```

- [ ] **Step 3: Run tests**

```bash
cargo test -p kebab-app --test schema_report 2>&1 | tail -10
cargo test -p kebab-cli --lib wire::tests 2>&1 | tail -10
```

Expected: both pass. (The wire schema test in kebab-cli builds a SchemaV1 fixture but doesn't check `mcp_server` — should be fine.)

- [ ] **Step 4: Commit**

```bash
git add crates/kebab-app/src/schema.rs crates/kebab-app/tests/schema_report.rs
git commit -m "✨ feat(kebab-app): capability flag mcp_server: false → true (fb-30)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 12 — Doc sync

**Files:**
- Modify: `README.md`
- Modify: `HANDOFF.md`
- Modify: `CLAUDE.md`
- Modify: `integrations/claude-code/kebab/SKILL.md`
- Modify: `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md`

- [ ] **Step 1: README — commands table + MCP usage section**

Open `README.md`. Find the `## 명령` table. Add row:

```markdown
| `kebab mcp` | MCP (Model Context Protocol) stdio server. agent host (Claude Code / Cursor / OpenAI Agents) 가 spawn 하여 tool 호출 (`search` / `ask` / `schema` / `doctor`). `--config` honor. |
```

After the commands table (or near the existing "wire schema" / configuration section), add a new subsection:

```markdown
## MCP 사용 (Claude Code 예시)

`~/.claude/mcp.json` (또는 host 의 동등 위치):

```json
{
  "mcpServers": {
    "kebab": {
      "command": "kebab",
      "args": ["mcp"]
    }
  }
}
```

Claude Code 가 session 시작 시 `kebab mcp` 를 spawn — process 가 session 동안 살아 있어 SQLite / Lance / fastembed 가 hot. 4 tool: `search` (lexical/vector/hybrid 검색), `ask` (RAG 답변), `schema` (capability 조회), `doctor` (health check). 모든 tool 의 결과는 wire schema v1 JSON 으로 text content 안에 직렬화 — agent 가 parse 후 사용.
```

- [ ] **Step 2: HANDOFF entry**

Open `HANDOFF.md`. In the `## 머지 후 발견된 결정 (요약)` section (the bulleted list), add at the top:

```markdown
- **2026-05-?? P9 post-도그푸딩 (p9-fb-30)** — `kebab mcp` 신규 subcommand + new crate `kebab-mcp` (lib only) — stdio JSON-RPC server. 4 read-only tool (`search` / `ask` / `schema` / `doctor`) 가 `kebab-app` facade 위에 build. rmcp 1.6 SDK 채택. `error_classify` 모듈을 `kebab-cli` → `kebab-app::error_wire` 로 promotion (UI crate 끼리 import 회피, facade 룰 준수) — kebab-cli + kebab-mcp 둘 다 동일 모듈 사용. capability flag `mcp_server` `false` → `true`. agent integration MVP 완성 — Claude Code / Cursor / OpenAI Agents 등 host-agnostic 사용 가능. spec: `tasks/p9/p9-fb-30-mcp-server.md`. design: `docs/superpowers/specs/2026-05-07-p9-fb-30-mcp-server-design.md`.
```

- [ ] **Step 3: CLAUDE.md facade rule update**

Open `/Users/user/Workspace/projects/kebab/CLAUDE.md`. Find the facade rule section:

```markdown
- UI crates (`kebab-cli`, future `kebab-tui`, `kebab-desktop`) MUST NOT import `kebab-store-*` / `kebab-llm-*` / `kebab-parse-*` directly — only `kebab-app`.
```

Update to include `kebab-mcp`:

```markdown
- UI crates (`kebab-cli`, `kebab-mcp`, `kebab-tui`, future `kebab-desktop`) MUST NOT import `kebab-store-*` / `kebab-llm-*` / `kebab-parse-*` directly — only `kebab-app`.
```

(`kebab-tui` may already be in the list — adjust.)

- [ ] **Step 4: Integrations skill — MCP usage**

Open `integrations/claude-code/kebab/SKILL.md`. After the "Capability discovery" section, add:

```markdown
## MCP server (recommended over CLI subprocess wrapping)

Since v0.4.0, `kebab` exposes an MCP (Model Context Protocol) stdio server. Configure once in `~/.claude/mcp.json`:

```json
{
  "mcpServers": {
    "kebab": {
      "command": "kebab",
      "args": ["mcp"]
    }
  }
}
```

Claude Code spawns `kebab mcp` at session start; the process stays alive across all tool calls so SQLite / Lance / fastembed are hot after the first call. 4 tools available: `search` / `ask` / `schema` / `doctor`. Same wire shapes as the CLI `--json` mode — see `Two surfaces, pick the right one` above for the same guidance.

If your host doesn't support MCP, the CLI subprocess pattern (`kebab search --json` / `kebab ask --json`) above continues to work.
```

- [ ] **Step 5: Design §10 update**

Open `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md`. Find §10.1 (added by fb-27). After it, add §10.2:

```markdown
### 10.2 MCP server transport (fb-30)

`kebab mcp` 가 stdio JSON-RPC server. Rust SDK = `rmcp 1.6`. Tool surface
v1: `search` / `ask` / `schema` / `doctor` (4 read-only). Resources /
Prompts / Sampling 미선언. Output 은 wire schema v1 JSON 을 MCP `text`
content block 으로 직렬화. Tool dispatch 실패는 `isError: true` + error.v1
content; refusal / no-hit / unhealthy 는 정상 응답 (semantic flag 으로
agent 가 분기). HTTP-SSE transport 는 fb-29 deferral 따라 P+. classify
모듈은 `kebab-app::error_wire` 에 single source — kebab-cli + kebab-mcp
공유.
```

- [ ] **Step 6: Commit**

```bash
git add README.md HANDOFF.md CLAUDE.md integrations/claude-code/kebab/SKILL.md docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
git commit -m "$(cat <<'EOF'
📝 docs: sync README / HANDOFF / CLAUDE / skill / design for fb-30

- README 명령 표 에 `kebab mcp` 추가 + Claude Code MCP config 예시
- HANDOFF post-도그푸딩 항목 한 줄
- CLAUDE.md facade 룰 의 UI crate 카테고리 에 `kebab-mcp` 추가
- integrations skill — MCP 사용 안내 (recommended over subprocess)
- design §10.2 MCP transport 절 신설

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 13 — HOTFIXES + task spec status flip

**Files:**
- Modify: `tasks/HOTFIXES.md`
- Modify: `tasks/p9/p9-fb-30-mcp-server.md`

- [ ] **Step 1: HOTFIXES entry**

Open `tasks/HOTFIXES.md`. Insert at top (after the opening explanation paragraphs, before the most recent existing entry):

```markdown
## 2026-05-?? — p9-fb-30 (post-dogfooding): MCP server (stdio) — agent integration MVP

**Source feedback**: 사용자 도그푸딩 2026-05-06 — Claude Code 같은 AI agent 가 kebab CLI 를 사용하는 것이 궁극 목표. 현재 surface 는 Claude Code 전용 skill (subprocess wrapper) 만 — host 무관 표준 통신 없음. fb-29 HTTP daemon 은 single-user local-first 환경 대비 비대로 deferred (2026-05-07), fb-30 stdio MCP 가 동일 사용자 가치 (agent integration + session 동안 hot cache) 를 daemon 복잡도 없이 제공.

**Live binding 변경**:

- 신규 subcommand `kebab mcp` — stdio JSON-RPC server, `--config <path>` honor.
- 신규 crate `kebab-mcp` (lib only) — `serve_stdio(Config)` entry. UI crate 카테고리 (kebab-cli + kebab-tui + kebab-mcp 가 facade 룰 동일 적용 — `kebab-app` facade 만 import).
- Tool surface v1 (read-only 4): `search` (lexical/vector/hybrid 검색), `ask` (RAG 답변, optional `session_id` for multi-turn), `schema` (introspection), `doctor` (health check). `ingest_*` / `fetch` / `list_docs` / `inspect_chunk` 는 fb-31 / fb-35 / 후속 task 머지 시 추가.
- Resources / Prompts / Sampling — 모두 미선언 (tools-only v1).
- Output: 모든 tool 이 wire schema v1 JSON 을 MCP `text` content block 으로 직렬화. CLI `--json` 모드와 동일 wire — single source.
- Error mapping: tool dispatch `Err(e)` 만 `isError: true` + error.v1 content. Refusal (`grounded: false`) / no-hit (empty array) / unhealthy (`ok: false`) 는 모두 정상 응답 — agent 가 wire payload semantic flag 으로 분기.
- `kebab-app::error_wire` 신규 — fb-27 의 `kebab-cli::error_classify` 코드 그대로 promotion (struct + classify + classify_llm + 7 unit test). kebab-cli + kebab-mcp 둘 다 동일 모듈 사용. reqwest dev-dep 도 함께 이동.
- `kebab-app::Capabilities::mcp_server`: `false` → `true`. `schema_report` 통합 테스트 1줄 갱신.
- Initialize handshake: `protocolVersion = <rmcp 가 pin 하는 version>`, `capabilities.tools = { listChanged: false }`, `serverInfo = { name: "kebab", version: <CARGO_PKG_VERSION> }`.

**Spec contract impact**: design §10 에 §10.2 MCP transport 절 추가.

**Tests added**: kebab-mcp unit (1: error helper), kebab-mcp integration (5: tools_call_search / tools_call_ask / tools_call_schema / tools_call_doctor / error_mapping + 1: tools_list 가 가능하면), kebab-cli integration (1: cli_mcp_smoke spawn + initialize + tools/list round-trip). 약 7-8 신규 테스트.

**Known limitation (deferred)**:

- HTTP-SSE transport — fb-29 P+ deferral 따라 stdio 단일. browser agent / remote 시나리오 등장 시 재개.
- Resources (`kebab://chunk/<id>` URI) — fb-35 verbatim fetch 와 함께 v2.
- Prompts — RAG 자체 prompt template 내장으로 사용자 가치 약함, defer.
- Streaming `ask` — fb-33 streaming ask 와 함께.
- `ingest_*` / `fetch` / `list_docs` / `inspect_chunk` tools — 후속 task 별로 추가.
- Server-scope state caching — 현재 매 tool call 마다 store open. 첫 call 시 `KebabAppState` 에 `OnceLock<SqliteStore>` 도입 검토 (post-merge 후속 PR).
- rmcp SDK API 호환성 — 1.6 채택, 미래 major bump 시 별 task.

**Amends**:
- design §10 (MCP transport subsection 추가).
- spec `tasks/p9/p9-fb-30-mcp-server.md` (status `open` → `completed`).
- spec stub 의 `transport: stdio default + http (fb-29 daemon) 위에 SSE 옵션` → 실제 채택 stdio 단일 (fb-29 deferral 결과).
```

- [ ] **Step 2: Flip task spec status**

Open `tasks/p9/p9-fb-30-mcp-server.md`. Change frontmatter:

```yaml
status: open
```

to:

```yaml
status: completed
```

Replace the warning banner at the top:

```markdown
> ⏳ **백로그 only — 미구현.** ...
```

with:

```markdown
> ✅ **구현 완료.** 본 spec 은 구현 시점의 frozen 상태. post-merge deviation 은 [HOTFIXES.md](../HOTFIXES.md) 의 `2026-05-?? — p9-fb-30` 항목 참조 — live source of truth.
```

- [ ] **Step 3: Commit**

```bash
git add tasks/HOTFIXES.md tasks/p9/p9-fb-30-mcp-server.md
git commit -m "📝 docs(tasks): HOTFIXES entry + p9-fb-30 status → completed

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 14 — Final workspace verification

No code changes — verification only.

- [ ] **Step 1: Workspace clippy**

```bash
cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -10
```

Expected: clean, zero warnings.

- [ ] **Step 2: Workspace test (single-thread linker)**

```bash
cargo test --workspace --no-fail-fast -j 1 2>&1 | tail -40
```

Expected: all kebab-* tests PASS. Known pre-existing failures (`kebab-app::reset::tests::enumerate_*`) are env-dependent (XDG_CONFIG_HOME) — accept if those are the ONLY failures.

- [ ] **Step 3: Manual end-to-end smoke (Claude Code MCP)**

```bash
# Verify kebab mcp boots + responds.
printf '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"smoke","version":"0"}}}\n{"jsonrpc":"2.0","method":"notifications/initialized"}\n{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}\n{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"schema","arguments":{}}}\n' | target/debug/kebab mcp 2>/dev/null | head -4
```

Expected: 3 JSON responses (initialize / tools/list / tools/call schema). Last one wraps schema.v1 inside MCP CallToolResult shape with text content.

If the user has Claude Code installed with MCP config pointed at this binary, an end-to-end agent-driven smoke is also valuable.

- [ ] **Step 4: No commit — verification confirms prior commits**

---

## Self-review checklist (run after Task 14)

- [ ] Spec section 1 (`kebab mcp` subcommand + crate boundary) — Tasks 2 + 10. ✅
- [ ] Spec section 2 (4 tool catalog) — Tasks 4 / 5 / 6 / 7. ✅
- [ ] Spec section 3 (lifecycle / error mapping / classify promotion) — Tasks 1 + 3 + 8. ✅
- [ ] Spec section 4 (testing strategy) — Tasks 4-9 + 14. ✅
- [ ] Spec section "doc sync" — Task 12. ✅
- [ ] Spec section "release trigger" — handled separately by version bump PR after merge (not in this plan). ✅
- [ ] Capability flag flip — Task 11. ✅
- [ ] HOTFIXES + status flip — Task 13. ✅

If anything missed, add the task before declaring the plan ready.

---

## rmcp 1.6 caveats (verify at Task 3)

The plan assumes rmcp 1.6 has these surface elements:
- `ServerHandler` trait with `get_info()`
- `transport::stdio()` returning a transport object
- `transport::serve_in_memory(handler)` for testing (Task 9)
- `#[tool_router]` / `#[tool]` / `#[tool_handler]` macros (Tasks 4-7)
- `Parameters<T: JsonSchema>` extractor for tool inputs
- `CallToolResult::error(...)` / `success(...)` constructors

If any of these differ in rmcp 1.6 reality, fall back to:
- Hand-roll `tools/list` + `tools/call` dispatch in lib.rs
- Manual `serde_json::Value` instead of `JsonSchema` derive
- Spawn-based smoke test instead of in-memory transport

Document the deviation in HOTFIXES if it affects the wire shape (tool name / inputSchema / output content).
