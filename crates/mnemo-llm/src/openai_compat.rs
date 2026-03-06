use reqwest::Client;
use serde::{Deserialize, Serialize};

use mnemo_core::error::MnemoError;
use mnemo_core::models::edge::ExtractedRelationship;
use mnemo_core::models::entity::{EntityType, ExtractedEntity};
use mnemo_core::traits::llm::{
    EmbeddingConfig, EmbeddingProvider, ExtractionResult, LlmConfig, LlmProvider, LlmResult,
};

/// OpenAI-compatible LLM provider.
///
/// Works with: OpenAI, Ollama, Liquid AI, vLLM, LM Studio, Together AI,
/// and any provider implementing the OpenAI chat completions API.
pub struct OpenAiCompatibleProvider {
    client: Client,
    config: LlmConfig,
    base_url: String,
}

/// OpenAI-compatible embedding provider.
pub struct OpenAiCompatibleEmbedder {
    client: Client,
    config: EmbeddingConfig,
    base_url: String,
}

// ─── Chat completion types ─────────────────────────────────────────

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    temperature: f32,
    max_tokens: u32,
}

#[derive(Serialize, Deserialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

// ─── Embedding types ───────────────────────────────────────────────

#[derive(Serialize)]
struct EmbedRequest {
    model: String,
    input: Vec<String>,
}

#[derive(Deserialize)]
struct EmbedResponse {
    data: Vec<EmbedData>,
}

#[derive(Deserialize)]
struct EmbedData {
    embedding: Vec<f32>,
}

// ─── Extraction prompt ─────────────────────────────────────────────

pub const EXTRACTION_SYSTEM_PROMPT: &str = r#"You are an entity and relationship extraction engine for a memory system. Given text, extract:

1. **Entities**: People, organizations, products, locations, events, concepts mentioned.
2. **Relationships**: How entities relate to each other, with temporal context.

Respond ONLY with valid JSON in this exact format:
{
  "entities": [
    {"name": "Entity Name", "type": "person|organization|product|location|event|concept", "summary": "Brief description"}
  ],
  "relationships": [
    {"source": "Source Entity", "target": "Target Entity", "label": "relationship_label", "fact": "Natural language fact description", "confidence": 0.95}
  ]
}

Rules:
- Use the canonical/full name for entities (e.g., "Nike" not "they")
- Relationship labels should be lowercase_snake_case (e.g., "prefers", "works_at", "purchased")
- Confidence is 0.0-1.0 based on how explicit the relationship is in the text
- Extract temporal cues when present (dates, relative time references)
- If existing entities are provided, reuse their exact names for consistency
- Do NOT hallucinate entities or relationships not supported by the text"#;

// ─── LLM extraction response parsing ──────────────────────────────

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

// ─── Implementation ────────────────────────────────────────────────

impl OpenAiCompatibleProvider {
    pub fn new(config: LlmConfig) -> Self {
        let base_url = config
            .base_url
            .clone()
            .unwrap_or_else(|| match config.provider.as_str() {
                "ollama" => "http://localhost:11434/v1".to_string(),
                "liquid" => "http://localhost:8000/v1".to_string(),
                _ => "https://api.openai.com/v1".to_string(),
            });

        let client = Client::new();
        Self {
            client,
            config,
            base_url,
        }
    }

    async fn chat_completion(&self, system: &str, user_msg: &str) -> LlmResult<String> {
        let request = ChatRequest {
            model: self.config.model.clone(),
            messages: vec![
                ChatMessage {
                    role: "system".into(),
                    content: system.into(),
                },
                ChatMessage {
                    role: "user".into(),
                    content: user_msg.into(),
                },
            ],
            temperature: self.config.temperature,
            max_tokens: self.config.max_tokens,
        };

        let mut req_builder = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .json(&request);

        // Add auth header
        if let Some(ref key) = self.config.api_key {
            req_builder = req_builder.header("Authorization", format!("Bearer {}", key));
        }

        let response = req_builder
            .send()
            .await
            .map_err(|e| MnemoError::LlmProvider {
                provider: self.config.provider.clone(),
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
                provider: self.config.provider.clone(),
                message: format!("HTTP {}: {}", status, body),
            });
        }

        let chat_response: ChatResponse =
            response.json().await.map_err(|e| MnemoError::LlmProvider {
                provider: self.config.provider.clone(),
                message: format!("Failed to parse response: {}", e),
            })?;

        chat_response
            .choices
            .first()
            .map(|c| c.message.content.clone())
            .ok_or_else(|| MnemoError::LlmProvider {
                provider: self.config.provider.clone(),
                message: "No choices in response".into(),
            })
    }

    /// Parse the JSON extraction response, handling markdown code fences.
    fn parse_extraction(raw: &str) -> LlmResult<ExtractionResponse> {
        // Strip markdown code fences if present
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

impl LlmProvider for OpenAiCompatibleProvider {
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
                "\n\nExisting entities in this user's graph (reuse these names if they appear): {}",
                names.join(", ")
            ));
        }

        let raw = self
            .chat_completion(EXTRACTION_SYSTEM_PROMPT, &user_msg)
            .await?;
        let parsed = Self::parse_extraction(&raw)?;

        let entities = parsed
            .entities
            .into_iter()
            .map(|e| ExtractedEntity {
                name: e.name,
                entity_type: EntityType::from_str_flexible(&e.entity_type),
                summary: e.summary,
            })
            .collect();

        let relationships = parsed
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
            .collect();

        Ok(ExtractionResult {
            entities,
            relationships,
        })
    }

    async fn summarize(&self, content: &str, max_tokens: u32) -> LlmResult<String> {
        let system = format!(
            "Summarize the following content concisely in {} tokens or fewer. \
             Focus on key facts, entities, and relationships.",
            max_tokens
        );
        self.chat_completion(&system, content).await
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

        let system =
            "You detect contradictions between facts. Respond with a JSON array of strings \
                       describing each contradiction found. If no contradictions, respond with [].";
        let user_msg = format!(
            "New fact: {}\n\nExisting facts:\n{}\n\nList contradictions as a JSON array of strings.",
            new_fact, existing
        );

        let raw = self.chat_completion(system, &user_msg).await?;
        let cleaned = raw.trim();
        let cleaned = if cleaned.starts_with("```") {
            let start = cleaned.find('\n').map(|i| i + 1).unwrap_or(0);
            let end = cleaned.rfind("```").unwrap_or(cleaned.len());
            &cleaned[start..end]
        } else {
            cleaned
        };

        Ok(serde_json::from_str(cleaned).unwrap_or_else(|_| Vec::new()))
    }

    fn provider_name(&self) -> &str {
        &self.config.provider
    }

    fn model_name(&self) -> &str {
        &self.config.model
    }
}

// ─── Embedding Provider ────────────────────────────────────────────

impl OpenAiCompatibleEmbedder {
    pub fn new(config: EmbeddingConfig) -> Self {
        let base_url = config
            .base_url
            .clone()
            .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
        let client = Client::new();
        Self {
            client,
            config,
            base_url,
        }
    }
}

impl EmbeddingProvider for OpenAiCompatibleEmbedder {
    async fn embed(&self, text: &str) -> LlmResult<Vec<f32>> {
        let batch = self.embed_batch(&[text.to_string()]).await?;
        batch
            .into_iter()
            .next()
            .ok_or_else(|| MnemoError::EmbeddingProvider {
                provider: self.config.provider.clone(),
                message: "Empty embedding response".into(),
            })
    }

    async fn embed_batch(&self, texts: &[String]) -> LlmResult<Vec<Vec<f32>>> {
        let request = EmbedRequest {
            model: self.config.model.clone(),
            input: texts.to_vec(),
        };

        let mut req_builder = self
            .client
            .post(format!("{}/embeddings", self.base_url))
            .json(&request);

        if let Some(ref key) = self.config.api_key {
            req_builder = req_builder.header("Authorization", format!("Bearer {}", key));
        }

        let response = req_builder
            .send()
            .await
            .map_err(|e| MnemoError::EmbeddingProvider {
                provider: self.config.provider.clone(),
                message: format!("Request failed: {}", e),
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(MnemoError::EmbeddingProvider {
                provider: self.config.provider.clone(),
                message: format!("HTTP {}: {}", status, body),
            });
        }

        let embed_response: EmbedResponse =
            response
                .json()
                .await
                .map_err(|e| MnemoError::EmbeddingProvider {
                    provider: self.config.provider.clone(),
                    message: format!("Failed to parse response: {}", e),
                })?;

        Ok(embed_response
            .data
            .into_iter()
            .map(|d| d.embedding)
            .collect())
    }

    fn dimensions(&self) -> u32 {
        self.config.dimensions
    }

    fn provider_name(&self) -> &str {
        &self.config.provider
    }
}

// ─── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use mnemo_core::traits::llm::{EmbeddingConfig, EmbeddingProvider, LlmConfig, LlmProvider};
    use wiremock::matchers::{body_string_contains, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn openai_config(base_url: &str) -> LlmConfig {
        LlmConfig {
            provider: "openai-test".to_string(),
            api_key: Some("test-key-123".to_string()),
            model: "gpt-4o-mini".to_string(),
            base_url: Some(base_url.to_string()),
            temperature: 0.0,
            max_tokens: 2048,
        }
    }

    fn embedding_config(base_url: &str) -> EmbeddingConfig {
        EmbeddingConfig {
            provider: "openai-test".to_string(),
            api_key: Some("test-key-123".to_string()),
            model: "text-embedding-3-small".to_string(),
            base_url: Some(base_url.to_string()),
            dimensions: 1536,
        }
    }

    fn valid_extraction_json() -> &'static str {
        r#"{
            "entities": [
                {"name": "Kendra", "type": "person", "summary": "A customer"},
                {"name": "Nike", "type": "organization", "summary": "Shoe brand"}
            ],
            "relationships": [
                {"source": "Kendra", "target": "Nike", "label": "prefers", "fact": "Kendra prefers Nike shoes", "confidence": 0.95}
            ]
        }"#
    }

    fn chat_response(content: &str) -> String {
        serde_json::json!({
            "choices": [{
                "message": {"role": "assistant", "content": content}
            }]
        })
        .to_string()
    }

    // ── LLM-01: OpenAI-compatible provider constructs valid prompts ──

    #[tokio::test]
    async fn test_openai_extraction_constructs_valid_prompt() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .and(header("Authorization", "Bearer test-key-123"))
            .and(body_string_contains("Extract entities and relationships"))
            .and(body_string_contains("Kendra bought Nike shoes yesterday"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(chat_response(valid_extraction_json())),
            )
            .expect(1)
            .mount(&server)
            .await;

        let provider = OpenAiCompatibleProvider::new(openai_config(&server.uri()));
        let result = provider
            .extract_entities_and_relationships("Kendra bought Nike shoes yesterday", &[])
            .await
            .expect("extraction should succeed");

        assert_eq!(result.entities.len(), 2);
        assert_eq!(result.entities[0].name, "Kendra");
        assert_eq!(result.relationships.len(), 1);
        assert_eq!(result.relationships[0].label, "prefers");
        assert!((result.relationships[0].confidence - 0.95).abs() < 0.01);
    }

    // ── LLM-01b: Extraction includes existing entity names in prompt ──

    #[tokio::test]
    async fn test_openai_extraction_includes_existing_entities() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .and(body_string_contains("Existing entities"))
            .and(body_string_contains("Kendra"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(chat_response(valid_extraction_json())),
            )
            .expect(1)
            .mount(&server)
            .await;

        let existing = vec![ExtractedEntity {
            name: "Kendra".to_string(),
            entity_type: EntityType::Person,
            summary: Some("A customer".to_string()),
        }];

        let provider = OpenAiCompatibleProvider::new(openai_config(&server.uri()));
        let result = provider
            .extract_entities_and_relationships("She bought new shoes", &existing)
            .await;
        assert!(result.is_ok());
    }

    // ── LLM-03: Provider handles malformed LLM response gracefully ──

    #[tokio::test]
    async fn test_openai_handles_malformed_json_gracefully() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(chat_response("this is not valid json at all {{{")),
            )
            .mount(&server)
            .await;

        let provider = OpenAiCompatibleProvider::new(openai_config(&server.uri()));
        let result = provider
            .extract_entities_and_relationships("some text", &[])
            .await;

        assert!(
            result.is_err(),
            "malformed JSON should return an error, not panic"
        );
        let err = result.unwrap_err();
        match err {
            MnemoError::ExtractionFailed(msg) => {
                assert!(
                    msg.contains("parse"),
                    "error should mention parsing: {}",
                    msg
                );
            }
            other => panic!("expected ExtractionFailed, got {:?}", other),
        }
    }

    // ── LLM-03b: Provider handles empty choices array ──

    #[tokio::test]
    async fn test_openai_handles_empty_choices() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"choices":[]}"#))
            .mount(&server)
            .await;

        let provider = OpenAiCompatibleProvider::new(openai_config(&server.uri()));
        let result = provider.summarize("some text", 100).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            MnemoError::LlmProvider { message, .. } => {
                assert!(message.contains("No choices"), "error: {}", message);
            }
            other => panic!("expected LlmProvider error, got {:?}", other),
        }
    }

    // ── LLM-04: Provider handles rate limit (429) with retry_after_ms ──

    #[tokio::test]
    async fn test_openai_handles_429_rate_limit() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "10"))
            .mount(&server)
            .await;

        let provider = OpenAiCompatibleProvider::new(openai_config(&server.uri()));
        let result = provider.summarize("some text", 100).await;

        assert!(result.is_err());
        match result.unwrap_err() {
            MnemoError::RateLimited { retry_after_ms } => {
                assert_eq!(
                    retry_after_ms, 10_000,
                    "retry-after: 10 should become 10000ms"
                );
            }
            other => panic!("expected RateLimited, got {:?}", other),
        }
    }

    // ── LLM-04b: 429 without retry-after header defaults to 5s ──

    #[tokio::test]
    async fn test_openai_429_defaults_to_5s_without_retry_header() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(429))
            .mount(&server)
            .await;

        let provider = OpenAiCompatibleProvider::new(openai_config(&server.uri()));
        let result = provider.summarize("text", 100).await;

        match result.unwrap_err() {
            MnemoError::RateLimited { retry_after_ms } => {
                assert_eq!(retry_after_ms, 5_000, "default retry should be 5000ms");
            }
            other => panic!("expected RateLimited, got {:?}", other),
        }
    }

    // ── LLM-05: Provider handles non-2xx errors gracefully ──

    #[tokio::test]
    async fn test_openai_handles_400_error() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(
                ResponseTemplate::new(400)
                    .set_body_string(r#"{"error":{"message":"token limit exceeded"}}"#),
            )
            .mount(&server)
            .await;

        let provider = OpenAiCompatibleProvider::new(openai_config(&server.uri()));
        let result = provider
            .summarize("x".repeat(1_000_000).as_str(), 100)
            .await;

        assert!(result.is_err());
        match result.unwrap_err() {
            MnemoError::LlmProvider { provider, message } => {
                assert_eq!(provider, "openai-test");
                assert!(
                    message.contains("400"),
                    "should contain status: {}",
                    message
                );
            }
            other => panic!("expected LlmProvider error, got {:?}", other),
        }
    }

    // ── LLM-05b: Provider handles 500 server error ──

    #[tokio::test]
    async fn test_openai_handles_500_error() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(500).set_body_string("Internal Server Error"))
            .mount(&server)
            .await;

        let provider = OpenAiCompatibleProvider::new(openai_config(&server.uri()));
        let result = provider.summarize("text", 100).await;

        assert!(result.is_err());
        match result.unwrap_err() {
            MnemoError::LlmProvider { message, .. } => {
                assert!(
                    message.contains("500"),
                    "should contain status: {}",
                    message
                );
            }
            other => panic!("expected LlmProvider, got {:?}", other),
        }
    }

    // ── LLM-03c: Extraction handles markdown code fences ──

    #[tokio::test]
    async fn test_openai_extraction_handles_code_fences() {
        let server = MockServer::start().await;

        let fenced = format!("```json\n{}\n```", valid_extraction_json());

        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_string(chat_response(&fenced)))
            .mount(&server)
            .await;

        let provider = OpenAiCompatibleProvider::new(openai_config(&server.uri()));
        let result = provider
            .extract_entities_and_relationships("test text", &[])
            .await;
        assert!(
            result.is_ok(),
            "code-fenced JSON should parse correctly: {:?}",
            result.err()
        );
        assert_eq!(result.unwrap().entities.len(), 2);
    }

    // ── Summarize returns content correctly ──

    #[tokio::test]
    async fn test_openai_summarize_returns_content() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(chat_response("This is a summary.")),
            )
            .mount(&server)
            .await;

        let provider = OpenAiCompatibleProvider::new(openai_config(&server.uri()));
        let result = provider.summarize("long text here", 100).await.unwrap();
        assert_eq!(result, "This is a summary.");
    }

    // ── Detect contradictions returns parsed array ──

    #[tokio::test]
    async fn test_openai_detect_contradictions() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_string(chat_response(
                r#"["Kendra now prefers Adidas, contradicting her Nike preference"]"#,
            )))
            .mount(&server)
            .await;

        let provider = OpenAiCompatibleProvider::new(openai_config(&server.uri()));
        let result = provider
            .detect_contradictions(
                "Kendra prefers Adidas",
                &["Kendra prefers Nike".to_string()],
            )
            .await
            .unwrap();
        assert_eq!(result.len(), 1);
        assert!(result[0].contains("contradicting"));
    }

    // ── Detect contradictions with empty existing facts returns empty ──

    #[tokio::test]
    async fn test_openai_detect_contradictions_empty_facts() {
        // No server needed — this should return early
        let provider = OpenAiCompatibleProvider::new(openai_config("http://unused:1234"));
        let result = provider
            .detect_contradictions("new fact", &[])
            .await
            .unwrap();
        assert!(result.is_empty());
    }

    // ── LLM-07: Embedder returns correct dimensions config ──

    #[tokio::test]
    async fn test_embedder_dimensions_match_config() {
        let embedder = OpenAiCompatibleEmbedder::new(EmbeddingConfig {
            provider: "test".into(),
            api_key: None,
            model: "test-model".into(),
            base_url: Some("http://unused:1234".into()),
            dimensions: 768,
        });
        assert_eq!(embedder.dimensions(), 768);
    }

    // ── LLM-07b: Embedder returns embeddings from mock server ──

    #[tokio::test]
    async fn test_embedder_returns_embeddings() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/embeddings"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(r#"{"data":[{"embedding":[0.1, 0.2, 0.3]}]}"#),
            )
            .mount(&server)
            .await;

        let embedder = OpenAiCompatibleEmbedder::new(embedding_config(&server.uri()));
        let result = embedder.embed("test text").await.unwrap();
        assert_eq!(result.len(), 3);
        assert!((result[0] - 0.1).abs() < 0.001);
    }

    // ── LLM-07c: Embedder handles error response ──

    #[tokio::test]
    async fn test_embedder_handles_error_response() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/embeddings"))
            .respond_with(ResponseTemplate::new(400).set_body_string("dimension mismatch"))
            .mount(&server)
            .await;

        let embedder = OpenAiCompatibleEmbedder::new(embedding_config(&server.uri()));
        let result = embedder.embed("test").await;
        assert!(result.is_err());
        match result.unwrap_err() {
            MnemoError::EmbeddingProvider { message, .. } => {
                assert!(
                    message.contains("400"),
                    "should contain status: {}",
                    message
                );
            }
            other => panic!("expected EmbeddingProvider error, got {:?}", other),
        }
    }

    // ── Provider name and model name ──

    #[tokio::test]
    async fn test_openai_provider_name_and_model() {
        let provider = OpenAiCompatibleProvider::new(openai_config("http://unused:1234"));
        assert_eq!(provider.provider_name(), "openai-test");
        assert_eq!(provider.model_name(), "gpt-4o-mini");
    }

    // ── Base URL defaults ──

    #[test]
    fn test_openai_base_url_defaults() {
        let ollama = OpenAiCompatibleProvider::new(LlmConfig {
            provider: "ollama".into(),
            api_key: None,
            model: "llama3".into(),
            base_url: None,
            temperature: 0.0,
            max_tokens: 2048,
        });
        assert_eq!(ollama.base_url, "http://localhost:11434/v1");

        let liquid = OpenAiCompatibleProvider::new(LlmConfig {
            provider: "liquid".into(),
            api_key: None,
            model: "lfm-7b".into(),
            base_url: None,
            temperature: 0.0,
            max_tokens: 2048,
        });
        assert_eq!(liquid.base_url, "http://localhost:8000/v1");

        let openai = OpenAiCompatibleProvider::new(LlmConfig {
            provider: "openai".into(),
            api_key: Some("key".into()),
            model: "gpt-4o".into(),
            base_url: None,
            temperature: 0.0,
            max_tokens: 2048,
        });
        assert_eq!(openai.base_url, "https://api.openai.com/v1");
    }
}
