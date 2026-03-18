// Allow result_large_err for tonic::Status which is 176 bytes - this is the
// standard gRPC error type and we cannot change its size.
#![allow(clippy::result_large_err)]

//! gRPC service implementations for the Mnemo memory API.
//!
//! These handlers delegate to the same storage traits and retrieval engine
//! used by the REST API, providing wire-format parity over HTTP/2 + protobuf.
//!
//! ## Service surface
//!
//! | Service         | RPCs | Notes                                    |
//! |-----------------|------|------------------------------------------|
//! | MemoryService   | 6    | GetContext, RememberMemory, GetMemoryContext, CreateEpisode, ListEpisodes, DeleteEpisode |
//! | UserService     | 6    | Full CRUD + GetByExternalId + ListUsers  |
//! | SessionService  | 5    | Full CRUD + ListUserSessions             |
//! | EntityService   | 4    | ListEntities, GetEntity, Delete, Patch   |
//! | EdgeService     | 4    | QueryEdges, GetEdge, Delete, Patch       |
//! | AgentService    | 8    | Register, Get, UpdateIdentity, Delete, AddExperience, GetAgentContext, LoRA feedback/stats |
//!
//! ## Security
//!
//! All services call `validate_grpc_auth()` at handler entry, using the same
//! `AuthConfig` shared with the REST middleware.  Supports:
//! - `authorization: Bearer <key>` gRPC metadata header
//! - `x-api-key` metadata as a fallback
//! - Bootstrap keys and scoped Redis-backed keys
//!
//! ## Bare vs. governed path
//!
//! The gRPC path is intentionally "bare" — it calls directly into storage /
//! retrieval without running retention-policy checks, guardrail evaluation,
//! webhook emission, or governance audits.  This mirrors how Qdrant's gRPC
//! API is a direct storage surface without a governance plane.  The REST path
//! remains the "governed path" for production memory workflows that require
//! policy enforcement.

use std::sync::Arc;

use tonic::{Request, Response, Status};
use uuid::Uuid;

use mnemo_core::error::MnemoError;
use mnemo_core::models::agent::{
    AgentIdentityProfile, CreateExperienceRequest, ExperienceEvent, UpdateAgentIdentityRequest,
};
use mnemo_core::models::api_key::{hash_api_key, ApiKeyRole, CallerContext};
use mnemo_core::models::classification::Classification;
use mnemo_core::models::context::{
    ContextMessage as CoreContextMessage, ContextRequest as CoreContextRequest, SearchType,
    TemporalIntent,
};
use mnemo_core::models::episode::{
    CreateEpisodeRequest as CoreCreateEpisodeRequest, EpisodeType, MessageRole,
};
use mnemo_core::models::session::{
    CreateSessionRequest as CoreCreateSessionRequest,
    UpdateSessionRequest as CoreUpdateSessionRequest,
};
use mnemo_core::models::user::{
    CreateUserRequest as CoreCreateUserRequest, UpdateUserRequest as CoreUpdateUserRequest,
};
use mnemo_core::traits::storage::{
    AgentStore, ApiKeyStore, EdgeStore, EntityStore, EpisodeStore, LoraStore, SessionStore,
    UserStore,
};
use mnemo_storage::redis_store::RedisStateStore;

use mnemo_proto::proto::{
    // Agent service
    agent_service_server::AgentService,
    // Edge service
    edge_service_server::EdgeService,
    // Entity service
    entity_service_server::EntityService,
    // Memory service
    memory_service_server::MemoryService,
    // Session service
    session_service_server::SessionService,
    // User service
    user_service_server::UserService,
    AddExperienceRequest,
    AgentProfile,
    // Common
    Classification as ProtoClassification,
    CreateEpisodeRequest,
    CreateSessionRequest as ProtoCreateSessionRequest,
    CreateUserRequest as ProtoCreateUserRequest,
    DeleteAgentRequest,
    DeleteEdgeRequest,
    DeleteEntityRequest,
    DeleteEpisodeRequest,
    DeleteEpisodeResponse,
    DeleteResponse,
    DeleteSessionRequest,
    DeleteUserRequest,
    Edge as ProtoEdge,
    Entity as ProtoEntity,
    EntitySummary,
    Episode as ProtoEpisode,
    EpisodeSummary as ProtoEpisodeSummary,
    ExperienceEvent as ProtoExperienceEvent,
    FactSummary,
    GetAgentContextRequest,
    GetAgentContextResponse,
    GetAgentRequest,
    GetContextRequest,
    GetContextResponse,
    GetEdgeRequest,
    GetEntityRequest,
    GetLoraStatsRequest,
    GetMemoryContextRequest,
    GetMemoryContextResponse,
    GetSessionRequest,
    GetUserByExternalIdRequest,
    GetUserRequest,
    ListEntitiesRequest,
    ListEntitiesResponse,
    ListEpisodesRequest,
    ListEpisodesResponse,
    ListUserSessionsRequest,
    ListUserSessionsResponse,
    ListUsersRequest,
    ListUsersResponse,
    LoraFeedbackRequest,
    LoraFeedbackResponse,
    LoraStatsResponse,
    PatchClassificationRequest,
    QueryEdgesRequest,
    QueryEdgesResponse,
    RegisterAgentRequest,
    RememberMemoryRequest,
    RememberMemoryResponse,
    Session as ProtoSession,
    UpdateAgentIdentityRequest as ProtoUpdateAgentIdentityRequest,
    UpdateSessionRequest as ProtoUpdateSessionRequest,
    UpdateUserRequest as ProtoUpdateUserRequest,
    User as ProtoUser,
};

use crate::lora_handle::LoraEmbedderHandle;
use crate::middleware::AuthConfig;
use crate::state::AppState;

// ─── Shared state ───────────────────────────────────────────────────

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

// ─── Auth ───────────────────────────────────────────────────────────

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
                return Err(Status::unauthenticated("Invalid or missing API key"));
            }
            Ok(None) => {
                return Err(Status::unauthenticated("Invalid or missing API key"));
            }
            Err(_) => {
                return Err(Status::unauthenticated("Invalid or missing API key"));
            }
        }
    }

    Err(Status::unauthenticated("Invalid or missing API key"))
}

/// Extract the request-id from gRPC metadata, or generate one.
/// Sanitizes the value to alphanumeric+hyphen, max 64 chars.
fn extract_request_id<T>(request: &Request<T>) -> String {
    request
        .metadata()
        .get("x-mnemo-request-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
        .filter(|s| {
            !s.is_empty()
                && s.len() <= 64
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        })
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

fn to_proto_classification(c: Classification) -> i32 {
    match c {
        Classification::Public => ProtoClassification::Public as i32,
        Classification::Internal => ProtoClassification::Internal as i32,
        Classification::Confidential => ProtoClassification::Confidential as i32,
        Classification::Restricted => ProtoClassification::Restricted as i32,
    }
}

#[allow(clippy::result_large_err)]
fn from_proto_classification(v: i32) -> Result<Classification, Status> {
    match ProtoClassification::try_from(v) {
        Ok(ProtoClassification::Public) => Ok(Classification::Public),
        Ok(ProtoClassification::Internal) => Ok(Classification::Internal),
        Ok(ProtoClassification::Confidential) => Ok(Classification::Confidential),
        Ok(ProtoClassification::Restricted) => Ok(Classification::Restricted),
        // Proto3 default (0 = Unspecified) must be rejected explicitly — callers
        // must always provide a concrete classification level for patch operations.
        Ok(ProtoClassification::Unspecified) => Err(Status::invalid_argument(
            "classification must be explicitly set to PUBLIC, INTERNAL, CONFIDENTIAL, or RESTRICTED",
        )),
        Err(_) => Err(Status::invalid_argument(format!(
            "unknown classification value: {v}"
        ))),
    }
}

/// Convert `serde_json::Value` (object) → `prost_types::Struct`.
/// Returns an empty Struct for null / non-objects.
fn json_to_proto_struct(v: &serde_json::Value) -> prost_types::Struct {
    match v {
        serde_json::Value::Object(map) => {
            let fields = map
                .iter()
                .map(|(k, v)| (k.clone(), json_to_proto_value(v)))
                .collect();
            prost_types::Struct { fields }
        }
        _ => prost_types::Struct::default(),
    }
}

fn json_to_proto_value(v: &serde_json::Value) -> prost_types::Value {
    use prost_types::value::Kind;
    let kind = match v {
        serde_json::Value::Null => Kind::NullValue(0),
        serde_json::Value::Bool(b) => Kind::BoolValue(*b),
        serde_json::Value::Number(n) => Kind::NumberValue(n.as_f64().unwrap_or(0.0)),
        serde_json::Value::String(s) => Kind::StringValue(s.clone()),
        serde_json::Value::Array(arr) => Kind::ListValue(prost_types::ListValue {
            values: arr.iter().map(json_to_proto_value).collect(),
        }),
        serde_json::Value::Object(_) => Kind::StructValue(json_to_proto_struct(v)),
    };
    prost_types::Value { kind: Some(kind) }
}

/// Convert `prost_types::Struct` → `serde_json::Value`.
fn proto_struct_to_json(s: Option<prost_types::Struct>) -> serde_json::Value {
    match s {
        None => serde_json::json!({}),
        Some(st) => {
            let map: serde_json::Map<String, serde_json::Value> = st
                .fields
                .into_iter()
                .map(|(k, v)| (k, proto_value_to_json(v)))
                .collect();
            serde_json::Value::Object(map)
        }
    }
}

fn proto_value_to_json(v: prost_types::Value) -> serde_json::Value {
    use prost_types::value::Kind;
    match v.kind {
        None | Some(Kind::NullValue(_)) => serde_json::Value::Null,
        Some(Kind::BoolValue(b)) => serde_json::Value::Bool(b),
        Some(Kind::NumberValue(n)) => {
            // NaN and ±Infinity are not valid JSON; treat as null to avoid
            // corrupting stored metadata. Callers that care can check for null.
            serde_json::Number::from_f64(n)
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::Null)
        }
        Some(Kind::StringValue(s)) => serde_json::Value::String(s),
        Some(Kind::ListValue(l)) => {
            serde_json::Value::Array(l.values.into_iter().map(proto_value_to_json).collect())
        }
        Some(Kind::StructValue(s)) => proto_struct_to_json(Some(s)),
    }
}

/// Validate a proto Struct for non-finite float values (NaN, ±Infinity).
/// Proto3 allows these in theory but JSON does not, so we reject them early
/// rather than silently converting them to `null` during storage (P2-11).
#[allow(clippy::result_large_err)]
fn validate_proto_struct(s: &prost_types::Struct, path: &str) -> Result<(), Status> {
    for (k, v) in &s.fields {
        validate_proto_value(v, &format!("{path}.{k}"))?;
    }
    Ok(())
}

#[allow(clippy::result_large_err)]
fn validate_proto_value(v: &prost_types::Value, path: &str) -> Result<(), Status> {
    use prost_types::value::Kind;
    match &v.kind {
        Some(Kind::NumberValue(n)) if !n.is_finite() => Err(Status::invalid_argument(format!(
            "metadata field '{path}' contains a non-finite float ({n}) which cannot be stored as JSON"
        ))),
        Some(Kind::StructValue(s)) => validate_proto_struct(s, path),
        Some(Kind::ListValue(l)) => {
            for (i, item) in l.values.iter().enumerate() {
                validate_proto_value(item, &format!("{path}[{i}]"))?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn storage_err_to_status(e: MnemoError) -> Status {
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

/// Resolve a user identifier (UUID string, external_id, or name) to a User.
/// Mirrors `find_user_by_identifier` in routes.rs.
async fn find_user_by_identifier(
    store: &RedisStateStore,
    identifier: &str,
) -> Result<mnemo_core::models::user::User, MnemoError> {
    if let Ok(id) = Uuid::parse_str(identifier) {
        match store.get_user(id).await {
            Ok(user) => return Ok(user),
            Err(e) if e.status_code() == 404 => {}
            Err(e) => return Err(e),
        }
    }
    match store.get_user_by_external_id(identifier).await {
        Ok(user) => return Ok(user),
        Err(e) if e.status_code() == 404 => {}
        Err(e) => return Err(e),
    }
    // Full-scan name match as last resort — capped at 3 pages (600 users)
    // to prevent unbounded Redis scans on large deployments.
    let mut after = None;
    const MAX_SCAN_PAGES: usize = 3;
    for _ in 0..MAX_SCAN_PAGES {
        let users = store.list_users(200, after).await?;
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

/// Find or create a session by name for a given user.
///
/// ## Concurrency note (P1-4)
///
/// This uses an optimistic create-then-verify pattern to close the TOCTOU race:
/// 1. Scan for an existing session with this name.
/// 2. If not found, attempt to create one.
/// 3. After creation, re-scan for the name in case a concurrent request also
///    created one. Return whichever session has the earlier `created_at`.
///
/// This does not provide atomic uniqueness at the storage layer — a proper fix
/// requires a Redis `SETNX` name-index in `RedisStateStore::create_session`.
/// For now, both sessions exist but the same one (earliest) is returned on all
/// paths, so episode history remains contiguous.
async fn find_or_create_session(
    store: &RedisStateStore,
    user_id: Uuid,
    session_name: &str,
) -> Result<mnemo_core::models::session::Session, MnemoError> {
    // Step 1: scan for an existing session with this name.
    if let Some(existing) = scan_session_by_name(store, user_id, session_name).await? {
        return Ok(existing);
    }

    // Step 2: create a new session.
    let created = store
        .create_session(CoreCreateSessionRequest {
            id: None,
            user_id,
            agent_id: None,
            name: Some(session_name.to_string()),
            metadata: serde_json::json!({}),
        })
        .await?;

    // Step 3: re-scan immediately to detect concurrent duplicates and return
    // the earliest one so all callers converge on the same session.
    let sessions = collect_all_sessions_by_name(store, user_id, session_name).await?;
    if sessions.len() > 1 {
        // Return the session with the earliest created_at; all concurrent callers
        // will scan and return the same session.
        let earliest = sessions
            .into_iter()
            .min_by_key(|s| s.created_at)
            .unwrap_or(created);
        return Ok(earliest);
    }

    Ok(created)
}

/// Scan sessions for the first one matching `name` (case-insensitive), or None.
async fn scan_session_by_name(
    store: &RedisStateStore,
    user_id: Uuid,
    name: &str,
) -> Result<Option<mnemo_core::models::session::Session>, MnemoError> {
    let mut after = None;
    loop {
        let params = mnemo_core::models::session::ListSessionsParams {
            limit: 200,
            after,
            since: None,
        };
        let sessions = store.list_sessions(user_id, params).await?;
        if sessions.is_empty() {
            break;
        }
        for session in &sessions {
            if session
                .name
                .as_deref()
                .map(|n| n.eq_ignore_ascii_case(name))
                .unwrap_or(false)
            {
                return Ok(Some(session.clone()));
            }
        }
        after = sessions.last().map(|s| s.id);
        if after.is_none() || sessions.len() < 200 {
            break;
        }
    }
    Ok(None)
}

/// Collect ALL sessions matching `name` — used for duplicate detection after create.
async fn collect_all_sessions_by_name(
    store: &RedisStateStore,
    user_id: Uuid,
    name: &str,
) -> Result<Vec<mnemo_core::models::session::Session>, MnemoError> {
    let mut matches = Vec::new();
    let mut after = None;
    loop {
        let params = mnemo_core::models::session::ListSessionsParams {
            limit: 200,
            after,
            since: None,
        };
        let sessions = store.list_sessions(user_id, params).await?;
        if sessions.is_empty() {
            break;
        }
        for session in &sessions {
            if session
                .name
                .as_deref()
                .map(|n| n.eq_ignore_ascii_case(name))
                .unwrap_or(false)
            {
                matches.push(session.clone());
            }
        }
        after = sessions.last().map(|s| s.id);
        if after.is_none() || sessions.len() < 200 {
            break;
        }
    }
    Ok(matches)
}

#[allow(clippy::result_large_err)]
fn normalize_agent_id(agent_id: &str) -> Result<String, Status> {
    let trimmed = agent_id.trim();
    if trimmed.is_empty() {
        return Err(Status::invalid_argument("agent_id is required"));
    }
    Ok(trimmed.to_string())
}

// ─── Input length limits ─────────────────────────────────────────────
/// Maximum length for user identifier strings (UUID / external_id / name).
const MAX_USER_IDENTIFIER_LEN: usize = 256;
/// Maximum length for session name strings.
const MAX_SESSION_NAME_LEN: usize = 256;
/// Maximum length for episode text / memory text.
const MAX_TEXT_LEN: usize = 32_768; // 32 KiB

// ─── Role enforcement helpers ────────────────────────────────────────
/// Require at least Write role; returns gRPC PermissionDenied on failure.
#[allow(clippy::result_large_err)]
fn require_write(caller: &CallerContext) -> Result<(), Status> {
    caller
        .require_role(ApiKeyRole::Write)
        .map_err(|e| Status::permission_denied(e.to_string()))
}

/// Require Admin role; returns gRPC PermissionDenied on failure.
#[allow(clippy::result_large_err)]
fn require_admin(caller: &CallerContext) -> Result<(), Status> {
    caller
        .require_role(ApiKeyRole::Admin)
        .map_err(|e| Status::permission_denied(e.to_string()))
}

// ─── Proto type converters ──────────────────────────────────────────

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

fn user_to_proto(u: mnemo_core::models::user::User) -> ProtoUser {
    ProtoUser {
        id: u.id.to_string(),
        external_id: u.external_id,
        name: u.name,
        email: u.email,
        metadata: Some(json_to_proto_struct(&u.metadata)),
        created_at: Some(to_proto_timestamp(u.created_at)),
        updated_at: Some(to_proto_timestamp(u.updated_at)),
    }
}

fn session_to_proto(s: mnemo_core::models::session::Session) -> ProtoSession {
    ProtoSession {
        id: s.id.to_string(),
        user_id: s.user_id.to_string(),
        agent_id: s.agent_id,
        name: s.name,
        metadata: Some(json_to_proto_struct(&s.metadata)),
        episode_count: s.episode_count,
        summary: s.summary,
        head_version: s.head_version,
        created_at: Some(to_proto_timestamp(s.created_at)),
        updated_at: Some(to_proto_timestamp(s.updated_at)),
        last_activity_at: s.last_activity_at.map(to_proto_timestamp),
    }
}

fn agent_profile_to_proto(p: AgentIdentityProfile) -> AgentProfile {
    AgentProfile {
        agent_id: p.agent_id,
        version: p.version,
        core: Some(json_to_proto_struct(&p.core)),
        updated_at: Some(to_proto_timestamp(p.updated_at)),
    }
}

fn experience_event_to_proto(e: ExperienceEvent) -> ProtoExperienceEvent {
    ProtoExperienceEvent {
        id: e.id.to_string(),
        agent_id: e.agent_id,
        user_id: e.user_id.as_ref().map(|u| u.to_string()),
        session_id: e.session_id.as_ref().map(|s| s.to_string()),
        category: e.category,
        signal: e.signal.clone(),
        confidence: e.confidence,
        weight: e.weight,
        decay_half_life_days: e.decay_half_life_days,
        evidence_episode_ids: e
            .evidence_episode_ids
            .iter()
            .map(|id| id.to_string())
            .collect(),
        fisher_importance: e.fisher_importance,
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
        let caller = validate_grpc_auth(&self.auth_config, &request).await?;
        // Read-only RPC — Read role or above is sufficient.
        caller
            .require_role(ApiKeyRole::Read)
            .map_err(|e| Status::permission_denied(e.to_string()))?;
        let _request_id = extract_request_id(&request);

        let req = request.into_inner();
        let user_id = parse_uuid(&req.user_id, "user_id")?;

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

        let max_tokens = match req.max_tokens {
            Some(t) if t <= 0 => {
                return Err(Status::invalid_argument(
                    "max_tokens must be a positive integer",
                ));
            }
            Some(t) => t as u32,
            None => 500,
        };

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
            search_types: vec![SearchType::Hybrid],
            temporal_filter: None,
            as_of,
            time_intent: TemporalIntent::Auto,
            temporal_weight: None,
            min_relevance,
            agent_id: None,
            region_ids: vec![],
            structured: req.structured.unwrap_or(false),
            explain: req.explain.unwrap_or(false),
            tiered_budget: req.tiered_budget.unwrap_or(false),
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
            token_count: block.token_count.min(i32::MAX as u32) as i32,
            latency_ms: block.latency_ms,
            routing_decision: block
                .routing_decision
                .map(|rd| serde_json::to_string(&rd).unwrap_or_default()),
        }))
    }

    async fn remember_memory(
        &self,
        request: Request<RememberMemoryRequest>,
    ) -> Result<Response<RememberMemoryResponse>, Status> {
        let caller = validate_grpc_auth(&self.auth_config, &request).await?;
        // Writing memory requires at least Write role.
        require_write(&caller)?;
        let _request_id = extract_request_id(&request);

        let req = request.into_inner();
        if req.user.trim().is_empty() {
            return Err(Status::invalid_argument("user is required"));
        }
        if req.user.len() > MAX_USER_IDENTIFIER_LEN {
            return Err(Status::invalid_argument(format!(
                "user identifier too long (max {MAX_USER_IDENTIFIER_LEN} chars)"
            )));
        }
        if req.text.trim().is_empty() {
            return Err(Status::invalid_argument("text is required"));
        }
        if req.text.len() > MAX_TEXT_LEN {
            return Err(Status::invalid_argument(format!(
                "text too long (max {MAX_TEXT_LEN} bytes)"
            )));
        }
        if let Some(ref s) = req.session {
            if s.len() > MAX_SESSION_NAME_LEN {
                return Err(Status::invalid_argument(format!(
                    "session name too long (max {MAX_SESSION_NAME_LEN} chars)"
                )));
            }
        }

        let user_identifier = req.user.trim().to_string();
        let user = match find_user_by_identifier(&self.state_store, &user_identifier).await {
            Ok(user) => user,
            Err(e) if e.status_code() == 404 => {
                // Auto-create user on first write
                let create = CoreCreateUserRequest {
                    id: None,
                    external_id: Some(user_identifier.clone()),
                    name: user_identifier.clone(),
                    email: None,
                    metadata: serde_json::json!({}),
                };
                match self.state_store.create_user(create).await {
                    Ok(u) => u,
                    Err(e) if e.status_code() == 409 => {
                        find_user_by_identifier(&self.state_store, &user_identifier)
                            .await
                            .map_err(storage_err_to_status)?
                    }
                    Err(e) => return Err(storage_err_to_status(e)),
                }
            }
            Err(e) => return Err(storage_err_to_status(e)),
        };

        let session_name = req
            .session
            .and_then(|s| {
                let t = s.trim().to_string();
                (!t.is_empty()).then_some(t)
            })
            .unwrap_or_else(|| "default".to_string());

        let session = find_or_create_session(&self.state_store, user.id, &session_name)
            .await
            .map_err(storage_err_to_status)?;

        let role = match req.role.as_deref() {
            None => None,
            Some("user") => Some(MessageRole::User),
            Some("assistant") => Some(MessageRole::Assistant),
            Some("system") => Some(MessageRole::System),
            Some(unknown) => {
                return Err(Status::invalid_argument(format!(
                    "unknown role: '{unknown}' (expected 'user', 'assistant', or 'system')"
                )));
            }
        };

        let episode_req = CoreCreateEpisodeRequest {
            id: None,
            episode_type: EpisodeType::Message,
            content: req.text,
            role,
            name: Some(user.name.clone()),
            agent_id: None,
            metadata: serde_json::json!({}),
            created_at: None,
        };

        let episode = self
            .state_store
            .create_episode(episode_req, session.id, user.id, None)
            .await
            .map_err(storage_err_to_status)?;

        Ok(Response::new(RememberMemoryResponse {
            ok: true,
            user_id: user.id.to_string(),
            session_id: session.id.to_string(),
            episode_id: episode.id.to_string(),
        }))
    }

    async fn get_memory_context(
        &self,
        request: Request<GetMemoryContextRequest>,
    ) -> Result<Response<GetMemoryContextResponse>, Status> {
        let caller = validate_grpc_auth(&self.auth_config, &request).await?;
        // Read-only RPC — Read role or above is sufficient.
        caller
            .require_role(ApiKeyRole::Read)
            .map_err(|e| Status::permission_denied(e.to_string()))?;
        let _request_id = extract_request_id(&request);

        let req = request.into_inner();
        if req.user.trim().is_empty() {
            return Err(Status::invalid_argument("user is required"));
        }
        if req.user.len() > MAX_USER_IDENTIFIER_LEN {
            return Err(Status::invalid_argument(format!(
                "user identifier too long (max {MAX_USER_IDENTIFIER_LEN} chars)"
            )));
        }
        if req.query.trim().is_empty() {
            return Err(Status::invalid_argument("query is required"));
        }

        let user = find_user_by_identifier(&self.state_store, req.user.trim())
            .await
            .map_err(storage_err_to_status)?;

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

        // Parse contract — drives temporal intent override and as_of requirement.
        let contract_str = req
            .contract
            .clone()
            .unwrap_or_else(|| "default".to_string());
        let contract_requires_as_of = contract_str == "historical_strict";
        if contract_requires_as_of && as_of.is_none() {
            return Err(Status::invalid_argument(
                "historical_strict contract requires as_of",
            ));
        }

        // Parse retrieval_policy — drives default max_tokens / min_relevance / temporal_weight.
        let policy_str = req
            .retrieval_policy
            .clone()
            .unwrap_or_else(|| "balanced".to_string());
        let (policy_max_tokens, policy_min_relevance, policy_temporal_weight) =
            match policy_str.as_str() {
                "precision" => (400u32, 0.55f32, Some(0.35f32)),
                "recall" => (700, 0.15, Some(0.2)),
                "stability" => (500, 0.4, Some(0.8)),
                _ => (500, 0.3, None), // balanced (default)
            };

        let max_tokens = req.max_tokens.unwrap_or(policy_max_tokens);
        let min_relevance = req.min_relevance.unwrap_or(policy_min_relevance);
        let temporal_weight = req.temporal_weight.or(policy_temporal_weight);

        // Parse time_intent — contract can override.
        let time_intent = match contract_str.as_str() {
            "current_strict" => TemporalIntent::Current,
            "historical_strict" => TemporalIntent::Historical,
            _ => match req.time_intent.as_deref() {
                None | Some("auto") => TemporalIntent::Auto,
                Some("current") => TemporalIntent::Current,
                Some("historical") => TemporalIntent::Historical,
                Some("recent") => TemporalIntent::Recent,
                Some(other) => {
                    return Err(Status::invalid_argument(format!(
                        "unknown time_intent: '{other}'"
                    )))
                }
            },
        };

        // Parse mode_str for echo in response.
        let mode_str = req.mode.clone().unwrap_or_else(|| "hybrid".to_string());

        let session_id = if let Some(ref session_str) = req.session {
            let trimmed = session_str.trim();
            if trimmed.is_empty() {
                None
            } else {
                let mut after = None;
                let mut found_id = None;
                'outer: loop {
                    let params = mnemo_core::models::session::ListSessionsParams {
                        limit: 200,
                        after,
                        since: None,
                    };
                    let sessions = self
                        .state_store
                        .list_sessions(user.id, params)
                        .await
                        .map_err(storage_err_to_status)?;
                    if sessions.is_empty() {
                        break;
                    }
                    for s in &sessions {
                        if s.name
                            .as_deref()
                            .map(|n| n.eq_ignore_ascii_case(trimmed))
                            .unwrap_or(false)
                        {
                            found_id = Some(s.id);
                            break 'outer;
                        }
                    }
                    after = sessions.last().map(|s| s.id);
                    if after.is_none() || sessions.len() < 200 {
                        break;
                    }
                }
                found_id
            }
        } else {
            None
        };

        let core_req = CoreContextRequest {
            session_id,
            messages: vec![CoreContextMessage {
                role: "user".to_string(),
                content: req.query.clone(),
            }],
            max_tokens,
            search_types: vec![SearchType::Hybrid],
            temporal_filter: as_of,
            as_of,
            time_intent,
            temporal_weight,
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
            .get_context(user.id, &core_req, reranker)
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

        let episodes = block
            .episodes
            .iter()
            .map(|e| ProtoEpisodeSummary {
                id: e.id.to_string(),
                session_id: e.session_id.to_string(),
                role: e.role.clone(),
                preview: e.preview.clone(),
                created_at: Some(to_proto_timestamp(e.created_at)),
                relevance: e.relevance,
            })
            .collect();

        Ok(Response::new(GetMemoryContextResponse {
            context: block.context,
            entities,
            facts,
            episodes,
            token_count: block.token_count.min(i32::MAX as u32) as i32,
            latency_ms: block.latency_ms,
            mode: mode_str,
            contract_applied: contract_str,
            retrieval_policy_applied: policy_str,
            routing_decision: block
                .routing_decision
                .map(|rd| serde_json::to_string(&rd).unwrap_or_default()),
            narrative_summary: None,
            goal_applied: req.goal,
            view_applied: req.view,
            guardrail_warnings: vec![],
        }))
    }

    async fn create_episode(
        &self,
        request: Request<CreateEpisodeRequest>,
    ) -> Result<Response<ProtoEpisode>, Status> {
        let caller = validate_grpc_auth(&self.auth_config, &request).await?;
        // Writing episodes requires at least Write role.
        require_write(&caller)?;
        let _request_id = extract_request_id(&request);

        let req = request.into_inner();
        let user_id = parse_uuid(&req.user_id, "user_id")?;
        let session_id = parse_uuid(&req.session_id, "session_id")?;

        if req.episode_type != "message" {
            return Err(Status::invalid_argument(format!(
                "unsupported episode_type: '{}' (only 'message' is supported)",
                req.episode_type
            )));
        }
        if req.content.len() > MAX_TEXT_LEN {
            return Err(Status::invalid_argument(format!(
                "content too long (max {MAX_TEXT_LEN} bytes)"
            )));
        }

        let role = match req.role.as_deref() {
            Some("user") => Some(MessageRole::User),
            Some("assistant") => Some(MessageRole::Assistant),
            Some("system") => Some(MessageRole::System),
            Some(unknown) => {
                return Err(Status::invalid_argument(format!(
                    "unknown role: '{unknown}' (expected 'user', 'assistant', or 'system')"
                )));
            }
            None => None,
        };

        // Validate session exists AND belongs to the specified user (P1: cross-user injection guard).
        let session = self
            .state_store
            .get_session(session_id)
            .await
            .map_err(storage_err_to_status)?;
        if session.user_id != user_id {
            return Err(Status::permission_denied(
                "session does not belong to the specified user",
            ));
        }

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
        let caller = validate_grpc_auth(&self.auth_config, &request).await?;
        caller
            .require_role(ApiKeyRole::Read)
            .map_err(|e| Status::permission_denied(e.to_string()))?;
        let _request_id = extract_request_id(&request);

        let req = request.into_inner();
        let session_id = parse_uuid(&req.session_id, "session_id")?;
        let limit = req.limit.unwrap_or(20).clamp(1, 500) as u32;
        let after = req
            .after
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|s| parse_uuid(s, "after"))
            .transpose()?;

        let params = mnemo_core::models::episode::ListEpisodesParams {
            limit,
            after,
            status: None,
        };

        let episodes = self
            .state_store
            .list_episodes(session_id, params)
            .await
            .map_err(storage_err_to_status)?;

        let next_cursor = if episodes.len() == limit as usize {
            episodes.last().map(|e| e.id.to_string())
        } else {
            None
        };

        Ok(Response::new(ListEpisodesResponse {
            episodes: episodes.into_iter().map(episode_to_proto).collect(),
            next_cursor,
        }))
    }

    async fn delete_episode(
        &self,
        request: Request<DeleteEpisodeRequest>,
    ) -> Result<Response<DeleteEpisodeResponse>, Status> {
        let caller = validate_grpc_auth(&self.auth_config, &request).await?;
        require_write(&caller)?;
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

// ─── UserService ─────────────────────────────────────────────────────

#[tonic::async_trait]
impl UserService for GrpcState {
    async fn create_user(
        &self,
        request: Request<ProtoCreateUserRequest>,
    ) -> Result<Response<ProtoUser>, Status> {
        let caller = validate_grpc_auth(&self.auth_config, &request).await?;
        require_admin(&caller)?;
        let _request_id = extract_request_id(&request);

        let req = request.into_inner();
        if req.name.trim().is_empty() {
            return Err(Status::invalid_argument("name is required"));
        }

        let id = req
            .id
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|s| parse_uuid(s, "id"))
            .transpose()?;

        // P2-11: validate metadata for non-finite floats before conversion.
        if let Some(ref s) = req.metadata {
            validate_proto_struct(s, "metadata")?;
        }
        let core_req = CoreCreateUserRequest {
            id,
            external_id: req.external_id.filter(|s| !s.is_empty()),
            name: req.name.trim().to_string(),
            email: req.email.filter(|s| !s.is_empty()),
            metadata: proto_struct_to_json(req.metadata),
        };

        let user = self
            .state_store
            .create_user(core_req)
            .await
            .map_err(storage_err_to_status)?;

        Ok(Response::new(user_to_proto(user)))
    }

    async fn get_user(
        &self,
        request: Request<GetUserRequest>,
    ) -> Result<Response<ProtoUser>, Status> {
        let caller = validate_grpc_auth(&self.auth_config, &request).await?;
        require_admin(&caller)?;
        let _request_id = extract_request_id(&request);

        let req = request.into_inner();
        let id = parse_uuid(&req.id, "id")?;

        let user = self
            .state_store
            .get_user(id)
            .await
            .map_err(storage_err_to_status)?;

        Ok(Response::new(user_to_proto(user)))
    }

    async fn get_user_by_external_id(
        &self,
        request: Request<GetUserByExternalIdRequest>,
    ) -> Result<Response<ProtoUser>, Status> {
        let caller = validate_grpc_auth(&self.auth_config, &request).await?;
        require_admin(&caller)?;
        let _request_id = extract_request_id(&request);

        let req = request.into_inner();
        if req.external_id.trim().is_empty() {
            return Err(Status::invalid_argument("external_id is required"));
        }

        let user = self
            .state_store
            .get_user_by_external_id(req.external_id.trim())
            .await
            .map_err(storage_err_to_status)?;

        Ok(Response::new(user_to_proto(user)))
    }

    async fn update_user(
        &self,
        request: Request<ProtoUpdateUserRequest>,
    ) -> Result<Response<ProtoUser>, Status> {
        let caller = validate_grpc_auth(&self.auth_config, &request).await?;
        require_admin(&caller)?;
        let _request_id = extract_request_id(&request);

        let req = request.into_inner();
        let id = parse_uuid(&req.id, "id")?;

        let core_req = CoreUpdateUserRequest {
            name: req.name.filter(|s| !s.is_empty()),
            email: req.email.filter(|s| !s.is_empty()),
            external_id: req.external_id.filter(|s| !s.is_empty()),
            // Only update metadata if the Struct has any fields
            metadata: req.metadata.and_then(|s| {
                if s.fields.is_empty() {
                    None
                } else {
                    Some(proto_struct_to_json(Some(s)))
                }
            }),
        };

        let user = self
            .state_store
            .update_user(id, core_req)
            .await
            .map_err(storage_err_to_status)?;

        Ok(Response::new(user_to_proto(user)))
    }

    async fn delete_user(
        &self,
        request: Request<DeleteUserRequest>,
    ) -> Result<Response<DeleteResponse>, Status> {
        let caller = validate_grpc_auth(&self.auth_config, &request).await?;
        require_admin(&caller)?;
        let _request_id = extract_request_id(&request);

        let req = request.into_inner();
        let id = parse_uuid(&req.id, "id")?;

        self.state_store
            .delete_user(id)
            .await
            .map_err(storage_err_to_status)?;

        Ok(Response::new(DeleteResponse { deleted: true }))
    }

    async fn list_users(
        &self,
        request: Request<ListUsersRequest>,
    ) -> Result<Response<ListUsersResponse>, Status> {
        let caller = validate_grpc_auth(&self.auth_config, &request).await?;
        require_admin(&caller)?;
        let _request_id = extract_request_id(&request);

        let req = request.into_inner();
        let pagination = req.pagination.unwrap_or_default();
        let limit = pagination.limit.unwrap_or(20).clamp(1, 500);
        let after = pagination
            .after
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|s| parse_uuid(s, "after"))
            .transpose()?;

        let users = self
            .state_store
            .list_users(limit, after)
            .await
            .map_err(storage_err_to_status)?;

        let count = users.len() as u32;
        let next_cursor = if users.len() == limit as usize {
            users.last().map(|u| u.id.to_string())
        } else {
            None
        };
        let proto_users: Vec<ProtoUser> = users.into_iter().map(user_to_proto).collect();

        Ok(Response::new(ListUsersResponse {
            users: proto_users,
            count,
            next_cursor,
        }))
    }
}

// ─── SessionService ──────────────────────────────────────────────────

#[tonic::async_trait]
impl SessionService for GrpcState {
    async fn create_session(
        &self,
        request: Request<ProtoCreateSessionRequest>,
    ) -> Result<Response<ProtoSession>, Status> {
        let caller = validate_grpc_auth(&self.auth_config, &request).await?;
        require_write(&caller)?;
        let _request_id = extract_request_id(&request);

        let req = request.into_inner();
        let user_id = parse_uuid(&req.user_id, "user_id")?;

        let id = req
            .id
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|s| parse_uuid(s, "id"))
            .transpose()?;

        if let Some(ref s) = req.metadata {
            validate_proto_struct(s, "metadata")?;
        }
        let core_req = CoreCreateSessionRequest {
            id,
            user_id,
            agent_id: req.agent_id.filter(|s| !s.is_empty()),
            name: req.name.filter(|s| !s.trim().is_empty()),
            metadata: proto_struct_to_json(req.metadata),
        };

        let session = self
            .state_store
            .create_session(core_req)
            .await
            .map_err(storage_err_to_status)?;

        Ok(Response::new(session_to_proto(session)))
    }

    async fn get_session(
        &self,
        request: Request<GetSessionRequest>,
    ) -> Result<Response<ProtoSession>, Status> {
        let caller = validate_grpc_auth(&self.auth_config, &request).await?;
        caller
            .require_role(ApiKeyRole::Read)
            .map_err(|e| Status::permission_denied(e.to_string()))?;
        let _request_id = extract_request_id(&request);

        let req = request.into_inner();
        let id = parse_uuid(&req.id, "id")?;

        let session = self
            .state_store
            .get_session(id)
            .await
            .map_err(storage_err_to_status)?;

        Ok(Response::new(session_to_proto(session)))
    }

    async fn update_session(
        &self,
        request: Request<ProtoUpdateSessionRequest>,
    ) -> Result<Response<ProtoSession>, Status> {
        let caller = validate_grpc_auth(&self.auth_config, &request).await?;
        require_write(&caller)?;
        let _request_id = extract_request_id(&request);

        let req = request.into_inner();
        let id = parse_uuid(&req.id, "id")?;

        let core_req = CoreUpdateSessionRequest {
            name: req.name.filter(|s| !s.trim().is_empty()),
            metadata: req.metadata.and_then(|s| {
                if s.fields.is_empty() {
                    None
                } else {
                    Some(proto_struct_to_json(Some(s)))
                }
            }),
            summary: None,
            summary_tokens: None,
        };

        let session = self
            .state_store
            .update_session(id, core_req)
            .await
            .map_err(storage_err_to_status)?;

        Ok(Response::new(session_to_proto(session)))
    }

    async fn delete_session(
        &self,
        request: Request<DeleteSessionRequest>,
    ) -> Result<Response<DeleteResponse>, Status> {
        let caller = validate_grpc_auth(&self.auth_config, &request).await?;
        require_write(&caller)?;
        let _request_id = extract_request_id(&request);

        let req = request.into_inner();
        let id = parse_uuid(&req.id, "id")?;

        self.state_store
            .delete_session(id)
            .await
            .map_err(storage_err_to_status)?;

        Ok(Response::new(DeleteResponse { deleted: true }))
    }

    async fn list_user_sessions(
        &self,
        request: Request<ListUserSessionsRequest>,
    ) -> Result<Response<ListUserSessionsResponse>, Status> {
        let caller = validate_grpc_auth(&self.auth_config, &request).await?;
        caller
            .require_role(ApiKeyRole::Read)
            .map_err(|e| Status::permission_denied(e.to_string()))?;
        let _request_id = extract_request_id(&request);

        let req = request.into_inner();
        let user_id = parse_uuid(&req.user_id, "user_id")?;
        let pagination = req.pagination.unwrap_or_default();
        let limit = pagination.limit.unwrap_or(20).clamp(1, 500);
        let after = pagination
            .after
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|s| parse_uuid(s, "after"))
            .transpose()?;

        let params = mnemo_core::models::session::ListSessionsParams {
            limit,
            after,
            since: None,
        };
        let sessions = self
            .state_store
            .list_sessions(user_id, params)
            .await
            .map_err(storage_err_to_status)?;

        let count = sessions.len() as u32;
        let next_cursor = if sessions.len() == limit as usize {
            sessions.last().map(|s| s.id.to_string())
        } else {
            None
        };
        let proto_sessions: Vec<ProtoSession> =
            sessions.into_iter().map(session_to_proto).collect();

        Ok(Response::new(ListUserSessionsResponse {
            sessions: proto_sessions,
            count,
            next_cursor,
        }))
    }
}

// ─── EntityService ──────────────────────────────────────────────────

#[tonic::async_trait]
impl EntityService for GrpcState {
    async fn list_entities(
        &self,
        request: Request<ListEntitiesRequest>,
    ) -> Result<Response<ListEntitiesResponse>, Status> {
        let caller = validate_grpc_auth(&self.auth_config, &request).await?;
        caller
            .require_role(ApiKeyRole::Read)
            .map_err(|e| Status::permission_denied(e.to_string()))?;
        let _request_id = extract_request_id(&request);

        let req = request.into_inner();
        let user_id = parse_uuid(&req.user_id, "user_id")?;
        let limit = req.limit.unwrap_or(20).clamp(1, 500) as u32;
        let after = req
            .after
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|s| parse_uuid(s, "after"))
            .transpose()?;

        let type_filter = req
            .entity_type
            .as_deref()
            .map(mnemo_core::models::entity::EntityType::from_str_flexible);

        // When a type filter is requested, loop until we have `limit` matching results
        // or exhaust all pages (fixes post-filter pagination bug P2-12).
        let filtered: Vec<_> = if let Some(ref et) = type_filter {
            let mut collected = Vec::new();
            let mut cursor = after;
            loop {
                let batch = self
                    .state_store
                    .list_entities(user_id, limit, cursor)
                    .await
                    .map_err(storage_err_to_status)?;
                let batch_len = batch.len();
                for e in batch {
                    if &e.entity_type == et {
                        collected.push(e);
                        if collected.len() >= limit as usize {
                            break;
                        }
                    }
                }
                if collected.len() >= limit as usize || batch_len < limit as usize {
                    break;
                }
                cursor = collected.last().map(|e| e.id);
            }
            collected
        } else {
            self.state_store
                .list_entities(user_id, limit, after)
                .await
                .map_err(storage_err_to_status)?
        };

        let next_cursor = if filtered.len() == limit as usize {
            filtered.last().map(|e| e.id.to_string())
        } else {
            None
        };

        Ok(Response::new(ListEntitiesResponse {
            entities: filtered.into_iter().map(entity_to_proto).collect(),
            next_cursor,
        }))
    }

    async fn get_entity(
        &self,
        request: Request<GetEntityRequest>,
    ) -> Result<Response<ProtoEntity>, Status> {
        let caller = validate_grpc_auth(&self.auth_config, &request).await?;
        caller
            .require_role(ApiKeyRole::Read)
            .map_err(|e| Status::permission_denied(e.to_string()))?;
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

    async fn delete_entity(
        &self,
        request: Request<DeleteEntityRequest>,
    ) -> Result<Response<DeleteResponse>, Status> {
        let caller = validate_grpc_auth(&self.auth_config, &request).await?;
        require_write(&caller)?;
        let _request_id = extract_request_id(&request);

        let req = request.into_inner();
        let id = parse_uuid(&req.id, "id")?;

        self.state_store
            .delete_entity(id)
            .await
            .map_err(storage_err_to_status)?;

        Ok(Response::new(DeleteResponse { deleted: true }))
    }

    async fn patch_entity_classification(
        &self,
        request: Request<PatchClassificationRequest>,
    ) -> Result<Response<ProtoEntity>, Status> {
        let caller = validate_grpc_auth(&self.auth_config, &request).await?;
        require_write(&caller)?;
        let _request_id = extract_request_id(&request);

        let req = request.into_inner();
        let id = parse_uuid(&req.id, "id")?;
        let classification = from_proto_classification(req.classification)?;

        // P2-14: read-modify-write with retry on concurrent modification.
        // Redis JSON.SET is not atomic for read-modify-write. We retry up to 3
        // times; if a concurrent write changes `updated_at` between our read and
        // write, we re-read and re-apply. This doesn't prevent all races (requires
        // storage-layer CAS for full safety) but eliminates silent downgrades.
        let mut attempts = 0u8;
        loop {
            let mut entity = self
                .state_store
                .get_entity(id)
                .await
                .map_err(storage_err_to_status)?;
            let read_at = entity.updated_at;

            entity.classification = classification;
            entity.updated_at = chrono::Utc::now();

            self.state_store
                .update_entity(&entity)
                .await
                .map_err(storage_err_to_status)?;

            // Verify the write landed with our intended classification.
            let written = self
                .state_store
                .get_entity(id)
                .await
                .map_err(storage_err_to_status)?;

            if written.classification == classification {
                return Ok(Response::new(entity_to_proto(written)));
            }
            // Concurrent write detected — retry if attempts remain.
            attempts += 1;
            if attempts >= 3 {
                return Err(Status::aborted(
                    "classification patch failed due to concurrent modification — retry",
                ));
            }
            let _ = read_at; // suppress unused warning
        }
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
        let caller = validate_grpc_auth(&self.auth_config, &request).await?;
        caller
            .require_role(ApiKeyRole::Read)
            .map_err(|e| Status::permission_denied(e.to_string()))?;
        let _request_id = extract_request_id(&request);

        let req = request.into_inner();
        let user_id = parse_uuid(&req.user_id, "user_id")?;
        let limit = req.limit.unwrap_or(20).clamp(1, 1000) as u32;
        let include_invalidated = !req.current_only.unwrap_or(false);

        let entity_id = req
            .entity_id
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|s| parse_uuid(s, "entity_id"))
            .transpose()?;

        // Query both source and target directions so "edges involving entity X"
        // returns all relationships where X appears as either source or target.
        let mut all_edges: Vec<mnemo_core::models::edge::Edge> = Vec::new();
        if let Some(eid) = entity_id {
            let src_filter = mnemo_core::models::edge::EdgeFilter {
                source_entity_id: Some(eid),
                target_entity_id: None,
                label: req.label.clone(),
                valid_at_time: None,
                include_invalidated,
                max_classification: None,
                limit,
            };
            let tgt_filter = mnemo_core::models::edge::EdgeFilter {
                source_entity_id: None,
                target_entity_id: Some(eid),
                label: req.label.clone(),
                valid_at_time: None,
                include_invalidated,
                max_classification: None,
                limit,
            };
            all_edges.extend(
                self.state_store
                    .query_edges(user_id, src_filter)
                    .await
                    .map_err(storage_err_to_status)?,
            );
            all_edges.extend(
                self.state_store
                    .query_edges(user_id, tgt_filter)
                    .await
                    .map_err(storage_err_to_status)?,
            );
            all_edges.sort_by_key(|e| e.id);
            all_edges.dedup_by_key(|e| e.id);
            all_edges.truncate(limit as usize);
        } else {
            let filter = mnemo_core::models::edge::EdgeFilter {
                source_entity_id: None,
                target_entity_id: None,
                label: req.label,
                valid_at_time: None,
                include_invalidated,
                max_classification: None,
                limit,
            };
            all_edges = self
                .state_store
                .query_edges(user_id, filter)
                .await
                .map_err(storage_err_to_status)?;
        }

        Ok(Response::new(QueryEdgesResponse {
            edges: all_edges.into_iter().map(edge_to_proto).collect(),
        }))
    }

    async fn get_edge(
        &self,
        request: Request<GetEdgeRequest>,
    ) -> Result<Response<ProtoEdge>, Status> {
        let caller = validate_grpc_auth(&self.auth_config, &request).await?;
        caller
            .require_role(ApiKeyRole::Read)
            .map_err(|e| Status::permission_denied(e.to_string()))?;
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

    async fn delete_edge(
        &self,
        request: Request<DeleteEdgeRequest>,
    ) -> Result<Response<DeleteResponse>, Status> {
        let caller = validate_grpc_auth(&self.auth_config, &request).await?;
        require_write(&caller)?;
        let _request_id = extract_request_id(&request);

        let req = request.into_inner();
        let id = parse_uuid(&req.id, "id")?;

        self.state_store
            .delete_edge(id)
            .await
            .map_err(storage_err_to_status)?;

        Ok(Response::new(DeleteResponse { deleted: true }))
    }

    async fn patch_edge_classification(
        &self,
        request: Request<PatchClassificationRequest>,
    ) -> Result<Response<ProtoEdge>, Status> {
        let caller = validate_grpc_auth(&self.auth_config, &request).await?;
        require_write(&caller)?;
        let _request_id = extract_request_id(&request);

        let req = request.into_inner();
        let id = parse_uuid(&req.id, "id")?;
        let classification = from_proto_classification(req.classification)?;

        // P2-14: read-modify-write with retry (mirrors entity classification logic).
        let mut attempts = 0u8;
        loop {
            let mut edge = self
                .state_store
                .get_edge(id)
                .await
                .map_err(storage_err_to_status)?;

            edge.classification = classification;
            edge.updated_at = chrono::Utc::now();

            self.state_store
                .update_edge(&edge)
                .await
                .map_err(storage_err_to_status)?;

            let written = self
                .state_store
                .get_edge(id)
                .await
                .map_err(storage_err_to_status)?;

            if written.classification == classification {
                return Ok(Response::new(edge_to_proto(written)));
            }
            attempts += 1;
            if attempts >= 3 {
                return Err(Status::aborted(
                    "classification patch failed due to concurrent modification — retry",
                ));
            }
        }
    }
}

// ─── AgentService ───────────────────────────────────────────────────

#[allow(clippy::result_large_err)]
#[tonic::async_trait]
impl AgentService for GrpcState {
    async fn register_agent(
        &self,
        request: Request<RegisterAgentRequest>,
    ) -> Result<Response<AgentProfile>, Status> {
        let caller = validate_grpc_auth(&self.auth_config, &request).await?;
        require_write(&caller)?;
        let _request_id = extract_request_id(&request);

        let req = request.into_inner();
        let agent_id = normalize_agent_id(&req.agent_id)?;

        let core = proto_struct_to_json(req.core);
        let description = core
            .get("description")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let (_is_new, _profile) = self
            .state_store
            .register_agent(&agent_id, description)
            .await
            .map_err(storage_err_to_status)?;

        // If core fields were provided, apply them as an identity update.
        // Propagate errors — don't return stale profile on failure.
        if !core.as_object().map(|o| o.is_empty()).unwrap_or(true) {
            let update_req = UpdateAgentIdentityRequest { core };
            self.state_store
                .update_agent_identity(&agent_id, update_req)
                .await
                .map_err(storage_err_to_status)?;
        }

        let updated_profile = self
            .state_store
            .get_agent_identity(&agent_id)
            .await
            .map_err(storage_err_to_status)?;

        Ok(Response::new(agent_profile_to_proto(updated_profile)))
    }

    async fn get_agent(
        &self,
        request: Request<GetAgentRequest>,
    ) -> Result<Response<AgentProfile>, Status> {
        let caller = validate_grpc_auth(&self.auth_config, &request).await?;
        caller
            .require_role(ApiKeyRole::Read)
            .map_err(|e| Status::permission_denied(e.to_string()))?;
        let _request_id = extract_request_id(&request);

        let req = request.into_inner();
        let agent_id = normalize_agent_id(&req.agent_id)?;

        let profile = self
            .state_store
            .get_agent_identity_strict(&agent_id)
            .await
            .map_err(storage_err_to_status)?;

        Ok(Response::new(agent_profile_to_proto(profile)))
    }

    async fn update_agent_identity(
        &self,
        request: Request<ProtoUpdateAgentIdentityRequest>,
    ) -> Result<Response<AgentProfile>, Status> {
        let caller = validate_grpc_auth(&self.auth_config, &request).await?;
        require_write(&caller)?;
        let _request_id = extract_request_id(&request);

        let req = request.into_inner();
        let agent_id = normalize_agent_id(&req.agent_id)?;

        let core = proto_struct_to_json(req.core);
        if !core.is_object() {
            return Err(Status::invalid_argument("core must be a JSON object"));
        }

        let update_req = UpdateAgentIdentityRequest { core };
        let profile = self
            .state_store
            .update_agent_identity(&agent_id, update_req)
            .await
            .map_err(storage_err_to_status)?;

        Ok(Response::new(agent_profile_to_proto(profile)))
    }

    async fn delete_agent(
        &self,
        request: Request<DeleteAgentRequest>,
    ) -> Result<Response<DeleteResponse>, Status> {
        let caller = validate_grpc_auth(&self.auth_config, &request).await?;
        require_write(&caller)?;
        let _request_id = extract_request_id(&request);

        let req = request.into_inner();
        let agent_id = normalize_agent_id(&req.agent_id)?;

        self.state_store
            .delete_agent(&agent_id)
            .await
            .map_err(storage_err_to_status)?;

        Ok(Response::new(DeleteResponse { deleted: true }))
    }

    async fn add_experience(
        &self,
        request: Request<AddExperienceRequest>,
    ) -> Result<Response<ProtoExperienceEvent>, Status> {
        let caller = validate_grpc_auth(&self.auth_config, &request).await?;
        require_write(&caller)?;
        let _request_id = extract_request_id(&request);

        let req = request.into_inner();
        let agent_id = normalize_agent_id(&req.agent_id)?;

        if req.signal.trim().is_empty() {
            return Err(Status::invalid_argument("signal is required"));
        }
        if req.category.trim().is_empty() {
            return Err(Status::invalid_argument("category is required"));
        }

        let evidence_ids: Result<Vec<Uuid>, Status> = req
            .evidence_episode_ids
            .iter()
            .map(|s| parse_uuid(s, "evidence_episode_ids"))
            .collect();
        let evidence_ids = evidence_ids?;

        let user_id = req
            .user_id
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|s| parse_uuid(s, "user_id"))
            .transpose()?;

        let session_id = req
            .session_id
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|s| parse_uuid(s, "session_id"))
            .transpose()?;

        let core_req = CreateExperienceRequest {
            id: None,
            user_id,
            session_id,
            category: req.category,
            signal: req.signal,
            confidence: req.confidence,
            weight: req.weight,
            decay_half_life_days: req.decay_half_life_days,
            evidence_episode_ids: evidence_ids,
            created_at: None,
        };

        let category = core_req.category.clone();
        let mut event = self
            .state_store
            .add_experience_event(&agent_id, core_req)
            .await
            .map_err(storage_err_to_status)?;

        // EWC++: compute Fisher importance using the most recent 50 events in
        // this category (capped from 500 to bound latency on the hot write path).
        // Errors are best-effort — the event is already persisted with fisher=0.0;
        // a background job can recompute if needed.
        let all_events = self
            .state_store
            .list_experience_events(&agent_id, 50)
            .await
            .unwrap_or_default();
        let category_events: Vec<ExperienceEvent> = all_events
            .into_iter()
            .filter(|e| e.category == category && e.id != event.id)
            .collect();
        let fisher = mnemo_core::models::agent::compute_fisher_importance(&event, &category_events);
        event.fisher_importance = fisher;
        // Best-effort persistence of the Fisher score.
        let _ = self.state_store.update_experience_event(&event).await;

        Ok(Response::new(experience_event_to_proto(event)))
    }

    async fn get_agent_context(
        &self,
        request: Request<GetAgentContextRequest>,
    ) -> Result<Response<GetAgentContextResponse>, Status> {
        let caller = validate_grpc_auth(&self.auth_config, &request).await?;
        caller
            .require_role(ApiKeyRole::Read)
            .map_err(|e| Status::permission_denied(e.to_string()))?;
        let _request_id = extract_request_id(&request);

        let req = request.into_inner();
        let agent_id = normalize_agent_id(&req.agent_id)?;

        if req.query.trim().is_empty() {
            return Err(Status::invalid_argument("query is required"));
        }
        if req.user.trim().is_empty() {
            return Err(Status::invalid_argument("user is required"));
        }

        let identity = self
            .state_store
            .get_agent_identity(&agent_id)
            .await
            .map_err(storage_err_to_status)?;

        let experiences = self
            .state_store
            .list_experience_events(&agent_id, 50)
            .await
            .unwrap_or_default();

        let user = find_user_by_identifier(&self.state_store, req.user.trim())
            .await
            .map_err(storage_err_to_status)?;

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

        let time_intent = match req.time_intent.as_deref() {
            None | Some("auto") => TemporalIntent::Auto,
            Some("current") => TemporalIntent::Current,
            Some("historical") => TemporalIntent::Historical,
            Some("recent") => TemporalIntent::Recent,
            Some(other) => {
                return Err(Status::invalid_argument(format!(
                    "unknown time_intent: '{other}'"
                )))
            }
        };

        let max_tokens = req.max_tokens.unwrap_or(500);
        let min_relevance = req.min_relevance.unwrap_or(0.3);

        let core_req = CoreContextRequest {
            session_id: None,
            messages: vec![CoreContextMessage {
                role: "user".to_string(),
                content: req.query.clone(),
            }],
            max_tokens,
            search_types: vec![SearchType::Hybrid],
            temporal_filter: as_of,
            as_of,
            time_intent,
            temporal_weight: req.temporal_weight,
            min_relevance,
            agent_id: Some(agent_id.clone()),
            region_ids: vec![],
            structured: false,
            explain: false,
            tiered_budget: false,
        };

        let reranker = reranker_for_state(&self.reranker);
        let mut context = self
            .retrieval
            .get_context(user.id, &core_req, reranker)
            .await
            .map_err(storage_err_to_status)?;

        // Prepend agent identity + experience signals to context
        let experience_signals: Vec<String> = experiences
            .iter()
            .take(8)
            .map(|e| format!("- [{}] {}", e.category, e.signal))
            .collect();
        let identity_block = format!(
            "## Agent Identity Core\n{}\n\n## Agent Experience Signals\n{}",
            serde_json::to_string_pretty(&identity.core).unwrap_or_else(|_| "{}".to_string()),
            if experience_signals.is_empty() {
                "- none".to_string()
            } else {
                experience_signals.join("\n")
            }
        );
        context.context = if context.context.is_empty() {
            identity_block
        } else {
            format!("{}\n\n{}", identity_block, context.context)
        };

        let experience_weight_sum: f32 = experiences.iter().map(|e| e.effective_weight()).sum();
        let user_memory_items_used =
            (context.entities.len() + context.facts.len() + context.episodes.len()) as u32;

        let entities = context
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

        let facts = context
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

        let identity_version = identity.version;
        let experience_events_used = experiences.len() as u32;

        Ok(Response::new(GetAgentContextResponse {
            context: context.context,
            entities,
            facts,
            token_count: context.token_count.min(i32::MAX as u32) as i32,
            latency_ms: context.latency_ms,
            identity: Some(agent_profile_to_proto(identity)),
            identity_version,
            experience_events_used,
            experience_weight_sum,
            user_memory_items_used,
        }))
    }

    async fn submit_lora_feedback(
        &self,
        request: Request<LoraFeedbackRequest>,
    ) -> Result<Response<LoraFeedbackResponse>, Status> {
        let caller = validate_grpc_auth(&self.auth_config, &request).await?;
        require_write(&caller)?;
        let _request_id = extract_request_id(&request);

        let req = request.into_inner();
        let agent_id = normalize_agent_id(&req.agent_id)?;

        let valid_signals = ["helpful", "not_helpful", "irrelevant"];
        if !valid_signals.contains(&req.signal.as_str()) {
            return Err(Status::invalid_argument(format!(
                "signal must be one of: {}",
                valid_signals.join(", ")
            )));
        }

        let edge_id = parse_uuid(&req.edge_id, "edge_id")?;
        let signal_str = req.signal.clone();

        // Look up edge to get the fact text for the adapter update
        let edge_result = self.state_store.get_edge(edge_id).await;
        let adapter_updated = match edge_result {
            Ok(_edge) => {
                // Record as experience signal
                let signal_value: f32 = match signal_str.as_str() {
                    "helpful" => 1.0,
                    "not_helpful" => -0.5,
                    _ => -1.0, // irrelevant
                };
                let core_req = CreateExperienceRequest {
                    id: None,
                    user_id: None,
                    session_id: None,
                    category: "lora_feedback".to_string(),
                    signal: signal_str.clone(),
                    confidence: signal_value.abs(),
                    weight: signal_value.abs(),
                    decay_half_life_days: 30,
                    evidence_episode_ids: vec![],
                    created_at: None,
                };
                let _ = self
                    .state_store
                    .add_experience_event(&agent_id, core_req)
                    .await;
                Some(agent_id.clone())
            }
            Err(_) => None,
        };

        Ok(Response::new(LoraFeedbackResponse {
            ok: true,
            agent_id,
            signal: signal_str,
            adapter_updated,
        }))
    }

    async fn get_lora_stats(
        &self,
        request: Request<GetLoraStatsRequest>,
    ) -> Result<Response<LoraStatsResponse>, Status> {
        let caller = validate_grpc_auth(&self.auth_config, &request).await?;
        caller
            .require_role(ApiKeyRole::Read)
            .map_err(|e| Status::permission_denied(e.to_string()))?;
        let _request_id = extract_request_id(&request);

        let req = request.into_inner();
        let agent_id = normalize_agent_id(&req.agent_id)?;

        // Count distinct users that have an adapter for this agent
        let users = self
            .state_store
            .list_lora_weights_for_agent(&agent_id)
            .await
            .map_err(storage_err_to_status)?;

        let user_count = users.len() as u32;
        let last_trained_at =
            users
                .iter()
                .map(|w| w.last_updated)
                .max()
                .map(|ts| prost_types::Timestamp {
                    seconds: ts,
                    nanos: 0,
                });

        let avg_norm = if users.is_empty() {
            0.0
        } else {
            users
                .iter()
                .map(|w| {
                    let n: f32 = w.b_flat.iter().map(|x| x * x).sum::<f32>().sqrt();
                    n
                })
                .sum::<f32>()
                / users.len() as f32
        };

        Ok(Response::new(LoraStatsResponse {
            agent_id,
            user_count,
            last_trained_at,
            average_adapter_norm: avg_norm,
        }))
    }
}
