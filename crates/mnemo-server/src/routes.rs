use axum::extract::{Json, Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{delete, get, post, put};
use axum::Router;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use mnemo_core::error::{ApiErrorResponse, MnemoError};
use mnemo_core::models::{
    context::{ContextBlock, ContextRequest},
    edge::{Edge, EdgeFilter},
    entity::Entity,
    episode::{
        BatchCreateEpisodesRequest, CreateEpisodeRequest, Episode, ListEpisodesParams,
    },
    session::{CreateSessionRequest, ListSessionsParams, Session, UpdateSessionRequest},
    user::{CreateUserRequest, UpdateUserRequest, User},
};
use mnemo_core::traits::storage::{
    UserStore, SessionStore, EpisodeStore, EntityStore, EdgeStore, VectorStore,
};

use crate::state::AppState;

// ─── Error handling ────────────────────────────────────────────────

impl IntoResponse for MnemoError {
    fn into_response(self) -> axum::response::Response {
        let status = StatusCode::from_u16(self.status_code())
            .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let body = ApiErrorResponse::from(self);
        (status, Json(body)).into_response()
    }
}

// ─── Response wrappers ─────────────────────────────────────────────

#[derive(Serialize)]
struct ListResponse<T: Serialize> {
    data: Vec<T>,
    count: usize,
}

impl<T: Serialize> ListResponse<T> {
    fn new(data: Vec<T>) -> Self {
        let count = data.len();
        Self { data, count }
    }
}

#[derive(Serialize)]
struct DeleteResponse {
    deleted: bool,
}

// ─── Pagination query params ───────────────────────────────────────

#[derive(Deserialize)]
pub struct PaginationParams {
    #[serde(default = "default_limit")]
    limit: u32,
    after: Option<Uuid>,
}

fn default_limit() -> u32 { 20 }

// ─── Router builder ────────────────────────────────────────────────

pub fn build_router(state: AppState) -> Router {
    Router::new()
        // Health
        .route("/health", get(health))
        .route("/healthz", get(health))
        // Users
        .route("/api/v1/users", post(create_user))
        .route("/api/v1/users", get(list_users))
        .route("/api/v1/users/:id", get(get_user))
        .route("/api/v1/users/:id", put(update_user))
        .route("/api/v1/users/:id", delete(delete_user))
        .route("/api/v1/users/external/:external_id", get(get_user_by_external_id))
        // Sessions
        .route("/api/v1/sessions", post(create_session))
        .route("/api/v1/sessions/:id", get(get_session))
        .route("/api/v1/sessions/:id", put(update_session))
        .route("/api/v1/sessions/:id", delete(delete_session))
        .route("/api/v1/users/:user_id/sessions", get(list_user_sessions))
        // Episodes
        .route("/api/v1/sessions/:session_id/episodes", post(add_episode))
        .route("/api/v1/sessions/:session_id/episodes/batch", post(add_episodes_batch))
        .route("/api/v1/sessions/:session_id/episodes", get(list_episodes))
        .route("/api/v1/episodes/:id", get(get_episode))
        // Entities
        .route("/api/v1/users/:user_id/entities", get(list_entities))
        .route("/api/v1/entities/:id", get(get_entity))
        .route("/api/v1/entities/:id", delete(delete_entity))
        // Edges
        .route("/api/v1/users/:user_id/edges", get(query_edges))
        .route("/api/v1/edges/:id", get(get_edge))
        .route("/api/v1/edges/:id", delete(delete_edge))
        // Context & Search
        .route("/api/v1/users/:user_id/context", post(get_context))
        // Graph
        .route("/api/v1/entities/:id/subgraph", get(get_subgraph))
        .with_state(state)
}

// ─── Health ────────────────────────────────────────────────────────

#[derive(Serialize)]
struct HealthResponse {
    status: String,
    version: String,
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".into(),
        version: env!("CARGO_PKG_VERSION").into(),
    })
}

// ─── User routes ───────────────────────────────────────────────────

async fn create_user(
    State(state): State<AppState>,
    Json(req): Json<CreateUserRequest>,
) -> Result<(StatusCode, Json<User>), MnemoError> {
    let user = state.state_store.create_user(req).await?;
    Ok((StatusCode::CREATED, Json(user)))
}

async fn get_user(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<User>, MnemoError> {
    let user = state.state_store.get_user(id).await?;
    Ok(Json(user))
}

async fn get_user_by_external_id(
    State(state): State<AppState>,
    Path(external_id): Path<String>,
) -> Result<Json<User>, MnemoError> {
    let user = state.state_store.get_user_by_external_id(&external_id).await?;
    Ok(Json(user))
}

async fn update_user(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateUserRequest>,
) -> Result<Json<User>, MnemoError> {
    let user = state.state_store.update_user(id, req).await?;
    Ok(Json(user))
}

async fn delete_user(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<DeleteResponse>, MnemoError> {
    state.state_store.delete_user(id).await?;
    // Also delete vectors for GDPR compliance
    let _ = state.vector_store.delete_user_vectors(id).await;
    Ok(Json(DeleteResponse { deleted: true }))
}

async fn list_users(
    State(state): State<AppState>,
    Query(params): Query<PaginationParams>,
) -> Result<Json<ListResponse<User>>, MnemoError> {
    let users = state.state_store.list_users(params.limit, params.after).await?;
    Ok(Json(ListResponse::new(users)))
}

// ─── Session routes ────────────────────────────────────────────────

async fn create_session(
    State(state): State<AppState>,
    Json(req): Json<CreateSessionRequest>,
) -> Result<(StatusCode, Json<Session>), MnemoError> {
    let session = state.state_store.create_session(req).await?;
    Ok((StatusCode::CREATED, Json(session)))
}

async fn get_session(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Session>, MnemoError> {
    Ok(Json(state.state_store.get_session(id).await?))
}

async fn update_session(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateSessionRequest>,
) -> Result<Json<Session>, MnemoError> {
    Ok(Json(state.state_store.update_session(id, req).await?))
}

async fn delete_session(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<DeleteResponse>, MnemoError> {
    state.state_store.delete_session(id).await?;
    Ok(Json(DeleteResponse { deleted: true }))
}

async fn list_user_sessions(
    State(state): State<AppState>,
    Path(user_id): Path<Uuid>,
    Query(params): Query<PaginationParams>,
) -> Result<Json<ListResponse<Session>>, MnemoError> {
    let list_params = ListSessionsParams {
        limit: params.limit,
        after: params.after,
        since: None,
    };
    let sessions = state.state_store.list_sessions(user_id, list_params).await?;
    Ok(Json(ListResponse::new(sessions)))
}

// ─── Episode routes ────────────────────────────────────────────────

async fn add_episode(
    State(state): State<AppState>,
    Path(session_id): Path<Uuid>,
    Json(req): Json<CreateEpisodeRequest>,
) -> Result<(StatusCode, Json<Episode>), MnemoError> {
    let session = state.state_store.get_session(session_id).await?;
    let episode = state.state_store.create_episode(req, session_id, session.user_id).await?;
    Ok((StatusCode::CREATED, Json(episode)))
}

async fn add_episodes_batch(
    State(state): State<AppState>,
    Path(session_id): Path<Uuid>,
    Json(req): Json<BatchCreateEpisodesRequest>,
) -> Result<(StatusCode, Json<ListResponse<Episode>>), MnemoError> {
    let session = state.state_store.get_session(session_id).await?;
    let episodes = state.state_store
        .create_episodes_batch(req.episodes, session_id, session.user_id)
        .await?;
    Ok((StatusCode::CREATED, Json(ListResponse::new(episodes))))
}

async fn get_episode(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Episode>, MnemoError> {
    Ok(Json(state.state_store.get_episode(id).await?))
}

async fn list_episodes(
    State(state): State<AppState>,
    Path(session_id): Path<Uuid>,
    Query(params): Query<PaginationParams>,
) -> Result<Json<ListResponse<Episode>>, MnemoError> {
    let list_params = ListEpisodesParams {
        limit: params.limit,
        after: params.after,
        status: None,
    };
    let episodes = state.state_store.list_episodes(session_id, list_params).await?;
    Ok(Json(ListResponse::new(episodes)))
}

// ─── Entity routes ─────────────────────────────────────────────────

async fn list_entities(
    State(state): State<AppState>,
    Path(user_id): Path<Uuid>,
    Query(params): Query<PaginationParams>,
) -> Result<Json<ListResponse<Entity>>, MnemoError> {
    let entities = state.state_store.list_entities(user_id, params.limit, params.after).await?;
    Ok(Json(ListResponse::new(entities)))
}

async fn get_entity(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Entity>, MnemoError> {
    Ok(Json(state.state_store.get_entity(id).await?))
}

async fn delete_entity(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<DeleteResponse>, MnemoError> {
    state.state_store.delete_entity(id).await?;
    Ok(Json(DeleteResponse { deleted: true }))
}

// ─── Edge routes ───────────────────────────────────────────────────

async fn query_edges(
    State(state): State<AppState>,
    Path(user_id): Path<Uuid>,
    Query(filter): Query<EdgeFilter>,
) -> Result<Json<ListResponse<Edge>>, MnemoError> {
    let edges = state.state_store.query_edges(user_id, filter).await?;
    Ok(Json(ListResponse::new(edges)))
}

async fn get_edge(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Edge>, MnemoError> {
    Ok(Json(state.state_store.get_edge(id).await?))
}

async fn delete_edge(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<DeleteResponse>, MnemoError> {
    state.state_store.delete_edge(id).await?;
    Ok(Json(DeleteResponse { deleted: true }))
}

// ─── Context route ─────────────────────────────────────────────────

async fn get_context(
    State(state): State<AppState>,
    Path(user_id): Path<Uuid>,
    Json(req): Json<ContextRequest>,
) -> Result<Json<ContextBlock>, MnemoError> {
    let context = state.retrieval.get_context(user_id, &req).await?;
    Ok(Json(context))
}

// ─── Graph route ───────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct SubgraphParams {
    #[serde(default = "default_depth")]
    depth: u32,
    #[serde(default = "default_max_nodes")]
    max_nodes: usize,
}

fn default_depth() -> u32 { 2 }
fn default_max_nodes() -> usize { 50 }

async fn get_subgraph(
    State(state): State<AppState>,
    Path(entity_id): Path<Uuid>,
    Query(params): Query<SubgraphParams>,
) -> Result<Json<serde_json::Value>, MnemoError> {
    let subgraph = state.graph.traverse_bfs(entity_id, params.depth, params.max_nodes, true).await?;

    // Serialize subgraph to JSON
    let nodes: Vec<serde_json::Value> = subgraph.nodes.iter().map(|n| {
        serde_json::json!({
            "entity": {
                "id": n.entity.id,
                "name": n.entity.name,
                "entity_type": n.entity.entity_type.as_str(),
                "summary": n.entity.summary,
            },
            "depth": n.depth,
            "outgoing_edges": n.outgoing.len(),
            "incoming_edges": n.incoming.len(),
        })
    }).collect();

    let edges: Vec<serde_json::Value> = subgraph.edges.iter().map(|e| {
        serde_json::json!({
            "id": e.id,
            "source_entity_id": e.source_entity_id,
            "target_entity_id": e.target_entity_id,
            "label": e.label,
            "fact": e.fact,
            "valid_at": e.valid_at,
            "invalid_at": e.invalid_at,
        })
    }).collect();

    Ok(Json(serde_json::json!({
        "nodes": nodes,
        "edges": edges,
        "entities_visited": subgraph.entities_visited,
    })))
}
