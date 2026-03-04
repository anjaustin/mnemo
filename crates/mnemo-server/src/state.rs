use std::collections::HashMap;
use std::sync::Arc;

use mnemo_graph::GraphEngine;
use mnemo_llm::OpenAiCompatibleEmbedder;
use mnemo_retrieval::RetrievalEngine;
use mnemo_storage::{QdrantVectorStore, RedisStateStore};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use uuid::Uuid;

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
    pub delivered_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryWebhookAuditRecord {
    pub id: Uuid,
    pub webhook_id: Uuid,
    pub action: String,
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

/// Shared application state passed to all Axum route handlers.
#[derive(Clone)]
pub struct AppState {
    pub state_store: Arc<RedisStateStore>,
    pub vector_store: Arc<QdrantVectorStore>,
    pub retrieval:
        Arc<RetrievalEngine<RedisStateStore, QdrantVectorStore, OpenAiCompatibleEmbedder>>,
    pub graph: Arc<GraphEngine<RedisStateStore>>,
    pub metadata_prefilter: MetadataPrefilterConfig,
    pub import_jobs: Arc<RwLock<HashMap<Uuid, ImportJobRecord>>>,
    pub import_idempotency: Arc<RwLock<HashMap<String, Uuid>>>,
    pub memory_webhooks: Arc<RwLock<HashMap<Uuid, MemoryWebhookSubscription>>>,
    pub memory_webhook_events: Arc<RwLock<HashMap<Uuid, Vec<MemoryWebhookEventRecord>>>>,
    pub memory_webhook_audit: Arc<RwLock<HashMap<Uuid, Vec<MemoryWebhookAuditRecord>>>>,
    pub webhook_runtime: Arc<RwLock<HashMap<Uuid, WebhookRuntimeState>>>,
    pub webhook_delivery: WebhookDeliveryConfig,
    pub webhook_http: Arc<reqwest::Client>,
    pub webhook_redis: Option<redis::aio::ConnectionManager>,
    pub webhook_redis_prefix: String,
}
