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
    /// If true, the server rejects non-TLS connections and non-https webhook targets.
    /// Set via `MNEMO_REQUIRE_TLS=true`. Default: false.
    #[serde(default)]
    pub require_tls: bool,
    /// HMAC secret for signing audit export responses. If set, audit exports include
    /// a `x-mnemo-audit-signature` header. Required for SOC 2 compliance posture.
    /// Set via `MNEMO_AUDIT_SIGNING_SECRET`.
    #[serde(default)]
    pub audit_signing_secret: Option<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
            workers: 0,
            require_tls: false,
            audit_signing_secret: None,
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
    /// How many episodes must accumulate in a session before the ingest worker
    /// generates (or re-generates) a progressive summary. Set to 0 to disable.
    #[serde(default = "default_session_summary_threshold")]
    pub session_summary_threshold: u32,
}

impl Default for ExtractionSection {
    fn default() -> Self {
        Self {
            batch_size: default_batch_size(),
            concurrency: default_concurrency(),
            max_retries: default_max_retries(),
            poll_interval_ms: default_poll_interval(),
            session_summary_threshold: default_session_summary_threshold(),
        }
    }
}

#[derive(Debug, Deserialize, Clone, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum RerankerConfig {
    /// Reciprocal Rank Fusion — boosts items that appear in multiple ranked
    /// lists. Default and recommended for most workloads.
    #[default]
    Rrf,
    /// Maximal Marginal Relevance — trades off relevance against diversity.
    /// Useful when query results tend to be near-duplicate.
    Mmr,
}

#[derive(Debug, Deserialize, Clone)]
pub struct RetrievalSection {
    #[serde(default)]
    pub metadata_prefilter_enabled: bool,
    #[serde(default = "default_metadata_scan_limit")]
    pub metadata_scan_limit: u32,
    #[serde(default)]
    pub metadata_relax_if_empty: bool,
    /// Reranking strategy applied after parallel search.
    #[serde(default)]
    pub reranker: RerankerConfig,
}

impl Default for RetrievalSection {
    fn default() -> Self {
        Self {
            metadata_prefilter_enabled: true,
            metadata_scan_limit: default_metadata_scan_limit(),
            metadata_relax_if_empty: false,
            reranker: RerankerConfig::default(),
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
fn default_session_summary_threshold() -> u32 {
    10
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
        if let Ok(v) = std::env::var("MNEMO_QDRANT_PREFIX") {
            config.qdrant.collection_prefix = v;
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
        if let Ok(v) = std::env::var("MNEMO_EMBEDDING_PROVIDER") {
            config.embedding.provider = v;
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

        // Extraction overrides
        if let Ok(v) = std::env::var("MNEMO_SESSION_SUMMARY_THRESHOLD") {
            if let Ok(n) = v.parse() {
                config.extraction.session_summary_threshold = n;
            }
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

        // SOC 2 compliance overrides
        if let Ok(v) = std::env::var("MNEMO_REQUIRE_TLS") {
            config.server.require_tls = v == "true" || v == "1";
        }
        if let Ok(v) = std::env::var("MNEMO_AUDIT_SIGNING_SECRET") {
            if !v.is_empty() {
                config.server.audit_signing_secret = Some(v);
            }
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

// =============================================================================
// Tests — QA/QC Phase 2: CFG-01 through CFG-06
// =============================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::Mutex;

    // Environment variable tests MUST be serialized because env vars are
    // process-global. This mutex prevents parallel test runners from
    // stomping each other's env state.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Helper: clear all MNEMO_* env vars to ensure a clean slate.
    fn clear_mnemo_env() {
        let vars: Vec<String> = std::env::vars()
            .filter(|(k, _)| k.starts_with("MNEMO_"))
            .map(|(k, _)| k)
            .collect();
        for k in vars {
            std::env::remove_var(&k);
        }
    }

    /// Helper: write a TOML string to a temp file and return its path.
    fn write_temp_toml(content: &str) -> String {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("mnemo_test_{}.toml", uuid::Uuid::now_v7()));
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        path.to_str().unwrap().to_string()
    }

    // =========================================================================
    // CFG-01: default.toml parses without error, all sections present
    // =========================================================================
    #[test]
    fn cfg01_default_toml_parses_all_sections() {
        let _lock = ENV_LOCK.lock().unwrap();
        clear_mnemo_env();

        let config = MnemoConfig::load(Some("../../config/default.toml"))
            .expect("default.toml must parse without error");

        // Server
        assert_eq!(config.server.host, "0.0.0.0");
        assert_eq!(config.server.port, 8080);

        // Auth
        assert!(!config.auth.enabled);
        assert!(config.auth.api_keys.is_empty());

        // Redis
        assert_eq!(config.redis.url, "redis://localhost:6379");
        assert_eq!(config.redis.prefix, "mnemo:");

        // Qdrant
        assert_eq!(config.qdrant.url, "http://localhost:6334");
        assert_eq!(config.qdrant.collection_prefix, "mnemo_");

        // LLM — default.toml sets anthropic, not openai
        assert_eq!(config.llm.provider, "anthropic");
        assert_eq!(config.llm.model, "claude-sonnet-4-20250514");
        assert_eq!(config.llm.max_tokens, 2048);
        assert!(config.llm.api_key.is_empty());

        // Embedding
        assert_eq!(config.embedding.provider, "openai");
        assert_eq!(config.embedding.model, "text-embedding-3-small");
        assert_eq!(config.embedding.dimensions, 1536);

        // Extraction
        assert_eq!(config.extraction.batch_size, 10);
        assert_eq!(config.extraction.concurrency, 4);
        assert_eq!(config.extraction.max_retries, 3);
        assert_eq!(config.extraction.poll_interval_ms, 500);

        // Retrieval
        assert!(config.retrieval.metadata_prefilter_enabled);
        assert_eq!(config.retrieval.metadata_scan_limit, 400);
        assert!(!config.retrieval.metadata_relax_if_empty);

        // Webhooks
        assert!(config.webhooks.enabled);
        assert_eq!(config.webhooks.max_attempts, 3);
        assert_eq!(config.webhooks.base_backoff_ms, 200);
        assert_eq!(config.webhooks.request_timeout_ms, 3000);
        assert_eq!(config.webhooks.max_events_per_webhook, 1000);
        assert_eq!(config.webhooks.rate_limit_per_minute, 120);
        assert_eq!(config.webhooks.circuit_breaker_threshold, 5);
        assert_eq!(config.webhooks.circuit_breaker_cooldown_ms, 60_000);
        assert!(config.webhooks.persistence_enabled);
        assert_eq!(config.webhooks.persistence_prefix, "webhooks");
    }

    #[test]
    fn cfg01_no_path_loads_defaults() {
        let _lock = ENV_LOCK.lock().unwrap();
        clear_mnemo_env();

        let config = MnemoConfig::load(None).expect("None path must produce default config");

        // Verify code defaults (these differ from default.toml for llm.provider)
        assert_eq!(config.server.host, "0.0.0.0");
        assert_eq!(config.server.port, 8080);
        assert_eq!(config.llm.provider, "openai"); // code default, not toml
        assert_eq!(config.llm.model, "gpt-4o-mini"); // code default
        assert_eq!(config.redis.url, "redis://localhost:6379");
    }

    // =========================================================================
    // CFG-02: Environment variable overrides work
    // =========================================================================
    #[test]
    fn cfg02_env_overrides_server() {
        let _lock = ENV_LOCK.lock().unwrap();
        clear_mnemo_env();

        std::env::set_var("MNEMO_SERVER_HOST", "127.0.0.1");
        std::env::set_var("MNEMO_SERVER_PORT", "9090");

        let config = MnemoConfig::load(None).unwrap();
        assert_eq!(config.server.host, "127.0.0.1");
        assert_eq!(config.server.port, 9090);

        clear_mnemo_env();
    }

    #[test]
    fn cfg02_env_overrides_redis_qdrant() {
        let _lock = ENV_LOCK.lock().unwrap();
        clear_mnemo_env();

        std::env::set_var("MNEMO_REDIS_URL", "redis://custom:6380");
        std::env::set_var("MNEMO_QDRANT_URL", "http://custom:6335");
        std::env::set_var("MNEMO_QDRANT_PREFIX", "mnemo_384_");

        let config = MnemoConfig::load(None).unwrap();
        assert_eq!(config.redis.url, "redis://custom:6380");
        assert_eq!(config.qdrant.url, "http://custom:6335");
        assert_eq!(config.qdrant.collection_prefix, "mnemo_384_");

        clear_mnemo_env();
    }

    #[test]
    fn cfg02_env_overrides_llm() {
        let _lock = ENV_LOCK.lock().unwrap();
        clear_mnemo_env();

        std::env::set_var("MNEMO_LLM_PROVIDER", "anthropic");
        std::env::set_var("MNEMO_LLM_API_KEY", "sk-test-key");
        std::env::set_var("MNEMO_LLM_MODEL", "claude-sonnet-4-20250514");
        std::env::set_var("MNEMO_LLM_BASE_URL", "https://custom.api.com/v1");

        let config = MnemoConfig::load(None).unwrap();
        assert_eq!(config.llm.provider, "anthropic");
        assert_eq!(config.llm.api_key, "sk-test-key");
        assert_eq!(config.llm.model, "claude-sonnet-4-20250514");
        assert_eq!(config.llm.base_url, "https://custom.api.com/v1");

        // Also verify the llm_config() converter
        let llm_cfg = config.llm_config();
        assert_eq!(llm_cfg.api_key, Some("sk-test-key".to_string()));
        assert_eq!(
            llm_cfg.base_url,
            Some("https://custom.api.com/v1".to_string())
        );

        clear_mnemo_env();
    }

    #[test]
    fn cfg02_env_overrides_embedding() {
        let _lock = ENV_LOCK.lock().unwrap();
        clear_mnemo_env();

        std::env::set_var("MNEMO_EMBEDDING_API_KEY", "embed-key");
        std::env::set_var("MNEMO_EMBEDDING_MODEL", "voyage-large-2");
        std::env::set_var("MNEMO_EMBEDDING_BASE_URL", "https://embed.api.com");
        std::env::set_var("MNEMO_EMBEDDING_DIMENSIONS", "1024");

        let config = MnemoConfig::load(None).unwrap();
        assert_eq!(config.embedding.api_key, "embed-key");
        assert_eq!(config.embedding.model, "voyage-large-2");
        assert_eq!(config.embedding.base_url, "https://embed.api.com");
        assert_eq!(config.embedding.dimensions, 1024);

        // Also verify the embedding_config() converter
        let embed_cfg = config.embedding_config();
        assert_eq!(embed_cfg.api_key, Some("embed-key".to_string()));
        assert_eq!(embed_cfg.dimensions, 1024);

        clear_mnemo_env();
    }

    #[test]
    fn cfg02_env_overrides_auth() {
        let _lock = ENV_LOCK.lock().unwrap();
        clear_mnemo_env();

        std::env::set_var("MNEMO_AUTH_ENABLED", "true");
        std::env::set_var("MNEMO_AUTH_API_KEYS", "key-1, key-2, key-3");

        let config = MnemoConfig::load(None).unwrap();
        assert!(config.auth.enabled);
        assert_eq!(config.auth.api_keys, vec!["key-1", "key-2", "key-3"]);

        clear_mnemo_env();
    }

    #[test]
    fn cfg02_env_overrides_retrieval() {
        let _lock = ENV_LOCK.lock().unwrap();
        clear_mnemo_env();

        std::env::set_var("MNEMO_METADATA_PREFILTER_ENABLED", "false");
        std::env::set_var("MNEMO_METADATA_SCAN_LIMIT", "800");
        std::env::set_var("MNEMO_METADATA_RELAX_IF_EMPTY", "1");

        let config = MnemoConfig::load(None).unwrap();
        assert!(!config.retrieval.metadata_prefilter_enabled);
        assert_eq!(config.retrieval.metadata_scan_limit, 800);
        assert!(config.retrieval.metadata_relax_if_empty);

        clear_mnemo_env();
    }

    #[test]
    fn cfg02_env_overrides_webhooks() {
        let _lock = ENV_LOCK.lock().unwrap();
        clear_mnemo_env();

        std::env::set_var("MNEMO_WEBHOOKS_ENABLED", "false");
        std::env::set_var("MNEMO_WEBHOOKS_MAX_ATTEMPTS", "5");
        std::env::set_var("MNEMO_WEBHOOKS_BASE_BACKOFF_MS", "500");
        std::env::set_var("MNEMO_WEBHOOKS_TIMEOUT_MS", "5000");
        std::env::set_var("MNEMO_WEBHOOKS_MAX_EVENTS_PER_WEBHOOK", "2000");
        std::env::set_var("MNEMO_WEBHOOKS_RATE_LIMIT_PER_MINUTE", "60");
        std::env::set_var("MNEMO_WEBHOOKS_CIRCUIT_BREAKER_THRESHOLD", "10");
        std::env::set_var("MNEMO_WEBHOOKS_CIRCUIT_BREAKER_COOLDOWN_MS", "120000");
        std::env::set_var("MNEMO_WEBHOOKS_PERSISTENCE_ENABLED", "false");
        std::env::set_var("MNEMO_WEBHOOKS_PERSISTENCE_PREFIX", "custom_wh");

        let config = MnemoConfig::load(None).unwrap();
        assert!(!config.webhooks.enabled);
        assert_eq!(config.webhooks.max_attempts, 5);
        assert_eq!(config.webhooks.base_backoff_ms, 500);
        assert_eq!(config.webhooks.request_timeout_ms, 5000);
        assert_eq!(config.webhooks.max_events_per_webhook, 2000);
        assert_eq!(config.webhooks.rate_limit_per_minute, 60);
        assert_eq!(config.webhooks.circuit_breaker_threshold, 10);
        assert_eq!(config.webhooks.circuit_breaker_cooldown_ms, 120_000);
        assert!(!config.webhooks.persistence_enabled);
        assert_eq!(config.webhooks.persistence_prefix, "custom_wh");

        clear_mnemo_env();
    }

    // =========================================================================
    // CFG-03: Every README-documented env var is actually read
    // =========================================================================
    #[test]
    fn cfg03_all_documented_env_vars_are_read() {
        let _lock = ENV_LOCK.lock().unwrap();
        clear_mnemo_env();

        // Every env var documented in README.md's Configuration table.
        // We set each to a unique sentinel value, load config, verify it landed.
        type EnvCheck<'a> = (&'a str, &'a str, Box<dyn Fn(&MnemoConfig) -> bool>);
        let env_expectations: Vec<EnvCheck> = vec![
            (
                "MNEMO_LLM_API_KEY",
                "sentinel_llm_key",
                Box::new(|c: &MnemoConfig| c.llm.api_key == "sentinel_llm_key"),
            ),
            (
                "MNEMO_LLM_PROVIDER",
                "sentinel_prov",
                Box::new(|c: &MnemoConfig| c.llm.provider == "sentinel_prov"),
            ),
            (
                "MNEMO_LLM_MODEL",
                "sentinel_model",
                Box::new(|c: &MnemoConfig| c.llm.model == "sentinel_model"),
            ),
            (
                "MNEMO_EMBEDDING_API_KEY",
                "sentinel_embed_key",
                Box::new(|c: &MnemoConfig| c.embedding.api_key == "sentinel_embed_key"),
            ),
            (
                "MNEMO_AUTH_ENABLED",
                "true",
                Box::new(|c: &MnemoConfig| c.auth.enabled),
            ),
            (
                "MNEMO_AUTH_API_KEYS",
                "sentinel_key",
                Box::new(|c: &MnemoConfig| c.auth.api_keys.contains(&"sentinel_key".to_string())),
            ),
            (
                "MNEMO_REDIS_URL",
                "redis://sentinel:6379",
                Box::new(|c: &MnemoConfig| c.redis.url == "redis://sentinel:6379"),
            ),
            (
                "MNEMO_QDRANT_URL",
                "http://sentinel:6334",
                Box::new(|c: &MnemoConfig| c.qdrant.url == "http://sentinel:6334"),
            ),
            (
                "MNEMO_METADATA_PREFILTER_ENABLED",
                "false",
                Box::new(|c: &MnemoConfig| !c.retrieval.metadata_prefilter_enabled),
            ),
            (
                "MNEMO_METADATA_SCAN_LIMIT",
                "999",
                Box::new(|c: &MnemoConfig| c.retrieval.metadata_scan_limit == 999),
            ),
            (
                "MNEMO_METADATA_RELAX_IF_EMPTY",
                "true",
                Box::new(|c: &MnemoConfig| c.retrieval.metadata_relax_if_empty),
            ),
            (
                "MNEMO_WEBHOOKS_ENABLED",
                "false",
                Box::new(|c: &MnemoConfig| !c.webhooks.enabled),
            ),
            (
                "MNEMO_WEBHOOKS_MAX_ATTEMPTS",
                "7",
                Box::new(|c: &MnemoConfig| c.webhooks.max_attempts == 7),
            ),
            (
                "MNEMO_WEBHOOKS_BASE_BACKOFF_MS",
                "999",
                Box::new(|c: &MnemoConfig| c.webhooks.base_backoff_ms == 999),
            ),
            (
                "MNEMO_WEBHOOKS_TIMEOUT_MS",
                "9999",
                Box::new(|c: &MnemoConfig| c.webhooks.request_timeout_ms == 9999),
            ),
            (
                "MNEMO_WEBHOOKS_MAX_EVENTS_PER_WEBHOOK",
                "5000",
                Box::new(|c: &MnemoConfig| c.webhooks.max_events_per_webhook == 5000),
            ),
            (
                "MNEMO_WEBHOOKS_RATE_LIMIT_PER_MINUTE",
                "42",
                Box::new(|c: &MnemoConfig| c.webhooks.rate_limit_per_minute == 42),
            ),
            (
                "MNEMO_WEBHOOKS_CIRCUIT_BREAKER_THRESHOLD",
                "99",
                Box::new(|c: &MnemoConfig| c.webhooks.circuit_breaker_threshold == 99),
            ),
            (
                "MNEMO_WEBHOOKS_CIRCUIT_BREAKER_COOLDOWN_MS",
                "99999",
                Box::new(|c: &MnemoConfig| c.webhooks.circuit_breaker_cooldown_ms == 99999),
            ),
            (
                "MNEMO_WEBHOOKS_PERSISTENCE_ENABLED",
                "false",
                Box::new(|c: &MnemoConfig| !c.webhooks.persistence_enabled),
            ),
            (
                "MNEMO_WEBHOOKS_PERSISTENCE_PREFIX",
                "sentinel_pfx",
                Box::new(|c: &MnemoConfig| c.webhooks.persistence_prefix == "sentinel_pfx"),
            ),
            (
                "MNEMO_SERVER_PORT",
                "3333",
                Box::new(|c: &MnemoConfig| c.server.port == 3333),
            ),
        ];

        // Test each env var individually
        for (env_key, env_val, check) in &env_expectations {
            clear_mnemo_env();
            std::env::set_var(env_key, env_val);

            let config = MnemoConfig::load(None)
                .unwrap_or_else(|e| panic!("Config load failed for {}: {}", env_key, e));

            assert!(
                check(&config),
                "Env var {} = {} was NOT reflected in the loaded config",
                env_key,
                env_val,
            );
        }

        clear_mnemo_env();
    }

    // =========================================================================
    // CFG-04: Invalid config values produce graceful behavior (no panic)
    // =========================================================================
    #[test]
    fn cfg04_invalid_port_does_not_panic() {
        let _lock = ENV_LOCK.lock().unwrap();
        clear_mnemo_env();

        // "abc" is not a valid u16 — the code does unwrap_or(default)
        std::env::set_var("MNEMO_SERVER_PORT", "abc");
        let config = MnemoConfig::load(None).unwrap();
        // Should fall back to default port
        assert_eq!(config.server.port, 8080);

        clear_mnemo_env();
    }

    #[test]
    fn cfg04_invalid_dimensions_does_not_panic() {
        let _lock = ENV_LOCK.lock().unwrap();
        clear_mnemo_env();

        std::env::set_var("MNEMO_EMBEDDING_DIMENSIONS", "not_a_number");
        let config = MnemoConfig::load(None).unwrap();
        // Should keep default
        assert_eq!(config.embedding.dimensions, 1536);

        clear_mnemo_env();
    }

    #[test]
    fn cfg04_invalid_numeric_webhook_fields_graceful() {
        let _lock = ENV_LOCK.lock().unwrap();
        clear_mnemo_env();

        std::env::set_var("MNEMO_WEBHOOKS_MAX_ATTEMPTS", "xyz");
        std::env::set_var("MNEMO_WEBHOOKS_BASE_BACKOFF_MS", "xyz");
        std::env::set_var("MNEMO_WEBHOOKS_TIMEOUT_MS", "xyz");
        std::env::set_var("MNEMO_WEBHOOKS_MAX_EVENTS_PER_WEBHOOK", "xyz");
        std::env::set_var("MNEMO_WEBHOOKS_RATE_LIMIT_PER_MINUTE", "xyz");
        std::env::set_var("MNEMO_WEBHOOKS_CIRCUIT_BREAKER_THRESHOLD", "xyz");
        std::env::set_var("MNEMO_WEBHOOKS_CIRCUIT_BREAKER_COOLDOWN_MS", "xyz");
        std::env::set_var("MNEMO_METADATA_SCAN_LIMIT", "xyz");

        let config = MnemoConfig::load(None).unwrap();
        // All should keep their defaults — no panic, no error
        assert_eq!(config.webhooks.max_attempts, 3);
        assert_eq!(config.webhooks.base_backoff_ms, 200);
        assert_eq!(config.webhooks.request_timeout_ms, 3000);
        assert_eq!(config.webhooks.max_events_per_webhook, 1000);
        assert_eq!(config.webhooks.rate_limit_per_minute, 120);
        assert_eq!(config.webhooks.circuit_breaker_threshold, 5);
        assert_eq!(config.webhooks.circuit_breaker_cooldown_ms, 60_000);
        assert_eq!(config.retrieval.metadata_scan_limit, 400);

        clear_mnemo_env();
    }

    #[test]
    fn cfg04_invalid_toml_returns_error() {
        let _lock = ENV_LOCK.lock().unwrap();
        clear_mnemo_env();

        let path = write_temp_toml("this is not valid [[[toml");
        let result = MnemoConfig::load(Some(&path));
        assert!(result.is_err(), "Invalid TOML must return Err");
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("TOML"),
            "Error should mention TOML: {}",
            err_msg
        );

        std::fs::remove_file(&path).ok();
    }

    // =========================================================================
    // CFG-05: Missing file produces clear error; defaults are sane
    // =========================================================================
    #[test]
    fn cfg05_nonexistent_file_returns_error() {
        let _lock = ENV_LOCK.lock().unwrap();
        clear_mnemo_env();

        let result = MnemoConfig::load(Some("/tmp/this_file_does_not_exist_12345.toml"));
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("Failed to read"),
            "Error should mention 'Failed to read': {}",
            err_msg,
        );
    }

    #[test]
    fn cfg05_default_config_has_sane_values() {
        let config = MnemoConfig::default();

        // Server
        assert_eq!(config.server.host, "0.0.0.0");
        assert_eq!(config.server.port, 8080);
        assert_eq!(config.server.workers, 0);

        // Auth off by default
        assert!(!config.auth.enabled);

        // Redis and Qdrant point to localhost
        assert!(config.redis.url.contains("localhost"));
        assert!(config.qdrant.url.contains("localhost"));

        // LLM has a sensible model
        assert!(!config.llm.provider.is_empty());
        assert!(!config.llm.model.is_empty());
        assert!(config.llm.max_tokens > 0);

        // Embedding has dimensions > 0
        assert!(config.embedding.dimensions > 0);

        // Extraction concurrency > 0
        assert!(config.extraction.concurrency > 0);
        assert!(config.extraction.batch_size > 0);
    }

    // =========================================================================
    // CFG-06: Auth enabled without keys — verify behavior
    // =========================================================================
    #[test]
    fn cfg06_auth_enabled_without_keys_produces_empty_key_list() {
        let _lock = ENV_LOCK.lock().unwrap();
        clear_mnemo_env();

        std::env::set_var("MNEMO_AUTH_ENABLED", "true");
        // Deliberately NOT setting MNEMO_AUTH_API_KEYS

        let config = MnemoConfig::load(None).unwrap();
        assert!(config.auth.enabled);
        // Note: The config layer does NOT error — it allows an empty key list.
        // This means the server will start with auth enabled but no valid keys,
        // effectively locking out all requests. This is arguably a footgun.
        // We document this behavior; the server layer should guard against it.
        assert!(
            config.auth.api_keys.is_empty(),
            "With MNEMO_AUTH_ENABLED=true but no keys, api_keys should be empty"
        );

        clear_mnemo_env();
    }

    #[test]
    fn cfg06_auth_disabled_ignores_keys() {
        let _lock = ENV_LOCK.lock().unwrap();
        clear_mnemo_env();

        // Auth disabled, but keys provided — keys should still be stored
        std::env::set_var("MNEMO_AUTH_ENABLED", "false");
        std::env::set_var("MNEMO_AUTH_API_KEYS", "key-a,key-b");

        let config = MnemoConfig::load(None).unwrap();
        assert!(!config.auth.enabled);
        // Keys are still parsed and stored even when auth is disabled
        assert_eq!(config.auth.api_keys.len(), 2);

        clear_mnemo_env();
    }

    // =========================================================================
    // Additional: TOML file with env override layering
    // =========================================================================
    #[test]
    fn cfg_toml_values_overridden_by_env() {
        let _lock = ENV_LOCK.lock().unwrap();
        clear_mnemo_env();

        let toml = r#"
[server]
port = 9999

[llm]
provider = "ollama"
model = "llama3"
"#;
        let path = write_temp_toml(toml);

        // TOML says port=9999, but env says 7777
        std::env::set_var("MNEMO_SERVER_PORT", "7777");

        let config = MnemoConfig::load(Some(&path)).unwrap();
        // Env wins over TOML
        assert_eq!(config.server.port, 7777);
        // TOML value preserved when no env override
        assert_eq!(config.llm.provider, "ollama");
        assert_eq!(config.llm.model, "llama3");

        std::fs::remove_file(&path).ok();
        clear_mnemo_env();
    }

    #[test]
    fn cfg_partial_toml_fills_remaining_with_defaults() {
        let _lock = ENV_LOCK.lock().unwrap();
        clear_mnemo_env();

        // Only specify [server], everything else should get defaults
        let toml = r#"
[server]
port = 4444
"#;
        let path = write_temp_toml(toml);

        let config = MnemoConfig::load(Some(&path)).unwrap();
        assert_eq!(config.server.port, 4444);
        assert_eq!(config.server.host, "0.0.0.0"); // default
                                                   // Other sections should all be defaults
        assert_eq!(config.redis.url, "redis://localhost:6379");
        assert_eq!(config.llm.provider, "openai"); // code default
        assert_eq!(config.embedding.dimensions, 1536);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn cfg_empty_toml_file_loads_all_defaults() {
        let _lock = ENV_LOCK.lock().unwrap();
        clear_mnemo_env();

        let path = write_temp_toml("");
        let config = MnemoConfig::load(Some(&path)).unwrap();

        // Everything should be defaults
        assert_eq!(config.server.port, 8080);
        assert_eq!(config.llm.provider, "openai");
        assert_eq!(config.redis.url, "redis://localhost:6379");

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn cfg_auth_keys_extend_toml_keys() {
        let _lock = ENV_LOCK.lock().unwrap();
        clear_mnemo_env();

        let toml = r#"
[auth]
enabled = true
api_keys = ["toml-key-1"]
"#;
        let path = write_temp_toml(toml);

        // Env EXTENDS (does not replace) TOML keys
        std::env::set_var("MNEMO_AUTH_API_KEYS", "env-key-1,env-key-2");

        let config = MnemoConfig::load(Some(&path)).unwrap();
        assert!(config.auth.enabled);
        // Should have all 3 keys
        assert_eq!(config.auth.api_keys.len(), 3);
        assert!(config.auth.api_keys.contains(&"toml-key-1".to_string()));
        assert!(config.auth.api_keys.contains(&"env-key-1".to_string()));
        assert!(config.auth.api_keys.contains(&"env-key-2".to_string()));

        std::fs::remove_file(&path).ok();
        clear_mnemo_env();
    }

    #[test]
    fn cfg_llm_config_converter_empty_key_is_none() {
        let config = MnemoConfig::default();
        let llm = config.llm_config();
        assert_eq!(llm.api_key, None);
        assert_eq!(llm.base_url, None);

        let embed = config.embedding_config();
        assert_eq!(embed.api_key, None);
        assert_eq!(embed.base_url, None);
    }

    #[test]
    fn cfg_auth_enabled_accepts_both_true_and_1() {
        let _lock = ENV_LOCK.lock().unwrap();
        clear_mnemo_env();

        std::env::set_var("MNEMO_AUTH_ENABLED", "1");
        let config = MnemoConfig::load(None).unwrap();
        assert!(config.auth.enabled);

        std::env::set_var("MNEMO_AUTH_ENABLED", "true");
        let config = MnemoConfig::load(None).unwrap();
        assert!(config.auth.enabled);

        // Anything else is false
        std::env::set_var("MNEMO_AUTH_ENABLED", "yes");
        let config = MnemoConfig::load(None).unwrap();
        assert!(!config.auth.enabled);

        clear_mnemo_env();
    }
}
