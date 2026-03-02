use reqwest::Client;
use serde::{Deserialize, Serialize};

use mnemo_core::error::MnemoError;
use mnemo_core::models::edge::ExtractedRelationship;
use mnemo_core::models::entity::{EntityType, ExtractedEntity};
use mnemo_core::traits::llm::{
    EmbeddingProvider, ExtractionResult, LlmConfig, LlmProvider, LlmResult,
    EmbeddingConfig,
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
        Self { client, config, base_url }
    }

    async fn chat_completion(&self, system: &str, user_msg: &str) -> LlmResult<String> {
        let request = ChatRequest {
            model: self.config.model.clone(),
            messages: vec![
                ChatMessage { role: "system".into(), content: system.into() },
                ChatMessage { role: "user".into(), content: user_msg.into() },
            ],
            temperature: self.config.temperature,
            max_tokens: self.config.max_tokens,
        };

        let mut req_builder = self.client
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

        let chat_response: ChatResponse = response
            .json()
            .await
            .map_err(|e| MnemoError::LlmProvider {
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
            MnemoError::ExtractionFailed(format!("Failed to parse extraction JSON: {}. Raw: {}", e, &raw[..raw.len().min(200)]))
        })
    }
}

impl LlmProvider for OpenAiCompatibleProvider {
    async fn extract_entities_and_relationships(
        &self,
        content: &str,
        existing_entities: &[ExtractedEntity],
    ) -> LlmResult<ExtractionResult> {
        let mut user_msg = format!("Extract entities and relationships from this text:\n\n{}", content);

        if !existing_entities.is_empty() {
            let names: Vec<&str> = existing_entities.iter().map(|e| e.name.as_str()).collect();
            user_msg.push_str(&format!(
                "\n\nExisting entities in this user's graph (reuse these names if they appear): {}",
                names.join(", ")
            ));
        }

        let raw = self.chat_completion(EXTRACTION_SYSTEM_PROMPT, &user_msg).await?;
        let parsed = Self::parse_extraction(&raw)?;

        let entities = parsed.entities.into_iter().map(|e| {
            ExtractedEntity {
                name: e.name,
                entity_type: EntityType::from_str_flexible(&e.entity_type),
                summary: e.summary,
            }
        }).collect();

        let relationships = parsed.relationships.into_iter().map(|r| {
            ExtractedRelationship {
                source_name: r.source,
                target_name: r.target,
                label: r.label,
                fact: r.fact,
                confidence: r.confidence,
                valid_at: None,
            }
        }).collect();

        Ok(ExtractionResult { entities, relationships })
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

        let existing = existing_facts.iter()
            .enumerate()
            .map(|(i, f)| format!("{}. {}", i + 1, f))
            .collect::<Vec<_>>()
            .join("\n");

        let system = "You detect contradictions between facts. Respond with a JSON array of strings \
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
        Self { client, config, base_url }
    }
}

impl EmbeddingProvider for OpenAiCompatibleEmbedder {
    async fn embed(&self, text: &str) -> LlmResult<Vec<f32>> {
        let batch = self.embed_batch(&[text.to_string()]).await?;
        batch.into_iter().next().ok_or_else(|| {
            MnemoError::EmbeddingProvider {
                provider: self.config.provider.clone(),
                message: "Empty embedding response".into(),
            }
        })
    }

    async fn embed_batch(&self, texts: &[String]) -> LlmResult<Vec<Vec<f32>>> {
        let request = EmbedRequest {
            model: self.config.model.clone(),
            input: texts.to_vec(),
        };

        let mut req_builder = self.client
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

        let embed_response: EmbedResponse = response
            .json()
            .await
            .map_err(|e| MnemoError::EmbeddingProvider {
                provider: self.config.provider.clone(),
                message: format!("Failed to parse response: {}", e),
            })?;

        Ok(embed_response.data.into_iter().map(|d| d.embedding).collect())
    }

    fn dimensions(&self) -> u32 {
        self.config.dimensions
    }

    fn provider_name(&self) -> &str {
        &self.config.provider
    }
}
