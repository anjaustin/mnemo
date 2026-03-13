//! gRPC integration tests for Mnemo's MemoryService, EntityService, and EdgeService.
//!
//! Spins up a tonic gRPC server against live Redis + Qdrant, then exercises
//! each RPC via a real tonic client. Mirrors the REST integration tests in
//! `memory_api.rs` but over protobuf/HTTP2.

use std::sync::Arc;

use uuid::Uuid;

use mnemo_core::models::classification::Classification;
use mnemo_core::models::edge::{Edge, ExtractedRelationship};
use mnemo_core::models::entity::{Entity, EntityType, ExtractedEntity};
use mnemo_core::models::session::CreateSessionRequest;
use mnemo_core::models::user::CreateUserRequest;
use mnemo_core::traits::fulltext::FullTextStore;
use mnemo_core::traits::llm::EmbeddingConfig;
use mnemo_core::traits::storage::{EdgeStore, EntityStore, SessionStore, UserStore};
use mnemo_graph::GraphEngine;
use mnemo_llm::{EmbedderKind, OpenAiCompatibleEmbedder};
use mnemo_retrieval::RetrievalEngine;
use mnemo_server::grpc::GrpcState;
use mnemo_server::state::{
    AppState, MetadataPrefilterConfig, RerankerMode, ServerMetrics, WebhookDeliveryConfig,
};
use mnemo_storage::{QdrantVectorStore, RedisStateStore};

use mnemo_proto::proto::{
    edge_service_client::EdgeServiceClient,
    entity_service_client::EntityServiceClient,
    memory_service_client::MemoryServiceClient,
    ContextMessage, CreateEpisodeRequest, DeleteEpisodeRequest, GetContextRequest,
    GetEdgeRequest, GetEntityRequest, ListEpisodesRequest, ListEntitiesRequest,
    QueryEdgesRequest,
};

// ─── Helpers ────────────────────────────────────────────────────────

/// Build an AppState connected to real Redis + Qdrant (test-prefixed).
async fn build_test_state() -> (AppState, Arc<RedisStateStore>) {
    let redis_url = std::env::var("MNEMO_TEST_REDIS_URL")
        .unwrap_or_else(|_| "redis://localhost:6379".to_string());
    let qdrant_url = std::env::var("MNEMO_TEST_QDRANT_URL")
        .unwrap_or_else(|_| "http://localhost:6334".to_string());

    let uid = Uuid::now_v7();
    let redis_prefix = format!("grpc_test:{}:", uid);
    let qdrant_prefix =
        std::env::var("MNEMO_TEST_QDRANT_PREFIX").unwrap_or_else(|_| "mnemo_".to_string());

    let state_store = Arc::new(
        RedisStateStore::new(&redis_url, &redis_prefix)
            .await
            .expect("Redis required for gRPC tests"),
    );
    state_store.ensure_indexes().await.unwrap();

    let vector_store = Arc::new(
        QdrantVectorStore::new(&qdrant_url, &qdrant_prefix, 1536)
            .await
            .expect("Qdrant required for gRPC tests"),
    );

    let embedder = Arc::new(EmbedderKind::OpenAiCompat(OpenAiCompatibleEmbedder::new(
        EmbeddingConfig {
            provider: "openai".to_string(),
            api_key: None,
            model: "text-embedding-3-small".to_string(),
            base_url: None,
            dimensions: 1536,
        },
    )));

    let retrieval = Arc::new(RetrievalEngine::new(
        state_store.clone(),
        vector_store.clone(),
        embedder,
    ));
    let graph = Arc::new(GraphEngine::new(state_store.clone()));

    let state = AppState {
        state_store: state_store.clone(),
        vector_store,
        retrieval,
        graph,
        llm: None,
        metadata_prefilter: MetadataPrefilterConfig {
            enabled: false,
            scan_limit: 400,
            relax_if_empty: false,
        },
        reranker: RerankerMode::Rrf,
        import_jobs: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        import_idempotency: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        memory_webhooks: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        memory_webhook_events: Arc::new(tokio::sync::RwLock::new(
            std::collections::HashMap::new(),
        )),
        memory_webhook_audit: Arc::new(tokio::sync::RwLock::new(
            std::collections::HashMap::new(),
        )),
        user_policies: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        governance_audit: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        webhook_runtime: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        webhook_delivery: WebhookDeliveryConfig {
            enabled: false,
            max_attempts: 3,
            base_backoff_ms: 20,
            request_timeout_ms: 150,
            max_events_per_webhook: 1000,
            rate_limit_per_minute: 120,
            circuit_breaker_threshold: 5,
            circuit_breaker_cooldown_ms: 200,
            persistence_enabled: false,
        },
        webhook_http: Arc::new(reqwest::Client::new()),
        webhook_redis: None,
        webhook_redis_prefix: "grpc_test:webhooks".to_string(),
        metrics: Arc::new(ServerMetrics::default()),
        llm_spans: Arc::new(tokio::sync::RwLock::new(std::collections::VecDeque::new())),
        memory_digests: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        require_tls: false,
        audit_signing_secret: None,
        compression_config: mnemo_retrieval::compression::CompressionConfig::default(),
        compression_stats: Arc::new(mnemo_retrieval::compression::CompressionStats::default()),
        embedding_dimensions: 384,
        hyperbolic_config: mnemo_retrieval::hyperbolic::HyperbolicConfig::default(),
        pipeline_metrics: Arc::new(mnemo_ingest::dag::PipelineMetrics::default()),
        sync_status: Arc::new(tokio::sync::RwLock::new(
            mnemo_core::sync::SyncStatus::disabled(),
        )),
    };

    (state, state_store)
}

/// Start a gRPC server on a random port and return (address, join handle).
async fn start_grpc_server(state: &AppState) -> (String, tokio::task::JoinHandle<()>) {
    let grpc_state = GrpcState::from_app_state(state);

    let (mut health_reporter, health_service) = tonic_health::server::health_reporter();
    health_reporter
        .set_serving::<mnemo_proto::proto::memory_service_server::MemoryServiceServer<GrpcState>>()
        .await;

    let reflection = tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(mnemo_proto::FILE_DESCRIPTOR_SET)
        .build_v1()
        .expect("reflection build");

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let addr_str = format!("http://{addr}");

    let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);

    let handle = tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(health_service)
            .add_service(reflection)
            .add_service(
                mnemo_proto::proto::memory_service_server::MemoryServiceServer::new(
                    grpc_state.clone(),
                ),
            )
            .add_service(
                mnemo_proto::proto::entity_service_server::EntityServiceServer::new(
                    grpc_state.clone(),
                ),
            )
            .add_service(
                mnemo_proto::proto::edge_service_server::EdgeServiceServer::new(grpc_state),
            )
            .serve_with_incoming(incoming)
            .await
            .unwrap();
    });

    // Give the server a moment to bind
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    (addr_str, handle)
}

/// Create a user + session via the store, returning (user_id, session_id).
async fn seed_user_session(store: &RedisStateStore) -> (Uuid, Uuid) {
    let user_id = Uuid::from_u128(1001);
    let user = store
        .create_user(CreateUserRequest {
            id: Some(user_id),
            external_id: None,
            name: "grpc-test-user".to_string(),
            email: None,
            metadata: serde_json::json!({}),
        })
        .await
        .unwrap();

    let session_id = Uuid::from_u128(2001);
    let session = store
        .create_session(CreateSessionRequest {
            id: Some(session_id),
            user_id: user.id,
            name: None,
            metadata: serde_json::json!({}),
        })
        .await
        .unwrap();

    (user.id, session.id)
}

// ─── Tests ──────────────────────────────────────────────────────────

#[tokio::test]
async fn test_grpc_create_and_list_episodes() {
    let (state, store) = build_test_state().await;
    let (addr, _handle) = start_grpc_server(&state).await;

    let (user_id, session_id) = seed_user_session(&store).await;

    let mut client = MemoryServiceClient::connect(addr).await.unwrap();

    // Create an episode
    let resp = client
        .create_episode(CreateEpisodeRequest {
            user_id: user_id.to_string(),
            session_id: session_id.to_string(),
            content: "I just bought new Nike running shoes".to_string(),
            episode_type: "message".to_string(),
            role: Some("user".to_string()),
        })
        .await
        .unwrap();

    let episode = resp.into_inner();
    assert_eq!(episode.user_id, user_id.to_string());
    assert_eq!(episode.session_id, session_id.to_string());
    assert_eq!(episode.content, "I just bought new Nike running shoes");
    assert_eq!(episode.role, Some("user".to_string()));
    assert_eq!(episode.status, "pending");
    assert!(!episode.id.is_empty());

    // Create a second episode
    client
        .create_episode(CreateEpisodeRequest {
            user_id: user_id.to_string(),
            session_id: session_id.to_string(),
            content: "They feel great for running".to_string(),
            episode_type: "message".to_string(),
            role: Some("user".to_string()),
        })
        .await
        .unwrap();

    // List episodes
    let list_resp = client
        .list_episodes(ListEpisodesRequest {
            user_id: user_id.to_string(),
            session_id: session_id.to_string(),
            limit: None,
        })
        .await
        .unwrap();

    let episodes = list_resp.into_inner().episodes;
    assert_eq!(episodes.len(), 2);
}

#[tokio::test]
async fn test_grpc_delete_episode() {
    let (state, store) = build_test_state().await;
    let (addr, _handle) = start_grpc_server(&state).await;

    let (user_id, session_id) = seed_user_session(&store).await;

    let mut client = MemoryServiceClient::connect(addr).await.unwrap();

    // Create
    let resp = client
        .create_episode(CreateEpisodeRequest {
            user_id: user_id.to_string(),
            session_id: session_id.to_string(),
            content: "To be deleted".to_string(),
            episode_type: "message".to_string(),
            role: Some("user".to_string()),
        })
        .await
        .unwrap();
    let ep_id = resp.into_inner().id;

    // Delete
    client
        .delete_episode(DeleteEpisodeRequest { id: ep_id.clone() })
        .await
        .unwrap();

    // List should be empty
    let list_resp = client
        .list_episodes(ListEpisodesRequest {
            user_id: user_id.to_string(),
            session_id: session_id.to_string(),
            limit: None,
        })
        .await
        .unwrap();
    assert_eq!(list_resp.into_inner().episodes.len(), 0);
}

#[tokio::test]
async fn test_grpc_create_episode_invalid_session() {
    let (state, _store) = build_test_state().await;
    let (addr, _handle) = start_grpc_server(&state).await;

    let mut client = MemoryServiceClient::connect(addr).await.unwrap();

    // Non-existent session
    let resp = client
        .create_episode(CreateEpisodeRequest {
            user_id: Uuid::from_u128(9999).to_string(),
            session_id: Uuid::from_u128(9999).to_string(),
            content: "Hello".to_string(),
            episode_type: "message".to_string(),
            role: None,
        })
        .await;

    assert!(resp.is_err());
    let status = resp.unwrap_err();
    assert_eq!(status.code(), tonic::Code::NotFound);
}

#[tokio::test]
async fn test_grpc_create_episode_invalid_uuid() {
    let (state, _store) = build_test_state().await;
    let (addr, _handle) = start_grpc_server(&state).await;

    let mut client = MemoryServiceClient::connect(addr).await.unwrap();

    let resp = client
        .create_episode(CreateEpisodeRequest {
            user_id: "not-a-uuid".to_string(),
            session_id: "also-not".to_string(),
            content: "Hello".to_string(),
            episode_type: "message".to_string(),
            role: None,
        })
        .await;

    assert!(resp.is_err());
    let status = resp.unwrap_err();
    assert_eq!(status.code(), tonic::Code::InvalidArgument);
}

#[tokio::test]
async fn test_grpc_list_entities() {
    let (state, store) = build_test_state().await;
    let (addr, _handle) = start_grpc_server(&state).await;

    let user_id = Uuid::from_u128(3001);
    store
        .create_user(CreateUserRequest {
            id: Some(user_id),
            external_id: None,
            name: "entity-test-user".to_string(),
            email: None,
            metadata: serde_json::json!({}),
        })
        .await
        .unwrap();

    // Seed entities
    let entity1 = Entity::from_extraction(
        &ExtractedEntity {
            name: "Nike".to_string(),
            entity_type: EntityType::Product,
            summary: Some("Athletic shoe brand".to_string()),
            classification: Classification::default(),
        },
        user_id,
        Uuid::from_u128(100),
    );
    let entity2 = Entity::from_extraction(
        &ExtractedEntity {
            name: "Kendra".to_string(),
            entity_type: EntityType::Person,
            summary: Some("A customer".to_string()),
            classification: Classification::default(),
        },
        user_id,
        Uuid::from_u128(101),
    );
    let e1_id = entity1.id;
    store.create_entity(entity1).await.unwrap();
    store.create_entity(entity2).await.unwrap();

    let mut client = EntityServiceClient::connect(addr).await.unwrap();

    // List all entities for user
    let resp = client
        .list_entities(ListEntitiesRequest {
            user_id: user_id.to_string(),
            limit: Some(10),
            entity_type: None,
        })
        .await
        .unwrap();

    let entities = resp.into_inner().entities;
    assert_eq!(entities.len(), 2);

    // Get single entity
    let resp = client
        .get_entity(GetEntityRequest {
            id: e1_id.to_string(),
        })
        .await
        .unwrap();

    let entity = resp.into_inner();
    assert_eq!(entity.name, "Nike");
    assert_eq!(entity.entity_type, "product");
    assert_eq!(entity.summary, Some("Athletic shoe brand".to_string()));
}

#[tokio::test]
async fn test_grpc_get_entity_not_found() {
    let (state, _store) = build_test_state().await;
    let (addr, _handle) = start_grpc_server(&state).await;

    let mut client = EntityServiceClient::connect(addr).await.unwrap();

    let resp = client
        .get_entity(GetEntityRequest {
            id: Uuid::from_u128(77777).to_string(),
        })
        .await;

    assert!(resp.is_err());
    assert_eq!(resp.unwrap_err().code(), tonic::Code::NotFound);
}

#[tokio::test]
async fn test_grpc_query_edges() {
    let (state, store) = build_test_state().await;
    let (addr, _handle) = start_grpc_server(&state).await;

    let user_id = Uuid::from_u128(4001);
    store
        .create_user(CreateUserRequest {
            id: Some(user_id),
            external_id: None,
            name: "edge-test-user".to_string(),
            email: None,
            metadata: serde_json::json!({}),
        })
        .await
        .unwrap();

    // Create entities + edges
    let src_id = Uuid::from_u128(5001);
    let tgt_id = Uuid::from_u128(5002);
    let ep_id = Uuid::from_u128(5003);

    let mut src = Entity::from_extraction(
        &ExtractedEntity {
            name: "Kendra".to_string(),
            entity_type: EntityType::Person,
            summary: None,
            classification: Classification::default(),
        },
        user_id,
        ep_id,
    );
    src.id = src_id;
    store.create_entity(src).await.unwrap();

    let mut tgt = Entity::from_extraction(
        &ExtractedEntity {
            name: "Nike".to_string(),
            entity_type: EntityType::Product,
            summary: None,
            classification: Classification::default(),
        },
        user_id,
        ep_id,
    );
    tgt.id = tgt_id;
    store.create_entity(tgt).await.unwrap();

    let edge = Edge::from_extraction(
        &ExtractedRelationship {
            source_name: "Kendra".to_string(),
            target_name: "Nike".to_string(),
            label: "loves".to_string(),
            fact: "Kendra loves Nike shoes".to_string(),
            confidence: 0.95,
            valid_at: None,
            classification: Classification::default(),
        },
        user_id,
        src_id,
        tgt_id,
        ep_id,
        chrono::Utc::now(),
    );
    let edge_id = edge.id;
    store.create_edge(edge).await.unwrap();

    let mut client = EdgeServiceClient::connect(addr).await.unwrap();

    // Query edges
    let resp = client
        .query_edges(QueryEdgesRequest {
            user_id: user_id.to_string(),
            entity_id: Some(src_id.to_string()),
            label: None,
            current_only: None,
            limit: Some(10),
        })
        .await
        .unwrap();

    let edges = resp.into_inner().edges;
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].label, "loves");
    assert_eq!(edges[0].fact, "Kendra loves Nike shoes");
    assert!(edges[0].is_current);
    assert!((edges[0].confidence - 0.95).abs() < 0.01);

    // Get single edge
    let resp = client
        .get_edge(GetEdgeRequest {
            id: edge_id.to_string(),
        })
        .await
        .unwrap();

    let edge = resp.into_inner();
    assert_eq!(edge.label, "loves");
    assert_eq!(edge.source_entity_id, src_id.to_string());
    assert_eq!(edge.target_entity_id, tgt_id.to_string());
}

#[tokio::test]
async fn test_grpc_get_edge_not_found() {
    let (state, _store) = build_test_state().await;
    let (addr, _handle) = start_grpc_server(&state).await;

    let mut client = EdgeServiceClient::connect(addr).await.unwrap();

    let resp = client
        .get_edge(GetEdgeRequest {
            id: Uuid::from_u128(88888).to_string(),
        })
        .await;

    assert!(resp.is_err());
    assert_eq!(resp.unwrap_err().code(), tonic::Code::NotFound);
}

#[tokio::test]
async fn test_grpc_get_context_requires_messages() {
    let (state, _store) = build_test_state().await;
    let (addr, _handle) = start_grpc_server(&state).await;

    let mut client = MemoryServiceClient::connect(addr).await.unwrap();

    // Empty messages should fail
    let resp = client
        .get_context(GetContextRequest {
            user_id: Uuid::from_u128(1).to_string(),
            messages: vec![],
            max_tokens: None,
            session_id: None,
            as_of: None,
            min_relevance: None,
        })
        .await;

    assert!(resp.is_err());
    assert_eq!(resp.unwrap_err().code(), tonic::Code::InvalidArgument);
}

#[tokio::test]
async fn test_grpc_get_context_basic() {
    let (state, store) = build_test_state().await;
    let (addr, _handle) = start_grpc_server(&state).await;

    let (_user_id, _session_id) = seed_user_session(&store).await;

    let mut client = MemoryServiceClient::connect(addr).await.unwrap();

    // GetContext with messages — even with no data, should return an empty context block
    let resp = client
        .get_context(GetContextRequest {
            user_id: _user_id.to_string(),
            messages: vec![ContextMessage {
                role: "user".to_string(),
                content: "What shoes does Kendra like?".to_string(),
            }],
            max_tokens: Some(500),
            session_id: None,
            as_of: None,
            min_relevance: None,
        })
        .await
        .unwrap();

    let ctx = resp.into_inner();
    // With no data seeded, context should be empty but the call should succeed
    assert!(ctx.token_count >= 0);
}

#[tokio::test]
async fn test_grpc_health_check() {
    let (state, _store) = build_test_state().await;
    let (addr, _handle) = start_grpc_server(&state).await;

    let channel = tonic::transport::Channel::from_shared(addr)
        .unwrap()
        .connect()
        .await
        .unwrap();

    let mut client = tonic_health::pb::health_client::HealthClient::new(channel);

    let resp = client
        .check(tonic_health::pb::HealthCheckRequest {
            service: "mnemo.v1.MemoryService".to_string(),
        })
        .await
        .unwrap();

    assert_eq!(
        resp.into_inner().status,
        tonic_health::pb::health_check_response::ServingStatus::Serving as i32,
    );
}
