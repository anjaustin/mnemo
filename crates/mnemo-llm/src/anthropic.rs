use reqwest::Client;
use serde::{Deserialize, Serialize};

use mnemo_core::error::MnemoError;
use mnemo_core::models::edge::ExtractedRelationship;
use mnemo_core::models::entity::{EntityType, ExtractedEntity};
use mnemo_core::traits::llm::{ExtractionResult, LlmConfig, LlmProvider, LlmResult};

use crate::openai_compat::EXTRACTION_SYSTEM_PROMPT;

/// Native Anthropic Messages API provider.
///
/// Uses Anthropic's native format:
/// - System prompt is a top-level `system` parameter
/// - Endpoint is `/v1/messages`
/// - Response uses `content` blocks, not `choices`
pub struct AnthropicProvider {
    client: Client,
    config: LlmConfig,
    base_url: String,
}

#[derive(Serialize)]
struct AnthropicRequest {
    model: String,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "is_zero")]
    temperature: f32,
}

fn is_zero(v: &f32) -> bool {
    *v == 0.0
}

#[derive(Serialize, Deserialize)]
struct AnthropicMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct AnthropicResponse {
    content: Vec<ContentBlock>,
}

#[derive(Deserialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    block_type: String,
    #[serde(default)]
    text: String,
}

#[derive(Deserialize)]
struct ExtractionResponse {
    #[serde(default)]
    entities: Vec<RawEntity>,
    #[serde(default)]
    relationships: Vec<RawRelationship>,
}

#[derive(Deserialize)]
struct RawEntity {
    name: String,
    #[serde(rename = "type")]
    entity_type: String,
    summary: Option<String>,
}

#[derive(Deserialize)]
struct RawRelationship {
    source: String,
    target: String,
    label: String,
    fact: String,
    #[serde(default = "default_confidence")]
    confidence: f32,
}

fn default_confidence() -> f32 {
    0.8
}

impl AnthropicProvider {
    pub fn new(config: LlmConfig) -> Self {
        let base_url = config
            .base_url
            .clone()
            .unwrap_or_else(|| "https://api.anthropic.com".to_string());
        let client = Client::new();
        Self {
            client,
            config,
            base_url,
        }
    }

    async fn message(&self, system: &str, user_msg: &str) -> LlmResult<String> {
        let request = AnthropicRequest {
            model: self.config.model.clone(),
            max_tokens: self.config.max_tokens,
            system: Some(system.to_string()),
            messages: vec![AnthropicMessage {
                role: "user".into(),
                content: user_msg.into(),
            }],
            temperature: self.config.temperature,
        };

        let api_key = self
            .config
            .api_key
            .as_deref()
            .ok_or_else(|| MnemoError::LlmProvider {
                provider: "anthropic".into(),
                message: "API key is required for Anthropic".into(),
            })?;

        let response = self
            .client
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| MnemoError::LlmProvider {
                provider: "anthropic".into(),
                message: format!("Request failed: {}", e),
            })?;

        if response.status() == 429 {
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(5);
            return Err(MnemoError::RateLimited {
                retry_after_ms: retry_after * 1000,
            });
        }

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(MnemoError::LlmProvider {
                provider: "anthropic".into(),
                message: format!("HTTP {}: {}", status, body),
            });
        }

        let api_response: AnthropicResponse =
            response.json().await.map_err(|e| MnemoError::LlmProvider {
                provider: "anthropic".into(),
                message: format!("Failed to parse response: {}", e),
            })?;

        let text = api_response
            .content
            .iter()
            .filter(|b| b.block_type == "text")
            .map(|b| b.text.as_str())
            .collect::<Vec<_>>()
            .join("");

        if text.is_empty() {
            return Err(MnemoError::LlmProvider {
                provider: "anthropic".into(),
                message: "No text content in response".into(),
            });
        }

        Ok(text)
    }

    fn parse_extraction(raw: &str) -> LlmResult<ExtractionResponse> {
        let cleaned = raw.trim();
        let cleaned = if cleaned.starts_with("```") {
            let start = cleaned.find('\n').map(|i| i + 1).unwrap_or(0);
            let end = cleaned.rfind("```").unwrap_or(cleaned.len());
            &cleaned[start..end]
        } else {
            cleaned
        };
        serde_json::from_str(cleaned).map_err(|e| {
            MnemoError::ExtractionFailed(format!(
                "Failed to parse extraction JSON: {}. Raw: {}",
                e,
                &raw[..raw.len().min(200)]
            ))
        })
    }
}

impl LlmProvider for AnthropicProvider {
    async fn extract_entities_and_relationships(
        &self,
        content: &str,
        existing_entities: &[ExtractedEntity],
    ) -> LlmResult<ExtractionResult> {
        let mut user_msg = format!(
            "Extract entities and relationships from this text:\n\n{}",
            content
        );
        if !existing_entities.is_empty() {
            let names: Vec<&str> = existing_entities.iter().map(|e| e.name.as_str()).collect();
            user_msg.push_str(&format!(
                "\n\nExisting entities (reuse these names if they appear): {}",
                names.join(", ")
            ));
        }

        let raw = self.message(EXTRACTION_SYSTEM_PROMPT, &user_msg).await?;
        let parsed = Self::parse_extraction(&raw)?;

        Ok(ExtractionResult {
            entities: parsed
                .entities
                .into_iter()
                .map(|e| ExtractedEntity {
                    name: e.name,
                    entity_type: EntityType::from_str_flexible(&e.entity_type),
                    summary: e.summary,
                })
                .collect(),
            relationships: parsed
                .relationships
                .into_iter()
                .map(|r| ExtractedRelationship {
                    source_name: r.source,
                    target_name: r.target,
                    label: r.label,
                    fact: r.fact,
                    confidence: r.confidence,
                    valid_at: None,
                })
                .collect(),
        })
    }

    async fn summarize(&self, content: &str, max_tokens: u32) -> LlmResult<String> {
        let system = format!(
            "Summarize the following content concisely in {} tokens or fewer. Focus on key facts.",
            max_tokens
        );
        self.message(&system, content).await
    }

    async fn detect_contradictions(
        &self,
        new_fact: &str,
        existing_facts: &[String],
    ) -> LlmResult<Vec<String>> {
        if existing_facts.is_empty() {
            return Ok(Vec::new());
        }
        let existing = existing_facts
            .iter()
            .enumerate()
            .map(|(i, f)| format!("{}. {}", i + 1, f))
            .collect::<Vec<_>>()
            .join("\n");
        let system = "You detect contradictions between facts. Respond with a JSON array of strings. If none, respond with [].";
        let user_msg = format!(
            "New fact: {}\n\nExisting facts:\n{}\n\nList contradictions as JSON array.",
            new_fact, existing
        );
        let raw = self.message(system, &user_msg).await?;
        let cleaned = raw.trim();
        let cleaned = if cleaned.starts_with("```") {
            let s = cleaned.find('\n').map(|i| i + 1).unwrap_or(0);
            let e = cleaned.rfind("```").unwrap_or(cleaned.len());
            &cleaned[s..e]
        } else {
            cleaned
        };
        Ok(serde_json::from_str(cleaned).unwrap_or_else(|_| Vec::new()))
    }

    fn provider_name(&self) -> &str {
        "anthropic"
    }
    fn model_name(&self) -> &str {
        &self.config.model
    }
}

// ─── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use mnemo_core::traits::llm::{LlmConfig, LlmProvider};
    use wiremock::matchers::{body_string_contains, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn anthropic_config(base_url: &str) -> LlmConfig {
        LlmConfig {
            provider: "anthropic".to_string(),
            api_key: Some("test-ant-key".to_string()),
            model: "claude-sonnet-4-20250514".to_string(),
            base_url: Some(base_url.to_string()),
            temperature: 0.0,
            max_tokens: 2048,
        }
    }

    fn anthropic_response(text: &str) -> String {
        serde_json::json!({
            "content": [{"type": "text", "text": text}]
        })
        .to_string()
    }

    fn valid_extraction_json() -> &'static str {
        r#"{
            "entities": [
                {"name": "Alice", "type": "person", "summary": "An engineer"}
            ],
            "relationships": [
                {"source": "Alice", "target": "Acme Corp", "label": "works_at", "fact": "Alice works at Acme Corp", "confidence": 0.9}
            ]
        }"#
    }

    // ── LLM-02: Anthropic provider constructs valid prompts ──

    #[tokio::test]
    async fn test_anthropic_extraction_constructs_valid_prompt() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .and(header("x-api-key", "test-ant-key"))
            .and(header("anthropic-version", "2023-06-01"))
            .and(body_string_contains("Extract entities and relationships"))
            .and(body_string_contains("Alice joined Acme Corp"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(anthropic_response(valid_extraction_json())),
            )
            .expect(1)
            .mount(&server)
            .await;

        let provider = AnthropicProvider::new(anthropic_config(&server.uri()));
        let result = provider
            .extract_entities_and_relationships("Alice joined Acme Corp", &[])
            .await
            .expect("extraction should succeed");

        assert_eq!(result.entities.len(), 1);
        assert_eq!(result.entities[0].name, "Alice");
        assert_eq!(result.relationships.len(), 1);
        assert_eq!(result.relationships[0].label, "works_at");
    }

    // ── LLM-02b: Anthropic requires API key ──

    #[tokio::test]
    async fn test_anthropic_requires_api_key() {
        let config = LlmConfig {
            provider: "anthropic".to_string(),
            api_key: None,
            model: "claude-sonnet-4-20250514".to_string(),
            base_url: Some("http://unused:1234".to_string()),
            temperature: 0.0,
            max_tokens: 2048,
        };

        let provider = AnthropicProvider::new(config);
        let result = provider.summarize("text", 100).await;

        assert!(result.is_err());
        match result.unwrap_err() {
            MnemoError::LlmProvider { message, .. } => {
                assert!(
                    message.contains("API key"),
                    "should mention API key: {}",
                    message
                );
            }
            other => panic!("expected LlmProvider error, got {:?}", other),
        }
    }

    // ── LLM-03 (Anthropic): Handles malformed JSON ──

    #[tokio::test]
    async fn test_anthropic_handles_malformed_json() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(anthropic_response("not valid json {{")),
            )
            .mount(&server)
            .await;

        let provider = AnthropicProvider::new(anthropic_config(&server.uri()));
        let result = provider
            .extract_entities_and_relationships("test", &[])
            .await;

        assert!(result.is_err(), "malformed JSON should error, not panic");
        match result.unwrap_err() {
            MnemoError::ExtractionFailed(_) => {}
            other => panic!("expected ExtractionFailed, got {:?}", other),
        }
    }

    // ── LLM-04 (Anthropic): Handles 429 rate limit ──

    #[tokio::test]
    async fn test_anthropic_handles_429_rate_limit() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "30"))
            .mount(&server)
            .await;

        let provider = AnthropicProvider::new(anthropic_config(&server.uri()));
        let result = provider.summarize("text", 100).await;

        match result.unwrap_err() {
            MnemoError::RateLimited { retry_after_ms } => {
                assert_eq!(retry_after_ms, 30_000);
            }
            other => panic!("expected RateLimited, got {:?}", other),
        }
    }

    // ── Anthropic handles empty content response ──

    #[tokio::test]
    async fn test_anthropic_handles_empty_content() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"content":[]}"#))
            .mount(&server)
            .await;

        let provider = AnthropicProvider::new(anthropic_config(&server.uri()));
        let result = provider.summarize("text", 100).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            MnemoError::LlmProvider { message, .. } => {
                assert!(message.contains("No text content"), "error: {}", message);
            }
            other => panic!("expected LlmProvider, got {:?}", other),
        }
    }

    // ── Provider name and model name ──

    #[tokio::test]
    async fn test_anthropic_provider_name_and_model() {
        let provider = AnthropicProvider::new(anthropic_config("http://unused:1234"));
        assert_eq!(provider.provider_name(), "anthropic");
        assert_eq!(provider.model_name(), "claude-sonnet-4-20250514");
    }

    // ── Base URL default ──

    #[test]
    fn test_anthropic_base_url_default() {
        let config = LlmConfig {
            provider: "anthropic".into(),
            api_key: Some("key".into()),
            model: "claude-sonnet-4-20250514".into(),
            base_url: None,
            temperature: 0.0,
            max_tokens: 2048,
        };
        let provider = AnthropicProvider::new(config);
        assert_eq!(provider.base_url, "https://api.anthropic.com");
    }
}
