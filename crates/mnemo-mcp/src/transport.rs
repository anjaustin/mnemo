//! Stdio transport for the MCP server.
//!
//! Reads newline-delimited JSON-RPC messages from stdin, dispatches them,
//! and writes responses to stdout. This is the standard MCP transport for
//! local tool integration (Claude Code, Cursor, etc.).

use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::protocol::*;
use crate::{McpServer, MCP_PROTOCOL_VERSION, SERVER_NAME, SERVER_VERSION};

/// Run the MCP stdio transport loop.
///
/// Reads JSON-RPC messages from stdin (one per line), dispatches them,
/// and writes responses to stdout (one per line).
pub async fn run_stdio(server: Arc<McpServer>) {
    let stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    let reader = BufReader::new(stdin);
    let mut lines = reader.lines();

    while let Ok(Some(line)) = lines.next_line().await {
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }

        let response = handle_message(&server, &line).await;

        if let Some(resp_json) = response {
            let mut output = resp_json;
            output.push('\n');
            if let Err(e) = stdout.write_all(output.as_bytes()).await {
                tracing::error!("Failed to write to stdout: {}", e);
                break;
            }
            if let Err(e) = stdout.flush().await {
                tracing::error!("Failed to flush stdout: {}", e);
                break;
            }
        }
    }
}

/// Handle a single JSON-RPC message and return the response (if any).
///
/// Returns `None` for notifications (which don't get responses).
pub async fn handle_message(server: &McpServer, line: &str) -> Option<String> {
    let request: JsonRpcRequest = match serde_json::from_str(line) {
        Ok(req) => req,
        Err(_) => {
            let err = JsonRpcError::parse_error();
            return Some(serde_json::to_string(&err).unwrap_or_default());
        }
    };

    // Notifications (no id) don't get responses
    let id = match request.id {
        Some(ref id) => id.clone(),
        None => {
            // Handle notification silently
            tracing::debug!(method = %request.method, "Received notification");
            return None;
        }
    };

    let response_value = dispatch_method(server, &request.method, request.params.as_ref(), &id).await;
    Some(response_value)
}

async fn dispatch_method(
    server: &McpServer,
    method: &str,
    params: Option<&serde_json::Value>,
    id: &serde_json::Value,
) -> String {
    match method {
        "initialize" => handle_initialize(id),

        "tools/list" => handle_tools_list(id),

        "tools/call" => handle_tools_call(server, params, id).await,

        "resources/list" => handle_resources_list(id),

        "resources/templates/list" => handle_resource_templates_list(id),

        "resources/read" => handle_resources_read(server, params, id).await,

        "ping" => {
            let resp = JsonRpcResponse::new(id.clone(), serde_json::json!({}));
            serde_json::to_string(&resp).unwrap_or_default()
        }

        _ => {
            let err = JsonRpcError::method_not_found(id.clone(), method);
            serde_json::to_string(&err).unwrap_or_default()
        }
    }
}

fn handle_initialize(id: &serde_json::Value) -> String {
    let result = InitializeResult {
        protocol_version: MCP_PROTOCOL_VERSION.to_string(),
        capabilities: ServerCapabilities {
            tools: ToolsCapability {
                list_changed: false,
            },
            resources: ResourcesCapability {
                list_changed: false,
                subscribe: false,
            },
        },
        server_info: ServerInfo {
            name: SERVER_NAME.to_string(),
            version: SERVER_VERSION.to_string(),
        },
    };
    let resp = JsonRpcResponse::new(
        id.clone(),
        serde_json::to_value(&result).unwrap_or_default(),
    );
    serde_json::to_string(&resp).unwrap_or_default()
}

fn handle_tools_list(id: &serde_json::Value) -> String {
    let tools = crate::tools::list_tools();
    let resp = JsonRpcResponse::new(
        id.clone(),
        serde_json::json!({ "tools": tools }),
    );
    serde_json::to_string(&resp).unwrap_or_default()
}

async fn handle_tools_call(
    server: &McpServer,
    params: Option<&serde_json::Value>,
    id: &serde_json::Value,
) -> String {
    let params = match params {
        Some(p) => p,
        None => {
            let err = JsonRpcError::invalid_params(id.clone(), "Missing params for tools/call");
            return serde_json::to_string(&err).unwrap_or_default();
        }
    };

    let call_params: ToolCallParams = match serde_json::from_value(params.clone()) {
        Ok(p) => p,
        Err(e) => {
            let err = JsonRpcError::invalid_params(
                id.clone(),
                format!("Invalid tools/call params: {}", e),
            );
            return serde_json::to_string(&err).unwrap_or_default();
        }
    };

    let arguments = call_params
        .arguments
        .unwrap_or_else(|| serde_json::json!({}));
    let result = crate::tools::dispatch_tool(server, &call_params.name, &arguments).await;

    let resp = JsonRpcResponse::new(
        id.clone(),
        serde_json::to_value(&result).unwrap_or_default(),
    );
    serde_json::to_string(&resp).unwrap_or_default()
}

fn handle_resources_list(id: &serde_json::Value) -> String {
    // Static resources are not listed (we use templates); return empty.
    let resp = JsonRpcResponse::new(
        id.clone(),
        serde_json::json!({ "resources": [] }),
    );
    serde_json::to_string(&resp).unwrap_or_default()
}

fn handle_resource_templates_list(id: &serde_json::Value) -> String {
    let templates = crate::resources::list_resource_templates();
    let resp = JsonRpcResponse::new(
        id.clone(),
        serde_json::json!({ "resourceTemplates": templates }),
    );
    serde_json::to_string(&resp).unwrap_or_default()
}

async fn handle_resources_read(
    server: &McpServer,
    params: Option<&serde_json::Value>,
    id: &serde_json::Value,
) -> String {
    let params = match params {
        Some(p) => p,
        None => {
            let err =
                JsonRpcError::invalid_params(id.clone(), "Missing params for resources/read");
            return serde_json::to_string(&err).unwrap_or_default();
        }
    };

    let read_params: ResourceReadParams = match serde_json::from_value(params.clone()) {
        Ok(p) => p,
        Err(e) => {
            let err = JsonRpcError::invalid_params(
                id.clone(),
                format!("Invalid resources/read params: {}", e),
            );
            return serde_json::to_string(&err).unwrap_or_default();
        }
    };

    match crate::resources::read_resource(server, &read_params.uri).await {
        Ok(result) => {
            let resp = JsonRpcResponse::new(
                id.clone(),
                serde_json::to_value(&result).unwrap_or_default(),
            );
            serde_json::to_string(&resp).unwrap_or_default()
        }
        Err(e) => {
            let err = JsonRpcError::internal_error(id.clone(), e);
            serde_json::to_string(&err).unwrap_or_default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn test_server() -> Arc<McpServer> {
        Arc::new(McpServer::new(crate::McpConfig {
            mnemo_base_url: "http://localhost:99999".to_string(),
            api_key: None,
            default_user: Some("test-user".to_string()),
        }))
    }

    #[tokio::test]
    async fn test_initialize_returns_capabilities() {
        let server = test_server();
        let msg = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}"#;
        let resp = handle_message(&server, msg).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&resp).unwrap();

        assert_eq!(parsed["jsonrpc"], "2.0");
        assert_eq!(parsed["id"], 1);
        assert!(parsed["result"]["capabilities"]["tools"].is_object());
        assert!(parsed["result"]["capabilities"]["resources"].is_object());
        assert_eq!(parsed["result"]["serverInfo"]["name"], SERVER_NAME);
    }

    #[tokio::test]
    async fn test_tools_list_returns_all_tools() {
        let server = test_server();
        let msg = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#;
        let resp = handle_message(&server, msg).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&resp).unwrap();

        let tools = parsed["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 7);
    }

    #[tokio::test]
    async fn test_resource_templates_list() {
        let server = test_server();
        let msg = r#"{"jsonrpc":"2.0","id":3,"method":"resources/templates/list"}"#;
        let resp = handle_message(&server, msg).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&resp).unwrap();

        let templates = parsed["result"]["resourceTemplates"].as_array().unwrap();
        assert_eq!(templates.len(), 2);
    }

    #[tokio::test]
    async fn test_ping_returns_empty_object() {
        let server = test_server();
        let msg = r#"{"jsonrpc":"2.0","id":4,"method":"ping"}"#;
        let resp = handle_message(&server, msg).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&resp).unwrap();

        assert_eq!(parsed["result"], serde_json::json!({}));
    }

    #[tokio::test]
    async fn test_unknown_method_returns_error() {
        let server = test_server();
        let msg = r#"{"jsonrpc":"2.0","id":5,"method":"nonexistent/method"}"#;
        let resp = handle_message(&server, msg).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&resp).unwrap();

        assert_eq!(parsed["error"]["code"], -32601);
        assert!(parsed["error"]["message"]
            .as_str()
            .unwrap()
            .contains("nonexistent/method"));
    }

    #[tokio::test]
    async fn test_notification_returns_none() {
        let server = test_server();
        // Notification: no "id" field
        let msg = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
        let resp = handle_message(&server, msg).await;
        assert!(resp.is_none(), "Notifications should not produce a response");
    }

    #[tokio::test]
    async fn test_parse_error_on_malformed_json() {
        let server = test_server();
        let msg = r#"this is not json"#;
        let resp = handle_message(&server, msg).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&resp).unwrap();

        assert_eq!(parsed["error"]["code"], -32700);
    }

    #[tokio::test]
    async fn test_tools_call_dispatches_correctly() {
        let server = test_server();
        // Call mnemo_health — will fail to connect but should not panic
        let msg = r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"mnemo_health","arguments":{}}}"#;
        let resp = handle_message(&server, msg).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&resp).unwrap();

        // Should get a tool result (even if it's an error because server is unreachable)
        assert!(parsed["result"]["content"].is_array());
    }

    #[tokio::test]
    async fn test_tools_call_missing_params_returns_error() {
        let server = test_server();
        let msg = r#"{"jsonrpc":"2.0","id":7,"method":"tools/call"}"#;
        let resp = handle_message(&server, msg).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&resp).unwrap();

        assert_eq!(parsed["error"]["code"], -32602);
    }

    #[tokio::test]
    async fn test_resources_read_invalid_uri() {
        let server = test_server();
        let msg = r#"{"jsonrpc":"2.0","id":8,"method":"resources/read","params":{"uri":"https://bad"}}"#;
        let resp = handle_message(&server, msg).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&resp).unwrap();

        // Should return an internal error (invalid URI)
        assert_eq!(parsed["error"]["code"], -32603);
    }

    #[tokio::test]
    async fn test_empty_line_ignored() {
        let server = test_server();
        let resp = handle_message(&server, "").await;
        // Empty lines are handled in run_stdio; handle_message would fail to parse
        // but the run loop skips them. Testing the parse error path here.
        let parsed: serde_json::Value = serde_json::from_str(&resp.unwrap()).unwrap();
        assert_eq!(parsed["error"]["code"], -32700);
    }
}
