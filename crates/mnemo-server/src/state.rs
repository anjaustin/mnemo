use std::collections::HashMap;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use mnemo_core::models::entity::ExtractedEntity;
use mnemo_core::traits::blob::{BlobMetadata, BlobResult, BlobStore, PresignOptions};
use mnemo_core::traits::document::{
    DocumentConfig, DocumentFormat, DocumentParser, DocumentResult, ParsedDocument,
};
use mnemo_core::traits::llm::{ExtractionResult, LlmProvider, LlmResult, TokenUsage};
use mnemo_core::traits::transcription::{
    AudioFormat, Transcription, TranscriptionProvider, TranscriptionResult,
};
use mnemo_core::traits::vision::{ImageFormat, VisionAnalysis, VisionProvider, VisionResult};
use mnemo_graph::GraphEngine;
use mnemo_llm::{
    AnthropicProvider, AnthropicVisionProvider, EmbedderKind, OpenAITranscriptionProvider,
    OpenAIVisionProvider, OpenAiCompatibleProvider, PdfParser, TextDocumentParser,
};
use mnemo_lora::LoraAdaptedEmbedder;
use mnemo_retrieval::RetrievalEngine;
use mnemo_storage::{LocalBlobStore, QdrantVectorStore, RedisStateStore, S3BlobStore};

use crate::lora_handle::LoraEmbedderHandle;
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

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ImportJobStatus {
    Queued,
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum MemoryWebhookEventType {
    FactAdded,
    FactSuperseded,
    HeadAdvanced,
    ConflictDetected,
    RevalidationNeeded,
    ClarificationGenerated,
    ClarificationResolved,
    NarrativeRefreshed,
    PromotionProposed,
    PromotionApproved,
    PromotionRejected,
    PromotionExpired,
    PromotionConflictDetected,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct MemoryWebhookSubscription {
    pub id: Uuid,
    pub user_id: Uuid,
    pub user_identifier: String,
    pub target_url: String,
    #[serde(skip_serializing)]
    #[schema(ignore)]
    pub signing_secret: Option<String>,
    pub events: Vec<MemoryWebhookEventType>,
    pub enabled: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct MemoryWebhookEventRecord {
    pub id: Uuid,
    pub webhook_id: Uuid,
    pub event_type: MemoryWebhookEventType,
    pub user_id: Uuid,
    #[schema(value_type = Object)]
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

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct MemoryWebhookAuditRecord {
    pub id: Uuid,
    pub webhook_id: Uuid,
    pub action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[schema(value_type = Object)]
    pub details: serde_json::Value,
    pub at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
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

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct GovernanceAuditRecord {
    pub id: Uuid,
    pub user_id: Uuid,
    pub action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[schema(value_type = Object)]
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
    /// Allow localhost/internal IPs for webhook targets. ONLY enable in test/dev.
    pub allow_localhost: bool,
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

/// Type-erased blob store handle that is `Clone + Send + Sync`.
///
/// Wraps concrete blob store implementations so we can store one in `AppState`
/// without boxing an `async fn in trait`.
#[derive(Clone)]
pub enum BlobHandle {
    Local(Arc<LocalBlobStore>),
    S3(Arc<S3BlobStore>),
}

impl BlobHandle {
    pub async fn put(&self, key: &str, data: Vec<u8>, content_type: &str) -> BlobResult<BlobMetadata> {
        match self {
            BlobHandle::Local(store) => store.put(key, data, content_type).await,
            BlobHandle::S3(store) => store.put(key, data, content_type).await,
        }
    }

    pub async fn get(&self, key: &str) -> BlobResult<(Vec<u8>, BlobMetadata)> {
        match self {
            BlobHandle::Local(store) => store.get(key).await,
            BlobHandle::S3(store) => store.get(key).await,
        }
    }

    pub async fn delete(&self, key: &str) -> BlobResult<()> {
        match self {
            BlobHandle::Local(store) => store.delete(key).await,
            BlobHandle::S3(store) => store.delete(key).await,
        }
    }

    pub async fn exists(&self, key: &str) -> BlobResult<bool> {
        match self {
            BlobHandle::Local(store) => store.exists(key).await,
            BlobHandle::S3(store) => store.exists(key).await,
        }
    }

    pub async fn head(&self, key: &str) -> BlobResult<BlobMetadata> {
        match self {
            BlobHandle::Local(store) => store.head(key).await,
            BlobHandle::S3(store) => store.head(key).await,
        }
    }

    pub async fn presign_get(&self, key: &str, options: PresignOptions) -> BlobResult<Option<String>> {
        match self {
            BlobHandle::Local(store) => store.presign_get(key, options).await,
            BlobHandle::S3(store) => store.presign_get(key, options).await,
        }
    }

    pub async fn presign_put(
        &self,
        key: &str,
        content_type: &str,
        options: PresignOptions,
    ) -> BlobResult<Option<String>> {
        match self {
            BlobHandle::Local(store) => store.presign_put(key, content_type, options).await,
            BlobHandle::S3(store) => store.presign_put(key, content_type, options).await,
        }
    }

    /// Get the backend name for logging/debugging.
    pub fn backend_name(&self) -> &'static str {
        match self {
            BlobHandle::Local(_) => "local",
            BlobHandle::S3(_) => "s3",
        }
    }
}

/// Type-erased vision provider handle that is `Clone + Send + Sync`.
///
/// Wraps concrete vision provider implementations so we can store one in `AppState`
/// without boxing an `async fn in trait`.
#[derive(Clone)]
pub enum VisionHandle {
    Anthropic(Arc<AnthropicVisionProvider>),
    OpenAI(Arc<OpenAIVisionProvider>),
}

impl VisionHandle {
    pub async fn analyze(
        &self,
        image_data: &[u8],
        format: ImageFormat,
        prompt: Option<&str>,
    ) -> VisionResult<VisionAnalysis> {
        match self {
            VisionHandle::Anthropic(provider) => {
                provider.analyze_image(image_data, format, prompt).await
            }
            VisionHandle::OpenAI(provider) => {
                provider.analyze_image(image_data, format, prompt).await
            }
        }
    }

    pub fn provider_name(&self) -> &str {
        match self {
            VisionHandle::Anthropic(provider) => provider.provider_name(),
            VisionHandle::OpenAI(provider) => provider.provider_name(),
        }
    }

    pub fn model_name(&self) -> &str {
        match self {
            VisionHandle::Anthropic(provider) => provider.model_name(),
            VisionHandle::OpenAI(provider) => provider.model_name(),
        }
    }

    pub fn max_image_size(&self) -> u64 {
        match self {
            VisionHandle::Anthropic(provider) => provider.max_image_size(),
            VisionHandle::OpenAI(provider) => provider.max_image_size(),
        }
    }

    pub fn supports_format(&self, format: ImageFormat) -> bool {
        match self {
            VisionHandle::Anthropic(provider) => provider.supports_format(format),
            VisionHandle::OpenAI(provider) => provider.supports_format(format),
        }
    }
}

/// Type-erased transcription provider handle that is `Clone + Send + Sync`.
///
/// Wraps concrete transcription provider implementations so we can store one
/// in `AppState` without boxing an `async fn in trait`.
#[derive(Clone)]
pub enum TranscriptionHandle {
    OpenAI(Arc<OpenAITranscriptionProvider>),
}

impl TranscriptionHandle {
    pub async fn transcribe(
        &self,
        audio_data: &[u8],
        format: AudioFormat,
        filename: Option<&str>,
    ) -> TranscriptionResult<Transcription> {
        match self {
            TranscriptionHandle::OpenAI(provider) => {
                provider.transcribe(audio_data, format, filename).await
            }
        }
    }

    pub fn provider_name(&self) -> &str {
        match self {
            TranscriptionHandle::OpenAI(provider) => provider.provider_name(),
        }
    }

    pub fn model_name(&self) -> &str {
        match self {
            TranscriptionHandle::OpenAI(provider) => provider.model_name(),
        }
    }

    pub fn max_audio_size(&self) -> u64 {
        match self {
            TranscriptionHandle::OpenAI(provider) => provider.max_audio_size(),
        }
    }

    pub fn supports_format(&self, format: AudioFormat) -> bool {
        match self {
            TranscriptionHandle::OpenAI(provider) => provider.supports_format(format),
        }
    }

    pub fn supports_diarization(&self) -> bool {
        match self {
            TranscriptionHandle::OpenAI(provider) => provider.supports_diarization(),
        }
    }
}

/// Type-erased document parser handle that is `Clone + Send + Sync`.
///
/// Wraps concrete document parser implementations so we can store one
/// in `AppState` without boxing an `async fn in trait`.
#[derive(Clone)]
pub struct DocumentHandle {
    pdf_parser: Arc<PdfParser>,
    text_parser: Arc<TextDocumentParser>,
}

impl DocumentHandle {
    pub fn new() -> Self {
        Self {
            pdf_parser: Arc::new(PdfParser::new()),
            text_parser: Arc::new(TextDocumentParser::new()),
        }
    }

    pub async fn parse(
        &self,
        data: &[u8],
        format: DocumentFormat,
        config: &DocumentConfig,
    ) -> DocumentResult<ParsedDocument> {
        match format {
            DocumentFormat::Pdf => self.pdf_parser.parse(data, format, config).await,
            DocumentFormat::Txt | DocumentFormat::Markdown | DocumentFormat::Html => {
                self.text_parser.parse(data, format, config).await
            }
            _ => Err(mnemo_core::error::MnemoError::UnsupportedMediaType(format!(
                "Document format {:?} not yet supported",
                format
            ))),
        }
    }

    pub fn supports_format(&self, format: DocumentFormat) -> bool {
        self.pdf_parser.supports_format(format) || self.text_parser.supports_format(format)
    }

    pub fn max_document_size(&self) -> u64 {
        50 * 1024 * 1024 // 50 MB
    }
}

impl Default for DocumentHandle {
    fn default() -> Self {
        Self::new()
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
    pub retrieval: Arc<RetrievalEngine<RedisStateStore, QdrantVectorStore, LoraEmbedderHandle>>,
    /// When TinyLoRA is enabled, holds the shared `LoraAdaptedEmbedder` so that
    /// the DELETE /lora handler can evict the in-memory cache after a reset.
    /// `None` when `MNEMO_LORA_ENABLED=false`.
    pub lora_embedder: Option<Arc<LoraAdaptedEmbedder<EmbedderKind, RedisStateStore>>>,
    pub graph: Arc<GraphEngine<RedisStateStore>>,
    /// LLM provider for on-demand extraction (e.g. `POST /api/v1/memory/extract`).
    /// `None` when no LLM is configured (no-op mode).
    pub llm: Option<LlmHandle>,
    /// Blob storage for multi-modal attachments (images, audio, documents).
    /// `None` when blob storage is not configured.
    pub blob_store: Option<BlobHandle>,
    /// Blob storage configuration (size limits, allowed types).
    pub blob_config: Option<crate::config::BlobSection>,
    /// Vision provider for image analysis (description, OCR, entity extraction).
    /// `None` when vision processing is not configured.
    pub vision: Option<VisionHandle>,
    /// Vision configuration (sync vs async processing, etc.).
    pub vision_config: Option<crate::config::VisionSection>,
    /// Transcription provider for audio-to-text (Whisper, etc.).
    /// `None` when transcription is not configured.
    pub transcription: Option<TranscriptionHandle>,
    /// Transcription configuration (sync vs async processing, etc.).
    pub transcription_config: Option<crate::config::TranscriptionSection>,
    /// Document parser for PDF, text, and other document formats.
    /// `None` when document parsing is not configured.
    pub document: Option<DocumentHandle>,
    /// Document parsing configuration (chunk strategy, size limits, etc.).
    pub document_config: Option<crate::config::DocumentSection>,
    pub metadata_prefilter: MetadataPrefilterConfig,
    pub reranker: RerankerMode,
    pub import_jobs: Arc<RwLock<HashMap<Uuid, ImportJobRecord>>>,
    pub import_idempotency: Arc<RwLock<HashMap<String, Uuid>>>,
    /// P1-2: Semaphore for limiting concurrent import jobs globally.
    pub import_semaphore: Arc<tokio::sync::Semaphore>,
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
    /// DAG pipeline metrics — per-step execution counts, latency, dead-letter queue.
    pub pipeline_metrics: Arc<mnemo_ingest::dag::PipelineMetrics>,
    /// Delta consensus sync status — node identity, vector clock, peer tracking.
    pub sync_status: Arc<RwLock<mnemo_core::sync::SyncStatus>>,
    /// Shared auth configuration — held here so REST handlers that revoke/rotate
    /// API keys can immediately invalidate the in-memory key cache, closing the
    /// 30-second revocation window (P1-5).
    pub auth_config: Arc<crate::middleware::AuthConfig>,
}
