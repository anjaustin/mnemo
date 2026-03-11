use axum::extract::{DefaultBodyLimit, Extension, Json, Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{delete, get, post, put};
use axum::Router;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::time::Duration;
use uuid::Uuid;

use hmac::{Hmac, Mac};
use redis::AsyncCommands;
use sha2::Sha256;
use tracing::warn;

use mnemo_core::error::{ApiErrorResponse, MnemoError};
use mnemo_core::models::{
    agent::{
        compute_fisher_importance, AgentIdentityAuditEvent, AgentIdentityProfile,
        CreateExperienceRequest, CreatePromotionProposalRequest, ExperienceEvent,
        IdentityRollbackRequest, PromotionProposal, PromotionStatus,
        UpdateAgentIdentityRequest,
    },
    context::{
        estimate_tokens, ContextBlock, ContextMessage, ContextRequest, EpisodeSummary, FactSummary,
        RetrievalSource, TemporalIntent,
    },
    edge::{Edge, EdgeFilter, ExtractedRelationship},
    entity::{Entity, EntityType, ExtractedEntity},
    episode::{
        BatchCreateEpisodesRequest, CreateEpisodeRequest, Episode, EpisodeType, ListEpisodesParams,
        MessageRole, ProcessingStatus,
    },
    session::{CreateSessionRequest, ListSessionsParams, Session, UpdateSessionRequest},
    user::{CreateUserRequest, UpdateUserRequest, User},
};
use mnemo_core::traits::storage::{
    AgentStore, EdgeStore, EntityStore, EpisodeStore, RawVectorStore, SessionStore, SpanStore,
    UserStore, VectorStore,
};

use mnemo_retrieval::Reranker;

use crate::middleware::RequestContext;
use crate::state::{
    AppState, GovernanceAuditRecord, ImportJobRecord, ImportJobStatus, MemoryWebhookAuditRecord,
    MemoryWebhookEventRecord, MemoryWebhookEventType, MemoryWebhookSubscription, RerankerMode,
    UserPolicyRecord, WebhookRuntimeState,
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

fn request_id_from_extension(ctx: Option<Extension<RequestContext>>) -> Option<String> {
    ctx.map(|Extension(ctx)| ctx.request_id)
}

fn extract_request_id_from_metadata(metadata: &serde_json::Value) -> Option<String> {
    metadata
        .get("request_id")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn metadata_with_request_id(
    metadata: serde_json::Value,
    request_id: Option<&str>,
) -> serde_json::Value {
    let Some(request_id) = request_id else {
        return metadata;
    };

    match metadata {
        serde_json::Value::Object(mut map) => {
            map.insert(
                "request_id".to_string(),
                serde_json::Value::String(request_id.to_string()),
            );
            serde_json::Value::Object(map)
        }
        other => serde_json::json!({
            "request_id": request_id,
            "_raw_metadata": other
        }),
    }
}

/// Convert the server-side `RerankerMode` config enum to the retrieval
/// engine's `Reranker` type.
fn reranker_for_state(state: &AppState) -> Reranker {
    match state.reranker {
        RerankerMode::Rrf => Reranker::Rrf,
        RerankerMode::Mmr => Reranker::Mmr,
    }
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

fn user_policies_key(state: &AppState) -> String {
    format!("{}:user_policies", state.webhook_redis_prefix)
}

fn governance_audit_key(state: &AppState) -> String {
    format!("{}:governance_audit", state.webhook_redis_prefix)
}

fn default_user_policy(user_id: Uuid, user_identifier: String) -> UserPolicyRecord {
    let now = chrono::Utc::now();
    UserPolicyRecord {
        user_id,
        user_identifier,
        retention_days_message: 3650,
        retention_days_text: 3650,
        retention_days_json: 3650,
        webhook_domain_allowlist: Vec::new(),
        default_memory_contract: "default".to_string(),
        default_retrieval_policy: "balanced".to_string(),
        created_at: now,
        updated_at: now,
    }
}

async fn get_or_create_user_policy(
    state: &AppState,
    user_id: Uuid,
    user_identifier: String,
) -> UserPolicyRecord {
    let (policy, created) = {
        let mut policies = state.user_policies.write().await;
        let is_new = !policies.contains_key(&user_id);
        let p = policies
            .entry(user_id)
            .or_insert_with(|| default_user_policy(user_id, user_identifier))
            .clone();
        (p, is_new)
    };
    if created {
        persist_webhook_state(state).await;
    }
    policy
}

fn normalize_domain_allowlist(values: Option<Vec<String>>) -> Vec<String> {
    values
        .unwrap_or_default()
        .into_iter()
        .map(|v| v.trim().to_ascii_lowercase())
        .filter(|v| !v.is_empty())
        .collect()
}

fn target_url_host(url: &str) -> Option<String> {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(|host| host.to_ascii_lowercase()))
}

fn is_target_url_allowed(policy: &UserPolicyRecord, target_url: &str) -> bool {
    if policy.webhook_domain_allowlist.is_empty() {
        return true;
    }
    let Some(host) = target_url_host(target_url) else {
        return false;
    };

    policy
        .webhook_domain_allowlist
        .iter()
        .any(|allowed| host == *allowed || host.ends_with(&format!(".{allowed}")))
}

fn apply_user_policy_patch(
    mut policy: UserPolicyRecord,
    req: &UpsertUserPolicyRequest,
    updated_at: chrono::DateTime<chrono::Utc>,
) -> UserPolicyRecord {
    if let Some(v) = req.retention_days_message {
        policy.retention_days_message = v.clamp(1, 3650);
    }
    if let Some(v) = req.retention_days_text {
        policy.retention_days_text = v.clamp(1, 3650);
    }
    if let Some(v) = req.retention_days_json {
        policy.retention_days_json = v.clamp(1, 3650);
    }
    if req.webhook_domain_allowlist.is_some() {
        policy.webhook_domain_allowlist =
            normalize_domain_allowlist(req.webhook_domain_allowlist.clone());
    }
    if let Some(v) = req.default_memory_contract.clone() {
        let normalized = v.trim().to_string();
        if !normalized.is_empty() {
            policy.default_memory_contract = normalized;
        }
    }
    if let Some(v) = req.default_retrieval_policy.clone() {
        let normalized = v.trim().to_string();
        if !normalized.is_empty() {
            policy.default_retrieval_policy = normalized;
        }
    }
    policy.updated_at = updated_at;
    policy
}

fn parse_memory_contract_default(value: &str) -> MemoryContract {
    match value.trim().to_ascii_lowercase().as_str() {
        "support_safe" => MemoryContract::SupportSafe,
        "current_strict" => MemoryContract::CurrentStrict,
        "historical_strict" => MemoryContract::HistoricalStrict,
        _ => MemoryContract::Default,
    }
}

fn parse_retrieval_policy_default(value: &str) -> AdaptiveRetrievalPolicy {
    match value.trim().to_ascii_lowercase().as_str() {
        "precision" => AdaptiveRetrievalPolicy::Precision,
        "recall" => AdaptiveRetrievalPolicy::Recall,
        "stability" => AdaptiveRetrievalPolicy::Stability,
        _ => AdaptiveRetrievalPolicy::Balanced,
    }
}

fn retention_days_for_episode_type(policy: &UserPolicyRecord, episode_type: EpisodeType) -> u32 {
    match episode_type {
        EpisodeType::Message => policy.retention_days_message,
        EpisodeType::Text => policy.retention_days_text,
        EpisodeType::Json => policy.retention_days_json,
    }
}

fn retention_cutoff(
    policy: &UserPolicyRecord,
    episode_type: EpisodeType,
) -> chrono::DateTime<chrono::Utc> {
    chrono::Utc::now()
        - chrono::Duration::days(retention_days_for_episode_type(policy, episode_type) as i64)
}

fn is_episode_within_retention(policy: &UserPolicyRecord, episode: &Episode) -> bool {
    episode.created_at >= retention_cutoff(policy, episode.episode_type)
}

fn is_episode_summary_within_retention(
    policy: &UserPolicyRecord,
    episode: &EpisodeSummary,
) -> bool {
    episode.created_at >= retention_cutoff(policy, EpisodeType::Message)
}

fn validate_episode_retention(
    policy: &UserPolicyRecord,
    req: &CreateEpisodeRequest,
) -> Result<(), AppError> {
    let now = chrono::Utc::now();
    let created_at = req.created_at.unwrap_or(now);
    let retention_days = retention_days_for_episode_type(policy, req.episode_type);
    let oldest_allowed = now - chrono::Duration::days(retention_days as i64);
    if created_at < oldest_allowed {
        return Err(AppError(MnemoError::Validation(format!(
            "episode created_at is older than retention policy ({} days) for {:?}",
            retention_days, req.episode_type
        ))));
    }
    Ok(())
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
    let user_policies_raw: Option<String> = conn
        .get(user_policies_key(state))
        .await
        .map_err(|err| MnemoError::Redis(err.to_string()))?;
    let governance_audit_raw: Option<String> = conn
        .get(governance_audit_key(state))
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
    if let Some(json) = user_policies_raw {
        let parsed: HashMap<Uuid, UserPolicyRecord> = serde_json::from_str(&json)
            .map_err(|err| MnemoError::Serialization(err.to_string()))?;
        let mut policies = state.user_policies.write().await;
        *policies = parsed;
    }
    if let Some(json) = governance_audit_raw {
        let parsed: HashMap<Uuid, Vec<GovernanceAuditRecord>> = serde_json::from_str(&json)
            .map_err(|err| MnemoError::Serialization(err.to_string()))?;
        let mut audit = state.governance_audit.write().await;
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
    let policy_snapshot = {
        let policies = state.user_policies.read().await;
        policies.clone()
    };
    let governance_audit_snapshot = {
        let audit = state.governance_audit.read().await;
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
    let user_policies_json = match serde_json::to_string(&policy_snapshot) {
        Ok(json) => json,
        Err(err) => {
            warn!(error = %err, "failed to serialize user policies for persistence");
            return;
        }
    };
    let governance_audit_json = match serde_json::to_string(&governance_audit_snapshot) {
        Ok(json) => json,
        Err(err) => {
            warn!(error = %err, "failed to serialize governance audit for persistence");
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

    if let Err(err) = redis::cmd("SET")
        .arg(user_policies_key(state))
        .arg(user_policies_json)
        .exec_async(&mut conn)
        .await
    {
        warn!(error = %err, "failed to persist user policies");
    }

    if let Err(err) = redis::cmd("SET")
        .arg(governance_audit_key(state))
        .arg(governance_audit_json)
        .exec_async(&mut conn)
        .await
    {
        warn!(error = %err, "failed to persist governance audit");
    }
}

async fn append_webhook_audit(
    state: &AppState,
    webhook_id: Uuid,
    action: &str,
    request_id: Option<String>,
    details: serde_json::Value,
) {
    const MAX_AUDIT_PER_WEBHOOK: usize = 1000;
    let record = MemoryWebhookAuditRecord {
        id: Uuid::now_v7(),
        webhook_id,
        action: action.to_string(),
        request_id,
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

async fn append_governance_audit(
    state: &AppState,
    user_id: Uuid,
    action: &str,
    request_id: Option<String>,
    details: serde_json::Value,
) {
    const MAX_AUDIT_PER_USER: usize = 1000;
    let record = GovernanceAuditRecord {
        id: Uuid::now_v7(),
        user_id,
        action: action.to_string(),
        request_id,
        details,
        at: chrono::Utc::now(),
    };
    {
        let mut audit = state.governance_audit.write().await;
        let rows = audit.entry(user_id).or_default();
        rows.push(record);
        if rows.len() > MAX_AUDIT_PER_USER {
            let overflow = rows.len() - MAX_AUDIT_PER_USER;
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

/// Emit a webhook event to all matching subscriptions for a user.
///
/// This is the core webhook delivery entry point. It records the event,
/// persists state, and spawns async delivery tasks for each subscribed hook.
/// Used by route handlers directly (for `HeadAdvanced`, `ConflictDetected`)
/// and by the ingest webhook receiver task (for `FactAdded`, `FactSuperseded`).
pub async fn emit_memory_webhook_event(
    state: &AppState,
    user_id: Uuid,
    event_type: MemoryWebhookEventType,
    request_id: Option<String>,
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
    {
        let mut event_map = state.memory_webhook_events.write().await;
        for webhook in &subscribed_hooks {
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
                request_id: request_id.clone(),
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
    } // write lock dropped here, before persist_webhook_state acquires read lock

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
        "request_id": event.request_id,
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
                event.request_id.clone(),
                serde_json::json!({
                    "event_id": event_id,
                    "reason": format!("serialize failure: {err}")
                }),
            )
            .await;
            state
                .metrics
                .webhook_deliveries_failure_total
                .fetch_add(1, Ordering::Relaxed);
            state
                .metrics
                .webhook_dead_letter_total
                .fetch_add(1, Ordering::Relaxed);
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
            state
                .metrics
                .webhook_deliveries_failure_total
                .fetch_add(1, Ordering::Relaxed);
            record_webhook_delivery_failure(&state, webhook_id).await;
            if dead_letter {
                append_webhook_audit(
                    &state,
                    webhook_id,
                    "delivery_dead_letter",
                    event.request_id.clone(),
                    serde_json::json!({
                        "event_id": event_id,
                        "reason": "circuit_or_rate_limited"
                    }),
                )
                .await;
                state
                    .metrics
                    .webhook_dead_letter_total
                    .fetch_add(1, Ordering::Relaxed);
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
        let delivery_id = Uuid::now_v7().to_string();
        let signature_header = webhook
            .signing_secret
            .as_ref()
            .map(|secret| build_webhook_signature(secret, &timestamp, &serialized));

        let mut request = state
            .webhook_http
            .post(&webhook.target_url)
            .header("content-type", "application/json")
            .header("x-mnemo-event-id", event.id.to_string())
            .header("x-mnemo-delivery-id", delivery_id)
            .header(
                "x-mnemo-event-type",
                webhook_event_type_str(event.event_type),
            )
            .header("x-mnemo-timestamp", timestamp)
            .timeout(timeout)
            .body(serialized.clone());

        if let Some(request_id) = event.request_id.as_ref() {
            request = request.header("x-mnemo-request-id", request_id.as_str());
        }

        if let Some(sig) = signature_header {
            request = request.header("x-mnemo-signature", sig);
        }

        match request.send().await {
            Ok(response) if response.status().is_success() => {
                update_webhook_delivery_status(
                    &state, webhook_id, event_id, attempt, true, false, None,
                )
                .await;
                state
                    .metrics
                    .webhook_deliveries_success_total
                    .fetch_add(1, Ordering::Relaxed);
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
                state
                    .metrics
                    .webhook_deliveries_failure_total
                    .fetch_add(1, Ordering::Relaxed);
                record_webhook_delivery_failure(&state, webhook_id).await;
                if dead_letter {
                    append_webhook_audit(
                        &state,
                        webhook_id,
                        "delivery_dead_letter",
                        event.request_id.clone(),
                        serde_json::json!({
                            "event_id": event_id,
                            "reason": err
                        }),
                    )
                    .await;
                    state
                        .metrics
                        .webhook_dead_letter_total
                        .fetch_add(1, Ordering::Relaxed);
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
                state
                    .metrics
                    .webhook_deliveries_failure_total
                    .fetch_add(1, Ordering::Relaxed);
                record_webhook_delivery_failure(&state, webhook_id).await;
                if dead_letter {
                    append_webhook_audit(
                        &state,
                        webhook_id,
                        "delivery_dead_letter",
                        event.request_id.clone(),
                        serde_json::json!({
                            "event_id": event_id,
                            "reason": err.to_string()
                        }),
                    )
                    .await;
                    state
                        .metrics
                        .webhook_dead_letter_total
                        .fetch_add(1, Ordering::Relaxed);
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
        .route("/metrics", get(metrics))
        .route("/api/v1/ops/summary", get(get_ops_summary))
        .route("/api/v1/ops/compression", get(get_ops_compression))
        .route("/api/v1/ops/incidents", get(get_ops_incidents))
        .route("/api/v1/traces/:request_id", get(get_trace_by_request_id))
        .route(
            "/api/v1/evidence/webhooks/:id/export",
            get(export_webhook_evidence_bundle),
        )
        .route(
            "/api/v1/evidence/governance/:user/export",
            get(export_governance_evidence_bundle),
        )
        .route(
            "/api/v1/evidence/traces/:request_id/export",
            get(export_trace_evidence_bundle),
        )
        .route("/api/v1/audit/export", get(audit_export))
        // Users
        .route("/api/v1/users", post(create_user))
        .route("/api/v1/users", get(list_users))
        .route("/api/v1/users/:id", get(get_user))
        .route("/api/v1/users/:id", put(update_user))
        .route("/api/v1/users/:id", delete(delete_user))
        .route("/api/v1/policies/:user", get(get_user_policy))
        .route("/api/v1/policies/:user", put(upsert_user_policy))
        .route("/api/v1/policies/:user/preview", post(preview_user_policy))
        .route("/api/v1/policies/:user/audit", get(list_user_policy_audit))
        .route(
            "/api/v1/policies/:user/violations",
            get(list_user_policy_violations),
        )
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
        // Session messages (for LangChain/LlamaIndex SDK adapters)
        .route(
            "/api/v1/sessions/:session_id/messages",
            get(get_session_messages).delete(delete_session_messages),
        )
        .route(
            "/api/v1/sessions/:session_id/messages/:idx",
            delete(delete_session_message_by_idx),
        )
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
        .route("/api/v1/memory/feedback", post(memory_retrieval_feedback))
        // Memory API (high-level DX)
        .route("/api/v1/memory", post(remember_memory))
        .route("/api/v1/memory/extract", post(extract_memory))
        .route("/api/v1/memory/:user/context", post(get_memory_context))
        .route(
            "/api/v1/memory/:user/changes_since",
            post(memory_changes_since),
        )
        .route("/api/v1/memory/:user/conflict_radar", post(conflict_radar))
        .route("/api/v1/users/:user/coherence", get(get_user_coherence))
        .route(
            "/api/v1/memory/:user/causal_recall",
            post(causal_recall_chains),
        )
        .route(
            "/api/v1/memory/:user/time_travel/trace",
            post(time_travel_trace),
        )
        .route(
            "/api/v1/memory/:user/time_travel/summary",
            post(time_travel_summary),
        )
        .route(
            "/api/v1/memory/webhooks",
            post(register_memory_webhook).get(list_memory_webhooks),
        )
        .route(
            "/api/v1/memory/webhooks/:id",
            get(get_memory_webhook)
                .patch(update_memory_webhook)
                .delete(delete_memory_webhook),
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
            "/api/v1/agents/:agent_id/experience/importance",
            get(list_experience_importance),
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
        // Graph knowledge API
        .route("/api/v1/entities/:id/subgraph", get(get_subgraph))
        .route("/api/v1/graph/:user/entities", get(graph_list_entities))
        .route(
            "/api/v1/graph/:user/entities/:entity_id",
            get(graph_get_entity),
        )
        .route("/api/v1/graph/:user/edges", get(graph_list_edges))
        .route(
            "/api/v1/graph/:user/neighbors/:entity_id",
            get(graph_neighbors),
        )
        .route("/api/v1/graph/:user/community", get(graph_community))
        .route("/api/v1/graph/:user/path", get(graph_shortest_path))
        // LLM span tracing
        .route(
            "/api/v1/spans/request/:request_id",
            get(list_spans_by_request),
        )
        .route("/api/v1/spans/user/:user_id", get(list_spans_by_user))
        // Sleep-time compute — memory digest
        .route(
            "/api/v1/memory/:user/digest",
            get(get_memory_digest).post(refresh_memory_digest),
        )
        // Raw Vector API (external vector DB interface for AnythingLLM, etc.)
        .route(
            "/api/v1/vectors/:namespace",
            post(vectors_upsert).delete(vectors_delete_namespace),
        )
        .route("/api/v1/vectors/:namespace/query", post(vectors_query))
        .route(
            "/api/v1/vectors/:namespace/delete",
            post(vectors_delete_ids),
        )
        .route("/api/v1/vectors/:namespace/count", get(vectors_count))
        .route("/api/v1/vectors/:namespace/exists", get(vectors_exists))
        // Allow larger request bodies for import payloads.
        .layer(DefaultBodyLimit::max(64 * 1024 * 1024))
        .with_state(state)
        // Dashboard (no state needed — serves embedded static files)
        .merge(crate::dashboard::dashboard_routes())
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

async fn metrics(
    State(state): State<AppState>,
) -> (StatusCode, [(&'static str, &'static str); 1], String) {
    let http_requests_total = state.metrics.http_requests_total.load(Ordering::Relaxed);
    let http_responses_2xx = state.metrics.http_responses_2xx.load(Ordering::Relaxed);
    let http_responses_4xx = state.metrics.http_responses_4xx.load(Ordering::Relaxed);
    let http_responses_5xx = state.metrics.http_responses_5xx.load(Ordering::Relaxed);
    let webhook_deliveries_success_total = state
        .metrics
        .webhook_deliveries_success_total
        .load(Ordering::Relaxed);
    let webhook_deliveries_failure_total = state
        .metrics
        .webhook_deliveries_failure_total
        .load(Ordering::Relaxed);
    let webhook_dead_letter_total = state
        .metrics
        .webhook_dead_letter_total
        .load(Ordering::Relaxed);
    let webhook_retry_queued_total = state
        .metrics
        .webhook_retry_queued_total
        .load(Ordering::Relaxed);
    let webhook_replay_requests_total = state
        .metrics
        .webhook_replay_requests_total
        .load(Ordering::Relaxed);
    let policy_update_total = state.metrics.policy_update_total.load(Ordering::Relaxed);
    let policy_violation_total = state.metrics.policy_violation_total.load(Ordering::Relaxed);
    let agent_identity_reads_total = state
        .metrics
        .agent_identity_reads_total
        .load(Ordering::Relaxed);
    let agent_identity_updates_total = state
        .metrics
        .agent_identity_updates_total
        .load(Ordering::Relaxed);
    let agent_experience_events_total = state
        .metrics
        .agent_experience_events_total
        .load(Ordering::Relaxed);
    let agent_promotion_proposals_total = state
        .metrics
        .agent_promotion_proposals_total
        .load(Ordering::Relaxed);

    let (webhook_events_total, webhook_events_pending, webhook_events_dead_letter) = {
        let events_map = state.memory_webhook_events.read().await;
        let total: usize = events_map.values().map(|rows| rows.len()).sum();
        let pending: usize = events_map
            .values()
            .map(|rows| {
                rows.iter()
                    .filter(|e| !e.delivered && !e.dead_letter)
                    .count()
            })
            .sum();
        let dead: usize = events_map
            .values()
            .map(|rows| rows.iter().filter(|e| e.dead_letter).count())
            .sum();
        (total, pending, dead)
    };

    let mut body = format!(
        "# HELP mnemo_http_requests_total Total HTTP requests observed\n\
# TYPE mnemo_http_requests_total counter\n\
mnemo_http_requests_total {}\n\
# HELP mnemo_http_responses_2xx Total 2xx HTTP responses\n\
# TYPE mnemo_http_responses_2xx counter\n\
mnemo_http_responses_2xx {}\n\
# HELP mnemo_http_responses_4xx Total 4xx HTTP responses\n\
# TYPE mnemo_http_responses_4xx counter\n\
mnemo_http_responses_4xx {}\n\
# HELP mnemo_http_responses_5xx Total 5xx HTTP responses\n\
# TYPE mnemo_http_responses_5xx counter\n\
mnemo_http_responses_5xx {}\n\
# HELP mnemo_webhook_deliveries_success_total Successful webhook deliveries\n\
# TYPE mnemo_webhook_deliveries_success_total counter\n\
mnemo_webhook_deliveries_success_total {}\n\
# HELP mnemo_webhook_deliveries_failure_total Failed webhook delivery attempts\n\
# TYPE mnemo_webhook_deliveries_failure_total counter\n\
mnemo_webhook_deliveries_failure_total {}\n\
# HELP mnemo_webhook_dead_letter_total Webhook events moved to dead-letter\n\
# TYPE mnemo_webhook_dead_letter_total counter\n\
mnemo_webhook_dead_letter_total {}\n\
# HELP mnemo_webhook_retry_queued_total Manual webhook retries queued\n\
# TYPE mnemo_webhook_retry_queued_total counter\n\
mnemo_webhook_retry_queued_total {}\n\
# HELP mnemo_webhook_replay_requests_total Webhook replay API requests\n\
# TYPE mnemo_webhook_replay_requests_total counter\n\
mnemo_webhook_replay_requests_total {}\n\
# HELP mnemo_webhook_events_total Retained webhook event rows\n\
# TYPE mnemo_webhook_events_total gauge\n\
mnemo_webhook_events_total {}\n\
# HELP mnemo_webhook_events_pending Retained pending webhook events\n\
# TYPE mnemo_webhook_events_pending gauge\n\
mnemo_webhook_events_pending {}\n\
# HELP mnemo_webhook_events_dead_letter Retained dead-letter webhook events\n\
# TYPE mnemo_webhook_events_dead_letter gauge\n\
mnemo_webhook_events_dead_letter {}\n",
        http_requests_total,
        http_responses_2xx,
        http_responses_4xx,
        http_responses_5xx,
        webhook_deliveries_success_total,
        webhook_deliveries_failure_total,
        webhook_dead_letter_total,
        webhook_retry_queued_total,
        webhook_replay_requests_total,
        webhook_events_total,
        webhook_events_pending,
        webhook_events_dead_letter,
    );

    body.push_str(&format!(
        "# HELP mnemo_policy_update_total User policy update operations\n\
# TYPE mnemo_policy_update_total counter\n\
mnemo_policy_update_total {}\n\
# HELP mnemo_policy_violation_total Policy violations blocked by server\n\
# TYPE mnemo_policy_violation_total counter\n\
mnemo_policy_violation_total {}\n\
# HELP mnemo_agent_identity_reads_total Agent identity read operations\n\
# TYPE mnemo_agent_identity_reads_total counter\n\
mnemo_agent_identity_reads_total {}\n\
# HELP mnemo_agent_identity_updates_total Agent identity update operations\n\
# TYPE mnemo_agent_identity_updates_total counter\n\
mnemo_agent_identity_updates_total {}\n\
# HELP mnemo_agent_experience_events_total Agent experience events created\n\
# TYPE mnemo_agent_experience_events_total counter\n\
mnemo_agent_experience_events_total {}\n\
# HELP mnemo_agent_promotion_proposals_total Agent promotion proposals created\n\
# TYPE mnemo_agent_promotion_proposals_total counter\n\
mnemo_agent_promotion_proposals_total {}\n",
        policy_update_total,
        policy_violation_total,
        agent_identity_reads_total,
        agent_identity_updates_total,
        agent_experience_events_total,
        agent_promotion_proposals_total,
    ));

    (
        StatusCode::OK,
        [("content-type", "text/plain; version=0.0.4")],
        body,
    )
}

async fn get_ops_summary(
    State(state): State<AppState>,
    Query(query): Query<OpsSummaryQuery>,
) -> Json<OpsSummaryResponse> {
    let window_seconds = query.window_seconds.unwrap_or(300).clamp(1, 86_400);
    let window_start = chrono::Utc::now() - chrono::Duration::seconds(window_seconds as i64);

    let http_requests_total = state.metrics.http_requests_total.load(Ordering::Relaxed);
    let http_responses_2xx = state.metrics.http_responses_2xx.load(Ordering::Relaxed);
    let http_responses_4xx = state.metrics.http_responses_4xx.load(Ordering::Relaxed);
    let http_responses_5xx = state.metrics.http_responses_5xx.load(Ordering::Relaxed);
    let webhook_deliveries_success_total = state
        .metrics
        .webhook_deliveries_success_total
        .load(Ordering::Relaxed);
    let webhook_deliveries_failure_total = state
        .metrics
        .webhook_deliveries_failure_total
        .load(Ordering::Relaxed);
    let webhook_dead_letter_total = state
        .metrics
        .webhook_dead_letter_total
        .load(Ordering::Relaxed);
    let policy_update_total = state.metrics.policy_update_total.load(Ordering::Relaxed);
    let policy_violation_total = state.metrics.policy_violation_total.load(Ordering::Relaxed);
    let agent_identity_reads_total = state
        .metrics
        .agent_identity_reads_total
        .load(Ordering::Relaxed);
    let agent_identity_updates_total = state
        .metrics
        .agent_identity_updates_total
        .load(Ordering::Relaxed);
    let agent_experience_events_total = state
        .metrics
        .agent_experience_events_total
        .load(Ordering::Relaxed);
    let agent_promotion_proposals_total = state
        .metrics
        .agent_promotion_proposals_total
        .load(Ordering::Relaxed);

    let active_webhooks = {
        let hooks = state.memory_webhooks.read().await;
        hooks.values().filter(|h| h.enabled).count()
    };

    let (dead_letter_backlog, pending_webhook_events, webhook_audit_events_in_window) = {
        let events_map = state.memory_webhook_events.read().await;
        let dead = events_map
            .values()
            .map(|rows| rows.iter().filter(|row| row.dead_letter).count())
            .sum();
        let pending = events_map
            .values()
            .map(|rows| {
                rows.iter()
                    .filter(|row| !row.delivered && !row.dead_letter)
                    .count()
            })
            .sum();

        let webhook_audit = state.memory_webhook_audit.read().await;
        let audit_in_window = webhook_audit
            .values()
            .map(|rows| rows.iter().filter(|row| row.at >= window_start).count())
            .sum();
        (dead, pending, audit_in_window)
    };

    let governance_audit_events_in_window = {
        let governance = state.governance_audit.read().await;
        governance
            .values()
            .map(|rows| rows.iter().filter(|row| row.at >= window_start).count())
            .sum()
    };

    Json(OpsSummaryResponse {
        window_seconds,
        http_requests_total,
        http_responses_2xx,
        http_responses_4xx,
        http_responses_5xx,
        webhook_deliveries_success_total,
        webhook_deliveries_failure_total,
        webhook_dead_letter_total,
        policy_update_total,
        policy_violation_total,
        agent_identity_reads_total,
        agent_identity_updates_total,
        agent_experience_events_total,
        agent_promotion_proposals_total,
        active_webhooks,
        dead_letter_backlog,
        pending_webhook_events,
        governance_audit_events_in_window,
        webhook_audit_events_in_window,
    })
}

// ─── Temporal Tensor Compression ───────────────────────────────────

async fn get_ops_compression(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(
        state
            .compression_stats
            .to_json(&state.compression_config, state.embedding_dimensions),
    )
}

/// Run one compression sweep: iterate episode collection, quantize old vectors.
///
/// This is public so `main.rs` can call it from the background task.
pub async fn run_compression_sweep(state: &AppState) -> Result<u64, MnemoError> {
    use mnemo_retrieval::compression::{quantize_for_tier, CompressionTier};

    let config = &state.compression_config;
    let stats = &state.compression_stats;

    stats.reset_tier_counts();
    let mut compressed_count: u64 = 0;
    let mut examined_count: u64 = 0;
    let mut offset: Option<String> = None;
    let batch_size = 100u32;

    loop {
        if examined_count >= config.max_points_per_sweep as u64 {
            break;
        }

        let (points, next_offset) = state
            .vector_store
            .scroll_collection("episodes", None, batch_size, offset)
            .await?;

        if points.is_empty() {
            break;
        }

        for pt in &points {
            examined_count += 1;

            // Determine created_at from payload
            let created_at_epoch = pt
                .payload
                .get("created_at")
                .and_then(|v| v.as_f64());

            let current_tier_str = pt
                .payload
                .get("compression_tier")
                .and_then(|v| v.as_str())
                .unwrap_or("full");
            let current_tier =
                CompressionTier::from_str_opt(current_tier_str).unwrap_or(CompressionTier::Full);

            // If no created_at, treat as Tier 0 (full)
            let target_tier = match created_at_epoch {
                Some(ts) => {
                    let created = chrono::DateTime::from_timestamp(ts as i64, 0)
                        .unwrap_or_else(|| chrono::Utc::now());
                    config.tier_for_timestamp(created)
                }
                None => CompressionTier::Full,
            };

            stats.increment_tier(target_tier);

            // Only compress if target tier is higher than current tier
            if target_tier > current_tier && !pt.vector.is_empty() {
                let compressed_vector = quantize_for_tier(&pt.vector, target_tier);

                // Build updated payload: merge compression_tier into existing payload
                let mut payload = pt.payload.clone();
                if let serde_json::Value::Object(ref mut map) = payload {
                    map.insert(
                        "compression_tier".to_string(),
                        serde_json::Value::String(target_tier.as_str().to_string()),
                    );
                }

                state
                    .vector_store
                    .upsert_compressed_point("episodes", pt.id, compressed_vector, payload)
                    .await?;
                compressed_count += 1;
            }
        }

        offset = next_offset;
        if offset.is_none() {
            break;
        }
    }

    stats
        .last_sweep_compressed
        .store(compressed_count, Ordering::Relaxed);
    stats
        .last_sweep_examined
        .store(examined_count, Ordering::Relaxed);
    stats.last_sweep_epoch.store(
        chrono::Utc::now().timestamp() as u64,
        Ordering::Relaxed,
    );
    stats.total_sweeps.fetch_add(1, Ordering::Relaxed);

    Ok(compressed_count)
}

async fn get_ops_incidents(
    State(state): State<AppState>,
    Query(query): Query<OpsSummaryQuery>,
) -> Json<OpsIncidentsResponse> {
    let window_seconds = query.window_seconds.unwrap_or(300).clamp(1, 86_400);
    let window_start = chrono::Utc::now() - chrono::Duration::seconds(window_seconds as i64);

    let http_responses_5xx = state.metrics.http_responses_5xx.load(Ordering::Relaxed);
    let policy_violation_total = state.metrics.policy_violation_total.load(Ordering::Relaxed);

    let (dead_letter_backlog, pending_webhook_events, open_circuit_incidents) = {
        let events_map = state.memory_webhook_events.read().await;
        let dead = events_map
            .values()
            .map(|rows| rows.iter().filter(|row| row.dead_letter).count())
            .sum::<usize>();
        let pending = events_map
            .values()
            .map(|rows| {
                rows.iter()
                    .filter(|row| !row.delivered && !row.dead_letter)
                    .count()
            })
            .sum::<usize>();

        let hooks = state.memory_webhooks.read().await;
        let runtime = state.webhook_runtime.read().await;
        let incidents = hooks
            .values()
            .filter_map(|hook| {
                let rt = runtime.get(&hook.id)?;
                let until = rt.circuit_open_until?;
                if until < chrono::Utc::now() {
                    return None;
                }
                Some(OpsIncidentResponse {
                    id: format!("circuit-open-{}", hook.id),
                    kind: "circuit_open".to_string(),
                    severity: "high".to_string(),
                    title: format!("Circuit open: {}", hook.target_url),
                    summary: format!(
                        "Webhook delivery is paused after {} consecutive failures.",
                        rt.consecutive_failures
                    ),
                    action_label: "Open Webhook Ops".to_string(),
                    action_href: format!("/_/webhooks/{}", hook.id),
                    resource_id: Some(hook.id.to_string()),
                    resource_label: Some(hook.target_url.clone()),
                    request_id: None,
                    opened_at: Some(until),
                })
            })
            .collect::<Vec<_>>();
        (dead, pending, incidents)
    };

    let mut incidents = Vec::new();

    if dead_letter_backlog > 0 {
        incidents.push(OpsIncidentResponse {
            id: "dead-letter-backlog".to_string(),
            kind: "dead_letter_spike".to_string(),
            severity: if dead_letter_backlog >= 10 {
                "high".to_string()
            } else {
                "medium".to_string()
            },
            title: format!("Dead-letter backlog: {} event(s)", dead_letter_backlog),
            summary: format!(
                "Webhook delivery has {} dead-letter event(s) awaiting operator action.",
                dead_letter_backlog
            ),
            action_label: "Review dead-letter queue".to_string(),
            action_href: "/_/webhooks?filter=dead-letter".to_string(),
            resource_id: None,
            resource_label: None,
            request_id: None,
            opened_at: None,
        });
    }

    if pending_webhook_events >= 25 {
        incidents.push(OpsIncidentResponse {
            id: "pending-webhook-backlog".to_string(),
            kind: "pending_backlog".to_string(),
            severity: "medium".to_string(),
            title: format!(
                "Pending delivery backlog: {} event(s)",
                pending_webhook_events
            ),
            summary: "Webhook deliveries are accumulating faster than they are clearing."
                .to_string(),
            action_label: "Inspect webhook throughput".to_string(),
            action_href: "/_/webhooks?filter=backlog".to_string(),
            resource_id: None,
            resource_label: None,
            request_id: None,
            opened_at: None,
        });
    }

    if http_responses_5xx > 0 {
        incidents.push(OpsIncidentResponse {
            id: "server-5xx".to_string(),
            kind: "server_errors".to_string(),
            severity: "high".to_string(),
            title: format!("Server 5xx responses observed: {}", http_responses_5xx),
            summary: "The API has emitted one or more server-side errors during the current process lifetime.".to_string(),
            action_label: "Inspect traces".to_string(),
            action_href: "/_/traces".to_string(),
            resource_id: None,
            resource_label: None,
            request_id: None,
            opened_at: None,
        });
    }

    let recent_policy_violations = {
        let governance = state.governance_audit.read().await;
        let mut rows = governance
            .values()
            .flat_map(|rows| rows.iter())
            .filter(|row| row.at >= window_start && row.action.contains("policy_violation"))
            .cloned()
            .collect::<Vec<GovernanceAuditRecord>>();
        rows.sort_by(|a, b| b.at.cmp(&a.at));
        rows
    };

    if policy_violation_total > 0 || !recent_policy_violations.is_empty() {
        for row in recent_policy_violations.iter().take(3) {
            incidents.push(OpsIncidentResponse {
                id: format!("policy-violation-{}", row.id),
                kind: "policy_violation".to_string(),
                severity: "medium".to_string(),
                title: format!("Policy violation: {}", row.action),
                summary: summarize_governance_violation(row),
                action_label: "Open governance center".to_string(),
                action_href: format!("/_/governance/{}", row.user_id),
                resource_id: Some(row.user_id.to_string()),
                resource_label: None,
                request_id: row.request_id.clone(),
                opened_at: Some(row.at),
            });
        }
        if recent_policy_violations.is_empty() {
            incidents.push(OpsIncidentResponse {
                id: "policy-violation-total".to_string(),
                kind: "policy_violation".to_string(),
                severity: "medium".to_string(),
                title: format!("Policy violations observed: {}", policy_violation_total),
                summary:
                    "One or more policy checks have blocked operations in this process lifetime."
                        .to_string(),
                action_label: "Open governance center".to_string(),
                action_href: "/_/governance".to_string(),
                resource_id: None,
                resource_label: None,
                request_id: None,
                opened_at: None,
            });
        }
    }

    incidents.extend(open_circuit_incidents);
    incidents.sort_by_key(|b| std::cmp::Reverse(incident_sort_key(b)));

    Json(OpsIncidentsResponse {
        window_seconds,
        total_active: incidents.len(),
        incidents,
    })
}

fn summarize_governance_violation(row: &GovernanceAuditRecord) -> String {
    if let Some(target) = row.details.get("target_url").and_then(|v| v.as_str()) {
        return format!("Blocked target: {}", target);
    }
    if let Some(kind) = row.details.get("episode_type").and_then(|v| v.as_str()) {
        return format!("Blocked {} policy action.", kind);
    }
    if let Some(reason) = row.details.get("reason").and_then(|v| v.as_str()) {
        return reason.to_string();
    }
    "A governance policy blocked an operation and should be reviewed.".to_string()
}

fn incident_sort_key(incident: &OpsIncidentResponse) -> (u8, chrono::DateTime<chrono::Utc>) {
    let sev = match incident.severity.as_str() {
        "high" => 3,
        "medium" => 2,
        _ => 1,
    };
    let at = incident
        .opened_at
        .unwrap_or(chrono::DateTime::<chrono::Utc>::UNIX_EPOCH);
    (sev, at)
}

// ── GET /api/v1/audit/export ────────────────────────────────────────
//
// SOC 2 / compliance audit log export.  Returns a unified, time-bounded
// list of all governance and webhook audit events, suitable for shipping
// to a SIEM, exporting for auditors, or feeding into compliance tooling.
//
// Query parameters:
//   from              ISO 8601 datetime (default: 30 days ago)
//   to                ISO 8601 datetime (default: now)
//   limit             Max events returned (default: 1000, max: 10000)
//   include_governance  bool (default: true)
//   include_webhook     bool (default: true)
//   user              Optional user UUID or external_id filter

fn default_audit_export_from() -> chrono::DateTime<chrono::Utc> {
    chrono::Utc::now() - chrono::Duration::days(30)
}

fn default_audit_export_to() -> chrono::DateTime<chrono::Utc> {
    chrono::Utc::now()
}

fn default_audit_export_limit() -> u32 {
    1000
}

fn default_true_audit() -> bool {
    true
}

#[derive(Debug, Deserialize)]
struct AuditExportQuery {
    #[serde(default = "default_audit_export_from")]
    from: chrono::DateTime<chrono::Utc>,
    #[serde(default = "default_audit_export_to")]
    to: chrono::DateTime<chrono::Utc>,
    #[serde(default = "default_audit_export_limit")]
    limit: u32,
    #[serde(default = "default_true_audit")]
    include_governance: bool,
    #[serde(default = "default_true_audit")]
    include_webhook: bool,
    #[serde(default)]
    user: Option<String>,
}

#[derive(Debug, Serialize)]
struct AuditExportRecord {
    /// "governance" or "webhook"
    audit_type: &'static str,
    id: Uuid,
    user_id: Uuid,
    action: String,
    at: chrono::DateTime<chrono::Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    request_id: Option<String>,
    details: serde_json::Value,
    /// Only present for webhook audit records
    #[serde(skip_serializing_if = "Option::is_none")]
    webhook_id: Option<Uuid>,
}

#[derive(Debug, Serialize)]
struct AuditExportResponse {
    ok: bool,
    from: chrono::DateTime<chrono::Utc>,
    to: chrono::DateTime<chrono::Utc>,
    total: usize,
    records: Vec<AuditExportRecord>,
}

async fn audit_export(
    State(state): State<AppState>,
    Query(query): Query<AuditExportQuery>,
) -> Result<Response, AppError> {
    if query.to <= query.from {
        return Err(AppError(MnemoError::Validation(
            "'to' must be after 'from'".into(),
        )));
    }

    let max_records = query.limit.clamp(1, 10_000) as usize;

    // Resolve optional user filter to a UUID
    let user_filter_uuid: Option<Uuid> = if let Some(ref ident) = query.user {
        match find_user_by_identifier(&state, ident.trim()).await {
            Ok(u) => Some(u.id),
            Err(_) => Some(Uuid::nil()), // unknown user — no records can match
        }
    } else {
        None
    };

    let mut records: Vec<AuditExportRecord> = Vec::new();

    // Governance audit
    if query.include_governance {
        let governance_map = state.governance_audit.read().await;
        for (user_id, events) in governance_map.iter() {
            if let Some(filter) = user_filter_uuid {
                if *user_id != filter {
                    continue;
                }
            }
            for ev in events {
                if ev.at < query.from || ev.at > query.to {
                    continue;
                }
                records.push(AuditExportRecord {
                    audit_type: "governance",
                    id: ev.id,
                    user_id: ev.user_id,
                    action: ev.action.clone(),
                    at: ev.at,
                    request_id: ev.request_id.clone(),
                    details: ev.details.clone(),
                    webhook_id: None,
                });
            }
        }
    }

    // Webhook audit
    if query.include_webhook {
        let webhook_audit_map = state.memory_webhook_audit.read().await;
        for (webhook_id, events) in webhook_audit_map.iter() {
            for ev in events {
                if ev.at < query.from || ev.at > query.to {
                    continue;
                }
                // Resolve user_id from webhook subscription for user filtering
                let webhook_user_id: Option<Uuid> = {
                    let webhooks = state.memory_webhooks.read().await;
                    webhooks.get(webhook_id).map(|wh| wh.user_id)
                };
                if let Some(filter) = user_filter_uuid {
                    if webhook_user_id != Some(filter) {
                        continue;
                    }
                }
                records.push(AuditExportRecord {
                    audit_type: "webhook",
                    id: ev.id,
                    user_id: webhook_user_id.unwrap_or(Uuid::nil()),
                    action: ev.action.clone(),
                    at: ev.at,
                    request_id: ev.request_id.clone(),
                    details: ev.details.clone(),
                    webhook_id: Some(*webhook_id),
                });
            }
        }
    }

    // Sort newest-first then cap
    records.sort_by(|a, b| b.at.cmp(&a.at));
    records.truncate(max_records);

    let total = records.len();
    let response_body = AuditExportResponse {
        ok: true,
        from: query.from,
        to: query.to,
        total,
        records,
    };

    // HMAC-sign the audit export for SOC 2 tamper evidence
    if let Some(ref secret) = state.audit_signing_secret {
        let serialized = serde_json::to_string(&response_body).unwrap_or_default();
        let timestamp = chrono::Utc::now().timestamp().to_string();
        let sig = build_webhook_signature(secret, &timestamp, &serialized);
        Ok((
            [(
                axum::http::header::HeaderName::from_static("x-mnemo-audit-signature"),
                axum::http::header::HeaderValue::from_str(&sig)
                    .unwrap_or_else(|_| axum::http::header::HeaderValue::from_static("invalid")),
            )],
            Json(response_body),
        )
            .into_response())
    } else {
        Ok(Json(response_body).into_response())
    }
}

async fn get_trace_by_request_id(
    State(state): State<AppState>,
    Path(request_id): Path<String>,
    Query(query): Query<TraceLookupQuery>,
) -> Result<Json<TraceLookupResponse>, AppError> {
    Ok(Json(
        lookup_trace_by_request_id(&state, request_id, query).await?,
    ))
}

async fn lookup_trace_by_request_id(
    state: &AppState,
    request_id: String,
    query: TraceLookupQuery,
) -> Result<TraceLookupResponse, AppError> {
    let request_id = request_id.trim().to_string();
    if request_id.is_empty() {
        return Err(AppError(MnemoError::Validation(
            "request_id is required".into(),
        )));
    }

    if query.to <= query.from {
        return Err(AppError(MnemoError::Validation(
            "'to' must be after 'from'".to_string(),
        )));
    }

    let max_matches = query.limit.clamp(1, 500) as usize;
    let user_filter = query
        .user
        .as_ref()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty());

    // Resolve optional user filter to a UUID for post-index filtering.
    // We do this eagerly so the index path doesn't need to scan all users.
    let user_filter_uuid: Option<Uuid> = if let Some(ref filter) = user_filter {
        match find_user_by_identifier(state, filter).await {
            Ok(u) => Some(u.id),
            Err(_) => {
                // Unknown user — no episodes can match
                Some(Uuid::nil())
            }
        }
    } else {
        None
    };

    let mut matched_episodes = Vec::new();
    if query.include_episodes {
        // O(1) index lookup: `rid_episodes:{request_id}` sorted set written at
        // episode create time. Falls back gracefully if the index has no entry
        // (e.g. the episode predates this feature or had no request_id).
        let from_ms = query.from.timestamp_millis();
        let to_ms = query.to.timestamp_millis();
        let index_hits = state
            .state_store
            .get_episodes_by_request_id(&request_id, from_ms, to_ms, max_matches)
            .await
            .unwrap_or_default();

        for (ep_id, uid, sess_id) in index_hits {
            // Apply user filter if provided
            if let Some(filter_uuid) = user_filter_uuid {
                if uid != filter_uuid {
                    continue;
                }
            }
            // Fetch the full episode to get created_at and content preview
            match state.state_store.get_episode(ep_id).await {
                Ok(episode) => {
                    matched_episodes.push(TraceEpisodeRef {
                        user_id: uid,
                        session_id: sess_id,
                        episode_id: ep_id,
                        created_at: episode.created_at,
                        preview: preview_text(&episode.content, 140),
                    });
                }
                Err(_) => {
                    // Episode was deleted or unavailable — skip silently
                }
            }
        }

        matched_episodes.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        matched_episodes.truncate(max_matches);
    }

    let matched_webhook_events: Vec<MemoryWebhookEventRecord> = {
        if !query.include_webhook_events {
            Vec::new()
        } else {
            let mut rows: Vec<MemoryWebhookEventRecord> = {
                let event_map = state.memory_webhook_events.read().await;
                event_map
                    .values()
                    .flat_map(|items| items.iter().cloned())
                    .filter(|row| {
                        row.request_id
                            .as_deref()
                            .is_some_and(|rid| rid == request_id)
                    })
                    .filter(|row| row.created_at >= query.from && row.created_at <= query.to)
                    .collect()
            };
            rows.sort_by(|a, b| b.created_at.cmp(&a.created_at));
            rows.truncate(max_matches);
            rows
        }
    };

    let matched_webhook_audit: Vec<MemoryWebhookAuditRecord> = {
        if !query.include_webhook_audit {
            Vec::new()
        } else {
            let mut rows: Vec<MemoryWebhookAuditRecord> = {
                let audit_map = state.memory_webhook_audit.read().await;
                audit_map
                    .values()
                    .flat_map(|items| items.iter().cloned())
                    .filter(|row| {
                        row.request_id
                            .as_deref()
                            .is_some_and(|rid| rid == request_id)
                    })
                    .filter(|row| row.at >= query.from && row.at <= query.to)
                    .collect()
            };
            rows.sort_by(|a, b| b.at.cmp(&a.at));
            rows.truncate(max_matches);
            rows
        }
    };

    let matched_governance_audit: Vec<GovernanceAuditRecord> = {
        if !query.include_governance_audit {
            Vec::new()
        } else {
            let mut rows: Vec<GovernanceAuditRecord> = {
                let audit_map = state.governance_audit.read().await;
                audit_map
                    .values()
                    .flat_map(|items| items.iter().cloned())
                    .filter(|row| {
                        row.request_id
                            .as_deref()
                            .is_some_and(|rid| rid == request_id)
                    })
                    .filter(|row| row.at >= query.from && row.at <= query.to)
                    .collect()
            };
            rows.sort_by(|a, b| b.at.cmp(&a.at));
            rows.truncate(max_matches);
            rows
        }
    };

    let summary = serde_json::json!({
        "episode_matches": matched_episodes.len(),
        "webhook_event_matches": matched_webhook_events.len(),
        "webhook_audit_matches": matched_webhook_audit.len(),
        "governance_audit_matches": matched_governance_audit.len(),
        "filters": {
            "from": query.from,
            "to": query.to,
            "limit": max_matches,
            "include_episodes": query.include_episodes,
            "include_webhook_events": query.include_webhook_events,
            "include_webhook_audit": query.include_webhook_audit,
            "include_governance_audit": query.include_governance_audit,
            "user": query.user,
        }
    });

    Ok(TraceLookupResponse {
        request_id,
        matched_episodes,
        matched_webhook_events,
        matched_webhook_audit,
        matched_governance_audit,
        summary,
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

async fn get_user_policy(
    State(state): State<AppState>,
    Path(user_identifier): Path<String>,
) -> Result<Json<UserPolicyResponse>, AppError> {
    let user_identifier_trimmed = user_identifier.trim().to_string();
    let user = find_user_by_identifier(&state, user_identifier_trimmed.as_str()).await?;
    let policy = get_or_create_user_policy(&state, user.id, user_identifier_trimmed).await;
    Ok(Json(UserPolicyResponse { policy }))
}

async fn upsert_user_policy(
    State(state): State<AppState>,
    ctx: Option<Extension<RequestContext>>,
    Path(user_identifier): Path<String>,
    Json(req): Json<UpsertUserPolicyRequest>,
) -> Result<Json<UserPolicyResponse>, AppError> {
    let request_id = request_id_from_extension(ctx);
    let user = find_user_by_identifier(&state, user_identifier.trim()).await?;

    let policy = {
        let mut policies = state.user_policies.write().await;
        let now = chrono::Utc::now();
        let record = policies
            .entry(user.id)
            .or_insert_with(|| default_user_policy(user.id, user_identifier.trim().to_string()));
        *record = apply_user_policy_patch(record.clone(), &req, now);
        record.clone()
    };

    state
        .metrics
        .policy_update_total
        .fetch_add(1, Ordering::Relaxed);

    append_governance_audit(
        &state,
        user.id,
        "policy_updated",
        request_id,
        serde_json::json!({
            "retention_days_message": policy.retention_days_message,
            "retention_days_text": policy.retention_days_text,
            "retention_days_json": policy.retention_days_json,
            "webhook_domain_allowlist": policy.webhook_domain_allowlist,
            "default_memory_contract": policy.default_memory_contract,
            "default_retrieval_policy": policy.default_retrieval_policy
        }),
    )
    .await;

    persist_webhook_state(&state).await;
    Ok(Json(UserPolicyResponse { policy }))
}

async fn preview_user_policy(
    State(state): State<AppState>,
    Path(user_identifier): Path<String>,
    Json(req): Json<PreviewUserPolicyRequest>,
) -> Result<Json<UserPolicyPreviewResponse>, AppError> {
    let user_identifier_trimmed = user_identifier.trim().to_string();
    let user = find_user_by_identifier(&state, user_identifier_trimmed.as_str()).await?;
    let current_policy = get_or_create_user_policy(&state, user.id, user_identifier_trimmed).await;
    let upsert_req: UpsertUserPolicyRequest = req.into();
    let preview_policy =
        apply_user_policy_patch(current_policy.clone(), &upsert_req, chrono::Utc::now());

    let sessions = list_all_sessions_for_user(&state, user.id).await?;
    let mut affected_total = 0usize;
    let mut affected_msg = 0usize;
    let mut affected_text = 0usize;
    let mut affected_json = 0usize;

    for session in sessions {
        let episodes = list_all_episodes_for_session(&state, session.id).await?;
        for episode in episodes {
            let before = is_episode_within_retention(&current_policy, &episode);
            let after = is_episode_within_retention(&preview_policy, &episode);
            if before && !after {
                affected_total += 1;
                match episode.episode_type {
                    EpisodeType::Message => affected_msg += 1,
                    EpisodeType::Text => affected_text += 1,
                    EpisodeType::Json => affected_json += 1,
                }
            }
        }
    }

    Ok(Json(UserPolicyPreviewResponse {
        user_id: user.id,
        current_policy,
        preview_policy,
        estimated_affected_episodes_total: affected_total,
        estimated_affected_message_episodes: affected_msg,
        estimated_affected_text_episodes: affected_text,
        estimated_affected_json_episodes: affected_json,
        confidence: "estimated".to_string(),
    }))
}

async fn list_user_policy_audit(
    State(state): State<AppState>,
    Path(user_identifier): Path<String>,
    Query(query): Query<UserPolicyAuditQuery>,
) -> Result<Json<UserPolicyAuditResponse>, AppError> {
    let user = find_user_by_identifier(&state, user_identifier.trim()).await?;
    let limit = query.limit.unwrap_or(100).clamp(1, 1000) as usize;

    let mut rows = {
        let audit = state.governance_audit.read().await;
        audit.get(&user.id).cloned().unwrap_or_default()
    };
    rows.sort_by(|a, b| b.at.cmp(&a.at));
    rows.truncate(limit);

    Ok(Json(UserPolicyAuditResponse {
        user_id: user.id,
        count: rows.len(),
        audit: rows,
    }))
}

async fn list_user_policy_violations(
    State(state): State<AppState>,
    Path(user_identifier): Path<String>,
    Query(query): Query<UserPolicyViolationQuery>,
) -> Result<Json<UserPolicyViolationResponse>, AppError> {
    let from = query.from.ok_or_else(|| {
        AppError(MnemoError::Validation(
            "'from' query parameter is required".to_string(),
        ))
    })?;
    let to = query.to.ok_or_else(|| {
        AppError(MnemoError::Validation(
            "'to' query parameter is required".to_string(),
        ))
    })?;
    if to <= from {
        return Err(AppError(MnemoError::Validation(
            "'to' must be after 'from'".to_string(),
        )));
    }
    let user = find_user_by_identifier(&state, user_identifier.trim()).await?;
    let limit = query.limit.unwrap_or(100).clamp(1, 1000) as usize;

    let mut rows = {
        let audit = state.governance_audit.read().await;
        audit.get(&user.id).cloned().unwrap_or_default()
    };
    rows.retain(|row| {
        row.action.starts_with("policy_violation_") && row.at >= from && row.at <= to
    });
    rows.sort_by(|a, b| b.at.cmp(&a.at));
    rows.truncate(limit);

    Ok(Json(UserPolicyViolationResponse {
        user_id: user.id,
        from,
        to,
        count: rows.len(),
        violations: rows,
    }))
}

async fn delete_user(
    State(state): State<AppState>,
    ctx: Option<Extension<RequestContext>>,
    Path(id): Path<Uuid>,
) -> Result<Json<DeleteResponse>, AppError> {
    let request_id = request_id_from_extension(ctx);
    append_governance_audit(
        &state,
        id,
        "user_deleted",
        request_id,
        serde_json::json!({}),
    )
    .await;
    state.state_store.delete_user(id).await?;
    // Also delete vectors for GDPR compliance
    let _ = state.vector_store.delete_user_vectors(id).await;
    {
        let mut policies = state.user_policies.write().await;
        policies.remove(&id);
    }
    persist_webhook_state(&state).await;
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
    ctx: Option<Extension<RequestContext>>,
    Path(id): Path<Uuid>,
) -> Result<Json<DeleteResponse>, AppError> {
    let request_id = request_id_from_extension(ctx);
    let session = state.state_store.get_session(id).await?;
    state.state_store.delete_session(id).await?;
    append_governance_audit(
        &state,
        session.user_id,
        "session_deleted",
        request_id,
        serde_json::json!({ "session_id": id }),
    )
    .await;
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
    ctx: Option<Extension<RequestContext>>,
    Path(session_id): Path<Uuid>,
    Json(req): Json<CreateEpisodeRequest>,
) -> Result<(StatusCode, Json<Episode>), AppError> {
    let request_id = request_id_from_extension(ctx);
    let session = state.state_store.get_session(session_id).await?;
    let policy =
        get_or_create_user_policy(&state, session.user_id, session.user_id.to_string()).await;
    validate_episode_retention(&policy, &req)?;
    let req = CreateEpisodeRequest {
        metadata: metadata_with_request_id(req.metadata, request_id.as_deref()),
        ..req
    };
    let episode = state
        .state_store
        .create_episode(req, session_id, session.user_id)
        .await?;

    emit_memory_webhook_event(
        &state,
        session.user_id,
        MemoryWebhookEventType::HeadAdvanced,
        request_id,
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
    ctx: Option<Extension<RequestContext>>,
    Path(session_id): Path<Uuid>,
    Json(req): Json<BatchCreateEpisodesRequest>,
) -> Result<(StatusCode, Json<ListResponse<Episode>>), AppError> {
    let request_id = request_id_from_extension(ctx);
    let session = state.state_store.get_session(session_id).await?;
    let policy =
        get_or_create_user_policy(&state, session.user_id, session.user_id.to_string()).await;
    for (idx, ep) in req.episodes.iter().enumerate() {
        if let Err(err) = validate_episode_retention(&policy, ep) {
            return Err(AppError(MnemoError::Validation(format!(
                "episodes[{}] failed retention check: {}",
                idx, err.0
            ))));
        }
    }
    let episodes_req: Vec<CreateEpisodeRequest> = req
        .episodes
        .into_iter()
        .map(|ep| CreateEpisodeRequest {
            metadata: metadata_with_request_id(ep.metadata, request_id.as_deref()),
            ..ep
        })
        .collect();
    let episodes = state
        .state_store
        .create_episodes_batch(episodes_req, session_id, session.user_id)
        .await?;

    if let Some(last) = episodes.last() {
        emit_memory_webhook_event(
            &state,
            session.user_id,
            MemoryWebhookEventType::HeadAdvanced,
            request_id,
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

// ─── Session message routes (framework adapter endpoints) ───────────
//
// These endpoints expose raw message access required by LangChain's
// BaseChatMessageHistory and LlamaIndex's BaseChatStore adapters.

#[derive(Serialize)]
struct MessageRecord {
    /// 0-based ordinal index within the session (chronological order).
    idx: usize,
    /// Episode ID (stable unique identifier).
    id: Uuid,
    role: Option<String>,
    content: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Serialize)]
struct MessagesResponse {
    messages: Vec<MessageRecord>,
    count: usize,
    session_id: Uuid,
}

#[derive(Serialize)]
struct ClearMessagesResponse {
    deleted: u32,
    session_id: Uuid,
}

/// `GET /api/v1/sessions/:session_id/messages`
///
/// Return messages for a session in chronological order.
/// Returns a flat message-shaped projection over episodes.
/// Query params: `limit` (default 100), `after` (cursor UUID).
async fn get_session_messages(
    State(state): State<AppState>,
    Path(session_id): Path<Uuid>,
    Query(params): Query<PaginationParams>,
) -> Result<Json<MessagesResponse>, AppError> {
    // Default to 100 for messages (callers typically want full history).
    let effective_limit = if params.limit == 20 {
        100
    } else {
        params.limit
    };
    let list_params = ListEpisodesParams {
        limit: effective_limit.clamp(1, 1000),
        after: params.after,
        status: None,
    };

    // list_episodes returns newest-first; reverse to get chronological order.
    let mut episodes = state
        .state_store
        .list_episodes(session_id, list_params)
        .await?;
    episodes.reverse();

    let messages: Vec<MessageRecord> = episodes
        .into_iter()
        .enumerate()
        .map(|(idx, ep)| MessageRecord {
            idx,
            id: ep.id,
            role: ep.role.map(|r| format!("{:?}", r).to_lowercase()),
            content: ep.content,
            created_at: ep.created_at,
        })
        .collect();

    let count = messages.len();
    Ok(Json(MessagesResponse {
        messages,
        count,
        session_id,
    }))
}

/// `DELETE /api/v1/sessions/:session_id/messages`
///
/// Clear all episodes/messages for a session without deleting the session.
/// Used by LangChain `clear()` and LlamaIndex `delete_messages()`.
async fn delete_session_messages(
    State(state): State<AppState>,
    Path(session_id): Path<Uuid>,
) -> Result<Json<ClearMessagesResponse>, AppError> {
    // Verify session exists
    state.state_store.get_session(session_id).await?;

    let deleted = state
        .state_store
        .delete_session_episodes(session_id)
        .await?;

    Ok(Json(ClearMessagesResponse {
        deleted,
        session_id,
    }))
}

/// `DELETE /api/v1/sessions/:session_id/messages/:idx`
///
/// Delete a specific message by 0-based ordinal index within the session.
/// Used by LlamaIndex `delete_message(key, idx)`.
async fn delete_session_message_by_idx(
    State(state): State<AppState>,
    Path((session_id, idx)): Path<(Uuid, usize)>,
) -> Result<Json<DeleteResponse>, AppError> {
    // List all episodes in chronological order
    let list_params = ListEpisodesParams {
        limit: 10000,
        after: None,
        status: None,
    };
    let mut episodes = state
        .state_store
        .list_episodes(session_id, list_params)
        .await?;
    episodes.reverse(); // chronological

    if idx >= episodes.len() {
        return Err(MnemoError::Validation(format!(
            "Index {} out of range — session has {} messages",
            idx,
            episodes.len()
        ))
        .into());
    }

    let episode_id = episodes[idx].id;
    state.state_store.delete_episode(episode_id).await?;

    Ok(Json(DeleteResponse { deleted: true }))
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
    ctx: Option<Extension<RequestContext>>,
    Path(id): Path<Uuid>,
) -> Result<Json<DeleteResponse>, AppError> {
    let request_id = request_id_from_extension(ctx);
    let entity = state.state_store.get_entity(id).await?;
    state.state_store.delete_entity(id).await?;
    append_governance_audit(
        &state,
        entity.user_id,
        "entity_deleted",
        request_id,
        serde_json::json!({ "entity_id": id, "entity_name": entity.name }),
    )
    .await;
    Ok(Json(DeleteResponse { deleted: true }))
}

// ─── Edge routes ───────────────────────────────────────────────────

async fn query_edges(
    State(state): State<AppState>,
    Path(user_id): Path<Uuid>,
    Query(mut filter): Query<EdgeFilter>,
) -> Result<Json<ListResponse<Edge>>, AppError> {
    filter.limit = filter.limit.clamp(1, 1000);
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
    ctx: Option<Extension<RequestContext>>,
    Path(id): Path<Uuid>,
) -> Result<Json<DeleteResponse>, AppError> {
    let request_id = request_id_from_extension(ctx);
    let edge = state.state_store.get_edge(id).await?;
    state.state_store.delete_edge(id).await?;
    append_governance_audit(
        &state,
        edge.user_id,
        "edge_deleted",
        request_id,
        serde_json::json!({ "edge_id": id, "edge_label": edge.label }),
    )
    .await;
    Ok(Json(DeleteResponse { deleted: true }))
}

// ─── Context route ─────────────────────────────────────────────────

async fn get_context(
    State(state): State<AppState>,
    Path(user_id): Path<Uuid>,
    Json(req): Json<ContextRequest>,
) -> Result<Json<ContextBlock>, AppError> {
    let context = state
        .retrieval
        .get_context(user_id, &req, reranker_for_state(&state))
        .await?;
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

#[derive(Debug, Deserialize)]
struct UpdateMemoryWebhookRequest {
    /// New target URL (optional; if provided, must pass TLS/domain checks).
    #[serde(default)]
    target_url: Option<String>,
    /// Replace the signing secret (optional).
    #[serde(default)]
    signing_secret: Option<String>,
    /// Replace subscribed event types (optional).
    #[serde(default)]
    events: Option<Vec<MemoryWebhookEventType>>,
    /// Enable or disable the webhook (optional).
    #[serde(default)]
    enabled: Option<bool>,
}

#[derive(Debug, Serialize)]
struct UpdateMemoryWebhookResponse {
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
    #[serde(skip_serializing_if = "Option::is_none")]
    event: Option<MemoryWebhookEventRecord>,
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
struct UpsertUserPolicyRequest {
    #[serde(default)]
    retention_days_message: Option<u32>,
    #[serde(default)]
    retention_days_text: Option<u32>,
    #[serde(default)]
    retention_days_json: Option<u32>,
    #[serde(default)]
    webhook_domain_allowlist: Option<Vec<String>>,
    #[serde(default)]
    default_memory_contract: Option<String>,
    #[serde(default)]
    default_retrieval_policy: Option<String>,
}

#[derive(Debug, Serialize)]
struct UserPolicyResponse {
    policy: UserPolicyRecord,
}

#[derive(Debug, Deserialize)]
struct UserPolicyAuditQuery {
    #[serde(default)]
    limit: Option<u32>,
}

#[derive(Debug, Serialize)]
struct UserPolicyAuditResponse {
    user_id: Uuid,
    count: usize,
    audit: Vec<GovernanceAuditRecord>,
}

#[derive(Debug, Deserialize)]
struct UserPolicyViolationQuery {
    #[serde(default)]
    from: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default)]
    to: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default)]
    limit: Option<u32>,
}

#[derive(Debug, Serialize)]
struct UserPolicyViolationResponse {
    user_id: Uuid,
    from: chrono::DateTime<chrono::Utc>,
    to: chrono::DateTime<chrono::Utc>,
    count: usize,
    violations: Vec<GovernanceAuditRecord>,
}

#[derive(Debug, Deserialize)]
struct PreviewUserPolicyRequest {
    #[serde(default)]
    retention_days_message: Option<u32>,
    #[serde(default)]
    retention_days_text: Option<u32>,
    #[serde(default)]
    retention_days_json: Option<u32>,
    #[serde(default)]
    webhook_domain_allowlist: Option<Vec<String>>,
    #[serde(default)]
    default_memory_contract: Option<String>,
    #[serde(default)]
    default_retrieval_policy: Option<String>,
}

impl From<PreviewUserPolicyRequest> for UpsertUserPolicyRequest {
    fn from(value: PreviewUserPolicyRequest) -> Self {
        Self {
            retention_days_message: value.retention_days_message,
            retention_days_text: value.retention_days_text,
            retention_days_json: value.retention_days_json,
            webhook_domain_allowlist: value.webhook_domain_allowlist,
            default_memory_contract: value.default_memory_contract,
            default_retrieval_policy: value.default_retrieval_policy,
        }
    }
}

#[derive(Debug, Serialize)]
struct UserPolicyPreviewResponse {
    user_id: Uuid,
    current_policy: UserPolicyRecord,
    preview_policy: UserPolicyRecord,
    estimated_affected_episodes_total: usize,
    estimated_affected_message_episodes: usize,
    estimated_affected_text_episodes: usize,
    estimated_affected_json_episodes: usize,
    confidence: String,
}

#[derive(Debug, Deserialize)]
struct TimeTravelSummaryRequest {
    query: String,
    from: chrono::DateTime<chrono::Utc>,
    to: chrono::DateTime<chrono::Utc>,
    #[serde(default)]
    session: Option<String>,
    #[serde(default)]
    contract: Option<MemoryContract>,
    #[serde(default)]
    retrieval_policy: Option<AdaptiveRetrievalPolicy>,
}

#[derive(Debug, Serialize)]
struct TimeTravelSummaryResponse {
    user_id: Uuid,
    from: chrono::DateTime<chrono::Utc>,
    to: chrono::DateTime<chrono::Utc>,
    contract_applied: MemoryContract,
    retrieval_policy_applied: AdaptiveRetrievalPolicy,
    fact_count_from: usize,
    fact_count_to: usize,
    episode_count_from: usize,
    episode_count_to: usize,
    gained_fact_count: usize,
    lost_fact_count: usize,
    gained_episode_count: usize,
    lost_episode_count: usize,
    summary: String,
}

#[derive(Debug, Deserialize)]
struct OpsSummaryQuery {
    #[serde(default)]
    window_seconds: Option<u64>,
}

#[derive(Debug, Serialize)]
struct OpsSummaryResponse {
    window_seconds: u64,
    http_requests_total: u64,
    http_responses_2xx: u64,
    http_responses_4xx: u64,
    http_responses_5xx: u64,
    webhook_deliveries_success_total: u64,
    webhook_deliveries_failure_total: u64,
    webhook_dead_letter_total: u64,
    policy_update_total: u64,
    policy_violation_total: u64,
    agent_identity_reads_total: u64,
    agent_identity_updates_total: u64,
    agent_experience_events_total: u64,
    agent_promotion_proposals_total: u64,
    active_webhooks: usize,
    dead_letter_backlog: usize,
    pending_webhook_events: usize,
    governance_audit_events_in_window: usize,
    webhook_audit_events_in_window: usize,
}

#[derive(Debug, Serialize)]
struct OpsIncidentsResponse {
    window_seconds: u64,
    total_active: usize,
    incidents: Vec<OpsIncidentResponse>,
}

#[derive(Debug, Clone, Serialize)]
struct OpsIncidentResponse {
    id: String,
    kind: String,
    severity: String,
    title: String,
    summary: String,
    action_label: String,
    action_href: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    resource_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    resource_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    opened_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Serialize)]
struct TraceLookupResponse {
    request_id: String,
    matched_episodes: Vec<TraceEpisodeRef>,
    matched_webhook_events: Vec<MemoryWebhookEventRecord>,
    matched_webhook_audit: Vec<MemoryWebhookAuditRecord>,
    matched_governance_audit: Vec<GovernanceAuditRecord>,
    summary: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct TraceLookupQuery {
    #[serde(default = "default_trace_lookup_from")]
    from: chrono::DateTime<chrono::Utc>,
    #[serde(default = "default_trace_lookup_to")]
    to: chrono::DateTime<chrono::Utc>,
    #[serde(default = "default_trace_lookup_limit")]
    limit: u32,
    #[serde(default = "default_true")]
    include_episodes: bool,
    #[serde(default = "default_true")]
    include_webhook_events: bool,
    #[serde(default = "default_true")]
    include_webhook_audit: bool,
    #[serde(default = "default_true")]
    include_governance_audit: bool,
    #[serde(default)]
    user: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TraceLookupQueryWithEvidence {
    #[serde(default = "default_trace_lookup_from")]
    from: chrono::DateTime<chrono::Utc>,
    #[serde(default = "default_trace_lookup_to")]
    to: chrono::DateTime<chrono::Utc>,
    #[serde(default = "default_trace_lookup_limit")]
    limit: u32,
    #[serde(default = "default_true")]
    include_episodes: bool,
    #[serde(default = "default_true")]
    include_webhook_events: bool,
    #[serde(default = "default_true")]
    include_webhook_audit: bool,
    #[serde(default = "default_true")]
    include_governance_audit: bool,
    #[serde(default)]
    user: Option<String>,
    #[serde(default)]
    focus: Option<String>,
    #[serde(default)]
    source_path: Option<String>,
}

#[derive(Debug, Deserialize)]
struct EvidenceExportQuery {
    #[serde(default)]
    focus: Option<String>,
    #[serde(default)]
    source_path: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GovernanceEvidenceExportQuery {
    #[serde(default)]
    focus: Option<String>,
    #[serde(default)]
    source_path: Option<String>,
    #[serde(default = "default_governance_evidence_from")]
    violations_from: chrono::DateTime<chrono::Utc>,
    #[serde(default = "default_governance_evidence_to")]
    violations_to: chrono::DateTime<chrono::Utc>,
    #[serde(default = "default_governance_evidence_limit")]
    limit: u32,
}

fn default_governance_evidence_from() -> chrono::DateTime<chrono::Utc> {
    chrono::Utc::now() - chrono::Duration::hours(24)
}

fn default_governance_evidence_to() -> chrono::DateTime<chrono::Utc> {
    chrono::Utc::now()
}

fn default_governance_evidence_limit() -> u32 {
    50
}

#[derive(Debug, Serialize)]
struct EvidenceBundleEnvelope<T: Serialize> {
    kind: &'static str,
    exported_at: chrono::DateTime<chrono::Utc>,
    source_path: String,
    payload: T,
}

#[derive(Debug, Serialize)]
struct WebhookEvidenceBundlePayload {
    webhook: MemoryWebhookSubscription,
    stats: WebhookStatsResponse,
    dead_letters: ListWebhookEventsResponse,
    audit: WebhookAuditResponse,
    #[serde(skip_serializing_if = "Option::is_none")]
    focus: Option<String>,
}

#[derive(Debug, Serialize)]
struct GovernanceEvidenceWindow {
    from: chrono::DateTime<chrono::Utc>,
    to: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize)]
struct GovernanceEvidenceBundlePayload {
    user: String,
    policy: UserPolicyRecord,
    violations: Vec<GovernanceAuditRecord>,
    audit: Vec<GovernanceAuditRecord>,
    violations_window: GovernanceEvidenceWindow,
    #[serde(skip_serializing_if = "Option::is_none")]
    focus: Option<String>,
}

#[derive(Debug, Serialize)]
struct TraceEvidenceBundlePayload {
    request_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    focus: Option<String>,
    trace: TraceLookupResponse,
}

fn default_trace_lookup_from() -> chrono::DateTime<chrono::Utc> {
    chrono::Utc::now() - chrono::Duration::days(30)
}

fn default_trace_lookup_to() -> chrono::DateTime<chrono::Utc> {
    chrono::Utc::now()
}

fn default_trace_lookup_limit() -> u32 {
    100
}

fn normalize_evidence_focus(focus: Option<String>) -> Option<String> {
    focus
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn default_evidence_source_path(source_path: Option<String>, fallback: String) -> String {
    source_path
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or(fallback)
}

async fn build_webhook_stats_response(
    state: &AppState,
    id: Uuid,
    query: WebhookStatsQuery,
) -> Result<WebhookStatsResponse, AppError> {
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

    Ok(WebhookStatsResponse {
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
    })
}

#[derive(Debug, Serialize)]
struct TraceEpisodeRef {
    user_id: Uuid,
    session_id: Uuid,
    episode_id: Uuid,
    created_at: chrono::DateTime<chrono::Utc>,
    preview: String,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    request_id: Option<String>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    request_id: Option<String>,
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
    ctx: Option<Extension<RequestContext>>,
    Json(req): Json<RememberMemoryRequest>,
) -> Result<(StatusCode, Json<RememberMemoryResponse>), AppError> {
    let request_id = request_id_from_extension(ctx);
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
        metadata: metadata_with_request_id(serde_json::json!({}), request_id.as_deref()),
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
        request_id,
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

// ── POST /api/v1/memory/extract ────────────────────────────────────
//
// Synchronously extract entities and relationships from a text without
// persisting anything. Returns what the LLM would extract if the text
// were written via POST /api/v1/memory.
//
// Useful for: previewing extraction before commit, building extraction
// test harnesses, and debugging configuration.
//
// If no LLM is configured the endpoint returns an empty extraction with
// a `no_llm` note rather than an error, so callers can detect the state.

#[derive(Debug, Deserialize)]
struct ExtractMemoryRequest {
    text: String,
    #[serde(default)]
    user: Option<String>, // if provided, existing entities for this user are used as dedup hints
}

#[derive(Serialize)]
struct ExtractMemoryResponse {
    ok: bool,
    entities: Vec<ExtractedEntity>,
    relationships: Vec<ExtractedRelationship>,
    entity_count: usize,
    relationship_count: usize,
    /// Present when LLM is not configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<String>,
    /// Provider + model used for this extraction.
    #[serde(skip_serializing_if = "Option::is_none")]
    provider: Option<String>,
}

async fn extract_memory(
    State(state): State<AppState>,
    Json(req): Json<ExtractMemoryRequest>,
) -> Result<Json<ExtractMemoryResponse>, AppError> {
    if req.text.trim().is_empty() {
        return Err(AppError(MnemoError::Validation("text is required".into())));
    }

    let Some(ref llm) = state.llm else {
        return Ok(Json(ExtractMemoryResponse {
            ok: true,
            entities: Vec::new(),
            relationships: Vec::new(),
            entity_count: 0,
            relationship_count: 0,
            note: Some(
                "no_llm: LLM is not configured; set MNEMO_LLM_API_KEY to enable extraction".into(),
            ),
            provider: None,
        }));
    };

    // Build dedup hints from the user's existing entities if a user is provided
    let hints: Vec<ExtractedEntity> = if let Some(ref user_id_or_name) = req.user {
        match find_user_by_identifier(&state, user_id_or_name.trim()).await {
            Ok(user) => {
                let existing = state
                    .state_store
                    .list_entities(user.id, 200, None)
                    .await
                    .unwrap_or_default();
                existing
                    .into_iter()
                    .map(|e| ExtractedEntity {
                        name: e.name,
                        entity_type: e.entity_type,
                        summary: e.summary,
                    })
                    .collect()
            }
            Err(_) => Vec::new(),
        }
    } else {
        Vec::new()
    };

    let provider_label = format!("{}/{}", llm.provider_name(), llm.model_name());

    let extract_started = std::time::Instant::now();
    let extraction = llm
        .extract(req.text.trim(), &hints)
        .await
        .map_err(AppError)?;
    let extract_latency = extract_started.elapsed().as_millis() as u64;

    // Record LLM span for extraction call
    let extract_user_id = if let Some(ref uid) = req.user {
        find_user_by_identifier(&state, uid.trim())
            .await
            .ok()
            .map(|u| u.id)
    } else {
        None
    };
    let extract_span = crate::state::LlmSpan {
        id: Uuid::now_v7(),
        request_id: None,
        user_id: extract_user_id,
        provider: llm.provider_name().to_string(),
        model: llm.model_name().to_string(),
        operation: "extract".to_string(),
        prompt_tokens: 0,
        completion_tokens: 0,
        total_tokens: 0,
        latency_ms: extract_latency,
        success: true,
        error: None,
        started_at: chrono::Utc::now() - chrono::Duration::milliseconds(extract_latency as i64),
        finished_at: chrono::Utc::now(),
    };
    record_llm_span(&state, extract_span).await;

    let entity_count = extraction.entities.len();
    let relationship_count = extraction.relationships.len();

    Ok(Json(ExtractMemoryResponse {
        ok: true,
        entities: extraction.entities,
        relationships: extraction.relationships,
        entity_count,
        relationship_count,
        note: None,
        provider: Some(provider_label),
    }))
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
    ctx: Option<Extension<RequestContext>>,
    Path(user): Path<String>,
    Json(req): Json<MemoryContextRequest>,
) -> Result<Json<MemoryContextResponse>, AppError> {
    let request_id = request_id_from_extension(ctx);
    if req.query.trim().is_empty() {
        return Err(AppError(MnemoError::Validation("query is required".into())));
    }

    let user_identifier = user.trim().to_string();
    let user = find_user_by_identifier(&state, user_identifier.as_str()).await?;
    let policy = get_or_create_user_policy(&state, user.id, user_identifier).await;

    let requested_session_name = req.session.and_then(|s| {
        let trimmed = s.trim().to_string();
        (!trimmed.is_empty()).then_some(trimmed)
    });
    let mode = req.mode.unwrap_or(MemoryContextMode::Hybrid);
    let contract = req
        .contract
        .unwrap_or_else(|| parse_memory_contract_default(&policy.default_memory_contract));
    let retrieval_policy = req
        .retrieval_policy
        .unwrap_or_else(|| parse_retrieval_policy_default(&policy.default_retrieval_policy));

    if req.contract.is_some() || req.retrieval_policy.is_some() {
        append_governance_audit(
            &state,
            user.id,
            "policy_override_memory_context",
            request_id,
            serde_json::json!({
                "requested_contract": req.contract,
                "requested_retrieval_policy": req.retrieval_policy,
                "effective_contract": contract,
                "effective_retrieval_policy": retrieval_policy,
                "default_contract": policy.default_memory_contract,
                "default_retrieval_policy": policy.default_retrieval_policy
            }),
        )
        .await;
    }
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

    let mut context = state
        .retrieval
        .get_context(user.id, &context_req, reranker_for_state(&state))
        .await?;
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
    context
        .episodes
        .retain(|episode| is_episode_summary_within_retention(&policy, episode));

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
    ctx: Option<Extension<RequestContext>>,
    Path(user_identifier): Path<String>,
    Json(req): Json<MemoryChangesSinceRequest>,
) -> Result<Json<MemoryChangesSinceResponse>, AppError> {
    let request_id = request_id_from_extension(ctx);
    if req.to <= req.from {
        return Err(AppError(MnemoError::Validation(
            "'to' must be after 'from'".to_string(),
        )));
    }

    let user_identifier_trimmed = user_identifier.trim().to_string();
    let user = find_user_by_identifier(&state, user_identifier_trimmed.as_str()).await?;
    let policy = get_or_create_user_policy(&state, user.id, user_identifier_trimmed).await;
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
                if !is_episode_within_retention(&policy, &episode) {
                    continue;
                }
                added_episodes.push(EpisodeChange {
                    episode_id: episode.id,
                    session_id: episode.session_id,
                    session_name: session.name.clone(),
                    role: episode.role,
                    created_at: episode.created_at,
                    preview: preview_text(&episode.content, 140),
                    request_id: extract_request_id_from_metadata(&episode.metadata),
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
            request_id.clone(),
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
            request_id,
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
    ctx: Option<Extension<RequestContext>>,
    Path(user_identifier): Path<String>,
    Json(req): Json<TimeTravelTraceRequest>,
) -> Result<Json<TimeTravelTraceResponse>, AppError> {
    let request_id = request_id_from_extension(ctx);
    if req.query.trim().is_empty() {
        return Err(AppError(MnemoError::Validation("query is required".into())));
    }
    if req.to <= req.from {
        return Err(AppError(MnemoError::Validation(
            "'to' must be after 'from'".to_string(),
        )));
    }

    let user_identifier = user_identifier.trim().to_string();
    let user = find_user_by_identifier(&state, user_identifier.as_str()).await?;
    let policy = get_or_create_user_policy(&state, user.id, user_identifier).await;
    let contract = req
        .contract
        .unwrap_or_else(|| parse_memory_contract_default(&policy.default_memory_contract));
    let retrieval_policy = req
        .retrieval_policy
        .unwrap_or_else(|| parse_retrieval_policy_default(&policy.default_retrieval_policy));
    if req.contract.is_some() || req.retrieval_policy.is_some() {
        append_governance_audit(
            &state,
            user.id,
            "policy_override_time_travel",
            request_id,
            serde_json::json!({
                "requested_contract": req.contract,
                "requested_retrieval_policy": req.retrieval_policy,
                "effective_contract": contract,
                "effective_retrieval_policy": retrieval_policy,
                "default_contract": policy.default_memory_contract,
                "default_retrieval_policy": policy.default_retrieval_policy
            }),
        )
        .await;
    }
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
        .get_context(
            user.id,
            &make_context_req(req.from),
            reranker_for_state(&state),
        )
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
    context_from
        .episodes
        .retain(|episode| is_episode_summary_within_retention(&policy, episode));

    let mut context_to = state
        .retrieval
        .get_context(
            user.id,
            &make_context_req(req.to),
            reranker_for_state(&state),
        )
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
    context_to
        .episodes
        .retain(|episode| is_episode_summary_within_retention(&policy, episode));

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
        .filter(|episode| is_episode_summary_within_retention(&policy, episode))
        .collect();
    let lost_episodes: Vec<EpisodeSummary> = from_episodes
        .iter()
        .filter(|(id, _)| !to_episodes.contains_key(id))
        .map(|(_, episode)| episode.clone())
        .filter(|episode| is_episode_summary_within_retention(&policy, episode))
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
    let mut episode_request_id_by_id: HashMap<Uuid, Option<String>> = HashMap::new();
    let mut episode_retained_by_id: HashMap<Uuid, bool> = HashMap::new();
    let mut timeline: Vec<TimeTravelTimelineEvent> = Vec::new();
    for session in &scoped_sessions {
        session_name_by_id.insert(session.id, session.name.clone());
        let episodes = list_all_episodes_for_session(&state, session.id).await?;
        for episode in episodes {
            episode_session_by_id.insert(episode.id, episode.session_id);
            episode_request_id_by_id.insert(
                episode.id,
                extract_request_id_from_metadata(&episode.metadata),
            );
            episode_retained_by_id
                .insert(episode.id, is_episode_within_retention(&policy, &episode));
            if episode.created_at > req.from && episode.created_at <= req.to {
                if !is_episode_within_retention(&policy, &episode) {
                    continue;
                }
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
                    request_id: extract_request_id_from_metadata(&episode.metadata),
                });
            }
        }

        if let Some(at) = session.head_updated_at {
            if at > req.from && at <= req.to {
                if let Some(head_episode_id) = session.head_episode_id {
                    if !episode_retained_by_id
                        .get(&head_episode_id)
                        .copied()
                        .unwrap_or(true)
                    {
                        continue;
                    }
                }
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
                    request_id: session.head_episode_id.and_then(|episode_id| {
                        episode_request_id_by_id.get(&episode_id).cloned().flatten()
                    }),
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
            if !episode_retained_by_id
                .get(&edge.source_episode_id)
                .copied()
                .unwrap_or(true)
            {
                continue;
            }
            timeline.push(TimeTravelTimelineEvent {
                at: edge.valid_at,
                event_type: "fact_added".to_string(),
                description: format!("{} {} {}", source_entity, edge.label, target_entity),
                session_id: episode_session_by_id.get(&edge.source_episode_id).copied(),
                episode_id: Some(edge.source_episode_id),
                edge_id: Some(edge.id),
                request_id: episode_request_id_by_id
                    .get(&edge.source_episode_id)
                    .cloned()
                    .flatten(),
            });
        }
        if let Some(invalid_at) = edge.invalid_at {
            if invalid_at > req.from && invalid_at <= req.to {
                if !episode_retained_by_id
                    .get(&edge.source_episode_id)
                    .copied()
                    .unwrap_or(true)
                {
                    continue;
                }
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
                    request_id: episode_request_id_by_id
                        .get(&edge.source_episode_id)
                        .cloned()
                        .flatten(),
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

async fn time_travel_summary(
    State(state): State<AppState>,
    ctx: Option<Extension<RequestContext>>,
    Path(user_identifier): Path<String>,
    Json(req): Json<TimeTravelSummaryRequest>,
) -> Result<Json<TimeTravelSummaryResponse>, AppError> {
    let request_id = request_id_from_extension(ctx);
    if req.query.trim().is_empty() {
        return Err(AppError(MnemoError::Validation("query is required".into())));
    }
    if req.to <= req.from {
        return Err(AppError(MnemoError::Validation(
            "'to' must be after 'from'".to_string(),
        )));
    }

    let user_identifier = user_identifier.trim().to_string();
    let user = find_user_by_identifier(&state, user_identifier.as_str()).await?;
    let policy = get_or_create_user_policy(&state, user.id, user_identifier).await;
    let contract = req
        .contract
        .unwrap_or_else(|| parse_memory_contract_default(&policy.default_memory_contract));
    let retrieval_policy = req
        .retrieval_policy
        .unwrap_or_else(|| parse_retrieval_policy_default(&policy.default_retrieval_policy));

    if req.contract.is_some() || req.retrieval_policy.is_some() {
        append_governance_audit(
            &state,
            user.id,
            "policy_override_time_travel_summary",
            request_id,
            serde_json::json!({
                "requested_contract": req.contract,
                "requested_retrieval_policy": req.retrieval_policy,
                "effective_contract": contract,
                "effective_retrieval_policy": retrieval_policy,
                "default_contract": policy.default_memory_contract,
                "default_retrieval_policy": policy.default_retrieval_policy
            }),
        )
        .await;
    }

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
    let min_relevance = match retrieval_policy {
        AdaptiveRetrievalPolicy::Balanced => 0.3,
        AdaptiveRetrievalPolicy::Precision => 0.55,
        AdaptiveRetrievalPolicy::Recall => 0.15,
        AdaptiveRetrievalPolicy::Stability => 0.4,
    };
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

    let make_context_req = |as_of: chrono::DateTime<chrono::Utc>| ContextRequest {
        session_id,
        messages: vec![ContextMessage {
            role: "user".to_string(),
            content: req.query.clone(),
        }],
        max_tokens: default_max_tokens,
        search_types: vec![mnemo_core::models::context::SearchType::Hybrid],
        temporal_filter: Some(as_of),
        as_of: Some(as_of),
        time_intent: temporal_intent,
        temporal_weight,
        min_relevance,
    };

    let mut context_from = state
        .retrieval
        .get_context(
            user.id,
            &make_context_req(req.from),
            reranker_for_state(&state),
        )
        .await?;
    maybe_attach_recent_episode_fallback(
        &state,
        user.id,
        session_id,
        default_max_tokens,
        temporal_intent,
        Some(req.from),
        &mut context_from,
    )
    .await?;
    apply_memory_contract(&mut context_from, contract);
    context_from
        .episodes
        .retain(|episode| is_episode_summary_within_retention(&policy, episode));

    let mut context_to = state
        .retrieval
        .get_context(
            user.id,
            &make_context_req(req.to),
            reranker_for_state(&state),
        )
        .await?;
    maybe_attach_recent_episode_fallback(
        &state,
        user.id,
        session_id,
        default_max_tokens,
        temporal_intent,
        Some(req.to),
        &mut context_to,
    )
    .await?;
    apply_memory_contract(&mut context_to, contract);
    context_to
        .episodes
        .retain(|episode| is_episode_summary_within_retention(&policy, episode));

    let from_fact_ids: std::collections::HashSet<Uuid> =
        context_from.facts.iter().map(|f| f.id).collect();
    let to_fact_ids: std::collections::HashSet<Uuid> =
        context_to.facts.iter().map(|f| f.id).collect();
    let gained_fact_count = to_fact_ids.difference(&from_fact_ids).count();
    let lost_fact_count = from_fact_ids.difference(&to_fact_ids).count();

    let from_episode_ids: std::collections::HashSet<Uuid> =
        context_from.episodes.iter().map(|e| e.id).collect();
    let to_episode_ids: std::collections::HashSet<Uuid> =
        context_to.episodes.iter().map(|e| e.id).collect();
    let gained_episode_count = to_episode_ids.difference(&from_episode_ids).count();
    let lost_episode_count = from_episode_ids.difference(&to_episode_ids).count();

    let summary = format!(
        "{} gained facts, {} lost facts; {} gained episodes, {} lost episodes",
        gained_fact_count, lost_fact_count, gained_episode_count, lost_episode_count
    );

    Ok(Json(TimeTravelSummaryResponse {
        user_id: user.id,
        from: req.from,
        to: req.to,
        contract_applied: contract,
        retrieval_policy_applied: retrieval_policy,
        fact_count_from: context_from.facts.len(),
        fact_count_to: context_to.facts.len(),
        episode_count_from: context_from.episodes.len(),
        episode_count_to: context_to.episodes.len(),
        gained_fact_count,
        lost_fact_count,
        gained_episode_count,
        lost_episode_count,
        summary,
    }))
}

async fn conflict_radar(
    State(state): State<AppState>,
    ctx: Option<Extension<RequestContext>>,
    Path(user_identifier): Path<String>,
    Json(req): Json<ConflictRadarRequest>,
) -> Result<Json<ConflictRadarResponse>, AppError> {
    let request_id = request_id_from_extension(ctx);
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
            request_id,
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

    let mut context = state
        .retrieval
        .get_context(user.id, &context_req, reranker_for_state(&state))
        .await?;
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
    ctx: Option<Extension<RequestContext>>,
    Json(req): Json<RegisterMemoryWebhookRequest>,
) -> Result<(StatusCode, Json<RegisterMemoryWebhookResponse>), AppError> {
    let request_id = request_id_from_extension(ctx);
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
    // SOC 2 TLS enforcement: reject non-https targets when require_tls is enabled
    if state.require_tls && !req.target_url.trim().starts_with("https://") {
        return Err(AppError(MnemoError::Validation(
            "require_tls is enabled; target_url must use https://".into(),
        )));
    }

    let user = find_user_by_identifier(&state, req.user.trim()).await?;
    let policy = get_or_create_user_policy(&state, user.id, req.user.trim().to_string()).await;
    if !is_target_url_allowed(&policy, req.target_url.trim()) {
        state
            .metrics
            .policy_violation_total
            .fetch_add(1, Ordering::Relaxed);
        append_governance_audit(
            &state,
            user.id,
            "policy_violation_webhook_domain",
            request_id.clone(),
            serde_json::json!({
                "target_url": req.target_url.trim(),
                "allowlist": policy.webhook_domain_allowlist,
            }),
        )
        .await;
        return Err(AppError(MnemoError::Validation(
            "target_url host is not allowed by policy webhook_domain_allowlist".into(),
        )));
    }
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
        request_id,
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

async fn list_memory_webhooks(
    State(state): State<AppState>,
) -> Json<ListResponse<MemoryWebhookSubscription>> {
    let hooks = state.memory_webhooks.read().await;
    let mut webhooks: Vec<MemoryWebhookSubscription> = hooks.values().cloned().collect();
    webhooks.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Json(ListResponse::new(webhooks))
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

async fn update_memory_webhook(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    ctx: Option<Extension<RequestContext>>,
    Json(req): Json<UpdateMemoryWebhookRequest>,
) -> Result<Json<UpdateMemoryWebhookResponse>, AppError> {
    let request_id = request_id_from_extension(ctx);

    // Look up the existing webhook
    let mut webhook = {
        let hooks = state.memory_webhooks.read().await;
        hooks.get(&id).cloned().ok_or_else(|| {
            AppError(MnemoError::NotFound {
                resource_type: "MemoryWebhook".into(),
                id: id.to_string(),
            })
        })?
    };

    // If target_url is being changed, apply the same validation as registration:
    // 1. Must be a valid HTTP(S) URL
    // 2. SOC 2 TLS enforcement (require_tls)
    // 3. Domain allowlist policy check
    if let Some(ref new_url) = req.target_url {
        let trimmed = new_url.trim();
        if trimmed.is_empty() {
            return Err(AppError(MnemoError::Validation(
                "target_url cannot be empty".into(),
            )));
        }
        if !is_http_url(trimmed) {
            return Err(AppError(MnemoError::Validation(
                "target_url must start with http:// or https://".into(),
            )));
        }
        // SOC 2 TLS enforcement: reject non-https targets when require_tls is enabled
        if state.require_tls && !trimmed.starts_with("https://") {
            return Err(AppError(MnemoError::Validation(
                "require_tls is enabled; target_url must use https://".into(),
            )));
        }
        // Domain allowlist policy check
        let policy = get_or_create_user_policy(
            &state,
            webhook.user_id,
            webhook.user_identifier.clone(),
        )
        .await;
        if !is_target_url_allowed(&policy, trimmed) {
            state
                .metrics
                .policy_violation_total
                .fetch_add(1, Ordering::Relaxed);
            append_governance_audit(
                &state,
                webhook.user_id,
                "policy_violation_webhook_domain",
                request_id.clone(),
                serde_json::json!({
                    "target_url": trimmed,
                    "allowlist": policy.webhook_domain_allowlist,
                    "action": "webhook_update",
                }),
            )
            .await;
            return Err(AppError(MnemoError::Validation(
                "target_url host is not allowed by policy webhook_domain_allowlist".into(),
            )));
        }
        webhook.target_url = trimmed.to_string();
    }

    // Apply optional field updates
    if let Some(ref secret) = req.signing_secret {
        let trimmed = secret.trim();
        webhook.signing_secret = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        };
    }
    if let Some(ref events) = req.events {
        if !events.is_empty() {
            webhook.events = events.clone();
        }
    }
    if let Some(enabled) = req.enabled {
        webhook.enabled = enabled;
    }

    webhook.updated_at = chrono::Utc::now();

    // Persist update
    {
        let mut hooks = state.memory_webhooks.write().await;
        hooks.insert(id, webhook.clone());
    }
    persist_webhook_state(&state).await;

    // Audit trail
    append_webhook_audit(
        &state,
        id,
        "webhook_updated",
        request_id,
        serde_json::json!({
            "target_url": webhook.target_url.clone(),
            "events": webhook.events.clone(),
            "enabled": webhook.enabled,
        }),
    )
    .await;

    Ok(Json(UpdateMemoryWebhookResponse {
        ok: true,
        webhook,
    }))
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
    ctx: Option<Extension<RequestContext>>,
    Path(id): Path<Uuid>,
    Query(query): Query<ReplayWebhookEventsQuery>,
) -> Result<Json<ReplayWebhookEventsResponse>, AppError> {
    let request_id = request_id_from_extension(ctx);
    state
        .metrics
        .webhook_replay_requests_total
        .fetch_add(1, Ordering::Relaxed);

    {
        let hooks = state.memory_webhooks.read().await;
        if !hooks.contains_key(&id) {
            return Err(AppError(MnemoError::NotFound {
                resource_type: "MemoryWebhook".into(),
                id: id.to_string(),
            }));
        }
    }

    append_webhook_audit(
        &state,
        id,
        "replay_requested",
        request_id,
        serde_json::json!({
            "after_event_id": query.after_event_id,
            "limit": query.limit,
            "include_delivered": query.include_delivered,
            "include_dead_letter": query.include_dead_letter
        }),
    )
    .await;

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
    ctx: Option<Extension<RequestContext>>,
    Path((webhook_id, event_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<RetryWebhookEventRequest>,
) -> Result<Json<RetryWebhookEventResponse>, AppError> {
    let request_id = request_id_from_extension(ctx);
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
    let mut event_snapshot: Option<MemoryWebhookEventRecord> = None;
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
                event_snapshot = Some(event.clone());
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
            request_id,
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
            event: event_snapshot,
        }));
    }

    persist_webhook_state(&state).await;
    append_webhook_audit(
        &state,
        webhook_id,
        "retry_queued",
        request_id,
        serde_json::json!({
            "event_id": event_id,
            "force": req.force.unwrap_or(false)
        }),
    )
    .await;

    let state_clone = state.clone();
    state
        .metrics
        .webhook_retry_queued_total
        .fetch_add(1, Ordering::Relaxed);
    tokio::spawn(async move {
        deliver_memory_webhook_event(state_clone, webhook_id, event_id).await;
    });

    Ok(Json(RetryWebhookEventResponse {
        webhook_id,
        event_id,
        queued: true,
        reason: "delivery retry queued".to_string(),
        event: event_snapshot,
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
    Ok(Json(build_webhook_stats_response(&state, id, query).await?))
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

async fn export_webhook_evidence_bundle(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(query): Query<EvidenceExportQuery>,
) -> Result<Json<EvidenceBundleEnvelope<WebhookEvidenceBundlePayload>>, AppError> {
    let focus = normalize_evidence_focus(query.focus);
    let source_path = default_evidence_source_path(
        query.source_path,
        format!(
            "/_/webhooks/{id}{}",
            focus
                .as_ref()
                .map(|v| format!("?focus={v}"))
                .unwrap_or_default()
        ),
    );
    let webhook = {
        let hooks = state.memory_webhooks.read().await;
        hooks.get(&id).cloned().ok_or_else(|| {
            AppError(MnemoError::NotFound {
                resource_type: "MemoryWebhook".into(),
                id: id.to_string(),
            })
        })?
    };
    let stats = build_webhook_stats_response(
        &state,
        id,
        WebhookStatsQuery {
            window_seconds: None,
        },
    )
    .await?;
    let dead_letters = {
        let mut events = {
            let event_map = state.memory_webhook_events.read().await;
            event_map.get(&id).cloned().unwrap_or_default()
        };
        events.retain(|event| event.dead_letter);
        events.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        events.truncate(50);
        ListWebhookEventsResponse {
            webhook_id: id,
            count: events.len(),
            events,
        }
    };
    let audit = {
        let mut rows = {
            let map = state.memory_webhook_audit.read().await;
            map.get(&id).cloned().unwrap_or_default()
        };
        rows.sort_by(|a, b| b.at.cmp(&a.at));
        rows.truncate(50);
        WebhookAuditResponse {
            webhook_id: id,
            count: rows.len(),
            audit: rows,
        }
    };

    Ok(Json(EvidenceBundleEnvelope {
        kind: "webhook_evidence_bundle",
        exported_at: chrono::Utc::now(),
        source_path,
        payload: WebhookEvidenceBundlePayload {
            webhook,
            stats,
            dead_letters,
            audit,
            focus,
        },
    }))
}

async fn export_governance_evidence_bundle(
    State(state): State<AppState>,
    Path(user_identifier): Path<String>,
    Query(query): Query<GovernanceEvidenceExportQuery>,
) -> Result<Json<EvidenceBundleEnvelope<GovernanceEvidenceBundlePayload>>, AppError> {
    if query.violations_to <= query.violations_from {
        return Err(AppError(MnemoError::Validation(
            "'violations_to' must be after 'violations_from'".to_string(),
        )));
    }

    let focus = normalize_evidence_focus(query.focus);
    let user_identifier = user_identifier.trim().to_string();
    let user = find_user_by_identifier(&state, &user_identifier).await?;
    let policy = get_or_create_user_policy(&state, user.id, user_identifier.clone()).await;
    let limit = query.limit.clamp(1, 200) as usize;
    let source_path =
        default_evidence_source_path(query.source_path, format!("/_/governance/{}", user.id));
    let all_rows = {
        let audit = state.governance_audit.read().await;
        audit.get(&user.id).cloned().unwrap_or_default()
    };

    let mut audit = all_rows.clone();
    audit.sort_by(|a, b| b.at.cmp(&a.at));
    audit.truncate(limit);

    let mut violations = all_rows
        .into_iter()
        .filter(|row| {
            row.action.starts_with("policy_violation_")
                && row.at >= query.violations_from
                && row.at <= query.violations_to
        })
        .collect::<Vec<_>>();
    violations.sort_by(|a, b| b.at.cmp(&a.at));
    violations.truncate(limit);

    Ok(Json(EvidenceBundleEnvelope {
        kind: "governance_evidence_bundle",
        exported_at: chrono::Utc::now(),
        source_path,
        payload: GovernanceEvidenceBundlePayload {
            user: user_identifier,
            policy,
            violations,
            audit,
            violations_window: GovernanceEvidenceWindow {
                from: query.violations_from,
                to: query.violations_to,
            },
            focus,
        },
    }))
}

async fn export_trace_evidence_bundle(
    State(state): State<AppState>,
    Path(request_id): Path<String>,
    Query(query): Query<TraceLookupQueryWithEvidence>,
) -> Result<Json<EvidenceBundleEnvelope<TraceEvidenceBundlePayload>>, AppError> {
    let focus = normalize_evidence_focus(query.focus.clone());
    let trace_query = TraceLookupQuery {
        from: query.from,
        to: query.to,
        limit: query.limit,
        include_episodes: query.include_episodes,
        include_webhook_events: query.include_webhook_events,
        include_webhook_audit: query.include_webhook_audit,
        include_governance_audit: query.include_governance_audit,
        user: query.user.clone(),
    };
    let trace = lookup_trace_by_request_id(&state, request_id.clone(), trace_query).await?;
    let source_path = default_evidence_source_path(
        query.source_path,
        format!(
            "/_/traces/{}{}",
            request_id,
            focus
                .as_ref()
                .map(|value| format!("?focus={value}"))
                .unwrap_or_default()
        ),
    );

    Ok(Json(EvidenceBundleEnvelope {
        kind: "trace_evidence_bundle",
        exported_at: chrono::Utc::now(),
        source_path,
        payload: TraceEvidenceBundlePayload {
            request_id: trace.request_id.clone(),
            focus,
            trace,
        },
    }))
}

async fn get_agent_identity(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> Result<Json<AgentIdentityProfile>, AppError> {
    let agent_id = normalize_agent_id(&agent_id)?;
    let identity = state.state_store.get_agent_identity(&agent_id).await?;
    state
        .metrics
        .agent_identity_reads_total
        .fetch_add(1, Ordering::Relaxed);
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
    state
        .metrics
        .agent_identity_updates_total
        .fetch_add(1, Ordering::Relaxed);
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

    let category = req.category.clone();
    let mut event = state
        .state_store
        .add_experience_event(&agent_id, req)
        .await?;

    // EWC++: compute Fisher importance relative to existing events in the same category
    let all_events = state
        .state_store
        .list_experience_events(&agent_id, 500)
        .await?;
    let category_events: Vec<ExperienceEvent> = all_events
        .into_iter()
        .filter(|e| e.category == category && e.id != event.id)
        .collect();
    let fisher = compute_fisher_importance(&event, &category_events);
    event.fisher_importance = fisher;
    state.state_store.update_experience_event(&event).await?;

    state
        .metrics
        .agent_experience_events_total
        .fetch_add(1, Ordering::Relaxed);
    Ok((StatusCode::CREATED, Json(event)))
}

/// Returns experience events ranked by Fisher importance (EWC++).
/// High-importance events are structurally load-bearing for the agent's identity.
async fn list_experience_importance(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Query(params): Query<ListLimitQuery>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    let agent_id = normalize_agent_id(&agent_id)?;
    let limit = params.limit.unwrap_or(50).clamp(1, 500);
    let mut events = state
        .state_store
        .list_experience_events(&agent_id, limit)
        .await?;

    // Sort by Fisher importance descending
    events.sort_by(|a, b| {
        b.fisher_importance
            .partial_cmp(&a.fisher_importance)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let ranked: Vec<serde_json::Value> = events
        .iter()
        .map(|e| {
            serde_json::json!({
                "id": e.id,
                "category": e.category,
                "signal": e.signal,
                "fisher_importance": e.fisher_importance,
                "effective_weight": e.effective_weight(),
                "raw_weight": e.weight,
                "confidence": e.confidence,
                "created_at": e.created_at,
            })
        })
        .collect();

    Ok(Json(ranked))
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

    // Validate that all source_event_ids reference existing experience events
    for event_id in &req.source_event_ids {
        if state
            .state_store
            .get_experience_event(*event_id)
            .await?
            .is_none()
        {
            return Err(AppError(MnemoError::Validation(format!(
                "source_event_id {} does not reference an existing experience event",
                event_id
            ))));
        }
    }

    let proposal = state
        .state_store
        .create_promotion_proposal(&agent_id, req)
        .await?;
    state
        .metrics
        .agent_promotion_proposals_total
        .fetch_add(1, Ordering::Relaxed);
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
    state
        .metrics
        .agent_identity_updates_total
        .fetch_add(1, Ordering::Relaxed);
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

    let mut context = state
        .retrieval
        .get_context(user.id, &context_req, reranker_for_state(&state))
        .await?;
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

    let response = AgentContextResponse {
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
    };
    state
        .metrics
        .agent_identity_reads_total
        .fetch_add(1, Ordering::Relaxed);
    Ok(Json(response))
}

// ─── Memory Retrieval Feedback ─────────────────────────────────────

#[derive(Deserialize)]
struct RetrievalFeedbackRequest {
    /// The entity IDs that were actually useful / cited by the agent.
    positive_entity_ids: Vec<Uuid>,
    /// All entity IDs that were returned by retrieval (for negative signal).
    #[serde(default)]
    all_entity_ids: Vec<Uuid>,
}

async fn memory_retrieval_feedback(
    State(state): State<AppState>,
    Json(req): Json<RetrievalFeedbackRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    if req.positive_entity_ids.is_empty() {
        return Err(AppError(MnemoError::Validation(
            "positive_entity_ids is required".into(),
        )));
    }

    // Build candidate list from all_entity_ids (or just positives if all not provided)
    let candidates: Vec<(Uuid, f64)> = if req.all_entity_ids.is_empty() {
        req.positive_entity_ids
            .iter()
            .map(|id| (*id, 1.0))
            .collect()
    } else {
        req.all_entity_ids
            .iter()
            .enumerate()
            .map(|(i, id)| (*id, 1.0 - (i as f64 * 0.05)))
            .collect()
    };

    let features_map = std::collections::HashMap::new();
    state
        .retrieval
        .apply_gnn_feedback(&candidates, &[], &features_map, &req.positive_entity_ids, 16)
        .await;

    Ok(Json(serde_json::json!({
        "accepted": true,
        "positive_count": req.positive_entity_ids.len(),
    })))
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

/// EWC++-enhanced effective weight: high-importance events resist decay.
fn effective_experience_weight(event: &ExperienceEvent) -> f32 {
    event.effective_weight()
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
            None,
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

// ─── Raw Vector API ────────────────────────────────────────────────
//
// These endpoints expose Mnemo as a pluggable vector database for external
// systems like AnythingLLM. Namespaces are isolated from Mnemo's internal
// entity/edge/episode collections (prefixed with `raw_`).

#[derive(Deserialize)]
struct VectorsUpsertRequest {
    vectors: Vec<VectorPoint>,
}

#[derive(Deserialize)]
struct VectorPoint {
    id: String,
    vector: Vec<f32>,
    #[serde(default)]
    metadata: serde_json::Value,
}

#[derive(Deserialize)]
struct VectorsQueryRequest {
    vector: Vec<f32>,
    #[serde(default = "default_top_k")]
    top_k: u32,
    #[serde(default = "default_min_score")]
    min_score: f32,
}

fn default_top_k() -> u32 {
    10
}
fn default_min_score() -> f32 {
    0.0
}

#[derive(Deserialize)]
struct VectorsDeleteIdsRequest {
    ids: Vec<String>,
}

/// `POST /api/v1/vectors/:namespace`
///
/// Upsert vectors into a namespace. Creates the namespace if it doesn't exist.
async fn vectors_upsert(
    State(state): State<AppState>,
    Path(namespace): Path<String>,
    Json(body): Json<VectorsUpsertRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    if body.vectors.is_empty() {
        return Err(MnemoError::Validation("vectors array must not be empty".into()).into());
    }

    let count = body.vectors.len();
    let vectors: Vec<(String, Vec<f32>, serde_json::Value)> = body
        .vectors
        .into_iter()
        .map(|p| (p.id, p.vector, p.metadata))
        .collect();

    state
        .vector_store
        .upsert_vectors(&namespace, vectors)
        .await?;

    Ok(Json(serde_json::json!({
        "ok": true,
        "namespace": namespace,
        "upserted": count,
    })))
}

/// `POST /api/v1/vectors/:namespace/query`
///
/// Search vectors by similarity in a namespace.
async fn vectors_query(
    State(state): State<AppState>,
    Path(namespace): Path<String>,
    Json(body): Json<VectorsQueryRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let exists = state.vector_store.has_namespace(&namespace).await?;

    if !exists {
        return Ok(Json(serde_json::json!({
            "results": [],
            "namespace": namespace,
        })));
    }

    let hits = state
        .vector_store
        .search_vectors(&namespace, body.vector, body.top_k, body.min_score)
        .await?;

    Ok(Json(serde_json::json!({
        "results": hits,
        "namespace": namespace,
    })))
}

/// `POST /api/v1/vectors/:namespace/delete`
///
/// Delete specific vectors by ID from a namespace.
async fn vectors_delete_ids(
    State(state): State<AppState>,
    Path(namespace): Path<String>,
    Json(body): Json<VectorsDeleteIdsRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    if body.ids.is_empty() {
        return Err(MnemoError::Validation("ids array must not be empty".into()).into());
    }

    let exists = state.vector_store.has_namespace(&namespace).await?;

    if !exists {
        return Ok(Json(serde_json::json!({
            "ok": true,
            "namespace": namespace,
            "deleted": 0,
        })));
    }

    let count = body.ids.len();
    state
        .vector_store
        .delete_vectors(&namespace, body.ids)
        .await?;

    Ok(Json(serde_json::json!({
        "ok": true,
        "namespace": namespace,
        "deleted": count,
    })))
}

/// `DELETE /api/v1/vectors/:namespace`
///
/// Delete an entire namespace and all its vectors.
async fn vectors_delete_namespace(
    State(state): State<AppState>,
    Path(namespace): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    state.vector_store.delete_namespace(&namespace).await?;

    Ok(Json(serde_json::json!({
        "ok": true,
        "namespace": namespace,
        "deleted": true,
    })))
}

/// `GET /api/v1/vectors/:namespace/count`
///
/// Count total vectors in a namespace.
async fn vectors_count(
    State(state): State<AppState>,
    Path(namespace): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let exists = state.vector_store.has_namespace(&namespace).await?;

    if !exists {
        return Ok(Json(serde_json::json!({
            "namespace": namespace,
            "count": 0,
        })));
    }

    let count = state.vector_store.count_vectors(&namespace).await?;

    Ok(Json(serde_json::json!({
        "namespace": namespace,
        "count": count,
    })))
}

/// `GET /api/v1/vectors/:namespace/exists`
///
/// Check whether a namespace exists.
async fn vectors_exists(
    State(state): State<AppState>,
    Path(namespace): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let exists = state.vector_store.has_namespace(&namespace).await?;

    Ok(Json(serde_json::json!({
        "namespace": namespace,
        "exists": exists,
    })))
}

// ─── Knowledge Graph API ───────────────────────────────────────────────────

/// Query parameters for entity listing with optional type/name filters.
#[derive(Deserialize)]
struct GraphEntitiesParams {
    #[serde(default = "default_limit")]
    limit: u32,
    after: Option<Uuid>,
    /// Filter by entity type (case-insensitive, e.g. "person", "concept").
    entity_type: Option<String>,
    /// Filter by entity name (case-insensitive substring match).
    name: Option<String>,
}

/// `GET /api/v1/graph/:user/entities`
///
/// List all entities for a user in a graph-API response envelope.
/// Supports optional `entity_type` and `name` query parameters for filtering.
async fn graph_list_entities(
    State(state): State<AppState>,
    Path(user): Path<String>,
    Query(params): Query<GraphEntitiesParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    let user_rec = find_user_by_identifier(&state, &user).await?;
    let clamped_limit = params.limit.clamp(1, 1000);
    // Fetch more than requested if filtering (filter reduces result count).
    // We over-fetch by 4x when filters are active to improve the chance of
    // filling the requested limit, capped at 1000.
    let has_filter = params.entity_type.is_some() || params.name.is_some();
    let fetch_limit = if has_filter {
        (clamped_limit * 4).min(1000)
    } else {
        clamped_limit
    };
    let entities = state
        .state_store
        .list_entities(user_rec.id, fetch_limit, params.after)
        .await?;

    // Apply optional filters
    let type_filter = params
        .entity_type
        .as_deref()
        .map(EntityType::from_str_flexible);
    let name_lower = params.name.as_deref().map(|n| n.to_lowercase());

    let filtered: Vec<&Entity> = entities
        .iter()
        .filter(|e| {
            if let Some(ref t) = type_filter {
                if e.entity_type.as_str() != t.as_str() {
                    return false;
                }
            }
            if let Some(ref n) = name_lower {
                if !e.name.to_lowercase().contains(n.as_str()) {
                    return false;
                }
            }
            true
        })
        .take(clamped_limit as usize)
        .collect();

    let data: Vec<serde_json::Value> = filtered
        .iter()
        .map(|e| {
            serde_json::json!({
                "id": e.id,
                "name": e.name,
                "entity_type": e.entity_type.as_str(),
                "summary": e.summary,
                "mention_count": e.mention_count,
                "community_id": e.community_id,
                "created_at": e.created_at,
                "updated_at": e.updated_at,
            })
        })
        .collect();
    Ok(Json(serde_json::json!({
        "data": data,
        "count": data.len(),
        "user_id": user_rec.id,
    })))
}

/// `GET /api/v1/graph/:user/entities/:entity_id`
///
/// Get a single entity with its adjacency.
async fn graph_get_entity(
    State(state): State<AppState>,
    Path((user, entity_id)): Path<(String, Uuid)>,
) -> Result<Json<serde_json::Value>, AppError> {
    let user_rec = find_user_by_identifier(&state, &user).await?;
    let entity = state.state_store.get_entity(entity_id).await?;
    // Verify the entity belongs to the resolved user (prevent cross-user data leak)
    if entity.user_id != user_rec.id {
        return Err(AppError(MnemoError::NotFound {
            resource_type: "entity".into(),
            id: entity_id.to_string(),
        }));
    }
    let outgoing = state
        .state_store
        .get_outgoing_edges(entity_id)
        .await
        .unwrap_or_default();
    let incoming = state
        .state_store
        .get_incoming_edges(entity_id)
        .await
        .unwrap_or_default();
    Ok(Json(serde_json::json!({
        "id": entity.id,
        "name": entity.name,
        "entity_type": entity.entity_type.as_str(),
        "summary": entity.summary,
        "mention_count": entity.mention_count,
        "community_id": entity.community_id,
        "created_at": entity.created_at,
        "updated_at": entity.updated_at,
        "outgoing_edges": outgoing.iter().map(|e| serde_json::json!({
            "id": e.id,
            "target_entity_id": e.target_entity_id,
            "label": e.label,
            "fact": e.fact,
            "valid": e.is_valid(),
        })).collect::<Vec<_>>(),
        "incoming_edges": incoming.iter().map(|e| serde_json::json!({
            "id": e.id,
            "source_entity_id": e.source_entity_id,
            "label": e.label,
            "fact": e.fact,
            "valid": e.is_valid(),
        })).collect::<Vec<_>>(),
    })))
}

/// `GET /api/v1/graph/:user/edges`
///
/// List edges for a user with optional label, source, and target filters.
#[derive(Deserialize)]
struct GraphEdgesParams {
    #[serde(default = "default_limit")]
    limit: u32,
    label: Option<String>,
    valid_only: Option<bool>,
    /// Filter edges by source entity ID.
    source_entity_id: Option<Uuid>,
    /// Filter edges by target entity ID.
    target_entity_id: Option<Uuid>,
}

async fn graph_list_edges(
    State(state): State<AppState>,
    Path(user): Path<String>,
    Query(params): Query<GraphEdgesParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    let user_rec = find_user_by_identifier(&state, &user).await?;
    let include_invalidated = !params.valid_only.unwrap_or(true);
    let filter = EdgeFilter {
        label: params.label.clone(),
        include_invalidated,
        limit: params.limit.clamp(1, 1000),
        source_entity_id: params.source_entity_id,
        target_entity_id: params.target_entity_id,
        ..Default::default()
    };
    let edges = state.state_store.query_edges(user_rec.id, filter).await?;
    let data: Vec<serde_json::Value> = edges
        .iter()
        .map(|e| {
            serde_json::json!({
                "id": e.id,
                "source_entity_id": e.source_entity_id,
                "target_entity_id": e.target_entity_id,
                "label": e.label,
                "fact": e.fact,
                "confidence": e.confidence,
                "valid_at": e.valid_at,
                "invalid_at": e.invalid_at,
                "valid": e.is_valid(),
                "created_at": e.created_at,
            })
        })
        .collect();
    Ok(Json(serde_json::json!({
        "data": data,
        "count": data.len(),
        "user_id": user_rec.id,
    })))
}

/// `GET /api/v1/graph/:user/neighbors/:entity_id`
///
/// Return 1-hop neighbors of an entity.
#[derive(Deserialize)]
struct NeighborsParams {
    #[serde(default = "default_neighbor_depth")]
    depth: u32,
    #[serde(default = "default_neighbor_max")]
    max_nodes: usize,
    #[serde(default = "default_true")]
    valid_only: bool,
}

fn default_neighbor_depth() -> u32 {
    1
}
fn default_neighbor_max() -> usize {
    50
}

async fn graph_neighbors(
    State(state): State<AppState>,
    Path((user, entity_id)): Path<(String, Uuid)>,
    Query(params): Query<NeighborsParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    let user_rec = find_user_by_identifier(&state, &user).await?;
    // Verify the seed entity belongs to the resolved user (prevent cross-user traversal)
    let seed_entity = state.state_store.get_entity(entity_id).await?;
    if seed_entity.user_id != user_rec.id {
        return Err(AppError(MnemoError::NotFound {
            resource_type: "entity".into(),
            id: entity_id.to_string(),
        }));
    }
    let subgraph = state
        .graph
        .traverse_bfs(
            entity_id,
            params.depth.min(10),
            params.max_nodes.min(500),
            params.valid_only,
        )
        .await?;
    let nodes: Vec<serde_json::Value> = subgraph
        .nodes
        .iter()
        .map(|n| {
            serde_json::json!({
                "id": n.entity.id,
                "name": n.entity.name,
                "entity_type": n.entity.entity_type.as_str(),
                "summary": n.entity.summary,
                "depth": n.depth,
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
                "valid": e.is_valid(),
            })
        })
        .collect();
    Ok(Json(serde_json::json!({
        "seed_entity_id": entity_id,
        "depth": params.depth,
        "nodes": nodes,
        "edges": edges,
        "entities_visited": subgraph.entities_visited,
    })))
}

/// `GET /api/v1/graph/:user/community`
///
/// Run community detection and return entity→community assignments.
#[derive(Deserialize)]
struct CommunityParams {
    #[serde(default = "default_community_iterations")]
    max_iterations: u32,
}

fn default_community_iterations() -> u32 {
    20
}

async fn graph_community(
    State(state): State<AppState>,
    Path(user): Path<String>,
    Query(params): Query<CommunityParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    let user_rec = find_user_by_identifier(&state, &user).await?;
    let clamped_iterations = params.max_iterations.clamp(1, 100);
    let labels = state
        .graph
        .detect_communities(user_rec.id, clamped_iterations)
        .await?;

    // Group entity ids by community id
    let mut communities: std::collections::HashMap<uuid::Uuid, Vec<uuid::Uuid>> =
        std::collections::HashMap::new();
    for (entity_id, community_id) in &labels {
        communities
            .entry(*community_id)
            .or_default()
            .push(*entity_id);
    }
    let community_list: Vec<serde_json::Value> = communities
        .iter()
        .map(|(cid, members)| {
            serde_json::json!({
                "community_id": cid,
                "member_count": members.len(),
                "entity_ids": members,
            })
        })
        .collect();

    Ok(Json(serde_json::json!({
        "user_id": user_rec.id,
        "total_entities": labels.len(),
        "community_count": communities.len(),
        "communities": community_list,
    })))
}

/// `GET /api/v1/graph/:user/path`
///
/// Find the shortest path between two entities in the user's knowledge graph.
#[derive(Deserialize)]
struct GraphPathParams {
    /// Source entity ID.
    from: Uuid,
    /// Target entity ID.
    to: Uuid,
    /// Maximum hops to search (default: 10, capped at 20).
    #[serde(default = "default_path_max_depth")]
    max_depth: u32,
    /// Only follow valid (non-invalidated) edges (default: true).
    #[serde(default = "default_true")]
    valid_only: bool,
}

fn default_path_max_depth() -> u32 {
    10
}

async fn graph_shortest_path(
    State(state): State<AppState>,
    Path(user): Path<String>,
    Query(params): Query<GraphPathParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    let user_rec = find_user_by_identifier(&state, &user).await?;

    // Verify both entities belong to the user
    let from_entity = state.state_store.get_entity(params.from).await?;
    if from_entity.user_id != user_rec.id {
        return Err(AppError(MnemoError::NotFound {
            resource_type: "entity".into(),
            id: params.from.to_string(),
        }));
    }
    let to_entity = state.state_store.get_entity(params.to).await?;
    if to_entity.user_id != user_rec.id {
        return Err(AppError(MnemoError::NotFound {
            resource_type: "entity".into(),
            id: params.to.to_string(),
        }));
    }

    let result = state
        .graph
        .find_shortest_path(
            params.from,
            params.to,
            params.max_depth.min(20),
            params.valid_only,
        )
        .await?;

    let steps: Vec<serde_json::Value> = result
        .steps
        .iter()
        .map(|s| {
            let mut step = serde_json::json!({
                "entity_id": s.entity.id,
                "entity_name": s.entity.name,
                "entity_type": s.entity.entity_type.as_str(),
                "depth": s.depth,
            });
            if let Some(ref edge) = s.edge {
                step["edge"] = serde_json::json!({
                    "id": edge.id,
                    "source_entity_id": edge.source_entity_id,
                    "target_entity_id": edge.target_entity_id,
                    "label": edge.label,
                    "fact": edge.fact,
                    "valid": edge.is_valid(),
                });
            }
            step
        })
        .collect();

    Ok(Json(serde_json::json!({
        "from": params.from,
        "to": params.to,
        "found": result.found,
        "path_length": if result.found { result.steps.len().saturating_sub(1) } else { 0 },
        "steps": steps,
        "entities_visited": result.entities_visited,
    })))
}

// ─── LLM Span Tracing API ──────────────────────────────────────────────────

use crate::state::{LlmSpan, MemoryDigest};

/// Maximum number of LLM spans retained in the in-memory ring buffer.
const MAX_LLM_SPANS: usize = 500;

/// Record an LLM span into the ring buffer and persist to Redis.
/// Redis persistence is best-effort; failures are logged but do not propagate.
async fn record_llm_span(state: &AppState, span: LlmSpan) {
    // Persist to Redis (best-effort)
    if let Err(e) = state.state_store.save_span(&span).await {
        tracing::warn!("Failed to persist LLM span to Redis: {e}");
    }

    // Also push to the in-memory ring buffer
    let mut spans = state.llm_spans.write().await;
    if spans.len() >= MAX_LLM_SPANS {
        spans.pop_front();
    }
    spans.push_back(span);
}

/// `GET /api/v1/spans/request/:request_id`
///
/// Return all LLM call spans associated with a specific request ID.
/// Reads from Redis first; falls back to the in-memory ring buffer if Redis
/// returns no results (e.g. spans were created before persistence was enabled).
async fn list_spans_by_request(
    State(state): State<AppState>,
    Path(request_id): Path<String>,
) -> Json<serde_json::Value> {
    // Try Redis first
    let matched: Vec<LlmSpan> = match state.state_store.get_spans_by_request(&request_id).await {
        Ok(spans) if !spans.is_empty() => spans,
        _ => {
            // Fallback to in-memory ring buffer
            let spans = state.llm_spans.read().await;
            spans
                .iter()
                .filter(|s| s.request_id.as_deref() == Some(request_id.as_str()))
                .cloned()
                .collect()
        }
    };
    Json(serde_json::json!({
        "request_id": request_id,
        "spans": matched,
        "count": matched.len(),
        "total_tokens": matched.iter().map(|s| s.total_tokens).sum::<u32>(),
        "total_latency_ms": matched.iter().map(|s| s.latency_ms).sum::<u64>(),
    }))
}

/// `GET /api/v1/spans/user/:user_id`
///
/// Return recent LLM spans for a given user ID.
/// Reads from Redis first; falls back to the in-memory ring buffer if Redis
/// returns no results.
#[derive(Deserialize)]
struct SpansUserParams {
    #[serde(default = "default_spans_limit")]
    limit: usize,
}

fn default_spans_limit() -> usize {
    100
}

async fn list_spans_by_user(
    State(state): State<AppState>,
    Path(user_id): Path<Uuid>,
    Query(params): Query<SpansUserParams>,
) -> Json<serde_json::Value> {
    let clamped_limit = params.limit.clamp(1, 1000);
    // Try Redis first
    let matched: Vec<LlmSpan> =
        match state.state_store.get_spans_by_user(user_id, clamped_limit).await {
            Ok(spans) if !spans.is_empty() => spans,
            _ => {
                // Fallback to in-memory ring buffer
                let spans = state.llm_spans.read().await;
                spans
                    .iter()
                    .rev()
                    .filter(|s| s.user_id == Some(user_id))
                    .take(clamped_limit)
                    .cloned()
                    .collect()
            }
        };
    Json(serde_json::json!({
        "user_id": user_id,
        "spans": matched,
        "count": matched.len(),
        "total_tokens": matched.iter().map(|s| s.total_tokens).sum::<u32>(),
        "total_latency_ms": matched.iter().map(|s| s.latency_ms).sum::<u64>(),
    }))
}

// ─── Sleep-time compute: Memory Digest API ────────────────────────────────

/// `GET /api/v1/memory/:user/digest`
///
/// Return the memory digest for a user. Reads from the in-memory cache first;
/// on cache miss, falls through to Redis (read-through) and populates the cache.
/// Returns 404 if no digest has been generated.
async fn get_memory_digest(
    State(state): State<AppState>,
    Path(user): Path<String>,
) -> Result<Json<MemoryDigest>, AppError> {
    let user_rec = find_user_by_identifier(&state, &user).await?;

    // Fast path: in-memory cache hit
    {
        let digests = state.memory_digests.read().await;
        if let Some(digest) = digests.get(&user_rec.id) {
            return Ok(Json(digest.clone()));
        }
    }

    // Slow path: read-through from Redis (handles cross-replica writes, restarts
    // where warm-up partially failed, etc.)
    {
        use mnemo_core::traits::storage::DigestStore as _;
        if let Ok(Some(digest)) = state.state_store.get_digest(user_rec.id).await {
            // Populate the in-memory cache so subsequent reads are fast
            let mut digests = state.memory_digests.write().await;
            digests.insert(user_rec.id, digest.clone());
            return Ok(Json(digest));
        }
    }

    Err(AppError(MnemoError::NotFound {
        resource_type: "memory_digest".into(),
        id: user.clone(),
    }))
}

/// `POST /api/v1/memory/:user/digest`
///
/// Trigger a fresh memory digest generation using the LLM.
/// The digest summarises the user's entity graph into a compact prose form.
async fn refresh_memory_digest(
    State(state): State<AppState>,
    Path(user): Path<String>,
) -> Result<Json<MemoryDigest>, AppError> {
    let user_rec = find_user_by_identifier(&state, &user).await?;

    let Some(ref llm) = state.llm else {
        return Err(AppError(MnemoError::Validation(
            "LLM provider is not configured; cannot generate memory digest".into(),
        )));
    };

    // Gather raw material: entities + top edges
    let entities = state
        .state_store
        .list_entities(user_rec.id, 200, None)
        .await?;
    let filter = EdgeFilter {
        include_invalidated: false,
        limit: 300,
        ..Default::default()
    };
    let edges = state.state_store.query_edges(user_rec.id, filter).await?;

    let entity_count = entities.len();
    let edge_count = edges.len();

    if entity_count == 0 {
        return Err(AppError(MnemoError::Validation(
            "No entities found for user — ingest some episodes first".into(),
        )));
    }

    // Build a compact prompt
    let entity_lines: Vec<String> = entities
        .iter()
        .take(80)
        .map(|e| {
            if let Some(ref s) = e.summary {
                format!("- {} ({}): {}", e.name, e.entity_type.as_str(), s)
            } else {
                format!("- {} ({})", e.name, e.entity_type.as_str())
            }
        })
        .collect();
    let edge_lines: Vec<String> = edges
        .iter()
        .take(60)
        .map(|e| format!("- {}", e.fact))
        .collect();

    let prompt = format!(
        "You are analyzing a user's long-term memory knowledge graph.\n\
        Entities ({entity_count} total, showing up to 80):\n{entities_block}\n\n\
        Key relationships ({edge_count} total, showing up to 60):\n{edges_block}\n\n\
        Respond with ONLY a JSON object (no markdown fences, no extra text) \
        matching this exact schema:\n\
        {{\n  \"summary\": \"<2-4 sentence prose summary of what this person knows, \
        their main areas of interest, and dominant themes>\",\n  \
        \"topics\": [\"topic1\", \"topic2\", \"topic3\"]\n}}\n\
        List 3-6 key topics. Do not include any text outside the JSON object.",
        entity_count = entity_count,
        entities_block = entity_lines.join("\n"),
        edge_count = edge_count,
        edges_block = edge_lines.join("\n"),
    );

    let model_name = llm.model_name().to_string();

    let started = std::time::Instant::now();
    let (raw, usage) = llm
        .summarize_with_usage(&prompt, 512)
        .await
        .map_err(|e: mnemo_core::error::MnemoError| {
            AppError(MnemoError::LlmProvider {
                provider: "digest".into(),
                message: e.to_string(),
            })
        })?;
    let latency_ms = started.elapsed().as_millis() as u64;

    // Record the LLM span for observability
    let span = LlmSpan {
        id: Uuid::now_v7(),
        request_id: None,
        user_id: Some(user_rec.id),
        provider: llm.provider_name().to_string(),
        model: model_name.clone(),
        operation: "digest".to_string(),
        prompt_tokens: usage.prompt_tokens,
        completion_tokens: usage.completion_tokens,
        total_tokens: usage.total_tokens,
        latency_ms,
        success: true,
        error: None,
        started_at: chrono::Utc::now() - chrono::Duration::milliseconds(latency_ms as i64),
        finished_at: chrono::Utc::now(),
    };
    record_llm_span(&state, span).await;

    // Parse structured JSON response with fallback to legacy TOPICS: format
    let (summary_text, dominant_topics) = mnemo_ingest::parse_digest_response(&raw);

    let digest = MemoryDigest {
        user_id: user_rec.id,
        summary: summary_text,
        entity_count,
        edge_count,
        dominant_topics,
        generated_at: chrono::Utc::now(),
        model: model_name,
        coherence_score: None,
    };

    // Persist to Redis for durability — fail the request if persistence fails,
    // so the client knows the digest is not durable.
    {
        use mnemo_core::traits::storage::DigestStore as _;
        state.state_store.save_digest(&digest).await?;
    }

    // Cache the digest in memory for fast reads (only after Redis persistence succeeds)
    {
        let mut digests = state.memory_digests.write().await;
        digests.insert(user_rec.id, digest.clone());
    }

    Ok(Json(digest))
}

// ─── Coherence Scoring ─────────────────────────────────────────────

/// `GET /api/v1/users/:user/coherence`
///
/// Returns a coherence report for a user's knowledge graph, measuring
/// internal consistency across four dimensions: entity coherence, fact
/// coherence, temporal coherence, and structural coherence.
async fn get_user_coherence(
    State(state): State<AppState>,
    Path(user_identifier): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let user = find_user_by_identifier(&state, user_identifier.trim()).await?;

    // Fetch all entities and edges (including invalidated for full picture)
    let entities = list_all_entities_for_user(&state, user.id).await?;
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

    // Run community detection for structural coherence
    let community_map = state
        .graph
        .detect_communities(user.id, 10)
        .await
        .unwrap_or_default();

    // Compute the full coherence report
    let report =
        mnemo_retrieval::coherence::compute_coherence_report(&entities, &edges, &community_map);

    Ok(Json(serde_json::json!({
        "user_id": user.id,
        "score": report.score,
        "entity_coherence": report.entity_coherence,
        "fact_coherence": report.fact_coherence,
        "temporal_coherence": report.temporal_coherence,
        "structural_coherence": report.structural_coherence,
        "recommendations": report.recommendations,
        "diagnostics": report.diagnostics,
    })))
}

#[cfg(test)]
mod tests {
    use super::{
        incident_sort_key, summarize_governance_violation, GovernanceAuditRecord,
        OpsIncidentResponse,
    };
    use serde_json::json;
    use uuid::Uuid;

    fn incident(
        severity: &str,
        opened_at: Option<chrono::DateTime<chrono::Utc>>,
    ) -> OpsIncidentResponse {
        OpsIncidentResponse {
            id: "incident-1".to_string(),
            kind: "policy_violation".to_string(),
            severity: severity.to_string(),
            title: "Test incident".to_string(),
            summary: "Test summary".to_string(),
            action_label: "Open".to_string(),
            action_href: "/_/governance".to_string(),
            resource_id: None,
            resource_label: None,
            request_id: None,
            opened_at,
        }
    }

    fn governance_record(details: serde_json::Value) -> GovernanceAuditRecord {
        GovernanceAuditRecord {
            id: Uuid::parse_str("11111111-1111-1111-1111-111111111111").expect("valid uuid"),
            user_id: Uuid::parse_str("22222222-2222-2222-2222-222222222222").expect("valid uuid"),
            request_id: None,
            action: "policy_violation.webhook_domain_allowlist".to_string(),
            details,
            at: chrono::Utc::now(),
        }
    }

    #[test]
    fn incident_sort_key_uses_stable_epoch_for_missing_timestamp() {
        let low_missing = incident("low", None);
        let low_with_time = incident("low", Some(chrono::DateTime::<chrono::Utc>::UNIX_EPOCH));

        assert_eq!(
            incident_sort_key(&low_missing),
            incident_sort_key(&low_with_time)
        );
    }

    #[test]
    fn incident_sort_key_prioritizes_severity_then_recency() {
        let older_high = incident(
            "high",
            Some(chrono::DateTime::from_timestamp(100, 0).expect("valid timestamp")),
        );
        let newer_medium = incident(
            "medium",
            Some(chrono::DateTime::from_timestamp(200, 0).expect("valid timestamp")),
        );
        let newer_high = incident(
            "high",
            Some(chrono::DateTime::from_timestamp(300, 0).expect("valid timestamp")),
        );

        assert!(incident_sort_key(&older_high) > incident_sort_key(&newer_medium));
        assert!(incident_sort_key(&newer_high) > incident_sort_key(&older_high));
    }

    #[test]
    fn summarize_governance_violation_prefers_target_url() {
        let record = governance_record(json!({
            "target_url": "https://blocked.example.com",
            "reason": "should not be used",
        }));

        assert_eq!(
            summarize_governance_violation(&record),
            "Blocked target: https://blocked.example.com"
        );
    }

    #[test]
    fn summarize_governance_violation_falls_back_to_reason() {
        let record = governance_record(json!({
            "reason": "Retention window exceeded",
        }));

        assert_eq!(
            summarize_governance_violation(&record),
            "Retention window exceeded"
        );
    }
}
