//! # mnemo-mcp
//!
//! MCP (Model Context Protocol) server for Mnemo. Exposes Mnemo's memory,
//! retrieval, agent identity, and graph query capabilities as MCP tools
//! that any MCP-compatible client (Claude, GPT, Cursor, etc.) can invoke.
//!
//! ## Architecture
//!
//! The MCP server is an HTTP client adapter — it reads JSON-RPC 2.0 messages
//! from stdin, dispatches them to a running Mnemo HTTP server via reqwest,
//! and writes JSON-RPC responses to stdout. This means:
//!
//! - Zero crate-level dependency on mnemo-server internals
//! - Works with any Mnemo deployment (local, Docker, cloud)
//! - stdio transport for Claude Code / Cursor integration
//!
//! ## Protocol
//!
//! MCP uses JSON-RPC 2.0 with newline-delimited messages over stdio.
//! The lifecycle:
//! 1. Client sends `initialize` request
//! 2. Server responds with capabilities (tools, resources)
//! 3. Client sends `initialized` notification
//! 4. Client calls `tools/call`, `resources/read`, etc.

pub mod protocol;
pub mod tools;
pub mod resources;
pub mod transport;

/// Configuration for the MCP server.
#[derive(Debug, Clone)]
pub struct McpConfig {
    /// Base URL of the Mnemo HTTP server (e.g., "http://localhost:3000").
    pub mnemo_base_url: String,
    /// Optional API key for authenticating with the Mnemo server.
    pub api_key: Option<String>,
    /// Default user identifier for tools that require a user context.
    pub default_user: Option<String>,
}

impl McpConfig {
    pub fn from_env() -> Self {
        Self {
            mnemo_base_url: std::env::var("MNEMO_MCP_BASE_URL")
                .unwrap_or_else(|_| "http://localhost:3000".to_string()),
            api_key: std::env::var("MNEMO_API_KEY").ok(),
            default_user: std::env::var("MNEMO_MCP_DEFAULT_USER").ok(),
        }
    }
}

/// Shared state for the MCP server.
pub struct McpServer {
    pub config: McpConfig,
    pub http: reqwest::Client,
}

impl McpServer {
    pub fn new(config: McpConfig) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("failed to build HTTP client");
        Self { config, http }
    }

    /// Build a GET request with auth headers.
    pub fn get(&self, path: &str) -> reqwest::RequestBuilder {
        let url = format!("{}{}", self.config.mnemo_base_url, path);
        let mut req = self.http.get(&url);
        if let Some(ref key) = self.config.api_key {
            req = req.header("X-Api-Key", key);
        }
        req
    }

    /// Build a POST request with auth headers and JSON body.
    pub fn post(&self, path: &str) -> reqwest::RequestBuilder {
        let url = format!("{}{}", self.config.mnemo_base_url, path);
        let mut req = self.http.post(&url);
        if let Some(ref key) = self.config.api_key {
            req = req.header("X-Api-Key", key);
        }
        req.header("Content-Type", "application/json")
    }

    /// Resolve a user identifier: use the provided one or fall back to default.
    pub fn resolve_user<'a>(&'a self, provided: Option<&'a str>) -> Result<&'a str, String> {
        provided
            .or(self.config.default_user.as_deref())
            .ok_or_else(|| {
                "No user specified and MNEMO_MCP_DEFAULT_USER not set. \
                 Provide a 'user' argument or set the environment variable."
                    .to_string()
            })
    }
}

/// MCP protocol version we implement.
pub const MCP_PROTOCOL_VERSION: &str = "2025-03-26";

/// Server info returned in the initialize response.
pub const SERVER_NAME: &str = "mnemo-mcp-server";
pub const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");
