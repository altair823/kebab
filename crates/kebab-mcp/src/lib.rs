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
