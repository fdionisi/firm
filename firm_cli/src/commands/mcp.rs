use anyhow::Result;
use firm_core::graph::EntityGraph;
use firm_mcp::FirmMcpServer;
use rmcp::{ServiceExt, transport::stdio};
use std::path::Path;

use crate::files::load_current_graph;

/// Start the MCP server with the given configuration.
pub async fn start_mcp_server(workspace_path: &Path) -> Result<()> {
    let graph = load_graph_or_create_empty(workspace_path)?;

    let server = FirmMcpServer::new(graph);

    start_stdio_server(server).await
}

async fn start_stdio_server(server: FirmMcpServer) -> Result<()> {
    let service = server
        .serve(stdio())
        .await
        .map_err(|e| anyhow::anyhow!("Failed to start MCP server: {:?}", e))?;

    match service.waiting().await {
        Ok(_) => Ok(()),
        Err(e) => Err(anyhow::anyhow!("MCP server error: {:?}", e)),
    }
}

fn load_graph_or_create_empty(workspace_path: &Path) -> Result<EntityGraph> {
    match load_current_graph(&workspace_path.to_path_buf()) {
        Ok(graph) => Ok(graph),
        Err(_) => Ok(EntityGraph::new()),
    }
}
