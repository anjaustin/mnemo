use qdrant_client::qdrant::{
    CreateCollectionBuilder, Distance, PointStruct, SearchPointsBuilder,
    UpsertPointsBuilder, VectorParamsBuilder, DeletePointsBuilder,
    Filter, FieldCondition, Match, MatchValue, PointId,
    value::Kind, Value as QdrantValue,
};
use qdrant_client::Qdrant;
use serde_json::Value;
use uuid::Uuid;

use mnemo_core::error::MnemoError;
use mnemo_core::traits::storage::{StorageResult, VectorStore};

/// Qdrant-backed vector store for embeddings and semantic search.
///
/// Collections:
/// - `{prefix}entities`  — entity embeddings
/// - `{prefix}edges`     — edge/fact embeddings
/// - `{prefix}episodes`  — episode content embeddings
///
/// All points include a `user_id` payload field for tenant filtering.
pub struct QdrantVectorStore {
    client: Qdrant,
    prefix: String,
    dimensions: u32,
}

impl QdrantVectorStore {
    pub async fn new(url: &str, prefix: &str, dimensions: u32) -> Result<Self, MnemoError> {
        let client = Qdrant::from_url(url)
            .build()
            .map_err(|e| MnemoError::Qdrant(format!("Failed to connect: {}", e)))?;

        let store = Self {
            client,
            prefix: prefix.to_string(),
            dimensions,
        };

        // Ensure collections exist
        store.ensure_collection("entities").await?;
        store.ensure_collection("edges").await?;
        store.ensure_collection("episodes").await?;

        Ok(store)
    }

    fn collection_name(&self, name: &str) -> String {
        format!("{}{}", self.prefix, name)
    }

    async fn ensure_collection(&self, name: &str) -> StorageResult<()> {
        let coll_name = self.collection_name(name);
        let exists = self.client
            .collection_exists(&coll_name)
            .await
            .map_err(|e| MnemoError::Qdrant(e.to_string()))?;

        if !exists {
            self.client
                .create_collection(
                    CreateCollectionBuilder::new(&coll_name)
                        .vectors_config(
                            VectorParamsBuilder::new(self.dimensions as u64, Distance::Cosine)
                        ),
                )
                .await
                .map_err(|e| MnemoError::Qdrant(format!("Failed to create collection: {}", e)))?;

            tracing::info!(collection = %coll_name, "Created Qdrant collection");
        }

        Ok(())
    }

    fn uuid_to_point_id(id: Uuid) -> PointId {
        PointId::from(id.to_string())
    }

    fn payload_from_json(user_id: Uuid, extra: Value) -> std::collections::HashMap<String, QdrantValue> {
        let mut payload = std::collections::HashMap::new();
        payload.insert(
            "user_id".to_string(),
            QdrantValue { kind: Some(Kind::StringValue(user_id.to_string())) },
        );

        if let Value::Object(map) = extra {
            for (k, v) in map {
                let qv = match v {
                    Value::String(s) => QdrantValue { kind: Some(Kind::StringValue(s)) },
                    Value::Number(n) => {
                        if let Some(f) = n.as_f64() {
                            QdrantValue { kind: Some(Kind::DoubleValue(f)) }
                        } else {
                            continue;
                        }
                    }
                    Value::Bool(b) => QdrantValue { kind: Some(Kind::BoolValue(b)) },
                    _ => QdrantValue { kind: Some(Kind::StringValue(v.to_string())) },
                };
                payload.insert(k, qv);
            }
        }

        payload
    }

    fn user_filter(user_id: Uuid) -> Filter {
        Filter::must([FieldCondition {
            key: "user_id".to_string(),
            r#match: Some(Match {
                match_value: Some(MatchValue::Keyword(user_id.to_string())),
            }),
            ..Default::default()
        }
        .into()])
    }

    async fn upsert_point(
        &self,
        collection: &str,
        id: Uuid,
        user_id: Uuid,
        embedding: Vec<f32>,
        payload: Value,
    ) -> StorageResult<()> {
        let coll_name = self.collection_name(collection);
        let point = PointStruct::new(
            Self::uuid_to_point_id(id),
            embedding,
            Self::payload_from_json(user_id, payload),
        );

        self.client
            .upsert_points(UpsertPointsBuilder::new(&coll_name, vec![point]).wait(true))
            .await
            .map_err(|e| MnemoError::Qdrant(format!("Upsert failed: {}", e)))?;

        Ok(())
    }

    async fn search_collection(
        &self,
        collection: &str,
        user_id: Uuid,
        query_embedding: Vec<f32>,
        limit: u32,
        min_score: f32,
    ) -> StorageResult<Vec<(Uuid, f32)>> {
        let coll_name = self.collection_name(collection);

        let results = self.client
            .search_points(
                SearchPointsBuilder::new(&coll_name, query_embedding, limit as u64)
                    .filter(Self::user_filter(user_id))
                    .score_threshold(min_score)
            )
            .await
            .map_err(|e| MnemoError::Qdrant(format!("Search failed: {}", e)))?;

        let mut hits = Vec::with_capacity(results.result.len());
        for point in results.result {
            if let Some(uuid) = extract_point_uuid(&point.id) {
                hits.push((uuid, point.score));
            }
        }

        Ok(hits)
    }
}

/// Extract a UUID from a Qdrant PointId.
/// Handles both string-based UUIDs and the UUID variant.
fn extract_point_uuid(point_id: &Option<PointId>) -> Option<Uuid> {
    let pid = point_id.as_ref()?;
    match &pid.point_id_options {
        Some(qdrant_client::qdrant::point_id::PointIdOptions::Uuid(s)) => {
            Uuid::parse_str(s).ok()
        }
        Some(qdrant_client::qdrant::point_id::PointIdOptions::Num(_)) => {
            None // We only use UUID-based IDs
        }
        None => None,
    }
}

impl VectorStore for QdrantVectorStore {
    async fn upsert_entity_embedding(
        &self,
        entity_id: Uuid,
        user_id: Uuid,
        embedding: Vec<f32>,
        payload: Value,
    ) -> StorageResult<()> {
        self.upsert_point("entities", entity_id, user_id, embedding, payload).await
    }

    async fn upsert_edge_embedding(
        &self,
        edge_id: Uuid,
        user_id: Uuid,
        embedding: Vec<f32>,
        payload: Value,
    ) -> StorageResult<()> {
        self.upsert_point("edges", edge_id, user_id, embedding, payload).await
    }

    async fn upsert_episode_embedding(
        &self,
        episode_id: Uuid,
        user_id: Uuid,
        embedding: Vec<f32>,
        payload: Value,
    ) -> StorageResult<()> {
        self.upsert_point("episodes", episode_id, user_id, embedding, payload).await
    }

    async fn search_entities(
        &self,
        user_id: Uuid,
        query_embedding: Vec<f32>,
        limit: u32,
        min_score: f32,
    ) -> StorageResult<Vec<(Uuid, f32)>> {
        self.search_collection("entities", user_id, query_embedding, limit, min_score).await
    }

    async fn search_edges(
        &self,
        user_id: Uuid,
        query_embedding: Vec<f32>,
        limit: u32,
        min_score: f32,
    ) -> StorageResult<Vec<(Uuid, f32)>> {
        self.search_collection("edges", user_id, query_embedding, limit, min_score).await
    }

    async fn search_episodes(
        &self,
        user_id: Uuid,
        query_embedding: Vec<f32>,
        limit: u32,
        min_score: f32,
    ) -> StorageResult<Vec<(Uuid, f32)>> {
        self.search_collection("episodes", user_id, query_embedding, limit, min_score).await
    }

    async fn delete_user_vectors(&self, user_id: Uuid) -> StorageResult<()> {
        let filter = Self::user_filter(user_id);
        for collection in &["entities", "edges", "episodes"] {
            let coll_name = self.collection_name(collection);
            self.client
                .delete_points(
                    DeletePointsBuilder::new(&coll_name)
                        .points(filter.clone())
                )
                .await
                .map_err(|e| MnemoError::Qdrant(format!("Delete failed: {}", e)))?;
        }
        tracing::info!(user_id = %user_id, "Deleted all vectors for user");
        Ok(())
    }
}
