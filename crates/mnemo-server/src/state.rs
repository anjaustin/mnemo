use std::collections::HashMap;
use std::sync::Arc;

use mnemo_graph::GraphEngine;
use mnemo_llm::OpenAiCompatibleEmbedder;
use mnemo_retrieval::RetrievalEngine;
use mnemo_storage::{QdrantVectorStore, RedisStateStore};
use serde::Serialize;
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
}
