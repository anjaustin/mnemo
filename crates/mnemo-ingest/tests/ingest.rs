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
                },
                ExtractedEntity {
                    name: "Nike".into(),
                    entity_type: EntityType::Organization,
                    summary: Some("Shoe company".into()),
                },
            ],
            relationships: vec![ExtractedRelationship {
                source_name: "Kendra".into(),
                target_name: "Nike".into(),
                label: "prefers".into(),
                fact: "Kendra prefers Nike running shoes".into(),
                confidence: 0.95,
                valid_at: None,
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
                created_at: None,
            },
            session_id,
            user_id,
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
                    created_at: None,
                },
                session_id,
                user_id,
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
                created_at: None,
            },
            session_id,
            user_id,
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
                content: "Kendra switched to Nike running shoes for her marathon training."
                    .into(),
                role: Some(MessageRole::User),
                name: Some("Kendra".into()),
                metadata: serde_json::json!({}),
                created_at: None,
            },
            session_id,
            user_id,
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
                created_at: None,
            },
            session_id,
            user_id,
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
