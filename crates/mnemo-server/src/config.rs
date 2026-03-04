use mnemo_core::error::MnemoError;
use serde::Deserialize;

#[derive(Debug, Deserialize, Clone, Default)]
pub struct MnemoConfig {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub auth: AuthSection,
    #[serde(default)]
    pub redis: RedisConfig,
    #[serde(default)]
    pub qdrant: QdrantConfig,
    #[serde(default)]
    pub llm: LlmSection,
    #[serde(default)]
    pub embedding: EmbeddingSection,
    #[serde(default)]
    pub extraction: ExtractionSection,
    #[serde(default)]
    pub retrieval: RetrievalSection,
    #[serde(default)]
    pub observability: ObservabilitySection,
    #[serde(default)]
    pub webhooks: WebhookSection,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ServerConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default)]
    pub workers: usize,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
            workers: 0,
        }
    }
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct AuthSection {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub api_keys: Vec<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct RedisConfig {
    #[serde(default = "default_redis_url")]
    pub url: String,
    #[serde(default = "default_prefix")]
    pub prefix: String,
}

impl Default for RedisConfig {
    fn default() -> Self {
        Self {
            url: default_redis_url(),
            prefix: default_prefix(),
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct QdrantConfig {
    #[serde(default = "default_qdrant_url")]
    pub url: String,
    #[serde(default = "default_qdrant_prefix")]
    pub collection_prefix: String,
}

impl Default for QdrantConfig {
    fn default() -> Self {
        Self {
            url: default_qdrant_url(),
            collection_prefix: default_qdrant_prefix(),
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct LlmSection {
    #[serde(default = "default_llm_provider")]
    pub provider: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default = "default_llm_model")]
    pub model: String,
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub temperature: f32,
    #[serde(default = "default_llm_max_tokens")]
    pub max_tokens: u32,
}

impl Default for LlmSection {
    fn default() -> Self {
        Self {
            provider: default_llm_provider(),
            api_key: String::new(),
            model: default_llm_model(),
            base_url: String::new(),
            temperature: 0.0,
            max_tokens: default_llm_max_tokens(),
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct EmbeddingSection {
    #[serde(default = "default_embed_provider")]
    pub provider: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default = "default_embed_model")]
    pub model: String,
    #[serde(default)]
    pub base_url: String,
    #[serde(default = "default_dimensions")]
    pub dimensions: u32,
}

impl Default for EmbeddingSection {
    fn default() -> Self {
        Self {
            provider: default_embed_provider(),
            api_key: String::new(),
            model: default_embed_model(),
            base_url: String::new(),
            dimensions: default_dimensions(),
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct ExtractionSection {
    #[serde(default = "default_batch_size")]
    pub batch_size: u32,
    #[serde(default = "default_concurrency")]
    pub concurrency: usize,
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    #[serde(default = "default_poll_interval")]
    pub poll_interval_ms: u64,
}

impl Default for ExtractionSection {
    fn default() -> Self {
        Self {
            batch_size: default_batch_size(),
            concurrency: default_concurrency(),
            max_retries: default_max_retries(),
            poll_interval_ms: default_poll_interval(),
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct RetrievalSection {
    #[serde(default)]
    pub metadata_prefilter_enabled: bool,
    #[serde(default = "default_metadata_scan_limit")]
    pub metadata_scan_limit: u32,
    #[serde(default)]
    pub metadata_relax_if_empty: bool,
}

impl Default for RetrievalSection {
    fn default() -> Self {
        Self {
            metadata_prefilter_enabled: true,
            metadata_scan_limit: default_metadata_scan_limit(),
            metadata_relax_if_empty: false,
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct ObservabilitySection {
    #[serde(default = "default_log_level")]
    pub log_level: String,
    #[serde(default = "default_log_format")]
    pub log_format: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct WebhookSection {
    #[serde(default = "default_webhook_enabled")]
    pub enabled: bool,
    #[serde(default = "default_webhook_max_attempts")]
    pub max_attempts: u32,
    #[serde(default = "default_webhook_backoff_ms")]
    pub base_backoff_ms: u64,
    #[serde(default = "default_webhook_timeout_ms")]
    pub request_timeout_ms: u64,
    #[serde(default = "default_webhook_max_events")]
    pub max_events_per_webhook: usize,
    #[serde(default = "default_webhook_rate_limit_per_minute")]
    pub rate_limit_per_minute: u32,
    #[serde(default = "default_webhook_circuit_threshold")]
    pub circuit_breaker_threshold: u32,
    #[serde(default = "default_webhook_circuit_cooldown_ms")]
    pub circuit_breaker_cooldown_ms: u64,
    #[serde(default = "default_webhook_persistence_enabled")]
    pub persistence_enabled: bool,
    #[serde(default = "default_webhook_prefix")]
    pub persistence_prefix: String,
}

impl Default for WebhookSection {
    fn default() -> Self {
        Self {
            enabled: default_webhook_enabled(),
            max_attempts: default_webhook_max_attempts(),
            base_backoff_ms: default_webhook_backoff_ms(),
            request_timeout_ms: default_webhook_timeout_ms(),
            max_events_per_webhook: default_webhook_max_events(),
            rate_limit_per_minute: default_webhook_rate_limit_per_minute(),
            circuit_breaker_threshold: default_webhook_circuit_threshold(),
            circuit_breaker_cooldown_ms: default_webhook_circuit_cooldown_ms(),
            persistence_enabled: default_webhook_persistence_enabled(),
            persistence_prefix: default_webhook_prefix(),
        }
    }
}

impl Default for ObservabilitySection {
    fn default() -> Self {
        Self {
            log_level: default_log_level(),
            log_format: default_log_format(),
        }
    }
}

// Default value functions
fn default_host() -> String {
    "0.0.0.0".into()
}
fn default_port() -> u16 {
    8080
}
fn default_redis_url() -> String {
    "redis://localhost:6379".into()
}
fn default_prefix() -> String {
    "mnemo:".into()
}
fn default_qdrant_url() -> String {
    "http://localhost:6334".into()
}
fn default_qdrant_prefix() -> String {
    "mnemo_".into()
}
fn default_llm_provider() -> String {
    "openai".into()
}
fn default_llm_model() -> String {
    "gpt-4o-mini".into()
}
fn default_llm_max_tokens() -> u32 {
    2048
}
fn default_embed_provider() -> String {
    "openai".into()
}
fn default_embed_model() -> String {
    "text-embedding-3-small".into()
}
fn default_dimensions() -> u32 {
    1536
}
fn default_batch_size() -> u32 {
    10
}
fn default_concurrency() -> usize {
    4
}
fn default_max_retries() -> u32 {
    3
}
fn default_poll_interval() -> u64 {
    500
}
fn default_metadata_scan_limit() -> u32 {
    400
}
fn default_log_level() -> String {
    "info".into()
}
fn default_log_format() -> String {
    "pretty".into()
}
fn default_webhook_enabled() -> bool {
    true
}
fn default_webhook_max_attempts() -> u32 {
    3
}
fn default_webhook_backoff_ms() -> u64 {
    200
}
fn default_webhook_timeout_ms() -> u64 {
    3000
}
fn default_webhook_max_events() -> usize {
    1000
}
fn default_webhook_rate_limit_per_minute() -> u32 {
    120
}
fn default_webhook_circuit_threshold() -> u32 {
    5
}
fn default_webhook_circuit_cooldown_ms() -> u64 {
    60_000
}
fn default_webhook_persistence_enabled() -> bool {
    true
}
fn default_webhook_prefix() -> String {
    "webhooks".into()
}

impl MnemoConfig {
    /// Load config from TOML file, then apply environment variable overrides.
    pub fn load(path: Option<&str>) -> Result<Self, MnemoError> {
        let mut config: MnemoConfig = if let Some(p) = path {
            let content = std::fs::read_to_string(p)
                .map_err(|e| MnemoError::Config(format!("Failed to read {}: {}", p, e)))?;
            toml::from_str(&content)
                .map_err(|e| MnemoError::Config(format!("Failed to parse TOML: {}", e)))?
        } else {
            MnemoConfig::default()
        };

        // Environment variable overrides
        if let Ok(v) = std::env::var("MNEMO_SERVER_HOST") {
            config.server.host = v;
        }
        if let Ok(v) = std::env::var("MNEMO_SERVER_PORT") {
            config.server.port = v.parse().unwrap_or(config.server.port);
        }
        if let Ok(v) = std::env::var("MNEMO_REDIS_URL") {
            config.redis.url = v;
        }
        if let Ok(v) = std::env::var("MNEMO_QDRANT_URL") {
            config.qdrant.url = v;
        }
        if let Ok(v) = std::env::var("MNEMO_LLM_PROVIDER") {
            config.llm.provider = v;
        }
        if let Ok(v) = std::env::var("MNEMO_LLM_API_KEY") {
            config.llm.api_key = v;
        }
        if let Ok(v) = std::env::var("MNEMO_LLM_MODEL") {
            config.llm.model = v;
        }
        if let Ok(v) = std::env::var("MNEMO_LLM_BASE_URL") {
            config.llm.base_url = v;
        }
        if let Ok(v) = std::env::var("MNEMO_EMBEDDING_API_KEY") {
            config.embedding.api_key = v;
        }
        if let Ok(v) = std::env::var("MNEMO_EMBEDDING_MODEL") {
            config.embedding.model = v;
        }
        if let Ok(v) = std::env::var("MNEMO_EMBEDDING_BASE_URL") {
            config.embedding.base_url = v;
        }
        if let Ok(v) = std::env::var("MNEMO_EMBEDDING_DIMENSIONS") {
            if let Ok(d) = v.parse() {
                config.embedding.dimensions = d;
            }
        }

        // Auth overrides
        if let Ok(v) = std::env::var("MNEMO_AUTH_ENABLED") {
            config.auth.enabled = v == "true" || v == "1";
        }
        if let Ok(v) = std::env::var("MNEMO_AUTH_API_KEYS") {
            let keys: Vec<String> = v
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            config.auth.api_keys.extend(keys);
        }

        // Retrieval / metadata prefilter overrides
        if let Ok(v) = std::env::var("MNEMO_METADATA_PREFILTER_ENABLED") {
            config.retrieval.metadata_prefilter_enabled = v == "true" || v == "1";
        }
        if let Ok(v) = std::env::var("MNEMO_METADATA_SCAN_LIMIT") {
            if let Ok(n) = v.parse() {
                config.retrieval.metadata_scan_limit = n;
            }
        }
        if let Ok(v) = std::env::var("MNEMO_METADATA_RELAX_IF_EMPTY") {
            config.retrieval.metadata_relax_if_empty = v == "true" || v == "1";
        }

        if let Ok(v) = std::env::var("MNEMO_WEBHOOKS_ENABLED") {
            config.webhooks.enabled = v == "true" || v == "1";
        }
        if let Ok(v) = std::env::var("MNEMO_WEBHOOKS_MAX_ATTEMPTS") {
            if let Ok(n) = v.parse() {
                config.webhooks.max_attempts = n;
            }
        }
        if let Ok(v) = std::env::var("MNEMO_WEBHOOKS_BASE_BACKOFF_MS") {
            if let Ok(n) = v.parse() {
                config.webhooks.base_backoff_ms = n;
            }
        }
        if let Ok(v) = std::env::var("MNEMO_WEBHOOKS_TIMEOUT_MS") {
            if let Ok(n) = v.parse() {
                config.webhooks.request_timeout_ms = n;
            }
        }
        if let Ok(v) = std::env::var("MNEMO_WEBHOOKS_MAX_EVENTS_PER_WEBHOOK") {
            if let Ok(n) = v.parse() {
                config.webhooks.max_events_per_webhook = n;
            }
        }
        if let Ok(v) = std::env::var("MNEMO_WEBHOOKS_RATE_LIMIT_PER_MINUTE") {
            if let Ok(n) = v.parse() {
                config.webhooks.rate_limit_per_minute = n;
            }
        }
        if let Ok(v) = std::env::var("MNEMO_WEBHOOKS_CIRCUIT_BREAKER_THRESHOLD") {
            if let Ok(n) = v.parse() {
                config.webhooks.circuit_breaker_threshold = n;
            }
        }
        if let Ok(v) = std::env::var("MNEMO_WEBHOOKS_CIRCUIT_BREAKER_COOLDOWN_MS") {
            if let Ok(n) = v.parse() {
                config.webhooks.circuit_breaker_cooldown_ms = n;
            }
        }
        if let Ok(v) = std::env::var("MNEMO_WEBHOOKS_PERSISTENCE_ENABLED") {
            config.webhooks.persistence_enabled = v == "true" || v == "1";
        }
        if let Ok(v) = std::env::var("MNEMO_WEBHOOKS_PERSISTENCE_PREFIX") {
            config.webhooks.persistence_prefix = v;
        }

        Ok(config)
    }

    pub fn llm_config(&self) -> mnemo_core::traits::llm::LlmConfig {
        mnemo_core::traits::llm::LlmConfig {
            provider: self.llm.provider.clone(),
            api_key: if self.llm.api_key.is_empty() {
                None
            } else {
                Some(self.llm.api_key.clone())
            },
            model: self.llm.model.clone(),
            base_url: if self.llm.base_url.is_empty() {
                None
            } else {
                Some(self.llm.base_url.clone())
            },
            temperature: self.llm.temperature,
            max_tokens: self.llm.max_tokens,
        }
    }

    pub fn embedding_config(&self) -> mnemo_core::traits::llm::EmbeddingConfig {
        mnemo_core::traits::llm::EmbeddingConfig {
            provider: self.embedding.provider.clone(),
            api_key: if self.embedding.api_key.is_empty() {
                None
            } else {
                Some(self.embedding.api_key.clone())
            },
            model: self.embedding.model.clone(),
            base_url: if self.embedding.base_url.is_empty() {
                None
            } else {
                Some(self.embedding.base_url.clone())
            },
            dimensions: self.embedding.dimensions,
        }
    }
}
