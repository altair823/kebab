//! MCP (Model Context Protocol) server over stdio. Exposes 8 tools
//! (`search` / `ask` / `schema` / `doctor` / `ingest_file` / `ingest_stdin`
//! / `fetch` / `bulk_search`) backed by `kebab-app` facade methods. Used by
//! `kebab-cli`'s `Cmd::Mcp` arm.
//!
//! See spec `docs/superpowers/specs/2026-05-07-p9-fb-30-mcp-server-design.md`.

use std::path::PathBuf;

use anyhow::Result;

use rmcp::ServerHandler;
use rmcp::handler::server::common::{schema_for_empty_input, schema_for_type};
use rmcp::model::{
    CallToolRequestParams, CallToolResult, Implementation, ListToolsResult, ServerCapabilities,
    ServerInfo, Tool,
};
use rmcp::service::{RequestContext, ServiceExt};
use rmcp::transport::stdio;
use rmcp::{ErrorData, RoleServer};

use kebab_config::Config;

pub mod error;
pub mod state;
pub mod tools;
pub use state::KebabAppState;

/// Build the canonical list of tools exposed by the MCP server.
///
/// Extracted from [`ServerHandler::list_tools`] so it can be called
/// directly in tests without constructing a `RequestContext`.
pub fn build_tools_vec() -> Vec<Tool> {
    vec![
        Tool::new(
            "schema",
            "Introspection — wire schemas, capabilities, model versions, index stats.",
            schema_for_empty_input(),
        ),
        Tool::new(
            "doctor",
            "Health check — verifies config, storage, models, and Ollama connectivity.",
            schema_for_empty_input(),
        ),
        Tool::new(
            "search",
            "Full-text / vector / hybrid search over the knowledge base. Returns search_hit.v1 array.",
            schema_for_type::<tools::search::SearchInput>(),
        ),
        Tool::new(
            "ask",
            "RAG question answering over the knowledge base. Returns answer.v1 JSON. Pass session_id for multi-turn context. Set multi_hop=true for compound / cross-doc questions (decompose → retrieve → synthesize; 2-5× LLM cost; per-hop trace on Answer.hops).",
            schema_for_type::<tools::ask::AskInput>(),
        ),
        Tool::new(
            "ingest_file",
            "Ingest a single file (path) into the knowledge base. Workspace external paths allowed — bytes are copied into _external/.",
            schema_for_type::<tools::ingest_file::IngestFileInput>(),
        ),
        Tool::new(
            "ingest_stdin",
            "Ingest markdown content into the knowledge base. v1 markdown only. Frontmatter (title + source_uri) auto-injected.",
            schema_for_type::<tools::ingest_stdin::IngestStdinInput>(),
        ),
        Tool::new(
            "fetch",
            "Verbatim fetch — chunk / doc / span modes. Returns fetch_result.v1 with the indexed text (no LLM rewrite).",
            schema_for_type::<tools::fetch::FetchInput>(),
        ),
        Tool::new(
            "bulk_search",
            "Bulk multi-query search — N queries per call (cap 100). Each query mirrors the `search` input shape; returns `bulk_search_response.v1` with per-query results + summary. Sequential execution reuses one App instance so cache / embedder cold-start cost amortizes.",
            schema_for_type::<tools::bulk_search::BulkSearchInput>(),
        ),
    ]
}

#[derive(Clone)]
pub struct KebabHandler {
    state: KebabAppState,
}

impl KebabHandler {
    pub fn new(state: KebabAppState) -> Self {
        Self { state }
    }

    pub fn state(&self) -> &KebabAppState {
        &self.state
    }

    /// Spawn a tool handler on the blocking pool. Used by tools that
    /// transitively touch reqwest::blocking::Client (search, ask) — calling
    /// from the async dispatch directly panics inside the runtime.
    async fn spawn_tool<I, F>(
        &self,
        args: serde_json::Map<String, serde_json::Value>,
        handle: F,
    ) -> Result<CallToolResult, ErrorData>
    where
        I: serde::de::DeserializeOwned + Send + 'static,
        F: FnOnce(KebabAppState, I) -> CallToolResult + Send + 'static,
    {
        let input: I = match serde_json::from_value(serde_json::Value::Object(args)) {
            Ok(i) => i,
            Err(e) => return Ok(error::to_tool_error(&anyhow::Error::from(e))),
        };
        let state = self.state.clone();
        tokio::task::spawn_blocking(move || handle(state, input))
            .await
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))
    }
}

impl ServerHandler for KebabHandler {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("kebab", env!("CARGO_PKG_VERSION")))
    }

    async fn list_tools(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(build_tools_vec()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        match request.name.as_ref() {
            "schema" => {
                let input = tools::schema::SchemaInput::default();
                Ok(tools::schema::handle(&self.state, input))
            }
            "doctor" => {
                let input = tools::doctor::DoctorInput::default();
                Ok(tools::doctor::handle(&self.state, input))
            }
            "search" => {
                let args = request.arguments.unwrap_or_default();
                self.spawn_tool(args, |state, input| tools::search::handle(&state, input))
                    .await
            }
            "ask" => {
                let args = request.arguments.unwrap_or_default();
                self.spawn_tool(args, |state, input| tools::ask::handle(&state, input))
                    .await
            }
            "ingest_file" => {
                let args = request.arguments.unwrap_or_default();
                self.spawn_tool(args, |state, input| {
                    tools::ingest_file::handle(&state, input)
                })
                .await
            }
            "ingest_stdin" => {
                let args = request.arguments.unwrap_or_default();
                self.spawn_tool(args, |state, input| {
                    tools::ingest_stdin::handle(&state, input)
                })
                .await
            }
            "fetch" => {
                let args = request.arguments.unwrap_or_default();
                self.spawn_tool(args, |state, input| tools::fetch::handle(&state, input))
                    .await
            }
            "bulk_search" => {
                let args = request.arguments.unwrap_or_default();
                self.spawn_tool(args, |state, input| {
                    tools::bulk_search::handle(&state, input)
                })
                .await
            }
            _other => Err(ErrorData::method_not_found::<
                rmcp::model::CallToolRequestMethod,
            >()),
        }
    }
}

/// Run the MCP server on stdio JSON-RPC. Blocks until the client closes
/// the stream (typically when the agent host exits).
///
/// `config_path` is the path passed via `--config <path>`, if any.
/// It is forwarded to `KebabAppState` so the doctor tool can honour the
/// same config file the server was started with (falls back to XDG default
/// when `None`).
pub fn serve_stdio(cfg: Config, config_path: Option<PathBuf>) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(serve_stdio_async(cfg, config_path))
}

async fn serve_stdio_async(cfg: Config, config_path: Option<PathBuf>) -> Result<()> {
    tracing::info!("kebab-mcp: starting stdio server");
    let state = KebabAppState::new(cfg, config_path);
    let handler = KebabHandler::new(state);
    let service = handler.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
