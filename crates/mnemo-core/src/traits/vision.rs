//! Vision provider trait for multi-modal image understanding.
//!
//! The [`VisionProvider`] trait defines the interface for extracting
//! descriptions and entities from images using vision-capable LLMs
//! (Claude Vision, GPT-4V, LLaVA, etc.).

use serde::{Deserialize, Serialize};

use crate::error::MnemoError;
use crate::models::entity::ExtractedEntity;

pub type VisionResult<T> = Result<T, MnemoError>;

/// Configuration for a vision provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisionConfig {
    /// Provider name: "anthropic", "openai", "ollama"
    pub provider: String,
    /// Model name (e.g., "claude-sonnet-4-20250514", "gpt-4o", "llava:13b")
    pub model: String,
    /// API key (required for cloud providers).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// Base URL override (for ollama or custom endpoints).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// Max tokens for vision response.
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    /// Temperature for vision tasks (lower = more consistent).
    #[serde(default = "default_temperature")]
    pub temperature: f32,
}

fn default_max_tokens() -> u32 {
    1024
}

fn default_temperature() -> f32 {
    0.2
}

impl Default for VisionConfig {
    fn default() -> Self {
        Self {
            provider: "anthropic".to_string(),
            model: "claude-sonnet-4-20250514".to_string(),
            api_key: None,
            base_url: None,
            max_tokens: default_max_tokens(),
            temperature: default_temperature(),
        }
    }
}

/// Result of vision analysis on an image.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisionAnalysis {
    /// Natural language description of the image content.
    pub description: String,

    /// Text extracted from the image (OCR-like, from vision model).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extracted_text: Option<String>,

    /// Entities identified in the image (people, products, logos, etc.).
    #[serde(default)]
    pub entities: Vec<ExtractedEntity>,

    /// Image classification tags.
    #[serde(default)]
    pub tags: Vec<String>,

    /// Detected scene type (e.g., "screenshot", "photo", "diagram", "chart").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene_type: Option<String>,

    /// Token usage for billing/monitoring.
    pub usage: VisionUsage,
}

/// Token usage information from a vision API call.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct VisionUsage {
    /// Prompt tokens (including image tokens).
    pub prompt_tokens: u32,
    /// Completion tokens.
    pub completion_tokens: u32,
    /// Total tokens.
    pub total_tokens: u32,
}

/// Supported image formats for vision processing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Jpeg,
    Png,
    Gif,
    Webp,
}

impl ImageFormat {
    /// Infer format from MIME type.
    pub fn from_mime(mime: &str) -> Option<Self> {
        match mime.to_lowercase().as_str() {
            "image/jpeg" | "image/jpg" => Some(Self::Jpeg),
            "image/png" => Some(Self::Png),
            "image/gif" => Some(Self::Gif),
            "image/webp" => Some(Self::Webp),
            _ => None,
        }
    }

    /// Get the media type string for API calls.
    pub fn media_type(&self) -> &'static str {
        match self {
            Self::Jpeg => "image/jpeg",
            Self::Png => "image/png",
            Self::Gif => "image/gif",
            Self::Webp => "image/webp",
        }
    }
}

/// Trait for vision-capable LLM providers.
///
/// Implementations exist for:
/// - Anthropic Claude (claude-sonnet-4-20250514, claude-3-haiku, etc.)
/// - OpenAI GPT-4V (gpt-4o, gpt-4-turbo)
/// - Ollama with vision models (llava, bakllava)
#[allow(async_fn_in_trait)]
pub trait VisionProvider: Send + Sync {
    /// Analyze an image and extract description, text, and entities.
    ///
    /// # Arguments
    /// * `image_data` - Raw image bytes
    /// * `format` - Image format (JPEG, PNG, etc.)
    /// * `prompt` - Optional custom prompt for analysis
    ///
    /// # Returns
    /// A `VisionAnalysis` containing description, extracted text, and entities.
    async fn analyze_image(
        &self,
        image_data: &[u8],
        format: ImageFormat,
        prompt: Option<&str>,
    ) -> VisionResult<VisionAnalysis>;

    /// Get the provider name for logging/debugging.
    fn provider_name(&self) -> &str;

    /// Get the model name being used.
    fn model_name(&self) -> &str;

    /// Check if the provider supports a given image format.
    fn supports_format(&self, format: ImageFormat) -> bool {
        // Most providers support all common formats
        matches!(format, ImageFormat::Jpeg | ImageFormat::Png | ImageFormat::Gif | ImageFormat::Webp)
    }

    /// Get the maximum supported image size in bytes.
    fn max_image_size(&self) -> u64 {
        20 * 1024 * 1024 // 20 MB default
    }
}

/// Default system prompt for image analysis.
pub const DEFAULT_VISION_PROMPT: &str = r#"Analyze this image and provide:

1. A detailed description of what you see in the image (2-4 sentences).

2. Any text visible in the image (signs, labels, UI elements, documents, etc.).

3. Notable entities you can identify:
   - People (names if known/visible, otherwise descriptions)
   - Products, brands, or logos
   - Locations or landmarks
   - Organizations
   - Technical elements (code, diagrams, charts)

4. The type of image (screenshot, photo, diagram, chart, document, etc.).

Format your response as JSON:
{
  "description": "...",
  "extracted_text": "..." or null,
  "entities": [
    {"name": "...", "entity_type": "person|product|organization|location|concept", "description": "..."}
  ],
  "tags": ["tag1", "tag2"],
  "scene_type": "screenshot|photo|diagram|chart|document|artwork|other"
}"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_image_format_from_mime() {
        assert_eq!(ImageFormat::from_mime("image/jpeg"), Some(ImageFormat::Jpeg));
        assert_eq!(ImageFormat::from_mime("image/png"), Some(ImageFormat::Png));
        assert_eq!(ImageFormat::from_mime("image/gif"), Some(ImageFormat::Gif));
        assert_eq!(ImageFormat::from_mime("image/webp"), Some(ImageFormat::Webp));
        assert_eq!(ImageFormat::from_mime("image/bmp"), None);
        assert_eq!(ImageFormat::from_mime("text/plain"), None);
    }

    #[test]
    fn test_image_format_media_type() {
        assert_eq!(ImageFormat::Jpeg.media_type(), "image/jpeg");
        assert_eq!(ImageFormat::Png.media_type(), "image/png");
    }

    #[test]
    fn test_vision_config_default() {
        let config = VisionConfig::default();
        assert_eq!(config.provider, "anthropic");
        assert_eq!(config.max_tokens, 1024);
        assert!(config.temperature < 1.0);
    }

    #[test]
    fn test_vision_analysis_serialization() {
        let analysis = VisionAnalysis {
            description: "A screenshot of a code editor".to_string(),
            extracted_text: Some("fn main() { }".to_string()),
            entities: vec![],
            tags: vec!["code".to_string(), "screenshot".to_string()],
            scene_type: Some("screenshot".to_string()),
            usage: VisionUsage {
                prompt_tokens: 100,
                completion_tokens: 50,
                total_tokens: 150,
            },
        };

        let json = serde_json::to_string(&analysis).unwrap();
        let de: VisionAnalysis = serde_json::from_str(&json).unwrap();
        assert_eq!(de.description, analysis.description);
        assert_eq!(de.scene_type, analysis.scene_type);
    }
}
