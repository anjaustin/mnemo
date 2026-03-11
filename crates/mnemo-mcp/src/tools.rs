//! MCP tool definitions and dispatch for Mnemo.
//!
//! Each tool maps to one or more Mnemo HTTP API calls.

use serde_json::Value;

use crate::protocol::{ToolCallResult, ToolDefinition};
use crate::McpServer;

/// Validate that an identifier is safe for URL path interpolation.
fn validate_path_segment(value: &str, field_name: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err(format!("'{}' cannot be empty", field_name));
    }
    if value.contains("..") || value.contains('/') || value.contains('\\') || value.contains('\0') {
        return Err(format!(
            "'{}' contains illegal characters: '{}'",
            field_name, value
        ));
    }
    if value.len() > 256 {
        return Err(format!("'{}' exceeds maximum length of 256 characters", field_name));
    }
    Ok(())
}

/// Return all tool definitions.
pub fn list_tools() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "mnemo_remember".to_string(),
            description: "Store a memory for a user. The text will be processed, entities and \
                          relationships extracted, and the knowledge graph updated."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "user": {
                        "type": "string",
                        "description": "User identifier (UUID, external_id, or name). Falls back to MNEMO_MCP_DEFAULT_USER if omitted."
                    },
                    "text": {
                        "type": "string",
                        "description": "The memory text to store."
                    },
                    "session": {
                        "type": "string",
                        "description": "Optional session name (default: 'default')."
                    }
                },
                "required": ["text"]
            }),
        },
        ToolDefinition {
            name: "mnemo_recall".to_string(),
            description: "Retrieve context from a user's memory for a given query. Returns \
                          relevant entities, facts, and episodes assembled into a context block."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "user": {
                        "type": "string",
                        "description": "User identifier."
                    },
                    "query": {
                        "type": "string",
                        "description": "The query to recall context for."
                    },
                    "session": {
                        "type": "string",
                        "description": "Optional session name."
                    },
                    "max_tokens": {
                        "type": "integer",
                        "description": "Maximum tokens in the assembled context (default: 1000)."
                    }
                },
                "required": ["query"]
            }),
        },
        ToolDefinition {
            name: "mnemo_graph_query".to_string(),
            description: "Query the knowledge graph. List entities, edges, find shortest paths, \
                          or detect communities for a user."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "user": {
                        "type": "string",
                        "description": "User identifier."
                    },
                    "operation": {
                        "type": "string",
                        "enum": ["list_entities", "list_edges", "communities"],
                        "description": "Graph operation to perform."
                    },
                    "entity_type": {
                        "type": "string",
                        "description": "Filter entities by type (for list_entities)."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum results (default: 50)."
                    }
                },
                "required": ["operation"]
            }),
        },
        ToolDefinition {
            name: "mnemo_agent_identity".to_string(),
            description: "Get or update an agent's identity profile. Supports reading the full \
                          profile or updating specific fields."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "agent_id": {
                        "type": "string",
                        "description": "Agent UUID."
                    },
                    "action": {
                        "type": "string",
                        "enum": ["get", "update"],
                        "description": "Action to perform on the identity."
                    },
                    "update": {
                        "type": "object",
                        "description": "Fields to update (only for action=update). See Mnemo API docs for schema."
                    }
                },
                "required": ["agent_id", "action"]
            }),
        },
        ToolDefinition {
            name: "mnemo_digest".to_string(),
            description: "Get or generate a prose memory digest for a user. The digest summarizes \
                          the user's knowledge graph into a human-readable narrative."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "user": {
                        "type": "string",
                        "description": "User identifier."
                    },
                    "action": {
                        "type": "string",
                        "enum": ["get", "generate"],
                        "description": "'get' retrieves cached digest, 'generate' creates a new one (requires LLM)."
                    }
                },
                "required": ["action"]
            }),
        },
        ToolDefinition {
            name: "mnemo_coherence".to_string(),
            description: "Get a coherence report for a user's knowledge graph. Measures internal \
                          consistency across entity, fact, temporal, and structural dimensions."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "user": {
                        "type": "string",
                        "description": "User identifier."
                    }
                },
                "required": []
            }),
        },
        ToolDefinition {
            name: "mnemo_health".to_string(),
            description: "Check the health of the Mnemo server.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        },
    ]
}

/// Dispatch a tool call to the appropriate handler.
pub async fn dispatch_tool(
    server: &McpServer,
    tool_name: &str,
    arguments: &Value,
) -> ToolCallResult {
    match tool_name {
        "mnemo_remember" => handle_remember(server, arguments).await,
        "mnemo_recall" => handle_recall(server, arguments).await,
        "mnemo_graph_query" => handle_graph_query(server, arguments).await,
        "mnemo_agent_identity" => handle_agent_identity(server, arguments).await,
        "mnemo_digest" => handle_digest(server, arguments).await,
        "mnemo_coherence" => handle_coherence(server, arguments).await,
        "mnemo_health" => handle_health(server).await,
        _ => ToolCallResult::error(format!("Unknown tool: {}", tool_name)),
    }
}

// ─── Tool handlers ────────────────────────────────────────────────

async fn handle_remember(server: &McpServer, args: &Value) -> ToolCallResult {
    let text = match args.get("text").and_then(|v| v.as_str()) {
        Some(t) if !t.trim().is_empty() => t,
        _ => return ToolCallResult::error("'text' argument is required and must be non-empty"),
    };

    let user = match server.resolve_user(args.get("user").and_then(|v| v.as_str())) {
        Ok(u) => u.to_string(),
        Err(e) => return ToolCallResult::error(e),
    };

    if let Err(e) = validate_path_segment(&user, "user") {
        return ToolCallResult::error(e);
    }

    let session = args
        .get("session")
        .and_then(|v| v.as_str())
        .unwrap_or("default");

    let body = serde_json::json!({
        "user": user,
        "text": text,
        "session": session,
    });

    match server
        .post("/api/v1/memory")
        .json(&body)
        .send()
        .await
    {
        Ok(resp) => {
            let status = resp.status();
            match resp.json::<Value>().await {
                Ok(json) if status.is_success() => ToolCallResult::json(&json),
                Ok(json) => ToolCallResult::error(format!(
                    "Mnemo API error ({}): {}",
                    status,
                    serde_json::to_string_pretty(&json).unwrap_or_default()
                )),
                Err(e) => ToolCallResult::error(format!("Failed to parse response: {}", e)),
            }
        }
        Err(e) => ToolCallResult::error(format!("HTTP request failed: {}", e)),
    }
}

async fn handle_recall(server: &McpServer, args: &Value) -> ToolCallResult {
    let query = match args.get("query").and_then(|v| v.as_str()) {
        Some(q) if !q.trim().is_empty() => q,
        _ => return ToolCallResult::error("'query' argument is required and must be non-empty"),
    };

    let user = match server.resolve_user(args.get("user").and_then(|v| v.as_str())) {
        Ok(u) => u.to_string(),
        Err(e) => return ToolCallResult::error(e),
    };

    if let Err(e) = validate_path_segment(&user, "user") {
        return ToolCallResult::error(e);
    }

    let session = args
        .get("session")
        .and_then(|v| v.as_str())
        .unwrap_or("default");
    let max_tokens = args
        .get("max_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(1000);

    let body = serde_json::json!({
        "query": query,
        "session": session,
        "max_tokens": max_tokens,
    });

    let path = format!("/api/v1/memory/{}/context", user);
    match server.post(&path).json(&body).send().await {
        Ok(resp) => {
            let status = resp.status();
            match resp.json::<Value>().await {
                Ok(json) if status.is_success() => ToolCallResult::json(&json),
                Ok(json) => ToolCallResult::error(format!(
                    "Mnemo API error ({}): {}",
                    status,
                    serde_json::to_string_pretty(&json).unwrap_or_default()
                )),
                Err(e) => ToolCallResult::error(format!("Failed to parse response: {}", e)),
            }
        }
        Err(e) => ToolCallResult::error(format!("HTTP request failed: {}", e)),
    }
}

async fn handle_graph_query(server: &McpServer, args: &Value) -> ToolCallResult {
    let operation = match args.get("operation").and_then(|v| v.as_str()) {
        Some(op) => op,
        None => return ToolCallResult::error("'operation' argument is required"),
    };

    let user = match server.resolve_user(args.get("user").and_then(|v| v.as_str())) {
        Ok(u) => u.to_string(),
        Err(e) => return ToolCallResult::error(e),
    };

    if let Err(e) = validate_path_segment(&user, "user") {
        return ToolCallResult::error(e);
    }

    let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(50);

    let path = match operation {
        "list_entities" => {
            let mut p = format!("/api/v1/graph/{}/entities?limit={}", user, limit);
            if let Some(et) = args.get("entity_type").and_then(|v| v.as_str()) {
                p.push_str(&format!("&type={}", et));
            }
            p
        }
        "list_edges" => format!("/api/v1/graph/{}/edges?limit={}", user, limit),
        "communities" => format!("/api/v1/graph/{}/communities", user),
        _ => {
            return ToolCallResult::error(format!(
                "Unknown operation '{}'. Must be one of: list_entities, list_edges, communities",
                operation
            ))
        }
    };

    match server.get(&path).send().await {
        Ok(resp) => {
            let status = resp.status();
            match resp.json::<Value>().await {
                Ok(json) if status.is_success() => ToolCallResult::json(&json),
                Ok(json) => ToolCallResult::error(format!(
                    "Mnemo API error ({}): {}",
                    status,
                    serde_json::to_string_pretty(&json).unwrap_or_default()
                )),
                Err(e) => ToolCallResult::error(format!("Failed to parse response: {}", e)),
            }
        }
        Err(e) => ToolCallResult::error(format!("HTTP request failed: {}", e)),
    }
}

async fn handle_agent_identity(server: &McpServer, args: &Value) -> ToolCallResult {
    let agent_id = match args.get("agent_id").and_then(|v| v.as_str()) {
        Some(id) => id,
        None => return ToolCallResult::error("'agent_id' argument is required"),
    };

    if let Err(e) = validate_path_segment(agent_id, "agent_id") {
        return ToolCallResult::error(e);
    }

    let action = match args.get("action").and_then(|v| v.as_str()) {
        Some(a) => a,
        None => return ToolCallResult::error("'action' argument is required"),
    };

    match action {
        "get" => {
            let path = format!("/api/v1/agents/{}/identity", agent_id);
            match server.get(&path).send().await {
                Ok(resp) => {
                    let status = resp.status();
                    match resp.json::<Value>().await {
                        Ok(json) if status.is_success() => ToolCallResult::json(&json),
                        Ok(json) => ToolCallResult::error(format!(
                            "Mnemo API error ({}): {}",
                            status,
                            serde_json::to_string_pretty(&json).unwrap_or_default()
                        )),
                        Err(e) => ToolCallResult::error(format!("Failed to parse response: {}", e)),
                    }
                }
                Err(e) => ToolCallResult::error(format!("HTTP request failed: {}", e)),
            }
        }
        "update" => {
            let update = match args.get("update") {
                Some(u) => u.clone(),
                None => {
                    return ToolCallResult::error("'update' argument is required for action=update")
                }
            };

            let path = format!("/api/v1/agents/{}/identity", agent_id);
            match server.put(&path).json(&update).send().await {
                Ok(resp) => {
                    let status = resp.status();
                    match resp.json::<Value>().await {
                        Ok(json) if status.is_success() => ToolCallResult::json(&json),
                        Ok(json) => ToolCallResult::error(format!(
                            "Mnemo API error ({}): {}",
                            status,
                            serde_json::to_string_pretty(&json).unwrap_or_default()
                        )),
                        Err(e) => ToolCallResult::error(format!("Failed to parse response: {}", e)),
                    }
                }
                Err(e) => ToolCallResult::error(format!("HTTP request failed: {}", e)),
            }
        }
        _ => ToolCallResult::error(format!("Unknown action '{}'. Must be 'get' or 'update'", action)),
    }
}

async fn handle_digest(server: &McpServer, args: &Value) -> ToolCallResult {
    let user = match server.resolve_user(args.get("user").and_then(|v| v.as_str())) {
        Ok(u) => u.to_string(),
        Err(e) => return ToolCallResult::error(e),
    };

    if let Err(e) = validate_path_segment(&user, "user") {
        return ToolCallResult::error(e);
    }

    let action = match args.get("action").and_then(|v| v.as_str()) {
        Some(a) => a,
        None => return ToolCallResult::error("'action' argument is required"),
    };

    match action {
        "get" => {
            let path = format!("/api/v1/memory/{}/digest", user);
            match server.get(&path).send().await {
                Ok(resp) => {
                    let status = resp.status();
                    match resp.json::<Value>().await {
                        Ok(json) if status.is_success() => ToolCallResult::json(&json),
                        Ok(json) => ToolCallResult::error(format!(
                            "Mnemo API error ({}): {}",
                            status,
                            serde_json::to_string_pretty(&json).unwrap_or_default()
                        )),
                        Err(e) => ToolCallResult::error(format!("Failed to parse response: {}", e)),
                    }
                }
                Err(e) => ToolCallResult::error(format!("HTTP request failed: {}", e)),
            }
        }
        "generate" => {
            let path = format!("/api/v1/memory/{}/digest", user);
            match server.post(&path).json(&serde_json::json!({})).send().await {
                Ok(resp) => {
                    let status = resp.status();
                    match resp.json::<Value>().await {
                        Ok(json) if status.is_success() => ToolCallResult::json(&json),
                        Ok(json) => ToolCallResult::error(format!(
                            "Mnemo API error ({}): {}",
                            status,
                            serde_json::to_string_pretty(&json).unwrap_or_default()
                        )),
                        Err(e) => ToolCallResult::error(format!("Failed to parse response: {}", e)),
                    }
                }
                Err(e) => ToolCallResult::error(format!("HTTP request failed: {}", e)),
            }
        }
        _ => ToolCallResult::error(format!(
            "Unknown action '{}'. Must be 'get' or 'generate'",
            action
        )),
    }
}

async fn handle_coherence(server: &McpServer, args: &Value) -> ToolCallResult {
    let user = match server.resolve_user(args.get("user").and_then(|v| v.as_str())) {
        Ok(u) => u.to_string(),
        Err(e) => return ToolCallResult::error(e),
    };

    if let Err(e) = validate_path_segment(&user, "user") {
        return ToolCallResult::error(e);
    }

    let path = format!("/api/v1/users/{}/coherence", user);
    match server.get(&path).send().await {
        Ok(resp) => {
            let status = resp.status();
            match resp.json::<Value>().await {
                Ok(json) if status.is_success() => ToolCallResult::json(&json),
                Ok(json) => ToolCallResult::error(format!(
                    "Mnemo API error ({}): {}",
                    status,
                    serde_json::to_string_pretty(&json).unwrap_or_default()
                )),
                Err(e) => ToolCallResult::error(format!("Failed to parse response: {}", e)),
            }
        }
        Err(e) => ToolCallResult::error(format!("HTTP request failed: {}", e)),
    }
}

async fn handle_health(server: &McpServer) -> ToolCallResult {
    match server.get("/health").send().await {
        Ok(resp) => {
            let status = resp.status();
            match resp.json::<Value>().await {
                Ok(json) if status.is_success() => ToolCallResult::json(&json),
                Ok(json) => ToolCallResult::error(format!(
                    "Mnemo health check failed ({}): {}",
                    status,
                    serde_json::to_string_pretty(&json).unwrap_or_default()
                )),
                Err(e) => ToolCallResult::error(format!("Failed to parse response: {}", e)),
            }
        }
        Err(e) => ToolCallResult::error(format!(
            "Cannot reach Mnemo server at {}: {}",
            server.config.mnemo_base_url, e
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_list_tools_returns_7_tools() {
        let tools = list_tools();
        assert_eq!(tools.len(), 7);

        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"mnemo_remember"));
        assert!(names.contains(&"mnemo_recall"));
        assert!(names.contains(&"mnemo_graph_query"));
        assert!(names.contains(&"mnemo_agent_identity"));
        assert!(names.contains(&"mnemo_digest"));
        assert!(names.contains(&"mnemo_coherence"));
        assert!(names.contains(&"mnemo_health"));
    }

    #[test]
    fn test_all_tools_have_valid_input_schemas() {
        for tool in list_tools() {
            assert_eq!(
                tool.input_schema["type"], "object",
                "Tool {} should have object input schema",
                tool.name
            );
            assert!(
                tool.input_schema.get("properties").is_some(),
                "Tool {} should have properties in schema",
                tool.name
            );
        }
    }

    #[test]
    fn test_all_tools_have_descriptions() {
        for tool in list_tools() {
            assert!(
                !tool.description.is_empty(),
                "Tool {} should have a description",
                tool.name
            );
            assert!(
                tool.description.len() >= 20,
                "Tool {} description too short: '{}'",
                tool.name,
                tool.description
            );
        }
    }

    #[test]
    fn test_remember_tool_requires_text() {
        let tool = list_tools()
            .into_iter()
            .find(|t| t.name == "mnemo_remember")
            .unwrap();
        let required = tool.input_schema["required"]
            .as_array()
            .unwrap();
        assert!(required.contains(&serde_json::json!("text")));
    }

    #[test]
    fn test_recall_tool_requires_query() {
        let tool = list_tools()
            .into_iter()
            .find(|t| t.name == "mnemo_recall")
            .unwrap();
        let required = tool.input_schema["required"]
            .as_array()
            .unwrap();
        assert!(required.contains(&serde_json::json!("query")));
    }

    #[test]
    fn test_graph_query_operations_documented() {
        let tool = list_tools()
            .into_iter()
            .find(|t| t.name == "mnemo_graph_query")
            .unwrap();
        let ops = tool.input_schema["properties"]["operation"]["enum"]
            .as_array()
            .unwrap();
        assert!(ops.contains(&serde_json::json!("list_entities")));
        assert!(ops.contains(&serde_json::json!("list_edges")));
        assert!(ops.contains(&serde_json::json!("communities")));
    }

    #[tokio::test]
    async fn test_dispatch_unknown_tool_returns_error() {
        let server = McpServer::new(crate::McpConfig {
            mnemo_base_url: "http://localhost:99999".to_string(),
            api_key: None,
            default_user: None,
        });
        let result = dispatch_tool(&server, "nonexistent_tool", &serde_json::json!({})).await;
        assert_eq!(result.is_error, Some(true));
        assert!(result.content[0].text.contains("Unknown tool"));
    }

    #[tokio::test]
    async fn test_remember_rejects_empty_text() {
        let server = McpServer::new(crate::McpConfig {
            mnemo_base_url: "http://localhost:99999".to_string(),
            api_key: None,
            default_user: Some("test-user".to_string()),
        });
        let result = dispatch_tool(
            &server,
            "mnemo_remember",
            &serde_json::json!({"text": ""}),
        )
        .await;
        assert_eq!(result.is_error, Some(true));
        assert!(result.content[0].text.contains("non-empty"));
    }

    #[tokio::test]
    async fn test_recall_rejects_empty_query() {
        let server = McpServer::new(crate::McpConfig {
            mnemo_base_url: "http://localhost:99999".to_string(),
            api_key: None,
            default_user: Some("test-user".to_string()),
        });
        let result = dispatch_tool(
            &server,
            "mnemo_recall",
            &serde_json::json!({"query": "  "}),
        )
        .await;
        assert_eq!(result.is_error, Some(true));
    }

    #[tokio::test]
    async fn test_remember_rejects_missing_user() {
        let server = McpServer::new(crate::McpConfig {
            mnemo_base_url: "http://localhost:99999".to_string(),
            api_key: None,
            default_user: None, // no default
        });
        let result = dispatch_tool(
            &server,
            "mnemo_remember",
            &serde_json::json!({"text": "hello"}),
        )
        .await;
        assert_eq!(result.is_error, Some(true));
        assert!(result.content[0].text.contains("MNEMO_MCP_DEFAULT_USER"));
    }

    #[tokio::test]
    async fn test_graph_query_rejects_unknown_operation() {
        let server = McpServer::new(crate::McpConfig {
            mnemo_base_url: "http://localhost:99999".to_string(),
            api_key: None,
            default_user: Some("test-user".to_string()),
        });
        let result = dispatch_tool(
            &server,
            "mnemo_graph_query",
            &serde_json::json!({"operation": "delete_everything"}),
        )
        .await;
        assert_eq!(result.is_error, Some(true));
        assert!(result.content[0].text.contains("Unknown operation"));
    }

    #[tokio::test]
    async fn test_agent_identity_rejects_missing_fields() {
        let server = McpServer::new(crate::McpConfig {
            mnemo_base_url: "http://localhost:99999".to_string(),
            api_key: None,
            default_user: None,
        });
        // Missing agent_id
        let result = dispatch_tool(
            &server,
            "mnemo_agent_identity",
            &serde_json::json!({"action": "get"}),
        )
        .await;
        assert_eq!(result.is_error, Some(true));

        // Missing action
        let result = dispatch_tool(
            &server,
            "mnemo_agent_identity",
            &serde_json::json!({"agent_id": "abc"}),
        )
        .await;
        assert_eq!(result.is_error, Some(true));
    }

    // ─── Falsification: adversarial tool tests ────────────────────

    #[tokio::test]
    async fn test_falsify_path_traversal_in_user_rejected() {
        let server = McpServer::new(crate::McpConfig {
            mnemo_base_url: "http://localhost:99999".to_string(),
            api_key: None,
            default_user: None,
        });
        // Path traversal attempt via user field
        let result = dispatch_tool(
            &server,
            "mnemo_recall",
            &serde_json::json!({"query": "test", "user": "../../etc/passwd"}),
        )
        .await;
        assert_eq!(result.is_error, Some(true));
        assert!(
            result.content[0].text.contains("illegal characters"),
            "Should reject path traversal: {}",
            result.content[0].text
        );
    }

    #[tokio::test]
    async fn test_falsify_path_traversal_in_agent_id_rejected() {
        let server = McpServer::new(crate::McpConfig {
            mnemo_base_url: "http://localhost:99999".to_string(),
            api_key: None,
            default_user: None,
        });
        let result = dispatch_tool(
            &server,
            "mnemo_agent_identity",
            &serde_json::json!({"agent_id": "../../../admin", "action": "get"}),
        )
        .await;
        assert_eq!(result.is_error, Some(true));
        assert!(result.content[0].text.contains("illegal characters"));
    }

    #[tokio::test]
    async fn test_falsify_null_byte_in_user_rejected() {
        let server = McpServer::new(crate::McpConfig {
            mnemo_base_url: "http://localhost:99999".to_string(),
            api_key: None,
            default_user: None,
        });
        let result = dispatch_tool(
            &server,
            "mnemo_coherence",
            &serde_json::json!({"user": "user\u{0000}admin"}),
        )
        .await;
        assert_eq!(result.is_error, Some(true));
        assert!(result.content[0].text.contains("illegal characters"));
    }

    #[tokio::test]
    async fn test_falsify_slash_in_user_rejected() {
        let server = McpServer::new(crate::McpConfig {
            mnemo_base_url: "http://localhost:99999".to_string(),
            api_key: None,
            default_user: None,
        });
        // Forward slash injection
        let result = dispatch_tool(
            &server,
            "mnemo_remember",
            &serde_json::json!({"text": "hello", "user": "user/admin"}),
        )
        .await;
        assert_eq!(result.is_error, Some(true));
    }

    #[tokio::test]
    async fn test_falsify_oversized_identifier_rejected() {
        let server = McpServer::new(crate::McpConfig {
            mnemo_base_url: "http://localhost:99999".to_string(),
            api_key: None,
            default_user: None,
        });
        let huge_user = "a".repeat(300);
        let result = dispatch_tool(
            &server,
            "mnemo_coherence",
            &serde_json::json!({"user": huge_user}),
        )
        .await;
        assert_eq!(result.is_error, Some(true));
        assert!(result.content[0].text.contains("maximum length"));
    }

    #[tokio::test]
    async fn test_falsify_remember_whitespace_only_text() {
        let server = McpServer::new(crate::McpConfig {
            mnemo_base_url: "http://localhost:99999".to_string(),
            api_key: None,
            default_user: Some("test-user".to_string()),
        });
        let result = dispatch_tool(
            &server,
            "mnemo_remember",
            &serde_json::json!({"text": "   \t\n  "}),
        )
        .await;
        assert_eq!(result.is_error, Some(true));
        assert!(result.content[0].text.contains("non-empty"));
    }

    #[tokio::test]
    async fn test_falsify_digest_invalid_action() {
        let server = McpServer::new(crate::McpConfig {
            mnemo_base_url: "http://localhost:99999".to_string(),
            api_key: None,
            default_user: Some("test-user".to_string()),
        });
        let result = dispatch_tool(
            &server,
            "mnemo_digest",
            &serde_json::json!({"action": "delete"}),
        )
        .await;
        assert_eq!(result.is_error, Some(true));
        assert!(result.content[0].text.contains("Unknown action"));
    }

    #[tokio::test]
    async fn test_falsify_agent_identity_update_without_body() {
        let server = McpServer::new(crate::McpConfig {
            mnemo_base_url: "http://localhost:99999".to_string(),
            api_key: None,
            default_user: None,
        });
        let result = dispatch_tool(
            &server,
            "mnemo_agent_identity",
            &serde_json::json!({"agent_id": "abc-123", "action": "update"}),
        )
        .await;
        assert_eq!(result.is_error, Some(true));
        assert!(result.content[0].text.contains("update"));
    }

    #[tokio::test]
    async fn test_falsify_agent_identity_invalid_action() {
        let server = McpServer::new(crate::McpConfig {
            mnemo_base_url: "http://localhost:99999".to_string(),
            api_key: None,
            default_user: None,
        });
        let result = dispatch_tool(
            &server,
            "mnemo_agent_identity",
            &serde_json::json!({"agent_id": "abc-123", "action": "delete"}),
        )
        .await;
        assert_eq!(result.is_error, Some(true));
        assert!(result.content[0].text.contains("Unknown action"));
    }

    #[tokio::test]
    async fn test_falsify_graph_query_missing_operation() {
        let server = McpServer::new(crate::McpConfig {
            mnemo_base_url: "http://localhost:99999".to_string(),
            api_key: None,
            default_user: Some("test-user".to_string()),
        });
        let result = dispatch_tool(
            &server,
            "mnemo_graph_query",
            &serde_json::json!({}),
        )
        .await;
        assert_eq!(result.is_error, Some(true));
        assert!(result.content[0].text.contains("operation"));
    }

    #[tokio::test]
    async fn test_falsify_recall_missing_query() {
        let server = McpServer::new(crate::McpConfig {
            mnemo_base_url: "http://localhost:99999".to_string(),
            api_key: None,
            default_user: Some("test-user".to_string()),
        });
        let result = dispatch_tool(
            &server,
            "mnemo_recall",
            &serde_json::json!({}),
        )
        .await;
        assert_eq!(result.is_error, Some(true));
    }

    #[tokio::test]
    async fn test_falsify_remember_non_string_text() {
        let server = McpServer::new(crate::McpConfig {
            mnemo_base_url: "http://localhost:99999".to_string(),
            api_key: None,
            default_user: Some("test-user".to_string()),
        });
        // text is a number, not a string
        let result = dispatch_tool(
            &server,
            "mnemo_remember",
            &serde_json::json!({"text": 12345}),
        )
        .await;
        assert_eq!(result.is_error, Some(true));
    }

    #[tokio::test]
    async fn test_falsify_default_user_with_path_traversal_rejected() {
        // Default user itself could be malicious
        let server = McpServer::new(crate::McpConfig {
            mnemo_base_url: "http://localhost:99999".to_string(),
            api_key: None,
            default_user: Some("../../../etc/shadow".to_string()),
        });
        let result = dispatch_tool(
            &server,
            "mnemo_coherence",
            &serde_json::json!({}),
        )
        .await;
        assert_eq!(result.is_error, Some(true));
        assert!(result.content[0].text.contains("illegal characters"));
    }
}
