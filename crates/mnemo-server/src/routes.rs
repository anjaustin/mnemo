use axum::extract::{DefaultBodyLimit, Json, Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{delete, get, post, put};
use axum::Router;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;
use uuid::Uuid;

use hmac::{Hmac, Mac};
use redis::AsyncCommands;
use sha2::Sha256;
use tracing::warn;

use mnemo_core::error::{ApiErrorResponse, MnemoError};
use mnemo_core::models::{
    agent::{
        AgentIdentityAuditEvent, AgentIdentityProfile, CreateExperienceRequest,
        CreatePromotionProposalRequest, ExperienceEvent, IdentityRollbackRequest,
        PromotionProposal, PromotionStatus, UpdateAgentIdentityRequest,
    },
    context::{
        estimate_tokens, ContextBlock, ContextMessage, ContextRequest, EpisodeSummary, FactSummary,
        RetrievalSource, TemporalIntent,
    },
    edge::{Edge, EdgeFilter},
    entity::Entity,
    episode::{
        BatchCreateEpisodesRequest, CreateEpisodeRequest, Episode, EpisodeType, ListEpisodesParams,
        MessageRole, ProcessingStatus,
    },
    session::{CreateSessionRequest, ListSessionsParams, Session, UpdateSessionRequest},
    user::{CreateUserRequest, UpdateUserRequest, User},
};
use mnemo_core::traits::storage::{
    AgentStore, EdgeStore, EntityStore, EpisodeStore, SessionStore, UserStore, VectorStore,
};

use crate::state::{
    AppState, ImportJobRecord, ImportJobStatus, MemoryWebhookAuditRecord, MemoryWebhookEventRecord,
    MemoryWebhookEventType, MemoryWebhookSubscription, WebhookRuntimeState,
};

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

fn default_true() -> bool {
    true
}

fn default_webhook_events() -> Vec<MemoryWebhookEventType> {
    vec![
        MemoryWebhookEventType::FactAdded,
        MemoryWebhookEventType::FactSuperseded,
        MemoryWebhookEventType::HeadAdvanced,
        MemoryWebhookEventType::ConflictDetected,
    ]
}

fn webhook_event_type_str(event_type: MemoryWebhookEventType) -> &'static str {
    match event_type {
        MemoryWebhookEventType::FactAdded => "fact_added",
        MemoryWebhookEventType::FactSuperseded => "fact_superseded",
        MemoryWebhookEventType::HeadAdvanced => "head_advanced",
        MemoryWebhookEventType::ConflictDetected => "conflict_detected",
    }
}

fn is_http_url(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

fn webhook_subscriptions_key(state: &AppState) -> String {
    format!("{}:subscriptions", state.webhook_redis_prefix)
}

fn webhook_events_key(state: &AppState) -> String {
    format!("{}:events", state.webhook_redis_prefix)
}

fn webhook_audit_key(state: &AppState) -> String {
    format!("{}:audit", state.webhook_redis_prefix)
}

pub async fn restore_webhook_state(state: &AppState) -> Result<(), MnemoError> {
    if !state.webhook_delivery.persistence_enabled {
        return Ok(());
    }
    let Some(mut conn) = state.webhook_redis.clone() else {
        return Ok(());
    };

    let subscriptions_raw: Option<String> = conn
        .get(webhook_subscriptions_key(state))
        .await
        .map_err(|err| MnemoError::Redis(err.to_string()))?;
    let events_raw: Option<String> = conn
        .get(webhook_events_key(state))
        .await
        .map_err(|err| MnemoError::Redis(err.to_string()))?;
    let audit_raw: Option<String> = conn
        .get(webhook_audit_key(state))
        .await
        .map_err(|err| MnemoError::Redis(err.to_string()))?;

    if let Some(json) = subscriptions_raw {
        let parsed: HashMap<Uuid, MemoryWebhookSubscription> = serde_json::from_str(&json)
            .map_err(|err| MnemoError::Serialization(err.to_string()))?;
        let mut hooks = state.memory_webhooks.write().await;
        *hooks = parsed;
    }
    if let Some(json) = events_raw {
        let parsed: HashMap<Uuid, Vec<MemoryWebhookEventRecord>> = serde_json::from_str(&json)
            .map_err(|err| MnemoError::Serialization(err.to_string()))?;
        let mut events = state.memory_webhook_events.write().await;
        *events = parsed;
    }
    if let Some(json) = audit_raw {
        let parsed: HashMap<Uuid, Vec<MemoryWebhookAuditRecord>> = serde_json::from_str(&json)
            .map_err(|err| MnemoError::Serialization(err.to_string()))?;
        let mut audit = state.memory_webhook_audit.write().await;
        *audit = parsed;
    }

    Ok(())
}

async fn persist_webhook_state(state: &AppState) {
    if !state.webhook_delivery.persistence_enabled {
        return;
    }
    let Some(mut conn) = state.webhook_redis.clone() else {
        return;
    };

    let hooks_snapshot = {
        let hooks = state.memory_webhooks.read().await;
        hooks.clone()
    };
    let events_snapshot = {
        let events = state.memory_webhook_events.read().await;
        events.clone()
    };
    let audit_snapshot = {
        let audit = state.memory_webhook_audit.read().await;
        audit.clone()
    };

    let hooks_json = match serde_json::to_string(&hooks_snapshot) {
        Ok(json) => json,
        Err(err) => {
            warn!(error = %err, "failed to serialize webhook subscriptions for persistence");
            return;
        }
    };
    let events_json = match serde_json::to_string(&events_snapshot) {
        Ok(json) => json,
        Err(err) => {
            warn!(error = %err, "failed to serialize webhook events for persistence");
            return;
        }
    };
    let audit_json = match serde_json::to_string(&audit_snapshot) {
        Ok(json) => json,
        Err(err) => {
            warn!(error = %err, "failed to serialize webhook audit for persistence");
            return;
        }
    };

    if let Err(err) = redis::cmd("SET")
        .arg(webhook_subscriptions_key(state))
        .arg(hooks_json)
        .exec_async(&mut conn)
        .await
    {
        warn!(error = %err, "failed to persist webhook subscriptions");
    }

    if let Err(err) = redis::cmd("SET")
        .arg(webhook_events_key(state))
        .arg(events_json)
        .exec_async(&mut conn)
        .await
    {
        warn!(error = %err, "failed to persist webhook events");
    }

    if let Err(err) = redis::cmd("SET")
        .arg(webhook_audit_key(state))
        .arg(audit_json)
        .exec_async(&mut conn)
        .await
    {
        warn!(error = %err, "failed to persist webhook audit");
    }
}

async fn append_webhook_audit(
    state: &AppState,
    webhook_id: Uuid,
    action: &str,
    details: serde_json::Value,
) {
    const MAX_AUDIT_PER_WEBHOOK: usize = 1000;
    let record = MemoryWebhookAuditRecord {
        id: Uuid::now_v7(),
        webhook_id,
        action: action.to_string(),
        details,
        at: chrono::Utc::now(),
    };
    {
        let mut audit = state.memory_webhook_audit.write().await;
        let rows = audit.entry(webhook_id).or_default();
        rows.push(record);
        if rows.len() > MAX_AUDIT_PER_WEBHOOK {
            let overflow = rows.len() - MAX_AUDIT_PER_WEBHOOK;
            rows.drain(0..overflow);
        }
    }
    persist_webhook_state(state).await;
}

async fn check_webhook_rate_and_circuit(state: &AppState, webhook_id: Uuid) -> Result<(), String> {
    let now = chrono::Utc::now();
    let mut runtime = state.webhook_runtime.write().await;
    let row = runtime.entry(webhook_id).or_insert(WebhookRuntimeState {
        window_started_at: now,
        sent_in_window: 0,
        consecutive_failures: 0,
        circuit_open_until: None,
    });

    if let Some(open_until) = row.circuit_open_until {
        if now < open_until {
            return Err(format!("circuit open until {open_until}"));
        }
        row.circuit_open_until = None;
    }

    if (now - row.window_started_at).num_seconds() >= 60 {
        row.window_started_at = now;
        row.sent_in_window = 0;
    }

    if row.sent_in_window >= state.webhook_delivery.rate_limit_per_minute.max(1) {
        return Err("rate limit exceeded for current minute window".to_string());
    }

    row.sent_in_window += 1;
    Ok(())
}

async fn record_webhook_delivery_success(state: &AppState, webhook_id: Uuid) {
    let mut runtime = state.webhook_runtime.write().await;
    if let Some(row) = runtime.get_mut(&webhook_id) {
        row.consecutive_failures = 0;
        row.circuit_open_until = None;
    }
}

async fn record_webhook_delivery_failure(state: &AppState, webhook_id: Uuid) {
    let now = chrono::Utc::now();
    let mut runtime = state.webhook_runtime.write().await;
    let row = runtime.entry(webhook_id).or_insert(WebhookRuntimeState {
        window_started_at: now,
        sent_in_window: 0,
        consecutive_failures: 0,
        circuit_open_until: None,
    });
    row.consecutive_failures = row.consecutive_failures.saturating_add(1);
    if row.consecutive_failures >= state.webhook_delivery.circuit_breaker_threshold.max(1) {
        row.circuit_open_until = Some(
            now + chrono::Duration::milliseconds(
                state.webhook_delivery.circuit_breaker_cooldown_ms as i64,
            ),
        );
        row.consecutive_failures = 0;
    }
}

async fn emit_memory_webhook_event(
    state: &AppState,
    user_id: Uuid,
    event_type: MemoryWebhookEventType,
    payload: serde_json::Value,
) {
    let subscribed_hooks: Vec<MemoryWebhookSubscription> = {
        let hooks = state.memory_webhooks.read().await;
        hooks
            .values()
            .filter(|hook| hook.enabled)
            .filter(|hook| hook.user_id == user_id)
            .filter(|hook| hook.events.contains(&event_type))
            .cloned()
            .collect()
    };

    if subscribed_hooks.is_empty() {
        return;
    }

    let now = chrono::Utc::now();
    let mut queued_deliveries: Vec<(Uuid, Uuid)> = Vec::new();
    let mut event_map = state.memory_webhook_events.write().await;
    for webhook in subscribed_hooks {
        let row = MemoryWebhookEventRecord {
            id: Uuid::now_v7(),
            webhook_id: webhook.id,
            event_type,
            user_id,
            payload: payload.clone(),
            created_at: now,
            attempts: 0,
            delivered: false,
            dead_letter: false,
            delivered_at: None,
            last_error: None,
        };
        let event_id = row.id;
        let rows = event_map.entry(webhook.id).or_default();
        rows.push(row);
        let max_events = state.webhook_delivery.max_events_per_webhook.max(1);
        if rows.len() > max_events {
            let overflow = rows.len() - max_events;
            rows.drain(0..overflow);
        }
        queued_deliveries.push((webhook.id, event_id));
    }

    persist_webhook_state(state).await;

    if !state.webhook_delivery.enabled {
        return;
    }

    for (webhook_id, event_id) in queued_deliveries {
        let state_clone = state.clone();
        tokio::spawn(async move {
            deliver_memory_webhook_event(state_clone, webhook_id, event_id).await;
        });
    }
}

async fn deliver_memory_webhook_event(state: AppState, webhook_id: Uuid, event_id: Uuid) {
    let webhook = {
        let hooks = state.memory_webhooks.read().await;
        hooks.get(&webhook_id).cloned()
    };
    let Some(webhook) = webhook else {
        return;
    };

    let event = {
        let events = state.memory_webhook_events.read().await;
        events
            .get(&webhook_id)
            .and_then(|rows| rows.iter().find(|row| row.id == event_id))
            .cloned()
    };
    let Some(event) = event else {
        return;
    };

    if event.delivered {
        return;
    }

    let body = serde_json::json!({
        "event_id": event.id,
        "event_type": event.event_type,
        "user_id": event.user_id,
        "payload": event.payload,
        "created_at": event.created_at
    });

    let serialized = match serde_json::to_string(&body) {
        Ok(value) => value,
        Err(err) => {
            update_webhook_delivery_status(
                &state,
                webhook_id,
                event_id,
                1,
                false,
                true,
                Some(format!("serialize failure: {err}")),
            )
            .await;
            append_webhook_audit(
                &state,
                webhook_id,
                "delivery_dead_letter",
                serde_json::json!({
                    "event_id": event_id,
                    "reason": format!("serialize failure: {err}")
                }),
            )
            .await;
            record_webhook_delivery_failure(&state, webhook_id).await;
            persist_webhook_state(&state).await;
            return;
        }
    };

    let max_attempts = state.webhook_delivery.max_attempts.max(1);
    let timeout = Duration::from_millis(state.webhook_delivery.request_timeout_ms.max(1));

    for attempt in 1..=max_attempts {
        if let Err(err) = check_webhook_rate_and_circuit(&state, webhook_id).await {
            let dead_letter = attempt >= max_attempts;
            update_webhook_delivery_status(
                &state,
                webhook_id,
                event_id,
                attempt,
                false,
                dead_letter,
                Some(err),
            )
            .await;
            record_webhook_delivery_failure(&state, webhook_id).await;
            if dead_letter {
                append_webhook_audit(
                    &state,
                    webhook_id,
                    "delivery_dead_letter",
                    serde_json::json!({
                        "event_id": event_id,
                        "reason": "circuit_or_rate_limited"
                    }),
                )
                .await;
                persist_webhook_state(&state).await;
                return;
            }
            let shift = (attempt - 1).min(10);
            let factor = 1u64 << shift;
            let delay_ms = state
                .webhook_delivery
                .base_backoff_ms
                .saturating_mul(factor)
                .max(1);
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            continue;
        }

        let timestamp = chrono::Utc::now().timestamp().to_string();
        let signature_header = webhook
            .signing_secret
            .as_ref()
            .map(|secret| build_webhook_signature(secret, &timestamp, &serialized));

        let mut request = state
            .webhook_http
            .post(&webhook.target_url)
            .header("content-type", "application/json")
            .header("x-mnemo-event-id", event.id.to_string())
            .header(
                "x-mnemo-event-type",
                webhook_event_type_str(event.event_type),
            )
            .header("x-mnemo-timestamp", timestamp)
            .timeout(timeout)
            .body(serialized.clone());

        if let Some(sig) = signature_header {
            request = request.header("x-mnemo-signature", sig);
        }

        match request.send().await {
            Ok(response) if response.status().is_success() => {
                update_webhook_delivery_status(
                    &state, webhook_id, event_id, attempt, true, false, None,
                )
                .await;
                record_webhook_delivery_success(&state, webhook_id).await;
                persist_webhook_state(&state).await;
                return;
            }
            Ok(response) => {
                let err = format!("http status {}", response.status());
                let dead_letter = attempt >= max_attempts;
                update_webhook_delivery_status(
                    &state,
                    webhook_id,
                    event_id,
                    attempt,
                    false,
                    dead_letter,
                    Some(err.clone()),
                )
                .await;
                record_webhook_delivery_failure(&state, webhook_id).await;
                if dead_letter {
                    append_webhook_audit(
                        &state,
                        webhook_id,
                        "delivery_dead_letter",
                        serde_json::json!({
                            "event_id": event_id,
                            "reason": err
                        }),
                    )
                    .await;
                    persist_webhook_state(&state).await;
                    return;
                }
                if attempt < max_attempts {
                    let shift = (attempt - 1).min(10);
                    let factor = 1u64 << shift;
                    let delay_ms = state
                        .webhook_delivery
                        .base_backoff_ms
                        .saturating_mul(factor)
                        .max(1);
                    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                }
            }
            Err(err) => {
                let dead_letter = attempt >= max_attempts;
                update_webhook_delivery_status(
                    &state,
                    webhook_id,
                    event_id,
                    attempt,
                    false,
                    dead_letter,
                    Some(err.to_string()),
                )
                .await;
                record_webhook_delivery_failure(&state, webhook_id).await;
                if dead_letter {
                    append_webhook_audit(
                        &state,
                        webhook_id,
                        "delivery_dead_letter",
                        serde_json::json!({
                            "event_id": event_id,
                            "reason": err.to_string()
                        }),
                    )
                    .await;
                    persist_webhook_state(&state).await;
                    return;
                }
                if attempt < max_attempts {
                    let shift = (attempt - 1).min(10);
                    let factor = 1u64 << shift;
                    let delay_ms = state
                        .webhook_delivery
                        .base_backoff_ms
                        .saturating_mul(factor)
                        .max(1);
                    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                }
            }
        }
    }

    persist_webhook_state(&state).await;
}

fn build_webhook_signature(secret: &str, timestamp: &str, body: &str) -> String {
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .expect("hmac key initialization should not fail");
    let signed = format!("{}.{}", timestamp, body);
    mac.update(signed.as_bytes());
    let digest = hex::encode(mac.finalize().into_bytes());
    format!("t={timestamp},v1={digest}")
}

async fn update_webhook_delivery_status(
    state: &AppState,
    webhook_id: Uuid,
    event_id: Uuid,
    attempts: u32,
    delivered: bool,
    dead_letter: bool,
    error: Option<String>,
) {
    let mut events = state.memory_webhook_events.write().await;
    if let Some(rows) = events.get_mut(&webhook_id) {
        if let Some(event) = rows.iter_mut().find(|row| row.id == event_id) {
            event.attempts = attempts;
            if delivered {
                event.delivered = true;
                event.dead_letter = false;
                event.delivered_at = Some(chrono::Utc::now());
                event.last_error = None;
            } else {
                event.dead_letter = dead_letter;
                event.last_error = error;
            }
        }
    }
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
        .route(
            "/api/v1/memory/:user/changes_since",
            post(memory_changes_since),
        )
        .route("/api/v1/memory/:user/conflict_radar", post(conflict_radar))
        .route(
            "/api/v1/memory/:user/causal_recall",
            post(causal_recall_chains),
        )
        .route(
            "/api/v1/memory/:user/time_travel/trace",
            post(time_travel_trace),
        )
        .route("/api/v1/memory/webhooks", post(register_memory_webhook))
        .route(
            "/api/v1/memory/webhooks/:id",
            get(get_memory_webhook).delete(delete_memory_webhook),
        )
        .route(
            "/api/v1/memory/webhooks/:id/events",
            get(list_memory_webhook_events),
        )
        .route(
            "/api/v1/memory/webhooks/:id/events/replay",
            get(replay_memory_webhook_events),
        )
        .route(
            "/api/v1/memory/webhooks/:id/events/:event_id/retry",
            post(retry_memory_webhook_event),
        )
        .route(
            "/api/v1/memory/webhooks/:id/events/dead-letter",
            get(list_memory_webhook_dead_letters),
        )
        .route(
            "/api/v1/memory/webhooks/:id/audit",
            get(list_memory_webhook_audit),
        )
        .route(
            "/api/v1/memory/webhooks/:id/stats",
            get(get_memory_webhook_stats),
        )
        // Import API
        .route("/api/v1/import/chat-history", post(import_chat_history))
        .route("/api/v1/import/jobs/:job_id", get(get_import_job))
        // Agent identity substrate (P0)
        .route("/api/v1/agents/:agent_id/identity", get(get_agent_identity))
        .route(
            "/api/v1/agents/:agent_id/identity",
            put(update_agent_identity),
        )
        .route(
            "/api/v1/agents/:agent_id/identity/versions",
            get(list_agent_identity_versions),
        )
        .route(
            "/api/v1/agents/:agent_id/identity/audit",
            get(list_agent_identity_audit),
        )
        .route(
            "/api/v1/agents/:agent_id/identity/rollback",
            post(rollback_agent_identity),
        )
        .route(
            "/api/v1/agents/:agent_id/experience",
            post(add_agent_experience),
        )
        .route(
            "/api/v1/agents/:agent_id/promotions",
            post(create_promotion_proposal).get(list_promotion_proposals),
        )
        .route(
            "/api/v1/agents/:agent_id/promotions/:proposal_id/approve",
            post(approve_promotion_proposal),
        )
        .route(
            "/api/v1/agents/:agent_id/promotions/:proposal_id/reject",
            post(reject_promotion_proposal),
        )
        .route("/api/v1/agents/:agent_id/context", post(get_agent_context))
        // Graph
        .route("/api/v1/entities/:id/subgraph", get(get_subgraph))
        // Allow larger request bodies for import payloads.
        .layer(DefaultBodyLimit::max(64 * 1024 * 1024))
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

    emit_memory_webhook_event(
        &state,
        session.user_id,
        MemoryWebhookEventType::HeadAdvanced,
        serde_json::json!({
            "session_id": session.id,
            "session_name": session.name,
            "head_episode_id": episode.id,
            "head_version": session.head_version.saturating_add(1)
        }),
    )
    .await;

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

    if let Some(last) = episodes.last() {
        emit_memory_webhook_event(
            &state,
            session.user_id,
            MemoryWebhookEventType::HeadAdvanced,
            serde_json::json!({
                "session_id": session.id,
                "session_name": session.name,
                "head_episode_id": last.id,
                "head_version": session.head_version.saturating_add(episodes.len() as u64)
            }),
        )
        .await;
    }

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

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ChatHistorySource {
    Ndjson,
    ChatgptExport,
    GeminiExport,
}

impl ChatHistorySource {
    fn as_str(&self) -> &'static str {
        match self {
            ChatHistorySource::Ndjson => "ndjson",
            ChatHistorySource::ChatgptExport => "chatgpt_export",
            ChatHistorySource::GeminiExport => "gemini_export",
        }
    }
}

#[derive(Debug, Clone)]
struct ImportMessage {
    session: Option<String>,
    role: MessageRole,
    content: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Deserialize)]
struct ImportChatHistoryRequest {
    user: String,
    source: ChatHistorySource,
    payload: serde_json::Value,
    #[serde(default)]
    default_session: Option<String>,
    #[serde(default)]
    dry_run: bool,
    #[serde(default)]
    idempotency_key: Option<String>,
}

#[derive(Serialize)]
struct ImportChatHistoryResponse {
    ok: bool,
    job_id: Uuid,
    status: ImportJobStatus,
}

#[derive(Debug, Deserialize)]
struct RegisterMemoryWebhookRequest {
    user: String,
    target_url: String,
    #[serde(default)]
    signing_secret: Option<String>,
    #[serde(default)]
    events: Option<Vec<MemoryWebhookEventType>>,
    #[serde(default = "default_true")]
    enabled: bool,
}

#[derive(Debug, Serialize)]
struct RegisterMemoryWebhookResponse {
    ok: bool,
    webhook: MemoryWebhookSubscription,
}

#[derive(Debug, Serialize)]
struct DeleteMemoryWebhookResponse {
    deleted: bool,
}

#[derive(Debug, Deserialize)]
struct ListWebhookEventsQuery {
    #[serde(default)]
    limit: Option<u32>,
    #[serde(default)]
    event_type: Option<MemoryWebhookEventType>,
}

#[derive(Debug, Serialize)]
struct ListWebhookEventsResponse {
    webhook_id: Uuid,
    count: usize,
    events: Vec<MemoryWebhookEventRecord>,
}

#[derive(Debug, Deserialize)]
struct ReplayWebhookEventsQuery {
    #[serde(default)]
    after_event_id: Option<Uuid>,
    #[serde(default)]
    limit: Option<u32>,
    #[serde(default)]
    include_delivered: Option<bool>,
    #[serde(default)]
    include_dead_letter: Option<bool>,
}

#[derive(Debug, Serialize)]
struct ReplayWebhookEventsResponse {
    webhook_id: Uuid,
    count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_after_event_id: Option<Uuid>,
    events: Vec<MemoryWebhookEventRecord>,
}

#[derive(Debug, Deserialize)]
struct RetryWebhookEventRequest {
    #[serde(default)]
    force: Option<bool>,
}

#[derive(Debug, Serialize)]
struct RetryWebhookEventResponse {
    webhook_id: Uuid,
    event_id: Uuid,
    queued: bool,
    reason: String,
}

#[derive(Debug, Deserialize)]
struct WebhookAuditQuery {
    #[serde(default)]
    limit: Option<u32>,
}

#[derive(Debug, Serialize)]
struct WebhookAuditResponse {
    webhook_id: Uuid,
    count: usize,
    audit: Vec<MemoryWebhookAuditRecord>,
}

#[derive(Debug, Deserialize)]
struct WebhookStatsQuery {
    #[serde(default)]
    window_seconds: Option<u64>,
}

#[derive(Debug, Serialize)]
struct WebhookStatsResponse {
    webhook_id: Uuid,
    total_events: usize,
    delivered_events: usize,
    pending_events: usize,
    dead_letter_events: usize,
    failed_events: usize,
    recent_failures: usize,
    circuit_open: bool,
    circuit_open_until: Option<chrono::DateTime<chrono::Utc>>,
    rate_limit_per_minute: u32,
}

#[derive(Debug, Deserialize)]
struct MemoryChangesSinceRequest {
    from: chrono::DateTime<chrono::Utc>,
    to: chrono::DateTime<chrono::Utc>,
    #[serde(default)]
    session: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TimeTravelTraceRequest {
    query: String,
    from: chrono::DateTime<chrono::Utc>,
    to: chrono::DateTime<chrono::Utc>,
    #[serde(default)]
    session: Option<String>,
    #[serde(default)]
    max_tokens: Option<u32>,
    #[serde(default)]
    min_relevance: Option<f32>,
    #[serde(default)]
    contract: Option<MemoryContract>,
    #[serde(default)]
    retrieval_policy: Option<AdaptiveRetrievalPolicy>,
}

#[derive(Debug, Serialize)]
struct TimeTravelTraceResponse {
    user_id: Uuid,
    query: String,
    from: chrono::DateTime<chrono::Utc>,
    to: chrono::DateTime<chrono::Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    session: Option<String>,
    contract_applied: MemoryContract,
    retrieval_policy_applied: AdaptiveRetrievalPolicy,
    retrieval_policy_diagnostics: RetrievalPolicyDiagnostics,
    snapshot_from: TimeTravelSnapshot,
    snapshot_to: TimeTravelSnapshot,
    gained_facts: Vec<FactSummary>,
    lost_facts: Vec<FactSummary>,
    gained_episodes: Vec<EpisodeSummary>,
    lost_episodes: Vec<EpisodeSummary>,
    timeline: Vec<TimeTravelTimelineEvent>,
    summary: String,
}

#[derive(Debug, Serialize)]
struct TimeTravelSnapshot {
    as_of: chrono::DateTime<chrono::Utc>,
    token_count: u32,
    fact_count: usize,
    episode_count: usize,
    top_facts: Vec<FactSummary>,
    top_episodes: Vec<EpisodeSummary>,
}

#[derive(Debug, Serialize)]
struct TimeTravelTimelineEvent {
    at: chrono::DateTime<chrono::Utc>,
    event_type: String,
    description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    episode_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    edge_id: Option<Uuid>,
}

#[derive(Debug, Serialize)]
struct MemoryChangesSinceResponse {
    user_id: Uuid,
    from: chrono::DateTime<chrono::Utc>,
    to: chrono::DateTime<chrono::Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    session: Option<String>,
    added_facts: Vec<FactChange>,
    superseded_facts: Vec<FactChange>,
    confidence_deltas: Vec<ConfidenceDelta>,
    head_changes: Vec<HeadChange>,
    added_episodes: Vec<EpisodeChange>,
    summary: String,
}

#[derive(Debug, Serialize)]
struct FactChange {
    edge_id: Uuid,
    fact: String,
    label: String,
    source_entity: String,
    target_entity: String,
    confidence: f32,
    occurred_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize)]
struct ConfidenceDelta {
    source_entity: String,
    target_entity: String,
    label: String,
    previous_confidence: f32,
    current_confidence: f32,
    delta: f32,
    at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize)]
struct HeadChange {
    session_id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    head_episode_id: Option<Uuid>,
    head_version: u64,
    at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize)]
struct EpisodeChange {
    episode_id: Uuid,
    session_id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<MessageRole>,
    created_at: chrono::DateTime<chrono::Utc>,
    preview: String,
}

#[derive(Debug, Deserialize)]
struct ConflictRadarRequest {
    #[serde(default)]
    as_of: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default)]
    include_resolved: Option<bool>,
    #[serde(default)]
    max_items: Option<u32>,
}

#[derive(Debug, Serialize)]
struct ConflictRadarResponse {
    user_id: Uuid,
    as_of: chrono::DateTime<chrono::Utc>,
    conflicts: Vec<ConflictCluster>,
    summary: ConflictRadarSummary,
}

#[derive(Debug, Serialize)]
struct ConflictRadarSummary {
    clusters: usize,
    needs_resolution: usize,
    high_severity: usize,
}

#[derive(Debug, Serialize)]
struct ConflictCluster {
    source_entity: String,
    label: String,
    severity: f32,
    active_edge_count: usize,
    recent_supersessions: usize,
    needs_resolution: bool,
    reason: String,
    edges: Vec<ConflictEdge>,
}

#[derive(Debug, Serialize)]
struct ConflictEdge {
    edge_id: Uuid,
    target_entity: String,
    fact: String,
    confidence: f32,
    valid_at: chrono::DateTime<chrono::Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    invalid_at: Option<chrono::DateTime<chrono::Utc>>,
    is_active: bool,
}

#[derive(Debug, Deserialize)]
struct CausalRecallRequest {
    query: String,
    #[serde(default)]
    session: Option<String>,
    #[serde(default)]
    max_tokens: Option<u32>,
    #[serde(default)]
    mode: Option<MemoryContextMode>,
    #[serde(default)]
    time_intent: Option<TemporalIntent>,
    #[serde(default)]
    as_of: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Serialize)]
struct CausalRecallResponse {
    query: String,
    user_id: Uuid,
    mode: MemoryContextMode,
    retrieval_sources: Vec<RetrievalSource>,
    chains: Vec<CausalRecallChain>,
    summary: String,
}

#[derive(Debug, Serialize)]
struct CausalRecallChain {
    id: String,
    confidence: f32,
    reason: String,
    fact: CausalFact,
    source_episodes: Vec<CausalEpisode>,
}

#[derive(Debug, Serialize)]
struct CausalFact {
    fact_id: Uuid,
    source_entity: String,
    target_entity: String,
    label: String,
    text: String,
    valid_at: chrono::DateTime<chrono::Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    invalid_at: Option<chrono::DateTime<chrono::Utc>>,
    relevance: f32,
}

#[derive(Debug, Serialize)]
struct CausalEpisode {
    episode_id: Uuid,
    session_id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    relevance: f32,
    preview: String,
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
    #[serde(default)]
    contract: Option<MemoryContract>,
    #[serde(default)]
    retrieval_policy: Option<AdaptiveRetrievalPolicy>,
    #[serde(default)]
    filters: Option<MemoryContextFilters>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct MemoryContextFilters {
    #[serde(default)]
    roles: Option<Vec<MessageRole>>,
    #[serde(default)]
    tags_any: Option<Vec<String>>,
    #[serde(default)]
    tags_all: Option<Vec<String>>,
    #[serde(default)]
    created_after: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default)]
    created_before: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default)]
    processing_status: Option<ProcessingStatus>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum MemoryContextMode {
    Head,
    Hybrid,
    Historical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum MemoryContract {
    Default,
    SupportSafe,
    CurrentStrict,
    HistoricalStrict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum AdaptiveRetrievalPolicy {
    Balanced,
    Precision,
    Recall,
    Stability,
}

#[derive(Debug, Serialize)]
struct RetrievalPolicyDiagnostics {
    effective_max_tokens: u32,
    effective_min_relevance: f32,
    effective_temporal_intent: TemporalIntent,
    #[serde(skip_serializing_if = "Option::is_none")]
    effective_temporal_weight: Option<f32>,
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
    contract_applied: MemoryContract,
    retrieval_policy_applied: AdaptiveRetrievalPolicy,
    retrieval_policy_diagnostics: RetrievalPolicyDiagnostics,
    #[serde(skip_serializing_if = "Option::is_none")]
    head: Option<MemoryHeadInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata_filter_diagnostics: Option<MetadataFilterDiagnostics>,
}

#[derive(Deserialize)]
struct AgentContextRequest {
    user: String,
    query: String,
    #[serde(default)]
    session: Option<String>,
    #[serde(default)]
    max_tokens: Option<u32>,
    #[serde(default)]
    min_relevance: Option<f32>,
    #[serde(default)]
    mode: Option<MemoryContextMode>,
    #[serde(default)]
    time_intent: Option<TemporalIntent>,
    #[serde(default)]
    as_of: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default)]
    temporal_weight: Option<f32>,
}

#[derive(Serialize)]
struct AgentContextResponse {
    #[serde(flatten)]
    context: ContextBlock,
    identity: AgentIdentityProfile,
    identity_version: u64,
    experience_events_used: u32,
    experience_weight_sum: f32,
    user_memory_items_used: u32,
    attribution_guards: serde_json::Value,
}

#[derive(Serialize)]
struct MetadataFilterDiagnostics {
    prefilter_enabled: bool,
    candidate_count_before_filters: u32,
    candidate_count_after_filters: u32,
    candidate_reduction_ratio: f32,
    planner_latency_ms: u64,
    relaxed_fallback_applied: bool,
    applied_filters: serde_json::Value,
}

#[derive(Deserialize)]
struct ListLimitQuery {
    limit: Option<u32>,
}

#[derive(Deserialize)]
struct RejectPromotionRequest {
    #[serde(default)]
    reason: Option<String>,
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
                Err(MnemoError::Duplicate(_)) => {
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

    emit_memory_webhook_event(
        &state,
        user.id,
        MemoryWebhookEventType::HeadAdvanced,
        serde_json::json!({
            "session_id": session.id,
            "session_name": session.name,
            "head_episode_id": episode.id,
            "head_version": session.head_version.saturating_add(1)
        }),
    )
    .await;

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

async fn import_chat_history(
    State(state): State<AppState>,
    Json(req): Json<ImportChatHistoryRequest>,
) -> Result<(StatusCode, Json<ImportChatHistoryResponse>), AppError> {
    if req.user.trim().is_empty() {
        return Err(AppError(MnemoError::Validation("user is required".into())));
    }

    let user = req.user.trim().to_string();
    let idempotency_key = req
        .idempotency_key
        .as_ref()
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty());

    if let Some(idempotency_key) = idempotency_key.as_ref() {
        let scoped_key = format!("{}:{}", user, idempotency_key);
        let existing_job_id = {
            let idempotency = state.import_idempotency.read().await;
            idempotency.get(&scoped_key).copied()
        };

        if let Some(existing_job_id) = existing_job_id {
            let existing_status = {
                let jobs = state.import_jobs.read().await;
                jobs.get(&existing_job_id).map(|j| j.status.clone())
            };
            if let Some(status) = existing_status {
                return Ok((
                    StatusCode::OK,
                    Json(ImportChatHistoryResponse {
                        ok: true,
                        job_id: existing_job_id,
                        status,
                    }),
                ));
            }
        }
    }

    let job_id = Uuid::now_v7();
    let now = chrono::Utc::now();
    let record = ImportJobRecord {
        id: job_id,
        source: req.source.as_str().to_string(),
        user: user.clone(),
        dry_run: req.dry_run,
        status: ImportJobStatus::Queued,
        total_messages: 0,
        imported_messages: 0,
        failed_messages: 0,
        sessions_touched: 0,
        errors: Vec::new(),
        created_at: now,
        started_at: None,
        finished_at: None,
    };

    {
        let mut jobs = state.import_jobs.write().await;
        jobs.insert(job_id, record);
    }

    if let Some(idempotency_key) = idempotency_key {
        let scoped_key = format!("{}:{}", user, idempotency_key);
        let mut idempotency = state.import_idempotency.write().await;
        idempotency.insert(scoped_key, job_id);
    }

    let source = req.source;
    let payload = req.payload;
    let default_session = req.default_session;
    let dry_run = req.dry_run;
    let state_for_job = state.clone();

    tokio::spawn(async move {
        run_import_job(
            state_for_job,
            job_id,
            source,
            user,
            payload,
            default_session,
            dry_run,
        )
        .await;
    });

    Ok((
        StatusCode::ACCEPTED,
        Json(ImportChatHistoryResponse {
            ok: true,
            job_id,
            status: ImportJobStatus::Queued,
        }),
    ))
}

async fn get_import_job(
    State(state): State<AppState>,
    Path(job_id): Path<Uuid>,
) -> Result<Json<ImportJobRecord>, AppError> {
    let jobs = state.import_jobs.read().await;
    let record = jobs.get(&job_id).cloned().ok_or_else(|| {
        AppError(MnemoError::NotFound {
            resource_type: "ImportJob".into(),
            id: job_id.to_string(),
        })
    })?;
    Ok(Json(record))
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
    let contract = req.contract.unwrap_or(MemoryContract::Default);
    let retrieval_policy = req
        .retrieval_policy
        .unwrap_or(AdaptiveRetrievalPolicy::Balanced);
    if matches!(contract, MemoryContract::HistoricalStrict) && req.as_of.is_none() {
        return Err(AppError(MnemoError::Validation(
            "historical_strict contract requires as_of".into(),
        )));
    }
    let scoped_session =
        resolve_session_scope(&state, user.id, mode, requested_session_name).await?;
    let mut session_id = scoped_session.as_ref().map(|s| s.id);

    let planner_started = std::time::Instant::now();
    let (candidate_episode_ids, metadata_filter_diagnostics) =
        if let Some(filters) = req.filters.as_ref() {
            if !state.metadata_prefilter.enabled {
                (
                    None,
                    Some(MetadataFilterDiagnostics {
                        prefilter_enabled: false,
                        candidate_count_before_filters: 0,
                        candidate_count_after_filters: 0,
                        candidate_reduction_ratio: 0.0,
                        planner_latency_ms: 0,
                        relaxed_fallback_applied: false,
                        applied_filters: serde_json::to_value(filters)
                            .unwrap_or_else(|_| serde_json::json!({})),
                    }),
                )
            } else {
                let metadata = collect_metadata_candidates(
                    &state,
                    user.id,
                    session_id,
                    filters,
                    state.metadata_prefilter.scan_limit,
                )
                .await?;

                let mut filtered_episodes = metadata.filtered_episodes;
                let mut relaxed_fallback_applied = false;
                if filtered_episodes.is_empty() && state.metadata_prefilter.relax_if_empty {
                    filtered_episodes = metadata.scanned_episodes;
                    relaxed_fallback_applied = true;
                }

                if session_id.is_none() {
                    if let Some(scoped) = dominant_session(&filtered_episodes) {
                        session_id = Some(scoped);
                    }
                }

                let before = metadata.scanned_count;
                let after = filtered_episodes.len() as u32;
                let reduction_ratio = if before == 0 {
                    0.0
                } else {
                    ((before.saturating_sub(after)) as f32) / (before as f32)
                };

                (
                    Some(
                        filtered_episodes
                            .iter()
                            .map(|e| e.id)
                            .collect::<std::collections::HashSet<_>>(),
                    ),
                    Some(MetadataFilterDiagnostics {
                        prefilter_enabled: true,
                        candidate_count_before_filters: before,
                        candidate_count_after_filters: after,
                        candidate_reduction_ratio: reduction_ratio,
                        planner_latency_ms: planner_started.elapsed().as_millis() as u64,
                        relaxed_fallback_applied,
                        applied_filters: serde_json::to_value(filters)
                            .unwrap_or_else(|_| serde_json::json!({})),
                    }),
                )
            }
        } else {
            (None, None)
        };

    let default_max_tokens = match retrieval_policy {
        AdaptiveRetrievalPolicy::Balanced => 500,
        AdaptiveRetrievalPolicy::Precision => 400,
        AdaptiveRetrievalPolicy::Recall => 700,
        AdaptiveRetrievalPolicy::Stability => 500,
    };
    let max_tokens = req.max_tokens.unwrap_or(default_max_tokens);

    let base_temporal_intent = match contract {
        MemoryContract::CurrentStrict => TemporalIntent::Current,
        MemoryContract::HistoricalStrict => TemporalIntent::Historical,
        _ => req.time_intent.unwrap_or(TemporalIntent::Auto),
    };
    let temporal_intent = if matches!(retrieval_policy, AdaptiveRetrievalPolicy::Stability)
        && !matches!(contract, MemoryContract::HistoricalStrict)
        && req.time_intent.is_none()
    {
        TemporalIntent::Current
    } else {
        base_temporal_intent
    };

    let effective_temporal_weight = req.temporal_weight.or(match retrieval_policy {
        AdaptiveRetrievalPolicy::Balanced => None,
        AdaptiveRetrievalPolicy::Precision => Some(0.35),
        AdaptiveRetrievalPolicy::Recall => Some(0.2),
        AdaptiveRetrievalPolicy::Stability => Some(0.8),
    });

    let min_relevance = req.min_relevance.unwrap_or(match retrieval_policy {
        AdaptiveRetrievalPolicy::Balanced => 0.3,
        AdaptiveRetrievalPolicy::Precision => 0.55,
        AdaptiveRetrievalPolicy::Recall => 0.15,
        AdaptiveRetrievalPolicy::Stability => 0.4,
    });
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
        temporal_weight: effective_temporal_weight,
        min_relevance,
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

    if let Some(candidate_episode_ids) = candidate_episode_ids.as_ref() {
        context
            .episodes
            .retain(|episode| candidate_episode_ids.contains(&episode.id));
    }

    apply_memory_contract(&mut context, contract);

    let head = scoped_session.as_ref().map(|session| MemoryHeadInfo {
        session_id: session.id,
        episode_id: session.head_episode_id,
        updated_at: session.head_updated_at.or(session.last_activity_at),
        version: session.head_version,
    });

    Ok(Json(MemoryContextResponse {
        context,
        mode,
        contract_applied: contract,
        retrieval_policy_applied: retrieval_policy,
        retrieval_policy_diagnostics: RetrievalPolicyDiagnostics {
            effective_max_tokens: max_tokens,
            effective_min_relevance: min_relevance,
            effective_temporal_intent: temporal_intent,
            effective_temporal_weight,
        },
        head,
        metadata_filter_diagnostics,
    }))
}

async fn memory_changes_since(
    State(state): State<AppState>,
    Path(user_identifier): Path<String>,
    Json(req): Json<MemoryChangesSinceRequest>,
) -> Result<Json<MemoryChangesSinceResponse>, AppError> {
    if req.to <= req.from {
        return Err(AppError(MnemoError::Validation(
            "'to' must be after 'from'".to_string(),
        )));
    }

    let user = find_user_by_identifier(&state, user_identifier.trim()).await?;
    let sessions = list_all_sessions_for_user(&state, user.id).await?;

    let scoped_sessions: Vec<Session> = if let Some(session_scope) = req
        .session
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        let matched: Vec<Session> = sessions
            .iter()
            .filter(|session| {
                session.id.to_string() == session_scope
                    || session
                        .name
                        .as_ref()
                        .map(|n| n == &session_scope)
                        .unwrap_or(false)
            })
            .cloned()
            .collect();
        if matched.is_empty() {
            return Err(AppError(MnemoError::NotFound {
                resource_type: "Session".into(),
                id: session_scope,
            }));
        }
        matched
    } else {
        sessions.clone()
    };

    let mut session_name_by_id: HashMap<Uuid, Option<String>> = HashMap::new();
    for session in &scoped_sessions {
        session_name_by_id.insert(session.id, session.name.clone());
    }

    let mut added_episodes: Vec<EpisodeChange> = Vec::new();
    let mut episode_session_by_id: HashMap<Uuid, Uuid> = HashMap::new();
    for session in &scoped_sessions {
        let episodes = list_all_episodes_for_session(&state, session.id).await?;
        for episode in episodes {
            episode_session_by_id.insert(episode.id, episode.session_id);
            if episode.created_at > req.from && episode.created_at <= req.to {
                added_episodes.push(EpisodeChange {
                    episode_id: episode.id,
                    session_id: episode.session_id,
                    session_name: session.name.clone(),
                    role: episode.role,
                    created_at: episode.created_at,
                    preview: preview_text(&episode.content, 140),
                });
            }
        }
    }

    let mut head_changes = Vec::new();
    for session in &scoped_sessions {
        if let Some(at) = session.head_updated_at {
            if at > req.from && at <= req.to {
                head_changes.push(HeadChange {
                    session_id: session.id,
                    session_name: session.name.clone(),
                    head_episode_id: session.head_episode_id,
                    head_version: session.head_version,
                    at,
                });
            }
        }
    }

    let edges = state
        .state_store
        .query_edges(
            user.id,
            EdgeFilter {
                include_invalidated: true,
                limit: 10_000,
                ..EdgeFilter::default()
            },
        )
        .await?;

    let entities = list_all_entities_for_user(&state, user.id).await?;
    let entity_name_by_id: HashMap<Uuid, String> =
        entities.into_iter().map(|e| (e.id, e.name)).collect();

    let in_scope_edge = |edge: &Edge| {
        if req.session.is_none() {
            return true;
        }
        episode_session_by_id
            .get(&edge.source_episode_id)
            .map(|sid| session_name_by_id.contains_key(sid))
            .unwrap_or(false)
    };

    let mut added_facts = Vec::new();
    let mut superseded_facts = Vec::new();
    for edge in &edges {
        if !in_scope_edge(edge) {
            continue;
        }

        if edge.valid_at > req.from && edge.valid_at <= req.to {
            added_facts.push(FactChange {
                edge_id: edge.id,
                fact: edge.fact.clone(),
                label: edge.label.clone(),
                source_entity: entity_name_by_id
                    .get(&edge.source_entity_id)
                    .cloned()
                    .unwrap_or_else(|| edge.source_entity_id.to_string()),
                target_entity: entity_name_by_id
                    .get(&edge.target_entity_id)
                    .cloned()
                    .unwrap_or_else(|| edge.target_entity_id.to_string()),
                confidence: edge.confidence,
                occurred_at: edge.valid_at,
            });
        }

        if let Some(invalid_at) = edge.invalid_at {
            if invalid_at > req.from && invalid_at <= req.to {
                superseded_facts.push(FactChange {
                    edge_id: edge.id,
                    fact: edge.fact.clone(),
                    label: edge.label.clone(),
                    source_entity: entity_name_by_id
                        .get(&edge.source_entity_id)
                        .cloned()
                        .unwrap_or_else(|| edge.source_entity_id.to_string()),
                    target_entity: entity_name_by_id
                        .get(&edge.target_entity_id)
                        .cloned()
                        .unwrap_or_else(|| edge.target_entity_id.to_string()),
                    confidence: edge.confidence,
                    occurred_at: invalid_at,
                });
            }
        }
    }

    let mut grouped: HashMap<(Uuid, Uuid, String), Vec<&Edge>> = HashMap::new();
    for edge in &edges {
        if !in_scope_edge(edge) {
            continue;
        }
        grouped
            .entry((
                edge.source_entity_id,
                edge.target_entity_id,
                edge.label.clone(),
            ))
            .or_default()
            .push(edge);
    }

    let mut confidence_deltas = Vec::new();
    for ((src_id, tgt_id, label), mut group) in grouped {
        group.sort_by(|a, b| a.valid_at.cmp(&b.valid_at));
        for pair in group.windows(2) {
            let previous = pair[0];
            let current = pair[1];
            if current.valid_at > req.from && current.valid_at <= req.to {
                confidence_deltas.push(ConfidenceDelta {
                    source_entity: entity_name_by_id
                        .get(&src_id)
                        .cloned()
                        .unwrap_or_else(|| src_id.to_string()),
                    target_entity: entity_name_by_id
                        .get(&tgt_id)
                        .cloned()
                        .unwrap_or_else(|| tgt_id.to_string()),
                    label: label.clone(),
                    previous_confidence: previous.confidence,
                    current_confidence: current.confidence,
                    delta: current.confidence - previous.confidence,
                    at: current.valid_at,
                });
            }
        }
    }

    let summary = format!(
        "{} added facts, {} superseded facts, {} confidence deltas, {} head changes, {} added episodes",
        added_facts.len(),
        superseded_facts.len(),
        confidence_deltas.len(),
        head_changes.len(),
        added_episodes.len()
    );

    if !added_facts.is_empty() {
        emit_memory_webhook_event(
            &state,
            user.id,
            MemoryWebhookEventType::FactAdded,
            serde_json::json!({
                "from": req.from,
                "to": req.to,
                "session": req.session.clone(),
                "count": added_facts.len()
            }),
        )
        .await;
    }

    if !superseded_facts.is_empty() {
        emit_memory_webhook_event(
            &state,
            user.id,
            MemoryWebhookEventType::FactSuperseded,
            serde_json::json!({
                "from": req.from,
                "to": req.to,
                "session": req.session.clone(),
                "count": superseded_facts.len()
            }),
        )
        .await;
    }

    Ok(Json(MemoryChangesSinceResponse {
        user_id: user.id,
        from: req.from,
        to: req.to,
        session: req.session,
        added_facts,
        superseded_facts,
        confidence_deltas,
        head_changes,
        added_episodes,
        summary,
    }))
}

async fn time_travel_trace(
    State(state): State<AppState>,
    Path(user_identifier): Path<String>,
    Json(req): Json<TimeTravelTraceRequest>,
) -> Result<Json<TimeTravelTraceResponse>, AppError> {
    if req.query.trim().is_empty() {
        return Err(AppError(MnemoError::Validation("query is required".into())));
    }
    if req.to <= req.from {
        return Err(AppError(MnemoError::Validation(
            "'to' must be after 'from'".to_string(),
        )));
    }

    let user = find_user_by_identifier(&state, user_identifier.trim()).await?;
    let contract = req.contract.unwrap_or(MemoryContract::Default);
    let retrieval_policy = req
        .retrieval_policy
        .unwrap_or(AdaptiveRetrievalPolicy::Balanced);
    let requested_session_name = req
        .session
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let scoped_session = resolve_session_scope(
        &state,
        user.id,
        MemoryContextMode::Hybrid,
        requested_session_name,
    )
    .await?;
    let session_id = scoped_session.as_ref().map(|s| s.id);

    let default_max_tokens = match retrieval_policy {
        AdaptiveRetrievalPolicy::Balanced => 500,
        AdaptiveRetrievalPolicy::Precision => 400,
        AdaptiveRetrievalPolicy::Recall => 700,
        AdaptiveRetrievalPolicy::Stability => 500,
    };
    let max_tokens = req.max_tokens.unwrap_or(default_max_tokens);

    let base_temporal_intent = match contract {
        MemoryContract::CurrentStrict => TemporalIntent::Current,
        MemoryContract::HistoricalStrict => TemporalIntent::Historical,
        _ => TemporalIntent::Historical,
    };
    let temporal_intent = if matches!(retrieval_policy, AdaptiveRetrievalPolicy::Stability)
        && !matches!(contract, MemoryContract::HistoricalStrict)
    {
        TemporalIntent::Current
    } else {
        base_temporal_intent
    };

    let temporal_weight = match retrieval_policy {
        AdaptiveRetrievalPolicy::Balanced => None,
        AdaptiveRetrievalPolicy::Precision => Some(0.35),
        AdaptiveRetrievalPolicy::Recall => Some(0.2),
        AdaptiveRetrievalPolicy::Stability => Some(0.8),
    };

    let min_relevance = req.min_relevance.unwrap_or(match retrieval_policy {
        AdaptiveRetrievalPolicy::Balanced => 0.3,
        AdaptiveRetrievalPolicy::Precision => 0.55,
        AdaptiveRetrievalPolicy::Recall => 0.15,
        AdaptiveRetrievalPolicy::Stability => 0.4,
    });

    let make_context_req = |as_of: chrono::DateTime<chrono::Utc>| ContextRequest {
        session_id,
        messages: vec![ContextMessage {
            role: "user".to_string(),
            content: req.query.clone(),
        }],
        max_tokens,
        search_types: vec![mnemo_core::models::context::SearchType::Hybrid],
        temporal_filter: Some(as_of),
        as_of: Some(as_of),
        time_intent: temporal_intent,
        temporal_weight,
        min_relevance,
    };

    let mut context_from = state
        .retrieval
        .get_context(user.id, &make_context_req(req.from))
        .await?;
    maybe_attach_recent_episode_fallback(
        &state,
        user.id,
        session_id,
        max_tokens,
        temporal_intent,
        Some(req.from),
        &mut context_from,
    )
    .await?;
    apply_memory_contract(&mut context_from, contract);

    let mut context_to = state
        .retrieval
        .get_context(user.id, &make_context_req(req.to))
        .await?;
    maybe_attach_recent_episode_fallback(
        &state,
        user.id,
        session_id,
        max_tokens,
        temporal_intent,
        Some(req.to),
        &mut context_to,
    )
    .await?;
    apply_memory_contract(&mut context_to, contract);

    let from_facts: HashMap<Uuid, FactSummary> = context_from
        .facts
        .iter()
        .cloned()
        .map(|f| (f.id, f))
        .collect();
    let to_facts: HashMap<Uuid, FactSummary> = context_to
        .facts
        .iter()
        .cloned()
        .map(|f| (f.id, f))
        .collect();
    let gained_facts: Vec<FactSummary> = to_facts
        .iter()
        .filter(|(id, _)| !from_facts.contains_key(id))
        .map(|(_, fact)| fact.clone())
        .collect();
    let lost_facts: Vec<FactSummary> = from_facts
        .iter()
        .filter(|(id, _)| !to_facts.contains_key(id))
        .map(|(_, fact)| fact.clone())
        .collect();

    let from_episodes: HashMap<Uuid, EpisodeSummary> = context_from
        .episodes
        .iter()
        .cloned()
        .map(|e| (e.id, e))
        .collect();
    let to_episodes: HashMap<Uuid, EpisodeSummary> = context_to
        .episodes
        .iter()
        .cloned()
        .map(|e| (e.id, e))
        .collect();
    let gained_episodes: Vec<EpisodeSummary> = to_episodes
        .iter()
        .filter(|(id, _)| !from_episodes.contains_key(id))
        .map(|(_, episode)| episode.clone())
        .collect();
    let lost_episodes: Vec<EpisodeSummary> = from_episodes
        .iter()
        .filter(|(id, _)| !to_episodes.contains_key(id))
        .map(|(_, episode)| episode.clone())
        .collect();

    let sessions = list_all_sessions_for_user(&state, user.id).await?;
    let scoped_sessions: Vec<Session> = if let Some(session_scope) = req
        .session
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        let matched: Vec<Session> = sessions
            .iter()
            .filter(|session| {
                session.id.to_string() == session_scope
                    || session
                        .name
                        .as_ref()
                        .map(|n| n == &session_scope)
                        .unwrap_or(false)
            })
            .cloned()
            .collect();
        if matched.is_empty() {
            return Err(AppError(MnemoError::NotFound {
                resource_type: "Session".into(),
                id: session_scope,
            }));
        }
        matched
    } else {
        sessions.clone()
    };

    let mut session_name_by_id: HashMap<Uuid, Option<String>> = HashMap::new();
    let mut episode_session_by_id: HashMap<Uuid, Uuid> = HashMap::new();
    let mut timeline: Vec<TimeTravelTimelineEvent> = Vec::new();
    for session in &scoped_sessions {
        session_name_by_id.insert(session.id, session.name.clone());
        let episodes = list_all_episodes_for_session(&state, session.id).await?;
        for episode in episodes {
            episode_session_by_id.insert(episode.id, episode.session_id);
            if episode.created_at > req.from && episode.created_at <= req.to {
                timeline.push(TimeTravelTimelineEvent {
                    at: episode.created_at,
                    event_type: "episode_added".to_string(),
                    description: format!(
                        "Episode added in session '{}'",
                        session.name.as_deref().unwrap_or("unknown")
                    ),
                    session_id: Some(session.id),
                    episode_id: Some(episode.id),
                    edge_id: None,
                });
            }
        }

        if let Some(at) = session.head_updated_at {
            if at > req.from && at <= req.to {
                timeline.push(TimeTravelTimelineEvent {
                    at,
                    event_type: "head_advanced".to_string(),
                    description: format!(
                        "Session '{}' head moved to version {}",
                        session.name.as_deref().unwrap_or("unknown"),
                        session.head_version
                    ),
                    session_id: Some(session.id),
                    episode_id: session.head_episode_id,
                    edge_id: None,
                });
            }
        }
    }

    let edges = state
        .state_store
        .query_edges(
            user.id,
            EdgeFilter {
                include_invalidated: true,
                limit: 10_000,
                ..EdgeFilter::default()
            },
        )
        .await?;
    let entities = list_all_entities_for_user(&state, user.id).await?;
    let entity_name_by_id: HashMap<Uuid, String> =
        entities.into_iter().map(|e| (e.id, e.name)).collect();

    let in_scope_edge = |edge: &Edge| {
        if req.session.is_none() {
            return true;
        }
        episode_session_by_id
            .get(&edge.source_episode_id)
            .map(|sid| session_name_by_id.contains_key(sid))
            .unwrap_or(false)
    };

    for edge in &edges {
        if !in_scope_edge(edge) {
            continue;
        }
        let source_entity = entity_name_by_id
            .get(&edge.source_entity_id)
            .cloned()
            .unwrap_or_else(|| edge.source_entity_id.to_string());
        let target_entity = entity_name_by_id
            .get(&edge.target_entity_id)
            .cloned()
            .unwrap_or_else(|| edge.target_entity_id.to_string());

        if edge.valid_at > req.from && edge.valid_at <= req.to {
            timeline.push(TimeTravelTimelineEvent {
                at: edge.valid_at,
                event_type: "fact_added".to_string(),
                description: format!("{} {} {}", source_entity, edge.label, target_entity),
                session_id: episode_session_by_id.get(&edge.source_episode_id).copied(),
                episode_id: Some(edge.source_episode_id),
                edge_id: Some(edge.id),
            });
        }
        if let Some(invalid_at) = edge.invalid_at {
            if invalid_at > req.from && invalid_at <= req.to {
                timeline.push(TimeTravelTimelineEvent {
                    at: invalid_at,
                    event_type: "fact_superseded".to_string(),
                    description: format!(
                        "Superseded: {} {} {}",
                        source_entity, edge.label, target_entity
                    ),
                    session_id: episode_session_by_id.get(&edge.source_episode_id).copied(),
                    episode_id: Some(edge.source_episode_id),
                    edge_id: Some(edge.id),
                });
            }
        }
    }

    timeline.sort_by(|a, b| {
        a.at.cmp(&b.at)
            .then_with(|| a.event_type.cmp(&b.event_type))
    });

    let snapshot_from = TimeTravelSnapshot {
        as_of: req.from,
        token_count: context_from.token_count,
        fact_count: context_from.facts.len(),
        episode_count: context_from.episodes.len(),
        top_facts: context_from.facts.iter().take(8).cloned().collect(),
        top_episodes: context_from.episodes.iter().take(8).cloned().collect(),
    };
    let snapshot_to = TimeTravelSnapshot {
        as_of: req.to,
        token_count: context_to.token_count,
        fact_count: context_to.facts.len(),
        episode_count: context_to.episodes.len(),
        top_facts: context_to.facts.iter().take(8).cloned().collect(),
        top_episodes: context_to.episodes.iter().take(8).cloned().collect(),
    };

    let summary = format!(
        "{} timeline events; {} gained facts, {} lost facts; {} gained episodes, {} lost episodes",
        timeline.len(),
        gained_facts.len(),
        lost_facts.len(),
        gained_episodes.len(),
        lost_episodes.len()
    );

    Ok(Json(TimeTravelTraceResponse {
        user_id: user.id,
        query: req.query,
        from: req.from,
        to: req.to,
        session: req.session,
        contract_applied: contract,
        retrieval_policy_applied: retrieval_policy,
        retrieval_policy_diagnostics: RetrievalPolicyDiagnostics {
            effective_max_tokens: max_tokens,
            effective_min_relevance: min_relevance,
            effective_temporal_intent: temporal_intent,
            effective_temporal_weight: temporal_weight,
        },
        snapshot_from,
        snapshot_to,
        gained_facts,
        lost_facts,
        gained_episodes,
        lost_episodes,
        timeline,
        summary,
    }))
}

async fn conflict_radar(
    State(state): State<AppState>,
    Path(user_identifier): Path<String>,
    Json(req): Json<ConflictRadarRequest>,
) -> Result<Json<ConflictRadarResponse>, AppError> {
    let user = find_user_by_identifier(&state, user_identifier.trim()).await?;
    let as_of = req.as_of.unwrap_or_else(chrono::Utc::now);
    let include_resolved = req.include_resolved.unwrap_or(false);
    let max_items = req.max_items.unwrap_or(50).clamp(1, 200) as usize;

    let edges = state
        .state_store
        .query_edges(
            user.id,
            EdgeFilter {
                include_invalidated: true,
                limit: 10_000,
                ..EdgeFilter::default()
            },
        )
        .await?;

    let entities = list_all_entities_for_user(&state, user.id).await?;
    let entity_name_by_id: HashMap<Uuid, String> =
        entities.into_iter().map(|e| (e.id, e.name)).collect();

    let mut grouped: HashMap<(Uuid, String), Vec<Edge>> = HashMap::new();
    for edge in edges {
        grouped
            .entry((edge.source_entity_id, edge.label.clone()))
            .or_default()
            .push(edge);
    }

    let mut conflicts = Vec::new();
    for ((source_entity_id, label), mut group) in grouped {
        group.sort_by(|a, b| a.valid_at.cmp(&b.valid_at));

        let active: Vec<&Edge> = group.iter().filter(|e| e.is_valid_at(as_of)).collect();
        let recent_supersessions = group
            .iter()
            .filter(|e| {
                e.invalid_at
                    .map(|t| t <= as_of && (as_of - t).num_days() <= 30)
                    .unwrap_or(false)
            })
            .count();

        let severity = if active.len() > 1 {
            (0.85 + ((active.len().saturating_sub(2)) as f32 * 0.05)).min(1.0)
        } else if recent_supersessions >= 2 {
            (0.6 + ((recent_supersessions.saturating_sub(2)) as f32 * 0.08)).min(0.9)
        } else if group.len() >= 3 {
            0.4
        } else {
            0.0
        };

        if !include_resolved && severity <= 0.0 {
            continue;
        }

        let needs_resolution = active.len() > 1 || recent_supersessions >= 3;
        let reason = if active.len() > 1 {
            "multiple simultaneously active facts".to_string()
        } else if recent_supersessions >= 2 {
            "frequent recent supersessions".to_string()
        } else if group.len() >= 3 {
            "high churn in fact lineage".to_string()
        } else {
            "resolved cluster".to_string()
        };

        let source_name = entity_name_by_id
            .get(&source_entity_id)
            .cloned()
            .unwrap_or_else(|| source_entity_id.to_string());

        let edges_view = group
            .iter()
            .map(|edge| ConflictEdge {
                edge_id: edge.id,
                target_entity: entity_name_by_id
                    .get(&edge.target_entity_id)
                    .cloned()
                    .unwrap_or_else(|| edge.target_entity_id.to_string()),
                fact: edge.fact.clone(),
                confidence: edge.confidence,
                valid_at: edge.valid_at,
                invalid_at: edge.invalid_at,
                is_active: edge.is_valid_at(as_of),
            })
            .collect::<Vec<_>>();

        conflicts.push(ConflictCluster {
            source_entity: source_name,
            label,
            severity,
            active_edge_count: active.len(),
            recent_supersessions,
            needs_resolution,
            reason,
            edges: edges_view,
        });
    }

    conflicts.sort_by(|a, b| {
        b.severity
            .partial_cmp(&a.severity)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.recent_supersessions.cmp(&a.recent_supersessions))
            .then_with(|| a.source_entity.cmp(&b.source_entity))
    });
    conflicts.truncate(max_items);

    let summary = ConflictRadarSummary {
        clusters: conflicts.len(),
        needs_resolution: conflicts.iter().filter(|c| c.needs_resolution).count(),
        high_severity: conflicts.iter().filter(|c| c.severity >= 0.8).count(),
    };

    if summary.needs_resolution > 0 {
        let top = conflicts.first();
        emit_memory_webhook_event(
            &state,
            user.id,
            MemoryWebhookEventType::ConflictDetected,
            serde_json::json!({
                "as_of": as_of,
                "clusters": summary.clusters,
                "needs_resolution": summary.needs_resolution,
                "high_severity": summary.high_severity,
                "top_cluster": top.map(|cluster| serde_json::json!({
                    "source_entity": cluster.source_entity,
                    "label": cluster.label,
                    "severity": cluster.severity,
                    "reason": cluster.reason
                }))
            }),
        )
        .await;
    }

    Ok(Json(ConflictRadarResponse {
        user_id: user.id,
        as_of,
        conflicts,
        summary,
    }))
}

async fn causal_recall_chains(
    State(state): State<AppState>,
    Path(user_identifier): Path<String>,
    Json(req): Json<CausalRecallRequest>,
) -> Result<Json<CausalRecallResponse>, AppError> {
    if req.query.trim().is_empty() {
        return Err(AppError(MnemoError::Validation("query is required".into())));
    }

    let user = find_user_by_identifier(&state, user_identifier.trim()).await?;
    let mode = req.mode.unwrap_or(MemoryContextMode::Hybrid);
    let requested_session_name = req.session.and_then(|s| {
        let trimmed = s.trim().to_string();
        (!trimmed.is_empty()).then_some(trimmed)
    });
    let scoped_session =
        resolve_session_scope(&state, user.id, mode, requested_session_name.clone()).await?;
    let session_id = scoped_session.as_ref().map(|s| s.id);
    let temporal_intent = req.time_intent.unwrap_or(TemporalIntent::Auto);
    let max_tokens = req.max_tokens.unwrap_or(700);

    let context_req = ContextRequest {
        session_id,
        messages: vec![ContextMessage {
            role: "user".to_string(),
            content: req.query.clone(),
        }],
        max_tokens,
        search_types: vec![mnemo_core::models::context::SearchType::Hybrid],
        temporal_filter: req.as_of,
        as_of: req.as_of,
        time_intent: temporal_intent,
        temporal_weight: None,
        min_relevance: 0.3,
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

    let query_terms = normalized_terms(&req.query);
    let mut chains: Vec<CausalRecallChain> = Vec::new();

    for fact in &context.facts {
        let mut linked_episodes = Vec::new();
        let source_l = fact.source_entity.to_lowercase();
        let target_l = fact.target_entity.to_lowercase();
        let label_l = fact.label.to_lowercase();

        for episode in &context.episodes {
            let preview_l = episode.preview.to_lowercase();
            if preview_l.contains(&source_l)
                || preview_l.contains(&target_l)
                || preview_l.contains(&label_l)
                || fact
                    .fact
                    .split_whitespace()
                    .take(6)
                    .map(|t| t.to_ascii_lowercase())
                    .any(|t| preview_l.contains(&t))
            {
                linked_episodes.push(CausalEpisode {
                    episode_id: episode.id,
                    session_id: episode.session_id,
                    role: episode.role.clone(),
                    created_at: episode.created_at,
                    relevance: episode.relevance,
                    preview: episode.preview.clone(),
                });
            }
        }

        if linked_episodes.is_empty() && !context.episodes.is_empty() {
            let top = &context.episodes[0];
            linked_episodes.push(CausalEpisode {
                episode_id: top.id,
                session_id: top.session_id,
                role: top.role.clone(),
                created_at: top.created_at,
                relevance: top.relevance,
                preview: top.preview.clone(),
            });
        }

        let mut confidence = fact.relevance;
        let fact_terms = normalized_terms(&fact.fact);
        let overlap = query_terms.intersection(&fact_terms).count() as f32;
        let denom = query_terms.len().max(1) as f32;
        confidence = (confidence + (overlap / denom)).min(1.0);

        let reason = format!(
            "Matched fact '{}' with {} supporting episode(s)",
            fact.label,
            linked_episodes.len()
        );

        chains.push(CausalRecallChain {
            id: format!("fact:{}", fact.id),
            confidence,
            reason,
            fact: CausalFact {
                fact_id: fact.id,
                source_entity: fact.source_entity.clone(),
                target_entity: fact.target_entity.clone(),
                label: fact.label.clone(),
                text: fact.fact.clone(),
                valid_at: fact.valid_at,
                invalid_at: fact.invalid_at,
                relevance: fact.relevance,
            },
            source_episodes: linked_episodes,
        });
    }

    if chains.is_empty() {
        for episode in context.episodes.iter().take(5) {
            chains.push(CausalRecallChain {
                id: format!("episode:{}", episode.id),
                confidence: episode.relevance,
                reason: "No graph facts available; using direct episode recall lineage".to_string(),
                fact: CausalFact {
                    fact_id: episode.id,
                    source_entity: "episode".to_string(),
                    target_entity: "context".to_string(),
                    label: "episode_recall".to_string(),
                    text: episode.preview.clone(),
                    valid_at: episode.created_at,
                    invalid_at: None,
                    relevance: episode.relevance,
                },
                source_episodes: vec![CausalEpisode {
                    episode_id: episode.id,
                    session_id: episode.session_id,
                    role: episode.role.clone(),
                    created_at: episode.created_at,
                    relevance: episode.relevance,
                    preview: episode.preview.clone(),
                }],
            });
        }
    }

    chains.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    chains.truncate(25);

    let summary = format!(
        "{} causal chains built from {} facts and {} episodes",
        chains.len(),
        context.facts.len(),
        context.episodes.len()
    );

    Ok(Json(CausalRecallResponse {
        query: req.query,
        user_id: user.id,
        mode,
        retrieval_sources: context.sources.clone(),
        chains,
        summary,
    }))
}

async fn register_memory_webhook(
    State(state): State<AppState>,
    Json(req): Json<RegisterMemoryWebhookRequest>,
) -> Result<(StatusCode, Json<RegisterMemoryWebhookResponse>), AppError> {
    if req.user.trim().is_empty() {
        return Err(AppError(MnemoError::Validation("user is required".into())));
    }
    if req.target_url.trim().is_empty() {
        return Err(AppError(MnemoError::Validation(
            "target_url is required".into(),
        )));
    }
    if !is_http_url(req.target_url.trim()) {
        return Err(AppError(MnemoError::Validation(
            "target_url must start with http:// or https://".into(),
        )));
    }

    let user = find_user_by_identifier(&state, req.user.trim()).await?;
    let now = chrono::Utc::now();
    let signing_secret = req
        .signing_secret
        .map(|secret| secret.trim().to_string())
        .filter(|secret| !secret.is_empty());
    let events = req
        .events
        .filter(|v| !v.is_empty())
        .unwrap_or_else(default_webhook_events);

    let webhook = MemoryWebhookSubscription {
        id: Uuid::now_v7(),
        user_id: user.id,
        user_identifier: req.user.trim().to_string(),
        target_url: req.target_url.trim().to_string(),
        signing_secret,
        events,
        enabled: req.enabled,
        created_at: now,
        updated_at: now,
    };

    {
        let mut hooks = state.memory_webhooks.write().await;
        hooks.insert(webhook.id, webhook.clone());
    }
    persist_webhook_state(&state).await;
    append_webhook_audit(
        &state,
        webhook.id,
        "webhook_registered",
        serde_json::json!({
            "target_url": webhook.target_url.clone(),
            "events": webhook.events.clone(),
            "enabled": webhook.enabled
        }),
    )
    .await;

    Ok((
        StatusCode::CREATED,
        Json(RegisterMemoryWebhookResponse { ok: true, webhook }),
    ))
}

async fn get_memory_webhook(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<MemoryWebhookSubscription>, AppError> {
    let hooks = state.memory_webhooks.read().await;
    let webhook = hooks.get(&id).cloned().ok_or_else(|| {
        AppError(MnemoError::NotFound {
            resource_type: "MemoryWebhook".into(),
            id: id.to_string(),
        })
    })?;
    Ok(Json(webhook))
}

async fn delete_memory_webhook(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<DeleteMemoryWebhookResponse>, AppError> {
    let removed = {
        let mut hooks = state.memory_webhooks.write().await;
        hooks.remove(&id).is_some()
    };
    {
        let mut events = state.memory_webhook_events.write().await;
        events.remove(&id);
    }
    {
        let mut audit = state.memory_webhook_audit.write().await;
        audit.remove(&id);
    }
    {
        let mut runtime = state.webhook_runtime.write().await;
        runtime.remove(&id);
    }
    persist_webhook_state(&state).await;
    Ok(Json(DeleteMemoryWebhookResponse { deleted: removed }))
}

async fn list_memory_webhook_events(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(query): Query<ListWebhookEventsQuery>,
) -> Result<Json<ListWebhookEventsResponse>, AppError> {
    {
        let hooks = state.memory_webhooks.read().await;
        if !hooks.contains_key(&id) {
            return Err(AppError(MnemoError::NotFound {
                resource_type: "MemoryWebhook".into(),
                id: id.to_string(),
            }));
        }
    }

    let limit = query.limit.unwrap_or(100).clamp(1, 1000) as usize;
    let mut events = {
        let event_map = state.memory_webhook_events.read().await;
        event_map.get(&id).cloned().unwrap_or_default()
    };

    if let Some(event_type) = query.event_type {
        events.retain(|event| event.event_type == event_type);
    }
    events.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    events.truncate(limit);

    Ok(Json(ListWebhookEventsResponse {
        webhook_id: id,
        count: events.len(),
        events,
    }))
}

async fn replay_memory_webhook_events(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(query): Query<ReplayWebhookEventsQuery>,
) -> Result<Json<ReplayWebhookEventsResponse>, AppError> {
    {
        let hooks = state.memory_webhooks.read().await;
        if !hooks.contains_key(&id) {
            return Err(AppError(MnemoError::NotFound {
                resource_type: "MemoryWebhook".into(),
                id: id.to_string(),
            }));
        }
    }

    let limit = query.limit.unwrap_or(100).clamp(1, 1000) as usize;
    let include_delivered = query.include_delivered.unwrap_or(true);
    let include_dead_letter = query.include_dead_letter.unwrap_or(true);
    let mut events = {
        let event_map = state.memory_webhook_events.read().await;
        event_map.get(&id).cloned().unwrap_or_default()
    };

    events.sort_by(|a, b| {
        a.created_at
            .cmp(&b.created_at)
            .then_with(|| a.id.cmp(&b.id))
    });
    if let Some(after_event_id) = query.after_event_id {
        if let Some(idx) = events.iter().position(|e| e.id == after_event_id) {
            events = events.into_iter().skip(idx + 1).collect();
        }
    }
    if !include_delivered {
        events.retain(|e| !e.delivered);
    }
    if !include_dead_letter {
        events.retain(|e| !e.dead_letter);
    }

    let page: Vec<MemoryWebhookEventRecord> = events.into_iter().take(limit).collect();
    let next_after_event_id = page.last().map(|e| e.id);

    Ok(Json(ReplayWebhookEventsResponse {
        webhook_id: id,
        count: page.len(),
        next_after_event_id,
        events: page,
    }))
}

async fn retry_memory_webhook_event(
    State(state): State<AppState>,
    Path((webhook_id, event_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<RetryWebhookEventRequest>,
) -> Result<Json<RetryWebhookEventResponse>, AppError> {
    {
        let hooks = state.memory_webhooks.read().await;
        if !hooks.contains_key(&webhook_id) {
            return Err(AppError(MnemoError::NotFound {
                resource_type: "MemoryWebhook".into(),
                id: webhook_id.to_string(),
            }));
        }
    }

    let mut found = false;
    let mut delivered = false;
    {
        let mut event_map = state.memory_webhook_events.write().await;
        if let Some(events) = event_map.get_mut(&webhook_id) {
            if let Some(event) = events.iter_mut().find(|e| e.id == event_id) {
                found = true;
                delivered = event.delivered;
                if !delivered || req.force.unwrap_or(false) {
                    event.dead_letter = false;
                    event.last_error = None;
                }
            }
        }
    }

    if !found {
        return Err(AppError(MnemoError::NotFound {
            resource_type: "WebhookEvent".into(),
            id: event_id.to_string(),
        }));
    }

    if delivered && !req.force.unwrap_or(false) {
        append_webhook_audit(
            &state,
            webhook_id,
            "retry_skipped",
            serde_json::json!({
                "event_id": event_id,
                "reason": "already_delivered"
            }),
        )
        .await;
        return Ok(Json(RetryWebhookEventResponse {
            webhook_id,
            event_id,
            queued: false,
            reason: "event already delivered; pass force=true to re-deliver".to_string(),
        }));
    }

    persist_webhook_state(&state).await;
    append_webhook_audit(
        &state,
        webhook_id,
        "retry_queued",
        serde_json::json!({
            "event_id": event_id,
            "force": req.force.unwrap_or(false)
        }),
    )
    .await;

    let state_clone = state.clone();
    tokio::spawn(async move {
        deliver_memory_webhook_event(state_clone, webhook_id, event_id).await;
    });

    Ok(Json(RetryWebhookEventResponse {
        webhook_id,
        event_id,
        queued: true,
        reason: "delivery retry queued".to_string(),
    }))
}

async fn list_memory_webhook_dead_letters(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(query): Query<ListWebhookEventsQuery>,
) -> Result<Json<ListWebhookEventsResponse>, AppError> {
    {
        let hooks = state.memory_webhooks.read().await;
        if !hooks.contains_key(&id) {
            return Err(AppError(MnemoError::NotFound {
                resource_type: "MemoryWebhook".into(),
                id: id.to_string(),
            }));
        }
    }

    let limit = query.limit.unwrap_or(100).clamp(1, 1000) as usize;
    let mut events = {
        let event_map = state.memory_webhook_events.read().await;
        event_map.get(&id).cloned().unwrap_or_default()
    };

    events.retain(|event| event.dead_letter);
    if let Some(event_type) = query.event_type {
        events.retain(|event| event.event_type == event_type);
    }
    events.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    events.truncate(limit);

    Ok(Json(ListWebhookEventsResponse {
        webhook_id: id,
        count: events.len(),
        events,
    }))
}

async fn get_memory_webhook_stats(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(query): Query<WebhookStatsQuery>,
) -> Result<Json<WebhookStatsResponse>, AppError> {
    {
        let hooks = state.memory_webhooks.read().await;
        if !hooks.contains_key(&id) {
            return Err(AppError(MnemoError::NotFound {
                resource_type: "MemoryWebhook".into(),
                id: id.to_string(),
            }));
        }
    }

    let window_seconds = query.window_seconds.unwrap_or(300).clamp(1, 86_400) as i64;
    let window_start = chrono::Utc::now() - chrono::Duration::seconds(window_seconds);
    let events = {
        let event_map = state.memory_webhook_events.read().await;
        event_map.get(&id).cloned().unwrap_or_default()
    };
    let runtime = {
        let runtime_map = state.webhook_runtime.read().await;
        runtime_map.get(&id).cloned()
    };

    let total_events = events.len();
    let delivered_events = events.iter().filter(|e| e.delivered).count();
    let dead_letter_events = events.iter().filter(|e| e.dead_letter).count();
    let pending_events = events
        .iter()
        .filter(|e| !e.delivered && !e.dead_letter)
        .count();
    let failed_events = events
        .iter()
        .filter(|e| !e.delivered && e.last_error.is_some())
        .count();
    let recent_failures = events
        .iter()
        .filter(|e| e.created_at >= window_start)
        .filter(|e| !e.delivered && e.last_error.is_some())
        .count();

    let circuit_open_until = runtime.and_then(|row| row.circuit_open_until);
    let circuit_open = circuit_open_until.is_some_and(|until| chrono::Utc::now() < until);

    Ok(Json(WebhookStatsResponse {
        webhook_id: id,
        total_events,
        delivered_events,
        pending_events,
        dead_letter_events,
        failed_events,
        recent_failures,
        circuit_open,
        circuit_open_until,
        rate_limit_per_minute: state.webhook_delivery.rate_limit_per_minute,
    }))
}

async fn list_memory_webhook_audit(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(query): Query<WebhookAuditQuery>,
) -> Result<Json<WebhookAuditResponse>, AppError> {
    {
        let hooks = state.memory_webhooks.read().await;
        if !hooks.contains_key(&id) {
            return Err(AppError(MnemoError::NotFound {
                resource_type: "MemoryWebhook".into(),
                id: id.to_string(),
            }));
        }
    }

    let limit = query.limit.unwrap_or(100).clamp(1, 1000) as usize;
    let mut audit = {
        let map = state.memory_webhook_audit.read().await;
        map.get(&id).cloned().unwrap_or_default()
    };
    audit.sort_by(|a, b| b.at.cmp(&a.at));
    audit.truncate(limit);

    Ok(Json(WebhookAuditResponse {
        webhook_id: id,
        count: audit.len(),
        audit,
    }))
}

async fn get_agent_identity(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> Result<Json<AgentIdentityProfile>, AppError> {
    let agent_id = normalize_agent_id(&agent_id)?;
    let identity = state.state_store.get_agent_identity(&agent_id).await?;
    Ok(Json(identity))
}

async fn update_agent_identity(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Json(req): Json<UpdateAgentIdentityRequest>,
) -> Result<Json<AgentIdentityProfile>, AppError> {
    let agent_id = normalize_agent_id(&agent_id)?;
    validate_identity_core(&req.core)?;
    let identity = state
        .state_store
        .update_agent_identity(&agent_id, req)
        .await?;
    Ok(Json(identity))
}

async fn list_agent_identity_versions(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Query(params): Query<ListLimitQuery>,
) -> Result<Json<Vec<AgentIdentityProfile>>, AppError> {
    let agent_id = normalize_agent_id(&agent_id)?;
    let limit = params.limit.unwrap_or(20).clamp(1, 200);
    let versions = state
        .state_store
        .list_agent_identity_versions(&agent_id, limit)
        .await?;
    Ok(Json(versions))
}

async fn list_agent_identity_audit(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Query(params): Query<ListLimitQuery>,
) -> Result<Json<Vec<AgentIdentityAuditEvent>>, AppError> {
    let agent_id = normalize_agent_id(&agent_id)?;
    let limit = params.limit.unwrap_or(50).clamp(1, 500);
    let audit = state
        .state_store
        .list_agent_identity_audit(&agent_id, limit)
        .await?;
    Ok(Json(audit))
}

async fn rollback_agent_identity(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Json(req): Json<IdentityRollbackRequest>,
) -> Result<Json<AgentIdentityProfile>, AppError> {
    let agent_id = normalize_agent_id(&agent_id)?;
    if req.target_version == 0 {
        return Err(AppError(MnemoError::Validation(
            "target_version must be >= 1".into(),
        )));
    }
    let identity = state
        .state_store
        .rollback_agent_identity(&agent_id, req.target_version, req.reason)
        .await?;
    Ok(Json(identity))
}

async fn add_agent_experience(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Json(req): Json<CreateExperienceRequest>,
) -> Result<(StatusCode, Json<ExperienceEvent>), AppError> {
    let agent_id = normalize_agent_id(&agent_id)?;
    if req.signal.trim().is_empty() {
        return Err(AppError(MnemoError::Validation(
            "signal is required".into(),
        )));
    }
    if req.category.trim().is_empty() {
        return Err(AppError(MnemoError::Validation(
            "category is required".into(),
        )));
    }

    let event = state
        .state_store
        .add_experience_event(&agent_id, req)
        .await?;
    Ok((StatusCode::CREATED, Json(event)))
}

async fn create_promotion_proposal(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Json(req): Json<CreatePromotionProposalRequest>,
) -> Result<(StatusCode, Json<PromotionProposal>), AppError> {
    let agent_id = normalize_agent_id(&agent_id)?;
    if req.proposal.trim().is_empty() {
        return Err(AppError(MnemoError::Validation(
            "proposal is required".into(),
        )));
    }
    if req.reason.trim().is_empty() {
        return Err(AppError(MnemoError::Validation(
            "reason is required".into(),
        )));
    }
    validate_identity_core(&req.candidate_core)?;
    if req.source_event_ids.len() < 3 {
        return Err(AppError(MnemoError::Validation(
            "promotion requires at least 3 source_event_ids".into(),
        )));
    }

    let proposal = state
        .state_store
        .create_promotion_proposal(&agent_id, req)
        .await?;
    Ok((StatusCode::CREATED, Json(proposal)))
}

async fn list_promotion_proposals(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Query(params): Query<ListLimitQuery>,
) -> Result<Json<Vec<PromotionProposal>>, AppError> {
    let agent_id = normalize_agent_id(&agent_id)?;
    let limit = params.limit.unwrap_or(50).clamp(1, 500);
    let proposals = state
        .state_store
        .list_promotion_proposals(&agent_id, limit)
        .await?;
    Ok(Json(proposals))
}

async fn approve_promotion_proposal(
    State(state): State<AppState>,
    Path((agent_id, proposal_id)): Path<(String, Uuid)>,
) -> Result<Json<PromotionProposal>, AppError> {
    let agent_id = normalize_agent_id(&agent_id)?;
    let mut proposal = state
        .state_store
        .get_promotion_proposal(&agent_id, proposal_id)
        .await?;

    if proposal.status != PromotionStatus::Pending {
        return Err(AppError(MnemoError::Validation(
            "proposal is not pending".into(),
        )));
    }

    validate_identity_core(&proposal.candidate_core)?;
    state
        .state_store
        .update_agent_identity(
            &agent_id,
            UpdateAgentIdentityRequest {
                core: proposal.candidate_core.clone(),
            },
        )
        .await?;

    proposal.status = PromotionStatus::Approved;
    proposal.approved_at = Some(chrono::Utc::now());
    state
        .state_store
        .update_promotion_proposal(&proposal)
        .await?;
    Ok(Json(proposal))
}

async fn reject_promotion_proposal(
    State(state): State<AppState>,
    Path((agent_id, proposal_id)): Path<(String, Uuid)>,
    Json(req): Json<RejectPromotionRequest>,
) -> Result<Json<PromotionProposal>, AppError> {
    let agent_id = normalize_agent_id(&agent_id)?;
    let mut proposal = state
        .state_store
        .get_promotion_proposal(&agent_id, proposal_id)
        .await?;

    if proposal.status != PromotionStatus::Pending {
        return Err(AppError(MnemoError::Validation(
            "proposal is not pending".into(),
        )));
    }

    proposal.status = PromotionStatus::Rejected;
    proposal.rejected_at = Some(chrono::Utc::now());
    if let Some(reason) = req.reason {
        if !reason.trim().is_empty() {
            proposal.reason = format!("{} | rejection_reason={}", proposal.reason, reason.trim());
        }
    }
    state
        .state_store
        .update_promotion_proposal(&proposal)
        .await?;
    Ok(Json(proposal))
}

async fn get_agent_context(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Json(req): Json<AgentContextRequest>,
) -> Result<Json<AgentContextResponse>, AppError> {
    if req.query.trim().is_empty() {
        return Err(AppError(MnemoError::Validation("query is required".into())));
    }
    if req.user.trim().is_empty() {
        return Err(AppError(MnemoError::Validation("user is required".into())));
    }

    let agent_id = normalize_agent_id(&agent_id)?;
    let identity = state.state_store.get_agent_identity(&agent_id).await?;
    let experiences = state
        .state_store
        .list_experience_events(&agent_id, 50)
        .await?;

    let user = find_user_by_identifier(&state, req.user.trim()).await?;
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
    context.token_count = estimate_tokens(&context.context);

    let experience_weight_sum: f32 = experiences.iter().map(effective_experience_weight).sum();

    Ok(Json(AgentContextResponse {
        identity_version: identity.version,
        experience_events_used: experiences.len() as u32,
        experience_weight_sum,
        user_memory_items_used: (context.entities.len()
            + context.facts.len()
            + context.episodes.len()) as u32,
        attribution_guards: serde_json::json!({
            "self_user_separation_enforced": true,
            "identity_plane_isolated": true
        }),
        context,
        identity,
    }))
}

fn normalize_agent_id(agent_id: &str) -> Result<String, AppError> {
    let trimmed = agent_id.trim();
    if trimmed.is_empty() {
        return Err(AppError(MnemoError::Validation(
            "agent_id is required".into(),
        )));
    }
    Ok(trimmed.to_string())
}

fn effective_experience_weight(event: &ExperienceEvent) -> f32 {
    let age_days = (chrono::Utc::now() - event.created_at).num_days().max(0) as f32;
    let half_life = event.decay_half_life_days.max(1) as f32;
    let decay_factor = 2f32.powf(-age_days / half_life);
    (event.weight * event.confidence * decay_factor).max(0.0)
}

fn validate_identity_core(core: &serde_json::Value) -> Result<(), AppError> {
    if !core.is_object() {
        return Err(AppError(MnemoError::Validation(
            "identity core must be a JSON object".into(),
        )));
    }

    let allowed_top_level = [
        "mission",
        "style",
        "boundaries",
        "capabilities",
        "values",
        "persona",
    ];

    if let Some(map) = core.as_object() {
        for key in map.keys() {
            if !allowed_top_level.contains(&key.as_str()) {
                return Err(AppError(MnemoError::Validation(format!(
                    "identity core key '{}' is not allowed",
                    key
                ))));
            }
        }
    }

    let forbidden_substrings = [
        "user",
        "session",
        "episode",
        "external_id",
        "email",
        "phone",
        "address",
    ];

    fn visit(
        value: &serde_json::Value,
        path: &str,
        forbidden_substrings: &[&str],
    ) -> Result<(), AppError> {
        match value {
            serde_json::Value::Object(map) => {
                for (k, v) in map {
                    let normalized = k.to_ascii_lowercase();
                    if forbidden_substrings
                        .iter()
                        .any(|token| normalized.contains(token))
                    {
                        return Err(AppError(MnemoError::Validation(format!(
                            "identity core contains forbidden key at {}{}",
                            path, k
                        ))));
                    }
                    let next = format!("{}{}/", path, k);
                    visit(v, &next, forbidden_substrings)?;
                }
            }
            serde_json::Value::Array(items) => {
                for (idx, item) in items.iter().enumerate() {
                    let next = format!("{}[{idx}]/", path);
                    visit(item, &next, forbidden_substrings)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    visit(core, "core/", &forbidden_substrings)
}

struct MetadataCandidates {
    scanned_count: u32,
    scanned_episodes: Vec<Episode>,
    filtered_episodes: Vec<Episode>,
}

async fn collect_metadata_candidates(
    state: &AppState,
    user_id: Uuid,
    session_id: Option<Uuid>,
    filters: &MemoryContextFilters,
    scan_limit: u32,
) -> Result<MetadataCandidates, MnemoError> {
    let mut episodes = Vec::new();

    if let Some(session_id) = session_id {
        let mut session_episodes = state
            .state_store
            .list_episodes(
                session_id,
                ListEpisodesParams {
                    limit: scan_limit,
                    after: None,
                    status: None,
                },
            )
            .await?;
        episodes.append(&mut session_episodes);
    } else {
        let sessions = state
            .state_store
            .list_sessions(
                user_id,
                ListSessionsParams {
                    limit: 12,
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
                        limit: 40,
                        after: None,
                        status: None,
                    },
                )
                .await?;
            episodes.append(&mut session_episodes);
            if episodes.len() >= scan_limit as usize {
                break;
            }
        }
    }

    let scanned_count = episodes.len() as u32;
    let scanned_episodes = episodes.clone();
    episodes.retain(|episode| matches_episode_filters(episode, filters));

    Ok(MetadataCandidates {
        scanned_count,
        scanned_episodes,
        filtered_episodes: episodes,
    })
}

fn matches_episode_filters(episode: &Episode, filters: &MemoryContextFilters) -> bool {
    if let Some(status) = filters.processing_status {
        if episode.processing_status != status {
            return false;
        }
    }

    if let Some(after) = filters.created_after {
        if episode.created_at < after {
            return false;
        }
    }

    if let Some(before) = filters.created_before {
        if episode.created_at > before {
            return false;
        }
    }

    if let Some(roles) = filters.roles.as_ref() {
        if !roles.is_empty() && !episode.role.map(|r| roles.contains(&r)).unwrap_or(false) {
            return false;
        }
    }

    let tag_values = episode
        .metadata
        .get("tags")
        .and_then(|t| t.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_lowercase()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    if let Some(tags_any) = filters.tags_any.as_ref() {
        let wanted: Vec<String> = tags_any.iter().map(|t| t.to_lowercase()).collect();
        if !wanted.is_empty() && !wanted.iter().any(|tag| tag_values.contains(tag)) {
            return false;
        }
    }

    if let Some(tags_all) = filters.tags_all.as_ref() {
        let wanted: Vec<String> = tags_all.iter().map(|t| t.to_lowercase()).collect();
        if !wanted.iter().all(|tag| tag_values.contains(tag)) {
            return false;
        }
    }

    true
}

fn dominant_session(episodes: &[Episode]) -> Option<Uuid> {
    let mut counts: std::collections::HashMap<Uuid, usize> = std::collections::HashMap::new();
    for episode in episodes {
        *counts.entry(episode.session_id).or_default() += 1;
    }
    counts
        .into_iter()
        .max_by(|(a_id, a_count), (b_id, b_count)| {
            a_count.cmp(b_count).then_with(|| a_id.cmp(b_id))
        })
        .map(|(session_id, _)| session_id)
}

async fn list_all_sessions_for_user(
    state: &AppState,
    user_id: Uuid,
) -> Result<Vec<Session>, MnemoError> {
    let mut out = Vec::new();
    let mut after = None;
    loop {
        let page = state
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
        if page.is_empty() {
            break;
        }
        after = page.last().map(|s| s.id);
        out.extend(page);
        if out.len() > 20_000 {
            break;
        }
    }
    Ok(out)
}

async fn list_all_episodes_for_session(
    state: &AppState,
    session_id: Uuid,
) -> Result<Vec<Episode>, MnemoError> {
    let mut out = Vec::new();
    let mut after = None;
    loop {
        let page = state
            .state_store
            .list_episodes(
                session_id,
                ListEpisodesParams {
                    limit: 500,
                    after,
                    status: None,
                },
            )
            .await?;
        if page.is_empty() {
            break;
        }
        after = page.last().map(|e| e.id);
        out.extend(page);
        if out.len() > 50_000 {
            break;
        }
    }
    Ok(out)
}

async fn list_all_entities_for_user(
    state: &AppState,
    user_id: Uuid,
) -> Result<Vec<Entity>, MnemoError> {
    let mut out = Vec::new();
    let mut after = None;
    loop {
        let page = state.state_store.list_entities(user_id, 500, after).await?;
        if page.is_empty() {
            break;
        }
        after = page.last().map(|e| e.id);
        out.extend(page);
        if out.len() > 50_000 {
            break;
        }
    }
    Ok(out)
}

fn preview_text(content: &str, max_chars: usize) -> String {
    let trimmed = content.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let mut out = String::new();
    for c in trimmed.chars().take(max_chars) {
        out.push(c);
    }
    out.push_str("...");
    out
}

fn normalized_terms(text: &str) -> std::collections::HashSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter_map(|t| {
            let s = t.trim().to_ascii_lowercase();
            (!s.is_empty()).then_some(s)
        })
        .collect()
}

fn apply_memory_contract(context: &mut ContextBlock, contract: MemoryContract) {
    match contract {
        MemoryContract::Default => {}
        MemoryContract::SupportSafe => {
            context
                .episodes
                .retain(|episode| episode.role.as_deref() == Some("user"));
            context.facts.retain(|fact| fact.invalid_at.is_none());
        }
        MemoryContract::CurrentStrict => {
            context.facts.retain(|fact| fact.invalid_at.is_none());
            context.episodes.retain(|episode| {
                episode
                    .role
                    .as_deref()
                    .map(|r| r == "user" || r == "assistant")
                    .unwrap_or(true)
            });
        }
        MemoryContract::HistoricalStrict => {
            // Keep both valid and invalidated facts for historical analysis.
        }
    }
}

async fn run_import_job(
    state: AppState,
    job_id: Uuid,
    source: ChatHistorySource,
    user_identifier: String,
    payload: serde_json::Value,
    default_session: Option<String>,
    dry_run: bool,
) {
    set_import_job_status(&state, job_id, ImportJobStatus::Running).await;

    let messages = match parse_import_messages(&source, payload) {
        Ok(messages) => messages,
        Err(err) => {
            finalize_import_job_failure(&state, job_id, vec![err]).await;
            return;
        }
    };

    update_import_job_totals(&state, job_id, messages.len() as u32).await;

    if dry_run {
        let mut session_names = std::collections::HashSet::new();
        for message in &messages {
            if let Some(name) = message.session.as_ref() {
                session_names.insert(name.clone());
            }
        }
        if session_names.is_empty() {
            if let Some(name) = default_session
                .as_ref()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
            {
                session_names.insert(name.to_string());
            } else {
                session_names.insert("imported".to_string());
            }
        }
        finalize_import_job_success(
            &state,
            job_id,
            messages.len() as u32,
            0,
            session_names.len() as u32,
            Vec::new(),
        )
        .await;
        return;
    }

    let user = match find_or_create_memory_user(&state, &user_identifier).await {
        Ok(user) => user,
        Err(err) => {
            finalize_import_job_failure(&state, job_id, vec![err.to_string()]).await;
            return;
        }
    };

    let mut imported_messages = 0u32;
    let mut failed_messages = 0u32;
    let mut sessions: HashMap<String, Session> = HashMap::new();
    let mut errors: Vec<String> = Vec::new();

    for (idx, message) in messages.into_iter().enumerate() {
        let session_name = message
            .session
            .or_else(|| {
                default_session
                    .as_ref()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
            })
            .unwrap_or_else(|| "imported".to_string());

        let session = if let Some(cached) = sessions.get(&session_name) {
            cached.clone()
        } else {
            match find_or_create_session(&state, user.id, &session_name).await {
                Ok(session) => {
                    sessions.insert(session_name.clone(), session.clone());
                    session
                }
                Err(err) => {
                    failed_messages += 1;
                    if errors.len() < 20 {
                        errors.push(format!(
                            "row {} session '{}' failed: {}",
                            idx + 1,
                            session_name,
                            err
                        ));
                    }
                    continue;
                }
            }
        };

        let episode_req = CreateEpisodeRequest {
            id: None,
            episode_type: EpisodeType::Message,
            content: message.content,
            role: Some(message.role),
            name: Some(user.name.clone()),
            metadata: serde_json::json!({ "import_source": source.as_str() }),
            created_at: Some(message.created_at),
        };

        match state
            .state_store
            .create_episode(episode_req, session.id, user.id)
            .await
        {
            Ok(_) => imported_messages += 1,
            Err(err) => {
                failed_messages += 1;
                if errors.len() < 20 {
                    errors.push(format!("row {} import failed: {}", idx + 1, err));
                }
            }
        }
    }

    finalize_import_job_success(
        &state,
        job_id,
        imported_messages,
        failed_messages,
        sessions.len() as u32,
        errors,
    )
    .await;

    if imported_messages > 0 {
        emit_memory_webhook_event(
            &state,
            user.id,
            MemoryWebhookEventType::HeadAdvanced,
            serde_json::json!({
                "job_id": job_id,
                "imported_messages": imported_messages,
                "failed_messages": failed_messages,
                "sessions_touched": sessions.len()
            }),
        )
        .await;
    }
}

fn parse_import_messages(
    source: &ChatHistorySource,
    payload: serde_json::Value,
) -> Result<Vec<ImportMessage>, String> {
    match source {
        ChatHistorySource::Ndjson => parse_ndjson_payload(payload),
        ChatHistorySource::ChatgptExport => parse_chatgpt_export_payload(payload),
        ChatHistorySource::GeminiExport => parse_gemini_export_payload(payload),
    }
}

fn parse_gemini_export_payload(payload: serde_json::Value) -> Result<Vec<ImportMessage>, String> {
    let obj = payload
        .as_object()
        .ok_or_else(|| "gemini export payload must be an object".to_string())?;

    let chunks = obj
        .get("chunkedPrompt")
        .and_then(|v| v.get("chunks"))
        .and_then(|v| v.as_array())
        .ok_or_else(|| "gemini export missing chunkedPrompt.chunks".to_string())?;

    let mut messages = Vec::new();
    let base_time = chrono::Utc::now();

    for (idx, chunk) in chunks.iter().enumerate() {
        let Some(item) = chunk.as_object() else {
            continue;
        };

        if item
            .get("isThought")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            continue;
        }

        let Some(role_raw) = item.get("role").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(role) = parse_role(role_raw) else {
            continue;
        };

        let content = item
            .get("text")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .or_else(|| {
                item.get("parts")
                    .and_then(|v| v.as_array())
                    .map(|parts| {
                        parts
                            .iter()
                            .filter_map(|p| p.get("text").and_then(|v| v.as_str()))
                            .collect::<Vec<_>>()
                            .join("\n")
                            .trim()
                            .to_string()
                    })
                    .filter(|s| !s.is_empty())
            });
        let Some(content) = content else {
            continue;
        };

        let created_at = base_time + chrono::Duration::seconds(idx as i64);

        messages.push(ImportMessage {
            session: None,
            role,
            content,
            created_at,
        });
    }

    if messages.is_empty() {
        return Err("no importable messages found in gemini export payload".into());
    }

    Ok(messages)
}

fn parse_ndjson_payload(payload: serde_json::Value) -> Result<Vec<ImportMessage>, String> {
    let mut raw_rows: Vec<serde_json::Value> = Vec::new();

    match payload {
        serde_json::Value::String(lines) => {
            for line in lines.lines().filter(|l| !l.trim().is_empty()) {
                let value: serde_json::Value =
                    serde_json::from_str(line).map_err(|e| format!("invalid NDJSON row: {e}"))?;
                raw_rows.push(value);
            }
        }
        serde_json::Value::Array(rows) => raw_rows = rows,
        serde_json::Value::Object(map) => {
            if let Some(rows) = map.get("messages").and_then(|v| v.as_array()) {
                raw_rows = rows.clone();
            } else if let Some(lines) = map.get("ndjson").and_then(|v| v.as_str()) {
                for line in lines.lines().filter(|l| !l.trim().is_empty()) {
                    let value: serde_json::Value = serde_json::from_str(line)
                        .map_err(|e| format!("invalid NDJSON row: {e}"))?;
                    raw_rows.push(value);
                }
            } else {
                return Err("ndjson payload requires string, array, or object.messages".into());
            }
        }
        _ => return Err("unsupported ndjson payload shape".into()),
    }

    let mut messages = Vec::new();
    for (idx, row) in raw_rows.into_iter().enumerate() {
        let obj = row
            .as_object()
            .ok_or_else(|| format!("row {} is not an object", idx + 1))?;

        let content = obj
            .get("content")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| format!("row {} missing content", idx + 1))?;

        let role = parse_role(
            obj.get("role")
                .and_then(|v| v.as_str())
                .ok_or_else(|| format!("row {} missing role", idx + 1))?,
        )
        .ok_or_else(|| format!("row {} invalid role", idx + 1))?;

        let created_at = parse_created_at(
            obj.get("created_at")
                .or_else(|| obj.get("timestamp"))
                .and_then(|v| v.as_str()),
        )?;

        let session = obj
            .get("session")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        messages.push(ImportMessage {
            session,
            role,
            content,
            created_at,
        });
    }

    Ok(messages)
}

fn parse_chatgpt_export_payload(payload: serde_json::Value) -> Result<Vec<ImportMessage>, String> {
    let conversations: Vec<serde_json::Value> = match payload {
        serde_json::Value::Array(items) => items,
        serde_json::Value::Object(map) => {
            if let Some(items) = map.get("conversations").and_then(|v| v.as_array()) {
                items.clone()
            } else {
                vec![serde_json::Value::Object(map)]
            }
        }
        _ => return Err("chatgpt export payload must be an object or array".into()),
    };

    let mut messages: Vec<ImportMessage> = Vec::new();

    for convo in conversations {
        let title = convo
            .get("title")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let Some(mapping) = convo.get("mapping").and_then(|v| v.as_object()) else {
            continue;
        };

        let mut extracted: Vec<(chrono::DateTime<chrono::Utc>, ImportMessage)> = Vec::new();
        for node in mapping.values() {
            let Some(message) = node.get("message") else {
                continue;
            };
            let Some(author_role) = message
                .get("author")
                .and_then(|v| v.get("role"))
                .and_then(|v| v.as_str())
            else {
                continue;
            };
            let Some(role) = parse_role(author_role) else {
                continue;
            };

            let content = extract_chatgpt_content(message);
            if content.trim().is_empty() {
                continue;
            }

            let created_at = if let Some(ts) = message.get("create_time").and_then(|v| v.as_f64()) {
                chrono::DateTime::<chrono::Utc>::from_timestamp(ts as i64, 0)
                    .unwrap_or_else(chrono::Utc::now)
            } else {
                chrono::Utc::now()
            };

            extracted.push((
                created_at,
                ImportMessage {
                    session: title.clone(),
                    role,
                    content,
                    created_at,
                },
            ));
        }

        extracted.sort_by(|(a_ts, _), (b_ts, _)| a_ts.cmp(b_ts));
        messages.extend(extracted.into_iter().map(|(_, msg)| msg));
    }

    if messages.is_empty() {
        return Err("no importable messages found in chatgpt export payload".into());
    }

    Ok(messages)
}

fn extract_chatgpt_content(message: &serde_json::Value) -> String {
    if let Some(parts) = message
        .get("content")
        .and_then(|v| v.get("parts"))
        .and_then(|v| v.as_array())
    {
        return parts
            .iter()
            .filter_map(|part| part.as_str())
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_string();
    }

    message
        .get("content")
        .and_then(|v| v.get("text"))
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn parse_role(input: &str) -> Option<MessageRole> {
    match input.to_ascii_lowercase().as_str() {
        "user" | "human" => Some(MessageRole::User),
        "assistant" | "ai" | "model" => Some(MessageRole::Assistant),
        "system" => Some(MessageRole::System),
        "tool" | "function" => Some(MessageRole::Tool),
        _ => None,
    }
}

fn parse_created_at(input: Option<&str>) -> Result<chrono::DateTime<chrono::Utc>, String> {
    if let Some(raw) = input {
        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(raw) {
            return Ok(dt.with_timezone(&chrono::Utc));
        }
        if let Ok(unix_seconds) = raw.parse::<i64>() {
            if let Some(dt) = chrono::DateTime::<chrono::Utc>::from_timestamp(unix_seconds, 0) {
                return Ok(dt);
            }
        }
        Err("invalid created_at timestamp (expected RFC3339 or unix seconds string)".into())
    } else {
        Ok(chrono::Utc::now())
    }
}

async fn set_import_job_status(state: &AppState, job_id: Uuid, status: ImportJobStatus) {
    let mut jobs = state.import_jobs.write().await;
    if let Some(job) = jobs.get_mut(&job_id) {
        if matches!(status, ImportJobStatus::Running) {
            job.started_at = Some(chrono::Utc::now());
        }
        job.status = status;
    }
}

async fn update_import_job_totals(state: &AppState, job_id: Uuid, total_messages: u32) {
    let mut jobs = state.import_jobs.write().await;
    if let Some(job) = jobs.get_mut(&job_id) {
        job.total_messages = total_messages;
    }
}

async fn finalize_import_job_failure(state: &AppState, job_id: Uuid, errors: Vec<String>) {
    let mut jobs = state.import_jobs.write().await;
    if let Some(job) = jobs.get_mut(&job_id) {
        job.status = ImportJobStatus::Failed;
        job.finished_at = Some(chrono::Utc::now());
        job.errors = errors;
    }
}

async fn finalize_import_job_success(
    state: &AppState,
    job_id: Uuid,
    imported_messages: u32,
    failed_messages: u32,
    sessions_touched: u32,
    errors: Vec<String>,
) {
    let mut jobs = state.import_jobs.write().await;
    if let Some(job) = jobs.get_mut(&job_id) {
        job.status = if failed_messages > 0 {
            ImportJobStatus::Failed
        } else {
            ImportJobStatus::Completed
        };
        job.imported_messages = imported_messages;
        job.failed_messages = failed_messages;
        job.sessions_touched = sessions_touched;
        job.errors = errors;
        job.finished_at = Some(chrono::Utc::now());
    }
}

async fn find_or_create_memory_user(
    state: &AppState,
    user_identifier: &str,
) -> Result<User, MnemoError> {
    match find_user_by_identifier(state, user_identifier).await {
        Ok(user) => Ok(user),
        Err(err) if is_user_not_found(&err) => {
            let create = CreateUserRequest {
                id: None,
                external_id: Some(user_identifier.to_string()),
                name: user_identifier.to_string(),
                email: None,
                metadata: serde_json::json!({}),
            };
            match state.state_store.create_user(create).await {
                Ok(user) => Ok(user),
                Err(MnemoError::Duplicate(_)) => {
                    find_user_by_identifier(state, user_identifier).await
                }
                Err(create_err) => Err(create_err),
            }
        }
        Err(err) => Err(err),
    }
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
            let is_better = match &best {
                Some(current_best) => {
                    compare_head_candidate(&session, current_best) == std::cmp::Ordering::Greater
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

fn compare_head_candidate(a: &Session, b: &Session) -> std::cmp::Ordering {
    let a_time = a
        .head_updated_at
        .or(a.last_activity_at)
        .unwrap_or(a.updated_at);
    let b_time = b
        .head_updated_at
        .or(b.last_activity_at)
        .unwrap_or(b.updated_at);

    match a_time.cmp(&b_time) {
        std::cmp::Ordering::Equal => match a.head_version.cmp(&b.head_version) {
            std::cmp::Ordering::Equal => a.id.cmp(&b.id),
            other => other,
        },
        other => other,
    }
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
