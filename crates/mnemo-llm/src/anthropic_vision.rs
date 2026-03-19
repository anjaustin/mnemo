//! Anthropic Claude Vision provider implementation.
//!
//! Uses the Anthropic Messages API with image content blocks for
//! vision-capable models like claude-sonnet-4-20250514 and claude-3-haiku.

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

/// Anthropic Claude Vision provider.
///
/// Supports vision-capable Claude models:
/// - claude-sonnet-4-20250514 (recommended)
/// - claude-3-5-sonnet-latest
/// - claude-3-haiku-20240307 (faster, cheaper)
pub struct AnthropicVisionProvider {
    client: Client,
    config: VisionConfig,
    base_url: String,
}

// ─── Request/Response Types ────────────────────────────────────────

#[derive(Serialize)]
struct VisionRequest {
    model: String,
    max_tokens: u32,
    messages: Vec<VisionMessage>,
    #[serde(skip_serializing_if = "is_zero")]
    temperature: f32,
}

fn is_zero(v: &f32) -> bool {
    *v == 0.0
}

#[derive(Serialize)]
struct VisionMessage {
    role: String,
    content: Vec<ContentBlock>,
}

#[derive(Serialize)]
#[serde(tag = "type")]
enum ContentBlock {
    #[serde(rename = "image")]
    Image { source: ImageSource },
    #[serde(rename = "text")]
    Text { text: String },
}

#[derive(Serialize)]
struct ImageSource {
    #[serde(rename = "type")]
    source_type: String,
    media_type: String,
    data: String,
}

#[derive(Deserialize)]
struct VisionResponse {
    content: Vec<ResponseBlock>,
    #[serde(default)]
    usage: Option<AnthropicUsage>,
}

#[derive(Deserialize)]
struct ResponseBlock {
    #[serde(rename = "type")]
    block_type: String,
    #[serde(default)]
    text: String,
}

#[derive(Deserialize)]
struct AnthropicUsage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
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

impl AnthropicVisionProvider {
    /// Create a new Anthropic vision provider.
    pub fn new(config: VisionConfig) -> Self {
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

    /// Create from environment variables.
    ///
    /// Reads:
    /// - `ANTHROPIC_API_KEY` or `MNEMO_VISION_API_KEY`
    /// - `MNEMO_VISION_MODEL` (default: "claude-sonnet-4-20250514")
    pub fn from_env() -> Result<Self, MnemoError> {
        let api_key = std::env::var("MNEMO_VISION_API_KEY")
            .or_else(|_| std::env::var("ANTHROPIC_API_KEY"))
            .map_err(|_| {
                MnemoError::Config(
                    "MNEMO_VISION_API_KEY or ANTHROPIC_API_KEY is required".to_string(),
                )
            })?;

        let model = std::env::var("MNEMO_VISION_MODEL")
            .unwrap_or_else(|_| "claude-sonnet-4-20250514".to_string());

        let config = VisionConfig {
            provider: "anthropic".to_string(),
            model,
            api_key: Some(api_key),
            ..Default::default()
        };

        Ok(Self::new(config))
    }
}

impl VisionProvider for AnthropicVisionProvider {
    #[instrument(skip(self, image_data), fields(format = ?format, size = image_data.len()))]
    async fn analyze_image(
        &self,
        image_data: &[u8],
        format: ImageFormat,
        prompt: Option<&str>,
    ) -> VisionResult<VisionAnalysis> {
        let api_key = self.config.api_key.as_deref().ok_or_else(|| {
            MnemoError::LlmProvider {
                provider: "anthropic_vision".into(),
                message: "API key is required".into(),
            }
        })?;

        // Encode image as base64
        let image_b64 = BASE64.encode(image_data);

        // Build the request
        let request = VisionRequest {
            model: self.config.model.clone(),
            max_tokens: self.config.max_tokens,
            temperature: self.config.temperature,
            messages: vec![VisionMessage {
                role: "user".to_string(),
                content: vec![
                    ContentBlock::Image {
                        source: ImageSource {
                            source_type: "base64".to_string(),
                            media_type: format.media_type().to_string(),
                            data: image_b64,
                        },
                    },
                    ContentBlock::Text {
                        text: prompt.unwrap_or(DEFAULT_VISION_PROMPT).to_string(),
                    },
                ],
            }],
        };

        debug!(
            model = %self.config.model,
            image_size = image_data.len(),
            "Sending vision request to Anthropic"
        );

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
                provider: "anthropic_vision".into(),
                message: format!("Request failed: {}", e),
            })?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(MnemoError::LlmProvider {
                provider: "anthropic_vision".into(),
                message: format!("API error {}: {}", status, error_text),
            });
        }

        let api_response: VisionResponse = response.json().await.map_err(|e| {
            MnemoError::LlmProvider {
                provider: "anthropic_vision".into(),
                message: format!("Failed to parse response: {}", e),
            }
        })?;

        // Extract text from response
        let response_text = api_response
            .content
            .iter()
            .filter(|b| b.block_type == "text")
            .map(|b| b.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        // Parse the JSON response
        let analysis = parse_vision_response(&response_text)?;

        // Build usage stats
        let usage = api_response
            .usage
            .map(|u| VisionUsage {
                prompt_tokens: u.input_tokens,
                completion_tokens: u.output_tokens,
                total_tokens: u.input_tokens + u.output_tokens,
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
        "anthropic_vision"
    }

    fn model_name(&self) -> &str {
        &self.config.model
    }
}

/// Parse the vision model's JSON response.
fn parse_vision_response(text: &str) -> VisionResult<ParsedVisionOutput> {
    // Try to extract JSON from the response
    // The model might return markdown code blocks or raw JSON

    // First, try to find JSON in code blocks
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
        // Find the matching closing brace
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

    serde_json::from_str(json_text.trim()).map_err(|e| {
        // If JSON parsing fails, create a basic analysis from the raw text
        debug!(error = %e, text = %text, "Failed to parse vision JSON, using fallback");

        // Return a fallback instead of error - use the raw text as description
        MnemoError::LlmProvider {
            provider: "anthropic_vision".into(),
            message: format!("Failed to parse vision response: {}. Raw: {}", e, text),
        }
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
    fn test_parse_vision_response_code_block() {
        let text = r#"Here's the analysis:

```json
{"description": "A screenshot", "entities": [], "tags": ["screenshot"]}
```

That's what I see."#;
        let result = parse_vision_response(text).unwrap();
        assert_eq!(result.description, "A screenshot");
    }

    #[test]
    fn test_parse_entity_type() {
        assert_eq!(parse_entity_type("person"), EntityType::Person);
        assert_eq!(parse_entity_type("ORGANIZATION"), EntityType::Organization);
        assert_eq!(parse_entity_type("product"), EntityType::Product);
        assert_eq!(parse_entity_type("unknown"), EntityType::Concept);
    }

    #[test]
    fn test_vision_config_from_env() {
        // This test just verifies the error handling when env vars are missing
        let result = AnthropicVisionProvider::from_env();
        // Should fail without env vars set
        assert!(result.is_err());
    }
}
