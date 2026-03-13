//! gRPC service implementations for the Mnemo memory API.
//!
//! These handlers delegate to the same storage traits and retrieval engine
//! used by the REST API, providing wire-format parity over HTTP/2 + protobuf.

use std::sync::Arc;

use tonic::{Request, Response, Status};
use uuid::Uuid;

use mnemo_core::models::context::{
    ContextMessage as CoreContextMessage, ContextRequest as CoreContextRequest,
};
use mnemo_core::models::episode::{CreateEpisodeRequest as CoreCreateEpisodeRequest, EpisodeType};
use mnemo_core::traits::storage::{EdgeStore, EntityStore, EpisodeStore, SessionStore};
use mnemo_storage::redis_store::RedisStateStore;

use mnemo_proto::proto::{
    // Memory service
    memory_service_server::MemoryService,
    EntitySummary, FactSummary, GetContextRequest,
    GetContextResponse,
    CreateEpisodeRequest, Episode as ProtoEpisode,
    ListEpisodesRequest, ListEpisodesResponse,
    DeleteEpisodeRequest, DeleteEpisodeResponse,
    // Entity service
    entity_service_server::EntityService,
    ListEntitiesRequest, ListEntitiesResponse,
    GetEntityRequest, Entity as ProtoEntity,
    // Edge service
    edge_service_server::EdgeService,
    QueryEdgesRequest, QueryEdgesResponse,
    GetEdgeRequest, Edge as ProtoEdge,
    // Common
    Classification as ProtoClassification,
};

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
            mnemo_llm::EmbedderKind,
        >,
    >,
    pub reranker: crate::state::RerankerMode,
}

impl GrpcState {
    pub fn from_app_state(app: &AppState) -> Self {
        Self {
            state_store: app.state_store.clone(),
            retrieval: app.retrieval.clone(),
            reranker: app.reranker.clone(),
        }
    }
}

// ─── Helpers ────────────────────────────────────────────────────────

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

fn reranker_for_state(
    mode: &crate::state::RerankerMode,
) -> mnemo_retrieval::Reranker {
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

#[tonic::async_trait]
impl MemoryService for GrpcState {
    async fn get_context(
        &self,
        request: Request<GetContextRequest>,
    ) -> Result<Response<GetContextResponse>, Status> {
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
            return Err(Status::invalid_argument(
                "at least one message is required",
            ));
        }

        let session_id = req
            .session_id
            .as_deref()
            .map(|s| parse_uuid(s, "session_id"))
            .transpose()?;

        let as_of = req.as_of.and_then(|s| {
            chrono::DateTime::parse_from_rfc3339(&s)
                .ok()
                .map(|dt| dt.with_timezone(&chrono::Utc))
        });

        let core_req = CoreContextRequest {
            session_id,
            messages,
            max_tokens: req.max_tokens.map(|t| t as u32).unwrap_or(500),
            search_types: vec![mnemo_core::models::context::SearchType::Hybrid],
            temporal_filter: None,
            as_of,
            time_intent: mnemo_core::models::context::TemporalIntent::Auto,
            temporal_weight: None,
            min_relevance: req.min_relevance.unwrap_or(0.3),
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
            routing_decision: block.routing_decision.map(|rd| serde_json::to_string(&rd).unwrap_or_default()),
        }))
    }

    async fn create_episode(
        &self,
        request: Request<CreateEpisodeRequest>,
    ) -> Result<Response<ProtoEpisode>, Status> {
        let req = request.into_inner();
        let user_id = parse_uuid(&req.user_id, "user_id")?;
        let session_id = parse_uuid(&req.session_id, "session_id")?;

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
            role: req.role.and_then(|r| {
                match r.to_lowercase().as_str() {
                    "user" => Some(mnemo_core::models::episode::MessageRole::User),
                    "assistant" => Some(mnemo_core::models::episode::MessageRole::Assistant),
                    "system" => Some(mnemo_core::models::episode::MessageRole::System),
                    _ => None,
                }
            }),
            name: None,
            metadata: serde_json::Value::Null,
            created_at: None,
        };

        let episode = self
            .state_store
            .create_episode(core_req, session_id, user_id)
            .await
            .map_err(storage_err_to_status)?;

        Ok(Response::new(episode_to_proto(episode)))
    }

    async fn list_episodes(
        &self,
        request: Request<ListEpisodesRequest>,
    ) -> Result<Response<ListEpisodesResponse>, Status> {
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
        let req = request.into_inner();
        let user_id = parse_uuid(&req.user_id, "user_id")?;
        let limit = req.limit.unwrap_or(20).clamp(1, 500) as u32;

        let entities = self
            .state_store
            .list_entities(user_id, limit, None)
            .await
            .map_err(storage_err_to_status)?;

        Ok(Response::new(ListEntitiesResponse {
            entities: entities.into_iter().map(entity_to_proto).collect(),
        }))
    }

    async fn get_entity(
        &self,
        request: Request<GetEntityRequest>,
    ) -> Result<Response<ProtoEntity>, Status> {
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

#[tonic::async_trait]
impl EdgeService for GrpcState {
    async fn query_edges(
        &self,
        request: Request<QueryEdgesRequest>,
    ) -> Result<Response<QueryEdgesResponse>, Status> {
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
