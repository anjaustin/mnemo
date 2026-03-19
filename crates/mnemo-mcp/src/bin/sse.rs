//! MCP SSE server binary.
//!
//! Runs the MCP server with HTTP/SSE transport for web-based agents.
//!
//! # Usage
//!
//! ```bash
//! # Run with default settings (localhost:3000)
//! mnemo-mcp-sse
//!
//! # Configure via environment
//! MNEMO_MCP_BASE_URL=http://localhost:8080 \
//! MNEMO_MCP_SSE_HOST=0.0.0.0 \
//! MNEMO_MCP_SSE_PORT=3001 \
//! mnemo-mcp-sse
//! ```
//!
//! # Environment Variables
//!
//! - `MNEMO_MCP_BASE_URL`: Mnemo HTTP server URL (default: http://localhost:8080)
//! - `MNEMO_API_KEY`: API key for Mnemo server authentication
//! - `MNEMO_MCP_DEFAULT_USER`: Default user for memory operations
//! - `MNEMO_MCP_SSE_HOST`: Host to bind SSE server (default: 127.0.0.1)
//! - `MNEMO_MCP_SSE_PORT`: Port to bind SSE server (default: 3000)
//! - `MNEMO_MCP_SSE_CORS`: Comma-separated CORS origins (default: permissive)
//! - `RUST_LOG`: Log level (default: warn)

use std::sync::Arc;

use mnemo_mcp::sse::{run_sse, SseConfig};
use mnemo_mcp::{McpConfig, McpServer};

#[tokio::main]
async fn main() {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    // Load configuration
    let mcp_config = McpConfig::from_env();
    let sse_config = SseConfig::from_env();

    tracing::info!(
        mnemo_url = %mcp_config.mnemo_base_url,
        "Connecting to Mnemo server"
    );

    // Create MCP server
    let server = Arc::new(McpServer::new(mcp_config));

    // Run SSE transport
    if let Err(e) = run_sse(server, sse_config).await {
        tracing::error!("SSE server error: {}", e);
        std::process::exit(1);
    }
}
