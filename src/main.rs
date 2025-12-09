//! Omniscient Code Index MCP Server
//!
//! Entry point for the MCP server that exposes OCI functionality to AI agents.

use anyhow::Result;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<()> {
    // Determine workspace root
    let workspace_root = std::env::var("OCI_WORKSPACE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    // Run the MCP server
    omni_index::mcp::run_server(workspace_root).await
}
