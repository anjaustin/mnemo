//! MCP resource definitions and dispatch for Mnemo.
//!
//! Resources are read-only data that MCP clients can access.

use serde_json::Value;

use crate::protocol::{ResourceContent, ResourceReadResult, ResourceTemplate};
use crate::McpServer;

/// Return all resource templates.
pub fn list_resource_templates() -> Vec<ResourceTemplate> {
    vec![
        ResourceTemplate {
            uri_template: "mnemo://users/{user}/memory".to_string(),
            name: "User Memory".to_string(),
            description: "Knowledge graph summary and coherence report for a user.".to_string(),
            mime_type: "application/json".to_string(),
        },
        ResourceTemplate {
            uri_template: "mnemo://users/{user}/episodes".to_string(),
            name: "Recent Episodes".to_string(),
            description: "List of recent memory episodes for a user.".to_string(),
            mime_type: "application/json".to_string(),
        },
        ResourceTemplate {
            uri_template: "mnemo://users/{user}/entities".to_string(),
            name: "User Entities".to_string(),
            description: "List of entities in the user's knowledge graph.".to_string(),
            mime_type: "application/json".to_string(),
        },
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
            description: "Recent experience events for agent identity evolution.".to_string(),
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
        let parts: Vec<&str> = rest.splitn(3, '/').collect();

        match parts.as_slice() {
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
            ["agents", agent_id, "identity"] => {
                validate_identifier(agent_id, "agent_id")?;
                read_agent_identity(server, agent_id, uri).await
            }
            ["agents", agent_id, "experience"] => {
                validate_identifier(agent_id, "agent_id")?;
                read_agent_experience(server, agent_id, uri).await
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_list_resource_templates_returns_all() {
        let templates = list_resource_templates();
        assert_eq!(templates.len(), 5);

        let uris: Vec<&str> = templates.iter().map(|t| t.uri_template.as_str()).collect();
        assert!(uris.contains(&"mnemo://users/{user}/memory"));
        assert!(uris.contains(&"mnemo://users/{user}/episodes"));
        assert!(uris.contains(&"mnemo://users/{user}/entities"));
        assert!(uris.contains(&"mnemo://agents/{agent_id}/identity"));
        assert!(uris.contains(&"mnemo://agents/{agent_id}/experience"));
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
}
