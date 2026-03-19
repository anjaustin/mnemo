//! MCP resource definitions and dispatch for Mnemo.
//!
//! Resources are read-only data that MCP clients can access.

use serde_json::Value;

use crate::protocol::{ResourceContent, ResourceReadResult, ResourceTemplate};
use crate::McpServer;

/// Return all resource templates.
pub fn list_resource_templates() -> Vec<ResourceTemplate> {
    vec![
        // ─── User Memory Resources ────────────────────────────────────
        ResourceTemplate {
            uri_template: "mnemo://users/{user}/memory".to_string(),
            name: "User Memory".to_string(),
            description: "Knowledge graph summary and coherence report for a user.".to_string(),
            mime_type: "application/json".to_string(),
        },
        ResourceTemplate {
            uri_template: "mnemo://users/{user}/episodes".to_string(),
            name: "Recent Episodes".to_string(),
            description: "List of recent memory episodes for a user (default: 20).".to_string(),
            mime_type: "application/json".to_string(),
        },
        ResourceTemplate {
            uri_template: "mnemo://users/{user}/entities".to_string(),
            name: "User Entities".to_string(),
            description: "List of entities in the user's knowledge graph (default: 50).".to_string(),
            mime_type: "application/json".to_string(),
        },
        ResourceTemplate {
            uri_template: "mnemo://users/{user}/search".to_string(),
            name: "Memory Search".to_string(),
            description: "Search user's memory. Append ?q=query for semantic search.".to_string(),
            mime_type: "application/json".to_string(),
        },
        ResourceTemplate {
            uri_template: "mnemo://users/{user}/digest".to_string(),
            name: "Memory Digest".to_string(),
            description: "Prose summary of user's knowledge graph.".to_string(),
            mime_type: "application/json".to_string(),
        },
        // ─── Episode Resources ────────────────────────────────────────
        ResourceTemplate {
            uri_template: "mnemo://episodes/{episode_id}".to_string(),
            name: "Episode".to_string(),
            description: "Single memory episode by ID with all turns.".to_string(),
            mime_type: "application/json".to_string(),
        },
        // ─── Agent Identity Resources ─────────────────────────────────
        ResourceTemplate {
            uri_template: "mnemo://agents/{agent_id}/identity".to_string(),
            name: "Agent Identity".to_string(),
            description: "Agent identity profile including core, experience, and version info."
                .to_string(),
            mime_type: "application/json".to_string(),
        },
        ResourceTemplate {
            uri_template: "mnemo://agents/{agent_id}/experience".to_string(),
            name: "Agent Experience".to_string(),
            description: "Recent experience events for agent identity evolution (default: 20).".to_string(),
            mime_type: "application/json".to_string(),
        },
        ResourceTemplate {
            uri_template: "mnemo://agents/{agent_id}/promotions".to_string(),
            name: "Promotion Proposals".to_string(),
            description: "Pending identity promotion proposals for this agent.".to_string(),
            mime_type: "application/json".to_string(),
        },
        // ─── Graph Resources ──────────────────────────────────────────
        ResourceTemplate {
            uri_template: "mnemo://users/{user}/graph/edges".to_string(),
            name: "Graph Edges".to_string(),
            description: "Relationships between entities in the knowledge graph.".to_string(),
            mime_type: "application/json".to_string(),
        },
        ResourceTemplate {
            uri_template: "mnemo://users/{user}/graph/communities".to_string(),
            name: "Graph Communities".to_string(),
            description: "Detected communities in the knowledge graph.".to_string(),
            mime_type: "application/json".to_string(),
        },
    ]
}

/// Validate that an identifier segment is safe (no path traversal, slashes, etc.).
fn validate_identifier(id: &str, field_name: &str) -> Result<(), String> {
    if id.is_empty() {
        return Err(format!("{} cannot be empty", field_name));
    }
    if id.contains("..") || id.contains('/') || id.contains('\\') || id.contains('\0') {
        return Err(format!(
            "Invalid {}: '{}' contains illegal characters (path traversal not allowed)",
            field_name, id
        ));
    }
    // Limit identifier length to prevent abuse
    if id.len() > 256 {
        return Err(format!(
            "{} exceeds maximum length of 256 characters",
            field_name
        ));
    }
    Ok(())
}

/// Parse a resource URI and dispatch to the appropriate handler.
pub async fn read_resource(server: &McpServer, uri: &str) -> Result<ResourceReadResult, String> {
    // Parse mnemo:// URIs
    if let Some(rest) = uri.strip_prefix("mnemo://") {
        // Handle query strings by splitting on '?'
        let (path_part, query_part) = rest.split_once('?').unwrap_or((rest, ""));
        let parts: Vec<&str> = path_part.splitn(4, '/').collect();

        match parts.as_slice() {
            // ─── User Memory Resources ────────────────────────────────────
            ["users", user, "memory"] => {
                validate_identifier(user, "user")?;
                read_user_memory(server, user, uri).await
            }
            ["users", user, "episodes"] => {
                validate_identifier(user, "user")?;
                read_user_episodes(server, user, uri).await
            }
            ["users", user, "entities"] => {
                validate_identifier(user, "user")?;
                read_user_entities(server, user, uri).await
            }
            ["users", user, "search"] => {
                validate_identifier(user, "user")?;
                read_user_search(server, user, query_part, uri).await
            }
            ["users", user, "digest"] => {
                validate_identifier(user, "user")?;
                read_user_digest(server, user, uri).await
            }
            // ─── Graph Resources ──────────────────────────────────────────
            ["users", user, "graph", "edges"] => {
                validate_identifier(user, "user")?;
                read_graph_edges(server, user, uri).await
            }
            ["users", user, "graph", "communities"] => {
                validate_identifier(user, "user")?;
                read_graph_communities(server, user, uri).await
            }
            // ─── Episode Resources ────────────────────────────────────────
            ["episodes", episode_id] => {
                validate_identifier(episode_id, "episode_id")?;
                read_episode(server, episode_id, uri).await
            }
            // ─── Agent Identity Resources ─────────────────────────────────
            ["agents", agent_id, "identity"] => {
                validate_identifier(agent_id, "agent_id")?;
                read_agent_identity(server, agent_id, uri).await
            }
            ["agents", agent_id, "experience"] => {
                validate_identifier(agent_id, "agent_id")?;
                read_agent_experience(server, agent_id, uri).await
            }
            ["agents", agent_id, "promotions"] => {
                validate_identifier(agent_id, "agent_id")?;
                read_agent_promotions(server, agent_id, uri).await
            }
            _ => Err(format!("Unknown resource URI: {}", uri)),
        }
    } else {
        Err(format!(
            "Invalid resource URI: {}. Must start with 'mnemo://'",
            uri
        ))
    }
}

async fn read_user_memory(
    server: &McpServer,
    user: &str,
    uri: &str,
) -> Result<ResourceReadResult, String> {
    // Fetch coherence report (lightweight summary of user's memory)
    let path = format!("/api/v1/users/{}/coherence", user);
    let resp = server
        .get(&path)
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {}", e))?;

    let status = resp.status();
    let json: Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    if !status.is_success() {
        return Err(format!(
            "Mnemo API error ({}): {}",
            status,
            serde_json::to_string_pretty(&json).unwrap_or_default()
        ));
    }

    Ok(ResourceReadResult {
        contents: vec![ResourceContent {
            uri: uri.to_string(),
            mime_type: "application/json".to_string(),
            text: serde_json::to_string_pretty(&json).unwrap_or_default(),
        }],
    })
}

async fn read_user_episodes(
    server: &McpServer,
    user: &str,
    uri: &str,
) -> Result<ResourceReadResult, String> {
    let path = format!("/api/v1/users/{}/episodes?limit=20", user);
    let resp = server
        .get(&path)
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {}", e))?;

    let status = resp.status();
    let json: Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    if !status.is_success() {
        return Err(format!(
            "Mnemo API error ({}): {}",
            status,
            serde_json::to_string_pretty(&json).unwrap_or_default()
        ));
    }

    Ok(ResourceReadResult {
        contents: vec![ResourceContent {
            uri: uri.to_string(),
            mime_type: "application/json".to_string(),
            text: serde_json::to_string_pretty(&json).unwrap_or_default(),
        }],
    })
}

async fn read_user_entities(
    server: &McpServer,
    user: &str,
    uri: &str,
) -> Result<ResourceReadResult, String> {
    let path = format!("/api/v1/graph/{}/entities?limit=50", user);
    let resp = server
        .get(&path)
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {}", e))?;

    let status = resp.status();
    let json: Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    if !status.is_success() {
        return Err(format!(
            "Mnemo API error ({}): {}",
            status,
            serde_json::to_string_pretty(&json).unwrap_or_default()
        ));
    }

    Ok(ResourceReadResult {
        contents: vec![ResourceContent {
            uri: uri.to_string(),
            mime_type: "application/json".to_string(),
            text: serde_json::to_string_pretty(&json).unwrap_or_default(),
        }],
    })
}

async fn read_agent_identity(
    server: &McpServer,
    agent_id: &str,
    uri: &str,
) -> Result<ResourceReadResult, String> {
    let path = format!("/api/v1/agents/{}/identity", agent_id);
    let resp = server
        .get(&path)
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {}", e))?;

    let status = resp.status();
    let json: Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    if !status.is_success() {
        return Err(format!(
            "Mnemo API error ({}): {}",
            status,
            serde_json::to_string_pretty(&json).unwrap_or_default()
        ));
    }

    Ok(ResourceReadResult {
        contents: vec![ResourceContent {
            uri: uri.to_string(),
            mime_type: "application/json".to_string(),
            text: serde_json::to_string_pretty(&json).unwrap_or_default(),
        }],
    })
}

async fn read_agent_experience(
    server: &McpServer,
    agent_id: &str,
    uri: &str,
) -> Result<ResourceReadResult, String> {
    let path = format!("/api/v1/agents/{}/experience?limit=20", agent_id);
    let resp = server
        .get(&path)
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {}", e))?;

    let status = resp.status();
    let json: Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    if !status.is_success() {
        return Err(format!(
            "Mnemo API error ({}): {}",
            status,
            serde_json::to_string_pretty(&json).unwrap_or_default()
        ));
    }

    Ok(ResourceReadResult {
        contents: vec![ResourceContent {
            uri: uri.to_string(),
            mime_type: "application/json".to_string(),
            text: serde_json::to_string_pretty(&json).unwrap_or_default(),
        }],
    })
}

async fn read_agent_promotions(
    server: &McpServer,
    agent_id: &str,
    uri: &str,
) -> Result<ResourceReadResult, String> {
    let path = format!("/api/v1/agents/{}/promotions", agent_id);
    let resp = server
        .get(&path)
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {}", e))?;

    let status = resp.status();
    let json: Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    if !status.is_success() {
        return Err(format!(
            "Mnemo API error ({}): {}",
            status,
            serde_json::to_string_pretty(&json).unwrap_or_default()
        ));
    }

    Ok(ResourceReadResult {
        contents: vec![ResourceContent {
            uri: uri.to_string(),
            mime_type: "application/json".to_string(),
            text: serde_json::to_string_pretty(&json).unwrap_or_default(),
        }],
    })
}

async fn read_user_search(
    server: &McpServer,
    user: &str,
    query_string: &str,
    uri: &str,
) -> Result<ResourceReadResult, String> {
    // Parse query parameter
    let query = query_string
        .strip_prefix("q=")
        .or_else(|| {
            query_string
                .split('&')
                .find(|p| p.starts_with("q="))
                .map(|p| &p[2..])
        })
        .unwrap_or("");

    if query.is_empty() {
        return Err("Search requires a 'q' query parameter (e.g., ?q=search+terms)".to_string());
    }

    // URL-decode the query
    let decoded_query =
        urlencoding::decode(query).map_err(|e| format!("Invalid query encoding: {}", e))?;

    // Validate query length
    if decoded_query.len() > 1024 {
        return Err("Search query exceeds maximum length of 1024 characters".to_string());
    }

    let path = format!(
        "/api/v1/users/{}/recall?query={}&limit=20",
        user,
        urlencoding::encode(&decoded_query)
    );
    let resp = server
        .get(&path)
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {}", e))?;

    let status = resp.status();
    let json: Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    if !status.is_success() {
        return Err(format!(
            "Mnemo API error ({}): {}",
            status,
            serde_json::to_string_pretty(&json).unwrap_or_default()
        ));
    }

    Ok(ResourceReadResult {
        contents: vec![ResourceContent {
            uri: uri.to_string(),
            mime_type: "application/json".to_string(),
            text: serde_json::to_string_pretty(&json).unwrap_or_default(),
        }],
    })
}

async fn read_user_digest(
    server: &McpServer,
    user: &str,
    uri: &str,
) -> Result<ResourceReadResult, String> {
    let path = format!("/api/v1/users/{}/digest", user);
    let resp = server
        .get(&path)
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {}", e))?;

    let status = resp.status();
    let json: Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    if !status.is_success() {
        return Err(format!(
            "Mnemo API error ({}): {}",
            status,
            serde_json::to_string_pretty(&json).unwrap_or_default()
        ));
    }

    Ok(ResourceReadResult {
        contents: vec![ResourceContent {
            uri: uri.to_string(),
            mime_type: "application/json".to_string(),
            text: serde_json::to_string_pretty(&json).unwrap_or_default(),
        }],
    })
}

async fn read_episode(
    server: &McpServer,
    episode_id: &str,
    uri: &str,
) -> Result<ResourceReadResult, String> {
    let path = format!("/api/v1/episodes/{}", episode_id);
    let resp = server
        .get(&path)
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {}", e))?;

    let status = resp.status();
    let json: Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    if !status.is_success() {
        return Err(format!(
            "Mnemo API error ({}): {}",
            status,
            serde_json::to_string_pretty(&json).unwrap_or_default()
        ));
    }

    Ok(ResourceReadResult {
        contents: vec![ResourceContent {
            uri: uri.to_string(),
            mime_type: "application/json".to_string(),
            text: serde_json::to_string_pretty(&json).unwrap_or_default(),
        }],
    })
}

async fn read_graph_edges(
    server: &McpServer,
    user: &str,
    uri: &str,
) -> Result<ResourceReadResult, String> {
    let path = format!("/api/v1/graph/{}/edges?limit=100", user);
    let resp = server
        .get(&path)
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {}", e))?;

    let status = resp.status();
    let json: Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    if !status.is_success() {
        return Err(format!(
            "Mnemo API error ({}): {}",
            status,
            serde_json::to_string_pretty(&json).unwrap_or_default()
        ));
    }

    Ok(ResourceReadResult {
        contents: vec![ResourceContent {
            uri: uri.to_string(),
            mime_type: "application/json".to_string(),
            text: serde_json::to_string_pretty(&json).unwrap_or_default(),
        }],
    })
}

async fn read_graph_communities(
    server: &McpServer,
    user: &str,
    uri: &str,
) -> Result<ResourceReadResult, String> {
    let path = format!("/api/v1/graph/{}/communities", user);
    let resp = server
        .get(&path)
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {}", e))?;

    let status = resp.status();
    let json: Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    if !status.is_success() {
        return Err(format!(
            "Mnemo API error ({}): {}",
            status,
            serde_json::to_string_pretty(&json).unwrap_or_default()
        ));
    }

    Ok(ResourceReadResult {
        contents: vec![ResourceContent {
            uri: uri.to_string(),
            mime_type: "application/json".to_string(),
            text: serde_json::to_string_pretty(&json).unwrap_or_default(),
        }],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_list_resource_templates_returns_all() {
        let templates = list_resource_templates();
        assert_eq!(templates.len(), 11);

        let uris: Vec<&str> = templates.iter().map(|t| t.uri_template.as_str()).collect();
        // User memory resources
        assert!(uris.contains(&"mnemo://users/{user}/memory"));
        assert!(uris.contains(&"mnemo://users/{user}/episodes"));
        assert!(uris.contains(&"mnemo://users/{user}/entities"));
        assert!(uris.contains(&"mnemo://users/{user}/search"));
        assert!(uris.contains(&"mnemo://users/{user}/digest"));
        // Episode resources
        assert!(uris.contains(&"mnemo://episodes/{episode_id}"));
        // Agent identity resources
        assert!(uris.contains(&"mnemo://agents/{agent_id}/identity"));
        assert!(uris.contains(&"mnemo://agents/{agent_id}/experience"));
        assert!(uris.contains(&"mnemo://agents/{agent_id}/promotions"));
        // Graph resources
        assert!(uris.contains(&"mnemo://users/{user}/graph/edges"));
        assert!(uris.contains(&"mnemo://users/{user}/graph/communities"));
    }

    #[test]
    fn test_all_templates_have_json_mime_type() {
        for tmpl in list_resource_templates() {
            assert_eq!(tmpl.mime_type, "application/json");
        }
    }

    #[test]
    fn test_all_templates_have_descriptions() {
        for tmpl in list_resource_templates() {
            assert!(!tmpl.description.is_empty());
        }
    }

    #[tokio::test]
    async fn test_read_resource_rejects_invalid_uri() {
        let server = McpServer::new(crate::McpConfig {
            mnemo_base_url: "http://localhost:99999".to_string(),
            api_key: None,
            default_user: None,
        });

        let result = read_resource(&server, "https://example.com").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Must start with 'mnemo://'"));
    }

    #[tokio::test]
    async fn test_read_resource_rejects_unknown_path() {
        let server = McpServer::new(crate::McpConfig {
            mnemo_base_url: "http://localhost:99999".to_string(),
            api_key: None,
            default_user: None,
        });

        let result = read_resource(&server, "mnemo://unknown/path").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unknown resource URI"));
    }

    // ─── Falsification: adversarial resource tests ────────────────

    #[tokio::test]
    async fn test_falsify_resource_path_traversal_user() {
        let server = McpServer::new(crate::McpConfig {
            mnemo_base_url: "http://localhost:99999".to_string(),
            api_key: None,
            default_user: None,
        });

        // With splitn(3, '/'), "../../etc/passwd/memory" after "users/" splits such that
        // the pattern ["users", user, "memory"] won't match (the ".." goes into the 2nd
        // slot and the rest into the 3rd, which isn't "memory"). Either way, traversal
        // is rejected — either by pattern mismatch or by identifier validation.
        let result = read_resource(&server, "mnemo://users/../../etc/passwd/memory").await;
        assert!(result.is_err(), "Path traversal in user must be rejected");

        // Also test a simpler traversal that DOES match the pattern shape:
        // "mnemo://users/..%2Fadmin/memory" — splitn would give ["users", "..%2Fadmin", "memory"]
        // but the ".." is inside the identifier, caught by validate_identifier
        let result2 = read_resource(&server, "mnemo://users/..evil/memory").await;
        assert!(result2.is_err());
        assert!(
            result2.unwrap_err().contains("illegal characters"),
            "Direct .. in identifier should be caught by validation"
        );
    }

    #[tokio::test]
    async fn test_falsify_resource_path_traversal_agent() {
        let server = McpServer::new(crate::McpConfig {
            mnemo_base_url: "http://localhost:99999".to_string(),
            api_key: None,
            default_user: None,
        });

        // "../admin" in agent segment: splitn produces ["agents", "..", "admin/identity"]
        // which doesn't match ["agents", agent_id, "identity"] pattern, so rejected by mismatch
        let result = read_resource(&server, "mnemo://agents/../admin/identity").await;
        assert!(
            result.is_err(),
            "Path traversal in agent_id must be rejected"
        );

        // Test a traversal that matches the pattern shape
        let result2 = read_resource(&server, "mnemo://agents/..evil/identity").await;
        assert!(result2.is_err());
        assert!(
            result2.unwrap_err().contains("illegal characters"),
            "Direct .. in identifier should be caught by validation"
        );
    }

    #[tokio::test]
    async fn test_falsify_resource_empty_user() {
        let server = McpServer::new(crate::McpConfig {
            mnemo_base_url: "http://localhost:99999".to_string(),
            api_key: None,
            default_user: None,
        });

        // Empty user segment: mnemo://users//memory
        let result = read_resource(&server, "mnemo://users//memory").await;
        // splitn will see ["users", "", "memory"] — empty string should be rejected
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_falsify_resource_oversized_identifier() {
        let server = McpServer::new(crate::McpConfig {
            mnemo_base_url: "http://localhost:99999".to_string(),
            api_key: None,
            default_user: None,
        });

        let huge = "x".repeat(300);
        let uri = format!("mnemo://users/{}/memory", huge);
        let result = read_resource(&server, &uri).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("maximum length"));
    }

    #[tokio::test]
    async fn test_falsify_resource_empty_uri() {
        let server = McpServer::new(crate::McpConfig {
            mnemo_base_url: "http://localhost:99999".to_string(),
            api_key: None,
            default_user: None,
        });

        let result = read_resource(&server, "").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_falsify_validate_identifier_unit() {
        // Direct unit tests for the identifier validator
        assert!(validate_identifier("valid-user-123", "user").is_ok());
        assert!(validate_identifier("550e8400-e29b-41d4-a716-446655440000", "user").is_ok());
        assert!(validate_identifier("", "user").is_err());
        assert!(validate_identifier("../evil", "user").is_err());
        assert!(validate_identifier("foo/bar", "user").is_err());
        assert!(validate_identifier("foo\\bar", "user").is_err());
        assert!(validate_identifier("foo\0bar", "user").is_err());
        assert!(validate_identifier(&"a".repeat(257), "user").is_err());
        assert!(validate_identifier(&"a".repeat(256), "user").is_ok());
    }

    // ─── Search resource tests ────────────────────────────────────

    #[tokio::test]
    async fn test_search_resource_requires_query() {
        let server = McpServer::new(crate::McpConfig {
            mnemo_base_url: "http://localhost:99999".to_string(),
            api_key: None,
            default_user: None,
        });

        // Missing query parameter
        let result = read_resource(&server, "mnemo://users/alice/search").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("requires a 'q' query parameter"));
    }

    #[tokio::test]
    async fn test_search_resource_rejects_oversized_query() {
        let server = McpServer::new(crate::McpConfig {
            mnemo_base_url: "http://localhost:99999".to_string(),
            api_key: None,
            default_user: None,
        });

        let huge_query = "x".repeat(1025);
        let uri = format!("mnemo://users/alice/search?q={}", huge_query);
        let result = read_resource(&server, &uri).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("maximum length"));
    }

    // ─── Graph resource tests ─────────────────────────────────────

    #[tokio::test]
    async fn test_graph_edges_path_traversal() {
        let server = McpServer::new(crate::McpConfig {
            mnemo_base_url: "http://localhost:99999".to_string(),
            api_key: None,
            default_user: None,
        });

        let result = read_resource(&server, "mnemo://users/..evil/graph/edges").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("illegal characters"));
    }

    #[tokio::test]
    async fn test_graph_communities_path_traversal() {
        let server = McpServer::new(crate::McpConfig {
            mnemo_base_url: "http://localhost:99999".to_string(),
            api_key: None,
            default_user: None,
        });

        let result = read_resource(&server, "mnemo://users/..evil/graph/communities").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("illegal characters"));
    }

    // ─── Episode resource tests ───────────────────────────────────

    #[tokio::test]
    async fn test_episode_path_traversal() {
        let server = McpServer::new(crate::McpConfig {
            mnemo_base_url: "http://localhost:99999".to_string(),
            api_key: None,
            default_user: None,
        });

        let result = read_resource(&server, "mnemo://episodes/../evil").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_episode_empty_id() {
        let server = McpServer::new(crate::McpConfig {
            mnemo_base_url: "http://localhost:99999".to_string(),
            api_key: None,
            default_user: None,
        });

        // This won't match the pattern since splitn would give ["episodes", ""]
        // which matches ["episodes", episode_id] with empty string - should be rejected
        let result = read_resource(&server, "mnemo://episodes/").await;
        // The trailing slash means episode_id is empty string
        assert!(result.is_err());
    }

    // ─── Agent promotions resource tests ──────────────────────────

    #[tokio::test]
    async fn test_promotions_path_traversal() {
        let server = McpServer::new(crate::McpConfig {
            mnemo_base_url: "http://localhost:99999".to_string(),
            api_key: None,
            default_user: None,
        });

        let result = read_resource(&server, "mnemo://agents/..evil/promotions").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("illegal characters"));
    }
}
