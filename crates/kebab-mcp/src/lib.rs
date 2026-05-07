//! MCP (Model Context Protocol) server over stdio. Exposes 4 read-only
//! tools (`search` / `ask` / `schema` / `doctor`) backed by `kebab-app`
//! facade methods. Used by `kebab-cli`'s `Cmd::Mcp` arm.
//!
//! See spec `docs/superpowers/specs/2026-05-07-p9-fb-30-mcp-server-design.md`.

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
        Ok(ListToolsResult::with_all_items(vec![
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
                "RAG question answering over the knowledge base. Returns answer.v1 JSON. Pass session_id for multi-turn context.",
                schema_for_type::<tools::ask::AskInput>(),
            ),
        ]))
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
                let input: tools::search::SearchInput =
                    match serde_json::from_value(serde_json::Value::Object(args)) {
                        Ok(i) => i,
                        Err(e) => {
                            return Ok(error::to_tool_error(&anyhow::Error::from(e)));
                        }
                    };
                Ok(tools::search::handle(&self.state, input))
            }
            "ask" => {
                let args = request.arguments.unwrap_or_default();
                let input: tools::ask::AskInput =
                    match serde_json::from_value(serde_json::Value::Object(args)) {
                        Ok(i) => i,
                        Err(e) => {
                            return Ok(error::to_tool_error(&anyhow::Error::from(e)));
                        }
                    };
                Ok(tools::ask::handle(&self.state, input))
            }
            _other => Err(ErrorData::method_not_found::<
                rmcp::model::CallToolRequestMethod,
            >()),
        }
    }
}

/// Run the MCP server on stdio JSON-RPC. Blocks until the client closes
/// the stream (typically when the agent host exits).
pub fn serve_stdio(cfg: Config) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(serve_stdio_async(cfg))
}

async fn serve_stdio_async(cfg: Config) -> Result<()> {
    tracing::info!("kebab-mcp: starting stdio server");
    let state = KebabAppState::new(cfg, None); // Plan Task 10 will thread the actual path
    let handler = KebabHandler::new(state);
    let service = handler.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
