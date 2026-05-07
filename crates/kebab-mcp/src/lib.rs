//! MCP (Model Context Protocol) server over stdio. Exposes 4 read-only
//! tools (`search` / `ask` / `schema` / `doctor`) backed by `kebab-app`
//! facade methods. Used by `kebab-cli`'s `Cmd::Mcp` arm.
//!
//! See spec `docs/superpowers/specs/2026-05-07-p9-fb-30-mcp-server-design.md`.

use anyhow::Result;

use rmcp::ServerHandler;
use rmcp::model::{Implementation, ServerCapabilities, ServerInfo};
use rmcp::service::ServiceExt;
use rmcp::transport::stdio;

use kebab_config::Config;

pub mod state;
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
    let state = KebabAppState::new(cfg);
    let handler = KebabHandler::new(state);
    let service = handler.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
