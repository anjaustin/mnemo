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
            uri_template: "mnemo://agents/{agent_id}/identity".to_string(),
            name: "Agent Identity".to_string(),
            description: "Agent identity profile including core, experience, and version info."
                .to_string(),
            mime_type: "application/json".to_string(),
        },
    ]
}

/// Parse a resource URI and dispatch to the appropriate handler.
pub async fn read_resource(server: &McpServer, uri: &str) -> Result<ResourceReadResult, String> {
    // Parse mnemo:// URIs
    if let Some(rest) = uri.strip_prefix("mnemo://") {
        let parts: Vec<&str> = rest.splitn(3, '/').collect();

        match parts.as_slice() {
            ["users", user, "memory"] => read_user_memory(server, user, uri).await,
            ["agents", agent_id, "identity"] => {
                read_agent_identity(server, agent_id, uri).await
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_list_resource_templates_returns_2() {
        let templates = list_resource_templates();
        assert_eq!(templates.len(), 2);

        let uris: Vec<&str> = templates.iter().map(|t| t.uri_template.as_str()).collect();
        assert!(uris.contains(&"mnemo://users/{user}/memory"));
        assert!(uris.contains(&"mnemo://agents/{agent_id}/identity"));
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
}
