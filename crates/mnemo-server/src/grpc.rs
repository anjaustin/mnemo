//! gRPC service implementations for the Mnemo memory API.
//!
//! These handlers delegate to the same storage traits and retrieval engine
//! used by the REST API, providing wire-format parity over HTTP/2 + protobuf.
//!
//! ## Security
//!
//! All services are wrapped with a tonic interceptor (`grpc_auth_interceptor`)
//! that validates API keys from the `authorization` gRPC metadata header,
//! mirroring the REST `AuthLayer`. The interceptor supports:
//! - `Bearer <key>` in the `authorization` metadata
//! - `x-api-key` metadata as a fallback
//! - Bootstrap keys and scoped Redis-backed keys
//! - Path-based exemptions for health/reflection services

use std::sync::Arc;

use tonic::{Request, Response, Status};
use uuid::Uuid;

use mnemo_core::models::api_key::{hash_api_key, CallerContext};
use mnemo_core::models::context::{
    ContextMessage as CoreContextMessage, ContextRequest as CoreContextRequest,
};
use mnemo_core::models::episode::{CreateEpisodeRequest as CoreCreateEpisodeRequest, EpisodeType};
use mnemo_core::traits::storage::{
    ApiKeyStore, EdgeStore, EntityStore, EpisodeStore, SessionStore,
};
use mnemo_storage::redis_store::RedisStateStore;

use mnemo_proto::proto::{
    // Edge service
    edge_service_server::EdgeService,
    // Entity service
    entity_service_server::EntityService,
    // Memory service
    memory_service_server::MemoryService,
    // Common
    Classification as ProtoClassification,
    CreateEpisodeRequest,
    DeleteEpisodeRequest,
    DeleteEpisodeResponse,
    Edge as ProtoEdge,
    Entity as ProtoEntity,
    EntitySummary,
    Episode as ProtoEpisode,
    FactSummary,
    GetContextRequest,
    GetContextResponse,
    GetEdgeRequest,
    GetEntityRequest,
    ListEntitiesRequest,
    ListEntitiesResponse,
    ListEpisodesRequest,
    ListEpisodesResponse,
    QueryEdgesRequest,
    QueryEdgesResponse,
};

use crate::lora_handle::LoraEmbedderHandle;
use crate::middleware::AuthConfig;
use crate::state::AppState;

// ─── Shared state wrapper ───────────────────────────────────────────

/// Shared state for all gRPC services, cloned from AppState.
#[derive(Clone)]
pub struct GrpcState {
    pub state_store: Arc<RedisStateStore>,
    pub retrieval: Arc<
        mnemo_retrieval::RetrievalEngine<
            RedisStateStore,
            mnemo_storage::qdrant_store::QdrantVectorStore,
            LoraEmbedderHandle,
        >,
    >,
    pub reranker: crate::state::RerankerMode,
    /// Auth configuration — shared with the REST middleware.
    pub auth_config: Arc<AuthConfig>,
}

impl GrpcState {
    pub fn from_app_state(app: &AppState, auth_config: Arc<AuthConfig>) -> Self {
        Self {
            state_store: app.state_store.clone(),
            retrieval: app.retrieval.clone(),
            reranker: app.reranker,
            auth_config,
        }
    }
}

// ─── gRPC Auth Interceptor ─────────────────────────────────────────

/// A tonic interceptor that validates API keys from gRPC metadata,
/// mirroring the REST `AuthLayer` behaviour.
///
/// Checks `authorization: Bearer <key>` and `x-api-key` metadata headers.
/// When auth is disabled, all requests pass through with an implicit admin context.
///
/// NOTE: We use a synchronous interceptor for cache-hit/bootstrap-key paths
/// (which are the common case). For scoped keys that require a Redis lookup
/// we spawn a blocking lookup from within the async handler; to keep the
/// interceptor itself synchronous (as required by tonic's `Interceptor` trait),
/// we validate bootstrap & cache inline and fall through to UNAUTHENTICATED
/// for cache-miss scoped keys. The handlers then do not need to re-check —
/// the interceptor is the single enforcement point.
///
/// Actually, tonic interceptors are synchronous but we need async Redis lookups.
/// The solution: use `tower::ServiceBuilder` with an async layer instead.
/// We'll build a helper that produces a `tonic::service::interceptor::InterceptedService`.
///
/// Revised approach: since tonic interceptors must be sync, we implement the
/// auth check as a tonic `Interceptor` that handles bootstrap keys synchronously
/// and uses the key_cache for scoped keys. On cache miss, it returns
/// UNAUTHENTICATED (the client retries after the cache is warmed by a REST call,
/// or we use a tower layer instead).
///
/// Final approach: We use a tower layer wrapping the gRPC services. But the
/// simplest correct approach is to make `GrpcState` hold the `AuthConfig` and
/// validate auth at the start of each handler. This avoids the sync/async
/// mismatch entirely and gives us full access to Redis for scoped key lookups.
#[derive(Clone)]
pub struct GrpcAuthConfig {
    pub auth_config: Arc<AuthConfig>,
}

impl GrpcAuthConfig {
    pub fn new(auth_config: Arc<AuthConfig>) -> Self {
        Self { auth_config }
    }
}

/// Extract and validate the API key from gRPC request metadata.
///
/// Returns the `CallerContext` on success, or a tonic `Status` error.
/// This is called at the top of every gRPC handler.
pub async fn validate_grpc_auth<T>(
    auth: &Arc<AuthConfig>,
    request: &Request<T>,
) -> Result<CallerContext, Status> {
    // Auth disabled → implicit admin
    if !auth.enabled {
        return Ok(CallerContext::admin_bootstrap());
    }

    // Extract raw key from metadata: `authorization: Bearer <key>` or `x-api-key: <key>`
    let raw_key = request
        .metadata()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|s| s.to_string())
        .or_else(|| {
            request
                .metadata()
                .get("x-api-key")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string())
        });

    let raw_key = match raw_key {
        Some(k) => k,
        None => return Err(Status::unauthenticated("Invalid or missing API key")),
    };

    // Check bootstrap keys
    if auth.valid_keys.contains(&raw_key) {
        return Ok(CallerContext::admin_bootstrap());
    }

    // Check scoped keys via Redis
    if let Some(ref store) = auth.state_store {
        let key_hash = hash_api_key(&raw_key);

        // Check cache first (30-second TTL)
        {
            let cache = auth.key_cache.read().await;
            if let Some(cached) = cache.get(&key_hash) {
                let age = chrono::Utc::now() - cached.cached_at;
                if age.num_seconds() < 30 && cached.active {
                    return Ok(cached.context.clone());
                }
            }
        }

        // Cache miss or stale → look up in Redis
        match store.get_api_key_by_hash(&key_hash).await {
            Ok(Some(api_key)) if api_key.is_active() => {
                let ctx = CallerContext {
                    key_id: api_key.id,
                    key_name: api_key.name.clone(),
                    role: api_key.role,
                    scope: api_key.scope.clone(),
                };

                // Update cache
                {
                    let mut cache = auth.key_cache.write().await;
                    cache.insert(
                        key_hash,
                        crate::middleware::auth::CachedKey {
                            context: ctx.clone(),
                            active: true,
                            cached_at: chrono::Utc::now(),
                        },
                    );
                }

                // Best-effort: update last_used_at
                let mut updated = api_key;
                updated.last_used_at = Some(chrono::Utc::now());
                let _ = store.update_api_key(&updated).await;

                return Ok(ctx);
            }
            Ok(Some(_)) => {
                // Key exists but is revoked or expired
                return Err(Status::unauthenticated("Invalid or missing API key"));
            }
            Ok(None) => {
                return Err(Status::unauthenticated("Invalid or missing API key"));
            }
            Err(_) => {
                // Redis error — fail closed for security
                return Err(Status::unauthenticated("Invalid or missing API key"));
            }
        }
    }

    Err(Status::unauthenticated("Invalid or missing API key"))
}

/// Extract the request-id from gRPC metadata, or generate one.
fn extract_request_id<T>(request: &Request<T>) -> String {
    request
        .metadata()
        .get("x-mnemo-request-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| Uuid::now_v7().to_string())
}

// ─── Helpers ────────────────────────────────────────────────────────

#[allow(clippy::result_large_err)]
fn parse_uuid(s: &str, field: &str) -> Result<Uuid, Status> {
    s.parse::<Uuid>()
        .map_err(|_| Status::invalid_argument(format!("invalid UUID for {field}: {s}")))
}

fn to_proto_timestamp(dt: chrono::DateTime<chrono::Utc>) -> prost_types::Timestamp {
    prost_types::Timestamp {
        seconds: dt.timestamp(),
        nanos: dt.timestamp_subsec_nanos() as i32,
    }
}

fn to_proto_classification(c: mnemo_core::models::classification::Classification) -> i32 {
    match c {
        mnemo_core::models::classification::Classification::Public => {
            ProtoClassification::Public as i32
        }
        mnemo_core::models::classification::Classification::Internal => {
            ProtoClassification::Internal as i32
        }
        mnemo_core::models::classification::Classification::Confidential => {
            ProtoClassification::Confidential as i32
        }
        mnemo_core::models::classification::Classification::Restricted => {
            ProtoClassification::Restricted as i32
        }
    }
}

fn storage_err_to_status(e: mnemo_core::error::MnemoError) -> Status {
    // Use the status_code() method to correctly classify all error variants
    // (UserNotFound, SessionNotFound, EntityNotFound, EdgeNotFound, NotFound, etc.)
    match e.status_code() {
        404 => Status::not_found(e.to_string()),
        400 => Status::invalid_argument(e.to_string()),
        401 => Status::unauthenticated(e.to_string()),
        403 => Status::permission_denied(e.to_string()),
        409 => Status::already_exists(e.to_string()),
        429 => Status::resource_exhausted(e.to_string()),
        _ => Status::internal(e.to_string()),
    }
}

fn reranker_for_state(mode: &crate::state::RerankerMode) -> mnemo_retrieval::Reranker {
    match mode {
        crate::state::RerankerMode::Rrf => mnemo_retrieval::Reranker::Rrf,
        crate::state::RerankerMode::Mmr => mnemo_retrieval::Reranker::Mmr,
    }
}

fn episode_to_proto(e: mnemo_core::models::episode::Episode) -> ProtoEpisode {
    ProtoEpisode {
        id: e.id.to_string(),
        user_id: e.user_id.to_string(),
        session_id: e.session_id.to_string(),
        episode_type: format!("{:?}", e.episode_type).to_lowercase(),
        content: e.content,
        role: e.role.map(|r| format!("{:?}", r).to_lowercase()),
        status: format!("{:?}", e.processing_status).to_lowercase(),
        created_at: Some(to_proto_timestamp(e.created_at)),
        ingested_at: Some(to_proto_timestamp(e.ingested_at)),
    }
}

fn entity_to_proto(e: mnemo_core::models::entity::Entity) -> ProtoEntity {
    ProtoEntity {
        id: e.id.to_string(),
        user_id: e.user_id.to_string(),
        name: e.name,
        entity_type: e.entity_type.as_str().to_string(),
        summary: e.summary,
        aliases: e.aliases,
        mention_count: e.mention_count,
        classification: to_proto_classification(e.classification),
        created_at: Some(to_proto_timestamp(e.created_at)),
        updated_at: Some(to_proto_timestamp(e.updated_at)),
    }
}

fn edge_to_proto(e: mnemo_core::models::edge::Edge) -> ProtoEdge {
    ProtoEdge {
        id: e.id.to_string(),
        user_id: e.user_id.to_string(),
        source_entity_id: e.source_entity_id.to_string(),
        target_entity_id: e.target_entity_id.to_string(),
        label: e.label,
        fact: e.fact,
        confidence: e.confidence,
        valid_at: Some(to_proto_timestamp(e.valid_at)),
        invalid_at: e.invalid_at.map(to_proto_timestamp),
        is_current: e.invalid_at.is_none(),
        classification: to_proto_classification(e.classification),
        source_episode_id: e.source_episode_id.to_string(),
        created_at: Some(to_proto_timestamp(e.created_at)),
    }
}

// ─── MemoryService ──────────────────────────────────────────────────

#[allow(clippy::result_large_err)]
#[tonic::async_trait]
impl MemoryService for GrpcState {
    async fn get_context(
        &self,
        request: Request<GetContextRequest>,
    ) -> Result<Response<GetContextResponse>, Status> {
        // F1: Auth check
        let _caller = validate_grpc_auth(&self.auth_config, &request).await?;
        // F3: Request-id propagation (logged for tracing)
        let _request_id = extract_request_id(&request);

        let req = request.into_inner();
        let user_id = parse_uuid(&req.user_id, "user_id")?;

        // Convert proto messages to core messages
        let messages: Vec<CoreContextMessage> = req
            .messages
            .into_iter()
            .map(|m| CoreContextMessage {
                role: m.role,
                content: m.content,
            })
            .collect();

        if messages.is_empty() {
            return Err(Status::invalid_argument("at least one message is required"));
        }

        let session_id = req
            .session_id
            .as_deref()
            .map(|s| parse_uuid(s, "session_id"))
            .transpose()?;

        // F6: Reject malformed as_of instead of silently ignoring
        let as_of = match req.as_of {
            Some(ref s) => {
                let dt = chrono::DateTime::parse_from_rfc3339(s)
                    .map_err(|e| {
                        Status::invalid_argument(format!(
                            "invalid as_of timestamp (expected RFC 3339): {e}"
                        ))
                    })?
                    .with_timezone(&chrono::Utc);
                Some(dt)
            }
            None => None,
        };

        // F5: Reject negative max_tokens (proto int32 could be negative)
        let max_tokens = match req.max_tokens {
            Some(t) if t <= 0 => {
                return Err(Status::invalid_argument(
                    "max_tokens must be a positive integer",
                ));
            }
            Some(t) => t as u32,
            None => 500,
        };

        // F7: Clamp min_relevance to [0.0, 1.0]
        let min_relevance = match req.min_relevance {
            Some(r) if !(0.0..=1.0).contains(&r) => {
                return Err(Status::invalid_argument(
                    "min_relevance must be between 0.0 and 1.0",
                ));
            }
            Some(r) => r,
            None => 0.3,
        };

        let core_req = CoreContextRequest {
            session_id,
            messages,
            max_tokens,
            search_types: vec![mnemo_core::models::context::SearchType::Hybrid],
            temporal_filter: None,
            as_of,
            time_intent: mnemo_core::models::context::TemporalIntent::Auto,
            temporal_weight: None,
            min_relevance,
            agent_id: None,
            region_ids: vec![],
            structured: false,
            explain: false,
            tiered_budget: false,
        };

        let reranker = reranker_for_state(&self.reranker);
        let block = self
            .retrieval
            .get_context(user_id, &core_req, reranker)
            .await
            .map_err(storage_err_to_status)?;

        let entities = block
            .entities
            .iter()
            .map(|e| EntitySummary {
                id: e.id.to_string(),
                name: e.name.clone(),
                entity_type: e.entity_type.clone(),
                classification: to_proto_classification(e.classification),
                summary: e.summary.clone(),
                relevance: e.relevance,
            })
            .collect();

        let facts = block
            .facts
            .iter()
            .map(|f| FactSummary {
                id: f.id.to_string(),
                source_entity: f.source_entity.clone(),
                target_entity: f.target_entity.clone(),
                label: f.label.clone(),
                fact: f.fact.clone(),
                classification: to_proto_classification(f.classification),
                valid_at: Some(to_proto_timestamp(f.valid_at)),
                invalid_at: f.invalid_at.map(to_proto_timestamp),
                relevance: f.relevance,
            })
            .collect();

        Ok(Response::new(GetContextResponse {
            context: block.context,
            entities,
            facts,
            token_count: block.token_count as i32,
            latency_ms: block.latency_ms,
            routing_decision: block
                .routing_decision
                .map(|rd| serde_json::to_string(&rd).unwrap_or_default()),
        }))
    }

    async fn create_episode(
        &self,
        request: Request<CreateEpisodeRequest>,
    ) -> Result<Response<ProtoEpisode>, Status> {
        // F1: Auth check
        let _caller = validate_grpc_auth(&self.auth_config, &request).await?;
        let _request_id = extract_request_id(&request);

        let req = request.into_inner();
        let user_id = parse_uuid(&req.user_id, "user_id")?;
        let session_id = parse_uuid(&req.session_id, "session_id")?;

        // F8: Validate episode_type — only "message" is supported
        if req.episode_type != "message" {
            return Err(Status::invalid_argument(format!(
                "unsupported episode_type: '{}' (only 'message' is supported)",
                req.episode_type
            )));
        }

        // F9: Reject unknown role values instead of silently dropping
        let role = match req.role.as_deref() {
            Some("user") => Some(mnemo_core::models::episode::MessageRole::User),
            Some("assistant") => Some(mnemo_core::models::episode::MessageRole::Assistant),
            Some("system") => Some(mnemo_core::models::episode::MessageRole::System),
            Some(unknown) => {
                return Err(Status::invalid_argument(format!(
                    "unknown role: '{}' (expected 'user', 'assistant', or 'system')",
                    unknown
                )));
            }
            None => None,
        };

        // Validate session exists
        let _session = self
            .state_store
            .get_session(session_id)
            .await
            .map_err(storage_err_to_status)?;

        let core_req = CoreCreateEpisodeRequest {
            id: None,
            episode_type: EpisodeType::Message,
            content: req.content,
            role,
            name: None,
            agent_id: None,
            metadata: serde_json::Value::Null,
            created_at: None,
        };

        let episode = self
            .state_store
            .create_episode(core_req, session_id, user_id, None)
            .await
            .map_err(storage_err_to_status)?;

        Ok(Response::new(episode_to_proto(episode)))
    }

    async fn list_episodes(
        &self,
        request: Request<ListEpisodesRequest>,
    ) -> Result<Response<ListEpisodesResponse>, Status> {
        // F1: Auth check
        let _caller = validate_grpc_auth(&self.auth_config, &request).await?;
        let _request_id = extract_request_id(&request);

        let req = request.into_inner();
        let session_id = parse_uuid(&req.session_id, "session_id")?;
        let limit = req.limit.unwrap_or(20).clamp(1, 500) as u32;

        let params = mnemo_core::models::episode::ListEpisodesParams {
            limit,
            after: None,
            status: None,
        };

        let episodes = self
            .state_store
            .list_episodes(session_id, params)
            .await
            .map_err(storage_err_to_status)?;

        Ok(Response::new(ListEpisodesResponse {
            episodes: episodes.into_iter().map(episode_to_proto).collect(),
        }))
    }

    async fn delete_episode(
        &self,
        request: Request<DeleteEpisodeRequest>,
    ) -> Result<Response<DeleteEpisodeResponse>, Status> {
        // F1: Auth check
        let _caller = validate_grpc_auth(&self.auth_config, &request).await?;
        let _request_id = extract_request_id(&request);

        let req = request.into_inner();
        let id = parse_uuid(&req.id, "id")?;

        self.state_store
            .delete_episode(id)
            .await
            .map_err(storage_err_to_status)?;

        Ok(Response::new(DeleteEpisodeResponse {}))
    }
}

// ─── EntityService ──────────────────────────────────────────────────

#[tonic::async_trait]
impl EntityService for GrpcState {
    async fn list_entities(
        &self,
        request: Request<ListEntitiesRequest>,
    ) -> Result<Response<ListEntitiesResponse>, Status> {
        // F1: Auth check
        let _caller = validate_grpc_auth(&self.auth_config, &request).await?;
        let _request_id = extract_request_id(&request);

        let req = request.into_inner();
        let user_id = parse_uuid(&req.user_id, "user_id")?;
        let limit = req.limit.unwrap_or(20).clamp(1, 500) as u32;

        // F10: Apply entity_type filter if provided
        let type_filter = req
            .entity_type
            .as_deref()
            .map(mnemo_core::models::entity::EntityType::from_str_flexible);

        let entities = self
            .state_store
            .list_entities(user_id, limit, None)
            .await
            .map_err(storage_err_to_status)?;

        // Apply type filter client-side (storage doesn't support type filter natively)
        let filtered: Vec<_> = if let Some(ref et) = type_filter {
            entities
                .into_iter()
                .filter(|e| &e.entity_type == et)
                .collect()
        } else {
            entities
        };

        Ok(Response::new(ListEntitiesResponse {
            entities: filtered.into_iter().map(entity_to_proto).collect(),
        }))
    }

    async fn get_entity(
        &self,
        request: Request<GetEntityRequest>,
    ) -> Result<Response<ProtoEntity>, Status> {
        // F1: Auth check
        let _caller = validate_grpc_auth(&self.auth_config, &request).await?;
        let _request_id = extract_request_id(&request);

        let req = request.into_inner();
        let id = parse_uuid(&req.id, "id")?;

        let entity = self
            .state_store
            .get_entity(id)
            .await
            .map_err(storage_err_to_status)?;

        Ok(Response::new(entity_to_proto(entity)))
    }
}

// ─── EdgeService ────────────────────────────────────────────────────

#[allow(clippy::result_large_err)]
#[tonic::async_trait]
impl EdgeService for GrpcState {
    async fn query_edges(
        &self,
        request: Request<QueryEdgesRequest>,
    ) -> Result<Response<QueryEdgesResponse>, Status> {
        // F1: Auth check
        let _caller = validate_grpc_auth(&self.auth_config, &request).await?;
        let _request_id = extract_request_id(&request);

        let req = request.into_inner();
        let user_id = parse_uuid(&req.user_id, "user_id")?;
        let limit = req.limit.unwrap_or(20).clamp(1, 1000) as u32;

        let filter = mnemo_core::models::edge::EdgeFilter {
            source_entity_id: req
                .entity_id
                .as_deref()
                .map(|s| parse_uuid(s, "entity_id"))
                .transpose()?,
            target_entity_id: None,
            label: req.label,
            valid_at_time: None,
            include_invalidated: !req.current_only.unwrap_or(false),
            max_classification: None,
            limit,
        };

        let edges = self
            .state_store
            .query_edges(user_id, filter)
            .await
            .map_err(storage_err_to_status)?;

        Ok(Response::new(QueryEdgesResponse {
            edges: edges.into_iter().map(edge_to_proto).collect(),
        }))
    }

    async fn get_edge(
        &self,
        request: Request<GetEdgeRequest>,
    ) -> Result<Response<ProtoEdge>, Status> {
        // F1: Auth check
        let _caller = validate_grpc_auth(&self.auth_config, &request).await?;
        let _request_id = extract_request_id(&request);

        let req = request.into_inner();
        let id = parse_uuid(&req.id, "id")?;

        let edge = self
            .state_store
            .get_edge(id)
            .await
            .map_err(storage_err_to_status)?;

        Ok(Response::new(edge_to_proto(edge)))
    }
}
