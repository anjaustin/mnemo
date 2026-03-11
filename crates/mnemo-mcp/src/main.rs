//! Mnemo MCP Server — stdio transport binary.
//!
//! Run this as a subprocess for MCP-compatible clients (Claude Code, Cursor, etc.).
//!
//! ## Configuration
//!
//! - `MNEMO_MCP_BASE_URL` — Mnemo HTTP server URL (default: `http://localhost:3000`)
//! - `MNEMO_API_KEY` — Optional API key for Mnemo authentication
//! - `MNEMO_MCP_DEFAULT_USER` — Default user identifier for tools that require one
//! - `RUST_LOG` — Log level (default: `warn`, logs go to stderr)
//!
//! ## Usage
//!
//! ```bash
//! # Run directly
//! mnemo-mcp-server
//!
//! # With configuration
//! MNEMO_MCP_BASE_URL=http://localhost:3000 MNEMO_MCP_DEFAULT_USER=kendra mnemo-mcp-server
//! ```
//!
//! ## Claude Code integration
//!
//! Add to your MCP settings:
//! ```json
//! {
//!   "mcpServers": {
//!     "mnemo": {
//!       "command": "mnemo-mcp-server",
//!       "env": {
//!         "MNEMO_MCP_BASE_URL": "http://localhost:3000",
//!         "MNEMO_MCP_DEFAULT_USER": "your-user-id"
//!       }
//!     }
//!   }
//! }
//! ```

use std::sync::Arc;

use mnemo_mcp::{McpConfig, McpServer};

#[tokio::main]
async fn main() {
    // Initialize logging to stderr (stdout is reserved for MCP protocol)
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .init();

    let config = McpConfig::from_env();

    tracing::info!(
        base_url = %config.mnemo_base_url,
        has_api_key = config.api_key.is_some(),
        default_user = ?config.default_user,
        "Starting Mnemo MCP server (stdio transport)"
    );

    let server = Arc::new(McpServer::new(config));

    mnemo_mcp::transport::run_stdio(server).await;
}
