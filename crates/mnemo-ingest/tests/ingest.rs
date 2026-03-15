//! Integration tests for the ingestion pipeline.
//!
//! Uses a mock LLM provider that returns predetermined extraction results,
//! and a mock embedding provider that returns fixed-dimension vectors.

use std::sync::Arc;
use uuid::Uuid;

use mnemo_core::models::edge::ExtractedRelationship;
use mnemo_core::models::entity::{EntityType, ExtractedEntity};
use mnemo_core::models::episode::{
    CreateEpisodeRequest, EpisodeType, MessageRole, ProcessingStatus,
};
use mnemo_core::models::session::CreateSessionRequest;
use mnemo_core::models::user::CreateUserRequest;
use mnemo_core::traits::llm::*;
use mnemo_core::traits::storage::*;

use mnemo_ingest::{IngestConfig, IngestWorker};
use mnemo_storage::RedisStateStore;

/// Mock LLM that returns a fixed extraction result.
struct MockLlm {
    entities: Vec<ExtractedEntity>,
    relationships: Vec<ExtractedRelationship>,
}

impl MockLlm {
    fn new() -> Self {
        Self {
            entities: vec![
                ExtractedEntity {
                    name: "Kendra".into(),
                    entity_type: EntityType::Person,
                    summary: Some("A runner".into()),
                    classification: Default::default(),
                },
                ExtractedEntity {
                    name: "Nike".into(),
                    entity_type: EntityType::Organization,
                    summary: Some("Shoe company".into()),
                    classification: Default::default(),
                },
            ],
            relationships: vec![ExtractedRelationship {
                source_name: "Kendra".into(),
                target_name: "Nike".into(),
                label: "prefers".into(),
                fact: "Kendra prefers Nike running shoes".into(),
                confidence: 0.95,
                valid_at: None,
                classification: Default::default(),
                temporal_scope: None,
            }],
        }
    }
}

impl LlmProvider for MockLlm {
    async fn extract_entities_and_relationships(
        &self,
        _content: &str,
        _existing: &[ExtractedEntity],
    ) -> LlmResult<ExtractionResult> {
        Ok(ExtractionResult {
            entities: self.entities.clone(),
            relationships: self.relationships.clone(),
        })
    }

    async fn summarize(&self, content: &str, _max_tokens: u32) -> LlmResult<String> {
        Ok(format!("Summary: {}", &content[..content.len().min(50)]))
    }

    async fn detect_contradictions(
        &self,
        _new: &str,
        _existing: &[String],
    ) -> LlmResult<Vec<String>> {
        Ok(Vec::new())
    }

    fn provider_name(&self) -> &str {
        "mock"
    }
    fn model_name(&self) -> &str {
        "mock-v1"
    }
}

/// Mock LLM that fails N times then succeeds.
struct FailingLlm {
    inner: MockLlm,
    fail_count: std::sync::atomic::AtomicU32,
    max_failures: u32,
}

impl FailingLlm {
    fn new(max_failures: u32) -> Self {
        Self {
            inner: MockLlm::new(),
            fail_count: std::sync::atomic::AtomicU32::new(0),
            max_failures,
        }
    }
}

impl LlmProvider for FailingLlm {
    async fn extract_entities_and_relationships(
        &self,
        content: &str,
        existing: &[ExtractedEntity],
    ) -> LlmResult<ExtractionResult> {
        let count = self
            .fail_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if count < self.max_failures {
            Err(mnemo_core::error::MnemoError::LlmProvider {
                provider: "mock".into(),
                message: format!("Simulated failure #{}", count + 1),
            })
        } else {
            self.inner
                .extract_entities_and_relationships(content, existing)
                .await
        }
    }

    async fn summarize(&self, content: &str, max_tokens: u32) -> LlmResult<String> {
        self.inner.summarize(content, max_tokens).await
    }

    async fn detect_contradictions(
        &self,
        new: &str,
        existing: &[String],
    ) -> LlmResult<Vec<String>> {
        self.inner.detect_contradictions(new, existing).await
    }

    fn provider_name(&self) -> &str {
        "mock-failing"
    }
    fn model_name(&self) -> &str {
        "mock-failing-v1"
    }
}

/// Mock embedder that returns zero vectors of the right dimension.
struct MockEmbedder;

impl EmbeddingProvider for MockEmbedder {
    async fn embed(&self, _text: &str) -> LlmResult<Vec<f32>> {
        Ok(vec![0.0; 384])
    }

    async fn embed_batch(&self, texts: &[String]) -> LlmResult<Vec<Vec<f32>>> {
        Ok(texts.iter().map(|_| vec![0.0; 384]).collect())
    }

    fn dimensions(&self) -> u32 {
        384
    }
    fn provider_name(&self) -> &str {
        "mock"
    }
}

/// Mock vector store that silently accepts and ignores all operations.
struct NoopVectorStore;

impl VectorStore for NoopVectorStore {
    async fn upsert_entity_embedding(
        &self,
        _: Uuid,
        _: Uuid,
        _: Vec<f32>,
        _: serde_json::Value,
    ) -> StorageResult<()> {
        Ok(())
    }
    async fn upsert_edge_embedding(
        &self,
        _: Uuid,
        _: Uuid,
        _: Vec<f32>,
        _: serde_json::Value,
    ) -> StorageResult<()> {
        Ok(())
    }
    async fn upsert_episode_embedding(
        &self,
        _: Uuid,
        _: Uuid,
        _: Vec<f32>,
        _: serde_json::Value,
    ) -> StorageResult<()> {
        Ok(())
    }
    async fn search_entities(
        &self,
        _: Uuid,
        _: Vec<f32>,
        _: u32,
        _: f32,
    ) -> StorageResult<Vec<(Uuid, f32)>> {
        Ok(Vec::new())
    }
    async fn search_edges(
        &self,
        _: Uuid,
        _: Vec<f32>,
        _: u32,
        _: f32,
    ) -> StorageResult<Vec<(Uuid, f32)>> {
        Ok(Vec::new())
    }
    async fn search_episodes(
        &self,
        _: Uuid,
        _: Vec<f32>,
        _: u32,
        _: f32,
    ) -> StorageResult<Vec<(Uuid, f32)>> {
        Ok(Vec::new())
    }
    async fn set_entity_payload(&self, _: Uuid, _: serde_json::Value) -> StorageResult<()> {
        Ok(())
    }
    async fn set_edge_payload(&self, _: Uuid, _: serde_json::Value) -> StorageResult<()> {
        Ok(())
    }
    async fn delete_user_vectors(&self, _: Uuid) -> StorageResult<()> {
        Ok(())
    }
}

/// Mock vector store that tracks `set_*_payload` calls for testing proactive re-ranking.
struct TrackingVectorStore {
    entity_payloads: tokio::sync::RwLock<Vec<(Uuid, serde_json::Value)>>,
    edge_payloads: tokio::sync::RwLock<Vec<(Uuid, serde_json::Value)>>,
}

impl TrackingVectorStore {
    fn new() -> Self {
        Self {
            entity_payloads: tokio::sync::RwLock::new(Vec::new()),
            edge_payloads: tokio::sync::RwLock::new(Vec::new()),
        }
    }
}

impl VectorStore for TrackingVectorStore {
    async fn upsert_entity_embedding(
        &self,
        _: Uuid,
        _: Uuid,
        _: Vec<f32>,
        _: serde_json::Value,
    ) -> StorageResult<()> {
        Ok(())
    }
    async fn upsert_edge_embedding(
        &self,
        _: Uuid,
        _: Uuid,
        _: Vec<f32>,
        _: serde_json::Value,
    ) -> StorageResult<()> {
        Ok(())
    }
    async fn upsert_episode_embedding(
        &self,
        _: Uuid,
        _: Uuid,
        _: Vec<f32>,
        _: serde_json::Value,
    ) -> StorageResult<()> {
        Ok(())
    }
    async fn search_entities(
        &self,
        _: Uuid,
        _: Vec<f32>,
        _: u32,
        _: f32,
    ) -> StorageResult<Vec<(Uuid, f32)>> {
        Ok(Vec::new())
    }
    async fn search_edges(
        &self,
        _: Uuid,
        _: Vec<f32>,
        _: u32,
        _: f32,
    ) -> StorageResult<Vec<(Uuid, f32)>> {
        Ok(Vec::new())
    }
    async fn search_episodes(
        &self,
        _: Uuid,
        _: Vec<f32>,
        _: u32,
        _: f32,
    ) -> StorageResult<Vec<(Uuid, f32)>> {
        Ok(Vec::new())
    }
    async fn set_entity_payload(&self, id: Uuid, payload: serde_json::Value) -> StorageResult<()> {
        self.entity_payloads.write().await.push((id, payload));
        Ok(())
    }
    async fn set_edge_payload(&self, id: Uuid, payload: serde_json::Value) -> StorageResult<()> {
        self.edge_payloads.write().await.push((id, payload));
        Ok(())
    }
    async fn delete_user_vectors(&self, _: Uuid) -> StorageResult<()> {
        Ok(())
    }
}

async fn setup_user_session(store: &RedisStateStore) -> (Uuid, Uuid) {
    let user = store
        .create_user(CreateUserRequest {
            id: None,
            name: "Test User".into(),
            email: None,
            external_id: None,
            metadata: serde_json::json!({}),
        })
        .await
        .unwrap();

    let session = store
        .create_session(CreateSessionRequest {
            id: None,
            user_id: user.id,
            agent_id: None,
            name: None,
            metadata: serde_json::json!({}),
        })
        .await
        .unwrap();

    (user.id, session.id)
}

async fn test_store(name: &str) -> RedisStateStore {
    let url = std::env::var("MNEMO_TEST_REDIS_URL")
        .unwrap_or_else(|_| "redis://localhost:6399".to_string());
    let prefix = format!(
        "test:{}:{}:",
        name,
        Uuid::now_v7().to_string().split('-').next().unwrap()
    );
    RedisStateStore::new(&url, &prefix)
        .await
        .expect("Failed to connect to test Redis")
}

#[tokio::test]
async fn test_ingest_full_pipeline() {
    let store = Arc::new(test_store("ingest_full").await);
    let (user_id, session_id) = setup_user_session(&store).await;

    // Add episode
    let episode = store
        .create_episode(
            CreateEpisodeRequest {
                id: None,
                episode_type: EpisodeType::Message,
                content: "I switched to Nike running shoes!".into(),
                role: Some(MessageRole::User),
                name: Some("Kendra".into()),
                metadata: serde_json::json!({}),
                agent_id: None,
                created_at: None,
            },
            session_id,
            user_id,
            None,
        )
        .await
        .unwrap();

    // Create worker with mock LLM
    let worker = IngestWorker::new(
        store.clone(),
        Arc::new(NoopVectorStore),
        Arc::new(MockLlm::new()),
        Arc::new(MockEmbedder),
        IngestConfig {
            poll_interval_ms: 100,
            batch_size: 10,
            concurrency: 1,
            max_retries: 3,
            session_summary_threshold: 0, // disable in tests
            sleep_enabled: false,
            ..Default::default()
        },
    );

    // Run one poll cycle
    // (We call the private method indirectly by running the worker briefly)
    tokio::select! {
        _ = worker.run() => {},
        _ = tokio::time::sleep(std::time::Duration::from_millis(500)) => {},
    }

    // Verify episode was processed
    let processed = store.get_episode(episode.id).await.unwrap();
    assert_eq!(processed.processing_status, ProcessingStatus::Completed);
    assert!(
        !processed.entity_ids.is_empty(),
        "Should have extracted entities"
    );
    assert!(
        !processed.edge_ids.is_empty(),
        "Should have extracted edges"
    );

    // Verify entities were created
    let entities = store.list_entities(user_id, 10, None).await.unwrap();
    assert!(entities.len() >= 2, "Should have at least Kendra and Nike");

    let kendra = store.find_entity_by_name(user_id, "Kendra").await.unwrap();
    assert!(kendra.is_some());

    let nike = store.find_entity_by_name(user_id, "Nike").await.unwrap();
    assert!(nike.is_some());

    // Verify edges were created
    let kendra_id = kendra.unwrap().id;
    let outgoing = store.get_outgoing_edges(kendra_id).await.unwrap();
    assert!(!outgoing.is_empty(), "Kendra should have outgoing edges");
    assert_eq!(outgoing[0].label, "prefers");
}

#[tokio::test]
async fn test_ingest_entity_dedup_across_episodes() {
    let store = Arc::new(test_store("ingest_dedup").await);
    let (user_id, session_id) = setup_user_session(&store).await;

    let worker = IngestWorker::new(
        store.clone(),
        Arc::new(NoopVectorStore),
        Arc::new(MockLlm::new()),
        Arc::new(MockEmbedder),
        IngestConfig {
            poll_interval_ms: 100,
            batch_size: 10,
            concurrency: 1,
            max_retries: 3,
            session_summary_threshold: 0, // disable in tests
            sleep_enabled: false,
            ..Default::default()
        },
    );

    // Add two episodes that mention the same entities
    for content in &[
        "Kendra loves Nike shoes",
        "Kendra just bought more Nike gear",
    ] {
        store
            .create_episode(
                CreateEpisodeRequest {
                    id: None,
                    episode_type: EpisodeType::Message,
                    content: content.to_string(),
                    role: Some(MessageRole::User),
                    name: None,
                    metadata: serde_json::json!({}),
                    agent_id: None,
                    created_at: None,
                },
                session_id,
                user_id,
                None,
            )
            .await
            .unwrap();
    }

    // Process both
    tokio::select! {
        _ = worker.run() => {},
        _ = tokio::time::sleep(std::time::Duration::from_millis(1000)) => {},
    }

    // Should have exactly 2 entities (Kendra, Nike) — not 4
    let entities = store.list_entities(user_id, 10, None).await.unwrap();
    assert_eq!(
        entities.len(),
        2,
        "Entities should be deduplicated: got {:?}",
        entities.iter().map(|e| &e.name).collect::<Vec<_>>()
    );

    // Kendra's mention_count should be 2
    let kendra = store
        .find_entity_by_name(user_id, "Kendra")
        .await
        .unwrap()
        .unwrap();
    assert!(
        kendra.mention_count >= 2,
        "Mention count should be at least 2, got {}",
        kendra.mention_count
    );
}

#[tokio::test]
async fn test_ingest_retry_on_failure() {
    let store = Arc::new(test_store("ingest_retry").await);
    let (user_id, session_id) = setup_user_session(&store).await;

    let episode = store
        .create_episode(
            CreateEpisodeRequest {
                id: None,
                episode_type: EpisodeType::Message,
                content: "Retry test content".into(),
                role: Some(MessageRole::User),
                name: None,
                metadata: serde_json::json!({}),
                agent_id: None,
                created_at: None,
            },
            session_id,
            user_id,
            None,
        )
        .await
        .unwrap();

    // Worker with LLM that fails twice then succeeds
    let worker = IngestWorker::new(
        store.clone(),
        Arc::new(NoopVectorStore),
        Arc::new(FailingLlm::new(2)),
        Arc::new(MockEmbedder),
        IngestConfig {
            poll_interval_ms: 50,
            batch_size: 10,
            concurrency: 1,
            max_retries: 3,
            session_summary_threshold: 0, // disable in tests
            sleep_enabled: false,
            ..Default::default()
        },
    );

    // Run long enough for retries (50ms poll + backoff delays)
    tokio::select! {
        _ = worker.run() => {},
        _ = tokio::time::sleep(std::time::Duration::from_secs(5)) => {},
    }

    // Episode should eventually succeed
    let processed = store.get_episode(episode.id).await.unwrap();
    assert_eq!(
        processed.processing_status,
        ProcessingStatus::Completed,
        "Episode should succeed after retries, status: {:?}, error: {:?}",
        processed.processing_status,
        processed.processing_error
    );
}

#[tokio::test]
async fn test_progressive_session_summarization_triggers_at_threshold() {
    // After processing exactly `session_summary_threshold` episodes, the
    // ingest worker must call LLM::summarize() and persist the result into
    // session.summary.  We set threshold=1 so it fires on every episode,
    // making the assertion unconditional after a single poll cycle.
    let store = Arc::new(test_store("ingest_summary").await);
    let (user_id, session_id) = setup_user_session(&store).await;

    // Pre-condition: session.summary must be None before ingest
    let session_before = store.get_session(session_id).await.unwrap();
    assert!(
        session_before.summary.is_none(),
        "session.summary must start as None"
    );

    store
        .create_episode(
            CreateEpisodeRequest {
                id: None,
                episode_type: EpisodeType::Message,
                content: "Kendra switched to Nike running shoes for her marathon training.".into(),
                role: Some(MessageRole::User),
                name: Some("Kendra".into()),
                metadata: serde_json::json!({}),
                agent_id: None,
                created_at: None,
            },
            session_id,
            user_id,
            None,
        )
        .await
        .unwrap();

    let worker = IngestWorker::new(
        store.clone(),
        Arc::new(NoopVectorStore),
        Arc::new(MockLlm::new()),
        Arc::new(MockEmbedder),
        IngestConfig {
            poll_interval_ms: 50,
            batch_size: 10,
            concurrency: 1,
            max_retries: 3,
            // threshold=1 means summarize after every single episode
            session_summary_threshold: 1,
            sleep_enabled: false,
            ..Default::default()
        },
    );

    tokio::select! {
        _ = worker.run() => {},
        _ = tokio::time::sleep(std::time::Duration::from_millis(800)) => {},
    }

    // After processing, session.summary must be non-None and non-empty
    let session_after = store.get_session(session_id).await.unwrap();
    assert!(
        session_after.summary.is_some(),
        "session.summary must be set after threshold episodes were processed"
    );
    let summary = session_after.summary.unwrap();
    assert!(
        !summary.is_empty(),
        "session.summary must be non-empty, got: {:?}",
        summary
    );
    assert!(
        session_after.summary_tokens > 0,
        "session.summary_tokens must be > 0"
    );
}

#[tokio::test]
async fn test_progressive_summarization_disabled_when_threshold_zero() {
    // When session_summary_threshold=0, no summarization must occur
    // even after processing episodes.
    let store = Arc::new(test_store("ingest_summary_disabled").await);
    let (user_id, session_id) = setup_user_session(&store).await;

    store
        .create_episode(
            CreateEpisodeRequest {
                id: None,
                episode_type: EpisodeType::Message,
                content: "This episode should not trigger summarization.".into(),
                role: Some(MessageRole::User),
                name: None,
                metadata: serde_json::json!({}),
                agent_id: None,
                created_at: None,
            },
            session_id,
            user_id,
            None,
        )
        .await
        .unwrap();

    let worker = IngestWorker::new(
        store.clone(),
        Arc::new(NoopVectorStore),
        Arc::new(MockLlm::new()),
        Arc::new(MockEmbedder),
        IngestConfig {
            poll_interval_ms: 50,
            batch_size: 10,
            concurrency: 1,
            max_retries: 3,
            session_summary_threshold: 0, // disabled
            sleep_enabled: false,
            sleep_idle_window_seconds: 300,
        },
    );

    tokio::select! {
        _ = worker.run() => {},
        _ = tokio::time::sleep(std::time::Duration::from_millis(800)) => {},
    }

    // session.summary must remain None
    let session_after = store.get_session(session_id).await.unwrap();
    assert!(
        session_after.summary.is_none(),
        "session.summary must stay None when threshold=0, got: {:?}",
        session_after.summary
    );
}

/// Test: proactive re-ranking writes relevance scores to the vector store
/// during idle windows, covering entity scores (mention_count, recency,
/// edge density) and edge scores (confidence, corroboration, recency).
///
/// This test uses a single worker that:
/// 1. Processes an episode (populates graph with entities & edges, records user activity)
/// 2. Waits 31s for the 30s idle window to expire
/// 3. Runs another poll cycle where sleep_time_consolidation detects the idle user
///    and runs proactive_rerank, which writes relevance payloads to the TrackingVectorStore
#[tokio::test]
async fn test_proactive_rerank_writes_relevance_scores() {
    let store = Arc::new(test_store("rerank").await);
    let (user_id, session_id) = setup_user_session(&store).await;

    // Create an episode for ingestion
    store
        .create_episode(
            CreateEpisodeRequest {
                id: None,
                episode_type: EpisodeType::Message,
                content: "Kendra switched to Nike running shoes.".into(),
                role: Some(MessageRole::User),
                name: Some("Kendra".into()),
                metadata: serde_json::json!({}),
                agent_id: None,
                created_at: None,
            },
            session_id,
            user_id,
            None,
        )
        .await
        .unwrap();

    let tracking = Arc::new(TrackingVectorStore::new());
    let worker = IngestWorker::new(
        store.clone(),
        tracking.clone(),
        Arc::new(MockLlm::new()),
        Arc::new(MockEmbedder),
        IngestConfig {
            poll_interval_ms: 50,
            batch_size: 10,
            concurrency: 1,
            max_retries: 3,
            session_summary_threshold: 0,
            sleep_enabled: true, // enabled so sleep_time_consolidation runs
            sleep_idle_window_seconds: 30, // minimum (clamped to 30s)
        },
    );

    // Phase 1: Process the episode. This populates the graph AND records user activity.
    // Sleep is enabled but user just had activity, so no idle consolidation runs yet.
    tokio::select! {
        _ = worker.run() => {},
        _ = tokio::time::sleep(std::time::Duration::from_millis(600)) => {},
    }

    // Verify entities and edges were created in the state store.
    let entities = store.list_entities(user_id, 100, None).await.unwrap();
    assert!(
        !entities.is_empty(),
        "Should have created entities from ingest"
    );
    let edges = store
        .query_edges(
            user_id,
            mnemo_core::models::edge::EdgeFilter {
                include_invalidated: false,
                limit: 100,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(!edges.is_empty(), "Should have created edges from ingest");

    // No reranking payloads yet (user is not idle)
    assert!(
        tracking.entity_payloads.read().await.is_empty(),
        "No entity rerank payloads should exist before idle window expires"
    );

    // Phase 2: Wait for the idle window (30s) to expire.
    tokio::time::sleep(std::time::Duration::from_secs(31)).await;

    // Phase 3: Run another poll cycle. sleep_time_consolidation will detect
    // the idle user and run both digest generation and proactive re-ranking.
    // The worker still has the same user_activity from Phase 1, now >30s old.
    tokio::select! {
        _ = worker.run() => {},
        _ = tokio::time::sleep(std::time::Duration::from_millis(2000)) => {},
    }

    // Verify: entity payloads should have been updated
    let ep = tracking.entity_payloads.read().await;
    assert!(
        !ep.is_empty(),
        "Proactive re-ranking should have written entity relevance payloads, but got 0 updates. \
         Entities in graph: {}",
        entities.len()
    );

    // Verify each entity payload has the expected fields
    for (eid, payload) in ep.iter() {
        let score = payload.get("relevance_score").and_then(|v| v.as_f64());
        assert!(
            score.is_some(),
            "Entity {:?} payload missing 'relevance_score': {:?}",
            eid,
            payload
        );
        let s = score.unwrap();
        assert!(
            (0.0..=1.0).contains(&s),
            "Entity {:?} relevance_score out of range: {}",
            eid,
            s
        );
        assert!(
            payload.get("mention_count").is_some(),
            "Entity {:?} payload missing 'mention_count'",
            eid
        );
        assert!(
            payload.get("edge_density").is_some(),
            "Entity {:?} payload missing 'edge_density'",
            eid
        );
        assert!(
            payload.get("reranked_at").is_some(),
            "Entity {:?} payload missing 'reranked_at'",
            eid
        );
    }

    // Verify: edge payloads should have been updated
    let edp = tracking.edge_payloads.read().await;
    assert!(
        !edp.is_empty(),
        "Proactive re-ranking should have written edge relevance payloads, but got 0 updates. \
         Edges in graph: {}",
        edges.len()
    );

    for (eid, payload) in edp.iter() {
        let score = payload.get("relevance_score").and_then(|v| v.as_f64());
        assert!(
            score.is_some(),
            "Edge {:?} payload missing 'relevance_score': {:?}",
            eid,
            payload
        );
        let s = score.unwrap();
        assert!(
            (0.0..=1.0).contains(&s),
            "Edge {:?} relevance_score out of range: {}",
            eid,
            s
        );
        assert!(
            payload.get("confidence").is_some(),
            "Edge {:?} payload missing 'confidence'",
            eid
        );
        assert!(
            payload.get("corroboration_count").is_some(),
            "Edge {:?} payload missing 'corroboration_count'",
            eid
        );
        assert!(
            payload.get("reranked_at").is_some(),
            "Edge {:?} payload missing 'reranked_at'",
            eid
        );
    }
}

/// Test: proactive re-ranking is NOT triggered twice in the same idle window.
/// After a successful rerank, the user is added to `rerank_generated` and
/// should not be re-ranked again until they become active.
#[tokio::test]
async fn test_proactive_rerank_idempotent_per_idle_window() {
    let store = Arc::new(test_store("rerank_idem").await);
    let (user_id, session_id) = setup_user_session(&store).await;

    // Ingest an episode
    store
        .create_episode(
            CreateEpisodeRequest {
                id: None,
                episode_type: EpisodeType::Message,
                content: "Kendra switched to Nike running shoes.".into(),
                role: Some(MessageRole::User),
                name: Some("Kendra".into()),
                metadata: serde_json::json!({}),
                agent_id: None,
                created_at: None,
            },
            session_id,
            user_id,
            None,
        )
        .await
        .unwrap();

    let tracking = Arc::new(TrackingVectorStore::new());
    let worker = IngestWorker::new(
        store.clone(),
        tracking.clone(),
        Arc::new(MockLlm::new()),
        Arc::new(MockEmbedder),
        IngestConfig {
            poll_interval_ms: 50,
            batch_size: 10,
            concurrency: 1,
            max_retries: 3,
            session_summary_threshold: 0,
            sleep_enabled: true,
            sleep_idle_window_seconds: 30,
        },
    );

    // Process the episode (records user activity)
    tokio::select! {
        _ = worker.run() => {},
        _ = tokio::time::sleep(std::time::Duration::from_millis(600)) => {},
    }

    // Wait for idle threshold
    tokio::time::sleep(std::time::Duration::from_secs(31)).await;

    // First consolidation cycle — should trigger rerank
    tokio::select! {
        _ = worker.run() => {},
        _ = tokio::time::sleep(std::time::Duration::from_millis(2000)) => {},
    }

    let count_after_first = tracking.entity_payloads.read().await.len();
    assert!(count_after_first > 0, "First rerank should produce updates");

    // Second consolidation cycle — should NOT trigger rerank again (already generated)
    tokio::select! {
        _ = worker.run() => {},
        _ = tokio::time::sleep(std::time::Duration::from_millis(1000)) => {},
    }

    let count_after_second = tracking.entity_payloads.read().await.len();
    assert_eq!(
        count_after_first, count_after_second,
        "Second idle cycle should NOT re-trigger rerank (idempotent per idle window)"
    );
}

// ── F-09: Proactive fact_added webhook event ────────────────────────────────

/// Verify that when the ingest worker creates an edge (fact), it sends a
/// `FactAdded` event through the webhook channel — no client poll required.
#[tokio::test]
async fn test_proactive_fact_added_webhook_event() {
    use mnemo_core::models::webhook_event::IngestWebhookEvent;

    let store = Arc::new(test_store("webhook_fact_added").await);
    let (user_id, session_id) = setup_user_session(&store).await;

    // Add an episode that the MockLlm will extract "Kendra prefers Nike" from
    store
        .create_episode(
            CreateEpisodeRequest {
                id: None,
                episode_type: EpisodeType::Message,
                content: "I love Nike running shoes!".into(),
                role: Some(MessageRole::User),
                name: Some("Kendra".into()),
                metadata: serde_json::json!({"request_id": "req-fact-added-001"}),
                agent_id: None,
                created_at: None,
            },
            session_id,
            user_id,
            None,
        )
        .await
        .unwrap();

    let (tx, mut rx) = tokio::sync::mpsc::channel::<IngestWebhookEvent>(64);

    let worker = IngestWorker::new(
        store.clone(),
        Arc::new(NoopVectorStore),
        Arc::new(MockLlm::new()),
        Arc::new(MockEmbedder),
        IngestConfig {
            poll_interval_ms: 100,
            batch_size: 10,
            concurrency: 1,
            max_retries: 3,
            session_summary_threshold: 0,
            sleep_enabled: false,
            ..Default::default()
        },
    )
    .with_webhook_sender(tx);

    // Run one poll cycle
    tokio::select! {
        _ = worker.run() => {},
        _ = tokio::time::sleep(std::time::Duration::from_millis(1500)) => {},
    }

    // Collect all events from the channel
    let mut events = Vec::new();
    while let Ok(event) = rx.try_recv() {
        events.push(event);
    }

    // MockLlm extracts 1 relationship ("Kendra prefers Nike"), so we expect
    // exactly 1 FactAdded event.
    let fact_added: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, IngestWebhookEvent::FactAdded { .. }))
        .collect();

    assert!(
        !fact_added.is_empty(),
        "Should have received at least one FactAdded event, got {} total events",
        events.len()
    );

    if let IngestWebhookEvent::FactAdded {
        user_id: evt_user,
        source_entity,
        target_entity,
        label,
        fact,
        request_id,
        ..
    } = &fact_added[0]
    {
        assert_eq!(*evt_user, user_id, "event user_id must match");
        assert_eq!(source_entity, "Kendra", "source entity must be Kendra");
        assert_eq!(target_entity, "Nike", "target entity must be Nike");
        assert_eq!(label, "prefers", "label must be 'prefers'");
        assert!(
            fact.contains("Nike"),
            "fact text should mention Nike: {}",
            fact
        );
        assert_eq!(
            request_id.as_deref(),
            Some("req-fact-added-001"),
            "request_id from episode metadata should propagate"
        );
    } else {
        panic!("expected FactAdded variant");
    }
}

// ── F-09: Proactive fact_superseded webhook event ───────────────────────────

/// Verify that when the ingest worker invalidates an existing edge because
/// a newer episode introduces a conflicting fact with the same (source, target,
/// label) triple, it sends a `FactSuperseded` event through the webhook channel.
#[tokio::test]
async fn test_proactive_fact_superseded_webhook_event() {
    use mnemo_core::models::webhook_event::IngestWebhookEvent;

    let store = Arc::new(test_store("webhook_fact_superseded").await);
    let (user_id, session_id) = setup_user_session(&store).await;

    // Phase 1: Create a first episode → extracts "Kendra prefers Nike"
    store
        .create_episode(
            CreateEpisodeRequest {
                id: None,
                episode_type: EpisodeType::Message,
                content: "I love Nike running shoes!".into(),
                role: Some(MessageRole::User),
                name: Some("Kendra".into()),
                metadata: serde_json::json!({}),
                agent_id: None,
                created_at: None,
            },
            session_id,
            user_id,
            None,
        )
        .await
        .unwrap();

    // Process without webhook channel (we don't care about the first batch)
    let worker_1 = IngestWorker::new(
        store.clone(),
        Arc::new(NoopVectorStore),
        Arc::new(MockLlm::new()),
        Arc::new(MockEmbedder),
        IngestConfig {
            poll_interval_ms: 100,
            batch_size: 10,
            concurrency: 1,
            max_retries: 3,
            session_summary_threshold: 0,
            sleep_enabled: false,
            ..Default::default()
        },
    );

    tokio::select! {
        _ = worker_1.run() => {},
        _ = tokio::time::sleep(std::time::Duration::from_millis(1500)) => {},
    }

    // Verify the first edge exists
    let edges_before = store
        .query_edges(
            user_id,
            mnemo_core::models::edge::EdgeFilter {
                label: Some("prefers".into()),
                limit: 10,
                include_invalidated: false,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(
        !edges_before.is_empty(),
        "Phase 1 should have created at least one 'prefers' edge"
    );

    // Phase 2: Create a second episode → same extraction ("Kendra prefers Nike")
    // The MockLlm returns the same entities and relationships, so
    // find_conflicting_edges will find the existing edge (same source, target, label)
    // and invalidate it before creating the new one.
    store
        .create_episode(
            CreateEpisodeRequest {
                id: None,
                episode_type: EpisodeType::Message,
                content: "Actually I still prefer Nike!".into(),
                role: Some(MessageRole::User),
                name: Some("Kendra".into()),
                metadata: serde_json::json!({"request_id": "req-supersede-001"}),
                agent_id: None,
                created_at: None,
            },
            session_id,
            user_id,
            None,
        )
        .await
        .unwrap();

    let (tx, mut rx) = tokio::sync::mpsc::channel::<IngestWebhookEvent>(64);

    let worker_2 = IngestWorker::new(
        store.clone(),
        Arc::new(NoopVectorStore),
        Arc::new(MockLlm::new()),
        Arc::new(MockEmbedder),
        IngestConfig {
            poll_interval_ms: 100,
            batch_size: 10,
            concurrency: 1,
            max_retries: 3,
            session_summary_threshold: 0,
            sleep_enabled: false,
            ..Default::default()
        },
    )
    .with_webhook_sender(tx);

    tokio::select! {
        _ = worker_2.run() => {},
        _ = tokio::time::sleep(std::time::Duration::from_millis(1500)) => {},
    }

    // Collect all events
    let mut events = Vec::new();
    while let Ok(event) = rx.try_recv() {
        events.push(event);
    }

    let superseded: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, IngestWebhookEvent::FactSuperseded { .. }))
        .collect();

    assert!(
        !superseded.is_empty(),
        "Should have received at least one FactSuperseded event, got events: {:?}",
        events
            .iter()
            .map(|e| match e {
                IngestWebhookEvent::FactAdded { label, .. } => format!("FactAdded({})", label),
                IngestWebhookEvent::FactSuperseded { label, .. } =>
                    format!("FactSuperseded({})", label),
            })
            .collect::<Vec<_>>()
    );

    if let IngestWebhookEvent::FactSuperseded {
        user_id: evt_user,
        source_entity,
        target_entity,
        label,
        old_fact,
        request_id,
        ..
    } = &superseded[0]
    {
        assert_eq!(*evt_user, user_id);
        assert_eq!(source_entity, "Kendra");
        assert_eq!(target_entity, "Nike");
        assert_eq!(label, "prefers");
        assert!(
            old_fact.contains("Nike"),
            "old fact should mention Nike: {}",
            old_fact
        );
        assert_eq!(
            request_id.as_deref(),
            Some("req-supersede-001"),
            "request_id from episode metadata should propagate"
        );
    } else {
        panic!("expected FactSuperseded variant");
    }

    // Also verify we got a FactAdded for the new replacement edge
    let fact_added: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, IngestWebhookEvent::FactAdded { .. }))
        .collect();
    assert!(
        !fact_added.is_empty(),
        "Should also have a FactAdded for the replacement edge"
    );
}
