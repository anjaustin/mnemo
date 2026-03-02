use axum::extract::{Json, Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{delete, get, post, put};
use axum::Router;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use mnemo_core::error::{ApiErrorResponse, MnemoError};
use mnemo_core::models::{
    context::{
        estimate_tokens, ContextBlock, ContextMessage, ContextRequest, EpisodeSummary,
        RetrievalSource, TemporalIntent,
    },
    edge::{Edge, EdgeFilter},
    entity::Entity,
    episode::{
        BatchCreateEpisodesRequest, CreateEpisodeRequest, Episode, EpisodeType, ListEpisodesParams,
        MessageRole,
    },
    session::{CreateSessionRequest, ListSessionsParams, Session, UpdateSessionRequest},
    user::{CreateUserRequest, UpdateUserRequest, User},
};
use mnemo_core::traits::storage::{
    EdgeStore, EntityStore, EpisodeStore, SessionStore, UserStore, VectorStore,
};

use crate::state::AppState;

// ─── Error handling ────────────────────────────────────────────────

struct AppError(MnemoError);

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status =
            StatusCode::from_u16(self.0.status_code()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let body = ApiErrorResponse::from(self.0);
        (status, Json(body)).into_response()
    }
}

use axum::response::Response;

impl From<MnemoError> for AppError {
    fn from(err: MnemoError) -> Self {
        AppError(err)
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

fn default_limit() -> u32 {
    20
}

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
        .route(
            "/api/v1/users/external/:external_id",
            get(get_user_by_external_id),
        )
        // Sessions
        .route("/api/v1/sessions", post(create_session))
        .route("/api/v1/sessions/:id", get(get_session))
        .route("/api/v1/sessions/:id", put(update_session))
        .route("/api/v1/sessions/:id", delete(delete_session))
        .route("/api/v1/users/:user_id/sessions", get(list_user_sessions))
        // Episodes
        .route("/api/v1/sessions/:session_id/episodes", post(add_episode))
        .route(
            "/api/v1/sessions/:session_id/episodes/batch",
            post(add_episodes_batch),
        )
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
        // Memory API (high-level DX)
        .route("/api/v1/memory", post(remember_memory))
        .route("/api/v1/memory/:user/context", post(get_memory_context))
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
) -> Result<(StatusCode, Json<User>), AppError> {
    let user = state.state_store.create_user(req).await?;
    Ok((StatusCode::CREATED, Json(user)))
}

async fn get_user(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<User>, AppError> {
    let user = state.state_store.get_user(id).await?;
    Ok(Json(user))
}

async fn get_user_by_external_id(
    State(state): State<AppState>,
    Path(external_id): Path<String>,
) -> Result<Json<User>, AppError> {
    let user = state
        .state_store
        .get_user_by_external_id(&external_id)
        .await?;
    Ok(Json(user))
}

async fn update_user(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateUserRequest>,
) -> Result<Json<User>, AppError> {
    let user = state.state_store.update_user(id, req).await?;
    Ok(Json(user))
}

async fn delete_user(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<DeleteResponse>, AppError> {
    state.state_store.delete_user(id).await?;
    // Also delete vectors for GDPR compliance
    let _ = state.vector_store.delete_user_vectors(id).await;
    Ok(Json(DeleteResponse { deleted: true }))
}

async fn list_users(
    State(state): State<AppState>,
    Query(params): Query<PaginationParams>,
) -> Result<Json<ListResponse<User>>, AppError> {
    let users = state
        .state_store
        .list_users(params.limit, params.after)
        .await?;
    Ok(Json(ListResponse::new(users)))
}

// ─── Session routes ────────────────────────────────────────────────

async fn create_session(
    State(state): State<AppState>,
    Json(req): Json<CreateSessionRequest>,
) -> Result<(StatusCode, Json<Session>), AppError> {
    let session = state.state_store.create_session(req).await?;
    Ok((StatusCode::CREATED, Json(session)))
}

async fn get_session(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Session>, AppError> {
    Ok(Json(state.state_store.get_session(id).await?))
}

async fn update_session(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateSessionRequest>,
) -> Result<Json<Session>, AppError> {
    Ok(Json(state.state_store.update_session(id, req).await?))
}

async fn delete_session(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<DeleteResponse>, AppError> {
    state.state_store.delete_session(id).await?;
    Ok(Json(DeleteResponse { deleted: true }))
}

async fn list_user_sessions(
    State(state): State<AppState>,
    Path(user_id): Path<Uuid>,
    Query(params): Query<PaginationParams>,
) -> Result<Json<ListResponse<Session>>, AppError> {
    let list_params = ListSessionsParams {
        limit: params.limit,
        after: params.after,
        since: None,
    };
    let sessions = state
        .state_store
        .list_sessions(user_id, list_params)
        .await?;
    Ok(Json(ListResponse::new(sessions)))
}

// ─── Episode routes ────────────────────────────────────────────────

async fn add_episode(
    State(state): State<AppState>,
    Path(session_id): Path<Uuid>,
    Json(req): Json<CreateEpisodeRequest>,
) -> Result<(StatusCode, Json<Episode>), AppError> {
    let session = state.state_store.get_session(session_id).await?;
    let episode = state
        .state_store
        .create_episode(req, session_id, session.user_id)
        .await?;
    Ok((StatusCode::CREATED, Json(episode)))
}

async fn add_episodes_batch(
    State(state): State<AppState>,
    Path(session_id): Path<Uuid>,
    Json(req): Json<BatchCreateEpisodesRequest>,
) -> Result<(StatusCode, Json<ListResponse<Episode>>), AppError> {
    let session = state.state_store.get_session(session_id).await?;
    let episodes = state
        .state_store
        .create_episodes_batch(req.episodes, session_id, session.user_id)
        .await?;
    Ok((StatusCode::CREATED, Json(ListResponse::new(episodes))))
}

async fn get_episode(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Episode>, AppError> {
    Ok(Json(state.state_store.get_episode(id).await?))
}

async fn list_episodes(
    State(state): State<AppState>,
    Path(session_id): Path<Uuid>,
    Query(params): Query<PaginationParams>,
) -> Result<Json<ListResponse<Episode>>, AppError> {
    let list_params = ListEpisodesParams {
        limit: params.limit,
        after: params.after,
        status: None,
    };
    let episodes = state
        .state_store
        .list_episodes(session_id, list_params)
        .await?;
    Ok(Json(ListResponse::new(episodes)))
}

// ─── Entity routes ─────────────────────────────────────────────────

async fn list_entities(
    State(state): State<AppState>,
    Path(user_id): Path<Uuid>,
    Query(params): Query<PaginationParams>,
) -> Result<Json<ListResponse<Entity>>, AppError> {
    let entities = state
        .state_store
        .list_entities(user_id, params.limit, params.after)
        .await?;
    Ok(Json(ListResponse::new(entities)))
}

async fn get_entity(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Entity>, AppError> {
    Ok(Json(state.state_store.get_entity(id).await?))
}

async fn delete_entity(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<DeleteResponse>, AppError> {
    state.state_store.delete_entity(id).await?;
    Ok(Json(DeleteResponse { deleted: true }))
}

// ─── Edge routes ───────────────────────────────────────────────────

async fn query_edges(
    State(state): State<AppState>,
    Path(user_id): Path<Uuid>,
    Query(filter): Query<EdgeFilter>,
) -> Result<Json<ListResponse<Edge>>, AppError> {
    let edges = state.state_store.query_edges(user_id, filter).await?;
    Ok(Json(ListResponse::new(edges)))
}

async fn get_edge(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Edge>, AppError> {
    Ok(Json(state.state_store.get_edge(id).await?))
}

async fn delete_edge(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<DeleteResponse>, AppError> {
    state.state_store.delete_edge(id).await?;
    Ok(Json(DeleteResponse { deleted: true }))
}

// ─── Context route ─────────────────────────────────────────────────

async fn get_context(
    State(state): State<AppState>,
    Path(user_id): Path<Uuid>,
    Json(req): Json<ContextRequest>,
) -> Result<Json<ContextBlock>, AppError> {
    let context = state.retrieval.get_context(user_id, &req).await?;
    Ok(Json(context))
}

#[derive(Deserialize)]
struct RememberMemoryRequest {
    user: String,
    text: String,
    #[serde(default)]
    session: Option<String>,
    #[serde(default)]
    role: Option<MessageRole>,
}

#[derive(Serialize)]
struct RememberMemoryResponse {
    ok: bool,
    user_id: Uuid,
    session_id: Uuid,
    episode_id: Uuid,
}

#[derive(Deserialize)]
struct MemoryContextRequest {
    query: String,
    #[serde(default)]
    session: Option<String>,
    #[serde(default)]
    max_tokens: Option<u32>,
    #[serde(default)]
    min_relevance: Option<f32>,
    #[serde(default)]
    time_intent: Option<TemporalIntent>,
    #[serde(default)]
    as_of: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default)]
    temporal_weight: Option<f32>,
    #[serde(default)]
    mode: Option<MemoryContextMode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum MemoryContextMode {
    Head,
    Hybrid,
    Historical,
}

#[derive(Serialize)]
struct MemoryHeadInfo {
    session_id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    episode_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    updated_at: Option<chrono::DateTime<chrono::Utc>>,
    version: u64,
}

#[derive(Serialize)]
struct MemoryContextResponse {
    #[serde(flatten)]
    context: ContextBlock,
    mode: MemoryContextMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    head: Option<MemoryHeadInfo>,
}

async fn remember_memory(
    State(state): State<AppState>,
    Json(req): Json<RememberMemoryRequest>,
) -> Result<(StatusCode, Json<RememberMemoryResponse>), AppError> {
    if req.user.trim().is_empty() {
        return Err(AppError(MnemoError::Validation("user is required".into())));
    }
    if req.text.trim().is_empty() {
        return Err(AppError(MnemoError::Validation("text is required".into())));
    }

    let user_identifier = req.user.trim();
    let user = match find_user_by_identifier(&state, user_identifier).await {
        Ok(user) => user,
        Err(err) if is_user_not_found(&err) => {
            let create = CreateUserRequest {
                id: None,
                external_id: Some(user_identifier.to_string()),
                name: user_identifier.to_string(),
                email: None,
                metadata: serde_json::json!({}),
            };
            match state.state_store.create_user(create).await {
                Ok(user) => user,
                Err(create_err) if matches!(create_err, MnemoError::Duplicate(_)) => {
                    find_user_by_identifier(&state, user_identifier).await?
                }
                Err(create_err) => return Err(create_err.into()),
            }
        }
        Err(err) => return Err(err.into()),
    };

    let session_name = req
        .session
        .and_then(|s| {
            let trimmed = s.trim().to_string();
            (!trimmed.is_empty()).then_some(trimmed)
        })
        .unwrap_or_else(|| "default".to_string());
    let session = find_or_create_session(&state, user.id, &session_name).await?;

    let episode_req = CreateEpisodeRequest {
        id: None,
        episode_type: EpisodeType::Message,
        content: req.text,
        role: Some(req.role.unwrap_or(MessageRole::User)),
        name: Some(user.name.clone()),
        metadata: serde_json::json!({}),
        created_at: None,
    };

    let episode = state
        .state_store
        .create_episode(episode_req, session.id, user.id)
        .await?;

    Ok((
        StatusCode::CREATED,
        Json(RememberMemoryResponse {
            ok: true,
            user_id: user.id,
            session_id: session.id,
            episode_id: episode.id,
        }),
    ))
}

async fn get_memory_context(
    State(state): State<AppState>,
    Path(user): Path<String>,
    Json(req): Json<MemoryContextRequest>,
) -> Result<Json<MemoryContextResponse>, AppError> {
    if req.query.trim().is_empty() {
        return Err(AppError(MnemoError::Validation("query is required".into())));
    }

    let user = find_user_by_identifier(&state, user.trim()).await?;

    let requested_session_name = req.session.and_then(|s| {
        let trimmed = s.trim().to_string();
        (!trimmed.is_empty()).then_some(trimmed)
    });
    let mode = req.mode.unwrap_or(MemoryContextMode::Hybrid);
    let scoped_session =
        resolve_session_scope(&state, user.id, mode, requested_session_name).await?;
    let session_id = scoped_session.as_ref().map(|s| s.id);

    let max_tokens = req.max_tokens.unwrap_or(500);
    let temporal_intent = req.time_intent.unwrap_or(TemporalIntent::Auto);
    let context_req = ContextRequest {
        session_id,
        messages: vec![ContextMessage {
            role: "user".to_string(),
            content: req.query,
        }],
        max_tokens,
        search_types: vec![mnemo_core::models::context::SearchType::Hybrid],
        temporal_filter: req.as_of,
        as_of: req.as_of,
        time_intent: temporal_intent,
        temporal_weight: req.temporal_weight,
        min_relevance: req.min_relevance.unwrap_or(0.3),
    };

    let mut context = state.retrieval.get_context(user.id, &context_req).await?;
    maybe_attach_recent_episode_fallback(
        &state,
        user.id,
        session_id,
        max_tokens,
        temporal_intent,
        req.as_of,
        &mut context,
    )
    .await?;

    let head = scoped_session.as_ref().map(|session| MemoryHeadInfo {
        session_id: session.id,
        episode_id: session.head_episode_id,
        updated_at: session.head_updated_at.or(session.last_activity_at),
        version: session.head_version,
    });

    Ok(Json(MemoryContextResponse {
        context,
        mode,
        head,
    }))
}

async fn resolve_session_scope(
    state: &AppState,
    user_id: Uuid,
    mode: MemoryContextMode,
    session_name: Option<String>,
) -> Result<Option<Session>, MnemoError> {
    if let Some(name) = session_name {
        let session = find_session_by_name(state, user_id, &name).await?;
        return Ok(Some(session));
    }

    match mode {
        MemoryContextMode::Head => find_head_session_for_user(state, user_id).await,
        MemoryContextMode::Hybrid | MemoryContextMode::Historical => Ok(None),
    }
}

async fn find_head_session_for_user(
    state: &AppState,
    user_id: Uuid,
) -> Result<Option<Session>, MnemoError> {
    let mut after = None;
    let mut best: Option<Session> = None;

    loop {
        let sessions = state
            .state_store
            .list_sessions(
                user_id,
                ListSessionsParams {
                    limit: 200,
                    after,
                    since: None,
                },
            )
            .await?;

        if sessions.is_empty() {
            break;
        }

        for session in sessions.iter().cloned() {
            let candidate_time = session
                .head_updated_at
                .or(session.last_activity_at)
                .unwrap_or(session.updated_at);

            let is_better = match &best {
                Some(current_best) => {
                    let current_time = current_best
                        .head_updated_at
                        .or(current_best.last_activity_at)
                        .unwrap_or(current_best.updated_at);
                    candidate_time > current_time
                }
                None => true,
            };

            if is_better {
                best = Some(session);
            }
        }

        after = sessions.last().map(|s| s.id);
        if after.is_none() || sessions.len() < 200 {
            break;
        }
    }

    Ok(best)
}

async fn maybe_attach_recent_episode_fallback(
    state: &AppState,
    user_id: Uuid,
    session_id: Option<Uuid>,
    max_tokens: u32,
    temporal_intent: TemporalIntent,
    as_of: Option<chrono::DateTime<chrono::Utc>>,
    context: &mut ContextBlock,
) -> Result<(), MnemoError> {
    if !context.context.trim().is_empty() {
        return Ok(());
    }

    let mut episodes = Vec::new();
    if let Some(session_id) = session_id {
        episodes = state
            .state_store
            .list_episodes(
                session_id,
                ListEpisodesParams {
                    limit: 8,
                    after: None,
                    status: None,
                },
            )
            .await?;
    } else {
        let sessions = state
            .state_store
            .list_sessions(
                user_id,
                ListSessionsParams {
                    limit: 5,
                    after: None,
                    since: None,
                },
            )
            .await?;

        for session in sessions {
            let mut session_episodes = state
                .state_store
                .list_episodes(
                    session.id,
                    ListEpisodesParams {
                        limit: 4,
                        after: None,
                        status: None,
                    },
                )
                .await?;
            episodes.append(&mut session_episodes);
        }

        episodes.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        episodes.truncate(8);
    }

    if episodes.is_empty() {
        return Ok(());
    }

    let now = chrono::Utc::now();
    episodes.sort_by(|a, b| {
        let a_score = fallback_temporal_score(a.created_at, temporal_intent, as_of, now);
        let b_score = fallback_temporal_score(b.created_at, temporal_intent, as_of, now);
        b_score
            .partial_cmp(&a_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut lines = Vec::new();
    lines.push("Recent conversation snippets (extraction may still be processing):".to_string());
    let mut running_tokens = estimate_tokens(&lines[0]);

    for episode in &episodes {
        let role = episode
            .role
            .map(|r| format!("{:?}", r).to_lowercase())
            .unwrap_or_else(|| "event".to_string());
        let content = if episode.content.len() > 180 {
            format!("{}...", &episode.content[..180])
        } else {
            episode.content.clone()
        };
        let line = format!("- [{}] {}", role, content.replace('\n', " "));
        let line_tokens = estimate_tokens(&line);
        if running_tokens + line_tokens > max_tokens {
            break;
        }
        lines.push(line);
        running_tokens += line_tokens;
    }

    if lines.len() == 1 {
        return Ok(());
    }

    context.context = lines.join("\n");
    context.token_count = estimate_tokens(&context.context);
    if !context.sources.contains(&RetrievalSource::EpisodeRecall) {
        context.sources.push(RetrievalSource::EpisodeRecall);
    }
    if !context.sources.contains(&RetrievalSource::TemporalScoring) {
        context.sources.push(RetrievalSource::TemporalScoring);
    }

    if context.episodes.is_empty() {
        for episode in episodes {
            let preview = if episode.content.len() > 200 {
                format!("{}...", &episode.content[..200])
            } else {
                episode.content
            };
            context.episodes.push(EpisodeSummary {
                id: episode.id,
                session_id: episode.session_id,
                role: episode.role.map(|r| format!("{:?}", r).to_lowercase()),
                preview,
                created_at: episode.created_at,
                relevance: 0.1,
            });
        }
    }

    Ok(())
}

fn fallback_temporal_score(
    created_at: chrono::DateTime<chrono::Utc>,
    temporal_intent: TemporalIntent,
    as_of: Option<chrono::DateTime<chrono::Utc>>,
    now: chrono::DateTime<chrono::Utc>,
) -> f64 {
    match temporal_intent {
        TemporalIntent::Historical => {
            if let Some(as_of) = as_of {
                let delta = (created_at - as_of).num_days().unsigned_abs() as f64;
                (-delta / 14.0).exp().clamp(0.0, 1.0)
            } else {
                0.5
            }
        }
        TemporalIntent::Recent | TemporalIntent::Current | TemporalIntent::Auto => {
            let age_days = (now - created_at).num_days().max(0) as f64;
            (-age_days / 21.0).exp().clamp(0.0, 1.0)
        }
    }
}

async fn find_user_by_identifier(state: &AppState, identifier: &str) -> Result<User, MnemoError> {
    if let Ok(id) = Uuid::parse_str(identifier) {
        match state.state_store.get_user(id).await {
            Ok(user) => return Ok(user),
            Err(err) if is_user_not_found(&err) => {}
            Err(err) => return Err(err),
        }
    }

    match state.state_store.get_user_by_external_id(identifier).await {
        Ok(user) => return Ok(user),
        Err(err) if is_user_not_found(&err) => {}
        Err(err) => return Err(err),
    }

    let mut after = None;
    loop {
        let users = state.state_store.list_users(200, after).await?;
        if users.is_empty() {
            break;
        }

        for user in &users {
            if user.name.eq_ignore_ascii_case(identifier) {
                return Ok(user.clone());
            }
        }

        after = users.last().map(|u| u.id);
        if after.is_none() || users.len() < 200 {
            break;
        }
    }

    Err(MnemoError::NotFound {
        resource_type: "User".into(),
        id: identifier.to_string(),
    })
}

async fn find_or_create_session(
    state: &AppState,
    user_id: Uuid,
    session_name: &str,
) -> Result<Session, MnemoError> {
    match find_session_by_name(state, user_id, session_name).await {
        Ok(session) => return Ok(session),
        Err(err) if is_session_not_found(&err) => {}
        Err(err) => return Err(err),
    }

    state
        .state_store
        .create_session(CreateSessionRequest {
            id: None,
            user_id,
            name: Some(session_name.to_string()),
            metadata: serde_json::json!({}),
        })
        .await
}

async fn find_session_by_name(
    state: &AppState,
    user_id: Uuid,
    session_name: &str,
) -> Result<Session, MnemoError> {
    let mut after = None;
    loop {
        let sessions = state
            .state_store
            .list_sessions(
                user_id,
                ListSessionsParams {
                    limit: 200,
                    after,
                    since: None,
                },
            )
            .await?;

        if sessions.is_empty() {
            break;
        }

        for session in &sessions {
            if session
                .name
                .as_deref()
                .is_some_and(|name| name.eq_ignore_ascii_case(session_name))
            {
                return Ok(session.clone());
            }
        }

        after = sessions.last().map(|s| s.id);
        if after.is_none() || sessions.len() < 200 {
            break;
        }
    }

    Err(MnemoError::NotFound {
        resource_type: "Session".into(),
        id: format!("{}:{}", user_id, session_name),
    })
}

fn is_user_not_found(err: &MnemoError) -> bool {
    matches!(err, MnemoError::UserNotFound(_))
        || matches!(err, MnemoError::NotFound { resource_type, .. } if resource_type.eq_ignore_ascii_case("user"))
}

fn is_session_not_found(err: &MnemoError) -> bool {
    matches!(err, MnemoError::SessionNotFound(_))
        || matches!(err, MnemoError::NotFound { resource_type, .. } if resource_type.eq_ignore_ascii_case("session"))
}

// ─── Graph route ───────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct SubgraphParams {
    #[serde(default = "default_depth")]
    depth: u32,
    #[serde(default = "default_max_nodes")]
    max_nodes: usize,
}

fn default_depth() -> u32 {
    2
}
fn default_max_nodes() -> usize {
    50
}

async fn get_subgraph(
    State(state): State<AppState>,
    Path(entity_id): Path<Uuid>,
    Query(params): Query<SubgraphParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    let subgraph = state
        .graph
        .traverse_bfs(entity_id, params.depth, params.max_nodes, true)
        .await?;

    // Serialize subgraph to JSON
    let nodes: Vec<serde_json::Value> = subgraph
        .nodes
        .iter()
        .map(|n| {
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
        })
        .collect();

    let edges: Vec<serde_json::Value> = subgraph
        .edges
        .iter()
        .map(|e| {
            serde_json::json!({
                "id": e.id,
                "source_entity_id": e.source_entity_id,
                "target_entity_id": e.target_entity_id,
                "label": e.label,
                "fact": e.fact,
                "valid_at": e.valid_at,
                "invalid_at": e.invalid_at,
            })
        })
        .collect();

    Ok(Json(serde_json::json!({
        "nodes": nodes,
        "edges": edges,
        "entities_visited": subgraph.entities_visited,
    })))
}
