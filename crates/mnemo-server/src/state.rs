use std::sync::Arc;

use mnemo_graph::GraphEngine;
use mnemo_llm::OpenAiCompatibleEmbedder;
use mnemo_retrieval::RetrievalEngine;
use mnemo_storage::{QdrantVectorStore, RedisStateStore};

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
}
