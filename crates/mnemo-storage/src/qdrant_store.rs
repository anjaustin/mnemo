use qdrant_client::qdrant::r#match::MatchValue;
use qdrant_client::qdrant::{
    value::Kind, CountPointsBuilder, CreateCollectionBuilder, DeletePointsBuilder, Distance,
    FieldCondition, Filter, Match, PointId, PointStruct, PointsIdsList, SearchPointsBuilder,
    UpsertPointsBuilder, Value as QdrantValue, VectorParamsBuilder,
};
use qdrant_client::Qdrant;
use serde_json::Value;
use uuid::Uuid;

use mnemo_core::error::MnemoError;
use mnemo_core::traits::storage::{RawVectorStore, StorageResult, VectorHit, VectorStore};

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
            .skip_compatibility_check()
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
        let exists = self
            .client
            .collection_exists(&coll_name)
            .await
            .map_err(|e| MnemoError::Qdrant(e.to_string()))?;

        if !exists {
            match self
                .client
                .create_collection(CreateCollectionBuilder::new(&coll_name).vectors_config(
                    VectorParamsBuilder::new(self.dimensions as u64, Distance::Cosine),
                ))
                .await
            {
                Ok(_) => {
                    tracing::info!(collection = %coll_name, "Created Qdrant collection");
                }
                Err(e) => {
                    // Handle TOCTOU race: another process may have created the collection
                    // between our existence check and create call.
                    let msg = e.to_string();
                    if msg.contains("already exists") {
                        tracing::debug!(collection = %coll_name, "Collection already exists (concurrent creation)");
                    } else {
                        return Err(MnemoError::Qdrant(format!(
                            "Failed to create collection: {}",
                            msg
                        )));
                    }
                }
            }
        }

        Ok(())
    }

    fn uuid_to_point_id(id: Uuid) -> PointId {
        PointId::from(id.to_string())
    }

    fn payload_from_json(
        user_id: Uuid,
        extra: Value,
    ) -> std::collections::HashMap<String, QdrantValue> {
        let mut payload = std::collections::HashMap::new();
        payload.insert(
            "user_id".to_string(),
            QdrantValue {
                kind: Some(Kind::StringValue(user_id.to_string())),
            },
        );

        if let Value::Object(map) = extra {
            for (k, v) in map {
                let qv = match v {
                    Value::String(s) => QdrantValue {
                        kind: Some(Kind::StringValue(s)),
                    },
                    Value::Number(n) => {
                        if let Some(f) = n.as_f64() {
                            QdrantValue {
                                kind: Some(Kind::DoubleValue(f)),
                            }
                        } else {
                            continue;
                        }
                    }
                    Value::Bool(b) => QdrantValue {
                        kind: Some(Kind::BoolValue(b)),
                    },
                    _ => QdrantValue {
                        kind: Some(Kind::StringValue(v.to_string())),
                    },
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

        let results = self
            .client
            .search_points(
                SearchPointsBuilder::new(&coll_name, query_embedding, limit as u64)
                    .filter(Self::user_filter(user_id))
                    .score_threshold(min_score),
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
        Some(qdrant_client::qdrant::point_id::PointIdOptions::Uuid(s)) => Uuid::parse_str(s).ok(),
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
        self.upsert_point("entities", entity_id, user_id, embedding, payload)
            .await
    }

    async fn upsert_edge_embedding(
        &self,
        edge_id: Uuid,
        user_id: Uuid,
        embedding: Vec<f32>,
        payload: Value,
    ) -> StorageResult<()> {
        self.upsert_point("edges", edge_id, user_id, embedding, payload)
            .await
    }

    async fn upsert_episode_embedding(
        &self,
        episode_id: Uuid,
        user_id: Uuid,
        embedding: Vec<f32>,
        payload: Value,
    ) -> StorageResult<()> {
        self.upsert_point("episodes", episode_id, user_id, embedding, payload)
            .await
    }

    async fn search_entities(
        &self,
        user_id: Uuid,
        query_embedding: Vec<f32>,
        limit: u32,
        min_score: f32,
    ) -> StorageResult<Vec<(Uuid, f32)>> {
        self.search_collection("entities", user_id, query_embedding, limit, min_score)
            .await
    }

    async fn search_edges(
        &self,
        user_id: Uuid,
        query_embedding: Vec<f32>,
        limit: u32,
        min_score: f32,
    ) -> StorageResult<Vec<(Uuid, f32)>> {
        self.search_collection("edges", user_id, query_embedding, limit, min_score)
            .await
    }

    async fn search_episodes(
        &self,
        user_id: Uuid,
        query_embedding: Vec<f32>,
        limit: u32,
        min_score: f32,
    ) -> StorageResult<Vec<(Uuid, f32)>> {
        self.search_collection("episodes", user_id, query_embedding, limit, min_score)
            .await
    }

    async fn delete_user_vectors(&self, user_id: Uuid) -> StorageResult<()> {
        let filter = Self::user_filter(user_id);
        for collection in &["entities", "edges", "episodes"] {
            let coll_name = self.collection_name(collection);
            self.client
                .delete_points(DeletePointsBuilder::new(&coll_name).points(filter.clone()))
                .await
                .map_err(|e| MnemoError::Qdrant(format!("Delete failed: {}", e)))?;
        }
        tracing::info!(user_id = %user_id, "Deleted all vectors for user");
        Ok(())
    }
}

// ─── Raw Vector Store (namespace-based) ────────────────────────────

impl QdrantVectorStore {
    /// Build a raw-namespace collection name.
    /// Raw namespaces are prefixed with `{prefix}raw_` to avoid collisions
    /// with internal entity/edge/episode collections.
    fn raw_collection_name(&self, namespace: &str) -> String {
        format!("{}raw_{}", self.prefix, namespace)
    }

    /// Convert an arbitrary string ID to a deterministic UUID v5.
    /// Qdrant requires UUID-format point IDs, so we hash arbitrary strings
    /// into valid UUIDs using a fixed namespace. The original ID is stored
    /// in the payload as `_mnemo_id` so it can be returned in search results.
    fn string_to_uuid(id: &str) -> Uuid {
        Uuid::new_v5(&Uuid::NAMESPACE_URL, id.as_bytes())
    }

    /// Convert Qdrant payload back to serde_json::Value.
    fn payload_to_json(
        payload: &std::collections::HashMap<String, QdrantValue>,
    ) -> serde_json::Value {
        let mut map = serde_json::Map::new();
        for (k, v) in payload {
            let jv = match &v.kind {
                Some(Kind::StringValue(s)) => serde_json::Value::String(s.clone()),
                Some(Kind::DoubleValue(d)) => {
                    serde_json::json!(*d)
                }
                Some(Kind::IntegerValue(i)) => serde_json::json!(*i),
                Some(Kind::BoolValue(b)) => serde_json::Value::Bool(*b),
                Some(Kind::NullValue(_)) => serde_json::Value::Null,
                _ => serde_json::Value::String(format!("{:?}", v)),
            };
            map.insert(k.clone(), jv);
        }
        serde_json::Value::Object(map)
    }

    /// Convert a serde_json::Value object to Qdrant payload (no user_id injection).
    fn json_to_payload(extra: serde_json::Value) -> std::collections::HashMap<String, QdrantValue> {
        let mut payload = std::collections::HashMap::new();
        if let serde_json::Value::Object(map) = extra {
            for (k, v) in map {
                let qv = match v {
                    serde_json::Value::String(s) => QdrantValue {
                        kind: Some(Kind::StringValue(s)),
                    },
                    serde_json::Value::Number(n) => {
                        if let Some(f) = n.as_f64() {
                            QdrantValue {
                                kind: Some(Kind::DoubleValue(f)),
                            }
                        } else {
                            continue;
                        }
                    }
                    serde_json::Value::Bool(b) => QdrantValue {
                        kind: Some(Kind::BoolValue(b)),
                    },
                    _ => QdrantValue {
                        kind: Some(Kind::StringValue(v.to_string())),
                    },
                };
                payload.insert(k, qv);
            }
        }
        payload
    }
}

impl RawVectorStore for QdrantVectorStore {
    async fn ensure_namespace(&self, namespace: &str, dimensions: u32) -> StorageResult<()> {
        let coll_name = self.raw_collection_name(namespace);
        let exists = self
            .client
            .collection_exists(&coll_name)
            .await
            .map_err(|e| MnemoError::Qdrant(e.to_string()))?;

        if !exists {
            self.client
                .create_collection(CreateCollectionBuilder::new(&coll_name).vectors_config(
                    VectorParamsBuilder::new(dimensions as u64, Distance::Cosine),
                ))
                .await
                .map_err(|e| {
                    MnemoError::Qdrant(format!("Failed to create raw collection: {}", e))
                })?;

            tracing::info!(collection = %coll_name, dimensions = dimensions, "Created raw vector namespace");
        }

        Ok(())
    }

    async fn has_namespace(&self, namespace: &str) -> StorageResult<bool> {
        let coll_name = self.raw_collection_name(namespace);
        self.client
            .collection_exists(&coll_name)
            .await
            .map_err(|e| MnemoError::Qdrant(e.to_string()))
    }

    async fn delete_namespace(&self, namespace: &str) -> StorageResult<()> {
        let coll_name = self.raw_collection_name(namespace);
        let exists = self
            .client
            .collection_exists(&coll_name)
            .await
            .map_err(|e| MnemoError::Qdrant(e.to_string()))?;

        if exists {
            self.client
                .delete_collection(&coll_name)
                .await
                .map_err(|e| MnemoError::Qdrant(format!("Failed to delete collection: {}", e)))?;

            tracing::info!(collection = %coll_name, "Deleted raw vector namespace");
        }

        Ok(())
    }

    async fn upsert_vectors(
        &self,
        namespace: &str,
        vectors: Vec<(String, Vec<f32>, serde_json::Value)>,
    ) -> StorageResult<()> {
        // Auto-detect dimensions from the first vector and ensure namespace exists.
        if let Some((_, first_vec, _)) = vectors.first() {
            self.ensure_namespace(namespace, first_vec.len() as u32)
                .await?;
        }

        let coll_name = self.raw_collection_name(namespace);

        // Batch in chunks of 500 (matching AnythingLLM's Qdrant connector pattern).
        for chunk in vectors.chunks(500) {
            let points: Vec<PointStruct> = chunk
                .iter()
                .map(|(id, embedding, metadata)| {
                    let uuid = Self::string_to_uuid(id);
                    // Inject original ID into payload so search results can return it.
                    let mut payload = Self::json_to_payload(metadata.clone());
                    payload.insert(
                        "_mnemo_id".to_string(),
                        QdrantValue {
                            kind: Some(Kind::StringValue(id.clone())),
                        },
                    );
                    PointStruct::new(Self::uuid_to_point_id(uuid), embedding.clone(), payload)
                })
                .collect();

            self.client
                .upsert_points(UpsertPointsBuilder::new(&coll_name, points).wait(true))
                .await
                .map_err(|e| MnemoError::Qdrant(format!("Raw upsert failed: {}", e)))?;
        }

        Ok(())
    }

    async fn search_vectors(
        &self,
        namespace: &str,
        query_vector: Vec<f32>,
        top_k: u32,
        min_score: f32,
    ) -> StorageResult<Vec<VectorHit>> {
        let coll_name = self.raw_collection_name(namespace);

        let results = self
            .client
            .search_points(
                SearchPointsBuilder::new(&coll_name, query_vector, top_k as u64)
                    .score_threshold(min_score)
                    .with_payload(true),
            )
            .await
            .map_err(|e| MnemoError::Qdrant(format!("Raw search failed: {}", e)))?;

        let hits = results
            .result
            .into_iter()
            .map(|point| {
                // Prefer the original caller-supplied ID stored in `_mnemo_id`.
                let id = point
                    .payload
                    .get("_mnemo_id")
                    .and_then(|v| match &v.kind {
                        Some(Kind::StringValue(s)) => Some(s.clone()),
                        _ => None,
                    })
                    .unwrap_or_else(|| {
                        // Fallback to Qdrant point ID
                        match &point.id {
                            Some(pid) => match &pid.point_id_options {
                                Some(qdrant_client::qdrant::point_id::PointIdOptions::Uuid(s)) => {
                                    s.clone()
                                }
                                Some(qdrant_client::qdrant::point_id::PointIdOptions::Num(n)) => {
                                    n.to_string()
                                }
                                None => String::new(),
                            },
                            None => String::new(),
                        }
                    });

                // Strip the internal `_mnemo_id` field from the returned payload.
                let mut payload_map = point.payload.clone();
                payload_map.remove("_mnemo_id");
                let payload = Self::payload_to_json(&payload_map);

                VectorHit {
                    id,
                    score: point.score,
                    payload,
                }
            })
            .collect();

        Ok(hits)
    }

    async fn delete_vectors(&self, namespace: &str, ids: Vec<String>) -> StorageResult<()> {
        let coll_name = self.raw_collection_name(namespace);

        let point_ids: Vec<PointId> = ids
            .into_iter()
            .map(|id| Self::uuid_to_point_id(Self::string_to_uuid(&id)))
            .collect();

        self.client
            .delete_points(
                DeletePointsBuilder::new(&coll_name)
                    .points(PointsIdsList { ids: point_ids })
                    .wait(true),
            )
            .await
            .map_err(|e| MnemoError::Qdrant(format!("Raw delete failed: {}", e)))?;

        Ok(())
    }

    async fn count_vectors(&self, namespace: &str) -> StorageResult<u64> {
        let coll_name = self.raw_collection_name(namespace);

        let result = self
            .client
            .count(CountPointsBuilder::new(&coll_name).exact(true))
            .await
            .map_err(|e| MnemoError::Qdrant(format!("Count failed: {}", e)))?;

        Ok(result.result.map(|r| r.count).unwrap_or(0))
    }
}
