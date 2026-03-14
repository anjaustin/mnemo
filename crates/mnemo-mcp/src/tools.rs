//! MCP tool definitions and dispatch for Mnemo.
//!
//! Each tool maps to one or more Mnemo HTTP API calls.
//!
//! ## Tool naming
//!
//! Tools use short names (`remember`, `recall`, etc.) since the MCP server is
//! already namespaced as "mnemo". For backward compatibility, the old
//! `mnemo_*`-prefixed names are accepted in `dispatch_tool` with a deprecation
//! warning logged to stderr.

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
        return Err(format!(
            "'{}' exceeds maximum length of 256 characters",
            field_name
        ));
    }
    Ok(())
}

/// Return all tool definitions.
pub fn list_tools() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "remember".to_string(),
            description: "Store a memory. The text will be processed, entities and relationships \
                          extracted, and the knowledge graph updated."
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
            name: "recall".to_string(),
            description: "Retrieve context from memory for a given query. Returns relevant \
                          entities, facts, and episodes assembled into a context block."
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
            name: "graph".to_string(),
            description: "Query the knowledge graph. List entities, edges, find shortest paths, \
                          or detect communities."
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
            name: "identity".to_string(),
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
            name: "digest".to_string(),
            description: "Get or generate a prose memory digest. The digest summarizes the \
                          knowledge graph into a human-readable narrative."
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
            name: "coherence".to_string(),
            description: "Get a coherence report for the knowledge graph. Measures internal \
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
            name: "health".to_string(),
            description: "Check the health of the Mnemo server.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        },
        // ─── New topology tools ───────────────────────────────────────
        ToolDefinition {
            name: "delegate".to_string(),
            description: "Grant another agent read access to a memory scope. Creates a memory \
                          region (if needed) and adds an ACL entry for the target agent."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "user": {
                        "type": "string",
                        "description": "User identifier whose memory to share."
                    },
                    "region_name": {
                        "type": "string",
                        "description": "Name for the memory region / scope."
                    },
                    "target_agent_id": {
                        "type": "string",
                        "description": "Agent UUID to grant access to."
                    },
                    "permission": {
                        "type": "string",
                        "enum": ["read", "write", "manage"],
                        "description": "Permission level to grant (default: 'read')."
                    }
                },
                "required": ["region_name", "target_agent_id"]
            }),
        },
        ToolDefinition {
            name: "revoke".to_string(),
            description: "Revoke a previously delegated memory scope access from an agent."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "region_id": {
                        "type": "string",
                        "description": "Memory region UUID."
                    },
                    "target_agent_id": {
                        "type": "string",
                        "description": "Agent UUID to revoke access from."
                    }
                },
                "required": ["region_id", "target_agent_id"]
            }),
        },
        ToolDefinition {
            name: "scopes".to_string(),
            description: "List memory scopes (regions) visible to the current agent or a \
                          specified user."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "user": {
                        "type": "string",
                        "description": "User identifier."
                    },
                    "agent_id": {
                        "type": "string",
                        "description": "Agent UUID to check scopes for. If omitted, lists all regions for the user."
                    }
                },
                "required": []
            }),
        },
    ]
}

/// Map deprecated `mnemo_*` names to their canonical short names.
///
/// Returns `Some(canonical_name)` if the input is a deprecated alias,
/// or `None` if it's not a known deprecated name.
fn resolve_deprecated_name(tool_name: &str) -> Option<&'static str> {
    match tool_name {
        "mnemo_remember" => Some("remember"),
        "mnemo_recall" => Some("recall"),
        "mnemo_graph_query" => Some("graph"),
        "mnemo_agent_identity" => Some("identity"),
        "mnemo_digest" => Some("digest"),
        "mnemo_coherence" => Some("coherence"),
        "mnemo_health" => Some("health"),
        _ => None,
    }
}

/// Dispatch a tool call to the appropriate handler.
///
/// Accepts both new short names and deprecated `mnemo_*` names (with a warning).
pub async fn dispatch_tool(
    server: &McpServer,
    tool_name: &str,
    arguments: &Value,
) -> ToolCallResult {
    // Resolve deprecated names
    let canonical = if let Some(new_name) = resolve_deprecated_name(tool_name) {
        tracing::warn!(
            old_name = tool_name,
            new_name = new_name,
            "Deprecated tool name used. Use '{}' instead of '{}'. \
             The mnemo_* prefix will be removed in a future release.",
            new_name,
            tool_name
        );
        new_name
    } else {
        tool_name
    };

    match canonical {
        "remember" => handle_remember(server, arguments).await,
        "recall" => handle_recall(server, arguments).await,
        "graph" => handle_graph_query(server, arguments).await,
        "identity" => handle_agent_identity(server, arguments).await,
        "digest" => handle_digest(server, arguments).await,
        "coherence" => handle_coherence(server, arguments).await,
        "health" => handle_health(server).await,
        "delegate" => handle_delegate(server, arguments).await,
        "revoke" => handle_revoke(server, arguments).await,
        "scopes" => handle_scopes(server, arguments).await,
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

    match server.post("/api/v1/memory").json(&body).send().await {
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
        _ => ToolCallResult::error(format!(
            "Unknown action '{}'. Must be 'get' or 'update'",
            action
        )),
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

// ─── Topology tool handlers ──────────────────────────────────────

async fn handle_delegate(server: &McpServer, args: &Value) -> ToolCallResult {
    let user = match server.resolve_user(args.get("user").and_then(|v| v.as_str())) {
        Ok(u) => u.to_string(),
        Err(e) => return ToolCallResult::error(e),
    };

    if let Err(e) = validate_path_segment(&user, "user") {
        return ToolCallResult::error(e);
    }

    let region_name = match args.get("region_name").and_then(|v| v.as_str()) {
        Some(n) if !n.trim().is_empty() => n,
        _ => {
            return ToolCallResult::error(
                "'region_name' argument is required and must be non-empty",
            )
        }
    };

    let target_agent_id = match args.get("target_agent_id").and_then(|v| v.as_str()) {
        Some(id) if !id.trim().is_empty() => id,
        _ => {
            return ToolCallResult::error(
                "'target_agent_id' argument is required and must be non-empty",
            )
        }
    };

    if let Err(e) = validate_path_segment(target_agent_id, "target_agent_id") {
        return ToolCallResult::error(e);
    }

    let permission = args
        .get("permission")
        .and_then(|v| v.as_str())
        .unwrap_or("read");

    if !["read", "write", "manage"].contains(&permission) {
        return ToolCallResult::error(format!(
            "Invalid permission '{}'. Must be one of: read, write, manage",
            permission
        ));
    }

    // Step 1: Create or find the memory region
    let region_body = serde_json::json!({
        "name": region_name,
        "user_id": user,
    });

    let region_id = match server
        .post("/api/v1/regions")
        .json(&region_body)
        .send()
        .await
    {
        Ok(resp) => {
            let status = resp.status();
            match resp.json::<Value>().await {
                Ok(json) if status.is_success() => match json.get("id").and_then(|v| v.as_str()) {
                    Some(id) => id.to_string(),
                    None => {
                        return ToolCallResult::error(
                            "Region created but response missing 'id' field",
                        )
                    }
                },
                Ok(json) => {
                    return ToolCallResult::error(format!(
                        "Failed to create region ({}): {}",
                        status,
                        serde_json::to_string_pretty(&json).unwrap_or_default()
                    ))
                }
                Err(e) => return ToolCallResult::error(format!("Failed to parse response: {}", e)),
            }
        }
        Err(e) => return ToolCallResult::error(format!("HTTP request failed: {}", e)),
    };

    // Step 2: Grant ACL on the region
    let acl_body = serde_json::json!({
        "agent_id": target_agent_id,
        "permission": permission,
    });

    let acl_path = format!("/api/v1/regions/{}/acl", region_id);
    match server.post(&acl_path).json(&acl_body).send().await {
        Ok(resp) => {
            let status = resp.status();
            match resp.json::<Value>().await {
                Ok(json) if status.is_success() => {
                    let result = serde_json::json!({
                        "region_id": region_id,
                        "region_name": region_name,
                        "target_agent_id": target_agent_id,
                        "permission": permission,
                        "status": "delegated",
                    });
                    ToolCallResult::json(&result)
                }
                Ok(json) => ToolCallResult::error(format!(
                    "Region created but ACL grant failed ({}): {}",
                    status,
                    serde_json::to_string_pretty(&json).unwrap_or_default()
                )),
                Err(e) => ToolCallResult::error(format!("Failed to parse ACL response: {}", e)),
            }
        }
        Err(e) => ToolCallResult::error(format!("ACL grant HTTP request failed: {}", e)),
    }
}

async fn handle_revoke(server: &McpServer, args: &Value) -> ToolCallResult {
    let region_id = match args.get("region_id").and_then(|v| v.as_str()) {
        Some(id) if !id.trim().is_empty() => id,
        _ => {
            return ToolCallResult::error(
                "'region_id' argument is required and must be non-empty",
            )
        }
    };

    if let Err(e) = validate_path_segment(region_id, "region_id") {
        return ToolCallResult::error(e);
    }

    let target_agent_id = match args.get("target_agent_id").and_then(|v| v.as_str()) {
        Some(id) if !id.trim().is_empty() => id,
        _ => {
            return ToolCallResult::error(
                "'target_agent_id' argument is required and must be non-empty",
            )
        }
    };

    if let Err(e) = validate_path_segment(target_agent_id, "target_agent_id") {
        return ToolCallResult::error(e);
    }

    let path = format!("/api/v1/regions/{}/acl/{}", region_id, target_agent_id);
    match server.delete(&path).send().await {
        Ok(resp) => {
            let status = resp.status();
            if status.is_success() {
                let result = serde_json::json!({
                    "region_id": region_id,
                    "target_agent_id": target_agent_id,
                    "status": "revoked",
                });
                ToolCallResult::json(&result)
            } else {
                match resp.json::<Value>().await {
                    Ok(json) => ToolCallResult::error(format!(
                        "Revoke failed ({}): {}",
                        status,
                        serde_json::to_string_pretty(&json).unwrap_or_default()
                    )),
                    Err(e) => ToolCallResult::error(format!(
                        "Revoke failed ({}) and response unparseable: {}",
                        status, e
                    )),
                }
            }
        }
        Err(e) => ToolCallResult::error(format!("HTTP request failed: {}", e)),
    }
}

async fn handle_scopes(server: &McpServer, args: &Value) -> ToolCallResult {
    let user = match server.resolve_user(args.get("user").and_then(|v| v.as_str())) {
        Ok(u) => u.to_string(),
        Err(e) => return ToolCallResult::error(e),
    };

    if let Err(e) = validate_path_segment(&user, "user") {
        return ToolCallResult::error(e);
    }

    let mut path = format!("/api/v1/regions?user_id={}", user);
    if let Some(agent_id) = args.get("agent_id").and_then(|v| v.as_str()) {
        if let Err(e) = validate_path_segment(agent_id, "agent_id") {
            return ToolCallResult::error(e);
        }
        path.push_str(&format!("&agent_id={}", agent_id));
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_list_tools_returns_10_tools() {
        let tools = list_tools();
        assert_eq!(tools.len(), 10);

        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"remember"));
        assert!(names.contains(&"recall"));
        assert!(names.contains(&"graph"));
        assert!(names.contains(&"identity"));
        assert!(names.contains(&"digest"));
        assert!(names.contains(&"coherence"));
        assert!(names.contains(&"health"));
        assert!(names.contains(&"delegate"));
        assert!(names.contains(&"revoke"));
        assert!(names.contains(&"scopes"));
    }

    #[test]
    fn test_no_tools_use_mnemo_prefix() {
        for tool in list_tools() {
            assert!(
                !tool.name.starts_with("mnemo_"),
                "Tool '{}' should not use deprecated mnemo_ prefix",
                tool.name
            );
        }
    }

    #[test]
    fn test_deprecated_name_resolution() {
        assert_eq!(resolve_deprecated_name("mnemo_remember"), Some("remember"));
        assert_eq!(resolve_deprecated_name("mnemo_recall"), Some("recall"));
        assert_eq!(
            resolve_deprecated_name("mnemo_graph_query"),
            Some("graph")
        );
        assert_eq!(
            resolve_deprecated_name("mnemo_agent_identity"),
            Some("identity")
        );
        assert_eq!(resolve_deprecated_name("mnemo_digest"), Some("digest"));
        assert_eq!(
            resolve_deprecated_name("mnemo_coherence"),
            Some("coherence")
        );
        assert_eq!(resolve_deprecated_name("mnemo_health"), Some("health"));
        assert_eq!(resolve_deprecated_name("remember"), None);
        assert_eq!(resolve_deprecated_name("unknown"), None);
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
            .find(|t| t.name == "remember")
            .unwrap();
        let required = tool.input_schema["required"].as_array().unwrap();
        assert!(required.contains(&serde_json::json!("text")));
    }

    #[test]
    fn test_recall_tool_requires_query() {
        let tool = list_tools()
            .into_iter()
            .find(|t| t.name == "recall")
            .unwrap();
        let required = tool.input_schema["required"].as_array().unwrap();
        assert!(required.contains(&serde_json::json!("query")));
    }

    #[test]
    fn test_graph_query_operations_documented() {
        let tool = list_tools()
            .into_iter()
            .find(|t| t.name == "graph")
            .unwrap();
        let ops = tool.input_schema["properties"]["operation"]["enum"]
            .as_array()
            .unwrap();
        assert!(ops.contains(&serde_json::json!("list_entities")));
        assert!(ops.contains(&serde_json::json!("list_edges")));
        assert!(ops.contains(&serde_json::json!("communities")));
    }

    #[test]
    fn test_delegate_tool_requires_fields() {
        let tool = list_tools()
            .into_iter()
            .find(|t| t.name == "delegate")
            .unwrap();
        let required = tool.input_schema["required"].as_array().unwrap();
        assert!(required.contains(&serde_json::json!("region_name")));
        assert!(required.contains(&serde_json::json!("target_agent_id")));
    }

    #[test]
    fn test_revoke_tool_requires_fields() {
        let tool = list_tools()
            .into_iter()
            .find(|t| t.name == "revoke")
            .unwrap();
        let required = tool.input_schema["required"].as_array().unwrap();
        assert!(required.contains(&serde_json::json!("region_id")));
        assert!(required.contains(&serde_json::json!("target_agent_id")));
    }

    #[test]
    fn test_delegate_permission_enum() {
        let tool = list_tools()
            .into_iter()
            .find(|t| t.name == "delegate")
            .unwrap();
        let perms = tool.input_schema["properties"]["permission"]["enum"]
            .as_array()
            .unwrap();
        assert!(perms.contains(&serde_json::json!("read")));
        assert!(perms.contains(&serde_json::json!("write")));
        assert!(perms.contains(&serde_json::json!("manage")));
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
    async fn test_deprecated_mnemo_remember_dispatches() {
        let server = McpServer::new(crate::McpConfig {
            mnemo_base_url: "http://localhost:99999".to_string(),
            api_key: None,
            default_user: Some("test-user".to_string()),
        });
        // Old name should still work (will fail at HTTP level, but dispatch succeeds)
        let result = dispatch_tool(
            &server,
            "mnemo_remember",
            &serde_json::json!({"text": "hello"}),
        )
        .await;
        // Should get a connection error, NOT "Unknown tool"
        assert!(
            !result.content[0].text.contains("Unknown tool"),
            "Deprecated name should dispatch: {}",
            result.content[0].text
        );
    }

    #[tokio::test]
    async fn test_remember_rejects_empty_text() {
        let server = McpServer::new(crate::McpConfig {
            mnemo_base_url: "http://localhost:99999".to_string(),
            api_key: None,
            default_user: Some("test-user".to_string()),
        });
        let result =
            dispatch_tool(&server, "remember", &serde_json::json!({"text": ""})).await;
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
        let result =
            dispatch_tool(&server, "recall", &serde_json::json!({"query": "  "})).await;
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
            "remember",
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
            "graph",
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
            "identity",
            &serde_json::json!({"action": "get"}),
        )
        .await;
        assert_eq!(result.is_error, Some(true));

        // Missing action
        let result = dispatch_tool(
            &server,
            "identity",
            &serde_json::json!({"agent_id": "abc"}),
        )
        .await;
        assert_eq!(result.is_error, Some(true));
    }

    // ─── New tool validation tests ────────────────────────────────

    #[tokio::test]
    async fn test_delegate_rejects_missing_region_name() {
        let server = McpServer::new(crate::McpConfig {
            mnemo_base_url: "http://localhost:99999".to_string(),
            api_key: None,
            default_user: Some("test-user".to_string()),
        });
        let result = dispatch_tool(
            &server,
            "delegate",
            &serde_json::json!({"target_agent_id": "agent-1"}),
        )
        .await;
        assert_eq!(result.is_error, Some(true));
        assert!(result.content[0].text.contains("region_name"));
    }

    #[tokio::test]
    async fn test_delegate_rejects_missing_target_agent() {
        let server = McpServer::new(crate::McpConfig {
            mnemo_base_url: "http://localhost:99999".to_string(),
            api_key: None,
            default_user: Some("test-user".to_string()),
        });
        let result = dispatch_tool(
            &server,
            "delegate",
            &serde_json::json!({"region_name": "shared-scope"}),
        )
        .await;
        assert_eq!(result.is_error, Some(true));
        assert!(result.content[0].text.contains("target_agent_id"));
    }

    #[tokio::test]
    async fn test_delegate_rejects_invalid_permission() {
        let server = McpServer::new(crate::McpConfig {
            mnemo_base_url: "http://localhost:99999".to_string(),
            api_key: None,
            default_user: Some("test-user".to_string()),
        });
        let result = dispatch_tool(
            &server,
            "delegate",
            &serde_json::json!({
                "region_name": "scope",
                "target_agent_id": "agent-1",
                "permission": "admin"
            }),
        )
        .await;
        assert_eq!(result.is_error, Some(true));
        assert!(result.content[0].text.contains("Invalid permission"));
    }

    #[tokio::test]
    async fn test_revoke_rejects_missing_region_id() {
        let server = McpServer::new(crate::McpConfig {
            mnemo_base_url: "http://localhost:99999".to_string(),
            api_key: None,
            default_user: None,
        });
        let result = dispatch_tool(
            &server,
            "revoke",
            &serde_json::json!({"target_agent_id": "agent-1"}),
        )
        .await;
        assert_eq!(result.is_error, Some(true));
        assert!(result.content[0].text.contains("region_id"));
    }

    #[tokio::test]
    async fn test_revoke_rejects_missing_target_agent() {
        let server = McpServer::new(crate::McpConfig {
            mnemo_base_url: "http://localhost:99999".to_string(),
            api_key: None,
            default_user: None,
        });
        let result = dispatch_tool(
            &server,
            "revoke",
            &serde_json::json!({"region_id": "some-id"}),
        )
        .await;
        assert_eq!(result.is_error, Some(true));
        assert!(result.content[0].text.contains("target_agent_id"));
    }

    #[tokio::test]
    async fn test_scopes_rejects_missing_user() {
        let server = McpServer::new(crate::McpConfig {
            mnemo_base_url: "http://localhost:99999".to_string(),
            api_key: None,
            default_user: None, // no default
        });
        let result = dispatch_tool(&server, "scopes", &serde_json::json!({})).await;
        assert_eq!(result.is_error, Some(true));
        assert!(result.content[0].text.contains("MNEMO_MCP_DEFAULT_USER"));
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
            "recall",
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
            "identity",
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
            "coherence",
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
            "remember",
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
            "coherence",
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
            "remember",
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
            "digest",
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
            "identity",
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
            "identity",
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
        let result = dispatch_tool(&server, "graph", &serde_json::json!({})).await;
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
        let result = dispatch_tool(&server, "recall", &serde_json::json!({})).await;
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
            "remember",
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
        let result = dispatch_tool(&server, "coherence", &serde_json::json!({})).await;
        assert_eq!(result.is_error, Some(true));
        assert!(result.content[0].text.contains("illegal characters"));
    }

    #[tokio::test]
    async fn test_falsify_delegate_path_traversal_in_target_agent() {
        let server = McpServer::new(crate::McpConfig {
            mnemo_base_url: "http://localhost:99999".to_string(),
            api_key: None,
            default_user: Some("test-user".to_string()),
        });
        let result = dispatch_tool(
            &server,
            "delegate",
            &serde_json::json!({
                "region_name": "scope",
                "target_agent_id": "../../../etc/passwd"
            }),
        )
        .await;
        assert_eq!(result.is_error, Some(true));
        assert!(result.content[0].text.contains("illegal characters"));
    }

    #[tokio::test]
    async fn test_falsify_revoke_path_traversal_in_region_id() {
        let server = McpServer::new(crate::McpConfig {
            mnemo_base_url: "http://localhost:99999".to_string(),
            api_key: None,
            default_user: None,
        });
        let result = dispatch_tool(
            &server,
            "revoke",
            &serde_json::json!({
                "region_id": "../../admin",
                "target_agent_id": "agent-1"
            }),
        )
        .await;
        assert_eq!(result.is_error, Some(true));
        assert!(result.content[0].text.contains("illegal characters"));
    }
}
