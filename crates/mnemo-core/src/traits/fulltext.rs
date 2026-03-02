use uuid::Uuid;

use crate::traits::storage::StorageResult;

/// Trait for full-text search operations (RediSearch / BM25).
///
/// Complements `VectorStore` (semantic search) with keyword-exact matching.
/// Results are combined using Reciprocal Rank Fusion in the retrieval engine.
#[allow(async_fn_in_trait)]
pub trait FullTextStore: Send + Sync {
    /// Search entities by name and summary text.
    async fn search_entities_ft(
        &self,
        user_id: Uuid,
        query: &str,
        limit: u32,
    ) -> StorageResult<Vec<(Uuid, f32)>>;

    /// Search edges/facts by fact text.
    async fn search_edges_ft(
        &self,
        user_id: Uuid,
        query: &str,
        limit: u32,
    ) -> StorageResult<Vec<(Uuid, f32)>>;

    /// Search episodes by content text.
    async fn search_episodes_ft(
        &self,
        user_id: Uuid,
        query: &str,
        limit: u32,
    ) -> StorageResult<Vec<(Uuid, f32)>>;

    /// Create or update RediSearch indexes. Call on startup.
    async fn ensure_indexes(&self) -> StorageResult<()>;
}
