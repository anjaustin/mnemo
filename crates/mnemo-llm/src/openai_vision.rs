//! OpenAI GPT-4V vision provider implementation.
//!
//! Uses the OpenAI Chat Completions API with image URLs or base64
//! for vision-capable models like gpt-4o and gpt-4-turbo.

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::{debug, instrument};

use mnemo_core::error::MnemoError;
use mnemo_core::models::classification::Classification;
use mnemo_core::models::entity::{EntityType, ExtractedEntity};
use mnemo_core::traits::vision::{
    ImageFormat, VisionAnalysis, VisionConfig, VisionProvider, VisionResult, VisionUsage,
    DEFAULT_VISION_PROMPT,
};

/// OpenAI GPT-4V vision provider.
///
/// Supports vision-capable OpenAI models:
/// - gpt-4o (recommended - fast and capable)
/// - gpt-4-turbo
/// - gpt-4-vision-preview (legacy)
///
/// Also works with OpenAI-compatible APIs that support vision
/// (e.g., Azure OpenAI, vLLM with vision models).
pub struct OpenAIVisionProvider {
    client: Client,
    config: VisionConfig,
    base_url: String,
}

// ─── Request/Response Types ────────────────────────────────────────

#[derive(Serialize)]
struct VisionRequest {
    model: String,
    max_tokens: u32,
    messages: Vec<OpenAIMessage>,
    #[serde(skip_serializing_if = "is_zero")]
    temperature: f32,
}

fn is_zero(v: &f32) -> bool {
    *v == 0.0
}

#[derive(Serialize)]
struct OpenAIMessage {
    role: String,
    content: Vec<ContentPart>,
}

#[derive(Serialize)]
#[serde(tag = "type")]
enum ContentPart {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image_url")]
    ImageUrl { image_url: ImageUrl },
}

#[derive(Serialize)]
struct ImageUrl {
    url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
}

#[derive(Deserialize)]
struct VisionResponse {
    choices: Vec<Choice>,
    #[serde(default)]
    usage: Option<OpenAIUsage>,
}

#[derive(Deserialize)]
struct Choice {
    message: ResponseMessage,
}

#[derive(Deserialize)]
struct ResponseMessage {
    content: String,
}

#[derive(Deserialize)]
struct OpenAIUsage {
    #[serde(default)]
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: u32,
    #[serde(default)]
    total_tokens: u32,
}

// ─── Parsed Vision Output ──────────────────────────────────────────

#[derive(Deserialize)]
struct ParsedVisionOutput {
    #[serde(default)]
    description: String,
    #[serde(default)]
    extracted_text: Option<String>,
    #[serde(default)]
    entities: Vec<RawEntity>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    scene_type: Option<String>,
}

#[derive(Deserialize)]
struct RawEntity {
    name: String,
    #[serde(rename = "entity_type", alias = "type")]
    entity_type: String,
    #[serde(default)]
    description: Option<String>,
}

// ─── Implementation ────────────────────────────────────────────────

impl OpenAIVisionProvider {
    /// Create a new OpenAI vision provider.
    pub fn new(config: VisionConfig) -> Self {
        let base_url = config
            .base_url
            .clone()
            .unwrap_or_else(|| "https://api.openai.com".to_string());
        let client = Client::new();
        Self {
            client,
            config,
            base_url,
        }
    }

    /// Create from environment variables.
    ///
    /// Reads:
    /// - `OPENAI_API_KEY` or `MNEMO_VISION_API_KEY`
    /// - `MNEMO_VISION_MODEL` (default: "gpt-4o")
    pub fn from_env() -> Result<Self, MnemoError> {
        let api_key = std::env::var("MNEMO_VISION_API_KEY")
            .or_else(|_| std::env::var("OPENAI_API_KEY"))
            .map_err(|_| {
                MnemoError::Config("MNEMO_VISION_API_KEY or OPENAI_API_KEY is required".to_string())
            })?;

        let model = std::env::var("MNEMO_VISION_MODEL").unwrap_or_else(|_| "gpt-4o".to_string());

        let config = VisionConfig {
            provider: "openai".to_string(),
            model,
            api_key: Some(api_key),
            ..Default::default()
        };

        Ok(Self::new(config))
    }
}

impl VisionProvider for OpenAIVisionProvider {
    #[instrument(skip(self, image_data), fields(format = ?format, size = image_data.len()))]
    async fn analyze_image(
        &self,
        image_data: &[u8],
        format: ImageFormat,
        prompt: Option<&str>,
    ) -> VisionResult<VisionAnalysis> {
        let api_key = self
            .config
            .api_key
            .as_deref()
            .ok_or_else(|| MnemoError::LlmProvider {
                provider: "openai_vision".into(),
                message: "API key is required".into(),
            })?;

        // Encode image as base64 data URL
        let image_b64 = BASE64.encode(image_data);
        let data_url = format!("data:{};base64,{}", format.media_type(), image_b64);

        // Build the request
        let request = VisionRequest {
            model: self.config.model.clone(),
            max_tokens: self.config.max_tokens,
            temperature: self.config.temperature,
            messages: vec![OpenAIMessage {
                role: "user".to_string(),
                content: vec![
                    ContentPart::ImageUrl {
                        image_url: ImageUrl {
                            url: data_url,
                            detail: Some("auto".to_string()),
                        },
                    },
                    ContentPart::Text {
                        text: prompt.unwrap_or(DEFAULT_VISION_PROMPT).to_string(),
                    },
                ],
            }],
        };

        debug!(
            model = %self.config.model,
            image_size = image_data.len(),
            "Sending vision request to OpenAI"
        );

        let response = self
            .client
            .post(format!("{}/v1/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| MnemoError::LlmProvider {
                provider: "openai_vision".into(),
                message: format!("Request failed: {}", e),
            })?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(MnemoError::LlmProvider {
                provider: "openai_vision".into(),
                message: format!("API error {}: {}", status, error_text),
            });
        }

        let api_response: VisionResponse =
            response.json().await.map_err(|e| MnemoError::LlmProvider {
                provider: "openai_vision".into(),
                message: format!("Failed to parse response: {}", e),
            })?;

        // Extract text from response
        let response_text = api_response
            .choices
            .first()
            .map(|c| c.message.content.as_str())
            .unwrap_or("");

        // Parse the JSON response
        let analysis = parse_vision_response(response_text)?;

        // Build usage stats
        let usage = api_response
            .usage
            .map(|u| VisionUsage {
                prompt_tokens: u.prompt_tokens,
                completion_tokens: u.completion_tokens,
                total_tokens: u.total_tokens,
            })
            .unwrap_or_default();

        debug!(
            description_len = analysis.description.len(),
            entities = analysis.entities.len(),
            usage = ?usage,
            "Vision analysis complete"
        );

        Ok(VisionAnalysis {
            description: analysis.description,
            extracted_text: analysis.extracted_text,
            entities: analysis
                .entities
                .into_iter()
                .map(|e| ExtractedEntity {
                    name: e.name,
                    entity_type: parse_entity_type(&e.entity_type),
                    summary: e.description,
                    classification: Classification::default(),
                })
                .collect(),
            tags: analysis.tags,
            scene_type: analysis.scene_type,
            usage,
        })
    }

    fn provider_name(&self) -> &str {
        "openai_vision"
    }

    fn model_name(&self) -> &str {
        &self.config.model
    }
}

/// Parse the vision model's JSON response.
fn parse_vision_response(text: &str) -> VisionResult<ParsedVisionOutput> {
    // Try to extract JSON from the response
    let json_text = if let Some(start) = text.find("```json") {
        let start = start + 7;
        if let Some(end) = text[start..].find("```") {
            &text[start..start + end]
        } else {
            text
        }
    } else if let Some(start) = text.find("```") {
        let start = start + 3;
        if let Some(end) = text[start..].find("```") {
            &text[start..start + end]
        } else {
            text
        }
    } else if let Some(start) = text.find('{') {
        let mut depth = 0;
        let mut end = start;
        for (i, c) in text[start..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = start + i + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        &text[start..end]
    } else {
        text
    };

    serde_json::from_str(json_text.trim()).map_err(|e| MnemoError::LlmProvider {
        provider: "openai_vision".into(),
        message: format!("Failed to parse vision response: {}. Raw: {}", e, text),
    })
}

/// Parse entity type string to EntityType enum.
fn parse_entity_type(s: &str) -> EntityType {
    match s.to_lowercase().as_str() {
        "person" | "people" => EntityType::Person,
        "organization" | "org" | "company" => EntityType::Organization,
        "location" | "place" => EntityType::Location,
        "product" | "brand" => EntityType::Product,
        "event" => EntityType::Event,
        _ => EntityType::Concept,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_vision_response_json() {
        let json = r#"{"description": "A test image", "entities": [], "tags": ["test"], "scene_type": "photo"}"#;
        let result = parse_vision_response(json).unwrap();
        assert_eq!(result.description, "A test image");
        assert_eq!(result.tags, vec!["test"]);
    }

    #[test]
    fn test_parse_entity_type() {
        assert_eq!(parse_entity_type("person"), EntityType::Person);
        assert_eq!(parse_entity_type("ORGANIZATION"), EntityType::Organization);
        assert_eq!(parse_entity_type("unknown"), EntityType::Concept);
    }
}
