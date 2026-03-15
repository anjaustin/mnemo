//! gRPC integration tests for Mnemo's gRPC services.
//!
//! Spins up a tonic gRPC server against live Redis + Qdrant, then exercises
//! each RPC via a real tonic client. Mirrors the REST integration tests in
//! `memory_api.rs` but over protobuf/HTTP2.
#![allow(clippy::result_large_err)]

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
use mnemo_server::lora_handle::LoraEmbedderHandle;
use mnemo_server::middleware::AuthConfig;
use mnemo_server::state::{
    AppState, MetadataPrefilterConfig, RerankerMode, ServerMetrics, WebhookDeliveryConfig,
};
use mnemo_storage::{QdrantVectorStore, RedisStateStore};

use mnemo_proto::proto::{
    agent_service_client::AgentServiceClient, edge_service_client::EdgeServiceClient,
    entity_service_client::EntityServiceClient, memory_service_client::MemoryServiceClient,
    session_service_client::SessionServiceClient, user_service_client::UserServiceClient,
    AddExperienceRequest as ProtoAddExperienceRequest, ContextMessage, CreateEpisodeRequest,
    CreateSessionRequest as ProtoCreateSessionRequest, CreateUserRequest as ProtoCreateUserRequest,
    DeleteEntityRequest, DeleteEpisodeRequest, DeleteSessionRequest, DeleteUserRequest,
    GetAgentRequest, GetContextRequest, GetEdgeRequest, GetEntityRequest, GetMemoryContextRequest,
    GetSessionRequest, GetUserRequest, ListEntitiesRequest, ListEpisodesRequest,
    ListUserSessionsRequest, ListUsersRequest, PaginationRequest, PatchClassificationRequest,
    QueryEdgesRequest, RegisterAgentRequest, RememberMemoryRequest,
    UpdateAgentIdentityRequest as ProtoUpdateAgentIdentityRequest,
    UpdateSessionRequest as ProtoUpdateSessionRequest, UpdateUserRequest as ProtoUpdateUserRequest,
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
        QdrantVectorStore::new(&qdrant_url, &qdrant_prefix, 1536, None)
            .await
            .expect("Qdrant required for gRPC tests"),
    );

    let base_embedder = Arc::new(EmbedderKind::OpenAiCompat(OpenAiCompatibleEmbedder::new(
        EmbeddingConfig {
            provider: "openai".to_string(),
            api_key: None,
            model: "text-embedding-3-small".to_string(),
            base_url: None,
            dimensions: 1536,
        },
    )));
    let embedder = Arc::new(LoraEmbedderHandle::Base(base_embedder));

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
        lora_embedder: None,
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
        memory_webhook_events: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        memory_webhook_audit: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
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
        auth_config: Arc::new(mnemo_server::middleware::AuthConfig::disabled()),
    };

    (state, state_store)
}

/// Start a gRPC server on a random port with auth DISABLED.
/// Returns (address, join handle).
async fn start_grpc_server(state: &AppState) -> (String, tokio::task::JoinHandle<()>) {
    start_grpc_server_with_auth(state, Arc::new(AuthConfig::disabled())).await
}

/// Start a gRPC server on a random port with the given auth config.
/// Returns (address, join handle).
async fn start_grpc_server_with_auth(
    state: &AppState,
    auth_config: Arc<AuthConfig>,
) -> (String, tokio::task::JoinHandle<()>) {
    let grpc_state = GrpcState::from_app_state(state, auth_config);

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

    // F2: Apply same message size limits as production
    const GRPC_MAX_DECODE_SIZE: usize = 4 * 1024 * 1024;
    const GRPC_MAX_ENCODE_SIZE: usize = 16 * 1024 * 1024;

    let handle = tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(health_service)
            .add_service(reflection)
            .add_service(
                mnemo_proto::proto::memory_service_server::MemoryServiceServer::new(
                    grpc_state.clone(),
                )
                .max_decoding_message_size(GRPC_MAX_DECODE_SIZE)
                .max_encoding_message_size(GRPC_MAX_ENCODE_SIZE),
            )
            .add_service(
                mnemo_proto::proto::user_service_server::UserServiceServer::new(grpc_state.clone())
                    .max_decoding_message_size(GRPC_MAX_DECODE_SIZE)
                    .max_encoding_message_size(GRPC_MAX_ENCODE_SIZE),
            )
            .add_service(
                mnemo_proto::proto::session_service_server::SessionServiceServer::new(
                    grpc_state.clone(),
                )
                .max_decoding_message_size(GRPC_MAX_DECODE_SIZE)
                .max_encoding_message_size(GRPC_MAX_ENCODE_SIZE),
            )
            .add_service(
                mnemo_proto::proto::entity_service_server::EntityServiceServer::new(
                    grpc_state.clone(),
                )
                .max_decoding_message_size(GRPC_MAX_DECODE_SIZE)
                .max_encoding_message_size(GRPC_MAX_ENCODE_SIZE),
            )
            .add_service(
                mnemo_proto::proto::edge_service_server::EdgeServiceServer::new(grpc_state.clone())
                    .max_decoding_message_size(GRPC_MAX_DECODE_SIZE)
                    .max_encoding_message_size(GRPC_MAX_ENCODE_SIZE),
            )
            .add_service(
                mnemo_proto::proto::agent_service_server::AgentServiceServer::new(grpc_state)
                    .max_decoding_message_size(GRPC_MAX_DECODE_SIZE)
                    .max_encoding_message_size(GRPC_MAX_ENCODE_SIZE),
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
            agent_id: None,
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
            after: None,
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
            after: None,
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
            after: None,
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
            temporal_scope: None,
        },
        user_id,
        src_id,
        tgt_id,
        ep_id,
        chrono::Utc::now(),
        None,
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
            structured: None,
            explain: None,
            tiered_budget: None,
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
            structured: None,
            explain: None,
            tiered_budget: None,
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

// ─── Red-team adversarial tests ─────────────────────────────────────

// F1: gRPC auth — unauthenticated requests MUST be rejected when auth is enabled

#[tokio::test]
async fn test_grpc_auth_rejects_without_key() {
    let (state, store) = build_test_state().await;
    let auth = Arc::new(AuthConfig::with_keys(vec!["secret-key-123".to_string()]));
    let (addr, _handle) = start_grpc_server_with_auth(&state, auth).await;

    let (user_id, session_id) = seed_user_session(&store).await;

    // Connect WITHOUT any auth metadata
    let mut client = MemoryServiceClient::connect(addr).await.unwrap();

    let resp = client
        .create_episode(CreateEpisodeRequest {
            user_id: user_id.to_string(),
            session_id: session_id.to_string(),
            content: "Should be rejected".to_string(),
            episode_type: "message".to_string(),
            role: Some("user".to_string()),
        })
        .await;

    assert!(resp.is_err());
    assert_eq!(resp.unwrap_err().code(), tonic::Code::Unauthenticated);
}

#[tokio::test]
async fn test_grpc_auth_rejects_wrong_key() {
    let (state, store) = build_test_state().await;
    let auth = Arc::new(AuthConfig::with_keys(vec!["correct-key".to_string()]));
    let (addr, _handle) = start_grpc_server_with_auth(&state, auth).await;

    let (user_id, session_id) = seed_user_session(&store).await;

    let channel = tonic::transport::Channel::from_shared(addr)
        .unwrap()
        .connect()
        .await
        .unwrap();

    // Attach a WRONG key via interceptor
    let mut client =
        MemoryServiceClient::with_interceptor(channel, |mut req: tonic::Request<()>| {
            req.metadata_mut()
                .insert("authorization", "Bearer wrong-key".parse().unwrap());
            Ok(req)
        });

    let resp = client
        .create_episode(CreateEpisodeRequest {
            user_id: user_id.to_string(),
            session_id: session_id.to_string(),
            content: "Should be rejected".to_string(),
            episode_type: "message".to_string(),
            role: Some("user".to_string()),
        })
        .await;

    assert!(resp.is_err());
    assert_eq!(resp.unwrap_err().code(), tonic::Code::Unauthenticated);
}

#[tokio::test]
async fn test_grpc_auth_accepts_valid_bearer() {
    let (state, store) = build_test_state().await;
    let auth = Arc::new(AuthConfig::with_keys(vec!["valid-key-abc".to_string()]));
    let (addr, _handle) = start_grpc_server_with_auth(&state, auth).await;

    let (user_id, session_id) = seed_user_session(&store).await;

    let channel = tonic::transport::Channel::from_shared(addr)
        .unwrap()
        .connect()
        .await
        .unwrap();

    // Attach the CORRECT key
    let mut client =
        MemoryServiceClient::with_interceptor(channel, |mut req: tonic::Request<()>| {
            req.metadata_mut()
                .insert("authorization", "Bearer valid-key-abc".parse().unwrap());
            Ok(req)
        });

    let resp = client
        .create_episode(CreateEpisodeRequest {
            user_id: user_id.to_string(),
            session_id: session_id.to_string(),
            content: "Should succeed".to_string(),
            episode_type: "message".to_string(),
            role: Some("user".to_string()),
        })
        .await;

    assert!(resp.is_ok());
    let episode = resp.unwrap().into_inner();
    assert_eq!(episode.content, "Should succeed");
}

#[tokio::test]
async fn test_grpc_auth_accepts_x_api_key() {
    let (state, store) = build_test_state().await;
    let auth = Arc::new(AuthConfig::with_keys(vec!["x-key-456".to_string()]));
    let (addr, _handle) = start_grpc_server_with_auth(&state, auth).await;

    let (user_id, session_id) = seed_user_session(&store).await;

    let channel = tonic::transport::Channel::from_shared(addr)
        .unwrap()
        .connect()
        .await
        .unwrap();

    // Use x-api-key metadata instead of authorization
    let mut client =
        MemoryServiceClient::with_interceptor(channel, |mut req: tonic::Request<()>| {
            req.metadata_mut()
                .insert("x-api-key", "x-key-456".parse().unwrap());
            Ok(req)
        });

    let resp = client
        .create_episode(CreateEpisodeRequest {
            user_id: user_id.to_string(),
            session_id: session_id.to_string(),
            content: "Via x-api-key".to_string(),
            episode_type: "message".to_string(),
            role: Some("user".to_string()),
        })
        .await;

    assert!(resp.is_ok());
}

#[tokio::test]
async fn test_grpc_auth_all_services_enforced() {
    // Verify auth is enforced on ALL services, not just MemoryService
    let (state, store) = build_test_state().await;
    let auth = Arc::new(AuthConfig::with_keys(vec!["secret".to_string()]));
    let (addr, _handle) = start_grpc_server_with_auth(&state, auth).await;

    let user_id = Uuid::from_u128(7001);
    store
        .create_user(CreateUserRequest {
            id: Some(user_id),
            external_id: None,
            name: "auth-test-user".to_string(),
            email: None,
            metadata: serde_json::json!({}),
        })
        .await
        .unwrap();

    // EntityService — no key
    let mut entity_client = EntityServiceClient::connect(addr.clone()).await.unwrap();
    let resp = entity_client
        .list_entities(ListEntitiesRequest {
            user_id: user_id.to_string(),
            limit: Some(10),
            entity_type: None,
            after: None,
        })
        .await;
    assert!(resp.is_err());
    assert_eq!(resp.unwrap_err().code(), tonic::Code::Unauthenticated);

    // EdgeService — no key
    let mut edge_client = EdgeServiceClient::connect(addr).await.unwrap();
    let resp = edge_client
        .query_edges(QueryEdgesRequest {
            user_id: user_id.to_string(),
            entity_id: None,
            label: None,
            current_only: None,
            limit: Some(10),
        })
        .await;
    assert!(resp.is_err());
    assert_eq!(resp.unwrap_err().code(), tonic::Code::Unauthenticated);
}

// F5: Negative max_tokens must be rejected

#[tokio::test]
async fn test_grpc_negative_max_tokens_rejected() {
    let (state, store) = build_test_state().await;
    let (addr, _handle) = start_grpc_server(&state).await;

    let (user_id, _session_id) = seed_user_session(&store).await;

    let mut client = MemoryServiceClient::connect(addr).await.unwrap();

    let resp = client
        .get_context(GetContextRequest {
            user_id: user_id.to_string(),
            messages: vec![ContextMessage {
                role: "user".to_string(),
                content: "Hello".to_string(),
            }],
            max_tokens: Some(-1),
            session_id: None,
            as_of: None,
            min_relevance: None,
            structured: None,
            explain: None,
            tiered_budget: None,
        })
        .await;

    assert!(resp.is_err());
    let status = resp.unwrap_err();
    assert_eq!(status.code(), tonic::Code::InvalidArgument);
    assert!(status.message().contains("max_tokens"));
}

#[tokio::test]
async fn test_grpc_zero_max_tokens_rejected() {
    let (state, store) = build_test_state().await;
    let (addr, _handle) = start_grpc_server(&state).await;

    let (user_id, _session_id) = seed_user_session(&store).await;

    let mut client = MemoryServiceClient::connect(addr).await.unwrap();

    let resp = client
        .get_context(GetContextRequest {
            user_id: user_id.to_string(),
            messages: vec![ContextMessage {
                role: "user".to_string(),
                content: "Hello".to_string(),
            }],
            max_tokens: Some(0),
            session_id: None,
            as_of: None,
            min_relevance: None,
            structured: None,
            explain: None,
            tiered_budget: None,
        })
        .await;

    assert!(resp.is_err());
    assert_eq!(resp.unwrap_err().code(), tonic::Code::InvalidArgument);
}

// F6: Malformed as_of timestamp must return an error

#[tokio::test]
async fn test_grpc_malformed_as_of_rejected() {
    let (state, store) = build_test_state().await;
    let (addr, _handle) = start_grpc_server(&state).await;

    let (user_id, _session_id) = seed_user_session(&store).await;

    let mut client = MemoryServiceClient::connect(addr).await.unwrap();

    let resp = client
        .get_context(GetContextRequest {
            user_id: user_id.to_string(),
            messages: vec![ContextMessage {
                role: "user".to_string(),
                content: "Hello".to_string(),
            }],
            max_tokens: None,
            session_id: None,
            as_of: Some("not-a-timestamp".to_string()),
            min_relevance: None,
            structured: None,
            explain: None,
            tiered_budget: None,
        })
        .await;

    assert!(resp.is_err());
    let status = resp.unwrap_err();
    assert_eq!(status.code(), tonic::Code::InvalidArgument);
    assert!(status.message().contains("as_of"));
}

// F7: min_relevance out of [0.0, 1.0] range must be rejected

#[tokio::test]
async fn test_grpc_min_relevance_negative_rejected() {
    let (state, store) = build_test_state().await;
    let (addr, _handle) = start_grpc_server(&state).await;

    let (user_id, _session_id) = seed_user_session(&store).await;

    let mut client = MemoryServiceClient::connect(addr).await.unwrap();

    let resp = client
        .get_context(GetContextRequest {
            user_id: user_id.to_string(),
            messages: vec![ContextMessage {
                role: "user".to_string(),
                content: "Hello".to_string(),
            }],
            max_tokens: None,
            session_id: None,
            as_of: None,
            min_relevance: Some(-0.5),
            structured: None,
            explain: None,
            tiered_budget: None,
        })
        .await;

    assert!(resp.is_err());
    let status = resp.unwrap_err();
    assert_eq!(status.code(), tonic::Code::InvalidArgument);
    assert!(status.message().contains("min_relevance"));
}

#[tokio::test]
async fn test_grpc_min_relevance_above_one_rejected() {
    let (state, store) = build_test_state().await;
    let (addr, _handle) = start_grpc_server(&state).await;

    let (user_id, _session_id) = seed_user_session(&store).await;

    let mut client = MemoryServiceClient::connect(addr).await.unwrap();

    let resp = client
        .get_context(GetContextRequest {
            user_id: user_id.to_string(),
            messages: vec![ContextMessage {
                role: "user".to_string(),
                content: "Hello".to_string(),
            }],
            max_tokens: None,
            session_id: None,
            as_of: None,
            min_relevance: Some(1.5),
            structured: None,
            explain: None,
            tiered_budget: None,
        })
        .await;

    assert!(resp.is_err());
    assert_eq!(resp.unwrap_err().code(), tonic::Code::InvalidArgument);
}

// F8: Invalid episode_type must be rejected

#[tokio::test]
async fn test_grpc_invalid_episode_type_rejected() {
    let (state, store) = build_test_state().await;
    let (addr, _handle) = start_grpc_server(&state).await;

    let (user_id, session_id) = seed_user_session(&store).await;

    let mut client = MemoryServiceClient::connect(addr).await.unwrap();

    let resp = client
        .create_episode(CreateEpisodeRequest {
            user_id: user_id.to_string(),
            session_id: session_id.to_string(),
            content: "Hello".to_string(),
            episode_type: "event".to_string(), // Not "message"
            role: Some("user".to_string()),
        })
        .await;

    assert!(resp.is_err());
    let status = resp.unwrap_err();
    assert_eq!(status.code(), tonic::Code::InvalidArgument);
    assert!(status.message().contains("episode_type"));
}

// F9: Unknown role must be rejected

#[tokio::test]
async fn test_grpc_unknown_role_rejected() {
    let (state, store) = build_test_state().await;
    let (addr, _handle) = start_grpc_server(&state).await;

    let (user_id, session_id) = seed_user_session(&store).await;

    let mut client = MemoryServiceClient::connect(addr).await.unwrap();

    let resp = client
        .create_episode(CreateEpisodeRequest {
            user_id: user_id.to_string(),
            session_id: session_id.to_string(),
            content: "Hello".to_string(),
            episode_type: "message".to_string(),
            role: Some("moderator".to_string()), // Invalid role
        })
        .await;

    assert!(resp.is_err());
    let status = resp.unwrap_err();
    assert_eq!(status.code(), tonic::Code::InvalidArgument);
    assert!(status.message().contains("role"));
}

// F10: entity_type filter works

#[tokio::test]
async fn test_grpc_entity_type_filter() {
    let (state, store) = build_test_state().await;
    let (addr, _handle) = start_grpc_server(&state).await;

    let user_id = Uuid::from_u128(8001);
    store
        .create_user(CreateUserRequest {
            id: Some(user_id),
            external_id: None,
            name: "filter-test-user".to_string(),
            email: None,
            metadata: serde_json::json!({}),
        })
        .await
        .unwrap();

    // Seed a Person and a Product entity
    let person = Entity::from_extraction(
        &ExtractedEntity {
            name: "Alice".to_string(),
            entity_type: EntityType::Person,
            summary: Some("A person".to_string()),
            classification: Classification::default(),
        },
        user_id,
        Uuid::from_u128(200),
    );
    let product = Entity::from_extraction(
        &ExtractedEntity {
            name: "Widget".to_string(),
            entity_type: EntityType::Product,
            summary: Some("A product".to_string()),
            classification: Classification::default(),
        },
        user_id,
        Uuid::from_u128(201),
    );
    store.create_entity(person).await.unwrap();
    store.create_entity(product).await.unwrap();

    let mut client = EntityServiceClient::connect(addr).await.unwrap();

    // Filter by "person" — should return only Alice
    let resp = client
        .list_entities(ListEntitiesRequest {
            user_id: user_id.to_string(),
            limit: Some(50),
            entity_type: Some("person".to_string()),
            after: None,
        })
        .await
        .unwrap();

    let entities = resp.into_inner().entities;
    assert_eq!(entities.len(), 1);
    assert_eq!(entities[0].name, "Alice");
    assert_eq!(entities[0].entity_type, "person");

    // Filter by "product" — should return only Widget
    let resp = client
        .list_entities(ListEntitiesRequest {
            user_id: user_id.to_string(),
            limit: Some(50),
            entity_type: Some("product".to_string()),
            after: None,
        })
        .await
        .unwrap();

    let entities = resp.into_inner().entities;
    assert_eq!(entities.len(), 1);
    assert_eq!(entities[0].name, "Widget");

    // No filter — should return both
    let resp = client
        .list_entities(ListEntitiesRequest {
            user_id: user_id.to_string(),
            limit: Some(50),
            entity_type: None,
            after: None,
        })
        .await
        .unwrap();

    let entities = resp.into_inner().entities;
    assert_eq!(entities.len(), 2);
}

// ═══════════════════════════════════════════════════════════════════
// UserService tests
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_grpc_user_create_and_get() {
    let (app, _store) = build_test_state().await;
    let (addr, _handle) = start_grpc_server(&app).await;
    let mut client = UserServiceClient::connect(addr).await.unwrap();

    // Create
    let resp = client
        .create_user(ProtoCreateUserRequest {
            id: None,
            external_id: Some("ext_grpc_001".to_string()),
            name: "Grpc Alice".to_string(),
            email: Some("alice@grpc.test".to_string()),
            metadata: None,
        })
        .await
        .unwrap()
        .into_inner();

    assert!(!resp.id.is_empty());
    assert_eq!(resp.name, "Grpc Alice");
    assert_eq!(resp.external_id.as_deref(), Some("ext_grpc_001"));
    assert_eq!(resp.email.as_deref(), Some("alice@grpc.test"));

    let user_id = resp.id.clone();

    // GetUser
    let got = client
        .get_user(GetUserRequest {
            id: user_id.clone(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(got.id, user_id);
    assert_eq!(got.name, "Grpc Alice");

    // GetUserByExternalId
    let got2 = client
        .get_user_by_external_id(mnemo_proto::proto::GetUserByExternalIdRequest {
            external_id: "ext_grpc_001".to_string(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(got2.id, user_id);
}

#[tokio::test]
async fn test_grpc_user_update() {
    let (app, _store) = build_test_state().await;
    let (addr, _handle) = start_grpc_server(&app).await;
    let mut client = UserServiceClient::connect(addr).await.unwrap();

    let user = client
        .create_user(ProtoCreateUserRequest {
            id: None,
            external_id: None,
            name: "UpdateMe".to_string(),
            email: None,
            metadata: None,
        })
        .await
        .unwrap()
        .into_inner();

    let updated = client
        .update_user(ProtoUpdateUserRequest {
            id: user.id.clone(),
            name: Some("UpdatedName".to_string()),
            email: Some("updated@grpc.test".to_string()),
            external_id: None,
            metadata: None,
        })
        .await
        .unwrap()
        .into_inner();

    assert_eq!(updated.name, "UpdatedName");
    assert_eq!(updated.email.as_deref(), Some("updated@grpc.test"));
}

#[tokio::test]
async fn test_grpc_user_delete() {
    let (app, _store) = build_test_state().await;
    let (addr, _handle) = start_grpc_server(&app).await;
    let mut client = UserServiceClient::connect(addr).await.unwrap();

    let user = client
        .create_user(ProtoCreateUserRequest {
            id: None,
            external_id: None,
            name: "DeleteMe".to_string(),
            email: None,
            metadata: None,
        })
        .await
        .unwrap()
        .into_inner();

    let del = client
        .delete_user(DeleteUserRequest {
            id: user.id.clone(),
        })
        .await
        .unwrap()
        .into_inner();
    assert!(del.deleted);

    // Should now return not found
    let err = client
        .get_user(GetUserRequest { id: user.id })
        .await
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::NotFound);
}

#[tokio::test]
async fn test_grpc_user_list() {
    let (app, _store) = build_test_state().await;
    let (addr, _handle) = start_grpc_server(&app).await;
    let mut client = UserServiceClient::connect(addr).await.unwrap();

    // Create 3 users
    for i in 0..3 {
        client
            .create_user(ProtoCreateUserRequest {
                id: None,
                external_id: None,
                name: format!("ListUser{i}"),
                email: None,
                metadata: None,
            })
            .await
            .unwrap();
    }

    let resp = client
        .list_users(ListUsersRequest {
            pagination: Some(PaginationRequest {
                limit: Some(100),
                after: None,
            }),
        })
        .await
        .unwrap()
        .into_inner();

    assert!(resp.users.len() >= 3);
}

// ═══════════════════════════════════════════════════════════════════
// SessionService tests
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_grpc_session_create_and_get() {
    let (app, store) = build_test_state().await;
    let (addr, _handle) = start_grpc_server(&app).await;
    let mut client = SessionServiceClient::connect(addr).await.unwrap();

    // Create a user first
    let user = store
        .create_user(CreateUserRequest {
            id: None,
            external_id: None,
            name: "SessionUser".to_string(),
            email: None,
            metadata: serde_json::json!({}),
        })
        .await
        .unwrap();

    // Create session via gRPC
    let session = client
        .create_session(ProtoCreateSessionRequest {
            id: None,
            user_id: user.id.to_string(),
            agent_id: Some("test-agent".to_string()),
            name: Some("Test Session".to_string()),
            metadata: None,
        })
        .await
        .unwrap()
        .into_inner();

    assert!(!session.id.is_empty());
    assert_eq!(session.user_id, user.id.to_string());
    assert_eq!(session.name.as_deref(), Some("Test Session"));
    assert_eq!(session.agent_id.as_deref(), Some("test-agent"));
    assert_eq!(session.episode_count, 0);

    let session_id = session.id.clone();

    // GetSession
    let got = client
        .get_session(GetSessionRequest {
            id: session_id.clone(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(got.id, session_id);
}

#[tokio::test]
async fn test_grpc_session_update() {
    let (app, store) = build_test_state().await;
    let (addr, _handle) = start_grpc_server(&app).await;
    let mut client = SessionServiceClient::connect(addr).await.unwrap();

    let user = store
        .create_user(CreateUserRequest {
            id: None,
            external_id: None,
            name: "SessionUpdateUser".to_string(),
            email: None,
            metadata: serde_json::json!({}),
        })
        .await
        .unwrap();

    let session = client
        .create_session(ProtoCreateSessionRequest {
            id: None,
            user_id: user.id.to_string(),
            agent_id: None,
            name: Some("Before".to_string()),
            metadata: None,
        })
        .await
        .unwrap()
        .into_inner();

    let updated = client
        .update_session(ProtoUpdateSessionRequest {
            id: session.id.clone(),
            name: Some("After".to_string()),
            metadata: None,
        })
        .await
        .unwrap()
        .into_inner();

    assert_eq!(updated.name.as_deref(), Some("After"));
}

#[tokio::test]
async fn test_grpc_session_delete() {
    let (app, store) = build_test_state().await;
    let (addr, _handle) = start_grpc_server(&app).await;
    let mut client = SessionServiceClient::connect(addr).await.unwrap();

    let user = store
        .create_user(CreateUserRequest {
            id: None,
            external_id: None,
            name: "SessionDeleteUser".to_string(),
            email: None,
            metadata: serde_json::json!({}),
        })
        .await
        .unwrap();

    let session = client
        .create_session(ProtoCreateSessionRequest {
            id: None,
            user_id: user.id.to_string(),
            agent_id: None,
            name: None,
            metadata: None,
        })
        .await
        .unwrap()
        .into_inner();

    let del = client
        .delete_session(DeleteSessionRequest {
            id: session.id.clone(),
        })
        .await
        .unwrap()
        .into_inner();
    assert!(del.deleted);

    let err = client
        .get_session(GetSessionRequest { id: session.id })
        .await
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::NotFound);
}

#[tokio::test]
async fn test_grpc_list_user_sessions() {
    let (app, store) = build_test_state().await;
    let (addr, _handle) = start_grpc_server(&app).await;
    let mut client = SessionServiceClient::connect(addr).await.unwrap();

    let user = store
        .create_user(CreateUserRequest {
            id: None,
            external_id: None,
            name: "ListSessionUser".to_string(),
            email: None,
            metadata: serde_json::json!({}),
        })
        .await
        .unwrap();

    // Create 2 sessions
    for i in 0..2 {
        client
            .create_session(ProtoCreateSessionRequest {
                id: None,
                user_id: user.id.to_string(),
                agent_id: None,
                name: Some(format!("session-{i}")),
                metadata: None,
            })
            .await
            .unwrap();
    }

    let resp = client
        .list_user_sessions(ListUserSessionsRequest {
            user_id: user.id.to_string(),
            pagination: Some(PaginationRequest {
                limit: Some(50),
                after: None,
            }),
        })
        .await
        .unwrap()
        .into_inner();

    assert_eq!(resp.sessions.len(), 2);
    assert_eq!(resp.count, 2);
}

// ═══════════════════════════════════════════════════════════════════
// Extended EntityService / EdgeService tests
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_grpc_delete_entity() {
    let (app, store) = build_test_state().await;
    let (addr, _handle) = start_grpc_server(&app).await;
    let mut client = EntityServiceClient::connect(addr).await.unwrap();

    let user = store
        .create_user(CreateUserRequest {
            id: None,
            external_id: None,
            name: "DeleteEntityUser".to_string(),
            email: None,
            metadata: serde_json::json!({}),
        })
        .await
        .unwrap();

    let entity = Entity::from_extraction(
        &ExtractedEntity {
            name: "ToDelete".into(),
            entity_type: EntityType::Concept,
            summary: None,
            classification: Default::default(),
        },
        user.id,
        Uuid::now_v7(),
    );
    let created = store.create_entity(entity).await.unwrap();

    let del = client
        .delete_entity(DeleteEntityRequest {
            id: created.id.to_string(),
        })
        .await
        .unwrap()
        .into_inner();
    assert!(del.deleted);

    let err = client
        .get_entity(GetEntityRequest {
            id: created.id.to_string(),
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::NotFound);
}

#[tokio::test]
async fn test_grpc_patch_entity_classification() {
    let (app, store) = build_test_state().await;
    let (addr, _handle) = start_grpc_server(&app).await;
    let mut client = EntityServiceClient::connect(addr).await.unwrap();

    let user = store
        .create_user(CreateUserRequest {
            id: None,
            external_id: None,
            name: "PatchClassUser".to_string(),
            email: None,
            metadata: serde_json::json!({}),
        })
        .await
        .unwrap();

    let entity = Entity::from_extraction(
        &ExtractedEntity {
            name: "PatchMe".into(),
            entity_type: EntityType::Person,
            summary: None,
            classification: Classification::Internal,
        },
        user.id,
        Uuid::now_v7(),
    );
    let created = store.create_entity(entity).await.unwrap();

    let patched = client
        .patch_entity_classification(PatchClassificationRequest {
            id: created.id.to_string(),
            classification: mnemo_proto::proto::Classification::Confidential as i32,
        })
        .await
        .unwrap()
        .into_inner();

    assert_eq!(
        patched.classification,
        mnemo_proto::proto::Classification::Confidential as i32
    );
}

#[tokio::test]
async fn test_grpc_delete_edge() {
    let (app, store) = build_test_state().await;
    let (addr, _handle) = start_grpc_server(&app).await;
    let mut client = EdgeServiceClient::connect(addr).await.unwrap();

    let user = store
        .create_user(CreateUserRequest {
            id: None,
            external_id: None,
            name: "DeleteEdgeUser".to_string(),
            email: None,
            metadata: serde_json::json!({}),
        })
        .await
        .unwrap();

    let src = Entity::from_extraction(
        &ExtractedEntity {
            name: "SrcE".into(),
            entity_type: EntityType::Person,
            summary: None,
            classification: Default::default(),
        },
        user.id,
        Uuid::now_v7(),
    );
    let tgt = Entity::from_extraction(
        &ExtractedEntity {
            name: "TgtE".into(),
            entity_type: EntityType::Organization,
            summary: None,
            classification: Default::default(),
        },
        user.id,
        Uuid::now_v7(),
    );
    let src = store.create_entity(src).await.unwrap();
    let tgt = store.create_entity(tgt).await.unwrap();

    let edge = Edge::from_extraction(
        &ExtractedRelationship {
            source_name: "SrcE".into(),
            target_name: "TgtE".into(),
            label: "works_at".into(),
            fact: "SrcE works at TgtE".into(),
            confidence: 0.9,
            valid_at: None,
            classification: Default::default(),
            temporal_scope: None,
        },
        user.id,
        src.id,
        tgt.id,
        Uuid::now_v7(),
        chrono::Utc::now(),
        None,
    );
    let edge = store.create_edge(edge).await.unwrap();

    let del = client
        .delete_edge(mnemo_proto::proto::DeleteEdgeRequest {
            id: edge.id.to_string(),
        })
        .await
        .unwrap()
        .into_inner();
    assert!(del.deleted);
}

// ═══════════════════════════════════════════════════════════════════
// MemoryService extensions — RememberMemory
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_grpc_remember_memory() {
    let (app, _store) = build_test_state().await;
    let (addr, _handle) = start_grpc_server(&app).await;
    let mut client = MemoryServiceClient::connect(addr).await.unwrap();

    let resp = client
        .remember_memory(RememberMemoryRequest {
            user: "grpc-remember-user".to_string(),
            text: "I prefer dark mode in my IDE".to_string(),
            session: Some("preferences".to_string()),
            role: Some("user".to_string()),
        })
        .await
        .unwrap()
        .into_inner();

    assert!(resp.ok);
    assert!(!resp.user_id.is_empty());
    assert!(!resp.session_id.is_empty());
    assert!(!resp.episode_id.is_empty());
}

#[tokio::test]
async fn test_grpc_remember_memory_auto_creates_user() {
    let (app, _store) = build_test_state().await;
    let (addr, _handle) = start_grpc_server(&app).await;
    let mut client = MemoryServiceClient::connect(addr).await.unwrap();

    // First call — user does not exist
    let r1 = client
        .remember_memory(RememberMemoryRequest {
            user: "brand-new-grpc-user-xyz".to_string(),
            text: "Hello from gRPC".to_string(),
            session: None,
            role: None,
        })
        .await
        .unwrap()
        .into_inner();

    assert!(r1.ok);
    let user_id = r1.user_id.clone();

    // Second call — same user, should reuse
    let r2 = client
        .remember_memory(RememberMemoryRequest {
            user: "brand-new-grpc-user-xyz".to_string(),
            text: "Second message".to_string(),
            session: None,
            role: None,
        })
        .await
        .unwrap()
        .into_inner();

    assert_eq!(r2.user_id, user_id);
}

// ═══════════════════════════════════════════════════════════════════
// AgentService tests
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_grpc_agent_register_and_get() {
    let (app, _store) = build_test_state().await;
    let (addr, _handle) = start_grpc_server(&app).await;
    let mut client = AgentServiceClient::connect(addr).await.unwrap();

    let resp = client
        .register_agent(RegisterAgentRequest {
            agent_id: "grpc-test-agent".to_string(),
            core: None,
        })
        .await
        .unwrap()
        .into_inner();

    assert_eq!(resp.agent_id, "grpc-test-agent");
    assert_eq!(resp.version, 1);

    // GetAgent
    let got = client
        .get_agent(GetAgentRequest {
            agent_id: "grpc-test-agent".to_string(),
        })
        .await
        .unwrap()
        .into_inner();

    assert_eq!(got.agent_id, "grpc-test-agent");
}

#[tokio::test]
async fn test_grpc_agent_update_identity() {
    let (app, _store) = build_test_state().await;
    let (addr, _handle) = start_grpc_server(&app).await;
    let mut client = AgentServiceClient::connect(addr).await.unwrap();

    client
        .register_agent(RegisterAgentRequest {
            agent_id: "grpc-identity-agent".to_string(),
            core: None,
        })
        .await
        .unwrap();

    // Build a Struct with { "role": "support-agent" }
    let mut fields = std::collections::BTreeMap::new();
    fields.insert(
        "role".to_string(),
        prost_types::Value {
            kind: Some(prost_types::value::Kind::StringValue(
                "support-agent".to_string(),
            )),
        },
    );
    let core = prost_types::Struct { fields };

    let updated = client
        .update_agent_identity(ProtoUpdateAgentIdentityRequest {
            agent_id: "grpc-identity-agent".to_string(),
            core: Some(core),
        })
        .await
        .unwrap()
        .into_inner();

    assert_eq!(updated.agent_id, "grpc-identity-agent");
    assert!(updated.version >= 1);
}

#[tokio::test]
async fn test_grpc_agent_delete() {
    let (app, _store) = build_test_state().await;
    let (addr, _handle) = start_grpc_server(&app).await;
    let mut client = AgentServiceClient::connect(addr).await.unwrap();

    client
        .register_agent(RegisterAgentRequest {
            agent_id: "grpc-delete-agent".to_string(),
            core: None,
        })
        .await
        .unwrap();

    let del = client
        .delete_agent(mnemo_proto::proto::DeleteAgentRequest {
            agent_id: "grpc-delete-agent".to_string(),
        })
        .await
        .unwrap()
        .into_inner();
    assert!(del.deleted);

    let err = client
        .get_agent(GetAgentRequest {
            agent_id: "grpc-delete-agent".to_string(),
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::NotFound);
}

#[tokio::test]
async fn test_grpc_agent_add_experience() {
    let (app, _store) = build_test_state().await;
    let (addr, _handle) = start_grpc_server(&app).await;
    let mut client = AgentServiceClient::connect(addr).await.unwrap();

    client
        .register_agent(RegisterAgentRequest {
            agent_id: "grpc-exp-agent".to_string(),
            core: None,
        })
        .await
        .unwrap();

    let event = client
        .add_experience(ProtoAddExperienceRequest {
            agent_id: "grpc-exp-agent".to_string(),
            user_id: None,
            session_id: None,
            category: "test_category".to_string(),
            signal: "user was satisfied with response".to_string(),
            confidence: 0.9,
            weight: 1.0,
            decay_half_life_days: 30,
            evidence_episode_ids: vec![],
        })
        .await
        .unwrap()
        .into_inner();

    assert_eq!(event.agent_id, "grpc-exp-agent");
    assert_eq!(event.category, "test_category");
    assert!(!event.id.is_empty());
}

// ═══════════════════════════════════════════════════════════════════
// Red-team: Role enforcement (P0-1)
// ═══════════════════════════════════════════════════════════════════

/// RT-1: Mutating RPCs must reject requests without any auth key.
#[tokio::test]
async fn test_grpc_rt_unauthenticated_write_rejected() {
    let (app, _store) = build_test_state().await;
    // Start server with auth ENABLED (one bootstrap key)
    let auth = Arc::new(AuthConfig::with_keys(vec!["valid-admin-key".to_string()]));
    let (addr, _handle) = start_grpc_server_with_auth(&app, auth).await;
    let mut client = MemoryServiceClient::connect(addr).await.unwrap();

    // No metadata key → should fail with Unauthenticated
    let err = client
        .remember_memory(RememberMemoryRequest {
            user: "should-fail".to_string(),
            text: "hello".to_string(),
            session: None,
            role: None,
        })
        .await
        .unwrap_err();
    assert_eq!(
        err.code(),
        tonic::Code::Unauthenticated,
        "unauthenticated write should fail: {err}"
    );
}

/// RT-2: Admin RPCs (UserService) must reject non-admin bootstrap callers.
/// Note: bootstrap keys are always Admin; this test verifies auth is active.
#[tokio::test]
async fn test_grpc_rt_admin_ops_require_auth() {
    let (app, _store) = build_test_state().await;
    let auth = Arc::new(AuthConfig::with_keys(vec!["admin-key".to_string()]));
    let (addr, _handle) = start_grpc_server_with_auth(&app, auth).await;
    let mut client = UserServiceClient::connect(addr).await.unwrap();

    // No key → Unauthenticated
    let err = client
        .create_user(ProtoCreateUserRequest {
            id: None,
            external_id: None,
            name: "should-fail".to_string(),
            email: None,
            metadata: None,
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::Unauthenticated);

    // Wrong key → Unauthenticated
    let mut req = tonic::Request::new(ProtoCreateUserRequest {
        id: None,
        external_id: None,
        name: "should-also-fail".to_string(),
        email: None,
        metadata: None,
    });
    req.metadata_mut()
        .insert("authorization", "Bearer wrong-key".parse().unwrap());
    let err = client.create_user(req).await.unwrap_err();
    assert_eq!(err.code(), tonic::Code::Unauthenticated);

    // Correct admin key → succeeds
    let mut req = tonic::Request::new(ProtoCreateUserRequest {
        id: None,
        external_id: None,
        name: "created-by-admin".to_string(),
        email: None,
        metadata: None,
    });
    req.metadata_mut()
        .insert("authorization", "Bearer admin-key".parse().unwrap());
    let resp = client.create_user(req).await.unwrap().into_inner();
    assert_eq!(resp.name, "created-by-admin");
}

/// RT-3: DeleteUser requires Admin — without key is rejected.
#[tokio::test]
async fn test_grpc_rt_delete_user_requires_admin() {
    let (app, store) = build_test_state().await;
    let auth = Arc::new(AuthConfig::with_keys(vec!["admin-only-key".to_string()]));
    let (addr, _handle) = start_grpc_server_with_auth(&app, auth).await;

    // Create user via store directly
    let user = store
        .create_user(CreateUserRequest {
            id: None,
            external_id: None,
            name: "ToDelete".to_string(),
            email: None,
            metadata: serde_json::json!({}),
        })
        .await
        .unwrap();

    let mut client = UserServiceClient::connect(addr.clone()).await.unwrap();

    // No key → Unauthenticated
    let err = client
        .delete_user(DeleteUserRequest {
            id: user.id.to_string(),
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::Unauthenticated);

    // With admin key → succeeds
    let mut client2 = UserServiceClient::connect(addr).await.unwrap();
    let mut req = tonic::Request::new(DeleteUserRequest {
        id: user.id.to_string(),
    });
    req.metadata_mut()
        .insert("authorization", "Bearer admin-only-key".parse().unwrap());
    let resp = client2.delete_user(req).await.unwrap().into_inner();
    assert!(resp.deleted);
}

// ═══════════════════════════════════════════════════════════════════
// Red-team: Ownership checks (P0-2, P1-6)
// ═══════════════════════════════════════════════════════════════════

/// RT-4: CreateEpisode must reject cross-user session injection.
/// User A's session_id cannot be used to inject episodes under User B's user_id.
#[tokio::test]
async fn test_grpc_rt_create_episode_cross_user_rejected() {
    let (app, store) = build_test_state().await;
    let (addr, _handle) = start_grpc_server(&app).await;
    let mut client = MemoryServiceClient::connect(addr).await.unwrap();

    // Create User A and their session
    let user_a = store
        .create_user(CreateUserRequest {
            id: None,
            external_id: None,
            name: "UserA".to_string(),
            email: None,
            metadata: serde_json::json!({}),
        })
        .await
        .unwrap();
    let session_a = store
        .create_session(mnemo_core::models::session::CreateSessionRequest {
            id: None,
            user_id: user_a.id,
            agent_id: None,
            name: Some("session-a".to_string()),
            metadata: serde_json::json!({}),
        })
        .await
        .unwrap();

    // Create User B
    let user_b = store
        .create_user(CreateUserRequest {
            id: None,
            external_id: None,
            name: "UserB".to_string(),
            email: None,
            metadata: serde_json::json!({}),
        })
        .await
        .unwrap();

    // Try to inject episode into User A's session using User B's user_id
    let err = client
        .create_episode(CreateEpisodeRequest {
            user_id: user_b.id.to_string(),       // ← User B
            session_id: session_a.id.to_string(), // ← User A's session
            content: "injected content".to_string(),
            episode_type: "message".to_string(),
            role: None,
        })
        .await
        .unwrap_err();

    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "cross-user session injection should be rejected: {err}"
    );
}

/// RT-5: CreateEpisode with matching user_id and session_id succeeds.
#[tokio::test]
async fn test_grpc_rt_create_episode_same_user_succeeds() {
    let (app, store) = build_test_state().await;
    let (addr, _handle) = start_grpc_server(&app).await;
    let mut client = MemoryServiceClient::connect(addr).await.unwrap();

    let user = store
        .create_user(CreateUserRequest {
            id: None,
            external_id: None,
            name: "EpisodeOwner".to_string(),
            email: None,
            metadata: serde_json::json!({}),
        })
        .await
        .unwrap();
    let session = store
        .create_session(mnemo_core::models::session::CreateSessionRequest {
            id: None,
            user_id: user.id,
            agent_id: None,
            name: None,
            metadata: serde_json::json!({}),
        })
        .await
        .unwrap();

    let episode = client
        .create_episode(CreateEpisodeRequest {
            user_id: user.id.to_string(),
            session_id: session.id.to_string(),
            content: "valid episode".to_string(),
            episode_type: "message".to_string(),
            role: Some("user".to_string()),
        })
        .await
        .unwrap()
        .into_inner();

    assert!(!episode.id.is_empty());
    assert_eq!(episode.user_id, user.id.to_string());
}

// ═══════════════════════════════════════════════════════════════════
// Red-team: Input validation (P1-3)
// ═══════════════════════════════════════════════════════════════════

/// RT-6: RememberMemory must reject oversized user identifiers.
#[tokio::test]
async fn test_grpc_rt_remember_memory_user_too_long() {
    let (app, _store) = build_test_state().await;
    let (addr, _handle) = start_grpc_server(&app).await;
    let mut client = MemoryServiceClient::connect(addr).await.unwrap();

    let long_user = "x".repeat(257); // exceeds MAX_USER_IDENTIFIER_LEN=256
    let err = client
        .remember_memory(RememberMemoryRequest {
            user: long_user,
            text: "hello".to_string(),
            session: None,
            role: None,
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
    assert!(err.message().contains("too long") || err.message().contains("max"));
}

/// RT-7: RememberMemory must reject oversized text payloads.
#[tokio::test]
async fn test_grpc_rt_remember_memory_text_too_long() {
    let (app, _store) = build_test_state().await;
    let (addr, _handle) = start_grpc_server(&app).await;
    let mut client = MemoryServiceClient::connect(addr).await.unwrap();

    let long_text = "y".repeat(32_769); // exceeds MAX_TEXT_LEN=32768
    let err = client
        .remember_memory(RememberMemoryRequest {
            user: "valid-user".to_string(),
            text: long_text,
            session: None,
            role: None,
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
    assert!(err.message().contains("too long") || err.message().contains("max"));
}

// ═══════════════════════════════════════════════════════════════════
// Red-team: Proto contract (P1-7, P3-18)
// ═══════════════════════════════════════════════════════════════════

/// RT-8: PatchClassification with UNSPECIFIED (0) must return InvalidArgument.
#[tokio::test]
async fn test_grpc_rt_patch_classification_unspecified_rejected() {
    let (app, store) = build_test_state().await;
    let (addr, _handle) = start_grpc_server(&app).await;
    let mut client = EntityServiceClient::connect(addr).await.unwrap();

    let user = store
        .create_user(CreateUserRequest {
            id: None,
            external_id: None,
            name: "PatchRejectUser".to_string(),
            email: None,
            metadata: serde_json::json!({}),
        })
        .await
        .unwrap();

    let entity = Entity::from_extraction(
        &ExtractedEntity {
            name: "PatchTarget".into(),
            entity_type: EntityType::Concept,
            summary: None,
            classification: Default::default(),
        },
        user.id,
        Uuid::now_v7(),
    );
    let created = store.create_entity(entity).await.unwrap();

    // Send CLASSIFICATION_UNSPECIFIED = 0 (proto3 default)
    let err = client
        .patch_entity_classification(PatchClassificationRequest {
            id: created.id.to_string(),
            classification: 0, // Unspecified
        })
        .await
        .unwrap_err();
    assert_eq!(
        err.code(),
        tonic::Code::InvalidArgument,
        "UNSPECIFIED classification should be rejected: {err}"
    );
}

/// RT-9: GetMemoryContext with historical_strict contract but no as_of must return InvalidArgument.
#[tokio::test]
async fn test_grpc_rt_historical_strict_requires_as_of() {
    let (app, _store) = build_test_state().await;
    let (addr, _handle) = start_grpc_server(&app).await;
    let mut client = MemoryServiceClient::connect(addr).await.unwrap();

    let err = client
        .get_memory_context(GetMemoryContextRequest {
            user: "some-user".to_string(),
            query: "what happened".to_string(),
            session: None,
            max_tokens: None,
            min_relevance: None,
            time_intent: None,
            as_of: None, // ← missing
            temporal_weight: None,
            mode: None,
            contract: Some("historical_strict".to_string()), // ← requires as_of
            retrieval_policy: None,
            include_narrative: None,
            goal: None,
            view: None,
        })
        .await
        .unwrap_err();
    assert_eq!(
        err.code(),
        tonic::Code::InvalidArgument,
        "historical_strict without as_of should be rejected: {err}"
    );
}
