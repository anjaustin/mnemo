use std::sync::Arc;

use mnemo_graph::GraphEngine;
use mnemo_retrieval::RetrievalEngine;
use mnemo_storage::{QdrantVectorStore, RedisStateStore};
use mnemo_llm::OpenAiCompatibleEmbedder;

/// Shared application state passed to all Axum route handlers.
#[derive(Clone)]
pub struct AppState {
    pub state_store: Arc<RedisStateStore>,
    pub vector_store: Arc<QdrantVectorStore>,
    pub retrieval: Arc<RetrievalEngine<RedisStateStore, QdrantVectorStore, OpenAiCompatibleEmbedder>>,
    pub graph: Arc<GraphEngine<RedisStateStore>>,
}
