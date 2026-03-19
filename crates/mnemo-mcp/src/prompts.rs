//! MCP prompt templates for Mnemo.
//!
//! Prompts are user-invokable templates that generate structured messages
//! for the LLM. They allow agents to discover and use pre-built workflows.

use crate::protocol::{
    PromptArgument, PromptContent, PromptDefinition, PromptGetResult, PromptMessage,
};
use crate::McpServer;
use std::collections::HashMap;

/// Return all available prompt templates.
pub fn list_prompts() -> Vec<PromptDefinition> {
    vec![
        // ─── Memory Prompts ───────────────────────────────────────────
        PromptDefinition {
            name: "memory-context".to_string(),
            title: Some("Load Memory Context".to_string()),
            description: Some(
                "Retrieve relevant memories for a topic and format them as context for the conversation."
                    .to_string(),
            ),
            arguments: Some(vec![
                PromptArgument {
                    name: "topic".to_string(),
                    description: Some("The topic or question to search memories for".to_string()),
                    required: Some(true),
                },
                PromptArgument {
                    name: "user".to_string(),
                    description: Some("User identifier (optional if default is set)".to_string()),
                    required: Some(false),
                },
            ]),
        },
        PromptDefinition {
            name: "memory-summary".to_string(),
            title: Some("Summarize Memory".to_string()),
            description: Some(
                "Generate a summary of what is known about a user from their memory."
                    .to_string(),
            ),
            arguments: Some(vec![PromptArgument {
                name: "user".to_string(),
                description: Some("User identifier".to_string()),
                required: Some(true),
            }]),
        },
        // ─── Identity Prompts ─────────────────────────────────────────
        PromptDefinition {
            name: "identity-reflection".to_string(),
            title: Some("Identity Reflection".to_string()),
            description: Some(
                "Reflect on agent identity and recent experiences to suggest improvements."
                    .to_string(),
            ),
            arguments: Some(vec![PromptArgument {
                name: "agent_id".to_string(),
                description: Some("Agent identifier".to_string()),
                required: Some(true),
            }]),
        },
        // ─── Graph Prompts ────────────────────────────────────────────
        PromptDefinition {
            name: "entity-analysis".to_string(),
            title: Some("Analyze Entity".to_string()),
            description: Some(
                "Analyze an entity and its relationships in the knowledge graph."
                    .to_string(),
            ),
            arguments: Some(vec![
                PromptArgument {
                    name: "entity".to_string(),
                    description: Some("The entity name to analyze".to_string()),
                    required: Some(true),
                },
                PromptArgument {
                    name: "user".to_string(),
                    description: Some("User identifier".to_string()),
                    required: Some(true),
                },
            ]),
        },
        // ─── Conversation Prompts ─────────────────────────────────────
        PromptDefinition {
            name: "remember-conversation".to_string(),
            title: Some("Remember This Conversation".to_string()),
            description: Some(
                "Generate a memory-optimized summary of the current conversation for storage."
                    .to_string(),
            ),
            arguments: Some(vec![
                PromptArgument {
                    name: "conversation".to_string(),
                    description: Some("The conversation text to summarize".to_string()),
                    required: Some(true),
                },
                PromptArgument {
                    name: "user".to_string(),
                    description: Some("User identifier".to_string()),
                    required: Some(true),
                },
            ]),
        },
    ]
}

/// Get a specific prompt with arguments filled in.
pub async fn get_prompt(
    server: &McpServer,
    name: &str,
    arguments: Option<&HashMap<String, String>>,
) -> Result<PromptGetResult, String> {
    let args = arguments.cloned().unwrap_or_default();

    match name {
        "memory-context" => get_memory_context_prompt(server, &args).await,
        "memory-summary" => get_memory_summary_prompt(server, &args).await,
        "identity-reflection" => get_identity_reflection_prompt(server, &args).await,
        "entity-analysis" => get_entity_analysis_prompt(server, &args).await,
        "remember-conversation" => get_remember_conversation_prompt(&args),
        _ => Err(format!("Unknown prompt: {}", name)),
    }
}

async fn get_memory_context_prompt(
    server: &McpServer,
    args: &HashMap<String, String>,
) -> Result<PromptGetResult, String> {
    let topic = args
        .get("topic")
        .ok_or("Missing required argument: topic")?;

    let user = args
        .get("user")
        .map(|s| s.as_str())
        .or(server.config.default_user.as_deref())
        .ok_or("Missing required argument: user (and no default set)")?;

    // Fetch memories for context
    let path = format!(
        "/api/v1/users/{}/recall?query={}&limit=10",
        urlencoding::encode(user),
        urlencoding::encode(topic)
    );

    let memories = match server.get(&path).send().await {
        Ok(resp) if resp.status().is_success() => {
            resp.json::<serde_json::Value>()
                .await
                .ok()
                .and_then(|v| serde_json::to_string_pretty(&v).ok())
                .unwrap_or_else(|| "No memories found.".to_string())
        }
        _ => "Unable to retrieve memories.".to_string(),
    };

    Ok(PromptGetResult {
        description: Some(format!("Memory context for topic: {}", topic)),
        messages: vec![
            PromptMessage {
                role: "user".to_string(),
                content: PromptContent::Text {
                    text: format!(
                        "Here is relevant context from memory about \"{}\":\n\n{}\n\nPlease use this context to inform your response.",
                        topic, memories
                    ),
                },
            },
        ],
    })
}

async fn get_memory_summary_prompt(
    server: &McpServer,
    args: &HashMap<String, String>,
) -> Result<PromptGetResult, String> {
    let user = args
        .get("user")
        .ok_or("Missing required argument: user")?;

    // Fetch digest
    let path = format!("/api/v1/users/{}/digest", urlencoding::encode(user));

    let digest = match server.get(&path).send().await {
        Ok(resp) if resp.status().is_success() => {
            resp.json::<serde_json::Value>()
                .await
                .ok()
                .and_then(|v| v.get("digest").and_then(|d| d.as_str()).map(String::from))
                .unwrap_or_else(|| "No summary available.".to_string())
        }
        _ => "Unable to retrieve memory summary.".to_string(),
    };

    Ok(PromptGetResult {
        description: Some(format!("Memory summary for user: {}", user)),
        messages: vec![PromptMessage {
            role: "user".to_string(),
            content: PromptContent::Text {
                text: format!(
                    "Here is a summary of what I know about this user:\n\n{}\n\nPlease acknowledge this context.",
                    digest
                ),
            },
        }],
    })
}

async fn get_identity_reflection_prompt(
    server: &McpServer,
    args: &HashMap<String, String>,
) -> Result<PromptGetResult, String> {
    let agent_id = args
        .get("agent_id")
        .ok_or("Missing required argument: agent_id")?;

    // Fetch identity profile
    let identity_path = format!("/api/v1/agents/{}/identity", urlencoding::encode(agent_id));
    let experience_path = format!(
        "/api/v1/agents/{}/experience?limit=10",
        urlencoding::encode(agent_id)
    );

    let identity = match server.get(&identity_path).send().await {
        Ok(resp) if resp.status().is_success() => resp
            .json::<serde_json::Value>()
            .await
            .ok()
            .and_then(|v| serde_json::to_string_pretty(&v).ok())
            .unwrap_or_else(|| "No identity found.".to_string()),
        _ => "Unable to retrieve identity.".to_string(),
    };

    let experiences = match server.get(&experience_path).send().await {
        Ok(resp) if resp.status().is_success() => resp
            .json::<serde_json::Value>()
            .await
            .ok()
            .and_then(|v| serde_json::to_string_pretty(&v).ok())
            .unwrap_or_else(|| "No recent experiences.".to_string()),
        _ => "Unable to retrieve experiences.".to_string(),
    };

    Ok(PromptGetResult {
        description: Some(format!("Identity reflection for agent: {}", agent_id)),
        messages: vec![PromptMessage {
            role: "user".to_string(),
            content: PromptContent::Text {
                text: format!(
                    r#"Please reflect on this agent's identity and recent experiences, then suggest potential improvements or learnings.

## Current Identity Profile
{}

## Recent Experience Events
{}

Based on these experiences, what patterns do you notice? What identity aspects could be updated or refined?"#,
                    identity, experiences
                ),
            },
        }],
    })
}

async fn get_entity_analysis_prompt(
    server: &McpServer,
    args: &HashMap<String, String>,
) -> Result<PromptGetResult, String> {
    let entity = args
        .get("entity")
        .ok_or("Missing required argument: entity")?;
    let user = args
        .get("user")
        .ok_or("Missing required argument: user")?;

    // Fetch entity neighbors
    let path = format!(
        "/api/v1/graph/{}/neighbors?entity={}&depth=2",
        urlencoding::encode(user),
        urlencoding::encode(entity)
    );

    let graph_data = match server.get(&path).send().await {
        Ok(resp) if resp.status().is_success() => resp
            .json::<serde_json::Value>()
            .await
            .ok()
            .and_then(|v| serde_json::to_string_pretty(&v).ok())
            .unwrap_or_else(|| "No relationships found.".to_string()),
        _ => "Unable to retrieve entity relationships.".to_string(),
    };

    Ok(PromptGetResult {
        description: Some(format!("Entity analysis for: {}", entity)),
        messages: vec![PromptMessage {
            role: "user".to_string(),
            content: PromptContent::Text {
                text: format!(
                    r#"Please analyze this entity and its relationships in the knowledge graph.

## Entity: {}

## Relationships
{}

What can you tell me about this entity based on its connections? Are there any interesting patterns or insights?"#,
                    entity, graph_data
                ),
            },
        }],
    })
}

fn get_remember_conversation_prompt(
    args: &HashMap<String, String>,
) -> Result<PromptGetResult, String> {
    let conversation = args
        .get("conversation")
        .ok_or("Missing required argument: conversation")?;
    let user = args
        .get("user")
        .ok_or("Missing required argument: user")?;

    Ok(PromptGetResult {
        description: Some("Generate memory-optimized conversation summary".to_string()),
        messages: vec![PromptMessage {
            role: "user".to_string(),
            content: PromptContent::Text {
                text: format!(
                    r#"Please analyze this conversation and extract the key information that should be remembered for user "{}".

## Conversation
{}

Please provide:
1. A concise summary of the important facts and decisions
2. Any preferences, opinions, or personal information mentioned
3. Any commitments or follow-up items
4. Entities (people, places, organizations) mentioned and their relationships

Format your response as a structured memory that can be stored for future recall."#,
                    user, conversation
                ),
            },
        }],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_list_prompts_returns_all() {
        let prompts = list_prompts();
        assert_eq!(prompts.len(), 5);

        let names: Vec<&str> = prompts.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"memory-context"));
        assert!(names.contains(&"memory-summary"));
        assert!(names.contains(&"identity-reflection"));
        assert!(names.contains(&"entity-analysis"));
        assert!(names.contains(&"remember-conversation"));
    }

    #[test]
    fn test_all_prompts_have_descriptions() {
        for prompt in list_prompts() {
            assert!(prompt.description.is_some(), "Prompt {} missing description", prompt.name);
        }
    }

    #[test]
    fn test_all_prompts_have_titles() {
        for prompt in list_prompts() {
            assert!(prompt.title.is_some(), "Prompt {} missing title", prompt.name);
        }
    }

    #[test]
    fn test_prompt_definitions_serialize() {
        let prompts = list_prompts();
        let json = serde_json::to_value(&prompts).unwrap();
        assert!(json.is_array());
        assert_eq!(json.as_array().unwrap().len(), 5);
    }

    #[tokio::test]
    async fn test_get_unknown_prompt_returns_error() {
        let server = McpServer::new(crate::McpConfig {
            mnemo_base_url: "http://localhost:99999".to_string(),
            api_key: None,
            default_user: None,
        });

        let result = get_prompt(&server, "nonexistent-prompt", None).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unknown prompt"));
    }

    #[tokio::test]
    async fn test_memory_context_requires_topic() {
        let server = McpServer::new(crate::McpConfig {
            mnemo_base_url: "http://localhost:99999".to_string(),
            api_key: None,
            default_user: Some("test-user".to_string()),
        });

        let args = HashMap::new();
        let result = get_prompt(&server, "memory-context", Some(&args)).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("topic"));
    }

    #[tokio::test]
    async fn test_remember_conversation_requires_args() {
        let server = McpServer::new(crate::McpConfig {
            mnemo_base_url: "http://localhost:99999".to_string(),
            api_key: None,
            default_user: None,
        });

        let args = HashMap::new();
        let result = get_prompt(&server, "remember-conversation", Some(&args)).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("conversation"));
    }

    #[tokio::test]
    async fn test_remember_conversation_generates_prompt() {
        let server = McpServer::new(crate::McpConfig {
            mnemo_base_url: "http://localhost:99999".to_string(),
            api_key: None,
            default_user: None,
        });

        let mut args = HashMap::new();
        args.insert("conversation".to_string(), "User: Hello\nAssistant: Hi there!".to_string());
        args.insert("user".to_string(), "alice".to_string());

        let result = get_prompt(&server, "remember-conversation", Some(&args)).await;
        assert!(result.is_ok());

        let prompt = result.unwrap();
        assert!(!prompt.messages.is_empty());
        assert_eq!(prompt.messages[0].role, "user");
    }
}
