use serde::{Deserialize, Serialize};

use crate::error::MnemoError;
use crate::models::edge::ExtractedRelationship;
use crate::models::entity::ExtractedEntity;

pub type LlmResult<T> = Result<T, MnemoError>;

/// Configuration for an LLM provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConfig {
    /// Provider name: "anthropic", "openai", "ollama", "liquid", "none"
    pub provider: String,
    /// API key (ignored for local providers).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// Model name (e.g., "claude-sonnet-4-20250514", "gpt-4o-mini", "liquid/lfm-7b").
    pub model: String,
    /// Base URL override (for ollama, vLLM, or custom endpoints).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// Temperature for extraction tasks (lower = more consistent).
    #[serde(default = "default_temperature")]
    pub temperature: f32,
    /// Max tokens for extraction responses.
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
}

fn default_temperature() -> f32 {
    0.0
}

fn default_max_tokens() -> u32 {
    2048
}

/// Configuration for an embedding provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingConfig {
    /// Provider: "openai", "voyage", "cohere", "ollama", "fastembed"
    pub provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// The dimensionality of the embeddings produced by this model.
    pub dimensions: u32,
}

/// The result of entity and relationship extraction from an episode.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractionResult {
    pub entities: Vec<ExtractedEntity>,
    pub relationships: Vec<ExtractedRelationship>,
}

/// Trait for LLM-powered operations.
///
/// Implementations exist for:
/// - Anthropic Claude (recommended for online/cloud)
/// - OpenAI-compatible APIs
/// - Liquid AI (recommended for offline/local)
/// - Ollama (local models)
/// - NoOp (rule-based extraction, no LLM dependency)
#[allow(async_fn_in_trait)]
pub trait LlmProvider: Send + Sync {
    /// Extract entities and relationships from episode content.
    ///
    /// This is the core LLM operation in Mnemo. The provider should:
    /// 1. Identify entities (people, products, orgs, etc.)
    /// 2. Identify relationships between entities
    /// 3. Extract temporal cues (dates, relative time references)
    /// 4. Return structured extraction results
    async fn extract_entities_and_relationships(
        &self,
        content: &str,
        existing_entities: &[ExtractedEntity],
    ) -> LlmResult<ExtractionResult>;

    /// Generate a summary of a set of facts or conversation.
    async fn summarize(&self, content: &str, max_tokens: u32) -> LlmResult<String>;

    /// Detect if a new fact contradicts existing facts.
    /// Returns descriptions of contradictions found.
    async fn detect_contradictions(
        &self,
        new_fact: &str,
        existing_facts: &[String],
    ) -> LlmResult<Vec<String>>;

    /// Get the provider name for logging/metrics.
    fn provider_name(&self) -> &str;

    /// Get the model name for logging/metrics.
    fn model_name(&self) -> &str;
}

/// Trait for embedding generation.
#[allow(async_fn_in_trait)]
pub trait EmbeddingProvider: Send + Sync {
    /// Generate an embedding for a single text.
    async fn embed(&self, text: &str) -> LlmResult<Vec<f32>>;

    /// Generate embeddings for multiple texts (batched for efficiency).
    async fn embed_batch(&self, texts: &[String]) -> LlmResult<Vec<Vec<f32>>>;

    /// The dimensionality of embeddings produced by this provider.
    fn dimensions(&self) -> u32;

    /// Get the provider name for logging/metrics.
    fn provider_name(&self) -> &str;
}
