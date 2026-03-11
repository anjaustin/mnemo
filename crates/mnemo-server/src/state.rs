use std::collections::HashMap;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use mnemo_core::models::entity::ExtractedEntity;
use mnemo_core::traits::llm::{ExtractionResult, LlmProvider, LlmResult, TokenUsage};
use mnemo_graph::GraphEngine;
use mnemo_llm::{AnthropicProvider, EmbedderKind, OpenAiCompatibleProvider};
use mnemo_retrieval::RetrievalEngine;
use mnemo_storage::{QdrantVectorStore, RedisStateStore};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use uuid::Uuid;

/// Which merge/rerank strategy the retrieval engine uses after parallel search.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RerankerMode {
    /// Reciprocal Rank Fusion — boosts consensus items across ranked lists.
    Rrf,
    /// Maximal Marginal Relevance — trades relevance for result diversity.
    Mmr,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportJobStatus {
    Queued,
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportJobRecord {
    pub id: Uuid,
    pub source: String,
    pub user: String,
    pub dry_run: bool,
    pub status: ImportJobStatus,
    pub total_messages: u32,
    pub imported_messages: u32,
    pub failed_messages: u32,
    pub sessions_touched: u32,
    pub errors: Vec<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,
    pub finished_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Clone, Copy)]
pub struct MetadataPrefilterConfig {
    pub enabled: bool,
    pub scan_limit: u32,
    pub relax_if_empty: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryWebhookEventType {
    FactAdded,
    FactSuperseded,
    HeadAdvanced,
    ConflictDetected,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryWebhookSubscription {
    pub id: Uuid,
    pub user_id: Uuid,
    pub user_identifier: String,
    pub target_url: String,
    #[serde(skip_serializing)]
    pub signing_secret: Option<String>,
    pub events: Vec<MemoryWebhookEventType>,
    pub enabled: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryWebhookEventRecord {
    pub id: Uuid,
    pub webhook_id: Uuid,
    pub event_type: MemoryWebhookEventType,
    pub user_id: Uuid,
    pub payload: serde_json::Value,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub attempts: u32,
    pub delivered: bool,
    pub dead_letter: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivered_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryWebhookAuditRecord {
    pub id: Uuid,
    pub webhook_id: Uuid,
    pub action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    pub details: serde_json::Value,
    pub at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserPolicyRecord {
    pub user_id: Uuid,
    pub user_identifier: String,
    pub retention_days_message: u32,
    pub retention_days_text: u32,
    pub retention_days_json: u32,
    pub webhook_domain_allowlist: Vec<String>,
    pub default_memory_contract: String,
    pub default_retrieval_policy: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GovernanceAuditRecord {
    pub id: Uuid,
    pub user_id: Uuid,
    pub action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    pub details: serde_json::Value,
    pub at: chrono::DateTime<chrono::Utc>,
}

#[derive(Clone, Copy)]
pub struct WebhookDeliveryConfig {
    pub enabled: bool,
    pub max_attempts: u32,
    pub base_backoff_ms: u64,
    pub request_timeout_ms: u64,
    pub max_events_per_webhook: usize,
    pub rate_limit_per_minute: u32,
    pub circuit_breaker_threshold: u32,
    pub circuit_breaker_cooldown_ms: u64,
    pub persistence_enabled: bool,
}

#[derive(Debug, Clone)]
pub struct WebhookRuntimeState {
    pub window_started_at: chrono::DateTime<chrono::Utc>,
    pub sent_in_window: u32,
    pub consecutive_failures: u32,
    pub circuit_open_until: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Default)]
pub struct ServerMetrics {
    pub http_requests_total: AtomicU64,
    pub http_responses_2xx: AtomicU64,
    pub http_responses_4xx: AtomicU64,
    pub http_responses_5xx: AtomicU64,
    pub webhook_deliveries_success_total: AtomicU64,
    pub webhook_deliveries_failure_total: AtomicU64,
    pub webhook_dead_letter_total: AtomicU64,
    pub webhook_retry_queued_total: AtomicU64,
    pub webhook_replay_requests_total: AtomicU64,
    pub policy_update_total: AtomicU64,
    pub policy_violation_total: AtomicU64,
    pub agent_identity_reads_total: AtomicU64,
    pub agent_identity_updates_total: AtomicU64,
    pub agent_experience_events_total: AtomicU64,
    pub agent_promotion_proposals_total: AtomicU64,
}

/// Type-erased LLM handle that is `Clone + Send + Sync`.
///
/// Wraps concrete provider types so we can store one in `AppState` without
/// boxing an `async fn in trait` (which is not yet dyn-compatible in stable
/// Rust without `async-trait`).
#[derive(Clone)]
pub enum LlmHandle {
    Anthropic(Arc<AnthropicProvider>),
    OpenAiCompat(Arc<OpenAiCompatibleProvider>),
}

impl LlmHandle {
    pub async fn extract(
        &self,
        content: &str,
        hints: &[ExtractedEntity],
    ) -> LlmResult<ExtractionResult> {
        match self {
            LlmHandle::Anthropic(llm) => {
                llm.extract_entities_and_relationships(content, hints).await
            }
            LlmHandle::OpenAiCompat(llm) => {
                llm.extract_entities_and_relationships(content, hints).await
            }
        }
    }

    pub fn provider_name(&self) -> &str {
        match self {
            LlmHandle::Anthropic(llm) => llm.provider_name(),
            LlmHandle::OpenAiCompat(llm) => llm.provider_name(),
        }
    }

    pub fn model_name(&self) -> &str {
        match self {
            LlmHandle::Anthropic(llm) => llm.model_name(),
            LlmHandle::OpenAiCompat(llm) => llm.model_name(),
        }
    }

    pub async fn summarize_with_usage(
        &self,
        content: &str,
        max_tokens: u32,
    ) -> LlmResult<(String, TokenUsage)> {
        match self {
            LlmHandle::Anthropic(llm) => llm.summarize_with_usage(content, max_tokens).await,
            LlmHandle::OpenAiCompat(llm) => llm.summarize_with_usage(content, max_tokens).await,
        }
    }
}

/// Re-export LlmSpan from mnemo-core so both the server routes and
/// the ingest worker share the same concrete type.
pub use mnemo_core::models::span::LlmSpan;

/// Re-export MemoryDigest from mnemo-ingest so both the server routes and
/// the ingest worker share the same concrete type.
pub use mnemo_ingest::MemoryDigest;

/// Shared application state passed to all Axum route handlers.
#[derive(Clone)]
pub struct AppState {
    pub state_store: Arc<RedisStateStore>,
    pub vector_store: Arc<QdrantVectorStore>,
    pub retrieval: Arc<RetrievalEngine<RedisStateStore, QdrantVectorStore, EmbedderKind>>,
    pub graph: Arc<GraphEngine<RedisStateStore>>,
    /// LLM provider for on-demand extraction (e.g. `POST /api/v1/memory/extract`).
    /// `None` when no LLM is configured (no-op mode).
    pub llm: Option<LlmHandle>,
    pub metadata_prefilter: MetadataPrefilterConfig,
    pub reranker: RerankerMode,
    pub import_jobs: Arc<RwLock<HashMap<Uuid, ImportJobRecord>>>,
    pub import_idempotency: Arc<RwLock<HashMap<String, Uuid>>>,
    pub memory_webhooks: Arc<RwLock<HashMap<Uuid, MemoryWebhookSubscription>>>,
    pub memory_webhook_events: Arc<RwLock<HashMap<Uuid, Vec<MemoryWebhookEventRecord>>>>,
    pub memory_webhook_audit: Arc<RwLock<HashMap<Uuid, Vec<MemoryWebhookAuditRecord>>>>,
    pub user_policies: Arc<RwLock<HashMap<Uuid, UserPolicyRecord>>>,
    pub governance_audit: Arc<RwLock<HashMap<Uuid, Vec<GovernanceAuditRecord>>>>,
    pub webhook_runtime: Arc<RwLock<HashMap<Uuid, WebhookRuntimeState>>>,
    pub webhook_delivery: WebhookDeliveryConfig,
    pub webhook_http: Arc<reqwest::Client>,
    pub webhook_redis: Option<redis::aio::ConnectionManager>,
    pub webhook_redis_prefix: String,
    pub metrics: Arc<ServerMetrics>,
    /// LLM call spans — keyed by request_id then by span id.
    /// Bounded ring-buffer per request (last 500 requests retained).
    pub llm_spans: Arc<RwLock<std::collections::VecDeque<LlmSpan>>>,
    /// Latest memory digest per user. Shared with the ingest worker for
    /// background sleep-time compute.
    pub memory_digests: mnemo_ingest::DigestCache,
    /// If true, reject non-https webhook targets (SOC 2 compliance).
    pub require_tls: bool,
    /// HMAC secret for signing audit export responses (SOC 2 compliance).
    pub audit_signing_secret: Option<String>,
    /// Temporal tensor compression config.
    pub compression_config: mnemo_retrieval::compression::CompressionConfig,
    /// Temporal tensor compression stats (atomic, shared with background sweep).
    pub compression_stats: Arc<mnemo_retrieval::compression::CompressionStats>,
    /// Embedding dimensions (needed for compression storage estimates).
    pub embedding_dimensions: u32,
    /// Hyperbolic HNSW config for Poincare ball entity re-ranking.
    pub hyperbolic_config: mnemo_retrieval::hyperbolic::HyperbolicConfig,
}
