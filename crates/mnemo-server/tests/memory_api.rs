use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::body::{to_bytes, Body};
use axum::extract::State;
use axum::http::{HeaderMap, Request, StatusCode};
use axum::middleware::from_fn_with_state;
use axum::routing::post;
use axum::Router;
use chrono::Utc;
use hmac::{Hmac, Mac};
use serde_json::Value;
use sha2::Sha256;
use tower::ServiceExt;
use uuid::Uuid;

use mnemo_core::models::edge::{Edge, ExtractedRelationship};
use mnemo_core::models::entity::{Entity, EntityType, ExtractedEntity};
use mnemo_core::traits::fulltext::FullTextStore;
use mnemo_core::traits::llm::EmbeddingConfig;
use mnemo_core::traits::storage::{EdgeStore, EntityStore, UserStore};
use mnemo_graph::GraphEngine;
use mnemo_llm::{EmbedderKind, OpenAiCompatibleEmbedder};
use mnemo_retrieval::RetrievalEngine;
use mnemo_server::middleware::{request_context_middleware, REQUEST_ID_HEADER};
use mnemo_server::routes::{build_router, restore_webhook_state};
use mnemo_server::state::{
    AppState, GovernanceAuditRecord, MemoryWebhookEventRecord, MetadataPrefilterConfig,
    RerankerMode, ServerMetrics, WebhookDeliveryConfig, WebhookRuntimeState,
};
use mnemo_storage::{QdrantVectorStore, RedisStateStore};

async fn build_test_app() -> axum::Router {
    build_test_harness_with_prefilter(MetadataPrefilterConfig {
        enabled: true,
        scan_limit: 400,
        relax_if_empty: false,
    })
    .await
    .0
}

async fn build_test_app_with_prefilter(prefilter: MetadataPrefilterConfig) -> axum::Router {
    build_test_harness_with_prefilter(prefilter).await.0
}

async fn build_test_harness_with_prefilter(
    prefilter: MetadataPrefilterConfig,
) -> (axum::Router, Arc<RedisStateStore>) {
    let (app, _state, state_store) = build_test_harness_with_state_and_prefilter_and_webhooks(
        prefilter,
        WebhookDeliveryConfig {
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
    )
    .await;
    (app, state_store)
}

async fn build_test_harness_with_prefilter_and_webhooks(
    prefilter: MetadataPrefilterConfig,
    webhook_delivery: WebhookDeliveryConfig,
) -> (axum::Router, Arc<RedisStateStore>) {
    let (app, _state, state_store) =
        build_test_harness_with_state_and_prefilter_and_webhooks(prefilter, webhook_delivery).await;
    (app, state_store)
}

async fn build_test_harness_with_state_and_prefilter_and_webhooks(
    prefilter: MetadataPrefilterConfig,
    webhook_delivery: WebhookDeliveryConfig,
) -> (axum::Router, AppState, Arc<RedisStateStore>) {
    let redis_url = std::env::var("MNEMO_TEST_REDIS_URL")
        .unwrap_or_else(|_| "redis://localhost:6379".to_string());
    let qdrant_url = std::env::var("MNEMO_TEST_QDRANT_URL")
        .unwrap_or_else(|_| "http://localhost:6334".to_string());

    let uid = Uuid::now_v7();
    let redis_prefix = format!("memory_api_test:{}:", uid);
    let qdrant_prefix =
        std::env::var("MNEMO_TEST_QDRANT_PREFIX").unwrap_or_else(|_| "mnemo_".to_string());

    let state_store = Arc::new(
        RedisStateStore::new(&redis_url, &redis_prefix)
            .await
            .unwrap_or_else(|e| {
                panic!(
                    "Redis required for memory API tests. Run: docker compose -f docker-compose.test.yml up -d. Error: {e}"
                )
            }),
    );
    state_store.ensure_indexes().await.unwrap();

    let vector_store = Arc::new(
        QdrantVectorStore::new(&qdrant_url, &qdrant_prefix, 1536)
            .await
            .unwrap_or_else(|e| {
                panic!(
                    "Qdrant required for memory API tests. Run: docker compose -f docker-compose.test.yml up -d. Error: {e}"
                )
            }),
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
        metadata_prefilter: prefilter,
        reranker: RerankerMode::Rrf,
        import_jobs: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        import_idempotency: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        memory_webhooks: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        memory_webhook_events: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        memory_webhook_audit: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        user_policies: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        governance_audit: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        webhook_runtime: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        webhook_delivery,
        webhook_http: Arc::new(reqwest::Client::new()),
        webhook_redis: None,
        webhook_redis_prefix: "mnemo_test:webhooks".to_string(),
        metrics: Arc::new(ServerMetrics::default()),
        llm_spans: Arc::new(tokio::sync::RwLock::new(std::collections::VecDeque::new())),
        memory_digests: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        require_tls: false,
        audit_signing_secret: None,
    };

    let app = build_router(state.clone()).layer(from_fn_with_state(
        state.clone(),
        request_context_middleware,
    ));

    (app, state, state_store.clone())
}

async fn json_request(
    app: &axum::Router,
    method: &str,
    path: &str,
    payload: Value,
) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json")
        .body(Body::from(payload.to_string()))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let parsed = if body.is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_slice::<Value>(&body).unwrap()
    };
    (status, parsed)
}

async fn json_request_with_header(
    app: &axum::Router,
    method: &str,
    path: &str,
    header_name: &str,
    header_value: &str,
    payload: Value,
) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json")
        .header(header_name, header_value)
        .body(Body::from(payload.to_string()))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let parsed = if body.is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_slice::<Value>(&body).unwrap()
    };
    (status, parsed)
}

async fn get_request(app: &axum::Router, path: &str) -> (StatusCode, Value) {
    let request = Request::builder()
        .method("GET")
        .uri(path)
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let parsed = if body.is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_slice::<Value>(&body).unwrap()
    };
    (status, parsed)
}

async fn wait_for_import_job(app: &axum::Router, job_id: &str) -> Value {
    for _ in 0..80 {
        let (status, job) = json_request(
            app,
            "GET",
            &format!("/api/v1/import/jobs/{job_id}"),
            serde_json::json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        if job["status"] == "completed" || job["status"] == "failed" {
            return job;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }

    panic!("import job {job_id} did not reach terminal state in time");
}

#[derive(Clone)]
struct DeliveryCapture {
    attempts: Arc<AtomicUsize>,
    fail_first: usize,
    deliveries: Arc<tokio::sync::Mutex<Vec<(HeaderMap, String)>>>,
}

async fn webhook_sink_handler(
    State(capture): State<DeliveryCapture>,
    headers: HeaderMap,
    body: String,
) -> impl axum::response::IntoResponse {
    let attempt = capture.attempts.fetch_add(1, Ordering::SeqCst) + 1;
    if attempt <= capture.fail_first {
        return StatusCode::INTERNAL_SERVER_ERROR;
    }

    {
        let mut rows = capture.deliveries.lock().await;
        rows.push((headers, body));
    }

    StatusCode::OK
}

async fn start_webhook_sink_server(
    fail_first: usize,
) -> (
    String,
    Arc<AtomicUsize>,
    Arc<tokio::sync::Mutex<Vec<(HeaderMap, String)>>>,
) {
    let attempts = Arc::new(AtomicUsize::new(0));
    let deliveries = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let capture = DeliveryCapture {
        attempts: attempts.clone(),
        fail_first,
        deliveries: deliveries.clone(),
    };

    let app = Router::new()
        .route("/hook", post(webhook_sink_handler))
        .with_state(capture);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    (format!("http://{addr}/hook"), attempts, deliveries)
}

fn compute_expected_signature(secret: &str, timestamp: &str, body: &str) -> String {
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
    let signed = format!("{timestamp}.{body}");
    mac.update(signed.as_bytes());
    let digest = hex::encode(mac.finalize().into_bytes());
    format!("t={timestamp},v1={digest}")
}

async fn wait_for_webhook_delivery(
    app: &axum::Router,
    webhook_id: &str,
    expected_delivered: bool,
) -> Value {
    for _ in 0..60 {
        let (status, body) = json_request(
            app,
            "GET",
            &format!("/api/v1/memory/webhooks/{webhook_id}/events?limit=1"),
            serde_json::json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        if body["count"].as_u64().unwrap_or(0) > 0 {
            let first = body["events"][0].clone();
            if first["delivered"].as_bool().unwrap_or(false) == expected_delivered {
                return first;
            }
        }

        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    panic!("webhook delivery status did not reach expected state");
}

async fn wait_for_webhook_event(app: &axum::Router, webhook_id: &str) -> Value {
    for _ in 0..60 {
        let (status, body) = json_request(
            app,
            "GET",
            &format!("/api/v1/memory/webhooks/{webhook_id}/events?limit=1"),
            serde_json::json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        if body["count"].as_u64().unwrap_or(0) > 0 {
            return body["events"][0].clone();
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("webhook event was not recorded");
}

async fn wait_for_dead_letter_event(app: &axum::Router, webhook_id: &str) -> Value {
    for _ in 0..80 {
        let (status, body) = json_request(
            app,
            "GET",
            &format!("/api/v1/memory/webhooks/{webhook_id}/events/dead-letter?limit=1"),
            serde_json::json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        if body["count"].as_u64().unwrap_or(0) > 0 {
            return body["events"][0].clone();
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("webhook event was not dead-lettered");
}

#[tokio::test]
async fn test_request_id_header_is_set_and_propagated() {
    let app = build_test_app().await;

    let request = Request::builder()
        .method("GET")
        .uri("/health")
        .header(REQUEST_ID_HEADER, "req-test-123")
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let echoed = response
        .headers()
        .get(REQUEST_ID_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert_eq!(echoed, "req-test-123");
}

#[tokio::test]
async fn test_metrics_endpoint_exposes_prometheus_text() {
    let app = build_test_app().await;

    let request = Request::builder()
        .method("GET")
        .uri("/metrics")
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(content_type.contains("text/plain"));

    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("mnemo_http_requests_total"));
    assert!(text.contains("mnemo_webhook_deliveries_success_total"));
}

#[tokio::test]
async fn test_memory_api_validation_and_resolution() {
    let app = build_test_app().await;

    let (status, body) = json_request(
        &app,
        "POST",
        "/api/v1/memory",
        serde_json::json!({"user": "   ", "text": "hello"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "validation_error");

    let (status, body) = json_request(
        &app,
        "POST",
        "/api/v1/memory",
        serde_json::json!({"user": "falsify-user", "text": "   "}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "validation_error");

    let (status, first) = json_request(
        &app,
        "POST",
        "/api/v1/memory",
        serde_json::json!({"user": "falsify-user", "text": "first fact", "session": "   "}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let user_id = first["user_id"].as_str().unwrap().to_string();
    let session_id = first["session_id"].as_str().unwrap().to_string();

    let (status, second) = json_request(
        &app,
        "POST",
        "/api/v1/memory",
        serde_json::json!({"user": "falsify-user", "text": "second fact"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(second["session_id"], session_id);

    let (status, _) = json_request(
        &app,
        "POST",
        "/api/v1/memory",
        serde_json::json!({"user": "falsify-user", "text": "assistant says hi", "role": "assistant"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, episodes) = json_request(
        &app,
        "GET",
        &format!("/api/v1/sessions/{session_id}/episodes?limit=10"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let roles: Vec<String> = episodes["data"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["role"].as_str().map(|r| r.to_string()))
        .collect();
    assert!(roles.contains(&"assistant".to_string()));
    assert!(roles.contains(&"user".to_string()));

    let (status, body) = json_request(
        &app,
        "POST",
        "/api/v1/memory/no-such-user/context",
        serde_json::json!({"query": "hi"}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "not_found");

    let (status, body) = json_request(
        &app,
        "POST",
        &format!("/api/v1/memory/{user_id}/context"),
        serde_json::json!({"query": "what do i know?"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.get("context").is_some());
}

#[tokio::test]
async fn test_memory_contract_historical_strict_requires_as_of() {
    let app = build_test_app().await;

    let (status, _) = json_request(
        &app,
        "POST",
        "/api/v1/users",
        serde_json::json!({
            "name": "historical-contract-user",
            "external_id": "historical-contract-user",
            "metadata": {}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, _) = json_request(
        &app,
        "POST",
        "/api/v1/memory/historical-contract-user/context",
        serde_json::json!({
            "query": "what changed?",
            "contract": "historical_strict"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_memory_contract_support_safe_filters_non_user_episodes() {
    let app = build_test_app().await;

    let (status, user) = json_request(
        &app,
        "POST",
        "/api/v1/users",
        serde_json::json!({
            "name": "contract-user",
            "external_id": "contract-user",
            "metadata": {}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let user_id = user["id"].as_str().unwrap().to_string();

    let (status, session) = json_request(
        &app,
        "POST",
        "/api/v1/sessions",
        serde_json::json!({"user_id": user_id, "name": "contract-session"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let session_id = session["id"].as_str().unwrap().to_string();

    let (status, _) = json_request(
        &app,
        "POST",
        &format!("/api/v1/sessions/{session_id}/episodes"),
        serde_json::json!({
            "type": "message",
            "role": "user",
            "content": "I prefer Nike.",
            "created_at": "2025-01-01T00:00:00Z"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, _) = json_request(
        &app,
        "POST",
        &format!("/api/v1/sessions/{session_id}/episodes"),
        serde_json::json!({
            "type": "message",
            "role": "assistant",
            "content": "Noted. You prefer Nike.",
            "created_at": "2025-01-01T00:00:01Z"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, body) = json_request(
        &app,
        "POST",
        "/api/v1/memory/contract-user/context",
        serde_json::json!({
            "query": "What do I prefer?",
            "session": "contract-session",
            "contract": "support_safe"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["contract_applied"], "support_safe");
    let episodes = body["episodes"].as_array().unwrap();
    assert!(episodes.iter().all(|e| e["role"] == "user"));
}

#[tokio::test]
async fn test_retrieval_policy_precision_reports_effective_thresholds() {
    let app = build_test_app().await;

    let (status, _) = json_request(
        &app,
        "POST",
        "/api/v1/memory",
        serde_json::json!({
            "user": "policy-precision-user",
            "session": "policy-session",
            "text": "I prefer Nike running shoes."
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, body) = json_request(
        &app,
        "POST",
        "/api/v1/memory/policy-precision-user/context",
        serde_json::json!({
            "query": "What do I prefer?",
            "session": "policy-session",
            "retrieval_policy": "precision"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["retrieval_policy_applied"], "precision");
    assert_eq!(
        body["retrieval_policy_diagnostics"]["effective_min_relevance"],
        serde_json::json!(0.55)
    );
    assert_eq!(
        body["retrieval_policy_diagnostics"]["effective_max_tokens"],
        serde_json::json!(400)
    );
}

#[tokio::test]
async fn test_retrieval_policy_stability_biases_temporal_intent_current() {
    let app = build_test_app().await;

    let (status, _) = json_request(
        &app,
        "POST",
        "/api/v1/memory",
        serde_json::json!({
            "user": "policy-stability-user",
            "session": "policy-session",
            "text": "I switched to Nike last week."
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, body) = json_request(
        &app,
        "POST",
        "/api/v1/memory/policy-stability-user/context",
        serde_json::json!({
            "query": "What is current?",
            "session": "policy-session",
            "retrieval_policy": "stability"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["retrieval_policy_applied"], "stability");
    assert_eq!(
        body["retrieval_policy_diagnostics"]["effective_temporal_intent"],
        "current"
    );
}

#[tokio::test]
async fn test_memory_changes_since_reports_episode_and_head_changes() {
    let app = build_test_app().await;

    let (status, user) = json_request(
        &app,
        "POST",
        "/api/v1/users",
        serde_json::json!({
            "name": "diff-user",
            "external_id": "diff-user",
            "metadata": {}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let user_id = user["id"].as_str().unwrap().to_string();

    let (status, session) = json_request(
        &app,
        "POST",
        "/api/v1/sessions",
        serde_json::json!({ "user_id": user_id, "name": "diff-session" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let session_id = session["id"].as_str().unwrap().to_string();

    let (status, _) = json_request(
        &app,
        "POST",
        &format!("/api/v1/sessions/{session_id}/episodes"),
        serde_json::json!({
            "type": "message",
            "role": "user",
            "content": "older event",
            "created_at": "2025-01-01T00:00:00Z"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, _) = json_request(
        &app,
        "POST",
        &format!("/api/v1/sessions/{session_id}/episodes"),
        serde_json::json!({
            "type": "message",
            "role": "user",
            "content": "newer event",
            "created_at": "2025-03-01T00:00:00Z"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, diff) = json_request(
        &app,
        "POST",
        "/api/v1/memory/diff-user/changes_since",
        serde_json::json!({
            "from": "2025-02-01T00:00:00Z",
            "to": "2025-04-01T00:00:00Z"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(!diff["added_episodes"].as_array().unwrap().is_empty());
    assert!(!diff["head_changes"].as_array().unwrap().is_empty());
    assert!(diff["summary"]
        .as_str()
        .unwrap_or_default()
        .contains("added episodes"));
}

#[tokio::test]
async fn test_memory_changes_since_rejects_invalid_window() {
    let app = build_test_app().await;

    let (status, _) = json_request(
        &app,
        "POST",
        "/api/v1/memory/nope/changes_since",
        serde_json::json!({
            "from": "2025-04-01T00:00:00Z",
            "to": "2025-01-01T00:00:00Z"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_time_travel_trace_reports_fact_shift_and_timeline() {
    let (app, state_store) = build_test_harness_with_prefilter(MetadataPrefilterConfig {
        enabled: true,
        scan_limit: 400,
        relax_if_empty: false,
    })
    .await;

    let (status, user) = json_request(
        &app,
        "POST",
        "/api/v1/users",
        serde_json::json!({
            "name": "trace-user",
            "external_id": "trace-user",
            "metadata": {}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let user_id = Uuid::parse_str(user["id"].as_str().unwrap()).unwrap();

    let (status, session) = json_request(
        &app,
        "POST",
        "/api/v1/sessions",
        serde_json::json!({ "user_id": user_id, "name": "trace-session" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let session_id = session["id"].as_str().unwrap().to_string();

    let (status, e1) = json_request(
        &app,
        "POST",
        &format!("/api/v1/sessions/{session_id}/episodes"),
        serde_json::json!({
            "type": "message",
            "role": "user",
            "content": "Kendra preferred Adidas before February",
            "created_at": "2025-01-10T00:00:00Z"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let episode_1 = Uuid::parse_str(e1["id"].as_str().unwrap()).unwrap();

    let (status, e2) = json_request(
        &app,
        "POST",
        &format!("/api/v1/sessions/{session_id}/episodes"),
        serde_json::json!({
            "type": "message",
            "role": "user",
            "content": "Kendra now prefers Nike",
            "created_at": "2025-03-10T00:00:00Z"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let episode_2 = Uuid::parse_str(e2["id"].as_str().unwrap()).unwrap();

    let src = state_store
        .create_entity(Entity::from_extraction(
            &ExtractedEntity {
                name: "Kendra".to_string(),
                entity_type: EntityType::Person,
                summary: None,
            },
            user_id,
            episode_1,
        ))
        .await
        .unwrap();
    let adidas = state_store
        .create_entity(Entity::from_extraction(
            &ExtractedEntity {
                name: "Adidas".to_string(),
                entity_type: EntityType::Organization,
                summary: None,
            },
            user_id,
            episode_1,
        ))
        .await
        .unwrap();
    let nike = state_store
        .create_entity(Entity::from_extraction(
            &ExtractedEntity {
                name: "Nike".to_string(),
                entity_type: EntityType::Organization,
                summary: None,
            },
            user_id,
            episode_2,
        ))
        .await
        .unwrap();

    let jan = chrono::DateTime::parse_from_rfc3339("2025-01-10T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let feb = chrono::DateTime::parse_from_rfc3339("2025-02-20T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let mar = chrono::DateTime::parse_from_rfc3339("2025-03-10T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc);

    let mut old_edge = state_store
        .create_edge(Edge::from_extraction(
            &ExtractedRelationship {
                source_name: "Kendra".to_string(),
                target_name: "Adidas".to_string(),
                label: "prefers".to_string(),
                fact: "Kendra prefers Adidas".to_string(),
                confidence: 0.85,
                valid_at: Some(jan),
            },
            user_id,
            src.id,
            adidas.id,
            episode_1,
            jan,
        ))
        .await
        .unwrap();
    old_edge.invalid_at = Some(feb);
    state_store.update_edge(&old_edge).await.unwrap();

    state_store
        .create_edge(Edge::from_extraction(
            &ExtractedRelationship {
                source_name: "Kendra".to_string(),
                target_name: "Nike".to_string(),
                label: "prefers".to_string(),
                fact: "Kendra prefers Nike".to_string(),
                confidence: 0.87,
                valid_at: Some(mar),
            },
            user_id,
            src.id,
            nike.id,
            episode_2,
            mar,
        ))
        .await
        .unwrap();

    let (status, trace) = json_request(
        &app,
        "POST",
        "/api/v1/memory/trace-user/time_travel/trace",
        serde_json::json!({
            "query": "What does Kendra prefer?",
            "session": "trace-session",
            "from": "2025-02-01T00:00:00Z",
            "to": "2025-04-01T00:00:00Z",
            "contract": "historical_strict",
            "retrieval_policy": "balanced",
            "min_relevance": 0.0
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(trace["contract_applied"], "historical_strict");
    assert_eq!(trace["retrieval_policy_applied"], "balanced");
    assert!(!trace["timeline"].as_array().unwrap().is_empty());
    assert!(trace["timeline"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["event_type"] == "fact_superseded"));
    assert!(trace["timeline"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["event_type"] == "fact_added"));
    assert!(trace["gained_facts"].as_array().unwrap().len() <= 8);
}

#[tokio::test]
async fn test_time_travel_trace_rejects_invalid_window() {
    let app = build_test_app().await;

    let (status, _) = json_request(
        &app,
        "POST",
        "/api/v1/memory/nope/time_travel/trace",
        serde_json::json!({
            "query": "what changed",
            "from": "2025-04-01T00:00:00Z",
            "to": "2025-01-01T00:00:00Z"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_time_travel_summary_reports_delta_counts() {
    let app = build_test_app().await;

    let (status, _) = json_request(
        &app,
        "POST",
        "/api/v1/memory",
        serde_json::json!({
            "user": "summary-user",
            "session": "summary-session",
            "text": "Kendra now prefers Nike"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, body) = json_request(
        &app,
        "POST",
        "/api/v1/memory/summary-user/time_travel/summary",
        serde_json::json!({
            "query": "What changed about Kendra preferences?",
            "session": "summary-session",
            "from": "2025-01-01T00:00:00Z",
            "to": "2025-04-01T00:00:00Z",
            "contract": "historical_strict",
            "retrieval_policy": "balanced"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["contract_applied"], "historical_strict");
    assert_eq!(body["retrieval_policy_applied"], "balanced");
    assert!(body["fact_count_from"].as_u64().is_some());
    assert!(body["fact_count_to"].as_u64().is_some());
    assert!(body["episode_count_from"].as_u64().is_some());
    assert!(body["episode_count_to"].as_u64().is_some());
    assert!(body["summary"]
        .as_str()
        .unwrap_or_default()
        .contains("facts"));
}

#[tokio::test]
async fn test_policy_preview_estimates_retention_impact() {
    let app = build_test_app().await;

    let (status, user) = json_request(
        &app,
        "POST",
        "/api/v1/users",
        serde_json::json!({
            "name": "preview-user",
            "external_id": "preview-user",
            "metadata": {}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let user_id = user["id"].as_str().unwrap();

    let (status, session) = json_request(
        &app,
        "POST",
        "/api/v1/sessions",
        serde_json::json!({ "user_id": user_id, "name": "preview-session" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let session_id = session["id"].as_str().unwrap();

    let (status, _) = json_request(
        &app,
        "POST",
        &format!("/api/v1/sessions/{session_id}/episodes"),
        serde_json::json!({
            "type": "message",
            "role": "user",
            "content": "older policy preview event",
            "created_at": "2025-01-01T00:00:00Z"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, preview) = json_request(
        &app,
        "POST",
        "/api/v1/policies/preview-user/preview",
        serde_json::json!({
            "retention_days_message": 1
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(preview["preview_policy"]["retention_days_message"], 1);
    assert!(
        preview["estimated_affected_episodes_total"]
            .as_u64()
            .unwrap_or(0)
            >= 1
    );
    assert_eq!(preview["confidence"], "estimated");
}

#[tokio::test]
async fn test_conflict_radar_detects_active_fact_conflict() {
    let (app, state_store) = build_test_harness_with_prefilter_and_webhooks(
        MetadataPrefilterConfig {
            enabled: true,
            scan_limit: 400,
            relax_if_empty: false,
        },
        WebhookDeliveryConfig {
            enabled: true,
            max_attempts: 5,
            base_backoff_ms: 10,
            request_timeout_ms: 500,
            max_events_per_webhook: 1000,
            rate_limit_per_minute: 120,
            circuit_breaker_threshold: 5,
            circuit_breaker_cooldown_ms: 200,
            persistence_enabled: false,
        },
    )
    .await;

    let (status, user) = json_request(
        &app,
        "POST",
        "/api/v1/users",
        serde_json::json!({
            "name": "conflict-user",
            "external_id": "conflict-user",
            "metadata": {}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let user_id = Uuid::parse_str(user["id"].as_str().unwrap()).unwrap();

    let episode_id = Uuid::now_v7();
    let src = state_store
        .create_entity(Entity::from_extraction(
            &ExtractedEntity {
                name: "Kendra".to_string(),
                entity_type: EntityType::Person,
                summary: None,
            },
            user_id,
            episode_id,
        ))
        .await
        .unwrap();
    let adidas = state_store
        .create_entity(Entity::from_extraction(
            &ExtractedEntity {
                name: "Adidas".to_string(),
                entity_type: EntityType::Organization,
                summary: None,
            },
            user_id,
            episode_id,
        ))
        .await
        .unwrap();
    let nike = state_store
        .create_entity(Entity::from_extraction(
            &ExtractedEntity {
                name: "Nike".to_string(),
                entity_type: EntityType::Organization,
                summary: None,
            },
            user_id,
            episode_id,
        ))
        .await
        .unwrap();

    let now = Utc::now();
    state_store
        .create_edge(Edge::from_extraction(
            &ExtractedRelationship {
                source_name: "Kendra".to_string(),
                target_name: "Adidas".to_string(),
                label: "prefers".to_string(),
                fact: "Kendra prefers Adidas".to_string(),
                confidence: 0.8,
                valid_at: Some(now - chrono::Duration::days(2)),
            },
            user_id,
            src.id,
            adidas.id,
            episode_id,
            now,
        ))
        .await
        .unwrap();
    state_store
        .create_edge(Edge::from_extraction(
            &ExtractedRelationship {
                source_name: "Kendra".to_string(),
                target_name: "Nike".to_string(),
                label: "prefers".to_string(),
                fact: "Kendra prefers Nike".to_string(),
                confidence: 0.85,
                valid_at: Some(now - chrono::Duration::days(1)),
            },
            user_id,
            src.id,
            nike.id,
            episode_id,
            now,
        ))
        .await
        .unwrap();

    let (status, radar) = json_request(
        &app,
        "POST",
        "/api/v1/memory/conflict-user/conflict_radar",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let clusters = radar["conflicts"].as_array().unwrap();
    assert!(!clusters.is_empty());
    let first = &clusters[0];
    assert_eq!(first["label"], "prefers");
    assert!(first["active_edge_count"].as_u64().unwrap_or(0) >= 2);
    assert!(first["needs_resolution"].as_bool().unwrap_or(false));
    assert!(first["severity"].as_f64().unwrap_or(0.0) >= 0.8);
}

#[tokio::test]
async fn test_causal_recall_chains_returns_fact_lineage() {
    let (app, state_store) = build_test_harness_with_prefilter(MetadataPrefilterConfig {
        enabled: true,
        scan_limit: 400,
        relax_if_empty: false,
    })
    .await;

    let (status, user) = json_request(
        &app,
        "POST",
        "/api/v1/users",
        serde_json::json!({
            "name": "causal-user",
            "external_id": "causal-user",
            "metadata": {}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let user_id = Uuid::parse_str(user["id"].as_str().unwrap()).unwrap();

    let (status, session) = json_request(
        &app,
        "POST",
        "/api/v1/sessions",
        serde_json::json!({"user_id": user_id, "name": "causal-session"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let session_id = session["id"].as_str().unwrap();

    let (status, ep) = json_request(
        &app,
        "POST",
        &format!("/api/v1/sessions/{session_id}/episodes"),
        serde_json::json!({
            "type": "message",
            "role": "user",
            "content": "Kendra prefers Nike running shoes.",
            "created_at": "2025-01-01T00:00:00Z"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let episode_id = Uuid::parse_str(ep["id"].as_str().unwrap()).unwrap();

    let src = state_store
        .create_entity(Entity::from_extraction(
            &ExtractedEntity {
                name: "Kendra".to_string(),
                entity_type: EntityType::Person,
                summary: None,
            },
            user_id,
            episode_id,
        ))
        .await
        .unwrap();
    let nike = state_store
        .create_entity(Entity::from_extraction(
            &ExtractedEntity {
                name: "Nike".to_string(),
                entity_type: EntityType::Organization,
                summary: None,
            },
            user_id,
            episode_id,
        ))
        .await
        .unwrap();
    state_store
        .create_edge(Edge::from_extraction(
            &ExtractedRelationship {
                source_name: "Kendra".to_string(),
                target_name: "Nike".to_string(),
                label: "prefers".to_string(),
                fact: "Kendra prefers Nike running shoes".to_string(),
                confidence: 0.92,
                valid_at: Some(Utc::now()),
            },
            user_id,
            src.id,
            nike.id,
            episode_id,
            Utc::now(),
        ))
        .await
        .unwrap();

    let (status, resp) = json_request(
        &app,
        "POST",
        "/api/v1/memory/causal-user/causal_recall",
        serde_json::json!({
            "query": "What does Kendra prefer?",
            "session": "causal-session"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(resp["mode"], "hybrid");
    assert!(resp["chains"].is_array());
    let chains = resp["chains"].as_array().unwrap();
    assert!(!chains.is_empty(), "expected at least one causal chain");
    assert!(chains[0]["fact"]["fact_id"].is_string());
    assert!(chains[0]["source_episodes"].is_array());
}

#[tokio::test]
async fn test_causal_recall_chains_rejects_empty_query() {
    let app = build_test_app().await;
    let (status, _) = json_request(
        &app,
        "POST",
        "/api/v1/memory/some-user/causal_recall",
        serde_json::json!({"query": "   "}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_memory_api_immediate_recall_fallback_contains_recent_text() {
    let app = build_test_app().await;
    let marker = "falsify-anchovy-orbit-9271";

    let (status, _) = json_request(
        &app,
        "POST",
        "/api/v1/memory",
        serde_json::json!({
            "user": "timing-user",
            "text": format!("My secret marker is {marker}.")
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, context) = json_request(
        &app,
        "POST",
        "/api/v1/memory/timing-user/context",
        serde_json::json!({"query": "What is my secret marker?"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let text = context["context"].as_str().unwrap_or_default();
    assert!(
        text.contains(marker),
        "expected immediate recall to include marker, got context: {text}"
    );
}

#[tokio::test]
async fn test_chat_history_import_ndjson_pathway() {
    let app = build_test_app().await;

    let (status, import_resp) = json_request(
        &app,
        "POST",
        "/api/v1/import/chat-history",
        serde_json::json!({
            "user": "import-user-1",
            "source": "ndjson",
            "payload": [
                {
                    "session": "Imported Thread",
                    "role": "user",
                    "content": "Imported message one",
                    "created_at": "2025-01-01T00:00:00Z"
                },
                {
                    "session": "Imported Thread",
                    "role": "assistant",
                    "content": "Imported response one",
                    "created_at": "2025-01-01T00:00:10Z"
                }
            ]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let job_id = import_resp["job_id"].as_str().unwrap().to_string();

    let mut completed = false;
    for _ in 0..40 {
        let (job_status, job) = json_request(
            &app,
            "GET",
            &format!("/api/v1/import/jobs/{job_id}"),
            serde_json::json!({}),
        )
        .await;
        assert_eq!(job_status, StatusCode::OK);

        if job["status"] == "completed" {
            completed = true;
            assert_eq!(job["imported_messages"], 2);
            assert_eq!(job["failed_messages"], 0);
            break;
        }
        if job["status"] == "failed" {
            panic!("import job failed unexpectedly: {job}");
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    assert!(completed, "import job did not complete in time");

    let (status, user) = json_request(
        &app,
        "GET",
        "/api/v1/users/external/import-user-1",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let user_id = user["id"].as_str().unwrap();

    let (status, context) = json_request(
        &app,
        "POST",
        &format!("/api/v1/memory/{user_id}/context"),
        serde_json::json!({"query": "What was imported?", "session": "Imported Thread"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let context_text = context["context"].as_str().unwrap_or_default();
    assert!(
        context_text.contains("Imported message one")
            || context_text.contains("Imported response one"),
        "expected imported content in context, got: {}",
        context_text
    );
}

#[tokio::test]
async fn test_chat_history_import_rejects_malformed_rows() {
    let app = build_test_app().await;

    let (status, import_resp) = json_request(
        &app,
        "POST",
        "/api/v1/import/chat-history",
        serde_json::json!({
            "user": "import-user-malformed",
            "source": "ndjson",
            "payload": [
                {
                    "session": "broken",
                    "role": "user",
                    "created_at": "2025-01-01T00:00:00Z"
                }
            ]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);

    let job_id = import_resp["job_id"].as_str().unwrap();
    let job = wait_for_import_job(&app, job_id).await;
    assert_eq!(job["status"], "failed");
    assert_eq!(job["total_messages"], 0);
    assert_eq!(job["imported_messages"], 0);
    assert!(job["errors"]
        .as_array()
        .map(|errors| !errors.is_empty())
        .unwrap_or(false));
}

#[tokio::test]
async fn test_chat_history_import_supports_mixed_timestamp_quality() {
    let app = build_test_app().await;

    let (status, import_resp) = json_request(
        &app,
        "POST",
        "/api/v1/import/chat-history",
        serde_json::json!({
            "user": "import-user-timestamps",
            "source": "ndjson",
            "default_session": "Timestamp Mix",
            "payload": [
                {
                    "role": "user",
                    "content": "RFC3339 timestamp row",
                    "created_at": "2025-01-01T00:00:00Z"
                },
                {
                    "role": "assistant",
                    "content": "Unix timestamp row",
                    "created_at": "1735689605"
                },
                {
                    "role": "user",
                    "content": "Missing timestamp row"
                }
            ]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);

    let job_id = import_resp["job_id"].as_str().unwrap();
    let job = wait_for_import_job(&app, job_id).await;
    assert_eq!(job["status"], "completed");
    assert_eq!(job["total_messages"], 3);
    assert_eq!(job["imported_messages"], 3);
    assert_eq!(job["failed_messages"], 0);
}

#[tokio::test]
async fn test_chat_history_import_idempotency_prevents_duplicate_replay() {
    let app = build_test_app().await;

    let payload = serde_json::json!({
        "user": "import-user-idempotent",
        "source": "ndjson",
        "idempotency_key": "replay-key-42",
        "default_session": "Idempotent Session",
        "payload": [
            {
                "role": "user",
                "content": "Import exactly once",
                "created_at": "2025-01-01T00:00:00Z"
            }
        ]
    });

    let (status, first) =
        json_request(&app, "POST", "/api/v1/import/chat-history", payload.clone()).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let first_job_id = first["job_id"].as_str().unwrap().to_string();

    let first_job = wait_for_import_job(&app, &first_job_id).await;
    assert_eq!(first_job["status"], "completed");
    assert_eq!(first_job["imported_messages"], 1);

    let (status, second) = json_request(&app, "POST", "/api/v1/import/chat-history", payload).await;
    assert_eq!(status, StatusCode::OK);
    let second_job_id = second["job_id"].as_str().unwrap().to_string();
    assert_eq!(first_job_id, second_job_id);

    let (status, user) = json_request(
        &app,
        "GET",
        "/api/v1/users/external/import-user-idempotent",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let user_id = user["id"].as_str().unwrap();

    let (status, sessions) = json_request(
        &app,
        "GET",
        &format!("/api/v1/users/{user_id}/sessions?limit=20"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let session_id = sessions["data"][0]["id"].as_str().unwrap();

    let (status, episodes) = json_request(
        &app,
        "GET",
        &format!("/api/v1/sessions/{session_id}/episodes?limit=50"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        episodes["count"], 1,
        "idempotent replay must not duplicate episodes"
    );
}

#[tokio::test]
async fn test_chat_history_import_chatgpt_export_pathway() {
    let app = build_test_app().await;

    let (status, import_resp) = json_request(
        &app,
        "POST",
        "/api/v1/import/chat-history",
        serde_json::json!({
            "user": "import-user-chatgpt",
            "source": "chatgpt_export",
            "payload": {
                "title": "Lab Notebook",
                "mapping": {
                    "m1": {
                        "message": {
                            "author": {"role": "user"},
                            "create_time": 1735689600,
                            "content": {"parts": ["first exported message"]}
                        }
                    },
                    "m2": {
                        "message": {
                            "author": {"role": "assistant"},
                            "create_time": 1735689610,
                            "content": {"parts": ["assistant exported reply"]}
                        }
                    }
                }
            }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);

    let job = wait_for_import_job(&app, import_resp["job_id"].as_str().unwrap()).await;
    assert_eq!(job["status"], "completed");
    assert_eq!(job["imported_messages"], 2);

    let (status, user) = json_request(
        &app,
        "GET",
        "/api/v1/users/external/import-user-chatgpt",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let user_id = user["id"].as_str().unwrap();

    let (status, context) = json_request(
        &app,
        "POST",
        &format!("/api/v1/memory/{user_id}/context"),
        serde_json::json!({"query": "what did we import?", "session": "Lab Notebook"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let text = context["context"].as_str().unwrap_or_default();
    assert!(
        text.contains("first exported message") || text.contains("assistant exported reply"),
        "expected chatgpt import content in context, got: {text}"
    );
}

#[tokio::test]
async fn test_chat_history_import_gemini_export_pathway() {
    let app = build_test_app().await;

    let (status, import_resp) = json_request(
        &app,
        "POST",
        "/api/v1/import/chat-history",
        serde_json::json!({
            "user": "import-user-gemini",
            "source": "gemini_export",
            "payload": {
                "chunkedPrompt": {
                    "chunks": [
                        {"text": "hello from gemini user", "role": "user"},
                        {"text": "internal thought should be skipped", "role": "model", "isThought": true},
                        {"text": "hello from gemini model", "role": "model"}
                    ]
                }
            },
            "default_session": "Gemini Import"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);

    let job = wait_for_import_job(&app, import_resp["job_id"].as_str().unwrap()).await;
    assert_eq!(job["status"], "completed");
    assert_eq!(job["imported_messages"], 2);
    assert_eq!(job["failed_messages"], 0);

    let (status, user) = json_request(
        &app,
        "GET",
        "/api/v1/users/external/import-user-gemini",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let user_id = user["id"].as_str().unwrap();

    let (status, episodes) = json_request(
        &app,
        "GET",
        &format!("/api/v1/users/{user_id}/sessions?limit=20"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let session_id = episodes["data"][0]["id"].as_str().unwrap();

    let (status, rows) = json_request(
        &app,
        "GET",
        &format!("/api/v1/sessions/{session_id}/episodes?limit=20"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(rows["count"], 2);
    let rendered = rows["data"].as_array().unwrap();
    let contents: Vec<&str> = rendered
        .iter()
        .filter_map(|r| r["content"].as_str())
        .collect();
    assert!(contents
        .iter()
        .any(|c| c.contains("hello from gemini user")));
    assert!(contents
        .iter()
        .any(|c| c.contains("hello from gemini model")));
    assert!(contents
        .iter()
        .all(|c| !c.contains("internal thought should be skipped")));
}

#[tokio::test]
async fn test_chat_history_import_dry_run_writes_no_data() {
    let app = build_test_app().await;

    let (status, import_resp) = json_request(
        &app,
        "POST",
        "/api/v1/import/chat-history",
        serde_json::json!({
            "user": "import-user-dry-run",
            "source": "ndjson",
            "dry_run": true,
            "payload": [
                {
                    "role": "user",
                    "content": "dry run row",
                    "created_at": "2025-01-01T00:00:00Z"
                }
            ]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);

    let job = wait_for_import_job(&app, import_resp["job_id"].as_str().unwrap()).await;
    assert_eq!(job["status"], "completed");
    assert_eq!(job["total_messages"], 1);
    assert_eq!(job["imported_messages"], 1);
    assert_eq!(job["failed_messages"], 0);

    let (status, body) = json_request(
        &app,
        "GET",
        "/api/v1/users/external/import-user-dry-run",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "not_found");
}

#[tokio::test]
async fn test_scientific_current_context_includes_episode_provenance() {
    let app = build_test_app().await;

    let (status, user) = json_request(
        &app,
        "POST",
        "/api/v1/users",
        serde_json::json!({
            "name": "science-provenance-user",
            "external_id": "science-provenance-user",
            "metadata": {}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let user_id = user["id"].as_str().unwrap().to_string();

    let (status, session) = json_request(
        &app,
        "POST",
        "/api/v1/sessions",
        serde_json::json!({"user_id": user_id, "name": "research-log"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let session_id = session["id"].as_str().unwrap().to_string();

    let (status, _) = json_request(
        &app,
        "POST",
        &format!("/api/v1/sessions/{session_id}/episodes"),
        serde_json::json!({
            "type": "message",
            "role": "user",
            "content": "Legacy model: regenerative patterning is gene-expression only.",
            "created_at": "2021-03-11T09:00:00Z"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, _) = json_request(
        &app,
        "POST",
        &format!("/api/v1/sessions/{session_id}/episodes"),
        serde_json::json!({
            "type": "message",
            "role": "user",
            "content": "Current model: regenerative patterning uses stable bioelectric prepatterns.",
            "created_at": "2025-08-19T10:15:00Z"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, context) = json_request(
        &app,
        "POST",
        "/api/v1/memory/science-provenance-user/context",
        serde_json::json!({
            "query": "What is our current model of regenerative patterning?",
            "time_intent": "current",
            "mode": "head",
            "max_tokens": 700
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let episodes = context["episodes"].as_array().unwrap();
    assert!(
        !episodes.is_empty(),
        "expected episode provenance in response"
    );
    assert!(episodes.iter().all(|e| e["id"].is_string()));
    assert!(episodes.iter().all(|e| e["session_id"].is_string()));
    assert!(episodes.iter().all(|e| e["created_at"].is_string()));
    assert!(
        episodes.iter().any(|e| {
            e["preview"]
                .as_str()
                .unwrap_or_default()
                .contains("stable bioelectric prepatterns")
        }),
        "expected provenance to cite current scientific claim"
    );
}

#[tokio::test]
async fn test_scientific_historical_context_cites_historical_episode() {
    let app = build_test_app().await;

    let (status, user) = json_request(
        &app,
        "POST",
        "/api/v1/users",
        serde_json::json!({
            "name": "science-historical-user",
            "external_id": "science-historical-user",
            "metadata": {}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let user_id = user["id"].as_str().unwrap().to_string();

    let (status, session) = json_request(
        &app,
        "POST",
        "/api/v1/sessions",
        serde_json::json!({"user_id": user_id, "name": "research-log"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let session_id = session["id"].as_str().unwrap().to_string();

    let (status, _) = json_request(
        &app,
        "POST",
        &format!("/api/v1/sessions/{session_id}/episodes"),
        serde_json::json!({
            "type": "message",
            "role": "user",
            "content": "Old framing: target morphology is a static endpoint snapshot only.",
            "created_at": "2022-05-09T13:20:00Z"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, _) = json_request(
        &app,
        "POST",
        &format!("/api/v1/sessions/{session_id}/episodes"),
        serde_json::json!({
            "type": "message",
            "role": "user",
            "content": "Updated framing: target morphology behaves as an attractor-like setpoint in morphospace.",
            "created_at": "2025-01-21T15:45:00Z"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, context) = json_request(
        &app,
        "POST",
        "/api/v1/memory/science-historical-user/context",
        serde_json::json!({
            "query": "What was our framing in 2022?",
            "mode": "historical",
            "time_intent": "historical",
            "as_of": "2022-09-01T00:00:00Z",
            "max_tokens": 700
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let episodes = context["episodes"].as_array().unwrap();
    assert!(
        !episodes.is_empty(),
        "expected historical provenance episodes"
    );
    assert!(
        episodes.iter().any(|e| {
            e["preview"]
                .as_str()
                .unwrap_or_default()
                .contains("static endpoint snapshot")
        }),
        "expected historical provenance to include the historical claim"
    );
}

#[tokio::test]
async fn test_memory_api_head_mode_returns_thread_head_diagnostics() {
    let app = build_test_app().await;

    let (status, user) = json_request(
        &app,
        "POST",
        "/api/v1/users",
        serde_json::json!({
            "name": "head-mode-user",
            "external_id": "head-mode-user",
            "metadata": {}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let user_id = user["id"].as_str().unwrap().to_string();

    let (status, session_a) = json_request(
        &app,
        "POST",
        "/api/v1/sessions",
        serde_json::json!({ "user_id": user_id, "name": "a" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let session_a_id = session_a["id"].as_str().unwrap().to_string();

    let (status, session_b) = json_request(
        &app,
        "POST",
        "/api/v1/sessions",
        serde_json::json!({ "user_id": user_id, "name": "b" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let session_b_id = session_b["id"].as_str().unwrap().to_string();

    let (status, _) = json_request(
        &app,
        "POST",
        &format!("/api/v1/sessions/{session_a_id}/episodes"),
        serde_json::json!({
            "type": "message",
            "role": "user",
            "content": "old-session-marker",
            "created_at": "2024-01-10T12:00:00Z"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, _) = json_request(
        &app,
        "POST",
        &format!("/api/v1/sessions/{session_b_id}/episodes"),
        serde_json::json!({
            "type": "message",
            "role": "user",
            "content": "new-session-marker",
            "created_at": "2026-03-01T12:00:00Z"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, context) = json_request(
        &app,
        "POST",
        "/api/v1/memory/head-mode-user/context",
        serde_json::json!({
            "query": "what is current?",
            "mode": "head",
            "max_tokens": 600
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    assert_eq!(context["mode"], "head");
    assert_eq!(context["head"]["session_id"], session_b_id);
    assert!(context["head"]["episode_id"].is_string());
    assert_eq!(context["head"]["version"], 1);
    assert!(
        context["context"]
            .as_str()
            .unwrap_or_default()
            .contains("new-session-marker"),
        "expected head mode context to include latest session marker"
    );
}

#[tokio::test]
async fn test_memory_api_head_mode_with_explicit_session_override() {
    let app = build_test_app().await;

    let (status, user) = json_request(
        &app,
        "POST",
        "/api/v1/users",
        serde_json::json!({
            "name": "head-override-user",
            "external_id": "head-override-user",
            "metadata": {}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let user_id = user["id"].as_str().unwrap().to_string();

    let (status, session_a) = json_request(
        &app,
        "POST",
        "/api/v1/sessions",
        serde_json::json!({ "user_id": user_id, "name": "a" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let session_a_id = session_a["id"].as_str().unwrap().to_string();

    let (status, session_b) = json_request(
        &app,
        "POST",
        "/api/v1/sessions",
        serde_json::json!({ "user_id": user_id, "name": "b" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let session_b_id = session_b["id"].as_str().unwrap().to_string();

    let (status, _) = json_request(
        &app,
        "POST",
        &format!("/api/v1/sessions/{session_a_id}/episodes"),
        serde_json::json!({
            "type": "message",
            "role": "user",
            "content": "override-marker-a",
            "created_at": "2024-01-10T12:00:00Z"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, _) = json_request(
        &app,
        "POST",
        &format!("/api/v1/sessions/{session_b_id}/episodes"),
        serde_json::json!({
            "type": "message",
            "role": "user",
            "content": "latest-marker-b",
            "created_at": "2026-03-01T12:00:00Z"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, context) = json_request(
        &app,
        "POST",
        "/api/v1/memory/head-override-user/context",
        serde_json::json!({
            "query": "what should head use?",
            "mode": "head",
            "session": "a",
            "max_tokens": 600
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    assert_eq!(context["mode"], "head");
    assert_eq!(context["head"]["session_id"], session_a_id);
    assert!(context["context"]
        .as_str()
        .unwrap_or_default()
        .contains("override-marker-a"));
}

#[tokio::test]
async fn test_memory_api_head_mode_without_sessions_returns_empty_head() {
    let app = build_test_app().await;

    let (status, _) = json_request(
        &app,
        "POST",
        "/api/v1/users",
        serde_json::json!({
            "name": "head-empty-user",
            "external_id": "head-empty-user",
            "metadata": {}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, context) = json_request(
        &app,
        "POST",
        "/api/v1/memory/head-empty-user/context",
        serde_json::json!({
            "query": "what is current?",
            "mode": "head",
            "max_tokens": 300
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(context["mode"], "head");
    assert!(context.get("head").is_none() || context["head"].is_null());
}

#[tokio::test]
async fn test_memory_api_temporal_intent_changes_rank_order() {
    let app = build_test_app().await;

    let (status, user) = json_request(
        &app,
        "POST",
        "/api/v1/users",
        serde_json::json!({
            "name": "temporal-intent-user",
            "external_id": "temporal-intent-user",
            "metadata": {}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let user_id = user["id"].as_str().unwrap().to_string();

    let (status, session) = json_request(
        &app,
        "POST",
        "/api/v1/sessions",
        serde_json::json!({ "user_id": user_id, "name": "default" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let session_id = session["id"].as_str().unwrap().to_string();

    let (status, _) = json_request(
        &app,
        "POST",
        &format!("/api/v1/sessions/{session_id}/episodes"),
        serde_json::json!({
            "type": "message",
            "role": "user",
            "content": "I prefer Adidas running shoes.",
            "created_at": "2024-01-10T12:00:00Z"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, _) = json_request(
        &app,
        "POST",
        &format!("/api/v1/sessions/{session_id}/episodes"),
        serde_json::json!({
            "type": "message",
            "role": "user",
            "content": "I switched and now prefer Nike running shoes.",
            "created_at": "2026-03-01T12:00:00Z"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, current) = json_request(
        &app,
        "POST",
        "/api/v1/memory/temporal-intent-user/context",
        serde_json::json!({
            "query": "What shoes do I prefer now?",
            "session": "default",
            "time_intent": "current",
            "temporal_weight": 0.9,
            "max_tokens": 600
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, historical) = json_request(
        &app,
        "POST",
        "/api/v1/memory/temporal-intent-user/context",
        serde_json::json!({
            "query": "What shoes did I prefer as of 2024?",
            "session": "default",
            "time_intent": "historical",
            "as_of": "2024-06-01T00:00:00Z",
            "temporal_weight": 0.9,
            "max_tokens": 600
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let current_context = current["context"].as_str().unwrap_or_default();
    let historical_context = historical["context"].as_str().unwrap_or_default();
    let current_top = current_context
        .lines()
        .find(|l| l.starts_with("- ["))
        .unwrap_or_default();
    let historical_top = historical_context
        .lines()
        .find(|l| l.starts_with("- ["))
        .unwrap_or_default();

    assert!(
        current_top.contains("Nike"),
        "expected current intent to rank Nike first, got: {current_top}"
    );
    assert!(
        historical_top.contains("Adidas"),
        "expected historical intent to rank Adidas first, got: {historical_top}"
    );

    assert_eq!(
        current["temporal_diagnostics"]["resolved_intent"],
        "current"
    );
    assert_eq!(
        historical["temporal_diagnostics"]["resolved_intent"],
        "historical"
    );
}

#[tokio::test]
async fn test_memory_api_metadata_filters_and_diagnostics() {
    let app = build_test_app().await;

    let (status, user) = json_request(
        &app,
        "POST",
        "/api/v1/users",
        serde_json::json!({
            "name": "metadata-filter-user",
            "external_id": "metadata-filter-user",
            "metadata": {}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let user_id = user["id"].as_str().unwrap().to_string();

    let (status, session) = json_request(
        &app,
        "POST",
        "/api/v1/sessions",
        serde_json::json!({ "user_id": user_id, "name": "default" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let session_id = session["id"].as_str().unwrap().to_string();

    let (status, _) = json_request(
        &app,
        "POST",
        &format!("/api/v1/sessions/{session_id}/episodes"),
        serde_json::json!({
            "type": "message",
            "role": "user",
            "content": "Priority outage in payments pipeline",
            "metadata": {"tags": ["priority", "incident"]},
            "created_at": "2026-03-01T12:00:00Z"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, _) = json_request(
        &app,
        "POST",
        &format!("/api/v1/sessions/{session_id}/episodes"),
        serde_json::json!({
            "type": "message",
            "role": "assistant",
            "content": "Normal weekly standup update",
            "metadata": {"tags": ["routine"]},
            "created_at": "2026-03-01T13:00:00Z"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, context) = json_request(
        &app,
        "POST",
        "/api/v1/memory/metadata-filter-user/context",
        serde_json::json!({
            "query": "What priority incidents did we discuss?",
            "session": "default",
            "filters": {
                "roles": ["user"],
                "tags_all": ["priority"],
                "created_after": "2026-03-01T00:00:00Z"
            },
            "max_tokens": 600
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    assert!(context["metadata_filter_diagnostics"].is_object());
    assert_eq!(
        context["metadata_filter_diagnostics"]["candidate_count_before_filters"],
        2
    );
    assert_eq!(
        context["metadata_filter_diagnostics"]["candidate_count_after_filters"],
        1
    );
    assert_eq!(
        context["metadata_filter_diagnostics"]["prefilter_enabled"],
        true
    );
    assert!(context["metadata_filter_diagnostics"]["planner_latency_ms"].is_number());

    let episodes = context["episodes"].as_array().cloned().unwrap_or_default();
    if !episodes.is_empty() {
        let top_preview = episodes[0]["preview"].as_str().unwrap_or_default();
        assert!(top_preview.contains("Priority outage"));
    }
}

#[tokio::test]
async fn test_memory_api_metadata_prefilter_disabled_diagnostics() {
    let app = build_test_app_with_prefilter(MetadataPrefilterConfig {
        enabled: false,
        scan_limit: 200,
        relax_if_empty: false,
    })
    .await;

    let (status, _) = json_request(
        &app,
        "POST",
        "/api/v1/memory",
        serde_json::json!({
            "user": "prefilter-disabled-user",
            "text": "Priority incident happened today"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, context) = json_request(
        &app,
        "POST",
        "/api/v1/memory/prefilter-disabled-user/context",
        serde_json::json!({
            "query": "What incidents happened?",
            "filters": {"tags_any": ["priority"]},
            "max_tokens": 600
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        context["metadata_filter_diagnostics"]["prefilter_enabled"],
        false
    );
}

#[tokio::test]
async fn test_memory_api_metadata_prefilter_relax_if_empty() {
    let app = build_test_app_with_prefilter(MetadataPrefilterConfig {
        enabled: true,
        scan_limit: 400,
        relax_if_empty: true,
    })
    .await;

    let (status, _) = json_request(
        &app,
        "POST",
        "/api/v1/memory",
        serde_json::json!({
            "user": "prefilter-relax-user",
            "text": "Priority incident happened today",
            "session": "default"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, context) = json_request(
        &app,
        "POST",
        "/api/v1/memory/prefilter-relax-user/context",
        serde_json::json!({
            "query": "What happened?",
            "session": "default",
            "filters": {"tags_all": ["does-not-exist"]},
            "max_tokens": 600
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        context["metadata_filter_diagnostics"]["relaxed_fallback_applied"],
        true
    );
    assert_eq!(
        context["metadata_filter_diagnostics"]["candidate_count_after_filters"],
        1
    );
}

#[tokio::test]
async fn test_memory_api_metadata_scan_limit_applies() {
    let app = build_test_app_with_prefilter(MetadataPrefilterConfig {
        enabled: true,
        scan_limit: 1,
        relax_if_empty: false,
    })
    .await;

    let (status, _) = json_request(
        &app,
        "POST",
        "/api/v1/memory",
        serde_json::json!({
            "user": "prefilter-scan-limit-user",
            "text": "Episode one",
            "session": "default"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, _) = json_request(
        &app,
        "POST",
        "/api/v1/memory",
        serde_json::json!({
            "user": "prefilter-scan-limit-user",
            "text": "Episode two",
            "session": "default"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, context) = json_request(
        &app,
        "POST",
        "/api/v1/memory/prefilter-scan-limit-user/context",
        serde_json::json!({
            "query": "What happened?",
            "session": "default",
            "filters": {"roles": ["user"]},
            "max_tokens": 600
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let scanned = context["metadata_filter_diagnostics"]["candidate_count_before_filters"]
        .as_u64()
        .unwrap_or(0);
    assert!(
        scanned <= 1,
        "expected scan limit to cap candidates, got {scanned}"
    );
}

#[tokio::test]
async fn test_agent_identity_substrate_endpoints_prototype() {
    let app = build_test_app().await;

    let (status, user) = json_request(
        &app,
        "POST",
        "/api/v1/users",
        serde_json::json!({
            "name": "identity-user",
            "external_id": "identity-user",
            "metadata": {}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let user_id = user["id"].as_str().unwrap().to_string();

    let (status, session) = json_request(
        &app,
        "POST",
        "/api/v1/sessions",
        serde_json::json!({ "user_id": user_id, "name": "default" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let session_id = session["id"].as_str().unwrap().to_string();

    let (status, identity) = json_request(
        &app,
        "GET",
        "/api/v1/agents/support-agent/identity",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(identity["agent_id"], "support-agent");

    let (status, identity) = json_request(
        &app,
        "PUT",
        "/api/v1/agents/support-agent/identity",
        serde_json::json!({
            "core": {
                "mission": "Resolve user issues accurately.",
                "boundaries": {"never_claim_human_experience": true}
            }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(identity["version"], 2);

    let (status, _) = json_request(
        &app,
        "POST",
        "/api/v1/agents/support-agent/experience",
        serde_json::json!({
            "user_id": user_id,
            "session_id": session_id,
            "category": "interaction_pattern",
            "signal": "user_prefers_bulleted_action_plans",
            "confidence": 0.8,
            "weight": 0.7,
            "decay_half_life_days": 30
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, _) = json_request(
        &app,
        "POST",
        &format!("/api/v1/sessions/{session_id}/episodes"),
        serde_json::json!({
            "type": "message",
            "role": "user",
            "content": "I prefer concise checklists.",
            "created_at": "2026-03-02T12:00:00Z"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, context) = json_request(
        &app,
        "POST",
        "/api/v1/agents/support-agent/context",
        serde_json::json!({
            "user": "identity-user",
            "query": "How should I respond to this user?",
            "session": "default",
            "mode": "head"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(context["identity_version"], 2);
    assert_eq!(context["experience_events_used"], 1);
    assert_eq!(
        context["attribution_guards"]["self_user_separation_enforced"],
        true
    );
    assert!(context["context"]
        .as_str()
        .unwrap_or_default()
        .contains("Agent Identity Core"));
}

#[tokio::test]
async fn test_agent_identity_rejects_user_memory_contamination_keys() {
    let app = build_test_app().await;

    let (status, body) = json_request(
        &app,
        "PUT",
        "/api/v1/agents/guard-test/identity",
        serde_json::json!({
            "core": {
                "mission": "be useful",
                "user_fact": "I am a doctor"
            }
        }),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "validation_error");
}

#[tokio::test]
async fn test_agent_identity_versions_audit_and_rollback() {
    let app = build_test_app().await;

    let (status, identity_v1) = json_request(
        &app,
        "GET",
        "/api/v1/agents/rollback-test/identity",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(identity_v1["version"], 1);

    let (status, identity_v2) = json_request(
        &app,
        "PUT",
        "/api/v1/agents/rollback-test/identity",
        serde_json::json!({
            "core": {"mission": "version-2"}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(identity_v2["version"], 2);

    let (status, identity_v3) = json_request(
        &app,
        "PUT",
        "/api/v1/agents/rollback-test/identity",
        serde_json::json!({
            "core": {"mission": "version-3"}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(identity_v3["version"], 3);

    let (status, versions) = json_request(
        &app,
        "GET",
        "/api/v1/agents/rollback-test/identity/versions?limit=10",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let version_count = versions.as_array().map(|a| a.len()).unwrap_or(0);
    assert!(version_count >= 3);

    let (status, rolled) = json_request(
        &app,
        "POST",
        "/api/v1/agents/rollback-test/identity/rollback",
        serde_json::json!({
            "target_version": 2,
            "reason": "revert to stable"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(rolled["version"], 4);
    assert_eq!(rolled["core"]["mission"], "version-2");

    let (status, audit) = json_request(
        &app,
        "GET",
        "/api/v1/agents/rollback-test/identity/audit?limit=20",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let events = audit.as_array().cloned().unwrap_or_default();
    assert!(!events.is_empty());
    assert!(events.iter().any(|e| e["action"] == "rolled_back"));
}

#[tokio::test]
async fn test_agent_promotion_gating_and_approval_flow() {
    let app = build_test_app().await;

    let (status, identity) = json_request(
        &app,
        "GET",
        "/api/v1/agents/promo-agent/identity",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(identity["version"], 1);

    // insufficient evidence should be rejected
    let (status, body) = json_request(
        &app,
        "POST",
        "/api/v1/agents/promo-agent/promotions",
        serde_json::json!({
            "proposal": "increase directness",
            "candidate_core": {"mission": "new-mission"},
            "reason": "single anecdote",
            "source_event_ids": [uuid::Uuid::now_v7()]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "validation_error");

    let source_ids = vec![
        uuid::Uuid::now_v7(),
        uuid::Uuid::now_v7(),
        uuid::Uuid::now_v7(),
    ];

    let (status, proposal) = json_request(
        &app,
        "POST",
        "/api/v1/agents/promo-agent/promotions",
        serde_json::json!({
            "proposal": "increase directness",
            "candidate_core": {"mission": "new-mission"},
            "reason": "repeated positive outcomes",
            "source_event_ids": source_ids
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(proposal["status"], "pending");
    let proposal_id = proposal["id"].as_str().unwrap().to_string();

    let (status, approved) = json_request(
        &app,
        "POST",
        &format!("/api/v1/agents/promo-agent/promotions/{proposal_id}/approve"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(approved["status"], "approved");

    let (status, identity_after) = json_request(
        &app,
        "GET",
        "/api/v1/agents/promo-agent/identity",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(identity_after["core"]["mission"], "new-mission");
    assert_eq!(identity_after["version"], 2);
}

#[tokio::test]
async fn test_agent_identity_drift_resistance_blocks_repeated_adversarial_mutations() {
    let app = build_test_app().await;

    let (status, _) = json_request(
        &app,
        "GET",
        "/api/v1/agents/drift-agent/identity",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    for _ in 0..20 {
        let (status, body) = json_request(
            &app,
            "PUT",
            "/api/v1/agents/drift-agent/identity",
            serde_json::json!({
                "core": {
                    "mission": "safe",
                    "user_profile": "I am definitely a doctor"
                }
            }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "unexpected body: {body:?}");
    }

    let (status, identity_after) = json_request(
        &app,
        "GET",
        "/api/v1/agents/drift-agent/identity",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(identity_after["version"], 1);
}

#[tokio::test]
async fn test_memory_webhooks_capture_head_advanced_event_after_remember() {
    let (sink_url, _, deliveries) = start_webhook_sink_server(0).await;
    let (app, _) = build_test_harness_with_prefilter_and_webhooks(
        MetadataPrefilterConfig {
            enabled: true,
            scan_limit: 400,
            relax_if_empty: false,
        },
        WebhookDeliveryConfig {
            enabled: true,
            max_attempts: 3,
            base_backoff_ms: 10,
            request_timeout_ms: 500,
            max_events_per_webhook: 1000,
            rate_limit_per_minute: 120,
            circuit_breaker_threshold: 5,
            circuit_breaker_cooldown_ms: 200,
            persistence_enabled: false,
        },
    )
    .await;
    let signing_secret = "whsec_head_test";

    let (status, _) = json_request(
        &app,
        "POST",
        "/api/v1/memory",
        serde_json::json!({
            "user": "webhook-head-user",
            "session": "default",
            "text": "Seed memory for webhook capture"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, registered) = json_request(
        &app,
        "POST",
        "/api/v1/memory/webhooks",
        serde_json::json!({
            "user": "webhook-head-user",
            "target_url": sink_url,
            "signing_secret": signing_secret,
            "events": ["head_advanced"]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let webhook_id = registered["webhook"]["id"].as_str().unwrap().to_string();

    let req_id = "trace-head-req-001";
    let (status, _) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/memory",
        REQUEST_ID_HEADER,
        req_id,
        serde_json::json!({
            "user": "webhook-head-user",
            "session": "default",
            "text": "This should advance head"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let row = wait_for_webhook_delivery(&app, &webhook_id, true).await;
    assert_eq!(row["event_type"], "head_advanced");
    assert!(row["delivered"].as_bool().unwrap_or(false));
    assert!(row["attempts"].as_u64().unwrap_or(0) >= 1);
    assert_eq!(row["request_id"], req_id);

    let captured = {
        let rows = deliveries.lock().await;
        rows.last().cloned()
    };
    let (headers, body) = captured.expect("expected webhook sink capture");
    let timestamp = headers
        .get("x-mnemo-timestamp")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let signature = headers
        .get("x-mnemo-signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let delivered_req_id = headers
        .get(REQUEST_ID_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert_eq!(delivered_req_id, req_id);
    let expected = compute_expected_signature(signing_secret, &timestamp, &body);
    assert_eq!(signature, expected);
}

#[tokio::test]
async fn test_memory_webhooks_capture_conflict_detected_event() {
    let (sink_url, _, _) = start_webhook_sink_server(0).await;
    let (app, state_store) = build_test_harness_with_prefilter_and_webhooks(
        MetadataPrefilterConfig {
            enabled: true,
            scan_limit: 400,
            relax_if_empty: false,
        },
        WebhookDeliveryConfig {
            enabled: true,
            max_attempts: 3,
            base_backoff_ms: 10,
            request_timeout_ms: 500,
            max_events_per_webhook: 1000,
            rate_limit_per_minute: 120,
            circuit_breaker_threshold: 5,
            circuit_breaker_cooldown_ms: 200,
            persistence_enabled: false,
        },
    )
    .await;

    let (status, user) = json_request(
        &app,
        "POST",
        "/api/v1/users",
        serde_json::json!({
            "name": "webhook-conflict-user",
            "external_id": "webhook-conflict-user",
            "metadata": {}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let user_id = Uuid::parse_str(user["id"].as_str().unwrap()).unwrap();

    let (status, registered) = json_request(
        &app,
        "POST",
        "/api/v1/memory/webhooks",
        serde_json::json!({
            "user": "webhook-conflict-user",
            "target_url": sink_url,
            "events": ["conflict_detected"]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let webhook_id = registered["webhook"]["id"].as_str().unwrap().to_string();

    let episode_id = Uuid::now_v7();
    let src = state_store
        .create_entity(Entity::from_extraction(
            &ExtractedEntity {
                name: "Kendra".to_string(),
                entity_type: EntityType::Person,
                summary: None,
            },
            user_id,
            episode_id,
        ))
        .await
        .unwrap();
    let adidas = state_store
        .create_entity(Entity::from_extraction(
            &ExtractedEntity {
                name: "Adidas".to_string(),
                entity_type: EntityType::Organization,
                summary: None,
            },
            user_id,
            episode_id,
        ))
        .await
        .unwrap();
    let nike = state_store
        .create_entity(Entity::from_extraction(
            &ExtractedEntity {
                name: "Nike".to_string(),
                entity_type: EntityType::Organization,
                summary: None,
            },
            user_id,
            episode_id,
        ))
        .await
        .unwrap();

    let now = Utc::now();
    state_store
        .create_edge(Edge::from_extraction(
            &ExtractedRelationship {
                source_name: "Kendra".to_string(),
                target_name: "Adidas".to_string(),
                label: "prefers".to_string(),
                fact: "Kendra prefers Adidas".to_string(),
                confidence: 0.8,
                valid_at: Some(now - chrono::Duration::days(2)),
            },
            user_id,
            src.id,
            adidas.id,
            episode_id,
            now,
        ))
        .await
        .unwrap();
    state_store
        .create_edge(Edge::from_extraction(
            &ExtractedRelationship {
                source_name: "Kendra".to_string(),
                target_name: "Nike".to_string(),
                label: "prefers".to_string(),
                fact: "Kendra prefers Nike".to_string(),
                confidence: 0.82,
                valid_at: Some(now - chrono::Duration::days(1)),
            },
            user_id,
            src.id,
            nike.id,
            episode_id,
            now,
        ))
        .await
        .unwrap();

    let (status, _) = json_request(
        &app,
        "POST",
        "/api/v1/memory/webhook-conflict-user/conflict_radar",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let row = wait_for_webhook_event(&app, &webhook_id).await;
    assert_eq!(row["event_type"], "conflict_detected");
}

#[tokio::test]
async fn test_memory_webhooks_retry_backoff_eventually_delivers() {
    let (sink_url, attempts, _) = start_webhook_sink_server(2).await;
    let (app, _) = build_test_harness_with_prefilter_and_webhooks(
        MetadataPrefilterConfig {
            enabled: true,
            scan_limit: 400,
            relax_if_empty: false,
        },
        WebhookDeliveryConfig {
            enabled: true,
            max_attempts: 5,
            base_backoff_ms: 10,
            request_timeout_ms: 500,
            max_events_per_webhook: 1000,
            rate_limit_per_minute: 120,
            circuit_breaker_threshold: 5,
            circuit_breaker_cooldown_ms: 200,
            persistence_enabled: false,
        },
    )
    .await;

    let (status, _) = json_request(
        &app,
        "POST",
        "/api/v1/memory",
        serde_json::json!({
            "user": "webhook-retry-user",
            "session": "default",
            "text": "seed"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, registered) = json_request(
        &app,
        "POST",
        "/api/v1/memory/webhooks",
        serde_json::json!({
            "user": "webhook-retry-user",
            "target_url": sink_url,
            "events": ["head_advanced"]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let webhook_id = registered["webhook"]["id"].as_str().unwrap().to_string();

    let (status, _) = json_request(
        &app,
        "POST",
        "/api/v1/memory",
        serde_json::json!({
            "user": "webhook-retry-user",
            "session": "default",
            "text": "trigger retries"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let final_row = wait_for_webhook_delivery(&app, &webhook_id, true).await;
    assert_eq!(final_row["event_type"], "head_advanced");
    assert!(final_row["attempts"].as_u64().unwrap_or(0) >= 3);
    assert!(attempts.load(Ordering::SeqCst) >= 3);
}

#[tokio::test]
async fn test_memory_webhook_dead_letter_and_stats_endpoint() {
    let (sink_url, attempts, _) = start_webhook_sink_server(100).await;
    let (app, _) = build_test_harness_with_prefilter_and_webhooks(
        MetadataPrefilterConfig {
            enabled: true,
            scan_limit: 400,
            relax_if_empty: false,
        },
        WebhookDeliveryConfig {
            enabled: true,
            max_attempts: 3,
            base_backoff_ms: 10,
            request_timeout_ms: 500,
            max_events_per_webhook: 1000,
            rate_limit_per_minute: 120,
            circuit_breaker_threshold: 5,
            circuit_breaker_cooldown_ms: 200,
            persistence_enabled: false,
        },
    )
    .await;

    let (status, _) = json_request(
        &app,
        "POST",
        "/api/v1/memory",
        serde_json::json!({
            "user": "webhook-dead-letter-user",
            "session": "default",
            "text": "seed"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, registered) = json_request(
        &app,
        "POST",
        "/api/v1/memory/webhooks",
        serde_json::json!({
            "user": "webhook-dead-letter-user",
            "target_url": sink_url,
            "events": ["head_advanced"]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let webhook_id = registered["webhook"]["id"].as_str().unwrap().to_string();

    let (status, _) = json_request(
        &app,
        "POST",
        "/api/v1/memory",
        serde_json::json!({
            "user": "webhook-dead-letter-user",
            "session": "default",
            "text": "this should dead-letter"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let dead = wait_for_dead_letter_event(&app, &webhook_id).await;
    assert_eq!(dead["event_type"], "head_advanced");
    assert!(dead["dead_letter"].as_bool().unwrap_or(false));
    assert!(dead["attempts"].as_u64().unwrap_or(0) >= 3);
    assert!(attempts.load(Ordering::SeqCst) >= 3);

    let (status, stats) = json_request(
        &app,
        "GET",
        &format!("/api/v1/memory/webhooks/{webhook_id}/stats"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(stats["total_events"].as_u64().unwrap_or(0) >= 1);
    assert!(stats["dead_letter_events"].as_u64().unwrap_or(0) >= 1);
}

#[tokio::test]
async fn test_memory_webhook_replay_retry_and_audit_endpoints() {
    let (sink_url, _, _) = start_webhook_sink_server(1).await;
    let (app, _) = build_test_harness_with_prefilter_and_webhooks(
        MetadataPrefilterConfig {
            enabled: true,
            scan_limit: 400,
            relax_if_empty: false,
        },
        WebhookDeliveryConfig {
            enabled: true,
            max_attempts: 1,
            base_backoff_ms: 5,
            request_timeout_ms: 500,
            max_events_per_webhook: 1000,
            rate_limit_per_minute: 120,
            circuit_breaker_threshold: 5,
            circuit_breaker_cooldown_ms: 200,
            persistence_enabled: false,
        },
    )
    .await;

    let (status, _) = json_request(
        &app,
        "POST",
        "/api/v1/memory",
        serde_json::json!({
            "user": "webhook-ops-user",
            "session": "default",
            "text": "seed"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, registered) = json_request(
        &app,
        "POST",
        "/api/v1/memory/webhooks",
        serde_json::json!({
            "user": "webhook-ops-user",
            "target_url": sink_url,
            "events": ["head_advanced"]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let webhook_id = registered["webhook"]["id"].as_str().unwrap().to_string();

    let (status, _) = json_request(
        &app,
        "POST",
        "/api/v1/memory",
        serde_json::json!({
            "user": "webhook-ops-user",
            "session": "default",
            "text": "trigger dead-letter first attempt"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let dead = wait_for_dead_letter_event(&app, &webhook_id).await;
    let event_id = dead["id"].as_str().unwrap().to_string();
    let req_id = "trace-ops-req-123";

    let (status, replay) = json_request_with_header(
        &app,
        "GET",
        &format!(
            "/api/v1/memory/webhooks/{webhook_id}/events/replay?limit=1&include_delivered=false&include_dead_letter=true"
        ),
        REQUEST_ID_HEADER,
        req_id,
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(replay["count"], 1);
    assert_eq!(replay["events"][0]["id"], event_id);

    let (status, retried) = json_request_with_header(
        &app,
        "POST",
        &format!("/api/v1/memory/webhooks/{webhook_id}/events/{event_id}/retry"),
        REQUEST_ID_HEADER,
        req_id,
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(retried["queued"], true);
    assert_eq!(retried["event"]["id"], event_id);
    assert_eq!(retried["event"]["dead_letter"], false);

    let delivered = wait_for_webhook_delivery(&app, &webhook_id, true).await;
    assert_eq!(delivered["id"], event_id);
    assert_eq!(delivered["delivered"], true);

    let (status, replay_after) = json_request(
        &app,
        "GET",
        &format!(
            "/api/v1/memory/webhooks/{webhook_id}/events/replay?after_event_id={event_id}&limit=10"
        ),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(replay_after["count"], 0);

    let (status, audit) = json_request(
        &app,
        "GET",
        &format!("/api/v1/memory/webhooks/{webhook_id}/audit?limit=20"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let rows = audit["audit"].as_array().cloned().unwrap_or_default();
    assert!(!rows.is_empty());
    assert!(rows.iter().any(|row| row["action"] == "webhook_registered"));
    assert!(rows.iter().any(|row| row["action"] == "retry_queued"));
    assert!(rows
        .iter()
        .any(|row| row["action"] == "retry_queued" && row["request_id"] == req_id));
    assert!(rows
        .iter()
        .any(|row| row["action"] == "replay_requested" && row["request_id"] == req_id));
}

#[tokio::test]
async fn test_policy_webhook_domain_allowlist_blocks_disallowed_target() {
    let app = build_test_app().await;

    let (status, _) = json_request(
        &app,
        "POST",
        "/api/v1/users",
        serde_json::json!({
            "name": "policy-user",
            "external_id": "policy-user",
            "metadata": {}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let req_id = "policy-allowlist-req-001";
    let (status, _) = json_request_with_header(
        &app,
        "PUT",
        "/api/v1/policies/policy-user",
        REQUEST_ID_HEADER,
        req_id,
        serde_json::json!({
            "webhook_domain_allowlist": ["hooks.acme.example"]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/memory/webhooks",
        REQUEST_ID_HEADER,
        req_id,
        serde_json::json!({
            "user": "policy-user",
            "target_url": "https://evil.example/webhook",
            "events": ["head_advanced"]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, audit) = json_request(
        &app,
        "GET",
        "/api/v1/policies/policy-user/audit?limit=20",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let rows = audit["audit"].as_array().cloned().unwrap_or_default();
    assert!(rows.iter().any(|row| row["action"] == "policy_updated"));
    assert!(rows.iter().any(|row| {
        row["action"] == "policy_violation_webhook_domain" && row["request_id"] == req_id
    }));
}

#[tokio::test]
async fn test_policy_violations_endpoint_filters_by_window() {
    let app = build_test_app().await;

    let (status, _) = json_request(
        &app,
        "POST",
        "/api/v1/users",
        serde_json::json!({
            "name": "policy-window-user",
            "external_id": "policy-window-user",
            "metadata": {}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, _) = json_request(
        &app,
        "PUT",
        "/api/v1/policies/policy-window-user",
        serde_json::json!({
            "webhook_domain_allowlist": ["hooks.acme.example"]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = json_request(
        &app,
        "POST",
        "/api/v1/memory/webhooks",
        serde_json::json!({
            "user": "policy-window-user",
            "target_url": "https://bad.example/webhook",
            "events": ["head_advanced"]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, body) = json_request(
        &app,
        "GET",
        "/api/v1/policies/policy-window-user/violations?from=2020-01-01T00:00:00Z&to=2100-01-01T00:00:00Z&limit=20",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["count"].as_u64().unwrap_or(0) >= 1);
    let rows = body["violations"].as_array().cloned().unwrap_or_default();
    assert!(body["violations"].as_array().is_some());
    assert!(rows
        .iter()
        .any(|row| row["action"] == "policy_violation_webhook_domain"));
}

#[tokio::test]
async fn test_policy_violations_endpoint_rejects_invalid_window() {
    let app = build_test_app().await;

    let (status, _) = json_request(
        &app,
        "GET",
        "/api/v1/policies/nope/violations?from=2026-04-01T00:00:00Z&to=2026-01-01T00:00:00Z",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_policy_audit_records_session_deletion() {
    let app = build_test_app().await;

    let (status, user) = json_request(
        &app,
        "POST",
        "/api/v1/users",
        serde_json::json!({
            "name": "policy-delete-user",
            "external_id": "policy-delete-user",
            "metadata": {}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let user_id = user["id"].as_str().unwrap();

    let (status, session) = json_request(
        &app,
        "POST",
        "/api/v1/sessions",
        serde_json::json!({ "user_id": user_id, "name": "to-delete" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let session_id = session["id"].as_str().unwrap();

    let req_id = "delete-session-req-77";
    let request = Request::builder()
        .method("DELETE")
        .uri(format!("/api/v1/sessions/{session_id}"))
        .header(REQUEST_ID_HEADER, req_id)
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let (status, audit) = json_request(
        &app,
        "GET",
        "/api/v1/policies/policy-delete-user/audit?limit=20",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let rows = audit["audit"].as_array().cloned().unwrap_or_default();
    assert!(rows.iter().any(|row| {
        row["action"] == "session_deleted"
            && row["request_id"] == req_id
            && row["details"]["session_id"] == session_id
    }));
}

#[tokio::test]
async fn test_policy_defaults_apply_to_memory_context_when_request_omits_them() {
    let app = build_test_app().await;

    let (status, _) = json_request(
        &app,
        "POST",
        "/api/v1/memory",
        serde_json::json!({
            "user": "policy-defaults-user",
            "session": "default",
            "text": "Acme renewal is at risk due to procurement constraints"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, _) = json_request(
        &app,
        "PUT",
        "/api/v1/policies/policy-defaults-user",
        serde_json::json!({
            "default_memory_contract": "support_safe",
            "default_retrieval_policy": "precision"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = json_request(
        &app,
        "POST",
        "/api/v1/memory/policy-defaults-user/context",
        serde_json::json!({
            "query": "What is Acme renewal risk?"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["contract_applied"], "support_safe");
    assert_eq!(body["retrieval_policy_applied"], "precision");
}

#[tokio::test]
async fn test_policy_retention_blocks_stale_episode_write() {
    let app = build_test_app().await;

    let (status, user) = json_request(
        &app,
        "POST",
        "/api/v1/users",
        serde_json::json!({
            "name": "policy-retention-user",
            "external_id": "policy-retention-user",
            "metadata": {}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let user_id = user["id"].as_str().unwrap();

    let (status, session) = json_request(
        &app,
        "POST",
        "/api/v1/sessions",
        serde_json::json!({ "user_id": user_id, "name": "retention" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let session_id = session["id"].as_str().unwrap();

    let (status, _) = json_request(
        &app,
        "PUT",
        "/api/v1/policies/policy-retention-user",
        serde_json::json!({
            "retention_days_message": 1
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let old_ts = (Utc::now() - chrono::Duration::days(10)).to_rfc3339();
    let (status, _) = json_request(
        &app,
        "POST",
        &format!("/api/v1/sessions/{session_id}/episodes"),
        serde_json::json!({
            "type": "message",
            "role": "user",
            "content": "This should be rejected by retention policy",
            "created_at": old_ts
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_ops_summary_endpoint_returns_operator_counters() {
    let app = build_test_app().await;

    let (status, _) = json_request(
        &app,
        "GET",
        "/api/v1/ops/summary?window_seconds=600",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) =
        json_request(&app, "GET", "/api/v1/ops/summary", serde_json::json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["http_requests_total"].as_u64().unwrap_or(0) >= 1);
    assert!(body.get("dead_letter_backlog").is_some());
    assert!(body.get("policy_update_total").is_some());
}

#[tokio::test]
async fn test_ops_incidents_endpoint_shapes_action_hrefs_for_drilldowns() {
    let (app, state, _store) = build_test_harness_with_state_and_prefilter_and_webhooks(
        MetadataPrefilterConfig {
            enabled: true,
            scan_limit: 400,
            relax_if_empty: false,
        },
        WebhookDeliveryConfig {
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
    )
    .await;

    let (status, user) = json_request(
        &app,
        "POST",
        "/api/v1/users",
        serde_json::json!({
            "name": "ops-incident-user",
            "external_id": "ops-incident-user",
            "metadata": {}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let user_id = user["id"].as_str().unwrap().to_string();

    let (status, registered) = json_request(
        &app,
        "POST",
        "/api/v1/memory/webhooks",
        serde_json::json!({
            "user": "ops-incident-user",
            "target_url": "https://hooks.acme.example/incident",
            "events": ["head_advanced"]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let webhook_id = registered["webhook"]["id"].as_str().unwrap().to_string();
    let webhook_uuid = Uuid::parse_str(&webhook_id).unwrap();
    let user_uuid = Uuid::parse_str(&user_id).unwrap();
    let now = Utc::now();

    let mut events = Vec::new();
    events.push(MemoryWebhookEventRecord {
        id: Uuid::now_v7(),
        webhook_id: webhook_uuid,
        event_type: mnemo_server::state::MemoryWebhookEventType::HeadAdvanced,
        user_id: user_uuid,
        payload: serde_json::json!({"kind": "dead-letter"}),
        created_at: now,
        attempts: 3,
        delivered: false,
        dead_letter: true,
        request_id: Some("ops-dead-letter-req".to_string()),
        delivered_at: None,
        last_error: Some("timeout".to_string()),
    });
    for _ in 0..25 {
        events.push(MemoryWebhookEventRecord {
            id: Uuid::now_v7(),
            webhook_id: webhook_uuid,
            event_type: mnemo_server::state::MemoryWebhookEventType::HeadAdvanced,
            user_id: user_uuid,
            payload: serde_json::json!({"kind": "pending"}),
            created_at: now,
            attempts: 0,
            delivered: false,
            dead_letter: false,
            request_id: None,
            delivered_at: None,
            last_error: None,
        });
    }
    state
        .memory_webhook_events
        .write()
        .await
        .insert(webhook_uuid, events);
    state.webhook_runtime.write().await.insert(
        webhook_uuid,
        WebhookRuntimeState {
            window_started_at: now,
            sent_in_window: 0,
            consecutive_failures: 5,
            circuit_open_until: Some(now + chrono::Duration::minutes(5)),
        },
    );
    state.governance_audit.write().await.insert(
        user_uuid,
        vec![GovernanceAuditRecord {
            id: Uuid::now_v7(),
            user_id: user_uuid,
            action: "policy_violation_webhook_domain".to_string(),
            request_id: Some("ops-policy-req".to_string()),
            details: serde_json::json!({
                "target_url": "https://blocked.example.com/hook"
            }),
            at: now,
        }],
    );
    state
        .metrics
        .policy_violation_total
        .fetch_add(1, Ordering::Relaxed);

    let (status, body) = json_request(
        &app,
        "GET",
        "/api/v1/ops/incidents?window_seconds=600",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let incidents = body["incidents"].as_array().cloned().unwrap_or_default();
    let dead_letter = incidents
        .iter()
        .find(|row| row["kind"] == "dead_letter_spike")
        .expect("dead-letter incident present");
    assert_eq!(
        dead_letter["action_href"],
        serde_json::json!("/_/webhooks?filter=dead-letter")
    );

    let backlog = incidents
        .iter()
        .find(|row| row["kind"] == "pending_backlog")
        .expect("backlog incident present");
    assert_eq!(
        backlog["action_href"],
        serde_json::json!("/_/webhooks?filter=backlog")
    );

    let circuit = incidents
        .iter()
        .find(|row| row["kind"] == "circuit_open")
        .expect("circuit incident present");
    assert_eq!(
        circuit["action_href"],
        serde_json::json!(format!("/_/webhooks/{webhook_id}"))
    );

    let policy = incidents
        .iter()
        .find(|row| row["kind"] == "policy_violation")
        .expect("policy incident present");
    assert_eq!(
        policy["action_href"],
        serde_json::json!(format!("/_/governance/{user_id}"))
    );
    assert_eq!(policy["request_id"], serde_json::json!("ops-policy-req"));
}

#[tokio::test]
async fn test_trace_lookup_joins_episode_webhook_and_governance_records() {
    let app = build_test_app().await;
    let req_id = "trace-join-req-9001";

    let (status, _) = json_request(
        &app,
        "POST",
        "/api/v1/users",
        serde_json::json!({
            "name": "trace-join-user",
            "external_id": "trace-join-user",
            "metadata": {}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, _) = json_request_with_header(
        &app,
        "PUT",
        "/api/v1/policies/trace-join-user",
        REQUEST_ID_HEADER,
        req_id,
        serde_json::json!({
            "webhook_domain_allowlist": []
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, registered) = json_request(
        &app,
        "POST",
        "/api/v1/memory/webhooks",
        serde_json::json!({
            "user": "trace-join-user",
            "target_url": "https://example.com/hooks/trace",
            "events": ["head_advanced"]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let webhook_id = registered["webhook"]["id"].as_str().unwrap().to_string();

    let (status, _) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/memory",
        REQUEST_ID_HEADER,
        req_id,
        serde_json::json!({
            "user": "trace-join-user",
            "session": "default",
            "text": "trace join payload"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, _) = json_request_with_header(
        &app,
        "GET",
        &format!("/api/v1/memory/webhooks/{webhook_id}/events/replay?limit=10"),
        REQUEST_ID_HEADER,
        req_id,
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, trace) = json_request(
        &app,
        "GET",
        &format!("/api/v1/traces/{req_id}"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(trace["summary"]["episode_matches"].as_u64().unwrap_or(0) >= 1);
    assert!(
        trace["summary"]["webhook_event_matches"]
            .as_u64()
            .unwrap_or(0)
            >= 1
    );
    assert!(
        trace["summary"]["webhook_audit_matches"]
            .as_u64()
            .unwrap_or(0)
            >= 1
    );
    assert!(
        trace["summary"]["governance_audit_matches"]
            .as_u64()
            .unwrap_or(0)
            >= 1
    );
}

#[tokio::test]
async fn test_trace_lookup_supports_source_filters_and_limits() {
    let app = build_test_app().await;
    let req_id = "trace-filter-req-42";

    let (status, _) = json_request(
        &app,
        "POST",
        "/api/v1/users",
        serde_json::json!({
            "name": "trace-filter-user",
            "external_id": "trace-filter-user",
            "metadata": {}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, _) = json_request_with_header(
        &app,
        "PUT",
        "/api/v1/policies/trace-filter-user",
        REQUEST_ID_HEADER,
        req_id,
        serde_json::json!({ "webhook_domain_allowlist": [] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/memory",
        REQUEST_ID_HEADER,
        req_id,
        serde_json::json!({
            "user": "trace-filter-user",
            "session": "default",
            "text": "trace filter payload"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, trace) = json_request(
        &app,
        "GET",
        &format!(
            "/api/v1/traces/{req_id}?include_episodes=false&include_webhook_events=false&limit=1"
        ),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(trace["summary"]["episode_matches"], 0);
    assert_eq!(trace["summary"]["webhook_event_matches"], 0);
    assert!(
        trace["summary"]["governance_audit_matches"]
            .as_u64()
            .unwrap_or(0)
            <= 1
    );
    assert_eq!(trace["matched_episodes"].as_array().unwrap().len(), 0);
    assert_eq!(trace["matched_webhook_events"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn test_trace_lookup_rejects_invalid_window() {
    let app = build_test_app().await;

    let (status, _) = json_request(
        &app,
        "GET",
        "/api/v1/traces/req-123?from=2026-03-05T00:00:00Z&to=2026-03-04T00:00:00Z",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_evidence_export_endpoints_return_request_centric_bundles() {
    let app = build_test_app().await;
    let req_id = "trace-export-req-77";

    let (status, created_user) = json_request(
        &app,
        "POST",
        "/api/v1/users",
        serde_json::json!({
            "name": "trace-export-user",
            "external_id": "trace-export-user",
            "metadata": {}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let user_id = created_user["id"].as_str().unwrap();

    let (status, _) = json_request_with_header(
        &app,
        "PUT",
        "/api/v1/policies/trace-export-user",
        REQUEST_ID_HEADER,
        req_id,
        serde_json::json!({
            "webhook_domain_allowlist": []
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, registered) = json_request(
        &app,
        "POST",
        "/api/v1/memory/webhooks",
        serde_json::json!({
            "user": "trace-export-user",
            "target_url": "https://example.com/hooks/export",
            "events": ["head_advanced"]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let webhook_id = registered["webhook"]["id"].as_str().unwrap().to_string();

    let (status, _) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/memory",
        REQUEST_ID_HEADER,
        req_id,
        serde_json::json!({
            "user": "trace-export-user",
            "session": "default",
            "text": "trace export payload"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, _) = json_request_with_header(
        &app,
        "GET",
        &format!("/api/v1/memory/webhooks/{webhook_id}/events/replay?limit=5"),
        REQUEST_ID_HEADER,
        req_id,
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, webhook_bundle) = get_request(
        &app,
        &format!(
            "/api/v1/evidence/webhooks/{webhook_id}/export?focus=dead-letter&source_path=%2F_%2Fwebhooks%2F{webhook_id}%3Ffocus%3Ddead-letter"
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        webhook_bundle["kind"],
        serde_json::json!("webhook_evidence_bundle")
    );
    assert_eq!(
        webhook_bundle["payload"]["webhook"]["id"],
        serde_json::json!(webhook_id)
    );
    assert_eq!(
        webhook_bundle["payload"]["focus"],
        serde_json::json!("dead-letter")
    );

    let (status, governance_bundle) = get_request(
        &app,
        "/api/v1/evidence/governance/trace-export-user/export?source_path=%2F_%2Fgovernance%2Ftrace-export-user",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        governance_bundle["kind"],
        serde_json::json!("governance_evidence_bundle")
    );
    assert_eq!(
        governance_bundle["payload"]["policy"]["user_identifier"],
        serde_json::json!("trace-export-user")
    );
    assert_eq!(
        governance_bundle["source_path"],
        serde_json::json!("/_/governance/trace-export-user")
    );

    let (status, trace_bundle) = get_request(
        &app,
        &format!(
            "/api/v1/evidence/traces/{req_id}/export?focus=governance&include_episodes=false&source_path=%2F_%2Ftraces%2F{req_id}%3Ffocus%3Dgovernance"
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        trace_bundle["kind"],
        serde_json::json!("trace_evidence_bundle")
    );
    assert_eq!(
        trace_bundle["payload"]["request_id"],
        serde_json::json!(req_id)
    );
    assert_eq!(
        trace_bundle["payload"]["focus"],
        serde_json::json!("governance")
    );
    assert_eq!(
        trace_bundle["payload"]["trace"]["summary"]["episode_matches"],
        serde_json::json!(0)
    );
    assert_eq!(
        trace_bundle["payload"]["trace"]["summary"]["governance_audit_matches"]
            .as_u64()
            .unwrap_or(0),
        1
    );
    assert_eq!(
        governance_bundle["payload"]["policy"]["user_id"],
        serde_json::json!(user_id)
    );
}

// ─── Falsification: Replay cursor pagination under sparse event IDs ───

#[tokio::test]
async fn test_replay_cursor_pagination_with_sparse_event_ids() {
    // Set up a webhook whose sink always fails => all events become dead-letter.
    let (sink_url, _, _) = start_webhook_sink_server(1000).await;
    let (app, _) = build_test_harness_with_prefilter_and_webhooks(
        MetadataPrefilterConfig {
            enabled: true,
            scan_limit: 400,
            relax_if_empty: false,
        },
        WebhookDeliveryConfig {
            enabled: true,
            max_attempts: 1,
            base_backoff_ms: 5,
            request_timeout_ms: 500,
            max_events_per_webhook: 1000,
            rate_limit_per_minute: 120,
            circuit_breaker_threshold: 100,
            circuit_breaker_cooldown_ms: 200,
            persistence_enabled: false,
        },
    )
    .await;

    // Seed user + webhook
    let (status, _) = json_request(
        &app,
        "POST",
        "/api/v1/memory",
        serde_json::json!({
            "user": "replay-cursor-user",
            "session": "default",
            "text": "seed"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, registered) = json_request(
        &app,
        "POST",
        "/api/v1/memory/webhooks",
        serde_json::json!({
            "user": "replay-cursor-user",
            "target_url": sink_url,
            "events": ["head_advanced"]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let webhook_id = registered["webhook"]["id"].as_str().unwrap().to_string();

    // Generate 3 events by posting 3 messages (each triggers head_advanced).
    for i in 1..=3 {
        let (status, _) = json_request(
            &app,
            "POST",
            "/api/v1/memory",
            serde_json::json!({
                "user": "replay-cursor-user",
                "session": "default",
                "text": format!("event trigger {i}")
            }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
    }

    // Wait for all 3 to become dead-letter.
    for _ in 0..120 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let (status, replay_all) = json_request(
            &app,
            "GET",
            &format!("/api/v1/memory/webhooks/{webhook_id}/events/replay?limit=100"),
            serde_json::json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        if replay_all["count"].as_u64().unwrap_or(0) >= 3 {
            break;
        }
    }

    // ── Test 1: Full replay returns all 3, sorted chronologically ──
    let (status, page1) = json_request(
        &app,
        "GET",
        &format!("/api/v1/memory/webhooks/{webhook_id}/events/replay?limit=100"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        page1["count"].as_u64().unwrap() >= 3,
        "Expected at least 3 events, got {}",
        page1["count"]
    );
    let all_events = page1["events"].as_array().unwrap();

    // Verify chronological ordering: created_at is non-decreasing.
    for w in all_events.windows(2) {
        assert!(w[0]["created_at"].as_str().unwrap() <= w[1]["created_at"].as_str().unwrap());
    }

    // ── Test 2: Paginate with limit=1, cursor through all events ──
    let mut collected_ids: Vec<String> = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..10 {
        let url = match &cursor {
            Some(c) => format!(
                "/api/v1/memory/webhooks/{webhook_id}/events/replay?limit=1&after_event_id={c}"
            ),
            None => format!("/api/v1/memory/webhooks/{webhook_id}/events/replay?limit=1"),
        };
        let (status, page) = json_request(&app, "GET", &url, serde_json::json!({})).await;
        assert_eq!(status, StatusCode::OK);
        let count = page["count"].as_u64().unwrap();
        if count == 0 {
            break;
        }
        let event_id = page["events"][0]["id"].as_str().unwrap().to_string();
        collected_ids.push(event_id);
        cursor = page["next_after_event_id"].as_str().map(|s| s.to_string());
    }
    assert!(
        collected_ids.len() >= 3,
        "Cursor pagination should yield at least 3 events, got {}",
        collected_ids.len()
    );

    // No duplicate IDs across pages.
    let unique_ids: std::collections::HashSet<&String> = collected_ids.iter().collect();
    assert_eq!(
        unique_ids.len(),
        collected_ids.len(),
        "Cursor pagination produced duplicate event IDs"
    );

    // ── Test 3: Unknown cursor ID silently resets to beginning ──
    let bogus_id = Uuid::now_v7();
    let (status, from_bogus) = json_request(
        &app,
        "GET",
        &format!(
            "/api/v1/memory/webhooks/{webhook_id}/events/replay?limit=100&after_event_id={bogus_id}"
        ),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    // When cursor is unknown, handler returns events from the beginning.
    assert!(
        from_bogus["count"].as_u64().unwrap() >= 3,
        "Unknown cursor should return all events from beginning"
    );

    // ── Test 4: Filtering — include_dead_letter=false excludes dead-letter events ──
    let (status, no_dead) = json_request(
        &app,
        "GET",
        &format!(
            "/api/v1/memory/webhooks/{webhook_id}/events/replay?limit=100&include_dead_letter=false"
        ),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    // All events are dead-letter in this test, so filtering them out yields 0.
    let no_dead_events = no_dead["events"].as_array().unwrap();
    for evt in no_dead_events {
        assert!(
            !evt["dead_letter"].as_bool().unwrap_or(true),
            "include_dead_letter=false should exclude dead-letter events"
        );
    }

    // ── Test 5: Cursor + filter interaction ──
    // Use first event's ID as cursor, filter out dead-letter.
    let first_id = &collected_ids[0];
    let (status, cursor_plus_filter) = json_request(
        &app,
        "GET",
        &format!(
            "/api/v1/memory/webhooks/{webhook_id}/events/replay?after_event_id={first_id}&limit=100&include_dead_letter=false"
        ),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    // Cursor resolution happens before filtering, so even though the cursor anchor
    // is a dead-letter event, events after it are still filtered.
    for evt in cursor_plus_filter["events"].as_array().unwrap() {
        assert!(!evt["dead_letter"].as_bool().unwrap_or(true));
    }

    // ── Test 6: limit=0 is clamped to 1 (not an error) ──
    let (status, clamped) = json_request(
        &app,
        "GET",
        &format!("/api/v1/memory/webhooks/{webhook_id}/events/replay?limit=0"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        clamped["count"].as_u64().unwrap() <= 1,
        "limit=0 should be clamped to 1"
    );

    // ── Test 7: Replay generates audit record ──
    let (status, audit) = json_request(
        &app,
        "GET",
        &format!("/api/v1/memory/webhooks/{webhook_id}/audit?limit=50"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let rows = audit["audit"].as_array().cloned().unwrap_or_default();
    let replay_audits: Vec<&Value> = rows
        .iter()
        .filter(|r| r["action"] == "replay_requested")
        .collect();
    assert!(
        !replay_audits.is_empty(),
        "Replay should generate audit records"
    );
}

// ─── Falsification: Contract/retrieval policy combination consistency ─────

#[tokio::test]
async fn test_time_travel_trace_contract_retrieval_policy_combinations() {
    let (app, state_store) = build_test_harness_with_prefilter(MetadataPrefilterConfig {
        enabled: true,
        scan_limit: 400,
        relax_if_empty: false,
    })
    .await;

    // Create user, session, episodes, entities, edges — same scenario as
    // the fact-shift trace test (Kendra: Adidas -> Nike preference change).
    let (status, user) = json_request(
        &app,
        "POST",
        "/api/v1/users",
        serde_json::json!({
            "name": "combo-user",
            "external_id": "combo-user",
            "metadata": {}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let user_id = Uuid::parse_str(user["id"].as_str().unwrap()).unwrap();

    let (status, session) = json_request(
        &app,
        "POST",
        "/api/v1/sessions",
        serde_json::json!({ "user_id": user_id, "name": "combo-session" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let session_id = session["id"].as_str().unwrap().to_string();

    let (status, e1) = json_request(
        &app,
        "POST",
        &format!("/api/v1/sessions/{session_id}/episodes"),
        serde_json::json!({
            "type": "message",
            "role": "user",
            "content": "Kendra preferred Adidas before February",
            "created_at": "2025-01-10T00:00:00Z"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let episode_1 = Uuid::parse_str(e1["id"].as_str().unwrap()).unwrap();

    let (status, _) = json_request(
        &app,
        "POST",
        &format!("/api/v1/sessions/{session_id}/episodes"),
        serde_json::json!({
            "type": "message",
            "role": "user",
            "content": "Kendra now prefers Nike",
            "created_at": "2025-03-10T00:00:00Z"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let src = state_store
        .create_entity(Entity::from_extraction(
            &ExtractedEntity {
                name: "Kendra".to_string(),
                entity_type: EntityType::Person,
                summary: None,
            },
            user_id,
            episode_1,
        ))
        .await
        .unwrap();
    let adidas = state_store
        .create_entity(Entity::from_extraction(
            &ExtractedEntity {
                name: "Adidas".to_string(),
                entity_type: EntityType::Organization,
                summary: None,
            },
            user_id,
            episode_1,
        ))
        .await
        .unwrap();
    let nike = state_store
        .create_entity(Entity::from_extraction(
            &ExtractedEntity {
                name: "Nike".to_string(),
                entity_type: EntityType::Organization,
                summary: None,
            },
            user_id,
            episode_1,
        ))
        .await
        .unwrap();

    let jan = chrono::DateTime::parse_from_rfc3339("2025-01-10T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let feb = chrono::DateTime::parse_from_rfc3339("2025-02-20T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let mar = chrono::DateTime::parse_from_rfc3339("2025-03-10T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc);

    let mut old_edge = state_store
        .create_edge(Edge::from_extraction(
            &ExtractedRelationship {
                source_name: "Kendra".to_string(),
                target_name: "Adidas".to_string(),
                label: "prefers".to_string(),
                fact: "Kendra prefers Adidas".to_string(),
                confidence: 0.85,
                valid_at: Some(jan),
            },
            user_id,
            src.id,
            adidas.id,
            episode_1,
            jan,
        ))
        .await
        .unwrap();
    old_edge.invalid_at = Some(feb);
    state_store.update_edge(&old_edge).await.unwrap();

    state_store
        .create_edge(Edge::from_extraction(
            &ExtractedRelationship {
                source_name: "Kendra".to_string(),
                target_name: "Nike".to_string(),
                label: "prefers".to_string(),
                fact: "Kendra prefers Nike".to_string(),
                confidence: 0.87,
                valid_at: Some(mar),
            },
            user_id,
            src.id,
            nike.id,
            episode_1,
            mar,
        ))
        .await
        .unwrap();

    // ─── Expected resolved diagnostics per (contract, policy) ──────────

    // (contract, policy) -> (max_tokens, min_relevance, temporal_intent, temporal_weight)
    struct Expected {
        contract: &'static str,
        policy: &'static str,
        max_tokens: u64,
        min_relevance: f64,
        temporal_intent: &'static str,
        temporal_weight: Option<f64>,
    }

    let cases = vec![
        // ── Default contract ──
        Expected {
            contract: "default",
            policy: "balanced",
            max_tokens: 500,
            min_relevance: 0.0,
            temporal_intent: "historical",
            temporal_weight: None,
        },
        Expected {
            contract: "default",
            policy: "precision",
            max_tokens: 400,
            min_relevance: 0.0,
            temporal_intent: "historical",
            temporal_weight: Some(0.35),
        },
        Expected {
            contract: "default",
            policy: "recall",
            max_tokens: 700,
            min_relevance: 0.0,
            temporal_intent: "historical",
            temporal_weight: Some(0.2),
        },
        Expected {
            contract: "default",
            policy: "stability",
            max_tokens: 500,
            min_relevance: 0.0,
            temporal_intent: "current",
            temporal_weight: Some(0.8),
        },
        // ── SupportSafe ──
        Expected {
            contract: "support_safe",
            policy: "balanced",
            max_tokens: 500,
            min_relevance: 0.0,
            temporal_intent: "historical",
            temporal_weight: None,
        },
        Expected {
            contract: "support_safe",
            policy: "precision",
            max_tokens: 400,
            min_relevance: 0.0,
            temporal_intent: "historical",
            temporal_weight: Some(0.35),
        },
        Expected {
            contract: "support_safe",
            policy: "recall",
            max_tokens: 700,
            min_relevance: 0.0,
            temporal_intent: "historical",
            temporal_weight: Some(0.2),
        },
        Expected {
            contract: "support_safe",
            policy: "stability",
            max_tokens: 500,
            min_relevance: 0.0,
            temporal_intent: "current",
            temporal_weight: Some(0.8),
        },
        // ── CurrentStrict ──
        Expected {
            contract: "current_strict",
            policy: "balanced",
            max_tokens: 500,
            min_relevance: 0.0,
            temporal_intent: "current",
            temporal_weight: None,
        },
        Expected {
            contract: "current_strict",
            policy: "precision",
            max_tokens: 400,
            min_relevance: 0.0,
            temporal_intent: "current",
            temporal_weight: Some(0.35),
        },
        Expected {
            contract: "current_strict",
            policy: "recall",
            max_tokens: 700,
            min_relevance: 0.0,
            temporal_intent: "current",
            temporal_weight: Some(0.2),
        },
        Expected {
            contract: "current_strict",
            policy: "stability",
            max_tokens: 500,
            min_relevance: 0.0,
            temporal_intent: "current",
            temporal_weight: Some(0.8),
        },
        // ── HistoricalStrict ──
        Expected {
            contract: "historical_strict",
            policy: "balanced",
            max_tokens: 500,
            min_relevance: 0.0,
            temporal_intent: "historical",
            temporal_weight: None,
        },
        Expected {
            contract: "historical_strict",
            policy: "precision",
            max_tokens: 400,
            min_relevance: 0.0,
            temporal_intent: "historical",
            temporal_weight: Some(0.35),
        },
        Expected {
            contract: "historical_strict",
            policy: "recall",
            max_tokens: 700,
            min_relevance: 0.0,
            temporal_intent: "historical",
            temporal_weight: Some(0.2),
        },
        // KEY CASE: Stability + HistoricalStrict does NOT override to current.
        Expected {
            contract: "historical_strict",
            policy: "stability",
            max_tokens: 500,
            min_relevance: 0.0,
            temporal_intent: "historical",
            temporal_weight: Some(0.8),
        },
    ];

    // min_relevance is explicitly set to 0.0 in the request to override policy defaults,
    // so we expect 0.0 in all diagnostics. This isolates the policy resolution logic from
    // the min_relevance default behavior.

    for (i, case) in cases.iter().enumerate() {
        let (status, trace) = json_request(
            &app,
            "POST",
            "/api/v1/memory/combo-user/time_travel/trace",
            serde_json::json!({
                "query": "What does Kendra prefer?",
                "from": "2025-02-01T00:00:00Z",
                "to": "2025-04-01T00:00:00Z",
                "session": "combo-session",
                "contract": case.contract,
                "retrieval_policy": case.policy,
                "min_relevance": 0.0
            }),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "Case {i} ({} + {}) returned {status}",
            case.contract,
            case.policy
        );
        assert_eq!(
            trace["contract_applied"], case.contract,
            "Case {i}: contract_applied mismatch"
        );
        assert_eq!(
            trace["retrieval_policy_applied"], case.policy,
            "Case {i}: retrieval_policy_applied mismatch"
        );

        let diag = &trace["retrieval_policy_diagnostics"];
        assert_eq!(
            diag["effective_max_tokens"].as_u64().unwrap(),
            case.max_tokens,
            "Case {i} ({} + {}): effective_max_tokens mismatch",
            case.contract,
            case.policy
        );
        let effective_min_rel = diag["effective_min_relevance"].as_f64().unwrap();
        assert!(
            (effective_min_rel - case.min_relevance).abs() < 0.01,
            "Case {i} ({} + {}): effective_min_relevance expected {} got {}",
            case.contract,
            case.policy,
            case.min_relevance,
            effective_min_rel
        );
        assert_eq!(
            diag["effective_temporal_intent"].as_str().unwrap(),
            case.temporal_intent,
            "Case {i} ({} + {}): effective_temporal_intent mismatch",
            case.contract,
            case.policy
        );
        match case.temporal_weight {
            Some(expected_w) => {
                let actual_w = diag["effective_temporal_weight"].as_f64().unwrap();
                assert!(
                    (actual_w - expected_w).abs() < 0.01,
                    "Case {i} ({} + {}): effective_temporal_weight expected {expected_w} got {actual_w}",
                    case.contract,
                    case.policy
                );
            }
            None => {
                assert!(
                    diag["effective_temporal_weight"].is_null(),
                    "Case {i} ({} + {}): expected null temporal_weight, got {:?}",
                    case.contract,
                    case.policy,
                    diag["effective_temporal_weight"]
                );
            }
        }

        // Every combination should return valid structural fields.
        assert!(
            trace["snapshot_from"].is_object(),
            "Case {i}: missing snapshot_from"
        );
        assert!(
            trace["snapshot_to"].is_object(),
            "Case {i}: missing snapshot_to"
        );
        assert!(
            trace["gained_facts"].is_array(),
            "Case {i}: missing gained_facts"
        );
        assert!(
            trace["lost_facts"].is_array(),
            "Case {i}: missing lost_facts"
        );
        assert!(
            trace["gained_episodes"].is_array(),
            "Case {i}: missing gained_episodes"
        );
        assert!(
            trace["lost_episodes"].is_array(),
            "Case {i}: missing lost_episodes"
        );
        assert!(trace["timeline"].is_array(), "Case {i}: missing timeline");
        assert!(trace["summary"].is_string(), "Case {i}: missing summary");
    }

    // ─── Specific falsification: Stability + HistoricalStrict vs Stability + Default ──
    // The ONLY case where Stability does NOT override temporal_intent to "current"
    // is when paired with HistoricalStrict. Verify this contrast explicitly.

    let (_, trace_stability_default) = json_request(
        &app,
        "POST",
        "/api/v1/memory/combo-user/time_travel/trace",
        serde_json::json!({
            "query": "What does Kendra prefer?",
            "from": "2025-02-01T00:00:00Z",
            "to": "2025-04-01T00:00:00Z",
            "contract": "default",
            "retrieval_policy": "stability",
            "min_relevance": 0.0
        }),
    )
    .await;
    let (_, trace_stability_hist) = json_request(
        &app,
        "POST",
        "/api/v1/memory/combo-user/time_travel/trace",
        serde_json::json!({
            "query": "What does Kendra prefer?",
            "from": "2025-02-01T00:00:00Z",
            "to": "2025-04-01T00:00:00Z",
            "contract": "historical_strict",
            "retrieval_policy": "stability",
            "min_relevance": 0.0
        }),
    )
    .await;

    assert_eq!(
        trace_stability_default["retrieval_policy_diagnostics"]["effective_temporal_intent"],
        "current",
        "Stability + Default should resolve temporal_intent to current"
    );
    assert_eq!(
        trace_stability_hist["retrieval_policy_diagnostics"]["effective_temporal_intent"],
        "historical",
        "Stability + HistoricalStrict should preserve temporal_intent as historical"
    );
}

// ── WH-15: Webhook persistence survives simulated restart ─────────

#[tokio::test]
async fn test_webhook_persistence_survives_restart() {
    // Phase 1: Build state with persistence_enabled=true and real Redis
    let redis_url = std::env::var("MNEMO_TEST_REDIS_URL")
        .unwrap_or_else(|_| "redis://localhost:6379".to_string());
    let qdrant_url = std::env::var("MNEMO_TEST_QDRANT_URL")
        .unwrap_or_else(|_| "http://localhost:6334".to_string());

    let uid = Uuid::now_v7();
    let redis_prefix = format!("wh15_test:{}:", uid);
    let qdrant_prefix =
        std::env::var("MNEMO_TEST_QDRANT_PREFIX").unwrap_or_else(|_| "mnemo_".to_string());
    let webhook_redis_prefix = format!("wh15_test:{}:webhooks", uid);

    let state_store = Arc::new(
        RedisStateStore::new(&redis_url, &redis_prefix)
            .await
            .expect("Redis required for WH-15 test"),
    );
    state_store.ensure_indexes().await.unwrap();

    let vector_store = Arc::new(
        QdrantVectorStore::new(&qdrant_url, &qdrant_prefix, 1536)
            .await
            .expect("Qdrant required for WH-15 test"),
    );

    let embedder = Arc::new(EmbedderKind::OpenAiCompat(OpenAiCompatibleEmbedder::new(
        mnemo_core::traits::llm::EmbeddingConfig {
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

    // Create Redis connection for webhook persistence
    let redis_client =
        redis::Client::open(redis_url.as_str()).expect("Failed to create Redis client");
    let webhook_conn = redis::aio::ConnectionManager::new(redis_client)
        .await
        .expect("Failed to create Redis connection for webhooks");

    let webhook_config = WebhookDeliveryConfig {
        enabled: true,
        max_attempts: 3,
        base_backoff_ms: 20,
        request_timeout_ms: 150,
        max_events_per_webhook: 1000,
        rate_limit_per_minute: 120,
        circuit_breaker_threshold: 5,
        circuit_breaker_cooldown_ms: 200,
        persistence_enabled: true,
    };

    let state1 = AppState {
        state_store: state_store.clone(),
        vector_store: vector_store.clone(),
        retrieval: retrieval.clone(),
        graph: graph.clone(),
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
        webhook_delivery: webhook_config,
        webhook_http: Arc::new(reqwest::Client::new()),
        webhook_redis: Some(webhook_conn.clone()),
        webhook_redis_prefix: webhook_redis_prefix.clone(),
        metrics: Arc::new(ServerMetrics::default()),
        llm_spans: Arc::new(tokio::sync::RwLock::new(std::collections::VecDeque::new())),
        memory_digests: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        require_tls: false,
        audit_signing_secret: None,
    };

    let app1 = build_router(state1.clone()).layer(from_fn_with_state(
        state1.clone(),
        request_context_middleware,
    ));

    // First create a user by writing memory
    let user_name = format!("wh15_user_{}", uid);
    let (mem_status, mem_body) = json_request(
        &app1,
        "POST",
        "/api/v1/memory",
        serde_json::json!({
            "user": user_name,
            "text": "Test message for WH-15",
            "role": "user"
        }),
    )
    .await;
    assert!(
        mem_status == StatusCode::OK || mem_status == StatusCode::CREATED,
        "memory write should succeed: {:?}",
        mem_body,
    );

    // Register a webhook via the API
    let (status, body) = json_request(
        &app1,
        "POST",
        "/api/v1/memory/webhooks",
        serde_json::json!({
            "user": user_name,
            "target_url": "https://example.com/webhook",
            "events": ["fact_added"],
            "signing_secret": "test-secret-123"
        }),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::CREATED,
        "webhook registration should succeed: {:?}",
        body
    );
    let webhook_id = body["webhook"]["id"]
        .as_str()
        .expect("webhook should have an id");

    // Verify the webhook is in state1's memory
    {
        let hooks = state1.memory_webhooks.read().await;
        assert_eq!(hooks.len(), 1, "state1 should have 1 webhook");
    }

    // Phase 2: Build a SECOND AppState with EMPTY in-memory maps (simulating restart)
    let redis_client2 =
        redis::Client::open(redis_url.as_str()).expect("Failed to create Redis client");
    let webhook_conn2 = redis::aio::ConnectionManager::new(redis_client2)
        .await
        .expect("Failed to create Redis connection for webhooks");

    let state2 = AppState {
        state_store: state_store.clone(),
        vector_store: vector_store.clone(),
        retrieval: retrieval.clone(),
        graph: graph.clone(),
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
        webhook_delivery: webhook_config,
        webhook_http: Arc::new(reqwest::Client::new()),
        webhook_redis: Some(webhook_conn2),
        webhook_redis_prefix: webhook_redis_prefix.clone(),
        metrics: Arc::new(ServerMetrics::default()),
        llm_spans: Arc::new(tokio::sync::RwLock::new(std::collections::VecDeque::new())),
        memory_digests: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        require_tls: false,
        audit_signing_secret: None,
    };

    // Verify state2 starts empty
    {
        let hooks2 = state2.memory_webhooks.read().await;
        assert!(hooks2.is_empty(), "state2 should start with empty webhooks");
    }

    // Call restore — this simulates what main.rs does on startup
    restore_webhook_state(&state2)
        .await
        .expect("restore_webhook_state should succeed");

    // Phase 3: Verify state2 now has the webhook from state1
    {
        let hooks2 = state2.memory_webhooks.read().await;
        assert_eq!(
            hooks2.len(),
            1,
            "after restore, state2 should have 1 webhook (WH-15: persistence survives restart)"
        );

        let webhook_uuid: Uuid = webhook_id.parse().expect("webhook_id should be valid UUID");
        let webhook = hooks2
            .get(&webhook_uuid)
            .expect("restored webhook should have the same UUID");
        assert_eq!(
            webhook.target_url, "https://example.com/webhook",
            "restored webhook should have correct target_url"
        );
    }
}

// =============================================================================
// MSG-07: Session Messages API — Rust integration tests
// =============================================================================

/// Helper: write a memory episode and return the session_id from the response.
async fn write_episode(app: &axum::Router, user: &str, session: &str, text: &str) -> String {
    let (status, body) = json_request(
        app,
        "POST",
        "/api/v1/memory",
        serde_json::json!({
            "user": user,
            "session": session,
            "text": text,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "write failed: {:?}", body);
    body["session_id"]
        .as_str()
        .expect("response must have session_id")
        .to_string()
}

#[tokio::test]
async fn msg07_get_messages_returns_chronological_order() {
    let app = build_test_app().await;
    let session_name = format!("msg07_chrono_{}", Uuid::now_v7());
    let user = format!("msg07_user_{}", Uuid::now_v7());

    // Write 3 episodes in order
    let mut session_id = String::new();
    for i in 1..=3 {
        session_id = write_episode(&app, &user, &session_name, &format!("message {}", i)).await;
        // Small delay to ensure distinct created_at timestamps
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // GET messages
    let (status, body) = json_request(
        &app,
        "GET",
        &format!("/api/v1/sessions/{}/messages", session_id),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["count"].as_u64().unwrap(), 3);

    let messages = body["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 3);

    // Verify chronological order (ascending created_at)
    for i in 0..messages.len() - 1 {
        let t1 = messages[i]["created_at"].as_str().unwrap();
        let t2 = messages[i + 1]["created_at"].as_str().unwrap();
        assert!(t1 <= t2, "Messages must be chronological: {} <= {}", t1, t2);
    }

    // Verify idx values are 0-based sequential
    for (i, msg) in messages.iter().enumerate() {
        assert_eq!(msg["idx"].as_u64().unwrap(), i as u64);
    }

    // Verify content
    assert!(messages[0]["content"]
        .as_str()
        .unwrap()
        .contains("message 1"));
    assert!(messages[2]["content"]
        .as_str()
        .unwrap()
        .contains("message 3"));
}

#[tokio::test]
async fn msg07_get_messages_with_limit() {
    let app = build_test_app().await;
    let session_name = format!("msg07_limit_{}", Uuid::now_v7());
    let user = format!("msg07_user_{}", Uuid::now_v7());

    let mut session_id = String::new();
    for i in 1..=5 {
        session_id = write_episode(&app, &user, &session_name, &format!("msg {}", i)).await;
        tokio::time::sleep(Duration::from_millis(15)).await;
    }

    // GET with limit=2
    let (status, body) = json_request(
        &app,
        "GET",
        &format!("/api/v1/sessions/{}/messages?limit=2", session_id),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["count"].as_u64().unwrap(), 2);
    let messages = body["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 2);
}

#[tokio::test]
async fn msg07_delete_by_index_removes_correct_message() {
    let app = build_test_app().await;
    let session_name = format!("msg07_delidx_{}", Uuid::now_v7());
    let user = format!("msg07_user_{}", Uuid::now_v7());

    let mut session_id = String::new();
    for i in 1..=3 {
        session_id = write_episode(&app, &user, &session_name, &format!("episode_{}", i)).await;
        tokio::time::sleep(Duration::from_millis(15)).await;
    }

    // Delete index 1 (the middle message)
    let (status, body) = json_request(
        &app,
        "DELETE",
        &format!("/api/v1/sessions/{}/messages/1", session_id),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["deleted"].as_bool().unwrap());

    // GET messages — should have 2 left
    let (status, body) = json_request(
        &app,
        "GET",
        &format!("/api/v1/sessions/{}/messages", session_id),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["count"].as_u64().unwrap(), 2);

    let messages = body["messages"].as_array().unwrap();
    // The remaining messages should be episode_1 and episode_3
    assert!(messages[0]["content"]
        .as_str()
        .unwrap()
        .contains("episode_1"));
    assert!(messages[1]["content"]
        .as_str()
        .unwrap()
        .contains("episode_3"));
}

#[tokio::test]
async fn msg07_delete_by_out_of_bounds_index_returns_400() {
    let app = build_test_app().await;
    let session_name = format!("msg07_oob_{}", Uuid::now_v7());
    let user = format!("msg07_user_{}", Uuid::now_v7());

    let session_id = write_episode(&app, &user, &session_name, "only message").await;

    // Delete index 999 — out of bounds
    let (status, body) = json_request(
        &app,
        "DELETE",
        &format!("/api/v1/sessions/{}/messages/999", session_id),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "Out-of-bounds index should return 400: {:?}",
        body
    );
}

#[tokio::test]
async fn msg07_delete_all_messages_clears_without_deleting_session() {
    let app = build_test_app().await;
    let session_name = format!("msg07_delall_{}", Uuid::now_v7());
    let user = format!("msg07_user_{}", Uuid::now_v7());

    let mut session_id = String::new();
    for i in 1..=3 {
        session_id = write_episode(&app, &user, &session_name, &format!("msg_{}", i)).await;
    }

    // DELETE all messages
    let (status, body) = json_request(
        &app,
        "DELETE",
        &format!("/api/v1/sessions/{}/messages", session_id),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["session_id"].as_str().unwrap(), session_id);
    // deleted count should be >= 3 (could be more if extraction created additional episodes)
    assert!(
        body["deleted"].as_u64().unwrap() >= 3,
        "Expected at least 3 deleted, got {}",
        body["deleted"]
    );

    // Verify session still exists (GET episodes should return empty list, not 404)
    let (status, body) = json_request(
        &app,
        "GET",
        &format!("/api/v1/sessions/{}/messages", session_id),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["count"].as_u64().unwrap(), 0);
    assert!(body["messages"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn msg07_get_messages_for_nonexistent_session_returns_empty() {
    let app = build_test_app().await;
    let fake_id = Uuid::now_v7();

    let (status, body) = json_request(
        &app,
        "GET",
        &format!("/api/v1/sessions/{}/messages", fake_id),
        serde_json::json!({}),
    )
    .await;
    // Should return 200 with empty list (not 404) — list_episodes returns [] for unknown session
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["count"].as_u64().unwrap(), 0);
}

#[tokio::test]
async fn msg07_messages_have_required_fields() {
    let app = build_test_app().await;
    let session_name = format!("msg07_fields_{}", Uuid::now_v7());
    let user = format!("msg07_user_{}", Uuid::now_v7());

    let session_id = write_episode(&app, &user, &session_name, "field check message").await;

    let (status, body) = json_request(
        &app,
        "GET",
        &format!("/api/v1/sessions/{}/messages", session_id),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let msg = &body["messages"][0];
    // Required fields per MessageRecord struct
    assert!(msg.get("idx").is_some(), "missing idx");
    assert!(msg.get("id").is_some(), "missing id");
    assert!(msg.get("content").is_some(), "missing content");
    assert!(msg.get("created_at").is_some(), "missing created_at");
    // role may be null but the field should exist
    assert!(msg.get("role").is_some(), "missing role field");
    // session_id in response envelope
    assert_eq!(body["session_id"].as_str().unwrap(), session_id);
}

// =============================================================================
// VEC-12: Raw Vector API — Rust integration tests
//
// Uses a single shared namespace to avoid Qdrant FD exhaustion from creating
// too many collections in parallel. Tests run sequentially within this single
// comprehensive test function.
// =============================================================================

/// Helper: create a random 1536-dimensional vector (normalized).
fn random_vector(seed: u64) -> Vec<f32> {
    let mut v: Vec<f32> = (0..1536)
        .map(|i| (seed as f32 * 0.31 + i as f32 * 0.73).sin())
        .collect();
    let mag = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    for x in v.iter_mut() {
        *x /= mag;
    }
    v
}

/// Comprehensive VEC-12 test covering all 6 raw vector endpoints.
/// Runs as a single test to use only 1 namespace (avoids Qdrant FD exhaustion).
#[tokio::test]
async fn vec12_raw_vector_api_comprehensive() {
    let app = build_test_app().await;
    let ns = format!("vec12_{}", Uuid::now_v7().simple());

    // --- VEC-01: Namespace should not exist initially ---
    let (status, body) = json_request(
        &app,
        "GET",
        &format!("/api/v1/vectors/{}/exists", ns),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        !body["exists"].as_bool().unwrap(),
        "VEC-01: namespace should not exist initially"
    );

    // --- VEC-09: Count returns 0 for non-existent namespace ---
    let (status, body) = json_request(
        &app,
        "GET",
        &format!("/api/v1/vectors/{}/count", ns),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["count"].as_u64().unwrap(),
        0,
        "VEC-09: count should be 0 for non-existent ns"
    );

    // --- VEC-04: Query non-existent namespace returns empty ---
    let (status, body) = json_request(
        &app,
        "POST",
        &format!("/api/v1/vectors/{}/query", ns),
        serde_json::json!({
            "vector": random_vector(300),
            "top_k": 5,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body["results"].as_array().unwrap().is_empty(),
        "VEC-04: query on non-existent ns should return []"
    );

    // --- Validation: empty vectors array returns 400 ---
    let (status, _) = json_request(
        &app,
        "POST",
        &format!("/api/v1/vectors/{}", ns),
        serde_json::json!({"vectors": []}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "Empty vectors should return 400"
    );

    // --- Validation: empty ids array returns 400 ---
    let (status, _) = json_request(
        &app,
        "POST",
        &format!("/api/v1/vectors/{}/delete", ns),
        serde_json::json!({"ids": []}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "Empty ids should return 400"
    );

    // --- VEC-01: Upsert creates namespace automatically ---
    let v1 = random_vector(1);
    let v2 = random_vector(2);
    let v3 = random_vector(3);

    let (status, body) = json_request(
        &app,
        "POST",
        &format!("/api/v1/vectors/{}", ns),
        serde_json::json!({
            "vectors": [
                {"id": "v1", "vector": v1.clone(), "metadata": {"key": "a"}},
                {"id": "v2", "vector": v2.clone(), "metadata": {"key": "b"}},
                {"id": "v3", "vector": v3, "metadata": {"key": "c"}},
            ]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "VEC-01 upsert failed: {:?}", body);
    assert!(body["ok"].as_bool().unwrap());
    assert_eq!(body["upserted"].as_u64().unwrap(), 3);

    // --- VEC-10: Namespace now exists ---
    let (status, body) = json_request(
        &app,
        "GET",
        &format!("/api/v1/vectors/{}/exists", ns),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body["exists"].as_bool().unwrap(),
        "VEC-10: namespace should exist after upsert"
    );

    // Count should be 3
    tokio::time::sleep(Duration::from_millis(300)).await;
    let (status, body) = json_request(
        &app,
        "GET",
        &format!("/api/v1/vectors/{}/count", ns),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["count"].as_u64().unwrap(),
        3,
        "Count should be 3 after upserting 3 vectors"
    );

    // --- VEC-02: Upsert same ID is idempotent (overwrites, count unchanged) ---
    let (status, body) = json_request(
        &app,
        "POST",
        &format!("/api/v1/vectors/{}", ns),
        serde_json::json!({
            "vectors": [{"id": "v1", "vector": random_vector(99), "metadata": {"key": "updated"}}]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["ok"].as_bool().unwrap());

    tokio::time::sleep(Duration::from_millis(300)).await;
    let (_, body) = json_request(
        &app,
        "GET",
        &format!("/api/v1/vectors/{}/count", ns),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(
        body["count"].as_u64().unwrap(),
        3,
        "VEC-02: count should remain 3 after idempotent upsert"
    );

    // --- VEC-03: Query returns semantically similar results ---
    let (status, body) = json_request(
        &app,
        "POST",
        &format!("/api/v1/vectors/{}/query", ns),
        serde_json::json!({
            "vector": v2,
            "top_k": 3,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let results = body["results"].as_array().unwrap();
    assert!(!results.is_empty(), "VEC-03: query should return results");
    // v2 queried with itself — should be top result with near-perfect score
    assert_eq!(results[0]["id"].as_str().unwrap(), "v2");
    assert!(
        results[0]["score"].as_f64().unwrap() > 0.99,
        "VEC-03: exact match should have score > 0.99"
    );

    // --- VEC-05: Delete IDs removes specific vectors ---
    let (status, body) = json_request(
        &app,
        "POST",
        &format!("/api/v1/vectors/{}/delete", ns),
        serde_json::json!({"ids": ["v2"]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["ok"].as_bool().unwrap());

    tokio::time::sleep(Duration::from_millis(300)).await;
    let (_, body) = json_request(
        &app,
        "GET",
        &format!("/api/v1/vectors/{}/count", ns),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(
        body["count"].as_u64().unwrap(),
        2,
        "VEC-05: count should be 2 after deleting 1 vector"
    );

    // --- VEC-06: Delete non-existent IDs is no-op ---
    let (status, body) = json_request(
        &app,
        "POST",
        &format!("/api/v1/vectors/{}/delete", ns),
        serde_json::json!({"ids": ["nonexistent_id_xyz"]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body["ok"].as_bool().unwrap(),
        "VEC-06: delete non-existent ID should not error"
    );

    // --- VEC-07: Delete namespace removes all vectors ---
    let (status, body) = json_request(
        &app,
        "DELETE",
        &format!("/api/v1/vectors/{}", ns),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["ok"].as_bool().unwrap());
    assert!(body["deleted"].as_bool().unwrap());

    // Namespace should no longer exist
    let (_, body) = json_request(
        &app,
        "GET",
        &format!("/api/v1/vectors/{}/exists", ns),
        serde_json::json!({}),
    )
    .await;
    assert!(
        !body["exists"].as_bool().unwrap(),
        "VEC-07: namespace should not exist after deletion"
    );

    // --- VEC-08: Delete non-existent namespace is no-op ---
    let fake_ns = format!("vec12_fake_{}", Uuid::now_v7().simple());
    let (status, body) = json_request(
        &app,
        "DELETE",
        &format!("/api/v1/vectors/{}", fake_ns),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body["ok"].as_bool().unwrap(),
        "VEC-08: delete non-existent namespace should be ok"
    );
}

// =============================================================================
// AUTH-07: Auth integration test with real server router
// =============================================================================

use mnemo_server::middleware::{AuthConfig, AuthLayer};

async fn build_authed_test_app(keys: Vec<String>) -> axum::Router {
    let (base_app, _store) = build_test_harness_with_prefilter(MetadataPrefilterConfig {
        enabled: true,
        scan_limit: 400,
        relax_if_empty: false,
    })
    .await;

    // Wrap with auth layer (same as main.rs does)
    base_app.layer(AuthLayer::new(AuthConfig::with_keys(keys)))
}

#[tokio::test]
async fn auth07_missing_key_returns_401_on_protected_endpoint() {
    let app = build_authed_test_app(vec!["test-secret-key".to_string()]).await;

    let (status, body) = json_request(
        &app,
        "POST",
        "/api/v1/memory",
        serde_json::json!({"user": "test", "session": "s1", "text": "hello"}),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"].as_str().unwrap(), "unauthorized");
}

#[tokio::test]
async fn auth07_wrong_key_returns_401() {
    let app = build_authed_test_app(vec!["correct-key".to_string()]).await;

    let (status, body) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/memory",
        "authorization",
        "Bearer wrong-key",
        serde_json::json!({"user": "test", "session": "s1", "text": "hello"}),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"].as_str().unwrap(), "unauthorized");
}

#[tokio::test]
async fn auth07_correct_bearer_key_allows_request() {
    let app = build_authed_test_app(vec!["correct-key".to_string()]).await;

    let (status, body) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/memory",
        "authorization",
        "Bearer correct-key",
        serde_json::json!({
            "user": format!("auth07_{}", Uuid::now_v7()),
            "session": "s1",
            "text": "authed write",
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "Authed request should succeed: {:?}",
        body
    );
    assert!(body["ok"].as_bool().unwrap());
}

#[tokio::test]
async fn auth07_correct_x_api_key_allows_request() {
    let app = build_authed_test_app(vec!["x-key-value".to_string()]).await;

    let (status, body) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/memory",
        "x-api-key",
        "x-key-value",
        serde_json::json!({
            "user": format!("auth07_{}", Uuid::now_v7()),
            "session": "s1",
            "text": "x-api-key write",
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "X-API-Key should work: {:?}",
        body
    );
    assert!(body["ok"].as_bool().unwrap());
}

#[tokio::test]
async fn auth07_health_bypasses_auth() {
    let app = build_authed_test_app(vec!["secret".to_string()]).await;

    // /health without any key should succeed
    let (status, body) = json_request(&app, "GET", "/health", serde_json::json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"].as_str().unwrap(), "ok");

    // /healthz too
    let (status, body) = json_request(&app, "GET", "/healthz", serde_json::json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"].as_str().unwrap(), "ok");
}

#[tokio::test]
async fn auth07_metrics_bypasses_auth() {
    let app = build_authed_test_app(vec!["secret".to_string()]).await;

    let request = Request::builder()
        .method("GET")
        .uri("/metrics")
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

// =============================================================================
// API-01: Request-id header systematically tested across key endpoints
// =============================================================================

#[tokio::test]
async fn api01_request_id_header_on_success_endpoints() {
    let app = build_test_app().await;
    let user = format!("api01_user_{}", Uuid::now_v7());

    // Test a variety of endpoints for x-mnemo-request-id in response
    let endpoints: Vec<(&str, &str, Value)> = vec![
        ("GET", "/health", serde_json::json!({})),
        ("GET", "/healthz", serde_json::json!({})),
        (
            "POST",
            "/api/v1/memory",
            serde_json::json!({"user": &user, "session": "api01_s", "text": "hello"}),
        ),
    ];

    for (method, path, payload) in &endpoints {
        let request = Request::builder()
            .method(*method)
            .uri(*path)
            .header("content-type", "application/json")
            .body(Body::from(payload.to_string()))
            .unwrap();

        let response = app.clone().oneshot(request).await.unwrap();
        let rid = response
            .headers()
            .get("x-mnemo-request-id")
            .map(|v| v.to_str().unwrap().to_string());
        assert!(
            rid.is_some(),
            "API-01: {} {} should have x-mnemo-request-id header",
            method,
            path
        );
        // Should be a valid UUID
        let rid_str = rid.unwrap();
        assert!(
            Uuid::parse_str(&rid_str).is_ok(),
            "API-01: request-id should be a valid UUID, got: {}",
            rid_str
        );
    }
}

#[tokio::test]
async fn api01_request_id_header_on_error_endpoints() {
    let app = build_test_app().await;

    // Test 1: Invalid body → 400/422
    {
        let request = Request::builder()
            .method("POST")
            .uri("/api/v1/memory")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({"bad": "payload"}).to_string(),
            ))
            .unwrap();

        let response = app.clone().oneshot(request).await.unwrap();
        let rid = response.headers().get("x-mnemo-request-id");
        assert!(
            rid.is_some(),
            "API-01: POST /api/v1/memory (invalid body) should have x-mnemo-request-id"
        );
    }

    // Test 2: Non-existent user context → should still have request-id
    {
        let context_path = format!("/api/v1/memory/{}/context", Uuid::now_v7());
        let request = Request::builder()
            .method("POST")
            .uri(&context_path)
            .header("content-type", "application/json")
            .body(Body::from(serde_json::json!({"query": "test"}).to_string()))
            .unwrap();

        let response = app.clone().oneshot(request).await.unwrap();
        let rid = response.headers().get("x-mnemo-request-id");
        assert!(
            rid.is_some(),
            "API-01: POST context (error case) should have x-mnemo-request-id"
        );
    }
}

#[tokio::test]
async fn api01_client_provided_request_id_is_echoed() {
    let app = build_test_app().await;
    let custom_id = "my-custom-request-id-12345";

    let request = Request::builder()
        .method("GET")
        .uri("/health")
        .header("x-mnemo-request-id", custom_id)
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    let rid = response
        .headers()
        .get("x-mnemo-request-id")
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(
        rid, custom_id,
        "API-01: client-provided request-id should be echoed back"
    );
}

// =============================================================================
// API-04: /healthz returns same response as /health
// =============================================================================

#[tokio::test]
async fn api04_healthz_mirrors_health() {
    let app = build_test_app().await;

    let (status1, body1) = json_request(&app, "GET", "/health", serde_json::json!({})).await;
    let (status2, body2) = json_request(&app, "GET", "/healthz", serde_json::json!({})).await;

    assert_eq!(status1, StatusCode::OK);
    assert_eq!(status2, StatusCode::OK);
    assert_eq!(body1["status"], body2["status"]);
    assert_eq!(body1["version"], body2["version"]);
}

// =============================================================================
// API-05: Unknown routes return 404, not 500
// =============================================================================

#[tokio::test]
async fn api05_unknown_route_returns_404() {
    let app = build_test_app().await;

    let request = Request::builder()
        .method("GET")
        .uri("/api/v1/nonexistent")
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "Unknown routes should return 404, not 500"
    );
}

// =============================================================================
// WH-13: Webhook rate limiting protects downstream
// =============================================================================

/// Helper: register a webhook for a user and return the webhook_id.
async fn register_webhook(app: &axum::Router, user: &str, sink_url: &str) -> String {
    // Ensure user exists
    let (status, _) = json_request(
        app,
        "POST",
        "/api/v1/memory",
        serde_json::json!({
            "user": user,
            "session": "default",
            "text": "seed for webhook"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, registered) = json_request(
        app,
        "POST",
        "/api/v1/memory/webhooks",
        serde_json::json!({
            "user": user,
            "target_url": sink_url,
            "signing_secret": "whsec_test",
            "events": ["head_advanced"]
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "webhook registration failed: {:?}",
        registered
    );
    registered["webhook"]["id"]
        .as_str()
        .expect("webhook id")
        .to_string()
}

#[tokio::test]
async fn wh13_rate_limiting_throttles_excess_deliveries() {
    // Set rate limit to 2 per minute — the 3rd webhook delivery should be throttled
    let (sink_url, attempts, _deliveries) = start_webhook_sink_server(0).await;
    let (app, _) = build_test_harness_with_prefilter_and_webhooks(
        MetadataPrefilterConfig {
            enabled: true,
            scan_limit: 400,
            relax_if_empty: false,
        },
        WebhookDeliveryConfig {
            enabled: true,
            max_attempts: 1, // single attempt so failures are immediate
            base_backoff_ms: 10,
            request_timeout_ms: 500,
            max_events_per_webhook: 1000,
            rate_limit_per_minute: 2, // Very low — only 2 allowed per minute
            circuit_breaker_threshold: 100, // High so circuit doesn't trip
            circuit_breaker_cooldown_ms: 60_000,
            persistence_enabled: false,
        },
    )
    .await;

    let user = format!("wh13_user_{}", Uuid::now_v7());
    let webhook_id = register_webhook(&app, &user, &sink_url).await;

    // Fire 4 events rapidly — only 2 should be delivered, rest rate-limited
    for i in 1..=4 {
        let (status, _) = json_request(
            &app,
            "POST",
            "/api/v1/memory",
            serde_json::json!({
                "user": &user,
                "session": "default",
                "text": format!("message {} for rate limit test", i)
            }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
    }

    // Wait for deliveries to complete/fail
    tokio::time::sleep(Duration::from_millis(1500)).await;

    // The sink should have received at most 2 successful deliveries
    let sink_attempts = attempts.load(Ordering::SeqCst);
    assert!(
        sink_attempts <= 2,
        "WH-13: Sink should receive at most 2 deliveries (rate limit), got {}",
        sink_attempts,
    );

    // Check stats — rate_limit_per_minute should be reflected
    let (status, stats) = json_request(
        &app,
        "GET",
        &format!("/api/v1/memory/webhooks/{}/stats", webhook_id),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        stats["rate_limit_per_minute"].as_u64().unwrap(),
        2,
        "WH-13: stats should show configured rate limit"
    );

    // Check that some events were dead-lettered or had delivery failures
    let (_, events) = json_request(
        &app,
        "GET",
        &format!("/api/v1/memory/webhooks/{}/events?limit=10", webhook_id),
        serde_json::json!({}),
    )
    .await;
    let all_events = events["events"].as_array().unwrap();
    let delivered_count = all_events
        .iter()
        .filter(|e| e["delivered"].as_bool().unwrap_or(false))
        .count();
    let dead_count = all_events
        .iter()
        .filter(|e| e["dead_letter"].as_bool().unwrap_or(false))
        .count();

    assert!(
        delivered_count <= 2,
        "WH-13: at most 2 events should be delivered, got {}",
        delivered_count,
    );
    assert!(
        dead_count >= 1,
        "WH-13: at least 1 event should be dead-lettered due to rate limit, got {}",
        dead_count,
    );
}

// =============================================================================
// WH-14: Circuit breaker opens after threshold failures
// =============================================================================

#[tokio::test]
async fn wh14_circuit_breaker_opens_after_threshold_failures() {
    // Sink always fails → circuit should trip after 2 consecutive failures
    let (sink_url, attempts, _) = start_webhook_sink_server(9999).await;
    let (app, _) = build_test_harness_with_prefilter_and_webhooks(
        MetadataPrefilterConfig {
            enabled: true,
            scan_limit: 400,
            relax_if_empty: false,
        },
        WebhookDeliveryConfig {
            enabled: true,
            max_attempts: 1, // single attempt, so each event produces exactly 1 failure
            base_backoff_ms: 10,
            request_timeout_ms: 200,
            max_events_per_webhook: 1000,
            rate_limit_per_minute: 120, // High so rate limit doesn't interfere
            circuit_breaker_threshold: 2, // Opens after 2 consecutive failures
            circuit_breaker_cooldown_ms: 30_000, // 30s — long enough that circuit stays open during test
            persistence_enabled: false,
        },
    )
    .await;

    let user = format!("wh14_user_{}", Uuid::now_v7());
    let webhook_id = register_webhook(&app, &user, &sink_url).await;

    // Fire 3 events — first 2 should attempt delivery (and fail), 3rd should be circuit-blocked
    for i in 1..=3 {
        let (status, _) = json_request(
            &app,
            "POST",
            "/api/v1/memory",
            serde_json::json!({
                "user": &user,
                "session": "default",
                "text": format!("message {} for circuit breaker test", i)
            }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        // Small gap to allow delivery processing
        tokio::time::sleep(Duration::from_millis(300)).await;
    }

    // Wait for all delivery attempts to complete
    tokio::time::sleep(Duration::from_millis(2000)).await;

    // Check stats — circuit should be open
    let (status, stats) = json_request(
        &app,
        "GET",
        &format!("/api/v1/memory/webhooks/{}/stats", webhook_id),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        stats["circuit_open"].as_bool().unwrap_or(false),
        "WH-14: circuit should be open after {} consecutive failures. Stats: {:?}",
        2,
        stats,
    );
    assert!(
        stats["circuit_open_until"].as_str().is_some(),
        "WH-14: circuit_open_until should be set"
    );

    // The sink should have received at most 2 actual HTTP attempts
    // (the 3rd+ events should be rejected by the circuit breaker without making HTTP calls)
    let total_attempts = attempts.load(Ordering::SeqCst);
    assert!(
        total_attempts <= 2,
        "WH-14: Sink should receive at most 2 HTTP attempts before circuit opens, got {}",
        total_attempts,
    );

    // Verify that at least one event was dead-lettered
    let (_, events) = json_request(
        &app,
        "GET",
        &format!("/api/v1/memory/webhooks/{}/events?limit=10", webhook_id),
        serde_json::json!({}),
    )
    .await;
    let all_events = events["events"].as_array().unwrap();
    let dead_count = all_events
        .iter()
        .filter(|e| e["dead_letter"].as_bool().unwrap_or(false))
        .count();
    assert!(
        dead_count >= 1,
        "WH-14: at least 1 event should be dead-lettered, got {}",
        dead_count,
    );
}

// ─── List Webhooks ─────────────────────────────────────────────────

#[tokio::test]
async fn test_list_memory_webhooks_returns_all_registered() {
    let (app, _) = build_test_harness_with_prefilter_and_webhooks(
        MetadataPrefilterConfig {
            enabled: true,
            scan_limit: 400,
            relax_if_empty: false,
        },
        WebhookDeliveryConfig {
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
    )
    .await;

    // Initially, list should be empty.
    let (status, body) = json_request(
        &app,
        "GET",
        "/api/v1/memory/webhooks",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["count"].as_u64().unwrap(), 0);
    assert!(body["data"].as_array().unwrap().is_empty());

    // Create users by writing a memory (webhook registration requires existing users).
    for user in &["list-wh-user-a", "list-wh-user-b"] {
        let (s, _) = json_request(
            &app,
            "POST",
            "/api/v1/memory",
            serde_json::json!({
                "user": user,
                "session": "default",
                "text": "seed"
            }),
        )
        .await;
        assert_eq!(s, StatusCode::CREATED, "seeding user {user} failed");
    }

    // Register two webhooks for different users.
    let (s1, r1) = json_request(
        &app,
        "POST",
        "/api/v1/memory/webhooks",
        serde_json::json!({
            "user": "list-wh-user-a",
            "target_url": "http://example.com/hook-a",
            "signing_secret": "secret_a",
            "events": ["head_advanced"]
        }),
    )
    .await;
    assert_eq!(s1, StatusCode::CREATED);
    let id_a = r1["webhook"]["id"].as_str().unwrap().to_string();

    let (s2, r2) = json_request(
        &app,
        "POST",
        "/api/v1/memory/webhooks",
        serde_json::json!({
            "user": "list-wh-user-b",
            "target_url": "http://example.com/hook-b",
            "signing_secret": "secret_b",
            "events": ["conflict_detected"]
        }),
    )
    .await;
    assert_eq!(s2, StatusCode::CREATED);
    let id_b = r2["webhook"]["id"].as_str().unwrap().to_string();

    // List should now contain both webhooks.
    let (status, body) = json_request(
        &app,
        "GET",
        "/api/v1/memory/webhooks",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["count"].as_u64().unwrap(), 2);
    let data = body["data"].as_array().unwrap();
    let ids: Vec<&str> = data.iter().map(|w| w["id"].as_str().unwrap()).collect();
    assert!(ids.contains(&id_a.as_str()), "webhook A missing from list");
    assert!(ids.contains(&id_b.as_str()), "webhook B missing from list");

    // signing_secret must NOT appear in the listing (serde skip_serializing).
    for wh in data {
        assert!(
            wh.get("signing_secret").is_none(),
            "signing_secret leaked in list response"
        );
    }

    // Sorted newest-first: B registered after A, so B should be first.
    assert_eq!(data[0]["id"].as_str().unwrap(), id_b);
    assert_eq!(data[1]["id"].as_str().unwrap(), id_a);
}

// ─── Dashboard Smoke ───────────────────────────────────────────────

#[tokio::test]
async fn test_dashboard_serves_index_and_static_assets() {
    let app = build_test_app().await;

    // Index page
    let request = Request::builder()
        .method("GET")
        .uri("/_/")
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let html = String::from_utf8_lossy(&body);
    assert!(
        html.contains("Mnemo"),
        "dashboard index should contain 'Mnemo'"
    );
    assert!(
        html.contains("<!DOCTYPE html>"),
        "dashboard should serve valid HTML"
    );

    // Static CSS
    let request = Request::builder()
        .method("GET")
        .uri("/_/static/style.css")
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let css = String::from_utf8_lossy(&body);
    assert!(css.contains(":root"), "CSS should contain :root variables");

    // Static JS
    let request = Request::builder()
        .method("GET")
        .uri("/_/static/app.js")
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let js = String::from_utf8_lossy(&body);
    assert!(js.contains("use strict"), "JS should contain 'use strict'");

    // SPA catch-all: /_/webhooks should serve index.html
    let request = Request::builder()
        .method("GET")
        .uri("/_/webhooks")
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let html = String::from_utf8_lossy(&body);
    assert!(
        html.contains("<!DOCTYPE html>"),
        "SPA route should serve index"
    );

    // Non-existent static asset should 404
    let request = Request::builder()
        .method("GET")
        .uri("/_/static/nonexistent.xyz")
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    // Bare /_ without trailing slash should redirect to /_/
    let request = Request::builder()
        .method("GET")
        .uri("/_")
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert!(
        response.status().is_redirection(),
        "/_ should redirect, got {}",
        response.status()
    );
    let location = response
        .headers()
        .get("location")
        .expect("/_ redirect should have Location header")
        .to_str()
        .unwrap();
    assert_eq!(location, "/_/", "redirect should point to /_/");
}

// ── Item 1: request_id index + O(1) trace lookup ───────────────────

#[tokio::test]
async fn test_trace_lookup_uses_request_id_index() {
    // Write a memory with a known request_id, then look it up via the trace
    // endpoint. The index should return the episode without scanning all users.
    let app = build_test_app().await;
    let user = format!("trace-index-{}", Uuid::now_v7());
    let request_id = format!("req-idx-{}", Uuid::now_v7().simple());

    // Write a memory carrying the custom request_id header
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/memory")
        .header("content-type", "application/json")
        .header("x-mnemo-request-id", &request_id)
        .body(Body::from(
            serde_json::json!({
                "user": user,
                "text": "Episode written with a known request_id for index lookup test."
            })
            .to_string(),
        ))
        .unwrap();
    let resp = app.clone().oneshot(request).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::CREATED,
        "memory write should succeed"
    );

    // Trace lookup — should find the episode via the O(1) index
    let (status, body) = get_request(&app, &format!("/api/v1/traces/{request_id}")).await;
    assert_eq!(status, StatusCode::OK, "trace lookup should succeed");
    let episodes = body["matched_episodes"]
        .as_array()
        .expect("episodes should be array");
    assert_eq!(episodes.len(), 1, "should find exactly 1 episode via index");
    let ep = &episodes[0];
    assert!(
        ep["preview"]
            .as_str()
            .unwrap_or("")
            .contains("known request_id"),
        "episode preview should contain written text"
    );
}

#[tokio::test]
async fn test_trace_lookup_returns_empty_for_unknown_request_id() {
    let app = build_test_app().await;
    let unknown_rid = format!("req-never-written-{}", Uuid::now_v7().simple());
    let (status, body) = get_request(&app, &format!("/api/v1/traces/{unknown_rid}")).await;
    assert_eq!(status, StatusCode::OK);
    let episodes = body["matched_episodes"]
        .as_array()
        .expect("episodes should be array");
    assert_eq!(
        episodes.len(),
        0,
        "unknown request_id should return 0 episodes"
    );
}

#[tokio::test]
async fn test_trace_lookup_user_filter_scopes_results() {
    let app = build_test_app().await;
    let user_a = format!("trace-user-a-{}", Uuid::now_v7());
    let user_b = format!("trace-user-b-{}", Uuid::now_v7());
    let request_id = format!("req-filter-{}", Uuid::now_v7().simple());

    // Write for user_a with the shared request_id
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/memory")
        .header("content-type", "application/json")
        .header("x-mnemo-request-id", &request_id)
        .body(Body::from(
            serde_json::json!({"user": user_a, "text": "user A episode"}).to_string(),
        ))
        .unwrap();
    let r = app.clone().oneshot(req).await.unwrap();
    assert_eq!(r.status(), StatusCode::CREATED);

    // Write for user_b with the same request_id
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/memory")
        .header("content-type", "application/json")
        .header("x-mnemo-request-id", &request_id)
        .body(Body::from(
            serde_json::json!({"user": user_b, "text": "user B episode"}).to_string(),
        ))
        .unwrap();
    let r = app.clone().oneshot(req).await.unwrap();
    assert_eq!(r.status(), StatusCode::CREATED);

    // Unfiltered: should return both
    let (status, body) = get_request(&app, &format!("/api/v1/traces/{request_id}")).await;
    assert_eq!(status, StatusCode::OK);
    let all_eps = body["matched_episodes"].as_array().unwrap();
    assert_eq!(
        all_eps.len(),
        2,
        "unfiltered trace should return episodes from both users"
    );

    // Filtered to user_a: should return only user_a's episode
    let (status, body) =
        get_request(&app, &format!("/api/v1/traces/{request_id}?user={user_a}")).await;
    assert_eq!(status, StatusCode::OK);
    let filtered_eps = body["matched_episodes"].as_array().unwrap();
    assert_eq!(
        filtered_eps.len(),
        1,
        "user_a filter should return 1 episode"
    );
}

// ── Item 2: POST /api/v1/memory/extract ────────────────────────────

#[tokio::test]
async fn test_extract_endpoint_returns_ok_with_no_llm() {
    // In the test harness LLM is None — the endpoint should return ok=true with
    // an empty extraction and a `note` field explaining no LLM is configured.
    let app = build_test_app().await;
    let (status, body) = json_request(
        &app,
        "POST",
        "/api/v1/memory/extract",
        serde_json::json!({ "text": "Alice works at Acme Corp as a senior engineer." }),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "extract should return 200 even with no LLM"
    );
    assert_eq!(body["ok"].as_bool(), Some(true));
    assert_eq!(
        body["entity_count"].as_u64(),
        Some(0),
        "no LLM → entity_count should be 0"
    );
    assert_eq!(
        body["relationship_count"].as_u64(),
        Some(0),
        "no LLM → relationship_count should be 0"
    );
    let note = body["note"]
        .as_str()
        .expect("no_llm note should be present");
    assert!(
        note.contains("no_llm"),
        "note should contain 'no_llm' marker"
    );
}

#[tokio::test]
async fn test_extract_endpoint_rejects_empty_text() {
    let app = build_test_app().await;
    let (status, body) = json_request(
        &app,
        "POST",
        "/api/v1/memory/extract",
        serde_json::json!({ "text": "" }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "empty text should return 400"
    );
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("text is required"),
        "error message should mention text required"
    );
}

#[tokio::test]
async fn test_extract_endpoint_accepts_optional_user() {
    // Passing a known user should not error even if user doesn't exist
    // (hints fall back to empty list gracefully).
    let app = build_test_app().await;
    let (status, body) = json_request(
        &app,
        "POST",
        "/api/v1/memory/extract",
        serde_json::json!({
            "text": "Bob loves hiking.",
            "user": "nonexistent-user-for-extract-test"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ok"].as_bool(), Some(true));
}

// ── Item 3: GET /api/v1/audit/export ───────────────────────────────

#[tokio::test]
async fn test_audit_export_returns_ok_when_empty() {
    let app = build_test_app().await;
    let (status, body) = get_request(&app, "/api/v1/audit/export").await;
    assert_eq!(status, StatusCode::OK, "audit/export should return 200");
    assert_eq!(body["ok"].as_bool(), Some(true));
    assert!(body["records"].is_array(), "records should be array");
    let total = body["total"].as_u64().unwrap_or(u64::MAX);
    assert!(
        total < 1000,
        "fresh test namespace should have < 1000 audit records"
    );
}

#[tokio::test]
async fn test_audit_export_returns_governance_events() {
    let app = build_test_app().await;
    let user = format!("audit-export-user-{}", Uuid::now_v7());

    // Create user first (policy endpoint requires user to exist)
    let (status, _) = json_request(
        &app,
        "POST",
        "/api/v1/memory",
        serde_json::json!({ "user": user, "text": "seed episode for audit export test" }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "seed memory write should succeed"
    );

    // Write a governance policy to generate an audit event
    let (status, _) = json_request(
        &app,
        "PUT",
        &format!("/api/v1/policies/{user}"),
        serde_json::json!({ "retention_days_message": 90 }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "policy set should succeed");

    // Export audit — should include the policy_updated event
    let (status, body) = get_request(&app, "/api/v1/audit/export?include_webhook=false").await;
    assert_eq!(status, StatusCode::OK);
    let records = body["records"].as_array().unwrap();
    let has_governance_event = records
        .iter()
        .any(|r| r["audit_type"].as_str() == Some("governance"));
    assert!(
        has_governance_event,
        "audit export should contain governance events after policy write"
    );
}

#[tokio::test]
async fn test_audit_export_from_to_filtering() {
    let app = build_test_app().await;
    // Request a window entirely in the past (year 2000) — should return 0 records
    let (status, body) = get_request(
        &app,
        "/api/v1/audit/export?from=2000-01-01T00:00:00Z&to=2000-01-02T00:00:00Z",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let records = body["records"].as_array().unwrap();
    assert_eq!(
        records.len(),
        0,
        "window in distant past should return 0 records"
    );
}

#[tokio::test]
async fn test_audit_export_to_before_from_returns_400() {
    let app = build_test_app().await;
    let (status, _) = get_request(
        &app,
        "/api/v1/audit/export?from=2025-12-01T00:00:00Z&to=2025-01-01T00:00:00Z",
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "to < from should return 400"
    );
}

#[tokio::test]
async fn test_audit_export_include_flags() {
    let app = build_test_app().await;

    // include_governance=false, include_webhook=false → should return 0 records
    let (status, body) = get_request(
        &app,
        "/api/v1/audit/export?include_governance=false&include_webhook=false",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let records = body["records"].as_array().unwrap();
    assert_eq!(
        records.len(),
        0,
        "both include flags false should return 0 records"
    );
}

// ─── Memory Digest Endpoint Tests ─────────────────────────────────

#[tokio::test]
async fn test_digest_get_returns_404_when_no_digest() {
    let app = build_test_app().await;

    // Create a user
    let (status, user_body) = json_request(
        &app,
        "POST",
        "/api/v1/users",
        serde_json::json!({
            "name": "digest-test-user",
            "external_id": "digest-ext-no-digest"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let user_id = user_body["id"].as_str().unwrap();

    // GET digest should return 404
    let (status, _body) = get_request(&app, &format!("/api/v1/memory/{user_id}/digest")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_digest_persisted_to_redis_and_served() {
    let (app, state_store) = build_test_harness_with_prefilter(MetadataPrefilterConfig {
        enabled: true,
        scan_limit: 400,
        relax_if_empty: false,
    })
    .await;

    // Create a user
    let (status, user_body) = json_request(
        &app,
        "POST",
        "/api/v1/users",
        serde_json::json!({
            "name": "digest-persist-user",
            "external_id": "digest-ext-persist"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let user_id: Uuid = user_body["id"].as_str().unwrap().parse().unwrap();

    // Manually save a digest to Redis via DigestStore (simulating what the ingest worker does).
    // Note: the in-memory cache is NOT populated — this tests the read-through fallback.
    use mnemo_core::traits::storage::DigestStore;
    let digest = mnemo_core::models::digest::MemoryDigest {
        user_id,
        summary: "User is passionate about distributed systems and Rust.".into(),
        entity_count: 12,
        edge_count: 7,
        dominant_topics: vec!["distributed systems".into(), "Rust".into()],
        generated_at: chrono::Utc::now(),
        model: "test-model".into(),
    };
    state_store.save_digest(&digest).await.unwrap();

    // GET should find the digest via read-through from Redis (cache miss → Redis → cache populate)
    let (status, body) = get_request(&app, &format!("/api/v1/memory/{user_id}/digest")).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "GET digest should succeed via read-through"
    );
    assert_eq!(body["summary"].as_str().unwrap(), digest.summary);
    assert_eq!(body["entity_count"].as_u64().unwrap(), 12);
    assert_eq!(body["model"].as_str().unwrap(), "test-model");
    let topics: Vec<&str> = body["dominant_topics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(topics, vec!["distributed systems", "Rust"]);

    // Second GET should hit the now-populated in-memory cache (fast path)
    let (status2, body2) = get_request(&app, &format!("/api/v1/memory/{user_id}/digest")).await;
    assert_eq!(status2, StatusCode::OK);
    assert_eq!(body2["summary"].as_str().unwrap(), digest.summary);

    // Verify list_digests returns it from Redis
    let all = state_store.list_digests().await.unwrap();
    assert!(all.iter().any(|d| d.user_id == user_id));
}

#[tokio::test]
async fn test_digest_post_without_llm_returns_error() {
    let app = build_test_app().await;

    // Create a user
    let (status, user_body) = json_request(
        &app,
        "POST",
        "/api/v1/users",
        serde_json::json!({
            "name": "digest-no-llm-user",
            "external_id": "digest-ext-no-llm"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let user_id = user_body["id"].as_str().unwrap();

    // POST digest without LLM should return 400 (validation error)
    let (status, body) = json_request(
        &app,
        "POST",
        &format!("/api/v1/memory/{user_id}/digest"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("LLM provider"),
        "error should mention LLM provider not configured, got: {body}"
    );
}

// ─── Graph API Integration Tests ───────────────────────────────────────────

/// Helper: create a user, entities, and edges for graph API tests.
/// Returns (user_id, entity_ids: [A, B, C, D]) where:
///   A -> B -> C (chain), A -> D (separate branch), all valid
async fn setup_graph_data(state_store: &Arc<RedisStateStore>) -> (Uuid, [Uuid; 4]) {
    use mnemo_core::models::user::CreateUserRequest;

    let user = state_store
        .create_user(CreateUserRequest {
            id: None,
            name: "graph-test-user".into(),
            external_id: Some(format!("graph-ext-{}", Uuid::now_v7())),
            email: None,
            metadata: serde_json::Value::Null,
        })
        .await
        .unwrap();

    let now = chrono::Utc::now();
    let mut entity_ids = [Uuid::nil(); 4];
    let names = ["Alpha", "Beta", "Gamma", "Delta"];
    let types = [
        EntityType::Person,
        EntityType::Concept,
        EntityType::Concept,
        EntityType::Location,
    ];

    for (i, (name, etype)) in names.iter().zip(types.iter()).enumerate() {
        let entity = Entity {
            id: Uuid::now_v7(),
            user_id: user.id,
            name: name.to_string(),
            entity_type: etype.clone(),
            summary: Some(format!("{name} summary")),
            aliases: vec![],
            metadata: serde_json::Value::Null,
            mention_count: 1,
            community_id: None,
            created_at: now,
            updated_at: now,
        };
        let created = state_store.create_entity(entity).await.unwrap();
        entity_ids[i] = created.id;
    }

    // Edges: A->B, B->C, A->D
    let edges = [
        (entity_ids[0], entity_ids[1], "knows"),
        (entity_ids[1], entity_ids[2], "related_to"),
        (entity_ids[0], entity_ids[3], "located_in"),
    ];
    for (src, tgt, label) in &edges {
        let edge = Edge {
            id: Uuid::now_v7(),
            user_id: user.id,
            source_entity_id: *src,
            target_entity_id: *tgt,
            label: label.to_string(),
            fact: format!("{} {} {}", src, label, tgt),
            valid_at: now,
            invalid_at: None,
            ingested_at: now,
            source_episode_id: Uuid::now_v7(),
            invalidated_by_episode_id: None,
            confidence: 0.9,
            corroboration_count: 1,
            metadata: serde_json::Value::Null,
            created_at: now,
            updated_at: now,
        };
        state_store.create_edge(edge).await.unwrap();
    }

    (user.id, entity_ids)
}

// ── GRAPH-01: List entities returns all entities for user ──────────

#[tokio::test]
async fn test_graph_list_entities() {
    let (app, state_store) = build_test_harness_with_prefilter(MetadataPrefilterConfig {
        enabled: false,
        scan_limit: 400,
        relax_if_empty: false,
    })
    .await;
    let (user_id, _eids) = setup_graph_data(&state_store).await;

    let (status, body) = get_request(&app, &format!("/api/v1/graph/{user_id}/entities")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["count"].as_u64().unwrap(), 4);
    assert_eq!(body["data"].as_array().unwrap().len(), 4);
    assert_eq!(body["user_id"].as_str().unwrap(), user_id.to_string());
}

// ── GRAPH-02: List entities with entity_type filter ────────────────

#[tokio::test]
async fn test_graph_list_entities_type_filter() {
    let (app, state_store) = build_test_harness_with_prefilter(MetadataPrefilterConfig {
        enabled: false,
        scan_limit: 400,
        relax_if_empty: false,
    })
    .await;
    let (user_id, _eids) = setup_graph_data(&state_store).await;

    // Filter by "concept" — should return Beta and Gamma
    let (status, body) = get_request(
        &app,
        &format!("/api/v1/graph/{user_id}/entities?entity_type=concept"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["count"].as_u64().unwrap(),
        2,
        "expected 2 concept entities, got: {body}"
    );

    // Filter by "person" — should return Alpha only
    let (status, body) = get_request(
        &app,
        &format!("/api/v1/graph/{user_id}/entities?entity_type=person"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["count"].as_u64().unwrap(), 1);
    assert_eq!(body["data"][0]["name"].as_str().unwrap(), "Alpha");
}

// ── GRAPH-03: List entities with name filter ───────────────────────

#[tokio::test]
async fn test_graph_list_entities_name_filter() {
    let (app, state_store) = build_test_harness_with_prefilter(MetadataPrefilterConfig {
        enabled: false,
        scan_limit: 400,
        relax_if_empty: false,
    })
    .await;
    let (user_id, _eids) = setup_graph_data(&state_store).await;

    // Substring match: "lpha" should match "Alpha" only
    let (status, body) =
        get_request(&app, &format!("/api/v1/graph/{user_id}/entities?name=lpha")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["count"].as_u64().unwrap(),
        1,
        "expected 1 entity matching 'lpha', got: {body}"
    );
    assert_eq!(body["data"][0]["name"].as_str().unwrap(), "Alpha");

    // Case-insensitive: "BETA" should match "Beta"
    let (status, body) =
        get_request(&app, &format!("/api/v1/graph/{user_id}/entities?name=BETA")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["count"].as_u64().unwrap(),
        1,
        "expected 1 entity matching 'BETA', got: {body}"
    );
}

// ── GRAPH-04: Get single entity with adjacency ─────────────────────

#[tokio::test]
async fn test_graph_get_entity() {
    let (app, state_store) = build_test_harness_with_prefilter(MetadataPrefilterConfig {
        enabled: false,
        scan_limit: 400,
        relax_if_empty: false,
    })
    .await;
    let (user_id, eids) = setup_graph_data(&state_store).await;

    let (status, body) = get_request(
        &app,
        &format!("/api/v1/graph/{user_id}/entities/{}", eids[0]),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["name"].as_str().unwrap(), "Alpha");
    // Alpha has 2 outgoing edges (->Beta, ->Delta) and 0 incoming
    assert_eq!(body["outgoing_edges"].as_array().unwrap().len(), 2);
    assert_eq!(body["incoming_edges"].as_array().unwrap().len(), 0);
}

// ── GRAPH-05: Get entity cross-user returns 404 ────────────────────

#[tokio::test]
async fn test_graph_get_entity_cross_user_404() {
    let (app, state_store) = build_test_harness_with_prefilter(MetadataPrefilterConfig {
        enabled: false,
        scan_limit: 400,
        relax_if_empty: false,
    })
    .await;
    let (_user_id, eids) = setup_graph_data(&state_store).await;

    // Create a different user
    let (status, other_user) = json_request(
        &app,
        "POST",
        "/api/v1/users",
        serde_json::json!({ "name": "other-user" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let other_id = other_user["id"].as_str().unwrap();

    // Try to access first user's entity via second user
    let (status, _body) = get_request(
        &app,
        &format!("/api/v1/graph/{other_id}/entities/{}", eids[0]),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ── GRAPH-06: List edges with filters ──────────────────────────────

#[tokio::test]
async fn test_graph_list_edges() {
    let (app, state_store) = build_test_harness_with_prefilter(MetadataPrefilterConfig {
        enabled: false,
        scan_limit: 400,
        relax_if_empty: false,
    })
    .await;
    let (user_id, eids) = setup_graph_data(&state_store).await;

    // List all edges
    let (status, body) = get_request(&app, &format!("/api/v1/graph/{user_id}/edges")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["count"].as_u64().unwrap(),
        3,
        "expected 3 edges, got: {body}"
    );

    // Filter by label
    let (status, body) =
        get_request(&app, &format!("/api/v1/graph/{user_id}/edges?label=knows")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["count"].as_u64().unwrap(), 1);

    // Filter by source_entity_id
    let (status, body) = get_request(
        &app,
        &format!("/api/v1/graph/{user_id}/edges?source_entity_id={}", eids[0]),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["count"].as_u64().unwrap(),
        2,
        "Alpha has 2 outgoing edges"
    );

    // Filter by target_entity_id
    let (status, body) = get_request(
        &app,
        &format!("/api/v1/graph/{user_id}/edges?target_entity_id={}", eids[1]),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["count"].as_u64().unwrap(),
        1,
        "Beta has 1 incoming edge"
    );
}

// ── GRAPH-07: Neighbors endpoint ───────────────────────────────────

#[tokio::test]
async fn test_graph_neighbors() {
    let (app, state_store) = build_test_harness_with_prefilter(MetadataPrefilterConfig {
        enabled: false,
        scan_limit: 400,
        relax_if_empty: false,
    })
    .await;
    let (user_id, eids) = setup_graph_data(&state_store).await;

    // 1-hop neighbors of Alpha: Beta and Delta
    let (status, body) = get_request(
        &app,
        &format!("/api/v1/graph/{user_id}/neighbors/{}", eids[0]),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["seed_entity_id"].as_str().unwrap(),
        eids[0].to_string()
    );
    // Nodes should include Alpha (seed, depth=0) + Beta + Delta (depth=1) = 3
    assert_eq!(
        body["nodes"].as_array().unwrap().len(),
        3,
        "expected 3 nodes (Alpha + 2 neighbors), got: {body}"
    );
}

// ── GRAPH-08: Community detection ──────────────────────────────────

#[tokio::test]
async fn test_graph_community() {
    let (app, state_store) = build_test_harness_with_prefilter(MetadataPrefilterConfig {
        enabled: false,
        scan_limit: 400,
        relax_if_empty: false,
    })
    .await;
    let (user_id, _eids) = setup_graph_data(&state_store).await;

    let (status, body) = get_request(&app, &format!("/api/v1/graph/{user_id}/community")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total_entities"].as_u64().unwrap(), 4);
    assert!(body["community_count"].as_u64().unwrap() >= 1);
    assert!(!body["communities"].as_array().unwrap().is_empty());
}

// ── GRAPH-09: Shortest path found ──────────────────────────────────

#[tokio::test]
async fn test_graph_shortest_path_found() {
    let (app, state_store) = build_test_harness_with_prefilter(MetadataPrefilterConfig {
        enabled: false,
        scan_limit: 400,
        relax_if_empty: false,
    })
    .await;
    let (user_id, eids) = setup_graph_data(&state_store).await;

    // Path from Alpha to Gamma: A -> B -> C (2 hops)
    let (status, body) = get_request(
        &app,
        &format!(
            "/api/v1/graph/{user_id}/path?from={}&to={}",
            eids[0], eids[2]
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["found"].as_bool().unwrap(), "path should be found");
    assert_eq!(
        body["path_length"].as_u64().unwrap(),
        2,
        "A->B->C is 2 hops"
    );
    assert_eq!(body["steps"].as_array().unwrap().len(), 3);
    assert_eq!(body["steps"][0]["entity_name"].as_str().unwrap(), "Alpha");
    assert_eq!(body["steps"][2]["entity_name"].as_str().unwrap(), "Gamma");
}

// ── GRAPH-10: Shortest path not found ──────────────────────────────

#[tokio::test]
async fn test_graph_shortest_path_not_found() {
    let (app, state_store) = build_test_harness_with_prefilter(MetadataPrefilterConfig {
        enabled: false,
        scan_limit: 400,
        relax_if_empty: false,
    })
    .await;
    let (user_id, _eids) = setup_graph_data(&state_store).await;

    // Create an isolated entity with no edges
    let now = chrono::Utc::now();
    let isolated = Entity {
        id: Uuid::now_v7(),
        user_id,
        name: "Isolated".to_string(),
        entity_type: EntityType::Concept,
        summary: None,
        aliases: vec![],
        metadata: serde_json::Value::Null,
        mention_count: 1,
        community_id: None,
        created_at: now,
        updated_at: now,
    };
    let isolated = state_store.create_entity(isolated).await.unwrap();

    let (status, body) = get_request(
        &app,
        &format!(
            "/api/v1/graph/{user_id}/path?from={}&to={}",
            _eids[0], isolated.id
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        !body["found"].as_bool().unwrap(),
        "path should not be found"
    );
    assert_eq!(body["path_length"].as_u64().unwrap(), 0);
    assert!(body["steps"].as_array().unwrap().is_empty());
}

// ── GRAPH-11: Shortest path cross-user entity returns 404 ──────────

#[tokio::test]
async fn test_graph_shortest_path_cross_user() {
    let (app, state_store) = build_test_harness_with_prefilter(MetadataPrefilterConfig {
        enabled: false,
        scan_limit: 400,
        relax_if_empty: false,
    })
    .await;
    let (_user_id, eids) = setup_graph_data(&state_store).await;

    // Create a different user
    let (status, other_user) = json_request(
        &app,
        "POST",
        "/api/v1/users",
        serde_json::json!({ "name": "other-graph-user" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let other_id = other_user["id"].as_str().unwrap();

    // Try to find path using other user — entities don't belong to them
    let (status, _body) = get_request(
        &app,
        &format!(
            "/api/v1/graph/{other_id}/path?from={}&to={}",
            eids[0], eids[1]
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ── GRAPH-12: Subgraph endpoint ────────────────────────────────────

#[tokio::test]
async fn test_graph_subgraph() {
    let (app, state_store) = build_test_harness_with_prefilter(MetadataPrefilterConfig {
        enabled: false,
        scan_limit: 400,
        relax_if_empty: false,
    })
    .await;
    let (_user_id, eids) = setup_graph_data(&state_store).await;

    let (status, body) = get_request(
        &app,
        &format!("/api/v1/entities/{}/subgraph?depth=1", eids[0]),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    // Alpha at depth=0, Beta+Delta at depth=1 = 3 nodes
    assert_eq!(
        body["nodes"].as_array().unwrap().len(),
        3,
        "expected 3 nodes in subgraph, got: {body}"
    );
    assert!(body["edges"].as_array().unwrap().len() >= 2);
}
